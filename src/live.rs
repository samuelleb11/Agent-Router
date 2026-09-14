//! Live session view — follow every running Claude Code session from the dashboard.
//!
//! Claude Code appends each session's conversation to
//! `~/.claude/projects/<cwd-slug>/<uuid>.jsonl` *as it happens*, so following a live
//! session needs no protocol at all: discover the recently-written transcripts, tail
//! them, and project each event down to something a phone can render.
//!
//! Two consequences worth stating, because they shaped the design:
//!
//!   * It covers sessions this router never proxied, and sessions hosted by the VSCode
//!     extension, because the transcript is written by the session itself.
//!   * It cannot break when the CLI updates. The alternative — the cross-session socket
//!     at `/tmp/cc-socks/<pid>.sock` — is an internal wire format; this is a file the
//!     CLI has written the same way for its whole life.
//!
//! Following is all this module does. *Replying* needs that socket, and answering a
//! pending prompt needs to be the process that spawned the session; neither lives here.
//!
//! This module is READ-ONLY, like `transcript`. It never writes to Claude Code's files.

use std::collections::HashSet;
use std::convert::Infallible;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::extract::Query;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::Json;
use futures_util::Stream;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A session counts as "active" if its transcript grew this recently.
const ACTIVE_WITHIN_MS: i64 = 120_000;
/// Sessions older than this are not offered at all.
const LISTED_WITHIN_MS: i64 = 7 * 24 * 60 * 60 * 1000;
/// Where the archive list lives. A session id in here is one the reader has finished
/// with; it stays hidden until it shows signs of life again.
fn archive_path() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_default())
        .join(".agent-router")
        .join("archived.json")
}

/// Sessions put away, each with the moment it was put away.
///
/// Archiving means "I am done with this", not "never show me this", so a session comes
/// back the moment it does something *new*. Knowing when it was shelved is what makes
/// "new" mean anything: without it the only available test is "moved lately", which is
/// true of the session you just finished with and so hides nothing.
///
/// The file used to be a bare list of ids, with no times. Those entries are read as
/// shelved since the end of time, which keeps them exactly where their reader left
/// them — nothing is known about when they were put away, and guessing would either
/// resurrect them at random or hide them for good.
fn archived_set() -> std::collections::HashMap<String, i64> {
    let Ok(raw) = std::fs::read_to_string(archive_path()) else {
        return Default::default();
    };
    if let Ok(map) = serde_json::from_str::<std::collections::HashMap<String, i64>>(&raw) {
        return map;
    }
    serde_json::from_str::<Vec<String>>(&raw)
        .map(|v| v.into_iter().map(|id| (id, i64::MAX)).collect())
        .unwrap_or_default()
}

/// Whether a session is on the shelf *now*: put away, and quiet ever since.
fn is_shelved(archived: &std::collections::HashMap<String, i64>, session_id: &str, last_activity_ms: i64) -> bool {
    archived.get(session_id).is_some_and(|at| last_activity_ms <= *at)
}

/// Write the shelf back, sorted so the file reads the same way twice.
fn write_archived(set: &std::collections::HashMap<String, i64>) -> std::io::Result<usize> {
    let ordered: std::collections::BTreeMap<&String, &i64> = set.iter().collect();
    let path = archive_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let json = serde_json::to_vec_pretty(&ordered).map_err(std::io::Error::other)?;
    std::fs::write(&path, json)?;
    Ok(set.len())
}

/// Most sessions listed, newest first.
const MAX_LISTED: usize = 60;
/// How far back to read when backfilling a stream, in bytes.
const BACKFILL_BYTES: u64 = 512 * 1024;
/// Longest tool result or text we send to the browser. Generous, because the UI
/// clamps what it *shows* and lets the reader expand — truncating here instead would
/// make the rest unreachable rather than merely folded.
const TEXT_CAP: usize = 6000;
/// Poll interval for transcript growth.
///
/// The real floor on latency is elsewhere: a message reaches the transcript only when
/// it is complete, so partial output cannot appear here at any poll rate. This is set
/// to be well under the gap between completed events rather than to chase tokens.
const POLL: Duration = Duration::from_millis(200);

