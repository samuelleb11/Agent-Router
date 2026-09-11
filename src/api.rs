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
use rusqlite::OptionalExtension;
use rusqlite::{Connection, OpenFlags};
use serde::Deserialize;
use serde_json::{json, Map, Value};

use crate::proxy::AppState;

const INDEX_HTML: &str = include_str!("../assets/index.html");

pub async fn index() -> Response {
    // The dashboard can switch and remove accounts, so it must not be framable: a
    // transparent frame over a decoy would let another origin borrow those clicks.
    (
        [
            (header::X_FRAME_OPTIONS, "DENY"),
            (
                header::CONTENT_SECURITY_POLICY,
                "frame-ancestors 'none'; base-uri 'none'; form-action 'none'",
            ),
        ],
        Html(INDEX_HTML),
    )
        .into_response()
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
    /// Which configured account served the request. Absent means every account.
    pub account: Option<String>,
    /// Lower bound on `requests.started_at`, in ms since the epoch. A time range the
    /// dashboard applies to everything on the page at once.
    pub since: Option<i64>,
    /// The viewer's UTC offset in minutes (JavaScript's `-getTimezoneOffset()`), so
    /// day-sized timeline buckets start at the viewer's midnight rather than UTC's.
    pub tz: Option<i64>,
}

/// The scope predicate every read shares. Requests are joined to their session as
/// `s` (LEFT, so a request whose transcript has not appeared yet still counts) and
/// the parameters are always bound in this order: project, session, since, account.
const SCOPE: &str = "(?1 IS NULL OR s.cwd = ?1)
                     AND (?2 IS NULL OR r.session_id = ?2)
                     AND (?3 IS NULL OR r.started_at >= ?3)
                     AND (?4 IS NULL OR r.account_id = ?4)";

fn scope_params(f: &Filter) -> [&dyn rusqlite::ToSql; 4] {
    [&f.project, &f.session, &f.since, &f.account]
}

/// A percentile of a request column within the scope, by ORDER BY + OFFSET. SQLite
/// has no percentile function, and the row counts here never justify pulling the
/// whole column into memory.
fn pctile(col: &str, pct: i64) -> String {
    format!(
        "(SELECT r.{col} FROM requests r LEFT JOIN sessions s ON s.id = r.session_id
          WHERE r.{col} IS NOT NULL AND {SCOPE}
          ORDER BY r.{col}
          LIMIT 1 OFFSET (SELECT (count(*) * {pct}) / 100
                          FROM requests r LEFT JOIN sessions s ON s.id = r.session_id
                          WHERE r.{col} IS NOT NULL AND {SCOPE}))"
    )
}

