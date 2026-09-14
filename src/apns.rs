//! Reaching a phone through Apple's push service.
//!
//! Token-based authentication: a JWT signed with the team's APNs key (ES256) over
//! HTTP/2. The key is the `.p8` Apple hands out once and never shows again; it is kept
//! in the router's secrets file beside the account credentials, and the token minted
//! from it is reused for fifty minutes, as Apple asks.
//!
//! One wrinkle worth knowing: Apple's `.p8` omits the public half of the key, and
//! `ring` refuses a PKCS#8 document without it. When that happens the key is passed
//! once through the system `openssl`, which fills the public half in.

use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper_util::client::legacy::{connect::HttpConnector, Client};
use hyper_util::rt::TokioExecutor;
use ring::signature::{EcdsaKeyPair, ECDSA_P256_SHA256_FIXED_SIGNING};
use serde_json::Value;

type H2Client = Client<hyper_rustls::HttpsConnector<HttpConnector>, Full<Bytes>>;

/// Apple: a provider token may be reused for up to an hour; refresh before that.
const TOKEN_LIFE_MS: i64 = 50 * 60_000;

#[derive(Clone, Debug)]
pub struct Key {
    pub team_id: String,
    pub key_id: String,
    /// PEM, as in the `.p8` file.
    pub p8: String,
}

#[derive(Debug)]
pub enum SendError {
    /// Apple says this token is gone for good; forget the device.
    Unregistered,
    /// Apple said no, and why.
    Rejected(u16, String),
    /// Apple was never reached.
    Transport(String),
}

impl std::fmt::Display for SendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SendError::Unregistered => write!(f, "device token no longer valid"),
            SendError::Rejected(code, why) => write!(f, "Apple refused ({code}): {why}"),
            SendError::Transport(why) => write!(f, "could not reach Apple: {why}"),
        }
    }
}

pub struct Apns {
    client: H2Client,
    /// (key id, token, minted at ms).
    jwt: Mutex<Option<(String, String, i64)>>,
}

impl Apns {
    pub fn new() -> Self {
        let tls = hyper_rustls::HttpsConnectorBuilder::new()
            .with_webpki_roots()
            .https_only()
            .enable_http2()
            .build();
        let client = Client::builder(TokioExecutor::new()).http2_only(true).build(tls);
        Self { client, jwt: Mutex::new(None) }
    }

    /// Development builds register with the sandbox; a device says which it is.
    pub fn host(environment: &str) -> &'static str {
        if environment == "production" {
            "api.push.apple.com"
        } else {
            "api.sandbox.push.apple.com"
        }
    }

    fn bearer(&self, key: &Key) -> anyhow::Result<String> {
        let now = crate::db::now_ms();
        if let Some((kid, tok, at)) = self.jwt.lock().unwrap().as_ref() {
            if kid == &key.key_id && now - at < TOKEN_LIFE_MS {
                return Ok(tok.clone());
            }
        }
        let tok = mint_jwt(key, now / 1000)?;
        *self.jwt.lock().unwrap() = Some((key.key_id.clone(), tok.clone(), now));
        Ok(tok)
    }

    pub fn forget_token(&self) {
        *self.jwt.lock().unwrap() = None;
    }

    pub async fn send(
        &self,
        key: &Key,
        device: &str,
        environment: &str,
        topic: &str,
        collapse_id: &str,
        payload: &Value,
    ) -> Result<(), SendError> {
        let bearer = self
            .bearer(key)
            .map_err(|e| SendError::Transport(format!("signing key: {e}")))?;
        let uri = format!("https://{}/3/device/{}", Self::host(environment), device);
        let req = hyper::Request::post(uri)
            .header("authorization", format!("bearer {bearer}"))
            .header("apns-topic", topic)
            .header("apns-push-type", "alert")
            .header("apns-priority", "10")
            .header("apns-collapse-id", &collapse_id[..collapse_id.len().min(64)])
            .header("content-type", "application/json")
            .body(Full::new(Bytes::from(payload.to_string())))
            .map_err(|e| SendError::Transport(e.to_string()))?;
        let resp = tokio::time::timeout(Duration::from_secs(15), self.client.request(req))
            .await
            .map_err(|_| SendError::Transport("timed out".into()))?
            .map_err(|e| SendError::Transport(e.to_string()))?;
        let status = resp.status();
        let body = resp
            .into_body()
            .collect()
            .await
            .map(|b| b.to_bytes())
            .unwrap_or_default();
        if status.is_success() {
            return Ok(());
        }
        let reason = serde_json::from_slice::<Value>(&body)
            .ok()
            .and_then(|v| v.get("reason").and_then(Value::as_str).map(str::to_owned))
            .unwrap_or_else(|| String::from_utf8_lossy(&body).trim().to_owned());
        if status.as_u16() == 410
            || matches!(reason.as_str(), "BadDeviceToken" | "Unregistered" | "DeviceTokenNotForTopic")
        {
            return Err(SendError::Unregistered);
        }
        if reason.contains("ProviderToken") {
            self.forget_token();
        }
        Err(SendError::Rejected(status.as_u16(), reason))
    }
}

/// A provider token: `ES256` over `{iss: team, iat: now}`, signed with the `.p8`.
pub fn mint_jwt(key: &Key, iat: i64) -> anyhow::Result<String> {
    let header = serde_json::json!({ "alg": "ES256", "kid": key.key_id }).to_string();
    let claims = serde_json::json!({ "iss": key.team_id, "iat": iat }).to_string();
    let input = format!("{}.{}", b64url(header.as_bytes()), b64url(claims.as_bytes()));
    let rng = ring::rand::SystemRandom::new();
    let pair = key_pair(&key.p8, &rng)?;
    let sig = pair
        .sign(&rng, input.as_bytes())
        .map_err(|_| anyhow::anyhow!("signing failed"))?;
    Ok(format!("{input}.{}", b64url(sig.as_ref())))
}