fn projects_root() -> PathBuf {
    std::env::var("HOME")
        .map(|h| PathBuf::from(h).join(".claude").join("projects"))
        .unwrap_or_default()
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn mtime_ms(meta: &fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[derive(Serialize)]
pub struct LiveSession {
    pub session_id: String,
    pub cwd: Option<String>,
    pub project: String,
    pub git_branch: Option<String>,
    /// Claude Code's own name for the session, falling back to what was first asked.
    pub title: Option<String>,
    /// When the session began. Ordering by this keeps a list of sessions still: order
    /// by activity and they reshuffle under the reader every few seconds.
    pub created_ms: Option<i64>,
    pub last_activity_ms: i64,
    pub active: bool,
    /// working | ask | idle | stalled | failed — what it is doing, without opening it.
    pub state: String,
    /// Put away by the reader. Ignored the moment the session is active again, because
    /// archiving says "I am done with this", not "never show me this".
    pub archived: bool,
    /// Running under the shim *and answering on its socket*, so a typed message or a
    /// stop would actually arrive. A leftover socket file does not count.
    pub wrapped: bool,
    pub bytes: u64,
}

/// Every transcript written in the last week, newest first.
///
/// Ordered by mtime rather than filtered by liveness, because "which of my sessions
/// moved most recently" is the question actually being asked from the sofa.
pub fn discover() -> Vec<LiveSession> {
    let root = projects_root();
    let now = now_ms();
    let archived = archived_set();
    let mut found: Vec<(i64, u64, PathBuf, String)> = Vec::new();

    let Ok(dirs) = fs::read_dir(&root) else {
        return Vec::new();
    };
    for dir in dirs.flatten() {
        if !dir.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let slug = dir.file_name().to_string_lossy().into_owned();
        let Ok(files) = fs::read_dir(dir.path()) else {
            continue;
        };
        for file in files.flatten() {
            let path = file.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            let Ok(meta) = file.metadata() else { continue };
            let m = mtime_ms(&meta);
            if now - m > LISTED_WITHIN_MS {
                continue;
            }
            found.push((m, meta.len(), path, slug.clone()));
        }
    }

    found.sort_by(|a, b| b.0.cmp(&a.0));
    found.truncate(MAX_LISTED);

    found
        .into_iter()
        .map(|(m, bytes, path, project)| {
            let session_id = path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            let head = read_head(&path);
            // Asked first: a stale socket also carries a stale `.pending`, and the state
            // below must not read "ask" off a prompt nobody is waiting on.
            let wrapped = crate::peer::wrapper_alive(&session_id);
            LiveSession {
                archived: is_shelved(&archived, &session_id, m),
                state: tail_state(&path, &session_id, now - m).to_string(),
                wrapped,
                title: read_ai_title(&path).or(head.title),
                created_ms: head.created_ms,
                session_id,
                cwd: head.cwd,
                project,
                git_branch: head.git_branch,
                last_activity_ms: m,
                active: now - m <= ACTIVE_WITHIN_MS,
                bytes,
            }
        })
        .collect()
}

#[derive(Default)]
struct Head {
    cwd: Option<String>,
    git_branch: Option<String>,
    title: Option<String>,
    created_ms: Option<i64>,
}

/// Claude Code names each session as it goes, appending an `ai-title` line whenever it
/// refines the name. The last one is the current name, so this reads from the end —
/// on a long transcript the good title is nowhere near the front.
fn read_ai_title(path: &Path) -> Option<String> {
    const TAIL: u64 = 512 * 1024;
    let mut f = fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let from = len.saturating_sub(TAIL);
    f.seek(SeekFrom::Start(from)).ok()?;
    let mut buf = Vec::new();
    f.take(TAIL).read_to_end(&mut buf).ok()?;
    let text = String::from_utf8_lossy(&buf);
    text.lines()
        .rev()
        .filter(|l| l.contains("\"ai-title\""))
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .find_map(|v| {
            v.get("aiTitle")
                .and_then(|t| t.as_str())
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .map(|t| clip(t, 90))
        })
}

/// Read enough of the front of a transcript to label it: the cwd, the branch, and the
/// first thing the user actually typed.
fn read_head(path: &Path) -> Head {
    let mut out = Head::default();
    let Ok(mut f) = fs::File::open(path) else {
        return out;
    };
    let mut buf = vec![0u8; 96 * 1024];
    let Ok(n) = f.read(&mut buf) else { return out };
    let text = String::from_utf8_lossy(&buf[..n]);

    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if out.cwd.is_none() {
            out.cwd = v.get("cwd").and_then(|c| c.as_str()).map(str::to_owned);
        }
        if out.created_ms.is_none() {
            out.created_ms = ts_ms(&v);
        }
        if out.git_branch.is_none() {
            out.git_branch = v
                .get("gitBranch")
                .and_then(|c| c.as_str())
                .filter(|s| !s.is_empty())
                .map(str::to_owned);
        }
        if out.title.is_none() && v.get("type").and_then(|t| t.as_str()) == Some("user") {
            // Skip sidechains: a subagent's first message is not the session's subject.
            if v.get("isSidechain").and_then(|b| b.as_bool()) == Some(true) {
                continue;
            }
            if let Some(t) = first_user_text(&v) {
                let t = t.trim();
                // Slash commands and pasted tool output make poor titles, but they are
                // still better than nothing, so they are only trimmed, not skipped.
                if !t.is_empty() {
                    out.title = Some(clip(t, 120));
                }
            }
        }
        if out.cwd.is_some() && out.title.is_some() && out.created_ms.is_some() {
            break;
        }
    }
    out
}

/// What a user-role text line really is. Besides what the person typed, Claude Code
/// stores its own bookkeeping under that role: interruption markers, and the echo of a
/// slash command — `<command-name>/model</command-name>…` followed by a
/// `<local-command-stdout>` with what it printed, each preceded by a caveat that is
/// addressed to the model, not to a reader.
enum UserText {
    Skip,
    Interrupt(String),
    Command(String),
    Text(String),
}

fn tag(s: &str, name: &str) -> Option<String> {
    let open = format!("<{name}>");
    let close = format!("</{name}>");
    let start = s.find(&open)? + open.len();
    let end = s[start..].find(&close)? + start;
    Some(s[start..end].trim().to_owned())
}

fn user_text(s: &str) -> UserText {
    let t = s.trim();
    if t.starts_with("<local-command-caveat>") {
        return UserText::Skip;
    }
    if is_marker(t) {
        return UserText::Interrupt(clip(t.trim_matches(['[', ']']), 200));
    }
    if let Some(name) = tag(t, "command-name") {
        let args = tag(t, "command-args").unwrap_or_default();
        let line = if args.is_empty() { name } else { format!("{name} {args}") };
        return UserText::Command(clip(&line, 200));
    }
    if let Some(out) = tag(t, "local-command-stdout") {
        return UserText::Command(clip(&out, TEXT_CAP));
    }
    UserText::Text(clip(t, TEXT_CAP))
}

/// Bracketed system markers Claude Code stores under the user role.
fn is_marker(s: &str) -> bool {
    let t = s.trim();
    t.starts_with('[')
        && t.ends_with(']')
        && t.len() < 200
        && (t.contains("interrupted") || t.contains("Request interrupted") || t.contains("No response"))
}

fn first_user_text(v: &Value) -> Option<String> {
    let content = v.get("message")?.get("content")?;
    if let Some(s) = content.as_str() {
        return Some(s.to_owned());
    }
    for block in content.as_array()? {
        if block.get("type").and_then(|t| t.as_str()) == Some("text") {
            if let Some(s) = block.get("text").and_then(|t| t.as_str()) {
                return Some(s.to_owned());
            }
        }
    }
    None
}

fn clip(s: &str, cap: usize) -> String {
    if s.chars().count() <= cap {
        return s.to_owned();
    }
    let kept: String = s.chars().take(cap).collect();
    format!("{kept}…")
}

/// A session's state, read from the end of its transcript.
///
/// The same reasoning the open view uses — a tool call with no result is work in
/// progress — but from the tail of the file rather than a live stream, so the list can
/// say what every session is doing without opening any of them.
fn tail_state(path: &Path, session_id: &str, quiet_ms: i64) -> &'static str {
    /// A tool call outstanding this long is not working, it is stuck or abandoned.
    const STALLED_AFTER_MS: i64 = 10 * 60 * 1000;
    const TAIL: u64 = 256 * 1024;

    // A wrapped session waiting on a prompt is a fact, not an inference.
    if crate::peer::pending_prompt(session_id, "AskUserQuestion").is_some() {
        return "ask";
    }

    let Ok(mut f) = fs::File::open(path) else {
        return "idle";
    };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    if f.seek(SeekFrom::Start(len.saturating_sub(TAIL))).is_err() {
        return "idle";
    }
    let mut buf = Vec::new();
    if f.take(TAIL).read_to_end(&mut buf).is_err() {
        return "idle";
    }
    let text = String::from_utf8_lossy(&buf);

    let mut prog = Progress::default();
    let mut failed = false;
    for line in text.lines().skip(1) {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        // An API error is the last thing that happened until something else happens.
        failed = match v.get("isApiErrorMessage").and_then(|b| b.as_bool()) {
            Some(true) => true,
            _ => {
                if v.get("type").and_then(|t| t.as_str()) == Some("assistant") {
                    false
                } else {
                    failed
                }
            }
        };
        advance(&v, &mut prog);
    }

    if prog.ask_open {
        "ask"
    } else if failed {
        "failed"
    } else if !prog.pending.is_empty() {
        if quiet_ms > STALLED_AFTER_MS { "stalled" } else { "working" }
    } else {
        "idle"
    }
}

/// One rendered row in the live view.
#[derive(Serialize)]
pub struct Item {
    /// Millisecond timestamp from the transcript, when it carries one.
    pub t: Option<String>,
    /// user | assistant | system
    pub role: String,
    /// text | thinking | tool_use | tool_result | ask | answer | interrupt | command | other
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Full AskUserQuestion payload, so the UI can render the real options.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ask: Option<Value>,
    /// Gap between this event and the one before it. What the real client shows as
    /// "Thought for 2s" is just this, on a thinking block.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dur_ms: Option<i64>,
    pub sidechain: bool,
}

/// What the session is doing right now, inferred from the tail of its transcript.
///
/// A tool call that has no matching result yet *is* the definition of "working": the
/// session wrote the call and has not written the answer, so it is inside that tool.
#[derive(Serialize, PartialEq, Clone)]
pub struct Status {
    /// ask | tool | thinking | idle | working
    pub state: String,
    pub detail: String,
    /// Milliseconds since the transcript last grew.
    pub quiet_ms: i64,
    /// Messages typed while a turn ran, still waiting their turn.
    pub queued: usize,
}

#[derive(Default)]
struct Progress {
    pending: HashSet<String>,
    last_tool: Option<String>,
    ask_open: bool,
    ended_turn: bool,
    last_ts_ms: Option<i64>,
    queued: usize,
}

fn ts_ms(v: &Value) -> Option<i64> {
    let s = v.get("timestamp")?.as_str()?;
    // RFC3339 without pulling in a date crate: the transcript is always UTC "...Z".
    let (date, rest) = s.split_once('T')?;
    let time = rest.trim_end_matches('Z');
    let d: Vec<i64> = date.split('-').filter_map(|x| x.parse().ok()).collect();
    let t: Vec<f64> = time.split(':').filter_map(|x| x.parse().ok()).collect();
    if d.len() != 3 || t.len() != 3 {
        return None;
    }
    // Days since epoch via a civil-from-days inverse; good enough for a duration.
    let (y, m, day) = (d[0], d[1], d[2]);
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = if y2 >= 0 { y2 } else { y2 - 399 } / 400;
    let yoe = y2 - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400_000 + (t[0] as i64) * 3_600_000 + (t[1] as i64) * 60_000 + (t[2] * 1000.0) as i64)
}

