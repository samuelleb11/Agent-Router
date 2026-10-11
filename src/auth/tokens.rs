//! Individual router access tokens. These never contain an upstream provider key.
//!
//! A random 256-bit secret is shown only in the creation response. The store contains
//! its SHA-256 digest and public metadata. Authentication reads the store afresh, so
//! revocation and expiration do not depend on a process-local credential cache.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Path as Params, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use ring::rand::SecureRandom;
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::proxy::AppState;
use crate::refusal::{failed, refuse, JsonBody};

pub const PREFIX: &str = "arr_";
const SECRET_BYTES: usize = 32;
const LAST_USED_INTERVAL_MS: i64 = 60_000;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    Client,
    Machine,
    Admin,
}

impl Scope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Client => "client",
            Self::Machine => "machine",
            Self::Admin => "admin",
        }
    }

    fn parse(value: &str) -> rusqlite::Result<Self> {
        match value {
            "client" => Ok(Self::Client),
            "machine" => Ok(Self::Machine),
            "admin" => Ok(Self::Admin),
            _ => Err(rusqlite::Error::InvalidQuery),
        }
    }
}

/// Safe to display and serialize: neither the token value nor its digest is present.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct Token {
    pub id: String,
    pub name: String,
    pub prefix: String,
    pub scope: Scope,
    pub created_at: i64,
    pub last_used_at: Option<i64>,
    pub expires_at: Option<i64>,
    pub revoked_at: Option<i64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Invalid {
    Malformed,
    Unknown,
    Expired,
    Revoked,
    /// An unavailable store fails closed, rather than accepting a cached credential.
    Unavailable,
}

/// A dedicated token header always identifies a token attempt, even when malformed.
/// Otherwise only our prefix identifies one, keeping native provider credentials and
/// the legacy dashboard password separate. Basic credentials work for legacy clients
/// that have a single password field, while Bearer and x-api-key are the usual forms.
pub fn presented(headers: &HeaderMap) -> Option<String> {
    if let Some(value) = headers.get("x-agent-router-token") {
        return Some(value.to_str().unwrap_or_default().trim().to_string());
    }
    let value = super::presented(headers)?;
    value.starts_with(PREFIX).then_some(value)
}

fn digest(token: &str) -> String {
    super::hex(&Sha256::digest(token.as_bytes()))
}

fn random_hex(bytes: usize) -> anyhow::Result<String> {
    let mut value = vec![0; bytes];
    ring::rand::SystemRandom::new()
        .fill(&mut value)
        .map_err(|_| {
            anyhow::anyhow!("the system random source could not generate an access token")
        })?;
    Ok(super::hex(&value))
}

fn reader(db: &Path) -> anyhow::Result<Connection> {
    let conn = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    conn.busy_timeout(Duration::from_secs(5))?;
    Ok(conn)
}

fn writer(db: &Path) -> anyhow::Result<Connection> {
    let conn = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    conn.busy_timeout(Duration::from_secs(5))?;
    Ok(conn)
}

fn token_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Token> {
    let scope: String = row.get(3)?;
    Ok(Token {
        id: row.get(0)?,
        name: row.get(1)?,
        prefix: row.get(2)?,
        scope: Scope::parse(&scope)?,
        created_at: row.get(4)?,
        last_used_at: row.get(5)?,
        expires_at: row.get(6)?,
        revoked_at: row.get(7)?,
    })
}

const PUBLIC_COLUMNS: &str =
    "id, name, prefix, scope, created_at, last_used_at, expires_at, revoked_at";

fn lookup(db: &Path, hash: &str, now: i64) -> Result<Token, Invalid> {
    let conn = reader(db).map_err(|_| Invalid::Unavailable)?;
    let mut token = conn
        .query_row(
            &format!("SELECT {PUBLIC_COLUMNS} FROM auth_tokens WHERE hash = ?1"),
            [hash],
            token_row,
        )
        .optional()
        .map_err(|_| Invalid::Unavailable)?
        .ok_or(Invalid::Unknown)?;
    if token.revoked_at.is_some() {
        return Err(Invalid::Revoked);
    }
    if token.expires_at.is_some_and(|at| at <= now) {
        return Err(Invalid::Expired);
    }
    // Usage metadata is best-effort, and at most one write per token per minute.
    // Failure to update it does not replace the fresh authentication decision.
    if token
        .last_used_at
        .is_none_or(|at| at <= now - LAST_USED_INTERVAL_MS)
    {
        if let Ok(conn) = writer(db) {
            if conn
                .execute(
                    "UPDATE auth_tokens SET last_used_at = ?2 WHERE id = ?1
                     AND revoked_at IS NULL AND (expires_at IS NULL OR expires_at > ?2)
                     AND (last_used_at IS NULL OR last_used_at <= ?3)",
                    params![token.id, now, now - LAST_USED_INTERVAL_MS],
                )
                .is_ok_and(|rows| rows == 1)
            {
                token.last_used_at = Some(now);
            }
        }
    }
    Ok(token)
}

