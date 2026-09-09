//! Byte-faithful reverse proxy to Anthropic with a credential swap.
//!
//! Invariants (see ARCHITECTURE.md):
//!   * The response streams straight back to the agent, unbuffered. Nothing on the
//!     hot path waits on capture, parsing or disk.
//!   * Bodies are never decoded, re-encoded or rewritten. `accept-encoding` is passed
//!     through untouched and the compressed bytes are relayed as-is.
//!   * The only header the router changes is `authorization`.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;

use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderName, HeaderValue, Request, Response, StatusCode, Uri};
use axum::response::IntoResponse;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;

use crate::config::Config;
use crate::credentials::{token_kind, Credential};

/// Headers that must not be forwarded across a proxy hop (RFC 9110 §7.6.1),
/// plus `host`, which is rewritten to the upstream authority.
const HOP_BY_HOP: &[&str] = &[
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailers",
    "transfer-encoding",
    "upgrade",
    "host",
];

type HttpsClient = Client<hyper_rustls::HttpsConnector<hyper_util::client::legacy::connect::HttpConnector>, Body>;

pub struct AppState {
    pub client: HttpsClient,
    pub config: Config,
    pub credential: Credential,
    dumps_left: AtomicUsize,
}

impl AppState {
    pub fn new(config: Config, credential: Credential) -> anyhow::Result<Arc<Self>> {
        let tls = hyper_rustls::HttpsConnectorBuilder::new()
            .with_webpki_roots()
            .https_only()
            .enable_http1()
            .build();
        let client = Client::builder(TokioExecutor::new()).build(tls);
        let dumps_left = AtomicUsize::new(config.dump_headers);
        Ok(Arc::new(Self { client, config, credential, dumps_left }))
    }
}

pub async fn handle(State(state): State<Arc<AppState>>, req: Request<Body>) -> Response<Body> {
    match forward(state, req).await {
        Ok(resp) => resp,
        Err(err) => {
            tracing::error!(error = %err, "upstream request failed");
            (StatusCode::BAD_GATEWAY, format!("agent-router: {err}\n")).into_response()
        }
    }
}

async fn forward(state: Arc<AppState>, req: Request<Body>) -> anyhow::Result<Response<Body>> {
    let started = Instant::now();
    let (parts, body) = req.into_parts();
    let path_and_query = parts
        .uri
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or("/")
        .to_string();

    // Phase 1 diagnostic: what does Claude Code actually put on the wire when its
    // base URL points somewhere other than Anthropic? Tokens are logged by family
    // prefix only, never by value.
    if state.dumps_left.fetch_sub(1, Ordering::Relaxed) > 0 {
        dump_request(&parts.method, &path_and_query, &parts.headers);
    }

    // The router holds a real subscription credential, so it must not lend it to
    // anything that wanders onto the port. A browser tab that hits 127.0.0.1:8787 —
    // or any page on this machine that POSTs to it — would otherwise have its
    // request signed with the user's token. Two cheap guards: only API paths are
    // proxied, and the caller must already have presented a credential of its own.
    if !path_and_query.starts_with("/v1/") {
        tracing::warn!(path = %path_and_query, "refused: not an API path");
        return Ok((StatusCode::NOT_FOUND, "agent-router: not an API path\n").into_response());
    }
    if !parts.headers.contains_key("authorization") && !parts.headers.contains_key("x-api-key") {
        tracing::warn!(path = %path_and_query, "refused: caller presented no credential");
        return Ok((StatusCode::UNAUTHORIZED, "agent-router: no credential presented\n")
            .into_response());
    }

    // Exact session identity, straight off the wire — Claude Code labels every request.
    let session_id = header_str(&parts.headers, "x-claude-code-session-id");

    let upstream: Uri = format!("{}{}", state.config.upstream, path_and_query).parse()?;
    let authority = upstream
        .authority()
        .map(|a| a.as_str().to_owned())
        .unwrap_or_default();

    let mut out = Request::builder().method(parts.method.clone()).uri(upstream);
    {
        let headers = out.headers_mut().expect("builder has headers");
        copy_forwardable(&parts.headers, headers);
        headers.insert("host", HeaderValue::from_str(&authority)?);
        // The credential swap: whatever the agent sent is discarded, ours goes on.
        headers.remove("x-api-key");
        let mut auth = HeaderValue::from_str(&format!("Bearer {}", state.credential.access_token))?;
        auth.set_sensitive(true);
        headers.insert("authorization", auth);
    }

    let upstream_resp = state.client.request(out.body(body)?).await?;
    let (resp_parts, resp_body) = upstream_resp.into_parts();

    // Latency to response headers. Full stream duration lands in Phase 2, where the
    // async tee owns the body and can time it without holding up the agent.
    let request_id = header_str(&resp_parts.headers, "request-id");
    tracing::info!(
        method = %parts.method,
        path = %path_and_query,
        status = resp_parts.status.as_u16(),
        ttfb_ms = started.elapsed().as_millis() as u64,
        request_id = request_id.as_deref().unwrap_or("-"),
        session = session_id.as_deref().unwrap_or("-"),
        "proxied"
    );
    if resp_parts.status.is_client_error() || resp_parts.status.is_server_error() {
        dump_response_headers(&resp_parts.headers);
    }

    let mut resp = Response::builder().status(resp_parts.status);
    {
        let headers = resp.headers_mut().expect("builder has headers");
        copy_forwardable(&resp_parts.headers, headers);
    }
    Ok(resp.body(Body::new(resp_body))?)
}

/// Copy every header except the hop-by-hop set. Values are moved verbatim — no
/// normalisation, no reordering of duplicates.
fn copy_forwardable(from: &HeaderMap, to: &mut HeaderMap) {
    for (name, value) in from {
        if HOP_BY_HOP.contains(&name.as_str()) {
            continue;
        }
        to.append(name.clone(), value.clone());
    }
}

fn header_str(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
}

fn dump_request(method: &axum::http::Method, path: &str, headers: &HeaderMap) {
    tracing::info!("--- inbound request headers: {method} {path} ---");
    let mut names: Vec<&HeaderName> = headers.keys().collect();
    names.sort_by_key(|n| n.as_str());
    for name in names {
        for value in headers.get_all(name) {
            let shown = match name.as_str() {
                "authorization" | "x-api-key" => value
                    .to_str()
                    .map(|v| format!("<redacted: {}>", token_kind(v)))
                    .unwrap_or_else(|_| "<redacted>".into()),
                _ => value.to_str().unwrap_or("<non-utf8>").to_string(),
            };
            tracing::info!("  {}: {}", name.as_str(), shown);
        }
    }
    if !headers.contains_key("authorization") && !headers.contains_key("x-api-key") {
        tracing::warn!("  (no credential header present — the agent sent none)");
    }
    tracing::info!("--- end headers ---");
}

fn dump_response_headers(headers: &HeaderMap) {
    for (name, value) in headers {
        tracing::warn!("  <- {}: {}", name.as_str(), value.to_str().unwrap_or("<non-utf8>"));
    }
}
