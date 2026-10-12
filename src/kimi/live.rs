//! The live routes for Kimi Code's sessions (AGEN-0112): each `/api/live/*` handler in `live`
//! hands a Kimi Code session here. Every session is watched: listed, its feed streamed, its
//! usage and any approval it waits on shown, all from its files (`session`). Only the ones
//! the router hosts (`web`) are driven: a prompt, an answer, an interrupt, a model, a mode.
//! A control sent to any other is refused, saying why ([`WATCHED`]).
//!
//! `POST /api/live/start {cwd, model}` starts a hosted session on this machine; a hosted
//! router relays it to the machine its body names.

use std::convert::Infallible;
use std::sync::OnceLock;
use std::time::Instant;

use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use super::session;
use super::web::Host;
use crate::live::{AnswerBody, StreamStatus};

/// Why a session the router did not start takes no control.
pub const WATCHED: &str = "This Kimi Code session runs in its own terminal or in VS Code, which takes no outside control. \
Start a hosted session to drive one.";

/// Whether this is a Kimi Code session's id: `session_<uuid>`, where Claude Code's is a bare
/// UUID.
pub fn is_kimi(session: &str) -> bool {
    session.starts_with("session_")
}

/// This machine's hosted sessions, if it has Kimi Code.
pub fn host() -> Option<&'static Host> {
    static HOST: OnceLock<Option<Host>> = OnceLock::new();
    HOST.get_or_init(Host::here).as_ref()
}

fn refuse(code: StatusCode, reason: &str) -> Response {
    (code, Json(json!({ "ok": false, "reason": reason }))).into_response()
}

fn done() -> Response {
    Json(json!({ "ok": true })).into_response()
}

/// The host of a session the router hosts; None for one it only watches.
fn driven<'a>(host: Option<&'a Host>, session: &str) -> Option<&'a Host> {
    host.filter(|h| h.hosts(session))
}

fn failed(e: anyhow::Error) -> Response {
    refuse(StatusCode::BAD_GATEWAY, &format!("{e:#}"))
}

/// A model as Kimi Code's `config.toml` names the router's: `agent-router/<id>`.
fn alias(model: &str) -> String {
    let model = model.trim();
    if model.contains('/') {
        model.to_string()
    } else {
        format!("{}/{model}", super::config::PROVIDER)
    }
}

/// Kimi Code's permission mode for one of Claude Code's, or its own name.
fn mode_of(mode: &str) -> Option<&'static str> {
    match mode {
        "manual" | "default" => Some("manual"),
        "auto" | "acceptEdits" => Some("auto"),
        "yolo" | "bypassPermissions" => Some("yolo"),
        _ => None,
    }
}

pub async fn send(host: Option<&Host>, session: &str, text: &str) -> Response {
    let Some(host) = driven(host, session) else { return refuse(StatusCode::CONFLICT, WATCHED) };
    match host.prompt(session, text).await {
        Ok(()) => Json(json!({ "ok": true, "via": "kimi-web", "framed": false })).into_response(),
        Err(e) => failed(e),
    }
}

/// An approval answered: the one named, else the only one waiting. Kimi Code's questions
/// are not answered from here yet.
pub async fn answer(host: Option<&Host>, body: &AnswerBody) -> Response {
    let Some(host) = driven(host, &body.session) else { return refuse(StatusCode::CONFLICT, WATCHED) };
    let Some(allow) = body.allow else {
        return refuse(StatusCode::CONFLICT, "A Kimi Code question is answered in its own window for now.");
    };
    let pending = match host.pending(&body.session).await {
        Ok(p) => p,
        Err(e) => return failed(e),
    };
    let chosen = match &body.request_id {
        Some(id) => pending.iter().find(|p| &p.approval_id == id),
        None if pending.len() == 1 => pending.first(),
        None => None,
    };
    let Some(approval) = chosen else {
        return refuse(StatusCode::CONFLICT, "That approval is no longer waiting.");
    };
    match host.answer(&body.session, &approval.approval_id, allow).await {
        Ok(()) => Json(json!({ "ok": true, "request_id": approval.approval_id })).into_response(),
        Err(e) => failed(e),
    }
}

pub async fn interrupt(host: Option<&Host>, session: &str) -> Response {
    let Some(host) = driven(host, session) else { return refuse(StatusCode::CONFLICT, WATCHED) };
    match host.interrupt(session).await {
        Ok(true) => done(),
        Ok(false) => refuse(StatusCode::CONFLICT, "Nothing is running to stop."),
        Err(e) => failed(e),
    }
}

