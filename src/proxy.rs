//! Byte-faithful reverse proxy to Anthropic with a credential swap.
//!
//! Invariants (see ARCHITECTURE.md):
//!   * The response streams straight back to the agent, unbuffered. Capture rides
//!     alongside it and can only ever drop events, never delay bytes.
//!   * Response bodies are relayed exactly as they arrive — still compressed, never
//!     re-encoded. Capture decompresses a *copy* so it can read the SSE.
//!   * The only header the router changes is `authorization`.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;

use axum::body::Body;
use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, Request, Response, StatusCode, Uri};
use axum::response::IntoResponse;
use futures_util::StreamExt;
use http_body_util::BodyStream;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use tokio::sync::mpsc::Sender;

use crate::analyze::{analyze_request, SseScanner};
use crate::capture::{CaptureEvent, Decoder, RateLimit, StoredBody};
use crate::config::Config;
use crate::credentials::{token_kind, Credential};
use crate::db::now_ms;
use crate::guard;

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

/// Ceiling on a buffered request body. Claude Code sends complete JSON with a
/// content-length, so buffering costs microseconds; this only stops a pathological
/// body from being held in memory.
const MAX_REQUEST_BODY: usize = 64 * 1024 * 1024;

pub const ACCOUNT_ID: &str = "default";

type HttpsClient = Client<
    hyper_rustls::HttpsConnector<hyper_util::client::legacy::connect::HttpConnector>,
    Body,
>;

pub struct AppState {
    pub client: HttpsClient,
    pub config: Config,
    /// Re-read rather than held: Claude Code refreshes this Keychain entry roughly
    /// hourly, and a token cached at startup would start 401ing mid-session. Phase 3
    /// replaces this with router-owned tokens the router refreshes itself.
    pub credential: tokio::sync::RwLock<Credential>,
    pub capture: Sender<CaptureEvent>,
    dumps_left: AtomicUsize,
    dropped: AtomicUsize,
}

impl AppState {
    pub fn new(
        config: Config,
        credential: Credential,
        capture: Sender<CaptureEvent>,
    ) -> anyhow::Result<Arc<Self>> {
        let tls = hyper_rustls::HttpsConnectorBuilder::new()
            .with_webpki_roots()
            .https_only()
            .enable_http1()
            .build();
        let client = Client::builder(TokioExecutor::new()).build(tls);
        let dumps_left = AtomicUsize::new(config.dump_headers);
        Ok(Arc::new(Self {
            client,
            config,
            credential: tokio::sync::RwLock::new(credential),
            capture,
            dumps_left,
            dropped: AtomicUsize::new(0),
        }))
    }

    /// The token to sign with, reloaded when it is close to expiry.
    async fn access_token(&self) -> String {
        const RENEW_WITHIN_SECS: i64 = 300;
        {
            let held = self.credential.read().await;
            // expires_at_ms == 0 means an explicitly supplied token with no known
            // expiry; leave it alone.
            if held.expires_at_ms == 0 || held.expires_in_secs() > RENEW_WITHIN_SECS {
                return held.access_token.clone();
            }
        }
        let mut held = self.credential.write().await;
        if held.expires_in_secs() <= RENEW_WITHIN_SECS {
            match tokio::task::spawn_blocking(crate::credentials::load).await {
                Ok(Ok(fresh)) => {
                    tracing::info!(
                        "credential reloaded from the Keychain; expires in {} min",
                        fresh.expires_in_secs() / 60
                    );
                    *held = fresh;
                }
                Ok(Err(err)) => tracing::warn!(error = %err, "could not reload the credential"),
                Err(err) => tracing::warn!(error = %err, "credential reload task failed"),
            }
        }
        held.access_token.clone()
    }

    /// Hand a finished request to the capture task, or drop it. Never blocks.
    fn offer(&self, ev: CaptureEvent) {
        if self.capture.try_send(ev).is_err() {
            let n = self.dropped.fetch_add(1, Ordering::Relaxed) + 1;
            if n.is_power_of_two() {
                tracing::warn!("capture queue full; {n} request(s) not recorded");
            }
        }
    }
}

