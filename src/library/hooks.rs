//! A forge's push webhook: `POST /hooks/library/{source}` re-imports that source at once.
//! docs/library.md, "Forge webhooks".
//!
//! A forge cannot hold the router's password, so this one path is open on a hosted router
//! (`guard`), and the forge proves itself with the source's own secret instead:
//! - GitLab sends the secret as `X-Gitlab-Token`;
//! - GitHub signs the body with it, as `X-Hub-Signature-256` (HMAC-SHA256).
//!
//! Both are compared in constant time. Only a push to the ref the source follows imports; a
//! GitHub ping is answered; a delivery already seen imports nothing. The import runs after
//! the answer, which a forge wants within seconds. The poller stays, for a hook that never
//! came.

use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Path as Params, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};

use super::forge::ForgeKind;
use super::store;
use crate::proxy::AppState;
use crate::refusal::{failed, refuse};

/// The most a hook's body may be: a push's description, never its files.
const BODY_MAX: usize = 1024 * 1024;

/// How long a delivery's id is remembered, against a forge sending it again.
const REMEMBER_MS: i64 = 7 * 24 * 60 * 60 * 1000;

/// Whether `path` is a source's hook, which a hosted router answers without the password.
pub fn is_open(path: &str) -> bool {
    path.strip_prefix("/hooks/library/").is_some_and(|key| !key.is_empty() && !key.contains('/'))
}

/// Make a source a new secret, and answer it — the only time it is shown. The one before
/// stops working.
pub async fn new_secret(State(state): State<Arc<AppState>>, Params(key): Params<String>) -> Response {
    let secret = {
        use ring::rand::SecureRandom;
        let mut buf = [0u8; 24];
        ring::rand::SystemRandom::new().fill(&mut buf).expect("the system's random source");
        buf.iter().map(|b| format!("{b:02x}")).collect::<String>()
    };
    let (db, wanted, kept) = (state.config.db_path.clone(), key.clone(), secret.clone());
    let set = tokio::task::spawn_blocking(move || -> anyhow::Result<usize> {
        Ok(store::writer(&db)?.execute("UPDATE library_sources SET hook_secret = ?2 WHERE key = ?1", params![wanted, kept])?)
    })
    .await;
    let path = format!("/hooks/library/{key}");
    match set {
        Ok(Ok(1)) => {
            let url = state.config.public_url.as_ref().map(|u| format!("{}{path}", u.trim_end_matches('/')));
            Json(json!({ "ok": true, "secret": secret, "path": path, "url": url })).into_response()
        }
        Ok(Ok(_)) => refuse(StatusCode::NOT_FOUND, format!("there is no source {key}")),
        Ok(Err(e)) => failed(e),
        Err(e) => failed(e),
    }
}

