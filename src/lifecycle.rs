//! Zero-downtime restarts.
//!
//! Restarting a proxy that agents are actively talking through normally means a
//! window where `connect()` fails and every in-flight stream is cut. Three pieces
//! remove that window:
//!
//! 1. **`SO_REUSEPORT`** — the incoming binary binds the port while the outgoing one
//!    is still serving on it, so the port is never unbound.
//! 2. **Handover** — the incoming binary asks `/_router/health` who is already there
//!    and sends that process a `SIGTERM`. A pid served by the live process beats a
//!    pidfile, which can outlive the process that wrote it.
//! 3. **Drain** — on `SIGTERM` the outgoing binary stops accepting, lets the requests
//!    and SSE streams it already holds run to completion, flushes its capture queue,
//!    and exits.
//!
//! Order matters: the health probe happens *before* the new bind, otherwise the new
//! process can answer its own probe and SIGTERM itself.

use std::net::SocketAddr;
use std::time::Duration;

use anyhow::{Context, Result};
use socket2::{Domain, Protocol, Socket, Type};

/// Who is currently serving this port, if anyone.
pub fn incumbent(bind: SocketAddr) -> Option<i32> {
    let pid = parse_pid(&probe_health(bind)?)?;
    // Never signal ourselves, however we came to answer our own probe.
    (pid != std::process::id() as i32).then_some(pid)
}

/// Who is serving, and which build they are: `(pid, version)`. A health body without a
/// version — a build older than this field — reports an empty string, which
/// [`is_newer`] treats as older than anything.
pub fn incumbent_version(bind: SocketAddr) -> Option<(i32, String)> {
    let body = probe_health(bind)?;
    let pid = parse_pid(&body)?;
    if pid == std::process::id() as i32 {
        return None;
    }
    Some((pid, parse_version(&body).unwrap_or_default()))
}

fn parse_version(body: &str) -> Option<String> {
    let after = body.split("\"version\":").nth(1)?;
    let rest = &after[after.find('"')? + 1..];
    Some(rest[..rest.find('"')?].to_string())
}

/// Whether `ours` is a newer build than `theirs`, by dotted number. Anything that does
/// not parse counts as older, so an incumbent of unknown vintage gets replaced.
pub fn is_newer(ours: &str, theirs: &str) -> bool {
    fn parts(v: &str) -> Vec<u64> {
        v.trim()
            .split(|c| c == '.' || c == '-' || c == '+')
            .take(3)
            .map(|p| p.parse().unwrap_or(0))
            .collect()
    }
    parts(ours) > parts(theirs)
}

/// Pull the pid out of a `/_router/health` response. Deliberately tolerant: the body
/// arrives with HTTP headers attached and is only ever produced by us.
fn parse_pid(body: &str) -> Option<i32> {
    body.split("\"pid\":")
        .nth(1)?
        .trim()
        .split(|c: char| !c.is_ascii_digit())
        .find(|s| !s.is_empty())?
        .parse::<i32>()
        .ok()
}

fn probe_health(bind: SocketAddr) -> Option<String> {
    use std::io::{Read, Write};
    let mut stream =
        std::net::TcpStream::connect_timeout(&bind, Duration::from_millis(300)).ok()?;
    stream.set_read_timeout(Some(Duration::from_millis(500))).ok()?;
    write!(
        stream,
        "GET /_router/health HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n"
    )
    .ok()?;
    let mut body = String::new();
    stream.read_to_string(&mut body).ok()?;
    Some(body)
}

/// Bind with `SO_REUSEPORT` so an incumbent can keep serving until it drains.
pub fn bind_reuseport(addr: SocketAddr) -> Result<tokio::net::TcpListener> {
    let domain = if addr.is_ipv4() { Domain::IPV4 } else { Domain::IPV6 };
    let socket = Socket::new(domain, Type::STREAM, Some(Protocol::TCP))?;
    socket.set_reuse_address(true)?;
    socket
        .set_reuse_port(true)
        .context("SO_REUSEPORT is required for handover")?;
    socket.set_nonblocking(true)?;
    socket
        .bind(&addr.into())
        .with_context(|| format!("binding {addr}"))?;
    socket.listen(1024)?;
    Ok(tokio::net::TcpListener::from_std(socket.into())?)
}

