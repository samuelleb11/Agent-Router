//! Read API for the dashboard, plus the embedded UI itself.
//!
//! Reads come from a small pool of long-lived connections. SQLite in WAL mode takes
//! concurrent readers alongside the capture task's single writer, so the pool is only
//! about what a connection carries: its page cache and its prepared statements. Opening
//! one per request threw both away every time, which against a multi-gigabyte store and
//! a handful of clients each polling every second or two cost more than the queries did.

use std::sync::{Arc, Mutex};

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
/// `(unresolved)` is the label the dashboard gives sessions with no project; no real
/// working directory is spelt that way, so it selects them rather than nothing.
const SCOPE: &str = "(?1 IS NULL OR s.cwd = ?1 OR (?1 = '(unresolved)' AND s.cwd IS NULL))
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

/// A few read-only connections, handed out and put back. Small on purpose: these queries
/// are CPU-bound on one core each, so more connections than there are cores buys queueing
/// rather than throughput.
pub struct ReadPool {
    path: std::path::PathBuf,
    idle: Mutex<Vec<Connection>>,
    max: usize,
}

impl ReadPool {
    pub fn new(path: impl Into<std::path::PathBuf>) -> Self {
        Self { path: path.into(), idle: Mutex::new(Vec::new()), max: 8 }
    }

    fn make(&self) -> anyhow::Result<Connection> {
        let conn = Connection::open_with_flags(
            &self.path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
        )?;
        // A connection's page cache is per connection, and the default holds about eight
        // megabytes against a store measured in gigabytes. Memory-mapped reads spare the
        // copy out of the page cache on the way in, and keeping temporary b-trees — which
        // the aggregates here do build — in memory keeps a big GROUP BY off the disk.
        conn.pragma_update(None, "cache_size", -65536)?; // 64 MB, negative means KiB
        conn.pragma_update(None, "mmap_size", 268_435_456i64)?; // 256 MB
        conn.pragma_update(None, "temp_store", "MEMORY")?;
        // Search reads the sidecar; every other query reads the main file. Attached
        // read-only and separately, because these connections are read-only and must
        // not be the ones that bring the bodies file into being — the capture task
        // creates it. A store that predates the index simply has no rows to match,
        // which is why a failure here is a warning and not a dead dashboard.
        let side = crate::db::bodies_path(&self.path);
        if let Err(err) = conn.execute(
            "ATTACH DATABASE ?1 AS bodies_db",
            [format!("file:{}?mode=ro", side.display())],
        ) {
            tracing::warn!(error = %err, path = %side.display(), "search index unavailable to readers");
        }
        Ok(conn)
    }

    pub fn get(&self) -> anyhow::Result<Pooled<'_>> {
        let held = self.idle.lock().unwrap().pop();
        let conn = match held {
            Some(c) => c,
            None => self.make()?,
        };
        Ok(Pooled { pool: self, conn: Some(conn) })
    }
}

/// A connection borrowed from the pool, returned when it goes out of scope.
pub struct Pooled<'a> {
    pool: &'a ReadPool,
    conn: Option<Connection>,
}

impl std::ops::Deref for Pooled<'_> {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        self.conn.as_ref().expect("connection is taken only on drop")
    }
}

impl Drop for Pooled<'_> {
    fn drop(&mut self) {
        if let Some(conn) = self.conn.take() {
            let mut idle = self.pool.idle.lock().unwrap();
            if idle.len() < self.pool.max {
                idle.push(conn);
            }
        }
    }
}

fn open(state: &AppState) -> anyhow::Result<Pooled<'_>> {
    state.reads.get()
}

/// A rendered answer, kept for a moment.
///
/// The dashboard, the menubar app and every phone ask the same handful of questions on
/// their own timers, and the expensive answers are aggregates over the whole store. One
/// client asking twice a second and twenty clients asking once each cost the same thing
/// here: the first caller computes, the rest read what it left. The window is shorter
/// than the fastest poll on the page, so nothing shown is staler than it already was.
///
/// The per-key lock is held across the computation on purpose — that is what makes a
/// crowd of identical requests one query instead of a stampede of them.
pub struct Memo {
    ttl: std::time::Duration,
    slots: Mutex<std::collections::HashMap<String, Arc<Mutex<Option<Fresh>>>>>,
}

type Fresh = (std::time::Instant, Arc<Value>);

impl Memo {
    pub fn new(ttl: std::time::Duration) -> Self {
        Self { ttl, slots: Mutex::new(std::collections::HashMap::new()) }
    }

    pub fn get_or_fill(
        &self,
        key: &str,
        fill: impl FnOnce() -> anyhow::Result<Value>,
    ) -> anyhow::Result<Value> {
        let slot = {
            let mut slots = self.slots.lock().unwrap();
            // Keys are filter combinations, so the set is small in practice but not
            // bounded by anything the router controls. Dropping the lot is fine: it
            // costs one recomputation each, not correctness.
            if slots.len() > 256 {
                slots.clear();
            }
            slots.entry(key.to_owned()).or_default().clone()
        };
        let mut held = slot.lock().unwrap();
        if let Some((at, value)) = held.as_ref() {
            if at.elapsed() < self.ttl {
                return Ok((**value).clone());
            }
        }
        let value = Arc::new(fill()?);
        *held = Some((std::time::Instant::now(), value.clone()));
        Ok((*value).clone())
    }
}

/// Everything that changes an answer, and nothing that does not.
fn memo_key(name: &str, f: &Filter) -> String {
    format!(
        "{name}|{:?}|{:?}|{:?}|{:?}|{:?}",
        f.project, f.session, f.since, f.account, f.tz
    )
}

/// Run a query and return its rows as JSON objects keyed by column name.
fn rows(conn: &Connection, sql: &str, params: &[&dyn rusqlite::ToSql]) -> anyhow::Result<Value> {
    let mut stmt = conn.prepare_cached(sql)?;
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
    // Clamp once, on the Filter itself, so every query below binds the same sane value:
    // the timeline does arithmetic on it, and i64::MAX would overflow (panic in debug).
    // Doing it before the key is built also keeps two spellings of "now" off separate
    // cache slots.
    f.since = f.since.map(|s| s.clamp(0, crate::db::now_ms()));
    let key = memo_key("summary", &f);
    Ok(Json(state.memo.get_or_fill(&key, || summary_inner(&state, &f))?))
}

