//! A transparent shim around the `claude` binary the VSCode extension spawns.
//!
//! The extension spawns Claude Code and keeps its stdio, which is an anonymous unix
//! socketpair — unnamed, and obtainable only by having been there at `exec`. That is
//! why nothing can type into a running session from outside (see ARCHITECTURE.md).
//! So the router stops trying to reach in, and instead *becomes the parent*:
//!
//! ```text
//! extension  <->  claude-wrapper  <->  claude (real)
//!                       ^
//!                       |  control socket: injected user turns
//!                  Agent Router
//! ```
//!
//! A write on the control socket is relayed into the child's stdin verbatim, so it is
//! the same bytes the extension writes when someone types. Not a peer message, not a
//! cross-session frame — an ordinary user turn. It goes in whole, as a line of its own
//! between two of the extension's, or not at all.
//!
//! # This process is in the critical path of every session
//!
//! If it dies, the session dies with it. So:
//!
//!   * The relay is a byte pump. It never parses anything it must understand, never
//!     transforms, and never buffers more than one read.
//!   * Every added feature — the sniffer, the recorder, the control socket — is allowed
//!     to fail. Any error there degrades this to a plain pass-through rather than taking
//!     the session with it.
//!   * No dependencies. std only, so there is nothing to break on an update.
//!
//! # What it leaves for the router
//!
//! One directory, every name in it the session id:
//!
//!   * `<id>.sock` — the control socket.
//!   * `<id>.ctl` — the last 40 control lines both ways, so the router can read back the
//!     CLI's answer to a request it injected.
//!   * `<id>.pending` — the permission prompts still waiting for an answer, oldest first,
//!     each line exactly as the CLI wrote it. Always replaced whole, never edited.
//!
//! The id is the conversation's, not the process's: when the CLI carries on in a new
//! conversation inside the same process (`/clear`, leaving plan mode with a cleared
//! context), the three files move to the new id, so the router keeps reaching the
//! conversation it lists. All of it is removed on the way out — also when the editor's
//! SIGTERM comes before the CLI is done, which is passed on to the CLI rather than taken
//! as the end (see `signals`).
//!
//! Install: move the real binary aside to `claude.real`, put this in its place.

use std::io::{ErrorKind, Read, Write};
use std::net::Shutdown;
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::process::ExitStatusExt;
use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::process::{ChildStdin, Command, Stdio};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::{Duration, Instant};

/// Where a wrapped session offers its control socket. Named by session id once known,
/// so the router can address a session by the same id its transcript carries.
const CONTROL_DIR: &str = "/tmp/agent-router-sessions";

/// Test-only: publish into this directory instead of [`CONTROL_DIR`].
///
/// The lab test runs this binary around a fake child, and a socket it left in the
/// directory the router watches would be taken for a session. Nothing in production sets
/// it; a relative path is ignored rather than resolved against wherever the CLI started.
const TEST_DIR_VAR: &str = "AGENT_ROUTER_WRAPPER_TEST_DIR";

/// The real binary lives beside us under this name.
const REAL_SUFFIX: &str = ".real";

/// How much of the stream may go by before the first session id. It arrives in the first
/// message or not at all.
const SNIFF_LIMIT: usize = 256 * 1024;

/// The longest line the recorder carries from one read to the next. Nothing it looks
/// for is longer; a line that is gets skipped, not buffered.
const LINE_LIMIT: usize = 256 * 1024;

/// How often an unchanged `.pending` is written again while prompts are open.
const HEAL_EVERY: Duration = Duration::from_secs(1);

/// The longest frame the control socket takes, its newline not counted. A longer one is
/// refused whole, never cut to size: the CLI exits on a stdin line it cannot parse, and a
/// frame cut short is one. The router refuses to send anything longer.
const FRAME_LIMIT: usize = 1024 * 1024;

/// How long an injected line may wait for the stream it joins to reach the end of a line
/// (see [`Shared`]). Past this it is refused — or, upstream, dropped — rather than risked.
const LINE_WAIT: Duration = Duration::from_secs(1);

/// How long the relay may keep draining once the child is gone. Its output is complete
/// when every holder of its stdout has closed it, but a background command the CLI started
/// can hold that pipe open long after the CLI exited — and with the child reaped, a
/// SIGTERM from the editor has nobody left to be passed on to. What the CLI itself left in
/// the pipe drains in far less than this.
const DRAIN_GRACE: Duration = Duration::from_secs(1);

/// The child's stdin, shared by the editor's pump and the control socket. Closed once the
/// editor has closed its end: dropping the handle is what lets the child see EOF.
type Sink = Arc<Shared<ChildStdin>>;

/// The editor's end, shared because answering a prompt has to speak upstream as well as
/// down.
type Up = Arc<Shared<std::io::Stdout>>;

/// The session id this process's files are named after: `None` until the stream names
/// one, then whatever conversation the CLI says it is in.
type Current = Arc<Mutex<Option<String>>>;

/// A lock that survives a panic elsewhere. What these locks guard is a handle or a name,
/// with no invariant a panicking holder could leave half-updated — and refusing the lock
/// would stop the relay, the one thing this process must never do.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A stream with two writers: the relay, which passes each read on as it came, and the
/// control socket, which adds lines of its own.
///
/// Between two of the relay's writes the stream can be in the middle of a line: a frame
/// larger than one read — a pasted block, an image, a long answer — goes through in
/// pieces. A line written in between would be spliced into it, and the reader gets one
/// line it cannot parse where there were two it could: the CLI exits on such a line, the
/// editor loses both frames. So the relay's writes go through as they come, and an added
/// line waits for the stream to be at the end of one.
struct Shared<W> {
    state: Mutex<Line<W>>,
    /// Signalled when the stream reaches the end of a line, or closes, while an added
    /// line is waiting for it.
    boundary: Condvar,
}

/// A [`Shared`] stream's writer, and where the stream stands.
struct Line<W> {
    /// `None` once closed.
    w: Option<W>,
    /// The last byte through was a newline, or nothing has been written yet.
    at_line_start: bool,
    /// Added lines waiting for a line end. The relay only signals when one is, so a read
    /// passed on costs it nothing it did not cost before.
    waiting: usize,
}

impl<W: Write> Shared<W> {
    fn new(w: Option<W>) -> Shared<W> {
        Shared {
            state: Mutex::new(Line {
                w,
                at_line_start: true,
                waiting: 0,
            }),
            boundary: Condvar::new(),
        }
    }

    /// The relay's write: one read's worth, passed on as it came. False once the stream
    /// is closed or will not take it. Never waits for anything but the writer itself.
    fn pass(&self, chunk: &[u8]) -> bool {
        let mut line = lock(&self.state);
        let Some(w) = line.w.as_mut() else {
            return false;
        };
        if w.write_all(chunk).is_err() || w.flush().is_err() {
            // Cut short, so perhaps mid-line: nothing may be added after it.
            line.at_line_start = false;
            return false;
        }
        if let Some(&last) = chunk.last() {
            line.at_line_start = last == b'\n';
        }
        if line.at_line_start && line.waiting > 0 {
            self.boundary.notify_all();
        }
        true
    }

    /// Add one whole line, newline included, where a line ends: at once if the stream is
    /// at one, else as soon as the relay finishes the line it is in. False, with nothing
    /// written, if that takes longer than `patience` or the stream closes first.
    fn insert(&self, line: &[u8], patience: Duration) -> bool {
        let deadline = Instant::now() + patience;
        let mut state = lock(&self.state);
        while state.w.is_some() && !state.at_line_start {
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            state.waiting += 1;
            state = self
                .boundary
                .wait_timeout(state, deadline - now)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
            state.waiting -= 1;
        }
        let Some(w) = state.w.as_mut() else {
            return false;
        };
        let ok = w.write_all(line).is_ok() && w.flush().is_ok();
        state.at_line_start = ok && line.last() == Some(&b'\n');
        ok
    }

    /// No more writes. Dropping the writer is what lets the reader see EOF; an added line
    /// still waiting gives up at once.
    fn close(&self) {
        drop(lock(&self.state).w.take());
        self.boundary.notify_all();
    }
}

/// An absolute path to something runnable: what the editor hands a process wrapper.
fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    p.is_absolute()
        && std::fs::metadata(p)
            .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
}

/// The directory this process publishes into, decided once.
fn control_dir() -> &'static Path {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| resolve_dir(std::env::var_os(TEST_DIR_VAR)))
}

