//! The Library's API on a hosted router (docs/library.md, "API"):
//!
//! - **Reading** the items, their versions and the files of any version.
//! - **Uploading** a version from the console, which goes live at once.
//! - **Deciding**: approving a pending version, rolling back to an older one, rejecting,
//!   and archiving an item.
//! - **The manifest and the blobs** every machine's agent syncs from.
//!
//! Every route is behind the password, as all of `/api` is. A router on a developer's
//! machine keeps no Library, and [`not_offered`] answers its paths in words.

use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use rusqlite::Connection;
use serde_json::{json, Value};

use super::bundle::{self, Bundle, BUNDLE_MAX};
use super::store::{self, Provenance, Published, State as Standing, Unmade, Version};
use super::{usage, Kind};
use crate::proxy::AppState;
use crate::refusal::{failed, refuse};

/// The most an upload may be as it arrives, compressed or not: what one version may hold.
/// The ingress lets 32 MiB through, so the router is what holds this line.
pub const UPLOAD_MAX: usize = BUNDLE_MAX;

const NOT_OFFERED: &str = "the Library is a hosted router's: it keeps the skills, commands and subagents of \
    the machines connected to it, and a router on a developer's machine has none";

/// Every Library path, on a router that keeps no Library.
pub async fn not_offered() -> Response {
    refuse(StatusCode::CONFLICT, NOT_OFFERED)
}

/// A refusal on its way out of a handler, boxed: a whole `Response` is too big to carry in
/// every `Result` it passes through.
pub struct Refusal(Box<Response>);

impl From<Response> for Refusal {
    fn from(response: Response) -> Self {
        Refusal(Box::new(response))
    }
}

impl IntoResponse for Refusal {
    fn into_response(self) -> Response {
        *self.0
    }
}

type Answer = Result<Response, Refusal>;

fn fail(why: impl std::fmt::Display) -> Refusal {
    failed(why).into()
}

/// `GET /api/library`: every item, archived ones included, each with its live version, how
/// many wait for approval, and how often it was used (`library::usage`).
pub async fn list(State(state): State<Arc<AppState>>) -> Answer {
    let (items, used) = read(&state, |conn| Ok((store::items(conn)?, usage::totals(conn, crate::db::now_ms())?))).await?;
    let rows: Vec<Value> = items
        .iter()
        .map(|item| {
            let used = used.get(&(item.kind, item.name.clone())).copied().unwrap_or_default();
            json!({
                "kind": item.kind,
                "name": item.name,
                "description": item.description(),
                "source": item.source,
                "created_at": item.created_at,
                "archived_at": item.archived_at,
                "live": item.live().map(brief),
                "newest": item.versions.first().map(brief),
                "pending": item.versions.iter().filter(|v| v.state == Standing::Pending).count(),
                "versions": item.versions.len(),
                "uses_30d": used.uses_30d,
                "last_used_at": used.last_used_at,
            })
        })
        .collect();
    Ok(Json(json!({ "items": rows })).into_response())
}

/// `GET /api/library/items/{kind}/{name}`: the item, every version of it, and the files of
/// its live version — or of its newest, while none is live.
pub async fn item(
    State(state): State<Arc<AppState>>,
    Path((kind, name)): Path<(String, String)>,
) -> Answer {
    let kind = kind_of(&kind)?;
    let looked_up = name.clone();
    let (item, blob) = read(&state, move |conn| {
        let Some(item) = store::item(conn, kind, &looked_up)? else { return Ok((None, None)) };
        let shown = item.live().or(item.versions.first()).map(|v| v.digest.clone());
        let blob = shown.map(|digest| store::blob(conn, &digest)).transpose()?.flatten();
        Ok((Some(item), blob))
    })
    .await?;
    let Some(item) = item else { return Err(no_item_named(kind, &name)) };
    let files: Vec<Value> = match blob {
        Some(blob) => bundle::read_tar(&blob)
            .map_err(fail)?
            .iter()
            .map(|f| json!({ "path": f.path, "size": f.bytes.len(), "executable": f.executable }))
            .collect(),
        None => Vec::new(),
    };
    Ok(Json(json!({
        "kind": item.kind,
        "name": item.name,
        "description": item.description(),
        "source": item.source,
        "created_at": item.created_at,
        "archived_at": item.archived_at,
        "versions": item.versions,
        "files": files,
    }))
    .into_response())
}

