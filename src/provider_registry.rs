//! Explicit services and client-visible model routes.

use std::sync::Arc;

use anyhow::{anyhow, bail, Context, Result};
use axum::extract::{Path as Params, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::accounts::{Provider, Source};
use crate::proxy::{Active, AppState};
use crate::refusal::{failed, refuse, JsonBody};
use crate::translate::Protocol;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WireProtocol {
    Messages,
    Responses,
}

impl WireProtocol {
    pub fn bridge(self) -> Protocol {
        match self {
            Self::Messages => Protocol::Messages,
            Self::Responses => Protocol::Responses,
        }
    }
    pub fn of(provider: Provider) -> Self {
        if provider == Provider::OpenAI {
            Self::Responses
        } else {
            Self::Messages
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuthProfile {
    Bearer,
    ApiKey,
}

fn yes() -> bool {
    true
}

/// Protocol capabilities configured by the operator, rather than model entitlement.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Capabilities {
    #[serde(default = "yes")]
    pub tools: bool,
    #[serde(default = "yes")]
    pub images: bool,
    #[serde(default = "yes")]
    pub reasoning: bool,
    #[serde(default)]
    pub compaction: bool,
    #[serde(default)]
    pub context_tokens: Option<u64>,
    #[serde(default)]
    pub output_tokens: Option<u64>,
}

impl Default for Capabilities {
    fn default() -> Self {
        Self {
            tools: true,
            images: true,
            reasoning: true,
            compaction: false,
            context_tokens: None,
            output_tokens: None,
        }
    }
}

impl Capabilities {
    fn valid(&self) -> Result<()> {
        for limit in [self.context_tokens, self.output_tokens]
            .into_iter()
            .flatten()
        {
            if limit == 0 || limit > 100_000_000 {
                bail!("token limits must be between 1 and 100,000,000");
            }
        }
        Ok(())
    }
    pub fn combine(&self, model: Option<&Self>) -> Self {
        let Some(model) = model else {
            return self.clone();
        };
        let limit = |a: Option<u64>, b: Option<u64>| match (a, b) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        Self {
            tools: self.tools && model.tools,
            images: self.images && model.images,
            reasoning: self.reasoning && model.reasoning,
            compaction: self.compaction && model.compaction,
            context_tokens: limit(self.context_tokens, model.context_tokens),
            output_tokens: limit(self.output_tokens, model.output_tokens),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Service {
    pub id: String,
    pub label: String,
    pub provider: Provider,
    pub base_url: String,
    pub protocol: WireProtocol,
    pub auth_profile: AuthProfile,
    #[serde(default)]
    pub capabilities: Capabilities,
    #[serde(default)]
    pub builtin: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelRoute {
    pub alias: String,
    pub service: String,
    pub upstream_model: String,
    pub account_id: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub capabilities: Option<Capabilities>,
}

pub struct RegisteredRoute {
    pub signer: Active,
    pub service: Service,
    pub model: ModelRoute,
}

pub fn builtins() -> Vec<Service> {
    [Provider::Anthropic, Provider::OpenAI, Provider::Kimi]
        .into_iter()
        .map(|provider| {
            let protocol = WireProtocol::of(provider);
            Service {
                id: provider.as_str().into(),
                label: provider.name().into(),
                provider,
                base_url: provider
                    .default_base_url()
                    .unwrap_or("https://api.anthropic.com")
                    .into(),
                protocol,
                auth_profile: if provider == Provider::Kimi {
                    AuthProfile::ApiKey
                } else {
                    AuthProfile::Bearer
                },
                capabilities: Capabilities {
                    compaction: protocol == WireProtocol::Responses,
                    ..Default::default()
                },
                builtin: true,
            }
        })
        .collect()
}

pub fn services(conn: &Connection) -> Result<Vec<Service>> {
    let mut out = builtins();
    if !has_table(conn, "provider_services")? {
        return Ok(out);
    }
    let mut query = conn.prepare("SELECT definition FROM provider_services ORDER BY id")?;
    for row in query.query_map([], |r| r.get::<_, String>(0))? {
        let mut service: Service =
            serde_json::from_str(&row?).context("stored service definition is invalid")?;
        if out.iter().any(|existing| existing.id == service.id) {
            bail!("stored service conflicts with a builtin service")
        }
        service.builtin = false;
        out.push(service);
    }
    Ok(out)
}

pub fn service(conn: &Connection, id: &str) -> Result<Option<Service>> {
    Ok(services(conn)?.into_iter().find(|service| service.id == id))
}

pub fn model_routes(conn: &Connection) -> Result<Vec<ModelRoute>> {
    if !has_table(conn, "provider_model_routes")? {
        return Ok(Vec::new());
    }
    let mut query = conn.prepare("SELECT definition FROM provider_model_routes ORDER BY alias")?;
    let rows = query.query_map([], |r| r.get::<_, String>(0))?;
    let out = rows
        .map(|row| serde_json::from_str(&row?).context("stored model route is invalid"))
        .collect();
    out
}

pub fn model_route(conn: &Connection, alias: &str) -> Result<Option<ModelRoute>> {
    if !has_table(conn, "provider_model_routes")? {
        return Ok(None);
    }
    let row: Option<String> = conn
        .query_row(
            "SELECT definition FROM provider_model_routes WHERE alias=?1",
            [alias],
            |r| r.get(0),
        )
        .optional()?;
    row.map(|row| serde_json::from_str(&row).context("stored model route is invalid"))
        .transpose()
}

pub fn metadata(
    conn: &Connection,
    model: &str,
    provider: Provider,
) -> Result<(Service, Capabilities)> {
    if let Some(route) = model_route(conn, model)? {
        let service =
            service(conn, &route.service)?.context("registered model service is unavailable")?;
        let capabilities = service.capabilities.combine(route.capabilities.as_ref());
        return Ok((service, capabilities));
    }
    let service = builtins()
        .into_iter()
        .find(|service| service.provider == provider)
        .unwrap();
    let capabilities = service.capabilities.clone();
    Ok((service, capabilities))
}

pub fn check_capabilities(body: &[u8], capabilities: &Capabilities) -> Result<()> {
    let request: Value =
        serde_json::from_slice(body).map_err(|_| anyhow!("model request must be valid JSON"))?;
    fn tool_history(value: &Value) -> bool {
        match value {
            Value::Array(items) => items.iter().any(tool_history),
            Value::Object(item) => {
                item.get("type")
                    .and_then(Value::as_str)
                    .is_some_and(|kind| {
                        matches!(
                            kind,
                            "tool_use"
                                | "tool_result"
                                | "function_call"
                                | "function_call_output"
                                | "custom_tool_call"
                                | "custom_tool_call_output"
                        )
                    })
                    || item.values().any(tool_history)
            }
            _ => false,
        }
    }
    let tool_choice = request
        .get("tool_choice")
        .filter(|choice| !choice.is_null())
        .is_some_and(|choice| {
            choice.as_str() != Some("none")
                && choice.get("type").and_then(Value::as_str) != Some("none")
        });
    if !capabilities.tools
        && (request
            .get("tools")
            .and_then(Value::as_array)
            .is_some_and(|tools| !tools.is_empty())
            || tool_choice
            || [request.get("input"), request.get("messages")]
                .into_iter()
                .flatten()
                .any(tool_history))
    {
        bail!("the registered model route declares tool use unavailable");
    }
    fn images(value: &Value) -> bool {
        match value {
            Value::Array(items) => items.iter().any(images),
            Value::Object(item) => {
                item.get("type")
                    .and_then(Value::as_str)
                    .is_some_and(|kind| matches!(kind, "image" | "input_image" | "image_url"))
                    || item.values().any(images)
            }
            _ => false,
        }
    }
    if !capabilities.images
        && [request.get("input"), request.get("messages")]
            .into_iter()
            .flatten()
            .any(images)
    {
        bail!("the registered model route declares image inputs unavailable");
    }
    let thinking = request
        .get("thinking")
        .and_then(|thinking| thinking.get("type"))
        .and_then(Value::as_str)
        .is_some_and(|kind| kind != "disabled");
    let reasoning = request
        .get("reasoning")
        .filter(|reasoning| !reasoning.is_null())
        .is_some_and(|reasoning| reasoning.get("effort").and_then(Value::as_str) != Some("none"));
    if !capabilities.reasoning && (thinking || reasoning) {
        bail!("the registered model route declares reasoning unavailable");
    }
    if let Some(limit) = capabilities.output_tokens {
        for budget in [request.get("max_tokens"), request.get("max_output_tokens")]
            .into_iter()
            .flatten()
            .filter_map(Value::as_u64)
        {
            if budget > limit {
                bail!("requested output budget exceeds the registered model route's {limit}-token limit");
            }
        }
    }
    Ok(())
}

fn has_table(conn: &Connection, name: &str) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
        [name],
        |r| r.get(0),
    )
}

fn id(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 64
        || !value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
    {
        bail!("service ID must use 1–64 lowercase letters, digits, underscores or hyphens");
    }
    Ok(())
}

fn model_name(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 200
        || value.trim() != value
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-./:".contains(&b))
    {
        bail!("model names must use 1–200 letters, digits, underscores, hyphens, dots, slashes or colons");
    }
    Ok(())
}

fn label(value: &str) -> Result<()> {
    if value.trim().is_empty() || value.chars().count() > 100 || value.chars().any(char::is_control)
    {
        bail!("labels must use 1–100 visible characters");
    }
    Ok(())
}

fn endpoint(value: &str) -> Result<()> {
    let uri: axum::http::Uri = value
        .parse()
        .map_err(|_| anyhow!("service base URL is invalid"))?;
    if uri.scheme_str() != Some("https")
        || uri.host().is_none()
        || uri.query().is_some()
        || value.contains('#')
        || uri.authority().is_some_and(|a| a.as_str().contains('@'))
    {
        bail!("service base URL must use HTTPS without credentials, query or fragment");
    }
    if uri.path().trim_end_matches('/').ends_with("/v1") {
        bail!("service base URL must omit the final /v1");
    }
    Ok(())
}

fn put_service(conn: &Connection, mut service: Service) -> Result<Service> {
    id(&service.id)?;
    label(&service.label)?;
    endpoint(&service.base_url)?;
    service.capabilities.valid()?;
    if builtins().iter().any(|builtin| builtin.id == service.id) {
        bail!("builtin services cannot be replaced");
    }
    if service.protocol == WireProtocol::Messages && service.capabilities.compaction {
        bail!("Responses compaction requires a Responses service");
    }
    for route in model_routes(conn)?
        .into_iter()
        .filter(|route| route.service == service.id)
    {
        let provider: Option<String> = conn
            .query_row(
                "SELECT provider FROM accounts WHERE id=?1",
                [&route.account_id],
                |r| r.get(0),
            )
            .optional()?;
        if provider.as_deref() != Some(service.provider.as_str()) {
            bail!("service provider cannot change while existing model routes select incompatible accounts");
        }
    }
    service.base_url = service.base_url.trim_end_matches('/').into();
    service.builtin = false;
    conn.execute("INSERT INTO provider_services(id,definition) VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET definition=excluded.definition",
        rusqlite::params![service.id,serde_json::to_string(&service)?])?;
    Ok(service)
}

pub(crate) fn put_model(conn: &Connection, route: ModelRoute) -> Result<ModelRoute> {
    model_name(&route.alias)?;
    model_name(&route.upstream_model)?;
    if let Some(display_name) = &route.display_name {
        label(display_name)?;
    }
    if let Some(capabilities) = &route.capabilities {
        capabilities.valid()?;
    }
    let service = service(conn, &route.service)?.context("model route names an unknown service")?;
    let account: Option<(String, String)> = conn
        .query_row(
            "SELECT provider,source FROM accounts WHERE id=?1",
            [&route.account_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let (provider, source) = account.context("model route names an unknown account")?;
    if provider != service.provider.as_str() {
        bail!("the selected account belongs to a different provider");
    }
    if !service.builtin && source != Source::Token.as_str() {
        bail!("native client logins can only use their builtin service; custom services need an API-key account");
    }
    conn.execute("INSERT INTO provider_model_routes(alias,definition) VALUES(?1,?2) ON CONFLICT(alias) DO UPDATE SET definition=excluded.definition",
        rusqlite::params![route.alias,serde_json::to_string(&route)?])?;
    Ok(route)
}

pub async fn resolve(state: &AppState, alias: &str) -> Result<Option<RegisteredRoute>> {
    let selected = {
        let conn = state.reads.get()?;
        model_route(&conn, alias)?
    };
    let Some(model) = selected else {
        return Ok(None);
    };
    let service = {
        let conn = state.reads.get()?;
        service(&conn, &model.service)?
            .context("the registered model's service no longer exists")?
    };
    let account = crate::accounts::Store::new(state.config.db_path.clone())
        .get(&model.account_id)?
        .context("the registered model's account no longer exists")?;
    if account.provider != service.provider {
        bail!("registered model account and service providers differ");
    }
    let mut signer = state.signer(&model.account_id).await?;
    if !service.builtin {
        if account.source != Source::Token
            || signer.openai_chatgpt
            || signer.token.starts_with("sk-ant-oat")
        {
            bail!("native subscription credentials cannot be redirected to a custom service; use an API-key account");
        }
        signer.base_url = Some(service.base_url.clone());
    }
    Ok(Some(RegisteredRoute {
        signer,
        service,
        model,
    }))
}

fn success(body: Value, status: StatusCode) -> Response {
    let mut response = (status, Json(body)).into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    response
}

pub async fn status(State(state): State<Arc<AppState>>) -> Response {
    let result = tokio::task::spawn_blocking(move || -> Result<Value> {
        let conn = state.reads.get()?;
        Ok(json!({"services":services(&conn)?,"models":model_routes(&conn)?}))
    })
    .await;
    match result {
        Ok(Ok(body)) => success(body, StatusCode::OK),
        Ok(Err(e)) => failed(e),
        Err(e) => failed(e),
    }
}

pub async fn add_service(
    State(state): State<Arc<AppState>>,
    JsonBody(service): JsonBody<Service>,
) -> Response {
    let result = tokio::task::spawn_blocking(move || {
        let conn = Connection::open(&state.config.db_path)?;
        let transaction = conn.unchecked_transaction()?;
        let service = put_service(&transaction, service)?;
        transaction.commit()?;
        Ok::<_, anyhow::Error>(service)
    })
    .await;
    match result {
        Ok(Ok(service)) => success(json!({"ok":true,"service":service}), StatusCode::CREATED),
        Ok(Err(e)) => refuse(StatusCode::BAD_REQUEST, e.to_string()),
        Err(e) => failed(e),
    }
}

pub async fn add_model(
    State(state): State<Arc<AppState>>,
    JsonBody(model): JsonBody<ModelRoute>,
) -> Response {
    let result = tokio::task::spawn_blocking(move || {
        let conn = Connection::open(&state.config.db_path)?;
        let transaction = conn.unchecked_transaction()?;
        let model = put_model(&transaction, model)?;
        transaction.commit()?;
        Ok::<_, anyhow::Error>(model)
    })
    .await;
    match result {
        Ok(Ok(model)) => success(json!({"ok":true,"model":model}), StatusCode::CREATED),
        Ok(Err(e)) => refuse(StatusCode::BAD_REQUEST, e.to_string()),
        Err(e) => failed(e),
    }
}

pub async fn remove_service(
    State(state): State<Arc<AppState>>,
    Params(id): Params<String>,
) -> Response {
    let result = tokio::task::spawn_blocking(move || -> Result<bool> {
        let conn = Connection::open(&state.config.db_path)?;
        let transaction = conn.unchecked_transaction()?;
        if builtins().iter().any(|s| s.id == id) {
            bail!("builtin services cannot be removed");
        }
        if model_routes(&transaction)?.iter().any(|m| m.service == id) {
            bail!("remove this service's model routes before removing the service");
        }
        let removed = transaction.execute("DELETE FROM provider_services WHERE id=?1", [id])? > 0;
        transaction.commit()?;
        Ok(removed)
    })
    .await;
    match result {
        Ok(Ok(true)) => success(json!({"ok":true}), StatusCode::OK),
        Ok(Ok(false)) => refuse(StatusCode::NOT_FOUND, "service not found"),
        Ok(Err(e)) => refuse(StatusCode::CONFLICT, e.to_string()),
        Err(e) => failed(e),
    }
}

pub async fn remove_model(
    State(state): State<Arc<AppState>>,
    Params(alias): Params<String>,
) -> Response {
    let result = tokio::task::spawn_blocking(move || -> Result<bool> {
        let conn = Connection::open(&state.config.db_path)?;
        Ok(conn.execute("DELETE FROM provider_model_routes WHERE alias=?1", [alias])? > 0)
    })
    .await;
    match result {
        Ok(Ok(true)) => success(json!({"ok":true}), StatusCode::OK),
        Ok(Ok(false)) => refuse(StatusCode::NOT_FOUND, "model route not found"),
        Ok(Err(e)) => failed(e),
        Err(e) => failed(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api_refusals::Lab;
    use axum::body::Body;
    use axum::extract::ConnectInfo;
    use axum::http::Request;
    use axum::routing::post;
    use tower::ServiceExt;

    async fn account(lab: &Lab, id: &str, provider: Provider, source: Source, credential: &str) {
        Connection::open(&lab.state.config.db_path).unwrap().execute("INSERT INTO accounts(id,label,source,provider,active,created_at) VALUES(?1,?1,?2,?3,1,1)",rusqlite::params![id,source.as_str(),provider.as_str()]).unwrap();
        lab.state.credentials.write().await.insert(
            id.into(),
            Active {
                account_id: id.into(),
                label: id.into(),
                identity: None,
                token: credential.into(),
                expires_at_ms: i64::MAX,
                provider,
                base_url: None,
                openai_chatgpt: false,
                chatgpt_account_id: None,
            },
        );
    }

    async fn add_service(
        lab: &Lab,
        id: &str,
        provider: Provider,
        protocol: &str,
        auth_profile: &str,
    ) -> Service {
        let (status,body)=lab.post("/api/provider-registry/services",&json!({"id":id,"label":id,"provider":provider.as_str(),"base_url":format!("https://{id}.invalid"),"protocol":protocol,"auth_profile":auth_profile}).to_string()).await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        serde_json::from_value(body["service"].clone()).unwrap()
    }

    async fn add_model(lab: &Lab, alias: &str, service: &str, account: &str) {
        let (status,body)=lab.post("/api/provider-registry/models",&json!({"alias":alias,"service":service,"upstream_model":"identical-model","account_id":account}).to_string()).await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
    }

    async fn delete(lab: &Lab, path: &str) -> (StatusCode, Value) {
        let mut request = Request::delete(path)
            .header("host", "127.0.0.1")
            .body(Body::empty())
            .unwrap();
        request
            .extensions_mut()
            .insert(ConnectInfo(std::net::SocketAddr::from((
                [127, 0, 0, 1],
                50000,
            ))));
        let response = lab.app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
            .await
            .unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    #[tokio::test]
    async fn registry_exposes_builtin_service_metadata_and_accepts_named_services() {
        let lab = Lab::local("provider-registry-contract");
        let (status, body) = lab.get("/api/provider-registry").await;
        assert_eq!(status, StatusCode::OK, "{body}");
        for id in ["anthropic", "openai", "kimi"] {
            assert!(body["services"]
                .as_array()
                .unwrap()
                .iter()
                .any(|s| s["id"] == id));
        }
        let (status,body) = lab.post("/api/provider-registry/services",&json!({
            "id":"my-service","label":"My service","provider":"openai","base_url":"https://service.invalid",
            "protocol":"responses","auth_profile":"bearer"
        }).to_string()).await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        assert_eq!(body["service"]["id"], "my-service");
    }

    #[tokio::test]
    async fn aliases_override_name_inference_and_preserve_distinct_services_and_accounts() {
        let lab = Lab::local("provider-registry-aliases");
        account(
            &lab,
            "first-account",
            Provider::OpenAI,
            Source::Token,
            "FIRST-SECRET",
        )
        .await;
        account(
            &lab,
            "second-account",
            Provider::OpenAI,
            Source::Token,
            "SECOND-SECRET",
        )
        .await;
        add_service(
            &lab,
            "first-service",
            Provider::OpenAI,
            "messages",
            "api_key",
        )
        .await;
        add_service(
            &lab,
            "second-service",
            Provider::OpenAI,
            "responses",
            "bearer",
        )
        .await;
        add_model(&lab, "claude-misleading", "first-service", "first-account").await;
        add_model(&lab, "k3", "second-service", "second-account").await;
        for (alias, service, account, protocol) in [
            (
                "claude-misleading",
                "first-service",
                "first-account",
                WireProtocol::Messages,
            ),
            (
                "k3",
                "second-service",
                "second-account",
                WireProtocol::Responses,
            ),
        ] {
            let crate::proxy::Route::Registered(route) =
                lab.state.route("/v1/responses", Some(alias)).await.unwrap()
            else {
                panic!("alias did not select a registered route")
            };
            assert_eq!(route.model.upstream_model, "identical-model");
            assert_eq!(route.service.id, service);
            assert_eq!(route.service.protocol, protocol);
            assert_eq!(route.signer.account_id, account);
            assert_eq!(
                route.signer.base_url.as_deref(),
                Some(format!("https://{service}.invalid").as_str())
            );
            let conn = Connection::open(&lab.state.config.db_path).unwrap();
            assert_eq!(
                crate::models::account_for(&conn, alias).unwrap(),
                Some((account.into(), Provider::OpenAI))
            );
            let catalogue = crate::models::catalogue(&conn).unwrap();
            assert_eq!(
                catalogue.iter().find(|m| m.id == alias).unwrap().provider,
                Provider::OpenAI
            );
        }
        let restarted = lab.restarted().await;
        // Restart coverage reads only the isolated database; fixture credentials are
        // cache-only and must never fall through to the user's credential store.
        let restarted_conn = restarted.state.reads.get().unwrap();
        assert_eq!(
            model_route(&restarted_conn, "k3").unwrap().unwrap().service,
            "second-service"
        );
        let conn = Connection::open(&lab.state.config.db_path).unwrap();
        assert_eq!(model_routes(&conn).unwrap().len(), 2);
        assert!(services(&conn)
            .unwrap()
            .iter()
            .any(|s| s.id == "first-service"));
    }

    #[tokio::test]
    async fn registry_refuses_unsafe_endpoints_wrong_provider_and_native_login_redirects() {
        let lab = Lab::local("provider-registry-refusals");
        account(
            &lab,
            "openai-key",
            Provider::OpenAI,
            Source::Token,
            "KEY-SECRET",
        )
        .await;
        account(
            &lab,
            "native-login",
            Provider::OpenAI,
            Source::Codex,
            "NATIVE-SECRET",
        )
        .await;
        account(
            &lab,
            "wrong-provider",
            Provider::Anthropic,
            Source::Token,
            "sk-ant-key-secret",
        )
        .await;
        add_service(&lab, "custom", Provider::OpenAI, "responses", "bearer").await;
        for url in [
            "http://example.invalid",
            "https://user:SECRET@example.invalid",
            "https://example.invalid?key=SECRET",
            "https://example.invalid/v1",
            "https://example.invalid/#SECRET",
        ] {
            let (status,body)=lab.post("/api/provider-registry/services",&json!({"id":"invalid","label":"Invalid","provider":"openai","base_url":url,"protocol":"responses","auth_profile":"bearer"}).to_string()).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
            assert!(!body.to_string().contains("SECRET"));
        }
        for id in ["native-login", "wrong-provider", "missing-account"] {
            let (status,body)=lab.post("/api/provider-registry/models",&json!({"alias":"unsafe","service":"custom","upstream_model":"model","account_id":id}).to_string()).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        }
        account(
            &lab,
            "oauth",
            Provider::Anthropic,
            Source::Token,
            "sk-ant-oat01-NATIVE-SECRET",
        )
        .await;
        add_service(
            &lab,
            "anthropic-copy",
            Provider::Anthropic,
            "messages",
            "bearer",
        )
        .await;
        add_model(&lab, "oauth-redirect", "anthropic-copy", "oauth").await;
        assert!(resolve(&lab.state, "oauth-redirect")
            .await
            .err()
            .unwrap()
            .to_string()
            .contains("cannot be redirected"));
        assert_eq!(
            model_routes(&Connection::open(&lab.state.config.db_path).unwrap())
                .unwrap()
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn registry_deletion_and_assignment_keep_routes_consistent() {
        let lab = Lab::local("provider-registry-delete");
        account(
            &lab,
            "first",
            Provider::OpenAI,
            Source::Token,
            "FIRST-SECRET",
        )
        .await;
        account(
            &lab,
            "second",
            Provider::OpenAI,
            Source::Token,
            "SECOND-SECRET",
        )
        .await;
        add_service(&lab, "custom", Provider::OpenAI, "responses", "bearer").await;
        add_model(&lab, "my/model", "custom", "first").await;
        assert_eq!(
            delete(&lab, "/api/provider-registry/services/custom")
                .await
                .0,
            StatusCode::CONFLICT
        );
        assert_eq!(
            delete(&lab, "/api/provider-registry/services/openai")
                .await
                .0,
            StatusCode::CONFLICT
        );
        let (status, body) = lab
            .post(
                "/api/models/assign",
                &json!({"model":"my/model","account":"second"}).to_string(),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(
            resolve(&lab.state, "my/model")
                .await
                .unwrap()
                .unwrap()
                .signer
                .account_id,
            "second"
        );
        assert_eq!(
            delete(&lab, "/api/provider-registry/models/my%2Fmodel")
                .await
                .0,
            StatusCode::OK
        );
        assert_eq!(
            delete(&lab, "/api/provider-registry/services/custom")
                .await
                .0,
            StatusCode::OK
        );
        assert!(resolve(&lab.state, "my/model").await.unwrap().is_none());
    }

    async fn cache_wire_case(streaming: bool) {
        let lab = Lab::local(&format!("provider-registry-kimi-responses-{streaming}"));
        account(
            &lab,
            "kimi-key",
            Provider::Kimi,
            Source::Token,
            "PROVIDER-KEY",
        )
        .await;
        let mut service = add_service(
            &lab,
            "responses-service",
            Provider::Kimi,
            "responses",
            "bearer",
        )
        .await;
        add_model(&lab, "claude-alias", "responses-service", "kimi-key").await;
        let (observed, mut observations) = tokio::sync::mpsc::channel(1);
        let output = json!({"id":"resp_registered","object":"response","status":"completed","model":"identical-model","output":[{"id":"item","type":"message","role":"assistant","content":[{"type":"output_text","text":"Hello","annotations":[]}]}],"usage":{"input_tokens":100,"input_tokens_details":{"cached_tokens":60},"output_tokens":3}});
        let reply = if streaming {
            [json!({"type":"response.created","response":{"id":"resp_registered","status":"in_progress","output":[]}}),json!({"type":"response.output_text.delta","item_id":"item","output_index":0,"content_index":0,"delta":"Hello"}),json!({"type":"response.completed","response":output})].iter().map(|value|format!("data: {value}\n\n")).collect::<String>()
        } else {
            output.to_string()
        };
        let handler = move |request: Request<Body>| {
            let observed = observed.clone();
            let reply = reply.clone();
            async move {
                let (parts, body) = request.into_parts();
                let bytes = axum::body::to_bytes(body, 1 << 20).await.unwrap();
                observed
                    .send((
                        parts.uri.path().to_string(),
                        parts.headers,
                        serde_json::from_slice::<Value>(&bytes).unwrap(),
                    ))
                    .await
                    .unwrap();
                (
                    [(
                        "content-type",
                        if streaming {
                            "text/event-stream"
                        } else {
                            "application/json"
                        },
                    )],
                    reply,
                )
            }
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        service.base_url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                axum::Router::new().route("/v1/responses", post(handler)),
            )
            .await
            .unwrap()
        });
        // HTTP exists only in this isolated mock fixture; registration requires HTTPS.
        Connection::open(&lab.state.config.db_path)
            .unwrap()
            .execute(
                "UPDATE provider_services SET definition=?1 WHERE id=?2",
                rusqlite::params![serde_json::to_string(&service).unwrap(), service.id],
            )
            .unwrap();
        let config = crate::config::Config {
            server: false,
            bind: ([127, 0, 0, 1], 0).into(),
            upstream: "https://unused.invalid".into(),
            db_path: lab.state.config.db_path.clone(),
            body_cap: 65536,
            dump_headers: 0,
            allow_hosts: vec![],
            public_url: None,
            push_keys_dir: None,
            agents_dir: None,
            drain: std::time::Duration::from_secs(1),
            stop_delay: std::time::Duration::ZERO,
        };
        let (capture, mut captured) = tokio::sync::mpsc::channel(1);
        let (network, _) = crate::network::switch();
        let mut state = AppState::new(config, capture, network).unwrap();
        let tls = hyper_rustls::HttpsConnectorBuilder::new()
            .with_webpki_roots()
            .https_or_http()
            .enable_http1()
            .build();
        Arc::get_mut(&mut state).unwrap().client =
            hyper_util::client::legacy::Client::builder(hyper_util::rt::TokioExecutor::new())
                .build(tls);
        state.credentials.write().await.insert(
            "kimi-key".into(),
            lab.state.credentials.read().await["kimi-key"].clone(),
        );
        let body = json!({"model":"claude-alias","messages":[{"role":"user","content":"Hello"}],"max_tokens":20,"stream":streaming});
        let mut request = Request::post("/v1/messages")
            .header("host", "127.0.0.1")
            .header("authorization", "Bearer LOCAL-CLIENT")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        request
            .extensions_mut()
            .insert(ConnectInfo(std::net::SocketAddr::from((
                [127, 0, 0, 1],
                50000,
            ))));
        let response = crate::app(state).oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
            .await
            .unwrap();
        let (path, headers, upstream) = observations.recv().await.unwrap();
        assert_eq!(path, "/v1/responses");
        assert_eq!(headers["authorization"], "Bearer PROVIDER-KEY");
        assert!(!headers.contains_key("x-api-key"));
        assert_eq!(upstream["model"], "identical-model");
        assert!(upstream.get("input").is_some());
        let event = captured.recv().await.unwrap();
        assert_eq!(event.model.as_deref(), Some("claude-alias"));
        assert_eq!(event.usage.input_tokens, 40);
        assert_eq!(event.usage.cache_read_tokens, 60);
        let shape = event.shape.unwrap();
        assert_eq!(shape.service_id.as_deref(), Some("responses-service"));
        assert_eq!(shape.upstream_model.as_deref(), Some("identical-model"));
        let mut parsed = crate::analyze::SseScanner::default();
        if streaming {
            parsed.feed(&bytes)
        } else {
            parsed.absorb_json_body(&bytes)
        };
        assert_eq!(parsed.usage.input_tokens, 40);
        assert_eq!(parsed.usage.cache_read_tokens, 60);
        server.abort();
    }

    #[tokio::test]
    async fn disabled_capabilities_refuse_before_upstream_and_default_output_is_bounded() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let lab = Lab::local("provider-registry-capability-wire");
        account(&lab, "key", Provider::OpenAI, Source::Token, "PROVIDER-KEY").await;
        let mut service =
            add_service(&lab, "bounded", Provider::OpenAI, "messages", "bearer").await;
        add_model(&lab, "bounded-model", "bounded", "key").await;
        service.capabilities = Capabilities {
            tools: false,
            images: false,
            reasoning: false,
            output_tokens: Some(10),
            ..Capabilities::default()
        };
        account(
            &lab,
            "subscription-key",
            Provider::OpenAI,
            Source::Token,
            "SUBSCRIPTION-FIXTURE",
        )
        .await;
        lab.state
            .credentials
            .write()
            .await
            .get_mut("subscription-key")
            .unwrap()
            .openai_chatgpt = true;
        let (status,body)=lab.post("/api/provider-registry/models",&json!({"alias":"bounded-subscription","service":"openai","upstream_model":"gpt-fixture","account_id":"subscription-key","capabilities":{"output_tokens":10}}).to_string()).await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = calls.clone();
        let (sent, mut received) = tokio::sync::mpsc::channel(1);
        let handler = move |request: Request<Body>| {
            let observed = observed.clone();
            let sent = sent.clone();
            async move {
                observed.fetch_add(1, Ordering::SeqCst);
                let bytes = axum::body::to_bytes(request.into_body(), 1 << 20)
                    .await
                    .unwrap();
                sent.send(serde_json::from_slice::<Value>(&bytes).unwrap())
                    .await
                    .unwrap();
                axum::Json(
                    json!({"id":"msg","type":"message","role":"assistant","model":"identical-model","content":[{"type":"text","text":"Hello"}],"stop_reason":"end_turn","usage":{"input_tokens":1,"output_tokens":1}}),
                )
            }
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        service.base_url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                axum::Router::new().route("/v1/messages", post(handler)),
            )
            .await
            .unwrap()
        });
        Connection::open(&lab.state.config.db_path)
            .unwrap()
            .execute(
                "UPDATE provider_services SET definition=?1 WHERE id=?2",
                rusqlite::params![serde_json::to_string(&service).unwrap(), service.id],
            )
            .unwrap();
        let config = crate::config::Config {
            server: false,
            bind: ([127, 0, 0, 1], 0).into(),
            upstream: "https://unused.invalid".into(),
            db_path: lab.state.config.db_path.clone(),
            body_cap: 65536,
            dump_headers: 0,
            allow_hosts: vec![],
            public_url: None,
            push_keys_dir: None,
            agents_dir: None,
            drain: std::time::Duration::from_secs(1),
            stop_delay: std::time::Duration::ZERO,
        };
        let (capture, _captured) = tokio::sync::mpsc::channel(64);
        let (network, _) = crate::network::switch();
        let mut state = AppState::new(config, capture, network).unwrap();
        let tls = hyper_rustls::HttpsConnectorBuilder::new()
            .with_webpki_roots()
            .https_or_http()
            .enable_http1()
            .build();
        Arc::get_mut(&mut state).unwrap().client =
            hyper_util::client::legacy::Client::builder(hyper_util::rt::TokioExecutor::new())
                .build(tls);
        state.credentials.write().await.insert(
            "key".into(),
            lab.state.credentials.read().await["key"].clone(),
        );
        state.credentials.write().await.insert(
            "subscription-key".into(),
            lab.state.credentials.read().await["subscription-key"].clone(),
        );
        let app = crate::app(state);
        let send = |body: Value, path: &str| {
            let app = app.clone();
            let mut request = Request::post(path)
                .header("host", "127.0.0.1")
                .header("authorization", "Bearer LOCAL-CLIENT")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap();
            request
                .extensions_mut()
                .insert(ConnectInfo(std::net::SocketAddr::from((
                    [127, 0, 0, 1],
                    50000,
                ))));
            async move { app.oneshot(request).await.unwrap() }
        };
        let base = json!({"model":"bounded-model","messages":[{"role":"user","content":"hello"}],"max_tokens":5});
        for (field, value, reason) in [
            (
                "tools",
                json!([{"name":"tool","input_schema":{"type":"object"}}]),
                "tool",
            ),
            ("tool_choice", json!({"type":"tool","name":"tool"}), "tool"),
            (
                "messages",
                json!([{"role":"assistant","content":[{"type":"tool_use","id":"call","name":"tool","input":{}}]}]),
                "tool",
            ),
            (
                "messages",
                json!([{"role":"user","content":[{"type":"image","source":{"type":"base64","media_type":"image/png","data":"AA=="}}]}]),
                "image",
            ),
            (
                "thinking",
                json!({"type":"enabled","budget_tokens":1}),
                "reasoning",
            ),
            ("reasoning", json!({"effort":"high"}), "reasoning"),
            ("max_tokens", json!(11), "output budget"),
        ] {
            let mut body = base.clone();
            body[field] = value;
            let response = send(body, "/v1/messages").await;
            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
            let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
                .await
                .unwrap();
            assert!(String::from_utf8_lossy(&bytes).contains(reason));
            assert_eq!(calls.load(Ordering::SeqCst), 0);
        }
        let response = send(
            json!({"model":"bounded-subscription","input":"hello"}),
            "/v1/responses",
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
            .await
            .unwrap();
        assert!(String::from_utf8_lossy(&bytes).contains("subscription backend"));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        let response = send(
            json!({"model":"bounded-model","input":"hello"}),
            "/v1/responses",
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        axum::body::to_bytes(response.into_body(), 1 << 20)
            .await
            .unwrap();
        let upstream = received.recv().await.unwrap();
        assert_eq!(
            upstream["max_tokens"], 10,
            "translated default must honor route cap"
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        server.abort();
    }

    #[tokio::test]
    async fn registered_protocol_controls_wire_translation_auth_and_cache_accounting() {
        cache_wire_case(false).await;
        cache_wire_case(true).await;
    }
}
