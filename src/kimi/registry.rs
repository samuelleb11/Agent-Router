//! Which Kimi Code sessions are live, from its hooks (AGEN-0109). Kimi Code runs
//! `agent-router hook kimi` at each event the router's entries in its `config.toml` name
//! (`config::HOOK_EVENTS`), with the event's JSON on stdin. The hook keeps one record per
//! session in `<data dir>/kimi-sessions/<session>.json`, and the agent reads them, as it reads
//! Claude Code's own registry (`registry`). A file rather than a message to the agent: the
//! hook is done at once whether the agent is running or not, and a record outlives the
//! agent's restarts.
//!
//! What the events say, measured on Kimi Code 2.1.1:
//! - SessionStart: live and idle. SessionHeartbeat, every 60 s while it runs: still live.
//! - UserPromptSubmit: busy. PermissionRequest: waiting on a person, until PermissionResult.
//! - Stop: idle again at the end of a turn. SessionEnd: ended.
//!
//! Killing the terminal UI fires no SessionEnd, so a session heard nothing from for three
//! minutes, three missed heartbeats, reads as ended.
//!
//! A hook's `session_id` is `session_<uuid>`: the id in the session's own `state.json`, and
//! the `metadata.user_id` its requests carry, so the id the router files them under. Its
//! `client_type` is `kimi_code_cli` in the terminal.

use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// How long a live session may go unheard before it reads as ended.
pub const SILENT_MS: i64 = 3 * 60 * 1000;

/// How long an ended session's record is kept, for a list that shows recent sessions.
const KEPT_MS: i64 = 24 * 60 * 60 * 1000;

/// What a session is doing, as its hooks last said.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Busy,
    Waiting,
    Idle,
    Ended,
}

/// One session, as its hooks have told it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Record {
    pub session: String,
    #[serde(default)]
    pub cwd: Option<String>,
    /// Kimi Code's `client_type`: which of its clients runs the session.
    #[serde(default)]
    pub client: Option<String>,
    pub status: Status,
    pub started_ms: i64,
    /// When it was last heard from.
    pub heard_ms: i64,
}

/// Where the records are.
pub fn dir() -> Result<PathBuf> {
    Ok(crate::config::data_dir()?.join("kimi-sessions"))
}

