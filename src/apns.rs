//! Reaching a phone through Apple's push service.
//!
//! Token-based authentication: a JWT signed with the team's APNs key (ES256) over
//! HTTP/2. The key is the `.p8` Apple hands out once and never shows again; it is kept
//! in the router's secrets file beside the account credentials, and the token minted
//! from it is reused for fifty minutes, as Apple asks.
//!
//! One wrinkle worth knowing: Apple's `.p8` omits the public half of the key, and
//! `ring` refuses a PKCS#8 document without it. When that happens the public point is
//! computed from the private one, in-process (`p256`). It used to be the system
//! `openssl` that filled it in, which a hosted router's image does not have.

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

/// How a send failed. Shared with `fcm`, which fails in exactly these three ways, so the
/// wording names no vendor: wherever one of these is shown the device's `platform` is
/// alongside it, and "Apple refused" over a Google error would be worse than saying less.
#[derive(Debug)]
pub enum SendError {
    /// The service says this token is gone for good; forget the device.
    Unregistered,
    /// The service said no, and why.
    Rejected(u16, String),
    /// The service was never reached.
    Transport(String),
}

impl std::fmt::Display for SendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SendError::Unregistered => write!(f, "device token no longer valid"),
            SendError::Rejected(code, why) => write!(f, "the push service refused ({code}): {why}"),
            SendError::Transport(why) => write!(f, "could not reach the push service: {why}"),
        }
    }
}

/// What Apple should do with a message: show it to a person, or wake the app quietly.
#[derive(Clone, Copy, Debug)]
pub enum Delivery<'a> {
    /// A banner, replacing any earlier one carrying the same collapse id.
    Alert(&'a str),
    /// Nothing is shown; the app is woken to tidy up after itself. Apple throttles
    /// these, withholds them while the app is force-quit or Low Power Mode is on, and
    /// promises nothing about when one arrives — so nothing may *depend* on one.
    Background,
}

