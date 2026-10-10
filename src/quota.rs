//! Live quota for the active account.
//!
//! Two sources, because neither covers every account:
//!
//! * **`/api/oauth/usage`** — the same endpoint Claude's own tooling uses. Accurate
//!   on demand and costs no tokens, so an idle machine still shows the truth. But a
//!   long-lived `claude setup-token` credential is **not** authorised for it (403);
//!   only an interactive OAuth session token is.
//! * **Captured response headers** — every proxied request carries
//!   `anthropic-ratelimit-unified-*`, which the store already records per account.
//!   Works for any credential, but only refreshes when traffic flows.
//!
//! So: try the endpoint, fall back to the store, and say in the payload which one the
//! numbers came from rather than pretending they are equivalent. Only the endpoint's
//! answer is cached. The store's is read again on every call, because the headers it
//! comes from move with every request the account serves.

use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use http_body_util::BodyExt;
use rusqlite::Connection;
use serde_json::{json, Value};

use crate::accounts::{Account, Provider, Store};
use crate::db::now_ms;
use crate::proxy::AppState;
use crate::refusal::{failed, refuse, JsonBody};

const UPSTREAM_PATH: &str = "/api/oauth/usage";

/// Anthropic's usage readings — `/api/oauth/usage` and claude.ai's — are asked with the
/// account's own credential, so only an Anthropic account's may go to them: another
/// provider's key sent there would be handed to a third party. Each fetch checks this
/// itself, before it builds a request, so no caller can get it wrong.
#[derive(Debug)]
pub struct NotAnthropic {
    id: String,
    provider: Provider,
}

impl std::fmt::Display for NotAnthropic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{:?} is a {} account, and Anthropic's usage readings are never asked with its key",
            self.id,
            self.provider.name()
        )
    }
}

impl std::error::Error for NotAnthropic {}

fn anthropic_only(account: &Account) -> anyhow::Result<()> {
    if account.provider != Provider::Anthropic {
        return Err(NotAnthropic { id: account.id.clone(), provider: account.provider }.into());
    }
    Ok(())
}

/// The account with this id, read from the store.
async fn account_of(state: &AppState, id: &str) -> anyhow::Result<Account> {
    let (db, id) = (state.config.db_path.clone(), id.to_string());
    tokio::task::spawn_blocking(move || {
        Store::new(db).get(&id)?.ok_or_else(|| anyhow::anyhow!("no account with id {id:?}"))
    })
    .await?
}
const CACHE_MS: i64 = 45_000;

pub struct Cached {
    account_id: String,
    fetched_at: i64,
    /// What the endpoint answered: its reading, or why there was none. Never the store's
    /// reading. That one used to be kept along with a failure, so through a 403's thirty
    /// minutes of backoff the meters showed the request before it, however many had come
    /// since.
    upstream: Result<Value, String>,
    /// Do not call upstream again before this instant. Without it a persistent 403
    /// turns every poll into a request and earns a 429 within a minute.
    retry_after: i64,
}

/// What the cache answers without calling upstream: the endpoint's reading and its age
/// while it is fresh, or the failure still being waited out. `None` calls upstream.
fn held(cached: Option<&Cached>, account_id: &str, now: i64) -> Option<Result<(Value, i64), String>> {
    let c = cached.filter(|c| c.account_id == account_id)?;
    let age = now - c.fetched_at;
    match &c.upstream {
        Ok(value) if age < CACHE_MS => Some(Ok((value.clone(), age))),
        Err(err) if now < c.retry_after => Some(Err(err.clone())),
        _ => None,
    }
}

/// Store a claude.ai session key for an account. Kept in the router rather than
/// asked for anywhere it could be seen: it goes straight to the Keychain.
#[derive(serde::Deserialize)]
pub struct UsageToken {
    pub account_id: String,
    #[serde(default)]
    pub token: String,
    /// Delete the stored key instead of setting one.
    #[serde(default)]
    pub clear: bool,
}

fn set_flag(state: &AppState, account_id: &str, set: bool) {
    if let Ok(conn) = rusqlite::Connection::open(&state.config.db_path) {
        let _ = conn.execute(
            "UPDATE accounts SET usage_key = ?2 WHERE id = ?1",
            rusqlite::params![account_id, set as i64],
        );
    }
}

