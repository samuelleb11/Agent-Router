//! Read API for the console (`console` serves the console itself).
//!
//! Reads come from a small pool of long-lived connections. SQLite in WAL mode takes
//! concurrent readers alongside the capture task's single writer, so the pool is only
//! about what a connection carries: its page cache and its prepared statements. Opening
//! one per request threw both away every time, which against a multi-gigabyte store and
//! a handful of clients each polling every second or two cost more than the queries did.

use std::sync::{Arc, Mutex};

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use rusqlite::types::ValueRef;
use rusqlite::{Connection, OpenFlags};
use serde::Deserialize;
use serde_json::{json, Map, Value};

use crate::proxy::AppState;

#[derive(Deserialize)]
pub struct Filter {
    pub project: Option<String>,
    pub session: Option<String>,
    /// Which configured account served the request. Absent means every account.
    pub account: Option<String>,
    /// Lower bound on `requests.started_at`, in ms since the epoch. A time range the
    /// dashboard applies to everything on the page at once.
    pub since: Option<i64>,
    /// Upper bound on `requests.started_at`, exclusive, beside `since`: what a session cost
    /// between two instants (Northstar's ADR 0028 asks it of the summary).
    pub until: Option<i64>,
    /// The viewer's UTC offset in minutes (JavaScript's `-getTimezoneOffset()`), so
    /// day-sized timeline buckets start at the viewer's midnight rather than UTC's.
    pub tz: Option<i64>,
}

/// Whether the session joined as `s` is in the project bound as parameter `?n`. A project
/// is `s.project`: the repository the session worked in, else its working directory
/// (`places`). `(unresolved)` is the label the dashboard gives sessions with no project; no
/// repository or directory is spelt that way, so it selects them rather than nothing.
///
/// The one project test: `SCOPE` is built from it, and so is every read that cannot take
/// the whole of `SCOPE` because its other bounds are not a request's (tool results, search).
/// Reads that wrote the test out again for themselves left out its `(unresolved)` branch,
/// and in that scope the Sessions, tool Results and search lists came up empty while the
/// summary counted the sessions they should have shown.
macro_rules! in_project {
    ($n:literal) => {
        concat!("(?", $n, " IS NULL OR s.project = ?", $n, " OR (?", $n, " = '(unresolved)' AND s.project IS NULL))")
    };
}

/// The scope predicate every read shares. Requests are joined to their session as
/// `s` (LEFT, so a request whose transcript has not appeared yet still counts) and
/// the parameters are always bound in this order: project, session, since, account, until.
const SCOPE: &str = concat!(
    in_project!(1),
    "
                     AND (?2 IS NULL OR r.session_id = ?2)
                     AND (?3 IS NULL OR r.started_at >= ?3)
                     AND (?4 IS NULL OR r.account_id = ?4)
                     AND (?5 IS NULL OR r.started_at < ?5)"
);

fn scope_params(f: &Filter) -> [&dyn rusqlite::ToSql; 5] {
    [&f.project, &f.session, &f.since, &f.account, &f.until]
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
        "{name}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}",
        f.project, f.session, f.since, f.account, f.tz, f.until
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

/// The parts of the summary a caller can name in `parts=`, in the order the answer gives
/// them. `totals` brings `unattributed` with it: both come from one statement.
const SUMMARY_PARTS: [&str; 11] = [
    "totals",
    "quota",
    "account",
    "context",
    "models",
    "threads",
    "latency",
    "stop_reasons",
    "by_account",
    "timeline",
    "quota_history",
];

/// `GET /api/summary`'s own parameter, beside the scope every read shares.
#[derive(Deserialize)]
pub struct SummaryQuery {
    /// The parts to answer, comma-separated. Absent means every part, which is what the
    /// phones ask for; the console asks only for what it is showing.
    pub parts: Option<String>,
}

/// The parts asked for, in the answer's order and each once, so two spellings of one list
/// share a memo slot. Every part when none is named; a refusal that names a part the
/// summary does not have, or a `parts=` that names none.
fn summary_parts(asked: Option<&str>) -> Result<Vec<&'static str>, Box<Response>> {
    let Some(asked) = asked else { return Ok(SUMMARY_PARTS.to_vec()) };
    let names: Vec<&str> = asked.split(',').map(str::trim).filter(|s| !s.is_empty()).collect();
    if names.is_empty() {
        let why = format!("parts= names no part; the summary's parts are {}", SUMMARY_PARTS.join(", "));
        return Err(Box::new(refuse(StatusCode::BAD_REQUEST, why)));
    }
    if let Some(unknown) = names.iter().find(|n| !SUMMARY_PARTS.contains(n)) {
        let why = format!("{unknown:?} is not a part of the summary; its parts are {}", SUMMARY_PARTS.join(", "));
        return Err(Box::new(refuse(StatusCode::BAD_REQUEST, why)));
    }
    Ok(SUMMARY_PARTS.into_iter().filter(|p| names.contains(p)).collect())
}

/// The summary's memo slot: its scope and the parts it answers.
fn summary_key(f: &Filter, parts: &[&str]) -> String {
    memo_key(&format!("summary[{}]", parts.join(",")), f)
}

/// Everything the dashboard shows above the lists, in one round trip, scoped by the
/// same project / session / since filter as the lists so every figure on the page
/// agrees with every other. Quota is the exception: it belongs to the account, not
/// the project, so it always describes the active account.
///
/// `parts=` narrows it to the parts named, and only their statements run: the console
/// asks for what its open tab shows, not for everything every five seconds.
pub async fn summary(
    State(state): State<Arc<AppState>>,
    Query(mut f): Query<Filter>,
    Query(asked): Query<SummaryQuery>,
) -> Response {
    let parts = match summary_parts(asked.parts.as_deref()) {
        Ok(parts) => parts,
        Err(refused) => return *refused,
    };
    // Clamp once, on the Filter itself, so every query below binds the same sane value:
    // the timeline does arithmetic on it, and i64::MAX would overflow (panic in debug).
    // Doing it before the key is built also keeps two spellings of "now" off separate
    // cache slots.
    f.since = f.since.map(|s| s.clamp(0, crate::db::now_ms()));
    let key = summary_key(&f, &parts);
    match state.memo.get_or_fill(&key, || summary_inner(&state, &f, &parts)) {
        Ok(value) => Json(value).into_response(),
        Err(e) => ApiError(e).into_response(),
    }
}

fn summary_inner(state: &AppState, f: &Filter, parts: &[&str]) -> anyhow::Result<Value> {
    let conn = open(state)?;
    summary_parts_of(&conn, f, parts)
}

/// The whole summary, every part.
#[cfg(test)]
fn summary_of(conn: &Connection, f: &Filter) -> anyhow::Result<Value> {
    summary_parts_of(conn, f, &SUMMARY_PARTS)
}