pub async fn check(db: &Path, credential: &str) -> Result<Token, Invalid> {
    let Some(secret) = credential.strip_prefix(PREFIX) else {
        return Err(Invalid::Malformed);
    };
    if secret.len() != SECRET_BYTES * 2 || !secret.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(Invalid::Malformed);
    }
    let (db, hash) = (db.to_path_buf(), digest(credential));
    tokio::task::spawn_blocking(move || lookup(&db, &hash, crate::db::now_ms()))
        .await
        .map_err(|_| Invalid::Unavailable)?
}

pub async fn active_count(db: &Path) -> anyhow::Result<usize> {
    let db = db.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let conn = reader(&db)?;
        Ok(conn.query_row(
            "SELECT count(*) FROM auth_tokens WHERE revoked_at IS NULL AND (expires_at IS NULL OR expires_at > ?1)",
            [crate::db::now_ms()],
            |row| row.get(0),
        )?)
    })
    .await?
}

/// Used by established machine connections, so a revoked or expired credential
/// stops working without waiting for the next reconnect. This never records usage.
pub async fn is_active(db: &Path, id: &str) -> anyhow::Result<bool> {
    let (db, id) = (db.to_path_buf(), id.to_string());
    tokio::task::spawn_blocking(move || {
        let conn = reader(&db)?;
        Ok(conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM auth_tokens WHERE id = ?1 AND revoked_at IS NULL
             AND (expires_at IS NULL OR expires_at > ?2))",
            params![id, crate::db::now_ms()],
            |row| row.get(0),
        )?)
    })
    .await?
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TokenIn {
    pub name: String,
    pub scope: Scope,
    pub expires_at: Option<i64>,
}

fn create_stored(db: &Path, body: TokenIn, now: i64) -> anyhow::Result<(Token, String)> {
    let secret = random_hex(SECRET_BYTES)?;
    let value = format!("{PREFIX}{secret}");
    let token = Token {
        id: random_hex(16)?,
        name: body.name,
        prefix: format!("{PREFIX}{}", &secret[..12]),
        scope: body.scope,
        created_at: now,
        last_used_at: None,
        expires_at: body.expires_at,
        revoked_at: None,
    };
    writer(db)?.execute(
        "INSERT INTO auth_tokens (id, name, prefix, hash, scope, created_at, expires_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            token.id,
            token.name,
            token.prefix,
            digest(&value),
            token.scope.as_str(),
            now,
            token.expires_at
        ],
    )?;
    Ok((token, value))
}

