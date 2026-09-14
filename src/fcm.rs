//! Reaching a phone through Google's push service.
//!
//! Apple takes a signed token as the bearer directly. Google wants it exchanged first: a
//! JWT signed with the service account's RSA key (RS256) buys an access token from
//! `oauth2.googleapis.com`, and *that* is the bearer on the send. It lasts an hour and is
//! reused for fifty-five minutes. Apart from the extra hop the shape is `apns`'s, down to
//! sharing its `Delivery` and `SendError`.
//!
//! Every message here is data-only: nothing asks Android to draw a notification. The app
//! draws its own, which is what lets a prompt replace the banner it raised earlier, group
//! with its session, and be taken down again by the same key. A message carrying a
//! `notification` block would be drawn by the system whenever the app was in the
//! background, and never reach the code that knows any of that.
//!
//! The credential is the service-account JSON from the Firebase console. Unlike Apple's
//! `.p8` it can be re-issued at will, so losing one costs nothing but a click.

use std::sync::Mutex;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper_util::client::legacy::{connect::HttpConnector, Client};
use hyper_util::rt::TokioExecutor;
use ring::signature::{RsaKeyPair, RSA_PKCS1_SHA256};
use serde_json::{json, Value};

use crate::apns::{b64url, pem_to_der, Delivery, SendError};

type HClient = Client<hyper_rustls::HttpsConnector<HttpConnector>, Full<Bytes>>;

/// Google's access token is good for an hour; refresh before that.
const TOKEN_LIFE_MS: i64 = 55 * 60_000;
/// The one scope a sender needs.
const SCOPE: &str = "https://www.googleapis.com/auth/firebase.messaging";
const SEND_HOST: &str = "fcm.googleapis.com";

/// A service account, as the Firebase console hands it out.
#[derive(Clone, Debug)]
pub struct Key {
    pub project_id: String,
    pub client_email: String,
    /// PKCS#8 PEM, the file's `private_key` verbatim.
    pub private_key: String,
    /// `private_key_id`; rides in the JWT header as `kid`, and is the only way to tell
    /// two service accounts apart at a glance. An id, not a secret.
    pub private_key_id: String,
    /// Where the assertion is exchanged. The file names it, so it is carried rather than
    /// assumed — Google has moved this host before.
    pub token_uri: String,
}

/// Read a service-account file. Returns what is missing rather than a bare failure,
/// because the usual mistake is dropping in the *other* JSON the console offers.
pub fn parse_service_account(text: &str) -> anyhow::Result<Key> {
    let v: Value = serde_json::from_str(text).map_err(|e| anyhow::anyhow!("not JSON: {e}"))?;
    if v["type"].as_str() != Some("service_account") {
        anyhow::bail!("not a service account file — its \"type\" should be \"service_account\"");
    }
    let field = |k: &str| -> anyhow::Result<String> {
        v[k].as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .ok_or_else(|| anyhow::anyhow!("no {k} in the service account file"))
    };
    Ok(Key {
        project_id: field("project_id")?,
        client_email: field("client_email")?,
        private_key: field("private_key")?,
        private_key_id: field("private_key_id")?,
        token_uri: field("token_uri")
            .unwrap_or_else(|_| "https://oauth2.googleapis.com/token".into()),
    })
}

pub struct Fcm {
    client: HClient,
    /// (client email, access token, minted at ms).
    token: Mutex<Option<(String, String, i64)>>,
}

impl Fcm {
    pub fn new() -> Self {
        let tls = hyper_rustls::HttpsConnectorBuilder::new()
            .with_webpki_roots()
            .https_only()
            .enable_all_versions()
            .build();
        let client = Client::builder(TokioExecutor::new()).build(tls);
        Self { client, token: Mutex::new(None) }
    }

    pub fn forget_token(&self) {
        *self.token.lock().unwrap() = None;
    }

    fn cached(&self, key: &Key, now: i64) -> Option<String> {
        let held = self.token.lock().unwrap();
        let (who, tok, at) = held.as_ref()?;
        (who == &key.client_email && now - at < TOKEN_LIFE_MS).then(|| tok.clone())
    }

