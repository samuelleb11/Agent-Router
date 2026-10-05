//! Sending input into a *running* Claude Code session.
//!
//! Every live session binds a Unix socket at `/tmp/cc-socks/<pid>.sock` and publishes a
//! token for it at `~/.claude/sessions/<pid>.<sha256(socket path)>.key`. A peer that can
//! read that key may open the socket and push a user message straight into the session's
//! input queue — the same queue the terminal writes into. Two newline-delimited JSON
//! frames, captured off the wire:
//!
//! ```text
//! {"type":"auth","token":"<32 hex from the key file>"}
//! {"msgV":1,"msg_id":"<uuid>","type":"user",
//!  "message":{"role":"user","content":"…"},"priority":"next"}
//! ```
//!
//! Sending the content bare, without the `<cross-session-message>` wrapper the
//! agent-to-agent path uses, does NOT make it arrive as an ordinary user turn: the
//! receiver adds its own framing regardless ("Another Claude session sent a message",
//! plus a paragraph telling it to treat the sender as a teammate rather than as its
//! user). That is correct for what this channel is and wrong for a message its user
//! typed on a phone, and it is not removable from this side — which is why
//! `send_via_wrapper` exists and is preferred.
//!
//! **This is an internal protocol.** Nothing about it is published, and a CLI update may
//! change it without warning. So every failure here is reported to the caller rather than
//! swallowed: the live view degrades to read-only and says why, instead of pretending a
//! message was delivered. Everything else in the live view reads transcripts and is
//! unaffected if this whole module stops working.

use std::io::Write;
#[cfg(unix)]
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use serde::Deserialize;
use sha2::{Digest, Sha256};

/// The key file's `procStart` is the same `ps lstart` rendering the session registry
/// stores, so one parser serves both.
use crate::registry::parse_proc_start;

#[cfg(unix)]
const SOCK_DIR: &str = "/tmp/cc-socks";
/// Where a wrapped session offers its control socket (see `wrapper/`).
#[cfg(unix)]
const WRAPPER_DIR: &str = "/tmp/agent-router-sessions";

/// Where wrapped sessions keep their sockets and records. A function rather than the
/// constant at every use, so everything that reads the directory has a `_in` twin that
/// takes it as an argument: tests point those at a temp dir instead of the real one,
/// which live sessions are using.
#[cfg(unix)]
pub fn wrapper_dir() -> PathBuf {
    PathBuf::from(WRAPPER_DIR)
}

/// On Windows the records live in the user's local application data, by the wrapper's own
/// rule (`wrapper/src/main.rs`, `sessions_dir_from`); the control channel itself is a named
/// pipe (see `pipe`). With neither variable absolute the wrapper writes no records at all,
/// and a directory nobody writes finds none.
#[cfg(windows)]
pub fn wrapper_dir() -> PathBuf {
    let absolute = |v: &str| std::env::var_os(v).map(PathBuf::from).filter(|p| p.is_absolute());
    absolute("LOCALAPPDATA")
        .or_else(|| absolute("USERPROFILE").map(|p| p.join("AppData").join("Local")))
        .map(|local| local.join("agent-router").join("sessions"))
        .unwrap_or_else(|| std::env::temp_dir().join("agent-router-no-profile"))
}
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
/// A session started within this long of a transcript's first event is taken to own it.
const START_TOLERANCE_MS: i64 = 120_000;