/// Refusals answer with an error status (see `refusal`). Unlike `quota` below, which keeps
/// its 200: what reads that is a menu bar and a status bar, not a form.
pub async fn set_usage_token(
    State(state): State<Arc<AppState>>,
    JsonBody(req): JsonBody<UsageToken>,
) -> Response {
    if req.clear {
        let id = req.account_id.clone();
        let _ = tokio::task::spawn_blocking(move || crate::accounts::forget_usage_token(&id)).await;
        set_flag(&state, &req.account_id, false);
        state.quota_cache.write().await.remove(&req.account_id);
        return Json(json!({ "ok": true, "cleared": true })).into_response();
    }
    // 400 for either refusal below: what was sent is not a session key — nothing, or most
    // often an API key or an OAuth token pasted into the wrong field — and no state of the
    // router's would make it one.
    let token = req.token.trim().to_string();
    if token.is_empty() {
        return refuse(StatusCode::BAD_REQUEST, "no token supplied");
    }
    if !token.starts_with("sk-ant-sid") {
        return refuse(
            StatusCode::BAD_REQUEST,
            "that does not look like a claude.ai session key (expected sk-ant-sid…)",
        );
    }
    let id = req.account_id.clone();
    let stored = tokio::task::spawn_blocking(move || crate::accounts::store_usage_token(&id, &token))
        .await;
    match stored {
        Ok(Ok(())) => {
            set_flag(&state, &req.account_id, true);
            // The stored key changes where quota comes from, so drop what we cached.
            state.quota_cache.write().await.remove(&req.account_id);
            Json(json!({ "ok": true, "account_id": req.account_id })).into_response()
        }
        // 500: the secrets file would not take it, which is the router failing.
        Ok(Err(e)) => failed(e),
        Err(e) => failed(e),
    }
}

/// The unparsed claude.ai usage payload, for working out its shape. Loopback-only
/// like everything else here.
pub async fn raw_usage(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let account_id = match state.signing_credential().await {
        Ok((id, _)) => id,
        Err(err) => return Json(json!({ "error": err.to_string() })),
    };
    let account = match account_of(&state, &account_id).await {
        Ok(account) => account,
        Err(err) => return Json(json!({ "error": err.to_string() })),
    };
    match fetch_claude_ai_raw(&state, &account).await {
        Ok(v) => Json(v),
        Err(e) => Json(json!({ "error": e.to_string() })),
    }
}

async fn get_json(
    state: &AppState,
    url: &str,
    session_key: &str,
) -> anyhow::Result<Value> {
    let uri: axum::http::Uri = url.parse()?;
    let mut req = axum::http::Request::builder()
        .method("GET")
        .uri(uri)
        .header("accept", "application/json")
        .header("user-agent", concat!("agent-router/", env!("CARGO_PKG_VERSION")));
    {
        let headers = req.headers_mut().expect("builder has headers");
        let mut cookie =
            axum::http::HeaderValue::from_str(&format!("sessionKey={session_key}"))?;
        cookie.set_sensitive(true);
        headers.insert("cookie", cookie);
    }
    let resp = tokio::time::timeout(
        Duration::from_secs(10),
        state.client.request(req.body(axum::body::Body::empty())?),
    )
    .await??;
    let status = resp.status();
    let bytes = resp.into_body().collect().await?.to_bytes();
    if !status.is_success() {
        anyhow::bail!("claude.ai returned {status}");
    }
    Ok(serde_json::from_slice(&bytes)?)
}

/// Fetch usage from claude.ai with the account's session key, discovering the
/// organisation id on the way. Never for another provider's account (`NotAnthropic`).
async fn fetch_claude_ai_raw(state: &AppState, account: &Account) -> anyhow::Result<Value> {
    anthropic_only(account)?;
    let id = account.id.clone();
    let key = tokio::task::spawn_blocking(move || crate::accounts::usage_token(&id))
        .await?
        .ok_or_else(|| anyhow::anyhow!("no claude.ai session key stored for this account"))?;

    let orgs = get_json(state, "https://claude.ai/api/organizations", &key).await?;
    let org_id = orgs
        .as_array()
        .and_then(|a| a.first())
        .and_then(|o| o.get("uuid"))
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("could not find an organisation id"))?;

    get_json(
        state,
        &format!("https://claude.ai/api/organizations/{org_id}/usage"),
        &key,
    )
    .await
}

pub async fn quota(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let (account_id, token) = match state.signing_credential().await {
        Ok(signing) => signing,
        Err(err) => return Json(json!({ "error": err.to_string() })),
    };

    // The endpoint's reading while it is fresh; while backing off from a failure, the
    // store's newest reading, read again.
    let held = held(state.quota_cache.read().await.get(&account_id), &account_id, now_ms());
    match held {
        Some(Ok((value, age))) => return Json(annotate(value, age)),
        Some(Err(err)) => return Json(fallback(&state, &account_id, &err)),
        None => {}
    }

    let fetched = match account_of(&state, &account_id).await {
        Ok(account) => fetch_upstream(&state, &account, &token).await,
        Err(err) => Err(err),
    };
    match fetched {
        Ok(value) => {
            let value = value_for(&account_id, value, "usage-api");
            state.quota_cache.write().await.insert(account_id.clone(), Cached {
                account_id,
                fetched_at: now_ms(),
                upstream: Ok(value.clone()),
                retry_after: 0,
            });
            Json(annotate(value, 0))
        }
        Err(err) => {
            let err = err.to_string();
            let backoff = backoff_for(&err);
            tracing::warn!(
                error = %err,
                backoff_s = backoff / 1000,
                "quota endpoint unavailable; falling back to captured headers"
            );
            let value = fallback(&state, &account_id, &err);
            state.quota_cache.write().await.insert(account_id.clone(), Cached {
                account_id,
                fetched_at: now_ms(),
                upstream: Err(err),
                retry_after: now_ms() + backoff,
            });
            Json(value)
        }
    }
}

