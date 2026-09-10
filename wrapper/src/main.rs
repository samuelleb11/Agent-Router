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
        std::thread::spawn(move || {
            let mut out = std::io::stdout();
            let mut src = child_stdout;
            let mut buf = [0u8; 64 * 1024];
            let mut scan: Vec<u8> = Vec::new();
            loop {
                let n = match src.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                // Downstream first, always. The sniffer only ever sees a copy.
                if out.write_all(&buf[..n]).is_err() || out.flush().is_err() {
                    break;
                }
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
                        std::thread::spawn(move || serve_control(id, sink));
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
    }
    std::process::exit(status.ok().and_then(|s| s.code()).unwrap_or(1));
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
fn serve_control(session_id: String, sink: Arc<Mutex<std::process::ChildStdin>>) {
    if std::fs::create_dir_all(CONTROL_DIR).is_err() {
        return;
    }
    let path = control_path(&session_id);
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

    for conn in listener.incoming() {
        let Ok(mut conn) = conn else { continue };
        let sink = Arc::clone(&sink);
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
            let _ = conn.write_all(if delivered { b"{\"ok\":true}\n" } else { b"{\"ok\":false}\n" });
            let _ = conn.shutdown(Shutdown::Both);
        });
    }
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
    fn control_path_is_named_by_session() {
        assert_eq!(
            control_path("abc-123"),
            PathBuf::from("/tmp/agent-router-sessions/abc-123.sock")
        );
    }
}