#[derive(Deserialize)]
struct KeyFile {
    #[serde(rename = "peerToken")]
    peer_token: String,
    #[serde(rename = "procStart")]
    proc_start: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Peer {
    pub pid: u32,
    pub socket: PathBuf,
    pub cwd: Option<String>,
    /// The session id this process was resumed from, when it was given one.
    pub resumed: Option<String>,
    pub proc_start_ms: Option<i64>,
}

/// Where a session publishes the token for its socket.
fn key_path(socket: &str) -> PathBuf {
    let digest = Sha256::digest(socket.as_bytes());
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    let pid = socket
        .rsplit('/')
        .next()
        .and_then(|f| f.strip_suffix(".sock"))
        .unwrap_or("");
    // Beside the session records, wherever `CLAUDE_CONFIG_DIR` puts them.
    crate::registry::sessions_dir().join(format!("{pid}.{hex}.key"))
}

#[cfg(unix)]
fn proc_cwd(pid: u32) -> Option<String> {
    let out = Command::new("/usr/sbin/lsof")
        .args(["-a", "-d", "cwd", "-p", &pid.to_string(), "-Fn"])
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .find_map(|l| l.strip_prefix('n').map(str::to_owned))
}

#[cfg(unix)]
fn proc_resumed(pid: u32) -> Option<String> {
    let out = Command::new("/bin/ps")
        .args(["-o", "command=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .find_map(|a| a.strip_prefix("--resume=").map(str::to_owned))
}

/// Claude Code's own peer sockets are a Unix-socket feature: on Windows the wrapper is the
/// only way in.
#[cfg(windows)]
pub fn live_peers() -> Vec<Peer> {
    Vec::new()
}

/// Every session currently listening for peer input.
#[cfg(unix)]
pub fn live_peers() -> Vec<Peer> {
    let Ok(dir) = std::fs::read_dir(SOCK_DIR) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for e in dir.flatten() {
        let path = e.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some(pid) = name.strip_suffix(".sock").and_then(|p| p.parse::<u32>().ok()) else {
            continue;
        };
        let proc_start_ms = std::fs::read_to_string(key_path(&path.to_string_lossy()))
            .ok()
            .and_then(|raw| serde_json::from_str::<KeyFile>(&raw).ok())
            .and_then(|k| k.proc_start)
            .as_deref()
            .and_then(parse_proc_start);
        out.push(Peer {
            pid,
            cwd: proc_cwd(pid),
            resumed: proc_resumed(pid),
            proc_start_ms,
            socket: path,
        });
    }
    out
}

/// Which live process owns a given transcript.
///
/// Two signals, because neither alone is enough: a resumed session names its transcript
/// on the command line, and a fresh one does not — but a fresh one's transcript begins
/// when its process did, which the key file records. The cwd narrows both.
pub fn resolve(session_id: &str, cwd: Option<&str>, first_event_ms: Option<i64>) -> Option<Peer> {
    let peers = live_peers();

    if let Some(p) = peers.iter().find(|p| p.resumed.as_deref() == Some(session_id)) {
        return Some(p.clone());
    }

    let mut in_dir: Vec<&Peer> = peers
        .iter()
        .filter(|p| match (p.cwd.as_deref(), cwd) {
            (Some(a), Some(b)) => a == b,
            _ => false,
        })
        // A process resumed into some *other* transcript cannot own this one.
        .filter(|p| p.resumed.is_none())
        .collect();

    if in_dir.len() == 1 {
        return in_dir.pop().cloned();
    }
    // Several sessions in one project: the one that started when this transcript did.
    let first = first_event_ms?;
    in_dir
        .into_iter()
        .filter_map(|p| p.proc_start_ms.map(|s| ((s - first).abs(), p)))
        .filter(|(d, _)| *d <= START_TOLERANCE_MS)
        .min_by_key(|(d, _)| *d)
        .map(|(_, p)| p.clone())
}

/// Is this session running under the wrapper?
pub fn wrapper_socket(session_id: &str) -> Option<PathBuf> {
    wrapper_socket_in(&wrapper_dir(), session_id)
}

pub fn wrapper_socket_in(dir: &Path, session_id: &str) -> Option<PathBuf> {
    if session_id.is_empty()
        || !session_id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
        return None;
    }
    #[cfg(unix)]
    {
        let p = dir.join(format!("{session_id}.sock"));
        p.exists().then_some(p)
    }
    // No file stands for a pipe: the name is the address, and present is whether a
    // wrapper holds it now.
    #[cfg(windows)]
    {
        let _ = dir;
        let name = pipe::name(session_id);
        pipe::present(&name).then(|| PathBuf::from(name))
    }
}

/// Is a wrapped session actually there to take a message?
///
/// A socket file is not a session. The wrapper unlinks its socket on the way out, but a
/// wrapper that was killed leaves the file behind, and a phone showing that session as
/// "live" is the one place the mistake costs the most — nothing on the phone can check.
/// So the file is asked rather than trusted: a connection nobody accepts means nothing
/// is listening, and the leftovers are removed so the next listing does not ask again.
/// The wrapper reads EOF from the bare connect, answers `{"ok":false}` and hangs up;
/// nothing reaches the session.
pub fn wrapper_alive(session_id: &str) -> bool {
    wrapper_alive_in(&wrapper_dir(), session_id)
}

#[cfg(windows)]
pub fn wrapper_alive_in(dir: &Path, session_id: &str) -> bool {
    // A pipe never outlives its wrapper, so asking whether the name is held is the whole
    // check, and costs the wrapper nothing. A wrapper killed mid-prompt leaves its
    // `.pending` behind, which must not read as a prompt still open.
    if wrapper_socket_in(dir, session_id).is_some() {
        return true;
    }
    if !session_id.is_empty() && session_id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        let _ = std::fs::remove_file(dir.join(format!("{session_id}.pending")));
    }
    false
}

#[cfg(unix)]
pub fn wrapper_alive_in(dir: &Path, session_id: &str) -> bool {
    let Some(path) = wrapper_socket_in(dir, session_id) else {
        return false;
    };
    let Ok(addr) = std::os::unix::net::SocketAddr::from_pathname(&path) else {
        return false;
    };
    match UnixStream::connect_addr(&addr) {
        Ok(s) => {
            let _ = s.shutdown(std::net::Shutdown::Both);
            true
        }
        Err(e)
            if matches!(
                e.kind(),
                std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::NotFound
            ) =>
        {
            let _ = std::fs::remove_file(&path);
            let _ = std::fs::remove_file(dir.join(format!("{session_id}.pending")));
            false
        }
        Err(_) => false,
    }
}

/// Inject a user turn through the wrapper — the bytes the editor writes when someone
/// types, with no framing of any kind.
///
/// The peer socket can only ever deliver a *peer* message, which Claude Code wraps in
/// "another Claude session sent a message" and a paragraph telling the receiver to
/// treat it as a teammate rather than as its user. That framing is correct for what
/// that channel is, and wrong for a message its user typed on a phone — which is the
/// whole reason the wrapper exists.
pub fn send_via_wrapper(session_id: &str, text: &str) -> anyhow::Result<()> {
    send_via_wrapper_in(&wrapper_dir(), session_id, text)
}

pub fn send_via_wrapper_in(dir: &Path, session_id: &str, text: &str) -> anyhow::Result<()> {
    write_via_wrapper_in(dir, session_id, &user_frame(text))
}

/// A typed message, as the editor frames it.
fn user_frame(text: &str) -> serde_json::Value {
    serde_json::json!({
        "type": "user",
        "message": { "role": "user", "content": [{ "type": "text", "text": text }] },
    })
}

/// Stop whatever the session is doing, the way the editor's stop button does.
///
/// `cancel_queued` matches the CLI's own interrupt payload: a stop pressed by a person
/// means "and don't then run the things I typed while it was busy", which is the whole
/// reason the button exists.
pub fn interrupt_via_wrapper(session_id: &str) -> anyhow::Result<()> {
    write_via_wrapper(
        session_id,
        &serde_json::json!({
            "type": "control_request",
            "request_id": uuid_v4(),
            "request": { "subtype": "interrupt", "cancel_queued": true },
        }),
    )
}

/// Switch the session's model, the way the editor's picker does: a `set_model`
/// control request. The CLI answers on its control channel, which the wrapper records;
/// `control_response` reads that answer back. Returns the request id to look for.
pub fn set_model_via_wrapper(session_id: &str, model: &str) -> anyhow::Result<String> {
    let request_id = uuid_v4();
    write_via_wrapper(
        session_id,
        &serde_json::json!({
            "type": "control_request",
            "request_id": request_id,
            "request": { "subtype": "set_model", "model": model },
        }),
    )?;
    Ok(request_id)
}

/// Set the effort level. There is no control request for it; `/effort <level>` is a
/// command the CLI runs itself in this mode rather than sending to the model, so it
/// goes in as a typed line.
pub fn set_effort_via_wrapper(session_id: &str, level: &str) -> anyhow::Result<()> {
    send_via_wrapper(session_id, &format!("/effort {level}"))
}

/// The CLI's answer to a control request we injected, once it has one: the
/// `response` object of the matching `control_response` in the session's recording.
pub fn control_response(session_id: &str, request_id: &str) -> Option<serde_json::Value> {
    control_response_in(&wrapper_dir(), session_id, request_id)
}

pub fn control_response_in(
    dir: &Path,
    session_id: &str,
    request_id: &str,
) -> Option<serde_json::Value> {
    let path = dir.join(format!("{session_id}.ctl"));
    let text = std::fs::read_to_string(path).ok()?;
    text.lines()
        .rev()
        .filter(|l| l.contains("\"type\":\"control_response\"") && l.contains(request_id))
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .find(|v| v["response"]["request_id"] == request_id)
        .map(|v| v["response"].clone())
}

/// The longest line a wrapper takes from its socket, newline included.
///
/// The builds already deployed inside long-running sessions read a frame in 8 KiB pieces,
/// stop as soon as they hold more than a mebibyte, newline or not, and write what they
/// have into the CLI anyway with a newline added — and the CLI treats a line it cannot
/// parse as fatal and exits, so the session is gone. The current wrapper refuses such a
/// frame whole instead. A line of this length or less is taken whole by both, so nothing
/// longer is sent.
pub const WRAPPER_LINE_MAX: usize = 1024 * 1024;

/// A frame longer than a wrapper can take whole (`WRAPPER_LINE_MAX`), refused before a
/// byte of it was sent.
#[derive(Debug)]
pub struct TooLong {
    /// The line as it would have been sent, newline included.
    pub bytes: usize,
}

impl std::fmt::Display for TooLong {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "a {} byte frame, over the {WRAPPER_LINE_MAX} bytes a wrapper takes in one line",
            self.bytes
        )
    }
}

