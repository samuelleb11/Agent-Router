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
//! cross-session frame — an ordinary user turn.
//!
//! # This process is in the critical path of every session
//!
//! If it dies, the session dies with it. So:
//!
//!   * The relay is a byte pump. It never parses anything it must understand, never
//!     transforms, and never buffers more than one read.
//!   * Every added feature — the sniffer, the control socket — is allowed to fail. Any
//!     error there degrades this to a plain pass-through rather than taking the session
//!     with it.
//!   * No dependencies. std only, so there is nothing to break on an update.
//!
//! Install: move the real binary aside to `claude.real`, put this in its place.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Where a wrapped session offers its control socket. Named by session id once known,
/// so the router can address a session by the same id its transcript carries.
const CONTROL_DIR: &str = "/tmp/agent-router-sessions";

/// The real binary lives beside us under this name.
const REAL_SUFFIX: &str = ".real";

fn main() {
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("claude"));
    let real = {
        let mut p = exe.clone().into_os_string();
        p.push(REAL_SUFFIX);
        PathBuf::from(p)
    };

    // Nothing to wrap: exec the real thing if we can find it, or fail exactly as the
    // missing binary would have.
    if !real.exists() {
        eprintln!("claude-wrapper: {} not found; is the shim half-installed?", real.display());
        std::process::exit(127);
    }

    let args: Vec<String> = std::env::args().skip(1).collect();
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

    let child_stdin = child.stdin.take().expect("piped");
    let child_stdout = child.stdout.take().expect("piped");

    // One writer, shared by the extension and by the control socket. Writes are whole
    // buffers under the lock, so an injected turn can never interleave into the middle
    // of a line the extension is sending.
    let sink = Arc::new(Mutex::new(child_stdin));
    // Shared, because answering a prompt has to speak upstream as well as down.
    let up = Arc::new(Mutex::new(std::io::stdout()));

    // extension -> child. This is the direction that must never stall.
    {
        let sink = Arc::clone(&sink);
        std::thread::spawn(move || {
            let mut buf = [0u8; 64 * 1024];
            let mut input = std::io::stdin();
            loop {
                match input.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let Ok(mut w) = sink.lock() else { break };
                        if w.write_all(&buf[..n]).is_err() || w.flush().is_err() {
                            break;
                        }
                    }
                }
            }
            // EOF upstream means the extension is done; dropping the sink closes the
            // child's stdin so it can shut down normally.
        });
    }

    // child -> extension, with the session id read out of the stream in passing.
    let session_id = Arc::new(Mutex::new(None::<String>));
    let found = Arc::new(AtomicBool::new(false));
    let relay = {
        let session_id = Arc::clone(&session_id);
        let found = Arc::clone(&found);
        let sink = Arc::clone(&sink);
        let up = Arc::clone(&up);
        std::thread::spawn(move || {
            let mut src = child_stdout;
            let mut buf = [0u8; 64 * 1024];
            let mut scan: Vec<u8> = Vec::new();
            let mut ctl: Vec<u8> = Vec::new();
            let mut pending: BTreeMap<String, String> = BTreeMap::new();
            loop {
                let n = match src.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                // Downstream first, always. The sniffer only ever sees a copy.
                {
                    let Ok(mut out) = up.lock() else { break };
                    if out.write_all(&buf[..n]).is_err() || out.flush().is_err() {
                        break;
                    }
                }
                // Control traffic, recorded so the router can see what the session is
                // blocked on. A prompt is a control_request the editor answers with a
                // matching control_response; without the request_id nothing else can
                // answer it. Best-effort, like everything else here.
                note_control(&session_id, &buf[..n], &mut ctl, &mut pending);

                if !found.load(Ordering::Relaxed) {
                    scan.extend_from_slice(&buf[..n]);
                    if let Some(id) = sniff_session_id(&scan) {
                        if let Ok(mut slot) = session_id.lock() {
                            *slot = Some(id.clone());
                        }
                        found.store(true, Ordering::Relaxed);
                        scan = Vec::new();
                        // Publishing is allowed to fail; a session without a control
                        // socket is simply one the router cannot type into.
                        let sink = Arc::clone(&sink);
                        let up = Arc::clone(&up);
                        std::thread::spawn(move || serve_control(id, sink, up));
                    } else if scan.len() > 256 * 1024 {
                        // The id arrives in the first message or not at all.
                        found.store(true, Ordering::Relaxed);
                        scan = Vec::new();
                    }
                }
            }
        })
    };

    let status = child.wait();
    let _ = relay.join();
    if let Ok(Some(id)) = session_id.lock().map(|g| g.clone()) {
        let _ = std::fs::remove_file(control_path(&id));
        let _ = std::fs::remove_file(PathBuf::from(CONTROL_DIR).join(format!("{id}.ctl")));
        let _ = std::fs::remove_file(PathBuf::from(CONTROL_DIR).join(format!("{id}.pending")));
    }
    std::process::exit(status.ok().and_then(|s| s.code()).unwrap_or(1));
}