/// `GET /api/library/items/{kind}/{name}/versions/{number}/files/{*path}`: one file of one
/// version, as it was sent. Served as plain text, or as bytes where it is not text, and under
/// a sandbox: a skill's HTML file must never run as a page of the router's origin.
pub async fn file(
    State(state): State<Arc<AppState>>,
    Path((kind, name, number, path)): Path<(String, String, String, String)>,
) -> Answer {
    let kind = kind_of(&kind)?;
    let number = number_of(&number)?;
    let looked_up = name.clone();
    let blob = read(&state, move |conn| {
        let Some(version) = store::get(conn, kind, &looked_up, number)? else { return Ok(None) };
        store::blob(conn, &version.digest)
    })
    .await?;
    let Some(blob) = blob else { return Err(no_version(kind, &name, number)) };
    let files = bundle::read_tar(&blob).map_err(fail)?;
    let Some(found) = files.into_iter().find(|f| f.path == path) else {
        return Err(refuse(StatusCode::NOT_FOUND, format!("version {number} of the {kind} {name} holds no file {path}")).into());
    };
    let text = std::str::from_utf8(&found.bytes).is_ok();
    Ok((
        [
            (header::CONTENT_TYPE, if text { "text/plain; charset=utf-8" } else { "application/octet-stream" }),
            (header::CONTENT_SECURITY_POLICY, "sandbox"),
            // A version never changes, so neither does a file of it.
            (header::CACHE_CONTROL, "private, max-age=31536000, immutable"),
        ],
        found.bytes,
    )
        .into_response())
}

/// Who is publishing, which decides the version's provenance and whether it goes live.
enum Publisher {
    /// The console, with the password: live at once.
    Console,
    /// A connected machine, with the password it holds and its own id
    /// (`agent-router library publish`): live only if the owner trusts machine publishes.
    Machine { machine: String, label: Option<String>, session: Option<String> },
    /// A publish token, within its scope: live as the token says.
    Token(super::tokens::PublishToken),
}

/// The headers `agent-router library publish` names its machine and session with.
pub const MACHINE: &str = "x-agent-router-machine";
pub const LABEL: &str = "x-agent-router-label";
pub const SESSION: &str = "x-claude-code-session";

/// `POST /api/library/items/{kind}/{name}/versions`: publish a version — a `.tar.gz`, a
/// `.zip`, a `.tar` or one Markdown file (`Bundle::from_upload`) — from the console, from a
/// connected machine (`agent-router library publish`), or with a publish token, which the
/// guard lets through on this route alone. A console upload goes live at once, and content
/// that is the item's newest version already is made live rather than stored twice; a
/// machine's or a token's goes live only as the owner trusts it. A body over [`UPLOAD_MAX`]
/// is refused before it is read whole.
pub async fn publish(
    State(state): State<Arc<AppState>>,
    Path((kind, name)): Path<(String, String)>,
    token: Option<axum::Extension<super::tokens::PublishToken>>,
    headers: HeaderMap,
    body: Body,
) -> Answer {
    let kind = kind_of(&kind)?;
    let text = |h: &str| headers.get(h).and_then(|v| v.to_str().ok()).map(str::trim).filter(|v| !v.is_empty()).map(str::to_string);
    let publisher = match (token, text(MACHINE)) {
        (Some(axum::Extension(token)), _) => {
            if !token.allows(kind, &name) {
                return Err(refuse(
                    StatusCode::FORBIDDEN,
                    format!("the token {} may publish {}, and not the {kind} {name}", token.name, token.scope()),
                )
                .into());
            }
            Publisher::Token(token)
        }
        (None, Some(machine)) => Publisher::Machine { machine, label: text(LABEL), session: text(SESSION) },
        (None, None) => Publisher::Console,
    };
    let declared = headers.get(header::CONTENT_LENGTH).and_then(|v| v.to_str().ok()?.parse::<u64>().ok());
    if declared.is_some_and(|n| n > UPLOAD_MAX as u64) {
        return Err(too_large());
    }
    let upload = axum::body::to_bytes(body, UPLOAD_MAX).await.map_err(|e| {
        let over = std::error::Error::source(&e).is_some_and(|s| s.is::<http_body_util::LengthLimitError>());
        if over {
            too_large()
        } else {
            refuse(StatusCode::BAD_REQUEST, format!("the upload could not be read: {e}")).into()
        }
    })?;
    let bundle = Bundle::from_upload(kind, &name, &upload).map_err(|why| Refusal::from(refuse(StatusCode::UNPROCESSABLE_ENTITY, why.0)))?;
    let (provenance, live) = match &publisher {
        Publisher::Console => (Provenance::Upload, true),
        Publisher::Machine { machine, label, session } => (
            Provenance::Machine { machine: machine.clone(), label: label.clone(), session: session.clone() },
            super::tokens::machine_publishes_live(&state.config.db_path),
        ),
        Publisher::Token(token) => (Provenance::Token { token: token.name.clone() }, token.live),
    };
    let console = matches!(publisher, Publisher::Console);
    let token_id = match &publisher {
        Publisher::Token(token) => Some(token.id.clone()),
        _ => None,
    };
    let published = write(&state, move |conn| {
        let now = crate::db::now_ms();
        if let Some(id) = &token_id {
            super::tokens::used(conn, id, now)?;
        }
        Ok(match store::publish(conn, &bundle, &provenance, live, now)? {
            Published::New(v) => Ok((true, v)),
            Published::Unchanged(v) if !console || v.state == Standing::Live => Ok((false, v)),
            Published::Unchanged(v) => {
                let made = store::make_live(conn, kind, bundle.name(), v.number, now)?;
                Ok((false, made.map_err(|unmade| anyhow::anyhow!("version {} did not go live: {unmade:?}", v.number))?))
            }
            Published::Refused(why) => Err(why),
        })
    })
    .await?;
    let (new, version) = published.map_err(|why| Refusal::from(refuse(StatusCode::CONFLICT, why)))?;
    if version.state == Standing::Live {
        super::push::push(&state);
    }
    let status = if new { StatusCode::CREATED } else { StatusCode::OK };
    Ok((status, Json(json!({ "ok": true, "unchanged": !new, "version": version }))).into_response())
}

