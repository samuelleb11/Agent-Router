//! The catalogue: every model the router can serve, each with its provider and the account
//! its requests go to (docs/sources-and-models.md, "The catalogue").
//!
//! A model belongs to one provider. One whose id contains `claude` is Anthropic's without
//! being listed, so a Claude model works the day it ships; every other model is the one
//! provider's whose list names it. A model may be assigned an account of its own provider,
//! and one with no assignment goes to its provider's active account. Switching provider is
//! therefore picking a model in the client: no account is flipped for it.

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use rusqlite::{Connection, OptionalExtension};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::accounts::{Conflict, Provider};
use crate::proxy::AppState;
use crate::refusal::{failed, refuse, JsonBody};

/// Kimi Code's models and the names its docs give them. Which of them a key may use is its
/// plan's: K3 needs Plus, and HighSpeed and K3's 1M window need Pro (kimi.com/code/docs,
/// read 2026-10-09). A model above the plan is refused by Kimi Code, in its own words.
const KIMI: &[(&str, &str)] = &[
    ("k3", "Kimi K3"),
    ("k3-256k", "Kimi K3 (256K)"),
    ("kimi-for-coding", "Kimi for Coding"),
    ("kimi-for-coding-highspeed", "Kimi for Coding HighSpeed"),
];

/// A model as a request names it, made comparable: Claude Code's `[1m]` is its own, and
/// never reaches the wire, but a person typing a model may still write it.
fn normal(model: &str) -> String {
    model.trim().trim_end_matches("[1m]").to_ascii_lowercase()
}

/// The provider whose model this is, if it is any provider's.
pub fn provider_of(model: &str) -> Option<Provider> {
    let id = normal(model);
    if id.contains("claude") {
        Some(Provider::Anthropic)
    } else if KIMI.iter().any(|(k, _)| *k == id) {
        Some(Provider::Kimi)
    } else if id.starts_with("gpt-") || id.strip_prefix("gpt").and_then(|tail| tail.chars().next()).is_some_and(|c| c.is_ascii_digit()) || id.starts_with("codex-")
        || ["o1", "o3", "o4"].iter().any(|family| id == *family || id.starts_with(&format!("{family}-")))
    {
        Some(Provider::OpenAI)
    } else {
        None
    }
}

/// One model in the catalogue.
#[derive(Debug, Clone, PartialEq)]
pub struct Model {
    pub id: String,
    pub provider: Provider,
    pub display_name: String,
    /// The account this model was assigned, if it was.
    pub assigned: Option<String>,
    /// The account its next request goes to: the assigned one, else its provider's active
    /// account.
    pub serving: Option<String>,
    /// When the router could first serve it: when its provider's first account was added.
    pub created_at: String,
}

/// Each provider that has an account, and when its first was added (RFC 3339, UTC).
fn providers(conn: &Connection) -> rusqlite::Result<Vec<(Provider, String)>> {
    let mut stmt = conn.prepare(
        "SELECT provider, strftime('%Y-%m-%dT%H:%M:%SZ', min(created_at) / 1000, 'unixepoch')
         FROM accounts GROUP BY provider ORDER BY min(created_at)",
    )?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    let mut out = Vec::new();
    for row in rows {
        let (provider, since) = row?;
        if let Ok(provider) = Provider::parse(&provider) {
            out.push((provider, since));
        }
    }
    Ok(out)
}

fn active_of(conn: &Connection, provider: Provider) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT id FROM accounts WHERE active = 1 AND provider = ?1 ORDER BY created_at LIMIT 1",
        [provider.as_str()],
        |r| r.get(0),
    )
    .optional()
}

/// Every model's assigned account, by model.
fn assignments(conn: &Connection) -> rusqlite::Result<HashMap<String, String>> {
    let mut stmt = conn.prepare("SELECT model, account_id FROM model_assignments")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    rows.collect()
}