impl std::error::Error for TooLong {}

/// One newline-delimited frame into a wrapped session's stdin.
fn write_via_wrapper(session_id: &str, frame: &serde_json::Value) -> anyhow::Result<()> {
    write_via_wrapper_in(&wrapper_dir(), session_id, frame)
}

fn write_via_wrapper_in(
    dir: &Path,
    session_id: &str,
    frame: &serde_json::Value,
) -> anyhow::Result<()> {
    let path = wrapper_socket_in(dir, session_id)
        .ok_or_else(|| anyhow::anyhow!("this session is not running under the wrapper"))?;
    // Measured as sent, escaping and all: that is what the wrapper counts.
    let line = format!("{frame}\n");
    if line.len() > WRAPPER_LINE_MAX {
        return Err(TooLong { bytes: line.len() }.into());
    }
    // The wrapper answers once it has written into the child's stdin, so unlike the
    // peer socket this is a real delivery receipt rather than a successful write.
    #[cfg(unix)]
    let reply = {
        let addr = std::os::unix::net::SocketAddr::from_pathname(&path)?;
        let mut s = UnixStream::connect_addr(&addr)?;
        s.set_write_timeout(Some(CONNECT_TIMEOUT))?;
        s.set_read_timeout(Some(CONNECT_TIMEOUT))?;
        s.write_all(line.as_bytes())?;
        s.flush()?;
        let mut reply = String::new();
        use std::io::Read;
        let _ = s.take(256).read_to_string(&mut reply);
        reply
    };
    #[cfg(windows)]
    let reply = pipe::exchange(&path.to_string_lossy(), line.as_bytes(), CONNECT_TIMEOUT)?;
    if reply.contains("\"ok\":true") {
        Ok(())
    } else {
        Err(anyhow::anyhow!("the wrapper did not accept the message"))
    }
}