/// Fold one transcript line into the running picture of what the session is doing.
fn advance(v: &Value, p: &mut Progress) {
    let ty = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
    // The CLI's own queue: typed while a turn ran, taken up when it ends.
    if ty == "queue-operation" {
        match v.get("operation").and_then(Value::as_str) {
            Some("enqueue") => p.queued += 1,
            Some("dequeue") | Some("remove") => p.queued = p.queued.saturating_sub(1),
            _ => {}
        }
        return;
    }
    let Some(content) = v.get("message").and_then(|m| m.get("content")) else {
        return;
    };
    // A user line stored as a plain string: the same words as the block form.
    if let Some(s) = content.as_str() {
        if ty == "user" {
            user_line(s, p);
        }
        return;
    }
    let Some(blocks) = content.as_array() else {
        return;
    };
    for b in blocks {
        match b.get("type").and_then(|t| t.as_str()).unwrap_or("") {
            "tool_use" => {
                if let Some(id) = b.get("id").and_then(|i| i.as_str()) {
                    p.pending.insert(id.to_string());
                }
                let name = b.get("name").and_then(|n| n.as_str()).unwrap_or("tool");
                p.last_tool = Some(name.to_string());
                p.ask_open = name == "AskUserQuestion";
                p.ended_turn = false;
            }
            "tool_result" => {
                if let Some(id) = b.get("tool_use_id").and_then(|i| i.as_str()) {
                    p.pending.remove(id);
                }
                p.ask_open = false;
            }
            "text" => {
                // Assistant prose with nothing outstanding means the turn is over and
                // the session is waiting on a human.
                if ty == "assistant" {
                    p.ended_turn = p.pending.is_empty();
                } else if ty == "user" {
                    if let Some(s) = b.get("text").and_then(|t| t.as_str()) {
                        user_line(s, p);
                    }
                }
            }
            _ => {}
        }
    }
}

/// What a user-role line does to the state. An interruption ends the turn outright —
/// the tool it cut short will never get a result, and a feed that keeps waiting for
/// one says "working" forever, with Stop lit and nothing to stop. A new prompt starts
/// a turn: the session is working until it answers.
fn user_line(s: &str, p: &mut Progress) {
    match user_text(s) {
        UserText::Interrupt(_) => {
            p.pending.clear();
            p.ask_open = false;
            p.ended_turn = true;
        }
        UserText::Text(_) => p.ended_turn = false,
        UserText::Command(_) | UserText::Skip => {}
    }
}

fn status_of(p: &Progress, quiet_ms: i64) -> Status {
    let queued = p.queued;
    if p.ask_open {
        return Status { state: "ask".into(), detail: "Waiting for your answer".into(), quiet_ms, queued };
    }
    if !p.pending.is_empty() {
        let what = p.last_tool.clone().unwrap_or_else(|| "a tool".into());
        return Status { state: "tool".into(), detail: format!("Running {what}"), quiet_ms, queued };
    }
    if p.ended_turn {
        return Status { state: "idle".into(), detail: "Waiting for you".into(), quiet_ms, queued };
    }
    Status { state: "working".into(), detail: "Working".into(), quiet_ms, queued }
}

/// Project one transcript line into zero or more rows.
///
/// One row per content block rather than per message: a turn that thinks, says
/// something and calls three tools reads far better as five rows than one blob.
fn project_value(v: &Value, prev_ts_ms: Option<i64>) -> Vec<Item> {
    let ty = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
    if !matches!(ty, "user" | "assistant" | "system") {
        return Vec::new();
    }
    let t = v
        .get("timestamp")
        .and_then(|x| x.as_str())
        .map(str::to_owned);
    let sidechain = v
        .get("isSidechain")
        .and_then(|b| b.as_bool())
        .unwrap_or(false);

    let dur_ms = match (ts_ms(v), prev_ts_ms) {
        (Some(now), Some(prev)) if now >= prev => Some(now - prev),
        _ => None,
    };

    let mut out = Vec::new();
    let mut push = |kind: &str, name: Option<String>, text: Option<String>, ask: Option<Value>| {
        out.push(Item {
            t: t.clone(),
            role: ty.to_string(),
            kind: kind.to_string(),
            name,
            text,
            ask,
            dur_ms,
            sidechain,
        });
    };

    if ty == "system" {
        let text = v
            .get("content")
            .and_then(|c| c.as_str())
            .map(|s| clip(s, TEXT_CAP));
        if text.is_some() {
            push("other", None, text, None);
        }
        return out;
    }

    let Some(content) = v.get("message").and_then(|m| m.get("content")) else {
        return out;
    };

    if let Some(s) = content.as_str() {
        // Claude Code writes interruption markers as user-role text, but nobody typed
        // them — showing them as the user's own words misattributes a cancellation to
        // a request, which reads as if they asked for something they did not.
        match user_text(s) {
            UserText::Skip => {}
            UserText::Interrupt(t) => push("interrupt", None, Some(t), None),
            UserText::Command(t) => push("command", None, Some(t), None),
            UserText::Text(t) => push("text", None, Some(t), None),
        }
        return out;
    }
    let Some(blocks) = content.as_array() else {
        return out;
    };

    for b in blocks {
        match b.get("type").and_then(|t| t.as_str()).unwrap_or("") {
            "text" => {
                if let Some(s) = b.get("text").and_then(|t| t.as_str()) {
                    if s.trim().is_empty() {
                        // nothing to show
                    } else if ty == "user" {
                        // Same markers as the string form: the transcript stores them
                        // either way, and the first fix only caught one of them.
                        match user_text(s) {
                            UserText::Skip => {}
                            UserText::Interrupt(t) => push("interrupt", None, Some(t), None),
                            UserText::Command(t) => push("command", None, Some(t), None),
                            UserText::Text(t) => push("text", None, Some(t), None),
                        }
                    } else {
                        push("text", None, Some(clip(s, TEXT_CAP)), None);
                    }
                }
            }
            "thinking" => {
                if let Some(s) = b.get("thinking").and_then(|t| t.as_str()) {
                    if !s.trim().is_empty() {
                        push("thinking", None, Some(clip(s, TEXT_CAP)), None);
                    }
                }
            }
            "tool_use" => {
                let name = b
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("tool")
                    .to_string();
                // An AskUserQuestion is the one tool call worth rendering in full:
                // it is the session asking *you* something, which is the whole point
                // of watching from elsewhere.
                if name == "AskUserQuestion" {
                    push("ask", Some(name), None, b.get("input").cloned());
                } else {
                    push("tool_use", Some(name), Some(summarise_input(b.get("input"))), None);
                }
            }
            "tool_result" => {
                let text = b.get("content").map(flatten_result).unwrap_or_default();
                let is_answer = text.starts_with("The user answered:");
                push(
                    if is_answer { "answer" } else { "tool_result" },
                    None,
                    Some(clip(&text, TEXT_CAP)),
                    None,
                );
            }
            _ => {}
        }
    }
    out
}

/// The most identifying field of a tool call, so a row reads like `Bash: git status`
/// rather than a wall of JSON.
fn summarise_input(input: Option<&Value>) -> String {
    let Some(v) = input else {
        return String::new();
    };
    for key in [
        "command",
        "file_path",
        "path",
        "pattern",
        "query",
        "url",
        "prompt",
        "description",
        "skill",
        "message",
    ] {
        if let Some(s) = v.get(key).and_then(|x| x.as_str()) {
            if !s.trim().is_empty() {
                return clip(s.trim(), 300);
            }
        }
    }
    clip(&v.to_string(), 300)
}

fn flatten_result(v: &Value) -> String {
    if let Some(s) = v.as_str() {
        return s.to_owned();
    }
    if let Some(arr) = v.as_array() {
        let mut parts = Vec::new();
        for b in arr {
            if let Some(s) = b.get("text").and_then(|t| t.as_str()) {
                parts.push(s.to_owned());
            }
        }
        return parts.join("\n");
    }
    v.to_string()
}

