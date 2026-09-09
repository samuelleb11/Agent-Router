//! Read API for the dashboard, plus the embedded UI itself.
//!
//! Reads open their own short-lived connection. SQLite in WAL mode takes concurrent
//! readers alongside the capture task's single writer, and a local dashboard polling
//! every few seconds is nowhere near the point where pooling would earn its keep.

use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::{header, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::Json;
use rusqlite::types::ValueRef;
use rusqlite::{Connection, OpenFlags};
use serde::Deserialize;
use serde_json::{json, Map, Value};

use crate::proxy::AppState;

const INDEX_HTML: &str = include_str!("../assets/index.html");

pub async fn index() -> Html<&'static str> {
    Html(INDEX_HTML)
}

pub async fn favicon() -> Response {
    // An inline SVG keeps the binary self-contained and the tab identifiable.
    (
        [(header::CONTENT_TYPE, "image/svg+xml")],
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32"><rect width="32" height="32" rx="7" fill="#4f46e5"/><path d="M8 21V11h3.2l4.8 6.4V11h3.2v10h-3.2l-4.8-6.4V21z" fill="#fff"/></svg>"##,
    )
        .into_response()
}

#[derive(Deserialize)]
pub struct Filter {
    pub project: Option<String>,
    pub session: Option<String>,
}

type ApiResult = Result<Json<Value>, ApiError>;

pub struct ApiError(anyhow::Error);

impl<E: Into<anyhow::Error>> From<E> for ApiError {
    fn from(e: E) -> Self {
        ApiError(e.into())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        tracing::warn!(error = %self.0, "api query failed");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": self.0.to_string() })),
        )
            .into_response()
    }
}

fn open(state: &AppState) -> anyhow::Result<Connection> {
    let conn = Connection::open_with_flags(
        &state.config.db_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )?;
    Ok(conn)
}