/// `POST …/versions/{number}/live`: approve a pending version, or roll back to an older one.
pub async fn make_live(
    State(state): State<Arc<AppState>>,
    Path((kind, name, number)): Path<(String, String, String)>,
) -> Answer {
    let decided = decide(&state, kind, name, number, store::make_live).await?;
    super::push::push(&state);
    Ok(decided)
}

/// `POST …/versions/{number}/reject`: turn down a version waiting for approval.
pub async fn reject(
    State(state): State<Arc<AppState>>,
    Path((kind, name, number)): Path<(String, String, String)>,
) -> Answer {
    decide(&state, kind, name, number, store::reject).await
}

type Decision = fn(&Connection, Kind, &str, i64, i64) -> anyhow::Result<Result<Version, Unmade>>;

async fn decide(state: &Arc<AppState>, kind: String, name: String, number: String, how: Decision) -> Answer {
    let kind = kind_of(&kind)?;
    let number = number_of(&number)?;
    let named = name.clone();
    let decided = write(state, move |conn| how(conn, kind, &named, number, crate::db::now_ms())).await?;
    match decided {
        Ok(version) => Ok(Json(json!({ "ok": true, "version": version })).into_response()),
        Err(Unmade::Missing) => Err(no_version(kind, &name, number)),
        Err(Unmade::Refused(why)) => Err(refuse(StatusCode::CONFLICT, why).into()),
    }
}

/// `POST /api/library/items/{kind}/{name}/archive`: take an item off every machine.
pub async fn archive(
    State(state): State<Arc<AppState>>,
    Path((kind, name)): Path<(String, String)>,
) -> Answer {
    let kind = kind_of(&kind)?;
    let named = name.clone();
    match write(&state, move |conn| store::archive(conn, kind, &named, crate::db::now_ms())).await? {
        Ok(()) => {
            super::push::push(&state);
            Ok(Json(json!({ "ok": true })).into_response())
        }
        Err(_) => Err(no_item_named(kind, &name)),
    }
}

/// `GET /api/library/manifest`: what every machine holds — each item's live version, its
/// digest and its size. An agent sends back the ETag it was given, and an unchanged Library
/// costs it a 304.
pub async fn manifest(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Answer {
    let live = read(&state, store::live).await?;
    let rows: Vec<Value> = live
        .iter()
        .map(|v| json!({ "kind": v.kind, "name": v.name, "version": v.number, "digest": v.digest, "size": v.size }))
        .collect();
    let body = serde_json::to_vec(&json!({ "items": rows })).map_err(fail)?;
    let tag = format!("\"{}\"", &bundle::digest(&body)[..32]);
    let fresh = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.split(',').any(|t| t.trim().trim_start_matches("W/") == tag));
    if fresh {
        return Ok((StatusCode::NOT_MODIFIED, [(header::ETAG, tag)]).into_response());
    }
    Ok((
        [(header::CONTENT_TYPE, "application/json".to_string()), (header::ETAG, tag), (header::CACHE_CONTROL, "no-cache".into())],
        body,
    )
        .into_response())
}

/// `GET /api/library/blobs/{digest}`: a version's canonical bytes, whose sha256 is the
/// digest. They never change under it, so they are cached for good.
pub async fn blob(State(state): State<Arc<AppState>>, Path(digest): Path<String>) -> Answer {
    let missing = || Refusal::from(refuse(StatusCode::NOT_FOUND, format!("the Library holds no blob {digest}")));
    if digest.len() != 64 || !digest.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
        return Err(missing());
    }
    let wanted = digest.clone();
    let bytes = read(&state, move |conn| store::blob(conn, &wanted)).await?.ok_or_else(missing)?;
    Ok((
        [(header::CONTENT_TYPE, "application/x-tar"), (header::CACHE_CONTROL, "private, max-age=31536000, immutable")],
        bytes,
    )
        .into_response())
}

