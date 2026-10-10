//! Publish tokens, and who may publish what: docs/library.md, "Agents publishing".
//!
//! A publish token can publish a version to the Library and do nothing else. It is for a
//! pipeline or a cloud session, which must never hold the router's password. A token is
//! shown once, when it is made, and kept only as its sha256. The guard lets one through on
//! the publish route alone ([`presented`]); every other route, reading included, takes it for
//! a wrong password. Each token has a scope (kinds, or named items) and its own trust.
//!
//! Publishes from connected machines (`agent-router library publish`) use the password the
//! machine already holds, and share one trust setting, [`machine_publishes_live`].

use std::path::Path;
use std::sync::Arc;

use axum::extract::{Path as Params, State};
use axum::http::{header, HeaderMap, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

use super::store;
use super::Kind;
use crate::proxy::AppState;
use crate::refusal::{failed, refuse, JsonBody};

/// Every token begins so, which is how the guard knows to look one up rather than check a
/// password.
pub const PREFIX: &str = "arl_";

const MACHINE_PUBLISHES: &str = "library.machine_publishes";

/// A token the guard let through, as the publish handler sees it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PublishToken {
    pub id: String,
    pub name: String,
    /// The kinds it may publish; empty, any.
    pub kinds: Vec<Kind>,
    /// The items it may publish, as `kind/name`; when any are named, only those.
    pub items: Vec<String>,
    /// Whether what it publishes goes live, or waits for approval.
    pub live: bool,
}

impl PublishToken {
    pub fn allows(&self, kind: Kind, name: &str) -> bool {
        if !self.items.is_empty() {
            return self.items.iter().any(|i| *i == format!("{kind}/{name}"));
        }
        self.kinds.is_empty() || self.kinds.contains(&kind)
    }

    pub fn scope(&self) -> String {
        if !self.items.is_empty() {
            self.items.join(", ")
        } else if !self.kinds.is_empty() {
            self.kinds.iter().map(|k| format!("any {k}")).collect::<Vec<_>>().join(", ")
        } else {
            "anything".to_string()
        }
    }
}

/// Whether this is the one route a token may use: publishing a version of an item.
pub fn is_publish(method: &Method, path: &str) -> bool {
    method == Method::POST
        && path.strip_prefix("/api/library/items/").is_some_and(|rest| {
            let parts: Vec<&str> = rest.split('/').collect();
            parts.len() == 3 && !parts[0].is_empty() && !parts[1].is_empty() && parts[2] == "versions"
        })
}

/// The publish token a request presents, if it presents a live one on the publish route.
pub async fn presented(state: &AppState, method: &Method, path: &str, headers: &HeaderMap) -> Option<PublishToken> {
    if !is_publish(method, path) {
        return None;
    }
    let token = headers.get(header::AUTHORIZATION)?.to_str().ok()?.strip_prefix("Bearer ")?.trim();
    if !token.starts_with(PREFIX) {
        return None;
    }
    let (db, hash) = (state.config.db_path.clone(), digest(token));
    tokio::task::spawn_blocking(move || lookup(&db, &hash)).await.ok().flatten()
}

fn lookup(db: &Path, hash: &str) -> Option<PublishToken> {
    let conn = Connection::open_with_flags(db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).ok()?;
    conn.query_row(
        "SELECT id, name, kinds, items, live FROM library_tokens WHERE hash = ?1 AND revoked_at IS NULL",
        [hash],
        |r| {
            let kinds: String = r.get(2)?;
            let items: String = r.get(3)?;
            Ok(PublishToken {
                id: r.get(0)?,
                name: r.get(1)?,
                kinds: serde_json::from_str(&kinds).unwrap_or_default(),
                items: serde_json::from_str(&items).unwrap_or_default(),
                live: r.get(4)?,
            })
        },
    )
    .ok()
}