/// Every tool-permission request a wrapped session has open, as the wrapper recorded
/// them — a Bash approval, an edit, an AskUserQuestion, a plan.
///
/// Written by the wrapper, which sees the control traffic between the CLI and whatever
/// hosts it. All of them, not only the last line: a session can have several open at
/// once (parallel tool calls, a subagent's prompt beside the main thread's), and the
/// file's order is not the order they arrived in — wrappers before the fix kept them
/// sorted by request id. Which is newest is the caller's to work out, from when it first
/// saw each (`live`'s first-seen memo); nothing here may pretend to know.
///
/// A prompt recorded by a wrapper that is gone is not pending, it is history, so the
/// caller asks `wrapper_alive` first — `live` does, once a second, and reads this only
/// when the wrapper answered. Only `can_use_tool` requests with an id: they are what the
/// file is for, and nothing without an id could be answered anyway.
pub fn read_pending_in(dir: &Path, session_id: &str) -> Vec<serde_json::Value> {
    if session_id.is_empty()
        || !session_id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
        return Vec::new();
    }
    let Ok(body) = std::fs::read_to_string(dir.join(format!("{session_id}.pending"))) else {
        return Vec::new();
    };
    body.lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter(|v| v["request"]["subtype"] == "can_use_tool" && v["request_id"].is_string())
        .collect()
}