/// Run a query and return its rows as JSON objects keyed by column name.
fn rows(conn: &Connection, sql: &str, params: &[&dyn rusqlite::ToSql]) -> anyhow::Result<Value> {
    let mut stmt = conn.prepare(sql)?;
    let names: Vec<String> = stmt.column_names().into_iter().map(str::to_owned).collect();
    let out = stmt
        .query_map(params, |row| {
            let mut obj = Map::new();
            for (i, name) in names.iter().enumerate() {
                obj.insert(name.clone(), value_of(row.get_ref(i)?));
            }
            Ok(Value::Object(obj))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Value::Array(out))
}

fn value_of(v: ValueRef<'_>) -> Value {
    match v {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(i) => json!(i),
        ValueRef::Real(f) => json!(f),
        ValueRef::Text(t) => json!(String::from_utf8_lossy(t)),
        ValueRef::Blob(b) => json!(b.len()),
    }
}

fn one(conn: &Connection, sql: &str, params: &[&dyn rusqlite::ToSql]) -> anyhow::Result<Value> {
    Ok(rows(conn, sql, params)?
        .as_array()
        .and_then(|a| a.first().cloned())
        .unwrap_or(Value::Null))
}

/// Everything the dashboard header needs, in one round trip.
pub async fn summary(State(state): State<Arc<AppState>>) -> ApiResult {
    let conn = open(&state)?;
    let totals = one(
        &conn,
        "SELECT count(*) AS requests,
                coalesce(sum(u.prompt_tokens),0)   AS prompt_tokens,
                coalesce(sum(u.output_tokens),0)   AS output_tokens,
                coalesce(sum(u.thinking_tokens),0) AS thinking_tokens,
                coalesce(sum(u.cache_read_tokens),0) AS cache_read_tokens,
                coalesce(sum(u.cache_creation_5m_tokens + u.cache_creation_1h_tokens),0) AS cache_write_tokens,
                (SELECT count(*) FROM sessions) AS sessions,
                (SELECT count(*) FROM projects) AS projects
         FROM requests r LEFT JOIN usage u ON u.request_id = r.id",
        &[],
    )?;
    let quota = one(
        &conn,
        "SELECT rl_status, rl_5h_utilization, rl_7d_utilization, rl_5h_reset, rl_7d_reset,
                started_at
         FROM requests WHERE rl_status IS NOT NULL ORDER BY started_at DESC LIMIT 1",
        &[],
    )?;
    let account = one(
        &conn,
        "SELECT id, label, subscription_type FROM accounts WHERE active = 1 LIMIT 1",
        &[],
    )?;
    let context = rows(
        &conn,
        "SELECT component, sum(est_tokens) AS est_tokens, sum(char_count) AS chars
         FROM context GROUP BY component ORDER BY est_tokens DESC",
        &[],
    )?;
    let models = rows(
        &conn,
        "SELECT r.model, count(*) AS requests,
                sum(u.prompt_tokens) AS prompt_tokens,
                sum(u.output_tokens) AS output_tokens,
                sum(u.cache_read_tokens) AS cache_read_tokens
         FROM requests r JOIN usage u ON u.request_id = r.id
         WHERE r.model IS NOT NULL GROUP BY r.model ORDER BY prompt_tokens DESC",
        &[],
    )?;
    let threads = rows(
        &conn,
        "SELECT CASE WHEN coalesce(r.is_sidechain,0) = 1 THEN 'subagent' ELSE 'main' END AS thread,
                count(*) AS requests, sum(u.prompt_tokens) AS prompt_tokens,
                sum(u.output_tokens) AS output_tokens
         FROM requests r JOIN usage u ON u.request_id = r.id
         GROUP BY coalesce(r.is_sidechain,0) ORDER BY coalesce(r.is_sidechain,0)",
        &[],
    )?;
    Ok(Json(
        json!({ "totals": totals, "quota": quota, "account": account,
                "context": context, "models": models, "threads": threads }),
    ))
}

pub async fn projects(State(state): State<Arc<AppState>>) -> ApiResult {
    let conn = open(&state)?;
    Ok(Json(rows(
        &conn,
        "SELECT s.cwd AS project,
                count(DISTINCT s.id) AS sessions,
                count(*)             AS requests,
                sum(u.prompt_tokens) AS prompt_tokens,
                sum(u.output_tokens) AS output_tokens,
                sum(u.cache_read_tokens) AS cache_read_tokens,
                sum(CASE WHEN coalesce(r.is_sidechain,0) = 1 THEN u.prompt_tokens ELSE 0 END) AS subagent_tokens,
                max(r.started_at)    AS last_seen
         FROM requests r
         JOIN usage u    ON u.request_id = r.id
         JOIN sessions s ON s.id = r.session_id
         GROUP BY s.cwd ORDER BY prompt_tokens DESC",
        &[],
    )?))
}

pub async fn sessions(State(state): State<Arc<AppState>>, Query(f): Query<Filter>) -> ApiResult {
    let conn = open(&state)?;
    let sql = "SELECT s.id, s.cwd AS project, s.git_branch, s.started_at, s.last_seen_at,
                      count(*) AS requests,
                      sum(u.prompt_tokens) AS prompt_tokens,
                      sum(u.output_tokens) AS output_tokens,
                      sum(u.cache_read_tokens) AS cache_read_tokens,
                      sum(CASE WHEN coalesce(r.is_sidechain,0) = 1 THEN u.prompt_tokens ELSE 0 END) AS subagent_tokens,
                      count(DISTINCT r.agent_id) AS subagents
               FROM requests r
               JOIN usage u    ON u.request_id = r.id
               JOIN sessions s ON s.id = r.session_id
               WHERE (?1 IS NULL OR s.cwd = ?1)
               GROUP BY s.id ORDER BY s.last_seen_at DESC LIMIT 200";
    Ok(Json(rows(&conn, sql, &[&f.project])?))
}

pub async fn requests(State(state): State<Arc<AppState>>, Query(f): Query<Filter>) -> ApiResult {
    let conn = open(&state)?;
    let sql = "SELECT r.id, r.request_id, r.model, r.status, r.ttfb_ms, r.duration_ms,
                      r.stop_reason, r.is_sidechain, r.agent_id, r.started_at,
                      u.prompt_tokens, u.output_tokens, u.thinking_tokens,
                      u.cache_read_tokens,
                      u.cache_creation_5m_tokens + u.cache_creation_1h_tokens AS cache_write_tokens,
                      (SELECT group_concat(name) FROM tool_invocations t WHERE t.request_id = r.id) AS tools
               FROM requests r LEFT JOIN usage u ON u.request_id = r.id
               WHERE (?1 IS NULL OR r.session_id = ?1)
               ORDER BY r.started_at DESC LIMIT 300";
    Ok(Json(rows(&conn, sql, &[&f.session])?))
}

pub async fn tools(State(state): State<Arc<AppState>>, Query(f): Query<Filter>) -> ApiResult {
    let conn = open(&state)?;
    let produced = rows(
        &conn,
        "SELECT tc.name, count(*) AS results, sum(tc.result_chars) AS chars,
                sum(tc.est_tokens) AS est_tokens, sum(tc.is_error) AS errors
         FROM tool_calls tc
         WHERE (?1 IS NULL OR tc.session_id = ?1)
         GROUP BY tc.name ORDER BY est_tokens DESC",
        &[&f.session],
    )?;
    let called = rows(
        &conn,
        "SELECT ti.name, count(*) AS calls
         FROM tool_invocations ti JOIN requests r ON r.id = ti.request_id
         WHERE (?1 IS NULL OR r.session_id = ?1)
         GROUP BY ti.name ORDER BY calls DESC",
        &[&f.session],
    )?;
    Ok(Json(json!({ "produced": produced, "called": called })))
}

// ---------------------------------------------------------------------------
// Accounts
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct AddAccount {
    pub label: String,
    pub source: String,
    /// Required for `token` accounts: the value `claude setup-token` printed.
    pub token: Option<String>,
}

#[derive(Deserialize)]
pub struct AccountRef {
    pub id: String,
}

pub async fn accounts_list(State(state): State<Arc<AppState>>) -> ApiResult {
    let conn = open(&state)?;
    let mut list = rows(
        &conn,
        "SELECT id, label, source, profile_dir, email, org_name, subscription_type,
                active, created_at
         FROM accounts ORDER BY created_at",
        &[],
    )?;

    // Report whether each account can actually produce a token right now, so a
    // half-finished profile login is visible in the UI instead of failing later.
    if let Some(items) = list.as_array_mut() {
        for item in items.iter_mut() {
            let Some(obj) = item.as_object_mut() else { continue };
            let account = crate::accounts::Account {
                id: obj.get("id").and_then(Value::as_str).unwrap_or_default().to_string(),
                label: String::new(),
                source: crate::accounts::Source::parse(
                    obj.get("source").and_then(Value::as_str).unwrap_or("keychain"),
                )
                .unwrap_or(crate::accounts::Source::Keychain),
                profile_dir: obj
                    .get("profile_dir")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            };
            let (ready, detail, expires) =
                match tokio::task::spawn_blocking(move || crate::accounts::resolve(&account)).await
                {
                    Ok(Ok(r)) => (
                        true,
                        crate::accounts::token_kind(&r.token).to_string(),
                        r.expires_at_ms,
                    ),
                    Ok(Err(e)) => (false, e.to_string(), 0),
                    Err(e) => (false, e.to_string(), 0),
                };
            obj.insert("ready".into(), json!(ready));
            obj.insert("detail".into(), json!(detail));
            obj.insert("expires_at".into(), json!(expires));
        }
    }
    Ok(Json(list))
}

pub async fn accounts_add(
    State(state): State<Arc<AppState>>,
    Json(req): Json<AddAccount>,
) -> ApiResult {
    let source = crate::accounts::Source::parse(&req.source)?;
    let id = crate::accounts::slugify(&req.label);
    let store = crate::accounts::Store::new(state.config.db_path.clone());

    let result = tokio::task::spawn_blocking(move || -> anyhow::Result<Value> {
        match source {
            crate::accounts::Source::Token => {
                let token = req
                    .token
                    .as_deref()
                    .map(str::trim)
                    .filter(|t| !t.is_empty())
                    .ok_or_else(|| anyhow::anyhow!("a token is required for this account type"))?;
                crate::accounts::store_token(&id, token)?;
                store.add(&id, &req.label, source, None, &Default::default())?;
                Ok(json!({ "id": id, "ready": true }))
            }
            crate::accounts::Source::Profile => {
                // The router creates and owns the profile directory; the login itself
                // is a terminal program, so the UI hands back the command to run.
                let dir = crate::accounts::profiles_root()?.join(&id);
                std::fs::create_dir_all(&dir)?;
                let status = crate::accounts::probe_profile(Some(&dir)).unwrap_or_default();
                store.add(&id, &req.label, source, Some(&dir.to_string_lossy()), &status)?;
                Ok(json!({
                    "id": id,
                    "ready": status.logged_in,
                    "profile_dir": dir.to_string_lossy(),
                    "command": format!("CLAUDE_CONFIG_DIR={} claude auth login", dir.display()),
                }))
            }
            crate::accounts::Source::Keychain => Err(anyhow::anyhow!(
                "the default Keychain account is discovered automatically and cannot be added"
            )),
        }
    })
    .await??;
    Ok(Json(result))
}

/// Re-check a profile account's login state, and pick up its email and org once the
/// terminal login completes. The UI polls this so the flow finishes on its own.
pub async fn accounts_probe(
    State(state): State<Arc<AppState>>,
    Json(req): Json<AccountRef>,
) -> ApiResult {
    let conn = open(&state)?;
    let dir: Option<String> = rows(
        &conn,
        "SELECT profile_dir FROM accounts WHERE id = ?1",
        &[&req.id],
    )?
    .as_array()
    .and_then(|a| a.first().cloned())
    .and_then(|v| v.get("profile_dir").and_then(Value::as_str).map(str::to_owned));
    let Some(dir) = dir else {
        return Ok(Json(json!({ "logged_in": false, "detail": "not a profile account" })));
    };

    let store = crate::accounts::Store::new(state.config.db_path.clone());
    let id = req.id.clone();
    let label_dir = dir.clone();
    let status = tokio::task::spawn_blocking(move || -> anyhow::Result<Value> {
        let status = crate::accounts::probe_profile(Some(std::path::Path::new(&label_dir)))?;
        if status.logged_in {
            let label = status.email.clone().unwrap_or_else(|| id.clone());
            store.add(
                &id,
                &label,
                crate::accounts::Source::Profile,
                Some(&label_dir),
                &status,
            )?;
        }
        Ok(serde_json::to_value(status)?)
    })
    .await??;
    Ok(Json(status))
}

pub async fn accounts_activate(
    State(state): State<Arc<AppState>>,
    Json(req): Json<AccountRef>,
) -> ApiResult {
    let store = crate::accounts::Store::new(state.config.db_path.clone());
    let id = req.id.clone();
    tokio::task::spawn_blocking(move || store.activate(&id)).await??;
    // Drop the cached credential: the switch takes effect on the very next request,
    // mid-session, with nothing restarted.
    state.invalidate().await;
    tracing::info!(account = %req.id, "active account switched");
    Ok(Json(json!({ "ok": true, "active": req.id })))
}

pub async fn accounts_remove(
    State(state): State<Arc<AppState>>,
    Json(req): Json<AccountRef>,
) -> ApiResult {
    if req.id == crate::proxy::ACCOUNT_ID {
        return Err(ApiError(anyhow::anyhow!(
            "the default account is discovered automatically and cannot be removed"
        )));
    }
    let store = crate::accounts::Store::new(state.config.db_path.clone());
    let id = req.id.clone();
    tokio::task::spawn_blocking(move || store.remove(&id)).await??;
    state.invalidate().await;
    Ok(Json(json!({ "ok": true })))
}
