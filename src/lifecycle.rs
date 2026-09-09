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