/// A session id that is safe as a file name; anything else is not Kimi Code's.
fn valid(session: &str) -> bool {
    !session.is_empty()
        && session.len() <= 128
        && session.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn path_of(dir: &Path, session: &str) -> PathBuf {
    dir.join(format!("{session}.json"))
}

fn read_one(path: &Path) -> Option<Record> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

/// Record one hook's event in `dir`. None for an event that says nothing of a session's
/// state, or a payload that names no session.
pub fn heard(dir: &Path, payload: &Value, now_ms: i64) -> Result<Option<Record>> {
    let field = |k: &str| payload.get(k).and_then(Value::as_str).filter(|v| !v.is_empty());
    let (Some(event), Some(session)) = (field("hook_event_name"), field("session_id")) else { return Ok(None) };
    if !valid(session) {
        return Ok(None);
    }
    let path = path_of(dir, session);
    let previous = read_one(&path).filter(|p| p.status != Status::Ended || event != "SessionStart");
    let status = match event {
        "SessionStart" | "Stop" => Status::Idle,
        "UserPromptSubmit" | "PermissionResult" => Status::Busy,
        "PermissionRequest" => Status::Waiting,
        "SessionEnd" => Status::Ended,
        // Still running, doing what it last said; a heartbeat from a session read as ended
        // says it was not.
        "SessionHeartbeat" => match previous.as_ref().map(|p| p.status) {
            Some(Status::Ended) | None => Status::Idle,
            Some(status) => status,
        },
        _ => return Ok(None),
    };
    let record = Record {
        session: session.to_string(),
        cwd: field("cwd").map(str::to_owned).or_else(|| previous.as_ref().and_then(|p| p.cwd.clone())),
        client: field("client_type").map(str::to_owned).or_else(|| previous.as_ref().and_then(|p| p.client.clone())),
        status,
        started_ms: match (&previous, event) {
            (Some(p), e) if e != "SessionStart" => p.started_ms,
            _ => now_ms,
        },
        heard_ms: now_ms,
    };
    std::fs::create_dir_all(dir)?;
    // Hooks of one session can run at once: each writes a file of its own, then renames it in.
    let tmp = dir.join(format!(".{session}.{}.tmp", std::process::id()));
    std::fs::write(&tmp, serde_json::to_vec(&record)?)?;
    std::fs::rename(&tmp, &path)?;
    Ok(Some(record))
}

/// Every session recorded in `dir`, as of `now_ms`: one unheard for `SILENT_MS` reads as
/// ended. Records of sessions over for a day are let go.
pub fn read(dir: &Path, now_ms: i64) -> Vec<Record> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(session) = name.strip_suffix(".json").filter(|s| valid(s)) else { continue };
        let Some(mut record) = read_one(&entry.path()).filter(|r| r.session == session) else { continue };
        if record.status != Status::Ended && now_ms - record.heard_ms > SILENT_MS {
            record.status = Status::Ended;
        }
        if record.status == Status::Ended && now_ms - record.heard_ms > KEPT_MS {
            let _ = std::fs::remove_file(entry.path());
            continue;
        }
        out.push(record);
    }
    out.sort_by(|a, b| b.heard_ms.cmp(&a.heard_ms));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const T0: i64 = 1_791_650_000_000;
    const SESSION: &str = "session_0f6c2a8e-6c1e-4f43-9d55-0b1d1a2e9c11";

    struct Dir(PathBuf);

    impl Dir {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("ar-kimi-registry-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            Self(dir)
        }
        fn hook(&self, event: &str, at: i64) -> Option<Record> {
            let payload = json!({
                "hook_event_name": event,
                "session_id": SESSION,
                "cwd": "/Users/u/code/agent-router",
                "client_type": "kimi_code_cli",
            });
            heard(&self.0, &payload, at).unwrap()
        }
        fn status(&self, at: i64) -> Option<Status> {
            read(&self.0, at).into_iter().find(|r| r.session == SESSION).map(|r| r.status)
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_kimi_hook_marks_its_session_live_then_ended() {
        let d = Dir::new("live");
        assert_eq!(d.status(T0), None);
        d.hook("SessionStart", T0);
        let record = read(&d.0, T0).pop().unwrap();
        assert_eq!(record.status, Status::Idle);
        assert_eq!(record.cwd.as_deref(), Some("/Users/u/code/agent-router"));
        assert_eq!(record.client.as_deref(), Some("kimi_code_cli"));
        assert_eq!(record.started_ms, T0);

        d.hook("UserPromptSubmit", T0 + 1_000);
        assert_eq!(d.status(T0 + 1_000), Some(Status::Busy));
        d.hook("SessionHeartbeat", T0 + 60_000);
        assert_eq!(d.status(T0 + 60_000), Some(Status::Busy), "a heartbeat keeps what it was doing");
        d.hook("Stop", T0 + 61_000);
        assert_eq!(d.status(T0 + 61_000), Some(Status::Idle));
        d.hook("SessionEnd", T0 + 62_000);
        assert_eq!(d.status(T0 + 62_000), Some(Status::Ended));
        assert_eq!(read(&d.0, T0 + 62_000)[0].started_ms, T0, "the start is kept");

        // Nothing that is not a session's event, nor a session id that is not one.
        assert_eq!(d.hook("PreToolUse", T0), None);
        let bad = json!({ "hook_event_name": "SessionStart", "session_id": "../../etc/passwd" });
        assert_eq!(heard(&d.0, &bad, T0).unwrap(), None);
        assert_eq!(read(&d.0, T0 + 62_000).len(), 1);
    }

    #[test]
    fn a_kimi_permission_request_hook_marks_the_session_waiting() {
        let d = Dir::new("waiting");
        d.hook("SessionStart", T0);
        d.hook("UserPromptSubmit", T0 + 1_000);
        d.hook("PermissionRequest", T0 + 2_000);
        assert_eq!(d.status(T0 + 2_000), Some(Status::Waiting));
        d.hook("SessionHeartbeat", T0 + 60_000);
        assert_eq!(d.status(T0 + 60_000), Some(Status::Waiting), "still waiting on a person");
        d.hook("PermissionResult", T0 + 90_000);
        assert_eq!(d.status(T0 + 90_000), Some(Status::Busy), "answered: the turn goes on");
    }

    #[test]
    fn a_kimi_session_without_a_heartbeat_reads_as_ended() {
        let d = Dir::new("silent");
        d.hook("SessionStart", T0);
        d.hook("SessionHeartbeat", T0 + 120_000);
        assert_eq!(d.status(T0 + 120_000 + SILENT_MS), Some(Status::Idle), "heard within three minutes");
        assert_eq!(d.status(T0 + 120_000 + SILENT_MS + 1), Some(Status::Ended), "killed: no SessionEnd came");

        // It was not over: a heartbeat says it is live again.
        d.hook("SessionHeartbeat", T0 + 400_000);
        assert_eq!(d.status(T0 + 400_000), Some(Status::Idle));

        // Over for a day, the record goes.
        d.hook("SessionEnd", T0 + 500_000);
        assert_eq!(d.status(T0 + 500_000 + KEPT_MS + 1), None);
        assert!(!path_of(&d.0, SESSION).exists());
    }
}