/// Append any `control_request` line to `<session>.ctl` beside the socket.
///
/// Line-oriented and bounded: the tail is kept so a long session cannot grow the file,
/// and a partial line is carried to the next read rather than parsed.
fn note_control(
    session_id: &Mutex<Option<String>>,
    chunk: &[u8],
    carry: &mut Vec<u8>,
    pending: &mut BTreeMap<String, String>,
) {
    if !chunk.contains(&b'\n') && carry.len() > 256 * 1024 {
        carry.clear(); // a stream with no newlines is not what we are looking for
        return;
    }
    carry.extend_from_slice(chunk);
    let Some(cut) = carry.iter().rposition(|&b| b == b'\n') else {
        return;
    };
    let complete: Vec<u8> = carry.drain(..=cut).collect();
    let Ok(guard) = session_id.lock() else { return };
    let Some(id) = guard.as_deref() else { return };
    let mut hits = String::new();
    for line in complete.split(|&b| b == b'\n') {
        if line.is_empty() {
            continue;
        }
        let is_control = find(line, b"\"type\":\"control_request\"").is_some()
            || find(line, b"\"type\":\"control_response\"").is_some();
        if !is_control {
            continue;
        }
        let Ok(text) = std::str::from_utf8(line) else {
            continue;
        };
        hits.push_str(text);
        hits.push('\n');

        // Track what is still waiting for an answer. The .ctl tail can lose a request
        // under a burst of MCP traffic, and a request nobody can find is a session
        // nobody can unblock.
        if let Some(id) = field(text, "\"request_id\":\"") {
            if text.contains(r#""type":"control_request""#)
                && text.contains(r#""subtype":"can_use_tool""#)
            {
                pending.insert(id, text.to_string());
            } else if text.contains(r#""type":"control_response""#) {
                pending.remove(&id);
            }
        }
    }
    write_pending(id, pending);
    if hits.is_empty() {
        return;
    }
    let path = PathBuf::from(CONTROL_DIR).join(format!("{id}.ctl"));
    let _ = std::fs::create_dir_all(CONTROL_DIR);
    // Keep only the tail: what is pending now is what matters.
    let mut existing = std::fs::read_to_string(&path).unwrap_or_default();
    existing.push_str(&hits);
    let kept: Vec<&str> = existing.lines().rev().take(40).collect();
    let out: String = kept.into_iter().rev().collect::<Vec<_>>().join("\n") + "\n";
    let _ = std::fs::write(&path, out);
}

/// Unanswered tool-permission requests, so the router can find what a session is
/// blocked on without racing the .ctl tail.
fn write_pending(session_id: &str, pending: &BTreeMap<String, String>) {
    let path = PathBuf::from(CONTROL_DIR).join(format!("{session_id}.pending"));
    if pending.is_empty() {
        let _ = std::fs::remove_file(&path);
        return;
    }
    let body: String = pending.values().cloned().collect::<Vec<_>>().join("\n") + "\n";
    let _ = std::fs::write(&path, body);
}

/// The value of a `"key":"…"` pair, without parsing the whole line.
fn field(line: &str, key: &str) -> Option<String> {
    let start = line.find(key)? + key.len();
    let rest = &line[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

fn control_path(session_id: &str) -> PathBuf {
    PathBuf::from(CONTROL_DIR).join(format!("{session_id}.sock"))
}

/// Pull `"session_id":"…"` out of the stream without parsing it.
///
/// A real JSON parse would mean buffering whole messages and having an opinion about
/// their shape, which is exactly the coupling this shim must not have.
fn sniff_session_id(buf: &[u8]) -> Option<String> {
    const KEY: &[u8] = b"\"session_id\":\"";
    let start = buf.windows(KEY.len()).position(|w| w == KEY)? + KEY.len();
    let rest = &buf[start..];
    let end = rest.iter().position(|&b| b == b'"')?;
    let id = std::str::from_utf8(&rest[..end]).ok()?;
    let ok = !id.is_empty()
        && id.len() <= 64
        && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    ok.then(|| id.to_string())
}

/// Accept injected user turns and write them into the child's stdin.
///
/// Every failure here is swallowed. This is an addition to a session that works fine
/// without it, and it must never be the reason one dies.
fn serve_control(
    session_id: String,
    sink: Arc<Mutex<std::process::ChildStdin>>,
    up: Arc<Mutex<std::io::Stdout>>,
) {
    if std::fs::create_dir_all(CONTROL_DIR).is_err() {
        return;
    }
    // Re-bind if the socket file ever goes away. It lives in /tmp, so a cleaner, a
    // reboot-adjacent sweep or a stray rm can unlink it while this process still holds
    // the fd — leaving a session that looks unwrapped forever despite being wrapped.
    loop {
        serve_once(&session_id, &sink, &up);
        std::thread::sleep(std::time::Duration::from_secs(3));
    }
}

fn serve_once(
    session_id: &str,
    sink: &Arc<Mutex<std::process::ChildStdin>>,
    up: &Arc<Mutex<std::io::Stdout>>,
) {
    let path = control_path(session_id);
    let _ = std::fs::remove_file(&path);
    let Ok(listener) = UnixListener::bind(&path) else {
        return;
    };
    // Non-blocking so the accept loop can notice the file vanishing and rebuild it.
    if listener.set_nonblocking(true).is_err() {
        return;
    }
    // Owner-only: this socket types into someone's editor.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }

    loop {
        match listener.accept() {
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if !path.exists() {
                    return; // rebind on the next pass
                }
                std::thread::sleep(std::time::Duration::from_millis(200));
                continue;
            }
            Err(_) => return,
            Ok((conn, _)) => {
                let mut conn = conn;
                let _ = conn.set_nonblocking(false);
                let sink = Arc::clone(sink);
                let up = Arc::clone(up);
                std::thread::spawn(move || {
                    let mut body = Vec::new();
            // A control message is one line; cap it so a stuck writer cannot grow us.
            let mut buf = [0u8; 8192];
            loop {
                match conn.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        body.extend_from_slice(&buf[..n]);
                        if body.len() > 1024 * 1024 || body.contains(&b'\n') {
                            break;
                        }
                    }
                }
            }
                    let delivered = write_line(&sink, &body);
                    // The host drew its own prompt when it saw the request and waits on
                    // its own UI; it cannot learn we answered. A cancel echoing the same
                    // id is how a request is withdrawn, so the picker comes down instead
                    // of hanging there answered-but-open.
                    if delivered {
                        if let Some(id) = answered_request_id(&body) {
                            let frame = format!(
                                "{{\"type\":\"control_cancel_request\",\"request_id\":\"{id}\"}}\n"
                            );
                            if let Ok(mut w) = up.lock() {
                                let _ = w.write_all(frame.as_bytes());
                                let _ = w.flush();
                            }
                        }
                    }
                    let _ = conn.write_all(if delivered { b"{\"ok\":true}\n" } else { b"{\"ok\":false}\n" });
                    let _ = conn.shutdown(Shutdown::Both);
                });
            }
        }
    }
}

/// The request id of an injected `control_response`, if that is what this was.
fn answered_request_id(body: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(body).ok()?;
    if !text.contains(r#""type":"control_response""#) {
        return None;
    }
    field(text, "\"request_id\":\"")
}

/// Write one newline-terminated message into the child's stdin, atomically.
fn write_line(sink: &Mutex<std::process::ChildStdin>, body: &[u8]) -> bool {
    let trimmed = body.strip_suffix(b"\n").unwrap_or(body);
    if trimmed.is_empty() {
        return false;
    }
    let mut line = trimmed.to_vec();
    line.push(b'\n');
    let Ok(mut w) = sink.lock() else { return false };
    w.write_all(&line).is_ok() && w.flush().is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(
            control_path("abc-123"),
            PathBuf::from("/tmp/agent-router-sessions/abc-123.sock")
        );
    }
}