/// `POST /hooks/library/{source}`, from the forge.
pub async fn hook(State(state): State<Arc<AppState>>, Params(key): Params<String>, headers: HeaderMap, body: Body) -> Response {
    let Ok(body) = axum::body::to_bytes(body, BODY_MAX).await else {
        return refuse(StatusCode::PAYLOAD_TOO_LARGE, "a hook's body may be at most 1 MiB");
    };
    let (db, wanted) = (state.config.db_path.clone(), key.clone());
    let found = tokio::task::spawn_blocking(move || -> anyhow::Result<Option<(ForgeKind, String, Option<String>)>> {
        let conn = store::writer(&db)?;
        Ok(conn
            .query_row(
                "SELECT f.kind, s.ref, s.hook_secret FROM library_sources s JOIN library_forges f ON f.key = s.forge WHERE s.key = ?1",
                [&wanted],
                |r| {
                    let kind: String = r.get(0)?;
                    Ok((ForgeKind::parse(&kind).unwrap_or(ForgeKind::GitLab), r.get(1)?, r.get(2)?))
                },
            )
            .optional()?)
    })
    .await;
    let (kind, reference, secret) = match found {
        Ok(Ok(Some(found))) => found,
        Ok(Ok(None)) => return refuse(StatusCode::NOT_FOUND, format!("there is no Library source {key}")),
        Ok(Err(e)) => return failed(e),
        Err(e) => return failed(e),
    };
    let Some(secret) = secret else {
        return refuse(StatusCode::FORBIDDEN, format!("the source {key} has no webhook secret yet: make one in Settings, then give it to the forge"));
    };
    let header = |name: &str| headers.get(name).and_then(|v| v.to_str().ok()).unwrap_or_default().to_string();

    let (proved, event, delivery) = match kind {
        ForgeKind::GitLab => (same(header("x-gitlab-token").as_bytes(), secret.as_bytes()), header("x-gitlab-event"), header("x-gitlab-event-uuid")),
        ForgeKind::GitHub => (signed(&secret, &body, &header("x-hub-signature-256")), header("x-github-event"), header("x-github-delivery")),
    };
    if !proved {
        tracing::warn!(source = %key, "a Library hook was refused: it did not prove itself with the source's secret");
        return refuse(StatusCode::UNAUTHORIZED, format!("this hook does not carry the source {key}'s secret"));
    }
    let push = match kind {
        ForgeKind::GitLab => matches!(event.as_str(), "Push Hook" | "Tag Push Hook"),
        ForgeKind::GitHub if event == "ping" => return Json(json!({ "ok": true, "pong": true })).into_response(),
        ForgeKind::GitHub => event == "push",
    };
    if !push {
        return Json(json!({ "ok": true, "importing": false, "why": format!("{event} is not a push") })).into_response();
    }
    let pushed = serde_json::from_slice::<Value>(&body).ok().and_then(|v| v.get("ref").and_then(Value::as_str).map(str::to_string)).unwrap_or_default();
    if !follows(&reference, &pushed) {
        return Json(json!({ "ok": true, "importing": false, "why": format!("a push to {pushed}; this source follows {reference}") }))
            .into_response();
    }
    if !delivery.is_empty() {
        let (db, id, source) = (state.config.db_path.clone(), delivery.clone(), key.clone());
        let fresh = tokio::task::spawn_blocking(move || -> anyhow::Result<bool> {
            let conn = store::writer(&db)?;
            let now = crate::db::now_ms();
            conn.execute("DELETE FROM library_hook_deliveries WHERE at_ms < ?1", [now - REMEMBER_MS])?;
            Ok(conn.execute(
                "INSERT OR IGNORE INTO library_hook_deliveries (id, source, at_ms) VALUES (?1, ?2, ?3)",
                params![id, source, now],
            )? == 1)
        })
        .await;
        match fresh {
            Ok(Ok(true)) => {}
            Ok(Ok(false)) => {
                return Json(json!({ "ok": true, "importing": false, "why": format!("delivery {delivery} was seen already") }))
                    .into_response()
            }
            Ok(Err(e)) => return failed(e),
            Err(e) => return failed(e),
        }
    }
    let state = state.clone();
    tokio::spawn(async move {
        match super::sources::import(&state.config.db_path, &key, true).await {
            Ok(Ok(imported)) => {
                tracing::info!(source = %key, commit = %imported.commit, items = imported.items.len(), "imported a source on its forge's push");
                if imported.went_live() {
                    super::push::push(&state);
                }
            }
            Ok(Err(why)) => tracing::warn!(source = %key, "a source's forge said no: {why}"),
            Err(e) => tracing::warn!(source = %key, "the source could not be imported: {e:#}"),
        }
    });
    (StatusCode::ACCEPTED, Json(json!({ "ok": true, "importing": true }))).into_response()
}

/// Whether a push to `pushed` is to the ref a source follows: a branch's or a tag's name,
/// or the whole ref.
fn follows(reference: &str, pushed: &str) -> bool {
    pushed == reference || pushed == format!("refs/heads/{reference}") || pushed == format!("refs/tags/{reference}")
}

/// Equal, in time that does not depend on where they differ.
fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |diff, (x, y)| diff | (x ^ y)) == 0 && !a.is_empty()
}

