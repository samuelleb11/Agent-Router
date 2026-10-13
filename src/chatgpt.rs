//! A ChatGPT sign-in pasted into the router: Codex's `auth.json`, from a login made for the
//! router with Codex's own interactive sign-in (AGEN-0199). The router keeps it in its
//! secrets file, signs Codex's requests with its access token, and refreshes it before the
//! token expires, the way Codex refreshes its own: a POST to OpenAI's token endpoint with the
//! refresh token and the access token's own OAuth client, keeping the rotated tokens.
//!
//! The login is the router's alone. A machine's Codex keeps its own, so two holders never
//! refresh one session — refresh tokens rotate, and a reused one can log the other out.

use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};

/// Where Codex refreshes a ChatGPT sign-in.
pub const TOKEN_URL: &str = "https://auth.openai.com/oauth/token";

/// How long before its access token expires a login is refreshed. Codex's tokens live ten
/// days; an hour leaves room for a router that was not asked anything for a while.
pub const RENEW_WITHIN_SECS: i64 = 3600;

/// The steps the dashboard and the phones show to extract a login for the router. `CODEX_HOME` is
/// exported, so the shell still names the folder when the login is printed and deleted.
pub const EXTRACT: &str = "On a machine with Codex, run `export CODEX_HOME=$(mktemp -d) && codex login -c cli_auth_credentials_store=file && cat \"$CODEX_HOME/auth.json\"`, sign in with ChatGPT and paste what it prints, then delete the folder with `rm -rf \"$CODEX_HOME\" && unset CODEX_HOME`.";

/// Why a pasted text is not a ChatGPT sign-in the router can keep, in words; `None` when it is.
pub fn login_problem(raw: &str) -> Option<String> {
    let Ok(v) = serde_json::from_str::<Value>(raw.trim()) else {
        return Some("that is not Codex's auth.json: paste the whole file, which is JSON".into());
    };
    let mode = v.get("auth_mode").and_then(Value::as_str);
    if matches!(mode, Some("apikey")) || v.get("OPENAI_API_KEY").and_then(Value::as_str).is_some_and(|k| !k.trim().is_empty()) {
        return Some("that login is an OpenAI API key, not a ChatGPT sign-in — add the key as an API-key account instead".into());
    }
    let tokens = v.get("tokens");
    let has = |key: &str| tokens.and_then(|t| t.get(key)).and_then(Value::as_str).is_some_and(|s| !s.trim().is_empty());
    if !has("access_token") || !has("refresh_token") || !has("id_token") {
        return Some("that auth.json holds no ChatGPT sign-in: sign in with ChatGPT in Codex first, then paste the file".into());
    }
    if claims(token(&v, "access_token")?).is_none() {
        return Some("that auth.json's access token cannot be read".into());
    }
    None
}

fn token<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get("tokens")?.get(key)?.as_str()
}

/// A JWT's claims, unverified: the router reads what the token says about itself, and OpenAI
/// is what checks it.
pub fn claims(jwt: &str) -> Option<Value> {
    let mut parts = jwt.split('.');
    let (header, payload, signature) = (parts.next()?, parts.next()?, parts.next()?);
    if header.is_empty() || payload.is_empty() || signature.is_empty() || parts.next().is_some() {
        return None;
    }
    let bytes = crate::auth::base64_decode(&payload.replace('-', "+").replace('_', "/"))?;
    serde_json::from_slice(&bytes).ok()
}

/// Who signed in and on which plan, from the id token: `(email, plan)`.
pub fn identity(raw: &str) -> (Option<String>, Option<String>) {
    let Ok(v) = serde_json::from_str::<Value>(raw.trim()) else { return (None, None) };
    let Some(id) = token(&v, "id_token").and_then(claims) else { return (None, None) };
    let text = |v: Option<&Value>| v.and_then(Value::as_str).filter(|s| !s.trim().is_empty()).map(str::to_owned);
    let email = text(id.get("email")).or_else(|| text(id.get("https://api.openai.com/profile").and_then(|p| p.get("email"))));
    let plan = text(id.get("https://api.openai.com/auth").and_then(|a| a.get("chatgpt_plan_type")));
    (email, plan)
}

/// When the access token expires, epoch milliseconds; 0 when it does not say.
pub fn expires_at_ms(raw: &str) -> i64 {
    serde_json::from_str::<Value>(raw.trim())
        .ok()
        .and_then(|v| token(&v, "access_token").and_then(claims))
        .and_then(|c| c.get("exp").and_then(Value::as_i64))
        .map_or(0, |exp| exp.saturating_mul(1000))
}

/// Whether the login's access token expires within `RENEW_WITHIN_SECS` of `now_ms`.
pub fn due(raw: &str, now_ms: i64) -> bool {
    let at = expires_at_ms(raw);
    at != 0 && at - now_ms < RENEW_WITHIN_SECS * 1000
}

/// The request a refresh sends: the grant, the refresh token and the OAuth client the access
/// token names. No scope: an omitted scope keeps the one granted (RFC 6749 §6).
pub fn refresh_body(raw: &str) -> Result<Value> {
    let v: Value = serde_json::from_str(raw.trim()).context("the stored ChatGPT login is not JSON")?;
    let refresh = token(&v, "refresh_token").filter(|t| !t.trim().is_empty()).ok_or_else(|| anyhow!("the stored ChatGPT login has no refresh token"))?;
    let client = token(&v, "access_token").and_then(claims).and_then(|c| c.get("client_id").and_then(Value::as_str).map(str::to_owned))
        .ok_or_else(|| anyhow!("the stored ChatGPT login's access token names no OAuth client"))?;
    Ok(json!({ "client_id": client, "grant_type": "refresh_token", "refresh_token": refresh }))
}

