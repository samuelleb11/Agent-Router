//! Access guards. The router holds a live subscription credential and signs every
//! request it forwards, so the set of callers allowed to reach it is kept as narrow
//! as it can be without a login of its own.
//!
//! Four independent layers, each closing a different hole:
//!
//! 1. **Loopback bind** (see `config`) — the socket is never exposed off-host.
//! 2. **Peer check** — the connecting address must itself be loopback, so a port
//!    forward or an SSH tunnel cannot smuggle a remote caller in.
//! 3. **Host header check** — defeats DNS rebinding, where a web page resolves its
//!    own domain to 127.0.0.1 so the browser will send credentialed requests here.
//!    The socket and peer checks both *pass* in that attack; only this one fails it.
//! 4. **Shape check** (see `proxy`) — only `/v1/` paths, and only for callers that
//!    already presented a credential of their own.

use std::net::{IpAddr, SocketAddr};

use axum::extract::{ConnectInfo, Request};
use axum::http::{HeaderMap, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

/// Hosts a legitimate local agent will address us by.
///
/// Parsed as an address rather than pattern-matched: `127.0.0.1.evil.com` and
/// `localhost.evil.com` are ordinary registrable domains an attacker can point at
/// 127.0.0.1, and a prefix or substring test would wave them straight through.
fn host_is_local(host: &str) -> bool {
    let bare = if let Some(rest) = host.strip_prefix('[') {
        // Bracketed IPv6, with or without a port: [::1]:8787
        match rest.split_once(']') {
            Some((inner, _)) => inner,
            None => return false,
        }
    } else {
        // Strip a trailing :port only when the host is not itself a bare IPv6
        // literal (which is full of colons).
        match host.split_once(':') {
            Some((h, p)) if !p.contains(':') && !p.is_empty() && p.bytes().all(|c| c.is_ascii_digit()) => h,
            _ => host,
        }
    };

    if bare.eq_ignore_ascii_case("localhost") {
        return true;
    }
    bare.parse::<IpAddr>().map(|ip| ip.is_loopback()).unwrap_or(false)
}

pub fn check_peer(peer: SocketAddr) -> Result<(), &'static str> {
    if peer.ip().is_loopback() {
        Ok(())
    } else {
        Err("caller is not on this machine")
    }
}

pub fn check_host(headers: &HeaderMap) -> Result<(), &'static str> {
    match headers.get("host").and_then(|v| v.to_str().ok()) {
        Some(host) if host_is_local(host) => Ok(()),
        Some(_) => Err("Host header is not a loopback address"),
        None => Err("no Host header"),
    }
}

/// Applied to every route, not just the proxy: `/api` serves captured request
/// bodies, which contain the user's source code, so it needs the same protection
/// as the credential itself.
pub async fn layer(
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    req: Request,
    next: Next,
) -> Response {
    if let Err(why) = check_peer(peer) {
        tracing::warn!(%peer, "refused: {why}");
        return (StatusCode::FORBIDDEN, format!("agent-router: {why}\n")).into_response();
    }
    if let Err(why) = check_host(req.headers()) {
        tracing::warn!(%peer, "refused: {why}");
        return (StatusCode::FORBIDDEN, format!("agent-router: {why}\n")).into_response();
    }
    next.run(req).await
}

#[cfg(test)]
mod tests {
    use super::host_is_local;

    #[test]
    fn accepts_loopback_forms() {
        for h in [
            "127.0.0.1:8787",
            "127.0.0.1",
            "localhost:8787",
            "localhost",
            "[::1]:8787",
            "::1",
            "127.5.5.5:8787",
        ] {
            assert!(host_is_local(h), "should accept {h}");
        }
    }

    #[test]
    fn rejects_rebinding_hosts() {
        // The DNS-rebinding case: a domain that currently resolves to 127.0.0.1.
        for h in [
            "evil.example.com",
            "evil.example.com:8787",
            "127.0.0.1.evil.com",
            "localhost.evil.com",
            "192.168.1.10:8787",
        ] {
            assert!(!host_is_local(h), "should reject {h}");
        }
    }
}