/// Let a pending tool-permission request through — what the editor does when someone
/// picks an option and hits Submit.
///
/// An AskUserQuestion is a permission request whose answer rides in `updatedInput`:
/// allowing the tool with an `answers` map is what turns the prompt into a tool result.
/// For every other tool the input goes back untouched, which is plain approval.
///
/// `updates` carries a standing permission change, so the same tool is not asked about
/// again — but only ever one the prompt itself offered in its `permission_suggestions`.
/// The editor filters anything else out ("Dropping permission update(s) the prompt did
/// not offer"), so inventing one here would be silently discarded at best.
pub fn allow_prompt_in(
    dir: &Path,
    session_id: &str,
    request_id: &str,
    updated_input: serde_json::Value,
    tool: &str,
    updates: Option<serde_json::Value>,
) -> anyhow::Result<()> {
    let mut response = serde_json::json!({
        "behavior": "allow",
        "updatedInput": updated_input,
        "tool_name": tool,
    });
    if let Some(updates) = updates {
        response["updatedPermissions"] = serde_json::json!([updates]);
    }
    write_via_wrapper_in(
        dir,
        session_id,
        &serde_json::json!({
            "type": "control_response",
            "response": {
                "subtype": "success",
                "request_id": request_id,
                "response": response,
            },
        }),
    )
}

/// Refuse a pending tool-permission request. The message is what the model is told, so
/// it reads as the reason a person declined rather than as a failure.
pub fn deny_prompt_in(
    dir: &Path,
    session_id: &str,
    request_id: &str,
    message: &str,
) -> anyhow::Result<()> {
    write_via_wrapper_in(
        dir,
        session_id,
        &serde_json::json!({
            "type": "control_response",
            "response": {
                "subtype": "success",
                "request_id": request_id,
                "response": { "behavior": "deny", "message": message },
            },
        }),
    )
}

/// Switch the session's permission mode — what shift+tab cycles in the terminal. A
/// `set_permission_mode` control request, answered on the control channel like
/// `set_model`, because it can be refused: `bypassPermissions` needs the session to
/// have been launched with `--dangerously-skip-permissions`, and `auto` can be gated
/// too. Returns the request id to look for.
pub fn set_mode_via_wrapper(session_id: &str, mode: &str) -> anyhow::Result<String> {
    let request_id = uuid_v4();
    write_via_wrapper(
        session_id,
        &serde_json::json!({
            "type": "control_request",
            "request_id": request_id,
            "request": { "subtype": "set_permission_mode", "mode": mode },
        }),
    )?;
    Ok(request_id)
}

#[cfg(windows)]
pub fn send(_peer: &Peer, _text: &str) -> anyhow::Result<String> {
    anyhow::bail!("Claude Code's peer sockets do not exist on Windows; only a wrapped session can take a message")
}

/// Push a user message into a live session's input queue.
#[cfg(unix)]
pub fn send(peer: &Peer, text: &str) -> anyhow::Result<String> {
    let raw = std::fs::read_to_string(key_path(&peer.socket.to_string_lossy()))
        .map_err(|e| anyhow::anyhow!("no peer key for pid {}: {e}", peer.pid))?;
    let key: KeyFile = serde_json::from_str(&raw)?;

    let addr = std::os::unix::net::SocketAddr::from_pathname(&peer.socket)?;
    let mut s = UnixStream::connect_addr(&addr)?;
    s.set_write_timeout(Some(CONNECT_TIMEOUT))?;

    let msg_id = uuid_v4();
    let auth = serde_json::json!({ "type": "auth", "token": key.peer_token });
    let frame = serde_json::json!({
        "msgV": 1,
        "msg_id": msg_id,
        "type": "user",
        "message": { "role": "user", "content": text },
        "priority": "next",
    });
    s.write_all(format!("{auth}\n{frame}\n").as_bytes())?;
    s.flush()?;
    // The receiver acknowledges by acting, not by replying on this socket, so the write
    // completing is all the confirmation the protocol offers.
    Ok(msg_id)
}

/// The wrapper's control channel on Windows: a named pipe per session, under a name the
/// wrapper and this module both derive (`wrapper/src/main.rs`, `pipe_name`), carrying the
/// same frames and receipts as the Unix socket. See `wrapper/src/windows.rs` for the
/// server side, and docs/hosting-machine-agent.md.
#[cfg(windows)]
mod pipe {
    use std::io::{Read, Write};
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;
    use std::time::{Duration, Instant};

