//! The live view for native Codex sessions. Rollouts are safe to watch while their
//! terminal or IDE owns execution; control requires a separately hosted app-server.

use std::convert::Infallible;
use std::time::Instant;

use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

use super::session;
use crate::live::StreamStatus;

pub const WATCHED: &str = "This Codex session is watched from its local transcript. Send messages and answer approvals in the terminal or IDE that owns it.";

pub fn is_codex(id: &str) -> bool { id.starts_with("codex:") }

pub fn unsupported() -> Response {
    (StatusCode::CONFLICT, Json(json!({ "ok": false, "reason": WATCHED }))).into_response()
}

pub fn prompt() -> Response {
    // Native rollouts do not persist a current approval channel. Reporting an old
    // request as pending would offer a control the router cannot actually apply.
    Json(json!({ "pending": false, "count": 0 })).into_response()
}

pub fn stream(session_id: &str) -> Response {
    let id = session_id.to_owned();
    let s = async_stream::stream! {
        let mut sent = 0usize;
        let mut last_status: Option<StreamStatus> = None;
        let mut last_sent = Instant::now();
        loop {
            let found = {
                let id = id.clone();
                tokio::task::spawn_blocking(move || session::find(&id)).await.ok().flatten()
            };
            let Some(session) = found else {
                yield Ok::<_, Infallible>(Event::default().event("error").data("no such session"));
                return;
            };
            let feed = session.feed();
            if feed.len() < sent { sent = 0; }
            for item in &feed[sent..] {
                if let Ok(json) = serde_json::to_string(item) { yield Ok(Event::default().data(json)); }
            }
            sent = feed.len();
            let now = crate::db::now_ms();
            let status = StreamStatus::of_activity(&session.activity(now), now);
            if crate::live::status_due(last_status.as_ref(), &status, last_sent.elapsed()) {
                if let Ok(json) = serde_json::to_string(&status) { yield Ok(Event::default().event("status").data(json)); }
                last_status = Some(status); last_sent = Instant::now();
            }
            // Drop the snapshot before the next fold, letting its memo append in place.
            drop(session);
            tokio::time::sleep(crate::live::POLL).await;
        }
    };
    Sse::new(s).keep_alive(KeepAlive::default()).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn watched_codex_control_says_where_the_session_can_be_driven() {
        assert!(is_codex("codex:thread-1"));
        assert!(!is_codex("session_kimi"));
        let response = unsupported();
        assert_eq!(response.status(), StatusCode::CONFLICT);
        let body = axum::body::to_bytes(response.into_body(), 4096).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body["ok"], false);
        assert_eq!(body["reason"], WATCHED);
    }
}