impl Delivery<'_> {
    fn push_type(&self) -> &'static str {
        match self {
            Delivery::Alert(_) => "alert",
            Delivery::Background => "background",
        }
    }

    /// Apple rejects a background push sent at 10.
    fn priority(&self) -> &'static str {
        match self {
            Delivery::Alert(_) => "10",
            Delivery::Background => "5",
        }
    }

    /// Collapsing is for banners. A background push must not carry the id of the banner
    /// it is chasing: Apple would treat the two as one message, and whichever arrived
    /// second would replace — not follow — the first.
    pub(crate) fn collapse_id(&self) -> Option<&str> {
        match self {
            Delivery::Alert(id) => Some(&id[..id.len().min(64)]),
            Delivery::Background => None,
        }
    }

    /// How long the message is still worth delivering. A prompt is worth announcing
    /// whenever the phone next appears; "that prompt is answered" is worth five minutes.
    pub(crate) fn life_secs(&self) -> Option<i64> {
        match self {
            Delivery::Alert(_) => None,
            Delivery::Background => Some(5 * 60),
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
        delivery: Delivery<'_>,
        payload: &Value,
    ) -> Result<(), SendError> {
        let bearer = self
            .bearer(key)
            .map_err(|e| SendError::Transport(format!("signing key: {e}")))?;
        let uri = format!("https://{}/3/device/{}", Self::host(environment), device);
        let mut req = hyper::Request::post(uri)
            .header("authorization", format!("bearer {bearer}"))
            .header("apns-topic", topic)
            .header("apns-push-type", delivery.push_type())
            .header("apns-priority", delivery.priority())
            .header("content-type", "application/json");
        if let Some(id) = delivery.collapse_id() {
            req = req.header("apns-collapse-id", id);
        }
        if let Some(secs) = delivery.life_secs() {
            // A message worth nothing later should not wake a phone that was off.
            req = req.header("apns-expiration", (crate::db::now_ms() / 1000 + secs).to_string());
        }
        let req = req
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

/// Parse the key; if `ring` wants the public half Apple leaves out, compute it from the
/// private half.
fn key_pair(pem: &str, rng: &dyn ring::rand::SecureRandom) -> anyhow::Result<EcdsaKeyPair> {
    use p256::elliptic_curve::sec1::ToSec1Point;

    let der = pem_to_der(pem).ok_or_else(|| anyhow::anyhow!("that is not a PEM private key"))?;
    if let Ok(pair) = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &der, rng) {
        return Ok(pair);
    }
    let unreadable = || anyhow::anyhow!("the key could not be read as a P-256 key");
    let scalar = private_scalar(&der).ok_or_else(unreadable)?;
    let secret = p256::SecretKey::from_slice(scalar).map_err(|_| unreadable())?;
    let public = secret.public_key().to_sec1_point(false);
    EcdsaKeyPair::from_private_key_and_public_key(&ECDSA_P256_SHA256_FIXED_SIGNING, scalar, public.as_bytes(), rng)
        .map_err(|e| anyhow::anyhow!("the key could not be read as a P-256 key: {e}"))
}

/// The private key inside a PKCS#8 P-256 document: the `ECPrivateKey` it wraps opens
/// with `INTEGER 1` and then the key as a 32-byte `OCTET STRING` (RFC 5915 fixes the
/// length), so it is the 32 bytes after DER `02 01 01 04 20`. Nothing earlier in the
/// document — a version and two OIDs — can contain that sequence.
fn private_scalar(der: &[u8]) -> Option<&[u8]> {
    const OPENS: [u8; 5] = [0x02, 0x01, 0x01, 0x04, 0x20];
    let at = der.windows(OPENS.len()).position(|w| w == OPENS)? + OPENS.len();
    der.get(at..at + 32)
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

pub(crate) fn b64url(bytes: &[u8]) -> String {
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

    /// Apple's shape: a PKCS#8 document whose `ECPrivateKey` carries the private key and
    /// nothing else — no public half, which is what `ring` refuses on its own.
    fn without_public_half(scalar: &[u8]) -> Vec<u8> {
        let mut der = vec![
            0x30, 0x41, 0x02, 0x01, 0x00, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01,
            0x06, 0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07, 0x04, 0x27, 0x30, 0x25, 0x02, 0x01,
            0x01, 0x04, 0x20,
        ];
        der.extend_from_slice(scalar);
        der
    }

    #[test]
    fn a_key_without_its_public_half_still_signs() {
        let rng = ring::rand::SystemRandom::new();
        let doc = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng).unwrap();
        let full = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, doc.as_ref(), &rng).unwrap();
        let stripped = without_public_half(private_scalar(doc.as_ref()).unwrap());
        assert!(
            EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &stripped, &rng).is_err(),
            "ring refuses it as it stands, which is the case this covers"
        );
        let filled = key_pair(&pem_of(&stripped), &rng).unwrap();
        assert_eq!(filled.public_key().as_ref(), full.public_key().as_ref(), "the same key, made whole");

        let key = Key { team_id: "ABCDE12345".into(), key_id: "ABC123DEF4".into(), p8: pem_of(&stripped) };
        let jwt = mint_jwt(&key, 1_789_200_000).unwrap();
        let (input, sig) = jwt.rsplit_once('.').unwrap();
        let sig = crate::auth::base64_decode(&sig.replace('-', "+").replace('_', "/")).unwrap();
        let public = ring::signature::UnparsedPublicKey::new(&ring::signature::ECDSA_P256_SHA256_FIXED, full.public_key().as_ref());
        public.verify(input.as_bytes(), &sig).expect("the token must verify against the original key");
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
    fn a_banner_and_a_tidy_up_are_sent_on_different_terms() {
        let alert = Delivery::Alert("req:abc");
        assert_eq!((alert.push_type(), alert.priority()), ("alert", "10"));
        assert_eq!(alert.collapse_id(), Some("req:abc"));
        assert_eq!(alert.life_secs(), None, "a waiting prompt keeps waiting");

        let quiet = Delivery::Background;
        assert_eq!((quiet.push_type(), quiet.priority()), ("background", "5"));
        assert_eq!(quiet.collapse_id(), None, "it must not replace the banner it chases");
        assert_eq!(quiet.life_secs(), Some(300));

        let long = format!("req:{}", "x".repeat(200));
        assert_eq!(Delivery::Alert(&long).collapse_id().unwrap().len(), 64, "Apple's limit");
    }

    #[test]
    fn hosts_follow_the_build() {
        assert_eq!(Apns::host("development"), "api.sandbox.push.apple.com");
        assert_eq!(Apns::host("production"), "api.push.apple.com");
        assert_eq!(b64url(b"hi"), "aGk");
        assert_eq!(b64url(b"hello"), "aGVsbG8");
    }
}