/// The store's newest reading for the account, or why there is none.
fn fallback(state: &AppState, account_id: &str, err: &str) -> Value {
    let stored = state.reads.get().ok().and_then(|conn| from_store(&conn, account_id));
    let value = stored
        .map(|v| value_for(account_id, v, "response-headers"))
        .unwrap_or_else(|| json!({
            "account_id": account_id,
            "source": "unavailable",
            "detail": err,
        }));
    annotate(value, 0)
}

/// A 403 will not start working on its own — this credential is simply not allowed
/// to read the endpoint — so back off hard rather than asking again in a minute.
fn backoff_for(err: &str) -> i64 {
    if err.contains("403") {
        30 * 60_000
    } else if err.contains("429") {
        5 * 60_000
    } else {
        60_000
    }
}

fn annotate(mut value: Value, age_ms: i64) -> Value {
    if let Some(obj) = value.as_object_mut() {
        obj.insert("age_ms".into(), json!(age_ms));
    }
    value
}

fn value_for(account_id: &str, mut value: Value, source: &str) -> Value {
    if let Some(obj) = value.as_object_mut() {
        obj.insert("account_id".into(), json!(account_id));
        obj.insert("source".into(), json!(source));
    }
    value
}

/// Anthropic's own reading of `account`'s windows, asked with `token`, its credential.
/// Never for another provider's account (`NotAnthropic`).
async fn fetch_upstream(state: &AppState, account: &Account, token: &str) -> anyhow::Result<Value> {
    anthropic_only(account)?;
    let uri: axum::http::Uri = format!("{}{UPSTREAM_PATH}", state.config.upstream).parse()?;

    let mut req = axum::http::Request::builder()
        .method("GET")
        .uri(uri)
        .header("accept", "application/json")
        .header("anthropic-beta", "oauth-2025-04-20")
        .header("user-agent", concat!("agent-router/", env!("CARGO_PKG_VERSION")));
    {
        let headers = req.headers_mut().expect("builder has headers");
        let mut auth = axum::http::HeaderValue::from_str(&format!("Bearer {token}"))?;
        auth.set_sensitive(true);
        headers.insert("authorization", auth);
    }

    let resp = tokio::time::timeout(
        Duration::from_secs(10),
        state.client.request(req.body(axum::body::Body::empty())?),
    )
    .await??;

    let status = resp.status();
    let bytes = resp.into_body().collect().await?.to_bytes();
    if !status.is_success() {
        anyhow::bail!("upstream returned {status}");
    }
    let raw: Value = serde_json::from_slice(&bytes)?;

    let window = |key: &str| {
        raw.get(key).map(|w| {
            json!({
                "utilization": w.get("utilization").and_then(Value::as_f64),
                "resets_at":   w.get("resets_at").and_then(Value::as_str),
            })
        })
    };
    let severity = raw
        .get("limits")
        .and_then(Value::as_array)
        .and_then(|l| {
            l.iter()
                .filter_map(|x| x.get("severity").and_then(Value::as_str))
                .max_by_key(|s| match *s {
                    "critical" => 3,
                    "warning" => 2,
                    "normal" => 1,
                    _ => 0,
                })
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "normal".into());

    Ok(json!({
        "five_hour": window("five_hour"),
        "seven_day": window("seven_day"),
        "severity":  severity,
    }))
}

/// `GET /api/quota`, and `GET /api/quota?account=<id>` for one account in particular: a
/// Kimi Code account's windows come from Kimi Code's own usage API (`kimi_quota`); any other
/// account named that is not the one signing is answered from the store, since only the
/// signing account's quota is asked of Anthropic.
#[derive(serde::Deserialize)]
pub struct QuotaQuery {
    pub account: Option<String>,
}

pub async fn quota_route(State(state): State<Arc<AppState>>, axum::extract::Query(q): axum::extract::Query<QuotaQuery>) -> Response {
    let Some(id) = q.account.filter(|a| !a.trim().is_empty()) else {
        return quota(State(state)).await.into_response();
    };
    let account = match account_of(&state, &id).await {
        Ok(account) => account,
        Err(e) => return Json(json!({ "error": e.to_string() })).into_response(),
    };
    if account.provider == Provider::Kimi {
        let base = account.base_url.clone().or(Provider::Kimi.default_base_url().map(str::to_owned)).unwrap_or_default();
        let fetch = |token: String| {
            let state = state.clone();
            async move { fetch_kimi_usages(&state, &base, &token).await }
        };
        return Json(kimi_quota(&state, &account, fetch).await).into_response();
    }
    match state.signing_credential().await {
        Ok((signing, _)) if signing == account.id => quota(State(state)).await.into_response(),
        _ => Json(fallback(&state, &account.id, "only the signing account's quota is asked of Anthropic")).into_response(),
    }
}

/// One window of a provider's own reading: how much of it is used (0..1), and when it
/// resets (epoch seconds), as the `rl_*` columns hold them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Window {
    pub used: f64,
    pub reset_at: i64,
}