fn no_store(mut response: Response) -> Response {
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

/// The secret is returned only here. It is never available from the list API.
pub async fn create(
    State(state): State<Arc<AppState>>,
    JsonBody(mut body): JsonBody<TokenIn>,
) -> Response {
    body.name = body.name.trim().to_string();
    if body.name.is_empty()
        || body.name.chars().count() > 64
        || body.name.chars().any(char::is_control)
    {
        return refuse(
            StatusCode::UNPROCESSABLE_ENTITY,
            "use a token name with 1 to 64 characters and no control characters",
        );
    }
    let now = crate::db::now_ms();
    if body.expires_at.is_some_and(|at| at <= now) {
        return refuse(
            StatusCode::UNPROCESSABLE_ENTITY,
            "the token expiration must be in the future",
        );
    }
    let db = state.config.db_path.clone();
    match tokio::task::spawn_blocking(move || create_stored(&db, body, now)).await {
        Ok(Ok((token, value))) => {
            let mut body = serde_json::to_value(token).expect("public token metadata serializes");
            body["ok"] = json!(true);
            body["token"] = json!(value);
            no_store((StatusCode::CREATED, Json(body)).into_response())
        }
        Ok(Err(error)) => failed(error),
        Err(error) => failed(error),
    }
}

fn list_stored(db: &Path) -> anyhow::Result<Vec<Token>> {
    let conn = reader(db)?;
    let mut stmt = conn.prepare(&format!(
        "SELECT {PUBLIC_COLUMNS} FROM auth_tokens ORDER BY created_at DESC, id"
    ))?;
    let rows = stmt.query_map([], token_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub async fn list(State(state): State<Arc<AppState>>) -> Response {
    let db = state.config.db_path.clone();
    match tokio::task::spawn_blocking(move || list_stored(&db)).await {
        Ok(Ok(tokens)) => no_store(Json(json!({ "tokens": tokens })).into_response()),
        Ok(Err(error)) => failed(error),
        Err(error) => failed(error),
    }
}

fn revoke_stored(db: PathBuf, id: String) -> anyhow::Result<usize> {
    Ok(writer(&db)?.execute(
        "UPDATE auth_tokens SET revoked_at = coalesce(revoked_at, ?2) WHERE id = ?1",
        params![id, crate::db::now_ms()],
    )?)
}

pub async fn revoke(State(state): State<Arc<AppState>>, Params(id): Params<String>) -> Response {
    let db = state.config.db_path.clone();
    match tokio::task::spawn_blocking(move || revoke_stored(db, id)).await {
        Ok(Ok(1)) => no_store(Json(json!({ "ok": true })).into_response()),
        Ok(Ok(_)) => refuse(StatusCode::NOT_FOUND, "that access token does not exist"),
        Ok(Err(error)) => failed(error),
        Err(error) => failed(error),
    }
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;

    use axum::body::Body;
    use axum::extract::ConnectInfo;
    use axum::http::HeaderValue;
    use axum::http::Request;
    use tower::ServiceExt;

    use super::*;
    use crate::api_refusals::Lab;

    fn make(db: &Path, name: &str, scope: Scope, expires_at: Option<i64>) -> (Token, String) {
        create_stored(
            db,
            TokenIn {
                name: name.into(),
                scope,
                expires_at,
            },
            crate::db::now_ms(),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn token_secrets_are_random_and_persist_only_as_digests() {
        let lab = Lab::local("auth-token-hash");
        let (token, value) = make(
            &lab.state.config.db_path,
            "Claude Code",
            Scope::Client,
            None,
        );
        let (_, other) = make(&lab.state.config.db_path, "Codex", Scope::Client, None);
        assert_ne!(value, other);
        assert_eq!(value.len(), PREFIX.len() + 64);
        let conn = reader(&lab.state.config.db_path).unwrap();
        let kept: String = conn
            .query_row(
                "SELECT hash FROM auth_tokens WHERE id = ?1",
                [&token.id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(kept, digest(&value));
        assert!(!kept.contains(&value));
        let metadata =
            serde_json::to_string(&list_stored(&lab.state.config.db_path).unwrap()).unwrap();
        assert!(
            !metadata.contains(&value) && !metadata.contains(&other) && !metadata.contains(&kept)
        );
        let checked = check(&lab.state.config.db_path, &value).await.unwrap();
        assert_eq!(checked.scope, Scope::Client);
        assert!(checked.last_used_at.is_some());
    }

    #[tokio::test]
    async fn revocation_expiration_and_restart_are_not_cached() {
        let lab = Lab::local("auth-token-revoke");
        let db = &lab.state.config.db_path;
        let (token, value) = make(db, "Machine", Scope::Machine, None);
        assert!(check(db, &value).await.is_ok());
        assert!(check(db, &value).await.is_ok());
        revoke_stored(db.clone(), token.id).unwrap();
        assert_eq!(check(db, &value).await.unwrap_err(), Invalid::Revoked);
        let (expires, expiring) = make(
            db,
            "Phone",
            Scope::Admin,
            Some(crate::db::now_ms() + 60_000),
        );
        assert!(check(db, &expiring).await.is_ok());
        writer(db)
            .unwrap()
            .execute(
                "UPDATE auth_tokens SET expires_at = ?2 WHERE id = ?1",
                params![expires.id, crate::db::now_ms() - 1],
            )
            .unwrap();
        assert_eq!(check(db, &expiring).await.unwrap_err(), Invalid::Expired);
        let (_, kept) = make(db, "Dashboard", Scope::Admin, None);
        let restarted = lab.restarted().await;
        assert_eq!(
            check(&restarted.state.config.db_path, &kept)
                .await
                .unwrap()
                .scope,
            Scope::Admin
        );
        assert_eq!(
            check(&restarted.state.config.db_path, &value)
                .await
                .unwrap_err(),
            Invalid::Revoked
        );
        assert_eq!(active_count(db).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn active_connection_checks_honor_revocation_and_expiration_without_recording_use() {
        let lab = Lab::local("auth-token-active");
        let db = &lab.state.config.db_path;
        let (token, _) = make(db, "Machine", Scope::Machine, None);
        assert!(is_active(db, &token.id).await.unwrap());
        assert!(list_stored(db).unwrap()[0].last_used_at.is_none());
        assert!(!is_active(db, "missing").await.unwrap());
        writer(db)
            .unwrap()
            .execute(
                "UPDATE auth_tokens SET expires_at = ?2 WHERE id = ?1",
                params![token.id, crate::db::now_ms()],
            )
            .unwrap();
        assert!(!is_active(db, &token.id).await.unwrap());
        writer(db)
            .unwrap()
            .execute(
                "UPDATE auth_tokens SET expires_at = NULL WHERE id = ?1",
                [&token.id],
            )
            .unwrap();
        assert!(is_active(db, &token.id).await.unwrap());
        revoke_stored(db.clone(), token.id.clone()).unwrap();
        assert!(!is_active(db, &token.id).await.unwrap());
    }

    #[tokio::test]
    async fn unknown_malformed_and_unavailable_tokens_fail_closed() {
        let lab = Lab::local("auth-token-invalid");
        for value in ["", "provider-key", "arr_00", "arr_zz", "arr_\u{00e9}"] {
            assert_eq!(
                check(&lab.state.config.db_path, value).await.unwrap_err(),
                Invalid::Malformed
            );
        }
        let unknown = format!("{PREFIX}{}", "00".repeat(32));
        assert_eq!(
            check(&lab.state.config.db_path, &unknown)
                .await
                .unwrap_err(),
            Invalid::Unknown
        );
        assert_eq!(
            check(
                &lab.state.config.db_path.with_file_name("missing.db"),
                &unknown
            )
            .await
            .unwrap_err(),
            Invalid::Unavailable
        );
    }

    #[test]
    fn dedicated_token_header_wins_over_native_provider_authentication() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer native-provider-key"),
        );
        headers.insert(
            "x-agent-router-password",
            HeaderValue::from_static("legacy-password"),
        );
        headers.insert(
            "x-agent-router-token",
            HeaderValue::from_static("arr_012345"),
        );
        assert_eq!(presented(&headers).as_deref(), Some("arr_012345"));
        assert_eq!(
            super::super::presented(&headers).as_deref(),
            Some("arr_012345")
        );
        headers.insert("x-agent-router-token", HeaderValue::from_static(""));
        assert_eq!(presented(&headers).as_deref(), Some(""));
        headers.remove("x-agent-router-token");
        assert_eq!(presented(&headers), None);
        headers.remove("x-agent-router-password");
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer arr_012345"),
        );
        assert_eq!(presented(&headers).as_deref(), Some("arr_012345"));
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Basic YW55OmFycl8wMTIzNDU="),
        );
        assert_eq!(presented(&headers).as_deref(), Some("arr_012345"));
        headers.remove(header::AUTHORIZATION);
        headers.insert("x-api-key", HeaderValue::from_static("arr_012345"));
        assert_eq!(presented(&headers).as_deref(), Some("arr_012345"));
    }

    #[tokio::test]
    async fn creation_validation_and_public_api_contract() {
        let lab = Lab::local("auth-token-api");
        for body in [
            json!({ "name": "", "scope": "client" }),
            json!({ "name": "x".repeat(65), "scope": "client" }),
            json!({ "name": "newline\nname", "scope": "client" }),
            json!({ "name": "expired", "scope": "client", "expires_at": 1 }),
        ] {
            let (status, _) = lab.post("/api/auth/tokens", &body.to_string()).await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        }
        let (status, _) = lab
            .post("/api/auth/tokens", r#"{"name":"wrong","scope":"root"}"#)
            .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        let (status, made) = lab
            .post(
                "/api/auth/tokens",
                r#"{"name":" Codex laptop ","scope":"client","expires_at":null}"#,
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{made}");
        assert_eq!(made["name"], "Codex laptop");
        assert_eq!(made["scope"], "client");
        assert!(made["token"].as_str().unwrap().starts_with(PREFIX));
        assert!(
            made["last_used_at"].is_null()
                && made["expires_at"].is_null()
                && made["revoked_at"].is_null()
        );
        let (status, listed) = lab.get("/api/auth/tokens").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(listed["tokens"][0]["id"], made["id"]);
        assert!(
            listed["tokens"][0].get("token").is_none() && listed["tokens"][0].get("hash").is_none()
        );
        let (_, auth) = lab.get("/api/auth").await;
        assert_eq!(auth["access_tokens"], 1);
        assert_eq!(
            auth["configured"], false,
            "token support does not change the password status contract"
        );
        let mut request =
            Request::delete(format!("/api/auth/tokens/{}", made["id"].as_str().unwrap()))
                .header(header::HOST, "127.0.0.1")
                .body(Body::empty())
                .unwrap();
        request
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 50_000))));
        let revoked = lab.app.clone().oneshot(request).await.unwrap();
        assert_eq!(revoked.status(), StatusCode::OK);
        assert_eq!(revoked.headers()[header::CACHE_CONTROL], "no-store");
        let value = made["token"].as_str().unwrap();
        assert_eq!(
            check(&lab.state.config.db_path, value).await.unwrap_err(),
            Invalid::Revoked
        );
        let (_, listed) = lab.get("/api/auth/tokens").await;
        assert!(listed["tokens"][0]["revoked_at"].is_i64());
        let (_, auth) = lab.get("/api/auth").await;
        assert_eq!(auth["access_tokens"], 0);
    }
}
