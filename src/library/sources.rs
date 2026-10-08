//! Where the Library imports from: forges, each a base URL with a token, and sources, each a
//! repository, a ref and a path on one. docs/library.md, "Forges and sources".
//!
//! Importing a source asks its forge for the ref's head and, when it moved, the files under
//! its path; every skill directory, `commands/*.md` and `agents/*.md` there is an item, and
//! each whose content changed becomes a version, live or waiting as the source is trusted. An
//! item a source creates is that source's alone (`store::publish`). The poller does this for
//! every source every five minutes; a forge's push webhook (AGEN-0048) does it at once.

use std::collections::{BTreeSet, HashSet};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Path as Params, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::api::Refusal;
use super::bundle::{Bundle, File};
use super::forge::{self, Forge, ForgeError, ForgeKind};
use super::store::{self, Provenance, Published, State as Standing, Unmade};
use super::{Kind, Refused};
use crate::proxy::AppState;
use crate::refusal::{failed, refuse, JsonBody};

/// How often the poller asks every source's forge whether its ref moved.
pub const POLL_EVERY: Duration = Duration::from_secs(5 * 60);

/// A forge as reads show it: never its token, only whether one is kept.
#[derive(Clone, Debug, Serialize)]
pub struct ForgeRow {
    pub key: String,
    pub kind: ForgeKind,
    pub base_url: String,
    pub token_present: bool,
    pub sources: i64,
    pub created_at: i64,
}

/// A source as reads show it.
#[derive(Clone, Debug, Serialize)]
pub struct SourceRow {
    pub key: String,
    pub forge: String,
    pub repository: String,
    #[serde(rename = "ref")]
    pub reference: String,
    pub path: String,
    /// The owner's trust: its new versions go live, or wait for approval.
    pub live: bool,
    /// The commit last imported.
    pub head: Option<String>,
    pub checked_at: Option<i64>,
    pub imported_at: Option<i64>,
    pub last_result: Option<Value>,
    pub created_at: i64,
}

/// What to do with a forge's token when it is written.
pub enum Token {
    Keep,
    Set(String),
    Clear,
}

/// A key a forge or a source is known by: the same shape as an item's name.
fn check_key(key: &str) -> Result<(), Refused> {
    super::check_name(key).map_err(|_| {
        Refused(format!("\"{key}\" cannot be a key: use lowercase letters and digits, with single hyphens between them"))
    })
}

