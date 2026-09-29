//! The lab: the real wrapper binary, end to end, around a fake child.
//!
//! Never a real `claude`, never a real session. The child is `/bin/sh` running the script
//! below: it announces a session id the way the CLI's `system/init` does, then echoes its
//! stdin to its stdout — and into a file — until EOF. So everything "the CLI says" in
//! these tests is something the test first wrote into the wrapper, and the wrapper cannot
//! tell the difference: it relays, sniffs, records and serves exactly as it does around
//! the real thing.
//!
//! The control directory is a fresh one under /tmp — short, because a unix socket path
//! is limited to 104 bytes on macOS — handed to the wrapper through its test-only
//! `AGENT_ROUTER_WRAPPER_TEST_DIR`. The directory the live router watches is never
//! touched.
//!
//! `CLAUDE_WRAPPER_LAB_BIN` runs the lab against another build of the wrapper, which has
//! to honour the same variable. That is how the baseline before these fixes was taken.
//! A build that does not — every build deployed before the variable existed — publishes
//! into `/tmp/agent-router-sessions`, the directory the live router watches: the lab's
//! fake sessions would land there, and stay there as dead sockets once the lab kills the
//! wrapper, until the router's janitor comes by. So such a build is refused before it is
//! ever started (see [`wrapper_bin`]).