/// Timeline bucket width: the smallest step that keeps the span under ~72 bars, so a
/// ten-minute session shows minutes and a month shows days.
fn pick_bucket(span_ms: i64) -> i64 {
    const STEPS: [i64; 8] = [
        60_000,
        300_000,
        900_000,
        1_800_000,
        3_600_000,
        3 * 3_600_000,
        6 * 3_600_000,
        86_400_000,
    ];
    STEPS
        .iter()
        .copied()
        .find(|b| span_ms / b <= 72)
        .unwrap_or(86_400_000)
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

/// Everything the dashboard shows above the lists, in one round trip, scoped by the
/// same project / session / since filter as the lists so every figure on the page
/// agrees with every other. Quota is the exception: it belongs to the account, not
/// the project, so it always describes the active account.
pub async fn summary(State(state): State<Arc<AppState>>, Query(mut f): Query<Filter>) -> ApiResult {
    let conn = open(&state)?;
    // Clamp once, on the Filter itself, so every query below binds the same sane value:
    // the timeline does arithmetic on it, and i64::MAX would overflow (panic in debug).
    let now = crate::db::now_ms();
    f.since = f.since.map(|s| s.clamp(0, now));
    let p = scope_params(&f);
    let totals = one(
        &conn,
        &format!(
            "SELECT count(*) AS requests,
                    coalesce(sum(u.prompt_tokens),0)   AS prompt_tokens,
                    coalesce(sum(c.cost),0) AS cost,
                    coalesce(sum(u.input_tokens),0)    AS input_tokens,
                    coalesce(sum(u.output_tokens),0)   AS output_tokens,
                    coalesce(sum(u.thinking_tokens),0) AS thinking_tokens,
                    coalesce(sum(u.cache_read_tokens),0) AS cache_read_tokens,
                    coalesce(sum(u.cache_creation_5m_tokens),0) AS cache_write_5m_tokens,
                    coalesce(sum(u.cache_creation_1h_tokens),0) AS cache_write_1h_tokens,
                    coalesce(sum(u.cache_creation_5m_tokens + u.cache_creation_1h_tokens),0) AS cache_write_tokens,
                    coalesce(sum(CASE WHEN r.status >= 400 OR r.error IS NOT NULL THEN 1 ELSE 0 END),0) AS errors,
                    coalesce(sum(CASE WHEN coalesce(r.is_sidechain,0) = 1 THEN 1 ELSE 0 END),0) AS subagent_requests,
                    count(DISTINCT r.session_id) AS sessions,
                    count(DISTINCT s.cwd) AS projects,
                    min(r.started_at) AS first_seen,
                    max(r.started_at) AS last_seen
             FROM requests r
             LEFT JOIN usage u    ON u.request_id = r.id
             LEFT JOIN request_cost c ON c.request_id = r.id
             LEFT JOIN sessions s ON s.id = r.session_id
             WHERE {SCOPE}"
        ),
        &p,
    )?;
    let quota = one(
        &conn,
        "SELECT rl_status, rl_5h_utilization, rl_7d_utilization, rl_5h_reset, rl_7d_reset,
                started_at
         FROM requests
         WHERE rl_status IS NOT NULL
           AND account_id = (SELECT id FROM accounts WHERE active = 1 LIMIT 1)
         ORDER BY started_at DESC LIMIT 1",
        &[],
    )?;
    let account = one(
        &conn,
        "SELECT id, label, email, org_name, subscription_type
         FROM accounts WHERE active = 1 LIMIT 1",
        &[],
    )?;
    let context = rows(
        &conn,
        &format!(
            "SELECT c.component, sum(c.est_tokens) AS est_tokens, sum(c.char_count) AS chars,
                    count(DISTINCT c.request_id) AS requests
             FROM context c
             JOIN requests r       ON r.id = c.request_id
             LEFT JOIN sessions s  ON s.id = r.session_id
             WHERE {SCOPE}
             GROUP BY c.component ORDER BY est_tokens DESC"
        ),
        &p,
    )?;
    let models = rows(
        &conn,
        &format!(
            "SELECT r.model, count(*) AS requests,
                    coalesce(sum(u.prompt_tokens),0)   AS prompt_tokens,
                    coalesce(sum(c.cost),0) AS cost,
                    coalesce(sum(u.output_tokens),0)   AS output_tokens,
                    coalesce(sum(u.thinking_tokens),0) AS thinking_tokens,
                    coalesce(sum(u.cache_read_tokens),0) AS cache_read_tokens,
                    coalesce(sum(CASE WHEN r.status >= 400 OR r.error IS NOT NULL THEN 1 ELSE 0 END),0) AS errors,
                    coalesce(sum(CASE WHEN coalesce(r.is_sidechain,0) = 1 THEN 1 ELSE 0 END),0) AS subagent_requests
             FROM requests r
             LEFT JOIN usage u    ON u.request_id = r.id
             LEFT JOIN request_cost c ON c.request_id = r.id
             LEFT JOIN sessions s ON s.id = r.session_id
             WHERE r.model IS NOT NULL AND {SCOPE}
             GROUP BY r.model ORDER BY prompt_tokens DESC"
        ),
        &p,
    )?;
    // Rows captured before the agent_id migration have NULL there; the coalesce keeps
    // them in the main-thread bucket instead of surfacing a third, nameless thread.
    let threads = rows(
        &conn,
        &format!(
            "SELECT CASE WHEN coalesce(r.is_sidechain,0) = 1 THEN 'subagent' ELSE 'main' END AS thread,
                    count(*) AS requests,
                    coalesce(sum(u.prompt_tokens),0) AS prompt_tokens,
                    coalesce(sum(c.cost),0) AS cost,
                    coalesce(sum(u.output_tokens),0) AS output_tokens,
                    coalesce(sum(u.cache_read_tokens),0) AS cache_read_tokens,
                    count(DISTINCT r.agent_id) AS agents
             FROM requests r
             LEFT JOIN usage u    ON u.request_id = r.id
             LEFT JOIN request_cost c ON c.request_id = r.id
             LEFT JOIN sessions s ON s.id = r.session_id
             WHERE {SCOPE}
             GROUP BY coalesce(r.is_sidechain,0) ORDER BY coalesce(r.is_sidechain,0)"
        ),
        &p,
    )?;
    let by_account = rows(
        &conn,
        "SELECT r.account_id AS id,
                coalesce(a.email, a.label, r.account_id) AS label,
                max(coalesce(a.active, 0)) AS active,
                count(*) AS requests,
                coalesce(sum(u.prompt_tokens),0) AS prompt_tokens,
                    coalesce(sum(c.cost),0) AS cost,
                coalesce(sum(u.output_tokens),0) AS output_tokens,
                count(DISTINCT r.session_id) AS sessions,
                max(r.started_at) AS last_seen
         FROM requests r
         LEFT JOIN usage u    ON u.request_id = r.id
         LEFT JOIN request_cost c ON c.request_id = r.id
         LEFT JOIN accounts a ON a.id = r.account_id
         WHERE (?1 IS NULL OR r.started_at >= ?1)
         GROUP BY r.account_id ORDER BY prompt_tokens DESC",
        &[&f.since],
    )?;
    let stop_reasons = rows(
        &conn,
        &format!(
            "SELECT coalesce(r.stop_reason, '(none)') AS stop_reason, count(*) AS requests
             FROM requests r LEFT JOIN sessions s ON s.id = r.session_id
             WHERE {SCOPE}
             GROUP BY 1 ORDER BY 2 DESC"
        ),
        &p,
    )?;
    let latency = one(
        &conn,
        &format!(
            "SELECT {} AS ttfb_p50, {} AS ttfb_p95, {} AS duration_p50, {} AS duration_p95,
                    (SELECT max(r.duration_ms) FROM requests r
                     LEFT JOIN sessions s ON s.id = r.session_id WHERE {SCOPE}) AS duration_max",
            pctile("ttfb_ms", 50),
            pctile("ttfb_ms", 95),
            pctile("duration_ms", 50),
            pctile("duration_ms", 95),
        ),
        &p,
    )?;

    // The timeline spans the requested range when one was given (so a quiet hour
    // still shows as an hour of quiet), and the data's own span otherwise.
    let bounds = one(
        &conn,
        &format!(
            "SELECT min(r.started_at) AS lo, max(r.started_at) AS hi
             FROM requests r LEFT JOIN sessions s ON s.id = r.session_id WHERE {SCOPE}"
        ),
        &p,
    )?;
    let lo = bounds.get("lo").and_then(Value::as_i64);
    let hi = bounds.get("hi").and_then(Value::as_i64);
    let tz_ms = f.tz.unwrap_or(0).clamp(-14 * 60, 14 * 60) * 60_000;
    let range = match (lo, hi) {
        (Some(lo), Some(hi)) => {
            let start = f.since.map_or(lo, |s| s.min(lo));
            let end = if f.since.is_some() { now.max(hi) } else { hi };
            Some((start, end))
        }
        // No requests in the window: still hand back the window, so the chart can
        // show an empty range rather than nothing at all.
        _ => f.since.map(|s| (s, now.max(s.saturating_add(1)))),
    };
    let (timeline, quota_history, bucket) = match range {
        Some((start, end)) => {
            let bucket = pick_bucket(end - start);
            let timeline = rows(
                &conn,
                &format!(
                    "SELECT ((r.started_at + ?5) / ?6) * ?6 - ?5 AS t, r.model,
                            count(*) AS requests,
                            coalesce(sum(u.prompt_tokens),0) AS prompt_tokens,
                    coalesce(sum(c.cost),0) AS cost,
                            coalesce(sum(u.output_tokens),0) AS output_tokens,
                            coalesce(sum(u.cache_read_tokens),0) AS cache_read_tokens,
                            coalesce(sum(CASE WHEN coalesce(r.is_sidechain,0) = 1 THEN u.prompt_tokens ELSE 0 END),0) AS subagent_tokens,
                            coalesce(sum(CASE WHEN r.status >= 400 OR r.error IS NOT NULL THEN 1 ELSE 0 END),0) AS errors
                     FROM requests r
                     LEFT JOIN usage u    ON u.request_id = r.id
                     LEFT JOIN request_cost c ON c.request_id = r.id
                     LEFT JOIN sessions s ON s.id = r.session_id
                     WHERE {SCOPE}
                     GROUP BY t, r.model ORDER BY t, r.model"
                ),
                &[&f.project, &f.session, &f.since, &f.account, &tz_ms, &bucket],
            )?;
            // Quota history is the active account's, over the same buckets so the
            // two charts share an axis. The worst reading in a bucket is the one
            // that matters for headroom.
            let quota_history = rows(
                &conn,
                "SELECT ((started_at + ?1) / ?2) * ?2 - ?1 AS t,
                        max(rl_5h_utilization) AS rl_5h, max(rl_7d_utilization) AS rl_7d
                 FROM requests
                 WHERE rl_status IS NOT NULL
                   AND started_at >= ?3 AND started_at <= ?4
                   AND account_id = (SELECT id FROM accounts WHERE active = 1 LIMIT 1)
                 GROUP BY t ORDER BY t",
                &[&tz_ms, &bucket, &start, &end],
            )?;
            (timeline, quota_history, bucket)
        }
        None => (Value::Array(vec![]), Value::Array(vec![]), 0),
    };
    Ok(Json(json!({
        "totals": totals, "quota": quota, "account": account, "context": context,
        "models": models, "threads": threads, "latency": latency,
        "stop_reasons": stop_reasons, "by_account": by_account,
        "timeline": {
            "start": range.map(|r| r.0), "end": range.map(|r| r.1),
            "bucket_ms": bucket, "rows": timeline,
        },
        "quota_history": quota_history,
    })))
}

/// The project navigator. Never narrowed by project or session (it is how you pick
/// one), but it does honour the time range.
pub async fn projects(State(state): State<Arc<AppState>>, Query(f): Query<Filter>) -> ApiResult {
    let conn = open(&state)?;
    Ok(Json(rows(
        &conn,
        "SELECT s.cwd AS project,
                count(DISTINCT s.id) AS sessions,
                count(*)             AS requests,
                coalesce(sum(u.prompt_tokens),0) AS prompt_tokens,
                    coalesce(sum(c.cost),0) AS cost,
                coalesce(sum(u.output_tokens),0) AS output_tokens,
                coalesce(sum(u.cache_read_tokens),0) AS cache_read_tokens,
                coalesce(sum(CASE WHEN coalesce(r.is_sidechain,0) = 1 THEN u.prompt_tokens ELSE 0 END),0) AS subagent_tokens,
                coalesce(sum(CASE WHEN r.status >= 400 OR r.error IS NOT NULL THEN 1 ELSE 0 END),0) AS errors,
                group_concat(DISTINCT r.model) AS models,
                min(r.started_at)    AS first_seen,
                max(r.started_at)    AS last_seen
         FROM requests r
         LEFT JOIN usage u    ON u.request_id = r.id
         LEFT JOIN request_cost c ON c.request_id = r.id
         JOIN sessions s   ON s.id = r.session_id
         WHERE (?1 IS NULL OR r.started_at >= ?1)
           AND (?2 IS NULL OR r.account_id = ?2)
         GROUP BY s.cwd ORDER BY prompt_tokens DESC",
        &[&f.since, &f.account],
    )?))
}

/// The session navigator: narrowed by project and time range, never by session.
pub async fn sessions(State(state): State<Arc<AppState>>, Query(f): Query<Filter>) -> ApiResult {
    let conn = open(&state)?;
    let sql = "SELECT s.id, s.cwd AS project, s.git_branch, s.entrypoint,
                      s.started_at, s.last_seen_at,
                      count(*) AS requests,
                      coalesce(sum(u.prompt_tokens),0) AS prompt_tokens,
                    coalesce(sum(c.cost),0) AS cost,
                      coalesce(sum(u.output_tokens),0) AS output_tokens,
                      coalesce(sum(u.cache_read_tokens),0) AS cache_read_tokens,
                      coalesce(sum(CASE WHEN coalesce(r.is_sidechain,0) = 1 THEN u.prompt_tokens ELSE 0 END),0) AS subagent_tokens,
                      coalesce(sum(CASE WHEN r.status >= 400 OR r.error IS NOT NULL THEN 1 ELSE 0 END),0) AS errors,
                      count(DISTINCT r.agent_id) AS subagents,
                      group_concat(DISTINCT r.model) AS models,
                      min(r.started_at) AS first_request_at,
                      max(r.started_at) AS last_request_at
               FROM requests r
               LEFT JOIN usage u    ON u.request_id = r.id
               LEFT JOIN request_cost c ON c.request_id = r.id
               JOIN sessions s   ON s.id = r.session_id
               WHERE (?1 IS NULL OR s.cwd = ?1) AND (?2 IS NULL OR r.started_at >= ?2)
                 AND (?3 IS NULL OR r.account_id = ?3)
               GROUP BY s.id ORDER BY s.last_seen_at DESC LIMIT 200";
    Ok(Json(rows(&conn, sql, &[&f.project, &f.since, &f.account])?))
}

pub async fn requests(State(state): State<Arc<AppState>>, Query(f): Query<Filter>) -> ApiResult {
    let conn = open(&state)?;
    let sql = format!(
        "SELECT r.id, r.request_id, r.session_id, s.cwd AS project, r.model, r.status,
                r.ttfb_ms, r.duration_ms, r.stop_reason, r.is_sidechain, r.agent_id,
                r.started_at, substr(r.error, 1, 200) AS error,
                u.prompt_tokens, u.input_tokens, u.output_tokens, u.thinking_tokens,
                c.cost AS cost,
                u.cache_read_tokens,
                u.cache_creation_5m_tokens + u.cache_creation_1h_tokens AS cache_write_tokens,
                (SELECT group_concat(name) FROM
                   (SELECT name FROM tool_invocations t WHERE t.request_id = r.id ORDER BY t.ordinal)
                ) AS tools,
                (SELECT group_concat(c.component || ':' || c.est_tokens)
                 FROM context c WHERE c.request_id = r.id) AS context
         FROM requests r
         LEFT JOIN usage u    ON u.request_id = r.id
         LEFT JOIN request_cost c ON c.request_id = r.id
         LEFT JOIN sessions s ON s.id = r.session_id
         WHERE {SCOPE}
         ORDER BY r.started_at DESC LIMIT 300"
    );
    Ok(Json(rows(&conn, &sql, &scope_params(&f))?))
}

pub async fn tools(State(state): State<Arc<AppState>>, Query(f): Query<Filter>) -> ApiResult {
    let conn = open(&state)?;
    // Results are keyed by tool_use_id and carry their own session and timestamp, so
    // they scope directly rather than through the request that first carried them.
    let produced = rows(
        &conn,
        "SELECT tc.name, count(*) AS results, sum(tc.result_chars) AS chars,
                sum(tc.est_tokens) AS est_tokens, max(tc.est_tokens) AS max_tokens,
                -- An estimate of an estimate, and labelled as one in the UI: these are
                -- guessed token counts for tool output, priced as if they were paid once
                -- as fresh input. The real cost is worse, because anything in the context
                -- is re-read on every later turn at the cache-read rate.
                coalesce(sum(tc.est_tokens * (
                    SELECT p.input FROM pricing p
                    WHERE instr(lower(coalesce(r.model,'')), p.model) > 0
                    ORDER BY length(p.model) DESC LIMIT 1)) / 1000000.0, 0) AS est_cost_once,
                sum(tc.is_error) AS errors
         FROM tool_calls tc
         LEFT JOIN sessions s ON s.id = tc.session_id
         LEFT JOIN requests r ON r.id = tc.first_request_id
         WHERE (?1 IS NULL OR s.cwd = ?1)
           AND (?2 IS NULL OR tc.session_id = ?2)
           AND (?3 IS NULL OR tc.seen_at >= ?3)
           AND (?4 IS NULL OR r.account_id = ?4)
         GROUP BY tc.name ORDER BY est_tokens DESC",
        &scope_params(&f),
    )?;
    let called = rows(
        &conn,
        &format!(
            "SELECT ti.name, count(*) AS calls,
                    count(DISTINCT ti.request_id) AS requests,
                    coalesce(sum(CASE WHEN coalesce(r.is_sidechain,0) = 1 THEN 1 ELSE 0 END),0) AS subagent_calls
             FROM tool_invocations ti
             JOIN requests r      ON r.id = ti.request_id
             LEFT JOIN sessions s ON s.id = r.session_id
             WHERE {SCOPE}
             GROUP BY ti.name ORDER BY calls DESC"
        ),
        &scope_params(&f),
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

#[derive(Deserialize)]
pub struct AccountEdit {
    pub id: String,
    pub label: Option<String>,
    /// The identity this account serves, when the router cannot discover it.
    pub email: Option<String>,
    /// Replace the credential this account signs with. Only meaningful for a `token`
    /// account — a Keychain account reads Claude Code's own entry, and a profile
    /// account reads its profile directory, so neither is ours to overwrite.
    pub token: Option<String>,
}

fn looks_like_email(s: &str) -> bool {
    let s = s.trim();
    match s.split_once('@') {
        Some((user, host)) => {
            !user.is_empty() && host.contains('.') && !host.starts_with('.') && !s.contains(' ')
        }
        None => false,
    }
}

/// Rename an account, or tell the router which identity it serves. The id and the
/// stored credential are untouched, so history keyed to this account stays intact.
pub async fn accounts_edit(
    State(state): State<Arc<AppState>>,
    Json(req): Json<AccountEdit>,
) -> ApiResult {
    let conn = rusqlite::Connection::open(&state.config.db_path)?;
    let label = req.label.map(|l| l.trim().to_string()).filter(|l| !l.is_empty());
    let email = req.email.map(|e| e.trim().to_string()).filter(|e| !e.is_empty());
    if let Some(email) = &email {
        if !looks_like_email(email) {
            return Err(ApiError(anyhow::anyhow!("{email:?} is not an email address")));
        }
    }
    let source: Option<String> = conn
        .query_row("SELECT source FROM accounts WHERE id = ?1", [&req.id], |r| {
            r.get::<_, Option<String>>(0)
        })
        .optional()?
        .flatten();
    let Some(source) = source else {
        return Err(ApiError(anyhow::anyhow!("no account with id {:?}", req.id)));
    };

    // Removing an account's stored session key must also clear the flag.
    // Rotating the credential: replace it in the Keychain, in place, so the account
    // keeps its id and everything recorded against it.
    if let Some(token) = req.token.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
        if source != "token" {
            return Err(ApiError(anyhow::anyhow!(
                "a {source} account takes its credential from Claude Code, so there is \
                 nothing here to replace — log in again instead"
            )));
        }
        if token.starts_with("sk-ant-sid") {
            return Err(ApiError(anyhow::anyhow!(
                "that is a claude.ai session key — it belongs in the session-key field, \
                 and cannot sign API requests"
            )));
        }
        if !token.starts_with("sk-ant-") {
            return Err(ApiError(anyhow::anyhow!(
                "expected a token starting sk-ant- (from `claude setup-token`)"
            )));
        }
        let id = req.id.clone();
        let token = token.to_string();
        tokio::task::spawn_blocking(move || crate::accounts::store_token(&id, &token)).await??;
    }

    conn.execute(
        "UPDATE accounts SET label = COALESCE(?2, label), email = COALESCE(?3, email)
         WHERE id = ?1",
        rusqlite::params![req.id, label, email],
    )?;
    // The cached credential carries the label and identity, so re-resolve it.
    state.invalidate().await;
    Ok(Json(json!({ "ok": true, "id": req.id })))
}

pub async fn accounts_list(State(state): State<Arc<AppState>>) -> ApiResult {
    let conn = open(&state)?;
    let mut list = rows(
        &conn,
        "SELECT id, label, source, profile_dir, email, org_name, subscription_type,
                active, created_at, coalesce(usage_key, 0) AS usage_key
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
                email: obj.get("email").and_then(Value::as_str).map(str::to_owned),
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
            let id_for_check = obj.get("id").and_then(Value::as_str).unwrap_or_default().to_string();
            obj.insert(
                "has_token".into(),
                json!(crate::accounts::has_token(&id_for_check)),
            );
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
                // A long-lived token cannot tell us who it belongs to — the usage
                // endpoint refuses it — so if the name given is an email address,
                // take it as the identity. It is the only signal available.
                let status = crate::accounts::AuthStatus {
                    email: looks_like_email(&req.label).then(|| req.label.trim().to_string()),
                    ..Default::default()
                };
                store.add(&id, &req.label, source, None, &status)?;
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

/// Health, and the pid a replacement instance needs in order to hand over.
pub async fn health() -> Json<Value> {
    Json(json!({
        "ok": true,
        "pid": std::process::id(),
        "version": env!("CARGO_PKG_VERSION"),
    }))
}
