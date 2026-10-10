//! Where sessions run: each machine's report of the sessions it lists, the directory each one
//! is in, and the git remotes of each directory, kept in the store. It is what makes a
//! project its repository (`repository`, Northstar's ADR 0028): `sessions.repository`, and the
//! key every read takes a project by, `sessions.project`, come from here.
//!
//! One way in, `record`, for both kinds of router. A machine agent posts its report to its
//! hosted router (`POST /api/machines/places`) at every connection and every minute
//! (`send`); a local router records its own machine's the same way on its own timer
//! (`keep_here`). A session is placed by its id alone: a machine places what it lists.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Result};
use axum::extract::State;
use axum::http::{header, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::proxy::AppState;
use crate::refusal::{failed, refuse, JsonBody};
use crate::repository::{self, Remote};

/// How often a machine reports its places, besides at every connection.
pub const EVERY: Duration = Duration::from_secs(60);

/// What one machine reports.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Report {
    /// The machine's id, as its agent keeps it; '' for a local router's own machine.
    pub machine: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub platform: Option<String>,
    /// The sessions it lists, each with the directory it started in.
    #[serde(default)]
    pub sessions: Vec<Listed>,
    /// Those directories, each with its git remotes.
    #[serde(default)]
    pub directories: Vec<Directory>,
    /// The coding clients it has (`clients`). None from an agent that does not say, which
    /// leaves what an earlier report said.
    #[serde(default)]
    pub clients: Option<Vec<crate::clients::Client>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Listed {
    pub session: String,
    #[serde(default)]
    pub cwd: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Directory {
    pub cwd: String,
    #[serde(default)]
    pub remotes: Vec<Remote>,
}

/// A remote as the store keeps it: its URL without credentials, and the repository it names.
#[derive(Serialize)]
struct Kept<'a> {
    name: &'a str,
    url: String,
    repository: Option<String>,
}

/// What `record` changed: sessions placed on the machine, sessions whose repository moved,
/// and directories whose remotes did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Recorded {
    pub placed: usize,
    pub repositories: usize,
    pub directories: usize,
}

/// Keep a machine's report. Only what changed is written; the machine itself is marked seen.
/// Every remote is canonicalised and stripped of credentials here, whoever reported it.
pub fn record(conn: &Connection, report: &Report, now: i64) -> Result<Recorded> {
    let tx = conn.unchecked_transaction()?;
    let machine = report.machine.as_str();
    if !machine.is_empty() {
        tx.execute(
            "INSERT INTO machines (id, label, platform, last_seen_at, clients) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(id) DO UPDATE SET label = coalesce(excluded.label, label),
               platform = coalesce(excluded.platform, platform), last_seen_at = excluded.last_seen_at,
               clients = coalesce(excluded.clients, clients)",
            params![machine, report.label, report.platform, now, report.clients.as_ref().map(serde_json::to_string).transpose()?],
        )?;
    }
    let mut done = Recorded::default();
    let mut repositories = Vec::new();
    for d in report.directories.iter().filter(|d| !d.cwd.trim().is_empty()) {
        let remotes: Vec<Remote> =
            d.remotes.iter().map(|r| Remote { name: r.name.clone(), url: repository::strip_credentials(&r.url) }).collect();
        let kept: Vec<Kept> =
            remotes.iter().map(|r| Kept { name: &r.name, url: r.url.clone(), repository: repository::canonical(&r.url) }).collect();
        let key = repository::key_of(&remotes);
        done.directories += tx.execute(
            "INSERT INTO directories (machine, cwd, remotes, repository, changed_at) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(machine, cwd) DO UPDATE SET remotes = excluded.remotes, repository = excluded.repository,
               changed_at = excluded.changed_at
             WHERE remotes IS NOT excluded.remotes OR repository IS NOT excluded.repository",
            params![machine, d.cwd, serde_json::to_string(&kept)?, key, now],
        )?;
        repositories.push((d.cwd.as_str(), key));
    }
    for s in &report.sessions {
        done.placed += tx.execute(
            "UPDATE sessions SET machine = ?1, cwd = coalesce(cwd, ?2)
             WHERE id = ?3 AND (machine IS NOT ?1 OR (cwd IS NULL AND ?2 IS NOT NULL))",
            params![machine, s.cwd, s.session],
        )?;
    }
    for (cwd, key) in repositories {
        done.repositories += tx.execute(
            "UPDATE sessions SET repository = ?3 WHERE machine = ?1 AND cwd = ?2 AND repository IS NOT ?3",
            params![machine, cwd, key],
        )?;
    }
    tx.commit()?;
    Ok(done)
}