/// Whether `header` is GitHub's `sha256=<hex>` HMAC of `body` under `secret`.
fn signed(secret: &str, body: &[u8], header: &str) -> bool {
    let Some(hex) = header.strip_prefix("sha256=") else { return false };
    let Some(given) = (0..hex.len())
        .step_by(2)
        .map(|i| hex.get(i..i + 2).and_then(|h| u8::from_str_radix(h, 16).ok()))
        .collect::<Option<Vec<u8>>>()
    else {
        return false;
    };
    let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, secret.as_bytes());
    ring::hmac::verify(&key, body, &given).is_ok()
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;
    use std::sync::atomic::Ordering;
    use std::time::Duration;

    use axum::extract::ConnectInfo;
    use axum::http::{header, Request};
    use tower::ServiceExt;

    use super::*;
    use crate::api_refusals::Lab;
    use crate::library::sources::tests::{ask, gitlab, import, put_ok, repository, serve_stub, skill_md, stub_of, Stub};

    /// Send a hook as a forge does: no password, its own headers.
    async fn deliver(lab: &Lab, key: &str, headers: &[(&str, String)], body: &str) -> (StatusCode, Value) {
        let mut req = Request::post(format!("/hooks/library/{key}")).header(header::HOST, "127.0.0.1").header(header::CONTENT_TYPE, "application/json");
        for (name, value) in headers {
            req = req.header(*name, value);
        }
        let mut req = req.body(Body::from(body.to_string())).unwrap();
        req.extensions_mut().insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 50_000))));
        let res = lab.app.clone().oneshot(req).await.unwrap();
        let status = res.status();
        let bytes = axum::body::to_bytes(res.into_body(), 1 << 20).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
    }

    async fn make_secret(lab: &Lab, key: &str) -> String {
        let (status, made, _) = ask(lab, "POST", &format!("/api/library/sources/{key}/secret"), Value::Null).await;
        assert_eq!(status, StatusCode::OK, "{made}");
        assert_eq!(made["path"], format!("/hooks/library/{key}"));
        made["secret"].as_str().unwrap().to_string()
    }

    async fn head(lab: &Lab, key: &str) -> Option<String> {
        let (_, sources, _) = ask(lab, "GET", "/api/library/sources", Value::Null).await;
        sources["sources"].as_array().unwrap().iter().find(|s| s["key"] == key).and_then(|s| s["head"].as_str().map(str::to_string))
    }

    async fn until_head(lab: &Lab, key: &str, commit: &str) {
        let waited = async {
            while head(lab, key).await.as_deref() != Some(commit) {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        };
        tokio::time::timeout(Duration::from_secs(10), waited).await.expect("the push was imported");
    }

    fn gitlab_push(secret: &str, delivery: &str) -> Vec<(&'static str, String)> {
        vec![("x-gitlab-token", secret.to_string()), ("x-gitlab-event", "Push Hook".into()), ("x-gitlab-event-uuid", delivery.into())]
    }

    fn github_signature(secret: &str, body: &str) -> String {
        let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, secret.as_bytes());
        let tag = ring::hmac::sign(&key, body.as_bytes());
        format!("sha256={}", tag.as_ref().iter().map(|b| format!("{b:02x}")).collect::<String>())
    }

    async fn gitlab_source(name: &str) -> (Lab, std::sync::Arc<Stub>, String) {
        let lab = Lab::hosted(name).await;
        let stub = stub_of("1000000000000000000000000000000000000001", repository());
        gitlab(&lab, &stub, None).await;
        put_ok(&lab, "/api/library/sources/skills", json!({ "forge": "home", "repository": "whnet/skills", "ref": "main", "path": "library", "live": true })).await;
        import(&lab, "skills").await;
        let secret = make_secret(&lab, "skills").await;
        (lab, stub, secret)
    }

    #[tokio::test]
    async fn a_signed_gitlab_push_reimports_its_source() {
        let (lab, stub, secret) = gitlab_source("hook-gitlab").await;
        let moved = "2000000000000000000000000000000000000002";
        stub.change(moved, "library/skills/bench/SKILL.md", &skill_md("bench", "Drives it, after a push."));
        let body = json!({ "ref": "refs/heads/main", "checkout_sha": moved }).to_string();
        let (status, answer) = deliver(&lab, "skills", &gitlab_push(&secret, "delivery-1"), &body).await;
        assert_eq!((status, &answer["importing"]), (StatusCode::ACCEPTED, &json!(true)), "{answer}");
        until_head(&lab, "skills", moved).await;
        let (_, item, _) = ask(&lab, "GET", "/api/library/items/skill/bench", Value::Null).await;
        assert_eq!(item["versions"][0]["description"], "Drives it, after a push.");
    }

    #[tokio::test]
    async fn a_signed_github_push_reimports_its_source() {
        let lab = Lab::hosted("hook-github").await;
        let stub = stub_of("3000000000000000000000000000000000000003", repository());
        let base = serve_stub(stub.clone()).await;
        put_ok(&lab, "/api/library/forges/hub", json!({ "kind": "github", "base_url": base })).await;
        put_ok(&lab, "/api/library/sources/skills", json!({ "forge": "hub", "repository": "someone/skills", "ref": "main", "path": "library", "live": true })).await;
        import(&lab, "skills").await;
        let secret = make_secret(&lab, "skills").await;

        let ping = json!({ "zen": "Keep it logically awesome." }).to_string();
        let headers = vec![("x-github-event", "ping".into()), ("x-github-delivery", "p-1".into()), ("x-hub-signature-256", github_signature(&secret, &ping))];
        let (status, answer) = deliver(&lab, "skills", &headers, &ping).await;
        assert_eq!((status, &answer["pong"]), (StatusCode::OK, &json!(true)));

        let moved = "4000000000000000000000000000000000000004";
        stub.change(moved, "library/commands/ship.md", "---\ndescription: Ship, after a push.\n---\nShip.\n");
        let body = json!({ "ref": "refs/heads/main", "after": moved }).to_string();
        let headers = vec![("x-github-event", "push".into()), ("x-github-delivery", "d-1".into()), ("x-hub-signature-256", github_signature(&secret, &body))];
        let (status, _) = deliver(&lab, "skills", &headers, &body).await;
        assert_eq!(status, StatusCode::ACCEPTED);
        until_head(&lab, "skills", moved).await;
    }

    #[tokio::test]
    async fn a_push_to_another_ref_imports_nothing() {
        let (lab, stub, secret) = gitlab_source("hook-other-ref").await;
        let before = stub.archives.load(Ordering::SeqCst);
        let body = json!({ "ref": "refs/heads/feature", "checkout_sha": "5000000000000000000000000000000000000005" }).to_string();
        let (status, answer) = deliver(&lab, "skills", &gitlab_push(&secret, "delivery-1"), &body).await;
        assert_eq!((status, &answer["importing"]), (StatusCode::OK, &json!(false)));
        assert!(answer["why"].as_str().unwrap().contains("refs/heads/feature; this source follows main"), "{answer}");
        let (status, answer) = deliver(&lab, "skills", &[("x-gitlab-token", secret.clone()), ("x-gitlab-event", "Issue Hook".into())], "{}").await;
        assert_eq!((status, &answer["importing"]), (StatusCode::OK, &json!(false)), "not a push");
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(stub.archives.load(Ordering::SeqCst), before);
        assert!(follows("main", "refs/heads/main") && follows("v1.2", "refs/tags/v1.2") && !follows("main", "refs/heads/mainline"));
    }

    #[tokio::test]
    async fn an_unproved_hook_is_refused() {
        let (lab, stub, secret) = gitlab_source("hook-unproved").await;
        let before = stub.archives.load(Ordering::SeqCst);
        let body = json!({ "ref": "refs/heads/main" }).to_string();
        for headers in [
            vec![("x-gitlab-event", "Push Hook".to_string())],
            vec![("x-gitlab-event", "Push Hook".to_string()), ("x-gitlab-token", format!("{secret}x"))],
            vec![("x-gitlab-event", "Push Hook".to_string()), ("x-gitlab-token", String::new())],
        ] {
            let (status, answer) = deliver(&lab, "skills", &headers, &body).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED, "{headers:?}");
            assert!(answer["error"].as_str().unwrap().contains("does not carry the source skills's secret"), "{answer}");
        }

        // GitHub's signature, missing and wrong.
        let hub = stub_of("6000000000000000000000000000000000000006", repository());
        let base = serve_stub(hub).await;
        put_ok(&lab, "/api/library/forges/hub", json!({ "kind": "github", "base_url": base })).await;
        put_ok(&lab, "/api/library/sources/hubbed", json!({ "forge": "hub", "repository": "someone/skills", "ref": "main", "live": true })).await;
        // A source with no secret yet is refused before anything is checked.
        let (status, answer) = deliver(&lab, "hubbed", &[("x-github-event", "push".into())], &body).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{answer}");
        let hub_secret = make_secret(&lab, "hubbed").await;
        for signature in [None, Some(github_signature("not the secret", &body)), Some(github_signature(&hub_secret, "another body"))] {
            let mut headers = vec![("x-github-event", "push".to_string())];
            headers.extend(signature.map(|s| ("x-hub-signature-256", s)));
            let (status, _) = deliver(&lab, "hubbed", &headers, &body).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED, "{headers:?}");
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(stub.archives.load(Ordering::SeqCst), before, "nothing was imported");
    }

    #[tokio::test]
    async fn a_hook_for_an_unknown_source_is_refused() {
        let lab = Lab::hosted("hook-unknown").await;
        let (status, answer) = deliver(&lab, "nothing", &[("x-gitlab-event", "Push Hook".into())], "{}").await;
        assert_eq!(status, StatusCode::NOT_FOUND, "answered without the password, and refused: {answer}");
        assert!(answer["error"].as_str().unwrap().contains("no Library source nothing"));
    }

    #[tokio::test]
    async fn a_replayed_delivery_imports_nothing() {
        let (lab, stub, secret) = gitlab_source("hook-replay").await;
        let moved = "7000000000000000000000000000000000000007";
        stub.change(moved, "library/skills/bench/SKILL.md", &skill_md("bench", "Once."));
        let body = json!({ "ref": "refs/heads/main" }).to_string();
        let (status, _) = deliver(&lab, "skills", &gitlab_push(&secret, "once"), &body).await;
        assert_eq!(status, StatusCode::ACCEPTED);
        until_head(&lab, "skills", moved).await;
        let after = stub.archives.load(Ordering::SeqCst);

        let (status, answer) = deliver(&lab, "skills", &gitlab_push(&secret, "once"), &body).await;
        assert_eq!((status, &answer["importing"]), (StatusCode::OK, &json!(false)));
        assert!(answer["why"].as_str().unwrap().contains("seen already"), "{answer}");
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(stub.archives.load(Ordering::SeqCst), after);
    }

    /// Every address a hosted router serves, from its route tables, asks for the password —
    /// all but health, the installer's files and the Library's hooks.
    #[tokio::test]
    async fn only_health_the_installer_and_library_hooks_are_open() {
        let lab = Lab::hosted("hook-open").await;
        let mut paths = Vec::new();
        for file in ["src/hosted.rs", "src/main.rs", "src/console.rs"] {
            let table = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(file)).unwrap();
            for (at, marker) in table.match_indices(".route(\"") {
                if let Some((path, _)) = table[at + marker.len()..].split_once('"') {
                    paths.push(path.to_string());
                }
            }
            for (at, marker) in table.match_indices(".route(\n") {
                if let Some(path) = table[at + marker.len()..].trim_start().strip_prefix('"').and_then(|p| p.split_once('"')).map(|(p, _)| p) {
                    paths.push(path.to_string());
                }
            }
        }
        paths.extend(["/", "/live", "/v1/messages", "/hooks/other/x", "/hooks/library", "/hooks/library/a/b"].map(String::from));
        assert!(paths.len() > 60, "the route tables were read: {paths:?}");
        let mut open = Vec::new();
        for path in &paths {
            let concrete: String = path.split('/').map(|part| if part.starts_with('{') { "x" } else { part }).collect::<Vec<_>>().join("/");
            for method in ["GET", "POST"] {
                let mut req = Request::builder().method(method).uri(&concrete).header(header::HOST, "127.0.0.1").body(Body::empty()).unwrap();
                req.extensions_mut().insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 50_000))));
                let status = lab.app.clone().oneshot(req).await.unwrap().status();
                if status != StatusCode::UNAUTHORIZED {
                    open.push(format!("{method} {concrete}"));
                }
            }
        }
        open.sort();
        open.dedup();
        assert_eq!(
            open,
            ["GET /_router/health", "GET /agent/install.ps1", "GET /agent/install.sh", "GET /hooks/library/x", "POST /_router/health", "POST /agent/install.ps1", "POST /agent/install.sh", "POST /hooks/library/x"],
            "every other address asks for the password"
        );
    }
}