pub async fn set_model(host: Option<&Host>, session: &str, model: &str) -> Response {
    let Some(host) = driven(host, session) else { return refuse(StatusCode::CONFLICT, WATCHED) };
    match host.set_model(session, &alias(model)).await {
        Ok(()) => done(),
        Err(e) => failed(e),
    }
}

pub async fn set_mode(host: Option<&Host>, session: &str, mode: &str) -> Response {
    let Some(host) = driven(host, session) else { return refuse(StatusCode::CONFLICT, WATCHED) };
    let Some(mode) = mode_of(mode) else {
        return refuse(StatusCode::BAD_REQUEST, "Kimi Code's permission modes are manual, auto and yolo.");
    };
    match host.set_mode(session, mode).await {
        Ok(()) => done(),
        Err(e) => failed(e),
    }
}

/// What a Kimi Code session cannot be asked from here, said for each control it lacks.
pub fn unsupported(what: &str) -> Response {
    refuse(StatusCode::CONFLICT, &format!("A Kimi Code session has no {what} from here."))
}

/// `GET /api/live/prompt` for a Kimi Code session: the approval it waits on, from its files.
pub async fn prompt(session_id: &str) -> Response {
    let id = session_id.to_string();
    let found = tokio::task::spawn_blocking(move || {
        let s = session::find(&id)?;
        let hooked = hooked(&id);
        Some(s.activity(hooked.as_ref(), crate::db::now_ms()))
    })
    .await
    .ok()
    .flatten();
    let Some(on) = found.and_then(|a| a.waiting_on) else {
        return Json(json!({ "pending": false, "count": 0 })).into_response();
    };
    Json(json!({
        "pending": true,
        "request_id": on.request_id,
        "tool": on.tool,
        "question": false,
        "display_name": on.tool,
        "description": on.summary,
        "reason": null,
        "input": null,
        "suggestions": [],
        "count": 1,
    }))
    .into_response()
}

/// What the hooks last said of a session.
fn hooked(session: &str) -> Option<super::registry::Record> {
    let dir = super::registry::dir().ok()?;
    super::registry::read(&dir, crate::db::now_ms()).into_iter().find(|r| r.session == session)
}

/// `GET /api/live/stream` for a Kimi Code session: its feed, then each new item as its files
/// grow, and its status beside them as Claude Code's are sent.
pub fn stream(session_id: &str) -> Response {
    let id = session_id.to_string();
    let s = async_stream::stream! {
        let mut sent = 0usize;
        let mut last_status: Option<StreamStatus> = None;
        let mut last_sent = Instant::now();
        loop {
            let read = {
                let id = id.clone();
                tokio::task::spawn_blocking(move || {
                    let s = session::find(&id)?;
                    let now = crate::db::now_ms();
                    let status = StreamStatus::of_activity(&s.activity(hooked(&id).as_ref(), now), now);
                    Some((s, status))
                })
                .await
                .ok()
                .flatten()
            };
            let Some((s, status)) = read else {
                yield Ok::<_, Infallible>(Event::default().event("error").data("no such session"));
                return;
            };
            let feed = s.feed();
            // A wire is append-only; one read shorter than what was sent is a new file.
            if feed.len() < sent {
                sent = 0;
            }
            for item in &feed[sent..] {
                if let Ok(json) = serde_json::to_string(item) {
                    yield Ok(Event::default().data(json));
                }
            }
            sent = feed.len();
            if crate::live::status_due(last_status.as_ref(), &status, last_sent.elapsed()) {
                if let Ok(json) = serde_json::to_string(&status) {
                    yield Ok(Event::default().event("status").data(json));
                }
                last_status = Some(status);
                last_sent = Instant::now();
            }
            tokio::time::sleep(crate::live::POLL).await;
        }
    };
    Sse::new(s).keep_alive(KeepAlive::default()).into_response()
}

#[derive(Deserialize)]
pub struct StartBody {
    /// The directory to start it in.
    pub cwd: String,
    /// One of the router's models, as the catalogue names it (`k3`) or as Kimi Code does
    /// (`agent-router/k3`).
    pub model: String,
    /// The machine to start it on; a hosted router relays by it, a machine ignores it.
    #[serde(default)]
    #[expect(dead_code, reason = "a hosted router relays by it; the machine itself has no use for it")]
    pub machine: Option<String>,
}