/// This machine's places, from its own list of sessions, and its clients: what an agent sends
/// and a local router records. Blocking: it reads the transcripts and asks `git`.
pub fn here(machine: &str, label: Option<&str>, platform: Option<&str>) -> Report {
    // Claude Code's sessions and Kimi Code's (`kimi::session`), whose ids are the ones its
    // requests are filed under.
    let listed: Vec<Listed> =
        crate::live::discover().into_iter().map(|s| Listed { session: s.session_id, cwd: s.cwd }).collect();
    let cwds: BTreeSet<String> = listed.iter().filter_map(|s| s.cwd.clone()).collect();
    Report {
        machine: machine.to_string(),
        label: label.map(str::to_owned),
        platform: platform.map(str::to_owned),
        sessions: listed,
        directories: cwds.into_iter().map(|cwd| Directory { remotes: repository::remotes_cached(&cwd), cwd }).collect(),
        clients: Some(crate::clients::here()),
    }
}

/// Send this machine's places to its hosted router, as its agent does.
pub async fn send(router: &crate::library::sync::Router, machine: &str, label: &str) -> Result<()> {
    let (machine, label) = (machine.to_string(), label.to_string());
    let report =
        tokio::task::spawn_blocking(move || here(&machine, Some(&label), Some(std::env::consts::OS))).await?;
    deliver(router, &report).await
}

/// Post a report to the router.
pub async fn deliver(router: &crate::library::sync::Router, report: &Report) -> Result<()> {
    let json = vec![(header::CONTENT_TYPE, "application/json".to_string())];
    let (status, _, answer) = router.send(Method::POST, "/api/machines/places", json, serde_json::to_vec(report)?.into()).await?;
    if !status.is_success() {
        bail!("the router answered {status} to this machine's places: {}", String::from_utf8_lossy(&answer).trim());
    }
    Ok(())
}

/// A local router keeps its own machine's places, every minute, as an agent reports them.
pub async fn keep_here(db_path: std::path::PathBuf) {
    loop {
        let path = db_path.clone();
        let kept = tokio::task::spawn_blocking(move || -> Result<Recorded> {
            let report = here("", None, None);
            record(&writer(&path)?, &report, crate::db::now_ms())
        })
        .await;
        match kept {
            Ok(Ok(_)) => {}
            Ok(Err(e)) => tracing::warn!("could not keep this machine's places: {e:#}"),
            Err(e) => tracing::warn!("could not keep this machine's places: {e}"),
        }
        tokio::time::sleep(EVERY).await;
    }
}

/// A connection that may write, waiting on the store's other writers as they wait on it.
fn writer(path: &std::path::Path) -> Result<Connection> {
    let conn = Connection::open(path)?;
    conn.busy_timeout(Duration::from_secs(30))?;
    Ok(conn)
}

/// Each machine's clients, as it last reported them, by its id: what `/api/machines` gives.
pub async fn clients(state: &Arc<AppState>) -> std::collections::HashMap<String, serde_json::Value> {
    let state = state.clone();
    let read = tokio::task::spawn_blocking(move || -> Result<_> {
        let conn = state.reads.get()?;
        let mut stmt = conn.prepare("SELECT id, clients FROM machines WHERE clients IS NOT NULL")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        Ok(rows.flatten().filter_map(|(id, kept)| Some((id, serde_json::from_str(&kept).ok()?))).collect())
    })
    .await;
    match read {
        Ok(Ok(clients)) => clients,
        Ok(Err(e)) => {
            tracing::warn!("machines' clients unread: {e:#}");
            Default::default()
        }
        Err(e) => {
            tracing::warn!("machines' clients unread: {e}");
            Default::default()
        }
    }
}