fn summary_inner(state: &AppState, f: &Filter) -> anyhow::Result<Value> {
    let conn = open(state)?;
    summary_of(&conn, f)
}

/// The summary itself, from whichever connection it is handed: the pool's in the router,
/// a store built for the purpose in a test.
fn summary_of(conn: &Connection, f: &Filter) -> anyhow::Result<Value> {
    let now = crate::db::now_ms();
    let p = scope_params(f);
    let mut totals = one(
        conn,
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
                    max(r.started_at) AS last_seen,
                    coalesce(sum(CASE WHEN s.id IS NULL THEN 1 ELSE 0 END),0) AS unattributed
             FROM requests r
             LEFT JOIN usage u    ON u.request_id = r.id
             LEFT JOIN request_cost c ON c.request_id = r.id
             LEFT JOIN sessions s ON s.id = r.session_id
             WHERE {SCOPE}"
        ),
        &p,
    )?;
    // The requests in the scope that no session holds: Claude Code sent no session id, or
    // sent one the store has no session row for. Neither the Projects nor the Sessions
    // list can show them — both join on the session — so the dashboard ends each list
    // with a row counting them, and the lists never sum to less than the tiles.
    //
    // Counted here, because the page cannot count it. It used to take the total and
    // subtract the rows it had been handed, which is only right while a list is whole,
    // and `/api/sessions` hands back the 200 most recent sessions at most: past 200
    // sessions in scope, every request of an older session was blamed on a transcript
    // that had not appeared. Counted in the totals' own statement, so it comes from the
    // same read as `requests` and can never claim more than the tiles count; then moved
    // out beside them, because it is not a figure the tiles show but the row the lists
    // end with. `shift_remove` keeps the other totals in their order.
    let unattributed = totals
        .as_object_mut()
        .and_then(|t| t.shift_remove("unattributed"))
        .unwrap_or(Value::Null);
    let quota = one(
        conn,
        "SELECT rl_status, rl_5h_utilization, rl_7d_utilization, rl_5h_reset, rl_7d_reset,
                started_at
         FROM requests
         WHERE rl_status IS NOT NULL
           AND account_id = (SELECT id FROM accounts WHERE active = 1 LIMIT 1)
         ORDER BY started_at DESC LIMIT 1",
        &[],
    )?;
    let account = one(
        conn,
        "SELECT id, label, email, org_name, subscription_type
         FROM accounts WHERE active = 1 LIMIT 1",
        &[],
    )?;
    let context = rows(
        conn,
        &format!(
            "SELECT c.component, sum(c.est_tokens) AS est_tokens, sum(c.char_count) AS chars,
                    -- Priced as if each component were paid once as fresh input. It is
                    -- a floor: whatever stays in the context is re-read every later
                    -- turn, and this table cannot know how many turns that was.
                    --
                    -- Priced through `model_rate` rather than by matching the rate table
                    -- against the model name here: this sums a row per component per
                    -- request, so a match written inline runs tens of thousands of times
                    -- for the handful of distinct models it can produce.
                    coalesce(sum(c.est_tokens * p.input) / 1000000.0, 0) AS est_cost_once,
                    count(DISTINCT c.request_id) AS requests
             FROM context c
             JOIN requests r       ON r.id = c.request_id
             LEFT JOIN model_rate m ON m.model = r.model
             LEFT JOIN pricing p    ON p.model = m.key
             LEFT JOIN sessions s  ON s.id = r.session_id
             WHERE {SCOPE}
             GROUP BY c.component ORDER BY est_tokens DESC"
        ),
        &p,
    )?;
    let models = rows(
        conn,
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
        conn,
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
        conn,
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
        conn,
        &format!(
            "SELECT coalesce(r.stop_reason, '(none)') AS stop_reason, count(*) AS requests
             FROM requests r LEFT JOIN sessions s ON s.id = r.session_id
             WHERE {SCOPE}
             GROUP BY 1 ORDER BY 2 DESC"
        ),
        &p,
    )?;
    let latency = one(
        conn,
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
        conn,
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
                conn,
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
                conn,
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
    Ok(json!({
        "totals": totals, "unattributed": unattributed,
        "quota": quota, "account": account, "context": context,
        "models": models, "threads": threads, "latency": latency,
        "stop_reasons": stop_reasons, "by_account": by_account,
        "timeline": {
            "start": range.map(|r| r.0), "end": range.map(|r| r.1),
            "bucket_ms": bucket, "rows": timeline,
        },
        "quota_history": quota_history,
    }))
}

/// The project navigator. Never narrowed by project or session (it is how you pick
/// one), but it does honour the time range.
pub async fn projects(State(state): State<Arc<AppState>>, Query(f): Query<Filter>) -> ApiResult {
    let key = memo_key("projects", &f);
    Ok(Json(state.memo.get_or_fill(&key, || projects_inner(&state, &f))?))
}

fn projects_inner(state: &AppState, f: &Filter) -> anyhow::Result<Value> {
    let conn = open(state)?;
    Ok(rows(
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
    )?)
}

/// The session navigator: narrowed by project and time range, never by session.
pub async fn sessions(State(state): State<Arc<AppState>>, Query(f): Query<Filter>) -> ApiResult {
    let key = memo_key("sessions", &f);
    Ok(Json(state.memo.get_or_fill(&key, || sessions_inner(&state, &f))?))
}

fn sessions_inner(state: &AppState, f: &Filter) -> anyhow::Result<Value> {
    let conn = open(state)?;
    sessions_of(&conn, f)
}

/// The 200 most recently seen sessions in scope, and no more: a list to pick from, never
/// a ledger to add up. What falls outside it is still counted in the summary's totals,
/// and the requests no session holds are counted there too (`unattributed`).
fn sessions_of(conn: &Connection, f: &Filter) -> anyhow::Result<Value> {
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
    rows(conn, sql, &[&f.project, &f.since, &f.account])
}