/// Parse the key; if `ring` wants the public half Apple leaves out, let `openssl`
/// supply it.
fn key_pair(pem: &str, rng: &dyn ring::rand::SecureRandom) -> anyhow::Result<EcdsaKeyPair> {
    let der = pem_to_der(pem).ok_or_else(|| anyhow::anyhow!("that is not a PEM private key"))?;
    if let Ok(pair) = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &der, rng) {
        return Ok(pair);
    }
    let filled = normalize_with_openssl(pem)?;
    let der = pem_to_der(&filled).ok_or_else(|| anyhow::anyhow!("openssl produced no key"))?;
    EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &der, rng)
        .map_err(|e| anyhow::anyhow!("the key could not be read as a P-256 key: {e}"))
}

fn normalize_with_openssl(pem: &str) -> anyhow::Result<String> {
    use std::io::Write;
    let mut child = Command::new("/usr/bin/openssl")
        .args(["pkey"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    child.stdin.take().expect("piped").write_all(pem.as_bytes())?;
    let out = child.wait_with_output()?;
    if !out.status.success() {
        // The first line says what happened; the rest is LibreSSL's call stack.
        let why = String::from_utf8_lossy(&out.stderr);
        anyhow::bail!("openssl could not read the key: {}", why.lines().next().unwrap_or("").trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The base64 between the PEM armour, whatever the line breaks.
pub fn pem_to_der(pem: &str) -> Option<Vec<u8>> {
    let body: String = pem
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("-----"))
        .collect();
    if body.is_empty() {
        return None;
    }
    crate::auth::base64_decode(&body)
}

fn b64url(bytes: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity((bytes.len() + 2) / 3 * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk.iter().fold(0u32, |acc, b| (acc << 8) | u32::from(*b)) << (8 * (3 - chunk.len()));
        for i in 0..=chunk.len() {
            out.push(A[((n >> (18 - 6 * i)) & 63) as usize] as char);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ring::signature::KeyPair;

    fn pem_of(der: &[u8]) -> String {
        // Standard base64 with padding, wrapped the way a .p8 is.
        const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut s = String::new();
        for chunk in der.chunks(3) {
            let n = chunk.iter().fold(0u32, |acc, b| (acc << 8) | u32::from(*b)) << (8 * (3 - chunk.len()));
            for i in 0..4 {
                s.push(if i <= chunk.len() { A[((n >> (18 - 6 * i)) & 63) as usize] as char } else { '=' });
            }
        }
        let lines: Vec<&str> = s.as_bytes().chunks(64).map(|c| std::str::from_utf8(c).unwrap()).collect();
        format!("-----BEGIN PRIVATE KEY-----\n{}\n-----END PRIVATE KEY-----\n", lines.join("\n"))
    }

    #[test]
    fn a_minted_token_verifies_against_the_keys_public_half() {
        let rng = ring::rand::SystemRandom::new();
        let doc = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng).unwrap();
        let key = Key { team_id: "ABCDE12345".into(), key_id: "ABC123DEF4".into(), p8: pem_of(doc.as_ref()) };
        let jwt = mint_jwt(&key, 1_789_200_000).unwrap();
        let parts: Vec<&str> = jwt.split('.').collect();
        assert_eq!(parts.len(), 3);
        let header = crate::auth::base64_decode(&parts[0].replace('-', "+").replace('_', "/")).unwrap();
        assert_eq!(String::from_utf8(header).unwrap(), r#"{"alg":"ES256","kid":"ABC123DEF4"}"#);
        let claims = crate::auth::base64_decode(&parts[1].replace('-', "+").replace('_', "/")).unwrap();
        assert_eq!(String::from_utf8(claims).unwrap(), r#"{"iss":"ABCDE12345","iat":1789200000}"#);
        let sig = crate::auth::base64_decode(&parts[2].replace('-', "+").replace('_', "/")).unwrap();
        assert_eq!(sig.len(), 64, "ES256 is raw r||s");
        let pair = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, doc.as_ref(), &rng).unwrap();
        let public = ring::signature::UnparsedPublicKey::new(&ring::signature::ECDSA_P256_SHA256_FIXED, pair.public_key().as_ref());
        public.verify(format!("{}.{}", parts[0], parts[1]).as_bytes(), &sig).expect("the token must verify");
    }

    #[test]
    fn a_key_that_is_not_a_key_is_refused_with_a_reason() {
        let key = Key { team_id: "T".into(), key_id: "K".into(), p8: "-----BEGIN PRIVATE KEY-----\nAAAA\n-----END PRIVATE KEY-----".into() };
        let err = mint_jwt(&key, 1).unwrap_err().to_string();
        assert!(err.contains("key"), "{err}");
        assert!(pem_to_der("").is_none());
        assert!(pem_to_der("AAAA").is_some(), "bare base64 is accepted too");
        assert!(pem_to_der("not base64 at all").is_none());
    }

    #[test]
    fn hosts_follow_the_build() {
        assert_eq!(Apns::host("development"), "api.sandbox.push.apple.com");
        assert_eq!(Apns::host("production"), "api.push.apple.com");
        assert_eq!(b64url(b"hi"), "aGk");
        assert_eq!(b64url(b"hello"), "aGVsbG8");
    }
}