/// Ask the incumbent to drain and exit.
///
/// This must be called *after* this process starts accepting. `SO_REUSEPORT` makes
/// the kernel distribute new connections across every socket bound to the port, so a
/// process that has joined the group but is not yet in its accept loop silently
/// collects connections nobody reads — they hang until it starts serving. Signalling
/// first and serving second turns a seamless handover into a stall.
pub fn signal_incumbent(pid: i32) -> bool {
    tracing::info!(pid, "an agent-router is already serving this port; asking it to drain");
    // SAFETY: kill(2) with a valid signal number; a stale pid just returns ESRCH.
    if unsafe { libc::kill(pid, libc::SIGTERM) } != 0 {
        tracing::warn!(pid, "could not signal the incumbent; both instances will serve");
        return false;
    }
    true
}

/// Watch the incumbent go away, without holding up our own accept loop.
pub fn watch_incumbent(pid: i32) {
    tokio::spawn(async move {
        for _ in 0..600 {
            tokio::time::sleep(Duration::from_millis(100)).await;
            // SAFETY: signal 0 only tests for the process's existence.
            if unsafe { libc::kill(pid, 0) } != 0 {
                tracing::info!(pid, "incumbent has exited; handover complete");
                return;
            }
        }
        tracing::warn!(pid, "incumbent still draining after a minute");
    });
}

/// Block until `pid` is gone, or until `within` elapses. Returns whether it went.
///
/// The watching above is for the log; this is for work that must not start while the
/// outgoing process is still touching shared state — see `db::migrate_bodies_if_needed`.
pub fn has_exited_within(pid: i32, within: Duration) -> bool {
    let deadline = std::time::Instant::now() + within;
    while std::time::Instant::now() < deadline {
        // SAFETY: signal 0 only tests for the process's existence.
        if unsafe { libc::kill(pid, 0) } != 0 {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

/// Resolves when the process is asked to stop, either way.
pub async fn shutdown_signal() -> &'static str {
    use tokio::signal::unix::{signal, SignalKind};
    let mut term = match signal(SignalKind::terminate()) {
        Ok(s) => s,
        Err(_) => {
            tokio::signal::ctrl_c().await.ok();
            return "interrupt";
        }
    };
    tokio::select! {
        _ = tokio::signal::ctrl_c() => "interrupt",
        _ = term.recv()             => "SIGTERM",
    }
}

#[cfg(test)]
mod tests {
    use super::parse_pid;

    #[test]
    fn reads_the_pid_out_of_a_health_response() {
        let body = "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\n\r\n\
                    {\"ok\":true,\"pid\":4242,\"version\":\"0.1.0\"}";
        assert_eq!(parse_pid(body), Some(4242));
    }

    #[test]
    fn tolerates_bodies_without_a_pid() {
        assert_eq!(parse_pid(""), None);
        assert_eq!(parse_pid("HTTP/1.1 404 Not Found\r\n\r\n"), None);
        assert_eq!(parse_pid("{\"ok\":true}"), None);
        // A different service on the port must not be mistaken for an incumbent.
        assert_eq!(parse_pid("{\"pid\":\"not-a-number\"}"), None);
    }
}

#[cfg(test)]
mod version_tests {
    use super::*;

    #[test]
    fn the_version_is_read_out_of_a_health_body_with_headers_attached() {
        let body = "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\n\r\n{\"ok\":true,\"pid\":4242,\"version\":\"0.1.0\"}";
        assert_eq!(parse_pid(body), Some(4242));
        assert_eq!(parse_version(body).as_deref(), Some("0.1.0"));
        assert_eq!(parse_version("{\"ok\":true,\"pid\":1}"), None, "a build from before the field");
    }

    #[test]
    fn newer_means_a_higher_dotted_number_and_unknown_is_oldest() {
        assert!(is_newer("0.2.0", "0.1.9"));
        assert!(is_newer("0.1.10", "0.1.9"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.1.0", "0.1.1"));
        assert!(is_newer("0.1.0", ""), "an incumbent that reports no version is replaced");
        assert!(!is_newer("", "0.1.0"));
        assert!(!is_newer("0.1.0-rc1", "0.1.0"), "pre-release tags do not count");
    }
}