/// The Claude models the request log has seen, from the pricing module's table of model
/// names (one row per name, kept as requests arrive), so listing them never reads the log.
fn seen_models(conn: &Connection, provider: Provider) -> rusqlite::Result<Vec<String>> {
    let kept: i64 = conn.query_row(
        "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = 'model_rate'",
        [],
        |r| r.get(0),
    )?;
    if kept == 0 {
        return Ok(Vec::new());
    }
    let mut stmt = conn.prepare("SELECT model FROM model_rate ORDER BY model")?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
    let mut out = Vec::new();
    for model in rows {
        let model = normal(&model?);
        if provider_of(&model) == Some(provider) && !out.contains(&model) {
            out.push(model);
        }
    }
    Ok(out)
}

/// Codex's own model catalogue, when it has cached one. It describes the installed
/// client, not a promise that the current account is entitled to every listed model.
/// Credentials and model instructions from the cache never enter a router response.
fn codex_models() -> Vec<(String, String)> {
    let Some(raw) = crate::accounts::codex_home().ok()
        .and_then(|home| std::fs::read(home.join("models_cache.json")).ok()) else { return Vec::new() };
    cached_codex_models(&raw)
}

fn cached_codex_models(raw: &[u8]) -> Vec<(String, String)> {
    let Ok(cache) = serde_json::from_slice::<Value>(raw) else { return Vec::new() };
    cache.get("models").and_then(Value::as_array).into_iter().flatten().filter_map(|m| {
        if m.get("visibility").and_then(Value::as_str).is_some_and(|v| v != "list") {
            return None;
        }
        let id = m.get("slug").or_else(|| m.get("id"))?.as_str()?;
        if provider_of(id) != Some(Provider::OpenAI) { return None; }
        Some((id.to_owned(), m.get("display_name").and_then(Value::as_str).unwrap_or(id).to_owned()))
    }).collect()
}

/// Keep names learned from an OpenAI Models API response, without pinning a release's
/// catalogue in code. Audio, image and realtime endpoints are outside coding clients.
pub fn record_openai_models(conn: &Connection, response: &Value) -> rusqlite::Result<usize> {
    let mut count = 0;
    for id in response.get("data").and_then(Value::as_array).into_iter().flatten()
        .filter_map(|m| m.get("id").and_then(Value::as_str))
    {
        let model = normal(id);
        if provider_of(&model) != Some(Provider::OpenAI)
            || ["image", "audio", "realtime", "transcribe", "tts"].iter().any(|part| model.split('-').any(|word| word == *part))
        {
            continue;
        }
        crate::pricing::resolve_one(conn, &model)?;
        count += 1;
    }
    Ok(count)
}