/// The summary itself, from whichever connection it is handed: the pool's in the router,
/// a store built for the purpose in a test. Only the parts named are read and answered.
fn summary_parts_of(conn: &Connection, f: &Filter, parts: &[&str]) -> anyhow::Result<Value> {
    let want = |part: &str| parts.contains(&part);
    let mut out = Map::new();
    let now = crate::db::now_ms();
    let p = scope_params(f);
    if want("totals") {
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
                        count(DISTINCT s.project) AS projects,
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
        out.insert("totals".into(), totals);
        out.insert("unattributed".into(), unattributed);
    }
    if want("quota") {
        let quota = one(
            conn,
            "SELECT rl_status, rl_5h_utilization, rl_7d_utilization, rl_5h_reset, rl_7d_reset,
                    started_at
             FROM requests
             WHERE rl_status IS NOT NULL
               AND account_id = (SELECT id FROM accounts WHERE active = 1 AND provider = 'anthropic' LIMIT 1)
             ORDER BY started_at DESC LIMIT 1",
            &[],
        )?;
        out.insert("quota".into(), quota);
    }
    if want("account") {
        let account = one(
            conn,
            "SELECT id, label, email, org_name, subscription_type
             FROM accounts WHERE active = 1 AND provider = 'anthropic' LIMIT 1",
            &[],
        )?;
        out.insert("account".into(), account);
    }
    if want("context") {
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
        out.insert("context".into(), context);
    }
    if want("models") {
        let models = rows(
            conn,
            &format!(
                "SELECT r.model, coalesce(max(r.provider), 'anthropic') AS provider, count(*) AS requests,
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
        out.insert("models".into(), models);
    }
    if want("threads") {
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
        out.insert("threads".into(), threads);
    }
    if want("latency") {
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
        out.insert("latency".into(), latency);
    }
    if want("stop_reasons") {
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
        out.insert("stop_reasons".into(), stop_reasons);
    }
    if want("by_account") {
        let by_account = rows(
            conn,
            "SELECT r.account_id AS id,
                    coalesce(a.email, a.label, r.account_id) AS label,
                    -- The account's own, else what its requests were signed for: an account
                    -- removed since keeps naming its provider.
                    coalesce(max(a.provider), max(r.provider), 'anthropic') AS provider,
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
               AND (?2 IS NULL OR r.started_at < ?2)
             GROUP BY r.account_id ORDER BY prompt_tokens DESC",
            &[&f.since, &f.until],
        )?;
        out.insert("by_account".into(), by_account);
    }
    if !want("timeline") && !want("quota_history") {
        return Ok(Value::Object(out));
    }

    // The timeline spans the requested range when one was given (so a quiet hour
    // still shows as an hour of quiet), and the data's own span otherwise. The quota
    // history is drawn over the same buckets, so it takes the same span.
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
            let end = match f.until {
                Some(until) => until.max(start.saturating_add(1)),
                None if f.since.is_some() => now.max(hi),
                None => hi,
            };
            Some((start, end))
        }
        // No requests in the window: still hand back the window, so the chart can
        // show an empty range rather than nothing at all.
        _ => f.since.map(|s| (s, f.until.unwrap_or(now).max(s.saturating_add(1)))),
    };
    let none = || Value::Array(vec![]);
    let (timeline, quota_history, bucket) = match range {
        Some((start, end)) => {
            let bucket = pick_bucket(end - start);
            let timeline = if want("timeline") {
                rows(
                    conn,
                    &format!(
                        "SELECT ((r.started_at + ?6) / ?7) * ?7 - ?6 AS t, r.model,
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
                    &[&f.project, &f.session, &f.since, &f.account, &f.until, &tz_ms, &bucket],
                )?
            } else {
                none()
            };
            // Quota history is the active account's, over the same buckets so the
            // two charts share an axis. The worst reading in a bucket is the one
            // that matters for headroom.
            let quota_history = if want("quota_history") {
                rows(
                    conn,
                    "SELECT ((started_at + ?1) / ?2) * ?2 - ?1 AS t,
                            max(rl_5h_utilization) AS rl_5h, max(rl_7d_utilization) AS rl_7d
                     FROM requests
                     WHERE rl_status IS NOT NULL
                       AND started_at >= ?3 AND started_at <= ?4
                       AND account_id = (SELECT id FROM accounts WHERE active = 1 AND provider = 'anthropic' LIMIT 1)
                     GROUP BY t ORDER BY t",
                    &[&tz_ms, &bucket, &start, &end],
                )?
            } else {
                none()
            };
            (timeline, quota_history, bucket)
        }
        None => (none(), none(), 0),
    };
    if want("timeline") {
        out.insert(
            "timeline".into(),
            json!({
                "start": range.map(|r| r.0), "end": range.map(|r| r.1),
                "bucket_ms": bucket, "rows": timeline,
            }),
        );
    }
    if want("quota_history") {
        out.insert("quota_history".into(), quota_history);
    }
    Ok(Value::Object(out))
}

/// The project navigator. Never narrowed by project or session (it is how you pick
/// one), but it does honour the time range.
pub async fn projects(State(state): State<Arc<AppState>>, Query(f): Query<Filter>) -> ApiResult {
    let key = memo_key("projects", &f);
    Ok(Json(state.memo.get_or_fill(&key, || projects_inner(&state, &f))?))
}

fn projects_inner(state: &AppState, f: &Filter) -> anyhow::Result<Value> {
    let conn = open(state)?;
    projects_of(&conn, f)
}

/// Every project, with what it spent in the range and where it ran: each directory its
/// sessions started in, the machine that listed them, and the directory's git remotes. A
/// project is its repository on every machine, else a directory (`places`), and Northstar
/// binds its own projects to these by those remotes (its ADR 0028).
fn projects_of(conn: &Connection, f: &Filter) -> anyhow::Result<Value> {
    let mut projects = rows(
        conn,
        "SELECT s.project AS project,
                max(s.repository)    AS repository,
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
           AND (?3 IS NULL OR r.started_at < ?3)
         GROUP BY s.project ORDER BY prompt_tokens DESC",
        &[&f.since, &f.account, &f.until],
    )?;

    // Where each project ran. A directory no machine has listed is named once, and only
    // where no machine has listed it either: it is the same directory, placed or not yet.
    let mut stmt = conn.prepare_cached(
        "SELECT DISTINCT s.project, s.cwd, s.machine, m.label, d.remotes
           FROM sessions s
           LEFT JOIN directories d ON d.machine = s.machine AND d.cwd = s.cwd
           LEFT JOIN machines m ON m.id = s.machine
          WHERE s.cwd IS NOT NULL
          ORDER BY s.project, m.label, s.cwd",
    )?;
    type Place = (Option<String>, String, Option<String>, Option<String>, Option<String>);
    let places: Vec<Place> =
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?.collect::<Result<_, _>>()?;
    let mut by_project: std::collections::HashMap<Option<String>, Vec<Value>> = Default::default();
    for (project, cwd, machine, label, remotes) in &places {
        let placed = |m: &Option<String>| m.as_deref().is_some_and(|m| !m.is_empty());
        if !placed(machine) && places.iter().any(|(p, c, m, _, _)| p == project && c == cwd && placed(m)) {
            continue;
        }
        let machine = match machine.as_deref().filter(|m| !m.is_empty()) {
            Some(id) => json!({ "id": id, "label": label }),
            None => Value::Null,
        };
        let remotes = remotes.as_deref().and_then(|r| serde_json::from_str::<Value>(r).ok()).unwrap_or_else(|| json!([]));
        by_project.entry(project.clone()).or_default().push(json!({ "cwd": cwd, "machine": machine, "remotes": remotes }));
    }
    if let Value::Array(list) = &mut projects {
        for p in list.iter_mut() {
            let key = p.get("project").and_then(Value::as_str).map(str::to_owned);
            p["directories"] = Value::Array(by_project.remove(&key).unwrap_or_default());
        }
    }
    Ok(projects)
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
///
/// Scoped by `SCOPE` itself, so a project lists the sessions the summary counts for it,
/// `(unresolved)` included. The one bound it never takes is the session: the list is how
/// a session is picked, so that place in the scope is bound empty.
pub(crate) fn sessions_of(conn: &Connection, f: &Filter) -> anyhow::Result<Value> {
    let sql = format!(
        "SELECT s.id, s.project, s.cwd, s.repository, s.machine AS machine_id, m.label AS machine_label,
                s.git_branch, s.entrypoint, s.started_at, s.last_seen_at,
                count(*) AS requests,
                coalesce(sum(u.prompt_tokens),0) AS prompt_tokens,
                coalesce(sum(c.cost),0) AS cost,
                coalesce(sum(u.output_tokens),0) AS output_tokens,
                coalesce(sum(u.cache_read_tokens),0) AS cache_read_tokens,
                coalesce(sum(CASE WHEN coalesce(r.is_sidechain,0) = 1 THEN u.prompt_tokens ELSE 0 END),0) AS subagent_tokens,
                coalesce(sum(CASE WHEN r.status >= 400 OR r.error IS NOT NULL THEN 1 ELSE 0 END),0) AS errors,
                count(DISTINCT r.agent_id) AS subagents,
                group_concat(DISTINCT r.model) AS models,
                group_concat(DISTINCT coalesce(r.provider, 'anthropic')) AS providers,
                min(r.started_at) AS first_request_at,
                max(r.started_at) AS last_request_at
         FROM requests r
         LEFT JOIN usage u    ON u.request_id = r.id
         LEFT JOIN request_cost c ON c.request_id = r.id
         JOIN sessions s   ON s.id = r.session_id
         LEFT JOIN machines m ON m.id = s.machine
         WHERE {SCOPE}
         GROUP BY s.id ORDER BY s.last_seen_at DESC LIMIT 200"
    );
    let every_session: Option<String> = None;
    let mut listed = rows(conn, &sql, &[&f.project, &every_session, &f.since, &f.account, &f.until])?;
    // The machine that listed it, as the live rows name one: absent on a local router.
    if let Value::Array(list) = &mut listed {
        for row in list.iter_mut().filter_map(Value::as_object_mut) {
            let id = row.remove("machine_id");
            let label = row.remove("machine_label").unwrap_or(Value::Null);
            let machine = match id {
                Some(Value::String(id)) if !id.is_empty() => json!({ "id": id, "label": label }),
                _ => Value::Null,
            };
            row.insert("machine".into(), machine);
            // Which client it was, from the User-Agent its requests carried.
            let client = row.get("entrypoint").and_then(Value::as_str).and_then(crate::analyze::client_of);
            row.insert("client".into(), json!(client));
            // Which providers served it, Anthropic's first: one session can move between them.
            let mut providers: Vec<String> =
                row.get("providers").and_then(Value::as_str).unwrap_or_default().split(',').filter(|p| !p.is_empty()).map(str::to_owned).collect();
            providers.sort_by_key(|p| (p != "anthropic", p.clone()));
            row.insert("providers".into(), json!(providers));
        }
    }
    Ok(listed)
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
    let sql = format!(
        "SELECT b.id AS block_id, b.kind, b.chars,
                st.session_id, st.role, st.ordinal, st.request_id, st.at,
                s.project AS project, s.label AS session_label,
                snippet(blocks_fts, 0, '\u{2039}', '\u{203a}', ' \u{2026} ', 18) AS snippet,
                bm25(blocks_fts) AS rank
         FROM bodies_db.blocks_fts
         JOIN bodies_db.blocks b       ON b.id  = blocks_fts.rowid
         JOIN bodies_db.block_sites st ON st.block_id = b.id
         LEFT JOIN main.sessions s     ON s.id  = st.session_id
         WHERE blocks_fts MATCH ?1
           AND {project}
           AND (?3 IS NULL OR st.session_id = ?3)
           AND (?4 IS NULL OR b.kind = ?4)
           AND (?5 IS NULL OR st.at >= ?5)
         ORDER BY rank LIMIT ?6",
        project = in_project!(2),
    );
    rows(conn, &sql, &[&expr, &f.project, &f.session, &f.kind, &f.since, &limit])
}

/// The same matches rolled up per session — "which conversations talk about this".
fn sessions_for(conn: &Connection, expr: &str, f: &SearchQuery) -> anyhow::Result<Value> {
    let sql = format!(
        "SELECT st.session_id, s.project AS project, s.label AS session_label,
                s.git_branch, s.started_at,
                count(DISTINCT b.id) AS blocks,
                count(*) AS hits,
                min(st.at) AS first_at, max(st.at) AS last_at
         FROM bodies_db.blocks_fts
         JOIN bodies_db.blocks b       ON b.id  = blocks_fts.rowid
         JOIN bodies_db.block_sites st ON st.block_id = b.id
         LEFT JOIN main.sessions s     ON s.id  = st.session_id
         WHERE blocks_fts MATCH ?1
           AND {project}
           AND (?3 IS NULL OR st.session_id = ?3)
           AND (?4 IS NULL OR b.kind = ?4)
           AND (?5 IS NULL OR st.at >= ?5)
         GROUP BY st.session_id
         ORDER BY hits DESC, last_at DESC LIMIT 200",
        project = in_project!(2),
    );
    rows(conn, &sql, &[&expr, &f.project, &f.session, &f.kind, &f.since])
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
    requests_of(&conn, f)
}

/// The 300 most recent requests in scope, from whichever connection it is handed.
pub(crate) fn requests_of(conn: &Connection, f: &Filter) -> anyhow::Result<Value> {
    let sql = format!(
        "SELECT r.id, r.request_id, r.session_id, s.project AS project, r.model, r.status,
                r.account_id, coalesce(r.provider, 'anthropic') AS provider,
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
    Ok(rows(conn, &sql, &scope_params(f))?)
}

pub async fn tools(State(state): State<Arc<AppState>>, Query(f): Query<Filter>) -> ApiResult {
    let key = memo_key("tools", &f);
    Ok(Json(state.memo.get_or_fill(&key, || tools_inner(&state, &f))?))
}

fn tools_inner(state: &AppState, f: &Filter) -> anyhow::Result<Value> {
    let conn = open(state)?;
    tools_of(&conn, f)
}

/// What was called, and what came back, from whichever connection it is handed, as with
/// `summary_of`.
fn tools_of(conn: &Connection, f: &Filter) -> anyhow::Result<Value> {
    // Results are keyed by tool_use_id and carry their own session and timestamp, so
    // they scope directly rather than through the request that first carried them: the
    // project test every read shares, and their own bounds for the rest.
    let produced = rows(
        conn,
        &format!(
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
             WHERE {project}
               AND (?2 IS NULL OR tc.session_id = ?2)
               AND (?3 IS NULL OR tc.seen_at >= ?3)
               AND (?4 IS NULL OR r.account_id = ?4)
               AND (?5 IS NULL OR tc.seen_at < ?5)
             GROUP BY tc.name ORDER BY est_tokens DESC",
            project = in_project!(1),
        ),
        &scope_params(f),
    )?;
    let called = rows(
        conn,
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
        &scope_params(f),
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
/// An account's windows from the readings its provider's usage API gave (`quota_readings`),
/// forecast as Anthropic's are: the work in the window is every request the account served,
/// and each reading is a utilisation at the time it was taken. A window the provider said is
/// spent (`spent_windows`) reads full until it resets, whatever the readings say, since its
/// counters can lag; the account is then `rejected`, as a spent Claude account is. A window
/// nothing gives is `null`; one whose reset has passed is `expired`. The second window carries
/// its own name: `seven_day`, else `month`.
fn reading_windows(
    conn: &Connection,
    id: &str,
    now: i64,
) -> anyhow::Result<(Map<String, Value>, Option<i64>, Option<&'static str>)> {
    use rusqlite::OptionalExtension;
    let mut windows = Map::new();
    windows.insert(crate::forecast::FIVE_HOUR.name.into(), Value::Null);
    windows.insert(crate::forecast::SEVEN_DAY.name.into(), Value::Null);
    let latest: Option<(i64, Option<i64>, Option<String>, Option<i64>)> = conn
        .query_row(
            "SELECT at, five_hour_reset, second_window, second_reset FROM quota_readings
             WHERE account_id = ?1 ORDER BY at DESC LIMIT 1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?;
    let spent = crate::quota::spent_windows(conn, id, now)?;
    let marker = |name: &str| spent.iter().find(|(w, _, _)| w == name).map(|(_, at, until)| (*at, *until));
    if latest.is_none() && spent.is_empty() {
        return Ok((windows, None, None));
    }
    let read = latest.is_some();
    let (read_at, five_reset, second_name, second_reset) = latest.unwrap_or((0, None, None, None));
    let seen_at = spent.iter().map(|(_, at, _)| *at).chain(read.then_some(read_at)).max();
    let month = second_name.as_deref() == Some("month") || (second_name.is_none() && marker("month").is_some());
    let second_kind = if month { crate::forecast::MONTH } else { crate::forecast::SEVEN_DAY };
    for (kind, reading_reset, used_col, reset_col, session_duty) in [
        (crate::forecast::FIVE_HOUR, five_reset, "five_hour_used", "five_hour_reset", true),
        (second_kind, second_reset, "second_used", "second_reset", false),
    ] {
        let spent_here = marker(kind.name);
        let Some(reset) = spent_here.map(|(_, until)| until).or(reading_reset) else { continue };
        if reset.saturating_mul(1000) <= now {
            windows.insert(kind.name.into(), json!({ "expired": true, "reset_at": reset, "seen_at": seen_at }));
            continue;
        }
        let from_ms = (reset - kind.span_ms / 1000).saturating_mul(1000);
        // No costs: these readings are of Kimi Code's usage, not of the requests that spent
        // it, so there is no load to weigh and the window carries no short trend.
        let mut readings: Vec<crate::forecast::Reading> = {
            let mut stmt = conn.prepare(
                "SELECT started_at FROM requests WHERE account_id = ?1 AND started_at >= ?2 ORDER BY started_at",
            )?;
            let work = stmt
                .query_map(rusqlite::params![id, from_ms], |r| Ok(crate::forecast::Reading { at_ms: r.get(0)?, util: None, cost: 0.0 }))?
                .collect::<Result<Vec<_>, _>>()?;
            work
        };
        {
            let mut stmt = conn.prepare(&format!(
                "SELECT at, {used_col} FROM quota_readings
                 WHERE account_id = ?1 AND {reset_col} = ?2 AND at >= ?3 AND {used_col} IS NOT NULL ORDER BY at"
            ))?;
            let taken = stmt
                .query_map(rusqlite::params![id, reset, from_ms], |r| {
                    Ok(crate::forecast::Reading { at_ms: r.get(0)?, util: Some(r.get(1)?), cost: 0.0 })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            readings.extend(taken);
        }
        // What the provider said outweighs a reading taken before or after it.
        if let Some((at, _)) = spent_here {
            readings.retain(|r| r.util.is_none());
            readings.push(crate::forecast::Reading { at_ms: at, util: Some(1.0), cost: 0.0 });
        }
        readings.sort_by_key(|r| r.at_ms);
        let win = Win { kind, reset_col: "", util_col: "", session_duty };
        let fc = crate::forecast::forecast(&readings, kind, reset, now, None, duty_band_for(conn, id, &win, &readings, reset, now)?);
        let mut v = serde_json::to_value(&fc)?;
        if let Some(o) = v.as_object_mut() {
            o.insert("expired".into(), json!(false));
            o.insert("seen_at".into(), json!(seen_at));
            o.insert("series".into(), json!([]));
            o.insert("spent".into(), json!(spent_here.is_some()));
        }
        windows.insert(kind.name.into(), v);
    }
    Ok((windows, seen_at, (!spent.is_empty()).then_some("rejected")))
}

fn window_readings(
    conn: &Connection,
    account: &str,
    win: &Win,
    reset: i64,
    from_ms: i64,
) -> anyhow::Result<Vec<crate::forecast::Reading>> {
    // Each request's cost is its weight in the load the short trend compares; one the
    // router could not price weighs nothing rather than counting as free work.
    let mut stmt = conn.prepare(&format!(
        "SELECT r.started_at, CASE WHEN r.{reset} = ?2 THEN r.{util} END, coalesce(c.cost, 0)
         FROM requests r LEFT JOIN request_cost c ON c.request_id = r.id
         WHERE r.account_id = ?1 AND r.started_at >= ?3
         ORDER BY r.started_at",
        reset = win.reset_col,
        util = win.util_col,
    ))?;
    let out = stmt
        .query_map(rusqlite::params![account, reset, from_ms], |row| {
            Ok(crate::forecast::Reading { at_ms: row.get(0)?, util: row.get(1)?, cost: row.get(2)? })
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

pub(crate) fn forecast_inner(state: &AppState, f: &Filter) -> anyhow::Result<Value> {
    let conn = open(state)?;
    let now = crate::db::now_ms();
    let since = f.since;
    let week_ago = now - 7 * 86_400_000;

    // Driven from `accounts` but unioned with anything the request log references, so a
    // removed account's history is still reported rather than silently dropped.
    let ids = rows(
        &conn,
        "SELECT a.id AS id, coalesce(a.email, a.label, a.id) AS label, a.email AS email,
                a.subscription_type AS plan, coalesce(a.active, 0) AS active, a.provider AS provider
         FROM accounts a
         UNION
         SELECT DISTINCT r.account_id, r.account_id, NULL, NULL, 0, NULL
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
                    -- Requests with usage the router could not price: a cost of 0 beside them
                    -- is not free, and the dashboard says so rather than showing $0.
                    coalesce(sum(CASE WHEN u.request_id IS NOT NULL AND c.request_id IS NULL
                                      THEN 1 ELSE 0 END),0) AS unpriced,
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

        // A provider that sends no rate-limit headers reports its windows through its own
        // usage API instead, and the router keeps each reading (`quota::kimi_quota`).
        if item.get("provider").and_then(Value::as_str) == Some("kimi") {
            let (windows, seen_at, status) = reading_windows(&conn, id, now)?;
            let mut row = item.clone();
            if let Some(o) = row.as_object_mut() {
                o.insert("usage".into(), usage);
                o.insert("week".into(), week);
                o.insert("seen_at".into(), json!(seen_at));
                o.insert("status".into(), json!(status));
                o.insert("pool".into(), Value::Null);
                o.extend(windows);
            }
            out.push(row);
            continue;
        }

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
//
// The writes refuse in `refusal`'s shape: 400 and `{ok: false, error}` for a request that
// is wrong whatever the store holds, 409 for one the store's state stands in the way of,
// and 500 only for the router failing. They all answered 500 once, refusals included, so a
// client could not tell a name already taken from a disk that would not write.

use crate::accounts::{Conflict, Provider, Source, Store};
use crate::refusal::{failed, refuse, JsonBody};

/// What an accounts write the store did not make answers: 409 and its words when the
/// store's state stood in the way ([`Conflict`]), and 500 when the store failed.
pub(crate) fn not_made(e: anyhow::Error) -> Response {
    let (status, why) = why_not(e);
    refuse(status, why)
}

/// The status and the words of an answer decided inside a blocking task, which becomes
/// a response once it is back out: a whole `Response` is too big to return through every
/// step of one.
type Unmade = (StatusCode, String);

fn why_not(e: anyhow::Error) -> Unmade {
    match e.downcast::<Conflict>() {
        Ok(conflict) => (StatusCode::CONFLICT, conflict.to_string()),
        Err(e) => failure(e),
    }
}

/// The router failed, and nothing the caller sent was wrong (see `refusal::failed`).
fn failure(e: impl std::fmt::Display) -> Unmade {
    (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}

#[derive(Deserialize)]
pub struct AddAccount {
    pub label: String,
    pub source: String,
    /// Required for `token` accounts: the value `claude setup-token` printed, or a Kimi
    /// Code console key.
    pub token: Option<String>,
    /// `anthropic` when absent: every client before there was a second provider sends none.
    pub provider: Option<String>,
    /// Where a Kimi Code account's requests go, when not to its provider's default.
    pub base_url: Option<String>,
}

/// The base URL an account of `provider` is stored with, from the one an add gave, or why
/// that one cannot be. Kimi Code's default is stored rather than left implicit, so the host
/// a key was added for is the host it keeps signing for.
fn base_url_for(provider: Provider, given: Option<&str>) -> Result<Option<String>, String> {
    let given = given.map(str::trim).filter(|u| !u.is_empty());
    match (provider.default_base_url(), given) {
        (None, None) => Ok(None),
        (None, Some(_)) => Err(format!(
            "an {} account goes to the router's own upstream, and takes no base URL",
            provider.name()
        )),
        (Some(default), None) => Ok(Some(default.to_string())),
        (Some(_), Some(url)) => {
            let url = url.trim_end_matches('/');
            let parsed: Option<axum::http::Uri> = url.parse().ok();
            match parsed {
                Some(uri) if uri.scheme_str() == Some("https") && uri.authority().is_some() => {
                    Ok(Some(url.to_string()))
                }
                _ => Err(format!("{url:?} is not an https:// address")),
            }
        }
    }
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
///
/// What the edit carries is checked first, against nothing the store holds (400); then
/// the account it names, which may be gone or may not be the router's to give a token
/// (409). A new credential is checked once the account is read, against its provider's
/// format (400): an Anthropic token and a Kimi Code key look nothing alike.
pub async fn accounts_edit(
    State(state): State<Arc<AppState>>,
    JsonBody(req): JsonBody<AccountEdit>,
) -> Response {
    let label = req.label.map(|l| l.trim().to_string()).filter(|l| !l.is_empty());
    let email = req.email.map(|e| e.trim().to_string()).filter(|e| !e.is_empty());
    if let Some(email) = &email {
        if !looks_like_email(email) {
            return refuse(StatusCode::BAD_REQUEST, format!("{email:?} is not an email address"));
        }
    }
    let token = req.token.as_deref().map(str::trim).filter(|t| !t.is_empty()).map(str::to_owned);

    let store = Store::new(state.config.db_path.clone());
    let id = req.id.clone();
    let done = tokio::task::spawn_blocking(move || -> Result<(), Unmade> {
        let Some(account) = store.get(&id).map_err(failure)? else {
            return Err((StatusCode::CONFLICT, Conflict::NoAccount(id).to_string()));
        };
        // Rotating the credential: replace it in the secrets file, in place, so the
        // account keeps its id and everything recorded against it.
        if let Some(token) = token {
            // Whose format a credential must have is the account's provider's, so it is
            // read before the credential is judged.
            if let Some(why) = account.provider.key_problem(&token) {
                return Err((StatusCode::BAD_REQUEST, why.to_string()));
            }
            if account.source != Source::Token {
                return Err((
                    StatusCode::CONFLICT,
                    format!(
                        "a {} account takes its credential from Claude Code, so there is \
                         nothing here to replace — log in again instead",
                        account.source.as_str()
                    ),
                ));
            }
            crate::accounts::store_token(&id, &token).map_err(failure)?;
        }
        store.edit(&id, label.as_deref(), email.as_deref()).map_err(why_not)
    })
    .await;
    match done {
        Ok(Ok(())) => {
            // The cached credential carries the label and identity, so re-resolve it.
            state.invalidate().await;
            Json(json!({ "ok": true, "id": req.id })).into_response()
        }
        Ok(Err((status, why))) => refuse(status, why),
        Err(e) => failed(e),
    }
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
        Ok(r) => (true, crate::accounts::credential_kind(account.provider, &r.token).to_string(), r.expires_at_ms),
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
            "SELECT id, label, source, provider, base_url, profile_dir, email, org_name,
                    subscription_type, active, created_at, coalesce(usage_key, 0) AS usage_key
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
            provider: Provider::parse(obj.get("provider").and_then(Value::as_str).unwrap_or("anthropic"))
                .unwrap_or(Provider::Anthropic),
            base_url: obj.get("base_url").and_then(Value::as_str).map(str::to_owned),
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

/// Add an account: a pasted `claude setup-token` value, or a Claude Code profile the
/// router makes for a login run in a terminal. A sign-in the router drives itself, the
/// third way in, is `login`.
///
/// Adding never replaces. The id is the name made safe (`accounts::slugify`), and a name
/// whose id another account has — a "Work" beside a "work", a "Default" beside the
/// built-in account — is refused with 409, naming that account, and nothing of it is
/// written: not its row, not its token, not its profile directory.
///
/// A hosted router's `POST /api/accounts` (`hosted::accounts_add`) reads the body first and
/// hands it on, so a body that cannot be read is refused in the same words on either.
pub async fn accounts_add(
    State(state): State<Arc<AppState>>,
    JsonBody(req): JsonBody<AddAccount>,
) -> Response {
    let source = match Source::parse(&req.source) {
        Ok(source) => source,
        Err(e) => return refuse(StatusCode::BAD_REQUEST, e.to_string()),
    };
    let token = req.token.as_deref().map(str::trim).filter(|t| !t.is_empty()).map(str::to_owned);
    // 400, both: whatever the store holds, neither request can be met.
    let token = match (source, token) {
        (Source::Keychain, _) => {
            return refuse(
                StatusCode::BAD_REQUEST,
                "the default Keychain account is discovered automatically and cannot be added",
            )
        }
        (Source::Token, None) => {
            return refuse(StatusCode::BAD_REQUEST, "a token is required for this account type")
        }
        (Source::Token, token) => token,
        (Source::Profile, _) => None,
    };
    let provider = match req.provider.as_deref().map(str::trim).filter(|p| !p.is_empty()) {
        None => Provider::Anthropic,
        Some(p) => match Provider::parse(p) {
            Ok(provider) => provider,
            Err(e) => return refuse(StatusCode::BAD_REQUEST, e.to_string()),
        },
    };
    // A Claude Code login and a profile are Anthropic's; every other provider's account is
    // a key pasted in, kept as a setup-token is.
    if provider != Provider::Anthropic && source != Source::Token {
        return refuse(
            StatusCode::BAD_REQUEST,
            format!("a {} account is added from its key, as a token account", provider.name()),
        );
    }
    if let Some(why) = token.as_deref().and_then(|t| provider.key_problem(t)) {
        return refuse(StatusCode::BAD_REQUEST, why);
    }
    let base_url = match base_url_for(provider, req.base_url.as_deref()) {
        Ok(url) => url,
        Err(why) => return refuse(StatusCode::BAD_REQUEST, why),
    };
    let id = crate::accounts::slugify(&req.label);
    let store = Store::new(state.config.db_path.clone());

    let done = tokio::task::spawn_blocking(move || -> anyhow::Result<Value> {
        match token {
            Some(token) => {
                // A long-lived token cannot tell us who it belongs to — the usage
                // endpoint refuses it — so if the name given is an email address,
                // take it as the identity. It is the only signal available.
                let status = crate::accounts::AuthStatus {
                    email: looks_like_email(&req.label).then(|| req.label.trim().to_string()),
                    ..Default::default()
                };
                // The token is stored by `add`, and only once the id is known to be free.
                store.add(&id, &req.label, source, provider, base_url.as_deref(), None, &status, || {
                    crate::accounts::store_token(&id, &token)
                })?;
                Ok(json!({ "id": id, "ready": true }))
            }
            None => {
                // Before the directory: for a name already taken, it is that account's.
                store.check_free(&id)?;
                // The router creates and owns the profile directory; the login itself
                // is a terminal program, so the UI hands back the command to run.
                let dir = crate::accounts::profiles_root()?.join(&id);
                std::fs::create_dir_all(&dir)?;
                let status = crate::accounts::probe_profile(Some(&dir)).unwrap_or_default();
                store.add(&id, &req.label, source, provider, None, Some(&dir.to_string_lossy()), &status, || Ok(()))?;
                Ok(json!({
                    "id": id,
                    "ready": status.logged_in,
                    "profile_dir": dir.to_string_lossy(),
                    "command": format!("CLAUDE_CONFIG_DIR={} claude auth login", dir.display()),
                }))
            }
        }
    })
    .await;
    match done {
        Ok(Ok(added)) => Json(added).into_response(),
        Ok(Err(e)) => not_made(e),
        Err(e) => failed(e),
    }
}

/// Re-check a profile account's login state, and pick up its email and org once the
/// terminal login completes. The UI polls this so the flow finishes on its own.
///
/// An id that names no profile account is refused with 409. It answered a 200 once, with
/// `logged_in: false`, which a poller could not tell from a login not finished yet.
pub async fn accounts_probe(
    State(state): State<Arc<AppState>>,
    JsonBody(req): JsonBody<AccountRef>,
) -> Response {
    let store = Store::new(state.config.db_path.clone());
    let id = req.id.clone();
    let done = tokio::task::spawn_blocking(move || -> Result<Value, Unmade> {
        let Some(account) = store.get(&id).map_err(failure)? else {
            return Err((StatusCode::CONFLICT, Conflict::NoAccount(id).to_string()));
        };
        // `claude auth status` is Claude Code's, and says who is logged in to Anthropic.
        if account.provider != Provider::Anthropic {
            return Err((
                StatusCode::CONFLICT,
                format!(
                    "{id:?} is a {} account, so it has no Claude Code login to check",
                    account.provider.name()
                ),
            ));
        }
        let Some(dir) = account.profile_dir else {
            return Err((
                StatusCode::CONFLICT,
                format!("{id:?} is not a profile account, so it has no login of its own to check"),
            ));
        };
        // 500 when `claude` cannot be asked: the router's machine, not the request.
        let status = crate::accounts::probe_profile(Some(std::path::Path::new(&dir))).map_err(failure)?;
        if status.logged_in {
            let label = status.email.clone().unwrap_or_else(|| id.clone());
            store.record_login(&id, &dir, &label, &status).map_err(failure)?;
        }
        serde_json::to_value(status).map_err(failure)
    })
    .await;
    match done {
        Ok(Ok(status)) => Json(status).into_response(),
        Ok(Err((status, why))) => refuse(status, why),
        Err(e) => failed(e),
    }
}

pub async fn accounts_activate(
    State(state): State<Arc<AppState>>,
    JsonBody(req): JsonBody<AccountRef>,
) -> Response {
    let store = Store::new(state.config.db_path.clone());
    let id = req.id.clone();
    let server = state.config.server;
    let done = tokio::task::spawn_blocking(move || -> Result<(), Unmade> {
        // A hosted router can only sign with a pasted token. Switching to a Claude Code
        // login that came with a copied store would leave every request with nothing to
        // sign with. 409: that account is what stands in the way.
        if server {
            match store.get(&id).map_err(failure)? {
                Some(account) if account.source != Source::Token => {
                    return Err((StatusCode::CONFLICT, crate::accounts::NOT_ON_A_SERVER.to_string()));
                }
                _ => {}
            }
        }
        store.activate(&id).map_err(why_not)
    })
    .await;
    match done {
        Ok(Ok(())) => {
            // Drop the cached credential: the switch takes effect on the very next request,
            // mid-session, with nothing restarted.
            state.invalidate().await;
            tracing::info!(account = %req.id, "active account switched");
            Json(json!({ "ok": true, "active": req.id })).into_response()
        }
        Ok(Err((status, why))) => refuse(status, why),
        Err(e) => failed(e),
    }
}

pub async fn accounts_remove(
    State(state): State<Arc<AppState>>,
    JsonBody(req): JsonBody<AccountRef>,
) -> Response {
    // On a hosted router there is no discovery to bring it back, and a `default` row
    // can only have come with a store copied from a machine: it can never sign there.
    // 400 on a machine: whatever the store holds, the next start would put it back.
    if req.id == crate::proxy::ACCOUNT_ID && !state.config.server {
        return refuse(
            StatusCode::BAD_REQUEST,
            "the default account is discovered automatically and cannot be removed",
        );
    }
    let store = Store::new(state.config.db_path.clone());
    let id = req.id.clone();
    // An id no account has is no refusal: what was asked for, no such account, holds.
    match tokio::task::spawn_blocking(move || store.remove(&id)).await {
        Ok(Ok(())) => {
            state.invalidate().await;
            Json(json!({ "ok": true })).into_response()
        }
        Ok(Err(e)) => failed(e),
        Err(e) => failed(e),
    }
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
            until: None,
            tz: None,
        }
    }

    /// How many requests the request list shows in a scope.
    fn requests_of_scope(conn: &Connection, f: &Filter) -> usize {
        requests_of(conn, f).unwrap().as_array().unwrap().len()
    }

    /// One session and its requests, one a second apart from `at`.
    fn seed(store: &Store, id: &str, cwd: &str, at: i64, requests: i64) {
        store
            .conn
            .execute(
                "INSERT INTO sessions (id, cwd, started_at, last_seen_at) VALUES (?1, ?2, ?3, ?3)",
                params![id, cwd, at],
            )
            .unwrap();
        for n in 0..requests {
            store
                .conn
                .execute(
                    "INSERT INTO requests (request_id, session_id, account_id, method, path, started_at)
                     VALUES (?1, ?2, 'work', 'POST', '/v1/messages', ?3)",
                    params![format!("{id}-{n}"), id, at + n * 1_000],
                )
                .unwrap();
        }
    }

    /// A clone on a Mac and a worktree on a Linux host are one project, its key the
    /// repository: one row of the project list, carrying each directory with the machine that
    /// listed it and its remotes, and the key every scoped read takes. A session no machine
    /// has listed keeps its directory, and a directory with no remote is its own project.
    #[test]
    fn one_repository_on_two_machines_is_one_project() {
        use crate::places::{record, Directory, Listed, Report};
        use crate::repository::Remote;
        const T0: i64 = 1_700_000_000_000;
        let store = store("repository");
        seed(&store, "mac-1", "/Volumes/x/ui-kit-web", T0, 3);
        seed(&store, "lin-1", "/home/u/ui-kit-web-lane-2", T0 + 10_000, 2);
        seed(&store, "lin-2", "/home/u/scratch", T0 + 20_000, 1);
        seed(&store, "old", "/Volumes/x/ui-kit-web", T0 - 100_000, 1);
        let origin = |url: &str| vec![Remote { name: "origin".into(), url: url.into() }];
        let report = |machine: &str, label: &str, listed: &[(&str, &str)], dirs: Vec<Directory>| Report {
            machine: machine.into(),
            label: Some(label.into()),
            platform: None,
            sessions: listed.iter().map(|(s, c)| Listed { session: (*s).into(), cwd: Some((*c).into()) }).collect(),
            directories: dirs,
            clients: None,
        };
        record(
            &store.conn,
            &report("mac", "Sample-Mac-mini", &[("mac-1", "/Volumes/x/ui-kit-web")], vec![Directory {
                cwd: "/Volumes/x/ui-kit-web".into(),
                remotes: origin("https://gitlab.example.com/acme/ui-kit-web.git"),
            }]),
            T0,
        )
        .unwrap();
        record(
            &store.conn,
            &report(
                "lin",
                "wh-vdesk03",
                &[("lin-1", "/home/u/ui-kit-web-lane-2"), ("lin-2", "/home/u/scratch")],
                vec![
                    Directory { cwd: "/home/u/ui-kit-web-lane-2".into(), remotes: origin("git@gitlab.example.com:acme/ui-kit-web.git") },
                    Directory { cwd: "/home/u/scratch".into(), remotes: vec![] },
                ],
            ),
            T0,
        )
        .unwrap();

        let conn = store.reads.get().unwrap();
        let everything = scope(None, None, None, None);
        let projects = projects_of(&conn, &everything).unwrap();
        let project = |key: &str| projects.as_array().unwrap().iter().find(|p| p["project"] == key).cloned().unwrap();
        let repo = "gitlab.example.com/acme/ui-kit-web";
        let one = project(repo);
        assert_eq!(one["repository"], repo);
        assert_eq!(one["sessions"], 2, "the clone and the worktree");
        assert_eq!(one["requests"], 5);
        let places: Vec<(String, String, String)> = one["directories"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| {
                (d["machine"]["label"].as_str().unwrap().into(), d["cwd"].as_str().unwrap().into(), d["remotes"][0]["repository"].as_str().unwrap().into())
            })
            .collect();
        assert_eq!(
            places,
            [
                ("Sample-Mac-mini".to_string(), "/Volumes/x/ui-kit-web".to_string(), repo.to_string()),
                ("wh-vdesk03".to_string(), "/home/u/ui-kit-web-lane-2".to_string(), repo.to_string()),
            ]
        );
        assert_eq!(project("/home/u/scratch")["repository"], Value::Null, "no remote: the directory is the project");
        let unplaced = project("/Volumes/x/ui-kit-web");
        assert_eq!(unplaced["sessions"], 1, "a session no machine listed keeps its directory");
        assert_eq!(unplaced["directories"][0]["machine"], Value::Null);

        let mine = scope(Some(repo), None, None, None);
        assert_eq!(summary_of(&conn, &mine).unwrap()["totals"]["requests"], 5);
        assert_eq!(summary_of(&conn, &everything).unwrap()["totals"]["projects"], 3);
        let listed = sessions_of(&conn, &mine).unwrap();
        let mut names: Vec<(String, String, String)> = listed
            .as_array()
            .unwrap()
            .iter()
            .map(|s| (s["id"].as_str().unwrap().into(), s["cwd"].as_str().unwrap().into(), s["machine"]["label"].as_str().unwrap().into()))
            .collect();
        names.sort();
        assert_eq!(
            names,
            [
                ("lin-1".to_string(), "/home/u/ui-kit-web-lane-2".to_string(), "wh-vdesk03".to_string()),
                ("mac-1".to_string(), "/Volumes/x/ui-kit-web".to_string(), "Sample-Mac-mini".to_string()),
            ]
        );
        assert!(listed.as_array().unwrap().iter().all(|s| s["project"] == repo && s["repository"] == repo));
        let requests = requests_of_scope(&conn, &mine);
        assert_eq!(requests, 5);
        assert!(tools_of(&conn, &mine).is_ok());
    }

    /// `until` bounds what `since` begins: the summary and every read that takes the scope.
    #[test]
    fn until_bounds_every_scoped_read() {
        const T0: i64 = 1_700_000_000_000;
        let store = store("until");
        seed(&store, "early", "/code/a", T0, 2);
        seed(&store, "late", "/code/a", T0 + 100_000, 3);
        let conn = store.reads.get().unwrap();
        let mut window = scope(None, None, None, Some(T0));
        window.until = Some(T0 + 100_000);
        let summary = summary_of(&conn, &window).unwrap();
        assert_eq!(summary["totals"]["requests"], 2, "the late session's requests are past the bound");
        assert_eq!(summary["timeline"]["end"], T0 + 100_000, "and the timeline ends there");
        let listed = sessions_of(&conn, &window).unwrap();
        assert_eq!(listed.as_array().unwrap().iter().map(|s| s["id"].as_str().unwrap()).collect::<Vec<_>>(), ["early"]);
        assert_eq!(projects_of(&conn, &window).unwrap()[0]["requests"], 2);
        assert_eq!(requests_of_scope(&conn, &window), 2);
        assert!(tools_of(&conn, &window).is_ok());
        window.until = None;
        assert_eq!(summary_of(&conn, &window).unwrap()["totals"]["requests"], 5);
    }

    /// The console asks the summary only for what its open tab shows: the parts named come
    /// back and nothing else, each as the whole answer gives it, and a part not named is not
    /// read at all — here its table is gone, and only the whole answer fails for it. Two
    /// lists of parts are two memo slots; one list spelt two ways is one.
    #[test]
    fn the_summary_answers_only_the_parts_asked_for() {
        let store = store("parts");
        seed(&store, "one", "/code/a", 1_700_000_000_000, 3);
        let conn = store.reads.get().unwrap();
        let everything = scope(None, None, None, None);
        let whole = summary_of(&conn, &everything).unwrap();
        let asked = summary_parts(Some("timeline, totals,timeline")).ok().unwrap();
        assert_eq!(asked, ["totals", "timeline"], "in the answer's order, each once");
        let part = summary_parts_of(&conn, &everything, &asked).unwrap();
        let keys: Vec<&str> = part.as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(keys, ["totals", "unattributed", "timeline"]);
        for key in keys {
            assert_eq!(part[key], whole[key], "{key}");
        }
        let history = summary_parts_of(&conn, &everything, &["quota_history"]).unwrap();
        assert_eq!(history.as_object().unwrap().keys().collect::<Vec<_>>(), ["quota_history"]);

        store.conn.execute_batch("DROP TABLE context").unwrap();
        assert!(summary_of(&conn, &everything).is_err(), "the whole answer reads the context");
        assert!(summary_parts_of(&conn, &everything, &asked).is_ok(), "this one never did");

        let key = |parts: Option<&str>| summary_key(&everything, &summary_parts(parts).ok().unwrap());
        assert_ne!(key(Some("totals")), key(Some("totals,models")));
        assert_ne!(key(Some("totals")), key(None));
        assert_eq!(key(Some("models,totals")), key(Some("totals, models,models")));
    }

    /// Every row the Overview reads that names an account names its provider too, a model its
    /// own, and a session every provider its requests went to (AGEN-0126). A request recorded
    /// before providers is Anthropic's.
    #[test]
    fn the_overviews_rows_name_their_provider() {
        let store = store("providers");
        store
            .conn
            .execute_batch(
                "INSERT INTO accounts (id, label, active, created_at, provider) VALUES ('work', 'work', 1, 1, 'anthropic');
                 INSERT INTO accounts (id, label, active, created_at, provider) VALUES ('kimi', 'kimi', 1, 2, 'kimi');
                 INSERT INTO sessions (id, cwd, started_at, last_seen_at) VALUES ('s', '/code/a', 1, 1);
                 INSERT INTO requests (request_id, session_id, account_id, method, path, started_at, model, provider)
                   VALUES ('a', 's', 'work', 'POST', '/v1/messages', 1700000000000, 'claude-opus-5-5', 'anthropic');
                 INSERT INTO requests (request_id, session_id, account_id, method, path, started_at, model, provider)
                   VALUES ('k', 's', 'kimi', 'POST', '/v1/messages', 1700000001000, 'k3', 'kimi');
                 INSERT INTO requests (request_id, session_id, account_id, method, path, started_at, model)
                   VALUES ('old', 's', 'gone', 'POST', '/v1/messages', 1700000002000, 'claude-sonnet-5-5');",
            )
            .unwrap();
        let conn = store.reads.get().unwrap();
        let all = scope(None, None, None, None);
        let part = summary_parts_of(&conn, &all, &["by_account", "models"]).unwrap();
        let named = |rows: &Value, key: &str| -> Vec<(String, String)> {
            let mut out: Vec<(String, String)> = rows
                .as_array()
                .unwrap()
                .iter()
                .map(|r| (r[key].as_str().unwrap().to_string(), r["provider"].as_str().unwrap().to_string()))
                .collect();
            out.sort();
            out
        };
        let pair = |a: &str, b: &str| (a.to_string(), b.to_string());
        assert_eq!(named(&part["by_account"], "id"), [pair("gone", "anthropic"), pair("kimi", "kimi"), pair("work", "anthropic")]);
        assert_eq!(
            named(&part["models"], "model"),
            [pair("claude-opus-5-5", "anthropic"), pair("claude-sonnet-5-5", "anthropic"), pair("k3", "kimi")]
        );
        assert_eq!(named(&requests_of(&conn, &all).unwrap(), "request_id").len(), 3);
        assert_eq!(named(&requests_of(&conn, &all).unwrap(), "request_id")[1], pair("k", "kimi"));
        let sessions = sessions_of(&conn, &all).unwrap();
        assert_eq!(sessions[0]["providers"], json!(["anthropic", "kimi"]), "Anthropic's first");
    }

    /// The phones ask with no `parts=` and decode every key as they always have; the iPhone
    /// fails on any one of nine missing. Without parts the summary is whole.
    #[test]
    fn without_parts_the_summary_answers_whole() {
        let store = store("whole");
        seed(&store, "one", "/code/a", 1_700_000_000_000, 2);
        let conn = store.reads.get().unwrap();
        // Parsed the way the handler's two `Query`s parse what a phone sends.
        let uri: axum::http::Uri = "/api/summary?since=0&tz=-240".parse().unwrap();
        let asked = Query::<SummaryQuery>::try_from_uri(&uri).unwrap().0;
        let f = Query::<Filter>::try_from_uri(&uri).unwrap().0;
        let parts = summary_parts(asked.parts.as_deref()).ok().unwrap();
        assert_eq!(parts, SUMMARY_PARTS);
        let whole = summary_parts_of(&conn, &f, &parts).unwrap();
        let keys: Vec<&str> = whole.as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(
            keys,
            [
                "totals", "unattributed", "quota", "account", "context", "models", "threads",
                "latency", "stop_reasons", "by_account", "timeline", "quota_history",
            ]
        );
        assert_eq!(whole["totals"]["requests"], 2);
    }

    /// A part the summary does not have is refused in words that name it, and so is a
    /// `parts=` that names none: answering less than was asked, and silently, would leave a
    /// panel empty with nothing to say why.
    #[tokio::test]
    async fn an_unknown_summary_part_is_refused_by_name() {
        for (asked, named) in [("totals,sessions", "\"sessions\""), (" , ", "names no part")] {
            let refused = summary_parts(Some(asked)).err().unwrap();
            assert_eq!(refused.status(), StatusCode::BAD_REQUEST, "{asked}");
            let body = axum::body::to_bytes(refused.into_body(), usize::MAX).await.unwrap();
            let body: Value = serde_json::from_slice(&body).unwrap();
            let why = body["error"].as_str().unwrap();
            assert!(why.contains(named), "{asked}: {why}");
            assert!(why.contains("quota_history"), "and it lists the parts there are: {why}");
            assert_eq!(body["ok"], false);
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

    /// In the `(unresolved)` scope, which the console's "(unresolved)" project row asks for
    /// and so does the iPhone's "—" row, the summary counted the sessions with no recorded
    /// directory and their requests while the Sessions and tool Results lists came up empty:
    /// both wrote their own project test, without the branch that selects those sessions.
    /// They take the one every read shares now, so each list holds what the summary counts,
    /// there and in a real project alike.
    #[test]
    fn unresolved_scope_lists_the_sessions_and_tool_calls_the_summary_counts() {
        use std::collections::BTreeMap;

        const T0: i64 = 1_700_000_000_000;
        const ALPHA: &str = "/Users/sample/code/alpha";
        // A session's first request calls its tools and the next carries their results back:
        // the calls are read from the requests, the tool Results list from the results.
        const SESSIONS: [(&str, Option<&str>, &str, i64, &[&str]); 5] = [
            ("alpha-1", Some(ALPHA), "work", T0, &["Read"]),
            ("alpha-2", Some(ALPHA), "work", T0 + 100_000, &["Read", "Edit"]),
            ("bare-1", None, "work", T0 + 200_000, &["Bash"]),
            ("bare-2", None, "work", T0 + 300_000, &["Bash", "Read"]),
            ("bare-3", None, "side", T0 + 400_000, &["Grep"]),
        ];
        fn request(conn: &Connection, id: &str, session: Option<&str>, account: &str, at: i64) -> i64 {
            conn.execute(
                "INSERT INTO requests (request_id, session_id, account_id, method, path, started_at)
                 VALUES (?1, ?2, ?3, 'POST', '/v1/messages', ?4)",
                params![id, session, account, at],
            )
            .unwrap();
            conn.last_insert_rowid()
        }

        let store = store("unresolved-lists");
        let tx = store.conn.unchecked_transaction().unwrap();
        for (id, cwd, account, at, tools) in SESSIONS {
            tx.execute(
                "INSERT INTO sessions (id, cwd, started_at, last_seen_at) VALUES (?1, ?2, ?3, ?4)",
                params![id, cwd, at, at + 2_000],
            )
            .unwrap();
            let call = request(&tx, &format!("{id}-call"), Some(id), account, at);
            let answer = request(&tx, &format!("{id}-answer"), Some(id), account, at + 1_000);
            for (n, name) in tools.iter().enumerate() {
                tx.execute(
                    "INSERT INTO tool_invocations (request_id, ordinal, name) VALUES (?1, ?2, ?3)",
                    params![call, n as i64, name],
                )
                .unwrap();
                tx.execute(
                    "INSERT INTO tool_calls (tool_use_id, session_id, first_request_id, name, seen_at)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![format!("{id}-use-{n}"), id, answer, name, at + 1_000],
                )
                .unwrap();
            }
        }
        // And a request no session holds, which `(unresolved)` takes as it does everywhere:
        // counted in the totals, and shown as the row the lists end with.
        request(&tx, "orphan", None, "work", T0 + 500_000);
        tx.commit().unwrap();

        let conn = store.reads.get().unwrap();
        // Parsed the way the handlers' `Query` parses what a client sends.
        let scope = |query: &str| {
            let uri: axum::http::Uri = format!("/api/sessions?{query}").parse().unwrap();
            Query::<Filter>::try_from_uri(&uri).unwrap().0
        };
        let ids = |list: &Value| -> Vec<String> {
            list.as_array().unwrap().iter().map(|s| s["id"].as_str().unwrap().to_owned()).collect()
        };
        let tally = |list: &Value, count: &str| -> BTreeMap<String, i64> {
            let tools = list.as_array().unwrap().iter();
            tools.map(|t| (t["name"].as_str().unwrap().to_owned(), t[count].as_i64().unwrap())).collect()
        };

        let after_bare_1 = format!("project=%28unresolved%29&since={}", T0 + 250_000);
        let cases: [(&str, &[&str]); 6] = [
            // The console's row, as its URLSearchParams spells it, and the iPhone's.
            ("project=%28unresolved%29", &["bare-3", "bare-2", "bare-1"]),
            ("project=(unresolved)", &["bare-3", "bare-2", "bare-1"]),
            // Narrowed further as any project is: by account, and by range.
            ("project=%28unresolved%29&account=side", &["bare-3"]),
            (&after_bare_1, &["bare-3", "bare-2"]),
            // A real project's scope is what it was: its own sessions, none without a directory.
            ("project=%2FUsers%2Fsample%2Fcode%2Falpha", &["alpha-2", "alpha-1"]),
            // And no project at all is every session.
            ("", &["bare-3", "bare-2", "bare-1", "alpha-2", "alpha-1"]),
        ];
        for (query, listed) in cases {
            let f = scope(query);
            let summary = summary_of(&conn, &f).unwrap();
            let sessions = sessions_of(&conn, &f).unwrap();
            assert_eq!(ids(&sessions), listed, "{query}: the Sessions list");

            // Every session the summary counts is in the list, and so is every request it
            // counts but those no session holds, which it counts apart for the closing row.
            let totals = &summary["totals"];
            let sessions = sessions.as_array().unwrap();
            assert_eq!(sessions.len() as i64, totals["sessions"].as_i64().unwrap(), "{query}");
            let in_list: i64 = sessions.iter().map(|s| s["requests"].as_i64().unwrap()).sum();
            let apart = summary["unattributed"].as_i64().unwrap();
            assert_eq!(in_list + apart, totals["requests"].as_i64().unwrap(), "{query}");

            // The tool lists count those sessions' calls, and the results that came back.
            let mut theirs = BTreeMap::new();
            for (.., tools) in SESSIONS.into_iter().filter(|s| listed.contains(&s.0)) {
                for name in tools {
                    *theirs.entry(name.to_string()).or_insert(0) += 1;
                }
            }
            let tools = tools_of(&conn, &f).unwrap();
            assert_eq!(tally(&tools["called"], "calls"), theirs, "{query}: the calls made");
            assert_eq!(tally(&tools["produced"], "results"), theirs, "{query}: the tool Results list");
        }

        // The Sessions list is never narrowed by session, since it is how one is picked; the
        // rest of the page is.
        let picked = scope("project=%28unresolved%29&session=bare-2");
        assert_eq!(ids(&sessions_of(&conn, &picked).unwrap()), ["bare-3", "bare-2", "bare-1"]);
        assert_eq!(summary_of(&conn, &picked).unwrap()["totals"]["requests"], 2);
        let tools = tools_of(&conn, &picked).unwrap();
        let bare_2 = BTreeMap::from([("Bash".to_string(), 1), ("Read".to_string(), 1)]);
        assert_eq!(tally(&tools["called"], "calls"), bare_2);
        assert_eq!(tally(&tools["produced"], "results"), bare_2);
    }

    /// Search is asked with the page's project too (the console's search panel sends it), and
    /// it wrote its own project test as well, so in the `(unresolved)` scope it found nothing
    /// at all. It finds what was said in the sessions with no recorded directory now: the
    /// hits the console labels "(unresolved)".
    #[test]
    fn unresolved_scope_search_finds_what_the_sessions_with_no_directory_said() {
        let store = store("unresolved-search");
        let side = crate::db::bodies_path(&store.dir.join("agent-router.db"));
        store.conn.execute("ATTACH DATABASE ?1 AS bodies_db", [side.to_str().unwrap()]).unwrap();
        // The same words, said in a project, in a session with no directory, and in one the
        // store holds no row for, which `(unresolved)` takes as the summary takes a request
        // of one.
        store
            .conn
            .execute_batch(
                "INSERT INTO sessions (id, cwd, started_at, last_seen_at)
                   VALUES ('alpha-1', '/Users/sample/code/alpha', 1, 2), ('bare-1', NULL, 3, 4);
                 INSERT INTO bodies_db.blocks (id, hash, kind, chars, text)
                   VALUES (1, x'01', 'text', 27, 'the heron waits at the weir');
                 INSERT INTO bodies_db.block_sites (block_id, session_id, ordinal, role, request_id, at)
                   VALUES (1, 'alpha-1', 0, 'user', 1, 10), (1, 'bare-1', 0, 'user', 2, 20),
                          (1, 'ghost-1', 0, 'user', 3, 30);",
            )
            .unwrap();

        let conn = store.reads.get().unwrap();
        // The sessions in each of the two lists a search answers with: the hits, and the
        // hits rolled up per session.
        let found = |scope: &str| -> [Vec<String>; 2] {
            let uri: axum::http::Uri = format!("/api/search?q=heron{scope}").parse().unwrap();
            let f = Query::<SearchQuery>::try_from_uri(&uri).unwrap().0;
            let sessions = |list: Value| -> Vec<String> {
                let mut ids: Vec<String> = list
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|h| h["session_id"].as_str().unwrap().to_owned())
                    .collect();
                ids.sort();
                ids
            };
            [
                sessions(hits_for(&conn, &f.q, &f, 60).unwrap()),
                sessions(sessions_for(&conn, &f.q, &f).unwrap()),
            ]
        };
        assert_eq!(found("&project=%28unresolved%29"), [["bare-1", "ghost-1"], ["bare-1", "ghost-1"]]);
        assert_eq!(found("&project=%2FUsers%2Fsample%2Fcode%2Falpha"), [["alpha-1"], ["alpha-1"]]);
        let everywhere = ["alpha-1", "bare-1", "ghost-1"];
        assert_eq!(found(""), [everywhere, everywhere]);
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

    #[tokio::test]
    async fn the_accounts_list_names_each_provider() {
        let store = store("accounts-providers");
        store
            .conn
            .execute_batch(
                "INSERT INTO accounts (id, label, source, active, created_at)
                 VALUES ('default', 'me@example.com', 'keychain', 1, 1);
                 INSERT INTO accounts (id, label, source, provider, base_url, active, created_at)
                 VALUES ('kimi', 'Kimi', 'token', 'kimi', 'https://api.kimi.ai/coding', 1, 2);",
            )
            .unwrap();
        let unchecked = |_: &Account| -> Credential { unreachable!("nothing asked for a check") };
        let list = accounts_of(&store.reads, &AccountsQuery { probe: None }, unchecked).await.unwrap();
        let list = list.as_array().unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!((&list[0]["provider"], &list[0]["base_url"]), (&json!("anthropic"), &Value::Null));
        assert_eq!(
            (&list[1]["provider"], &list[1]["base_url"]),
            (&json!("kimi"), &json!("https://api.kimi.ai/coding"))
        );
        assert_eq!((&list[0]["active"], &list[1]["active"]), (&json!(1), &json!(1)), "one active account per provider");
    }

    /// Not a credential: no Kimi Code key looks like this.
    const KIMI_KEY: &str = "sk-kimi-test-0123456789abcdefghijklmnopqrstuvwxyz";

    /// Add `label` as a Kimi Code account through the API, and read it back from the store.
    async fn add_kimi(lab: &crate::api_refusals::Lab, label: &str, base_url: Option<&str>) -> Account {
        let mut body = json!({ "label": label, "source": "token", "provider": "kimi", "token": KIMI_KEY });
        if let Some(url) = base_url {
            body["base_url"] = json!(url);
        }
        let (status, added) = lab.post("/api/accounts", &body.to_string()).await;
        assert_eq!(status, StatusCode::OK, "{added}");
        let id = added["id"].as_str().unwrap().to_string();
        let account = crate::accounts::Store::new(lab.state.config.db_path.clone()).get(&id).unwrap().unwrap();
        assert_eq!(account.provider, Provider::Kimi);
        assert_eq!(account.source, Source::Token, "kept as a setup-token is");
        assert_eq!(crate::accounts::resolve(&account).unwrap().token, KIMI_KEY, "it signs with the key it was given");
        account
    }

    #[tokio::test]
    async fn a_kimi_account_is_added_from_its_key_locally() {
        let lab = crate::api_refusals::Lab::local("kimi-local");
        let kimi = add_kimi(&lab, "Kimi local", None).await;
        assert_eq!(kimi.base_url.as_deref(), Some("https://api.kimi.ai/coding"), "its provider's default, stored");
        let store = crate::accounts::Store::new(lab.state.config.db_path.clone());
        assert_eq!(store.active_for(Provider::Kimi).unwrap().id, "kimi-local", "the first of its provider");

        // Kimi Code's China host, given with the slash its guide puts on the end.
        let china = add_kimi(&lab, "Kimi local China", Some("https://api.kimi.com/coding/")).await;
        assert_eq!(china.base_url.as_deref(), Some("https://api.kimi.com/coding"));
        assert_eq!(store.active_for(Provider::Kimi).unwrap().id, "kimi-local", "a second one waits its turn");
    }

    #[tokio::test]
    async fn a_kimi_account_is_added_from_its_key_on_a_hosted_router() {
        let lab = crate::api_refusals::Lab::hosted("kimi-hosted").await;
        add_kimi(&lab, "Kimi hosted", None).await;
        let store = crate::accounts::Store::new(lab.state.config.db_path.clone());
        assert_eq!(store.active_for(Provider::Kimi).unwrap().id, "kimi-hosted");
        assert!(store.active().is_err(), "a Kimi Code key never becomes what Claude's requests are signed with");
        let (status, body) = lab.post("/api/accounts/activate", r#"{"id":"kimi-hosted"}"#).await;
        assert_eq!(status, StatusCode::OK, "a hosted router signs with pasted keys, and this is one: {body}");
    }

    #[tokio::test]
    async fn a_key_is_checked_against_its_providers_format() {
        use crate::api_refusals::{words, Lab};
        let lab = Lab::local("key-format");
        let kimi_wants = "that is an Anthropic token — a Kimi Code account takes a key from the Kimi Code console";
        for (body, said) in [
            (json!({ "label": "kimi-format", "source": "token", "provider": "kimi", "token": "sk-ant-oat01-abc" }), kimi_wants),
            (
                json!({ "label": "claude-format", "source": "token", "token": KIMI_KEY }),
                "expected a token starting sk-ant- (from `claude setup-token`)",
            ),
            (
                json!({ "label": "claude-format", "source": "token", "provider": "anthropic", "token": "sk-ant-sid01-abc" }),
                "that is a claude.ai session key — it belongs in the session-key field, and cannot sign API requests",
            ),
            (
                json!({ "label": "kimi-format", "source": "profile", "provider": "kimi" }),
                "a Kimi Code account is added from its key, as a token account",
            ),
            (json!({ "label": "kimi-format", "source": "token", "provider": "openai", "token": KIMI_KEY }), r#"unknown provider "openai""#),
            (
                json!({ "label": "kimi-format", "source": "token", "provider": "kimi", "token": KIMI_KEY, "base_url": "http://api.kimi.ai/coding" }),
                r#""http://api.kimi.ai/coding" is not an https:// address"#,
            ),
            (
                json!({ "label": "claude-format", "source": "token", "token": "sk-ant-oat01-abc", "base_url": "https://example.com" }),
                "an Anthropic account goes to the router's own upstream, and takes no base URL",
            ),
        ] {
            assert_eq!(words(lab.post("/api/accounts", &body.to_string()).await, StatusCode::BAD_REQUEST), said, "{body}");
        }
        let store = crate::accounts::Store::new(lab.state.config.db_path.clone());
        for id in ["kimi-format", "claude-format"] {
            assert!(store.get(id).unwrap().is_none(), "{id}: nothing refused was stored");
        }

        // A new credential for an account is held to that account's provider's format.
        let kimi = add_kimi(&lab, "Kimi rotate", None).await;
        let rotate = |token: &str| json!({ "id": kimi.id, "token": token }).to_string();
        assert_eq!(words(lab.post("/api/accounts/edit", &rotate("sk-ant-oat01-abc")).await, StatusCode::BAD_REQUEST), kimi_wants);
        assert_eq!(crate::accounts::resolve(&kimi).unwrap().token, KIMI_KEY, "the refused one was not stored");
        assert_eq!(lab.post("/api/accounts/edit", &rotate("sk-kimi-rotated")).await.0, StatusCode::OK);
        assert_eq!(crate::accounts::resolve(&kimi).unwrap().token, "sk-kimi-rotated");
    }

    #[tokio::test]
    async fn the_profile_probe_skips_a_kimi_account() {
        use crate::api_refusals::{words, Lab};
        let lab = Lab::local("probe-kimi");
        let kimi = add_kimi(&lab, "Kimi probe", None).await;
        let refused = lab.post("/api/accounts/probe", &json!({ "id": kimi.id }).to_string()).await;
        assert_eq!(
            words(refused, StatusCode::CONFLICT),
            r#""kimi-probe" is a Kimi Code account, so it has no Claude Code login to check"#
        );
    }

    #[tokio::test]
    async fn the_forecast_gives_a_kimi_accounts_windows() {
        let lab = crate::api_refusals::Lab::local("forecast-kimi");
        let add = json!({ "label": "Forecast kimi", "source": "token", "provider": "kimi", "token": "sk-kimi-not-a-credential" });
        assert_eq!(lab.post("/api/accounts", &add.to_string()).await.0, StatusCode::OK);
        // Two readings of Kimi Code's usage API, as the reader keeps them, an hour apart.
        let now = crate::db::now_ms();
        let (five_reset, month_reset) = (now / 1000 + 3 * 3600, now / 1000 + 20 * 86_400);
        let conn = Connection::open(&lab.state.config.db_path).unwrap();
        // The rate table and its cost view, as the router sets them up at start.
        crate::pricing::ensure_table(&conn).unwrap();
        for (at, five, month) in [(now - 3_600_000, 0.1, 0.30), (now - 60_000, 0.4, 0.35)] {
            conn.execute(
                "INSERT INTO quota_readings (account_id, at, five_hour_used, five_hour_reset, second_window, second_used, second_reset)
                 VALUES ('forecast-kimi', ?1, ?2, ?3, 'month', ?4, ?5)",
                params![at, five, five_reset, month, month_reset],
            )
            .unwrap();
        }
        let everything = Filter { project: None, session: None, account: None, since: None, until: None, tz: None };
        let got = forecast_inner(&lab.state, &everything).unwrap();
        let card = got["accounts"].as_array().unwrap().iter().find(|a| a["id"] == "forecast-kimi").unwrap().clone();
        assert_eq!(card["provider"], "kimi");
        assert_eq!(card["seen_at"], json!(now - 60_000), "seen when it was last read");
        assert_eq!((&card["five_hour"]["used"], &card["five_hour"]["reset_at"]), (&json!(0.4), &json!(five_reset)));
        assert_eq!(card["five_hour"]["expired"], json!(false));
        assert_eq!((&card["month"]["used"], &card["month"]["reset_at"]), (&json!(0.35), &json!(month_reset)));
        assert_eq!(card["seven_day"], Value::Null, "a monthly plan has no weekly window");
    }

    #[tokio::test]
    async fn the_forecast_counts_an_accounts_unpriced_requests() {
        let lab = crate::api_refusals::Lab::local("forecast-unpriced");
        let conn = Connection::open(&lab.state.config.db_path).unwrap();
        crate::pricing::ensure_table(&conn).unwrap();
        conn.execute_batch(
            "INSERT INTO accounts (id, label, source, active, created_at) VALUES ('work', 'Work', 'token', 1, 1);
             INSERT INTO requests (request_id, account_id, model, method, path, started_at)
             VALUES ('priced', 'work', 'claude-opus-5-5', 'POST', '/v1/messages', 1000),
                    ('unpriced', 'work', 'gpt-9', 'POST', '/v1/messages', 2000),
                    ('refused', 'work', 'claude-opus-5-5', 'POST', '/v1/messages', 3000);
             INSERT INTO usage (request_id, input_tokens, output_tokens)
             SELECT id, 1000, 100 FROM requests WHERE request_id IN ('priced', 'unpriced');",
        )
        .unwrap();
        crate::pricing::resolve_models(&conn).unwrap();
        let everything = Filter { project: None, session: None, account: None, since: None, until: None, tz: None };
        let got = forecast_inner(&lab.state, &everything).unwrap();
        let card = got["accounts"].as_array().unwrap().iter().find(|a| a["id"] == "work").unwrap().clone();
        // A request with no usage (refused) has nothing to price; only the one with usage and
        // no rate is unpriced, and the priced one still has its cost.
        assert_eq!((&card["usage"]["requests"], &card["usage"]["unpriced"]), (&json!(3), &json!(1)), "{card}");
        assert!(card["usage"]["cost"].as_f64().unwrap() > 0.0);
    }

    /// Three hours of one session, then half an hour of three: the 7-day window's short
    /// trend carries the new load, weighed by what the requests cost, beside the long one.
    #[tokio::test]
    async fn the_forecast_gives_a_short_trend() {
        let lab = crate::api_refusals::Lab::local("forecast-recent");
        let conn = Connection::open(&lab.state.config.db_path).unwrap();
        crate::pricing::ensure_table(&conn).unwrap();
        conn.execute_batch("INSERT INTO accounts (id, label, source, active, created_at) VALUES ('work', 'Work', 'token', 1, 1);")
            .unwrap();
        let now = crate::db::now_ms();
        let (five_reset, week_reset) = (now / 1000 + 2 * 3600, now / 1000 + 3 * 86_400);
        let start = now - 210 * 60_000;
        // One request a minute for three hours, then one every twenty seconds; each the same
        // size, so cost follows the request count. The meter climbs a percent every 18 minutes.
        let times: Vec<i64> =
            (0..180).map(|i| start + i * 60_000).chain((0..90).map(|i| start + 180 * 60_000 + i * 20_000)).collect();
        for (i, at) in times.iter().enumerate() {
            let week = 0.10 + (at - start) as f64 / (18.0 * 60_000.0) * 0.01;
            conn.execute(
                "INSERT INTO requests (request_id, account_id, model, method, path, started_at,
                                       rl_status, rl_5h_utilization, rl_5h_reset, rl_7d_utilization, rl_7d_reset)
                 VALUES (?1, 'work', 'claude-opus-5-5', 'POST', '/v1/messages', ?2, 'allowed', 0.2, ?3, ?4, ?5)",
                params![format!("r{i}"), at, five_reset, (week * 100.0).floor() / 100.0, week_reset],
            )
            .unwrap();
        }
        conn.execute_batch("INSERT INTO usage (request_id, input_tokens, output_tokens) SELECT id, 1000, 100 FROM requests;")
            .unwrap();
        crate::pricing::resolve_models(&conn).unwrap();

        let everything = Filter { project: None, session: None, account: None, since: None, until: None, tz: None };
        let got = forecast_inner(&lab.state, &everything).unwrap();
        let card = got["accounts"].as_array().unwrap().iter().find(|a| a["id"] == "work").unwrap().clone();
        let week = &card["seven_day"];
        assert_eq!(week["basis"], "this-window", "{week}");
        let recent = &week["recent"];
        let load = recent["load"].as_f64().unwrap_or_else(|| panic!("no short trend: {week}"));
        // Three times the requests in the last half hour, against a long stretch that
        // includes it: about 2.6×.
        assert!((2.2..3.0).contains(&load), "load {load}");
        let (long, short) = (week["burn_per_active_hour"].as_f64().unwrap(), recent["burn_per_active_hour"].as_f64().unwrap());
        assert!((short - long * load).abs() < 1e-9);
        assert_eq!(recent["measured_until"], json!(times.last().unwrap()));
        // A flat 5-hour meter has no slope, so no long trend and no short one beside it.
        assert_eq!(card["five_hour"]["recent"], Value::Null, "{}", card["five_hour"]);
    }
}