/// `POST /api/machines/places`: a machine's report of its places, from its agent or from the
/// script that places past sessions (`scripts/backfill-repositories.py`).
pub async fn reported(State(state): State<Arc<AppState>>, JsonBody(report): JsonBody<Report>) -> Response {
    if report.machine.trim().is_empty() {
        return refuse(StatusCode::BAD_REQUEST, "a report names the machine it comes from");
    }
    let path = state.config.db_path.clone();
    let kept = tokio::task::spawn_blocking(move || record(&writer(&path)?, &report, crate::db::now_ms())).await;
    match kept {
        Ok(Ok(done)) => Json(json!({
            "ok": true,
            "placed": done.placed,
            "repositories": done.repositories,
            "directories": done.directories,
        }))
        .into_response(),
        Ok(Err(e)) => failed(e),
        Err(e) => failed(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(name: &str) -> (std::path::PathBuf, Connection) {
        let dir = std::env::temp_dir().join(format!("ar-places-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("agent-router.db");
        crate::db::Db::open(&path).unwrap();
        let conn = Connection::open(&path).unwrap();
        (dir, conn)
    }

    fn session(conn: &Connection, id: &str, cwd: Option<&str>) {
        conn.execute(
            "INSERT INTO sessions (id, cwd, started_at, last_seen_at) VALUES (?1, ?2, 1, 1)",
            params![id, cwd],
        )
        .unwrap();
    }

    fn row(conn: &Connection, id: &str) -> (Option<String>, Option<String>, Option<String>, Option<String>) {
        conn.query_row("SELECT machine, cwd, repository, project FROM sessions WHERE id = ?1", [id], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })
        .unwrap()
    }

    fn remote(name: &str, url: &str) -> Remote {
        Remote { name: name.into(), url: url.into() }
    }

    /// A machine's report places the sessions it lists, gives each its directory's
    /// repository, keeps the directory's remotes without their credentials, and changes
    /// nothing when it says the same again.
    #[test]
    fn a_machine_places_its_sessions_and_their_repository() {
        let (dir, conn) = store("record");
        session(&conn, "lane", Some("/home/u/code/ui-kit-web-lane-2"));
        session(&conn, "plain", Some("/home/u/scratch"));
        session(&conn, "late", None);
        session(&conn, "elsewhere", Some("/home/u/code/ui-kit-web-lane-2"));
        let report = Report {
            machine: "m-1".into(),
            label: Some("wh-vdesk03".into()),
            platform: Some("linux".into()),
            sessions: vec![
                Listed { session: "lane".into(), cwd: Some("/home/u/code/ui-kit-web-lane-2".into()) },
                Listed { session: "plain".into(), cwd: Some("/home/u/scratch".into()) },
                Listed { session: "late".into(), cwd: Some("/home/u/code/ui-kit-web-lane-2".into()) },
                Listed { session: "unknown-yet".into(), cwd: Some("/home/u/scratch".into()) },
            ],
            directories: vec![
                Directory {
                    cwd: "/home/u/code/ui-kit-web-lane-2".into(),
                    remotes: vec![
                        remote("upstream", "git@gitlab.example.com:acme/ui-kit.git"),
                        remote("origin", "https://oauth2:glpat-secret@GitLab.Example.com/acme/ui-kit-web.git"),
                    ],
                },
                Directory { cwd: "/home/u/scratch".into(), remotes: vec![] },
            ],
            clients: None,
        };
        let done = record(&conn, &report, 100).unwrap();
        assert_eq!(done, Recorded { placed: 3, repositories: 2, directories: 2 });

        let repo = Some("gitlab.example.com/acme/ui-kit-web".to_string());
        assert_eq!(row(&conn, "lane"), (Some("m-1".into()), Some("/home/u/code/ui-kit-web-lane-2".into()), repo.clone(), repo.clone()));
        assert_eq!(row(&conn, "late"), (Some("m-1".into()), Some("/home/u/code/ui-kit-web-lane-2".into()), repo.clone(), repo.clone()), "a session the request named no directory for takes the machine's");
        assert_eq!(row(&conn, "plain"), (Some("m-1".into()), Some("/home/u/scratch".into()), None, Some("/home/u/scratch".into())), "no remote: its directory is its key");
        assert_eq!(row(&conn, "elsewhere").0, None, "a session no machine lists is placed nowhere");

        let (remotes, label): (String, String) = conn
            .query_row(
                "SELECT d.remotes, m.label FROM directories d JOIN machines m ON m.id = d.machine WHERE d.cwd = '/home/u/code/ui-kit-web-lane-2'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(label, "wh-vdesk03");
        assert!(!remotes.contains("glpat-secret") && !remotes.contains("oauth2"), "no credential is kept: {remotes}");
        assert!(remotes.contains(r#""repository":"gitlab.example.com/acme/ui-kit""#), "each remote names its own repository: {remotes}");

        assert_eq!(record(&conn, &report, 200).unwrap(), Recorded::default(), "the same report again changes nothing");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An agent's report, sent the way its timer sends it, reaches a hosted router's store.
    #[tokio::test]
    async fn an_agent_reports_its_places_through_the_router() {
        let lab = crate::api_refusals::Lab::hosted("places-report").await;
        let conn = Connection::open(&lab.state.config.db_path).unwrap();
        session(&conn, "s-1", Some("/Users/u/code/ui-kit-web"));

        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = lab.app.clone().into_make_service_with_connect_info::<std::net::SocketAddr>();
        tokio::spawn(async move { axum::serve(listener, app).await });

        let router = crate::library::sync::Router::new(&format!("http://{addr}"), lab.password.unwrap()).unwrap();
        let report = Report {
            machine: "mac".into(),
            label: Some("Sample-Mac-mini".into()),
            platform: Some("macos".into()),
            sessions: vec![Listed { session: "s-1".into(), cwd: Some("/Users/u/code/ui-kit-web".into()) }],
            directories: vec![Directory {
                cwd: "/Users/u/code/ui-kit-web".into(),
                remotes: vec![remote("origin", "https://gitlab.example.com/acme/ui-kit-web.git")],
            }],
            clients: None,
        };
        deliver(&router, &report).await.unwrap();
        assert_eq!(
            row(&conn, "s-1"),
            (
                Some("mac".into()),
                Some("/Users/u/code/ui-kit-web".into()),
                Some("gitlab.example.com/acme/ui-kit-web".into()),
                Some("gitlab.example.com/acme/ui-kit-web".into())
            )
        );

        let nameless = Report { machine: " ".into(), ..Default::default() };
        let refused = deliver(&router, &nameless).await.unwrap_err().to_string();
        assert!(refused.contains("400"), "a report that names no machine is refused: {refused}");
    }
}