pub async fn handle(
    State(state): State<Arc<AppState>>,
    ConnectInfo(peer): ConnectInfo<std::net::SocketAddr>,
    req: Request<Body>,
) -> Response<Body> {
    // Layers 2 and 3 of the access guards; see `guard`. Refused before the request
    // touches the credential or the network.
    if let Err(why) = guard::check_peer(peer) {
        tracing::warn!(%peer, "refused: {why}");
        return (StatusCode::FORBIDDEN, format!("agent-router: {why}\n")).into_response();
    }
    if let Err(why) = guard::check_host(req.headers()) {
        tracing::warn!(%peer, "refused: {why}");
        return (StatusCode::FORBIDDEN, format!("agent-router: {why}\n")).into_response();
    }
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
    let started_at_ms = now_ms();
    let (parts, body) = req.into_parts();
    let path_and_query = parts
        .uri
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or("/")
        .to_string();

    if state.dumps_left.fetch_sub(1, Ordering::Relaxed) > 0 {
        dump_request(&parts.method, &path_and_query, &parts.headers);
    }

    // Layer 4: the router holds a real subscription credential, so it must not lend
    // it to anything that wanders onto the port.
    if !path_and_query.starts_with("/v1/") {
        tracing::warn!(path = %path_and_query, "refused: not an API path");
        return Ok((StatusCode::NOT_FOUND, "agent-router: not an API path\n").into_response());
    }
    if !parts.headers.contains_key("authorization") && !parts.headers.contains_key("x-api-key") {
        tracing::warn!(path = %path_and_query, "refused: caller presented no credential");
        return Ok(
            (StatusCode::UNAUTHORIZED, "agent-router: no credential presented\n").into_response(),
        );
    }

    // Exact session identity, straight off the wire — Claude Code labels every request.
    let session_id = header_str(&parts.headers, "x-claude-code-session-id");
    let entrypoint = header_str(&parts.headers, "user-agent");
    // Subagents carry this; the main thread does not.
    let agent_id = header_str(&parts.headers, "x-claude-code-agent-id");

    // Claude Code sends a complete JSON body with a content-length, so collecting it
    // is effectively free and lets the capture task see what filled the context. The
    // latency-sensitive direction — the response stream — is never buffered.
    let request_bytes = axum::body::to_bytes(body, MAX_REQUEST_BODY).await?;
    let shape = analyze_request(&request_bytes);

    let upstream: Uri = format!("{}{}", state.config.upstream, path_and_query).parse()?;
    let authority = upstream
        .authority()
        .map(|a| a.as_str().to_owned())
        .unwrap_or_default();

    let token = state.access_token().await;
    let mut out = Request::builder().method(parts.method.clone()).uri(upstream);
    {
        let headers = out.headers_mut().expect("builder has headers");
        copy_forwardable(&parts.headers, headers);
        headers.insert("host", HeaderValue::from_str(&authority)?);
        // The credential swap: whatever the agent sent is discarded, ours goes on.
        headers.remove("x-api-key");
        let mut auth = HeaderValue::from_str(&format!("Bearer {token}"))?;
        auth.set_sensitive(true);
        headers.insert("authorization", auth);
    }

    let upstream_resp = state
        .client
        .request(out.body(Body::from(request_bytes.clone()))?)
        .await?;
    let (resp_parts, resp_body) = upstream_resp.into_parts();

    let ttfb_ms = started.elapsed().as_millis() as i64;
    let request_id = header_str(&resp_parts.headers, "request-id");
    let status = resp_parts.status.as_u16() as i64;
    let rl = RateLimit::from_headers(&resp_parts.headers);
    let encoding = header_str(&resp_parts.headers, "content-encoding");
    let is_sse = header_str(&resp_parts.headers, "content-type")
        .map(|ct| ct.contains("event-stream"))
        .unwrap_or(false);

    tracing::info!(
        method = %parts.method,
        path = %path_and_query,
        status,
        ttfb_ms,
        request_id = request_id.as_deref().unwrap_or("-"),
        session = session_id.as_deref().unwrap_or("-"),
        subagent = agent_id.is_some(),
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

    // The tee. Each chunk is forwarded downstream first, then copied into the
    // scanner. Capture can only ever fall behind by dropping, never by blocking.
    let method = parts.method.to_string();
    let body_cap = state.config.body_cap;
    let tee = async_stream::stream! {
        let mut stream = BodyStream::new(resp_body);
        let mut decoder = Decoder::for_encoding(encoding.as_deref());
        let mut scanner = SseScanner::default();
        let mut raw = Vec::new();
        let mut wire_bytes: i64 = 0;
        let mut transport_error: Option<String> = None;

        while let Some(item) = stream.next().await {
            match item {
                Ok(frame) => {
                    let Ok(data) = frame.into_data() else { continue };
                    wire_bytes += data.len() as i64;
                    let plain = decoder.feed(&data);
                    if !plain.is_empty() {
                        if raw.len() < body_cap {
                            let room = body_cap - raw.len();
                            raw.extend_from_slice(&plain[..plain.len().min(room)]);
                        }
                        if is_sse {
                            scanner.feed(&plain);
                        }
                    }
                    yield Ok::<_, std::io::Error>(data);
                }
                Err(err) => {
                    transport_error = Some(err.to_string());
                    yield Err(std::io::Error::other(err.to_string()));
                    break;
                }
            }
        }

        let tail = decoder.finish();
        if !tail.is_empty() {
            if raw.len() < body_cap {
                let room = body_cap - raw.len();
                raw.extend_from_slice(&tail[..tail.len().min(room)]);
            }
            if is_sse {
                scanner.feed(&tail);
            }
        }
        if !is_sse && !raw.is_empty() {
            scanner.absorb_json_body(&raw);
        }

        state.offer(CaptureEvent {
            request_id,
            message_id: scanner.message_id.clone(),
            session_id,
            agent_id,
            account_id: ACCOUNT_ID.to_string(),
            entrypoint,
            model: scanner
                .model
                .clone()
                .or_else(|| shape.as_ref().and_then(|s| s.model.clone())),
            method,
            path: path_and_query,
            started_at_ms,
            ttfb_ms,
            duration_ms: started.elapsed().as_millis() as i64,
            status,
            // The response is the authority; the request's `stream` flag is the
            // fallback for a call that errored before any event arrived.
            streaming: is_sse || shape.as_ref().map(|s| s.streaming).unwrap_or(false),
            stop_reason: scanner.stop_reason.clone(),
            service_tier: scanner.service_tier.clone(),
            error: transport_error.or(scanner.error.clone()),
            rl,
            usage: scanner.usage.clone(),
            tool_calls: scanner.tool_calls.clone(),
            request_body: StoredBody::new(&request_bytes, body_cap),
            response_body: StoredBody::new(&raw, body_cap),
            request_bytes: request_bytes.len() as i64,
            response_bytes: wire_bytes,
            shape,
        });
    };

    Ok(resp.body(Body::from_stream(tee))?)
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
    tracing::info!("--- end headers ---");
}

fn dump_response_headers(headers: &HeaderMap) {
    for (name, value) in headers {
        tracing::warn!("  <- {}: {}", name.as_str(), value.to_str().unwrap_or("<non-utf8>"));
    }
}