/// Kimi Code's windows as its usage API gives them: a 5-hour window, and a second one, named
/// as the forecast names it, that is weekly where the plan counts a week (legacy plans) and
/// the monthly total otherwise.
#[derive(Debug, Clone, PartialEq)]
pub struct KimiReading {
    pub five_hour: Option<Window>,
    pub second: Option<(&'static str, Window)>,
}

/// Read both shapes `/v1/usages` has answered in (docs/sources-and-models.md): the newer,
/// pools of `{used_ratio, reset_time}` under `usages`, and the older, a weekly `usage` beside
/// `limits[]`, whose counts are strings. `None` when neither gives a window.
pub fn kimi_windows(body: &Value) -> Option<KimiReading> {
    fn reset(v: Option<&Value>) -> Option<i64> {
        crate::fold::parse_rfc3339_ms(v?.as_str()?).map(|ms| ms / 1000)
    }
    fn num(v: Option<&Value>) -> Option<f64> {
        let v = v?;
        v.as_f64().or_else(|| v.as_str()?.trim().parse().ok())
    }
    let pool = |name: &str| -> Option<Window> {
        let p = body.get("usages")?.get(name)?;
        Some(Window { used: num(p.get("used_ratio"))?.clamp(0.0, 1.0), reset_at: reset(p.get("reset_time"))? })
    };
    // A count: what is used of a limit, from `used`, else from what `remaining` leaves.
    let count = |d: &Value| -> Option<Window> {
        let limit = num(d.get("limit")).filter(|l| *l > 0.0)?;
        let used = num(d.get("used")).or_else(|| Some(limit - num(d.get("remaining"))?))?;
        Some(Window { used: (used / limit).clamp(0.0, 1.0), reset_at: reset(d.get("resetTime"))? })
    };
    let five_hour_limit = body.get("limits").and_then(Value::as_array).and_then(|limits| {
        limits.iter().find_map(|l| {
            let w = l.get("window")?;
            let minutes = num(w.get("duration"))?;
            (minutes == 300.0 && w.get("timeUnit").and_then(Value::as_str) == Some("TIME_UNIT_MINUTE"))
                .then(|| count(l.get("detail")?))
                .flatten()
        })
    });
    let five_hour = pool("limit_5h").or(five_hour_limit);
    let second = pool("limit_7d")
        .map(|w| ("seven_day", w))
        .or_else(|| pool("limit_month_total").map(|w| ("month", w)))
        .or_else(|| body.get("usage").and_then(count).map(|w| ("seven_day", w)));
    (five_hour.is_some() || second.is_some()).then_some(KimiReading { five_hour, second })
}

/// The request for an account's windows: its key as a bearer token, and the router's own
/// User-Agent, as Kimi Code's terms ask of every tool. Never Kimi's CLI's.
pub fn kimi_usages_request(base_url: &str, key: &str) -> anyhow::Result<axum::http::Request<axum::body::Body>> {
    let mut auth = axum::http::HeaderValue::from_str(&format!("Bearer {key}"))?;
    auth.set_sensitive(true);
    Ok(axum::http::Request::builder()
        .method("GET")
        .uri(format!("{}/v1/usages", base_url.trim_end_matches('/')))
        .header("accept", "application/json")
        .header("user-agent", concat!("agent-router/", env!("CARGO_PKG_VERSION")))
        .header("authorization", auth)
        .body(axum::body::Body::empty())?)
}

async fn fetch_kimi_usages(state: &AppState, base_url: &str, key: &str) -> anyhow::Result<Value> {
    let resp = tokio::time::timeout(Duration::from_secs(10), state.client.request(kimi_usages_request(base_url, key)?))
        .await??;
    let status = resp.status();
    let bytes = resp.into_body().collect().await?.to_bytes();
    if !status.is_success() {
        anyhow::bail!("Kimi Code returned {status}");
    }
    Ok(serde_json::from_slice(&bytes)?)
}

/// A reading as `/api/quota` answers, in the shape Anthropic's is answered in: each window's
/// utilisation 0..100 and its reset in RFC 3339, under the forecast's name for it.
fn kimi_value(account_id: &str, r: &KimiReading) -> Value {
    let iso = |at: i64| {
        let days = at.div_euclid(86_400);
        let secs = at.rem_euclid(86_400);
        // Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = doy - (153 * mp + 2) / 5 + 1;
        let m = if mp < 10 { mp + 3 } else { mp - 9 };
        let y = yoe + era * 400 + i64::from(m <= 2);
        format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", secs / 3600, secs / 60 % 60, secs % 60)
    };
    let window = |w: &Window| json!({ "utilization": w.used * 100.0, "resets_at": iso(w.reset_at) });
    let most = r.five_hour.iter().chain(r.second.iter().map(|(_, w)| w)).map(|w| w.used).fold(0.0, f64::max);
    let mut v = json!({
        "account_id": account_id,
        "source": "kimi-usages",
        "severity": if most >= 0.95 { "critical" } else if most >= 0.8 { "warning" } else { "normal" },
        "five_hour": r.five_hour.as_ref().map(window),
    });
    if let Some((name, w)) = &r.second {
        v[*name] = window(w);
        v["second_window"] = json!(name);
    }
    v
}

/// Keep a reading, for the forecast: one row per reading, as `requests` keeps a row per
/// response for Anthropic's.
fn store_reading(db: &std::path::Path, account_id: &str, r: &KimiReading) -> anyhow::Result<()> {
    let conn = Connection::open(db)?;
    conn.execute(
        "INSERT INTO quota_readings (account_id, at, five_hour_used, five_hour_reset, second_window, second_used, second_reset)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        rusqlite::params![
            account_id,
            now_ms(),
            r.five_hour.map(|w| w.used),
            r.five_hour.map(|w| w.reset_at),
            r.second.map(|(n, _)| n),
            r.second.map(|(_, w)| w.used),
            r.second.map(|(_, w)| w.reset_at),
        ],
    )?;
    Ok(())
}

