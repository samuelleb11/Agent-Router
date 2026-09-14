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
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use serde::Deserialize;
use sha2::{Digest, Sha256};

const SOCK_DIR: &str = "/tmp/cc-socks";
/// Where a wrapped session offers its control socket (see `wrapper/`).
const WRAPPER_DIR: &str = "/tmp/agent-router-sessions";
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
    let home = std::env::var("HOME").unwrap_or_default();
    PathBuf::from(home)
        .join(".claude")
        .join("sessions")
        .join(format!("{pid}.{hex}.key"))
}

/// `Thu Sep 10 07:35:03 2026` — the one format the key file uses.
fn parse_proc_start(s: &str) -> Option<i64> {
    let f: Vec<&str> = s.split_whitespace().collect();
    if f.len() < 5 {
        return None;
    }
    let mon = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ]
    .iter()
    .position(|m| *m == f[1])? as i64
        + 1;
    let day: i64 = f[2].parse().ok()?;
    let t: Vec<i64> = f[3].split(':').filter_map(|x| x.parse().ok()).collect();
    let year: i64 = f[4].parse().ok()?;
    if t.len() != 3 {
        return None;
    }
    let y = if mon <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (mon + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some((days * 86_400 + t[0] * 3600 + t[1] * 60 + t[2]) * 1000)
}

fn proc_cwd(pid: u32) -> Option<String> {
    let out = Command::new("/usr/sbin/lsof")
        .args(["-a", "-d", "cwd", "-p", &pid.to_string(), "-Fn"])
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .find_map(|l| l.strip_prefix('n').map(str::to_owned))
}

fn proc_resumed(pid: u32) -> Option<String> {
    let out = Command::new("/bin/ps")
        .args(["-o", "command=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .find_map(|a| a.strip_prefix("--resume=").map(str::to_owned))
}

/// Every session currently listening for peer input.
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
    if session_id.is_empty()
        || !session_id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
        return None;
    }
    let p = PathBuf::from(WRAPPER_DIR).join(format!("{session_id}.sock"));
    p.exists().then_some(p)
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
    let Some(path) = wrapper_socket(session_id) else {
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
            let _ = std::fs::remove_file(
                PathBuf::from(WRAPPER_DIR).join(format!("{session_id}.pending")),
            );
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
    write_via_wrapper(
        session_id,
        &serde_json::json!({
            "type": "user",
            "message": { "role": "user", "content": [{ "type": "text", "text": text }] },
        }),
    )
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
    let path = PathBuf::from(WRAPPER_DIR).join(format!("{session_id}.ctl"));
    let text = std::fs::read_to_string(path).ok()?;
    text.lines()
        .rev()
        .filter(|l| l.contains("\"type\":\"control_response\"") && l.contains(request_id))
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .find(|v| v["response"]["request_id"] == request_id)
        .map(|v| v["response"].clone())
}

/// One newline-delimited frame into a wrapped session's stdin.
fn write_via_wrapper(session_id: &str, frame: &serde_json::Value) -> anyhow::Result<()> {
    let path = wrapper_socket(session_id)
        .ok_or_else(|| anyhow::anyhow!("this session is not running under the wrapper"))?;
    let addr = std::os::unix::net::SocketAddr::from_pathname(&path)?;
    let mut s = UnixStream::connect_addr(&addr)?;
    s.set_write_timeout(Some(CONNECT_TIMEOUT))?;
    s.set_read_timeout(Some(CONNECT_TIMEOUT))?;

    s.write_all(format!("{frame}\n").as_bytes())?;
    s.flush()?;

    // The wrapper answers once it has written into the child's stdin, so unlike the
    // peer socket this is a real delivery receipt rather than a successful write.
    let mut reply = String::new();
    use std::io::Read;
    let _ = s.take(256).read_to_string(&mut reply);
    if reply.contains("\"ok\":true") {
        Ok(())
    } else {
        Err(anyhow::anyhow!("the wrapper did not accept the message"))
    }
}

/// The tool-permission request a wrapped session is currently blocked on, if any.
///
/// Written by the wrapper, which sees the control traffic between the CLI and whatever
/// hosts it. Only the newest is returned: a session waits on one prompt at a time.
pub fn pending_prompt(session_id: &str, tool: &str) -> Option<serde_json::Value> {
    // A prompt recorded by a wrapper that is gone is not pending; it is history.
    if !wrapper_alive(session_id) {
        return None;
    }
    let path = PathBuf::from(WRAPPER_DIR).join(format!("{session_id}.pending"));
    let body = std::fs::read_to_string(path).ok()?;
    body.lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter(|v| v["request"]["tool_name"] == tool)
        .next_back()
}

/// Answer a pending tool-permission request — what the editor does when someone picks
/// an option and hits Submit.
///
/// An AskUserQuestion is a permission request whose answer rides in `updatedInput`:
/// allowing the tool with an `answers` map is what turns the prompt into a tool result.
pub fn answer_prompt(
    session_id: &str,
    request_id: &str,
    updated_input: serde_json::Value,
    tool: &str,
) -> anyhow::Result<()> {
    write_via_wrapper(
        session_id,
        &serde_json::json!({
            "type": "control_response",
            "response": {
                "subtype": "success",
                "request_id": request_id,
                "response": {
                    "behavior": "allow",
                    "updatedInput": updated_input,
                    "tool_name": tool,
                },
            },
        }),
    )
}

/// Push a user message into a live session's input queue.
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

    #[test]
    fn a_socket_nobody_listens_on_is_not_a_live_session() {
        let id = format!("test-{}", std::process::id());
        let dir = PathBuf::from(WRAPPER_DIR);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{id}.sock"));
        let _ = std::fs::remove_file(&path);
        assert!(!wrapper_alive(&id), "no file, no session");

        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        assert!(wrapper_alive(&id), "a listener is a session");

        // What a killed wrapper leaves behind: the file, and nothing on the other end.
        drop(listener);
        assert!(path.exists(), "dropping a listener does not unlink its file");
        assert!(!wrapper_alive(&id), "a file with nobody behind it is not a session");
        assert!(!path.exists(), "and the stale file is cleaned up");
    }

    #[test]
    fn a_recorded_answer_is_found_by_its_request_id() {
        let id = format!("test-ctl-{}", std::process::id());
        let dir = PathBuf::from(WRAPPER_DIR);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{id}.ctl"));
        std::fs::write(&path, concat!(
            r#"{"type":"control_request","request_id":"r-1","request":{"subtype":"set_model","model":"opus"}}"#, "\n",
            r#"{"type":"control_response","response":{"subtype":"error","request_id":"r-0","error":"older"}}"#, "\n",
            r#"{"type":"control_response","response":{"subtype":"success","request_id":"r-1","response":{"notices":["Switched to opus"]}}}"#, "\n",
        )).unwrap();
        let got = control_response(&id, "r-1").expect("the answer is there");
        assert_eq!(got["subtype"], "success");
        assert_eq!(got["response"]["notices"][0], "Switched to opus");
        assert!(control_response(&id, "r-2").is_none(), "no answer yet");
        let _ = std::fs::remove_file(&path);
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