pub fn forges(conn: &Connection) -> anyhow::Result<Vec<ForgeRow>> {
    let mut stmt = conn.prepare(
        "SELECT f.key, f.kind, f.base_url, t.forge IS NOT NULL, (SELECT count(*) FROM library_sources s WHERE s.forge = f.key), f.created_at
         FROM library_forges f LEFT JOIN library_forge_tokens t ON t.forge = f.key ORDER BY f.key",
    )?;
    let rows = stmt.query_map([], |r| {
        let kind: String = r.get(1)?;
        Ok(ForgeRow {
            key: r.get(0)?,
            kind: ForgeKind::parse(&kind).unwrap_or(ForgeKind::GitLab),
            base_url: r.get(2)?,
            token_present: r.get(3)?,
            sources: r.get(4)?,
            created_at: r.get(5)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// A forge with its token, for reaching it. Never answered to anyone.
fn forge(conn: &Connection, key: &str) -> anyhow::Result<Option<Forge>> {
    Ok(conn
        .query_row(
            "SELECT f.kind, f.base_url, t.token FROM library_forges f LEFT JOIN library_forge_tokens t ON t.forge = f.key WHERE f.key = ?1",
            [key],
            |r| {
                let kind: String = r.get(0)?;
                Ok(Forge {
                    key: key.to_string(),
                    kind: ForgeKind::parse(&kind).unwrap_or(ForgeKind::GitLab),
                    base_url: r.get(1)?,
                    token: r.get(2)?,
                })
            },
        )
        .optional()?)
}

/// Add or change a forge. A token never follows a forge to another address: moving the base
/// URL drops it unless a new one is sent.
pub fn put_forge(conn: &Connection, key: &str, kind: ForgeKind, base_url: &str, token: Token, now: i64) -> anyhow::Result<Result<(), Refused>> {
    if let Err(why) = check_key(key) {
        return Ok(Err(why));
    }
    let base_url = match forge::check_base_url(base_url) {
        Ok(url) => url,
        Err(why) => return Ok(Err(why)),
    };
    let tx = rusqlite::Transaction::new_unchecked(conn, rusqlite::TransactionBehavior::Immediate)?;
    let taken: Option<String> =
        tx.query_row("SELECT key FROM library_forges WHERE base_url = ?1 AND key != ?2", params![base_url, key], |r| r.get(0)).optional()?;
    if let Some(other) = taken {
        return Ok(Err(Refused(format!("the forge {other} is at {base_url} already: one forge per address"))));
    }
    let before: Option<String> = tx.query_row("SELECT base_url FROM library_forges WHERE key = ?1", [key], |r| r.get(0)).optional()?;
    tx.execute(
        "INSERT INTO library_forges (key, kind, base_url, created_at) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(key) DO UPDATE SET kind = excluded.kind, base_url = excluded.base_url",
        params![key, kind.as_str(), base_url, now],
    )?;
    let moved = before.is_some_and(|b| b != base_url);
    match token {
        Token::Set(token) => {
            tx.execute(
                "INSERT INTO library_forge_tokens (forge, token, set_at) VALUES (?1, ?2, ?3)
                 ON CONFLICT(forge) DO UPDATE SET token = excluded.token, set_at = excluded.set_at",
                params![key, token, now],
            )?;
        }
        Token::Clear => {
            tx.execute("DELETE FROM library_forge_tokens WHERE forge = ?1", [key])?;
        }
        Token::Keep if moved => {
            tx.execute("DELETE FROM library_forge_tokens WHERE forge = ?1", [key])?;
        }
        Token::Keep => {}
    }
    tx.commit()?;
    Ok(Ok(()))
}

pub fn delete_forge(conn: &Connection, key: &str) -> anyhow::Result<Result<(), Unmade>> {
    let used: i64 = conn.query_row("SELECT count(*) FROM library_sources WHERE forge = ?1", [key], |r| r.get(0))?;
    if used > 0 {
        return Ok(Err(Unmade::Refused(format!("{used} source(s) import from the forge {key}: remove them first"))));
    }
    Ok(if conn.execute("DELETE FROM library_forges WHERE key = ?1", [key])? == 1 { Ok(()) } else { Err(Unmade::Missing) })
}

const SOURCE_COLUMNS: &str = "key, forge, repository, ref, path, live, head, checked_at, imported_at, last_result, created_at";

fn source_row(r: &rusqlite::Row) -> rusqlite::Result<SourceRow> {
    let result: Option<String> = r.get(9)?;
    Ok(SourceRow {
        key: r.get(0)?,
        forge: r.get(1)?,
        repository: r.get(2)?,
        reference: r.get(3)?,
        path: r.get(4)?,
        live: r.get(5)?,
        head: r.get(6)?,
        checked_at: r.get(7)?,
        imported_at: r.get(8)?,
        last_result: result.and_then(|r| serde_json::from_str(&r).ok()),
        created_at: r.get(10)?,
    })
}

pub fn sources(conn: &Connection) -> anyhow::Result<Vec<SourceRow>> {
    let mut stmt = conn.prepare(&format!("SELECT {SOURCE_COLUMNS} FROM library_sources ORDER BY key"))?;
    let rows = stmt.query_map([], source_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn source(conn: &Connection, key: &str) -> anyhow::Result<Option<SourceRow>> {
    Ok(conn.query_row(&format!("SELECT {SOURCE_COLUMNS} FROM library_sources WHERE key = ?1"), [key], source_row).optional()?)
}

/// What a source is: where it imports from, and whether its versions go live.
#[derive(Deserialize)]
pub struct SourceIn {
    pub forge: String,
    pub repository: String,
    #[serde(rename = "ref")]
    pub reference: String,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub live: bool,
}

pub fn put_source(conn: &Connection, key: &str, src: &SourceIn, now: i64) -> anyhow::Result<Result<(), Refused>> {
    if let Err(why) = check_key(key) {
        return Ok(Err(why));
    }
    let Some(at) = forge(conn, &src.forge)? else {
        return Ok(Err(Refused(format!("there is no forge {}: add it first", src.forge))));
    };
    if let Err(why) = forge::check_repository(at.kind, &src.repository) {
        return Ok(Err(why));
    }
    if src.reference.trim().is_empty() {
        return Ok(Err(Refused("a source needs a ref: a branch, a tag or a commit".into())));
    }
    let path = src.path.trim().trim_matches('/');
    if path.split('/').any(|part| part == ".." || part == ".") || path.contains('\\') {
        return Ok(Err(Refused(format!("{path} is not a path inside the repository"))));
    }
    conn.execute(
        "INSERT INTO library_sources (key, forge, repository, ref, path, live, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(key) DO UPDATE SET forge = excluded.forge, repository = excluded.repository, ref = excluded.ref,
           path = excluded.path, live = excluded.live,
           head = CASE WHEN library_sources.repository = excluded.repository AND library_sources.path = excluded.path
                       THEN library_sources.head END",
        params![key, src.forge, src.repository, src.reference.trim(), path, src.live, now],
    )?;
    Ok(Ok(()))
}

/// Remove a source. Its items stay, and become anyone's to change: uploads and publishes
/// may add to them from then on.
pub fn delete_source(conn: &Connection, key: &str) -> anyhow::Result<Result<(), Unmade>> {
    let tx = rusqlite::Transaction::new_unchecked(conn, rusqlite::TransactionBehavior::Immediate)?;
    if tx.execute("DELETE FROM library_sources WHERE key = ?1", [key])? != 1 {
        return Ok(Err(Unmade::Missing));
    }
    tx.execute("UPDATE library_items SET source = NULL WHERE source = ?1", [format!("source:{key}")])?;
    tx.commit()?;
    Ok(Ok(()))
}

/// An item found under a source's path.
#[derive(Debug, Clone)]
pub struct Found {
    pub kind: Kind,
    pub name: String,
    /// Where it is in the repository.
    pub path: String,
    pub files: Vec<File>,
}

fn join(a: &str, b: &str) -> String {
    match (a.is_empty(), b.is_empty()) {
        (true, _) => b.to_string(),
        (_, true) => a.to_string(),
        _ => format!("{a}/{b}"),
    }
}

/// The items under a source's path, its files given relative to that path:
/// - every directory holding a `SKILL.md` is a skill, named by the directory — the outermost,
///   where one sits inside another — and the path itself, when it holds one, is the one skill;
/// - `commands/<name>.md` is a command, and `agents/<name>.md` a subagent.
pub fn discover(files: Vec<File>, source_path: &str, repository: &str) -> Vec<Found> {
    let skill_dirs: BTreeSet<String> = files
        .iter()
        .filter_map(|f| if f.path == "SKILL.md" { Some(String::new()) } else { f.path.strip_suffix("/SKILL.md").map(str::to_string) })
        .collect();
    let outermost: Vec<String> = skill_dirs
        .iter()
        .filter(|d| !skill_dirs.iter().any(|o| o != *d && (o.is_empty() || d.starts_with(&format!("{o}/")))))
        .filter(|d| !d.starts_with("commands/") && !d.starts_with("agents/"))
        .cloned()
        .collect();
    let mut found = Vec::new();
    for dir in &outermost {
        let name = if dir.is_empty() {
            let whole = if source_path.is_empty() { repository } else { source_path };
            whole.rsplit('/').next().unwrap_or(whole).to_string()
        } else {
            dir.rsplit('/').next().unwrap_or(dir).to_string()
        };
        let prefix = if dir.is_empty() { String::new() } else { format!("{dir}/") };
        let mine = files
            .iter()
            .filter_map(|f| f.path.strip_prefix(&prefix).map(|rest| File { path: rest.to_string(), ..f.clone() }))
            .collect();
        found.push(Found { kind: Kind::Skill, name, path: join(source_path, dir), files: mine });
    }
    if !outermost.iter().any(String::is_empty) {
        for file in &files {
            let parts: Vec<&str> = file.path.split('/').collect();
            let kind = match parts.as_slice() {
                ["commands", one] if one.ends_with(".md") => Kind::Command,
                ["agents", one] if one.ends_with(".md") => Kind::Subagent,
                _ => continue,
            };
            let name = parts[1].trim_end_matches(".md").to_string();
            let only = File { path: format!("{name}.md"), ..file.clone() };
            found.push(Found { kind, name, path: join(source_path, &file.path), files: vec![only] });
        }
    }
    found.sort_by(|a, b| (a.kind.as_str(), &a.name).cmp(&(b.kind.as_str(), &b.name)));
    found
}

/// What importing a source did.
#[derive(Clone, Debug, Serialize)]
pub struct Imported {
    pub source: String,
    pub commit: String,
    /// Whether the ref's head had moved since the last import (or the import was asked for).
    pub fetched: bool,
    pub items: Vec<ImportLine>,
    /// Items this source made that its path no longer holds. They stay, and are shown here.
    pub gone: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ImportLine {
    pub kind: Kind,
    pub name: String,
    pub path: String,
    /// `new`, `unchanged`, `refused` (another source of truth owns it) or `failed` (it is not
    /// a valid item).
    pub outcome: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<Standing>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
}

impl Imported {
    /// Whether anything went live, which machines should hear about.
    pub fn went_live(&self) -> bool {
        self.items.iter().any(|l| l.outcome == "new" && l.state == Some(Standing::Live))
    }
}

/// Import one source. Its forge is asked for the ref's head; the files are fetched only when
/// that moved since the last import, or always when `every_time` (an import asked for).
/// `Ok(Err(words))` is the forge's refusal, kept on the source as its last result.
pub async fn import(db: &Path, key: &str, every_time: bool) -> anyhow::Result<Result<Imported, String>> {
    let (path, wanted) = (db.to_path_buf(), key.to_string());
    let read = blocking(move || {
        let conn = store::writer(&path)?;
        let Some(src) = source(&conn, &wanted)? else { return Ok(None) };
        let at = forge(&conn, &src.forge)?;
        Ok(Some((src, at)))
    })
    .await?;
    let Some((src, Some(at))) = read else {
        return Ok(Err(format!("there is no source {key}, or its forge is gone")));
    };

    let outcome = async {
        let commit = at.head(&src.repository, &src.reference).await?;
        if !every_time && src.head.as_deref() == Some(commit.as_str()) {
            return Ok::<_, ForgeError>((commit, None));
        }
        let files = at.files(&src.repository, &commit, &src.path).await?;
        Ok((commit, Some(files)))
    }
    .await;

    let path = db.to_path_buf();
    blocking(move || {
        let conn = store::writer(&path)?;
        let now = crate::db::now_ms();
        let (commit, files) = match outcome {
            Ok(fetched) => fetched,
            Err(why) => {
                let words = why.to_string();
                record(&conn, &src.key, now, None, &json!({ "ok": false, "error": words }))?;
                return Ok(Err(words));
            }
        };
        let Some(files) = files else {
            conn.execute("UPDATE library_sources SET checked_at = ?2 WHERE key = ?1", params![src.key, now])?;
            return Ok(Ok(Imported { source: src.key, commit, fetched: false, items: Vec::new(), gone: Vec::new() }));
        };
        let mut items = Vec::new();
        let mut seen = HashSet::new();
        for found in discover(files, &src.path, &src.repository) {
            let line = |outcome, version: Option<&store::Version>, why: Option<String>| ImportLine {
                kind: found.kind,
                name: found.name.clone(),
                path: found.path.clone(),
                outcome,
                version: version.map(|v| v.number),
                state: version.map(|v| v.state),
                why,
            };
            if !seen.insert((found.kind, found.name.clone())) {
                items.push(line("failed", None, Some(format!("a second {} called {} under this source", found.kind, found.name))));
                continue;
            }
            let bundle = match Bundle::check(found.kind, &found.name, found.files.clone()) {
                Ok(bundle) => bundle,
                Err(why) => {
                    items.push(line("failed", None, Some(why.0)));
                    continue;
                }
            };
            let provenance = Provenance::Forge {
                source: src.key.clone(),
                forge: at.key.clone(),
                repository: src.repository.clone(),
                commit: commit.clone(),
                path: found.path.clone(),
            };
            items.push(match store::publish(&conn, &bundle, &provenance, src.live, now)? {
                Published::New(v) => line("new", Some(&v), None),
                Published::Unchanged(v) => line("unchanged", Some(&v), None),
                Published::Refused(why) => line("refused", None, Some(why)),
            });
        }
        let mut stmt = conn.prepare("SELECT kind, name FROM library_items WHERE source = ?1 AND archived_at IS NULL")?;
        let owned: Vec<(String, String)> =
            stmt.query_map([format!("source:{}", src.key)], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
        let gone = owned
            .into_iter()
            .filter(|(kind, name)| !seen.iter().any(|(k, n)| k.as_str() == kind && n == name))
            .map(|(kind, name)| format!("{kind}/{name}"))
            .collect();
        let imported = Imported { source: src.key.clone(), commit: commit.clone(), fetched: true, items, gone };
        record(&conn, &src.key, now, Some(&commit), &serde_json::to_value(&imported)?)?;
        Ok(Ok(imported))
    })
    .await
}

fn record(conn: &Connection, key: &str, now: i64, head: Option<&str>, result: &Value) -> anyhow::Result<()> {
    match head {
        Some(head) => conn.execute(
            "UPDATE library_sources SET head = ?2, checked_at = ?3, imported_at = ?3, last_result = ?4 WHERE key = ?1",
            params![key, head, now, result.to_string()],
        )?,
        None => conn.execute(
            "UPDATE library_sources SET checked_at = ?2, last_result = ?3 WHERE key = ?1",
            params![key, now, result.to_string()],
        )?,
    };
    Ok(())
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> anyhow::Result<T> + Send + 'static) -> anyhow::Result<T> {
    tokio::task::spawn_blocking(f).await?
}

/// Ask every source once whether its ref moved, import those that did, and tell the machines
/// when anything went live.
pub async fn poll_once(state: &Arc<AppState>) -> Vec<(String, Result<Imported, String>)> {
    let path = state.config.db_path.clone();
    let keys = blocking(move || Ok(sources(&store::writer(&path)?)?.into_iter().map(|s| s.key).collect::<Vec<_>>()))
        .await
        .unwrap_or_default();
    let mut out = Vec::new();
    for key in keys {
        match import(&state.config.db_path, &key, false).await {
            Ok(result) => out.push((key, result)),
            Err(e) => tracing::warn!(source = %key, "the source could not be imported: {e:#}"),
        }
    }
    let live = out.iter().any(|(_, r)| r.as_ref().is_ok_and(Imported::went_live));
    if live {
        super::push::push(state);
    }
    out
}

/// The poller: every source, every [`POLL_EVERY`], for the life of a hosted router.
pub async fn poll(state: Arc<AppState>) {
    let mut every = tokio::time::interval(POLL_EVERY);
    every.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        every.tick().await;
        for (key, result) in poll_once(&state).await {
            match result {
                Ok(imported) if imported.fetched => {
                    tracing::info!(source = %key, commit = %imported.commit, items = imported.items.len(), "imported a source");
                }
                Ok(_) => {}
                Err(why) => tracing::warn!(source = %key, "a source's forge said no: {why}"),
            }
        }
    }
}

/// Run `f` on a writing connection off the async threads, answering a failure as a 500.
async fn write<T: Send + 'static>(state: &Arc<AppState>, f: impl FnOnce(&Connection) -> anyhow::Result<T> + Send + 'static) -> Result<T, Refusal> {
    let path = state.config.db_path.clone();
    blocking(move || f(&store::writer(&path)?)).await.map_err(|e| Refusal::from(failed(e)))
}

/// `GET /api/library/forges`.
pub async fn list_forges(State(state): State<Arc<AppState>>) -> Response {
    match write(&state, forges).await {
        Ok(rows) => Json(json!({ "forges": rows })).into_response(),
        Err(refusal) => refusal.into_response(),
    }
}

#[derive(Deserialize)]
pub struct ForgeIn {
    kind: String,
    base_url: String,
    /// Absent: keep the one kept. `null` or empty: keep none. A string: keep that one.
    #[serde(default)]
    token: Option<Value>,
}

/// `PUT /api/library/forges/{key}`.
pub async fn put_forge_api(State(state): State<Arc<AppState>>, Params(key): Params<String>, JsonBody(body): JsonBody<ForgeIn>) -> Response {
    let Some(kind) = ForgeKind::parse(&body.kind) else {
        return refuse(StatusCode::UNPROCESSABLE_ENTITY, format!("a forge is gitlab or github, not {}", body.kind));
    };
    let token = match body.token {
        None => Token::Keep,
        Some(Value::String(t)) if !t.trim().is_empty() => Token::Set(t.trim().to_string()),
        Some(Value::String(_) | Value::Null) => Token::Clear,
        Some(_) => return refuse(StatusCode::BAD_REQUEST, "a token is a string"),
    };
    match write(&state, move |conn| put_forge(conn, &key, kind, &body.base_url, token, crate::db::now_ms())).await {
        Ok(Ok(())) => Json(json!({ "ok": true })).into_response(),
        Ok(Err(why)) => refuse(StatusCode::UNPROCESSABLE_ENTITY, why.0),
        Err(refusal) => refusal.into_response(),
    }
}

/// `DELETE /api/library/forges/{key}`.
pub async fn delete_forge_api(State(state): State<Arc<AppState>>, Params(key): Params<String>) -> Response {
    let named = key.clone();
    match write(&state, move |conn| delete_forge(conn, &named)).await {
        Ok(Ok(())) => Json(json!({ "ok": true })).into_response(),
        Ok(Err(Unmade::Missing)) => refuse(StatusCode::NOT_FOUND, format!("there is no forge {key}")),
        Ok(Err(Unmade::Refused(why))) => refuse(StatusCode::CONFLICT, why),
        Err(refusal) => refusal.into_response(),
    }
}

/// `POST /api/library/forges/{key}/check`: whether the forge answers and takes its token —
/// for the first source's repository on it, when there is one. Answers 200 either way, with
/// the result named: `ok`, `unauthorised`, `insufficient_scope`, `not_found`, `unreachable`
/// or `unexpected`.
pub async fn check_forge(State(state): State<Arc<AppState>>, Params(key): Params<String>) -> Response {
    let named = key.clone();
    let found = write(&state, move |conn| {
        let at = forge(conn, &named)?;
        let repository = sources(conn)?.into_iter().find(|s| s.forge == named).map(|s| s.repository);
        Ok(at.map(|at| (at, repository)))
    })
    .await;
    let (at, repository) = match found {
        Ok(Some(found)) => found,
        Ok(None) => return refuse(StatusCode::NOT_FOUND, format!("there is no forge {key}")),
        Err(refusal) => return refusal.into_response(),
    };
    let (result, words) = match at.check(repository.as_deref()).await {
        Ok(()) if at.token.is_some() => ("ok", "the forge answered, and took the token".to_string()),
        Ok(()) => ("ok", "the forge answered; with no token, it reads public repositories only".to_string()),
        Err(e @ ForgeError::Unauthorised) => ("unauthorised", e.to_string()),
        Err(e @ ForgeError::InsufficientScope) => ("insufficient_scope", e.to_string()),
        Err(e @ ForgeError::NotFound(_)) => ("not_found", e.to_string()),
        Err(e @ ForgeError::Unreachable(_)) => ("unreachable", e.to_string()),
        Err(e @ ForgeError::Unexpected(_)) => ("unexpected", e.to_string()),
    };
    Json(json!({ "ok": true, "result": result, "words": words })).into_response()
}

/// `GET /api/library/sources`.
pub async fn list_sources(State(state): State<Arc<AppState>>) -> Response {
    match write(&state, sources).await {
        Ok(rows) => Json(json!({ "sources": rows })).into_response(),
        Err(refusal) => refusal.into_response(),
    }
}

/// `PUT /api/library/sources/{key}`.
pub async fn put_source_api(State(state): State<Arc<AppState>>, Params(key): Params<String>, JsonBody(body): JsonBody<SourceIn>) -> Response {
    match write(&state, move |conn| put_source(conn, &key, &body, crate::db::now_ms())).await {
        Ok(Ok(())) => Json(json!({ "ok": true })).into_response(),
        Ok(Err(why)) => refuse(StatusCode::UNPROCESSABLE_ENTITY, why.0),
        Err(refusal) => refusal.into_response(),
    }
}

/// `DELETE /api/library/sources/{key}`.
pub async fn delete_source_api(State(state): State<Arc<AppState>>, Params(key): Params<String>) -> Response {
    let named = key.clone();
    match write(&state, move |conn| delete_source(conn, &named)).await {
        Ok(Ok(())) => Json(json!({ "ok": true })).into_response(),
        Ok(Err(_)) => refuse(StatusCode::NOT_FOUND, format!("there is no source {key}")),
        Err(refusal) => refusal.into_response(),
    }
}

/// `POST /api/library/sources/{key}/import`: import now, whether or not the ref moved.
pub async fn import_now(State(state): State<Arc<AppState>>, Params(key): Params<String>) -> Response {
    match import(&state.config.db_path, &key, true).await {
        Ok(Ok(imported)) => {
            if imported.went_live() {
                super::push::push(&state);
            }
            Json(json!({ "ok": true, "imported": imported })).into_response()
        }
        Ok(Err(why)) if why.starts_with("there is no source") => refuse(StatusCode::NOT_FOUND, why),
        Ok(Err(why)) => refuse(StatusCode::BAD_GATEWAY, why),
        Err(e) => failed(e),
    }
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    use axum::body::Body;
    use axum::extract::{ConnectInfo, Query};
    use axum::http::{header, HeaderMap, Request};
    use axum::routing::get;
    use tower::ServiceExt;

    use super::*;
    use crate::api_refusals::Lab;
    use crate::library::bundle::made;

    /// A forge on loopback that answers as GitLab (`/api/v4`) and as a GitHub Enterprise
    /// server (`/api/v3`), over one repository whose files and head the test changes.
    #[derive(Default)]
    struct Stub {
        commit: Mutex<String>,
        files: Mutex<Vec<(String, String)>>,
        /// The token it takes; none: a public repository.
        token: Option<String>,
        /// It answers 403 to everything: a token that may not read.
        scoped: bool,
        archives: AtomicUsize,
        /// Each request's credential, and its agent.
        seen: Mutex<Vec<(String, Option<String>, Option<String>)>>,
        port: Mutex<u16>,
    }

    impl Stub {
        fn new(commit: &str, files: &[(&str, &str)]) -> Arc<Stub> {
            let stub = Stub::default();
            *stub.commit.lock().unwrap() = commit.into();
            *stub.files.lock().unwrap() = files.iter().map(|(p, t)| (p.to_string(), t.to_string())).collect();
            Arc::new(stub)
        }

        fn change(&self, commit: &str, path: &str, text: &str) {
            *self.commit.lock().unwrap() = commit.into();
            let mut files = self.files.lock().unwrap();
            files.retain(|(p, _)| p != path);
            files.push((path.into(), text.into()));
        }

        /// Whether a request may read: its credential, as each forge sends one.
        fn allowed(&self, uri: &str, headers: &HeaderMap) -> Result<(), StatusCode> {
            let sent = headers
                .get("private-token")
                .or_else(|| headers.get(header::AUTHORIZATION))
                .and_then(|v| v.to_str().ok())
                .map(|v| v.trim_start_matches("Bearer ").to_string());
            let agent = headers.get(header::USER_AGENT).and_then(|v| v.to_str().ok()).map(str::to_string);
            self.seen.lock().unwrap().push((uri.to_string(), sent.clone(), agent));
            if self.scoped {
                return Err(StatusCode::FORBIDDEN);
            }
            match (&self.token, sent) {
                (None, _) => Ok(()),
                (Some(want), Some(got)) if *want == got => Ok(()),
                _ => Err(StatusCode::UNAUTHORIZED),
            }
        }

        /// The repository at its commit, as a forge's archive: one directory around it all.
        fn archive(&self, top: &str) -> Vec<u8> {
            self.archives.fetch_add(1, Ordering::SeqCst);
            let files: Vec<File> = self
                .files
                .lock()
                .unwrap()
                .iter()
                .map(|(p, t)| File { path: format!("{top}/{p}"), bytes: t.as_bytes().to_vec(), executable: false })
                .collect();
            made::tar_gz(&files)
        }
    }

    async fn serve_stub(stub: Arc<Stub>) -> String {
        type S = State<Arc<Stub>>;
        async fn gl_commit(State(s): S, uri: axum::http::Uri, h: HeaderMap) -> Response {
            if let Err(status) = s.allowed(&uri.to_string(), &h) {
                return status.into_response();
            }
            Json(json!({ "id": *s.commit.lock().unwrap() })).into_response()
        }
        async fn gl_archive(State(s): S, uri: axum::http::Uri, h: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
            if let Err(status) = s.allowed(&uri.to_string(), &h) {
                return status.into_response();
            }
            assert_eq!(q.get("sha"), Some(&*s.commit.lock().unwrap()), "an archive at the head asked for");
            let top = format!("skills-{}", q["sha"]);
            s.archive(&top).into_response()
        }
        async fn gl_check(State(s): S, uri: axum::http::Uri, h: HeaderMap) -> Response {
            match s.allowed(&uri.to_string(), &h) {
                Ok(()) => Json(json!({ "version": "17.0" })).into_response(),
                Err(status) => status.into_response(),
            }
        }
        async fn gh_commit(State(s): S, uri: axum::http::Uri, h: HeaderMap) -> Response {
            if let Err(status) = s.allowed(&uri.to_string(), &h) {
                return status.into_response();
            }
            assert_eq!(h.get(header::ACCEPT).and_then(|v| v.to_str().ok()), Some("application/vnd.github.sha"));
            s.commit.lock().unwrap().clone().into_response()
        }
        async fn gh_tarball(State(s): S, uri: axum::http::Uri, h: HeaderMap, Params((owner, repo, sha)): Params<(String, String, String)>) -> Response {
            if let Err(status) = s.allowed(&uri.to_string(), &h) {
                return status.into_response();
            }
            // As GitHub does: the archive comes from another host.
            let port = *s.port.lock().unwrap();
            let to = format!("http://localhost:{port}/codeload/{owner}/{repo}/tar.gz/{sha}");
            (StatusCode::FOUND, [(header::LOCATION, to)]).into_response()
        }
        async fn codeload(State(s): S, uri: axum::http::Uri, h: HeaderMap, Params((owner, repo, sha)): Params<(String, String, String)>) -> Response {
            let sent = h.get(header::AUTHORIZATION).is_some();
            s.seen.lock().unwrap().push((uri.to_string(), sent.then(|| "LEAKED".into()), None));
            s.archive(&format!("{owner}-{repo}-{}", &sha[..7])).into_response()
        }
        let app = axum::Router::new()
            .route("/api/v4/version", get(gl_check))
            .route("/api/v4/projects/{project}", get(gl_check))
            .route("/api/v4/projects/{project}/repository/commits/{reference}", get(gl_commit))
            .route("/api/v4/projects/{project}/repository/archive.tar.gz", get(gl_archive))
            .route("/api/v3/repos/{owner}/{repo}", get(gl_check))
            .route("/api/v3/rate_limit", get(gl_check))
            .route("/api/v3/user", get(gl_check))
            .route("/api/v3/repos/{owner}/{repo}/commits/{reference}", get(gh_commit))
            .route("/api/v3/repos/{owner}/{repo}/tarball/{sha}", get(gh_tarball))
            .route("/codeload/{owner}/{repo}/tar.gz/{sha}", get(codeload))
            .with_state(stub.clone());
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        *stub.port.lock().unwrap() = addr.port();
        tokio::spawn(async move { axum::serve(listener, app).await });
        format!("http://127.0.0.1:{}", addr.port())
    }

    use std::collections::HashMap;

    async fn ask(lab: &Lab, method: &str, path: &str, body: Value) -> (StatusCode, Value, String) {
        let mut req = Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, "127.0.0.1")
            .header(header::AUTHORIZATION, format!("Bearer {}", lab.password.unwrap()))
            .header(header::CONTENT_TYPE, "application/json")
            .body(if body.is_null() { Body::empty() } else { Body::from(body.to_string()) })
            .unwrap();
        req.extensions_mut().insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 50_000))));
        let res = lab.app.clone().oneshot(req).await.unwrap();
        let status = res.status();
        let bytes = axum::body::to_bytes(res.into_body(), 1 << 20).await.unwrap();
        let text = String::from_utf8_lossy(&bytes).to_string();
        (status, serde_json::from_str(&text).unwrap_or(Value::Null), text)
    }

    async fn put_ok(lab: &Lab, path: &str, body: Value) {
        let (status, answer, _) = ask(lab, "PUT", path, body).await;
        assert_eq!(status, StatusCode::OK, "{path}: {answer}");
    }

    async fn import(lab: &Lab, key: &str) -> Value {
        let (status, answer, _) = ask(lab, "POST", &format!("/api/library/sources/{key}/import"), Value::Null).await;
        assert_eq!(status, StatusCode::OK, "{answer}");
        answer["imported"].clone()
    }

    fn lines(imported: &Value) -> Vec<(String, String, String)> {
        imported["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|l| (l["kind"].as_str().unwrap().into(), l["name"].as_str().unwrap().into(), l["outcome"].as_str().unwrap().into()))
            .collect()
    }

    fn skill_md(name: &str, description: &str) -> String {
        format!("---\nname: {name}\ndescription: {description}\n---\n")
    }

    /// A repository with a library of items under `library/`, and one skill outside it.
    fn repository() -> Vec<(&'static str, String)> {
        vec![
            ("library/skills/bench/SKILL.md", skill_md("bench", "Drives the bench.")),
            ("library/skills/bench/scripts/run.sh", "echo hi\n".into()),
            ("library/skills/tools/vault/SKILL.md", skill_md("vault", "Keeps secrets.")),
            ("library/commands/ship.md", "---\ndescription: Ship the branch.\n---\nShip it.\n".into()),
            ("library/agents/reviewer.md", "---\nname: reviewer\ndescription: Reviews a diff.\n---\nReview.\n".into()),
            ("library/README.md", "# the library\n".into()),
            ("other/skills/stray/SKILL.md", skill_md("stray", "Not under the path.")),
        ]
    }

    async fn gitlab(lab: &Lab, stub: &Arc<Stub>, token: Option<&str>) -> String {
        let base = serve_stub(stub.clone()).await;
        put_ok(lab, "/api/library/forges/home", json!({ "kind": "gitlab", "base_url": base, "token": token })).await;
        base
    }

    fn stub_of(commit: &str, files: Vec<(&'static str, String)>) -> Arc<Stub> {
        let refs: Vec<(&str, &str)> = files.iter().map(|(p, t)| (*p, t.as_str())).collect();
        Stub::new(commit, &refs)
    }

    #[tokio::test]
    async fn a_forge_token_is_never_read_back() {
        let lab = Lab::hosted("forge-token").await;
        let token = "a-test-token-value-that-must-stay-inside";
        put_ok(&lab, "/api/library/forges/home", json!({ "kind": "gitlab", "base_url": "https://gitlab.example.com", "token": token })).await;
        put_ok(&lab, "/api/library/sources/skills", json!({ "forge": "home", "repository": "tools/skills", "ref": "main" })).await;
        for path in ["/api/library/forges", "/api/library/sources", "/api/library", "/api/machines"] {
            let (_, _, text) = ask(&lab, "GET", path, Value::Null).await;
            assert!(!text.contains(token), "{path} gave the token back: {text}");
        }
        let (_, forges, _) = ask(&lab, "GET", "/api/library/forges", Value::Null).await;
        assert_eq!((&forges["forges"][0]["token_present"], &forges["forges"][0]["sources"]), (&json!(true), &json!(1)));

        // Sent again without one, the forge keeps its token; moved to another address, it
        // does not, so a token never follows its forge somewhere else.
        put_ok(&lab, "/api/library/forges/home", json!({ "kind": "gitlab", "base_url": "https://gitlab.example.com/" })).await;
        let (_, forges, _) = ask(&lab, "GET", "/api/library/forges", Value::Null).await;
        assert_eq!(forges["forges"][0]["token_present"], true);
        put_ok(&lab, "/api/library/forges/home", json!({ "kind": "gitlab", "base_url": "https://git.example.org" })).await;
        let (_, forges, _) = ask(&lab, "GET", "/api/library/forges", Value::Null).await;
        assert_eq!(forges["forges"][0]["token_present"], false);

        let (status, answer, _) = ask(&lab, "PUT", "/api/library/forges/plain", json!({ "kind": "gitlab", "base_url": "http://gitlab.example.com" })).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(answer["error"].as_str().unwrap().contains("plain http"), "{answer}");
        let (status, answer, _) = ask(&lab, "DELETE", "/api/library/forges/home", Value::Null).await;
        assert_eq!(status, StatusCode::CONFLICT, "a forge in use stays: {answer}");
    }

    #[tokio::test]
    async fn a_public_github_repository_needs_no_token() {
        let lab = Lab::hosted("forge-public").await;
        let stub = stub_of("1111111111111111111111111111111111111111", repository());
        let base = serve_stub(stub.clone()).await;
        put_ok(&lab, "/api/library/forges/hub", json!({ "kind": "github", "base_url": base })).await;
        put_ok(&lab, "/api/library/sources/public", json!({ "forge": "hub", "repository": "someone/skills", "ref": "main", "path": "library", "live": true })).await;
        let imported = import(&lab, "public").await;
        assert_eq!(lines(&imported).len(), 4, "{imported}");
        let seen = stub.seen.lock().unwrap().clone();
        assert!(seen.iter().all(|(_, credential, _)| credential.is_none()), "nothing sent but the request: {seen:?}");
        assert!(seen.iter().any(|(uri, _, _)| uri.starts_with("/codeload/")), "the archive came from where GitHub sent it");
        assert!(
            seen.iter().filter(|(uri, _, _)| uri.starts_with("/api/")).all(|(_, _, agent)| agent.as_deref().is_some_and(|a| a.starts_with("agent-router/"))),
            "GitHub asks every caller to name itself: {seen:?}"
        );

        // With a token, the API is shown it, and the archive's other host is not.
        let private = Arc::new(Stub { token: Some("tok".into()), ..Default::default() });
        *private.commit.lock().unwrap() = "2222222222222222222222222222222222222222".into();
        *private.files.lock().unwrap() = vec![("SKILL.md".into(), skill_md("solo", "One skill."))];
        let base = serve_stub(private.clone()).await;
        put_ok(&lab, "/api/library/forges/hub2", json!({ "kind": "github", "base_url": base, "token": "tok" })).await;
        put_ok(&lab, "/api/library/sources/private", json!({ "forge": "hub2", "repository": "someone/solo", "ref": "main", "live": true })).await;
        let imported = import(&lab, "private").await;
        assert_eq!(lines(&imported), [("skill".into(), "solo".into(), "new".into())], "a repository that is one skill");
        let seen = private.seen.lock().unwrap().clone();
        assert!(seen.iter().filter(|(uri, _, _)| uri.starts_with("/codeload/")).all(|(_, c, _)| c.is_none()), "{seen:?}");
    }

    #[tokio::test]
    async fn a_forge_check_names_what_is_wrong() {
        let lab = Lab::hosted("forge-check").await;
        let stub = Arc::new(Stub { token: Some("right".into()), ..Default::default() });
        let base = serve_stub(stub).await;
        let check = |key: &'static str| {
            let lab = &lab;
            async move {
                let (status, answer, _) = ask(lab, "POST", &format!("/api/library/forges/{key}/check"), Value::Null).await;
                assert_eq!(status, StatusCode::OK, "{answer}");
                (answer["result"].as_str().unwrap().to_string(), answer["words"].as_str().unwrap().to_string())
            }
        };
        put_ok(&lab, "/api/library/forges/good", json!({ "kind": "gitlab", "base_url": base, "token": "right" })).await;
        assert_eq!(check("good").await.0, "ok");
        let other = base.replace("127.0.0.1", "localhost");
        put_ok(&lab, "/api/library/forges/bad", json!({ "kind": "gitlab", "base_url": other, "token": "wrong" })).await;
        let (result, words) = check("bad").await;
        assert_eq!(result, "unauthorised");
        assert!(words.contains("refused the token"), "{words}");

        let scoped = Arc::new(Stub { scoped: true, ..Default::default() });
        let base = serve_stub(scoped).await;
        put_ok(&lab, "/api/library/forges/narrow", json!({ "kind": "gitlab", "base_url": base, "token": "any" })).await;
        let (result, words) = check("narrow").await;
        assert_eq!(result, "insufficient_scope");
        assert!(words.contains("read_api"), "{words}");

        put_ok(&lab, "/api/library/forges/gone", json!({ "kind": "gitlab", "base_url": "http://127.0.0.1:9", "token": "any" })).await;
        let (result, words) = check("gone").await;
        assert_eq!(result, "unreachable");
        assert!(words.contains("could not be reached"), "{words}");
    }

    #[tokio::test]
    async fn an_import_finds_every_item_under_the_path() {
        let lab = Lab::hosted("import-finds").await;
        let stub = stub_of("3333333333333333333333333333333333333333", repository());
        gitlab(&lab, &stub, None).await;
        put_ok(&lab, "/api/library/sources/skills", json!({ "forge": "home", "repository": "whnet/skills", "ref": "main", "path": "library", "live": true })).await;
        let imported = import(&lab, "skills").await;
        assert_eq!(
            lines(&imported),
            [
                ("command".into(), "ship".into(), "new".into()),
                ("skill".into(), "bench".into(), "new".into()),
                ("skill".into(), "vault".into(), "new".into()),
                ("subagent".into(), "reviewer".into(), "new".into()),
            ]
        );
        let (_, item, _) = ask(&lab, "GET", "/api/library/items/skill/bench", Value::Null).await;
        let files: Vec<&str> = item["files"].as_array().unwrap().iter().map(|f| f["path"].as_str().unwrap()).collect();
        assert_eq!(files, ["SKILL.md", "scripts/run.sh"]);

        // What counts, read straight: a path that is itself a skill is that one skill, and a
        // skill inside another is part of it.
        let file = |p: &str, t: &str| File { path: p.into(), bytes: t.as_bytes().to_vec(), executable: false };
        let one = discover(vec![file("SKILL.md", &skill_md("bench", "x")), file("examples/inner/SKILL.md", "y")], "library/skills/bench", "whnet/skills");
        assert_eq!(one.len(), 1);
        assert_eq!((one[0].name.as_str(), one[0].path.as_str(), one[0].files.len()), ("bench", "library/skills/bench", 2));
    }

    #[tokio::test]
    async fn an_import_records_repository_commit_and_path() {
        let lab = Lab::hosted("import-records").await;
        let commit = "4444444444444444444444444444444444444444";
        let stub = stub_of(commit, repository());
        gitlab(&lab, &stub, Some("tok")).await;
        put_ok(&lab, "/api/library/sources/skills", json!({ "forge": "home", "repository": "whnet/skills", "ref": "main", "path": "library", "live": true })).await;
        import(&lab, "skills").await;
        let (_, item, _) = ask(&lab, "GET", "/api/library/items/skill/vault", Value::Null).await;
        assert_eq!(
            item["versions"][0]["provenance"],
            json!({ "from": "forge", "source": "skills", "forge": "home", "repository": "whnet/skills", "commit": commit, "path": "library/skills/tools/vault" })
        );
        let (_, sources, _) = ask(&lab, "GET", "/api/library/sources", Value::Null).await;
        assert_eq!(sources["sources"][0]["head"], commit);
    }

    #[tokio::test]
    async fn an_unchanged_import_makes_no_version() {
        let lab = Lab::hosted("import-unchanged").await;
        let stub = stub_of("5555555555555555555555555555555555555555", repository());
        gitlab(&lab, &stub, None).await;
        put_ok(&lab, "/api/library/sources/skills", json!({ "forge": "home", "repository": "whnet/skills", "ref": "main", "path": "library", "live": true })).await;
        import(&lab, "skills").await;
        let again = import(&lab, "skills").await;
        assert!(lines(&again).iter().all(|(_, _, outcome)| outcome == "unchanged"), "{again}");

        stub.change("6666666666666666666666666666666666666666", "library/skills/bench/SKILL.md", &skill_md("bench", "Drives it better."));
        let changed = import(&lab, "skills").await;
        let outcomes: Vec<_> = lines(&changed).into_iter().map(|(_, name, outcome)| (name, outcome)).collect();
        assert_eq!(
            outcomes,
            [("ship".into(), "unchanged".into()), ("bench".into(), "new".into()), ("vault".into(), "unchanged".into()), ("reviewer".into(), "unchanged".into())]
        );
        let (_, item, _) = ask(&lab, "GET", "/api/library/items/skill/bench", Value::Null).await;
        assert_eq!(item["versions"].as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn a_trusted_sources_versions_go_live_and_others_wait() {
        let lab = Lab::hosted("import-trust").await;
        let stub = stub_of("7777777777777777777777777777777777777777", repository());
        gitlab(&lab, &stub, None).await;
        put_ok(&lab, "/api/library/sources/trusted", json!({ "forge": "home", "repository": "whnet/skills", "ref": "main", "path": "library/skills/bench", "live": true })).await;
        put_ok(&lab, "/api/library/sources/careful", json!({ "forge": "home", "repository": "whnet/skills", "ref": "main", "path": "library/skills/tools/vault" })).await;
        let trusted = import(&lab, "trusted").await;
        assert_eq!(trusted["items"][0]["state"], "live");
        let careful = import(&lab, "careful").await;
        assert_eq!(careful["items"][0]["state"], "pending");

        let (_, manifest, _) = ask(&lab, "GET", "/api/library/manifest", Value::Null).await;
        let names: Vec<&str> = manifest["items"].as_array().unwrap().iter().map(|i| i["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["bench"], "what waits reaches no machine");
        let (status, _, _) = ask(&lab, "POST", "/api/library/items/skill/vault/versions/1/live", Value::Null).await;
        assert_eq!(status, StatusCode::OK, "until the owner approves it");
    }

    #[tokio::test]
    async fn the_poller_imports_only_when_the_head_moved() {
        let lab = Lab::hosted("import-poll").await;
        let stub = stub_of("8888888888888888888888888888888888888888", repository());
        gitlab(&lab, &stub, None).await;
        put_ok(&lab, "/api/library/sources/skills", json!({ "forge": "home", "repository": "whnet/skills", "ref": "main", "path": "library", "live": true })).await;

        let first = poll_once(&lab.state).await;
        assert!(first[0].1.as_ref().unwrap().fetched);
        assert_eq!(stub.archives.load(Ordering::SeqCst), 1);
        let (_, sources, _) = ask(&lab, "GET", "/api/library/sources", Value::Null).await;
        let checked = sources["sources"][0]["checked_at"].as_i64().unwrap();

        tokio::time::sleep(Duration::from_millis(5)).await;
        let second = poll_once(&lab.state).await;
        assert!(!second[0].1.as_ref().unwrap().fetched);
        assert_eq!(stub.archives.load(Ordering::SeqCst), 1, "the head did not move, so nothing was fetched");
        let (_, sources, _) = ask(&lab, "GET", "/api/library/sources", Value::Null).await;
        assert!(sources["sources"][0]["checked_at"].as_i64().unwrap() > checked, "but it was asked");

        stub.change("9999999999999999999999999999999999999999", "library/commands/ship.md", "---\ndescription: Ship it now.\n---\nShip.\n");
        let third = poll_once(&lab.state).await;
        assert!(third[0].1.as_ref().unwrap().fetched);
        assert_eq!(stub.archives.load(Ordering::SeqCst), 2);
        assert_eq!(POLL_EVERY, Duration::from_secs(300));
    }

    #[tokio::test]
    async fn an_item_from_a_source_refuses_other_writes() {
        let lab = Lab::hosted("import-owns").await;
        let stub = stub_of("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", repository());
        gitlab(&lab, &stub, None).await;
        put_ok(&lab, "/api/library/sources/skills", json!({ "forge": "home", "repository": "whnet/skills", "ref": "main", "path": "library", "live": true })).await;
        import(&lab, "skills").await;

        // An upload from the console: one Markdown file, sent as it is.
        let mut req = Request::post("/api/library/items/skill/bench/versions")
            .header(header::HOST, "127.0.0.1")
            .header(header::AUTHORIZATION, format!("Bearer {}", lab.password.unwrap()))
            .body(Body::from(skill_md("bench", "Mine.")))
            .unwrap();
        req.extensions_mut().insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 50_000))));
        let res = lab.app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::CONFLICT);
        let words: Value = serde_json::from_slice(&axum::body::to_bytes(res.into_body(), 1 << 20).await.unwrap()).unwrap();
        assert!(words["error"].as_str().unwrap().contains("comes from whnet/skills (main): change it there"), "{words}");

        // Nor a publish token, nor another source.
        let conn = store::writer(&lab.state.config.db_path).unwrap();
        let bench = Bundle::check(Kind::Skill, "bench", vec![File { path: "SKILL.md".into(), bytes: skill_md("bench", "CI's.").into_bytes(), executable: false }]).unwrap();
        assert!(matches!(store::publish(&conn, &bench, &Provenance::Token { token: "ci".into() }, false, 1).unwrap(), Published::Refused(_)));
        put_ok(&lab, "/api/library/sources/rival", json!({ "forge": "home", "repository": "whnet/skills", "ref": "main", "path": "library/skills/bench", "live": true })).await;
        let rival = import(&lab, "rival").await;
        assert_eq!(rival["items"][0]["outcome"], "refused");
        assert!(rival["items"][0]["why"].as_str().unwrap().contains("whnet/skills"), "{rival}");

        // Removing the source hands its items to everyone.
        let (status, _, _) = ask(&lab, "DELETE", "/api/library/sources/skills", Value::Null).await;
        assert_eq!(status, StatusCode::OK);
        assert!(matches!(store::publish(&conn, &bench, &Provenance::Token { token: "ci".into() }, false, 2).unwrap(), Published::New(_)));
    }
}
