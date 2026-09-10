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
//! Sending the content bare — without the `<cross-session-message>` wrapper that the
//! agent-to-agent path uses — makes it arrive as an ordinary user turn, which is what a
//! message typed on a phone should look like.
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
    fn uuid_has_the_right_shape_and_version() {
        let u = uuid_v4();
        let parts: Vec<&str> = u.split('-').collect();
        assert_eq!(parts.iter().map(|p| p.len()).collect::<Vec<_>>(), vec![8, 4, 4, 4, 12]);
        assert_eq!(&u[14..15], "4", "version nibble");
        assert!(u != uuid_v4(), "ids differ between calls");
    }
}
