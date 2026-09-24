//! Access guards. The router holds a live subscription credential and signs every
//! request it forwards, so the set of callers allowed to reach it is kept as narrow
//! as it can be.
//!
//! Five independent layers, each closing a different hole:
//!
//! 1. **Loopback bind** (see `config` and `network`) — the socket is never exposed
//!    off-host unless the stored setting or `AGENT_ROUTER_BIND` says so.
//! 2. **Peer check** — the connecting address must itself be loopback, so a port
//!    forward or an SSH tunnel cannot smuggle a remote caller in.
//! 3. **Host header check** — defeats DNS rebinding, where a web page resolves its
//!    own domain to 127.0.0.1 so the browser will send credentialed requests here.
//!    The socket and peer checks both *pass* in that attack; only this one fails it.
//! 4. **Shape check** (see `proxy`) — only `/v1/` paths, and only for callers that
//!    already presented a credential of their own.
//! 5. **Password** (see `auth`) — a caller that is not on this machine, which layers
//!    1 and 2 only admit when the bind was widened on purpose, must present the
//!    router's password. Without one set, it is refused outright.

use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use axum::extract::{ConnectInfo, Request, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::auth::Verdict;
use crate::proxy::AppState;

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

/// Follows the socket: set at startup from the bind address, and again whenever the
/// switch in `network` adds or removes the wider listener. When the socket is
/// deliberately off-loopback, layers 2 and 3 cannot demand loopback or they would
/// refuse the very callers the operator just chose to allow.
static OFF_HOST: AtomicBool = AtomicBool::new(false);

pub fn allow_off_host(on: bool) {
    OFF_HOST.store(on, Ordering::Relaxed);
}

fn off_host() -> bool {
    OFF_HOST.load(Ordering::Relaxed)
}

pub fn check_peer(peer: SocketAddr) -> Result<(), &'static str> {
    if peer.ip().is_loopback() || off_host() {
        Ok(())
    } else {
        Err("caller is not on this machine")
    }
}

pub fn check_host(headers: &HeaderMap) -> Result<(), &'static str> {
    match headers.get("host").and_then(|v| v.to_str().ok()) {
        Some(host) if host_is_local(host) => Ok(()),
        // Off-loopback, the browser addresses us by bare IP. Bare IPs are still
        // checked, and hostnames still refused, because DNS rebinding needs a *name*
        // to point at us — so layer 3 keeps its whole purpose even when widened.
        Some(host) if off_host() && host_is_ip_literal(host) => Ok(()),
        Some(_) => Err("Host header is not an address we serve"),
        None => Err("no Host header"),
    }
}

/// True when the Host header is a bare IP literal rather than a registrable name.
fn host_is_ip_literal(host: &str) -> bool {
    let bare = if let Some(rest) = host.strip_prefix('[') {
        match rest.split_once(']') {
            Some((inner, _)) => inner,
            None => return false,
        }
    } else {
        match host.split_once(':') {
            Some((h, p)) if !p.contains(':') && !p.is_empty() && p.bytes().all(|c| c.is_ascii_digit()) => h,
            _ => host,
        }
    };
    bare.parse::<IpAddr>().is_ok()
}

/// Applied to every route, not just the proxy: `/api` serves captured request
/// bodies, which contain the user's source code, so it needs the same protection
/// as the credential itself.
pub async fn layer(
    State(state): State<Arc<AppState>>,
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
    // Layer 5. Health stays open: it is pid and version, and the handover probe in
    // `lifecycle` reads it before it has anything to authenticate with.
    if !peer.ip().is_loopback() && req.uri().path() != "/_router/health" {
        match state.auth.check(req.headers()).await {
            Verdict::Ok => {}
            Verdict::NoPassword => {
                tracing::warn!(%peer, "refused: no password is set");
                return (
                    StatusCode::FORBIDDEN,
                    "agent-router: no password set — set one from the dashboard on the machine running the router\n",
                )
                    .into_response();
            }
            // A browser's first hit on a page is always this, so it is not a warning —
            // but a client that should be presenting the password and is not looks
            // identical, and this is the only place that can say so.
            Verdict::Missing => {
                tracing::info!(%peer, path = %req.uri().path(), "challenged: no password presented");
                return challenge(req.uri().path());
            }
            Verdict::Wrong => {
                tracing::warn!(%peer, path = %req.uri().path(), "refused: wrong password");
                return challenge(req.uri().path());
            }
        }
    }
    next.run(req).await
}

/// A browser gets the Basic challenge and shows its prompt. An agent path does not:
/// nothing on `/v1/` is a browser, and the fix there is an environment variable.
fn challenge(path: &str) -> Response {
    if path.starts_with("/v1/") {
        return (
            StatusCode::UNAUTHORIZED,
            "agent-router: this router has a password — set ANTHROPIC_AUTH_TOKEN to it\n",
        )
            .into_response();
    }
    (
        StatusCode::UNAUTHORIZED,
        [(header::WWW_AUTHENTICATE, "Basic realm=\"agent-router\", charset=\"UTF-8\"")],
        "agent-router: password required\n",
    )
        .into_response()
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