/// Every model the router can serve now. A provider's models are listed once it has an
/// account: Anthropic's are the Claude models seen and those assigned an account, and Kimi
/// Code's are its list. A Claude model never seen is still Anthropic's (`provider_of`); it
/// is only not listed.
pub fn catalogue(conn: &Connection) -> rusqlite::Result<Vec<Model>> {
    let assigned = assignments(conn)?;
    let mut out = Vec::new();
    for (provider, since) in providers(conn)? {
        let active = active_of(conn, provider)?;
        let mut models: Vec<(String, String)> = match provider {
            Provider::Anthropic => {
                let mut ids = seen_models(conn, Provider::Anthropic)?;
                for model in assigned.keys() {
                    if provider_of(model) == Some(Provider::Anthropic) && !ids.contains(model) {
                        ids.push(model.clone());
                    }
                }
                ids.sort();
                ids.into_iter().map(|id| (id.clone(), id)).collect()
            }
            Provider::Kimi => KIMI.iter().map(|(id, name)| (id.to_string(), name.to_string())).collect(),
            Provider::OpenAI => {
                let mut models = codex_models();
                for id in seen_models(conn, Provider::OpenAI)?.into_iter()
                    .chain(assigned.keys().filter(|id| provider_of(id) == Some(Provider::OpenAI)).cloned())
                {
                    if !models.iter().any(|(known, _)| known == &id) {
                        models.push((id.clone(), id));
                    }
                }
                models.sort_by(|a, b| a.0.cmp(&b.0));
                models
            }
        };
        for (id, display_name) in models.drain(..) {
            let assigned = assigned.get(&id).cloned();
            out.push(Model {
                serving: assigned.clone().or_else(|| active.clone()),
                assigned,
                provider,
                display_name,
                created_at: since.clone(),
                id,
            });
        }
    }
    for route in crate::provider_registry::model_routes(conn).map_err(|e|rusqlite::Error::ToSqlConversionFailure(e.into()))? {
        let Some(service)=crate::provider_registry::service(conn,&route.service).map_err(|e|rusqlite::Error::ToSqlConversionFailure(e.into()))? else {continue};
        let account:Option<(String,String)>=conn.query_row("SELECT provider,strftime('%Y-%m-%dT%H:%M:%SZ',created_at/1000,'unixepoch') FROM accounts WHERE id=?1",[&route.account_id],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        let serving=account.as_ref().filter(|(provider,_)|provider==service.provider.as_str()).map(|_|route.account_id.clone());
        let created_at=account.map(|(_,since)|since).unwrap_or_else(||"1970-01-01T00:00:00Z".into());
        out.retain(|model|model.id!=route.alias);
        out.push(Model{id:route.alias.clone(),provider:service.provider,display_name:route.display_name.unwrap_or(route.alias),assigned:Some(route.account_id),serving,created_at});
    }
    Ok(out)
}

/// The account `model`'s next request goes to, and its provider: the account it was
/// assigned, else its provider's active account. `None` when no provider serves the model,
/// or its provider has no account.
pub fn account_for(conn: &Connection, model: &str) -> rusqlite::Result<Option<(String, Provider)>> {
    if let Some(route)=crate::provider_registry::model_route(conn,model.trim()).map_err(|e|rusqlite::Error::ToSqlConversionFailure(e.into()))? {
        let service=crate::provider_registry::service(conn,&route.service).map_err(|e|rusqlite::Error::ToSqlConversionFailure(e.into()))?;
        if let Some(service)=service {
            let provider:Option<String>=conn.query_row("SELECT provider FROM accounts WHERE id=?1",[&route.account_id],|r|r.get(0)).optional()?;
            return Ok(provider.filter(|provider|provider==service.provider.as_str()).map(|_|(route.account_id,service.provider)));
        }
        return Ok(None);
    }
    let Some(provider) = provider_of(model) else {
        return Ok(None);
    };
    let assigned: Option<String> = conn
        .query_row(
            "SELECT m.account_id FROM model_assignments m JOIN accounts a ON a.id = m.account_id
             WHERE m.model = ?1 AND a.provider = ?2",
            [normal(model), provider.as_str().to_string()],
            |r| r.get(0),
        )
        .optional()?;
    let account = match assigned {
        Some(id) => Some(id),
        None => active_of(conn, provider)?,
    };
    Ok(account.map(|id| (id, provider)))
}

/// Why an assignment cannot be made.
#[derive(Debug, PartialEq)]
pub enum Refused {
    /// No provider serves the model: nothing the store holds would change that (400).
    NoProvider(String),
    /// The account is gone, or is another provider's (409).
    Account(String),
}

/// Assign `model` to `account`, or, with `None`, give it back to its provider's active
/// account. Only to an account of the model's own provider: a Kimi Code key cannot serve a
/// Claude model.
pub fn assign(conn: &Connection, model: &str, account: Option<&str>) -> Result<(), AssignError> {
    if let Some(mut route)=crate::provider_registry::model_route(conn,model.trim()).map_err(|e|rusqlite::Error::ToSqlConversionFailure(e.into()))? {
        let Some(account)=account else { return Err(AssignError::Refused(Refused::Account("registered models require an explicit account; edit or remove the route in the provider registry".into()))); };
        route.account_id=account.into();
        crate::provider_registry::put_model(conn,route).map_err(|e|AssignError::Refused(Refused::Account(e.to_string())))?;
        return Ok(());
    }
    let Some(provider) = provider_of(model) else {
        return Err(AssignError::Refused(Refused::NoProvider(format!(
            "no source serves the model {:?}",
            model.trim()
        ))));
    };
    let model = normal(model);
    let Some(account) = account else {
        conn.execute("DELETE FROM model_assignments WHERE model = ?1", [&model])?;
        return Ok(());
    };
    let theirs: Option<String> = conn
        .query_row("SELECT provider FROM accounts WHERE id = ?1", [account], |r| r.get(0))
        .optional()?;
    let Some(theirs) = theirs else {
        return Err(AssignError::Refused(Refused::Account(Conflict::NoAccount(account.to_string()).to_string())));
    };
    let theirs = Provider::parse(&theirs).unwrap_or(Provider::Anthropic);
    if theirs != provider {
        return Err(AssignError::Refused(Refused::Account(format!(
            "{account:?} is {} {} account, and {model:?} is {}'s",
            theirs.article(),
            theirs.name(),
            provider.name()
        ))));
    }
    conn.execute(
        "INSERT INTO model_assignments (model, account_id) VALUES (?1, ?2)
         ON CONFLICT(model) DO UPDATE SET account_id = excluded.account_id",
        [&model, account],
    )?;
    Ok(())
}

#[derive(Debug)]
pub enum AssignError {
    Refused(Refused),
    Store(rusqlite::Error),
}

impl From<rusqlite::Error> for AssignError {
    fn from(e: rusqlite::Error) -> Self {
        AssignError::Store(e)
    }
}

/// A model as `GET /api/models` answers it, and as the installers read it.
pub(crate) fn row(m: &Model) -> Value {
    json!({
        "id": m.id,
        "provider": m.provider.as_str(),
        "display_name": m.display_name,
        "assigned": m.assigned,
        "serving": m.serving,
    })
}

/// `GET /api/models`: the catalogue, each model with its provider, the account it was
/// assigned and the account its next request goes to.
pub async fn list(State(state): State<Arc<AppState>>) -> Response {
    match state.reads.get().and_then(|conn| Ok(catalogue(&conn)?)) {
        Ok(models) => Json(models.iter().map(row).collect::<Vec<_>>()).into_response(),
        Err(e) => failed(e),
    }
}

#[derive(Deserialize)]
pub struct Assignment {
    pub model: String,
    /// `null` gives the model back to its provider's active account.
    #[serde(default)]
    pub account: Option<String>,
}

/// `POST /api/models/assign {model, account}`.
pub async fn assign_route(State(state): State<Arc<AppState>>, JsonBody(req): JsonBody<Assignment>) -> Response {
    let db = state.config.db_path.clone();
    let (model, account) = (req.model.clone(), req.account.clone());
    let done = tokio::task::spawn_blocking(move || -> Result<(), AssignError> {
        let conn = Connection::open(db)?;
        assign(&conn, &model, account.as_deref())
    })
    .await;
    match done {
        Ok(Ok(())) => Json(json!({ "ok": true, "model": normal(&req.model), "account": req.account })).into_response(),
        Ok(Err(AssignError::Refused(Refused::NoProvider(why)))) => refuse(StatusCode::BAD_REQUEST, why),
        Ok(Err(AssignError::Refused(Refused::Account(why)))) => refuse(StatusCode::CONFLICT, why),
        Ok(Err(AssignError::Store(e))) => failed(e),
        Err(e) => failed(e),
    }
}

/// `GET /v1/models`, answered by the router itself rather than forwarded: the catalogue in
/// the Anthropic Models API's shape, for any client that lists the models it can pick.
/// Claude Code's gateway discovery keeps only the ids containing `claude` of it, which is
/// Claude Code's choice; the router names every model as it is.
pub async fn v1_models(State(state): State<Arc<AppState>>, headers: axum::http::HeaderMap) -> Response {
    if let Some(refused) = crate::proxy::no_credential(&headers) {
        return refused;
    }
    let models = match state.reads.get().and_then(|conn| Ok(catalogue(&conn)?)) {
        Ok(models) => models,
        Err(e) => return failed(e),
    };
    let data: Vec<Value> = models
        .iter()
        .map(|m| json!({ "type": "model", "id": m.id, "display_name": m.display_name, "created_at": m.created_at }))
        .collect();
    Json(json!({
        "data": data,
        "has_more": false,
        "first_id": models.first().map(|m| m.id.clone()),
        "last_id": models.last().map(|m| m.id.clone()),
    }))
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openai_models_are_discovered_without_a_fixed_release_catalogue() {
        for id in ["gpt-future", "gpt6synthetic", "codex-mini-latest", "o1", "o3-mini", "o4-mini"] {
            assert_eq!(provider_of(id), Some(Provider::OpenAI), "{id}");
        }
        assert_eq!(provider_of("unknown-model"), None);
        let cache = br#"{"models":[
            {"slug":"gpt-test","display_name":"Test","visibility":"list","base_instructions":"private prompt"},
            {"slug":"gpt6synthetic","display_name":"Synthetic native model","visibility":"list"},
            {"slug":"gpt-hidden","visibility":"hide"},
            {"slug":"claude-test","visibility":"list"}
        ]}"#;
        assert_eq!(cached_codex_models(cache), vec![("gpt-test".into(), "Test".into()), ("gpt6synthetic".into(), "Synthetic native model".into())]);

        let s = scratch("openai-models");
        s.conn.execute_batch("INSERT INTO accounts (id,label,source,provider,active,created_at)
            VALUES ('openai','OpenAI','token','openai',1,1760000003000);
            INSERT INTO model_rate (model,key) VALUES ('gpt-observed',NULL);").unwrap();
        assign(&s.conn, "gpt-future", Some("openai")).unwrap();
        assert_eq!(account_for(&s.conn, "gpt-future").unwrap(), Some(("openai".into(), Provider::OpenAI)));
        assert_eq!(account_for(&s.conn, "gpt-unseen").unwrap(), Some(("openai".into(), Provider::OpenAI)));
        let listed = catalogue(&s.conn).unwrap();
        for id in ["gpt-future", "gpt-observed"] {
            let model = listed.iter().find(|m| m.id == id).unwrap();
            assert_eq!(model.provider, Provider::OpenAI);
            assert_eq!(model.serving.as_deref(), Some("openai"));
        }
        assert!(assign(&s.conn, "gpt-future", Some("work")).is_err());
        let fetched = json!({ "data": [ { "id": "gpt-new-model" }, { "id": "o3-mini" },
            { "id": "gpt-image-1" }, { "id": "gpt-4o-mini-tts" }, { "id": "gpt-realtime" }, { "id": "unrelated" } ] });
        assert_eq!(record_openai_models(&s.conn, &fetched).unwrap(), 2);
        let list = catalogue(&s.conn).unwrap();
        assert!(list.iter().any(|m| m.id == "gpt-new-model"));
        assert!(!list.iter().any(|m| m.id == "gpt-image-1"));
    }

    /// A store of its own, opened as the router opens one, with the pricing module's table of
    /// model names.
    struct Scratch {
        dir: std::path::PathBuf,
        conn: Connection,
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn scratch(tag: &str) -> Scratch {
        let dir = std::env::temp_dir().join(format!("ar-models-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("agent-router.db");
        crate::db::Db::open(&path).unwrap();
        let conn = Connection::open(&path).unwrap();
        crate::pricing::ensure_table(&conn).unwrap();
        conn.execute_batch(
            "INSERT INTO accounts (id, label, source, provider, active, created_at)
             VALUES ('work', 'Work', 'token', 'anthropic', 1, 1760000000000),
                    ('home', 'Home', 'token', 'anthropic', 0, 1760000001000),
                    ('kimi', 'Kimi', 'token', 'kimi', 1, 1760000002000);
             INSERT INTO model_rate (model, key) VALUES ('claude-opus-5-5', NULL), ('claude-haiku-5-5', NULL);",
        )
        .unwrap();
        Scratch { dir, conn }
    }

    /// GET `path` through the router's own routes and guard, as a page on this machine asks.
    async fn get(lab: &crate::api_refusals::Lab, path: &str) -> (StatusCode, Value) {
        use axum::body::Body;
        use axum::extract::ConnectInfo;
        use axum::http::{header, Request};
        use tower::ServiceExt;

        let mut req = Request::get(path).header(header::HOST, "127.0.0.1").body(Body::empty()).unwrap();
        req.extensions_mut().insert(ConnectInfo(std::net::SocketAddr::from(([127, 0, 0, 1], 50_000))));
        let res = lab.app.clone().oneshot(req).await.unwrap();
        let status = res.status();
        let body = axum::body::to_bytes(res.into_body(), 1 << 20).await.unwrap();
        (status, serde_json::from_slice(&body).unwrap())
    }

    /// A router of its own with a Claude account and a Kimi Code one, added through the API.
    async fn lab_with_both(tag: &str) -> crate::api_refusals::Lab {
        let lab = crate::api_refusals::Lab::local(tag);
        for add in [
            json!({ "label": format!("{tag} work"), "source": "token", "token": "sk-ant-oat01-not-a-credential" }),
            json!({ "label": format!("{tag} kimi"), "source": "token", "provider": "kimi", "token": "sk-kimi-not-a-credential" }),
        ] {
            let (status, body) = lab.post("/api/accounts", &add.to_string()).await;
            assert_eq!(status, StatusCode::OK, "{body}");
        }
        lab
    }

    #[tokio::test]
    async fn the_catalogue_lists_every_providers_models_with_their_accounts() {
        let s = scratch("catalogue");
        assign(&s.conn, "claude-haiku-5-5", Some("home")).unwrap();
        let models = catalogue(&s.conn).unwrap();
        let ids: Vec<(&str, Provider)> = models.iter().map(|m| (m.id.as_str(), m.provider)).collect();
        assert_eq!(
            ids,
            [
                ("claude-haiku-5-5", Provider::Anthropic),
                ("claude-opus-5-5", Provider::Anthropic),
                ("k3", Provider::Kimi),
                ("k3-256k", Provider::Kimi),
                ("kimi-for-coding", Provider::Kimi),
                ("kimi-for-coding-highspeed", Provider::Kimi),
            ]
        );
        let of = |id: &str| models.iter().find(|m| m.id == id).unwrap();
        assert_eq!((of("claude-haiku-5-5").assigned.as_deref(), of("claude-haiku-5-5").serving.as_deref()), (Some("home"), Some("home")));
        assert_eq!((of("claude-opus-5-5").assigned.as_deref(), of("claude-opus-5-5").serving.as_deref()), (None, Some("work")));
        assert_eq!(of("k3").serving.as_deref(), Some("kimi"));
        assert_eq!(of("k3").display_name, "Kimi K3");
        assert_eq!(of("claude-opus-5-5").created_at, "2025-10-09T08:53:20Z", "when its provider's first account was added");

        // A provider with no account has nothing to serve with, so none of its models is listed.
        s.conn.execute("DELETE FROM accounts WHERE provider = 'kimi'", []).unwrap();
        assert!(catalogue(&s.conn).unwrap().iter().all(|m| m.provider == Provider::Anthropic));

        // The same, as GET /api/models answers it.
        let lab = lab_with_both("models-list").await;
        let (status, _) = lab.post("/api/models/assign", r#"{"model":"claude-opus-5-5","account":"models-list-work"}"#).await;
        assert_eq!(status, StatusCode::OK);
        let (status, list) = get(&lab, "/api/models").await;
        assert_eq!(status, StatusCode::OK, "{list}");
        let list = list.as_array().unwrap();
        assert_eq!(
            list[0],
            json!({ "id": "claude-opus-5-5", "provider": "anthropic", "display_name": "claude-opus-5-5",
                    "assigned": "models-list-work", "serving": "models-list-work" })
        );
        assert_eq!(
            list[1],
            json!({ "id": "k3", "provider": "kimi", "display_name": "Kimi K3", "assigned": null, "serving": "models-list-kimi" })
        );
        assert_eq!(list.len(), 1 + KIMI.len());
    }

    #[tokio::test]
    async fn assigning_a_model_routes_its_next_request_to_that_account() {
        let s = scratch("assign");
        assert_eq!(account_for(&s.conn, "claude-opus-5-5").unwrap(), Some(("work".into(), Provider::Anthropic)));
        assign(&s.conn, "claude-opus-5-5", Some("home")).unwrap();
        assert_eq!(account_for(&s.conn, "claude-opus-5-5").unwrap(), Some(("home".into(), Provider::Anthropic)));
        assert_eq!(account_for(&s.conn, "claude-haiku-5-5").unwrap(), Some(("work".into(), Provider::Anthropic)), "only that model");
        assert_eq!(account_for(&s.conn, "k3").unwrap(), Some(("kimi".into(), Provider::Kimi)));

        // `null` gives it back to its provider's active account.
        assign(&s.conn, "claude-opus-5-5", None).unwrap();
        assert_eq!(account_for(&s.conn, "claude-opus-5-5").unwrap(), Some(("work".into(), Provider::Anthropic)));

        // Removing the assigned account gives its models back too.
        assign(&s.conn, "claude-opus-5-5", Some("home")).unwrap();
        let path = s.dir.join("agent-router.db");
        crate::accounts::Store::new(path).remove("home").unwrap();
        assert_eq!(account_for(&s.conn, "claude-opus-5-5").unwrap(), Some(("work".into(), Provider::Anthropic)));
        assert_eq!(account_for(&s.conn, "gpt-9").unwrap(), None, "no provider serves it");

        // The same through POST /api/models/assign.
        let lab = lab_with_both("models-assign").await;
        let conn = Connection::open(&lab.state.config.db_path).unwrap();
        lab.post("/api/accounts", &json!({ "label": "models-assign home", "source": "token", "token": "sk-ant-oat01-x" }).to_string()).await;
        let (status, body) = lab.post("/api/models/assign", r#"{"model":"claude-opus-5-5","account":"models-assign-home"}"#).await;
        assert_eq!((status, &body["ok"]), (StatusCode::OK, &json!(true)), "{body}");
        assert_eq!(account_for(&conn, "claude-opus-5-5").unwrap(), Some(("models-assign-home".into(), Provider::Anthropic)));
        let (status, _) = lab.post("/api/models/assign", r#"{"model":"claude-opus-5-5","account":null}"#).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(account_for(&conn, "claude-opus-5-5").unwrap(), Some(("models-assign-work".into(), Provider::Anthropic)));
    }

    #[tokio::test]
    async fn an_assignment_across_providers_is_refused() {
        let s = scratch("assign-across");
        assert_eq!(
            assign(&s.conn, "claude-opus-5-5", Some("kimi")).unwrap_err().refusal(),
            Some(Refused::Account(r#""kimi" is a Kimi Code account, and "claude-opus-5-5" is Anthropic's"#.into()))
        );
        assert_eq!(
            assign(&s.conn, "k3", Some("work")).unwrap_err().refusal(),
            Some(Refused::Account(r#""work" is an Anthropic account, and "k3" is Kimi Code's"#.into()))
        );
        assert_eq!(
            assign(&s.conn, "gpt-9", Some("work")).unwrap_err().refusal(),
            Some(Refused::Account(r#""work" is an Anthropic account, and "gpt-9" is OpenAI's"#.into()))
        );
        assert_eq!(
            assign(&s.conn, "unknown-model", Some("work")).unwrap_err().refusal(),
            Some(Refused::NoProvider(r#"no source serves the model "unknown-model""#.into()))
        );
        assert_eq!(
            assign(&s.conn, "k3", Some("nobody")).unwrap_err().refusal(),
            Some(Refused::Account(r#"no account with id "nobody""#.into()))
        );
        assert!(assignments(&s.conn).unwrap().is_empty(), "nothing refused was kept");

        // The same through POST /api/models/assign: the account in the way is 409, and a
        // model nothing serves is 400, whatever the store holds.
        use crate::api_refusals::words;
        let lab = lab_with_both("models-across").await;
        let refused = lab.post("/api/models/assign", r#"{"model":"claude-opus-5-5","account":"models-across-kimi"}"#).await;
        assert_eq!(
            words(refused, StatusCode::CONFLICT),
            r#""models-across-kimi" is a Kimi Code account, and "claude-opus-5-5" is Anthropic's"#
        );
        let refused = lab.post("/api/models/assign", r#"{"model":"gpt-9","account":"models-across-work"}"#).await;
        assert_eq!(words(refused, StatusCode::CONFLICT), r#""models-across-work" is an Anthropic account, and "gpt-9" is OpenAI's"#);
        let refused = lab.post("/api/models/assign", r#"{"model":"unknown-model","account":"models-across-work"}"#).await;
        assert_eq!(words(refused, StatusCode::BAD_REQUEST), r#"no source serves the model "unknown-model""#);
        let conn = Connection::open(&lab.state.config.db_path).unwrap();
        assert!(assignments(&conn).unwrap().is_empty(), "nothing refused was kept");
    }

    #[test]
    fn a_new_claude_model_belongs_to_anthropic_unlisted() {
        let s = scratch("unlisted");
        assert_eq!(provider_of("claude-opus-6-0"), Some(Provider::Anthropic));
        assert_eq!(provider_of("claude-sonnet-5-5[1m]"), Some(Provider::Anthropic));
        assert_eq!(provider_of("K3"), Some(Provider::Kimi));
        assert_eq!(provider_of("gpt-9"), Some(Provider::OpenAI));
        assert!(catalogue(&s.conn).unwrap().iter().all(|m| m.id != "claude-opus-6-0"), "never seen, so not listed");
        assert_eq!(account_for(&s.conn, "claude-opus-6-0").unwrap(), Some(("work".into(), Provider::Anthropic)), "and still served");
    }

    #[tokio::test]
    async fn v1_models_answers_the_catalogue_in_the_models_api_shape() {
        use axum::body::Body;
        use axum::extract::ConnectInfo;
        use axum::http::{header, Request};
        use tower::ServiceExt;

        let lab = crate::api_refusals::Lab::local("v1-models");
        let add = json!({ "label": "V1 models Kimi", "source": "token", "provider": "kimi", "token": "sk-kimi-not-a-credential" });
        assert_eq!(lab.post("/api/accounts", &add.to_string()).await.0, StatusCode::OK);

        // Asked as Claude Code asks, through the same routes and guard, with its own credential.
        let mut req = Request::get("/v1/models?limit=1000")
            .header(header::HOST, "127.0.0.1")
            .header("x-api-key", "the client's own")
            .header("anthropic-version", "2023-06-01")
            .body(Body::empty())
            .unwrap();
        req.extensions_mut().insert(ConnectInfo(std::net::SocketAddr::from(([127, 0, 0, 1], 50_000))));
        let res = lab.app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK, "answered by the router, not forwarded to an upstream that is nowhere");
        let body: Value = serde_json::from_slice(&axum::body::to_bytes(res.into_body(), 1 << 20).await.unwrap()).unwrap();
        let data = body["data"].as_array().unwrap();
        assert_eq!(data.len(), KIMI.len(), "{body}");
        assert_eq!(data[0], json!({ "type": "model", "id": "k3", "display_name": "Kimi K3", "created_at": data[0]["created_at"] }));
        assert!(
            crate::fold::parse_rfc3339_ms(data[0]["created_at"].as_str().unwrap()).is_some(),
            "created_at is RFC 3339: {body}"
        );
        assert_eq!((&body["has_more"], &body["first_id"], &body["last_id"]), (&json!(false), &json!("k3"), &json!("kimi-for-coding-highspeed")));
    }

    impl AssignError {
        /// The refusal, if this is one.
        fn refusal(self) -> Option<Refused> {
            match self {
                AssignError::Refused(r) => Some(r),
                AssignError::Store(_) => None,
            }
        }
    }
}
