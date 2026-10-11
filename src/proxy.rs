//! Provider routing, credential replacement, and client protocol adaptation.
//!
//! Same-protocol traffic is byte faithful. Cross-protocol SSE is translated
//! incrementally; a JSON caller may require collecting a streaming-only upstream.
//! Capture reads the upstream representation independently of client adaptation.

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
use crate::accounts::{self, credential_kind, token_kind, Provider};
use crate::config::Config;
use crate::db::now_ms;
use crate::translate::Protocol;

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
    if headers.contains_key("authorization") || headers.contains_key("x-api-key") || headers.contains_key("x-agent-router-password") || headers.contains_key("x-agent-router-token") {
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
    /// Native Codex's ChatGPT plan credential uses the Codex backend.
    pub openai_chatgpt: bool,
    pub chatgpt_account_id: Option<String>,
}

impl Active {
    /// The address a request for `path_and_query` goes to: the router's `upstream` for an
    /// Anthropic account, the account's own base URL, else its provider's, for any other.
    pub(crate) fn upstream(&self, router_upstream: &str, path_and_query: &str) -> String {
        if self.provider == Provider::OpenAI && self.openai_chatgpt && self.base_url.is_none() {
            return format!("https://chatgpt.com/backend-api/codex{}", path_and_query.strip_prefix("/v1").unwrap_or(path_and_query));
        }
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
    /// An operator-defined alias supplies its service protocol and upstream model.
    Registered(Box<crate::provider_registry::RegisteredRoute>),
    /// Nothing serves the model it asks for; the router answers it, and nothing goes upstream.
    NoSource(String),
}

/// A client's explicit session header. Namespaces keep other clients' ids distinct
/// from Claude Code's transcript ids. Affinity headers are not session identity.
fn header_session_of(headers: &HeaderMap) -> Option<String> {
    let present = |name| header_str(headers, name).filter(|id| !id.is_empty());
    present("x-claude-code-session-id")
        .or_else(|| present("x-opencode-session-id").map(|id| format!("opencode:{id}")))
        .or_else(|| {
            present("thread-id").or_else(|| present("session-id"))
                .or_else(|| present("session_id")).or_else(|| present("x-codex-session-id"))
                .map(|id| format!("codex:{id}"))
        })
}

/// Prefer the client's session header to the session a Kimi Code request names in
/// its body (`RequestShape::body_session`).
pub(crate) fn session_of(header: Option<String>, shape: Option<&crate::analyze::RequestShape>) -> Option<String> {
    header.or_else(|| shape.and_then(|s| s.body_session.clone()))
}

/// Whether a path carries a model to route by. Every other `/v1/` path goes to the active
/// Anthropic account, as every path did before there was a second provider.
fn routed_by_model(path_and_query: &str) -> bool {
    let path = path_and_query.split('?').next().unwrap_or_default();
    matches!(path, "/v1/messages" | "/v1/messages/count_tokens" | "/v1/responses" | "/codex/responses" | "/v1/responses/compact" | "/codex/responses/compact")
}

fn protocol(path: &str) -> Option<Protocol> {
    match path.split('?').next()? {
        "/v1/messages" => Some(Protocol::Messages),
        "/v1/responses" | "/codex/responses" | "/v1/responses/compact" | "/codex/responses/compact" => Some(Protocol::Responses),
        _ => None,
    }
}

fn provider_protocol(provider: Provider) -> Protocol {
    match provider {
        Provider::Anthropic | Provider::Kimi => Protocol::Messages,
        Provider::OpenAI => Protocol::Responses,
    }
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
        if let Some(route)=crate::provider_registry::resolve(self,model).await? {
            return Ok(Route::Registered(Box::new(route)));
        }
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
            if let Some(a) = held.get(account_id).filter(|_| account_id != crate::codex::ACCOUNT_ID) {
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
                openai_chatgpt: resolved.openai_chatgpt,
                chatgpt_account_id: resolved.chatgpt_account_id,
            })
        })
        .await??;
        tracing::info!(
            account = %loaded.label,
            provider = loaded.provider.as_str(),
            identity = loaded.identity.as_deref().unwrap_or("-"),
            credential = credential_kind(loaded.provider, &loaded.token),
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

    if state.dumps_left.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |left| left.checked_sub(1)).is_ok() {
        dump_request(&parts.method, &path_and_query, &parts.headers);
    }

    // Layer 4: the router holds a real subscription credential, so it must not lend
    // it to anything that wanders onto the port.
    if !path_and_query.starts_with("/v1/") && protocol(&path_and_query) != Some(Protocol::Responses) {
        tracing::warn!(path = %path_and_query, "refused: not an API path");
        return Ok((StatusCode::NOT_FOUND, "agent-router: not an API path\n").into_response());
    }
    if let Some(refused) = no_credential(&parts.headers) {
        tracing::warn!(path = %path_and_query, "refused: caller presented no credential");
        return Ok(refused);
    }

    // Exact session identity, straight off the wire.
    let client_protocol = protocol(&path_and_query);
    let session_id = header_session_of(&parts.headers);
    let entrypoint = header_str(&parts.headers, "user-agent");
    // Subagents carry this; the main thread does not.
    let agent_id = header_str(&parts.headers, "x-claude-code-agent-id");

    // Claude Code sends a complete JSON body with a content-length, so collecting it
    // is effectively free and lets the capture task see what filled the context. The
    // streaming response is translated incrementally. A non-streaming client
    // collects a streaming-only upstream into the JSON envelope it asked for.
    let request_bytes = axum::body::to_bytes(body, MAX_REQUEST_BODY).await?;
    let mut shape = if client_protocol == Some(Protocol::Responses) {
        crate::responses::analyze_request(&request_bytes)
    } else {
        analyze_request(&request_bytes)
    };
    let session_id = session_of(session_id, shape.as_ref());

    // The account the request goes to, by the model it asks for.
    let model = shape.as_ref().and_then(|s| s.model.clone());
    if let Some(protocol) = client_protocol {
        if model.as_deref().is_none_or(|m| m.trim().is_empty()) {
            return Ok(translation_refusal(protocol, "a model is required"));
        }
    }
    let (signer, registered) = match state.route(&path_and_query, model.as_deref()).await? {
        Route::Sign(signer) => (signer,None),
        Route::Registered(route) => {
            if let Some(shape)=&mut shape {
                shape.service_id=Some(route.service.id.clone());
                shape.upstream_model=Some(route.model.upstream_model.clone());
            }
            (route.signer,Some((route.service,route.model)))
        },
        Route::NoSource(model) => {
            tracing::warn!(path = %path_and_query, model = %model, "refused: no source serves the model");
            return Ok(no_source(&model));
        }
    };
    let upstream_protocol = registered.as_ref().map(|(service,_)|service.protocol.bridge()).unwrap_or_else(||provider_protocol(signer.provider));
    if upstream_protocol == Protocol::Responses && path_and_query.split('?').next() == Some("/v1/messages/count_tokens") {
        return Ok((StatusCode::BAD_REQUEST, axum::Json(serde_json::json!({"type":"error","error":{"type":"invalid_request_error","message":"agent-router: Messages token counting is unavailable for OpenAI; send the Messages request to translate it"}}))).into_response());
    }
    let translating = client_protocol.is_some_and(|p| p != upstream_protocol);
    let compaction = path_and_query.split('?').next().is_some_and(|path| path.ends_with("/responses/compact"));
    if compaction && registered.as_ref().is_some_and(|(service,model)|!service.capabilities.combine(model.capabilities.as_ref()).compaction) {
        return Ok(translation_refusal(Protocol::Responses,"the registered model route does not support Responses compaction"));
    }
    if let Some((service,model))=&registered {
        let capabilities=service.capabilities.combine(model.capabilities.as_ref());
        if signer.openai_chatgpt && capabilities.output_tokens.is_some() && !compaction {
            return Ok(translation_refusal(client_protocol.unwrap_or(upstream_protocol),"configured output budgets cannot be enforced by the native Codex subscription backend; use an API-key account for this bounded route"));
        }
        if let Err(error)=crate::provider_registry::check_capabilities(&request_bytes,&capabilities) {
            return Ok(translation_refusal(client_protocol.unwrap_or(upstream_protocol),&error.to_string()));
        }
    }
    if compaction && translating {
        return Ok(translation_refusal(Protocol::Responses, "Responses compaction requires an OpenAI model; cross-provider compaction is unavailable"));
    }
    let upstream_path = match client_protocol {
        Some(_) if !translating && path_and_query.starts_with("/v1/") => path_and_query.as_str(),
        Some(_) if compaction => "/v1/responses/compact",
        Some(_) => match upstream_protocol {
            Protocol::Messages => "/v1/messages",
            Protocol::Responses => "/v1/responses",
        },
        None => &path_and_query,
    };
    let mut wire_bytes = if translating {
        match crate::translate::request(&request_bytes, client_protocol.unwrap(), upstream_protocol) {
            Ok(body) => bytes::Bytes::from(body),
            Err(e) => return Ok(translation_refusal(client_protocol.unwrap(), &e.to_string())),
        }
    } else {
        request_bytes.clone()
    };
    if let Some((service,model))=&registered {
        let mut body:serde_json::Value=serde_json::from_slice(&wire_bytes)?;
        let object=body.as_object_mut().ok_or_else(||anyhow::anyhow!("registered model request must be an object"))?;
        object.insert("model".into(),serde_json::json!(model.upstream_model));
        if let Some(limit)=service.capabilities.combine(model.capabilities.as_ref()).output_tokens.filter(|_|matches!(upstream_path.split('?').next(),Some("/v1/messages"|"/v1/responses"))) {
            let field=if upstream_protocol==Protocol::Messages {"max_tokens"} else {"max_output_tokens"};
            let budget=object.get(field).and_then(serde_json::Value::as_u64).unwrap_or(limit).min(limit);
            object.insert(field.into(),serde_json::json!(budget));
        }
        wire_bytes=bytes::Bytes::from(serde_json::to_vec(&body)?);
    }
    // The ChatGPT Codex backend streams and does not store responses. Its own CLI
    // sends these settings; a translated Messages request must do the same.
    if signer.openai_chatgpt && client_protocol.is_some() && !compaction {
        wire_bytes = match crate::codex::backend_request(&wire_bytes) {
            Ok(body) => bytes::Bytes::from(body),
            Err(e) => return Ok(translation_refusal(client_protocol.unwrap(), &e.to_string())),
        };
    }
    let upstream: Uri = signer.upstream(&state.config.upstream, upstream_path).parse()?;
    let mut out = outgoing(&parts.method, &parts.headers, upstream, &signer, wire_bytes.clone())?;
    if let Some((service,_))=&registered {
        out.headers_mut().remove("authorization");
        out.headers_mut().remove("x-api-key");
        let (name,value)=match service.auth_profile {
            crate::provider_registry::AuthProfile::Bearer=>("authorization",format!("Bearer {}",signer.token)),
            crate::provider_registry::AuthProfile::ApiKey=>("x-api-key",signer.token.clone()),
        };
        let mut value=HeaderValue::from_str(&value)?;
        value.set_sensitive(true);
        out.headers_mut().insert(name,value);
    }
    if translating || wire_bytes != request_bytes {
        out.headers_mut().remove("anthropic-beta");
        out.headers_mut().remove("anthropic-version");
        out.headers_mut().remove("content-length");
        out.headers_mut().insert("content-type", HeaderValue::from_static("application/json"));
        out.headers_mut().insert("accept-encoding", HeaderValue::from_static("identity"));
        if upstream_protocol == Protocol::Messages {
            out.headers_mut().insert("anthropic-version", HeaderValue::from_static("2023-06-01"));
            if signer.provider == Provider::Anthropic && signer.token.starts_with("sk-ant-oat") {
                out.headers_mut().insert("anthropic-beta", HeaderValue::from_static("oauth-2025-04-20"));
            }
        }
    }
    if signer.openai_chatgpt && !out.headers().contains_key("originator") {
        out.headers_mut().insert("originator", HeaderValue::from_static("agent_router"));
    }
    let upstream_resp = state.client.request(out).await?;
    let (resp_parts, resp_body) = upstream_resp.into_parts();

    let ttfb_ms = started.elapsed().as_millis() as i64;
    let request_id = header_str(&resp_parts.headers, "request-id").or_else(|| header_str(&resp_parts.headers, "x-request-id"));
    let status = resp_parts.status.as_u16() as i64;
    let rl = RateLimit::from_headers(&resp_parts.headers);
    let encoding = header_str(&resp_parts.headers, "content-encoding");
    let is_sse = header_str(&resp_parts.headers, "content-type")
        .map(|ct| ct.contains("event-stream"))
        .unwrap_or(false)
        // The Codex ChatGPT backend can omit Content-Type on successful SSE.
        // backend_request always asks it to stream; error replies remain JSON.
        || (signer.openai_chatgpt && client_protocol.is_some() && !compaction && resp_parts.status.is_success());

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
        if is_sse && !headers.contains_key("content-type") {
            headers.insert("content-type", HeaderValue::from_static("text/event-stream"));
        }
        if translating || (signer.openai_chatgpt && !shape.as_ref().is_some_and(|s| s.streaming)) {
            headers.remove("content-length");
            headers.remove("content-encoding");
        }
    }

    // The tee. Each chunk is forwarded downstream first, then copied into the
    // accumulator. Capture can only ever fall behind by dropping, never by blocking,
    // and the accumulator emits its event on Drop so a response the consumer stops
    // reading early — every content-length error body — is still recorded.
    let client_streaming = shape.as_ref().is_some_and(|s| s.streaming);
    let chat_original = parts.extensions.get::<crate::chat_completions::Original>();
    let pending = Pending {
        request_id,
        session_id,
        agent_id,
        account_id: signer.account_id.clone(),
        account_email: signer.identity.clone(),
        provider: signer.provider,
        entrypoint,
        method: parts.method.to_string(),
        path: chat_original.map(|original|original.path.clone()).unwrap_or(path_and_query),
        started_at_ms,
        ttfb_ms,
        status,
        streaming_requested: shape.as_ref().map(|s| s.streaming).unwrap_or(false),
        rl,
        request_bytes: chat_original.map(|original|original.request_bytes).unwrap_or(request_bytes.len() as i64),
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
    tee.set_upstream_protocol(upstream_protocol);

    let needs_json = client_protocol.is_some() && !client_streaming && is_sse && resp_parts.status.is_success();
    if needs_json || (translating && !is_sse) {
        let raw = match axum::body::to_bytes(Body::new(resp_body), MAX_REQUEST_BODY).await {
            Ok(raw) => raw,
            Err(error) => {
                tee.fail(error.to_string());
                return Err(error.into());
            }
        };
        tee.feed(&raw);
        let mut decoder = crate::capture::Decoder::for_encoding(encoding.as_deref());
        let mut plain = decoder.feed(&raw);
        plain.extend(decoder.finish());
        // Error envelopes already carry a portable error.message. Keep the
        // provider's HTTP status and detail; parsing an error as a successful
        // message would turn quota/auth failures into router translation errors.
        if !resp_parts.status.is_success() {
            drop(tee);
            return Ok(resp.body(Body::from(plain))?);
        }
        let translated = (|| -> anyhow::Result<Vec<u8>> {
            if is_sse {
                plain = crate::translate::collect_sse(&plain, upstream_protocol)?;
            }
            if signer.provider == Provider::Kimi && upstream_protocol == Protocol::Messages && translating {
                let mut response: serde_json::Value = serde_json::from_slice(&plain)?;
                if let Some(usage) = response.get_mut("usage").and_then(serde_json::Value::as_object_mut) {
                    let cached = usage.get("cache_read_input_tokens").and_then(serde_json::Value::as_i64).unwrap_or(0);
                    if let Some(input) = usage.get("input_tokens").and_then(serde_json::Value::as_i64) {
                        usage.insert("input_tokens".into(), serde_json::json!((input - cached).max(0)));
                    }
                }
                plain = serde_json::to_vec(&response)?;
            }
            crate::translate::response_with_request(&plain, upstream_protocol, client_protocol.unwrap(), &request_bytes)
        })();
        let translated = match translated {
            Ok(translated) => translated,
            Err(error) => {
                tee.fail(error.to_string());
                return Err(error);
            }
        };
        drop(tee);
        resp.headers_mut().expect("builder has headers").insert("content-type", HeaderValue::from_static("application/json"));
        return Ok(resp.body(Body::from(translated))?);
    }
    let translator = if translating && is_sse && resp_parts.status.is_success() {
        let mut translator = crate::translate::StreamTranslator::with_request(upstream_protocol, client_protocol.unwrap(), &request_bytes)?;
        translator.set_input_includes_cache(signer.provider == Provider::Kimi && upstream_protocol == Protocol::Messages);
        Some(translator)
    } else {
        None
    };
    let body = async_stream::stream! {
        let mut translator = translator;
        let mut stream_failed = false;
        let mut decoder = crate::capture::Decoder::for_encoding(encoding.as_deref());
        let mut stream = BodyStream::new(resp_body);
        while let Some(item) = stream.next().await {
            match item {
                Ok(frame) => {
                    let Ok(data) = frame.into_data() else { continue };
                    tee.feed(&data);
                    if let Some(translator) = &mut translator {
                        match translator.push(&decoder.feed(&data)) {
                            Ok(bytes) if !bytes.is_empty() => yield Ok::<_, std::io::Error>(bytes::Bytes::from(bytes)),
                            Ok(_) => {},
                            Err(e) => {
                                tee.fail(e.to_string());
                                stream_failed = true;
                                yield Err(std::io::Error::other(e.to_string()));
                                break;
                            },
                        }
                    } else {
                        yield Ok::<_, std::io::Error>(data);
                    }
                }
                Err(err) => {
                    tee.fail(err.to_string());
                    stream_failed = true;
                    yield Err(std::io::Error::other(err.to_string()));
                    break;
                }
            }
        }
        if let Some(translator) = translator.as_mut().filter(|_| !stream_failed) {
            let tail = translator.push(&decoder.finish()).and_then(|mut bytes| { bytes.extend(translator.finish()?); Ok(bytes) });
            match tail {
                Ok(bytes) if !bytes.is_empty() => yield Ok(bytes::Bytes::from(bytes)),
                Ok(_) => {},
                Err(e) => {
                    tee.fail(e.to_string());
                    yield Err(std::io::Error::other(e.to_string()));
                },
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
    headers.remove("x-agent-router-password");
    headers.remove("x-agent-router-token");
    headers.remove("chatgpt-account-id");
    let (name, value) = match signer.provider {
        Provider::Anthropic | Provider::OpenAI => ("authorization", format!("Bearer {}", signer.token)),
        Provider::Kimi => ("x-api-key", signer.token.clone()),
    };
    let mut value = HeaderValue::from_str(&value)?;
    value.set_sensitive(true);
    headers.insert(name, value);
    if let Some(id) = &signer.chatgpt_account_id {
        let mut value = HeaderValue::from_str(id)?;
        value.set_sensitive(true);
        headers.insert("chatgpt-account-id", value);
    }
    Ok(())
}

fn translation_refusal(protocol: Protocol, message: &str) -> Response<Body> {
    let message = message.strip_prefix("agent-router: ").unwrap_or(message);
    let error = serde_json::json!({"type":"invalid_request_error", "message": format!("agent-router: {message}")});
    let body = match protocol {
        Protocol::Messages => serde_json::json!({"type":"error", "error":error}),
        Protocol::Responses => serde_json::json!({"error":error}),
    };
    (StatusCode::BAD_REQUEST, axum::Json(body)).into_response()
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
                "authorization" | "x-api-key" | "x-agent-router-password" | "x-agent-router-token" => value
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

    #[test]
    fn explicit_client_sessions_win_over_affinity_headers() {
        let mut headers = HeaderMap::new();
        headers.insert("x-session-affinity", HeaderValue::from_static("routing-key"));
        headers.insert("x-session-id", HeaderValue::from_static("generic-id"));
        assert_eq!(header_session_of(&headers), None);
        headers.insert("session-id", HeaderValue::from_static("codex-affinity"));
        headers.insert("thread-id", HeaderValue::from_static("native-thread"));
        assert_eq!(header_session_of(&headers).as_deref(), Some("codex:native-thread"));
        headers.insert("x-opencode-session-id", HeaderValue::from_static("ses_test"));
        assert_eq!(header_session_of(&headers).as_deref(), Some("opencode:ses_test"));
        headers.insert("x-claude-code-session-id", HeaderValue::from_static("claude-session"));
        assert_eq!(header_session_of(&headers).as_deref(), Some("claude-session"));
        headers.insert("x-claude-code-session-id", HeaderValue::from_static(""));
        assert_eq!(header_session_of(&headers).as_deref(), Some("opencode:ses_test"));
    }

    /// Exercise the actual HTTP adapter against a loopback provider, without a real
    /// credential, TLS bypass in production, or access to the user's client settings.
    async fn bridge_case(client_protocol: Protocol, provider: Provider, client_streams: bool, upstream_streams: bool) {
        use axum::routing::post;
        use http_body_util::BodyExt;
        let (observed, mut requests) = tokio::sync::mpsc::channel(1);
        let model = match provider { Provider::Anthropic => "claude-test", Provider::Kimi => "k3", Provider::OpenAI => "gpt-test" };
        // Kimi's native Messages input includes its cached tokens. Anthropic's
        // input counts only fresh tokens; both requests therefore used 100 input.
        let messages_input = if provider == Provider::Kimi {100} else {40};
        let messages = json!({"id":"msg_test","type":"message","role":"assistant","model":model,"content":[{"type":"text","text":"Hello"}],"stop_reason":"end_turn","usage":{"input_tokens":messages_input,"cache_read_input_tokens":60,"output_tokens":3}});
        let responses = json!({"id":"resp_test","object":"response","model":model,"status":"completed","output":[{"id":"item_test","type":"message","role":"assistant","content":[{"type":"output_text","text":"Hello","annotations":[]}]}],"usage":{"input_tokens":100,"input_tokens_details":{"cached_tokens":60},"output_tokens":3}});
        let upstream_protocol = provider_protocol(provider);
        let upstream = match (upstream_protocol, upstream_streams) {
            (Protocol::Messages, false) => messages.to_string(),
            (Protocol::Responses, false) => responses.to_string(),
            (Protocol::Messages, true) => [
                json!({"type":"message_start","message":{"id":"msg_test","model":model,"usage":{"input_tokens":messages_input,"cache_read_input_tokens":60,"output_tokens":0}}}),
                json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}),
                json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}),
                json!({"type":"content_block_stop","index":0}),
                json!({"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":3}}),
                json!({"type":"message_stop"}),
            ].iter().map(|v| format!("data: {v}\n\n")).collect(),
            (Protocol::Responses, true) => [
                json!({"type":"response.created","response":{"id":"resp_test","model":model,"status":"in_progress","output":[]}}),
                json!({"type":"response.output_item.added","output_index":0,"item":{"id":"item_test","type":"message","role":"assistant","content":[]}}),
                json!({"type":"response.content_part.added","output_index":0,"content_index":0,"part":{"type":"output_text","text":"","annotations":[]}}),
                json!({"type":"response.output_text.delta","output_index":0,"content_index":0,"delta":"Hello"}),
                json!({"type":"response.content_part.done","output_index":0,"content_index":0,"part":{"type":"output_text","text":"Hello"}}),
                json!({"type":"response.output_item.done","output_index":0,"item":responses["output"][0]}),
                json!({"type":"response.completed","response":responses}),
            ].iter().map(|v| format!("data: {v}\n\n")).collect(),
        };
        let upstream_bytes = upstream.clone();
        let handler = move |req: Request<Body>| {
            let observed = observed.clone();
            let upstream = upstream.clone();
            async move {
                let (parts, body) = req.into_parts();
                let body = axum::body::to_bytes(body, MAX_REQUEST_BODY).await.unwrap();
                observed.send((parts.uri.path().to_owned(), parts.headers, body)).await.unwrap();
                let chunks: Vec<_> = upstream.as_bytes().chunks(7).map(|c| Ok::<_, std::io::Error>(bytes::Bytes::copy_from_slice(c))).collect();
                Response::builder().header("content-type", if upstream_streams {"text/event-stream"} else {"application/json"})
                    .header("x-request-id", "req_bridge").body(Body::from_stream(futures_util::stream::iter(chunks))).unwrap()
            }
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, axum::Router::new().route("/v1/messages", post(handler.clone())).route("/v1/responses", post(handler))).await.unwrap(); });
        let tag = format!("bridge-{}-{}-{client_streams}-{upstream_streams}", provider.as_str(), if client_protocol == Protocol::Messages {"messages"} else {"responses"});
        let lab = Lab::local(&tag);
        let conn = rusqlite::Connection::open(&lab.state.config.db_path).unwrap();
        conn.execute("INSERT INTO accounts(id,label,source,provider,active,created_at) VALUES('bridge','Bridge','token',?1,1,1)", [provider.as_str()]).unwrap();
        let config = Config { server:false,bind:([127,0,0,1],0).into(),upstream:base.clone(),db_path:lab.state.config.db_path.clone(),body_cap:65536,dump_headers:0,allow_hosts:vec![],public_url:None,push_keys_dir:None,agents_dir:None,drain:std::time::Duration::from_secs(1),stop_delay:std::time::Duration::ZERO };
        let (capture, mut captured) = tokio::sync::mpsc::channel(2);
        let (network, _) = crate::network::switch();
        let mut state = AppState::new(config, capture, network).unwrap();
        let tls = hyper_rustls::HttpsConnectorBuilder::new().with_webpki_roots().https_or_http().enable_http1().build();
        Arc::get_mut(&mut state).unwrap().client = Client::builder(TokioExecutor::new()).build(tls);
        let chatgpt = provider == Provider::OpenAI && upstream_streams && !client_streams;
        state.credentials.write().await.insert("bridge".into(), Active {account_id:"bridge".into(),label:"Bridge".into(),identity:None,token:"TEST-UPSTREAM-TOKEN".into(),expires_at_ms:0,provider,base_url:Some(base),openai_chatgpt:chatgpt,chatgpt_account_id:chatgpt.then(|| "workspace-test".into())});
        let body = match client_protocol {
            Protocol::Messages => json!({"model":model,"system":"Be helpful","metadata":{"user_id":"client-test"},"temperature":0.2,"top_p":0.9,"max_tokens":100,"stream":client_streams,"messages":[{"role":"user","content":"Hi"}]}),
            Protocol::Responses => json!({"model":model,"stream":client_streams,"input":[{"role":"user","content":[{"type":"input_text","text":"Hi"}]}]}),
        };
        let original_body = body.to_string();
        let path = if client_protocol == Protocol::Messages {"/v1/messages"} else {"/v1/responses"};
        let response = forward(state, Request::post(path).header("authorization","Bearer CLIENT-TOKEN").header("content-type","application/json")
            .header("x-api-key","CLIENT-KEY").header("thread-id","native-session").header("session-id","affinity-key").header("x-agent-router-password","ROUTER-PASSWORD").header("x-agent-router-token","ROUTER-TOKEN").body(Body::from(original_body.clone())).unwrap()).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["content-type"], if client_streams {"text/event-stream"} else {"application/json"});
        let output = response.into_body().collect().await.unwrap().to_bytes();
        if client_protocol == upstream_protocol && client_streams == upstream_streams && !chatgpt {
            assert_eq!(output.as_ref(), upstream_bytes.as_bytes(), "native response bytes must remain intact for {}", provider.as_str());
        }
        let reply: Value = if client_streams { serde_json::from_slice(&crate::translate::collect_sse(&output, client_protocol).unwrap()).unwrap() } else { serde_json::from_slice(&output).unwrap() };
        assert_eq!(reply["model"], model);
        assert_eq!(reply["usage"]["output_tokens"], 3);
        if client_protocol == Protocol::Messages {
            assert_eq!(reply["content"][0]["text"], "Hello");
            assert_eq!(reply["usage"]["input_tokens"], messages_input, "native Messages usage must retain the provider's original input count");
            assert_eq!(reply["usage"]["cache_read_input_tokens"], 60);
        } else {
            assert_eq!(reply["output"][0]["content"][0]["text"], "Hello");
            assert_eq!(reply["usage"]["input_tokens"], 100);
            assert_eq!(reply["usage"]["input_tokens_details"]["cached_tokens"], 60);
        }
        let (path, headers, sent_bytes) = requests.recv().await.unwrap();
        let body: Value = serde_json::from_slice(&sent_bytes).unwrap();
        assert_eq!(path, if upstream_protocol == Protocol::Messages {"/v1/messages"} else {"/v1/responses"});
        if provider == Provider::Kimi {
            assert_eq!(headers["x-api-key"], "TEST-UPSTREAM-TOKEN");
            assert!(headers.get("authorization").is_none(), "Kimi must receive only the selected account's x-api-key");
        } else {
            assert_eq!(headers["authorization"], "Bearer TEST-UPSTREAM-TOKEN");
            assert!(headers.get("x-api-key").is_none(), "the client key must never accompany the selected bearer token");
        }
        assert!(headers.get("x-agent-router-password").is_none());
        assert!(headers.get("x-agent-router-token").is_none());
        if client_protocol == upstream_protocol && !chatgpt {
            assert_eq!(sent_bytes.as_ref(), original_body.as_bytes(), "native request bytes must remain intact for {}", provider.as_str());
        }
        assert_eq!(body["model"], model);
        assert!(body.get(if upstream_protocol == Protocol::Messages {"messages"} else {"input"}).is_some());
        if chatgpt {
            assert_eq!(headers["chatgpt-account-id"], "workspace-test");
            assert_eq!(body["stream"], true);
            assert_eq!(body["store"], false);
            assert_eq!(body["input"][0]["role"], "developer");
            assert_eq!(body["input"][0]["content"][0]["text"], "Be helpful");
            for field in ["instructions", "metadata", "temperature", "top_p", "max_output_tokens"] { assert!(body.get(field).is_none(), "{field}"); }
        }
        let captured = captured.recv().await.unwrap();
        assert_eq!(captured.provider, provider);
        assert_eq!(captured.model.as_deref(), Some(model));
        assert_eq!(captured.usage.input_tokens, 40);
        assert_eq!(captured.usage.prompt_tokens(), 100);
        assert_eq!(captured.usage.cache_read_tokens, 60);
        assert_eq!(captured.usage.output_tokens, 3);
        assert_eq!(captured.request_id.as_deref(), Some("req_bridge"));
        assert_eq!(captured.session_id.as_deref(), Some("codex:native-session"));
        server.abort();
    }

    #[tokio::test]
    async fn both_clients_use_either_protocol_through_the_http_proxy() {
        for provider in [Provider::Anthropic, Provider::OpenAI, Provider::Kimi] {
            for client in [Protocol::Messages, Protocol::Responses] {
                for streams in [false, true] {
                    bridge_case(client, provider, streams, streams).await;
                }
            }
        }
        // ChatGPT's backend always streams, even for a JSON Messages caller.
        bridge_case(Protocol::Messages, Provider::OpenAI, false, true).await;
    }

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
            Route::Registered(route) => route.signer,
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
            openai_chatgpt: false,
            chatgpt_account_id: None,
        };
        let mut from = HeaderMap::new();
        from.insert("x-api-key", HeaderValue::from_static("the deployment's own"));
        let uri: Uri = "https://upstream.invalid/v1/messages".parse().unwrap();
        let out = outgoing(&axum::http::Method::POST, &from, uri, &signer, bytes::Bytes::new()).unwrap();
        assert_eq!(out.headers()["authorization"], format!("Bearer {WORK}").as_str());
        assert!(out.headers().get("x-api-key").is_none());
        assert!(out.headers().values().all(|v| v.to_str().map(|v| !v.contains(password)).unwrap_or(true)));
    }

    // An explicitly invoked smoke with a real CLI and local fake model servers.
    // Kept outside the ordinary suite: no installed client or network download required.
    #[tokio::test]
    async fn translated_response_failures_survive_capture_storage() {
        use axum::routing::post;

        for (case, client_streams, upstream_streams) in [
            ("truncated-stream", true, true),
            ("truncated-json", false, true),
            ("unsupported-json", false, false),
        ] {
            let lab = Lab::local(&format!("translation-capture-{case}"));
            let db_path = lab.state.config.db_path.clone();
            rusqlite::Connection::open(&db_path)
                .unwrap()
                .execute("INSERT INTO accounts(id,label,source,provider,active,created_at) VALUES('openai','OpenAI','token','openai',1,1)", [])
                .unwrap();
            let request_id = format!("req_capture_{case}");
            let reply = if upstream_streams {
                [
                    json!({"type":"response.created","response":{"id":"resp_truncated","model":"gpt-test","status":"in_progress","output":[],"usage":{"input_tokens":12,"input_tokens_details":{"cached_tokens":4},"output_tokens":0}}}),
                    json!({"type":"response.output_item.added","output_index":0,"item":{"id":"msg_truncated","type":"message","role":"assistant","content":[]}}),
                    json!({"type":"response.content_part.added","output_index":0,"content_index":0,"part":{"type":"output_text","text":"","annotations":[]}}),
                    json!({"type":"response.output_text.delta","output_index":0,"content_index":0,"delta":"Partial answer"}),
                ]
                .iter()
                .map(|event| format!("data: {event}\n\n"))
                .collect::<String>()
            } else {
                json!({"id":"resp_unsupported","object":"response","model":"gpt-test","status":"completed","output":[{"type":"computer_call","id":"call_unsupported","call_id":"call_unsupported","action":{"type":"screenshot"}}],"usage":{"input_tokens":12,"output_tokens":1}}).to_string()
            };
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let upstream = format!("http://{}", listener.local_addr().unwrap());
            let fake = axum::Router::new().route("/v1/responses", post(move |req: Request<Body>| {
                let reply = reply.clone();
                let request_id = request_id.clone();
                async move {
                    assert_eq!(req.headers()["authorization"], "Bearer TEST-UPSTREAM");
                    let input = axum::body::to_bytes(req.into_body(), MAX_REQUEST_BODY).await.unwrap();
                    let input: Value = serde_json::from_slice(&input).unwrap();
                    assert_eq!(input["model"], "gpt-test");
                    let chunks = async_stream::stream! {
                        // Let the HTTP client receive the streaming headers and
                        // partial content before the provider ends without a terminal event.
                        yield Ok::<_, std::io::Error>(bytes::Bytes::from(reply));
                        if upstream_streams {
                            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                        }
                    };
                    Response::builder()
                        .header("content-type", if upstream_streams {"text/event-stream"} else {"application/json"})
                        .header("x-request-id", request_id)
                        .body(Body::from_stream(chunks))
                        .unwrap()
                }
            }));
            let fake_server = tokio::spawn(async move { axum::serve(listener, fake).await.unwrap() });
            let config = Config {
                server: true, bind: ([127, 0, 0, 1], 0).into(), upstream: upstream.clone(),
                db_path: db_path.clone(), body_cap: 65536, dump_headers: 0, allow_hosts: vec![],
                public_url: None, push_keys_dir: None, agents_dir: None,
                drain: std::time::Duration::from_secs(1), stop_delay: std::time::Duration::ZERO,
            };
            let (capture, mut captured) = tokio::sync::mpsc::channel(2);
            let (network, _) = crate::network::switch();
            let mut state = AppState::new(config, capture, network).unwrap();
            let tls = hyper_rustls::HttpsConnectorBuilder::new().with_webpki_roots().https_or_http().enable_http1().build();
            Arc::get_mut(&mut state).unwrap().client = Client::builder(TokioExecutor::new()).build(tls);
            state.auth.pin(&db_path, NATIVE_SMOKE_ADMIN_PASSWORD).await.unwrap();
            state.credentials.write().await.insert("openai".into(), Active {
                account_id: "openai".into(), label: "OpenAI".into(), identity: None,
                token: "TEST-UPSTREAM".into(), expires_at_ms: 0, provider: Provider::OpenAI,
                base_url: Some(upstream), openai_chatgpt: false, chatgpt_account_id: None,
            });
            let http = state.client.clone();
            let app = crate::app(state);
            let (_, token) = native_client_token(&app, &db_path, "Translation failure regression").await;
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let router = format!("http://{}", listener.local_addr().unwrap());
            let router_server = tokio::spawn(async move {
                axum::serve(listener, app.into_make_service_with_connect_info::<std::net::SocketAddr>()).await.unwrap()
            });
            let body = json!({"model":"gpt-test","messages":[{"role":"user","content":"Hello"}],"max_tokens":32,"stream":client_streams});
            let response = http.request(Request::post(format!("{router}/v1/messages"))
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .body(Body::from(body.to_string())).unwrap()).await.unwrap();
            if client_streams {
                assert_eq!(response.status(), StatusCode::OK);
                assert!(axum::body::to_bytes(Body::new(response.into_body()), 65536).await.is_err(), "the client must observe the truncated stream");
            } else {
                assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
                let bytes = axum::body::to_bytes(Body::new(response.into_body()), 65536).await.unwrap();
                assert!(!bytes.is_empty());
            }
            let event = tokio::time::timeout(std::time::Duration::from_secs(2), captured.recv()).await.unwrap().unwrap();
            let expected = if upstream_streams {"upstream stream ended before its terminal event"} else {"cannot translate Responses output item"};
            let error = event.error.as_deref().expect("a failed translation must be captured as an error");
            assert!(error.contains(expected), "{case}: {error}");
            assert_eq!(event.path, "/v1/messages");
            assert_eq!(event.status, 200, "capture keeps the provider's HTTP status alongside the translation error");
            let request_id = event.request_id.clone().unwrap();
            crate::pricing::ensure_table(&rusqlite::Connection::open(&db_path).unwrap()).unwrap();
            let mut db = crate::db::Db::open(&db_path).unwrap();
            db.record(&event).unwrap();
            drop(db);
            let conn = rusqlite::Connection::open(&db_path).unwrap();
            let stored: (String, i64, String, String) = conn.query_row(
                "SELECT error,status,path,provider FROM requests WHERE request_id=?1",
                [request_id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            ).unwrap();
            assert_eq!(stored, (error.to_owned(), 200, "/v1/messages".into(), "openai".into()));
            router_server.abort();
            fake_server.abort();
        }
    }

    include!("../scripts/opencode-routing.rs");
    include!("../scripts/codex-routing.rs");
    include!("../scripts/claude-routing.rs");
    include!("../scripts/live-client-routing.rs");
    include!("../scripts/openai-sdk-routing.rs");
}