    /// The access token, minted if the held one is missing or old.
    async fn bearer(&self, key: &Key) -> Result<String, SendError> {
        let now = crate::db::now_ms();
        if let Some(tok) = self.cached(key, now) {
            return Ok(tok);
        }
        let assertion = mint_jwt(key, now / 1000)
            .map_err(|e| SendError::Transport(format!("signing key: {e}")))?;
        // The assertion is base64url and the grant type is a constant, so neither needs
        // percent-encoding; nothing else goes in the body.
        let form = format!(
            "grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Ajwt-bearer&assertion={assertion}"
        );
        let req = hyper::Request::post(&key.token_uri)
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Full::new(Bytes::from(form)))
            .map_err(|e| SendError::Transport(e.to_string()))?;
        let (status, body) = self.round_trip(req).await?;
        if !status.is_success() {
            let why = describe(&body);
            let why = format!("token exchange refused: {why}");
            return Err(SendError::Rejected(status.as_u16(), why));
        }
        let tok = serde_json::from_slice::<Value>(&body)
            .ok()
            .and_then(|v| v["access_token"].as_str().map(str::to_owned))
            .ok_or_else(|| SendError::Transport("no access token in Google's reply".into()))?;
        *self.token.lock().unwrap() = Some((key.client_email.clone(), tok.clone(), now));
        Ok(tok)
    }

    async fn round_trip(
        &self,
        req: hyper::Request<Full<Bytes>>,
    ) -> Result<(hyper::StatusCode, Bytes), SendError> {
        let resp = tokio::time::timeout(Duration::from_secs(15), self.client.request(req))
            .await
            .map_err(|_| SendError::Transport("timed out".into()))?
            .map_err(|e| SendError::Transport(e.to_string()))?;
        let status = resp.status();
        let body = resp.into_body().collect().await.map(|b| b.to_bytes()).unwrap_or_default();
        Ok((status, body))
    }

    /// Send one data-only message to one device.
    pub async fn send(
        &self,
        key: &Key,
        device: &str,
        delivery: Delivery<'_>,
        data: &Value,
    ) -> Result<(), SendError> {
        let bearer = self.bearer(key).await?;
        let uri = format!("https://{SEND_HOST}/v1/projects/{}/messages:send", key.project_id);
        let req = hyper::Request::post(uri)
            .header("authorization", format!("Bearer {bearer}"))
            .header("content-type", "application/json")
            .body(Full::new(Bytes::from(message(device, delivery, data).to_string())))
            .map_err(|e| SendError::Transport(e.to_string()))?;
        let (status, body) = self.round_trip(req).await?;
        if status.is_success() {
            return Ok(());
        }
        let code = error_code(&body);
        if status.as_u16() == 401 {
            // The token went stale early; the next send mints a fresh one.
            self.forget_token();
        }
        if forget_device(status.as_u16(), &code, &body) {
            return Err(SendError::Unregistered);
        }
        Err(SendError::Rejected(status.as_u16(), describe(&body)))
    }
}

/// One FCM v1 message. `data` only — see the module note on why.
fn message(device: &str, delivery: Delivery<'_>, data: &Value) -> Value {
    // Both deliveries go high priority. A normal-priority data message is held until the
    // next Doze maintenance window, which for a dismissal means a banner sitting on a
    // phone for minutes after the prompt behind it was answered.
    let mut android = json!({ "priority": "high" });
    if let Some(id) = delivery.collapse_id() {
        android["collapse_key"] = json!(id);
    }
    if let Some(secs) = delivery.life_secs() {
        android["ttl"] = json!(format!("{secs}s"));
    }
    json!({ "message": { "token": device, "data": stringify(data), "android": android } })
}

/// FCM takes a flat map of strings and refuses anything else outright. Nulls and empty
/// strings are dropped rather than sent as `""`, so the app has one case for "absent"
/// instead of two; anything not already a string is rendered as compact JSON, which is
/// how a list of answered prompts crosses as a single value.
fn stringify(data: &Value) -> Value {
    let Some(map) = data.as_object() else { return json!({}) };
    let out: serde_json::Map<String, Value> = map
        .iter()
        // `from`, `google*` and `gcm*` are FCM's own; a message using one is rejected.
        .filter(|(k, _)| *k != "from" && !k.starts_with("google") && !k.starts_with("gcm"))
        .filter(|(_, v)| !v.is_null() && v.as_str() != Some(""))
        .map(|(k, v)| {
            let text = match v {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            (k.clone(), Value::String(text))
        })
        .collect();
    Value::Object(out)
}

/// Google's own name for what went wrong, out of `error.details[].errorCode`.
fn error_code(body: &[u8]) -> String {
    serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|v| {
            v["error"]["details"]
                .as_array()?
                .iter()
                .find_map(|d| d["errorCode"].as_str().map(str::to_owned))
        })
        .unwrap_or_default()
}