/// `POST /api/live/start`: a hosted Kimi Code session on this machine.
pub async fn start(host: Option<&Host>, body: &StartBody) -> Response {
    let Some(host) = host else {
        return refuse(StatusCode::CONFLICT, "This machine has no Kimi Code to host a session with.");
    };
    let cwd = std::path::Path::new(body.cwd.trim());
    if !cwd.is_absolute() || !cwd.is_dir() {
        return refuse(StatusCode::BAD_REQUEST, "Start a session in a directory that exists on that machine, by its full path.");
    }
    match host.start(cwd, &alias(&body.model)).await {
        Ok(id) => Json(json!({ "ok": true, "session": id })).into_response(),
        Err(e) => failed(e),
    }
}

/// Stop the hosted sessions' server when it has sat unused for `web::IDLE`, checked every
/// few minutes for as long as the router or agent runs.
pub async fn keep_host() {
    let Ok(Some(host)) = tokio::task::spawn_blocking(host).await else { return };
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(5 * 60)).await;
        match host.stop_if_idle(super::web::IDLE).await {
            Ok(true) => tracing::info!("Kimi Code's server stopped: nothing it hosts was in use"),
            Ok(false) => {}
            Err(e) => tracing::warn!("Kimi Code's server could not be checked: {e:#}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kimi::web::tests::standin;
    use crate::kimi::web::Server;
    use serde_json::Value;

    async fn read(r: Response) -> (StatusCode, Value) {
        let status = r.status();
        let bytes = axum::body::to_bytes(r.into_body(), 1 << 20).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    fn kept(name: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("ar-kimi-live-{name}-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&p);
        p
    }

    const WATCHED_ONE: &str = "session_fd31bc41-dc2f-432b-8be3-2ae7f81240a6";

    #[tokio::test]
    async fn a_watched_kimi_session_refuses_control_saying_why() {
        let (base, server) = standin().await;
        let host = Host::serving(Server::new(&base, "t").unwrap(), kept("watched"));
        let answer: AnswerBody =
            serde_json::from_value(json!({ "session": WATCHED_ONE, "allow": true })).unwrap();
        for (what, r) in [
            ("send", send(Some(&host), WATCHED_ONE, "hello").await),
            ("answer", super::answer(Some(&host), &answer).await),
            ("interrupt", interrupt(Some(&host), WATCHED_ONE).await),
            ("model", set_model(Some(&host), WATCHED_ONE, "k3").await),
            ("mode", set_mode(Some(&host), WATCHED_ONE, "auto").await),
            // A machine with no Kimi Code to host with watches every session too.
            ("send, unhosted", send(None, WATCHED_ONE, "hello").await),
        ] {
            let (status, body) = read(r).await;
            assert_eq!(status, StatusCode::CONFLICT, "{what}");
            assert_eq!(body["ok"], false, "{what}");
            assert_eq!(body["reason"], WATCHED, "{what}");
            assert!(WATCHED.contains("takes no outside control"));
        }
        assert!(server.heard().is_empty(), "nothing reached Kimi Code's server");
        assert!(is_kimi(WATCHED_ONE) && !is_kimi("0f6c2a8e-6c1e-4f43-9d55-0b1d1a2e9c11"));
    }

    #[tokio::test]
    async fn live_start_starts_a_hosted_kimi_session() {
        let (base, server) = standin().await;
        let host = Host::serving(Server::new(&base, "t").unwrap(), kept("start"));
        let cwd = std::env::temp_dir();
        let body = StartBody { cwd: cwd.display().to_string(), model: "k3".into(), machine: Some("mac".into()) };
        let (status, answer) = read(start(Some(&host), &body).await).await;
        assert_eq!(status, StatusCode::OK, "{answer}");
        let id = answer["session"].as_str().unwrap().to_string();
        assert!(host.hosts(&id), "the router hosts what it started");

        let heard = server.heard();
        assert_eq!(heard[0].2["metadata"]["cwd"], cwd.display().to_string());
        assert_eq!(heard[1].2["agent_config"]["model"], "agent-router/k3", "the catalogue's model, as Kimi Code names it");

        // Now driven: a prompt reaches Kimi Code's server.
        let (status, _) = read(send(Some(&host), &id, "Run echo, please.").await).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(server.heard()[0].1, format!("/api/v1/sessions/{id}/prompts"));

        // A directory that is not one is refused before Kimi Code is asked.
        let nowhere = StartBody { cwd: "relative/dir".into(), model: "k3".into(), machine: None };
        assert_eq!(read(start(Some(&host), &nowhere).await).await.0, StatusCode::BAD_REQUEST);
        assert_eq!(read(start(None, &body).await).await.0, StatusCode::CONFLICT);
        assert!(server.heard().is_empty());
    }
}
