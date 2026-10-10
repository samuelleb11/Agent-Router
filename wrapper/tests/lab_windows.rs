//! The lab on Windows: the real wrapper binary, end to end, around a fake child.
//!
//! Never a real `claude`, never a real session. The child is the `fake_cli` example, which
//! `cargo test` builds beside the tests: it announces a session id the way the CLI's
//! `system/init` does, then echoes its stdin to its stdout, and into a file, until EOF. So
//! everything "the CLI says" here is something the test first wrote into the wrapper. The
//! unix lab (`lab.rs`) does the same with `/bin/sh`.
//!
//! The records directory is a fresh one under `%TEMP%`, handed to the wrapper through its
//! test-only `AGENT_ROUTER_WRAPPER_TEST_DIR`. A wrapper given that directory publishes its
//! pipe as `\\.\pipe\agent-router-<user>-lab-<directory name>-<session id>`, never under a
//! name the router would take for a real session. The name is computed here from the
//! contract (see [`pipe`]), not from the wrapper's code, so this checks the contract too.
//!
//! Compiled for Windows only; never run on the machine this was written on. Run it with
//! `cargo test -p claude-wrapper` on Windows.

#![cfg(windows)]

use std::io::{Read, Write};
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{ERROR_SEM_TIMEOUT, WAIT_OBJECT_0};
use windows_sys::Win32::Storage::FileSystem::SECURITY_IDENTIFICATION;
use windows_sys::Win32::System::Pipes::{WaitNamedPipeW, NMPWAIT_NOWAIT};
use windows_sys::Win32::System::Threading::{
    OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE,
};
use windows_sys::Win32::System::WindowsProgramming::GetUserNameW;

/// How long anything in the lab may take before it counts as never. Generous: a first
/// process start on Windows can wait on a virus scan.
const PATIENCE: Duration = Duration::from_secs(20);

/// The editor's grace between ending a session's stdin and killing it.
const EDITOR_GRACE: Duration = Duration::from_secs(2);

const A: &str = "aaaaaaaa-1ab0-4000-8000-000000000001";
const B: &str = "bbbbbbbb-1ab0-4000-8000-000000000002";

/// What the fake child prints once its stdin has closed.
const EOF_MARKER: &str = r#"{"type":"lab","event":"stdin-closed"}"#;

/// The wrapper's limit on an injected frame, newline not counted.
const FRAME_LIMIT: usize = 1024 * 1024;

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

fn user_turn(text: &str) -> String {
    format!(
        r#"{{"type":"user","message":{{"role":"user","content":[{{"type":"text","text":"{text}"}}]}}}}"#
    )
}

/// A frame of exactly `len` bytes, newline not counted, that the CLI would take and ignore.
fn padded(len: usize) -> String {
    const HEAD: &str = r#"{"type":"keep_alive","pad":""#;
    const TAIL: &str = r#""}"#;
    format!("{HEAD}{}{TAIL}", "x".repeat(len - HEAD.len() - TAIL.len()))
}

/// The contract's piece of a pipe name: ASCII letters and digits, `_` and `-` kept, any other
/// character an `_`, at most 64 of them.
fn part(raw: &str) -> String {
    raw.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .take(64)
        .collect()
}

/// `GetUserNameW`, as the contract names the user.
fn user_name() -> String {
    let mut buf = [0u16; 257];
    let mut len = buf.len() as u32;
    // SAFETY: a buffer of `len` characters.
    assert!(
        unsafe { GetUserNameW(buf.as_mut_ptr(), &mut len) } != 0,
        "GetUserNameW"
    );
    let name = buf.split(|&c| c == 0).next().unwrap_or_default();
    String::from_utf16_lossy(name)
}

/// The pipe a wrapper publishing into `dir` answers under for `session_id`.
fn pipe(dir: &Path, session_id: &str) -> String {
    let tag = part(&dir.file_name().unwrap().to_string_lossy());
    format!(
        r"\\.\pipe\agent-router-{}-lab-{tag}-{session_id}",
        part(&user_name())
    )
}