    use windows_sys::Win32::Foundation::{
        CloseHandle, GetLastError, ERROR_ACCESS_DENIED, ERROR_PIPE_BUSY, ERROR_SEM_TIMEOUT, HANDLE,
    };
    use windows_sys::Win32::Security::{GetLengthSid, GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER};
    use windows_sys::Win32::System::Pipes::{GetNamedPipeServerProcessId, WaitNamedPipeW};
    use windows_sys::Win32::System::Threading::{OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION};
    use windows_sys::Win32::System::WindowsProgramming::GetUserNameW;

    /// `SecurityIdentification << 16`: the server may learn who connected, and may not act
    /// as us — a pipe whose name someone else took cannot borrow this process's identity.
    const SECURITY_IDENTIFICATION: u32 = 1 << 16;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// `\\.\pipe\agent-router-<user>-<session>`, the user part cut down exactly as the
    /// wrapper does it (`pipe_name_part`): `[A-Za-z0-9_-]`, anything else `_`, at most 64
    /// characters. Without a user name the wrapper publishes no pipe, and an empty part
    /// names one nobody holds.
    pub fn name(session_id: &str) -> String {
        static USER: std::sync::OnceLock<String> = std::sync::OnceLock::new();
        let user = USER.get_or_init(|| {
            // UNLEN, the longest user name, and its terminating nul.
            let mut buf = [0u16; 257];
            let mut len = buf.len() as u32;
            // SAFETY: a buffer of `len` characters.
            if unsafe { GetUserNameW(buf.as_mut_ptr(), &mut len) } == 0 {
                return String::new();
            }
            let raw = buf.split(|&c| c == 0).next().unwrap_or_default();
            String::from_utf16_lossy(raw)
                .chars()
                .map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' })
                .take(64)
                .collect()
        });
        format!(r"\\.\pipe\agent-router-{user}-{session_id}")
    }

    /// Whether a wrapper holds the name now. Waiting a millisecond answers at once: true
    /// when an instance listens, a timeout when every instance is busy (still alive), and
    /// "not found" when no wrapper holds it.
    pub fn present(name: &str) -> bool {
        let w = wide(name);
        // SAFETY: a NUL-terminated name.
        if unsafe { WaitNamedPipeW(w.as_ptr(), 1) } != 0 {
            return true;
        }
        (unsafe { GetLastError() }) == ERROR_SEM_TIMEOUT
    }

    /// One frame, one receipt. On a worker thread, because a pipe handle has no read or
    /// write timeout, and an exchange must never hold up a request for long.
    pub fn exchange(name: &str, line: &[u8], budget: Duration) -> anyhow::Result<String> {
        let (name, line) = (name.to_string(), line.to_vec());
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(exchange_blocking(&name, &line, budget));
        });
        rx.recv_timeout(budget + Duration::from_secs(1))
            .map_err(|_| anyhow::anyhow!("the wrapper did not answer in time"))?
    }

    fn exchange_blocking(name: &str, line: &[u8], budget: Duration) -> anyhow::Result<String> {
        let deadline = Instant::now() + budget;
        let mut file = loop {
            let opened = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .security_qos_flags(SECURITY_IDENTIFICATION)
                .open(name);
            match opened {
                Ok(f) => break f,
                // The instant between one client and the wrapper's next instance.
                Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY as i32) && Instant::now() < deadline => {
                    // SAFETY: a NUL-terminated name.
                    unsafe { WaitNamedPipeW(wide(name).as_ptr(), 250) };
                }
                // The wrapper's pipe admits its own user only, and an elevated editor's
                // admits only elevated processes of that user.
                Err(e) if e.raw_os_error() == Some(ERROR_ACCESS_DENIED as i32) => anyhow::bail!(
                    "that session's control pipe does not admit this agent: it belongs to another \
                     user, or the editor runs as administrator and the agent does not"
                ),
                Err(e) => return Err(e.into()),
            }
        };
        // The name is machine-wide: a wrapper that died and lapsed can have its name taken
        // by another account's process, which must never receive what someone typed.
        anyhow::ensure!(
            served_by_this_user(file.as_raw_handle() as HANDLE),
            "that session's control pipe is not served by this user; refusing to write to it"
        );
        file.write_all(line)?;
        let mut reply = String::new();
        let _ = (&mut file).take(256).read_to_string(&mut reply);
        Ok(reply)
    }

    fn served_by_this_user(pipe: HANDLE) -> bool {
        let mut pid = 0u32;
        // SAFETY: a valid client handle to a named pipe.
        if unsafe { GetNamedPipeServerProcessId(pipe, &mut pid) } == 0 {
            return false;
        }
        match (user_sid(pid), user_sid(std::process::id())) {
            (Some(theirs), Some(ours)) => theirs == ours,
            _ => false,
        }
    }

    /// The bytes of the SID a process runs as.
    fn user_sid(pid: u32) -> Option<Vec<u8>> {
        // SAFETY: handles opened here are closed here; the token buffer is u64-aligned and
        // as long as GetTokenInformation asked for.
        unsafe {
            let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if process.is_null() {
                return None;
            }
            let mut token: HANDLE = std::ptr::null_mut();
            let opened = OpenProcessToken(process, TOKEN_QUERY, &mut token) != 0;
            CloseHandle(process);
            if !opened {
                return None;
            }
            let mut len = 0u32;
            GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut len);
            let mut buf = vec![0u64; (len as usize).div_ceil(8).max(1)];
            let got = GetTokenInformation(token, TokenUser, buf.as_mut_ptr().cast(), len, &mut len) != 0;
            CloseHandle(token);
            if !got {
                return None;
            }
            let user = &*(buf.as_ptr() as *const TOKEN_USER);
            let sid = user.User.Sid;
            let sid_len = GetLengthSid(sid) as usize;
            Some(std::slice::from_raw_parts(sid as *const u8, sid_len).to_vec())
        }
    }
}