/// A Kimi Code account's windows, asked of its usage API with `fetch` (given the account's
/// key), cached and backed off as Anthropic's endpoint is. When asking fails, the newest kept
/// reading answers, saying so; with none, why there is none.
pub async fn kimi_quota<F, Fut>(state: &AppState, account: &Account, fetch: F) -> Value
where
    F: FnOnce(String) -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<Value>>,
{
    let id = account.id.clone();
    match held(state.quota_cache.read().await.get(&id), &id, now_ms()) {
        Some(Ok((value, age))) => return annotate(value, age),
        Some(Err(err)) => return kimi_fallback(state, &id, &err),
        None => {}
    }
    let asked = async {
        let signer = state.signer(&id).await?;
        let body = fetch(signer.token).await?;
        kimi_windows(&body).ok_or_else(|| anyhow::anyhow!("Kimi Code's usage answer gave no window"))
    }
    .await;
    match asked {
        Ok(reading) => {
            let (db, account_id, kept) = (state.config.db_path.clone(), id.clone(), reading.clone());
            if let Ok(Err(e)) = tokio::task::spawn_blocking(move || store_reading(&db, &account_id, &kept)).await {
                tracing::warn!(account = %id, "a Kimi Code reading could not be kept: {e:#}");
            }
            let value = kimi_value(&id, &reading);
            state.quota_cache.write().await.insert(
                id.clone(),
                Cached { account_id: id.clone(), fetched_at: now_ms(), upstream: Ok(value.clone()), retry_after: 0 },
            );
            annotate(value, 0)
        }
        Err(err) => {
            let err = err.to_string();
            let backoff = backoff_for(&err);
            tracing::warn!(account = %id, error = %err, "Kimi Code's usage API unavailable");
            state.quota_cache.write().await.insert(
                id.clone(),
                Cached { account_id: id.clone(), fetched_at: now_ms(), upstream: Err(err.clone()), retry_after: now_ms() + backoff },
            );
            kimi_fallback(state, &id, &err)
        }
    }
}

/// The newest kept reading for a Kimi Code account, or why there is none.
fn kimi_fallback(state: &AppState, account_id: &str, err: &str) -> Value {
    let kept = state.reads.get().ok().and_then(|conn| {
        conn.query_row(
            "SELECT five_hour_used, five_hour_reset, second_window, second_used, second_reset, at
             FROM quota_readings WHERE account_id = ?1 ORDER BY at DESC LIMIT 1",
            [account_id],
            |r| {
                let five = match (r.get::<_, Option<f64>>(0)?, r.get::<_, Option<i64>>(1)?) {
                    (Some(used), Some(reset_at)) => Some(Window { used, reset_at }),
                    _ => None,
                };
                let second = match (r.get::<_, Option<String>>(2)?, r.get::<_, Option<f64>>(3)?, r.get::<_, Option<i64>>(4)?) {
                    (Some(n), Some(used), Some(reset_at)) => {
                        Some((if n == "month" { "month" } else { "seven_day" }, Window { used, reset_at }))
                    }
                    _ => None,
                };
                Ok((KimiReading { five_hour: five, second }, r.get::<_, i64>(5)?))
            },
        )
        .ok()
    });
    match kept {
        Some((reading, at)) => {
            let mut v = kimi_value(account_id, &reading);
            v["source"] = json!("kimi-usages-kept");
            v["seen_at"] = json!(at);
            v["detail"] = json!(err);
            annotate(v, 0)
        }
        None => annotate(json!({ "account_id": account_id, "source": "unavailable", "detail": err }), 0),
    }
}