/// Whether a refusal means this device is gone for good.
///
/// The two unambiguous codes, plus the one case of `INVALID_ARGUMENT` that names the
/// token as the offending field. A bare 400 is deliberately *not* enough: it is equally
/// what a malformed payload earns, and treating that as a dead device would empty the
/// whole register the first time we shipped a bad message.
fn forget_device(status: u16, code: &str, body: &[u8]) -> bool {
    match (status, code) {
        (404, "UNREGISTERED") | (403, "SENDER_ID_MISMATCH") => true,
        (400, "INVALID_ARGUMENT") => blames_the_token(body),
        _ => false,
    }
}

/// Whether Google's complaint is about the registration token rather than the message.
fn blames_the_token(body: &[u8]) -> bool {
    let Ok(v) = serde_json::from_slice::<Value>(body) else { return false };
    let named = v["error"]["details"]
        .as_array()
        .map(|ds| {
            ds.iter().any(|d| {
                d["fieldViolations"]
                    .as_array()
                    .map(|fs| fs.iter().any(|f| f["field"].as_str() == Some("message.token")))
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false);
    let said = v["error"]["message"]
        .as_str()
        .map(|m| m.to_ascii_lowercase().contains("registration token"))
        .unwrap_or(false);
    named || said
}

/// A refusal in one line: Google's message when there is one, else the raw body.
fn describe(body: &[u8]) -> String {
    let text = String::from_utf8_lossy(body);
    serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|v| {
            let msg = v["error"]["message"].as_str()?.to_owned();
            let code = error_code(body);
            Some(if code.is_empty() { msg } else { format!("{code}: {msg}") })
        })
        .unwrap_or_else(|| text.trim().chars().take(200).collect())
}

/// An assertion: `RS256` over the service account's claim to the messaging scope.
pub fn mint_jwt(key: &Key, iat: i64) -> anyhow::Result<String> {
    let header = json!({ "alg": "RS256", "typ": "JWT", "kid": key.private_key_id }).to_string();
    let claims = json!({
        "iss": key.client_email,
        "scope": SCOPE,
        "aud": key.token_uri,
        "iat": iat,
        "exp": iat + 3600,
    })
    .to_string();
    let input = format!("{}.{}", b64url(header.as_bytes()), b64url(claims.as_bytes()));
    let der = pem_to_der(&key.private_key)
        .ok_or_else(|| anyhow::anyhow!("the private key is not PEM"))?;
    let pair = RsaKeyPair::from_pkcs8(&der)
        .map_err(|e| anyhow::anyhow!("the private key could not be read as RSA: {e}"))?;
    let mut sig = vec![0u8; pair.public().modulus_len()];
    pair.sign(&RSA_PKCS1_SHA256, &ring::rand::SystemRandom::new(), input.as_bytes(), &mut sig)
        .map_err(|_| anyhow::anyhow!("signing failed"))?;
    Ok(format!("{input}.{}", b64url(&sig)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(extra: &str) -> String {
        format!(
            r#"{{"type":"service_account","project_id":"agent-router","private_key_id":"abc123",
                 "private_key":"-----BEGIN PRIVATE KEY-----\nAAAA\n-----END PRIVATE KEY-----\n",
                 "client_email":"push@agent-router.iam.gserviceaccount.com",
                 "token_uri":"https://oauth2.googleapis.com/token"{extra}}}"#
        )
    }

    #[test]
    fn a_service_account_is_read_and_anything_else_is_refused_with_a_reason() {
        let key = parse_service_account(&account("")).expect("a whole file is taken");
        assert_eq!(key.project_id, "agent-router");
        assert_eq!(key.private_key_id, "abc123");
        assert!(key.private_key.contains("BEGIN PRIVATE KEY"), "the PEM survives parsing");

        // The console offers a second JSON — google-services.json, for the app — and
        // dropping that one in instead is the mistake worth naming.
        let app = r#"{"project_info":{"project_id":"agent-router"},"client":[]}"#;
        let why = parse_service_account(app).unwrap_err().to_string();
        assert!(why.contains("service_account"), "says what it wanted, got: {why}");

        let missing = r#"{"type":"service_account","project_id":"p"}"#;
        let why = parse_service_account(missing).unwrap_err().to_string();
        assert!(why.contains("client_email"), "names the missing field, got: {why}");
    }

    #[test]
    fn the_token_uri_is_taken_from_the_file_but_need_not_be_there() {
        let mut text = account("");
        text = text.replace(r#","token_uri":"https://oauth2.googleapis.com/token""#, "");
        let key = parse_service_account(&text).expect("token_uri is optional");
        assert_eq!(key.token_uri, "https://oauth2.googleapis.com/token");
    }

    #[test]
    fn data_carries_strings_and_drops_absences() {
        let out = stringify(&json!({
            "title": "agent-router",
            "detail": "",
            "session": Value::Null,
            "queued": 3,
            "dismiss": ["req:a", "req:b"],
            "from": "reserved",
            "google_x": "reserved",
        }));
        let map = out.as_object().unwrap();
        assert_eq!(map["title"], json!("agent-router"));
        assert_eq!(map["queued"], json!("3"), "numbers are rendered, not passed as numbers");
        assert_eq!(map["dismiss"], json!(r#"["req:a","req:b"]"#), "a list crosses as one value");
        assert!(!map.contains_key("detail"), "an empty string is an absence");
        assert!(!map.contains_key("session"), "so is a null");
        assert!(
            !map.contains_key("from") && !map.contains_key("google_x"),
            "FCM's own keys are left out"
        );
        for v in map.values() {
            assert!(v.is_string(), "every value is a string, or FCM refuses the lot");
        }
    }

    #[test]
    fn a_banner_and_a_tidy_up_are_sent_on_different_terms() {
        let banner = message("tok", Delivery::Alert("req:abc"), &json!({ "title": "x" }));
        let android = &banner["message"]["android"];
        assert_eq!(android["priority"], "high");
        assert_eq!(android["collapse_key"], "req:abc", "a repeat replaces the banner it raised");
        assert!(
            android["ttl"].is_null(),
            "a waiting prompt is worth announcing whenever the phone next appears"
        );
        assert_eq!(banner["message"]["token"], "tok");

        let tidy = message("tok", Delivery::Background, &json!({ "dismiss": ["req:abc"] }));
        let android = &tidy["message"]["android"];
        assert_eq!(android["ttl"], "300s", "\"that is answered\" is worth five minutes");
        assert!(
            android["collapse_key"].is_null(),
            "a take-down must not collapse with the banner it chases, or one replaces the other"
        );
        assert!(
            tidy["message"]["notification"].is_null(),
            "nothing here asks Android to draw anything"
        );
    }

    #[test]
    fn only_the_devices_own_refusals_forget_it() {
        let fcm_error = "type.googleapis.com/google.firebase.fcm.v1.FcmError";
        let gone = format!(
            r#"{{"error":{{"code":404,"status":"NOT_FOUND","message":"Requested entity was not found.",
               "details":[{{"@type":"{fcm_error}","errorCode":"UNREGISTERED"}}]}}}}"#
        );
        let gone = gone.as_bytes();
        assert_eq!(error_code(gone), "UNREGISTERED");
        assert!(forget_device(404, "UNREGISTERED", gone), "an uninstalled app is gone for good");

        let theirs = format!(
            r#"{{"error":{{"code":403,"status":"PERMISSION_DENIED","message":"SenderId mismatch",
               "details":[{{"@type":"{fcm_error}","errorCode":"SENDER_ID_MISMATCH"}}]}}}}"#
        );
        assert!(
            forget_device(403, "SENDER_ID_MISMATCH", theirs.as_bytes()),
            "this router will never reach that phone"
        );

        // The distinction the whole register depends on: our bad payload is not a dead phone.
        let ours = br#"{"error":{"code":400,"status":"INVALID_ARGUMENT",
            "message":"Invalid value at 'message.android.ttl'",
            "details":[{"@type":"type.googleapis.com/google.rpc.BadRequest",
                        "fieldViolations":[{"field":"message.android.ttl","description":"bad"}]}]}}"#;
        assert!(
            !forget_device(400, "INVALID_ARGUMENT", ours),
            "a payload bug must never empty the register"
        );

        let bad_token = br#"{"error":{"code":400,"status":"INVALID_ARGUMENT",
            "message":"The registration token is not a valid FCM registration token",
            "details":[{"@type":"type.googleapis.com/google.rpc.BadRequest",
                        "fieldViolations":[{"field":"message.token","description":"bad"}]}]}}"#;
        assert!(
            forget_device(400, "INVALID_ARGUMENT", bad_token),
            "but a 400 naming the token is the phone"
        );

        let transient =
            [(429, "QUOTA_EXCEEDED"), (500, "INTERNAL"), (503, "UNAVAILABLE"), (401, "UNAUTHENTICATED")];
        for t in transient {
            assert!(!forget_device(t.0, t.1, b"{}"), "{t:?} is not a dead device");
        }
    }

    #[test]
    fn a_refusal_reads_as_one_line() {
        let body = br#"{"error":{"code":404,"message":"Requested entity was not found.",
            "details":[{"@type":"type.googleapis.com/google.firebase.fcm.v1.FcmError",
                        "errorCode":"UNREGISTERED"}]}}"#;
        assert_eq!(describe(body), "UNREGISTERED: Requested entity was not found.");
        assert_eq!(describe(b"upstream is having a moment"), "upstream is having a moment");
    }

    #[test]
    fn a_key_that_is_not_a_key_is_refused_with_a_reason() {
        let mut key = parse_service_account(&account("")).unwrap();
        key.private_key = "not a pem at all".into();
        let why = mint_jwt(&key, 0).unwrap_err().to_string();
        assert!(why.contains("PEM"), "says what it wanted, got: {why}");

        // Valid PEM armour, but the bytes inside are not an RSA key.
        key.private_key = "-----BEGIN PRIVATE KEY-----\nAAAA\n-----END PRIVATE KEY-----\n".into();
        let why = mint_jwt(&key, 0).unwrap_err().to_string();
        assert!(why.contains("RSA"), "names the kind of key it wanted, got: {why}");
    }

    /// Signing needs a real RSA key, and `ring` cannot make one; `openssl` can, and is
    /// already how `apns` copes with Apple's half-a-key. Skipped where it is missing.
    #[test]
    fn an_assertion_says_who_it_is_for_and_for_how_long() {
        let Ok(out) = std::process::Command::new("/usr/bin/openssl")
            .args(["genpkey", "-algorithm", "RSA", "-pkeyopt", "rsa_keygen_bits:2048"])
            .output()
        else {
            return;
        };
        if !out.status.success() {
            return;
        }
        let mut key = parse_service_account(&account("")).unwrap();
        key.private_key = String::from_utf8_lossy(&out.stdout).into_owned();

        let jwt = mint_jwt(&key, 1_700_000_000).expect("a real RSA key signs");
        let parts: Vec<&str> = jwt.split('.').collect();
        assert_eq!(parts.len(), 3, "header.claims.signature");

        let un = |s: &str| {
            let v = crate::auth::base64_decode(&s.replace('-', "+").replace('_', "/")).unwrap();
            serde_json::from_slice::<Value>(&v).unwrap()
        };
        let header = un(parts[0]);
        assert_eq!(header["alg"], "RS256");
        assert_eq!(header["kid"], "abc123", "the key id rides along so Google can pick the key");

        let claims = un(parts[1]);
        assert_eq!(claims["iss"], "push@agent-router.iam.gserviceaccount.com");
        assert_eq!(claims["aud"], "https://oauth2.googleapis.com/token");
        assert_eq!(claims["scope"], SCOPE);
        assert_eq!(claims["iat"], 1_700_000_000);
        assert_eq!(
            claims["exp"].as_i64().unwrap() - claims["iat"].as_i64().unwrap(),
            3600,
            "Google refuses an assertion that outlives the hour"
        );
    }
}