/// What to look for, and where. Separate from `Filter` because none of the page-wide
/// controls mean the same thing here: a search is not scoped to the dashboard's time
/// window unless the caller says so.
#[derive(Deserialize)]
pub struct SearchQuery {
    pub q: String,
    pub project: Option<String>,
    pub session: Option<String>,
    /// text | thinking | tool_use | tool_result
    pub kind: Option<String>,
    pub since: Option<i64>,
    pub limit: Option<i64>,
}

/// Find every session and every message whose text matches.
///
/// Deliberately not memoized: the other endpoints answer a handful of fixed questions,
/// while the space of search queries is whatever anyone types, so a memo of it would
/// grow without bound and hit on almost nothing. It does not need one — the index is
/// the point.
pub async fn search(
    State(state): State<Arc<AppState>>,
    Query(f): Query<SearchQuery>,
) -> ApiResult {
    Ok(Json(search_inner(&state, &f)?))
}

fn search_inner(state: &AppState, f: &SearchQuery) -> anyhow::Result<Value> {
    let raw = f.q.trim();
    if raw.is_empty() {
        return Ok(json!({ "query": "", "expression": "", "sessions": [], "hits": [] }));
    }
    let conn = open(state)?;
    let limit = f.limit.unwrap_or(60).clamp(1, 500);

    // FTS5 has a query language of its own, and it is worth having: `"exact phrase"`,
    // `term*`, `NEAR(a b)` and `a OR b` all do the obvious thing. So the typed text is
    // offered to it verbatim first.
    //
    // But that language also rejects a great deal of ordinary typing — `don't`, a bare
    // `AND`, an unclosed bracket, a lone `*` — and it rejects each with a differently
    // worded error. Rather than try to tell its complaints apart from a real failure,
    // anything that does not run is retried once as a plain AND of quoted words. That
    // form is always valid FTS5, so if the retry fails too it is the store talking and
    // not the query, and it is allowed to surface.
    let (expression, hits) = match hits_for(&conn, raw, f, limit) {
        Ok(rows) => (raw.to_string(), rows),
        Err(_) => {
            let literal = literal_query(raw);
            (literal.clone(), hits_for(&conn, &literal, f, limit)?)
        }
    };
    let sessions = sessions_for(&conn, &expression, f)?;
    Ok(json!({
        "query": raw,
        "expression": expression,
        "sessions": sessions,
        "hits": hits,
    }))
}

/// The matching blocks themselves, best match first.
fn hits_for(
    conn: &Connection,
    expr: &str,
    f: &SearchQuery,
    limit: i64,
) -> anyhow::Result<Value> {
    let sql = "SELECT b.id AS block_id, b.kind, b.chars,
                      st.session_id, st.role, st.ordinal, st.request_id, st.at,
                      s.cwd AS project, s.label AS session_label,
                      snippet(blocks_fts, 0, '\u{2039}', '\u{203a}', ' \u{2026} ', 18) AS snippet,
                      bm25(blocks_fts) AS rank
               FROM bodies_db.blocks_fts
               JOIN bodies_db.blocks b       ON b.id  = blocks_fts.rowid
               JOIN bodies_db.block_sites st ON st.block_id = b.id
               LEFT JOIN main.sessions s     ON s.id  = st.session_id
               WHERE blocks_fts MATCH ?1
                 AND (?2 IS NULL OR s.cwd = ?2)
                 AND (?3 IS NULL OR st.session_id = ?3)
                 AND (?4 IS NULL OR b.kind = ?4)
                 AND (?5 IS NULL OR st.at >= ?5)
               ORDER BY rank LIMIT ?6";
    rows(conn, sql, &[&expr, &f.project, &f.session, &f.kind, &f.since, &limit])
}

/// The same matches rolled up per session — "which conversations talk about this".
fn sessions_for(conn: &Connection, expr: &str, f: &SearchQuery) -> anyhow::Result<Value> {
    let sql = "SELECT st.session_id, s.cwd AS project, s.label AS session_label,
                      s.git_branch, s.started_at,
                      count(DISTINCT b.id) AS blocks,
                      count(*) AS hits,
                      min(st.at) AS first_at, max(st.at) AS last_at
               FROM bodies_db.blocks_fts
               JOIN bodies_db.blocks b       ON b.id  = blocks_fts.rowid
               JOIN bodies_db.block_sites st ON st.block_id = b.id
               LEFT JOIN main.sessions s     ON s.id  = st.session_id
               WHERE blocks_fts MATCH ?1
                 AND (?2 IS NULL OR s.cwd = ?2)
                 AND (?3 IS NULL OR st.session_id = ?3)
                 AND (?4 IS NULL OR b.kind = ?4)
                 AND (?5 IS NULL OR st.at >= ?5)
               GROUP BY st.session_id
               ORDER BY hits DESC, last_at DESC LIMIT 200";
    rows(conn, sql, &[&expr, &f.project, &f.session, &f.kind, &f.since])
}

