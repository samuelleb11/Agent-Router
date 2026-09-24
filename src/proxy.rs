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
use axum::extract::State;
use axum::http::{HeaderMap, HeaderName, HeaderValue, Request, Response, StatusCode, Uri};
use axum::response::IntoResponse;
use futures_util::StreamExt;
use http_body_util::BodyStream;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use tokio::sync::mpsc::Sender;

use crate::analyze::analyze_request;
use crate::capture::{CaptureEvent, Pending, RateLimit, StoredBody, Tee};
use crate::accounts::{self, token_kind};
use crate::config::Config;
use crate::db::now_ms;

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

/// The credential currently being signed with, cached between requests.
pub struct Active {
    pub account_id: String,
    pub label: String,
    /// Who the credential actually belonged to when it was resolved.
    pub identity: Option<String>,
    pub token: String,
    pub expires_at_ms: i64,
}

type HttpsClient = Client<
    hyper_rustls::HttpsConnector<hyper_util::client::legacy::connect::HttpConnector>,
    Body,
>;

pub struct AppState {
    pub client: HttpsClient,
    pub config: Config,
    /// Cached, not held: a Keychain-sourced token is refreshed by Claude Code roughly
    /// hourly, so one cached at startup would begin 401ing mid-session. Cleared
    /// outright when the active account changes, so a switch takes effect on the very
    /// next request with nothing restarted.
    pub active: tokio::sync::RwLock<Option<Active>>,
    /// Last quota reading; see `quota`. Keyed by account, because one account's
    /// numbers must never be shown under another's name.
    pub quota_cache: tokio::sync::RwLock<Option<crate::quota::Cached>>,
    /// The in-progress `setup-token` sign-in, if any. One at a time.
    pub login: crate::login::Shared,
    /// The router's password, asked of callers that are not on this machine.
    pub auth: crate::auth::Auth,
    /// Whether we also listen beyond loopback, and the lever that changes it while
    /// the router runs. See `network`.
    pub network: crate::network::Switch,
    /// Phones to tell when a session is waiting on a person.
    pub push: std::sync::Arc<crate::notify::Push>,
    pub capture: Sender<CaptureEvent>,
    /// Read-only connections for the dashboard API, kept warm. See `api::ReadPool`.
    pub reads: crate::api::ReadPool,
    /// The expensive dashboard answers, held for a moment so a room full of clients
    /// polling the same figures costs one computation. See `api::Memo`.
    pub memo: crate::api::Memo,
    dumps_left: AtomicUsize,
    dropped: AtomicUsize,
}

impl AppState {
    pub fn new(
        config: Config,
        capture: Sender<CaptureEvent>,
        network: crate::network::Switch,
    ) -> anyhow::Result<Arc<Self>> {
        let tls = hyper_rustls::HttpsConnectorBuilder::new()
            .with_webpki_roots()
            .https_only()
            .enable_http1()
            .build();
        let client = Client::builder(TokioExecutor::new()).build(tls);
        let dumps_left = AtomicUsize::new(config.dump_headers);
        let auth = crate::auth::Auth::load(&config.db_path)?;
        let push = std::sync::Arc::new(crate::notify::Push::new(&config.db_path));
        let reads = crate::api::ReadPool::new(&config.db_path);
        // Shorter than the fastest thing the dashboard polls (1.5s), so the page is no
        // less live than it was before there was a cache here.
        let memo = crate::api::Memo::new(std::time::Duration::from_millis(1000));
        Ok(Arc::new(Self {
            client,
            config,
            active: tokio::sync::RwLock::new(None),
            quota_cache: tokio::sync::RwLock::new(None),
            login: Default::default(),
            auth,
            network,
            push,
            capture,
            reads,
            memo,
            dumps_left,
            dropped: AtomicUsize::new(0),
        }))
    }

    /// Drop the cached credential so the next request re-resolves it.
    pub async fn invalidate(&self) {
        *self.active.write().await = None;
        // The quota reading belongs to the account that produced it, so a switch
        // must drop it too — otherwise the new account inherits the old one's numbers.
        *self.quota_cache.write().await = None;
    }