/// [`CONTROL_DIR`], unless the test-only variable names an absolute directory.
fn resolve_dir(test_dir: Option<std::ffi::OsString>) -> PathBuf {
    test_dir
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| PathBuf::from(CONTROL_DIR))
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    // Two ways to be run. As Claude Code's `claudeProcessWrapper` setting, the editor
    // passes the real binary's path as the first argument. As the shim that replaced the
    // editor's own `claude`, the real binary lives beside us as `claude.real`.
    let real = match args.first().map(PathBuf::from) {
        Some(p) if is_executable(&p) => {
            args.remove(0);
            p
        }
        _ => {
            let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("claude"));
            let mut p = exe.into_os_string();
            p.push(REAL_SUFFIX);
            PathBuf::from(p)
        }
    };
    if std::env::var_os("AGENT_ROUTER_WRAPPER_DEBUG").is_some() {
        eprintln!("claude-wrapper: real={} args={args:?}", real.display());
    }

    // Nothing to wrap: exec the real thing if we can find it, or fail exactly as the
    // missing binary would have.
    if !real.exists() {
        eprintln!("claude-wrapper: {} not found; is the shim half-installed?", real.display());
        std::process::exit(127);
    }

    let mut child = match Command::new(&real)
        .args(&args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            eprintln!("claude-wrapper: could not start {}: {e}", real.display());
            std::process::exit(127);
        }
    };
    // From here the editor's SIGTERM goes on to the CLI instead of ending this process
    // before its cleanup.
    signals::forward_to(child.id());

    let dir = control_dir();
    let child_stdout = child.stdout.take().expect("piped");

    // One writer, shared by the extension and by the control socket. The extension's
    // bytes go through as they come; an injected turn goes in only between two of its
    // lines, never into the middle of one (see [`Shared`]).
    let sink: Sink = Arc::new(Shared::new(child.stdin.take()));
    let up: Up = Arc::new(Shared::new(Some(std::io::stdout())));

    // extension -> child. This is the direction that must never stall.
    {
        let sink = Arc::clone(&sink);
        std::thread::spawn(move || pump_stdin(&sink));
    }

    // child -> extension, with the session id read out of the stream in passing.
    let current: Current = Arc::new(Mutex::new(None));
    let relay = {
        let current = Arc::clone(&current);
        let sink = Arc::clone(&sink);
        let up = Arc::clone(&up);
        std::thread::spawn(move || {
            let mut src = child_stdout;
            let mut buf = [0u8; 64 * 1024];
            let mut scan: Vec<u8> = Vec::new();
            let mut found = false;
            let mut recorder = Some(Recorder::new(dir, Arc::clone(&current)));
            loop {
                let n = match src.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                // Downstream first, always. Everything below only ever sees a copy.
                if !up.pass(&buf[..n]) {
                    break;
                }

                if !found {
                    scan.extend_from_slice(&buf[..n]);
                    if let Some(id) = sniff_session_id(&scan) {
                        // Before anything answers under the name, and before this read is
                        // recorded — a prompt in it is this session's own.
                        remove_leftovers(dir, &id);
                        *lock(&current) = Some(id);
                        found = true;
                        scan = Vec::new();
                        // Publishing is allowed to fail, down to the thread it runs on; a
                        // session without a control socket is simply one the router
                        // cannot type into. `thread::spawn` would panic instead, here in
                        // the relay, and take the session with it.
                        let current = Arc::clone(&current);
                        let sink = Arc::clone(&sink);
                        let up = Arc::clone(&up);
                        let _ = std::thread::Builder::new()
                            .spawn(move || serve_control(dir, &current, &sink, &up));
                    } else if scan.len() > SNIFF_LIMIT {
                        // The id arrives in the first message or not at all.
                        found = true;
                        scan = Vec::new();
                    }
                }

                // Control traffic, recorded so the router can see what the session is
                // blocked on. A prompt is a control_request the editor answers with a
                // matching control_response; without the request_id nothing else can
                // answer it. Best-effort, like everything else here: a panic in the
                // recorder retires the recorder, never the relay.
                if let Some(r) = recorder.as_mut() {
                    let chunk = &buf[..n];
                    if std::panic::catch_unwind(AssertUnwindSafe(|| r.observe(chunk))).is_err() {
                        recorder = None;
                    }
                }
            }
        })
    };

    let status = child.wait();
    signals::child_reaped();
    let drained_by = Instant::now() + DRAIN_GRACE;
    while !relay.is_finished() && Instant::now() < drained_by {
        std::thread::sleep(Duration::from_millis(10));
    }
    // A relay still running holds nothing of the CLI's any more; exiting ends it.
    if relay.is_finished() {
        let _ = relay.join();
    }
    // Whatever name the session ended under.
    let id = lock(&current).clone();
    if let Some(id) = id {
        remove_records(dir, &id);
    }
    // The child's end is this process's: its exit code, or the signal it died of when
    // that was one passed on to it (see `signals`).
    if let Some(sig) = status.as_ref().ok().and_then(|s| s.signal()) {
        signals::die_of_if_passed_on(sig);
    }
    std::process::exit(status.ok().and_then(|s| s.code()).unwrap_or(1));
}

/// extension -> child, until the editor closes its end — and then the child's stdin is
/// closed too.
///
/// The handle is shared with the control socket, so merely leaving this loop would keep
/// the pipe open and the child would never see the EOF the editor sent. Closing the
/// shared handle passes the EOF on, and an injection after that finds nothing to write
/// into and fails cleanly.
///
/// The editor closes a session with "end stdin, wait 2 s, SIGTERM". The EOF alone is
/// enough only for an idle session: an idle CLI exits within milliseconds of it, and the
/// normal way out removes the files long before the SIGTERM. A busy one — a turn
/// streaming, a background task running, a prompt open — carries on well past 2 s, and
/// for that one it is the SIGTERM, passed on to the CLI (see `signals`), that ends it
/// and lets the same way out run.
fn pump_stdin(sink: &Shared<ChildStdin>) {
    let mut buf = [0u8; 64 * 1024];
    let mut input = std::io::stdin();
    loop {
        let n = match input.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            // A signal is not the end of the stream, and ending here closes the child's
            // stdin: only a real EOF or a real error may do that.
            Err(e) if e.kind() == ErrorKind::Interrupted => continue,
            Err(_) => break,
        };
        if !sink.pass(&buf[..n]) {
            break;
        }
    }
    sink.close();
}

/// What one line of the child's stdout means to the recorder.
#[derive(Debug, PartialEq)]
enum Frame {
    /// A control request, kept in `.ctl`. `prompt` when it is a `can_use_tool` the host
    /// has to answer; `hook` when it is a `hook_callback`, which can name the session
    /// (see [`hook_session`] — only asked when that is wanted).
    Request {
        request_id: Option<String>,
        prompt: bool,
        hook: bool,
    },
    /// An answer to a control request, kept in `.ctl`. The CLI echoes the host's answers
    /// (it runs with `--replay-user-messages`), so a prompt answered in the editor shows
    /// up here as well as one answered through the socket.
    Response { request_id: Option<String> },
    /// The CLI withdrew a request: interrupted, answered some other way, timed out. Its
    /// `"type":"control_cancel_request"` does not contain `"type":"control_request"`, so
    /// before this had a name of its own a withdrawn prompt stayed pending for good.
    Cancel { request_id: Option<String> },
    /// `system/init`, naming the session the CLI says it is.
    Init { session_id: String },
    /// `conversation_reset`: the CLI threw the conversation away and goes on under a new
    /// session id, which this frame does not carry (see [`Recorder::observe`]).
    Reset,
    Other,
}

