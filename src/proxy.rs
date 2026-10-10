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
use crate::capture::{CaptureEvent, Pending, RateLimit, Tee};
use crate::accounts::{self, token_kind, Provider};
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

/// Headers a proxy in front of the router adds about the client — its address, the name
/// it asked for, the scheme it spoke. They describe a developer's machine and network,
/// not the request, and Anthropic has no use for them: dropped before the request goes
/// upstream. An ingress sets them on every request to a hosted router; nothing sets them
/// locally, so there this changes nothing.
const FORWARDING: &[&str] = &["x-real-ip", "forwarded", "via", "x-request-id"];

fn is_forwarding(name: &str) -> bool {
    name.starts_with("x-forwarded-") || FORWARDING.contains(&name)
}

/// Ceiling on a buffered request body. Claude Code sends complete JSON with a
/// content-length, so buffering costs microseconds; this only stops a pathological
/// body from being held in memory.
const MAX_REQUEST_BODY: usize = 64 * 1024 * 1024;

pub const ACCOUNT_ID: &str = "default";

/// The refusal for an API call that presents no credential of its own, in the proxy's
/// words. Every `/v1/` path asks for one, the ones the router answers itself as well.
pub(crate) fn no_credential(headers: &HeaderMap) -> Option<axum::response::Response> {
    if headers.contains_key("authorization") || headers.contains_key("x-api-key") {
        return None;
    }
    Some((StatusCode::UNAUTHORIZED, "agent-router: no credential presented\n").into_response())
}

/// A credential being signed with, cached between requests.
#[derive(Clone)]
pub struct Active {
    pub account_id: String,
    pub label: String,
    /// Who the credential actually belonged to when it was resolved.
    pub identity: Option<String>,
    pub token: String,
    pub expires_at_ms: i64,
    /// The account's provider, which fixes how a request is signed and where it goes.
    pub provider: Provider,
    /// Where its requests go, when not its provider's default (`Account::base_url`).
    pub base_url: Option<String>,
}

impl Active {
    /// The address a request for `path_and_query` goes to: the router's `upstream` for an
    /// Anthropic account, the account's own base URL, else its provider's, for any other.
    pub(crate) fn upstream(&self, router_upstream: &str, path_and_query: &str) -> String {
        let base = self
            .base_url
            .as_deref()
            .or(self.provider.default_base_url())
            .unwrap_or(router_upstream);
        format!("{}{path_and_query}", base.trim_end_matches('/'))
    }
}

/// Where a request goes.
pub(crate) enum Route {
    /// Signed with this account's credential.
    Sign(Active),
    /// Nothing serves the model it asks for; the router answers it, and nothing goes upstream.
    NoSource(String),
}

/// Whether a path carries a model to route by. Every other `/v1/` path goes to the active
/// Anthropic account, as every path did before there was a second provider.
fn routed_by_model(path_and_query: &str) -> bool {
    let path = path_and_query.split('?').next().unwrap_or_default();
    path == "/v1/messages" || path == "/v1/messages/count_tokens"
}

type HttpsClient = Client<
    hyper_rustls::HttpsConnector<hyper_util::client::legacy::connect::HttpConnector>,
    Body,
>;