/// A version in a list: what a row shows, without the description each version repeats.
fn brief(v: &Version) -> Value {
    json!({
        "number": v.number,
        "digest": v.digest,
        "size": v.size,
        "state": v.state,
        "provenance": v.provenance,
        "created_at": v.created_at,
    })
}

fn kind_of(kind: &str) -> Result<Kind, Refusal> {
    Kind::parse(kind).ok_or_else(|| {
        refuse(StatusCode::NOT_FOUND, format!("the Library keeps skills, commands and subagents, and no kind called {kind}")).into()
    })
}

fn number_of(number: &str) -> Result<i64, Refusal> {
    number
        .parse::<i64>()
        .ok()
        .filter(|n| *n > 0)
        .ok_or_else(|| refuse(StatusCode::NOT_FOUND, format!("there is no version {number}: versions are numbered from 1")).into())
}

fn no_item_named(kind: Kind, name: &str) -> Refusal {
    refuse(StatusCode::NOT_FOUND, format!("the Library has no {kind} called {name}")).into()
}

fn no_version(kind: Kind, name: &str, number: i64) -> Refusal {
    refuse(StatusCode::NOT_FOUND, format!("the Library has no version {number} of a {kind} called {name}")).into()
}

fn too_large() -> Refusal {
    refuse(
        StatusCode::PAYLOAD_TOO_LARGE,
        format!("an upload may be at most {} MiB, which is what one version may hold", UPLOAD_MAX / (1024 * 1024)),
    )
    .into()
}

/// Run `f` on one of the warm read-only connections, off the async threads.
pub(super) async fn read<T: Send + 'static>(
    state: &Arc<AppState>,
    f: impl FnOnce(&Connection) -> anyhow::Result<T> + Send + 'static,
) -> Result<T, Refusal> {
    let state = state.clone();
    tokio::task::spawn_blocking(move || {
        let conn = state.reads.get()?;
        f(&conn)
    })
    .await
    .map_err(fail)?
    .map_err(fail)
}