/// A v4 UUID without pulling in a crate for it.
fn uuid_v4() -> String {
    let mut b = [0u8; 16];
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut x = (seed as u64) ^ (std::process::id() as u64) << 32 ^ 0x9e37_79b9_7f4a_7c15;
    for chunk in b.chunks_mut(8) {
        // xorshift64*, plenty for a message id
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        let v = x.wrapping_mul(0x2545_F491_4F6C_DD1D);
        for (i, slot) in chunk.iter_mut().enumerate() {
            *slot = (v >> (i * 8)) as u8;
        }
    }
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: Vec<String> = b.iter().map(|x| format!("{x:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        h[0..4].concat(),
        h[4..6].concat(),
        h[6..8].concat(),
        h[8..10].concat(),
        h[10..16].concat()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_path_is_the_sha256_of_the_socket_path() {
        // Verified against a real published key file on this machine.
        let p = key_path("/tmp/cc-socks/26643.sock");
        let name = p.file_name().unwrap().to_string_lossy().into_owned();
        assert_eq!(
            name,
            "26643.500170752ab115d9ee6df309d9d49d7aafa493e56afb7849401611b7a469086d.key"
        );
    }

    #[test]
    fn parses_the_key_files_date_format() {
        // Ground truth: pid 26643 reported this, and its transcript opened 17s later.
        let ms = parse_proc_start("Thu Sep 10 07:35:03 2026").expect("parses");
        assert_eq!(ms, 1_789_025_703_000);
    }

    #[test]
    fn rejects_a_date_it_does_not_understand() {
        assert!(parse_proc_start("").is_none());
        assert!(parse_proc_start("2026-09-10T07:35:03Z").is_none());
    }

    /// A private stand-in for `/tmp/agent-router-sessions`, which live sessions are using.
    /// Short, because a unix socket's path has to fit in 104 bytes.
    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("arp-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    #[cfg(unix)]
    fn a_socket_nobody_listens_on_is_not_a_live_session() {
        let dir = scratch("sock");
        let id = "s-1";
        let path = dir.join(format!("{id}.sock"));
        let pending = dir.join(format!("{id}.pending"));
        assert!(!wrapper_alive_in(&dir, id), "no file, no session");

        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        std::fs::write(&pending, "{}\n").unwrap();
        assert!(wrapper_alive_in(&dir, id), "a listener is a session");
        assert!(pending.exists(), "a live session's prompts are left alone");

        // What a killed wrapper leaves behind: the file, and nothing on the other end.
        drop(listener);
        assert!(path.exists(), "dropping a listener does not unlink its file");
        assert!(!wrapper_alive_in(&dir, id), "a file with nobody behind it is not a session");
        assert!(!path.exists(), "and the stale file is cleaned up");
        assert!(!pending.exists(), "with the prompts nobody can answer any more");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_recorded_answer_is_found_by_its_request_id() {
        let dir = scratch("ctl");
        let id = "s-ctl";
        std::fs::write(dir.join(format!("{id}.ctl")), concat!(
            r#"{"type":"control_request","request_id":"r-1","request":{"subtype":"set_model","model":"opus"}}"#, "\n",
            r#"{"type":"control_response","response":{"subtype":"error","request_id":"r-0","error":"older"}}"#, "\n",
            r#"{"type":"control_response","response":{"subtype":"success","request_id":"r-1","response":{"notices":["Switched to opus"]}}}"#, "\n",
        )).unwrap();
        let got = control_response_in(&dir, id, "r-1").expect("the answer is there");
        assert_eq!(got["subtype"], "success");
        assert_eq!(got["response"]["notices"][0], "Switched to opus");
        assert!(control_response_in(&dir, id, "r-2").is_none(), "no answer yet");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn every_open_prompt_is_read_not_only_the_last_line() {
        let dir = scratch("pend");
        let id = "s-pend";
        // An old wrapper's order: by request id, so the newest is not last.
        std::fs::write(dir.join(format!("{id}.pending")), concat!(
            r#"{"type":"control_request","request_id":"aa-newer","request":{"subtype":"can_use_tool","tool_name":"Bash","input":{"command":"ls"},"tool_use_id":"toolu_2"}}"#, "\n",
            r#"{"type":"control_request","request_id":"zz-older","request":{"subtype":"can_use_tool","tool_name":"Edit","input":{"file_path":"/x"},"tool_use_id":"toolu_1"}}"#, "\n",
            r#"{"type":"control_request","request_id":"h-1","request":{"subtype":"hook_callback"}}"#, "\n",
            "{\"type\":\"control_request\",\"request_id\":\"torn", "\n",
        )).unwrap();
        let got: Vec<String> = read_pending_in(&dir, id)
            .iter()
            .map(|v| v["request_id"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(got, ["aa-newer", "zz-older"], "both prompts, in the file's order");
        assert!(read_pending_in(&dir, "no-such").is_empty());
        assert!(read_pending_in(&dir, "../etc/passwd").is_empty(), "an id that is not one");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Every deployed wrapper reads a frame in 8 KiB pieces and stops once it holds more
    /// than a mebibyte, newline or not — then writes what it has into the CLI anyway,
    /// newline added, and the CLI exits on the line it cannot parse. A frame up to the
    /// limit arrives whole; anything longer is refused here, and never connects.
    #[test]
    #[cfg(unix)]
    fn a_frame_the_wrapper_would_cut_short_is_never_sent() {
        use std::io::Read;
        let dir = scratch("big");
        let id = "s-big";
        let listener =
            std::os::unix::net::UnixListener::bind(dir.join(format!("{id}.sock"))).unwrap();
        let wrapper = std::thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            let mut body = Vec::new();
            let mut buf = [0u8; 8192];
            while !body.contains(&b'\n') {
                match conn.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => body.extend_from_slice(&buf[..n]),
                }
            }
            conn.write_all(b"{\"ok\":true}\n").unwrap();
            (listener, body.len())
        });
        // Exactly at the limit: the typed text plus the frame around it and its newline.
        let around = format!("{}\n", user_frame("")).len();
        let text = "x".repeat(WRAPPER_LINE_MAX - around);
        send_via_wrapper_in(&dir, id, &text).expect("a frame at the limit is delivered");
        let (listener, got) = wrapper.join().unwrap();
        assert_eq!(got, WRAPPER_LINE_MAX, "whole, newline included");

        // One byte more is refused, and nothing so much as connects.
        listener.set_nonblocking(true).unwrap();
        let err = send_via_wrapper_in(&dir, id, &format!("{text}x")).unwrap_err();
        let too_long = err.downcast_ref::<TooLong>().expect("refused as too long");
        assert_eq!(too_long.bytes, WRAPPER_LINE_MAX + 1);
        assert_eq!(
            listener.accept().map(|_| ()).unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock,
            "the socket was never touched"
        );
        // What counts is the frame as sent: escaping can make it far longer than the text.
        let quotes = "\"".repeat(WRAPPER_LINE_MAX / 2);
        assert!(send_via_wrapper_in(&dir, id, &quotes)
            .unwrap_err()
            .is::<TooLong>());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn uuid_has_the_right_shape_and_version() {
        let u = uuid_v4();
        let parts: Vec<&str> = u.split('-').collect();
        assert_eq!(parts.iter().map(|p| p.len()).collect::<Vec<_>>(), vec![8, 4, 4, 4, 12]);
        assert_eq!(&u[14..15], "4", "version nibble");
        assert!(u != uuid_v4(), "ids differ between calls");
    }
}