/// Whether anyone serves a pipe of that name, asked without connecting.
fn in_use(name: &str) -> bool {
    let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
    // SAFETY: a nul-terminated name, alive for the call.
    if unsafe { WaitNamedPipeW(wide.as_ptr(), NMPWAIT_NOWAIT) } != 0 {
        return true;
    }
    std::io::Error::last_os_error().raw_os_error() == Some(ERROR_SEM_TIMEOUT as i32)
}

/// The router's side: the pipe opened as a file, the server allowed to identify the client
/// and no more.
fn connect(name: &str) -> std::io::Result<std::fs::File> {
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .security_qos_flags(SECURITY_IDENTIFICATION)
        .open(name)
}

/// Write `bytes` into the pipe, then read the reply to the end. The write may fail part way,
/// when the wrapper stops reading a frame over the limit; the reply is read all the same.
fn exchange(name: &str, bytes: &[u8]) -> (String, bool) {
    let mut pipe = connect(name).expect("connect to the control pipe");
    let wrote = pipe.write_all(bytes).is_ok();
    let mut reply = String::new();
    let _ = pipe.read_to_string(&mut reply);
    (reply, wrote)
}

/// One frame, the way the router sends it; the wrapper's reply.
fn inject_at(name: &str, frame: &str) -> String {
    exchange(name, format!("{frame}\n").as_bytes()).0
}

/// Whether the process `pid` is gone, or goes within `limit`.
fn ends_within(pid: u32, limit: Duration) -> bool {
    // SAFETY: a plain open; a null handle means there is no such process (any more).
    let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
    if handle.is_null() {
        return true;
    }
    // SAFETY: a handle OpenProcess just returned, owned by nothing else.
    let handle = unsafe { OwnedHandle::from_raw_handle(handle) };
    let ms = u32::try_from(limit.as_millis()).unwrap_or(u32::MAX);
    // SAFETY: a process handle of ours.
    unsafe { WaitForSingleObject(handle.as_raw_handle(), ms) == WAIT_OBJECT_0 }
}

/// The fake CLI, which `cargo test` builds into the examples beside this test's directory.
fn fake_cli() -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    let profile = exe.parent().and_then(Path::parent).unwrap();
    let cli = profile.join("examples").join("fake_cli.exe");
    assert!(
        cli.is_file(),
        "{} is missing: a plain `cargo test -p claude-wrapper` builds it; for a run of this file \
         alone, `cargo build -p claude-wrapper --example fake_cli` first",
        cli.display()
    );
    cli
}

#[derive(Default)]
struct Output {
    bytes: Vec<u8>,
    closed: bool,
}

