//! The router's password.
//!
//! One rule: a request from a peer that is **not on this machine** must carry the
//! password — as `Authorization: Basic` (a browser; the username is ignored) or as
//! `Authorization: Bearer <password>` (an agent: a remote Claude Code sets
//! `ANTHROPIC_AUTH_TOKEN` to it and needs nothing else). Loopback callers are never
//! asked. Claude Code, the wrapper, the menubar app and the local browser all reach the
//! router over loopback, and none of them can do Basic auth without plumbing that would
//! buy nothing on a single-user machine: whoever sits here already holds the Keychain.
//!
//! The bearer form exists because `/v1/*` already carries `Authorization: Bearer` from
//! Claude Code, which `proxy` requires and then swaps for the active account's token.
//! Demanding Basic there would break the swap; accepting the password *as* the bearer
//! keeps a remote agent possible and still closes the hole.
//!
//! With no password set, a non-loopback caller is refused outright. That is the safe
//! default, and it makes the first set loopback-only by construction: nothing off-host
//! can reach `POST /api/auth` until a password exists. It is also the recovery path —
//! loopback may replace a forgotten password without knowing it.
//!
//! This is HTTP, so the password crosses the network in the clear. The bind that makes
//! remote callers possible already assumes a network you trust; this does not change that.

use std::num::NonZeroU32;
use std::path::Path;
use std::time::Duration;

use axum::http::HeaderMap;
use ring::rand::SecureRandom;
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use sha2::{Digest, Sha256};

/// OWASP's 2023 figure for PBKDF2-HMAC-SHA256. About 300 ms here, paid once per
/// browser session thanks to `REMEMBER`, and paid in full by every wrong guess.
const ITERATIONS: u32 = 600_000;
const SALT_LEN: usize = 16;
const HASH_LEN: usize = 32;
pub const MIN_LEN: usize = 8;
const KEY: &str = "password";
/// Credentials already verified, so a dashboard polling eight endpoints every five
/// seconds costs one derivation per session rather than eight per poll.
const REMEMBER: usize = 8;
/// Every wrong guess waits this long while holding the verification lock: roughly two
/// and a half guesses a second, network-wide, whatever the attacker's parallelism.
const WRONG_DELAY: Duration = Duration::from_millis(400);

/// `pbkdf2-sha256$<iterations>$<salt hex>$<hash hex>`. The iteration count travels
/// with the record so it can be raised later without invalidating what is stored.
pub fn hash_password(password: &str) -> String {
    hash_with(password, ITERATIONS)
}

fn hash_with(password: &str, iterations: u32) -> String {
    let mut salt = [0u8; SALT_LEN];
    ring::rand::SystemRandom::new()
        .fill(&mut salt)
        .expect("system randomness is available");
    let mut out = [0u8; HASH_LEN];
    ring::pbkdf2::derive(
        ring::pbkdf2::PBKDF2_HMAC_SHA256,
        NonZeroU32::new(iterations).expect("a positive iteration count"),
        &salt,
        password.as_bytes(),
        &mut out,
    );
    format!("pbkdf2-sha256${iterations}${}${}", hex(&salt), hex(&out))
}