fn transcript_path(session_id: &str) -> Option<PathBuf> {
    // The session id names the file, but not the directory, so the project dirs are
    // scanned. Ids are UUIDs, so a hit is unambiguous.
    if !session_id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-')
        || session_id.is_empty()
    {
        return None;
    }
    let root = projects_root();
    let dirs = fs::read_dir(&root).ok()?;
    for dir in dirs.flatten() {
        let candidate = dir.path().join(format!("{session_id}.jsonl"));
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// HTTP
// ---------------------------------------------------------------------------

const LIVE_HTML: &str = include_str!("../assets/live.html");

pub async fn page() -> Response {
    // The page ships inside the binary and changes with every build, so a cached copy
    // is a stale copy — and a phone holding one looks exactly like "it's not working".
    (
        [(
            axum::http::header::CACHE_CONTROL,
            "no-store, no-cache, must-revalidate",
        )],
        axum::response::Html(LIVE_HTML),
    )
        .into_response()
}

pub async fn sessions() -> Response {
    (
        [(axum::http::header::CACHE_CONTROL, "no-store")],
        Json(discover()),
    )
        .into_response()
}

#[derive(Deserialize)]
pub struct SendBody {
    pub session: String,
    pub text: String,
}

/// Push a message from the web UI into a live session's input queue.
///
/// Resolving which process owns a transcript needs the transcript's own cwd and start
/// time, so those are read here rather than trusted from the client.
pub async fn send(Json(body): Json<SendBody>) -> Response {
    let text = body.text.trim().to_string();
    if text.is_empty() {
        return bad(axum::http::StatusCode::BAD_REQUEST, "Nothing to send.");
    }
    // A wrapped session is addressed by id alone. Only the peer fallback needs the
    // transcript, to work out which process owns it — so requiring one up front would
    // refuse the very sessions we can serve best.
    let session = body.session.clone();
    let (cwd, first_ms) = match transcript_path(&body.session) {
        Some(path) => (read_head(&path).cwd, first_event_ms(&path)),
        None if crate::peer::wrapper_socket(&body.session).is_some() => (None, None),
        None => return bad(axum::http::StatusCode::NOT_FOUND, "No such session."),
    };

    let sent = tokio::task::spawn_blocking(move || {
        // The wrapper first, always: it delivers the message as the user's own turn.
        // The peer socket can only deliver a *peer* message, which the receiver frames
        // as coming from another Claude rather than from the person who typed it.
        if crate::peer::wrapper_socket(&session).is_some() {
            return crate::peer::send_via_wrapper(&session, &text).map(|()| ("wrapper", 0, String::new()));
        }
        let peer = crate::peer::resolve(&session, cwd.as_deref(), first_ms).ok_or_else(|| {
            anyhow::anyhow!("That session isn't running any more, so there's nothing to send to.")
        })?;
        crate::peer::send(&peer, &text).map(|id| ("peer", peer.pid, id))
    })
    .await;

    match sent {
        Ok(Ok((via, pid, msg_id))) => {
            tracing::info!(session = %body.session, via, pid, "delivered a message from the live view");
            Json(serde_json::json!({
                "ok": true, "via": via, "pid": pid, "msg_id": msg_id,
                // The UI says so, because the difference is visible in the session and
                // the user would otherwise think we were mangling their message.
                "framed": via == "peer",
            }))
            .into_response()
        }
        Ok(Err(e)) => bad(axum::http::StatusCode::SERVICE_UNAVAILABLE, &e.to_string()),
        Err(e) => bad(axum::http::StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

fn bad(code: axum::http::StatusCode, reason: &str) -> Response {
    (code, Json(serde_json::json!({ "ok": false, "reason": reason }))).into_response()
}

/// Timestamp of a transcript's first event, which dates the session itself.
fn first_event_ms(path: &Path) -> Option<i64> {
    let mut f = fs::File::open(path).ok()?;
    let mut buf = vec![0u8; 8192];
    let n = f.read(&mut buf).ok()?;
    let text = String::from_utf8_lossy(&buf[..n]);
    let line = text.lines().next()?;
    ts_ms(&serde_json::from_str::<Value>(line).ok()?)
}

/// What a session has spent, and how it is configured.
#[derive(Serialize, Default)]
pub struct Stats {
    pub model: Option<String>,
    pub effort: Option<String>,
    pub input: i64,
    pub output: i64,
    pub cache_read: i64,
    pub cache_write: i64,
    /// Split out because the two durations bill at different multipliers.
    pub cache_write_5m: i64,
    pub cache_write_1h: i64,
    /// US dollars, from the rate table. None when the model is not in it.
    pub cost: Option<f64>,
    /// Everything the model read or wrote, cache included — the number that maps to
    /// what the session actually cost.
    pub total: i64,
    pub turns: i64,
}

/// Totals for one session, read straight from its transcript.
///
/// The transcript is used rather than the router's own capture because it covers every
/// session, including ones that never went through the proxy.
pub async fn stats(Query(q): Query<StreamQuery>) -> Response {
    let Some(path) = transcript_path(&q.session) else {
        return bad(axum::http::StatusCode::NOT_FOUND, "No such session.");
    };
    let out = tokio::task::spawn_blocking(move || scan_stats(&path)).await;
    match out {
        Ok(st) => Json(st).into_response(),
        Err(e) => bad(axum::http::StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

fn scan_stats(path: &Path) -> Stats {
    use std::io::BufRead;
    let mut st = Stats::default();
    let Ok(f) = fs::File::open(path) else {
        return st;
    };
    for line in std::io::BufReader::new(f).lines().map_while(Result::ok) {
        let Ok(v) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        // Effort is stamped per turn; the last one is the mode in force now.
        // as_str() before the fallback, not after: an explicit null is still `Some`,
        // so falling back on the Option alone never reaches the second field.
        if let Some(e) = v
            .get("perTurnEffort")
            .and_then(|x| x.as_str())
            .or_else(|| v.get("effort").and_then(|x| x.as_str()))
        {
            st.effort = Some(e.to_string());
        }
        let Some(m) = v.get("message") else { continue };
        if let Some(model) = m.get("model").and_then(|x| x.as_str()) {
            st.model = Some(model.to_string());
        }
        let Some(u) = m.get("usage") else { continue };
        let get = |k: &str| u.get(k).and_then(|x| x.as_i64()).unwrap_or(0);
        st.input += get("input_tokens");
        st.output += get("output_tokens");
        st.cache_read += get("cache_read_input_tokens");
        st.cache_write += get("cache_creation_input_tokens");
        // The durations bill differently (1.25x vs 2x base), so they are kept apart.
        // Older lines carry only the total; those land in the 5m bucket, which is the
        // cheaper of the two and so cannot overstate a bill.
        if let Some(c) = m.get("cache_creation") {
            let g = |k: &str| c.get(k).and_then(|x| x.as_i64()).unwrap_or(0);
            st.cache_write_5m += g("ephemeral_5m_input_tokens");
            st.cache_write_1h += g("ephemeral_1h_input_tokens");
        } else {
            st.cache_write_5m += get("cache_creation_input_tokens");
        }
        st.turns += 1;
    }
    st.total = st.input + st.output + st.cache_read + st.cache_write;
    st.cost = price(&st);
    st
}

/// What a scanned session cost, if its model is in the rate table.
fn price(st: &Stats) -> Option<f64> {
    let model = st.model.as_deref()?;
    let db = rusqlite::Connection::open(db_path()).ok()?;
    let table = crate::pricing::load(&db).ok()?;
    let rate = crate::pricing::rate_for(&table, model)?;
    Some(crate::pricing::cost(
        rate,
        &crate::pricing::Tokens {
            input: st.input,
            cache_write_5m: st.cache_write_5m,
            cache_write_1h: st.cache_write_1h,
            cache_read: st.cache_read,
            output: st.output,
        },
    ))
}

fn db_path() -> PathBuf {
    std::env::var("AGENT_ROUTER_DB").map(PathBuf::from).unwrap_or_else(|_| {
        PathBuf::from(std::env::var("HOME").unwrap_or_default())
            .join(".agent-router")
            .join("agent-router.db")
    })
}

#[derive(Deserialize)]
pub struct ProjectQuery {
    /// The working directory the sessions share.
    pub cwd: String,
}

/// Everything a project has spent, summed over the sessions on screen.
///
/// Archived and long-dead sessions are left out deliberately: the number is meant to
/// answer "what is this project costing me now", and a year of finished sessions
/// answers a different question.
pub async fn project_stats(Query(q): Query<ProjectQuery>) -> Response {
    let cwd = q.cwd.clone();
    let out = tokio::task::spawn_blocking(move || {
        let archived = archived_set();
        let mut total = Stats::default();
        let mut spend = 0.0f64;
        let mut sessions = 0i64;
        let mut models: std::collections::BTreeSet<String> = Default::default();

        for s in discover() {
            if s.cwd.as_deref() != Some(cwd.as_str()) {
                continue;
            }
            if is_shelved(&archived, &s.session_id, s.last_activity_ms) {
                continue;
            }
            let Some(path) = transcript_path(&s.session_id) else {
                continue;
            };
            let st = scan_stats(&path);
            if let Some(m) = st.model.clone() {
                models.insert(m);
            }
            total.input += st.input;
            total.output += st.output;
            total.cache_read += st.cache_read;
            total.cache_write += st.cache_write;
            total.cache_write_5m += st.cache_write_5m;
            total.cache_write_1h += st.cache_write_1h;
            total.turns += st.turns;
            // Summed per session rather than over the total, because a project can mix
            // models and each session's tokens must meet its own rates.
            if let Some(c) = st.cost {
                spend += c;
            }
            sessions += 1;
        }
        total.total = total.input + total.output + total.cache_read + total.cache_write;
        (total, spend, sessions, models.into_iter().collect::<Vec<_>>())
    })
    .await;

    match out {
        Ok((total, spend, sessions, models)) => Json(serde_json::json!({
            "sessions": sessions,
            "models": models,
            "total": total.total,
            "output": total.output,
            "cache_read": total.cache_read,
            "turns": total.turns,
            "cost": spend,
        }))
        .into_response(),
        Err(e) => bad(axum::http::StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

#[derive(Deserialize)]
pub struct ArchiveBody {
    pub session: String,
    pub archived: bool,
}

/// Put a session away, or bring it back.
///
/// Stored as a plain list of ids rather than in the database, because it is the
/// reader's shelf and should survive the database being deleted or rebuilt.
pub async fn archive(Json(body): Json<ArchiveBody>) -> Response {
    if transcript_path(&body.session).is_none() {
        return bad(axum::http::StatusCode::NOT_FOUND, "No such session.");
    }
    let done = tokio::task::spawn_blocking(move || -> anyhow::Result<usize> {
        let mut set = archived_set();
        if body.archived {
            // Stamped now: anything this session does after this moment brings it back.
            set.insert(body.session, now_ms());
        } else {
            set.remove(&body.session);
        }
        Ok(write_archived(&set)?)
    })
    .await;

    match done {
        Ok(Ok(n)) => Json(serde_json::json!({ "ok": true, "archived": n })).into_response(),
        Ok(Err(e)) => bad(axum::http::StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
        Err(e) => bad(axum::http::StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

#[derive(Deserialize)]
pub struct AnswerBody {
    pub session: String,
    /// Question text -> the answer chosen for it.
    pub answers: std::collections::HashMap<String, String>,
}

/// What the session is waiting to be asked, so the UI can answer the live prompt rather
/// than the one its transcript happened to record.
pub async fn prompt(Query(q): Query<StreamQuery>) -> Response {
    let session = q.session.clone();
    match tokio::task::spawn_blocking(move || {
        crate::peer::pending_prompt(&session, "AskUserQuestion")
    })
    .await
    {
        Ok(Some(v)) => Json(serde_json::json!({
            "pending": true,
            "request_id": v["request_id"],
            "input": v["request"]["input"],
        }))
        .into_response(),
        Ok(None) => Json(serde_json::json!({ "pending": false })).into_response(),
        Err(e) => bad(axum::http::StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

/// Answer the pending question — the editor's Submit, from anywhere.
pub async fn answer(Json(body): Json<AnswerBody>) -> Response {
    const TOOL: &str = "AskUserQuestion";
    let session = body.session.clone();
    let answers = body.answers.clone();

    let done = tokio::task::spawn_blocking(move || {
        let req = crate::peer::pending_prompt(&session, TOOL).ok_or_else(|| {
            anyhow::anyhow!("That session isn't waiting on a question right now.")
        })?;
        let request_id = req["request_id"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("the pending request has no id"))?
            .to_string();

        // The answers ride inside the tool's own input, beside the questions they
        // answer — the shape the tool reads when it builds its result.
        let mut input = req["request"]["input"].clone();
        input["answers"] = serde_json::to_value(&answers)?;

        crate::peer::answer_prompt(&session, &request_id, input, TOOL).map(|()| request_id)
    })
    .await;

    match done {
        Ok(Ok(id)) => {
            tracing::info!(session = %body.session, request_id = %id, "answered a prompt from the live view");
            Json(serde_json::json!({ "ok": true, "request_id": id })).into_response()
        }
        Ok(Err(e)) => bad(axum::http::StatusCode::SERVICE_UNAVAILABLE, &e.to_string()),
        Err(e) => bad(axum::http::StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

#[derive(Deserialize)]
pub struct InterruptBody {
    pub session: String,
}

/// The levels the CLI's `/effort` accepts, in order. `max` is silently downgraded by
/// the CLI for models that do not offer it.
pub const EFFORTS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];

/// The aliases the CLI's model picker knows; each also takes a `[1m]` suffix for the
/// million-token context. A full model id works too.
pub const MODEL_ALIASES: [&str; 6] = ["opus", "sonnet", "haiku", "fable", "mythos", "opusplan"];

/// A model name the CLI could accept: an alias, an id, an optional `[1m]`.
pub fn valid_model(m: &str) -> bool {
    let m = m.trim();
    !m.is_empty()
        && m.len() <= 64
        && m.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_' | '[' | ']' | ':' | '/'))
}

/// What the two UIs offer, from one place.
pub async fn options() -> Json<serde_json::Value> {
    let mut models: Vec<String> = Vec::new();
    for a in MODEL_ALIASES {
        models.push(a.to_string());
        models.push(format!("{a}[1m]"));
    }
    Json(serde_json::json!({ "models": models, "efforts": EFFORTS }))
}

#[derive(Deserialize)]
pub struct ModelBody {
    pub session: String,
    pub model: String,
}

/// Change a running session's model — the editor's picker, from anywhere. The CLI
/// confirms or refuses on its control channel; we wait briefly for that answer so a
/// refused name is reported rather than silently ignored.
pub async fn set_model(Json(body): Json<ModelBody>) -> Response {
    let model = body.model.trim().to_string();
    if !valid_model(&model) {
        return bad(axum::http::StatusCode::BAD_REQUEST, "That is not a model name.");
    }
    if !crate::peer::wrapper_alive(&body.session) {
        return bad(axum::http::StatusCode::SERVICE_UNAVAILABLE, "Only a session under the wrapper can change model; restart it after installing the wrapper.");
    }
    let session = body.session.clone();
    let m = model.clone();
    let sent = tokio::task::spawn_blocking(move || crate::peer::set_model_via_wrapper(&session, &m)).await;
    let request_id = match sent {
        Ok(Ok(id)) => id,
        Ok(Err(e)) => return bad(axum::http::StatusCode::SERVICE_UNAVAILABLE, &e.to_string()),
        Err(e) => return bad(axum::http::StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    // The answer usually lands within a few hundred milliseconds.
    for _ in 0..12 {
        tokio::time::sleep(Duration::from_millis(250)).await;
        if let Some(r) = crate::peer::control_response(&body.session, &request_id) {
            let ok = r["subtype"] == "success";
            let notices: Vec<String> = r["response"]["notices"]
                .as_array()
                .map(|a| a.iter().filter_map(|n| n.as_str().map(str::to_owned)).collect())
                .unwrap_or_default();
            if ok {
                tracing::info!(session = %body.session, %model, "model switched from the live view");
                return Json(serde_json::json!({ "ok": true, "model": model, "notices": notices })).into_response();
            }
            let why = r["error"].as_str().unwrap_or("the session refused the model").to_string();
            return bad(axum::http::StatusCode::UNPROCESSABLE_ENTITY, &why);
        }
    }
    Json(serde_json::json!({ "ok": true, "model": model, "pending": true })).into_response()
}

#[derive(Deserialize)]
pub struct EffortBody {
    pub session: String,
    pub effort: String,
}

/// Set the effort level, as `/effort <level>` typed into the session.
pub async fn set_effort(Json(body): Json<EffortBody>) -> Response {
    let level = body.effort.trim().to_ascii_lowercase();
    if !EFFORTS.contains(&level.as_str()) {
        return bad(axum::http::StatusCode::BAD_REQUEST, "Effort is one of low, medium, high, xhigh, max.");
    }
    if !crate::peer::wrapper_alive(&body.session) {
        return bad(axum::http::StatusCode::SERVICE_UNAVAILABLE, "Only a session under the wrapper can change effort; restart it after installing the wrapper.");
    }
    let session = body.session.clone();
    let l = level.clone();
    match tokio::task::spawn_blocking(move || crate::peer::set_effort_via_wrapper(&session, &l)).await {
        Ok(Ok(())) => {
            tracing::info!(session = %body.session, %level, "effort set from the live view");
            Json(serde_json::json!({ "ok": true, "effort": level })).into_response()
        }
        Ok(Err(e)) => bad(axum::http::StatusCode::SERVICE_UNAVAILABLE, &e.to_string()),
        Err(e) => bad(axum::http::StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

/// Stop a running turn — the live view's equivalent of the editor's stop button.
///
/// Only wrapped sessions can be stopped: an interrupt is a control frame on stdin, and
/// the peer socket carries user messages, not control.
pub async fn interrupt(Json(body): Json<InterruptBody>) -> Response {
    let session = body.session.clone();
    let done = tokio::task::spawn_blocking(move || crate::peer::interrupt_via_wrapper(&session)).await;
    match done {
        Ok(Ok(())) => {
            tracing::info!(session = %body.session, "interrupted from the live view");
            Json(serde_json::json!({ "ok": true })).into_response()
        }
        Ok(Err(e)) => bad(axum::http::StatusCode::SERVICE_UNAVAILABLE, &e.to_string()),
        Err(e) => bad(axum::http::StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

#[derive(Deserialize)]
pub struct AsideBody {
    pub session: String,
    pub text: String,
}

const ASIDE_PROMPT: &str = "This is a quick side question, asked from a phone while the main \
conversation continues elsewhere. Answer it directly and briefly from what you already know \
of this conversation. Do not use tools, do not start new work, do not restate the plan.";

/// The Claude Code binary to ask a side question with: the most recently installed
/// extension's — the real binary, not our shim, since the fork needs no control channel —
/// or a plain install.
fn claude_binary() -> Option<PathBuf> {
    let newest = crate::shim::targets()
        .into_iter()
        .map(|t| {
            let shim = PathBuf::from(&t.path);
            let real = PathBuf::from(format!("{}.real", t.path));
            if real.is_file() { real } else { shim }
        })
        .filter(|p| p.is_file())
        .max_by_key(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok());
    if newest.is_some() {
        return newest;
    }
    let home = std::env::var("HOME").unwrap_or_default();
    [format!("{home}/.local/bin/claude"), "/opt/homebrew/bin/claude".into(), "/usr/local/bin/claude".into()]
        .into_iter()
        .map(PathBuf::from)
        .find(|p| p.is_file())
}

/// Put a session on the shelf without a request: for the forks a side question leaves.
fn shelve(session_id: &str) {
    let mut set = archived_set();
    set.insert(session_id.to_string(), now_ms());
    let _ = write_archived(&set);
}

/// A side question — what the CLI's own `/btw` does, which the editor's stream refuses.
/// The session is resumed *forked*: a new session id, the original untouched, no tools,
/// a brief answer, and the fork shelved so it does not clutter the list. It costs one
/// replay of the session's context, mostly cached.
pub async fn aside(Json(body): Json<AsideBody>) -> Response {
    let text = body.text.trim().to_string();
    if text.is_empty() {
        return bad(axum::http::StatusCode::BAD_REQUEST, "Nothing to ask.");
    }
    let Some(path) = transcript_path(&body.session) else {
        return bad(axum::http::StatusCode::NOT_FOUND, "No such session.");
    };
    let Some(claude) = claude_binary() else {
        return bad(axum::http::StatusCode::SERVICE_UNAVAILABLE, "No Claude Code binary on the Mac to ask with.");
    };
    // The fork is written under the project of the directory it runs from, so run from
    // the session's own — or home when that directory is gone, since `--resume` finds
    // the session either way.
    let cwd = read_head(&path)
        .cwd
        .filter(|c| Path::new(c).is_dir())
        .or_else(|| std::env::var("HOME").ok());
    let session = body.session.clone();
    let started = now_ms();
    let run = tokio::task::spawn_blocking(move || {
        let mut cmd = std::process::Command::new(&claude);
        cmd.args(["-p", "--resume", &session, "--fork-session", "--output-format", "json",
                  "--tools", "", "--append-system-prompt", ASIDE_PROMPT, "--", &text]);
        if let Some(c) = &cwd {
            cmd.current_dir(c);
        }
        cmd.stdin(std::process::Stdio::null());
        cmd.output()
    });
    let out = match tokio::time::timeout(Duration::from_secs(180), run).await {
        Ok(Ok(Ok(o))) => o,
        Ok(Ok(Err(e))) => return bad(axum::http::StatusCode::SERVICE_UNAVAILABLE, &format!("could not start Claude Code: {e}")),
        Ok(Err(e)) => return bad(axum::http::StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
        Err(_) => return bad(axum::http::StatusCode::GATEWAY_TIMEOUT, "No answer within three minutes."),
    };
    let stdout = String::from_utf8_lossy(&out.stdout);
    // `--output-format json` prints one result object; be tolerant of anything around it.
    let result = stdout
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .find(|v| v["type"] == "result")
        .or_else(|| serde_json::from_str::<Value>(stdout.trim()).ok());
    let Some(r) = result else {
        let err = String::from_utf8_lossy(&out.stderr);
        return bad(axum::http::StatusCode::BAD_GATEWAY, &format!("Claude Code gave no answer: {}", err.lines().last().unwrap_or("").trim()));
    };
    if r["is_error"] == true {
        return bad(axum::http::StatusCode::BAD_GATEWAY, r["result"].as_str().unwrap_or("the side question failed"));
    }
    let fork = r["session_id"].as_str().unwrap_or("").to_string();
    if !fork.is_empty() && fork != body.session {
        shelve(&fork);
    }
    tracing::info!(session = %body.session, %fork, "side question answered");
    Json(serde_json::json!({
        "ok": true,
        "answer": r["result"].as_str().unwrap_or(""),
        "fork": fork,
        "cost": r["total_cost_usd"],
        "took_ms": now_ms() - started,
    }))
    .into_response()
}

#[derive(Deserialize)]
pub struct StreamQuery {
    session: String,
}

/// Tail one transcript over SSE.
///
/// Polling rather than filesystem events: a 500 ms poll of a file whose length is all
/// that matters costs a `stat`, works identically on every platform, and cannot miss a
/// write the way a coalesced event can.
pub async fn stream(
    Query(q): Query<StreamQuery>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let path = transcript_path(&q.session);

    let s = async_stream::stream! {
        let Some(path) = path else {
            yield Ok(Event::default().event("error").data("no such session"));
            return;
        };

        // Backfill: start from a bounded window at the end of the file rather than the
        // whole thing, which can be megabytes.
        let mut offset: u64 = 0;
        if let Ok(meta) = fs::metadata(&path) {
            offset = meta.len().saturating_sub(BACKFILL_BYTES);
        }
        // A mid-line start would fail to parse, so drop the first partial line.
        let mut first_read = true;
        let mut prog = Progress::default();
        let mut last_status: Option<Status> = None;

        loop {
            let len = fs::metadata(&path).map(|m| m.len()).unwrap_or(offset);
            if len < offset {
                // Truncated or replaced under us; restart from the top.
                offset = 0;
            }
            if len > offset {
                let mut chunk = Vec::new();
                if let Ok(mut f) = fs::File::open(&path) {
                    if f.seek(SeekFrom::Start(offset)).is_ok() {
                        let mut take = f.take(len - offset);
                        let _ = take.read_to_end(&mut chunk);
                    }
                }
                if !chunk.is_empty() {
                    // Only consume through the last newline; a partial trailing line is
                    // left for the next poll, when the writer has finished it.
                    let consumed = match chunk.iter().rposition(|&b| b == b'\n') {
                        Some(i) => i + 1,
                        None => 0,
                    };
                    if consumed > 0 {
                        let text = String::from_utf8_lossy(&chunk[..consumed]).into_owned();
                        offset += consumed as u64;
                        for (i, line) in text.lines().enumerate() {
                            if first_read && i == 0 && offset > consumed as u64 {
                                continue; // dropped partial first line
                            }
                            let Ok(v) = serde_json::from_str::<Value>(line) else {
                                continue;
                            };
                            for item in project_value(&v, prog.last_ts_ms) {
                                if let Ok(json) = serde_json::to_string(&item) {
                                    yield Ok(Event::default().data(json));
                                }
                            }
                            advance(&v, &mut prog);
                            if let Some(ts) = ts_ms(&v) {
                                prog.last_ts_ms = Some(ts);
                            }
                        }
                        first_read = false;
                    }
                }
            }
            // Status rides the same stream on a named event, so the header can show
            // what the session is doing without a second connection. Only changes are
            // sent — a session sitting still should not generate traffic.
            let quiet = fs::metadata(&path)
                .map(|m| now_ms() - mtime_ms(&m))
                .unwrap_or(0);
            let st = status_of(&prog, quiet.max(0));
            let changed = last_status
                .as_ref()
                .map(|old| old.state != st.state || old.detail != st.detail || old.queued != st.queued)
                .unwrap_or(true);
            // Resend a quiet reading periodically so "waiting 4m" keeps counting up.
            if changed || quiet > 3_000 {
                if let Ok(json) = serde_json::to_string(&st) {
                    yield Ok(Event::default().event("status").data(json));
                }
                last_status = Some(st);
            }

            tokio::time::sleep(POLL).await;
        }
    };

    Sse::new(s).keep_alive(KeepAlive::default())
}

/// What to call a session in a notification: its project, and its title if it has one.
pub fn session_label(session_id: &str) -> Option<(String, Option<String>)> {
    let path = transcript_path(session_id)?;
    let head = read_head(&path);
    let project = head
        .cwd
        .as_deref()
        .and_then(|c| c.rsplit('/').next())
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| "session".into());
    Some((project, read_ai_title(&path).or(head.title)))
}

/// Transcripts written within the last `within_ms`, as (session id, path).
pub fn recent_transcripts(within_ms: i64) -> Vec<(String, PathBuf)> {
    let now = now_ms();
    let Ok(dirs) = fs::read_dir(projects_root()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for dir in dirs.flatten() {
        let Ok(files) = fs::read_dir(dir.path()) else { continue };
        for file in files.flatten() {
            let path = file.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            let Ok(meta) = file.metadata() else { continue };
            if now - mtime_ms(&meta) > within_ms {
                continue;
            }
            if let Some(id) = path.file_stem().and_then(|s| s.to_str()) {
                out.push((id.to_owned(), path.clone()));
            }
        }
    }
    out
}

/// The AskUserQuestion this transcript is waiting on, if any: the tool call's id and
/// its first question. For a session with no wrapper this is the only way to know.
/// Whether this session is still waiting on this particular question — asked of its
/// transcript directly, with no recency filter. `recent_transcripts` only reads sessions
/// that moved lately, and a session sitting on a question does not move: after a while
/// it stops being listed as waiting although nobody has answered it. Anything that acts
/// on a prompt *leaving* the waiting set has to tell those two apart.
pub fn still_asking(session_id: &str, ask_id: &str) -> bool {
    transcript_path(session_id)
        .and_then(|path| pending_ask(&path))
        .is_some_and(|(id, _)| id == ask_id)
}

pub fn pending_ask(path: &Path) -> Option<(String, String)> {
    const TAIL: u64 = 256 * 1024;
    let mut f = fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    f.seek(SeekFrom::Start(len.saturating_sub(TAIL))).ok()?;
    let mut buf = Vec::new();
    f.take(TAIL).read_to_end(&mut buf).ok()?;
    let text = String::from_utf8_lossy(&buf);
    let mut open: Vec<(String, String)> = Vec::new();
    for line in text.lines().skip(1) {
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        let Some(blocks) = v.get("message").and_then(|m| m.get("content")).and_then(Value::as_array) else { continue };
        for b in blocks {
            match b.get("type").and_then(Value::as_str) {
                Some("tool_use") if b.get("name").and_then(Value::as_str) == Some("AskUserQuestion") => {
                    let id = b.get("id").and_then(Value::as_str).unwrap_or_default().to_owned();
                    let q = b
                        .get("input")
                        .and_then(|i| i.get("questions"))
                        .and_then(Value::as_array)
                        .and_then(|qs| qs.first())
                        .and_then(|q| q.get("question"))
                        .and_then(Value::as_str)
                        .unwrap_or("A question is waiting")
                        .to_owned();
                    if !id.is_empty() {
                        open.push((id, q));
                    }
                }
                Some("tool_result") => {
                    if let Some(id) = b.get("tool_use_id").and_then(Value::as_str) {
                        open.retain(|(o, _)| o != id);
                    }
                }
                _ => {}
            }
        }
    }
    open.pop()
}

/// Single-line convenience wrapper, used by the tests.
#[cfg(test)]
fn project(line: &str) -> Vec<Item> {
    match serde_json::from_str::<Value>(line) {
        Ok(v) => project_value(&v, None),
        Err(_) => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_interruption_ends_the_turn_and_a_prompt_starts_one() {
        let mut p = Progress::default();
        let call = serde_json::json!({"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"Bash","input":{}}]}});
        advance(&call, &mut p);
        assert_eq!(status_of(&p, 0).state, "tool");
        // The person hit stop: the CLI writes the marker, the tool never answers.
        let stop = serde_json::json!({"type":"user","message":{"role":"user","content":"[Request interrupted by user for tool use]"}});
        advance(&stop, &mut p);
        assert_eq!(status_of(&p, 0).state, "idle", "an interrupted turn is over");
        // The block form of the same marker, on a fresh pending call.
        advance(&call, &mut p);
        let stop2 = serde_json::json!({"type":"user","message":{"role":"user","content":[{"type":"text","text":"[Request interrupted by user]"}]}});
        advance(&stop2, &mut p);
        assert_eq!(status_of(&p, 0).state, "idle");
        // A new prompt: working until the assistant speaks.
        let ask = serde_json::json!({"type":"user","message":{"role":"user","content":"and now do the other thing"}});
        advance(&ask, &mut p);
        assert_eq!(status_of(&p, 0).state, "working");
        let said = serde_json::json!({"type":"assistant","message":{"content":[{"type":"text","text":"done"}]}});
        advance(&said, &mut p);
        assert_eq!(status_of(&p, 0).state, "idle");
        // A command echo changes nothing.
        let echo = serde_json::json!({"type":"user","message":{"role":"user","content":"<command-name>/effort</command-name>"}});
        advance(&echo, &mut p);
        assert_eq!(status_of(&p, 0).state, "idle");
    }

    #[test]
    fn the_queue_is_counted_from_the_transcripts_own_bookkeeping() {
        let mut p = Progress::default();
        for op in ["enqueue", "enqueue", "dequeue"] {
            advance(&serde_json::json!({"type":"queue-operation","operation":op}), &mut p);
        }
        assert_eq!(status_of(&p, 0).queued, 1);
        advance(&serde_json::json!({"type":"queue-operation","operation":"remove"}), &mut p);
        advance(&serde_json::json!({"type":"queue-operation","operation":"remove"}), &mut p);
        assert_eq!(status_of(&p, 0).queued, 0, "never below zero");
    }

    #[test]
    fn a_slash_commands_echo_reads_as_a_command_not_as_the_users_words() {
        let caveat = r#"{"type":"user","message":{"role":"user","content":"<local-command-caveat>Caveat: The messages below were generated by the user while running local commands.</local-command-caveat>"}}"#;
        assert!(project(caveat).is_empty(), "a note addressed to the model is not shown");
        let name = r#"{"type":"user","message":{"role":"user","content":"<command-name>/model</command-name>\n<command-message>model</command-message>\n<command-args>haiku</command-args>"}}"#;
        let it = &project(name)[0];
        assert_eq!((it.kind.as_str(), it.text.as_deref()), ("command", Some("/model haiku")));
        let out = r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"<local-command-stdout>Set model to `haiku (claude-haiku-4-5-20251001)`</local-command-stdout>"}]}}"#;
        let it = &project(out)[0];
        assert_eq!((it.kind.as_str(), it.text.as_deref()), ("command", Some("Set model to `haiku (claude-haiku-4-5-20251001)`")));
        let plain = r#"{"type":"user","message":{"role":"user","content":"please run /model for me"}}"#;
        assert_eq!(project(plain)[0].kind, "text", "words about a command are still words");
    }

    #[test]
    fn model_names_are_checked_before_they_reach_a_session() {
        for ok in ["opus", "opus[1m]", "claude-opus-5-20260115", "sonnet", "fable[1m]"] {
            assert!(valid_model(ok), "{ok}");
        }
        for bad in ["", "opus; rm -rf /", "a name with spaces", "x\n/exit", &"o".repeat(65)] {
            assert!(!valid_model(bad), "{bad:?}");
        }
        assert!(EFFORTS.contains(&"xhigh"));
    }

    #[test]
    fn a_null_per_turn_effort_falls_through_to_the_session_effort() {
        let dir = std::env::temp_dir().join(format!("stats-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("s.jsonl");
        std::fs::write(&p, concat!(
            r#"{"perTurnEffort":null,"effort":"high","message":{"model":"claude-opus-5",
               "usage":{"input_tokens":10,"output_tokens":5,"cache_read_input_tokens":100}}}"#,
        ).replace('\n', "").replace("               ", "")).unwrap();
        let st = scan_stats(&p);
        assert_eq!(st.effort.as_deref(), Some("high"), "null must not shadow the fallback");
        assert_eq!(st.model.as_deref(), Some("claude-opus-5"));
        assert_eq!(st.total, 115, "cache counts toward what the session spent");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn interruption_markers_are_not_the_users_words() {
        let line = r#"{"type":"user","message":{"role":"user","content":"[Request interrupted by user for tool use]"}}"#;
        let items = project(line);
        assert_eq!(items[0].kind, "interrupt", "must not render as a user message");

        let real = r#"{"type":"user","message":{"role":"user","content":"[thinking] is this mine?"}}"#;
        assert_eq!(project(real)[0].kind, "text", "a real message starting with a bracket stays a message");
    }

    #[test]
    fn markers_are_caught_in_block_form_too() {
        // The transcript stores these both as a bare string and as a text block.
        let block = r#"{"type":"user","message":{"role":"user","content":[
            {"type":"text","text":"[Request interrupted by user for tool use]"}]}}"#;
        assert_eq!(project(block)[0].kind, "interrupt");

        let assistant = r#"{"type":"assistant","message":{"role":"assistant","content":[
            {"type":"text","text":"[not a marker] I am explaining brackets"}]}}"#;
        assert_eq!(project(assistant)[0].kind, "text", "only the user role carries them");
    }

    #[test]
    fn the_newest_ai_title_wins() {
        let dir = std::env::temp_dir().join(format!("title-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("s.jsonl");
        std::fs::write(&p, concat!(
            r#"{"type":"ai-title","aiTitle":"First guess"}"#, "\n",
            r#"{"type":"user","message":{"role":"user","content":"hello"}}"#, "\n",
            r#"{"type":"ai-title","aiTitle":"What it turned out to be"}"#, "\n",
        )).unwrap();
        assert_eq!(read_ai_title(&p).as_deref(), Some("What it turned out to be"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_transcript_with_no_title_falls_back() {
        let dir = std::env::temp_dir().join(format!("notitle-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("s.jsonl");
        std::fs::write(&p, r#"{"type":"user","message":{"role":"user","content":"just this"}}"#).unwrap();
        assert!(read_ai_title(&p).is_none());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn projects_an_assistant_turn_into_one_row_per_block() {
        let line = r#"{"type":"assistant","timestamp":"2026-09-10T07:00:00Z","message":{"role":"assistant","content":[
            {"type":"thinking","thinking":"hmm"},
            {"type":"text","text":"here goes"},
            {"type":"tool_use","id":"t1","name":"Bash","input":{"command":"git status"}}
        ]}}"#;
        let items = project(line);
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].kind, "thinking");
        assert_eq!(items[1].kind, "text");
        assert_eq!(items[2].kind, "tool_use");
        assert_eq!(items[2].name.as_deref(), Some("Bash"));
        assert_eq!(items[2].text.as_deref(), Some("git status"));
    }

    #[test]
    fn ask_user_question_keeps_its_full_payload() {
        let line = r#"{"type":"assistant","message":{"role":"assistant","content":[
            {"type":"tool_use","id":"t1","name":"AskUserQuestion","input":{"questions":[
                {"question":"Which?","header":"Scope","multiSelect":false,
                 "options":[{"label":"A","description":"first"},{"label":"B","description":"second"}]}
            ]}}
        ]}}"#;
        let items = project(line);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].kind, "ask");
        let ask = items[0].ask.as_ref().expect("payload kept");
        let opts = ask["questions"][0]["options"].as_array().unwrap();
        assert_eq!(opts.len(), 2, "options survive projection");
        assert_eq!(opts[1]["label"], "B");
    }

    #[test]
    fn an_answer_is_distinguished_from_an_ordinary_tool_result() {
        let line = r#"{"type":"user","message":{"role":"user","content":[
            {"type":"tool_result","tool_use_id":"t1","content":"The user answered: \"Which?\"=\"B\""}
        ]}}"#;
        let items = project(line);
        assert_eq!(items[0].kind, "answer");
    }

    #[test]
    fn sidechain_is_carried_through() {
        let line = r#"{"type":"assistant","isSidechain":true,"message":{"role":"assistant","content":[
            {"type":"text","text":"subagent talking"}
        ]}}"#;
        assert!(project(line)[0].sidechain);
    }

    #[test]
    fn junk_lines_are_dropped_not_fatal() {
        assert!(project("not json").is_empty());
        assert!(project(r#"{"type":"summary","summary":"x"}"#).is_empty());
    }

    #[test]
    fn clip_counts_characters_not_bytes() {
        // A naive byte slice here would panic on the multi-byte boundary.
        let s = "é".repeat(10);
        assert_eq!(clip(&s, 4), "ééé é".replace(' ', "") + "…");
    }

    #[test]
    fn a_shelved_session_comes_back_only_when_it_moves_again() {
        let mut shelf = std::collections::HashMap::new();
        shelf.insert("s-1".to_string(), 1_000i64);

        // Put away at 1000: everything it did up to then is why you put it away.
        assert!(is_shelved(&shelf, "s-1", 900), "older activity keeps it on the shelf");
        assert!(is_shelved(&shelf, "s-1", 1_000), "the moment itself is not new activity");
        assert!(!is_shelved(&shelf, "s-1", 1_001), "one millisecond later it is awake again");
        assert!(!is_shelved(&shelf, "never-shelved", 900));
    }

    #[test]
    fn the_old_bare_list_of_ids_still_reads() {
        let dir = std::env::temp_dir().join(format!("ar-shelf-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".agent-router")).unwrap();
        let path = dir.join(".agent-router").join("archived.json");

        // The format before this had no times at all.
        std::fs::write(&path, br#"["old-1","old-2"]"#).unwrap();
        let legacy: std::collections::HashMap<String, i64> =
            serde_json::from_str::<Vec<String>>(&std::fs::read_to_string(&path).unwrap())
                .map(|v| v.into_iter().map(|id| (id, i64::MAX)).collect())
                .unwrap();
        assert_eq!(legacy.len(), 2);
        assert!(is_shelved(&legacy, "old-1", now_ms()), "a timeless entry stays put away");

        // And the format with them round-trips.
        std::fs::write(&path, br#"{"s-1":1000}"#).unwrap();
        let map: std::collections::HashMap<String, i64> =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(map.get("s-1"), Some(&1000));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn transcript_path_rejects_traversal() {
        assert!(transcript_path("../../etc/passwd").is_none());
        assert!(transcript_path("").is_none());
    }
}