/// The login with a token endpoint's answer folded in: each token it sent replaces the old,
/// and `last_refresh` says when, as Codex records it.
pub fn rotated(raw: &str, answer: &Value, now_ms: i64) -> Result<String> {
    let mut v: Value = serde_json::from_str(raw.trim()).context("the stored ChatGPT login is not JSON")?;
    let tokens = v.get_mut("tokens").and_then(Value::as_object_mut).ok_or_else(|| anyhow!("the stored ChatGPT login has no tokens"))?;
    let mut any = false;
    for key in ["access_token", "refresh_token", "id_token"] {
        if let Some(t) = answer.get(key).and_then(Value::as_str).filter(|t| !t.trim().is_empty()) {
            tokens.insert(key.into(), json!(t));
            any = true;
        }
    }
    if !any || answer.get("access_token").and_then(Value::as_str).is_none() {
        return Err(anyhow!("OpenAI's token endpoint answered without an access token"));
    }
    v["last_refresh"] = json!(rfc3339(now_ms));
    Ok(serde_json::to_string_pretty(&v)?)
}

/// A moment in UTC as Codex writes `last_refresh`: `2026-10-13T12:00:00.000000Z`.
fn rfc3339(ms: i64) -> String {
    let (secs, millis) = (ms.div_euclid(1000), ms.rem_euclid(1000));
    let (days, rest) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:06}Z", rest / 3600, rest % 3600 / 60, rest % 60, millis * 1000)
}

/// Why OpenAI refused a refresh, in words that say what mends it.
pub fn refusal(status: u16, body: &[u8]) -> String {
    let v: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
    let code = v.get("error").and_then(|e| e.as_str().map(str::to_owned).or_else(|| e.get("code").and_then(Value::as_str).map(str::to_owned)))
        .unwrap_or_else(|| format!("HTTP {status}"));
    format!("OpenAI refused to refresh this ChatGPT login ({code}) — paste a fresh login from a new Codex sign-in")
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// An unsigned JWT with these claims: enough for the router, which never verifies one.
    pub(crate) fn jwt(claims: Value) -> String {
        let enc = |v: &[u8]| crate::tunnel::base64(v).trim_end_matches('=').replace('+', "-").replace('/', "_");
        format!("{}.{}.{}", enc(br#"{"alg":"RS256"}"#), enc(claims.to_string().as_bytes()), enc(b"not-a-signature"))
    }

    /// A ChatGPT sign-in as Codex writes it, its access token expiring at `exp` (seconds).
    pub(crate) fn login(exp: i64, refresh: &str) -> String {
        json!({
            "auth_mode": "chatgpt",
            "OPENAI_API_KEY": null,
            "tokens": {
                "id_token": jwt(json!({"email": "you@example.com", "https://api.openai.com/auth": {"chatgpt_plan_type": "pro", "chatgpt_account_id": "workspace-test"}})),
                "access_token": jwt(json!({"exp": exp, "client_id": "app_test_client", "https://api.openai.com/auth": {"chatgpt_account_id": "workspace-test"}})),
                "refresh_token": refresh,
                "account_id": "workspace-test",
            },
            "last_refresh": "2026-10-03T00:00:00Z",
        })
        .to_string()
    }

    #[test]
    fn chatgpt_login_is_read_for_its_identity_plan_and_expiry() {
        let raw = login(1_900_000_000, "rt-1");
        assert_eq!(login_problem(&raw), None);
        assert_eq!(identity(&raw), (Some("you@example.com".into()), Some("pro".into())));
        assert_eq!(expires_at_ms(&raw), 1_900_000_000_000);
        assert!(!due(&raw, 1_900_000_000_000 - 2 * 3_600_000), "two hours left is not due");
        assert!(due(&raw, 1_900_000_000_000 - 1_800_000), "half an hour left is due");
    }

    #[test]
    fn chatgpt_login_that_is_not_a_chatgpt_sign_in_is_refused_in_words() {
        assert!(login_problem("sk-proj-not-json").unwrap().contains("not Codex's auth.json"));
        assert!(login_problem(r#"{"auth_mode":"apikey","OPENAI_API_KEY":"sk-x"}"#).unwrap().contains("API key"));
        assert!(login_problem(r#"{"auth_mode":"chatgpt","tokens":{"access_token":"a.b.c"}}"#).unwrap().contains("no ChatGPT sign-in"));
    }

    #[test]
    fn chatgpt_login_refresh_sends_the_tokens_own_client_and_no_scope() {
        let body = refresh_body(&login(1_900_000_000, "rt-1")).unwrap();
        assert_eq!(body, json!({"client_id": "app_test_client", "grant_type": "refresh_token", "refresh_token": "rt-1"}));
        let answer = json!({"access_token": jwt(json!({"exp": 1_900_864_000, "client_id": "app_test_client"})), "refresh_token": "rt-2"});
        let now = crate::fold::parse_rfc3339_ms("2026-10-13T12:00:00Z").unwrap();
        let next = rotated(&login(1_900_000_000, "rt-1"), &answer, now).unwrap();
        let v: Value = serde_json::from_str(&next).unwrap();
        assert_eq!(v["tokens"]["refresh_token"], "rt-2", "the rotated refresh token is kept");
        assert_eq!(expires_at_ms(&next), 1_900_864_000_000);
        assert_eq!(identity(&next).0.as_deref(), Some("you@example.com"), "an id token not sent again is kept");
        assert_eq!(v["last_refresh"], "2026-10-13T12:00:00.000000Z");
        assert!(refusal(401, br#"{"error":{"code":"refresh_token_reused"}}"#).contains("refresh_token_reused"));
    }
}