/// Classify a line by its `type`, without parsing it.
///
/// The CLI writes every frame with `type` as its first key, so for all but a handful of
/// lines — above all the stream of partial messages — one prefix comparison settles it.
/// A line in any other shape falls back to the substring test this recorder has always
/// used for control traffic, so an oddly ordered control frame is still kept.
fn classify(line: &[u8]) -> Frame {
    let kind = match frame_type(line) {
        Some(kind) => kind,
        None if find(line, br#""type":"control_request""#).is_some() => {
            b"control_request".as_slice()
        }
        None if find(line, br#""type":"control_response""#).is_some() => {
            b"control_response".as_slice()
        }
        None => return Frame::Other,
    };
    let text = |key: &[u8]| {
        field_bytes(line, key)
            .and_then(|v| std::str::from_utf8(v).ok())
            .map(str::to_string)
    };
    match kind {
        b"control_request" => {
            // The first `subtype` in a request is the request's own: `type`, then
            // `request_id`, then `request` with `subtype` leading it.
            let subtype = field_bytes(line, br#""subtype":""#);
            Frame::Request {
                request_id: text(br#""request_id":""#),
                prompt: subtype == Some(b"can_use_tool".as_slice()),
                hook: subtype == Some(b"hook_callback".as_slice()),
            }
        }
        b"control_response" => Frame::Response {
            request_id: text(br#""request_id":""#),
        },
        b"control_cancel_request" => Frame::Cancel {
            request_id: text(br#""request_id":""#),
        },
        b"system" if field_bytes(line, br#""subtype":""#) == Some(b"init".as_slice()) => {
            match init_session(line) {
                Some(session_id) => Frame::Init { session_id },
                None => Frame::Other,
            }
        }
        b"conversation_reset" => Frame::Reset,
        _ => Frame::Other,
    }
}

/// The `type` of a frame, read from where the CLI always writes it: first. `None` for a
/// line in any other shape.
fn frame_type(line: &[u8]) -> Option<&[u8]> {
    let rest = line.strip_prefix(br#"{"type":""#)?;
    let end = rest.iter().position(|&b| b == b'"')?;
    Some(&rest[..end])
}

/// The session a `system/init` names, unless it is a subagent's.
fn init_session(line: &[u8]) -> Option<String> {
    // A subagent's frames carry the `parent_tool_use_id` of the call that started it; an
    // init among them would describe that agent, not this session.
    if find(line, br#""parent_tool_use_id":""#).is_some() {
        return None;
    }
    valid_id(field_bytes(line, br#""session_id":""#)?)
}

/// The session a main-thread `hook_callback` ran in, when its transcript agrees.
///
/// The CLI builds every hook input from the live session: `session_id` first, then
/// `transcript_path`, the file that id names. So after a reset a hook is a second witness
/// to the new id — the fallback for a CLI that has not named it in a `system/init`, which
/// is how the CLIs seen so far announce it (see [`Recorder::observe`]). A subagent's
/// hook, which carries `agent_id`, is not taken: a background agent can run on after a
/// reset. Nor is an id whose transcript path does not end in `<id>.jsonl` (a hook served
/// for a remote caller carries `served:…` and an empty path).
fn hook_session(line: &[u8]) -> Option<String> {
    if find(line, br#""agent_id":""#).is_some() {
        return None;
    }
    let id = valid_id(field_bytes(line, br#""session_id":""#)?)?;
    let transcript = field_bytes(line, br#""transcript_path":""#)?;
    transcript
        .ends_with(format!("/{id}.jsonl").as_bytes())
        .then_some(id)
}

/// The `can_use_tool` requests still waiting for an answer, oldest first.
///
/// Kept in the order they arrived, which is what a reader needs: the newest prompt is the
/// last line. A map keyed by request id kept them in the order of their ids — random
/// UUIDs — so "the last line" was an arbitrary prompt, and an answered one could hide the
/// one still open.
#[derive(Default)]
struct Pending(Vec<(String, String)>);

impl Pending {
    /// More than a session could plausibly have open at once. A stream that opens prompts
    /// and never settles them must not grow us without bound: the oldest go first.
    const LIMIT: usize = 64;

    /// Record a prompt; whether anything changed. The same request again keeps its place
    /// — it arrived when it first did — and takes the newer text.
    fn open(&mut self, request_id: String, line: String) -> bool {
        if let Some(entry) = self.0.iter_mut().find(|(id, _)| *id == request_id) {
            if entry.1 == line {
                return false;
            }
            entry.1 = line;
            return true;
        }
        self.0.push((request_id, line));
        if self.0.len() > Self::LIMIT {
            self.0.remove(0);
        }
        true
    }

    /// Forget a prompt, answered or withdrawn; whether it was there.
    fn close(&mut self, request_id: &str) -> bool {
        let before = self.0.len();
        self.0.retain(|(id, _)| id != request_id);
        self.0.len() != before
    }

    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The file's contents: one request per line, exactly as the CLI wrote it.
    fn body(&self) -> String {
        let mut out = String::new();
        for (_, line) in &self.0 {
            out.push_str(line);
            out.push('\n');
        }
        out
    }
}

/// What the relay learns from the child's stdout, and the files it keeps from it.
///
/// Owned by the relay thread and fed a copy of each chunk after it has gone downstream.
/// Line-oriented and bounded: a partial line is carried to the next read rather than
/// parsed, and a line longer than anything we look for is skipped rather than buffered.
struct Recorder {
    dir: PathBuf,
    current: Current,
    carry: Vec<u8>,
    /// The line being carried outgrew [`LINE_LIMIT`]: drop bytes up to its newline.
    skipping: bool,
    pending: Pending,
    /// When `.pending` was last written or removed.
    written: Option<Instant>,
    /// A `conversation_reset` went by, and the new session has not been named yet.
    reset: bool,
}

impl Recorder {
    fn new(dir: &Path, current: Current) -> Recorder {
        Recorder {
            dir: dir.to_path_buf(),
            current,
            carry: Vec::new(),
            skipping: false,
            pending: Pending::default(),
            written: None,
            reset: false,
        }
    }

    /// Take in one chunk of the child's stdout.
    ///
    /// Control lines go to `<session>.ctl`; `can_use_tool` requests stay in `.pending`
    /// until an answer or a withdrawal for the same request id goes by.
    ///
    /// And the name follows the conversation. The CLI can carry on under a new session
    /// id in the same process: `/clear` (and its aliases), leaving plan mode with a
    /// cleared context, a fresh session to implement a plan. The new transcript and the
    /// CLI's own registry record take the new id; a socket left under the old one would
    /// show the old conversation as the one to type into, and the new one as unwrapped.
    /// Two frames name the session:
    ///
    ///   * a `system/init` whose `session_id` differs — the CLI saying outright which
    ///     session it is. This is what moves the files after `/clear`. The CLI (2.1.278,
    ///     .280 and .284, watched doing it with no hooks registered) writes the
    ///     `conversation_reset` still stamped with the old id, a few milliseconds later a
    ///     `system/init` naming the new one, and from then on every frame carries the new
    ///     id; it names itself in a `system/init` again at the start of every turn;
    ///   * after a `conversation_reset`, the next hook the main thread runs whose
    ///     `session_id` differs — only a fallback, should a CLI not re-announce itself.
    ///
    /// Not the reset frame's own `new_conversation_id`: the CLI (2.1.278, .280, .282, .283
    /// and .284, read from their bundles) mints that uuid with one `randomUUID()` and the
    /// new session id with another, so no transcript is named by it. Nor the `session_id`
    /// the reset frame is stamped with, which is still the old one.
    fn observe(&mut self, chunk: &[u8]) {
        let complete = self.complete_lines(chunk);
        if complete.is_empty() {
            return;
        }
        let Some(id) = lock(&self.current).clone() else {
            return;
        };

        let mut hits = String::new();
        let mut changed = false;
        let mut successor: Option<String> = None;
        for line in complete.split(|&b| b == b'\n') {
            if line.is_empty() {
                continue;
            }
            match classify(line) {
                Frame::Request { request_id, prompt, hook } => {
                    let Ok(text) = std::str::from_utf8(line) else {
                        continue;
                    };
                    hits.push_str(text);
                    hits.push('\n');
                    // Track what is still waiting for an answer. The .ctl tail can lose a
                    // request under a burst of MCP traffic, and a request nobody can find
                    // is a session nobody can unblock.
                    if let (true, Some(request_id)) = (prompt, request_id) {
                        changed |= self.pending.open(request_id, text.to_string());
                    }
                    // Hooks run on nearly every file tool call; which session one names
                    // only matters in the window after a reset.
                    if hook && self.reset {
                        if let Some(next) = hook_session(line).filter(|next| *next != id) {
                            successor = Some(next);
                        }
                    }
                }
                Frame::Response { request_id } => {
                    let Ok(text) = std::str::from_utf8(line) else {
                        continue;
                    };
                    hits.push_str(text);
                    hits.push('\n');
                    if let Some(request_id) = request_id {
                        changed |= self.pending.close(&request_id);
                    }
                }
                Frame::Cancel { request_id } => {
                    if let Some(request_id) = request_id {
                        changed |= self.pending.close(&request_id);
                    }
                }
                Frame::Init { session_id } => {
                    if session_id != id {
                        successor = Some(session_id);
                    }
                }
                Frame::Reset => self.reset = true,
                Frame::Other => {}
            }
        }

        let id = match successor {
            Some(next) => {
                let now = self.follow(&id, &next);
                // The conversation after the reset has its name. Until a move goes
                // through, the next hook naming it tries again.
                if now == next {
                    self.reset = false;
                }
                now
            }
            None => id,
        };
        if changed || self.heal_due() {
            self.write_pending(&id);
        }
        if !hits.is_empty() {
            append_ctl(&self.dir, &id, &hits);
        }
    }

    /// The bytes of the lines this chunk completes, each with its newline; empty if it
    /// completes none. A partial line waits in `carry` for the rest of it.
    fn complete_lines(&mut self, chunk: &[u8]) -> Vec<u8> {
        let mut rest = chunk;
        if self.skipping {
            let Some(end) = rest.iter().position(|&b| b == b'\n') else {
                return Vec::new();
            };
            self.skipping = false;
            rest = &rest[end + 1..];
        }
        let Some(cut) = rest.iter().rposition(|&b| b == b'\n') else {
            if self.carry.len() + rest.len() > LINE_LIMIT {
                // Longer than anything we look for. Drop it whole — skip to its end —
                // rather than buffer it, or read its tail as if it were a line.
                self.carry = Vec::new();
                self.skipping = true;
            } else {
                self.carry.extend_from_slice(rest);
            }
            return Vec::new();
        };
        let mut complete = std::mem::take(&mut self.carry);
        complete.extend_from_slice(&rest[..=cut]);
        let tail = &rest[cut + 1..];
        if tail.len() > LINE_LIMIT {
            self.skipping = true;
        } else {
            self.carry.extend_from_slice(tail);
        }
        complete
    }

    /// Move the socket and records to the id the CLI now names, and return the id in
    /// force afterwards: `new`, or `old` if the move did not go through cleanly.
    ///
    /// Under the name lock, which `serve_once` also holds to bind and to check its file,
    /// so it never sees the old name gone before the new one is in place. The listener is
    /// bound to the socket file, not to its name: a renamed socket keeps accepting.
    fn follow(&self, old: &str, new: &str) -> String {
        let mut slot = lock(&self.current);
        if move_records(&self.dir, old, new).is_ok() {
            *slot = Some(new.to_string());
            return new.to_string();
        }
        old.to_string()
    }

    /// Whether `.pending` is due a rewrite although nothing in it changed. The router
    /// deletes a session's `.pending` when its socket refuses a connection; while a prompt
    /// is open and the child is talking, a live session puts the file back within a
    /// second instead of never.
    fn heal_due(&self) -> bool {
        !self.pending.is_empty() && self.written.is_none_or(|at| at.elapsed() >= HEAL_EVERY)
    }

    /// Unanswered tool-permission requests, so the router can find what a session is
    /// blocked on without racing the .ctl tail.
    ///
    /// Replaced whole, by rename: a reader opens the old list or the new one, never the
    /// empty file a truncate-then-write shows for a moment. It was rewritten that way on
    /// every chunk, and a reader polling while other output flowed saw no prompt in 0.02 %
    /// to 1.5 % of its reads — each one a prompt that looked answered.
    fn write_pending(&mut self, id: &str) {
        let path = record_path(&self.dir, id, "pending");
        if self.pending.is_empty() {
            let _ = std::fs::remove_file(&path);
        } else {
            let _ = std::fs::create_dir_all(&self.dir);
            let _ = write_atomic(&path, self.pending.body().as_bytes());
        }
        self.written = Some(Instant::now());
    }
}

/// Append control lines to `<session>.ctl`, keeping only the tail: what is pending now is
/// what matters, and a long session must not grow the file.
fn append_ctl(dir: &Path, id: &str, hits: &str) {
    let path = record_path(dir, id, "ctl");
    let _ = std::fs::create_dir_all(dir);
    let mut existing = std::fs::read_to_string(&path).unwrap_or_default();
    existing.push_str(hits);
    let kept: Vec<&str> = existing.lines().rev().take(40).collect();
    let out: String = kept.into_iter().rev().collect::<Vec<_>>().join("\n") + "\n";
    let _ = std::fs::write(&path, out);
}

/// Replace `path` whole: write a sibling `.tmp`, then rename it over the original.
fn write_atomic(path: &Path, body: &[u8]) -> std::io::Result<()> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, body)
        .and_then(|()| std::fs::rename(&tmp, path))
        .inspect_err(|_| {
            let _ = std::fs::remove_file(&tmp);
        })
}

/// Rename `<old>.sock`, `.ctl` and `.pending` to `<new>.*` — all of them, or none.
///
/// A file that does not exist yet has nothing to move: the socket may not be bound yet,
/// no prompt may be open. A socket already under the new name is refused rather than
/// replaced: it belongs to some other process, and taking its name would bring that
/// session's messages here. Any other failure puts back what had moved.
fn move_records(dir: &Path, old: &str, new: &str) -> std::io::Result<()> {
    if new == old || valid_id(new.as_bytes()).is_none() {
        return Err(std::io::Error::new(ErrorKind::InvalidInput, "not a session id to move to"));
    }
    if record_path(dir, new, "sock").symlink_metadata().is_ok() {
        return Err(std::io::Error::new(ErrorKind::AlreadyExists, "that name is taken"));
    }
    let mut moved: Vec<(PathBuf, PathBuf)> = Vec::new();
    for ext in ["sock", "ctl", "pending"] {
        let from = record_path(dir, old, ext);
        let to = record_path(dir, new, ext);
        match std::fs::rename(&from, &to) {
            Ok(()) => moved.push((from, to)),
            Err(e) if e.kind() == ErrorKind::NotFound => {}
            Err(e) => {
                for (from, to) in moved.into_iter().rev() {
                    let _ = std::fs::rename(&to, &from);
                }
                return Err(e);
            }
        }
    }
    Ok(())
}

/// Everything a session leaves in the directory, removed on the way out.
fn remove_records(dir: &Path, id: &str) {
    for ext in ["sock", "ctl", "pending", "pending.tmp"] {
        let _ = std::fs::remove_file(record_path(dir, id, ext));
    }
}

/// What a predecessor under the same session id may have left: a wrapper killed with no
/// chance to clean up (a crash, a force quit) leaves its open prompts on file, and a
/// session resumed under that id would be served them as its own, dead requests the CLI
/// no longer waits on. Removed as the id is learned, before this process answers under
/// it. Not the socket, which `serve_once` replaces as it binds; nor the `.ctl`, which the
/// router only reads for the answer to a request it has just sent, found by that
/// request's id.
fn remove_leftovers(dir: &Path, id: &str) {
    for ext in ["pending", "pending.tmp"] {
        let _ = std::fs::remove_file(record_path(dir, id, ext));
    }
}

/// `<dir>/<session>.<ext>`: how the router finds each of a session's files.
fn record_path(dir: &Path, session_id: &str, ext: &str) -> PathBuf {
    dir.join(format!("{session_id}.{ext}"))
}

/// The value of a `"key":"…"` pair, without parsing the whole line.
fn field(line: &str, key: &str) -> Option<String> {
    field_bytes(line.as_bytes(), key.as_bytes())
        .and_then(|v| std::str::from_utf8(v).ok())
        .map(str::to_string)
}

/// [`field`] on raw bytes: the value up to the next quote, from the first occurrence of
/// the key.
fn field_bytes<'a>(line: &'a [u8], key: &[u8]) -> Option<&'a [u8]> {
    let start = find(line, key)? + key.len();
    let rest = &line[start..];
    let end = rest.iter().position(|&b| b == b'"')?;
    Some(&rest[..end])
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// A session id we are willing to put in a file name. It becomes a path under the
/// control directory, so nothing that could climb out of it.
fn valid_id(raw: &[u8]) -> Option<String> {
    let id = std::str::from_utf8(raw).ok()?;
    let ok = !id.is_empty()
        && id.len() <= 64
        && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    ok.then(|| id.to_string())
}

/// Pull `"session_id":"…"` out of the stream without parsing it.
///
/// A real JSON parse would mean buffering whole messages and having an opinion about
/// their shape, which is exactly the coupling this shim must not have.
fn sniff_session_id(buf: &[u8]) -> Option<String> {
    valid_id(field_bytes(buf, br#""session_id":""#)?)
}

/// Accept injected user turns and write them into the child's stdin.
///
/// Every failure here is swallowed. This is an addition to a session that works fine
/// without it, and it must never be the reason one dies.
fn serve_control(dir: &Path, current: &Current, sink: &Sink, up: &Up) {
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    // Re-bind if the socket file ever goes away. It lives in /tmp, so a cleaner, a
    // reboot-adjacent sweep or a stray rm can unlink it while this process still holds
    // the fd — leaving a session that looks unwrapped forever despite being wrapped.
    loop {
        serve_once(dir, current, sink, up);
        std::thread::sleep(Duration::from_secs(3));
    }
}

fn serve_once(dir: &Path, current: &Current, sink: &Sink, up: &Up) {
    // Bound under the name lock, so a move cannot slip in between reading the name and
    // binding it.
    let listener = {
        let slot = lock(current);
        let Some(id) = slot.as_deref() else {
            return;
        };
        let path = record_path(dir, id, "sock");
        let _ = std::fs::remove_file(&path);
        let Ok(listener) = UnixListener::bind(&path) else {
            return;
        };
        // Owner-only: this socket types into someone's editor.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
        listener
    };
    // Non-blocking so the accept loop can notice the file vanishing and rebuild it.
    if listener.set_nonblocking(true).is_err() {
        return;
    }

    loop {
        match listener.accept() {
            Err(ref e) if e.kind() == ErrorKind::WouldBlock => {
                if !socket_present(dir, current) {
                    return; // rebind on the next pass
                }
                std::thread::sleep(Duration::from_millis(200));
                continue;
            }
            Err(_) => return,
            Ok((conn, _)) => {
                let _ = conn.set_nonblocking(false);
                let sink = Arc::clone(sink);
                let up = Arc::clone(up);
                // A thread that cannot be had costs this one connection — dropped, so it
                // closes — never the listener, as a panic here would.
                let _ = std::thread::Builder::new()
                    .spawn(move || serve_connection(conn, &sink, &up));
            }
        }
    }
}

/// One control connection: a frame in, delivered whole or not at all, and `{"ok":…}` back.
fn serve_connection(mut conn: UnixStream, sink: &Shared<ChildStdin>, up: &Shared<std::io::Stdout>) {
    let frame = read_frame(&mut conn);
    let delivered = frame.as_deref().is_some_and(|frame| write_line(sink, frame));
    let _ = conn.write_all(if delivered { b"{\"ok\":true}\n" } else { b"{\"ok\":false}\n" });
    let _ = conn.shutdown(Shutdown::Both);

    // The host drew its own prompt when it saw the request and waits on its own UI; it
    // cannot learn we answered. A cancel echoing the same id is how a request is
    // withdrawn, so the picker comes down instead of hanging there answered-but-open.
    // Upstream too a line only goes in where one ends: one that finds no line end within
    // LINE_WAIT is dropped. A picker left up is a nuisance; a frame spliced into the
    // middle of the CLI's is garbage to the editor.
    let answered = frame.as_deref().filter(|_| delivered).and_then(answered_request_id);
    if let Some(id) = answered {
        let cancel = format!("{{\"type\":\"control_cancel_request\",\"request_id\":\"{id}\"}}\n");
        up.insert(cancel.as_bytes(), LINE_WAIT);
    }
}

/// One frame off a control connection: the bytes up to and including its first newline,
/// or all of them when the writer hangs up without one. Nothing after that first newline.
///
/// `None` when the frame cannot be had whole — longer than [`FRAME_LIMIT`], or cut off by
/// a read error — so that nothing at all is written: a truncated frame is a line the CLI
/// cannot parse, and it exits on one. Reading stops at the limit, so a stuck or runaway
/// writer cannot grow us.
fn read_frame(conn: &mut impl Read) -> Option<Vec<u8>> {
    let mut frame = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = match conn.read(&mut buf) {
            Ok(0) => return (frame.len() <= FRAME_LIMIT).then_some(frame),
            Ok(n) => n,
            Err(e) if e.kind() == ErrorKind::Interrupted => continue,
            Err(_) => return None,
        };
        let seen = frame.len();
        frame.extend_from_slice(&buf[..n]);
        if let Some(at) = frame[seen..].iter().position(|&b| b == b'\n') {
            let end = seen + at;
            if end > FRAME_LIMIT {
                return None;
            }
            frame.truncate(end + 1);
            return Some(frame);
        }
        if frame.len() > FRAME_LIMIT {
            return None;
        }
    }
}

/// Whether the socket file for the name in force is still there. Checked under the name
/// lock, which a move holds throughout: "the old name is gone and the new one is not in
/// place yet" is never observed, so a move is never taken for the file vanishing.
fn socket_present(dir: &Path, current: &Current) -> bool {
    let slot = lock(current);
    slot.as_deref()
        .is_some_and(|id| record_path(dir, id, "sock").exists())
}

/// The request id of an injected `control_response`, if that is what this was.
fn answered_request_id(body: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(body).ok()?;
    if !text.contains(r#""type":"control_response""#) {
        return None;
    }
    field(text, "\"request_id\":\"")
}

/// Write one message into the child's stdin as a line of its own, whole or not at all.
///
/// False once the editor has closed the session — nothing is typed into a child that is
/// on its way out — and false, with nothing written, while the editor stays in the middle
/// of a frame for longer than [`LINE_WAIT`].
fn write_line(sink: &Shared<ChildStdin>, body: &[u8]) -> bool {
    let trimmed = body.strip_suffix(b"\n").unwrap_or(body);
    if trimmed.is_empty() {
        return false;
    }
    let mut line = trimmed.to_vec();
    line.push(b'\n');
    sink.insert(&line, LINE_WAIT)
}

/// SIGTERM and SIGHUP, passed on to the CLI instead of ending this process first.
///
/// The editor closes a session with "end stdin, wait 2 s, SIGTERM" — sent to the process
/// it spawned, this one, and to no other. An idle CLI is long gone by then (the EOF
/// reaches it, see [`pump_stdin`]); a busy one is not: it finishes the turn, waits on a
/// background task, or settles an open prompt first. A wrapper that died of that SIGTERM
/// ran no cleanup, so its socket and records stayed behind, and it left the CLI running
/// on with nobody reading it, where unwrapped the same SIGTERM would have stopped it. So
/// the signal goes on to the CLI, and once the CLI is gone the wrapper leaves the way it
/// always does — the relay drains, the files are removed — and ends as the CLI did: with
/// its exit code (143, for a CLI that handles SIGTERM), or of the signal passed on, if
/// that is what the CLI died of. The extension's SIGKILL comes 5 s after its SIGTERM; a
/// CLI takes milliseconds to stop.
///
/// std only, like the rest: a handler that does nothing but write the signal's number
/// into a socket pair, and a thread that reads it and passes it on. If any of that cannot
/// be set up the default stays in place, which is what there was before.
mod signals {
    use std::io::{ErrorKind, Read};
    use std::os::raw::c_int;
    use std::os::unix::io::IntoRawFd;
    use std::os::unix::net::UnixStream;
    use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering};

    // The same numbers on every platform the wrapper ships for, macOS and Linux.
    const SIGHUP: c_int = 1;
    const SIGTERM: c_int = 15;
    /// `SIG_DFL` and `SIG_ERR`, in the form `signal` takes and returns them.
    const SIG_DFL: usize = 0;
    const SIG_ERR: usize = usize::MAX;

    extern "C" {
        /// BSD semantics on macOS, glibc and musl alike: the handler stays installed, and
        /// a system call it interrupts is restarted.
        fn signal(sig: c_int, handler: usize) -> usize;
        fn kill(pid: i32, sig: c_int) -> c_int;
        fn raise(sig: c_int) -> c_int;
        fn write(fd: c_int, buf: *const u8, len: usize) -> isize;
        #[cfg_attr(target_os = "linux", link_name = "__errno_location")]
        #[cfg_attr(not(target_os = "linux"), link_name = "__error")]
        fn errno_location() -> *mut c_int;
    }

    /// The end of the socket pair the handler writes into; -1 until there is one.
    static WAKE: AtomicI32 = AtomicI32::new(-1);
    /// Bit `n` set: signal `n` has been passed on to the child.
    static PASSED_ON: AtomicU32 = AtomicU32::new(0);
    /// The child has been waited for, so its pid may name some other process now.
    static REAPED: AtomicBool = AtomicBool::new(false);

    /// Only what a signal handler may do: an atomic load and one write(2) — into a
    /// non-blocking socket, so it never waits — with errno left as the code it
    /// interrupted had it.
    extern "C" fn on_signal(sig: c_int) {
        let fd = WAKE.load(Ordering::SeqCst);
        if fd < 0 {
            return;
        }
        // SAFETY: errno_location is this thread's own errno, and the byte outlives the
        // write.
        unsafe {
            let errno = errno_location();
            let saved = *errno;
            let byte = sig as u8;
            write(fd, &byte, 1);
            *errno = saved;
        }
    }

    /// From now on, pass SIGTERM and SIGHUP on to `child`.
    pub fn forward_to(child: u32) {
        let Ok(pid) = i32::try_from(child) else {
            return;
        };
        let Ok((mut rx, tx)) = UnixStream::pair() else {
            return;
        };
        if tx.set_nonblocking(true).is_err() {
            return;
        }
        let forwarder = std::thread::Builder::new().spawn(move || {
            let mut byte = [0u8; 1];
            loop {
                match rx.read(&mut byte) {
                    Ok(1) => {}
                    Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                    _ => break,
                }
                // A signal read in the instant between the child's exit and `REAPED` can
                // still reach its pid; for that to hit another process, the system would
                // have to hand the pid out again within that instant.
                if !REAPED.load(Ordering::SeqCst) {
                    let sig = c_int::from(byte[0]);
                    if let Some(bit) = 1u32.checked_shl(u32::from(byte[0])) {
                        PASSED_ON.fetch_or(bit, Ordering::SeqCst);
                    }
                    // SAFETY: kill(2) on our own child.
                    unsafe { kill(pid, sig) };
                }
            }
            // Nobody to pass them on from here: the default back, rather than signals
            // swallowed.
            for sig in [SIGTERM, SIGHUP] {
                // SAFETY: restoring the default action.
                unsafe { signal(sig, SIG_DFL) };
            }
        });
        // Nobody to pass them on: keep the default, and a SIGTERM ends this process as it
        // always did.
        if forwarder.is_err() {
            return;
        }
        WAKE.store(tx.into_raw_fd(), Ordering::SeqCst);
        for sig in [SIGTERM, SIGHUP] {
            // SAFETY: the handler is async-signal-safe (see there).
            unsafe { signal(sig, on_signal as extern "C" fn(c_int) as usize) };
        }
    }

    /// The child has been waited for: its pid is no longer ours to signal.
    pub fn child_reaped() {
        REAPED.store(true, Ordering::SeqCst);
    }

    /// The child died of `sig`: if that is a signal this process passed on to it, die of
    /// the same one, now that the cleanup is done — the end the editor would have seen
    /// with no wrapper in between. Any other signal is the child's own business, and the
    /// caller exits as it always has.
    pub fn die_of_if_passed_on(sig: c_int) {
        let passed_on = PASSED_ON.load(Ordering::SeqCst);
        if !(1..32).contains(&sig) || passed_on & (1 << sig) == 0 {
            return;
        }
        // SAFETY: the default action back, then the signal raised on this thread.
        unsafe {
            if signal(sig, SIG_DFL) != SIG_ERR {
                raise(sig);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn finds_the_session_id_in_an_init_message() {
        let line = br#"{"type":"system","subtype":"init","session_id":"bdcd43d7-fbad-4309-9aa5-94fdc9927c96","tools":[]}"#;
        assert_eq!(
            sniff_session_id(line).as_deref(),
            Some("bdcd43d7-fbad-4309-9aa5-94fdc9927c96")
        );
    }

    #[test]
    fn finds_it_across_a_chunk_boundary_once_both_halves_arrive() {
        let mut buf = Vec::from(&b"{\"session_i"[..]);
        assert!(sniff_session_id(&buf).is_none(), "not yet decidable");
        buf.extend_from_slice(b"d\":\"abc-123\",\"x\":1}");
        assert_eq!(sniff_session_id(&buf).as_deref(), Some("abc-123"));
    }

    #[test]
    fn refuses_an_id_that_could_escape_the_socket_directory() {
        assert!(sniff_session_id(br#"{"session_id":"../../etc/passwd"}"#).is_none());
        assert!(sniff_session_id(br#"{"session_id":""}"#).is_none());
    }

    #[test]
    fn tolerates_a_stream_with_no_id_at_all() {
        assert!(sniff_session_id(b"just some bytes").is_none());
        assert!(sniff_session_id(b"").is_none());
    }

    #[test]
    fn only_a_control_response_triggers_a_cancel_upstream() {
        let resp = br#"{"type":"control_response","response":{"request_id":"r-9","subtype":"success"}}"#;
        assert_eq!(answered_request_id(resp), Some("r-9".into()));
        let user = br#"{"type":"user","message":{"role":"user","content":"hi"}}"#;
        assert_eq!(answered_request_id(user), None, "a typed message must not cancel anything");
    }

    #[test]
    fn field_reads_a_flat_json_value() {
        let line = r#"{"type":"control_request","request_id":"abc-1","request":{}}"#;
        assert_eq!(field(line, "\"request_id\":\""), Some("abc-1".into()));
        assert_eq!(field(line, "\"nope\":\""), None);
    }

    #[test]
    fn control_path_is_named_by_session() {
        let dir = Path::new(CONTROL_DIR);
        assert_eq!(
            record_path(dir, "abc-123", "sock"),
            PathBuf::from("/tmp/agent-router-sessions/abc-123.sock")
        );
        assert_eq!(
            record_path(dir, "abc-123", "pending"),
            PathBuf::from("/tmp/agent-router-sessions/abc-123.pending")
        );
    }

    #[test]
    fn the_test_directory_is_honoured_only_when_absolute() {
        assert_eq!(resolve_dir(None), PathBuf::from(CONTROL_DIR));
        assert_eq!(resolve_dir(Some("relative/dir".into())), PathBuf::from(CONTROL_DIR));
        assert_eq!(resolve_dir(Some("".into())), PathBuf::from(CONTROL_DIR));
        assert_eq!(resolve_dir(Some("/tmp/lab-1".into())), PathBuf::from("/tmp/lab-1"));
    }

    // Frames in the shapes CLI 2.1.284 writes them (key order as in the recordings).

    const A: &str = "aaaaaaaa-0000-4000-8000-000000000001";
    const B: &str = "bbbbbbbb-0000-4000-8000-000000000002";
    const C: &str = "cccccccc-0000-4000-8000-000000000003";
    const P: &str = "eeeeeeee-0000-4000-8000-00000000000e";

    fn prompt(request_id: &str) -> String {
        format!(
            r#"{{"type":"control_request","request_id":"{request_id}","request":{{"subtype":"can_use_tool","tool_name":"Bash","display_name":"Bash","input":{{"command":"make deploy"}},"tool_use_id":"toolu_{request_id}"}}}}"#
        )
    }

    fn answer(request_id: &str) -> String {
        format!(
            r#"{{"type":"control_response","response":{{"subtype":"success","request_id":"{request_id}","response":{{"behavior":"allow","updatedInput":{{}}}}}}}}"#
        )
    }

    fn cancel(request_id: &str) -> String {
        format!(r#"{{"type":"control_cancel_request","request_id":"{request_id}"}}"#)
    }

    fn init(session_id: &str) -> String {
        format!(
            r#"{{"type":"system","subtype":"init","cwd":"/work","session_id":"{session_id}","tools":["Bash"],"model":"m"}}"#
        )
    }

    fn reset(new_conversation_id: &str, stamped: &str) -> String {
        format!(
            r#"{{"type":"conversation_reset","new_conversation_id":"{new_conversation_id}","trigger":"clear","timestamp":"2026-09-29T10:00:00.000Z","uuid":"u-1","session_id":"{stamped}"}}"#
        )
    }

    /// A PreToolUse hook as the extension's hooks see it; `agent` makes it a subagent's.
    fn hook(session_id: &str, agent: bool) -> String {
        let agent = if agent {
            r#""agent_id":"a1b2c3","agent_type":"workflow-subagent","#
        } else {
            ""
        };
        format!(
            r#"{{"type":"control_request","request_id":"h-{session_id}","request":{{"subtype":"hook_callback","callback_id":"hook_0","input":{{"session_id":"{session_id}","transcript_path":"/Users/me/.claude/projects/-work/{session_id}.jsonl","cwd":"/work","permission_mode":"default",{agent}"hook_event_name":"PreToolUse","tool_name":"Read","tool_input":{{"file_path":"/work/x"}},"tool_use_id":"toolu_1"}},"tool_use_id":"toolu_1"}}}}"#
        )
    }

    #[test]
    fn classify_reads_each_frame_the_recorder_acts_on() {
        let req = |id: &str, prompt: bool, hook: bool| Frame::Request {
            request_id: Some(id.to_string()),
            prompt,
            hook,
        };
        assert_eq!(classify(prompt("r-1").as_bytes()), req("r-1", true, false));
        assert_eq!(
            classify(answer("r-1").as_bytes()),
            Frame::Response { request_id: Some("r-1".into()) },
            "the answer's request_id is the one nested in `response`"
        );
        assert_eq!(
            classify(cancel("r-1").as_bytes()),
            Frame::Cancel { request_id: Some("r-1".into()) }
        );
        assert_eq!(classify(init(B).as_bytes()), Frame::Init { session_id: B.into() });
        assert_eq!(classify(reset(P, A).as_bytes()), Frame::Reset);
        let hook_a = format!("h-{A}");
        assert_eq!(classify(hook(A, false).as_bytes()), req(&hook_a, false, true));
        assert_eq!(classify(hook(A, true).as_bytes()), req(&hook_a, false, true));
        let initialize = br#"{"type":"control_request","request_id":"i-1","request":{"subtype":"initialize","hooks":{}}}"#;
        assert_eq!(classify(initialize), req("i-1", false, false));
        for other in [
            &br#"{"type":"system","subtype":"status","status":"requesting","session_id":"x"}"#[..],
            br#"{"type":"stream_event","event":{"type":"content_block_delta"},"session_id":"x"}"#,
            br#"{"type":"assistant","message":{"content":[]},"session_id":"x"}"#,
            br#"{"type":"result","subtype":"success","session_id":"x"}"#,
            b"not json at all",
            b"",
        ] {
            assert_eq!(classify(other), Frame::Other, "{}", String::from_utf8_lossy(other));
        }
    }

    #[test]
    fn a_control_frame_in_an_unusual_key_order_is_still_recorded() {
        let line = br#"{"request_id":"r-2","type":"control_request","request":{"subtype":"can_use_tool","tool_name":"Bash"}}"#;
        assert_eq!(
            classify(line),
            Frame::Request { request_id: Some("r-2".into()), prompt: true, hook: false }
        );
    }

    #[test]
    fn text_that_merely_mentions_a_control_frame_is_not_one() {
        // A user message quoting a control frame: inside a JSON string the quotes are
        // escaped, so the bytes never read as the frame itself.
        let line = br#"{"type":"user","message":{"content":"{\"type\":\"control_request\",\"request_id\":\"x\"}"}}"#;
        assert_eq!(classify(line), Frame::Other);
    }

    #[test]
    fn a_hook_names_a_session_only_when_its_transcript_agrees() {
        let mismatched = hook(A, false).replace(&format!("{A}.jsonl"), &format!("{B}.jsonl"));
        assert_eq!(hook_session(mismatched.as_bytes()), None);
        let served = hook(A, false)
            .replace(&format!(r#""session_id":"{A}""#), r#""session_id":"served:remote-1""#)
            .replace(&format!("/Users/me/.claude/projects/-work/{A}.jsonl"), "");
        assert_eq!(hook_session(served.as_bytes()), None);
        assert_eq!(hook_session(hook(A, true).as_bytes()), None, "a subagent's hook");
        assert_eq!(hook_session(hook(A, false).as_bytes()).as_deref(), Some(A));
    }

    #[test]
    fn a_subagents_init_names_no_session() {
        let line = init(B).replace(r#""model":"m""#, r#""model":"m","parent_tool_use_id":"toolu_9""#);
        assert_eq!(classify(line.as_bytes()), Frame::Other);
    }

    #[test]
    fn pending_keeps_arrival_order_not_request_id_order() {
        let mut p = Pending::default();
        assert!(p.open("zz-older".into(), prompt("zz-older")));
        assert!(p.open("aa-newer".into(), prompt("aa-newer")));
        assert_eq!(p.body(), format!("{}\n{}\n", prompt("zz-older"), prompt("aa-newer")));
        assert!(p.close("zz-older"));
        assert!(!p.close("zz-older"), "already gone");
        assert_eq!(p.body(), format!("{}\n", prompt("aa-newer")));
    }

    #[test]
    fn the_same_request_again_keeps_its_place() {
        let mut p = Pending::default();
        p.open("r-1".into(), prompt("r-1"));
        p.open("r-2".into(), prompt("r-2"));
        assert!(!p.open("r-1".into(), prompt("r-1")), "an identical replay changes nothing");
        assert!(p.open("r-1".into(), prompt("r-1").replace("make deploy", "make test")));
        let ids: Vec<&str> = p.0.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(ids, ["r-1", "r-2"]);
    }

    #[test]
    fn pending_is_bounded_and_drops_the_oldest() {
        let mut p = Pending::default();
        for i in 0..Pending::LIMIT + 5 {
            p.open(format!("r-{i}"), prompt(&format!("r-{i}")));
        }
        assert_eq!(p.0.len(), Pending::LIMIT);
        assert_eq!(p.0[0].0, "r-5");
    }

    /// A directory of its own for one test, removed afterwards.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Scratch {
            static N: AtomicUsize = AtomicUsize::new(0);
            let dir = std::env::temp_dir().join(format!(
                "claude-wrapper-unit-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Scratch(dir)
        }

        fn file(&self, id: &str, ext: &str) -> PathBuf {
            record_path(&self.0, id, ext)
        }

        fn names(&self) -> Vec<String> {
            let mut names: Vec<String> = std::fs::read_dir(&self.0)
                .unwrap()
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            names
        }

        /// A recorder whose session is `id`, with a stand-in socket file under that name
        /// (the unit tests move names, not listeners; the lab moves a live socket).
        fn recorder(&self, id: &str) -> Recorder {
            std::fs::write(self.file(id, "sock"), b"").unwrap();
            Recorder::new(&self.0, Arc::new(Mutex::new(Some(id.to_string()))))
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn lines(frames: &[String]) -> Vec<u8> {
        frames.iter().flat_map(|f| format!("{f}\n").into_bytes()).collect()
    }

    fn pending_ids(path: &Path) -> Option<Vec<String>> {
        let body = std::fs::read_to_string(path).ok()?;
        Some(body.lines().map(|l| field(l, "\"request_id\":\"").unwrap()).collect())
    }

    fn current(r: &Recorder) -> Option<String> {
        lock(&r.current).clone()
    }

    #[test]
    fn prompts_are_written_in_arrival_order_and_a_cancel_clears_one() {
        let s = Scratch::new();
        let mut r = s.recorder(A);
        r.observe(&lines(&[prompt("zz-older"), prompt("aa-newer")]));
        assert_eq!(
            pending_ids(&s.file(A, "pending")).unwrap(),
            ["zz-older", "aa-newer"],
            "arrival order: a map by request id would put aa-newer first"
        );
        r.observe(&lines(&[cancel("zz-older")]));
        assert_eq!(pending_ids(&s.file(A, "pending")).unwrap(), ["aa-newer"]);
        r.observe(&lines(&[answer("aa-newer")]));
        assert!(!s.file(A, "pending").exists(), "nothing open, no file");
        assert!(!s.file(A, "pending.tmp").exists());
        // Requests and answers are kept in .ctl; the withdrawal is not a control line.
        let ctl = std::fs::read_to_string(s.file(A, "ctl")).unwrap();
        assert_eq!(ctl.lines().count(), 3, "{ctl}");
    }

    #[test]
    fn a_line_split_across_reads_is_taken_once_whole() {
        let s = Scratch::new();
        let mut r = s.recorder(A);
        let frame = format!("{}\n", prompt("r-split"));
        let (head, tail) = frame.as_bytes().split_at(40);
        r.observe(head);
        assert!(!s.file(A, "pending").exists(), "half a line is not a request yet");
        r.observe(tail);
        assert_eq!(pending_ids(&s.file(A, "pending")).unwrap(), ["r-split"]);
    }

    #[test]
    fn an_oversized_line_is_skipped_to_its_end_and_not_read_from_the_middle() {
        let s = Scratch::new();
        let mut r = s.recorder(A);
        // A huge frame whose tail, read on its own, would look like a prompt.
        let mut huge = br#"{"type":"user","message":{"content":""#.to_vec();
        huge.extend(std::iter::repeat_n(b'x', LINE_LIMIT + 10));
        let fake_tail = format!(r#""}}}},{}"#, prompt("r-tail"));
        huge.extend_from_slice(fake_tail.as_bytes());
        huge.push(b'\n');
        for part in huge.chunks(64 * 1024) {
            r.observe(part);
        }
        assert!(!s.file(A, "pending").exists(), "no request was read out of the tail");
        // And the stream is back in step at the next line.
        r.observe(&lines(&[prompt("r-after")]));
        assert_eq!(pending_ids(&s.file(A, "pending")).unwrap(), ["r-after"]);
    }

    #[test]
    fn an_init_naming_another_session_moves_socket_and_records() {
        let s = Scratch::new();
        let mut r = s.recorder(A);
        r.observe(&lines(&[prompt("r-1")]));
        assert_eq!(s.names(), [format!("{A}.ctl"), format!("{A}.pending"), format!("{A}.sock")]);
        r.observe(&lines(&[init(A)]));
        assert_eq!(current(&r).as_deref(), Some(A), "the same id again is no change");
        r.observe(&lines(&[init(B), prompt("r-2")]));
        assert_eq!(current(&r).as_deref(), Some(B));
        assert_eq!(s.names(), [format!("{B}.ctl"), format!("{B}.pending"), format!("{B}.sock")]);
        assert_eq!(pending_ids(&s.file(B, "pending")).unwrap(), ["r-1", "r-2"]);
    }

    /// The fallback, for a CLI that does not name the new session in a `system/init` after
    /// a reset (every one seen so far does; see the test after this one).
    #[test]
    fn without_an_init_a_reset_is_followed_to_the_session_the_next_main_thread_hook_names() {
        let s = Scratch::new();
        let mut r = s.recorder(A);
        // Without a reset, a hook naming another session moves nothing.
        r.observe(&lines(&[hook(C, false)]));
        assert_eq!(current(&r).as_deref(), Some(A));
        // The reset frame's own new_conversation_id is not the session id, and a
        // subagent's hook after the reset does not speak for the session either.
        r.observe(&lines(&[reset(P, A), hook(C, true), hook(A, false)]));
        assert_eq!(current(&r).as_deref(), Some(A));
        assert!(s.file(A, "sock").exists());
        assert!(!s.file(P, "sock").exists());
        // The main thread's next hook in the new conversation carries its id.
        r.observe(&lines(&[hook(B, false)]));
        assert_eq!(current(&r).as_deref(), Some(B));
        assert!(s.file(B, "sock").exists() && !s.file(A, "sock").exists());
        // One reset, one move: a later hook naming yet another session is not followed.
        r.observe(&lines(&[hook(C, false)]));
        assert_eq!(current(&r).as_deref(), Some(B));
    }

    /// The result the CLI writes for `/clear`, keys in its order: `type` comes late.
    fn clear_result(session_id: &str) -> String {
        format!(
            r#"{{"is_error":false,"duration_api_ms":0,"num_turns":0,"stop_reason":null,"session_id":"{session_id}","total_cost_usd":0,"subtype":"success","result":"","local_command":"clear","type":"result","duration_ms":8,"uuid":"u-2"}}"#
        )
    }

    #[test]
    fn after_clear_the_files_move_on_the_init_that_names_the_new_session() {
        // What CLI 2.1.278, .280 and .284 write for /clear with no hooks registered: the
        // reset, still stamped with the old id and carrying an unrelated uuid; a few ms
        // later a system/init naming the new session; the command's result under the new
        // id. Then, at the start of the next turn, the same init again.
        let s = Scratch::new();
        let mut r = s.recorder(A);
        r.observe(&lines(&[prompt("r-1")]));
        r.observe(&lines(&[reset(P, A)]));
        assert_eq!(current(&r).as_deref(), Some(A), "the reset itself names no session");
        assert_eq!(s.names(), [format!("{A}.ctl"), format!("{A}.pending"), format!("{A}.sock")]);

        r.observe(&lines(&[init(B)]));
        assert_eq!(current(&r).as_deref(), Some(B), "the init after it does");
        assert_eq!(s.names(), [format!("{B}.ctl"), format!("{B}.pending"), format!("{B}.sock")]);
        assert!(!r.reset, "the reset has its successor; no hook is waited for");

        r.observe(&lines(&[clear_result(B), init(B), prompt("r-2")]));
        assert_eq!(current(&r).as_deref(), Some(B));
        assert_eq!(pending_ids(&s.file(B, "pending")).unwrap(), ["r-1", "r-2"]);
        assert_eq!(s.names(), [format!("{B}.ctl"), format!("{B}.pending"), format!("{B}.sock")]);
    }

    #[test]
    fn a_predecessors_leftover_prompts_go_and_nothing_else_does() {
        let s = Scratch::new();
        for ext in ["sock", "ctl", "pending", "pending.tmp"] {
            std::fs::write(s.file(A, ext), b"a dead wrapper's\n").unwrap();
        }
        std::fs::write(s.file(B, "pending"), b"another session's\n").unwrap();
        remove_leftovers(&s.0, A);
        assert_eq!(s.names(), [format!("{A}.ctl"), format!("{A}.sock"), format!("{B}.pending")]);
        remove_leftovers(&s.0, A);
    }

    /// What has gone into a [`Shared`] stream so far.
    fn written(s: &Shared<Vec<u8>>) -> Vec<u8> {
        lock(&s.state).w.clone().unwrap_or_default()
    }

    #[test]
    fn a_line_goes_in_at_once_where_the_stream_is_between_lines() {
        let s = Shared::new(Some(Vec::new()));
        let first = b"{\"first\":1}\n";
        assert!(s.insert(first, Duration::ZERO), "nothing written yet is a line start");
        assert!(s.pass(b"{\"relayed\":2}\n"));
        assert!(s.insert(b"{\"added\":3}\n", Duration::ZERO));
        assert_eq!(written(&s), b"{\"first\":1}\n{\"relayed\":2}\n{\"added\":3}\n");
    }

    #[test]
    fn a_line_waits_for_the_relay_to_finish_the_line_it_is_in() {
        let s = Arc::new(Shared::new(Some(Vec::new())));
        // The first piece of a frame larger than one read.
        assert!(s.pass(b"{\"type\":\"user\",\"text\":\"a long pas"));
        let adder = {
            let s = Arc::clone(&s);
            std::thread::spawn(move || s.insert(b"{\"from\":\"phone\"}\n", Duration::from_secs(10)))
        };
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(written(&s), b"{\"type\":\"user\",\"text\":\"a long pas", "not into the middle");
        // More of the same frame, still not the end of it.
        assert!(s.pass(b"te\",\"more\":"));
        std::thread::sleep(Duration::from_millis(50));
        assert!(!adder.is_finished(), "still waiting");
        assert!(s.pass(b"1}\n"));
        assert!(adder.join().unwrap());
        assert_eq!(
            written(&s),
            b"{\"type\":\"user\",\"text\":\"a long paste\",\"more\":1}\n{\"from\":\"phone\"}\n"
        );
    }

    #[test]
    fn a_line_that_finds_no_line_end_in_time_is_refused_and_writes_nothing() {
        let s = Shared::new(Some(Vec::new()));
        assert!(s.pass(b"{\"half\":"));
        let started = Instant::now();
        assert!(!s.insert(b"{\"from\":\"phone\"}\n", Duration::from_millis(150)));
        assert!(started.elapsed() >= Duration::from_millis(150));
        assert!(s.pass(b"1}\n"));
        assert_eq!(written(&s), b"{\"half\":1}\n", "the frame is whole, and alone");
    }

    #[test]
    fn a_closed_stream_takes_nothing_and_lets_a_waiting_line_go_at_once() {
        let s = Arc::new(Shared::new(Some(Vec::new())));
        assert!(s.pass(b"{\"half\":"));
        let adder = {
            let s = Arc::clone(&s);
            std::thread::spawn(move || {
                let started = Instant::now();
                (s.insert(b"{\"late\":1}\n", Duration::from_secs(10)), started.elapsed())
            })
        };
        std::thread::sleep(Duration::from_millis(50));
        s.close();
        let (taken, waited) = adder.join().unwrap();
        assert!(!taken);
        assert!(waited < Duration::from_secs(5), "released by the close, not at {waited:?}");
        assert!(!s.pass(b"1}\n"));
        assert!(!s.insert(b"{\"later\":1}\n", Duration::ZERO));
    }

    /// A writer that takes a few bytes, then fails, as a pipe does once its reader is gone.
    struct Breaks(usize);

    impl Write for Breaks {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            if self.0 == 0 {
                return Err(ErrorKind::BrokenPipe.into());
            }
            let n = buf.len().min(self.0);
            self.0 -= n;
            Ok(n)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn nothing_is_added_after_a_write_that_was_cut_short() {
        let s = Shared::new(Some(Breaks(4)));
        assert!(!s.pass(b"{\"cut\":\"short\"}\n"), "only part of it went");
        assert!(!s.insert(b"{\"from\":\"phone\"}\n", Duration::from_millis(20)));
    }

    /// A control connection that hands out one scripted piece per read, then EOF.
    struct Script(std::collections::VecDeque<std::io::Result<Vec<u8>>>);

    impl Script {
        fn new(pieces: Vec<std::io::Result<Vec<u8>>>) -> Script {
            Script(pieces.into())
        }
    }

    impl Read for Script {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            match self.0.pop_front() {
                None => Ok(0),
                Some(Err(e)) => Err(e),
                Some(Ok(mut piece)) => {
                    let n = piece.len().min(buf.len());
                    buf[..n].copy_from_slice(&piece[..n]);
                    if n < piece.len() {
                        self.0.push_front(Ok(piece.split_off(n)));
                    }
                    Ok(n)
                }
            }
        }
    }

    fn frame_of(pieces: Vec<std::io::Result<Vec<u8>>>) -> Option<Vec<u8>> {
        read_frame(&mut Script::new(pieces))
    }

    fn bytes(b: &[u8]) -> std::io::Result<Vec<u8>> {
        Ok(b.to_vec())
    }

    #[test]
    fn a_frame_is_read_up_to_its_first_newline_and_no_further() {
        assert_eq!(frame_of(vec![bytes(b"{\"a\":1}\n")]).unwrap(), b"{\"a\":1}\n");
        assert_eq!(
            frame_of(vec![bytes(b"{\"a\":"), bytes(b"1}\n{\"b\":2}\n")]).unwrap(),
            b"{\"a\":1}\n",
            "one frame per connection; the rest is not typed into anyone's session"
        );
        let interrupted = Err(std::io::Error::from(ErrorKind::Interrupted));
        let pieces = vec![bytes(b"{\"a\""), interrupted, bytes(b":1}\n")];
        assert_eq!(frame_of(pieces).unwrap(), b"{\"a\":1}\n");
    }

    #[test]
    fn a_frame_whose_writer_hangs_up_without_a_newline_is_taken_whole() {
        assert_eq!(frame_of(vec![bytes(b"{\"a\":"), bytes(b"1}")]).unwrap(), b"{\"a\":1}");
        assert_eq!(frame_of(vec![]).unwrap(), b"", "a bare connect: nothing, which types nothing");
        let at_limit = vec![b'x'; FRAME_LIMIT];
        assert_eq!(frame_of(vec![bytes(&at_limit)]).unwrap().len(), FRAME_LIMIT);
    }

    #[test]
    fn a_frame_cut_off_by_a_read_error_is_not_delivered_at_all() {
        let reset = Err(std::io::Error::from(ErrorKind::ConnectionReset));
        assert_eq!(frame_of(vec![bytes(b"{\"type\":\"user\",\"mess"), reset]), None);
    }

    #[test]
    fn a_frame_over_the_limit_is_refused_whole_and_reading_stops_there() {
        let mut at_limit = vec![b'x'; FRAME_LIMIT];
        at_limit.push(b'\n');
        let taken = frame_of(vec![bytes(&at_limit)]).unwrap();
        assert_eq!(taken.len(), FRAME_LIMIT + 1, "the limit itself is fine");

        let mut over = vec![b'x'; FRAME_LIMIT + 1];
        assert_eq!(frame_of(vec![bytes(&over)]), None, "over it, even with a clean hang-up");
        over.push(b'\n');
        assert_eq!(frame_of(vec![bytes(&over)]), None, "or with a newline right after");

        // No newline in sight: reading stops once there is more than a frame can be.
        let runaway = vec![b'x'; FRAME_LIMIT + 1];
        let mut conn = Script::new(vec![bytes(&runaway), bytes(b"never read\n")]);
        assert_eq!(read_frame(&mut conn), None);
        assert_eq!(conn.0.len(), 1, "what came after the limit was left unread");
    }

    #[test]
    fn a_move_that_fails_after_a_reset_is_tried_again_by_the_next_hook() {
        let s = Scratch::new();
        let mut r = s.recorder(A);
        std::fs::write(s.file(B, "sock"), b"in the way").unwrap();
        r.observe(&lines(&[reset(P, A), hook(B, false)]));
        assert_eq!(current(&r).as_deref(), Some(A), "the name was taken");
        std::fs::remove_file(s.file(B, "sock")).unwrap();
        r.observe(&lines(&[hook(B, false)]));
        assert_eq!(current(&r).as_deref(), Some(B), "still armed, and now it goes through");
    }

    #[test]
    fn a_move_refuses_a_name_that_is_already_taken_and_keeps_the_old_one() {
        let s = Scratch::new();
        let mut r = s.recorder(A);
        r.observe(&lines(&[prompt("r-1")]));
        std::fs::write(s.file(B, "sock"), b"someone else's").unwrap();
        r.observe(&lines(&[init(B)]));
        assert_eq!(current(&r).as_deref(), Some(A));
        assert!(s.file(A, "sock").exists() && s.file(A, "ctl").exists());
        assert_eq!(std::fs::read(s.file(B, "sock")).unwrap(), b"someone else's");
        // And the session carries on under its old name.
        r.observe(&lines(&[prompt("r-2")]));
        assert_eq!(pending_ids(&s.file(A, "pending")).unwrap(), ["r-1", "r-2"]);
    }

    #[test]
    fn a_move_is_all_or_nothing() {
        let s = Scratch::new();
        std::fs::write(s.file(A, "sock"), b"").unwrap();
        std::fs::write(s.file(A, "ctl"), b"ctl\n").unwrap();
        std::fs::write(s.file(A, "pending"), b"p\n").unwrap();
        // A directory where the new .pending would go: that rename fails.
        std::fs::create_dir(s.file(B, "pending")).unwrap();
        std::fs::write(s.file(B, "pending").join("x"), b"").unwrap();
        assert!(move_records(&s.0, A, B).is_err());
        assert!(s.file(A, "sock").exists(), "the socket was put back");
        assert_eq!(std::fs::read(s.file(A, "ctl")).unwrap(), b"ctl\n");
        assert_eq!(std::fs::read(s.file(A, "pending")).unwrap(), b"p\n");
        assert!(!s.file(B, "sock").exists() && !s.file(B, "ctl").exists());
    }

    #[test]
    fn a_move_with_nothing_there_yet_succeeds() {
        let s = Scratch::new();
        assert!(move_records(&s.0, A, B).is_ok());
        assert!(s.names().is_empty());
    }

    #[test]
    fn a_move_refuses_an_id_that_could_escape_the_directory() {
        let s = Scratch::new();
        std::fs::write(s.file(A, "sock"), b"").unwrap();
        for bad in ["../x", "", "a/b", A] {
            assert!(move_records(&s.0, A, bad).is_err(), "{bad:?}");
        }
        assert_eq!(s.names(), [format!("{A}.sock")]);
    }

    #[test]
    fn an_atomic_write_replaces_the_file_and_leaves_no_temporary() {
        let s = Scratch::new();
        let path = s.file(A, "pending");
        write_atomic(&path, b"one\n").unwrap();
        write_atomic(&path, b"two\n").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"two\n");
        assert_eq!(s.names(), [format!("{A}.pending")]);
    }

    #[test]
    fn an_open_prompt_is_written_again_once_its_file_is_gone() {
        let s = Scratch::new();
        let mut r = s.recorder(A);
        r.observe(&lines(&[prompt("r-1")]));
        std::fs::remove_file(s.file(A, "pending")).unwrap();
        r.written = Some(Instant::now() - HEAL_EVERY);
        r.observe(b"{\"type\":\"stream_event\",\"event\":{}}\n");
        assert_eq!(pending_ids(&s.file(A, "pending")).unwrap(), ["r-1"]);
    }

    #[test]
    fn nothing_is_recorded_before_the_session_is_known() {
        let s = Scratch::new();
        let mut r = Recorder::new(&s.0, Arc::new(Mutex::new(None)));
        r.observe(&lines(&[prompt("r-1")]));
        assert!(s.names().is_empty());
    }
}
