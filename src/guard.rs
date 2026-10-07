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
//! 6. **Origin check** — a page on another site can make a browser send a write here,
//!    with the router's cached Basic credentials attached, and every layer above passes
//!    it: it is the right host, from the right machine. `Origin` gives it away.
//!
//! A hosted router (`serve --server`) changes three of these. Layer 1 is a bind it was
//! given and nothing else. Layer 3 also admits the names it was told it is reached by —
//! its ingress host and its Service names — and still refuses every other name. Layer 5
//! asks loopback too: in a pod, loopback is a port-forward or a mesh sidecar, not the
//! owner at the keyboard.

use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use axum::extract::{ConnectInfo, Request, State};
use axum::http::{header, HeaderMap, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::auth::Verdict;
use crate::proxy::AppState;

/// A Host header without its port: `[::1]:8787` is `::1`, `example.com:443` is
/// `example.com`, and a bare IPv6 literal (which is full of colons) is itself. `None`
/// for an unterminated bracket.
fn bare_host(host: &str) -> Option<&str> {
    if let Some(rest) = host.strip_prefix('[') {
        // Bracketed IPv6, with or without a port: [::1]:8787
        return rest.split_once(']').map(|(inner, _)| inner);
    }
    // Strip a trailing :port only when the host is not itself a bare IPv6 literal.
    Some(match host.split_once(':') {
        Some((h, p)) if !p.contains(':') && !p.is_empty() && p.bytes().all(|c| c.is_ascii_digit()) => h,
        _ => host,
    })
}

/// Hosts a legitimate local agent will address us by.
///
/// Parsed as an address rather than pattern-matched: `127.0.0.1.evil.com` and
/// `localhost.evil.com` are ordinary registrable domains an attacker can point at
/// 127.0.0.1, and a prefix or substring test would wave them straight through.
fn host_is_local(host: &str) -> bool {
    let Some(bare) = bare_host(host) else {
        return false;
    };
    if bare.eq_ignore_ascii_case("localhost") {
        return true;
    }
    bare.parse::<IpAddr>().map(|ip| ip.is_loopback()).unwrap_or(false)
}

/// One of the names a hosted router was told it is reached by (`--allow-host`,
/// `--public-url`). Exact and case-insensitive, whatever the port, with one trailing dot
/// dropped — the same normalisation `config` applied to the list. Never a suffix or a
/// pattern: `router.example.com.evil.com` is somebody else's name.
fn host_is_allowed(host: &str, allow: &[String]) -> bool {
    if allow.is_empty() {
        return false;
    }
    let Some(bare) = bare_host(host) else {
        return false;
    };
    let bare = bare.strip_suffix('.').unwrap_or(bare);
    allow.iter().any(|name| name.eq_ignore_ascii_case(bare))
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

pub fn check_host(headers: &HeaderMap, allow: &[String]) -> Result<(), String> {
    match headers.get("host").and_then(|v| v.to_str().ok()) {
        Some(host) if host_is_local(host) => Ok(()),
        // Off-loopback, the browser addresses us by bare IP. Bare IPs are still
        // checked, and hostnames still refused, because DNS rebinding needs a *name*
        // to point at us — so layer 3 keeps its whole purpose even when widened.
        Some(host) if off_host() && host_is_ip_literal(host) => Ok(()),
        // A hosted router is reached by name, through its ingress or as a Service in
        // the cluster: the names it was given, and no others. Only a hosted router has
        // any, because only there does every caller, loopback included, present the
        // password that a rebinding page could not.
        Some(host) if host_is_allowed(host, allow) => Ok(()),
        Some(_) if !allow.is_empty() => Err(format!(
            "Host header is not an address we serve (this router answers to {})",
            allow.join(", ")
        )),
        Some(_) => Err("Host header is not an address we serve".into()),
        None => Err("no Host header".into()),
    }
}

/// True when the Host header is a bare IP literal rather than a registrable name.
fn host_is_ip_literal(host: &str) -> bool {
    bare_host(host).is_some_and(|bare| bare.parse::<IpAddr>().is_ok())
}

/// Layer 6. A write from a browser carries the `Origin` of the page that made it, and a
/// page cannot forge it, so one whose origin is not this router's own is another site
/// driving the user's browser — refused before it reaches a handler. Reads are left
/// alone: the browser will not show another site what they return. Nothing that
/// legitimately writes is caught: this router's own pages are same-origin, and the
/// phones, Claude Code and Northstar send no `Origin` at all. `Origin: null` (a sandboxed
/// frame, a file) is refused, since it names no site that could be this one.
pub fn check_origin(method: &Method, headers: &HeaderMap) -> Result<(), &'static str> {
    if matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS) {
        return Ok(());
    }
    let Some(origin) = headers.get(header::ORIGIN) else {
        return Ok(());
    };
    const REFUSED: &str = "cross-site request refused";
    let origin = origin.to_str().map_err(|_| REFUSED)?;
    let authority = origin
        .split_once("://")
        .map(|(_, rest)| rest)
        .filter(|a| !a.is_empty())
        .ok_or(REFUSED)?;
    let host = headers.get("host").and_then(|v| v.to_str().ok()).unwrap_or("");
    if same_authority(authority, host) {
        Ok(())
    } else {
        Err(REFUSED)
    }
}

