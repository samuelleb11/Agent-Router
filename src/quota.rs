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

use crate::db::now_ms;
use crate::proxy::AppState;
use crate::refusal::{failed, refuse, JsonBody};

const UPSTREAM_PATH: &str = "/api/oauth/usage";
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
        *state.quota_cache.write().await = None;
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
            *state.quota_cache.write().await = None;
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
    match fetch_claude_ai_raw(&state, &account_id).await {
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
/// organisation id on the way.
async fn fetch_claude_ai_raw(state: &AppState, account_id: &str) -> anyhow::Result<Value> {
    let id = account_id.to_string();
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
    let account_id = match state.signing_credential().await {
        Ok((id, _)) => id,
        Err(err) => return Json(json!({ "error": err.to_string() })),
    };

    // The endpoint's reading while it is fresh; while backing off from a failure, the
    // store's newest reading, read again.
    let held = held(state.quota_cache.read().await.as_ref(), &account_id, now_ms());
    match held {
        Some(Ok((value, age))) => return Json(annotate(value, age)),
        Some(Err(err)) => return Json(fallback(&state, &account_id, &err)),
        None => {}
    }

    match fetch_upstream(&state).await {
        Ok(value) => {
            let value = value_for(&account_id, value, "usage-api");
            *state.quota_cache.write().await = Some(Cached {
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
            *state.quota_cache.write().await = Some(Cached {
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

async fn fetch_upstream(state: &AppState) -> anyhow::Result<Value> {
    let (_, token) = state.signing_credential().await?;
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
}