/// Ask every Kimi Code account's usage API now and then, so its card on the forecast stays
/// current with nobody looking at its quota. Started once with the router.
pub fn keep_kimi_readings(state: Arc<AppState>) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(5 * 60));
        loop {
            tick.tick().await;
            let db = state.config.db_path.clone();
            let ids: Vec<String> = tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<String>> {
                let conn = Connection::open(db)?;
                let mut stmt = conn.prepare("SELECT id FROM accounts WHERE provider = 'kimi'")?;
                let ids = stmt.query_map([], |r| r.get(0))?.collect::<Result<Vec<String>, _>>()?;
                Ok(ids)
            })
            .await
            .ok()
            .and_then(Result::ok)
            .unwrap_or_default();
            for id in ids {
                let Ok(account) = account_of(&state, &id).await else { continue };
                let base = account.base_url.clone().or(Provider::Kimi.default_base_url().map(str::to_owned)).unwrap_or_default();
                let fetch = |token: String| {
                    let state = state.clone();
                    async move { fetch_kimi_usages(&state, &base, &token).await }
                };
                kimi_quota(&state, &account, fetch).await;
            }
        }
    });
}

/// The most recent rate-limit headers this account actually received. Stored as a
/// 0..1 fraction; the endpoint reports 0..100, so scale to match.
fn from_store(conn: &Connection, account_id: &str) -> Option<Value> {
    let row = conn
        .query_row(
            "SELECT rl_5h_utilization, rl_7d_utilization, rl_5h_reset, rl_7d_reset,
                    rl_status, started_at
             FROM requests
             WHERE account_id = ?1 AND rl_status IS NOT NULL
             ORDER BY started_at DESC LIMIT 1",
            [account_id],
            |r| {
                Ok((
                    r.get::<_, Option<f64>>(0)?,
                    r.get::<_, Option<f64>>(1)?,
                    r.get::<_, Option<i64>>(2)?,
                    r.get::<_, Option<i64>>(3)?,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, i64>(5)?,
                ))
            },
        )
        .ok()?;
    let (u5, u7, r5, r7, status, seen_at) = row;
    let iso = |epoch: Option<i64>| epoch.map(|e| format!("@{e}"));
    let severity = match status.as_deref() {
        Some(s) if s.contains("critical") => "critical",
        Some(s) if s.contains("warning") => "warning",
        _ => "normal",
    };
    Some(json!({
        "five_hour": { "utilization": u5.map(|v| v * 100.0), "resets_at": iso(r5) },
        "seven_day": { "utilization": u7.map(|v| v * 100.0), "resets_at": iso(r7) },
        "severity":  severity,
        "seen_at":   seen_at,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;

    const T0: i64 = 1_700_000_000_000;
    const MIN: i64 = 60_000;

    fn failed(account_id: &str, at: i64, err: &str) -> Cached {
        Cached {
            account_id: account_id.into(),
            fetched_at: at,
            upstream: Err(err.into()),
            retry_after: at + backoff_for(err),
        }
    }

    /// A 403 is waited out for thirty minutes, and through all of them the store is read
    /// again rather than the reading taken beside the failure: `held` hands back the
    /// failure, never a value. A fresh reading from the endpoint is served as it is.
    #[test]
    fn a_failure_is_waited_out_without_keeping_a_reading_and_a_fresh_one_is_served() {
        let forbidden = failed("work", T0, "upstream returned 403 Forbidden");
        for at in [T0 + MIN, T0 + 10 * MIN, T0 + 29 * MIN] {
            assert_eq!(held(Some(&forbidden), "work", at), Some(Err("upstream returned 403 Forbidden".into())));
        }
        assert_eq!(held(Some(&forbidden), "work", T0 + 30 * MIN), None, "the endpoint is asked again");
        assert_eq!(held(Some(&forbidden), "side", T0 + MIN), None, "another account's failure is not this one's");

        let reading = json!({ "five_hour": { "utilization": 41.0 }, "source": "usage-api" });
        let fresh = Cached { account_id: "work".into(), fetched_at: T0, upstream: Ok(reading.clone()), retry_after: 0 };
        assert_eq!(held(Some(&fresh), "work", T0 + 10_000), Some(Ok((reading, 10_000))));
        assert_eq!(held(Some(&fresh), "work", T0 + CACHE_MS), None);
        assert_eq!(held(None, "work", T0), None);
    }

    /// The store's reading is the account's newest request that carried the headers,
    /// scaled to the endpoint's 0..100.
    #[test]
    fn the_store_answers_with_the_accounts_newest_rate_limit_reading() {
        let dir = std::env::temp_dir().join(format!("ar-quota-store-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("agent-router.db");
        crate::db::Db::open(&path).unwrap();
        let conn = Connection::open(&path).unwrap();
        let request = "INSERT INTO requests (request_id, account_id, method, path, started_at,
                                             rl_status, rl_5h_utilization, rl_7d_utilization, rl_5h_reset, rl_7d_reset)
                       VALUES (?1, ?2, 'POST', '/v1/messages', ?3, ?4, ?5, ?6, ?7, ?8)";
        let none: Option<&str> = None;
        let reset = (T0 + 300 * MIN) / 1000;
        conn.execute(request, params!["older", "work", T0, "allowed", 0.33, 0.5, reset, reset]).unwrap();
        conn.execute(request, params!["newest", "work", T0 + 20 * MIN, "allowed_warning", 0.52, 0.51, reset, reset]).unwrap();
        // Later, but without the headers, or for another account.
        conn.execute(request, params!["bare", "work", T0 + 25 * MIN, none, None::<f64>, None::<f64>, None::<i64>, None::<i64>]).unwrap();
        conn.execute(request, params!["side", "side", T0 + 30 * MIN, "allowed", 0.9, 0.9, reset, reset]).unwrap();

        let got = from_store(&conn, "work").unwrap();
        assert_eq!(got["five_hour"]["utilization"], json!(52.0));
        assert_eq!(got["seven_day"]["utilization"], json!(51.0));
        assert_eq!(got["five_hour"]["resets_at"], json!(format!("@{reset}")));
        assert_eq!(got["severity"], json!("warning"));
        assert_eq!(got["seen_at"], json!(T0 + 20 * MIN));
        assert_eq!(from_store(&conn, "nobody"), None);

        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn account(id: &str, provider: Provider) -> Account {
        Account {
            id: id.into(),
            label: id.into(),
            source: crate::accounts::Source::Token,
            provider,
            base_url: provider.default_base_url().map(str::to_owned),
            profile_dir: None,
            email: None,
        }
    }

    #[tokio::test]
    async fn quota_never_sends_a_kimi_key_to_anthropic() {
        let lab = crate::api_refusals::Lab::local("quota-kimi");
        let kimi = account("quota-kimi", Provider::Kimi);
        let refused = fetch_upstream(&lab.state, &kimi, "sk-kimi-not-a-credential").await.unwrap_err();
        assert!(refused.downcast_ref::<NotAnthropic>().is_some(), "refused before any request: {refused:#}");
        assert!(anthropic_only(&account("work", Provider::Anthropic)).is_ok(), "an Anthropic account's is asked");

        // A router whose only account is a Kimi Code one: /api/quota has no Anthropic account
        // to read for, and says so rather than reading for the Kimi Code one.
        let lab = crate::api_refusals::Lab::hosted("quota-kimi-only").await;
        let add = json!({ "label": "Quota Kimi only", "source": "token", "provider": "kimi", "token": "sk-kimi-not-a-credential" });
        assert_eq!(lab.post("/api/accounts", &add.to_string()).await.0, StatusCode::OK);
        let answer = quota(State(lab.state.clone())).await.into_response();
        let answer: Value =
            serde_json::from_slice(&axum::body::to_bytes(answer.into_body(), 1 << 20).await.unwrap()).unwrap();
        assert!(answer["error"].as_str().unwrap_or_default().contains("no account to sign with"), "{answer}");
        assert!(answer.get("source").is_none(), "{answer}");
    }

    #[tokio::test]
    async fn the_claude_ai_fallback_skips_a_kimi_account() {
        let lab = crate::api_refusals::Lab::local("quota-kimi-sid");
        let kimi = account("quota-kimi-sid", Provider::Kimi);
        // Even with a claude.ai session key stored under it, nothing goes to claude.ai.
        crate::accounts::store_usage_token(&kimi.id, "sk-ant-sid01-not-a-credential").unwrap();
        let refused = fetch_claude_ai_raw(&lab.state, &kimi).await.unwrap_err();
        crate::accounts::forget_usage_token(&kimi.id).unwrap();
        assert!(refused.downcast_ref::<NotAnthropic>().is_some(), "refused before any request: {refused:#}");
    }

    #[test]
    fn kimi_usages_newer_shape_gives_its_windows() {
        // As recorded from the owner's plan on 2026-10-10 (fixtures/kimi/usages.json).
        let recorded: Value = serde_json::from_str(include_str!("../fixtures/kimi/usages.json")).unwrap();
        assert_eq!(
            kimi_windows(&recorded),
            Some(KimiReading {
                five_hour: Some(Window { used: 0.0, reset_at: 1_791_618_562 }),
                second: Some(("month", Window { used: 0.0, reset_at: 1_794_355_200 })),
            })
        );
        // A weekly pool, where a plan counts a week, is the second window before the month.
        let weekly = json!({ "usages": {
            "limit_5h": { "used_ratio": 0.25, "reset_time": "2026-10-10T07:49:22Z" },
            "limit_7d": { "used_ratio": 0.42, "reset_time": "2026-11-11T00:00:00Z" },
            "limit_month_total": { "used_ratio": 0.9, "reset_time": "2026-11-11T00:00:00Z" } } });
        let r = kimi_windows(&weekly).unwrap();
        assert_eq!(r.second, Some(("seven_day", Window { used: 0.42, reset_at: 1_794_355_200 })));
        // With no 5-hour pool, the 300-minute count window stands in, from what remains of it.
        let counted = json!({ "limits": [{ "window": { "duration": 300, "timeUnit": "TIME_UNIT_MINUTE" },
            "detail": { "limit": "100", "remaining": "60", "resetTime": "2026-10-10T07:49:23.451603Z" } }] });
        assert_eq!(kimi_windows(&counted).unwrap().five_hour, Some(Window { used: 0.4, reset_at: 1_791_618_563 }));
        assert_eq!(kimi_windows(&json!({ "nothing": true })), None);
    }

    #[test]
    fn kimi_usages_older_shape_gives_its_windows() {
        // The shape third-party meters read before `usages`: a weekly `usage` beside
        // `limits[]`, counts as strings.
        let older = json!({
            "usage": { "limit": "100", "used": "51", "remaining": "49", "resetTime": "2026-08-31T23:09:00Z" },
            "limits": [{ "window": { "duration": 300, "timeUnit": "TIME_UNIT_MINUTE" },
                         "detail": { "limit": "100", "used": "100", "remaining": "0", "resetTime": "2026-08-30T01:09:00Z" } }],
        });
        assert_eq!(
            kimi_windows(&older),
            Some(KimiReading {
                five_hour: Some(Window { used: 1.0, reset_at: 1_788_052_140 }),
                second: Some(("seven_day", Window { used: 0.51, reset_at: 1_788_217_740 })),
            })
        );
    }

    #[test]
    fn kimi_usages_is_asked_as_agent_router() {
        let req = kimi_usages_request("https://api.kimi.com/coding/", "sk-kimi-not-a-credential").unwrap();
        assert_eq!(req.method(), "GET");
        assert_eq!(req.uri().to_string(), "https://api.kimi.com/coding/v1/usages", "at the account's own base URL");
        assert_eq!(req.headers()["authorization"], "Bearer sk-kimi-not-a-credential", "with the account's key");
        let ua = req.headers()["user-agent"].to_str().unwrap();
        assert!(ua.starts_with("agent-router/"), "the router's own User-Agent: {ua}");
        assert!(!ua.to_ascii_lowercase().contains("kimi"), "never Kimi's CLI's: {ua}");
    }

    #[tokio::test]
    async fn api_quota_answers_a_kimi_accounts_windows() {
        let lab = crate::api_refusals::Lab::local("quota-kimi-windows");
        let add = json!({ "label": "Quota kimi windows", "source": "token", "provider": "kimi", "token": "sk-kimi-not-a-credential" });
        assert_eq!(lab.post("/api/accounts", &add.to_string()).await.0, StatusCode::OK);
        let account = account_of(&lab.state, "quota-kimi-windows").await.unwrap();

        // Kimi Code's usage API, as the reader asks it: with the account's own key.
        let answer = json!({ "usages": {
            "limit_5h": { "used_ratio": 0.25, "reset_time": "2099-01-01T05:00:00Z" },
            "limit_month_total": { "used_ratio": 0.5, "reset_time": "2099-02-01T00:00:00Z" } } });
        let asked = std::sync::Arc::new(std::sync::Mutex::new(None));
        let fetch = |token: String| {
            let (asked, answer) = (asked.clone(), answer.clone());
            async move {
                *asked.lock().unwrap() = Some(token);
                Ok(answer)
            }
        };
        let got = kimi_quota(&lab.state, &account, fetch).await;
        assert_eq!(asked.lock().unwrap().as_deref(), Some("sk-kimi-not-a-credential"));
        assert_eq!((&got["source"], &got["second_window"]), (&json!("kimi-usages"), &json!("month")));
        assert_eq!(got["five_hour"], json!({ "utilization": 25.0, "resets_at": "2099-01-01T05:00:00Z" }));
        assert_eq!(got["month"], json!({ "utilization": 50.0, "resets_at": "2099-02-01T00:00:00Z" }));
        let kept: i64 = Connection::open(&lab.state.config.db_path)
            .unwrap()
            .query_row("SELECT count(*) FROM quota_readings WHERE account_id = 'quota-kimi-windows'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(kept, 1, "the reading is kept for the forecast");

        // GET /api/quota?account= answers the same, from the reading just taken.
        let query = QuotaQuery { account: Some("quota-kimi-windows".into()) };
        let res = quota_route(State(lab.state.clone()), axum::extract::Query(query)).await;
        let routed: Value = serde_json::from_slice(&axum::body::to_bytes(res.into_body(), 1 << 20).await.unwrap()).unwrap();
        assert_eq!((&routed["five_hour"], &routed["month"]), (&got["five_hour"], &got["month"]), "{routed}");
    }
}