/// One wrapper around one fake child, in a directory of its own. Dropping it kills whatever
/// is still running and removes the directory.
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
        Lab::spawn(id, 0.0, 0, |_| {})
    }

    /// The wrapper around `fake_cli <id> <stdin copy> <linger> <code>`, in a directory of its
    /// own that `prepare` gets first; returned once the control pipe is there.
    fn spawn(id: &str, linger_s: f32, code: i32, prepare: impl FnOnce(&Path)) -> Lab {
        let cli = fake_cli();
        let dir = lab_dir();
        prepare(&dir);
        let mut wrapper = Command::new(env!("CARGO_BIN_EXE_claude-wrapper"))
            .arg(&cli)
            .arg(id)
            .arg(dir.join("child-stdin.bin"))
            .arg(linger_s.to_string())
            .arg(code.to_string())
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
        let lab = Lab {
            dir,
            wrapper,
            stdin,
            out,
            owed,
        };
        lab.wait_for("the control pipe", || in_use(&lab.pipe(id)));
        lab
    }

    fn pipe(&self, id: &str) -> String {
        pipe(&self.dir, id)
    }

    fn file(&self, id: &str, ext: &str) -> PathBuf {
        self.dir.join(format!("{id}.{ext}"))
    }

    fn send(&mut self, bytes: &[u8]) {
        let stdin = self.stdin.as_mut().expect("stdin is still open");
        stdin
            .write_all(bytes)
            .and_then(|()| stdin.flush())
            .expect("the wrapper takes its input");
        self.owed += bytes.len();
    }

    fn send_line(&mut self, frame: &str) {
        self.send(format!("{frame}\n").as_bytes());
    }

    fn wait_echo(&self) {
        let owed = self.owed;
        self.wait_output("the echo of everything sent", |out| out.len() >= owed);
    }

    fn close_stdin(&mut self) {
        drop(self.stdin.take());
    }

    fn wait_for(&self, what: &str, mut cond: impl FnMut() -> bool) {
        let deadline = Instant::now() + PATIENCE;
        while !cond() {
            assert!(Instant::now() < deadline, "never happened: {what}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn wait_output(&self, what: &str, pred: impl Fn(&[u8]) -> bool) {
        let (m, cv) = &*self.out;
        let deadline = Instant::now() + PATIENCE;
        let mut o = lock(m);
        while !pred(&o.bytes) {
            let now = Instant::now();
            assert!(
                now < deadline && !o.closed,
                "never came out of the wrapper: {what}"
            );
            o = cv
                .wait_timeout(o, deadline - now)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }

    fn wait_closed(&self) {
        let (m, cv) = &*self.out;
        let deadline = Instant::now() + PATIENCE;
        let mut o = lock(m);
        while !o.closed {
            let now = Instant::now();
            assert!(now < deadline, "the wrapper's stdout never closed");
            o = cv
                .wait_timeout(o, deadline - now)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }

    fn output(&self) -> Vec<u8> {
        lock(&self.out.0).bytes.clone()
    }

    fn copy(&self) -> Vec<u8> {
        std::fs::read(self.dir.join("child-stdin.bin")).unwrap_or_default()
    }

    fn child_pid(&self) -> u32 {
        let path = self.dir.join("child-stdin.bin.pid");
        std::fs::read_to_string(path)
            .unwrap()
            .trim()
            .parse()
            .unwrap()
    }

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

    fn inject(&self, id: &str, frame: &str) -> String {
        inject_at(&self.pipe(id), frame)
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
            .flat_map(|id| ["ctl", "pending", "pending.tmp"].map(|ext| format!("{id}.{ext}")))
            .filter(|name| self.dir.join(name).exists())
            .collect()
    }

    /// Close stdin the way the editor ends a session, and see the wrapper out: it exits by
    /// itself with the child's code and leaves nothing behind for any of `ids`, its pipes
    /// included. How long it took from the close.
    fn finish(&mut self, ids: &[&str]) -> (Duration, ExitStatus) {
        self.close_stdin();
        let closed = Instant::now();
        let status = self
            .wait_exit(Duration::from_secs(10))
            .expect("the wrapper still runs 10 s after its stdin closed: the child never saw EOF");
        let took = closed.elapsed();
        self.wait_closed();
        let left = self.leftovers(ids);
        assert!(left.is_empty(), "left behind after a clean exit: {left:?}");
        for id in ids {
            assert!(
                !in_use(&self.pipe(id)),
                "a pipe under {id} outlived the wrapper"
            );
        }
        (took, status)
    }
}

impl Drop for Lab {
    fn drop(&mut self) {
        let _ = self.wrapper.kill();
        let _ = self.wrapper.wait();
        // The child goes with the wrapper's job; its files may stay locked a moment longer.
        for _ in 0..50 {
            if std::fs::remove_dir_all(&self.dir).is_ok() || !self.dir.exists() {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A fresh directory for one lab run, its name unique to it and with it its pipe names.
fn lab_dir() -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "cwlab-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
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

fn assert_same(what: &str, got: &[u8], want: &[u8]) {
    if got == want {
        return;
    }
    let at = got
        .iter()
        .zip(want)
        .position(|(g, w)| g != w)
        .unwrap_or(got.len().min(want.len()));
    let show = |b: &[u8]| {
        String::from_utf8_lossy(&b[at.min(b.len())..(at + 60).min(b.len())]).into_owned()
    };
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
    let mut small = Vec::new();
    for i in 0..300 {
        small.extend_from_slice(
            format!(r#"{{"type":"stream_event","event":{{"type":"content_block_delta","index":0,"delta":{{"type":"text_delta","text":"tok {i}"}}}},"session_id":"{A}"}}"#)
                .as_bytes(),
        );
        small.push(b'\n');
    }
    lab.send(&small);
    sent.extend_from_slice(&small);
    // Far past one read each way, and past the recorder's carry.
    let big = format!(
        r#"{{"type":"user","message":{{"content":"{}"}}}}"#,
        "x".repeat(3 << 20)
    ) + "\n";
    lab.send(big.as_bytes());
    sent.extend_from_slice(big.as_bytes());
    let split = format!("{}\n", prompt("r-split"));
    for part in [&split.as_bytes()[..17], &split.as_bytes()[17..]] {
        lab.send(part);
        sent.extend_from_slice(part);
        std::thread::sleep(Duration::from_millis(50));
    }
    // Neither UTF-8 nor JSON, a CRLF, an empty line, and a last line with no newline.
    let odd = b"\xff\xfe\x00 not json \x80\r\n\n{\"type\":\"lab\",\"partial\":true";
    lab.send(odd);
    sent.extend_from_slice(odd);
    lab.wait_pending(A, &["r-split"]);

    let (took, status) = lab.finish(&[A]);
    assert!(status.success(), "{status}");
    assert!(
        took < EDITOR_GRACE,
        "{took:?} to exit after its stdin closed"
    );
    let mut expected = format!("{}\n", init(A)).into_bytes();
    expected.extend_from_slice(&sent);
    expected.extend_from_slice(format!("{EOF_MARKER}\n").as_bytes());
    assert_same("what the editor received", &lab.output(), &expected);
    assert_same("what the child received", &lab.copy(), &sent);
}

#[test]
fn a_frame_through_the_pipe_reaches_the_child_and_a_bare_connect_types_nothing() {
    let mut lab = Lab::start(A);
    // The router's liveness probe the unix way: connect, and hang up with nothing written.
    drop(connect(&lab.pipe(A)).expect("connect"));
    let turn = user_turn("typed on a phone");
    assert_eq!(lab.inject(A, &turn), "{\"ok\":true}\n");
    lab.wait_output("the injected turn, echoed", |out| {
        contains(out, turn.as_bytes())
    });
    assert_eq!(
        lab.inject(A, ""),
        "{\"ok\":false}\n",
        "an empty line types nothing"
    );
    lab.close_stdin();
    lab.wait_output("the EOF", |out| contains(out, EOF_MARKER.as_bytes()));
    assert_same(
        "what the child received",
        &lab.copy(),
        format!("{turn}\n").as_bytes(),
    );
    lab.finish(&[A]);
}

#[test]
fn prompts_are_kept_in_the_sessions_directory_until_answered() {
    let mut lab = Lab::start(A);
    lab.send_line(&prompt("zz-older"));
    lab.send_line(&prompt("aa-newer"));
    lab.wait_pending(A, &["zz-older", "aa-newer"]);
    // Answered through the pipe: the CLI (here, its echo) carries the answer back.
    assert_eq!(lab.inject(A, &answer("zz-older")), "{\"ok\":true}\n");
    lab.wait_pending(A, &["aa-newer"]);
    let ctl = std::fs::read_to_string(lab.file(A, "ctl")).unwrap();
    assert!(ctl.contains(&answer("zz-older")), "{ctl}");
    // The editor's picker for it is taken down with a cancel, upstream.
    lab.wait_output("the cancel upstream", |out| {
        contains(
            out,
            br#"{"type":"control_cancel_request","request_id":"zz-older"}"#,
        )
    });
    lab.finish(&[A]);
}

#[test]
fn an_init_naming_a_new_session_moves_the_pipe_and_the_records() {
    let mut lab = Lab::start(A);
    lab.send_line(&prompt("r-1"));
    lab.wait_pending(A, &["r-1"]);
    lab.send_line(&init(B));
    lab.wait_for("the pipe to move to B", || {
        in_use(&lab.pipe(B)) && !in_use(&lab.pipe(A))
    });
    lab.wait_pending(B, &["r-1"]);
    assert!(lab.file(B, "ctl").exists() && !lab.file(A, "ctl").exists());
    assert!(!lab.file(A, "pending").exists());
    let turn = user_turn("typed after /clear");
    assert_eq!(lab.inject(B, &turn), "{\"ok\":true}\n");
    lab.wait_output("the injected turn, echoed", |out| {
        contains(out, turn.as_bytes())
    });
    assert!(
        connect(&lab.pipe(A)).is_err(),
        "nothing answers under the old name"
    );
    lab.finish(&[A, B]);
}

#[test]
fn a_frame_over_the_limit_is_refused_whole_and_the_session_carries_on() {
    let mut lab = Lab::start(A);
    let at_limit = padded(FRAME_LIMIT);
    assert_eq!(lab.inject(A, &at_limit), "{\"ok\":true}\n");
    lab.owed += at_limit.len() + 1;
    lab.wait_echo();
    // One byte more is refused, and not cut to size. The wrapper stops reading at the limit,
    // so the rest of the write may fail; the refusal is there to read all the same.
    let over = format!("{}\n", padded(FRAME_LIMIT + 1));
    assert_eq!(
        exchange(&lab.pipe(A), over.as_bytes()).0,
        "{\"ok\":false}\n"
    );
    let far_over = format!("{}\n", padded(FRAME_LIMIT + 100 * 1024));
    assert_eq!(
        exchange(&lab.pipe(A), far_over.as_bytes()).0,
        "{\"ok\":false}\n"
    );
    let after = r#"{"type":"keep_alive","after":"the refusals"}"#;
    lab.send_line(after);
    lab.wait_echo();
    assert_same(
        "what the child received",
        &lab.copy(),
        format!("{at_limit}\n{after}\n").as_bytes(),
    );
    lab.finish(&[A]);
}

#[test]
fn a_dead_predecessors_prompts_are_gone_before_the_pipe_answers() {
    let mut lab = Lab::spawn(A, 0.0, 0, |dir| {
        std::fs::write(
            dir.join(format!("{A}.pending")),
            format!("{}\n", prompt("r-dead")),
        )
        .unwrap();
        std::fs::write(dir.join(format!("{A}.pending.tmp")), "half a rewrite").unwrap();
    });
    assert_eq!(
        lab.pending_ids(A),
        None,
        "a dead request, served as this session's"
    );
    assert!(!lab.file(A, "pending.tmp").exists());
    lab.send_line(&prompt("r-live"));
    lab.wait_pending(A, &["r-live"]);
    lab.finish(&[A]);
}

#[test]
fn once_the_editor_has_closed_stdin_an_injection_fails_cleanly() {
    let mut lab = Lab::spawn(A, 1.5, 0, |_| {});
    lab.close_stdin();
    lab.wait_output("the child seeing EOF", |out| {
        contains(out, EOF_MARKER.as_bytes())
    });
    assert_eq!(lab.inject(A, &user_turn("too late")), "{\"ok\":false}\n");
    lab.finish(&[A]);
    assert!(
        !contains(&lab.copy(), b"too late"),
        "nothing reached the child after its EOF"
    );
}

#[test]
fn the_wrapper_ends_with_the_clis_exit_code() {
    let mut lab = Lab::spawn(A, 0.0, 7, |_| {});
    let (_, status) = lab.finish(&[A]);
    assert_eq!(status.code(), Some(7));
}

/// The editor closes a busy session with "end stdin, wait 2 s, TerminateProcess" on the
/// wrapper. Nothing runs in the wrapper after that, so the CLI must end with it, as it would
/// have unwrapped. The pipe goes with the wrapper; the records stay for the router's janitor.
#[test]
fn the_cli_ends_with_the_wrapper_when_the_editor_kills_it() {
    let mut lab = Lab::spawn(A, 30.0, 0, |_| {});
    lab.send_line(&prompt("r-open"));
    lab.wait_pending(A, &["r-open"]);
    let child = lab.child_pid();
    lab.close_stdin();
    lab.wait_output("the child seeing EOF", |out| {
        contains(out, EOF_MARKER.as_bytes())
    });
    assert!(
        lab.wait_exit(Duration::from_millis(500)).is_none(),
        "still busy after the EOF"
    );
    lab.wrapper
        .kill()
        .expect("TerminateProcess, as the editor does");
    let _ = lab.wrapper.wait();
    assert!(
        ends_within(child, Duration::from_secs(5)),
        "the CLI was left running, with nobody reading it"
    );
    assert!(!in_use(&lab.pipe(A)), "the pipe outlived the wrapper");
}