/// Run `f` on a connection of its own that may write, off the async threads.
pub(super) async fn write<T: Send + 'static>(
    state: &Arc<AppState>,
    f: impl FnOnce(&Connection) -> anyhow::Result<T> + Send + 'static,
) -> Result<T, Refusal> {
    let path = state.config.db_path.clone();
    tokio::task::spawn_blocking(move || {
        let conn = store::writer(&path)?;
        f(&conn)
    })
    .await
    .map_err(fail)?
    .map_err(fail)
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;
    use std::time::Duration;

    use axum::extract::ConnectInfo;
    use axum::http::{HeaderName, Request};
    use tower::ServiceExt;

    use super::*;
    use crate::api_refusals::{words, Lab};
    use crate::library::bundle::{made, File};

    struct Answer {
        status: StatusCode,
        headers: HeaderMap,
        body: Vec<u8>,
    }

    impl Answer {
        fn json(&self) -> Value {
            serde_json::from_slice(&self.body)
                .unwrap_or_else(|_| panic!("{} with no JSON: {}", self.status, String::from_utf8_lossy(&self.body)))
        }

        fn words(&self, status: StatusCode) -> String {
            words((self.status, self.json()), status)
        }

        fn header(&self, name: HeaderName) -> Option<&str> {
            self.headers.get(name).and_then(|v| v.to_str().ok())
        }
    }

    /// Ask as a page on this machine would: with the password, unless `bare`.
    async fn ask(lab: &Lab, method: &str, path: &str, headers: &[(HeaderName, &str)], body: Body, bare: bool) -> Answer {
        let mut req = Request::builder().method(method).uri(path).header(header::HOST, "127.0.0.1");
        for (name, value) in headers {
            req = req.header(name, *value);
        }
        if let (Some(password), false) = (lab.password, bare) {
            req = req.header(header::AUTHORIZATION, format!("Bearer {password}"));
        }
        let mut req = req.body(body).unwrap();
        req.extensions_mut().insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 50_000))));
        let res = lab.app.clone().oneshot(req).await.unwrap();
        let (status, headers) = (res.status(), res.headers().clone());
        let body = axum::body::to_bytes(res.into_body(), 64 << 20).await.unwrap().to_vec();
        Answer { status, headers, body }
    }

    async fn get(lab: &Lab, path: &str) -> Answer {
        ask(lab, "GET", path, &[], Body::empty(), false).await
    }

    async fn post(lab: &Lab, path: &str, body: impl Into<Body>) -> Answer {
        ask(lab, "POST", path, &[], body.into(), false).await
    }

    fn file(path: &str, text: &str) -> File {
        File { path: path.into(), bytes: text.as_bytes().to_vec(), executable: false }
    }

    fn skill_md(name: &str, description: &str) -> String {
        format!("---\nname: {name}\ndescription: {description}\n---\n\n# {name}\n")
    }

    /// Upload a skill of one SKILL.md from the console, and answer the version it made.
    async fn upload(lab: &Lab, name: &str, description: &str) -> Value {
        let sent = post(lab, &format!("/api/library/items/skill/{name}/versions"), skill_md(name, description)).await;
        assert!(sent.status.is_success(), "{}: {}", sent.status, String::from_utf8_lossy(&sent.body));
        sent.json()["version"].clone()
    }

    /// Store a version that waits for approval, as an untrusted source's would.
    fn pending(lab: &Lab, kind: Kind, name: &str, files: Vec<File>) -> Version {
        let conn = Connection::open(&lab.state.config.db_path).unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        let bundle = Bundle::check(kind, name, files).unwrap();
        store::publish(&conn, &bundle, &Provenance::Token { token: "ci".into() }, false, 1).unwrap().version().clone()
    }

    async fn manifest_of(lab: &Lab) -> Vec<(String, String, i64)> {
        let items = get(lab, "/api/library/manifest").await.json()["items"].as_array().unwrap().clone();
        items
            .iter()
            .map(|i| (i["kind"].as_str().unwrap().into(), i["name"].as_str().unwrap().into(), i["version"].as_i64().unwrap()))
            .collect()
    }

    fn manifest(rows: &[(&str, &str, i64)]) -> Vec<(String, String, i64)> {
        rows.iter().map(|(k, n, v)| (k.to_string(), n.to_string(), *v)).collect()
    }

    async fn states(lab: &Lab, name: &str) -> Vec<(i64, String)> {
        let item = get(lab, &format!("/api/library/items/skill/{name}")).await.json();
        item["versions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| (v["number"].as_i64().unwrap(), v["state"].as_str().unwrap().to_string()))
            .collect()
    }

    #[tokio::test]
    async fn an_upload_goes_live_and_reads_back_as_sent() {
        let lab = Lab::hosted("library-upload").await;
        let skill = skill_md("bench", "Drives the bench.");
        let script = File { path: "bench/scripts/run.sh".into(), bytes: b"#!/bin/sh\necho hi\n".to_vec(), executable: true };
        // As `tar czf` makes it on a Mac: a folder around the item, Finder's files beside it.
        let upload = made::tar_gz(&[
            file("bench/SKILL.md", &skill),
            script,
            file("bench/._SKILL.md", "apple double"),
            file("bench/.DS_Store", "finder"),
        ]);
        let sent = post(&lab, "/api/library/items/skill/bench/versions", upload.clone()).await;
        assert_eq!(sent.status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&sent.body));
        let version = sent.json()["version"].clone();
        assert_eq!((&version["number"], &version["state"], &version["provenance"]["from"]), (&json!(1), &json!("live"), &json!("upload")));

        let listed = get(&lab, "/api/library").await.json();
        let row = &listed["items"][0];
        assert_eq!((&row["name"], &row["description"], &row["live"]["number"], &row["pending"]), (&json!("bench"), &json!("Drives the bench."), &json!(1), &json!(0)));
        let item = get(&lab, "/api/library/items/skill/bench").await.json();
        assert_eq!(
            item["files"],
            json!([
                { "path": "SKILL.md", "size": skill.len(), "executable": false },
                { "path": "scripts/run.sh", "size": 18, "executable": true },
            ])
        );
        let read = get(&lab, "/api/library/items/skill/bench/versions/1/files/SKILL.md").await;
        assert_eq!((read.status, read.body.as_slice()), (StatusCode::OK, skill.as_bytes()));
        assert_eq!(read.header(header::CONTENT_TYPE), Some("text/plain; charset=utf-8"));
        assert_eq!(read.header(header::CONTENT_SECURITY_POLICY), Some("sandbox"));
        let read = get(&lab, "/api/library/items/skill/bench/versions/1/files/scripts/run.sh").await;
        assert_eq!(read.body, b"#!/bin/sh\necho hi\n");
        let why = get(&lab, "/api/library/items/skill/bench/versions/1/files/nothing.md").await.words(StatusCode::NOT_FOUND);
        assert!(why.contains("holds no file nothing.md"), "{why}");

        // The same upload again is the same version, still live.
        let again = post(&lab, "/api/library/items/skill/bench/versions", upload).await;
        assert_eq!(again.status, StatusCode::OK);
        assert_eq!((&again.json()["unchanged"], &again.json()["version"]["number"]), (&json!(true), &json!(1)));

        // A zip, as claude.ai and Finder make them, and a command sent as its one file.
        let vault = skill_md("vault", "Keeps secrets.");
        let zipped = made::zip(&[file("vault/SKILL.md", &vault), file("__MACOSX/vault/._SKILL.md", "apple double")]);
        assert_eq!(post(&lab, "/api/library/items/skill/vault/versions", zipped).await.status, StatusCode::CREATED);
        assert_eq!(get(&lab, "/api/library/items/skill/vault/versions/1/files/SKILL.md").await.body, vault.as_bytes());
        let command = "---\ndescription: Ship it.\n---\nRun the release.\n";
        assert_eq!(post(&lab, "/api/library/items/command/ship/versions", command).await.status, StatusCode::CREATED);
        let item = get(&lab, "/api/library/items/command/ship").await.json();
        assert_eq!(item["files"][0]["path"], "ship.md");
        assert_eq!(get(&lab, "/api/library/items/command/ship/versions/1/files/ship.md").await.body, command.as_bytes());
    }

    #[tokio::test]
    async fn approving_a_pending_version_makes_it_live() {
        let lab = Lab::hosted("library-approve").await;
        upload(&lab, "bench", "One.").await;
        let waiting = pending(&lab, Kind::Skill, "bench", vec![file("SKILL.md", &skill_md("bench", "Two."))]);
        assert_eq!((waiting.number, waiting.state), (2, Standing::Pending));
        assert_eq!(get(&lab, "/api/library").await.json()["items"][0]["pending"], 1);
        assert_eq!(manifest_of(&lab).await, manifest(&[("skill", "bench", 1)]), "a pending version reaches no machine");

        let approved = post(&lab, "/api/library/items/skill/bench/versions/2/live", Body::empty()).await;
        assert_eq!(approved.status, StatusCode::OK, "{}", String::from_utf8_lossy(&approved.body));
        assert_eq!(approved.json()["version"]["state"], "live");
        assert_eq!(states(&lab, "bench").await, [(2, "live".into()), (1, "superseded".into())]);
        assert_eq!(manifest_of(&lab).await, manifest(&[("skill", "bench", 2)]));
    }

    #[tokio::test]
    async fn rolling_back_makes_an_older_version_live_again() {
        let lab = Lab::hosted("library-rollback").await;
        let first = upload(&lab, "bench", "One.").await;
        upload(&lab, "bench", "Two.").await;
        assert_eq!(manifest_of(&lab).await, manifest(&[("skill", "bench", 2)]));

        let back = post(&lab, "/api/library/items/skill/bench/versions/1/live", Body::empty()).await;
        assert_eq!(back.status, StatusCode::OK);
        assert_eq!(states(&lab, "bench").await, [(2, "superseded".into()), (1, "live".into())]);
        let held = get(&lab, "/api/library/manifest").await.json();
        assert_eq!((&held["items"][0]["version"], &held["items"][0]["digest"]), (&json!(1), &first["digest"]));

        let why = post(&lab, "/api/library/items/skill/bench/versions/7/live", Body::empty()).await.words(StatusCode::NOT_FOUND);
        assert!(why.contains("no version 7 of a skill called bench"), "{why}");
        let why = post(&lab, "/api/library/items/skill/bench/versions/latest/live", Body::empty()).await.words(StatusCode::NOT_FOUND);
        assert!(why.contains("numbered from 1"), "{why}");
    }

    #[tokio::test]
    async fn a_rejected_version_never_goes_live() {
        let lab = Lab::hosted("library-reject").await;
        upload(&lab, "bench", "One.").await;
        let second = vec![file("SKILL.md", &skill_md("bench", "Two."))];
        pending(&lab, Kind::Skill, "bench", second.clone());

        let rejected = post(&lab, "/api/library/items/skill/bench/versions/2/reject", Body::empty()).await;
        assert_eq!((rejected.status, &rejected.json()["version"]["state"]), (StatusCode::OK, &json!("rejected")));
        assert_eq!(manifest_of(&lab).await, manifest(&[("skill", "bench", 1)]));
        let why = post(&lab, "/api/library/items/skill/bench/versions/2/live", Body::empty()).await.words(StatusCode::CONFLICT);
        assert!(why.contains("was rejected"), "{why}");

        // The same content sent to wait again is not asked about again.
        let again = pending(&lab, Kind::Skill, "bench", second);
        assert_eq!((again.number, again.state), (2, Standing::Rejected));
        assert_eq!(manifest_of(&lab).await, manifest(&[("skill", "bench", 1)]));

        let why = post(&lab, "/api/library/items/skill/bench/versions/1/reject", Body::empty()).await.words(StatusCode::CONFLICT);
        assert!(why.contains("only a pending version"), "{why}");
        post(&lab, "/api/library/items/skill/bench/versions/9/reject", Body::empty()).await.words(StatusCode::NOT_FOUND);

        // The owner uploading it after all is a new decision: a new version, live.
        let mine = upload(&lab, "bench", "Two.").await;
        assert_eq!((&mine["number"], &mine["state"]), (&json!(3), &json!("live")));
    }

    #[tokio::test]
    async fn an_archived_item_leaves_the_manifest() {
        let lab = Lab::hosted("library-archive").await;
        upload(&lab, "bench", "One.").await;
        upload(&lab, "vault", "Keeps secrets.").await;
        assert_eq!(manifest_of(&lab).await, manifest(&[("skill", "bench", 1), ("skill", "vault", 1)]));

        let archived = post(&lab, "/api/library/items/skill/bench/archive", Body::empty()).await;
        assert_eq!(archived.status, StatusCode::OK);
        assert_eq!(manifest_of(&lab).await, manifest(&[("skill", "vault", 1)]));
        assert!(get(&lab, "/api/library").await.json()["items"][0]["archived_at"].is_i64());
        post(&lab, "/api/library/items/skill/nothing/archive", Body::empty()).await.words(StatusCode::NOT_FOUND);

        // Going live again brings it back.
        upload(&lab, "bench", "Two.").await;
        assert_eq!(manifest_of(&lab).await, manifest(&[("skill", "bench", 2), ("skill", "vault", 1)]));
        assert!(get(&lab, "/api/library").await.json()["items"][0]["archived_at"].is_null());
    }

    #[tokio::test]
    async fn the_manifest_lists_only_live_versions() {
        let lab = Lab::hosted("library-manifest").await;
        let bench = upload(&lab, "bench", "One.").await;
        pending(&lab, Kind::Skill, "bench", vec![file("SKILL.md", &skill_md("bench", "Two."))]);
        let command = "---\ndescription: Ship it.\n---\nRun the release.\n";
        assert_eq!(post(&lab, "/api/library/items/command/ship/versions", command).await.status, StatusCode::CREATED);
        let subagent = "---\nname: reviewer\ndescription: Reviews a diff.\n---\nYou review.\n";
        pending(&lab, Kind::Subagent, "reviewer", vec![file("reviewer.md", subagent)]);

        assert_eq!(manifest_of(&lab).await, manifest(&[("command", "ship", 1), ("skill", "bench", 1)]));
        let held = get(&lab, "/api/library/manifest").await.json();
        let row = &held["items"][1];
        assert_eq!((&row["digest"], &row["size"]), (&bench["digest"], &bench["size"]));
    }

    #[tokio::test]
    async fn the_manifest_answers_not_modified_to_its_etag() {
        let lab = Lab::hosted("library-etag").await;
        upload(&lab, "bench", "One.").await;
        let first = get(&lab, "/api/library/manifest").await;
        let tag = first.header(header::ETAG).expect("an ETag").to_string();

        for sent in [tag.clone(), format!("W/{tag}"), format!("\"other\", {tag}")] {
            let again = ask(&lab, "GET", "/api/library/manifest", &[(header::IF_NONE_MATCH, &sent)], Body::empty(), false).await;
            assert_eq!((again.status, again.body.len()), (StatusCode::NOT_MODIFIED, 0), "{sent}");
            assert_eq!(again.header(header::ETAG), Some(tag.as_str()));
        }

        upload(&lab, "bench", "Two.").await;
        let changed = ask(&lab, "GET", "/api/library/manifest", &[(header::IF_NONE_MATCH, &tag)], Body::empty(), false).await;
        assert_eq!(changed.status, StatusCode::OK);
        assert_ne!(changed.header(header::ETAG), Some(tag.as_str()));
    }

    #[tokio::test]
    async fn a_blob_downloads_with_its_digest() {
        let lab = Lab::hosted("library-blob").await;
        let version = upload(&lab, "bench", "One.").await;
        let digest = version["digest"].as_str().unwrap();
        let got = get(&lab, &format!("/api/library/blobs/{digest}")).await;
        assert_eq!(got.status, StatusCode::OK);
        assert_eq!(got.header(header::CONTENT_TYPE), Some("application/x-tar"));
        assert_eq!(got.header(header::CACHE_CONTROL), Some("private, max-age=31536000, immutable"));
        assert_eq!(bundle::digest(&got.body), digest);
        let unpacked = Bundle::unpack(Kind::Skill, "bench", &got.body).unwrap();
        assert_eq!(unpacked.description(), "One.");

        get(&lab, &format!("/api/library/blobs/{}", "0".repeat(64))).await.words(StatusCode::NOT_FOUND);
        get(&lab, "/api/library/blobs/not-a-digest").await.words(StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn library_routes_need_the_password() {
        let lab = Lab::hosted("library-password").await;
        let zeros = "0".repeat(64);
        let blob = format!("/api/library/blobs/{zeros}");
        for (method, path) in [
            ("GET", "/api/library"),
            ("GET", "/api/library/manifest"),
            ("GET", blob.as_str()),
            ("GET", "/api/library/items/skill/bench"),
            ("GET", "/api/library/items/skill/bench/versions/1/files/SKILL.md"),
            ("POST", "/api/library/items/skill/bench/versions"),
            ("POST", "/api/library/items/skill/bench/versions/1/live"),
            ("POST", "/api/library/items/skill/bench/versions/1/reject"),
            ("POST", "/api/library/items/skill/bench/archive"),
        ] {
            let answer = ask(&lab, method, path, &[], Body::from(skill_md("bench", "One.")), true).await;
            assert_eq!(answer.status, StatusCode::UNAUTHORIZED, "{method} {path}");
        }
        assert_eq!(get(&lab, "/api/library").await.json()["items"], json!([]), "nothing was stored");
    }

    #[tokio::test]
    async fn library_writes_refuse_a_bad_body_in_words() {
        let lab = Lab::hosted("library-bad-body").await;
        let to = "/api/library/items/skill/bench/versions";
        let why = post(&lab, to, vec![0xff, 0xfe, 0, 1, 2]).await.words(StatusCode::UNPROCESSABLE_ENTITY);
        assert!(why.contains("not a .tar.gz, a .zip, a .tar, nor a Markdown file"), "{why}");
        let why = post(&lab, to, made::tar_gz(&[file("README.md", "# bench\n")])).await.words(StatusCode::UNPROCESSABLE_ENTITY);
        assert!(why.contains("needs a SKILL.md"), "{why}");
        let why = post(&lab, to, vec![0x1f, 0x8b, 8, 0, 1, 2, 3]).await.words(StatusCode::UNPROCESSABLE_ENTITY);
        assert!(why.contains("cannot be read"), "{why}");
        let mut zipped = made::zip(&[file("SKILL.md", &skill_md("bench", "One."))]);
        zipped[40] ^= 0xff;
        let why = post(&lab, to, zipped).await.words(StatusCode::UNPROCESSABLE_ENTITY);
        assert!(why.contains("damaged"), "{why}");
        let why = post(&lab, to, "# bench, with no frontmatter\n").await.words(StatusCode::UNPROCESSABLE_ENTITY);
        assert!(why.contains("has no frontmatter"), "{why}");
        let why = post(&lab, "/api/library/items/skill/Bench/versions", skill_md("Bench", "One.")).await.words(StatusCode::UNPROCESSABLE_ENTITY);
        assert!(why.contains("cannot name an item"), "{why}");
        let why = post(&lab, "/api/library/items/plugin/bench/versions", skill_md("bench", "One.")).await.words(StatusCode::NOT_FOUND);
        assert!(why.contains("no kind called plugin"), "{why}");
        assert_eq!(get(&lab, "/api/library").await.json()["items"], json!([]), "nothing was stored");
    }

    #[tokio::test]
    async fn an_upload_over_the_limit_is_refused_unread() {
        use futures_util::StreamExt;
        let lab = Lab::hosted("library-limit").await;
        let to = "/api/library/items/skill/bench/versions";

        // Declared too big: refused on the header, while its body is still arriving — this one
        // never finishes.
        let endless = futures_util::stream::once(async { Ok::<_, std::io::Error>(bytes::Bytes::from(vec![b'x'; 64 * 1024])) })
            .chain(futures_util::stream::pending());
        let declared = (UPLOAD_MAX + 1).to_string();
        let answer = tokio::time::timeout(
            Duration::from_secs(5),
            ask(&lab, "POST", to, &[(header::CONTENT_LENGTH, &declared)], Body::from_stream(endless), false),
        )
        .await
        .expect("answered without waiting for the body");
        assert!(answer.words(StatusCode::PAYLOAD_TOO_LARGE).contains("at most 8 MiB"));

        // Not declared: refused once more than the limit has arrived.
        let why = post(&lab, to, vec![b'x'; UPLOAD_MAX + 1]).await.words(StatusCode::PAYLOAD_TOO_LARGE);
        assert!(why.contains("at most 8 MiB"), "{why}");
        assert_eq!(get(&lab, "/api/library").await.json()["items"], json!([]), "nothing was stored");
    }

    #[tokio::test]
    async fn a_local_router_does_not_offer_the_library() {
        let lab = Lab::local("library-local");
        for (method, path) in [
            ("GET", "/api/library"),
            ("GET", "/api/library/manifest"),
            ("GET", "/api/library/blobs/abc"),
            ("GET", "/api/library/items/skill/bench"),
            ("POST", "/api/library/items/skill/bench/versions"),
            ("POST", "/api/library/items/skill/bench/archive"),
        ] {
            let answer = ask(&lab, method, path, &[], Body::from(skill_md("bench", "One.")), false).await;
            let why = answer.words(StatusCode::CONFLICT);
            assert!(why.contains("the Library is a hosted router's"), "{method} {path}: {why}");
        }
    }
}