/// What a token is kept as: its sha256, in lowercase hex.
pub fn digest(token: &str) -> String {
    Sha256::digest(token.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

fn random_hex(bytes: usize) -> String {
    use ring::rand::SecureRandom;
    let mut buf = vec![0u8; bytes];
    ring::rand::SystemRandom::new().fill(&mut buf).expect("the system's random source");
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

/// Note that a token was used, for the Settings page.
pub fn used(conn: &Connection, id: &str, now: i64) -> anyhow::Result<()> {
    conn.execute("UPDATE library_tokens SET last_used_at = ?2 WHERE id = ?1", params![id, now])?;
    Ok(())
}

/// Whether publishes from connected machines go live, rather than wait for approval. They
/// wait unless the owner said otherwise.
pub fn machine_publishes_live(db: &Path) -> bool {
    crate::db::setting(db, MACHINE_PUBLISHES).as_deref() == Some("live")
}

#[derive(Deserialize)]
pub struct TokenIn {
    name: String,
    #[serde(default)]
    kinds: Vec<String>,
    #[serde(default)]
    items: Vec<String>,
    #[serde(default)]
    live: bool,
}

/// `POST /api/library/tokens`: make a token. Its value is in this answer and nowhere else, ever.
pub async fn create(State(state): State<Arc<AppState>>, JsonBody(body): JsonBody<TokenIn>) -> Response {
    let name = body.name.trim().to_string();
    if name.is_empty() || name.len() > 64 {
        return refuse(StatusCode::UNPROCESSABLE_ENTITY, "a token needs a name, at most 64 characters: versions it publishes are signed with it");
    }
    let mut kinds = Vec::new();
    for kind in &body.kinds {
        match Kind::parse(kind) {
            Some(k) => kinds.push(k),
            None => return refuse(StatusCode::UNPROCESSABLE_ENTITY, format!("no kind is called {kind}: skill, command or subagent")),
        }
    }
    for item in &body.items {
        let fits = item.split_once('/').is_some_and(|(k, n)| Kind::parse(k).is_some() && super::check_name(n).is_ok());
        if !fits {
            return refuse(StatusCode::UNPROCESSABLE_ENTITY, format!("{item} is not an item: write kind/name, as skill/bench"));
        }
    }
    let (id, token) = (random_hex(8), format!("{PREFIX}{}", random_hex(20)));
    let (db, hash, live, items) = (state.config.db_path.clone(), digest(&token), body.live, body.items.clone());
    let kinds_json = serde_json::to_string(&kinds).unwrap_or_else(|_| "[]".into());
    let (row_id, row_name) = (id.clone(), name.clone());
    let made = tokio::task::spawn_blocking(move || -> anyhow::Result<bool> {
        let conn = store::writer(&db)?;
        let taken: Option<String> = conn.query_row("SELECT id FROM library_tokens WHERE name = ?1", [&row_name], |r| r.get(0)).optional()?;
        if taken.is_some() {
            return Ok(false);
        }
        conn.execute(
            "INSERT INTO library_tokens (id, name, hash, kinds, items, live, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![row_id, row_name, hash, kinds_json, serde_json::to_string(&items)?, live, crate::db::now_ms()],
        )?;
        Ok(true)
    })
    .await;
    match made {
        Ok(Ok(true)) => (StatusCode::CREATED, Json(json!({ "ok": true, "id": id, "name": name, "token": token }))).into_response(),
        Ok(Ok(false)) => refuse(
            StatusCode::CONFLICT,
            format!("a token called {name} exists already; a revoked one keeps its name, so what it published still names it"),
        ),
        Ok(Err(e)) => failed(e),
        Err(e) => failed(e),
    }
}

/// `GET /api/library/tokens`: every token, revoked ones included, without its value or hash.
pub async fn list(State(state): State<Arc<AppState>>) -> Response {
    let db = state.config.db_path.clone();
    let rows = tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<serde_json::Value>> {
        let conn = store::writer(&db)?;
        let mut stmt = conn.prepare(
            "SELECT id, name, kinds, items, live, created_at, last_used_at, revoked_at FROM library_tokens ORDER BY created_at",
        )?;
        let rows = stmt.query_map([], |r| {
            let kinds: String = r.get(2)?;
            let items: String = r.get(3)?;
            Ok(json!({
                "id": r.get::<_, String>(0)?,
                "name": r.get::<_, String>(1)?,
                "kinds": serde_json::from_str::<serde_json::Value>(&kinds).unwrap_or_default(),
                "items": serde_json::from_str::<serde_json::Value>(&items).unwrap_or_default(),
                "live": r.get::<_, bool>(4)?,
                "created_at": r.get::<_, i64>(5)?,
                "last_used_at": r.get::<_, Option<i64>>(6)?,
                "revoked_at": r.get::<_, Option<i64>>(7)?,
            }))
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    })
    .await;
    match rows {
        Ok(Ok(rows)) => Json(json!({ "tokens": rows })).into_response(),
        Ok(Err(e)) => failed(e),
        Err(e) => failed(e),
    }
}

/// `DELETE /api/library/tokens/{id}`: revoke a token. It publishes nothing from then on;
/// its row stays, so the versions it published still name it.
pub async fn revoke(State(state): State<Arc<AppState>>, Params(id): Params<String>) -> Response {
    let (db, wanted) = (state.config.db_path.clone(), id.clone());
    let done = tokio::task::spawn_blocking(move || -> anyhow::Result<usize> {
        let conn = store::writer(&db)?;
        Ok(conn.execute(
            "UPDATE library_tokens SET revoked_at = coalesce(revoked_at, ?2) WHERE id = ?1",
            params![wanted, crate::db::now_ms()],
        )?)
    })
    .await;
    match done {
        Ok(Ok(1)) => Json(json!({ "ok": true })).into_response(),
        Ok(Ok(_)) => refuse(StatusCode::NOT_FOUND, format!("there is no token {id}")),
        Ok(Err(e)) => failed(e),
        Err(e) => failed(e),
    }
}

#[derive(Deserialize)]
pub struct SettingsIn {
    machine_publishes: String,
}

/// `GET /api/library/settings`: whether publishes from machines go live.
pub async fn settings(State(state): State<Arc<AppState>>) -> Response {
    let live = machine_publishes_live(&state.config.db_path);
    Json(json!({ "machine_publishes": if live { "live" } else { "approval" } })).into_response()
}

/// `PUT /api/library/settings`: `{"machine_publishes": "live" | "approval"}`.
pub async fn set_settings(State(state): State<Arc<AppState>>, JsonBody(body): JsonBody<SettingsIn>) -> Response {
    if !matches!(body.machine_publishes.as_str(), "live" | "approval") {
        return refuse(StatusCode::UNPROCESSABLE_ENTITY, "machine_publishes is live or approval");
    }
    match crate::db::set_setting(&state.config.db_path, MACHINE_PUBLISHES, &body.machine_publishes) {
        Ok(()) => Json(json!({ "ok": true, "machine_publishes": body.machine_publishes })).into_response(),
        Err(e) => failed(e),
    }
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;

    use axum::body::Body;
    use axum::extract::ConnectInfo;
    use axum::http::Request;
    use serde_json::Value;
    use tower::ServiceExt;

    use super::*;
    use crate::api_refusals::Lab;

    enum As<'a> {
        Password,
        Token(&'a str),
        Nobody,
    }

    async fn send(lab: &Lab, method: &str, path: &str, who: As<'_>, headers: &[(&str, &str)], body: impl Into<Body>) -> (StatusCode, Value) {
        let mut req = Request::builder().method(method).uri(path).header(header::HOST, "127.0.0.1");
        req = match who {
            As::Password => req.header(header::AUTHORIZATION, format!("Bearer {}", lab.password.unwrap())),
            As::Token(token) => req.header(header::AUTHORIZATION, format!("Bearer {token}")),
            As::Nobody => req,
        };
        for (name, value) in headers {
            req = req.header(*name, *value);
        }
        let mut req = req.body(body.into()).unwrap();
        req.extensions_mut().insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 50_000))));
        let res = lab.app.clone().oneshot(req).await.unwrap();
        let status = res.status();
        let bytes = axum::body::to_bytes(res.into_body(), 1 << 20).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
    }

    fn skill_md(name: &str, description: &str) -> String {
        format!("---\nname: {name}\ndescription: {description}\n---\n")
    }

    async fn make(lab: &Lab, body: Value) -> (String, String) {
        let (status, made) = send(lab, "POST", "/api/library/tokens", As::Password, &[("content-type", "application/json")], body.to_string()).await;
        assert_eq!(status, StatusCode::CREATED, "{made}");
        (made["id"].as_str().unwrap().to_string(), made["token"].as_str().unwrap().to_string())
    }

    async fn manifest(lab: &Lab) -> Vec<String> {
        let (_, held) = send(lab, "GET", "/api/library/manifest", As::Password, &[], Body::empty()).await;
        held["items"].as_array().unwrap().iter().map(|i| i["name"].as_str().unwrap().to_string()).collect()
    }

    #[tokio::test]
    async fn a_machines_publish_waits_unless_trusted() {
        let lab = Lab::hosted("publish-trust").await;
        let machine = [(crate::library::api::MACHINE, "machine-1"), (crate::library::api::LABEL, "bench mac")];
        let (status, sent) = send(&lab, "POST", "/api/library/items/skill/bench/versions", As::Password, &machine, skill_md("bench", "One.")).await;
        assert_eq!(status, StatusCode::CREATED, "{sent}");
        assert_eq!(sent["version"]["state"], "pending");
        assert!(manifest(&lab).await.is_empty(), "it reaches no machine until approved");
        let (_, said) = send(&lab, "GET", "/api/library/settings", As::Password, &[], Body::empty()).await;
        assert_eq!(said["machine_publishes"], "approval");

        let json = [("content-type", "application/json")];
        let (status, _) = send(&lab, "PUT", "/api/library/settings", As::Password, &json, r#"{"machine_publishes":"live"}"#).await;
        assert_eq!(status, StatusCode::OK);
        let (_, sent) = send(&lab, "POST", "/api/library/items/skill/bench/versions", As::Password, &machine, skill_md("bench", "Two.")).await;
        assert_eq!(sent["version"]["state"], "live");
        assert_eq!(manifest(&lab).await, ["bench"]);
        let (status, _) = send(&lab, "PUT", "/api/library/settings", As::Password, &json, r#"{"machine_publishes":"sometimes"}"#).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn a_publish_token_is_kept_only_as_a_hash() {
        let lab = Lab::hosted("token-hash").await;
        let (_, token) = make(&lab, json!({ "name": "ci" })).await;
        assert!(token.starts_with(PREFIX) && token.len() == PREFIX.len() + 40, "{token}");

        let conn = store::writer(&lab.state.config.db_path).unwrap();
        let kept: String = conn.query_row("SELECT hash FROM library_tokens WHERE name = 'ci'", [], |r| r.get(0)).unwrap();
        assert_eq!(kept, digest(&token));
        let everything: Vec<String> = conn
            .prepare("SELECT id || name || hash || kinds || items FROM library_tokens")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert!(everything.iter().all(|row| !row.contains(&token)), "the value itself is kept nowhere");
        let (_, listed) = send(&lab, "GET", "/api/library/tokens", As::Password, &[], Body::empty()).await;
        assert!(!listed.to_string().contains(&token) && !listed.to_string().contains(&kept), "{listed}");
        assert_eq!(listed["tokens"][0]["name"], "ci");

        let (status, _) = send(&lab, "POST", "/api/library/tokens", As::Password, &[("content-type", "application/json")], r#"{"name":"ci"}"#).await;
        assert_eq!(status, StatusCode::CONFLICT, "one name, one token");
    }

    #[tokio::test]
    async fn a_publish_token_can_only_publish_within_its_scope() {
        let lab = Lab::hosted("token-scope").await;
        let (_, bench_only) = make(&lab, json!({ "name": "bench-ci", "items": ["skill/bench"] })).await;
        let (status, sent) = send(&lab, "POST", "/api/library/items/skill/bench/versions", As::Token(&bench_only), &[], skill_md("bench", "One.")).await;
        assert_eq!(status, StatusCode::CREATED, "{sent}");
        assert_eq!((&sent["version"]["state"], &sent["version"]["provenance"]), (&json!("pending"), &json!({ "from": "token", "token": "bench-ci" })));
        let (status, refused) = send(&lab, "POST", "/api/library/items/skill/vault/versions", As::Token(&bench_only), &[], skill_md("vault", "One.")).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert!(refused["error"].as_str().unwrap().contains("may publish skill/bench, and not the skill vault"), "{refused}");

        // A token for a kind, trusted to go live.
        let (_, commands) = make(&lab, json!({ "name": "commands-ci", "kinds": ["command"], "live": true })).await;
        let command = "---\ndescription: Ship it.\n---\nShip.\n";
        let (status, sent) = send(&lab, "POST", "/api/library/items/command/ship/versions", As::Token(&commands), &[], command).await;
        assert_eq!((status, &sent["version"]["state"]), (StatusCode::CREATED, &json!("live")));
        let (status, _) = send(&lab, "POST", "/api/library/items/skill/other/versions", As::Token(&commands), &[], skill_md("other", "x")).await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        // It is no password anywhere else: not to read, not to decide, not to make tokens.
        for (method, path) in [
            ("GET", "/api/library"),
            ("GET", "/api/library/manifest"),
            ("GET", "/api/library/items/skill/bench"),
            ("POST", "/api/library/items/skill/bench/versions/1/live"),
            ("POST", "/api/library/items/skill/bench/archive"),
            ("POST", "/api/library/tokens"),
            ("GET", "/api/machines"),
            ("POST", "/v1/messages"),
        ] {
            let (status, _) = send(&lab, method, path, As::Token(&bench_only), &[], Body::empty()).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED, "{method} {path}");
        }
        let (_, used) = send(&lab, "GET", "/api/library/tokens", As::Password, &[], Body::empty()).await;
        assert!(used["tokens"][0]["last_used_at"].is_i64(), "{used}");
    }

    #[tokio::test]
    async fn a_revoked_token_publishes_nothing() {
        let lab = Lab::hosted("token-revoked").await;
        let (id, token) = make(&lab, json!({ "name": "ci", "live": true })).await;
        let (status, _) = send(&lab, "POST", "/api/library/items/skill/bench/versions", As::Token(&token), &[], skill_md("bench", "One.")).await;
        assert_eq!(status, StatusCode::CREATED);
        let (status, _) = send(&lab, "DELETE", &format!("/api/library/tokens/{id}"), As::Password, &[], Body::empty()).await;
        assert_eq!(status, StatusCode::OK);
        let (status, _) = send(&lab, "POST", "/api/library/items/skill/bench/versions", As::Token(&token), &[], skill_md("bench", "Two.")).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        let (_, item) = send(&lab, "GET", "/api/library/items/skill/bench", As::Password, &[], Body::empty()).await;
        assert_eq!(item["versions"].as_array().unwrap().len(), 1, "nothing more was published");
        assert_eq!(item["versions"][0]["provenance"]["token"], "ci", "what it published still names it");
        let (_, listed) = send(&lab, "GET", "/api/library/tokens", As::Password, &[], Body::empty()).await;
        assert!(listed["tokens"][0]["revoked_at"].is_i64());
        let (status, _) = send(&lab, "DELETE", "/api/library/tokens/nothing", As::Password, &[], Body::empty()).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (status, _) = send(&lab, "POST", "/api/library/items/skill/bench/versions", As::Nobody, &[], skill_md("bench", "x")).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
}