/// Whether an `Origin`'s authority and a `Host` header name the same place. Browsers
/// leave the default port out of `Origin`; a proxy may leave it in `Host`.
fn same_authority(origin: &str, host: &str) -> bool {
    let plain = |s: &str| {
        let s = s.to_ascii_lowercase();
        match s.strip_suffix(":443").or_else(|| s.strip_suffix(":80")) {
            Some(bare) => bare.to_string(),
            None => s,
        }
    };
    !host.is_empty() && plain(origin) == plain(host)
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
    if let Err(why) = check_host(req.headers(), &state.config.allow_hosts) {
        tracing::warn!(%peer, "refused: {why}");
        return (StatusCode::FORBIDDEN, format!("agent-router: {why}\n")).into_response();
    }
    if let Err(why) = check_origin(req.method(), req.headers()) {
        tracing::warn!(%peer, path = %req.uri().path(), "refused: {why}");
        return (StatusCode::FORBIDDEN, format!("agent-router: {why}\n")).into_response();
    }
    // Layer 5. Health stays open: it is pid and version, and the handover probe in
    // `lifecycle` reads it before it has anything to authenticate with — as do the
    // kubelet's probes, for a hosted router. A hosted router asks loopback too, and
    // leaves its agent installer open: a machine being connected has no password to
    // present until the script asks for one, and nothing it fetches is secret
    // (`downloads`).
    let asks = !peer.ip().is_loopback() || state.config.server;
    let path = req.uri().path();
    let open = path == "/_router/health" || (state.config.server && crate::downloads::is_open(path));
    if asks && !open {
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
    use super::{check_origin, host_is_allowed, host_is_local};
    use axum::http::{HeaderMap, HeaderValue, Method};

    fn allow() -> Vec<String> {
        vec!["router.example.com".into(), "agent-router.tools.svc".into()]
    }

    #[test]
    fn a_hosted_router_answers_to_its_names_only() {
        for h in [
            "router.example.com",
            "Router.Example.COM",
            "router.example.com:443",
            "router.example.com.",
            "agent-router.tools.svc:8787",
        ] {
            assert!(host_is_allowed(h, &allow()), "should accept {h}");
        }
        for h in [
            "router.example.com.evil.com",
            "evil-router.example.com",
            "example.com",
            "tools.svc",
            "[router.example.com",
            "",
        ] {
            assert!(!host_is_allowed(h, &allow()), "should reject {h}");
        }
        assert!(!host_is_allowed("router.example.com", &[]), "a local router has no names");
    }

    fn headers(pairs: &[(&'static str, &'static str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(*k, HeaderValue::from_static(v));
        }
        h
    }

    #[test]
    fn same_origin_writes_pass_and_cross_site_ones_do_not() {
        // The dashboard on loopback, by LAN address, and behind an ingress.
        for (origin, host) in [
            ("http://127.0.0.1:8787", "127.0.0.1:8787"),
            ("http://localhost:8787", "localhost:8787"),
            ("http://192.0.2.10:8787", "192.0.2.10:8787"),
            ("https://router.example.com", "router.example.com"),
            ("https://Router.example.com", "router.example.com:443"),
        ] {
            let h = headers(&[("origin", origin), ("host", host)]);
            assert!(check_origin(&Method::POST, &h).is_ok(), "{origin} -> {host} should pass");
        }
        for (origin, host) in [
            ("https://evil.example.com", "127.0.0.1:8787"),
            ("http://127.0.0.1:9999", "127.0.0.1:8787"),
            ("null", "127.0.0.1:8787"),
            ("https://router.example.com.evil.com", "router.example.com"),
        ] {
            let h = headers(&[("origin", origin), ("host", host)]);
            assert!(check_origin(&Method::POST, &h).is_err(), "{origin} -> {host} should be refused");
        }
        // No Origin (phones, Claude Code, Northstar) and reads are never refused.
        assert!(check_origin(&Method::POST, &headers(&[("host", "127.0.0.1:8787")])).is_ok());
        let cross = headers(&[("origin", "https://evil.example.com"), ("host", "127.0.0.1:8787")]);
        assert!(check_origin(&Method::GET, &cross).is_ok());
    }

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