pub struct AppState {
    pub client: HttpsClient,
    pub config: Config,
    /// Each account's credential, by account id. Cached, not held: a Keychain-sourced token
    /// is refreshed by Claude Code roughly hourly, so one cached at startup would begin
    /// 401ing mid-session. Cleared outright when an account changes, so a switch takes
    /// effect on the very next request with nothing restarted.
    pub credentials: tokio::sync::RwLock<std::collections::HashMap<String, Active>>,
    /// Last quota reading; see `quota`. Keyed by account, because one account's
    /// numbers must never be shown under another's name.
    pub quota_cache: tokio::sync::RwLock<std::collections::HashMap<String, crate::quota::Cached>>,
    /// The in-progress `setup-token` sign-in, if any. One at a time.
    pub login: crate::login::Shared,
    /// The router's password, asked of callers that are not on this machine.
    pub auth: crate::auth::Auth,
    /// Whether we also listen beyond loopback, and the lever that changes it while
    /// the router runs. See `network`.
    pub network: crate::network::Switch,
    /// Phones to tell when a session is waiting on a person.
    pub push: std::sync::Arc<crate::notify::Push>,
    /// A hosted router's connected machine agents. Empty, and never used, locally.
    pub machines: std::sync::Arc<crate::machines::Machines>,
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
        let push = std::sync::Arc::new(
            crate::notify::Push::new(&config.db_path).with_keys_dir(config.push_keys_dir.clone()),
        );
        let reads = crate::api::ReadPool::new(&config.db_path);
        // Shorter than the fastest thing the dashboard polls (1.5s), so the page is no
        // less live than it was before there was a cache here.
        let memo = crate::api::Memo::new(std::time::Duration::from_millis(1000));
        Ok(Arc::new(Self {
            client,
            config,
            credentials: Default::default(),
            quota_cache: Default::default(),
            login: Default::default(),
            auth,
            network,
            push,
            machines: Default::default(),
            capture,
            reads,
            memo,
            dumps_left,
            dropped: AtomicUsize::new(0),
        }))
    }

    /// Drop every cached credential so the next request re-resolves its own.
    pub async fn invalidate(&self) {
        self.credentials.write().await.clear();
        // The quota reading belongs to the account that produced it, so a switch
        // must drop it too — otherwise the new account inherits the old one's numbers.
        self.quota_cache.write().await.clear();
    }

    /// The active Anthropic account's id and token: what a request with no model to route
    /// by is signed with, and whose quota the dashboard shows.
    pub(crate) async fn signing_credential(&self) -> anyhow::Result<(String, String)> {
        self.signing_credential_full()
            .await
            .map(|(id, _identity, token)| (id, token))
    }

    /// The active Anthropic account, the identity it resolved to, and its token.
    pub(crate) async fn signing_credential_full(
        &self,
    ) -> anyhow::Result<(String, Option<String>, String)> {
        let a = self.anthropic_signer().await?;
        Ok((a.account_id, a.identity, a.token))
    }

    async fn anthropic_signer(&self) -> anyhow::Result<Active> {
        let db = self.config.db_path.clone();
        let id = tokio::task::spawn_blocking(move || accounts::Store::new(db).active().map(|a| a.id)).await??;
        self.signer(&id).await
    }

    /// The account a request goes to: for a path that carries a model, the account that
    /// model is assigned, else its provider's active one (`models::account_for`); for any
    /// other path, the active Anthropic account.
    pub(crate) async fn route(&self, path_and_query: &str, model: Option<&str>) -> anyhow::Result<Route> {
        let Some(model) = model.filter(|_| routed_by_model(path_and_query)) else {
            return Ok(Route::Sign(self.anthropic_signer().await?));
        };
        let chosen = crate::models::account_for(&*self.reads.get()?, model)?;
        match (chosen, crate::models::provider_of(model)) {
            (Some((id, _)), _) => Ok(Route::Sign(self.signer(&id).await?)),
            // A Claude model with no Anthropic account to sign it: the refusal there has
            // always been, in its words.
            (None, Some(Provider::Anthropic)) => Ok(Route::Sign(self.anthropic_signer().await?)),
            (None, _) => Ok(Route::NoSource(model.to_string())),
        }
    }

    /// The credential to sign with for `account_id`, re-resolved when the cache does not
    /// hold it or its token is close to expiry.
    pub(crate) async fn signer(&self, account_id: &str) -> anyhow::Result<Active> {
        const RENEW_WITHIN_SECS: i64 = 300;
        {
            let held = self.credentials.read().await;
            if let Some(a) = held.get(account_id) {
                let fresh = a.expires_at_ms == 0
                    || (a.expires_at_ms - crate::db::now_ms()) / 1000 > RENEW_WITHIN_SECS;
                if fresh {
                    return Ok(a.clone());
                }
            }
        }
        let mut held = self.credentials.write().await;
        let db_path = self.config.db_path.clone();
        let id = account_id.to_string();
        let loaded = tokio::task::spawn_blocking(move || {
            let account = accounts::Store::new(db_path)
                .get(&id)?
                .ok_or_else(|| anyhow::anyhow!("no account with id {id:?}"))?;
            let resolved = accounts::resolve(&account)?;
            Ok::<_, anyhow::Error>(Active {
                account_id: account.id,
                label: account.label,
                identity: resolved.identity.or(account.email),
                token: resolved.token,
                expires_at_ms: resolved.expires_at_ms,
                provider: account.provider,
                base_url: account.base_url,
            })
        })
        .await??;
        tracing::info!(
            account = %loaded.label,
            provider = loaded.provider.as_str(),
            identity = loaded.identity.as_deref().unwrap_or("-"),
            credential = token_kind(&loaded.token),
            "signing credential resolved"
        );
        held.insert(loaded.account_id.clone(), loaded.clone());
        Ok(loaded)
    }

    /// What an upstream's answer says about the credential that signed it. A 401 is the
    /// provider no longer taking it — revoked, rotated elsewhere, replaced in a console — so
    /// that account's is forgotten, and the next request reads the stored one again instead
    /// of repeating the refusal until a restart. Only that account's: the others still sign.
    pub(crate) async fn heard(&self, account_id: &str, status: StatusCode) {
        if status == StatusCode::UNAUTHORIZED {
            self.credentials.write().await.remove(account_id);
            tracing::warn!(account = account_id, "upstream refused the credential; it will be read again for the next request");
        }
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
    if let Some(refused) = no_credential(&parts.headers) {
        tracing::warn!(path = %path_and_query, "refused: caller presented no credential");
        return Ok(refused);
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

    // The account the request goes to, by the model it asks for.
    let model = shape.as_ref().and_then(|s| s.model.clone());
    let signer = match state.route(&path_and_query, model.as_deref()).await? {
        Route::Sign(signer) => signer,
        Route::NoSource(model) => {
            tracing::warn!(path = %path_and_query, model = %model, "refused: no source serves the model");
            return Ok(no_source(&model));
        }
    };
    let upstream: Uri = signer.upstream(&state.config.upstream, &path_and_query).parse()?;
    let out = outgoing(&parts.method, &parts.headers, upstream, &signer, request_bytes.clone())?;
    let upstream_resp = state.client.request(out).await?;
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
    state.heard(&signer.account_id, resp_parts.status).await;

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
        account_id: signer.account_id.clone(),
        account_email: signer.identity.clone(),
        provider: signer.provider,
        entrypoint,
        method: parts.method.to_string(),
        path: path_and_query,
        started_at_ms,
        ttfb_ms,
        status,
        streaming_requested: shape.as_ref().map(|s| s.streaming).unwrap_or(false),
        rl,
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

/// The request that goes upstream: the client's, its forwarding headers dropped, its host
/// and its credential replaced, and its body exactly as it was sent.
fn outgoing(
    method: &axum::http::Method,
    from: &HeaderMap,
    uri: Uri,
    signer: &Active,
    body: bytes::Bytes,
) -> anyhow::Result<Request<Body>> {
    let authority = uri.authority().map(|a| a.as_str().to_owned()).unwrap_or_default();
    let mut out = Request::builder().method(method.clone()).uri(uri);
    {
        let headers = out.headers_mut().expect("builder has headers");
        copy_forwardable(from, headers);
        let forwarding: Vec<HeaderName> =
            headers.keys().filter(|n| is_forwarding(n.as_str())).cloned().collect();
        for name in forwarding {
            headers.remove(&name);
        }
        headers.insert("host", HeaderValue::from_str(&authority)?);
        sign(headers, signer)?;
    }
    Ok(out.body(Body::from(body))?)
}

/// The credential swap: whatever the client sent is discarded, and the account's goes on,
/// the way its provider takes it — Anthropic's as a bearer token, Kimi Code's as
/// `x-api-key`, as its own Claude Code guide sets it.
fn sign(headers: &mut HeaderMap, signer: &Active) -> anyhow::Result<()> {
    headers.remove("x-api-key");
    headers.remove("authorization");
    let (name, value) = match signer.provider {
        Provider::Anthropic => ("authorization", format!("Bearer {}", signer.token)),
        Provider::Kimi => ("x-api-key", signer.token.clone()),
    };
    let mut value = HeaderValue::from_str(&value)?;
    value.set_sensitive(true);
    headers.insert(name, value);
    Ok(())
}

/// The router's answer to a request for a model nothing serves, in the shape of an API
/// error so the client shows its words as they are.
fn no_source(model: &str) -> Response<Body> {
    let body = serde_json::json!({
        "type": "error",
        "error": {
            "type": "not_found_error",
            "message": format!("agent-router: no source serves the model {model:?}"),
        },
    });
    (StatusCode::NOT_FOUND, axum::Json(body)).into_response()
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

#[cfg(test)]
mod tests {
    use super::is_forwarding;

    #[test]
    fn what_a_proxy_adds_about_the_client_stays_behind() {
        for h in [
            "x-forwarded-for",
            "x-forwarded-host",
            "x-forwarded-proto",
            "x-forwarded-port",
            "x-forwarded-scheme",
            "x-real-ip",
            "forwarded",
            "via",
            "x-request-id",
        ] {
            assert!(is_forwarding(h), "{h} should be dropped");
        }
        for h in [
            "authorization",
            "anthropic-beta",
            "anthropic-version",
            "x-claude-code-session-id",
            "x-claude-code-agent-id",
            "user-agent",
            "x-app",
            "content-type",
            "x-stainless-retry-count",
        ] {
            assert!(!is_forwarding(h), "{h} must reach Anthropic");
        }
    }

    use super::*;
    use crate::api_refusals::Lab;
    use serde_json::{json, Value};

    /// Not credentials: no key of either provider looks like these.
    const WORK: &str = "sk-ant-oat01-route-work-not-a-credential";
    const HOME: &str = "sk-ant-oat01-route-home-not-a-credential";
    const KIMI: &str = "sk-kimi-route-not-a-credential";

    /// A router with two Claude accounts and a Kimi Code one, added as the dashboard adds
    /// them; `<tag>-work` is the active Anthropic one, being the first.
    async fn lab_with_accounts(tag: &str) -> Lab {
        let lab = Lab::local(tag);
        for add in [
            json!({ "label": format!("{tag} work"), "source": "token", "token": WORK }),
            json!({ "label": format!("{tag} home"), "source": "token", "token": HOME }),
            json!({ "label": format!("{tag} kimi"), "source": "token", "provider": "kimi", "token": KIMI }),
        ] {
            let (status, body) = lab.post("/api/accounts", &add.to_string()).await;
            assert_eq!(status, StatusCode::OK, "{body}");
        }
        lab
    }

    async fn signer_for(lab: &Lab, path: &str, model: &str) -> Active {
        match lab.state.route(path, Some(model)).await.unwrap() {
            Route::Sign(signer) => signer,
            Route::NoSource(model) => panic!("nothing serves {model}"),
        }
    }

    #[tokio::test]
    async fn a_request_goes_to_the_account_its_model_is_assigned_to() {
        let lab = lab_with_accounts("route-assigned").await;
        let assign = json!({ "model": "claude-opus-5-5", "account": "route-assigned-home" }).to_string();
        assert_eq!(lab.post("/api/models/assign", &assign).await.0, StatusCode::OK);

        let path = "/v1/messages?beta=true";
        let home = signer_for(&lab, path, "claude-opus-5-5").await;
        assert_eq!((home.account_id.as_str(), home.token.as_str()), ("route-assigned-home", HOME));
        assert_eq!(home.upstream(&lab.state.config.upstream, path), "https://upstream.invalid/v1/messages?beta=true");

        let kimi = signer_for(&lab, path, "k3").await;
        assert_eq!((kimi.account_id.as_str(), kimi.token.as_str(), kimi.provider), ("route-assigned-kimi", KIMI, Provider::Kimi));
        assert_eq!(kimi.upstream(&lab.state.config.upstream, path), "https://api.kimi.ai/coding/v1/messages?beta=true");
    }

    #[tokio::test]
    async fn an_unassigned_claude_model_goes_to_the_active_anthropic_account() {
        let lab = lab_with_accounts("route-unassigned").await;
        let path = "/v1/messages?beta=true";
        assert_eq!(signer_for(&lab, path, "claude-sonnet-5-5").await.account_id, "route-unassigned-work");
        // Switching the active Anthropic account takes effect on the very next request.
        assert_eq!(lab.post("/api/accounts/activate", r#"{"id":"route-unassigned-home"}"#).await.0, StatusCode::OK);
        assert_eq!(signer_for(&lab, path, "claude-sonnet-5-5").await.account_id, "route-unassigned-home");
        // A path with no model to route by goes there too.
        let Route::Sign(other) = lab.state.route("/v1/files", None).await.unwrap() else { panic!() };
        assert_eq!(other.account_id, "route-unassigned-home");
        // And a Kimi Code model still goes to the Kimi Code account.
        assert_eq!(signer_for(&lab, path, "kimi-for-coding").await.account_id, "route-unassigned-kimi");
    }

    #[tokio::test]
    async fn a_kimi_request_is_signed_with_x_api_key_and_forwarded_unchanged() {
        let lab = lab_with_accounts("route-kimi-wire").await;
        let path = "/v1/messages?beta=true";
        let signer = signer_for(&lab, path, "k3").await;

        // What Claude Code sends, behind a proxy that added its own header.
        let mut from = HeaderMap::new();
        for (name, value) in [
            ("authorization", "Bearer the-clients-own"),
            ("x-api-key", "the-clients-own"),
            ("user-agent", "claude-cli/2.1.267 (external, cli)"),
            ("anthropic-version", "2023-06-01"),
            ("anthropic-beta", "claude-code-20250219"),
            ("x-claude-code-session-id", "6f708192-0000-4000-8000-00000000000f"),
            ("content-type", "application/json"),
            ("x-forwarded-for", "10.0.0.9"),
            ("host", "127.0.0.1:8787"),
        ] {
            from.insert(HeaderName::from_static(name), HeaderValue::from_static(value));
        }
        let body = bytes::Bytes::from_static(br#"{"model":"k3","max_tokens":1,"messages":[{"role":"user","content":"hi"}]}"#);
        let uri: Uri = signer.upstream(&lab.state.config.upstream, path).parse().unwrap();
        let out = outgoing(&axum::http::Method::POST, &from, uri, &signer, body.clone()).unwrap();

        assert_eq!(out.uri().to_string(), "https://api.kimi.ai/coding/v1/messages?beta=true");
        let headers = out.headers();
        assert_eq!(headers.get("x-api-key").unwrap(), KIMI, "the account's key, as x-api-key");
        assert!(headers.get("authorization").is_none(), "no bearer token beside it");
        assert_eq!(headers.get("user-agent").unwrap(), "claude-cli/2.1.267 (external, cli)", "the client's own, as Kimi's terms ask");
        for kept in ["anthropic-version", "anthropic-beta", "x-claude-code-session-id", "content-type"] {
            assert_eq!(headers.get(kept), from.get(kept), "{kept}");
        }
        assert_eq!(headers.get("host").unwrap(), "api.kimi.ai");
        assert!(headers.get("x-forwarded-for").is_none());
        let sent = axum::body::to_bytes(out.into_body(), 1 << 20).await.unwrap();
        assert_eq!(sent, body, "the body byte for byte");
    }

    #[tokio::test]
    async fn count_tokens_is_routed_by_its_model() {
        let lab = lab_with_accounts("route-count").await;
        let path = "/v1/messages/count_tokens?beta=true";
        let kimi = signer_for(&lab, path, "k3").await;
        assert_eq!(kimi.account_id, "route-count-kimi");
        assert_eq!(kimi.upstream(&lab.state.config.upstream, path), "https://api.kimi.ai/coding/v1/messages/count_tokens?beta=true");
        let claude = signer_for(&lab, path, "claude-haiku-5-5").await;
        assert_eq!(claude.account_id, "route-count-work");
        assert_eq!(claude.upstream(&lab.state.config.upstream, path), "https://upstream.invalid/v1/messages/count_tokens?beta=true");
    }

    #[tokio::test]
    async fn a_model_no_source_serves_is_refused_naming_it() {
        use axum::extract::ConnectInfo;
        use tower::ServiceExt;

        // Only a Claude account: neither an unknown model nor a Kimi Code one has a source.
        let lab = Lab::local("route-no-source");
        let add = json!({ "label": "route-no-source work", "source": "token", "token": WORK }).to_string();
        assert_eq!(lab.post("/api/accounts", &add).await.0, StatusCode::OK);
        for model in ["gpt-9", "k3"] {
            let body = json!({ "model": model, "max_tokens": 1, "messages": [] }).to_string();
            let mut req = Request::post("/v1/messages?beta=true")
                .header("host", "127.0.0.1")
                .header("content-type", "application/json")
                .header("x-api-key", "the-clients-own")
                .body(Body::from(body))
                .unwrap();
            req.extensions_mut().insert(ConnectInfo(std::net::SocketAddr::from(([127, 0, 0, 1], 50_000))));
            let res = lab.app.clone().oneshot(req).await.unwrap();
            // Anything sent to the test's upstream, which is nowhere, would come back a 502.
            assert_eq!(res.status(), StatusCode::NOT_FOUND, "{model}");
            let answer: Value = serde_json::from_slice(&axum::body::to_bytes(res.into_body(), 1 << 20).await.unwrap()).unwrap();
            assert_eq!(
                answer,
                json!({ "type": "error", "error": { "type": "not_found_error",
                        "message": format!("agent-router: no source serves the model \"{model}\"") } })
            );
        }
    }

    #[tokio::test]
    async fn a_refusal_drops_only_the_refused_accounts_credential() {
        let lab = lab_with_accounts("route-refused").await;
        let path = "/v1/messages?beta=true";
        signer_for(&lab, path, "claude-opus-5-5").await;
        signer_for(&lab, path, "k3").await;
        let held = |lab: &Lab| {
            let lab = lab.state.clone();
            async move {
                let mut ids: Vec<String> = lab.credentials.read().await.keys().cloned().collect();
                ids.sort();
                ids
            }
        };
        assert_eq!(held(&lab).await, ["route-refused-kimi", "route-refused-work"]);

        // Kimi Code answered 200 to one and 401 to the other: only the refused one goes.
        lab.state.heard("route-refused-kimi", StatusCode::OK).await;
        assert_eq!(held(&lab).await.len(), 2);
        lab.state.heard("route-refused-kimi", StatusCode::UNAUTHORIZED).await;
        assert_eq!(held(&lab).await, ["route-refused-work"], "the Claude account keeps signing from the cache");
        // The next request reads the refused one's stored key again.
        assert_eq!(signer_for(&lab, path, "k3").await.token, KIMI);
    }

    /// A Kimi Code provider sends its key as `x-api-key` and no `Authorization`, so on a
    /// hosted router that key is the password. Whatever it carried, the request upstream is
    /// signed with the account's credential and carries no trace of it.
    #[tokio::test]
    async fn a_hosted_router_takes_its_password_as_x_api_key() {
        use axum::extract::ConnectInfo;
        use tower::ServiceExt;

        let lab = Lab::hosted("x-api-key-password").await;
        let password = lab.password.unwrap();
        let ask = |key: Option<&str>| {
            let mut req = Request::get("/api/accounts").header("host", "127.0.0.1");
            if let Some(key) = key {
                req = req.header("x-api-key", key);
            }
            let mut req = req.body(Body::empty()).unwrap();
            req.extensions_mut().insert(ConnectInfo(std::net::SocketAddr::from(([127, 0, 0, 1], 50_000))));
            lab.app.clone().oneshot(req)
        };
        assert_eq!(ask(Some(password)).await.unwrap().status(), StatusCode::OK, "the password as x-api-key is admitted");
        assert_eq!(ask(Some("not the password")).await.unwrap().status(), StatusCode::UNAUTHORIZED);
        assert_eq!(ask(None).await.unwrap().status(), StatusCode::UNAUTHORIZED);

        // Upstream, the account's credential and nothing of the password.
        let signer = Active {
            account_id: "work".into(),
            label: "work".into(),
            identity: None,
            token: WORK.into(),
            expires_at_ms: 0,
            provider: Provider::Anthropic,
            base_url: None,
        };
        let mut from = HeaderMap::new();
        from.insert("x-api-key", HeaderValue::from_static("the deployment's own"));
        let uri: Uri = "https://upstream.invalid/v1/messages".parse().unwrap();
        let out = outgoing(&axum::http::Method::POST, &from, uri, &signer, bytes::Bytes::new()).unwrap();
        assert_eq!(out.headers()["authorization"], format!("Bearer {WORK}").as_str());
        assert!(out.headers().get("x-api-key").is_none());
        assert!(out.headers().values().all(|v| v.to_str().map(|v| !v.contains(password)).unwrap_or(true)));
    }
}