/// Every word as a literal term, all of them required. `"` is FTS5's own escape for a
/// quote inside a quoted string, which is what makes this safe for arbitrary input.
fn literal_query(raw: &str) -> String {
    // A quoted empty string is not a query; `MATCH ''` is an error rather than a miss.
    if raw.split_whitespace().next().is_none() {
        return "\"\"".into();
    }
    raw.split_whitespace()
        .map(|t| format!("\"{}\"", t.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" AND ")
}

pub async fn requests(State(state): State<Arc<AppState>>, Query(f): Query<Filter>) -> ApiResult {
    let key = memo_key("requests", &f);
    Ok(Json(state.memo.get_or_fill(&key, || requests_inner(&state, &f))?))
}

fn requests_inner(state: &AppState, f: &Filter) -> anyhow::Result<Value> {
    let conn = open(state)?;
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
    Ok(rows(&conn, &sql, &scope_params(f))?)
}

pub async fn tools(State(state): State<Arc<AppState>>, Query(f): Query<Filter>) -> ApiResult {
    let key = memo_key("tools", &f);
    Ok(Json(state.memo.get_or_fill(&key, || tools_inner(&state, &f))?))
}

fn tools_inner(state: &AppState, f: &Filter) -> anyhow::Result<Value> {
    let conn = open(state)?;
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
    Ok(json!({ "produced": produced, "called": called }))
}

// ---------------------------------------------------------------------------
// Forecast
// ---------------------------------------------------------------------------

/// The two rate-limit windows, paired with the columns that carry them.
struct Win {
    kind: crate::forecast::WindowKind,
    reset_col: &'static str,
    util_col: &'static str,
    /// Whether duty is the session you are in, or the habit you keep. See `duty_for`.
    session_duty: bool,
}

const WINDOWS: [Win; 2] = [
    Win {
        kind: crate::forecast::FIVE_HOUR,
        reset_col: "rl_5h_reset",
        util_col: "rl_5h_utilization",
        session_duty: true,
    },
    Win {
        kind: crate::forecast::SEVEN_DAY,
        reset_col: "rl_7d_reset",
        util_col: "rl_7d_utilization",
        session_duty: false,
    },
];

/// Every request an account made from `from_ms` onward, carrying the utilisation only
/// for rows belonging to `reset`. Rows outside that window — and rows that carried no
/// rate-limit header at all — still come back, because they are work, and work is the
/// denominator the burn rate divides by.
fn window_readings(
    conn: &Connection,
    account: &str,
    win: &Win,
    reset: i64,
    from_ms: i64,
) -> anyhow::Result<Vec<crate::forecast::Reading>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT r.started_at, CASE WHEN r.{reset} = ?2 THEN r.{util} END
         FROM requests r WHERE r.account_id = ?1 AND r.started_at >= ?3
         ORDER BY r.started_at",
        reset = win.reset_col,
        util = win.util_col,
    ))?;
    let out = stmt
        .query_map(rusqlite::params![account, reset, from_ms], |row| {
            Ok(crate::forecast::Reading { at_ms: row.get(0)?, util: row.get(1)? })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(out)
}

/// The account's own recent average, for a window still too young to have a slope.
/// The most recent completed window that yields one wins; looking past three is
/// reaching for data old enough to describe different habits.
fn trailing_burn(conn: &Connection, account: &str, win: &Win, before: i64) -> Option<f64> {
    let mut stmt = conn
        .prepare(&format!(
            "SELECT DISTINCT {reset} FROM requests
             WHERE account_id = ?1 AND {reset} IS NOT NULL AND {reset} < ?2
             ORDER BY {reset} DESC LIMIT 3",
            reset = win.reset_col
        ))
        .ok()?;
    let resets: Vec<i64> = stmt
        .query_map(rusqlite::params![account, before], |r| r.get(0))
        .ok()?
        .filter_map(Result::ok)
        .collect();
    for reset in resets {
        let from = (reset - win.kind.span_ms / 1000).saturating_mul(1000);
        let rs = window_readings(conn, account, win, reset, from).ok()?;
        if let Some((burn, _)) = crate::forecast::burn_from(&rs) {
            return Some(burn);
        }
    }
    None
}

/// How long a stretch of history counts as "your habits". The trailing window is the
/// recent estimate; complete days older than it are the baseline that estimate shrinks
/// toward, and what you were doing three weeks ago is not what you are doing now.
const HABIT_DAYS: i64 = 21;
const DAY_MS: i64 = 86_400_000;

/// Work ÷ clock, over a horizon chosen to match the window — with the spread of your
/// own days either side of it, for the windows where that spread is the whole question.
///
/// A **5-hour** window is forecast against the session you are in: once you are
/// working you tend to keep working, and the store bears that out — these windows run
/// at 71–100% duty from the first request to the last. So the clock starts at your
/// first request in the window, and the band has no width: this is a measurement of
/// the session, not a guess about a habit.
///
/// A **7-day** window is forecast against the habit, sleep included — 2.5–8.2 active
/// hours a day here. Measuring a week the way a session is measured is the difference
/// between "you will finish your week on Tuesday" and "you will not finish it at all":
/// a 7-day window that opened this morning has only a session inside it, and reading
/// 81% duty off that projects four days of solid work nobody was going to do.
///
/// The long horizon reports three numbers rather than one, because a week of long days
/// projected flat across four more is how a dead heat ends up rendered in red:
///
/// * `recent` — work ÷ clock over the trailing window, exactly as this used to return.
///   Numerator and denominator cover the same stretch, so it is unbiased as a rate.
/// * `baseline` — the mean of every complete day on record, which is what the days
///   ahead are more likely to look like than a hot streak is.
/// * `mid` — the two blended, weighted by how many days each speaks for. With only a
///   week on file there is nothing older to shrink toward, `baseline` and `recent`
///   describe the same days, and this is the old number to the digit; the shrinkage
///   only bites once there is genuinely older history. Recent habit never falls below
///   a third of the weight, so a real change of pace still moves it.
/// * the spread — the standard deviation of those daily duties, divided by the square
///   root of the days left. The forecast averages over the days between now and the
///   reset, so day-to-day variance shrinks accordingly, and the band tightens on its
///   own as the reset approaches.
fn duty_band_for(
    conn: &Connection,
    account: &str,
    win: &Win,
    readings: &[crate::forecast::Reading],
    reset_at: i64,
    now_ms: i64,
) -> anyhow::Result<crate::forecast::Duty> {
    use crate::forecast::{active_ms, Duty};

    if win.session_duty {
        let Some(first) = readings.first() else { return Ok(Duty::flat(0.0)) };
        let span = (now_ms - first.at_ms) as f64;
        if span <= 0.0 {
            return Ok(Duty::flat(0.0));
        }
        let times: Vec<i64> = readings.iter().map(|r| r.at_ms).collect();
        return Ok(Duty::flat((active_ms(&times) as f64 / span).clamp(0.0, 1.0)));
    }

    let from = now_ms - HABIT_DAYS * DAY_MS;
    let mut stmt = conn.prepare(
        "SELECT started_at FROM requests
         WHERE account_id = ?1 AND started_at >= ?2 ORDER BY started_at",
    )?;
    let times: Vec<i64> = stmt
        .query_map(rusqlite::params![account, from], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    let first_ever: Option<i64> = conn.query_row(
        "SELECT min(started_at) FROM requests WHERE account_id = ?1",
        [account],
        |r| r.get(0),
    )?;
    let since = first_ever.unwrap_or(from).max(from);

    let recent_from = (now_ms - win.kind.span_ms).max(since);
    let recent_span = (now_ms - recent_from) as f64;
    if recent_span <= 0.0 {
        return Ok(Duty::flat(0.0));
    }
    let recent_times: Vec<i64> = times.iter().copied().filter(|t| *t >= recent_from).collect();
    let recent = (active_ms(&recent_times) as f64 / recent_span).clamp(0.0, 1.0);

    // Complete 24-hour days, counted back from now. Anchoring to the clock rather than
    // the calendar dodges both timezones and the half-day at either end of the record:
    // every bucket is a full day, and a day with no work in it is a zero — which is
    // exactly the time off the flat estimate was pretending would not happen again.
    let complete = (((now_ms - since) / DAY_MS) as usize).min(HABIT_DAYS as usize);
    if complete < 3 {
        return Ok(Duty::flat(recent));
    }
    let mut per_day = vec![0i64; complete];
    for w in times.windows(2) {
        let idx = ((now_ms - w[0]) / DAY_MS) as usize;
        if idx < complete {
            per_day[idx] += (w[1] - w[0]).clamp(0, crate::forecast::IDLE_GAP_MS);
        }
    }
    let duties: Vec<f64> = per_day.iter().map(|ms| *ms as f64 / DAY_MS as f64).collect();

    let n_recent = (win.kind.span_ms / DAY_MS).max(1) as usize;
    let n_base = duties.len().saturating_sub(n_recent).min(2 * n_recent);
    let baseline = duties.iter().sum::<f64>() / duties.len() as f64;
    let mid = if n_base == 0 {
        recent
    } else {
        (n_recent as f64 * recent + n_base as f64 * baseline) / (n_recent + n_base) as f64
    };

    let var = duties.iter().map(|d| (d - baseline).powi(2)).sum::<f64>() / duties.len() as f64;
    let days_left =
        ((reset_at.saturating_mul(1000) - now_ms) as f64 / DAY_MS as f64).max(1.0);
    Ok(Duty::spread(mid, var.sqrt() / days_left.sqrt()))
}

/// Rate-limit windows projected to their end, for every account, with a rollup.
///
/// Unlike every other read in this file, this one ignores `project` and `session`
/// entirely: a quota belongs to an account, not to whatever you happened to spend it
/// on. Only the usage figures honour `since`.
pub async fn forecast(State(state): State<Arc<AppState>>, Query(mut f): Query<Filter>) -> ApiResult {
    f.since = f.since.map(|s| s.clamp(0, crate::db::now_ms()));
    let key = memo_key("forecast", &f);
    Ok(Json(state.memo.get_or_fill(&key, || forecast_inner(&state, &f))?))
}

fn forecast_inner(state: &AppState, f: &Filter) -> anyhow::Result<Value> {
    let conn = open(state)?;
    let now = crate::db::now_ms();
    let since = f.since;
    let week_ago = now - 7 * 86_400_000;

    // Driven from `accounts` but unioned with anything the request log references, so a
    // removed account's history is still reported rather than silently dropped.
    let ids = rows(
        &conn,
        "SELECT a.id AS id, coalesce(a.email, a.label, a.id) AS label, a.email AS email,
                a.subscription_type AS plan, coalesce(a.active, 0) AS active
         FROM accounts a
         UNION
         SELECT DISTINCT r.account_id, r.account_id, NULL, NULL, 0
         FROM requests r
         WHERE r.account_id IS NOT NULL
           AND r.account_id NOT IN (SELECT id FROM accounts)",
        &[],
    )?;

    let mut out = Vec::new();
    for item in ids.as_array().map(Vec::as_slice).unwrap_or_default() {
        let Some(id) = item.get("id").and_then(Value::as_str) else { continue };

        let usage = one(
            &conn,
            "SELECT count(*) AS requests,
                    coalesce(sum(u.prompt_tokens),0)  AS prompt_tokens,
                    coalesce(sum(u.output_tokens),0)  AS output_tokens,
                    coalesce(sum(c.cost),0)           AS cost,
                    max(r.started_at)                 AS last_seen
             FROM requests r
             LEFT JOIN usage u        ON u.request_id = r.id
             LEFT JOIN request_cost c ON c.request_id = r.id
             WHERE r.account_id = ?1 AND (?2 IS NULL OR r.started_at >= ?2)",
            &[&id, &since],
        )?;

        // Spend over the trailing week, whatever range the page is showing. A pace is
        // only meaningful against a fixed horizon.
        let week = one(
            &conn,
            "SELECT coalesce(sum(c.cost),0) AS cost,
                    coalesce(sum(u.prompt_tokens),0) AS prompt_tokens,
                    count(*) AS requests
             FROM requests r
             LEFT JOIN usage u        ON u.request_id = r.id
             LEFT JOIN request_cost c ON c.request_id = r.id
             WHERE r.account_id = ?1 AND r.started_at >= ?2",
            &[&id, &week_ago],
        )?;

        let latest = one(
            &conn,
            "SELECT rl_5h_reset, rl_7d_reset, rl_status, started_at
             FROM requests
             WHERE account_id = ?1 AND rl_status IS NOT NULL
             ORDER BY started_at DESC LIMIT 1",
            &[&id],
        )?;
        let seen_at = latest.get("started_at").and_then(Value::as_i64);

        let mut windows = Map::new();
        for win in &WINDOWS {
            let reset = latest.get(win.reset_col).and_then(Value::as_i64);
            let Some(reset) = reset else {
                windows.insert(win.kind.name.into(), Value::Null);
                continue;
            };
            // A window whose reset has already passed rolled over while we were not
            // looking. We know nothing about the one that replaced it — reporting the
            // old utilisation as current would be a lie with a number on it.
            if reset.saturating_mul(1000) <= now {
                windows.insert(
                    win.kind.name.into(),
                    json!({ "expired": true, "reset_at": reset, "seen_at": seen_at }),
                );
                continue;
            }
            let from_ms = (reset - win.kind.span_ms / 1000).saturating_mul(1000);
            let readings = window_readings(&conn, id, win, reset, from_ms)?;
            let fallback = if crate::forecast::burn_from(&readings).is_none() {
                trailing_burn(&conn, id, win, reset)
            } else {
                None
            };
            let fc = crate::forecast::forecast(
                &readings,
                win.kind,
                reset,
                now,
                fallback,
                duty_band_for(&conn, id, win, &readings, reset, now)?,
            );
            // A downsampled curve for the burn-down chart. The worst reading in a
            // bucket is the one that matters for headroom, so max() — the same rule
            // `summary`'s quota history uses.
            let bucket = (win.kind.span_ms / 48).max(1);
            let series = rows(
                &conn,
                &format!(
                    "SELECT (r.started_at / ?3) * ?3 AS t, max(r.{util}) AS u
                     FROM requests r
                     WHERE r.account_id = ?1 AND r.{reset} = ?2 AND r.{util} IS NOT NULL
                     GROUP BY t ORDER BY t",
                    util = win.util_col,
                    reset = win.reset_col,
                ),
                &[&id, &reset, &bucket],
            )?;
            let mut v = serde_json::to_value(&fc)?;
            if let Some(o) = v.as_object_mut() {
                o.insert("expired".into(), json!(false));
                o.insert("seen_at".into(), json!(seen_at));
                o.insert("series".into(), series);
            }
            windows.insert(win.kind.name.into(), v);
        }

        let mut row = item.clone();
        if let Some(o) = row.as_object_mut() {
            o.insert("usage".into(), usage);
            o.insert("week".into(), week);
            o.insert("seen_at".into(), json!(seen_at));
            o.insert("status".into(), latest.get("rl_status").cloned().unwrap_or(Value::Null));
            o.insert(
                "pool".into(),
                latest.get("rl_7d_reset").cloned().unwrap_or(Value::Null),
            );
            o.extend(windows);
        }
        out.push(row);
    }

    Ok(json!({
        "generated_at": now,
        "since": since,
        "accounts": out,
        "global": global_rollup(&out, now),
    }))
}

/// What totals across accounts and what does not.
///
/// Spend, tokens and requests add up. Utilisation does not — each account's percentage
/// is of its own limit — so headroom is totalled in the one unit that composes: hours
/// of work left. Two credentials onto one Anthropic account share a quota and report
/// the same 7-day reset, so they are counted once.
///
/// This is a reporting figure. The router does not route around a spent limit; the
/// active-account switch stays where it was, in the user's hands.
fn global_rollup(accounts: &[Value], now: i64) -> Value {
    let sum = |key: &str, field: &str| -> f64 {
        accounts
            .iter()
            .filter_map(|a| a.get(key)?.get(field)?.as_f64())
            .sum()
    };

    let mut pools: Map<String, Value> = Map::new();
    let mut soonest: Option<(i64, String, &str)> = None;
    for a in accounts {
        let id = a.get("id").and_then(Value::as_str).unwrap_or("").to_string();
        let pool = a
            .get("pool")
            .and_then(Value::as_i64)
            .map_or_else(|| id.clone(), |p| p.to_string());
        for key in ["five_hour", "seven_day"] {
            let Some(w) = a.get(key).filter(|w| w.is_object()) else { continue };
            if w.get("expired").and_then(Value::as_bool).unwrap_or(false) {
                continue;
            }
            if let Some(at) = w.get("exhausts_at").and_then(Value::as_i64) {
                if soonest.as_ref().is_none_or(|(best, ..)| at < *best) {
                    soonest = Some((at, id.clone(), key));
                }
            }
            // Per pool, the binding constraint is whichever window runs dry first.
            if let Some(left) = w.get("active_hours_left").and_then(Value::as_f64) {
                let slot = pools.entry(pool.clone()).or_insert_with(|| json!(left));
                if slot.as_f64().is_some_and(|cur| left < cur) {
                    *slot = json!(left);
                }
            }
        }
    }
    let headroom: f64 = pools.values().filter_map(Value::as_f64).sum();

    let week_cost = sum("week", "cost");
    let week_tokens = sum("week", "prompt_tokens");
    json!({
        "requests":      sum("usage", "requests"),
        "prompt_tokens": sum("usage", "prompt_tokens"),
        "output_tokens": sum("usage", "output_tokens"),
        "cost":          sum("usage", "cost"),
        "accounts":      accounts.len(),
        "combined_headroom": {
            "active_hours_left": if pools.is_empty() { Value::Null } else { json!(headroom) },
            "pools": pools.len(),
            "first_to_exhaust": soonest.map(|(at, id, window)| json!({
                "account_id": id, "window": window, "at": at, "in_ms": at - now,
            })),
        },
        "pace_30d": {
            "cost":          week_cost * 30.0 / 7.0,
            "prompt_tokens": week_tokens * 30.0 / 7.0,
            "basis":         "the last 7 days of recorded spend",
        },
    })
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

/// What `GET /api/accounts` takes.
#[derive(Deserialize)]
pub struct AccountsQuery {
    /// `1` asks for every account's credential to be checked. Anything else, or nothing,
    /// is the cheap read: the rows as stored, with no credential touched.
    pub probe: Option<String>,
}

impl AccountsQuery {
    fn probing(&self) -> bool {
        self.probe.as_deref() == Some("1")
    }
}

/// What checking one account's credential found.
struct Credential {
    /// Whether the router could produce a token to sign with right now.
    ready: bool,
    /// The token's kind when it could, and the whole reason when it could not.
    detail: String,
    /// When that token stops working, in ms since the epoch: 0 when it never does, or
    /// when there was no token to read an expiry from.
    expires_at: i64,
    /// Whether the router holds a token of its own for this account. The value never
    /// leaves the router; this only lets the edit form say the field is already filled.
    has_token: bool,
}

/// Check one credential the way the router would sign with it. Blocking, and not cheap:
/// a Claude Code login is resolved by running `claude auth status` and reading Claude
/// Code's Keychain entry or the profile's credentials file, and a token account's token —
/// like `has_token`, for every account — is read from the router's secrets file.
fn check_credential(account: &crate::accounts::Account) -> Credential {
    let (ready, detail, expires_at) = match crate::accounts::resolve(account) {
        Ok(r) => (true, crate::accounts::token_kind(&r.token).to_string(), r.expires_at_ms),
        Err(e) => (false, e.to_string(), 0),
    };
    Credential { ready, detail, expires_at, has_token: crate::accounts::has_token(&account.id) }
}

/// `GET /api/accounts`: every configured account, oldest first — and, only when the caller
/// asks with `probe=1`, whether each one's credential works right now.
///
/// Checking used to happen on every call, and it is not free: a process and a Keychain
/// read per account, on every poll of every open dashboard tab and of the menubar app,
/// every 30 seconds, whether or not anyone was looking at an account. The rows alone say
/// what a poll mostly needs — the names, and which one is active. So a check is asked for
/// by whatever shows its result: the dashboard while its Settings are open, the iPhone's
/// Accounts page when it opens.
///
/// A row nobody asked about carries no `ready`, `detail`, `expires_at` or `has_token`.
/// Absent rather than false, because "not checked" is a different fact from "checked, and
/// it failed", and the dashboard says "not checked" for it. A hosted router answers
/// through this same handler (see `hosted`), and a check there runs no process: `resolve`
/// refuses a Claude Code login before it would run `claude`, and a pasted token is read
/// from the secrets file.
pub async fn accounts_list(
    State(state): State<Arc<AppState>>,
    Query(q): Query<AccountsQuery>,
) -> ApiResult {
    Ok(Json(accounts_of(&state.reads, &q, check_credential).await?))
}

/// The answer, with the check handed in: `check_credential` in the router, and in a test
/// one that only counts, so no test ever reaches a Keychain or a `claude` binary.
async fn accounts_of<C>(reads: &ReadPool, q: &AccountsQuery, check: C) -> anyhow::Result<Value>
where
    C: Fn(&crate::accounts::Account) -> Credential + Clone + Send + 'static,
{
    // Read, and give the connection back to the pool before anything is checked: a check
    // can take seconds, and the warm connection is better spent on the next dashboard read.
    let mut list = {
        let conn = reads.get()?;
        rows(
            &conn,
            "SELECT id, label, source, profile_dir, email, org_name, subscription_type,
                    active, created_at, coalesce(usage_key, 0) AS usage_key
             FROM accounts ORDER BY created_at",
            &[],
        )?
    };
    if !q.probing() {
        return Ok(list);
    }

    // Report whether each account can actually produce a token right now, so a
    // half-finished profile login is visible in the UI instead of failing later. A
    // blocking task per account, so a check that panics fails its own row, not the list.
    for obj in list.as_array_mut().into_iter().flatten().filter_map(Value::as_object_mut) {
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
        let check = check.clone();
        match tokio::task::spawn_blocking(move || check(&account)).await {
            Ok(found) => {
                obj.insert("has_token".into(), json!(found.has_token));
                obj.insert("ready".into(), json!(found.ready));
                obj.insert("detail".into(), json!(found.detail));
                obj.insert("expires_at".into(), json!(found.expires_at));
            }
            // Whether a token is stored was part of what never came back, so it is left
            // unsaid rather than guessed.
            Err(e) => {
                obj.insert("ready".into(), json!(false));
                obj.insert("detail".into(), json!(e.to_string()));
                obj.insert("expires_at".into(), json!(0));
            }
        }
    }
    Ok(list)
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
    // A hosted router can only sign with a pasted token. Switching to a Claude Code login
    // that came with a copied store would leave every request with nothing to sign with.
    let server = state.config.server;
    tokio::task::spawn_blocking(move || {
        if server && store.source_of(&id)? != Some(crate::accounts::Source::Token) {
            anyhow::bail!("{}", crate::accounts::NOT_ON_A_SERVER);
        }
        store.activate(&id)
    })
    .await??;
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
    // On a hosted router there is no discovery to bring it back, and a `default` row
    // can only have come with a store copied from a machine: it can never sign there.
    if req.id == crate::proxy::ACCOUNT_ID && !state.config.server {
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

/// Health, and the pid a replacement instance needs in order to hand over. `build` names
/// the source tree the binary was compiled from (see `build.rs`): the crate version is
/// the same for every build, so it cannot say whether what serves is what was deployed.
pub async fn health() -> Json<Value> {
    Json(json!({
        "ok": true,
        "pid": std::process::id(),
        "version": env!("CARGO_PKG_VERSION"),
        "build": env!("AGENT_ROUTER_BUILD"),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::{Account, Source};
    use rusqlite::params;

    /// A store of its own under the temp directory, with the schema and the rate table the
    /// router sets up at startup, read through a pool like the router's. Never the real
    /// store: nothing here knows where that is.
    struct Store {
        dir: std::path::PathBuf,
        /// For the test to fill the store with.
        conn: Connection,
        reads: ReadPool,
    }

    impl Drop for Store {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn store(tag: &str) -> Store {
        let dir = std::env::temp_dir().join(format!("ar-api-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("agent-router.db");
        crate::db::Db::open(&path).unwrap();
        let conn = Connection::open(&path).unwrap();
        crate::pricing::ensure_table(&conn).unwrap();
        Store { reads: ReadPool::new(&path), conn, dir }
    }

    fn scope(
        project: Option<&str>,
        session: Option<&str>,
        account: Option<&str>,
        since: Option<i64>,
    ) -> Filter {
        Filter {
            project: project.map(str::to_owned),
            session: session.map(str::to_owned),
            account: account.map(str::to_owned),
            since,
            tz: None,
        }
    }

    /// The dashboard's "not yet attributed" row used to be the total less the rows of the
    /// Sessions list — and that list stops at the 200 most recent sessions, so past 200
    /// every request of an older session was counted there as if its transcript were
    /// missing. The router counts the row itself now: the requests in scope that carry no
    /// session, which is to say no session id, or one the store holds no session for.
    #[test]
    fn unattributed_is_counted_by_the_router_past_the_200_sessions_the_list_holds() {
        const T0: i64 = 1_700_000_000_000;
        let store = store("unattributed");
        let tx = store.conn.unchecked_transaction().unwrap();
        let request = "INSERT INTO requests
                         (request_id, session_id, account_id, method, path, started_at)
                       VALUES (?1, ?2, ?3, 'POST', '/v1/messages', ?4)";
        // 250 sessions in two projects, two requests each: more than the list holds.
        for i in 0..250i64 {
            let id = format!("session-{i:03}");
            let at = T0 + i * 1_000;
            let cwd = if i % 2 == 0 { "/code/a" } else { "/code/b" };
            tx.execute(
                "INSERT INTO sessions (id, cwd, started_at, last_seen_at) VALUES (?1, ?2, ?3, ?4)",
                params![id, cwd, at, at + 500],
            )
            .unwrap();
            for n in 1..=2i64 {
                let rid = format!("{id}-{n}");
                tx.execute(request, params![rid, id, "work", at + n * 100]).unwrap();
            }
        }
        // Seven with no session id at all, split between two accounts...
        for k in 0..7i64 {
            let account = if k % 2 == 0 { "work" } else { "side" };
            let none: Option<&str> = None;
            let at = T0 + k * 10_000;
            tx.execute(request, params![format!("orphan-{k}"), none, account, at]).unwrap();
        }
        // ...and two naming a session the store has no row for, which neither list can
        // show either: both join on the session.
        for k in 0..2i64 {
            let ghost = format!("ghost-{k}");
            tx.execute(request, params![ghost, ghost, "work", T0 + 260_000 + k]).unwrap();
        }
        tx.commit().unwrap();

        let conn = store.reads.get().unwrap();
        let everything = scope(None, None, None, None);
        let listed = sessions_of(&conn, &everything).unwrap();
        let listed = listed.as_array().unwrap();
        assert_eq!(listed.len(), 200, "the Sessions list stops at the 200 most recent");

        let summary = summary_of(&conn, &everything).unwrap();
        assert_eq!(summary["totals"]["requests"], 509);
        // Seven without a session id, and two whose session has no row.
        assert_eq!(summary["unattributed"], 9);
        assert!(summary["totals"].get("unattributed").is_none(), "said once, beside the totals");
        // What the page worked out instead: the 50 sessions past the cap, 100 requests,
        // blamed on transcripts that had not appeared.
        let in_list: i64 = listed.iter().map(|s| s["requests"].as_i64().unwrap()).sum();
        assert_eq!(509 - in_list, 109);

        // Only what is in scope counts: the range, and the account.
        let since = summary_of(&conn, &scope(None, None, None, Some(T0 + 30_000))).unwrap();
        assert_eq!(since["unattributed"], 6, "the last four without an id, and both ghosts");
        let side = summary_of(&conn, &scope(None, None, Some("side"), None)).unwrap();
        assert_eq!(side["totals"]["requests"], 3);
        assert_eq!(side["unattributed"], 3);
        // A project or a session in scope holds only requests that have one.
        let project = summary_of(&conn, &scope(Some("/code/a"), None, None, None)).unwrap();
        assert_eq!(project["totals"]["requests"], 250);
        assert_eq!(project["unattributed"], 0);
        let session = summary_of(&conn, &scope(None, Some("session-007"), None, None)).unwrap();
        assert_eq!(session["totals"]["requests"], 2);
        assert_eq!(session["unattributed"], 0);
    }

    /// GET /api/accounts used to check every credential on every call — `claude auth
    /// status` and a Keychain read per account — for the menubar's poll and the
    /// dashboard's alike. It checks only when asked now, and then each account once.
    #[tokio::test]
    async fn accounts_probe_checks_each_credential_once_and_only_when_asked() {
        let store = store("accounts");
        store
            .conn
            .execute_batch(
                "INSERT INTO accounts (id, label, source, profile_dir, email, active, created_at)
                 VALUES ('default', 'me@example.com', 'keychain', NULL, 'me@example.com', 1, 1),
                        ('work', 'work', 'token', NULL, NULL, 0, 2),
                        ('side', 'side', 'profile', '/nowhere/side', NULL, 0, 3);",
            )
            .unwrap();

        // In place of `claude auth status`, the Keychain and the secrets file: it answers
        // from the account alone, and keeps a list of whom it was asked about.
        let asked = Arc::new(Mutex::new(Vec::<String>::new()));
        let check = {
            let asked = asked.clone();
            move |a: &Account| {
                asked.lock().unwrap().push(a.id.clone());
                Credential {
                    ready: a.source != Source::Profile,
                    detail: format!("checked as {}", a.source.as_str()),
                    expires_at: 0,
                    has_token: a.source == Source::Token,
                }
            }
        };
        // Parsed the way the handler's `Query` parses it.
        let query = |uri: &str| {
            let uri: axum::http::Uri = uri.parse().unwrap();
            Query::<AccountsQuery>::try_from_uri(&uri).unwrap().0
        };

        // The menubar's read and the dashboard's poll: the rows as stored, nothing checked,
        // and nothing said about any credential.
        for uri in ["/api/accounts", "/api/accounts?probe=0", "/api/accounts?probe="] {
            let list = accounts_of(&store.reads, &query(uri), check.clone()).await.unwrap();
            assert!(asked.lock().unwrap().is_empty(), "{uri} checked a credential");
            let list = list.as_array().unwrap();
            assert_eq!(list.len(), 3, "{uri}");
            assert_eq!(list[0]["active"], 1, "{uri} still says which account is active");
            for row in list {
                for unsaid in ["ready", "detail", "expires_at", "has_token"] {
                    assert!(row.get(unsaid).is_none(), "{uri}: an unchecked row says {unsaid}");
                }
            }
        }

        // Asked: every account checked, each exactly once, each row saying what its own
        // check found.
        let list = accounts_of(&store.reads, &query("/api/accounts?probe=1"), check.clone())
            .await
            .unwrap();
        let mut seen = asked.lock().unwrap().clone();
        seen.sort();
        assert_eq!(seen, ["default", "side", "work"]);
        for row in list.as_array().unwrap() {
            let source = row["source"].as_str().unwrap();
            assert_eq!(row["ready"], source != "profile", "{source}");
            assert_eq!(row["detail"], format!("checked as {source}"));
            assert_eq!(row["has_token"], source == "token", "{source}");
            assert_eq!(row["expires_at"], 0, "{source}");
        }
    }
}