use std::io::{Read, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::os::unix::net::UnixStream;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::{Duration, Instant};

/// Stand-in for `claude.real`.
const FAKE_CLAUDE: &str = r##"#!/bin/sh
# Stand-in for claude.real in the wrapper's lab. Never the real thing.
#   $1  the session id to announce, as the CLI's system/init does
#   $2  a file that gets a copy of everything read from stdin; the child's pid goes in $2.pid
#   $3  seconds to stay up after stdin closes, as a CLI finishing a turn does
#   $4  optional: one more line, written together with the init in the same write
printf '%s' "$$" > "$2.pid"
printf '%s\n' "{\"type\":\"system\",\"subtype\":\"init\",\"cwd\":\"/lab\",\"session_id\":\"$1\",\"tools\":[]}" ${4:+"$4"}
tee "$2"
printf '%s\n' '{"type":"lab","event":"stdin-closed"}'
exec sleep "${3:-0}"
"##;

/// Stand-in for `claude.real` that stops in the middle of a line of its own, the way a
/// frame larger than one read reaches the wrapper.
const FAKE_MIDLINE: &str = r##"#!/bin/sh
#   $1  the session id to announce
#   $2  a file that gets a copy of everything read from stdin
#   $3  seconds to stay in the middle of the line
#   $4  the text the line carries, written half before the pause and half after
printf '%s\n' "{\"type\":\"system\",\"subtype\":\"init\",\"cwd\":\"/lab\",\"session_id\":\"$1\",\"tools\":[]}"
printf '%s' "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"$4"
sleep "$3"
printf '%s\n' "$4\"}]}}"
exec tee "$2"
"##;

/// How the line [`FAKE_MIDLINE`] stops in ends.
const MIDLINE_END: &[u8] = b"\"}]}}\n";

/// Stand-in for `claude.real` that handles its signals as the CLI does: a SIGTERM ends it
/// with exit code 143. And this one shrugs a SIGHUP off.
const FAKE_TRAPPING: &str = r##"#!/bin/sh
#   $1  the session id to announce
#   $2  a file that gets a copy of everything read from stdin; the child's pid goes in $2.pid
#   $3  seconds to stay up after stdin closes
printf '%s' "$$" > "$2.pid"
trap '' HUP
trap 'kill "$busy" 2>/dev/null; exit 143' TERM
printf '%s\n' "{\"type\":\"system\",\"subtype\":\"init\",\"cwd\":\"/lab\",\"session_id\":\"$1\",\"tools\":[]}"
tee "$2"
printf '%s\n' '{"type":"lab","event":"stdin-closed"}'
sleep "${3:-0}" > /dev/null &
busy=$!
wait "$busy"
"##;

/// Stand-in for `claude.real` that exits once its stdin closes but leaves a background
/// command behind holding its stdout, as a CLI can with a background shell of its own.
const FAKE_BACKGROUNDED: &str = r##"#!/bin/sh
#   $1  the session id to announce
#   $2  a file that gets a copy of everything read from stdin; the background command's
#       pid goes in $2.bg
#   $3  seconds the background command keeps running
printf '%s\n' "{\"type\":\"system\",\"subtype\":\"init\",\"cwd\":\"/lab\",\"session_id\":\"$1\",\"tools\":[]}"
sleep "$3" &
printf '%s' "$!" > "$2.bg"
tee "$2"
printf '%s\n' '{"type":"lab","event":"stdin-closed"}'
"##;

/// What the fake child prints once its stdin has closed.
const EOF_MARKER: &str = r#"{"type":"lab","event":"stdin-closed"}"#;

/// How long anything in the lab may take before it counts as never.
const PATIENCE: Duration = Duration::from_secs(10);

/// The editor's grace between ending a session's stdin and SIGTERM (extension.js,
/// `ProcessTransport.close`: `iG0=2000`; a SIGKILL follows 5 s after the SIGTERM). An idle
/// session is gone well before it; a busy one is ended by the SIGTERM, which the wrapper
/// passes on to the CLI before cleaning up.
const EDITOR_GRACE: Duration = Duration::from_secs(2);

const A: &str = "aaaaaaaa-1ab0-4000-8000-000000000001";
const B: &str = "bbbbbbbb-1ab0-4000-8000-000000000002";
const C: &str = "cccccccc-1ab0-4000-8000-000000000003";
const P: &str = "eeeeeeee-1ab0-4000-8000-00000000000e";

// Frames in the shapes CLI 2.1.284 writes them, key order as in real recordings.

fn init(session_id: &str) -> String {
    format!(
        r#"{{"type":"system","subtype":"init","cwd":"/lab","session_id":"{session_id}","tools":[]}}"#
    )
}

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

fn reset(new_conversation_id: &str, stamped: &str) -> String {
    format!(
        r#"{{"type":"conversation_reset","new_conversation_id":"{new_conversation_id}","trigger":"clear","timestamp":"2026-09-29T10:00:00.000Z","uuid":"u-1","session_id":"{stamped}"}}"#
    )
}

/// A PreToolUse hook as the extension's hooks receive it; `agent` makes it a subagent's.
fn hook(session_id: &str, agent: bool) -> String {
    let agent = if agent {
        r#""agent_id":"a1b2c3","agent_type":"workflow-subagent","#
    } else {
        ""
    };
    format!(
        r#"{{"type":"control_request","request_id":"h-{session_id}","request":{{"subtype":"hook_callback","callback_id":"hook_0","input":{{"session_id":"{session_id}","transcript_path":"/Users/me/.claude/projects/-lab/{session_id}.jsonl","cwd":"/lab","permission_mode":"default",{agent}"hook_event_name":"PreToolUse","tool_name":"Read","tool_input":{{"file_path":"/lab/x"}},"tool_use_id":"toolu_1"}},"tool_use_id":"toolu_1"}}}}"#
    )
}

/// A typed turn, as the router's `send_via_wrapper` writes it into the socket.
fn user_turn(text: &str) -> String {
    format!(
        r#"{{"type":"user","message":{{"role":"user","content":[{{"type":"text","text":"{text}"}}]}}}}"#
    )
}

/// The result the CLI writes for `/clear`, keys in its order: `type` comes late.
fn clear_result(session_id: &str) -> String {
    format!(
        r#"{{"is_error":false,"duration_api_ms":0,"num_turns":0,"stop_reason":null,"session_id":"{session_id}","total_cost_usd":0,"subtype":"success","result":"","local_command":"clear","type":"result","duration_ms":8,"uuid":"u-2"}}"#
    )
}

/// A frame of exactly `len` bytes, newline not counted, that the CLI would take and
/// ignore.
fn padded(len: usize) -> String {
    const HEAD: &str = r#"{"type":"keep_alive","pad":""#;
    const TAIL: &str = r#""}"#;
    format!("{HEAD}{}{TAIL}", "x".repeat(len - HEAD.len() - TAIL.len()))
}

/// The wrapper's limit on an injected frame, newline not counted (`FRAME_LIMIT` in
/// main.rs; the router refuses anything longer).
const FRAME_LIMIT: usize = 1024 * 1024;

#[derive(Default)]
struct Output {
    bytes: Vec<u8>,
    closed: bool,
}

/// One wrapper around one fake child, in a directory of its own. Dropping it kills
/// whatever is still running and removes the directory.
struct Lab {
    dir: PathBuf,
    wrapper: Child,
    stdin: Option<ChildStdin>,
    out: Arc<(Mutex<Output>, Condvar)>,
    /// Bytes the child owes us: its init line, then an echo of everything sent.
    owed: usize,
}

impl Lab {
    fn start(id: &str) -> Lab {
        Lab::start_lingering(id, 0.0)
    }

    /// As [`Lab::start`], with a child that stays up `linger_s` seconds after its stdin
    /// closes.
    fn start_lingering(id: &str, linger_s: f32) -> Lab {
        Lab::spawn(id, FAKE_CLAUDE, &[&format!("{linger_s}")], |_| {})
    }

    /// As [`Lab::start`], with `line` written by the child in the same write as its init,
    /// so that the wrapper reads both at once; `prepare` gets the directory before the
    /// wrapper starts.
    fn start_saying(id: &str, line: &str, prepare: impl FnOnce(&Path)) -> Lab {
        let mut lab = Lab::spawn(id, FAKE_CLAUDE, &["0", line], prepare);
        lab.owed += line.len() + 1;
        lab
    }

    /// The wrapper around `/bin/sh <script> <id> <stdin copy> <args…>`, in a directory of
    /// its own that `prepare` gets first; returned once the control socket is there.
    fn spawn(id: &str, script: &str, args: &[&str], prepare: impl FnOnce(&Path)) -> Lab {
        // First, so that a build the lab refuses leaves no directory behind.
        let bin = wrapper_bin();
        let dir = lab_dir();
        prepare(&dir);
        let path = dir.join("fake-claude.sh");
        std::fs::write(&path, script).unwrap();
        // `/bin/sh` is the "real binary" the wrapper is handed, as the editor hands it
        // claude.real; the script is only read, never executed from a fresh file.
        let mut wrapper = Command::new(bin)
            .arg("/bin/sh")
            .arg(&path)
            .arg(id)
            .arg(dir.join("child-stdin.bin"))
            .args(args)
            .env("AGENT_ROUTER_WRAPPER_TEST_DIR", &dir)
            .env_remove("AGENT_ROUTER_WRAPPER_DEBUG")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("start the wrapper");
        let stdin = wrapper.stdin.take();
        let mut stdout = wrapper.stdout.take().unwrap();
        let out = Arc::new((Mutex::new(Output::default()), Condvar::new()));
        {
            let out = Arc::clone(&out);
            std::thread::spawn(move || {
                let mut buf = [0u8; 64 * 1024];
                loop {
                    let n = stdout.read(&mut buf).unwrap_or(0);
                    let (m, cv) = &*out;
                    let mut o = lock(m);
                    if n == 0 {
                        o.closed = true;
                        cv.notify_all();
                        return;
                    }
                    o.bytes.extend_from_slice(&buf[..n]);
                    cv.notify_all();
                }
            });
        }
        let owed = init(id).len() + 1;
        let lab = Lab { dir, wrapper, stdin, out, owed };
        lab.wait_for("the control socket", || lab.file(id, "sock").exists());
        lab
    }

    fn file(&self, id: &str, ext: &str) -> PathBuf {
        self.dir.join(format!("{id}.{ext}"))
    }

    /// Bytes into the wrapper's stdin, as the editor writes them.
    fn send(&mut self, bytes: &[u8]) {
        let stdin = self.stdin.as_mut().expect("stdin is still open");
        stdin
            .write_all(bytes)
            .and_then(|()| stdin.flush())
            .expect("the wrapper takes its input");
        self.owed += bytes.len();
    }

    /// Wait until everything sent so far has come back out: the wrapper has relayed it,
    /// so the next thing sent arrives in a read of its own.
    fn wait_echo(&self) {
        let owed = self.owed;
        self.wait_output("the echo of everything sent", |out| out.len() >= owed);
    }

    /// One frame for the child to "say": it echoes whatever it is given.
    fn send_line(&mut self, frame: &str) {
        self.send(format!("{frame}\n").as_bytes());
    }

    /// End the session the way the editor does first: close stdin.
    fn close_stdin(&mut self) {
        drop(self.stdin.take());
    }

    /// Poll until `cond` holds; fail naming `what` if it never does.
    fn wait_for(&self, what: &str, mut cond: impl FnMut() -> bool) {
        let deadline = Instant::now() + PATIENCE;
        while !cond() {
            assert!(Instant::now() < deadline, "never happened: {what}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// Wait until the wrapper's output satisfies `pred`.
    fn wait_output(&self, what: &str, pred: impl Fn(&[u8]) -> bool) {
        let (m, cv) = &*self.out;
        let deadline = Instant::now() + PATIENCE;
        let mut o = lock(m);
        while !pred(&o.bytes) {
            let now = Instant::now();
            assert!(now < deadline && !o.closed, "never came out of the wrapper: {what}");
            o = cv.wait_timeout(o, deadline - now).unwrap_or_else(PoisonError::into_inner).0;
        }
    }

    fn wait_closed(&self) {
        let (m, cv) = &*self.out;
        let deadline = Instant::now() + PATIENCE;
        let mut o = lock(m);
        while !o.closed {
            let now = Instant::now();
            assert!(now < deadline, "the wrapper's stdout never closed");
            o = cv.wait_timeout(o, deadline - now).unwrap_or_else(PoisonError::into_inner).0;
        }
    }

    fn output(&self) -> Vec<u8> {
        lock(&self.out.0).bytes.clone()
    }

    /// The request ids in `<id>.pending`, in file order; `None` if there is no file.
    fn pending_ids(&self, id: &str) -> Option<Vec<String>> {
        let body = std::fs::read_to_string(self.file(id, "pending")).ok()?;
        Some(body.lines().map(request_id).collect())
    }

    fn wait_pending(&self, id: &str, expected: &[&str]) {
        let deadline = Instant::now() + PATIENCE;
        loop {
            let seen = self.pending_ids(id);
            if seen.as_ref().is_some_and(|ids| ids == expected) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "{id}.pending never read {expected:?}; last read {seen:?}"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// Write one frame into `<id>.sock` the way the router does; the wrapper's reply.
    fn inject(&self, id: &str, frame: &str) -> String {
        inject_at(&self.file(id, "sock"), frame)
    }

    /// Write `bytes` into `<id>.sock` while reading the reply, as a writer that the
    /// wrapper may stop reading from part way; the reply, and whether every byte went.
    fn inject_raw(&self, id: &str, bytes: &[u8]) -> (String, bool) {
        let sock = self.file(id, "sock");
        let mut s = UnixStream::connect(sock).expect("connect to the control socket");
        s.set_read_timeout(Some(PATIENCE)).unwrap();
        s.set_write_timeout(Some(PATIENCE)).unwrap();
        let mut writer = s.try_clone().unwrap();
        let bytes = bytes.to_vec();
        let wrote = std::thread::spawn(move || writer.write_all(&bytes).is_ok());
        let mut reply = String::new();
        let _ = s.read_to_string(&mut reply);
        (reply, wrote.join().unwrap())
    }

    /// The fake child's pid, as it wrote it down.
    fn child_pid(&self) -> u32 {
        let path = self.dir.join("child-stdin.bin.pid");
        std::fs::read_to_string(path).unwrap().trim().parse().unwrap()
    }

    /// Send the wrapper a signal, by name, the way the editor sends its SIGTERM.
    fn signal(&self, name: &str) {
        let pid = self.wrapper.id().to_string();
        assert!(sh_kill(&format!("-{name}"), &pid), "could not send SIG{name}");
    }

    /// A bare connect with nothing written: the router's liveness probe.
    fn probe(&self, id: &str) -> String {
        let mut s = UnixStream::connect(self.file(id, "sock")).expect("connect to the control socket");
        s.set_read_timeout(Some(PATIENCE)).unwrap();
        s.shutdown(std::net::Shutdown::Write).unwrap();
        let mut reply = String::new();
        let _ = s.read_to_string(&mut reply);
        reply
    }

    fn wait_exit(&mut self, limit: Duration) -> Option<ExitStatus> {
        let deadline = Instant::now() + limit;
        loop {
            if let Some(status) = self.wrapper.try_wait().unwrap() {
                return Some(status);
            }
            if Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    /// Which of the files a session leaves are still there, for each of `ids`.
    fn leftovers(&self, ids: &[&str]) -> Vec<String> {
        ids.iter()
            .flat_map(|id| ["sock", "ctl", "pending", "pending.tmp"].map(|ext| format!("{id}.{ext}")))
            .filter(|name| self.dir.join(name).symlink_metadata().is_ok())
            .collect()
    }

    /// Close stdin the way the editor ends a session, and see the wrapper out: it has to
    /// exit by itself, successfully, and leave nothing behind for any of `ids`. How long
    /// it took from the close.
    fn finish(&mut self, ids: &[&str]) -> Duration {
        self.close_stdin();
        let closed = Instant::now();
        let status = self.wait_exit(Duration::from_secs(5)).unwrap_or_else(|| {
            panic!("the wrapper is still running 5 s after its stdin closed: the child never saw EOF")
        });
        let took = closed.elapsed();
        assert!(status.success(), "the wrapper exited with {status}");
        self.wait_closed();
        let left = self.leftovers(ids);
        assert!(left.is_empty(), "left behind after a clean exit: {left:?}");
        took
    }
}

impl Drop for Lab {
    fn drop(&mut self) {
        let _ = self.wrapper.kill();
        let _ = self.wrapper.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// The output buffer's lock, still usable after a failed assertion panicked holding it,
/// so one failure is reported once rather than again from the reader thread.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The build under test: this one, or the one `CLAUDE_WRAPPER_LAB_BIN` names — and that
/// one only if it reads the lab's directory variable. A build that does not publishes the
/// lab's fake sessions where the live router takes them for real ones (see the module
/// doc), so it is refused here, before it is ever started.
fn wrapper_bin() -> PathBuf {
    static BIN: OnceLock<PathBuf> = OnceLock::new();
    BIN.get_or_init(|| match std::env::var_os("CLAUDE_WRAPPER_LAB_BIN").map(PathBuf::from) {
        Some(bin) => {
            assert!(
                honours_test_dir(&bin),
                "refusing to run {}: it never reads AGENT_ROUTER_WRAPPER_TEST_DIR, so it would \
                 publish the lab's fake sessions into /tmp/agent-router-sessions, where the live \
                 router takes them for real ones",
                bin.display()
            );
            bin
        }
        None => PathBuf::from(env!("CARGO_BIN_EXE_claude-wrapper")),
    })
    .clone()
}

/// Whether a wrapper build reads the lab's directory variable: every build that does has
/// its name among its bytes.
fn honours_test_dir(bin: &Path) -> bool {
    std::fs::read(bin).is_ok_and(|bytes| contains(&bytes, b"AGENT_ROUTER_WRAPPER_TEST_DIR"))
}

/// Write one frame into the socket at `sock` the way the router does; the wrapper's reply.
fn inject_at(sock: &Path, frame: &str) -> String {
    let mut s = UnixStream::connect(sock).expect("connect to the control socket");
    s.set_read_timeout(Some(PATIENCE)).unwrap();
    s.write_all(format!("{frame}\n").as_bytes()).unwrap();
    let mut reply = String::new();
    let _ = s.read_to_string(&mut reply);
    reply
}

/// `kill <flag> <pid>` by the shell's own `kill`, which every system has at /bin/sh.
fn sh_kill(flag: &str, pid: &str) -> bool {
    Command::new("/bin/sh")
        .args(["-c", r#"kill "$0" "$1" 2>/dev/null"#, flag, pid])
        .status()
        .is_ok_and(|s| s.success())
}

/// Whether a process is there to be signalled.
fn alive(pid: u32) -> bool {
    sh_kill("-0", &pid.to_string())
}

/// A fresh, short directory for one lab run.
fn lab_dir() -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let dir = PathBuf::from(format!(
        "/tmp/cwlab-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    assert_ne!(dir, Path::new("/tmp/agent-router-sessions"), "never the router's directory");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn request_id(line: &str) -> String {
    const KEY: &str = "\"request_id\":\"";
    let start = line.find(KEY).map(|i| i + KEY.len()).unwrap_or(line.len());
    let rest = &line[start..];
    rest[..rest.find('"').unwrap_or(rest.len())].to_string()
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

fn inode(p: &Path) -> u64 {
    std::fs::metadata(p).unwrap().ino()
}

fn mode(p: &Path) -> u32 {
    std::fs::metadata(p).unwrap().permissions().mode() & 0o777
}

/// Byte-for-byte equality, reported by where it first differs rather than by printing
/// megabytes.
fn assert_same(what: &str, got: &[u8], want: &[u8]) {
    if got == want {
        return;
    }
    let at = got.iter().zip(want).position(|(g, w)| g != w).unwrap_or(got.len().min(want.len()));
    let show = |b: &[u8]| String::from_utf8_lossy(&b[at.min(b.len())..(at + 60).min(b.len())]).into_owned();
    panic!(
        "{what}: {} bytes where {} were expected, first difference at byte {at}: got {:?}, expected {:?}",
        got.len(),
        want.len(),
        show(got),
        show(want)
    );
}

#[test]
fn bytes_relay_unchanged_both_ways_and_eof_reaches_the_child() {
    let mut lab = Lab::start(A);
    let mut sent: Vec<u8> = Vec::new();

    // Hundreds of small frames, as a streaming turn writes them.
    let mut small = Vec::new();
    for i in 0..300 {
        small.extend_from_slice(
            format!(
                r#"{{"type":"stream_event","event":{{"type":"content_block_delta","index":0,"delta":{{"type":"text_delta","text":"tok {i}"}}}},"session_id":"{A}"}}"#
            )
            .as_bytes(),
        );
        small.push(b'\n');
    }
    lab.send(&small);
    sent.extend_from_slice(&small);

    // One frame of 3 MiB: dozens of reads each way, and far past the recorder's 256 KiB
    // carry, which has to skip it without disturbing a byte of it.
    let big = format!(
        r#"{{"type":"user","message":{{"role":"user","content":[{{"type":"tool_result","tool_use_id":"t","content":"{}"}}]}},"isReplay":true}}"#,
        "x".repeat(3 << 20)
    ) + "\n";
    lab.send(big.as_bytes());
    sent.extend_from_slice(big.as_bytes());

    // A prompt in three pieces with pauses between, the way a frame larger than the
    // socketpair buffer arrives.
    let split = format!("{}\n", prompt("r-split"));
    let bytes = split.as_bytes();
    for part in [&bytes[..17], &bytes[17..90], &bytes[90..]] {
        lab.send(part);
        sent.extend_from_slice(part);
        std::thread::sleep(Duration::from_millis(50));
    }

    // Bytes that are neither UTF-8 nor JSON, a CRLF, an empty line, and a last line with
    // no newline at all.
    let odd = b"\xff\xfe\x00 not json \x80\r\n\n{\"type\":\"lab\",\"partial\":true";
    lab.send(odd);
    sent.extend_from_slice(odd);

    // The recorder took the split prompt whole, from the copy it is shown.
    lab.wait_pending(A, &["r-split"]);

    let took = lab.finish(&[A]);
    assert!(
        took < EDITOR_GRACE,
        "the wrapper took {took:?} to exit after its stdin closed; the editor SIGTERMs at {EDITOR_GRACE:?}"
    );

    let mut expected = format!("{}\n", init(A)).into_bytes();
    expected.extend_from_slice(&sent);
    expected.extend_from_slice(format!("{EOF_MARKER}\n").as_bytes());
    assert_same("what the editor received", &lab.output(), &expected);
    let copy = std::fs::read(lab.dir.join("child-stdin.bin")).unwrap();
    assert_same("what the child received", &copy, &sent);
    println!("lab: relayed {} bytes each way; exit {took:?} after stdin closed", sent.len());
}

#[test]
fn a_withdrawn_or_answered_prompt_leaves_pending_and_the_rest_keep_arrival_order() {
    let mut lab = Lab::start(A);
    lab.send_line(&prompt("zz-older"));
    lab.send_line(&prompt("aa-newer"));
    // Arrival order. A map keyed by request id put aa-newer first, so the "newest"
    // prompt a reader took from the last line was the older one.
    lab.wait_pending(A, &["zz-older", "aa-newer"]);

    // The CLI withdrew one.
    lab.send_line(&cancel("zz-older"));
    lab.wait_pending(A, &["aa-newer"]);

    // One answered in the editor, echoed by the CLI; the fence shows both went by.
    lab.send_line(&prompt("r-answered"));
    lab.send_line(&answer("r-answered"));
    lab.send_line(&prompt("fence"));
    lab.wait_pending(A, &["aa-newer", "fence"]);

    // Closed with prompts still open: the way out removes them with the rest.
    lab.finish(&[A]);
}

#[test]
fn pending_is_never_seen_missing_empty_or_torn_while_it_changes() {
    // Only the write itself is on trial here. The prompts that come and go are closed by
    // answers, which every build handled, and their ids sort in the order they arrive,
    // so neither the cancel fix nor the ordering fix is what makes this pass.
    let mut lab = Lab::start(A);
    let stable = prompt("a-stable");
    lab.send_line(&stable);
    lab.wait_pending(A, &["a-stable"]);

    // A reader in the router's place, polling the file as fast as it can while the set
    // around one prompt that stays open changes a thousand times.
    let path = lab.file(A, "pending");
    let stop = Arc::new(AtomicBool::new(false));
    let reader = {
        let stop = Arc::clone(&stop);
        let head = format!("{stable}\n");
        std::thread::spawn(move || {
            let (mut reads, mut bad, mut first_bad) = (0u64, 0u64, None);
            // Each replacement is a new file, so a change of inode between two reads is a
            // rewrite that happened while the reader was at it. Transitions, not distinct
            // values: a filesystem may hand a freed inode number straight back.
            let (mut last_inode, mut replaced) = (None, 0u64);
            while !stop.load(Ordering::Relaxed) {
                reads += 1;
                let verdict = match std::fs::File::open(&path) {
                    Ok(mut file) => {
                        if let Ok(meta) = file.metadata() {
                            if last_inode.is_some_and(|i| i != meta.ino()) {
                                replaced += 1;
                            }
                            last_inode = Some(meta.ino());
                        }
                        let mut body = String::new();
                        match file.read_to_string(&mut body) {
                            Ok(_)
                                if body.starts_with(&head)
                                    && body.ends_with('\n')
                                    && body.lines().all(|l| {
                                        l.starts_with(r#"{"type":"control_request""#)
                                            && l.ends_with('}')
                                    }) =>
                            {
                                None
                            }
                            Ok(_) => Some(format!(
                                "{} bytes: {:?}",
                                body.len(),
                                &body[..body.len().min(60)]
                            )),
                            Err(e) => Some(format!("{e}")),
                        }
                    }
                    Err(e) => Some(format!("{e}")),
                };
                if let Some(what) = verdict {
                    bad += 1;
                    first_bad.get_or_insert(what);
                }
            }
            (reads, bad, first_bad, replaced)
        })
    };
    // Paced: sent in a burst, these coalesce into a handful of 64 KiB reads and the file
    // is replaced a handful of times. One line per read replaces it for every line.
    for i in 0..500 {
        lab.send_line(&prompt(&format!("z-flap-{i:03}")));
        lab.wait_echo();
        lab.send_line(&answer(&format!("z-flap-{i:03}")));
        lab.wait_echo();
    }
    lab.send_line(&prompt("z-fence"));
    lab.wait_pending(A, &["a-stable", "z-fence"]);
    stop.store(true, Ordering::Relaxed);
    let (reads, bad, first_bad, replaced) = reader.join().unwrap();
    assert_eq!(
        bad, 0,
        "{bad} of {reads} reads saw no prompt, or part of a file, while one was open; first: {first_bad:?}"
    );
    assert!(reads >= 100, "the reader only managed {reads} reads");
    assert!(
        replaced >= 10,
        "the file was replaced only {replaced} times during {reads} reads: nothing was tested"
    );
    println!(
        "lab: {reads} reads of a .pending replaced {replaced} times meanwhile, none missing, empty or torn"
    );

    lab.finish(&[A]);
}

#[test]
fn an_init_naming_a_new_session_moves_everything_and_the_moved_socket_still_answers() {
    let mut lab = Lab::start(A);
    lab.send_line(&prompt("r-1"));
    lab.wait_pending(A, &["r-1"]);
    assert!(lab.file(A, "ctl").exists());
    assert_eq!(lab.probe(A), "{\"ok\":false}\n", "a bare connect is answered, and nothing typed");
    let socket = inode(&lab.file(A, "sock"));

    lab.send_line(&init(B));
    lab.wait_for("the socket to move to B", || {
        lab.file(B, "sock").exists() && !lab.file(A, "sock").exists()
    });
    assert_eq!(inode(&lab.file(B, "sock")), socket, "the same socket, renamed, not a new one");
    assert_eq!(mode(&lab.file(B, "sock")), 0o600, "still owner-only");
    assert!(lab.file(B, "ctl").exists() && !lab.file(A, "ctl").exists());
    lab.wait_pending(B, &["r-1"]);
    assert!(!lab.file(A, "pending").exists());

    // The moved socket still takes connections, and what it is given reaches the child.
    assert_eq!(lab.probe(B), "{\"ok\":false}\n");
    let turn = user_turn("typed on a phone after the move");
    assert_eq!(lab.inject(B, &turn), "{\"ok\":true}\n");
    lab.wait_output("the injected turn, echoed by the child", |out| {
        contains(out, turn.as_bytes())
    });
    // Nothing answers under the old name any more.
    assert!(UnixStream::connect(lab.file(A, "sock")).is_err());

    lab.finish(&[A, B]);
}

/// The fallback, should a CLI not name the new session in a `system/init` after a reset
/// (every one seen so far does: see the test after this one).
#[test]
fn without_an_init_after_a_reset_the_socket_follows_the_session_the_main_thread_names() {
    let mut lab = Lab::start(A);
    let socket = inode(&lab.file(A, "sock"));

    // None of these names the session this process is in:
    lab.send_line(&hook(C, false)); // another session's id, with no reset before it
    lab.send_line(&reset(P, A)); // the reset's own new_conversation_id is not a session id
    lab.send_line(&hook(C, true)); // a subagent's hook, after the reset
    // A fence: once this prompt is on file, every line before it has been read.
    lab.send_line(&prompt("fence-1"));
    lab.wait_pending(A, &["fence-1"]);
    assert!(lab.file(A, "sock").exists(), "still under the old name");
    for other in [C, P] {
        assert!(!lab.file(other, "sock").exists(), "moved to {other}");
    }

    // The main thread's next hook, in the new conversation, carries its id.
    lab.send_line(&hook(B, false));
    lab.wait_for("the socket to move to B", || {
        lab.file(B, "sock").exists() && !lab.file(A, "sock").exists()
    });
    assert_eq!(inode(&lab.file(B, "sock")), socket);
    lab.wait_pending(B, &["fence-1"]);

    let turn = user_turn("typed on a phone after /clear");
    assert_eq!(lab.inject(B, &turn), "{\"ok\":true}\n");
    lab.wait_output("the injected turn, echoed by the child", |out| {
        contains(out, turn.as_bytes())
    });

    lab.finish(&[A, B, C, P]);
}

#[test]
fn after_clear_the_socket_moves_on_the_init_that_names_the_new_session() {
    // What CLI 2.1.278, .280 and .284 write for /clear with no hooks registered: the
    // reset, still stamped with the old id and carrying an unrelated uuid; a few ms later
    // a system/init naming the new session; the command's result under the new id; and
    // at the start of every turn after, the same init again.
    let mut lab = Lab::start(A);
    let socket = inode(&lab.file(A, "sock"));
    lab.send_line(&reset(P, A));
    lab.send_line(&prompt("fence-1"));
    lab.wait_pending(A, &["fence-1"]);
    assert!(lab.file(A, "sock").exists(), "the reset itself names no session");
    assert!(!lab.file(P, "sock").exists(), "and its new_conversation_id is no session id");

    lab.send_line(&init(B));
    lab.wait_for("the socket to move to B on the init", || {
        lab.file(B, "sock").exists() && !lab.file(A, "sock").exists()
    });
    assert_eq!(inode(&lab.file(B, "sock")), socket, "the same socket, renamed");
    lab.wait_pending(B, &["fence-1"]);

    lab.send_line(&clear_result(B));
    lab.send_line(&init(B));
    lab.send_line(&prompt("fence-2"));
    lab.wait_pending(B, &["fence-1", "fence-2"]);
    assert_eq!(inode(&lab.file(B, "sock")), socket, "the next turn's init changes nothing");

    let turn = user_turn("typed on a phone after /clear");
    assert_eq!(lab.inject(B, &turn), "{\"ok\":true}\n");
    lab.wait_output("the injected turn, echoed by the child", |out| {
        contains(out, turn.as_bytes())
    });
    lab.finish(&[A, B, P]);
}

#[test]
fn a_dead_predecessors_prompts_are_gone_before_the_socket_answers() {
    // A wrapper killed with no chance to clean up (a crash, a force quit) left an open
    // prompt, and half of a rewrite, under the id this session is resumed under.
    let mut lab = Lab::spawn(A, FAKE_CLAUDE, &["0"], |dir| {
        let dead = format!("{}\n", prompt("r-dead"));
        std::fs::write(dir.join(format!("{A}.pending")), dead).unwrap();
        std::fs::write(dir.join(format!("{A}.pending.tmp")), "half a rewrite").unwrap();
    });
    // The socket is there, so from now on the router may read the file: whatever is on
    // it is taken for this session's.
    assert_eq!(lab.pending_ids(A), None, "a dead request, served as this session's");
    assert!(!lab.file(A, "pending.tmp").exists());

    lab.send_line(&prompt("r-live"));
    lab.wait_pending(A, &["r-live"]);
    lab.finish(&[A]);
}

#[test]
fn a_prompt_in_the_same_read_as_the_session_id_is_kept() {
    // The leftovers go before this read is recorded, not after: the session's own first
    // prompt, in the very read that named it, stays on file.
    let mut lab = Lab::start_saying(A, &prompt("r-first"), |dir| {
        let dead = format!("{}\n", prompt("r-dead"));
        std::fs::write(dir.join(format!("{A}.pending")), dead).unwrap();
    });
    lab.wait_pending(A, &["r-first"]);
    lab.finish(&[A]);
}

#[test]
fn a_frame_over_the_limit_is_refused_whole_and_the_session_carries_on() {
    let mut lab = Lab::start(A);
    let copy = lab.dir.join("child-stdin.bin");

    // The limit itself goes through, whole.
    let at_limit = padded(FRAME_LIMIT);
    assert_eq!(lab.inject(A, &at_limit), "{\"ok\":true}\n");
    lab.owed += at_limit.len() + 1;
    lab.wait_echo();

    // One byte more is refused, and not cut to size: a stump of it would be a line the
    // CLI cannot parse, and it exits on one.
    let over = format!("{}\n", padded(FRAME_LIMIT + 1));
    assert_eq!(lab.inject_raw(A, over.as_bytes()), ("{\"ok\":false}\n".to_string(), true));
    // As is a frame far over it, as a long message from a phone would be; the wrapper
    // stops reading it at the limit.
    let far_over = format!("{}\n", padded(FRAME_LIMIT + 100 * 1024));
    let (reply, _) = lab.inject_raw(A, far_over.as_bytes());
    assert_eq!(reply, "{\"ok\":false}\n");

    // Nothing of either reached the child, and the session carries on.
    let after = r#"{"type":"keep_alive","after":"the refusals"}"#;
    lab.send_line(after);
    lab.wait_echo();
    let got = std::fs::read(&copy).unwrap();
    assert_same("what the child received", &got, format!("{at_limit}\n{after}\n").as_bytes());
    lab.finish(&[A]);
}

#[test]
fn an_injection_waits_for_the_editors_frame_in_flight_and_goes_in_after_it() {
    let mut lab = Lab::start(A);
    // A frame larger than one read, as a pasted block or an image is: the first half of
    // it has reached the child when a phone's message comes in.
    let frame = format!("{}\n", padded(20_000));
    let (head, tail) = frame.as_bytes().split_at(frame.len() / 2);
    lab.send(head);
    lab.wait_echo();
    let turn = user_turn("typed on a phone mid-frame");
    let injector = {
        let sock = lab.file(A, "sock");
        let turn = turn.clone();
        std::thread::spawn(move || inject_at(&sock, &turn))
    };
    std::thread::sleep(Duration::from_millis(200));
    assert!(!injector.is_finished(), "it went in without waiting for the frame to end");
    lab.send(tail);
    assert_eq!(injector.join().unwrap(), "{\"ok\":true}\n");
    lab.wait_output("the injected turn, echoed by the child", |out| {
        contains(out, turn.as_bytes())
    });
    let got = std::fs::read(lab.dir.join("child-stdin.bin")).unwrap();
    assert_same("what the child received", &got, format!("{frame}{turn}\n").as_bytes());
    lab.finish(&[A]);
}

#[test]
fn an_injection_the_editors_frame_keeps_waiting_is_refused_and_writes_nothing() {
    let mut lab = Lab::start(A);
    let frame = format!("{}\n", padded(20_000));
    let (head, tail) = frame.as_bytes().split_at(frame.len() / 2);
    lab.send(head);
    lab.wait_echo();
    let asked = Instant::now();
    assert_eq!(lab.inject(A, &user_turn("too long a wait")), "{\"ok\":false}\n");
    let waited = asked.elapsed();
    assert!(
        waited >= Duration::from_millis(900) && waited < PATIENCE,
        "refused after {waited:?}; it waits about a second for the frame to end"
    );
    lab.send(tail);
    lab.wait_echo();
    let got = std::fs::read(lab.dir.join("child-stdin.bin")).unwrap();
    assert_same("what the child received", &got, frame.as_bytes());
    lab.finish(&[A]);
}

/// Every line the editor received, each checked whole: a frame in, a frame out.
fn assert_whole_lines(out: &[u8]) {
    for line in out.split(|&b| b == b'\n').filter(|l| !l.is_empty()) {
        assert!(
            line.starts_with(b"{\"type\":\"") && line.ends_with(b"}"),
            "a broken line on the editor's side: {}…",
            String::from_utf8_lossy(&line[..line.len().min(100)])
        );
    }
}

#[test]
fn the_cancel_for_an_answered_prompt_waits_for_the_clis_line_in_flight() {
    let text = "y".repeat(3000);
    let mut lab = Lab::spawn(A, FAKE_MIDLINE, &["0.8", &text], |_| {});
    lab.wait_output("the first half of the CLI's line", |out| contains(out, b"\"text\":\"yyyy"));
    // The router answers a prompt through the socket. The editor still shows its own
    // picker for it, which the wrapper takes down with a cancel — upstream, where the
    // CLI is in the middle of a line.
    assert_eq!(lab.inject(A, &answer("r-7")), "{\"ok\":true}\n");
    assert!(
        !contains(&lab.output(), b"control_cancel_request"),
        "the cancel went into the middle of the CLI's line"
    );
    lab.wait_output("the cancel, once the CLI's line ended", |out| {
        contains(out, b"control_cancel_request")
    });
    let out = lab.output();
    assert_whole_lines(&out);
    assert!(out.split(|&b| b == b'\n').any(|l| l == cancel("r-7").as_bytes()));
    lab.finish(&[A]);
}

#[test]
fn a_cancel_that_finds_no_line_end_in_time_is_dropped_not_spliced() {
    let text = "z".repeat(3000);
    let mut lab = Lab::spawn(A, FAKE_MIDLINE, &["2.5", &text], |_| {});
    lab.wait_output("the first half of the CLI's line", |out| contains(out, b"\"text\":\"zzzz"));
    assert_eq!(lab.inject(A, &answer("r-8")), "{\"ok\":true}\n");
    // The CLI finishes its line only after the wrapper has given up on the cancel.
    lab.wait_output("the rest of the CLI's line", |out| contains(out, MIDLINE_END));
    std::thread::sleep(Duration::from_millis(300));
    let out = lab.output();
    assert!(!contains(&out, b"control_cancel_request"), "written late, or into the line");
    assert_whole_lines(&out);
    lab.finish(&[A]);
}

/// The editor closes a panel while the CLI is still at work: "end stdin, wait 2 s,
/// SIGTERM" — to the wrapper's pid, and only to it. How the wrapper ended.
fn close_while_busy(lab: &mut Lab, name: &str) -> ExitStatus {
    // A prompt open, so that every record is on file.
    lab.send_line(&prompt("r-open"));
    lab.wait_pending(A, &["r-open"]);
    assert!(lab.file(A, "ctl").exists());
    let child = lab.child_pid();
    lab.close_stdin();
    lab.wait_output("the child seeing EOF", |out| contains(out, EOF_MARKER.as_bytes()));
    assert!(lab.wait_exit(Duration::from_millis(300)).is_none(), "still busy after the EOF");

    lab.signal(name);
    let status = lab.wait_exit(Duration::from_secs(4)).unwrap_or_else(|| {
        panic!("still running 4 s after SIG{name}; the editor's SIGKILL comes at 5 s")
    });
    let orphan = alive(child);
    if orphan {
        // A wrapper that died without passing the signal on left it running.
        sh_kill("-KILL", &child.to_string());
    }
    assert!(!orphan, "the CLI was left running, with nobody reading it");
    let left = lab.leftovers(&[A]);
    assert!(left.is_empty(), "left behind: {left:?}");
    status
}

#[test]
fn a_cli_that_handles_sigterm_ends_the_wrapper_with_its_own_exit_code() {
    // The CLI exits 143 on SIGTERM; unwrapped, that is what the editor would see.
    let mut lab = Lab::spawn(A, FAKE_TRAPPING, &["10"], |_| {});
    let status = close_while_busy(&mut lab, "TERM");
    assert_eq!(status.code(), Some(143), "the CLI's own end, not the signal's: {status}");
}

#[test]
fn a_signal_the_cli_shrugs_off_leaves_the_session_running() {
    let mut lab = Lab::spawn(A, FAKE_TRAPPING, &["0"], |_| {});
    lab.signal("HUP");
    // Still relaying both ways, and still taking injections.
    lab.send_line(&prompt("r-after-hup"));
    lab.wait_pending(A, &["r-after-hup"]);
    let turn = user_turn("typed on a phone after the SIGHUP");
    assert_eq!(lab.inject(A, &turn), "{\"ok\":true}\n");
    lab.wait_output("the injected turn, echoed by the child", |out| {
        contains(out, turn.as_bytes())
    });
    // And when the session does end, it ends as the CLI did — not of a signal it shrugged
    // off long before.
    lab.finish(&[A]);
}

/// The CLI has exited, but a background command it started still holds its stdout. The
/// relay would wait for that command — and with the child reaped, the editor's SIGTERM
/// has nobody left to be passed on to — so the wrapper finishes a moment after its child
/// all the same.
#[test]
fn a_command_the_cli_left_holding_its_output_does_not_keep_the_wrapper_up() {
    let mut lab = Lab::spawn(A, FAKE_BACKGROUNDED, &["20"], |_| {});
    let bg = lab.dir.join("child-stdin.bin.bg");
    lab.wait_for("the background command's pid", || bg.exists());
    let pid = std::fs::read_to_string(&bg).unwrap();
    let took = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| lab.finish(&[A])));
    // Not left for 20 s whatever happened above.
    sh_kill("-KILL", pid.trim());
    let took = took.unwrap_or_else(|e| std::panic::resume_unwind(e));
    assert!(took < Duration::from_secs(3), "waited {took:?} on a command the CLI left behind");
}

fn a_busy_session_closed_by(name: &str, number: i32) {
    // A CLI that carries on after its stdin closes, as one mid-turn does, and dies of the
    // signal when it comes.
    let mut lab = Lab::start_lingering(A, 10.0);
    let status = close_while_busy(&mut lab, name);
    assert_eq!(status.signal(), Some(number), "it ends as the CLI did, of SIG{name}: {status}");
}

#[test]
fn a_session_still_busy_at_the_editors_sigterm_is_stopped_and_cleaned_up() {
    a_busy_session_closed_by("TERM", 15);
}

#[test]
fn a_session_still_busy_at_a_sighup_is_stopped_and_cleaned_up() {
    a_busy_session_closed_by("HUP", 1);
}

#[test]
fn another_build_is_run_only_if_it_honours_the_test_directory() {
    assert!(honours_test_dir(Path::new(env!("CARGO_BIN_EXE_claude-wrapper"))));
    let dir = lab_dir();
    let deployed = dir.join("claude-wrapper");
    std::fs::write(&deployed, "a build that knows only /tmp/agent-router-sessions").unwrap();
    assert!(!honours_test_dir(&deployed));
    assert!(!honours_test_dir(&dir.join("no-such-build")));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn once_the_editor_has_closed_stdin_an_injection_fails_cleanly() {
    // A child that stays up a moment after EOF, so there is a window to try.
    let mut lab = Lab::start_lingering(A, 1.5);
    lab.close_stdin();
    lab.wait_output("the child seeing EOF", |out| contains(out, EOF_MARKER.as_bytes()));
    assert_eq!(lab.inject(A, &user_turn("too late")), "{\"ok\":false}\n");
    lab.finish(&[A]);
    let copy = std::fs::read(lab.dir.join("child-stdin.bin")).unwrap();
    assert!(!contains(&copy, b"too late"), "nothing reached the child after its EOF");
}