    /// The account id and token to sign with.
    pub(crate) async fn signing_credential(&self) -> anyhow::Result<(String, String)> {
        self.signing_credential_full()
            .await
            .map(|(id, _identity, token)| (id, token))
    }

    /// The account, the identity it resolved to, and the token — re-resolved when the
    /// cache is empty or the token is close to expiry.
    pub(crate) async fn signing_credential_full(
        &self,
    ) -> anyhow::Result<(String, Option<String>, String)> {
        const RENEW_WITHIN_SECS: i64 = 300;
        {
            let held = self.active.read().await;
            if let Some(a) = held.as_ref() {
                let fresh = a.expires_at_ms == 0
                    || (a.expires_at_ms - crate::db::now_ms()) / 1000 > RENEW_WITHIN_SECS;
                if fresh {
                    return Ok((a.account_id.clone(), a.identity.clone(), a.token.clone()));
                }
            }
        }
        let mut held = self.active.write().await;
        let db_path = self.config.db_path.clone();
        let loaded = tokio::task::spawn_blocking(move || {
            let store = accounts::Store::new(db_path);
            let account = store.active()?;
            let resolved = accounts::resolve(&account)?;
            Ok::<_, anyhow::Error>(Active {
                account_id: account.id,
                label: account.label,
                identity: resolved.identity.or(account.email),
                token: resolved.token,
                expires_at_ms: resolved.expires_at_ms,
            })
        })
        .await??;
        tracing::info!(
            account = %loaded.label,
            identity = loaded.identity.as_deref().unwrap_or("-"),
            credential = token_kind(&loaded.token),
            "signing credential resolved"
        );
        let out = (
            loaded.account_id.clone(),
            loaded.identity.clone(),
            loaded.token.clone(),
        );
        *held = Some(loaded);
        Ok(out)
    }

    /// Hand a finished request to the capture task, or drop it. Never blocks.
    pub(crate) fn offer(&self, ev: CaptureEvent) {
        if self.capture.try_send(ev).is_err() {
            let n = self.dropped.fetch_add(1, Ordering::Relaxed) + 1;
            if n.is_power_of_two() {
                tracing::warn!("capture queue full; {n} request(s) not recorded");
            }
        }
    }
}

// Access layers 1-3 (loopback bind, peer, Host) are applied to every route by
// `guard::layer`. Layer 4 — path shape and credential presence — lives in `forward`,
// since it is specific to proxying.
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

    let (account_id, account_email, token) = state.signing_credential_full().await?;
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
    // accumulator. Capture can only ever fall behind by dropping, never by blocking,
    // and the accumulator emits its event on Drop so a response the consumer stops
    // reading early — every content-length error body — is still recorded.
    let pending = Pending {
        request_id,
        session_id,
        agent_id,
        account_id,
        account_email,
        entrypoint,
        method: parts.method.to_string(),
        path: path_and_query,
        started_at_ms,
        ttfb_ms,
        status,
        streaming_requested: shape.as_ref().map(|s| s.streaming).unwrap_or(false),
        rl,
        request_body: StoredBody::new(&request_bytes, state.config.body_cap),
        request_bytes: request_bytes.len() as i64,
        shape,
    };
    let mut tee = Tee::new(
        state.clone(),
        pending,
        encoding.as_deref(),
        is_sse,
        state.config.body_cap,
        started,
    );

    let body = async_stream::stream! {
        let mut stream = BodyStream::new(resp_body);
        while let Some(item) = stream.next().await {
            match item {
                Ok(frame) => {
                    let Ok(data) = frame.into_data() else { continue };
                    tee.feed(&data);
                    yield Ok::<_, std::io::Error>(data);
                }
                Err(err) => {
                    tee.fail(err.to_string());
                    yield Err(std::io::Error::other(err.to_string()));
                    break;
                }
            }
        }
        // `tee` is dropped here on normal completion, and equally if this generator is
        // dropped early because the consumer had all the bytes it was promised.
        drop(tee);
    };

    Ok(resp.body(Body::from_stream(body))?)
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