/// Constant-time, via ring. Anything that does not parse as one of our records is a
/// refusal, never a pass.
pub fn verify_password(record: &str, password: &str) -> bool {
    let mut parts = record.split('$');
    let (Some("pbkdf2-sha256"), Some(iters), Some(salt), Some(hash), None) =
        (parts.next(), parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return false;
    };
    let Some(iters) = iters.parse::<u32>().ok().and_then(NonZeroU32::new) else {
        return false;
    };
    let (Some(salt), Some(hash)) = (unhex(salt), unhex(hash)) else {
        return false;
    };
    ring::pbkdf2::verify(
        ring::pbkdf2::PBKDF2_HMAC_SHA256,
        iters,
        &salt,
        password.as_bytes(),
        &hash,
    )
    .is_ok()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

/// Standard alphabet, padding optional. Small enough not to be worth a crate.
pub(crate) fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => break,
            _ => return None,
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

/// The password a request presents, whichever way it presents it.
pub fn presented(headers: &HeaderMap) -> Option<String> {
    let value = headers.get("authorization")?.to_str().ok()?.trim();
    let (scheme, rest) = value.split_once(' ')?;
    let rest = rest.trim();
    if scheme.eq_ignore_ascii_case("basic") {
        let text = String::from_utf8(base64_decode(rest)?).ok()?;
        // `user-id ":" password` — the user-id is whatever the browser was told to
        // send, and only the password counts.
        let (_, password) = text.split_once(':')?;
        Some(password.to_string())
    } else if scheme.eq_ignore_ascii_case("bearer") {
        (!rest.is_empty()).then(|| rest.to_string())
    } else {
        None
    }
}

/// Fixed-length compare with no early exit, so the time taken says nothing about
/// where two digests first differ.
fn ct_eq(a: &[u8; 32], b: &[u8; 32]) -> bool {
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

struct Record {
    hash: String,
    updated_at: i64,
}

pub enum Verdict {
    Ok,
    /// Nothing is set, so there is nothing a remote caller could present.
    NoPassword,
    Missing,
    Wrong,
}

pub struct Auth {
    record: tokio::sync::RwLock<Option<Record>>,
    /// SHA-256 of `Authorization` values that have verified — never the values
    /// themselves, so the cache holds no password material.
    accepted: std::sync::Mutex<Vec<[u8; 32]>>,
    /// One derivation at a time. A flood of guesses costs one core, not the proxy's.
    verifying: tokio::sync::Mutex<()>,
}

impl Auth {
    pub fn load(db_path: &Path) -> anyhow::Result<Self> {
        let conn = Connection::open_with_flags(
            db_path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
        )?;
        let record = conn
            .query_row(
                "SELECT value, updated_at FROM settings WHERE key = ?1",
                [KEY],
                |r| Ok(Record { hash: r.get(0)?, updated_at: r.get(1)? }),
            )
            .optional()?;
        Ok(Self::with(record))
    }

    fn with(record: Option<Record>) -> Self {
        Self {
            record: tokio::sync::RwLock::new(record),
            accepted: std::sync::Mutex::new(Vec::new()),
            verifying: tokio::sync::Mutex::new(()),
        }
    }

    pub async fn configured(&self) -> bool {
        self.record.read().await.is_some()
    }

    pub async fn updated_at(&self) -> Option<i64> {
        self.record.read().await.as_ref().map(|r| r.updated_at)
    }

    pub async fn check(&self, headers: &HeaderMap) -> Verdict {
        let Some(record) = self.record.read().await.as_ref().map(|r| r.hash.clone()) else {
            return Verdict::NoPassword;
        };
        let Some(raw) = headers.get("authorization").and_then(|v| v.to_str().ok()) else {
            return Verdict::Missing;
        };
        let Some(password) = presented(headers) else {
            return Verdict::Missing;
        };
        let digest: [u8; 32] = Sha256::digest(raw.as_bytes()).into();
        if self.remembered(&digest) {
            return Verdict::Ok;
        }
        let _one_at_a_time = self.verifying.lock().await;
        // A sibling request — the dashboard fires eight at once — may have verified
        // this very credential while we waited for the lock.
        if self.remembered(&digest) {
            return Verdict::Ok;
        }
        let ok = tokio::task::spawn_blocking(move || verify_password(&record, &password))
            .await
            .unwrap_or(false);
        if ok {
            self.remember(digest);
            Verdict::Ok
        } else {
            tokio::time::sleep(WRONG_DELAY).await;
            Verdict::Wrong
        }
    }

    fn remembered(&self, digest: &[u8; 32]) -> bool {
        self.accepted.lock().unwrap().iter().any(|a| ct_eq(a, digest))
    }

    fn remember(&self, digest: [u8; 32]) {
        let mut v = self.accepted.lock().unwrap();
        if v.len() >= REMEMBER {
            v.remove(0);
        }
        v.push(digest);
    }

    pub async fn set(&self, db_path: &Path, password: &str) -> anyhow::Result<()> {
        let owned = password.to_string();
        let hash = tokio::task::spawn_blocking(move || hash_password(&owned)).await?;
        let now = crate::db::now_ms();
        write(db_path, Some((&hash, now)))?;
        *self.record.write().await = Some(Record { hash, updated_at: now });
        self.accepted.lock().unwrap().clear();
        Ok(())
    }

    pub async fn clear(&self, db_path: &Path) -> anyhow::Result<()> {
        write(db_path, None)?;
        *self.record.write().await = None;
        self.accepted.lock().unwrap().clear();
        Ok(())
    }
}

fn write(db_path: &Path, value: Option<(&str, i64)>) -> anyhow::Result<()> {
    let conn = Connection::open(db_path)?;
    match value {
        Some((hash, at)) => conn.execute(
            "INSERT INTO settings (key, value, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
            rusqlite::params![KEY, hash, at],
        )?,
        None => conn.execute("DELETE FROM settings WHERE key = ?1", [KEY])?,
    };
    Ok(())
}

pub mod api {
    use std::net::SocketAddr;
    use std::sync::Arc;

    use axum::extract::{ConnectInfo, State};
    use axum::Json;
    use serde::Deserialize;
    use serde_json::{json, Value};

    use crate::proxy::AppState;

    #[derive(Deserialize)]
    pub struct SetReq {
        #[serde(default)]
        pub password: String,
        /// Remove the password instead of setting one. Remote callers go back to
        /// being refused until a new one is set from this machine.
        #[serde(default)]
        pub clear: bool,
    }

    /// Reachable by loopback, or by a remote caller that already presented the
    /// password — `guard` enforces that, so there is nothing to check here.
    pub async fn status(
        State(state): State<Arc<AppState>>,
        ConnectInfo(peer): ConnectInfo<SocketAddr>,
    ) -> Json<Value> {
        let mut body = json!({
            "configured":    state.auth.configured().await,
            "updated_at":    state.auth.updated_at().await,
            "from_loopback": peer.ip().is_loopback(),
        });
        // `remote_bind` and the rest of the socket's story come from `network`, which
        // owns the answer — it can change while the router runs. Both phone apps read
        // it from here, so here is where it stays.
        body.as_object_mut()
            .expect("just built as an object")
            .extend(crate::network::report(&state));
        Json(body)
    }

    pub async fn set(State(state): State<Arc<AppState>>, Json(req): Json<SetReq>) -> Json<Value> {
        if req.clear {
            return match state.auth.clear(&state.config.db_path).await {
                Ok(()) => Json(json!({ "ok": true, "configured": false })),
                Err(e) => Json(json!({ "error": e.to_string() })),
            };
        }
        // Not trimmed: a password is whatever was typed, spaces included.
        if req.password.chars().count() < super::MIN_LEN {
            return Json(json!({ "error": format!("use at least {} characters", super::MIN_LEN) }));
        }
        match state.auth.set(&state.config.db_path, &req.password).await {
            Ok(()) => Json(json!({ "ok": true, "configured": true })),
            Err(e) => Json(json!({ "error": e.to_string() })),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    /// Fast records for tests; the count travels with the record, so verify does not care.
    const QUICK: u32 = 1_000;

    fn base64_encode(bytes: &[u8]) -> String {
        const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for chunk in bytes.chunks(3) {
            let n = chunk.iter().fold(0u32, |acc, b| (acc << 8) | u32::from(*b)) << (8 * (3 - chunk.len()));
            for i in 0..4 {
                if i <= chunk.len() {
                    out.push(A[((n >> (18 - 6 * i)) & 63) as usize] as char);
                } else {
                    out.push('=');
                }
            }
        }
        out
    }

    fn headers(auth: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert("authorization", HeaderValue::from_str(auth).unwrap());
        h
    }

    #[test]
    fn a_password_round_trips_and_the_wrong_one_does_not() {
        let rec = hash_with("correct horse battery", QUICK);
        assert!(rec.starts_with("pbkdf2-sha256$1000$"));
        assert!(verify_password(&rec, "correct horse battery"));
        assert!(!verify_password(&rec, "correct horse batter"));
        assert!(!verify_password(&rec, ""));
        // Two hashes of one password differ: the salt is doing its job.
        assert_ne!(rec, hash_with("correct horse battery", QUICK));
    }

    #[test]
    fn a_tampered_or_foreign_record_fails_closed() {
        let rec = hash_with("hunter22", QUICK);
        let flipped = {
            let mut s = rec.clone();
            let last = s.pop().unwrap();
            s.push(if last == '0' { '1' } else { '0' });
            s
        };
        for bad in [
            flipped.as_str(),
            "",
            "plaintext",
            "md5$1$00$00",
            "pbkdf2-sha256$0$00$00",
            "pbkdf2-sha256$1000$zz$zz",
            "pbkdf2-sha256$1000$00$00$extra",
        ] {
            assert!(!verify_password(bad, "hunter22"), "{bad:?} must not verify");
        }
    }

    #[test]
    fn basic_takes_the_password_and_ignores_the_user() {
        let b = |s: &str| format!("Basic {}", base64_encode(s.as_bytes()));
        assert_eq!(presented(&headers(&b("anyone:hunter22"))).as_deref(), Some("hunter22"));
        assert_eq!(presented(&headers(&b(":hunter22"))).as_deref(), Some("hunter22"));
        // A colon in the password belongs to the password.
        assert_eq!(presented(&headers(&b("u:pa:ss"))).as_deref(), Some("pa:ss"));
        // No colon is no password, not a password with an empty user.
        assert_eq!(presented(&headers(&b("nocolon"))), None);
        assert_eq!(presented(&headers("Basic !!notbase64!!")), None);
        assert_eq!(presented(&headers("basic ")), None);
        assert_eq!(presented(&HeaderMap::new()), None);
    }

    #[test]
    fn bearer_is_the_password_itself() {
        assert_eq!(presented(&headers("Bearer hunter22")).as_deref(), Some("hunter22"));
        assert_eq!(presented(&headers("bearer  hunter22 ")).as_deref(), Some("hunter22"));
        assert_eq!(presented(&headers("Bearer ")), None);
        assert_eq!(presented(&headers("Digest hunter22")), None);
    }

    #[test]
    fn base64_decodes_with_and_without_padding() {
        assert_eq!(base64_decode("aHVudGVyMjI=").unwrap(), b"hunter22");
        assert_eq!(base64_decode("aHVudGVyMjI").unwrap(), b"hunter22");
        assert_eq!(base64_decode("").unwrap(), b"");
        assert_eq!(base64_decode("YQ==").unwrap(), b"a");
        assert_eq!(base64_decode("YWI=").unwrap(), b"ab");
        assert_eq!(base64_decode("YWJj").unwrap(), b"abc");
        assert!(base64_decode("Y W==").is_none());
    }

    #[test]
    fn digests_compare_without_a_short_circuit() {
        let a = [7u8; 32];
        let mut b = a;
        assert!(ct_eq(&a, &b));
        b[31] ^= 1;
        assert!(!ct_eq(&a, &b));
    }

    #[tokio::test]
    async fn the_verdicts_line_up_with_the_rule() {
        let none = Auth::with(None);
        assert!(matches!(none.check(&headers("Bearer anything")).await, Verdict::NoPassword));

        let auth = Auth::with(Some(Record { hash: hash_with("hunter22", QUICK), updated_at: 1 }));
        assert!(matches!(auth.check(&HeaderMap::new()).await, Verdict::Missing));
        assert!(matches!(auth.check(&headers("Digest x")).await, Verdict::Missing));
        let ok = format!("Basic {}", base64_encode(b"phone:hunter22"));
        assert!(matches!(auth.check(&headers(&ok)).await, Verdict::Ok));
        // Remembered now: the same header verifies without another derivation.
        assert_eq!(auth.accepted.lock().unwrap().len(), 1);
        assert!(matches!(auth.check(&headers(&ok)).await, Verdict::Ok));
        assert_eq!(auth.accepted.lock().unwrap().len(), 1);
        assert!(matches!(auth.check(&headers("Bearer hunter22")).await, Verdict::Ok));
        assert!(matches!(auth.check(&headers("Bearer hunter23")).await, Verdict::Wrong));
        // A wrong guess is never remembered.
        assert_eq!(auth.accepted.lock().unwrap().len(), 2);
    }
}
