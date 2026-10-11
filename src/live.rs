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
//! What a session is *doing* is a different question, and the transcript alone cannot
//! answer it: it is silent while the model generates and while background agents work,
//! it never records a permission prompt, it says nothing about whether the process is
//! alive, and its mtime moves on bookkeeping. `session_activity` is the one place that
//! question is answered — the CLI's own session registry verified against the live
//! process, the wrapper's open prompts, the subagent transcripts, and an incremental fold
//! of the transcript, handed to `activity::derive`. The list, the open view's status, the
//! prompt endpoints, the stop guard and the notifier all ask it, and nothing else derives
//! state. docs/session-liveness.md has the evidence behind every rule.
//!
//! This module is READ-ONLY, like `transcript`. It never writes to Claude Code's files.

use std::collections::{HashMap, HashSet};
use std::convert::Infallible;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::extract::Query;
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::Json;
use futures_util::Stream;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::activity::{Activity, Background, Evidence, Kind, WaitingOn, WrapperPrompt};
use crate::fold::{clip, is_marker, ts_ms, Fold, FoldSummary};
use crate::refusal::{JsonBody, LiveBody};
use crate::registry::{ProcRecord, RegStatus, Snapshot};

/// Sessions quiet longer than this are not offered — unless a live process holds them.
/// Judged on the file's mtime, which is fine for a pre-filter: it is never *older* than
/// the content, so nothing recent is left out; it only lets a few stale ones through,
/// and those sort by their content anyway.
const LISTED_WITHIN_MS: i64 = 7 * 24 * 60 * 60 * 1000;
/// Where the archive list lives. A session id in here is one the reader has finished
/// with; it stays hidden until it shows signs of life again.
fn archive_path() -> PathBuf {
    crate::config::data_dir()
        .unwrap_or_else(|_| PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".agent-router"))
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
pub(crate) fn archived_set() -> std::collections::HashMap<String, i64> {
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
pub(crate) fn is_shelved(archived: &std::collections::HashMap<String, i64>, session_id: &str, last_activity_ms: i64) -> bool {
    archived.get(session_id).is_some_and(|at| last_activity_ms <= *at)
}

/// Write the shelf back, sorted so the file reads the same way twice.
pub(crate) fn write_archived(set: &std::collections::HashMap<String, i64>) -> std::io::Result<usize> {
    let ordered: std::collections::BTreeMap<&String, &i64> = set.iter().collect();
    let path = archive_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let json = serde_json::to_vec_pretty(&ordered).map_err(std::io::Error::other)?;
    std::fs::write(&path, json)?;
    Ok(set.len())
}

/// Most sessions listed, newest first — open ones are always listed on top of this.
const MAX_LISTED: usize = 60;
/// The one tool whose prompt is a question with its own options, rather than a request
/// for permission to do something.
const ASK_TOOL: &str = "AskUserQuestion";
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
pub(crate) const POLL: Duration = Duration::from_millis(200);
/// An unchanged status is re-sent this often, so the phones' "· 12s" counters tick. The
/// counter is recomputed from each frame's `quiet_ms`; any slower and it jumps.
const HEARTBEAT: Duration = Duration::from_secs(1);
/// How long a wrapper's answer to "are you there" is reused. Every probe makes the
/// wrapper — a process on that session's critical path — accept a connection and spawn
/// a thread for it, and the list, the stream and the notifier all ask.
const WRAPPER_PROBE_TTL: Duration = Duration::from_secs(1);
/// How long the newest subagent write is reused: it is a directory walk, and it only has
/// to be good to the second the activity rules measure it in.
const SUBAGENT_TTL: Duration = Duration::from_secs(1);
/// How long a transcript nobody asks about stays folded in memory. The list keeps what it
/// shows warm every few seconds and the notifier keeps each open session warm every
/// second, so what ages out is only what nobody is looking at — and a re-fold costs one
/// read of its last 2 MiB.
const TRACKED_IDLE_MS: i64 = 30 * 60 * 1000;
/// More than this appended since the last look and the fold starts over from the window
/// rather than reading it all: a session that wrote that much has left the old state
/// far behind, and a cold fold is bounded where a catch-up would not be.
const CATCH_UP_LIMIT: u64 = 4 * crate::fold::INITIAL_WINDOW;

/// Claude Code's transcripts: one directory per working directory, under the same config
/// dir the session registry lives in, so the two are always read from the same place.
fn projects_root() -> PathBuf {
    crate::registry::claude_dir().join("projects")
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

/// One row of the list.
///
/// The legacy keys keep their names, types and vocabulary exactly: installed iPhones
/// decode this strictly, and one null or one unexpected type blanks their whole list.
/// Everything newer rides in optional keys that old clients never look at.
#[derive(Serialize, Debug)]
pub struct LiveSession {
    pub session_id: String,
    pub cwd: Option<String>,
    pub project: String,
    pub git_branch: Option<String>,
    /// The repository the session's directory is a checkout of, spelt as Northstar spells it
    /// (`repository`): its project, the same on every machine. Absent when the directory has
    /// no remote, which leaves the directory as its project.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
    /// Claude Code's own name for the session, falling back to what was first asked.
    pub title: Option<String>,
    /// When the session began. Ordering by this keeps a list of sessions still: order
    /// by activity and they reshuffle under the reader every few seconds.
    pub created_ms: Option<i64>,
    /// The newest *content* — a prompt, a reply, a tool result, a subagent's write —
    /// never the file's mtime, which bookkeeping moves: an exit flush, a title, a ledger
    /// written to an idle session every few hours. Always a number.
    pub last_activity_ms: i64,
    /// Busy or waiting on you: `Activity::active`, the same answer every surface gets.
    pub active: bool,
    /// working | ask | idle | stalled | failed — the legacy vocabulary, whatever the
    /// activity underneath (`Activity::legacy_list_state`).
    pub state: String,
    /// Put away by the reader. Ignored the moment the session is active again, because
    /// archiving says "I am done with this", not "never show me this".
    pub archived: bool,
    /// Running under the shim *and answering on its socket*, so a typed message or a
    /// stop would actually arrive. A leftover socket file does not count, and neither
    /// does the socket of a conversation its process has left.
    pub wrapped: bool,
    pub bytes: u64,
    /// closed | idle | generating | tool | waiting | background | failed | stalled
    pub activity: &'static str,
    /// Whether a live process holds the session; absent when nothing can say.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub open: Option<bool>,
    /// The header line. Never carries a relative time: clients add that from `since_ms`.
    pub detail: String,
    /// When the current activity began.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub since_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_ms: Option<i64>,
    /// closed: "exit" when the process left cleanly, "unknown" otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_how: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mid_turn: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub waiting_on: Option<WaitingOn>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background: Option<Background>,
    /// The colour class, chosen here so every client colours a state alike.
    pub tone: &'static str,
    /// "registry" when the CLI's own status decided, "transcript" when inferred.
    pub source: &'static str,
    /// The client running it, when it is not Claude Code: `kimi-code` (`kimi::session`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client: Option<&'static str>,
    /// For a Kimi Code session: whether the router hosts it, and so drives it
    /// (`kimi::live`); one it does not is watched only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hosted: Option<bool>,
    /// The router's clock when this was worked out, so a client whose own clock is off
    /// still shows the right elapsed times.
    pub as_of_ms: i64,
}

/// Every session worth showing: each one a live process holds, however long it has been
/// quiet, and then the most recently active of the rest, up to `MAX_LISTED`.
pub fn discover() -> Vec<LiveSession> {
    let reg = crate::registry::snapshot();
    let (archived, now) = (archived_set(), now_ms());
    let mut rows = discover_in(&Roots::system(), &reg, &archived, now);
    // Kimi Code's, from its own files and hooks (`kimi::session`).
    rows.extend(crate::kimi::session::discover(&archived, now));
    rows.extend(crate::codex::session::discover(&archived, now));
    rows.sort_by_key(|r| std::cmp::Reverse(r.last_activity_ms));
    rows
}

fn discover_in(
    roots: &Roots,
    reg: &Snapshot,
    archived: &HashMap<String, i64>,
    now: i64,
) -> Vec<LiveSession> {
    struct Candidate {
        id: String,
        path: PathBuf,
        project: String,
        mtime: i64,
        open: bool,
    }
    let mut found: Vec<Candidate> = Vec::new();
    let Ok(dirs) = fs::read_dir(&roots.projects) else {
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
            let Some(id) = path.file_stem().and_then(|s| s.to_str()).map(str::to_owned) else {
                continue;
            };
            let Ok(meta) = file.metadata() else { continue };
            let mtime = mtime_ms(&meta);
            // A live process is listed however long it has sat idle: nine days waiting
            // for you is still waiting for you.
            let open = reg.by_session.contains_key(&id);
            if !open && now - mtime > LISTED_WITHIN_MS {
                continue;
            }
            found.push(Candidate { id, path, project: slug.clone(), mtime, open });
        }
    }

    // Open sessions first, then the rest by mtime, newest first. A transcript's content
    // is never newer than its mtime, so once the rows kept so far all beat the next
    // candidate's mtime, no later candidate can make the cut and none is folded.
    found.sort_by_key(|c| (!c.open, std::cmp::Reverse(c.mtime)));
    let open_count = found.iter().filter(|c| c.open).count();
    let slots = MAX_LISTED.saturating_sub(open_count);
    let mut open_rows: Vec<LiveSession> = Vec::new();
    let mut rows: Vec<LiveSession> = Vec::new();
    for c in found {
        // Only where the registry decides what is open: without it, a subagent write can
        // make a row newer than its own transcript.
        if !c.open && reg.available && rows.len() >= slots {
            let mut kept: Vec<i64> = rows.iter().map(|r| r.last_activity_ms).collect();
            kept.sort_unstable_by(|a, b| b.cmp(a));
            if kept.get(slots.saturating_sub(1)).is_some_and(|floor| c.mtime < *floor) {
                break;
            }
        }
        if !c.open && is_router_fork(reg, &c.id) {
            continue;
        }
        let seen = observe(roots, reg, &c.id, Some(&c.path), now);
        let row = list_row(&seen, &c.id, c.project, archived);
        if c.open {
            open_rows.push(row);
        } else {
            rows.push(row);
        }
    }
    rows.sort_by_key(|r| std::cmp::Reverse(r.last_activity_ms));
    rows.truncate(slots);
    rows.extend(open_rows);
    rows.sort_by_key(|r| std::cmp::Reverse(r.last_activity_ms));
    rows
}

/// One list row, from what `observe` found.
fn list_row(
    o: &Observed,
    session_id: &str,
    project: String,
    archived: &HashMap<String, i64>,
) -> LiveSession {
    let a = &o.activity;
    let last = o.last_activity_ms();
    let active = a.active();
    LiveSession {
        session_id: session_id.to_owned(),
        cwd: o.seen.head.cwd.clone(),
        project,
        git_branch: o.seen.head.git_branch.clone(),
        repository: o.seen.head.cwd.as_deref().and_then(|cwd| crate::repository::key_of(&crate::repository::remotes_cached(cwd))),
        title: o.seen.fold.title.clone().or_else(|| o.seen.head.title.clone()),
        created_ms: o.seen.head.created_ms,
        last_activity_ms: last,
        active,
        state: a.legacy_list_state().to_owned(),
        // Shelved with the same content clock, so an exit flush or a ledger line no
        // longer brings a finished session back; and never while it needs you or works.
        archived: is_shelved(archived, session_id, last) && !active,
        wrapped: o.wrapped(),
        bytes: o.seen.bytes,
        activity: a.kind.as_str(),
        open: a.open,
        detail: a.detail.clone(),
        since_ms: a.since_ms,
        ended_ms: a.ended_ms,
        ended_how: a.ended_how,
        mid_turn: a.mid_turn,
        waiting_on: a.waiting_on.clone(),
        background: a.background,
        tone: a.tone(),
        source: a.source,
        client: None,
        hosted: None,
        as_of_ms: o.as_of_ms,
    }
}

/// A transcript the router's own side-question fork is writing (`aside`): not a session
/// anyone opened, and shelved the moment the fork is done, so it is left out while it
/// runs rather than showing as a new session for up to three minutes. Recognised by the
/// registry naming it only under a process the router spawned. That process may name the
/// *parent* for a moment while it resumes, which is why a session a side question is
/// being asked of right now is never taken for its fork.
fn is_router_fork(reg: &Snapshot, session_id: &str) -> bool {
    reg.spawned.contains_key(session_id)
        && !reg.by_session.contains_key(session_id)
        && !asking(session_id)
}

/// The sessions a side question is being asked of right now, with how many at once.
fn asides() -> &'static Mutex<HashMap<String, usize>> {
    static ASKING: OnceLock<Mutex<HashMap<String, usize>>> = OnceLock::new();
    ASKING.get_or_init(Default::default)
}

fn asking(session_id: &str) -> bool {
    asides()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(session_id)
        .is_some_and(|n| *n > 0)
}

/// Marks a session as being asked a side question for as long as it lives.
struct Asking(String);

impl Asking {
    fn new(session_id: &str) -> Self {
        *asides()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(session_id.to_owned())
            .or_default() += 1;
        Asking(session_id.to_owned())
    }
}

impl Drop for Asking {
    fn drop(&mut self) {
        let mut all = asides().lock().unwrap_or_else(|e| e.into_inner());
        if let Some(n) = all.get_mut(&self.0) {
            *n = n.saturating_sub(1);
            if *n == 0 {
                all.remove(&self.0);
            }
        }
    }
}

/// The front of a transcript: where the session runs, on what branch, when it began, and
/// the first thing asked. Read once per transcript (see `tracked`), never per request.
#[derive(Clone, Debug, Default)]
pub(crate) struct Head {
    pub cwd: Option<String>,
    pub git_branch: Option<String>,
    pub title: Option<String>,
    pub created_ms: Option<i64>,
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

/// One rendered row in the live view.
#[derive(Serialize, Clone)]
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

/// The open view's `status` event: what the session is doing, from `session_activity`
/// like everything else, so the header and the list can no longer disagree.
///
/// The first four keys are the ones installed phones decode, strictly, with the legacy
/// meanings: `state` in the stream's vocabulary (ask | tool | idle | working | failed),
/// `quiet_ms` the time spent in the current activity. The rest are for newer clients.
#[derive(Serialize, PartialEq, Clone, Debug)]
pub struct StreamStatus {
    pub state: &'static str,
    pub detail: String,
    /// Now minus `since_ms`, never negative: how long it has been doing this.
    pub quiet_ms: i64,
    /// Messages typed (or sent by a peer) while a turn ran, still waiting their turn.
    /// Task notifications passing through the queue are not counted; nothing is queued
    /// in a session that has no process.
    pub queued: usize,
    pub activity: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub open: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub since_ms: Option<i64>,
    pub as_of_ms: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub waiting_on: Option<WaitingOn>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background: Option<Background>,
    pub tone: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_ms: Option<i64>,
}

impl StreamStatus {
    /// The status of a session whose activity is all there is to say: Kimi Code's
    /// (`kimi::live`), with nothing queued.
    pub(crate) fn of_activity(a: &Activity, as_of_ms: i64) -> Self {
        StreamStatus {
            state: a.legacy_stream_state(),
            detail: a.detail.clone(),
            quiet_ms: a.since_ms.map_or(0, |since| (as_of_ms - since).max(0)),
            queued: 0,
            activity: a.kind.as_str(),
            open: a.open,
            since_ms: a.since_ms,
            as_of_ms,
            waiting_on: a.waiting_on.clone(),
            background: a.background,
            tone: a.tone(),
            ended_ms: a.ended_ms,
        }
    }

    fn of(o: &Observed) -> Self {
        let a = &o.activity;
        StreamStatus {
            state: a.legacy_stream_state(),
            detail: a.detail.clone(),
            quiet_ms: a.since_ms.map_or(0, |since| (o.as_of_ms - since).max(0)),
            queued: if a.open == Some(false) { 0 } else { o.seen.fold.queued },
            activity: a.kind.as_str(),
            open: a.open,
            since_ms: a.since_ms,
            as_of_ms: o.as_of_ms,
            waiting_on: a.waiting_on.clone(),
            background: a.background,
            tone: a.tone(),
            ended_ms: a.ended_ms,
        }
    }

    /// A change worth sending at once. Anything else waits for the heartbeat: the clock
    /// fields move every tick and are not news.
    fn differs(&self, other: &StreamStatus) -> bool {
        let request = |s: &StreamStatus| {
            s.waiting_on
                .as_ref()
                .and_then(|w| w.request_id.clone())
        };
        self.state != other.state
            || self.detail != other.detail
            || self.activity != other.activity
            || self.queued != other.queued
            || self.open != other.open
            || request(self) != request(other)
            || self.background != other.background
    }
}

/// Whether to send a status now: on a change, and otherwise once per `HEARTBEAT`.
pub(crate) fn status_due(last: Option<&StreamStatus>, now: &StreamStatus, since_sent: Duration) -> bool {
    match last {
        None => true,
        Some(old) => old.differs(now) || since_sent >= HEARTBEAT,
    }
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
                    push("tool_use", Some(name), Some(crate::fold::summarise_input(b.get("input"))), None);
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
    transcript_path_in(&projects_root(), session_id)
}

pub(crate) fn transcript_path_in(root: &Path, session_id: &str) -> Option<PathBuf> {
    // The session id names the file, but not the directory, so the project dirs are
    // scanned. Ids are UUIDs, so a hit is unambiguous.
    if !session_id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-')
        || session_id.is_empty()
    {
        return None;
    }
    let dirs = fs::read_dir(root).ok()?;
    for dir in dirs.flatten() {
        let candidate = dir.path().join(format!("{session_id}.jsonl"));
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// What each session is doing
// ---------------------------------------------------------------------------

/// Where the evidence lives. Production reads the real directories; tests point every
/// one at a temp dir, because the real ones belong to sessions that are running.
#[derive(Clone, Debug)]
pub(crate) struct Roots {
    /// `<claude dir>/projects`: the transcripts, one directory per working directory.
    pub projects: PathBuf,
    /// The wrapper's sockets and prompt records.
    pub wrapper: PathBuf,
}

impl Roots {
    pub(crate) fn system() -> Self {
        Roots {
            projects: projects_root(),
            wrapper: crate::peer::wrapper_dir(),
        }
    }
}

/// A transcript as of the last look.
#[derive(Clone, Debug, Default)]
pub(crate) struct Seen {
    pub fold: FoldSummary,
    pub head: Head,
    pub bytes: u64,
    /// Only ever a last resort for `last_activity_ms`; never evidence of activity.
    pub mtime_ms: i64,
}

/// One transcript as the router last read it.
struct Tracked {
    /// (device, inode): a transcript replaced under the same name is folded afresh.
    id: (u64, u64),
    /// Bytes of the file read so far.
    offset: u64,
    /// The start of a line still being written when it was read, finished by a later one.
    carry: Vec<u8>,
    /// The first read begins mid-file, almost always mid-line.
    skip_first_line: bool,
    fold: Fold,
    summary: FoldSummary,
    head: Head,
}

struct Slot {
    /// When anyone last asked, in epoch ms: readable without waiting on a fold in progress.
    used_ms: AtomicI64,
    tracked: Mutex<Option<Tracked>>,
}

/// The fold of one transcript, brought up to date: a `stat`, then only the bytes appended
/// since the last look. The first look folds the last 2 MiB (`fold::INITIAL_WINDOW`),
/// minus the partial line it starts in; a file that shrank or was replaced starts over.
///
/// One lock per transcript, so the list can fold a cold one while the stream of another
/// reads on; the map's own lock is held only to find the slot.
pub(crate) fn tracked(path: &Path) -> Option<Seen> {
    /// Every transcript's slot, and when the idle ones were last let go.
    type Slots = (HashMap<PathBuf, Arc<Slot>>, i64);
    static CACHE: OnceLock<Mutex<Slots>> = OnceLock::new();
    let now = now_ms();
    let slot = {
        let mut guard = CACHE
            .get_or_init(Default::default)
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let (map, swept) = &mut *guard;
        // A router runs for weeks: what nobody has asked about for a while is let go.
        if now - *swept > 60_000 {
            *swept = now;
            map.retain(|_, s| now - s.used_ms.load(Ordering::Relaxed) < TRACKED_IDLE_MS);
        }
        map.entry(path.to_path_buf())
            .or_insert_with(|| {
                Arc::new(Slot {
                    used_ms: AtomicI64::new(now),
                    tracked: Mutex::new(None),
                })
            })
            .clone()
    };
    slot.used_ms.store(now, Ordering::Relaxed);
    let mut guard = slot.tracked.lock().unwrap_or_else(|e| e.into_inner());

    // Opened first and measured through the handle, so the length and the bytes read
    // are the same file even if it is replaced in between.
    let mut file = fs::File::open(path).ok()?;
    let meta = file.metadata().ok()?;
    let id = file_identity(&file, &meta);
    let len = meta.len();
    let stale = match guard.as_ref() {
        Some(t) => t.id != id || len < t.offset || len - t.offset > CATCH_UP_LIMIT,
        None => true,
    };
    if stale {
        let offset = crate::fold::initial_offset(len);
        *guard = Some(Tracked {
            id,
            offset,
            carry: Vec::new(),
            skip_first_line: offset > 0,
            fold: Fold::new(),
            summary: FoldSummary::default(),
            head: read_head(path),
        });
    }
    let t = guard.as_mut()?;
    if len > t.offset {
        let mut fresh = Vec::new();
        if file.seek(SeekFrom::Start(t.offset)).is_ok() {
            let _ = (&mut file).take(len - t.offset).read_to_end(&mut fresh);
        }
        if !fresh.is_empty() {
            t.offset += fresh.len() as u64;
            let mut buf = std::mem::take(&mut t.carry);
            buf.extend_from_slice(&fresh);
            let skip = crate::fold::skip_partial_line(&buf, t.skip_first_line);
            t.skip_first_line = false;
            let used = t.fold.feed(&buf[skip..]);
            t.carry = buf[skip + used..].to_vec();
            t.summary = t.fold.summary();
        }
    }
    Some(Seen {
        fold: t.summary.clone(),
        head: t.head.clone(),
        bytes: len,
        mtime_ms: mtime_ms(&meta),
    })
}

/// Whether a wrapped session's control socket answers, asked at most once a
/// `WRAPPER_PROBE_TTL` per session. `peer::wrapper_alive` also clears away what a dead
/// wrapper left behind.
fn wrapper_answers(dir: &Path, session_id: &str) -> bool {
    static SEEN: OnceLock<Mutex<HashMap<PathBuf, (Instant, bool)>>> = OnceLock::new();
    let memo = SEEN.get_or_init(Default::default);
    let key = dir.join(session_id);
    if let Some((at, alive)) = memo.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
        if at.elapsed() < WRAPPER_PROBE_TTL {
            return *alive;
        }
    }
    let alive = crate::peer::wrapper_alive_in(dir, session_id);
    let mut memo = memo.lock().unwrap_or_else(|e| e.into_inner());
    if memo.len() > 1024 {
        memo.retain(|_, (at, _)| at.elapsed() < Duration::from_secs(60));
    }
    memo.insert(key, (Instant::now(), alive));
    alive
}

/// A wrapper prompt as the endpoints need it: what `derive` reads, and the request
/// itself, which an answer has to echo back.
#[derive(Clone, Debug)]
pub(crate) struct LivePrompt {
    pub prompt: WrapperPrompt,
    pub raw: Value,
}

/// What the router remembers about one session's prompts between reads.
#[derive(Default)]
struct PromptMemo {
    /// request id -> when this router first saw it. The order prompts arrived in, which
    /// an old wrapper's file does not keep (it sorted them by request id).
    first_seen: HashMap<String, i64>,
    /// The previous read, as read.
    last: Vec<Value>,
    used_ms: i64,
}

/// The open `can_use_tool` requests of a wrapped session whose wrapper answers, oldest
/// first by when they were first seen (ties keep the file's order), minus any whose tool
/// call the transcript shows already resolved — answered in the editor, cancelled by the
/// harness, or answered over a channel the wrapper cannot see.
///
/// An old wrapper rewrites its file by truncating it first, so a read now and then finds
/// it empty mid-write (0.02–1.5 % of reads, measured): a prompt missing from one read
/// that was there in the one before is kept for one more read.
fn wrapper_prompts(
    dir: &Path,
    session_id: &str,
    transcript: Option<&Path>,
    now: i64,
) -> Vec<LivePrompt> {
    static MEMO: OnceLock<Mutex<HashMap<PathBuf, PromptMemo>>> = OnceLock::new();
    let read = crate::peer::read_pending_in(dir, session_id);
    let request_id = |v: &Value| v["request_id"].as_str().unwrap_or_default().to_owned();
    let dated: Vec<(i64, Value)> = {
        let mut all = MEMO
            .get_or_init(Default::default)
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if all.len() > 256 {
            all.retain(|_, m| now - m.used_ms < 3_600_000);
        }
        let memo = all.entry(dir.join(session_id)).or_default();
        // The last read is only the *previous* read if it was recent: across a long gap —
        // the wrapper gone and the session resumed under a new one, or nobody asking — a
        // prompt it held is history, not a torn read.
        if now - memo.used_ms > 30_000 {
            memo.last.clear();
        }
        memo.used_ms = now;
        let present: HashSet<String> = read.iter().map(request_id).collect();
        let mut values: Vec<Value> = memo
            .last
            .iter()
            .filter(|v| !present.contains(&request_id(v)))
            .cloned()
            .collect();
        values.extend(read.iter().cloned());
        memo.last = read;
        let ids: HashSet<String> = values.iter().map(request_id).collect();
        memo.first_seen.retain(|id, _| ids.contains(id));
        let mut dated: Vec<(i64, Value)> = values
            .into_iter()
            .map(|v| (*memo.first_seen.entry(request_id(&v)).or_insert(now), v))
            .collect();
        dated.sort_by_key(|(first, _)| *first);
        dated
    };
    dated
        .into_iter()
        .filter(|(_, v)| {
            let settled = match (transcript, v["request"]["tool_use_id"].as_str()) {
                (Some(path), Some(id)) => tool_settled(path, id),
                _ => false,
            };
            !settled
        })
        .filter_map(|(first_seen_ms, raw)| {
            let asked = crate::notify::prompt_of(&raw)?;
            // The notification's words, so the header and the banner say the same thing.
            let (headline, summary) = crate::notify::describe(&asked);
            let tool_use_id = raw["request"]["tool_use_id"].as_str().map(str::to_owned);
            Some(LivePrompt {
                prompt: WrapperPrompt {
                    request_id: asked.request_id,
                    tool: asked.tool,
                    tool_use_id,
                    headline,
                    summary,
                    first_seen_ms,
                },
                raw,
            })
        })
        .collect()
}

/// The newest write under a session's subagent transcripts: `<session>/subagents/*.jsonl`
/// and each workflow's `<session>/subagents/workflows/<wf>/*.jsonl`. Background agents
/// write there block by block while the main transcript sits silent after its turn, so
/// this is the only heartbeat they have.
fn subagent_write_ms(transcript: &Path) -> Option<i64> {
    /// transcript -> (when looked, the newest write then).
    type Looked = HashMap<PathBuf, (Instant, Option<i64>)>;
    static SEEN: OnceLock<Mutex<Looked>> = OnceLock::new();
    let memo = SEEN.get_or_init(Default::default);
    if let Some((at, ms)) = memo.lock().unwrap_or_else(|e| e.into_inner()).get(transcript) {
        if at.elapsed() < SUBAGENT_TTL {
            return *ms;
        }
    }
    fn newest_in(dir: &Path) -> Option<i64> {
        fs::read_dir(dir)
            .ok()?
            .flatten()
            .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("jsonl"))
            .filter_map(|e| e.metadata().ok().map(|m| mtime_ms(&m)))
            .max()
    }
    let base = transcript.with_extension("").join("subagents");
    let mut newest = newest_in(&base);
    for wf in fs::read_dir(base.join("workflows")).into_iter().flatten().flatten() {
        if wf.file_type().is_ok_and(|t| t.is_dir()) {
            newest = newest.max(newest_in(&wf.path()));
        }
    }
    let mut memo = memo.lock().unwrap_or_else(|e| e.into_inner());
    if memo.len() > 1024 {
        memo.retain(|_, (at, _)| at.elapsed() < Duration::from_secs(60));
    }
    memo.insert(transcript.to_path_buf(), (Instant::now(), newest));
    newest
}

/// Whether the derivation needs the subagent heartbeat for this session: background
/// agents or workflows outstanding (the Background class looks for their writes), the
/// registry busy after the turn is over (the writes are its only activity), or a turn
/// quiet long enough for the stalled check, which subagents writing must hold off.
fn needs_subagents(f: &FoldSummary, r: Option<&ProcRecord>, now: i64) -> bool {
    let counts = crate::activity::turn_counts(f, r);
    let busy = r.and_then(|r| r.status.as_ref()) == Some(&RegStatus::Busy);
    let quiet = max_opt(f.last_content_ts, r.and_then(|r| r.status_updated_ms))
        .is_none_or(|t| now - t > crate::activity::STALLED_AFTER_MS);
    f.bg.agents + f.bg.workflows > 0
        || (busy && !counts)
        || (counts && f.pending.is_empty() && quiet)
}

/// Everything the router knows about one session at one moment, and what it makes of it.
#[derive(Clone, Debug)]
pub(crate) struct Observed {
    pub activity: Activity,
    /// The transcript as of this look; empty when the session has none yet.
    pub seen: Seen,
    pub record: Option<ProcRecord>,
    pub wrapper_alive: bool,
    /// The wrapper prompts that count (`activity::live_prompts`), oldest first: what the
    /// prompt endpoint offers, what an answer may name, what the notifier announces.
    pub prompts: Vec<LivePrompt>,
    pub subagent_write_ms: Option<i64>,
    pub as_of_ms: i64,
}

impl Observed {
    /// The newest content — a transcript line, or a subagent's write when that was looked
    /// at — or failing both, when the session began, then the file's mtime. Always a
    /// number: an installed iPhone decodes the list strictly.
    pub fn last_activity_ms(&self) -> i64 {
        max_opt(self.seen.fold.last_content_ts, self.subagent_write_ms)
            .or(self.seen.head.created_ms)
            .unwrap_or(self.seen.mtime_ms)
    }

    /// The legacy `wrapped`: not known to be closed, and the wrapper answering.
    pub fn wrapped(&self) -> bool {
        self.activity.open != Some(false) && self.wrapper_alive
    }
}

/// What one session is doing, from everything there is to go on (§2–§3 of the design;
/// docs/session-liveness.md). Blocking: it reads files and may wait on the registry, so
/// async handlers call it from `spawn_blocking`.
pub(crate) fn session_activity(session_id: &str, now: i64) -> Observed {
    let reg = crate::registry::snapshot();
    let roots = Roots::system();
    let path = transcript_path_in(&roots.projects, session_id);
    observe(&roots, &reg, session_id, path.as_deref(), now)
}

/// `session_activity` with every input explicit.
pub(crate) fn observe(
    roots: &Roots,
    reg: &Snapshot,
    session_id: &str,
    path: Option<&Path>,
    now: i64,
) -> Observed {
    let record = reg
        .available
        .then(|| reg.by_session.get(session_id))
        .flatten();
    let seen = path.and_then(tracked).unwrap_or_default();
    // Asked only of a session a process holds, or when nothing else can tell: probing a
    // closed session's socket would only find it dead.
    let wrapper_alive =
        (!reg.available || record.is_some()) && wrapper_answers(&roots.wrapper, session_id);
    let prompts = if wrapper_alive {
        wrapper_prompts(&roots.wrapper, session_id, path, now)
    } else {
        Vec::new()
    };
    let closed = reg.available && record.is_none();
    let subagent_write_ms = match path {
        Some(p) if !closed && needs_subagents(&seen.fold, record, now) => subagent_write_ms(p),
        _ => None,
    };
    let offered: Vec<WrapperPrompt> = prompts.iter().map(|p| p.prompt.clone()).collect();
    let evidence = Evidence {
        now_ms: now,
        registry_available: reg.available,
        record,
        wrapper_alive,
        prompts: &offered,
        fold: &seen.fold,
        subagent_write_ms,
    };
    let activity = crate::activity::derive(&evidence);
    let counted: HashSet<String> = crate::activity::live_prompts(&evidence)
        .into_iter()
        .map(|p| p.request_id.clone())
        .collect();
    let prompts = prompts
        .into_iter()
        .filter(|p| counted.contains(&p.prompt.request_id))
        .collect();
    Observed {
        activity,
        seen,
        record: record.cloned(),
        wrapper_alive,
        prompts,
        subagent_write_ms,
        as_of_ms: now,
    }
}

fn max_opt(a: Option<i64>, b: Option<i64>) -> Option<i64> {
    match (a, b) {
        (Some(x), Some(y)) => Some(x.max(y)),
        (x, None) => x,
        (None, y) => y,
    }
}

// ---------------------------------------------------------------------------
// HTTP
// ---------------------------------------------------------------------------

pub async fn sessions() -> Response {
    // Off the async workers: it reads the registry, stats every recent transcript, and
    // folds the ones it has not seen yet.
    match tokio::task::spawn_blocking(discover).await {
        Ok(list) => ([(axum::http::header::CACHE_CONTROL, "no-store")], Json(list)).into_response(),
        Err(e) => bad(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
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
pub async fn send(JsonBody(body): LiveBody<SendBody>) -> Response {
    let text = body.text.trim().to_string();
    if text.is_empty() {
        return bad(axum::http::StatusCode::BAD_REQUEST, "Nothing to send.");
    }
    if crate::codex::live::is_codex(&body.session) {
        return crate::codex::live::unsupported();
    }
    if crate::kimi::live::is_kimi(&body.session) {
        return crate::kimi::live::send(crate::kimi::live::host(), &body.session, &text).await;
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
        Ok(Err(e)) => {
            let (code, why) = delivery_failure(e);
            bad(code, &why)
        }
        Err(e) => bad(axum::http::StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

/// A live endpoint's refusal: `{ok: false, reason}`. A body the endpoint cannot read is
/// refused before the handler runs, with the same words under `error` too (`LiveBody`).
fn bad(code: axum::http::StatusCode, reason: &str) -> Response {
    (code, Json(serde_json::json!({ "ok": false, "reason": reason }))).into_response()
}

/// How a failed write into a session is reported. A frame too long for the wrapper was
/// refused before a byte of it went (`peer::TooLong`): the sender's to fix, 413. Anything
/// else means the session could not be reached, 503.
fn delivery_failure(e: anyhow::Error) -> (StatusCode, String) {
    match e.downcast_ref::<crate::peer::TooLong>() {
        Some(t) => (
            StatusCode::PAYLOAD_TOO_LARGE,
            // In KiB, both of them: a frame one byte over would otherwise read "1.00 MB"
            // beside a limit of "1 MB".
            format!(
                "That is too long to send: {} KiB, and a session takes at most {} KiB at a \
                 time — anything longer could end the session. Split it up, or save it to a \
                 file and ask the session to read it.",
                t.bytes.div_ceil(1024),
                crate::peer::WRAPPER_LINE_MAX / 1024
            ),
        ),
        None => (StatusCode::SERVICE_UNAVAILABLE, e.to_string()),
    }
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
#[derive(Serialize, Default, Clone)]
pub struct Stats {
    pub model: Option<String>,
    pub effort: Option<String>,
    /// The permission mode in force, as the session last stamped it on a turn.
    pub mode: Option<String>,
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
    let out = tokio::task::spawn_blocking(move || {
        if crate::codex::live::is_codex(&q.session) {
            let mut st = crate::codex::session::find(&q.session)?.stats();
            st.cost = price(&st);
            return Some(st);
        }
        // Kimi Code's sessions are read from its own files (`kimi::session`).
        if let Some(kimi) = crate::kimi::session::find(&q.session) {
            let mut st = kimi.stats();
            st.cost = price(&st);
            return Some(st);
        }
        transcript_path(&q.session).map(|path| scan_stats(&path))
    })
    .await;
    let out = match out {
        Ok(Some(st)) => Ok(st),
        Ok(None) => return bad(axum::http::StatusCode::NOT_FOUND, "No such session."),
        Err(e) => Err(e),
    };
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
        // Stamped on each turn the same way, so the last one is the mode in force now.
        // This is why showing the mode needs no wrapper change: a mode changed with
        // shift+tab in the editor lands here too, not only one we set ourselves.
        if let Some(m) = v.get("permissionMode").and_then(|x| x.as_str()) {
            st.mode = Some(m.to_string());
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
    crate::config::db_path().unwrap_or_else(|_| {
        PathBuf::from(std::env::var("HOME").unwrap_or_default())
            .join(".agent-router")
            .join("agent-router.db")
    })
}

#[derive(Deserialize)]
pub struct ProjectQuery {
    /// The project's key: the repository its sessions work in, else their directory
    /// (`project_of`).
    pub project: String,
}

/// A listed session's project: the repository its directory is a checkout of, else the
/// directory. The key the router's own reads take a project by (`sessions.project`).
pub fn project_of(s: &LiveSession) -> Option<&str> {
    s.repository.as_deref().or(s.cwd.as_deref())
}

/// Everything a project has spent, summed over the sessions on screen: on this machine, and
/// on a hosted router over every machine's (`machines::api::project`), so one repository's
/// clones and worktrees add up wherever they are.
///
/// Archived and long-dead sessions are left out deliberately: the number is meant to
/// answer "what is this project costing me now", and a year of finished sessions
/// answers a different question.
pub async fn project_stats(Query(q): Query<ProjectQuery>) -> Response {
    let out = tokio::task::spawn_blocking(move || {
        let reg = crate::registry::snapshot();
        project_stats_in(&Roots::system(), &reg, &archived_set(), &q.project, now_ms())
    })
    .await;
    match out {
        Ok(totals) => Json(totals).into_response(),
        Err(e) => bad(axum::http::StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

/// `project_stats` over the sessions these roots list.
fn project_stats_in(roots: &Roots, reg: &Snapshot, archived: &HashMap<String, i64>, project: &str, now: i64) -> Value {
    let mut total = Stats::default();
    let mut spend = 0.0f64;
    let mut sessions = 0i64;
    let mut models: std::collections::BTreeSet<String> = Default::default();

    let mut listed = discover_in(roots, reg, archived, now);
    if roots.projects == projects_root() {
        listed.extend(crate::kimi::session::discover(archived, now));
        listed.extend(crate::codex::session::discover(archived, now));
    }
    for s in listed {
        if project_of(&s) != Some(project) {
            continue;
        }
        if is_shelved(archived, &s.session_id, s.last_activity_ms) {
            continue;
        }
        let st = if crate::codex::live::is_codex(&s.session_id) {
            let Some(session) = crate::codex::session::find(&s.session_id) else { continue };
            let mut st = session.stats(); st.cost = price(&st); st
        } else if crate::kimi::live::is_kimi(&s.session_id) {
            let Some(session) = crate::kimi::session::find(&s.session_id) else { continue };
            let mut st = session.stats(); st.cost = price(&st); st
        } else {
            let Some(path) = transcript_path_in(&roots.projects, &s.session_id) else { continue };
            scan_stats(&path)
        };
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
    serde_json::json!({
        "sessions": sessions,
        "models": models.into_iter().collect::<Vec<_>>(),
        "total": total.total,
        "output": total.output,
        "cache_read": total.cache_read,
        "turns": total.turns,
        "cost": spend,
    })
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
pub async fn archive(JsonBody(body): LiveBody<ArchiveBody>) -> Response {
    let exists = if crate::codex::live::is_codex(&body.session) {
        crate::codex::session::find(&body.session).is_some()
    } else if crate::kimi::live::is_kimi(&body.session) {
        crate::kimi::session::find(&body.session).is_some()
    } else { transcript_path(&body.session).is_some() };
    if !exists {
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
    /// Question text -> the answer chosen for it. An AskUserQuestion is answered this
    /// way and always has been, so a client that knows nothing of the fields below
    /// keeps working unchanged.
    #[serde(default)]
    pub answers: std::collections::HashMap<String, String>,
    /// A permission request: let it through, or refuse it. Absent means "this is a
    /// question", which is how the older clients speak.
    #[serde(default)]
    pub allow: Option<bool>,
    /// Which of the prompt's own offered suggestions to also apply, by index.
    #[serde(default)]
    pub suggestion: Option<usize>,
    /// What to tell the model when refusing.
    #[serde(default)]
    pub message: Option<String>,
    /// The prompt being answered, as `/api/live/prompt` named it. With it, exactly that
    /// prompt is answered or none is; without it (an older client) an answer goes through
    /// only when a single prompt is open, so a tap can never land on a prompt its sender
    /// never saw.
    #[serde(default)]
    pub request_id: Option<String>,
}

/// What the session is waiting to be asked, so the UI can answer the live prompt rather
/// than the one its transcript happened to record: the newest of the prompts that count
/// (the one the session's status names), and how many there are in all.
pub async fn prompt(Query(q): Query<StreamQuery>) -> Response {
    if crate::codex::live::is_codex(&q.session) {
        return crate::codex::live::prompt();
    }
    if crate::kimi::live::is_kimi(&q.session) {
        return crate::kimi::live::prompt(&q.session).await;
    }
    let session = q.session.clone();
    let seen = tokio::task::spawn_blocking(move || session_activity(&session, now_ms())).await;
    let o = match seen {
        Ok(o) => o,
        Err(e) => return bad(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    let named = match (&o.activity.kind, &o.activity.waiting_on) {
        (Kind::Waiting, Some(w)) => w.request_id.clone(),
        _ => None,
    };
    let Some(p) = named.and_then(|id| o.prompts.iter().find(|p| p.prompt.request_id == id)) else {
        return Json(serde_json::json!({ "pending": false, "count": 0 })).into_response();
    };
    let v = &p.raw;
    let req = &v["request"];
    let tool = req["tool_name"].as_str().unwrap_or_default().to_string();
    // Two kinds of prompt reach the same place. A question carries its own options and
    // is answered by choosing among them; everything else is a permission request,
    // answered by allowing or refusing it.
    let question = tool == ASK_TOOL;
    let offered = req["permission_suggestions"].as_array().cloned().unwrap_or_default();
    let suggestions: Vec<serde_json::Value> = offered
        .iter()
        .enumerate()
        .map(|(i, s)| serde_json::json!({ "index": i, "label": suggestion_label(s) }))
        .collect();
    Json(serde_json::json!({
        "pending": true,
        "request_id": v["request_id"],
        "tool": tool,
        "question": question,
        // What the editor puts in its dialog: the tool's own summary of what it is about
        // to do, and why it had to ask.
        "display_name": req["display_name"],
        "description": req["description"],
        "reason": req["decision_reason"],
        "input": req["input"],
        // Only ever what the prompt offered; see `peer::allow_prompt_in`.
        "suggestions": suggestions,
        // More than one can be open at once — parallel calls, a subagent's beside the
        // main thread's. This is the newest; answering it brings up the next.
        "count": o.prompts.len(),
    }))
    .into_response()
}

/// Whether the tool call a prompt was asking about has already been resolved.
///
/// A prompt clears from `<session>.pending` when the wrapper sees the answer go past on
/// stdin — but an answer can happen out of its sight: a tool call cancelled by the
/// harness, or a client that answered over a channel of its own. The transcript settles
/// it either way, because a resolved call always ends with a `tool_result` carrying its
/// `tool_use_id`, whether it was allowed, refused or cancelled.
///
/// Without this a prompt that is long gone keeps a session showing as waiting and offers
/// buttons that answer nothing.
///
/// The whole file is read the first time, not the tail: the result sits beside its
/// *request*, which may be hours and megabytes back. After that only what was appended:
/// an unsettled id is remembered with how far it has been checked, and a result is
/// always written after its call, so nothing before that point can settle it. A settled
/// id never unsettles, so it is remembered outright. A session that is genuinely blocked
/// writes nothing more, so the status stream asking five times a second costs a `stat`.
fn tool_settled(path: &Path, tool_use_id: &str) -> bool {
    use std::io::BufRead;

    type Key = (PathBuf, String);
    /// Settled for good, and unsettled with how far the file has been checked.
    type Memo = (HashSet<Key>, HashMap<Key, u64>);
    static SEEN: OnceLock<Mutex<Memo>> = OnceLock::new();
    let seen = SEEN.get_or_init(Default::default);
    let key: Key = (path.to_path_buf(), tool_use_id.to_owned());
    let len = fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    let from = {
        let mut memo = seen.lock().unwrap_or_else(|e| e.into_inner());
        if memo.0.contains(&key) {
            return true;
        }
        let from = memo.1.get(&key).copied().unwrap_or(0);
        if from == len {
            return false;
        }
        // A router runs for weeks; neither memo is allowed to grow without bound.
        if memo.0.len() > 512 {
            memo.0.clear();
        }
        if memo.1.len() > 512 {
            memo.1.clear();
        }
        // Shorter than when last checked: not the same file any more.
        if from > len {
            0
        } else {
            from
        }
    };

    let Ok(mut f) = fs::File::open(path) else { return false };
    if f.seek(SeekFrom::Start(from)).is_err() {
        return false;
    }
    let mut reader = std::io::BufReader::new(f.take(len - from));
    let mut checked = from;
    let mut found = false;
    let mut line = Vec::new();
    loop {
        line.clear();
        match reader.read_until(b'\n', &mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        // A line still being written is left for the next look, whole.
        if line.last() != Some(&b'\n') {
            break;
        }
        checked += line.len() as u64;
        // Cheap reject first: almost no line mentions this id at all.
        if !String::from_utf8_lossy(&line).contains(tool_use_id) {
            continue;
        }
        let Ok(v) = serde_json::from_slice::<Value>(&line) else { continue };
        let blocks = v["message"]["content"].as_array().map(|a| a.as_slice()).unwrap_or_default();
        if blocks.iter().any(|b| b["tool_use_id"] == tool_use_id) {
            found = true;
            break;
        }
    }

    let mut memo = seen.lock().unwrap_or_else(|e| e.into_inner());
    if found {
        memo.1.remove(&key);
        memo.0.insert(key);
    } else {
        memo.1.insert(key, checked);
    }
    found
}

/// A human sentence for one of the prompt's own `permission_suggestions`, so three
/// clients do not each invent their own wording for the same offer.
fn suggestion_label(s: &Value) -> String {
    let scope = match s["destination"].as_str() {
        Some("session") => "for the rest of this session",
        Some("localSettings") => "in this project",
        Some("projectSettings") => "in this project, for everyone",
        Some("userSettings") => "everywhere",
        _ => "from now on",
    };
    match s["type"].as_str() {
        Some("addRules") => {
            let rules = s["rules"].as_array().map(|a| a.as_slice()).unwrap_or_default();
            let what = rules
                .iter()
                .map(|r| match (r["toolName"].as_str(), r["ruleContent"].as_str()) {
                    (Some(t), Some(c)) => format!("{t}({c})"),
                    (Some(t), None) => t.to_string(),
                    _ => "this tool".to_string(),
                })
                .collect::<Vec<_>>()
                .join(", ");
            let verb = if s["behavior"] == "deny" { "Never allow" } else { "Always allow" };
            format!("{verb} {what} {scope}")
        }
        Some("setMode") => {
            format!("Switch to {} mode {scope}", s["mode"].as_str().unwrap_or("another"))
        }
        Some("addDirectories") => format!("Allow these directories {scope}"),
        _ => format!("Remember this choice {scope}"),
    }
}

/// Which open prompt an answer is for: the one it names, if that is still open; for a
/// client that names none, the only one. Never a guess among several — with two open, or
/// one replaced between the poll and the tap, the guess approves a prompt the person
/// never saw, and may apply a standing permission they chose for a different one.
fn pick_prompt<'a>(
    open: &'a [LivePrompt],
    named: Option<&str>,
) -> Result<&'a LivePrompt, (StatusCode, &'static str)> {
    match named {
        Some(id) => open
            .iter()
            .find(|p| p.prompt.request_id == id)
            .ok_or((StatusCode::CONFLICT, "That prompt is no longer waiting")),
        None => match open {
            [] => Err((
                StatusCode::SERVICE_UNAVAILABLE,
                "That session isn't waiting on anything right now.",
            )),
            [only] => Ok(only),
            _ => Err((
                StatusCode::CONFLICT,
                "More than one prompt is waiting — refresh and answer from the list",
            )),
        },
    }
}

/// Answer the pending question — the editor's Submit, from anywhere.
pub async fn answer(JsonBody(body): LiveBody<AnswerBody>) -> Response {
    if crate::codex::live::is_codex(&body.session) {
        return crate::codex::live::unsupported();
    }
    if crate::kimi::live::is_kimi(&body.session) {
        return crate::kimi::live::answer(crate::kimi::live::host(), &body).await;
    }
    let session = body.session.clone();
    let done = tokio::task::spawn_blocking(move || {
        let reg = crate::registry::snapshot();
        answer_in(&Roots::system(), &reg, &body, now_ms())
    })
    .await;

    match done {
        Ok(Ok(id)) => {
            tracing::info!(session = %session, request_id = %id, "answered a prompt from the live view");
            Json(serde_json::json!({ "ok": true, "request_id": id })).into_response()
        }
        Ok(Err((code, why))) => bad(code, &why),
        Err(e) => bad(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

/// `answer` with every input explicit: the request id of the prompt answered, or why not.
fn answer_in(
    roots: &Roots,
    reg: &Snapshot,
    body: &AnswerBody,
    now: i64,
) -> Result<String, (StatusCode, String)> {
    let session = body.session.as_str();
    let unavailable = |e: anyhow::Error| (StatusCode::SERVICE_UNAVAILABLE, e.to_string());
    // Whatever it is waiting on, not only a question: a Bash approval left unanswered is
    // the case this exists for. The same prompts the session's status counts, so what the
    // list says is waiting is exactly what can be answered.
    let path = transcript_path_in(&roots.projects, session);
    let o = observe(roots, reg, session, path.as_deref(), now);
    let req = &pick_prompt(&o.prompts, body.request_id.as_deref())
        .map_err(|(code, why)| (code, why.to_owned()))?
        .raw;
    let request_id = req["request_id"]
        .as_str()
        .ok_or_else(|| unavailable(anyhow::anyhow!("the pending request has no id")))?
        .to_string();
    let tool = req["request"]["tool_name"].as_str().unwrap_or_default().to_string();

    // A refusal needs no input at all, so it is decided first.
    if body.allow == Some(false) {
        let why = body.message.as_deref().unwrap_or("Declined from the live view.");
        return crate::peer::deny_prompt_in(&roots.wrapper, session, &request_id, why)
            .map(|()| request_id)
            .map_err(delivery_failure);
    }
    if let Some(why) = approval_refusal(body.allow, &tool) {
        return Err((StatusCode::CONFLICT, why));
    }
    if let Some(why) = answers_refusal(&tool, &req["request"]["input"], &body.answers) {
        return Err((StatusCode::CONFLICT, why.to_owned()));
    }

    // Only a suggestion the prompt itself offered may be sent back; anything else is
    // discarded by the client that receives it, so refusing here says so plainly.
    let updates = match body.suggestion {
        None => None,
        Some(i) => Some(req["request"]["permission_suggestions"].get(i).cloned().ok_or_else(
            || unavailable(anyhow::anyhow!("this prompt does not offer that option")),
        )?),
    };

    let mut input = req["request"]["input"].clone();
    if tool == ASK_TOOL {
        // The answers ride inside the tool's own input, beside the questions they answer
        // — the shape the tool reads when it builds its result.
        input["answers"] = serde_json::to_value(&body.answers).map_err(|e| unavailable(e.into()))?;
    }
    // Every other tool gets its input back untouched, which is plain approval.
    crate::peer::allow_prompt_in(&roots.wrapper, session, &request_id, input, &tool, updates)
        .map(|()| request_id)
        .map_err(delivery_failure)
}

#[derive(Deserialize)]
pub struct InterruptBody {
    pub session: String,
}

/// The levels the CLI's `/effort` accepts, in order. `max` is silently downgraded by
/// the CLI for models that do not offer it.
pub const EFFORTS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];

/// The permission modes the CLI accepts, with the label to show for each. Taken from
/// the CLI itself, which names them when it refuses one it does not know: `acceptEdits,
/// auto, bypassPermissions, default, dontAsk, plan`.
///
/// Two can be refused at the moment they are asked for, whatever is listed here:
/// `bypassPermissions` unless the session was launched with
/// `--dangerously-skip-permissions` (or it is disabled by settings), and `auto` when its
/// gate is off. Both refusals come back from the CLI with a reason, which is why
/// `set_mode` waits for the answer instead of assuming.
pub const MODES: [(&str, &str); 6] = [
    ("default", "Ask"),
    ("acceptEdits", "Accept edits"),
    ("auto", "Auto"),
    ("dontAsk", "Don't ask"),
    ("plan", "Plan"),
    ("bypassPermissions", "Bypass"),
];

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

/// What the UIs offer, from one place. `models` is every name a session can be moved to,
/// as every client before there was a second provider reads it; `by_provider` is the same
/// names grouped by the provider that serves them, Claude's aliases under Anthropic and each
/// other provider's models from the catalogue (`models`) under it, for a picker to show.
pub async fn options(axum::extract::State(state): axum::extract::State<Arc<crate::proxy::AppState>>) -> Response {
    let catalogue = match state.reads.get().and_then(|conn| Ok(crate::models::catalogue(&conn)?)) {
        Ok(catalogue) => catalogue,
        Err(e) => return bad(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    let mut models: Vec<String> = Vec::new();
    let mut anthropic: Vec<Value> = Vec::new();
    for a in MODEL_ALIASES {
        for name in [a.to_string(), format!("{a}[1m]")] {
            anthropic.push(serde_json::json!({ "id": name, "display_name": name }));
            models.push(name);
        }
    }
    let mut by_provider = vec![serde_json::json!({
        "provider": crate::accounts::Provider::Anthropic.as_str(),
        "name": crate::accounts::Provider::Anthropic.name(),
        "models": anthropic,
    })];
    // Claude's own ids stay behind its aliases, as the CLI's picker has them.
    let mut others: Vec<(crate::accounts::Provider, Vec<Value>)> = Vec::new();
    for m in catalogue.iter().filter(|m| m.provider != crate::accounts::Provider::Anthropic) {
        let entry = serde_json::json!({ "id": m.id, "display_name": m.display_name });
        models.push(m.id.clone());
        match others.iter_mut().find(|(p, _)| *p == m.provider) {
            Some((_, list)) => list.push(entry),
            None => others.push((m.provider, vec![entry])),
        }
    }
    for (provider, list) in others {
        by_provider.push(serde_json::json!({ "provider": provider.as_str(), "name": provider.name(), "models": list }));
    }
    let modes: Vec<serde_json::Value> = MODES
        .iter()
        .map(|(value, label)| serde_json::json!({ "value": value, "label": label }))
        .collect();
    Json(serde_json::json!({ "models": models, "by_provider": by_provider, "efforts": EFFORTS, "modes": modes })).into_response()
}

/// Why no source serves `model`, if none does: a name the CLI knows as one of Claude's
/// aliases, or that is Anthropic's, is taken; any other must be a model of a provider the
/// router has an account at, as the proxy would route it (`models::account_for`).
fn unserved(state: &crate::proxy::AppState, model: &str) -> anyhow::Result<Option<(StatusCode, String)>> {
    let base = model.trim_end_matches("[1m]");
    if MODEL_ALIASES.contains(&base) {
        return Ok(None);
    }
    let provider = crate::models::provider_of(model);
    if provider == Some(crate::accounts::Provider::Anthropic)
        || crate::models::account_for(&*state.reads.get()?, model)?.is_some()
    {
        return Ok(None);
    }
    let why = format!("no source serves the model {model:?}");
    // A provider's model with no account to serve it waits on the store; any other name
    // waits on nothing the router holds.
    let code = if provider.is_some() { StatusCode::CONFLICT } else { StatusCode::BAD_REQUEST };
    Ok(Some((code, why)))
}

/// The router's refusal of a session's move to `model`, if nothing serves it. Asked by the
/// router before anything reaches the session, on this machine (`set_model_checked`) and on
/// a hosted router before the request goes down a machine's tunnel; the machine agent's own
/// `set_model` has no catalogue to ask.
pub fn refuse_unserved(state: &crate::proxy::AppState, model: &str) -> Option<Response> {
    match unserved(state, model.trim()) {
        Ok(None) => None,
        Ok(Some((code, why))) => Some(bad(code, &why)),
        Err(e) => Some(bad(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string())),
    }
}

/// `set_model` on a router: a model nothing serves is refused before the session is asked,
/// since it would only fail the session's next turn.
pub async fn set_model_checked(
    axum::extract::State(state): axum::extract::State<Arc<crate::proxy::AppState>>,
    JsonBody(body): LiveBody<ModelBody>,
) -> Response {
    if valid_model(&body.model) {
        if let Some(refused) = refuse_unserved(&state, &body.model) {
            return refused;
        }
    }
    set_model(JsonBody(body)).await
}

#[derive(Deserialize)]
pub struct ModelBody {
    pub session: String,
    pub model: String,
}

/// Change a running session's model — the editor's picker, from anywhere. The CLI
/// confirms or refuses on its control channel; we wait briefly for that answer so a
/// refused name is reported rather than silently ignored.
pub async fn set_model(JsonBody(body): LiveBody<ModelBody>) -> Response {
    let model = body.model.trim().to_string();
    if !valid_model(&model) {
        return bad(axum::http::StatusCode::BAD_REQUEST, "That is not a model name.");
    }
    if crate::codex::live::is_codex(&body.session) {
        return crate::codex::live::unsupported();
    }
    if crate::kimi::live::is_kimi(&body.session) {
        return crate::kimi::live::set_model(crate::kimi::live::host(), &body.session, &model).await;
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
pub async fn set_effort(JsonBody(body): LiveBody<EffortBody>) -> Response {
    if crate::codex::live::is_codex(&body.session) {
        return crate::codex::live::unsupported();
    }
    if crate::kimi::live::is_kimi(&body.session) {
        return crate::kimi::live::unsupported("effort to set");
    }
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

#[derive(Deserialize)]
pub struct ModeBody {
    pub session: String,
    pub mode: String,
}

/// Change a running session's permission mode — what shift+tab cycles in the terminal.
///
/// The CLI can refuse: `bypassPermissions` needs the session to have been launched with
/// `--dangerously-skip-permissions`, and `auto` can be gated. So the answer is waited
/// for and the CLI's own sentence is passed back, rather than reporting a success the
/// session did not agree to.
pub async fn set_mode(JsonBody(body): LiveBody<ModeBody>) -> Response {
    if crate::codex::live::is_codex(&body.session) {
        return crate::codex::live::unsupported();
    }
    if crate::kimi::live::is_kimi(&body.session) {
        return crate::kimi::live::set_mode(crate::kimi::live::host(), &body.session, &body.mode).await;
    }
    let mode = body.mode.trim().to_string();
    if !MODES.iter().any(|(v, _)| *v == mode) {
        return bad(axum::http::StatusCode::BAD_REQUEST, "That is not a permission mode.");
    }
    if !crate::peer::wrapper_alive(&body.session) {
        return bad(axum::http::StatusCode::SERVICE_UNAVAILABLE, "Only a session under the wrapper can change mode; restart it after installing the wrapper.");
    }
    let session = body.session.clone();
    let m = mode.clone();
    let sent =
        tokio::task::spawn_blocking(move || crate::peer::set_mode_via_wrapper(&session, &m)).await;
    let request_id = match sent {
        Ok(Ok(id)) => id,
        Ok(Err(e)) => return bad(axum::http::StatusCode::SERVICE_UNAVAILABLE, &e.to_string()),
        Err(e) => return bad(axum::http::StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    for _ in 0..12 {
        tokio::time::sleep(Duration::from_millis(250)).await;
        if let Some(r) = crate::peer::control_response(&body.session, &request_id) {
            if r["subtype"] == "success" {
                // The CLI echoes the mode it actually took, so the clients can show it
                // at once instead of waiting for the next turn to stamp the transcript.
                let took = r["response"]["mode"].as_str().unwrap_or(&mode).to_string();
                tracing::info!(session = %body.session, mode = %took, "permission mode switched from the live view");
                return Json(serde_json::json!({ "ok": true, "mode": took })).into_response();
            }
            let why = r["error"].as_str().unwrap_or("the session refused that mode").to_string();
            return bad(axum::http::StatusCode::UNPROCESSABLE_ENTITY, &why);
        }
    }
    Json(serde_json::json!({ "ok": true, "mode": mode, "pending": true })).into_response()
}

/// Why an answer must not be applied as an approval, if it must not.
///
/// A body that carries `answers` and no `allow` is how a question is answered, and how
/// every older client speaks. Sent to anything but an AskUserQuestion it would go
/// through as plain approval — and an answer without a `request_id` lands on whatever
/// single prompt is open by the time it arrives. So a phone answering a question that
/// has just gone could approve the Bash command that took its place. Letting a
/// permission request through takes an explicit `allow: true`, which every client that
/// shows one sends.
fn approval_refusal(allow: Option<bool>, tool: &str) -> Option<String> {
    (allow.is_none() && tool != ASK_TOOL).then(|| {
        format!("The waiting prompt asks permission to run {tool}; it is not a question. Answer it with Allow or Deny.")
    })
}

/// Why these answers must not be given to this question, if they must not.
///
/// A question is answered in its own words: the tool builds its result by looking each of
/// its questions up in `answers`, and drops any answer it did not ask. So answers keyed by
/// another question — a phone's form for a question answered in the editor since, sent
/// under the id of the one that took its place — would settle this one as "The user did
/// not answer the questions." while the phone said "Answered."; an empty map would do the
/// same. A question takes at least one answer, and none to anything it did not ask.
fn answers_refusal(
    tool: &str,
    input: &Value,
    answers: &HashMap<String, String>,
) -> Option<&'static str> {
    if tool != ASK_TOOL {
        return None;
    }
    let asked: HashSet<&str> = input["questions"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|q| q["question"].as_str())
        .collect();
    let its_own = !answers.is_empty() && answers.keys().all(|k| asked.contains(k.as_str()));
    (!its_own).then_some("That question is no longer waiting")
}

/// Why a stop must not be sent, if it must not.
///
/// A closed session has nothing to stop. Background work is the subtle one: the turn is
/// over, so the only thing an interrupt (`cancel_queued: true`) could reach is the queue
/// the background agents report back through, and what it does to them is unmeasured —
/// it may discard their results. New clients hide Stop there; old phones show Stop for
/// the legacy `working` that background work maps to, so the refusal lives here. The kind
/// already carries the evidence: busy held past the end with nothing to see (a Stop hook,
/// a task no transcript records) is Background too, since nothing can say it is not
/// background work, while busy that began after the end is Generating and goes through.
fn stop_refusal(kind: Kind) -> Option<&'static str> {
    match kind {
        Kind::Closed => Some("That session isn't running any more."),
        // Background also covers busy held past the end with nothing to show for it — a
        // Stop hook, or work the transcript cannot see — so the reason names neither.
        Kind::Background => Some(
            "The turn is over, but the session is still busy after it (background work or a \
             hook). Stopping now could discard what it is doing — stop it from the editor.",
        ),
        _ => None,
    }
}

/// Stop a running turn — the live view's equivalent of the editor's stop button.
///
/// Only wrapped sessions can be stopped: an interrupt is a control frame on stdin, and
/// the peer socket carries user messages, not control.
pub async fn interrupt(JsonBody(body): LiveBody<InterruptBody>) -> Response {
    if crate::codex::live::is_codex(&body.session) {
        return crate::codex::live::unsupported();
    }
    if crate::kimi::live::is_kimi(&body.session) {
        return crate::kimi::live::interrupt(crate::kimi::live::host(), &body.session).await;
    }
    let session = body.session.clone();
    let done = tokio::task::spawn_blocking(move || -> Result<(), (StatusCode, String)> {
        let kind = session_activity(&session, now_ms()).activity.kind;
        if let Some(why) = stop_refusal(kind) {
            return Err((StatusCode::CONFLICT, why.to_owned()));
        }
        crate::peer::interrupt_via_wrapper(&session)
            .map_err(|e| (StatusCode::SERVICE_UNAVAILABLE, e.to_string()))
    })
    .await;
    match done {
        Ok(Ok(())) => {
            tracing::info!(session = %body.session, "interrupted from the live view");
            Json(serde_json::json!({ "ok": true })).into_response()
        }
        Ok(Err((code, why))) => bad(code, &why),
        Err(e) => bad(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
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
            let real = crate::shim::real_beside(&shim);
            if real.is_file() { real } else { shim }
        })
        .filter(|p| p.is_file())
        .max_by_key(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok());
    if newest.is_some() {
        return newest;
    }
    let home = crate::setup::home_dir().unwrap_or_default();
    #[cfg(windows)]
    let candidates = vec![home.join(".local").join("bin").join("claude.exe")];
    #[cfg(not(windows))]
    let candidates = vec![
        home.join(".local/bin/claude"),
        PathBuf::from("/opt/homebrew/bin/claude"),
        PathBuf::from("/usr/local/bin/claude"),
    ];
    candidates.into_iter().find(|p| p.is_file())
}

/// Which file this is, beyond its name: a transcript replaced under the same path is a
/// new file to read from the start. Device and inode on Unix; on Windows the volume and
/// file index, asked of the open handle. Not the creation time there: NTFS gives a file
/// made under a name that was deleted moments before the old file's creation time.
#[cfg(unix)]
fn file_identity(_file: &fs::File, meta: &fs::Metadata) -> (u64, u64) {
    use std::os::unix::fs::MetadataExt;
    (meta.dev(), meta.ino())
}

#[cfg(windows)]
fn file_identity(file: &fs::File, meta: &fs::Metadata) -> (u64, u64) {
    use std::os::windows::fs::MetadataExt;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION};
    // SAFETY: an all-zero BY_HANDLE_FILE_INFORMATION is valid, and the handle is open.
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
        return (0, meta.creation_time());
    }
    let index = (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow);
    (u64::from(info.dwVolumeSerialNumber), index)
}

/// The last part of a path, whichever separator the machine that wrote it uses.
pub(crate) fn basename(path: &str) -> &str {
    path.trim_end_matches(['/', '\\']).rsplit(['/', '\\']).next().unwrap_or(path)
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
pub async fn aside(JsonBody(body): LiveBody<AsideBody>) -> Response {
    if crate::codex::live::is_codex(&body.session) {
        return crate::codex::live::unsupported();
    }
    if crate::kimi::live::is_kimi(&body.session) {
        return crate::kimi::live::unsupported("side question to ask");
    }
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
    // For as long as the fork runs: its registry record may name this session for a
    // moment while it resumes, and the list must not take this session for the fork.
    let asking = Asking::new(&body.session);
    let run = tokio::task::spawn_blocking(move || {
        let _asking = asking;
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
pub async fn stream(Query(q): Query<StreamQuery>) -> Response {
    if crate::codex::live::is_codex(&q.session) {
        return crate::codex::live::stream(&q.session);
    }
    if crate::kimi::live::is_kimi(&q.session) {
        return crate::kimi::live::stream(&q.session);
    }
    claude_stream(q).into_response()
}

fn claude_stream(q: StreamQuery) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let path = transcript_path(&q.session);
    let session = q.session.clone();

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
        let mut last_ts_ms: Option<i64> = None;
        let mut last_status: Option<StreamStatus> = None;
        let mut last_sent = Instant::now();

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
                            for item in project_value(&v, last_ts_ms) {
                                if let Ok(json) = serde_json::to_string(&item) {
                                    yield Ok(Event::default().data(json));
                                }
                            }
                            if let Some(ts) = ts_ms(&v) {
                                last_ts_ms = Some(ts);
                            }
                        }
                        first_read = false;
                    }
                }
            }
            // Status rides the same stream on a named event, so the header can show
            // what the session is doing without a second connection. The same answer
            // the list gives, worked out the same way: sent when it changes, and once a
            // second otherwise so the elapsed time on the phones keeps counting.
            let (id, p) = (session.clone(), path.clone());
            let seen =
                tokio::task::spawn_blocking(move || observe_session(&id, Some(&p), now_ms())).await;
            if let Ok(o) = seen {
                let st = StreamStatus::of(&o);
                if status_due(last_status.as_ref(), &st, last_sent.elapsed()) {
                    if let Ok(json) = serde_json::to_string(&st) {
                        yield Ok(Event::default().event("status").data(json));
                    }
                    last_status = Some(st);
                    last_sent = Instant::now();
                }
            }

            tokio::time::sleep(POLL).await;
        }
    };

    Sse::new(s).keep_alive(KeepAlive::default())
}

/// `session_activity` for a caller that already knows the transcript.
fn observe_session(session_id: &str, path: Option<&Path>, now: i64) -> Observed {
    let reg = crate::registry::snapshot();
    observe(&Roots::system(), &reg, session_id, path, now)
}

/// What to call a session in a notification: its project, and its title if it has one.
pub fn session_label(session_id: &str) -> Option<(String, Option<String>)> {
    if crate::codex::live::is_codex(session_id) {
        let session = crate::codex::session::find(session_id)?;
        let project = session.cwd.as_deref().map(basename).filter(|s| !s.is_empty()).unwrap_or("session").to_owned();
        return Some((project, session.title.clone()));
    }
    let path = transcript_path(session_id)?;
    let seen = tracked(&path)?;
    let project = seen
        .head
        .cwd
        .as_deref()
        .map(basename)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| "session".into());
    Some((project, seen.fold.title.or(seen.head.title)))
}

/// Transcripts under `root` written within the last `within_ms`, as (session id, path).
/// Only for when there is no registry to say which sessions are open: mtime is the one
/// hint left then.
pub(crate) fn recent_transcripts_in(root: &Path, within_ms: i64) -> Vec<(String, PathBuf)> {
    let now = now_ms();
    let Ok(dirs) = fs::read_dir(root) else {
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

/// Whether this session's transcript still shows this call unanswered — for when there
/// is no registry to ask, asked of the transcript directly and with no recency filter.
/// `recent_transcripts` only reads sessions that moved lately, and a session sitting on a
/// question does not move: after a while it stops being swept although nobody has
/// answered it. Anything that acts on a prompt *leaving* the waiting set has to tell
/// those two apart, and quiet is never an answer.
pub(crate) fn still_asking_in(root: &Path, session_id: &str, ask_id: &str) -> bool {
    transcript_path_in(root, session_id)
        .and_then(|path| tracked(&path))
        .is_some_and(|seen| {
            seen.fold.ask_pending.as_ref().is_some_and(|a| a.id == ask_id)
                || seen.fold.pending.iter().any(|p| p.id == ask_id)
        })
}

/// Fixtures shared by this module's tests and the notifier's: a private copy of the
/// directories live sessions use, transcript lines as the CLI writes them, and registry
/// snapshots made to order — so no test reads, let alone writes, the real ones.
#[cfg(test)]
pub(crate) mod testkit {
    use super::*;
    use crate::registry::civil_from_days;
    use serde_json::json;

    /// Removed when dropped. Short names: a unix socket's path must fit in 104 bytes.
    pub(crate) struct Lab {
        root: PathBuf,
        pub roots: Roots,
    }

    impl Lab {
        pub fn new(tag: &str) -> Lab {
            let root = std::env::temp_dir().join(format!("arl-{tag}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&root);
            let roots = Roots { projects: root.join("p"), wrapper: root.join("w") };
            fs::create_dir_all(roots.projects.join("-work")).unwrap();
            fs::create_dir_all(&roots.wrapper).unwrap();
            Lab { root, roots }
        }

        pub fn path(&self, id: &str) -> PathBuf {
            self.roots.projects.join("-work").join(format!("{id}.jsonl"))
        }

        /// Append whole lines to a session's transcript, creating it if need be.
        pub fn write(&self, id: &str, lines: &[Value]) -> PathBuf {
            use std::io::Write;
            let path = self.path(id);
            let mut f = fs::OpenOptions::new().create(true).append(true).open(&path).unwrap();
            for l in lines {
                writeln!(f, "{l}").unwrap();
            }
            path
        }

        pub fn observe(&self, reg: &Snapshot, id: &str, now: i64) -> Observed {
            let path = self.path(id);
            observe(&self.roots, reg, id, path.is_file().then_some(path.as_path()), now)
        }
    }

    impl Drop for Lab {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    /// Epoch ms as the transcript writes it: `2026-09-29T09:41:53.044Z`.
    pub fn iso(ms: i64) -> String {
        let secs = ms.div_euclid(1000);
        let (y, m, d) = civil_from_days(secs.div_euclid(86_400));
        let rem = secs.rem_euclid(86_400);
        format!(
            "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{:03}Z",
            rem / 3600,
            rem / 60 % 60,
            rem % 60,
            ms.rem_euclid(1000)
        )
    }

    pub fn typed(ts: i64, text: &str) -> Value {
        json!({"type":"user","timestamp":iso(ts),"promptSource":"typed",
               "message":{"role":"user","content":text}})
    }

    pub fn call(ts: i64, id: &str, name: &str, input: Value) -> Value {
        json!({"type":"assistant","timestamp":iso(ts),"message":{"id":format!("m-{id}"),
               "role":"assistant","stop_reason":"tool_use",
               "content":[{"type":"tool_use","id":id,"name":name,"input":input}]}})
    }

    pub fn result(ts: i64, id: &str, text: &str) -> Value {
        json!({"type":"user","timestamp":iso(ts),"message":{"role":"user",
               "content":[{"type":"tool_result","tool_use_id":id,"content":text}]}})
    }

    pub fn reply(ts: i64, text: &str) -> Value {
        json!({"type":"assistant","timestamp":iso(ts),"message":{"id":format!("m-r{ts}"),
               "role":"assistant","stop_reason":"end_turn","content":[{"type":"text","text":text}]}})
    }

    pub fn record(id: &str, status: Option<RegStatus>, status_at: Option<i64>, started: i64) -> ProcRecord {
        ProcRecord {
            pid: 4242,
            session_id: id.to_owned(),
            cwd: Some("/work".into()),
            started_at_ms: Some(started),
            entrypoint: Some("claude-vscode".into()),
            status,
            status_updated_ms: status_at,
            ..Default::default()
        }
    }

    pub fn registry(records: Vec<ProcRecord>, now: i64) -> Snapshot {
        Snapshot {
            available: true,
            by_session: records.into_iter().map(|r| (r.session_id.clone(), r)).collect(),
            spawned: HashMap::new(),
            taken_ms: now,
        }
    }

    pub fn no_registry() -> Snapshot {
        Snapshot::default()
    }

    /// A wrapper prompt line, as `.pending` holds it.
    pub fn pending_line(rid: &str, tool: &str, tool_use: &str, input: Value) -> String {
        json!({"type":"control_request","request_id":rid,"request":{"subtype":"can_use_tool",
               "tool_name":tool,"display_name":tool,"input":input,"tool_use_id":tool_use}})
        .to_string()
    }
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

    use super::testkit::*;
    use serde_json::json;

    /// The open view's header, fed line by line, with nothing but the transcript to go
    /// on (no registry, no wrapper) — what it was before there was anything else.
    #[test]
    fn an_interruption_ends_the_turn_and_a_prompt_starts_one() {
        let lab = Lab::new("turns");
        let now = now_ms();
        let reg = no_registry();
        let status = |lab: &Lab| StreamStatus::of(&lab.observe(&reg, "s-t", now));

        lab.write("s-t", &[typed(now - 9_000, "run it"), call(now - 8_000, "t1", "Bash", json!({"command":"ls"}))]);
        let st = status(&lab);
        assert_eq!((st.state, st.detail.as_str(), st.activity), ("tool", "Running Bash", "tool"));
        // The person hit stop: the CLI writes the marker, the tool never answers.
        lab.write("s-t", &[json!({"type":"user","timestamp":iso(now - 7_000),
            "message":{"role":"user","content":"[Request interrupted by user for tool use]"}})]);
        let st = status(&lab);
        assert_eq!((st.state, st.detail.as_str()), ("idle", "Interrupted"), "an interrupted turn is over");
        // The block form of the same marker, on a fresh pending call.
        lab.write("s-t", &[typed(now - 6_000, "again"), call(now - 5_500, "t2", "Bash", json!({})),
            json!({"type":"user","timestamp":iso(now - 5_000),"message":{"role":"user",
                   "content":[{"type":"text","text":"[Request interrupted by user]"}]}})]);
        assert_eq!(status(&lab).state, "idle");
        // A new prompt: working until the assistant speaks — the silence while the model
        // thinks is work, not idleness.
        lab.write("s-t", &[json!({"type":"user","timestamp":iso(now - 4_000),
            "message":{"role":"user","content":"and now do the other thing"}})]);
        let st = status(&lab);
        assert_eq!((st.state, st.activity, st.detail.as_str()), ("working", "generating", "Thinking"));
        lab.write("s-t", &[reply(now - 3_000, "done")]);
        let st = status(&lab);
        assert_eq!((st.state, st.detail.as_str()), ("idle", "Waiting for you"));
        assert_eq!(st.quiet_ms, 3_000, "quiet is the time since the turn ended");
        // A command echo changes nothing.
        lab.write("s-t", &[json!({"type":"user","timestamp":iso(now - 2_000),
            "message":{"role":"user","content":"<command-name>/effort</command-name>"}})]);
        assert_eq!(status(&lab).state, "idle");
    }

    #[test]
    fn the_queue_is_counted_from_the_transcripts_own_bookkeeping() {
        let mut f = Fold::new();
        for op in ["enqueue", "enqueue", "dequeue"] {
            f.line(&json!({"type":"queue-operation","operation":op}));
        }
        assert_eq!(f.summary().queued, 1);
        f.line(&json!({"type":"queue-operation","operation":"remove"}));
        f.line(&json!({"type":"queue-operation","operation":"remove"}));
        assert_eq!(f.summary().queued, 0, "never below zero");
        // A background task reporting in travels the same queue; nobody typed it.
        f.line(&json!({"type":"queue-operation","operation":"enqueue",
            "content":"<task-notification><task-id>b1</task-id><status>completed</status></task-notification>"}));
        assert_eq!(f.summary().queued, 0, "a task notification is not a queued message");
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
        let lab = Lab::new("title");
        let p = lab.write("s-title", &[
            json!({"type":"ai-title","aiTitle":"First guess"}),
            json!({"type":"user","message":{"role":"user","content":"hello"}}),
            json!({"type":"ai-title","aiTitle":"What it turned out to be"}),
        ]);
        assert_eq!(tracked(&p).unwrap().fold.title.as_deref(), Some("What it turned out to be"));
        // And a later refinement, read incrementally, replaces it.
        lab.write("s-title", &[json!({"type":"ai-title","aiTitle":"Its final name"})]);
        assert_eq!(tracked(&p).unwrap().fold.title.as_deref(), Some("Its final name"));
    }

    #[test]
    fn a_transcript_with_no_title_falls_back() {
        let lab = Lab::new("notitle");
        let p = lab.write("s-nt", &[json!({"type":"user","message":{"role":"user","content":"just this"}})]);
        let seen = tracked(&p).unwrap();
        assert!(seen.fold.title.is_none());
        // What was first asked stands in, as the list shows it.
        let o = lab.observe(&no_registry(), "s-nt", now_ms());
        let row = list_row(&o, "s-nt", "-work".into(), &HashMap::new());
        assert_eq!(row.title.as_deref(), Some("just this"));
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

    #[test]
    fn a_transcript_is_folded_once_then_only_what_is_appended() {
        let lab = Lab::new("track");
        let now = now_ms();
        let p = lab.write("s-tr", &[typed(now - 5_000, "go")]);
        assert!(tracked(&p).unwrap().fold.turn_open);

        // Half a line: the writer is not done with it, and it is not folded yet.
        let line = reply(now - 4_000, "done").to_string();
        let (a, b) = line.split_at(line.len() / 2);
        {
            use std::io::Write;
            let mut f = fs::OpenOptions::new().append(true).open(&p).unwrap();
            f.write_all(a.as_bytes()).unwrap();
        }
        assert!(tracked(&p).unwrap().fold.turn_open, "a partial line is carried, not guessed at");
        {
            use std::io::Write;
            let mut f = fs::OpenOptions::new().append(true).open(&p).unwrap();
            writeln!(f, "{b}").unwrap();
        }
        let seen = tracked(&p).unwrap();
        assert!(!seen.fold.turn_open, "finished by the next read");
        assert_eq!(seen.fold.last_content_ts, Some(now - 4_000));
        assert_eq!(seen.bytes, fs::metadata(&p).unwrap().len());

        // Replaced under the same name: a new file, folded from scratch.
        fs::remove_file(&p).unwrap();
        lab.write("s-tr", &[typed(now - 1_000, "a new start")]);
        let seen = tracked(&p).unwrap();
        assert!(seen.fold.turn_open);
        assert_eq!(seen.fold.last_content_ts, Some(now - 1_000));
        assert_eq!(seen.head.title.as_deref(), Some("a new start"), "and its head read again");
    }

    #[test]
    fn a_long_transcript_is_folded_from_its_last_two_mebibytes() {
        let lab = Lab::new("window");
        let now = now_ms();
        // An old unanswered call far back, then enough history to push it out of the
        // window, then a finished turn. Only the window is read, from a line boundary.
        let mut lines = vec![typed(now - 90_000, "old"), call(now - 89_000, "t-old", "Bash", json!({}))];
        let filler = "x".repeat(4_000);
        for i in 0..600 {
            lines.push(json!({"type":"attachment","attachment":{"type":"note","text":filler},"n":i}));
        }
        lines.push(typed(now - 3_000, "latest"));
        lines.push(reply(now - 2_000, "all done"));
        let p = lab.write("s-win", &lines);
        assert!(fs::metadata(&p).unwrap().len() > crate::fold::INITIAL_WINDOW);
        let seen = tracked(&p).unwrap();
        assert!(!seen.fold.turn_open);
        assert!(seen.fold.pending.is_empty(), "the old call lies outside the window");
        assert_eq!(seen.fold.last_content_ts, Some(now - 2_000));
        assert_eq!(seen.head.title.as_deref(), Some("old"), "the head is still the file's front");
    }

    /// The orchestrator's own shape (d6e4d7da): its turn ended after launching a workflow,
    /// the registry has said busy since before that, and the workflow's agents write.
    #[test]
    fn a_turn_over_with_a_workflow_running_is_background_everywhere() {
        let lab = Lab::new("bg");
        let now = now_ms();
        let started = now - 600_000;
        lab.write("s-bg", &[
            typed(now - 120_000, "run the workflow"),
            call(now - 110_000, "w1", "Workflow", json!({"description":"build it"})),
            result(now - 109_900, "w1", "Workflow launched in background. Task ID: wf1 (running)"),
            reply(now - 100_000, "It is running; I will report back."),
        ]);
        let agents = lab.roots.projects.join("-work/s-bg/subagents/workflows/wf_1");
        fs::create_dir_all(&agents).unwrap();
        fs::write(agents.join("agent-a.jsonl"), "{}\n").unwrap();
        let reg = registry(vec![record("s-bg", Some(RegStatus::Busy), Some(now - 120_050), started)], now);

        let o = lab.observe(&reg, "s-bg", now);
        assert_eq!(o.activity.kind, Kind::Background, "{:?}", o.activity);
        assert_eq!(o.activity.detail, "Background workflow running");
        assert!(o.subagent_write_ms.is_some_and(|t| now - t < 60_000));
        let row = list_row(&o, "s-bg", "-work".into(), &HashMap::new());
        assert_eq!((row.state.as_str(), row.active, row.activity, row.tone), ("working", true, "background", "busy"));
        assert!(row.last_activity_ms > now - 60_000, "its agents' writes are its activity");
        let st = StreamStatus::of(&o);
        assert_eq!((st.state, st.queued), ("working", 0));
        assert_eq!(st.background.map(|b| b.workflows), Some(1));
        assert!(stop_refusal(o.activity.kind).is_some(), "Stop could discard what it is waiting for");
    }

    #[test]
    fn a_closed_session_reads_closed_and_keeps_its_content_clock() {
        let lab = Lab::new("closed");
        let now = now_ms();
        let last = now - 5 * 86_400_000;
        let exit = now - 60_000;
        lab.write("s-cl", &[
            typed(last - 2_000, "one more thing"),
            reply(last, "done"),
            json!({"type":"last-prompt","lastPrompt":"one more thing"}),
            json!({"type":"cost-state","startTime": last - 3_600_000,"totalDuration": exit - (last - 3_600_000)}),
        ]);
        // Registry readable, and no live process holds it.
        let reg = registry(vec![], now);
        let o = lab.observe(&reg, "s-cl", now);
        let a = &o.activity;
        assert_eq!((a.kind, a.open, a.ended_how, a.mid_turn), (Kind::Closed, Some(false), Some("exit"), Some(false)));
        assert_eq!(a.ended_ms, Some(exit));
        assert!(!o.wrapper_alive, "a closed session's socket is not even asked");
        // Shelved after its last real message: the exit flush five days later is not news.
        let shelf = HashMap::from([("s-cl".to_owned(), last + 1_000)]);
        let row = list_row(&o, "s-cl", "-work".into(), &shelf);
        assert_eq!(row.last_activity_ms, last, "content, not the exit flush or the mtime");
        assert_eq!((row.state.as_str(), row.active, row.wrapped, row.archived), ("idle", false, false, true));
        assert_eq!((row.activity, row.tone, row.detail.as_str()), ("closed", "closed", "Session ended"));
        let st = StreamStatus::of(&o);
        assert_eq!((st.state, st.queued, st.ended_ms), ("idle", 0, Some(exit)));
        assert!(stop_refusal(a.kind).is_some());
    }

    #[test]
    #[cfg(unix)]
    fn wrapper_prompts_keep_the_order_they_arrived_in_and_ride_out_a_torn_read() {
        let lab = Lab::new("prompts");
        let now = now_ms();
        lab.write("s-w", &[
            typed(now - 20_000, "tidy up"),
            call(now - 19_000, "toolu_1", "Bash", json!({"command":"rm -r build"})),
            call(now - 18_000, "toolu_2", "Edit", json!({"file_path":"/w/a.rs"})),
            result(now - 17_000, "toolu_2", "edited"),
        ]);
        let _listener = std::os::unix::net::UnixListener::bind(lab.roots.wrapper.join("s-w.sock")).unwrap();
        let pending = lab.roots.wrapper.join("s-w.pending");
        let request = pending_line;
        let reg = registry(vec![record("s-w", Some(RegStatus::Busy), Some(now - 20_050), now - 60_000)], now);

        fs::write(&pending, request("zz-first", "Bash", "toolu_1", json!({"command":"rm -r build"})) + "\n").unwrap();
        let o = lab.observe(&reg, "s-w", now);
        assert_eq!(o.activity.kind, Kind::Waiting);
        assert_eq!(o.activity.detail, "Bash wants to run");
        assert!(o.wrapped());

        // An old wrapper sorts by request id, so the newer prompt comes first in the file,
        // and a prompt already settled in the transcript is still listed.
        let newer = request("aa-second", "Bash", "toolu_3", json!({"command":"make"}));
        let settled = request("mm-done", "Edit", "toolu_2", json!({"file_path":"/w/a.rs"}));
        let older = request("zz-first", "Bash", "toolu_1", json!({"command":"rm -r build"}));
        fs::write(&pending, format!("{newer}\n{settled}\n{older}\n")).unwrap();
        let o = lab.observe(&reg, "s-w", now + 1_000);
        let ids: Vec<&str> = o.prompts.iter().map(|p| p.prompt.request_id.as_str()).collect();
        assert_eq!(ids, ["zz-first", "aa-second"], "oldest first by when seen; the settled one gone");
        let on = o.activity.waiting_on.clone().unwrap();
        assert_eq!(on.request_id.as_deref(), Some("aa-second"), "the newest is the one waited on");
        assert_eq!((on.kind, on.summary.as_deref()), ("permission", Some("make")));

        // Answers: exactly the one named, or — named none — only ever a single one.
        assert_eq!(pick_prompt(&o.prompts, Some("zz-first")).unwrap().prompt.request_id, "zz-first");
        assert_eq!(pick_prompt(&o.prompts, Some("mm-done")).unwrap_err().0, StatusCode::CONFLICT);
        assert_eq!(pick_prompt(&o.prompts, None).unwrap_err().0, StatusCode::CONFLICT);
        assert_eq!(pick_prompt(&o.prompts[..1], None).unwrap().prompt.request_id, "zz-first");
        assert_eq!(pick_prompt(&[], None).unwrap_err().0, StatusCode::SERVICE_UNAVAILABLE);

        // Caught mid-rewrite: one empty read keeps what the last read had...
        fs::write(&pending, "").unwrap();
        let o = lab.observe(&reg, "s-w", now + 2_000);
        assert_eq!(o.prompts.len(), 2, "one torn read is ridden out");
        // ...and a second one is believed.
        let o = lab.observe(&reg, "s-w", now + 3_000);
        assert!(o.prompts.is_empty());
        assert_eq!(o.activity.kind, Kind::Tool, "back to the call it is running");
    }

    #[test]
    #[cfg(unix)]
    fn an_empty_read_after_a_long_gap_is_believed() {
        let lab = Lab::new("gap");
        let now = now_ms();
        lab.write("s-gap", &[typed(now - 5_000, "go"), call(now - 4_000, "toolu_g", "Bash", json!({"command":"ls"}))]);
        let _listener = std::os::unix::net::UnixListener::bind(lab.roots.wrapper.join("s-gap.sock")).unwrap();
        let pending = lab.roots.wrapper.join("s-gap.pending");
        let reg = registry(vec![record("s-gap", Some(RegStatus::Busy), Some(now - 5_050), now - 60_000)], now);
        fs::write(&pending, pending_line("r-g", "Bash", "toolu_g", json!({"command":"ls"})) + "\n").unwrap();
        assert_eq!(lab.observe(&reg, "s-gap", now).prompts.len(), 1);
        // A minute without a look, then an empty file: not a torn read, the prompt is gone.
        fs::write(&pending, "").unwrap();
        assert!(lab.observe(&reg, "s-gap", now + 60_000).prompts.is_empty());
    }

    #[test]
    #[cfg(unix)]
    fn a_prompt_left_over_in_an_idle_session_is_not_waited_on() {
        let lab = Lab::new("idleprompt");
        let now = now_ms();
        lab.write("s-ip", &[typed(now - 60_000, "hi"), reply(now - 59_000, "hello")]);
        let _listener = std::os::unix::net::UnixListener::bind(lab.roots.wrapper.join("s-ip.sock")).unwrap();
        fs::write(lab.roots.wrapper.join("s-ip.pending"), concat!(
            r#"{"type":"control_request","request_id":"r-stale","request":{"subtype":"can_use_tool","tool_name":"Bash","input":{"command":"ls"},"tool_use_id":"toolu_x"}}"#, "\n",
        )).unwrap();
        let reg = registry(vec![record("s-ip", Some(RegStatus::Idle), Some(now - 59_000), now - 120_000)], now);
        // Newly seen: it may have landed between the two reads.
        let o = lab.observe(&reg, "s-ip", now);
        assert_eq!(o.activity.kind, Kind::Waiting);
        // Still there eleven seconds on while the registry says idle: a leftover.
        let o = lab.observe(&reg, "s-ip", now + 11_000);
        assert_eq!(o.activity.kind, Kind::Idle);
        assert!(o.prompts.is_empty(), "nothing offered to answer, nothing to announce");
    }

    #[test]
    fn the_list_holds_every_open_session_and_the_newest_of_the_rest() {
        let lab = Lab::new("list");
        let now = now_ms();
        let day = 86_400_000;
        let set_mtime = |p: &Path, ms: i64| {
            let t = UNIX_EPOCH + Duration::from_millis(ms as u64);
            fs::File::options().write(true).open(p).unwrap().set_modified(t).unwrap();
        };
        // Open for nine days without a word: listed anyway, it is still waiting for you.
        let quiet = lab.write("s-open", &[typed(now - 9 * day, "hi"), reply(now - 9 * day + 1_000, "hello")]);
        set_mtime(&quiet, now - 9 * day);
        // Closed and eight days old: not listed.
        let old = lab.write("s-old", &[typed(now - 8 * day, "hi"), reply(now - 8 * day + 1_000, "hello")]);
        set_mtime(&old, now - 8 * day);
        // Sixty-two closed, one minute apart.
        for i in 1..=62 {
            let t = now - i * 60_000;
            let p = lab.write(&format!("s-{i:02}"), &[typed(t - 1_000, "q"), reply(t, "a")]);
            set_mtime(&p, t);
        }
        let reg = registry(vec![record("s-open", Some(RegStatus::Idle), Some(now - 9 * day), now - 10 * day)], now);
        let rows = discover_in(&lab.roots, &reg, &HashMap::new(), now);
        assert_eq!(rows.len(), MAX_LISTED);
        assert!(rows.iter().any(|r| r.session_id == "s-open" && r.open == Some(true)));
        assert!(!rows.iter().any(|r| r.session_id == "s-old"));
        assert!(rows.windows(2).all(|w| w[0].last_activity_ms >= w[1].last_activity_ms), "newest first");
        // The newest 59 of the closed ones, by what they said last.
        assert_eq!(rows[0].session_id, "s-01");
        assert!(rows.iter().any(|r| r.session_id == "s-59"));
        assert!(!rows.iter().any(|r| r.session_id == "s-60"));
        for r in &rows {
            assert_legacy_row(&serde_json::to_value(r).unwrap());
        }
    }

    /// A git checkout under the lab, its remotes as given: a clone, a worktree beside it, or
    /// a directory that is no checkout at all when there are none.
    fn checkout(root: &Path, name: &str, remotes: &[(&str, &str)]) -> String {
        let dir = root.join(name);
        fs::create_dir_all(&dir).unwrap();
        if !remotes.is_empty() {
            let git = |args: &[&str]| {
                let ok = std::process::Command::new("git").arg("-C").arg(&dir).args(args).output().unwrap().status.success();
                assert!(ok, "git {args:?}");
            };
            git(&["init", "-q"]);
            for (name, url) in remotes {
                git(&["remote", "add", name, url]);
            }
        }
        dir.to_string_lossy().into_owned()
    }

    /// A session's first line, as Claude Code writes it: where it runs.
    fn typed_in(ts: i64, text: &str, cwd: &str) -> Value {
        let mut line = typed(ts, text);
        line["cwd"] = json!(cwd);
        line
    }

    #[test]
    fn a_listed_session_names_its_repository() {
        let lab = Lab::new("repo");
        let now = now_ms();
        let root = lab.roots.projects.parent().unwrap().join("code");
        let clone = checkout(&root, "ui-kit-web", &[("upstream", "git@gitlab.example.com:acme/ui-kit.git"), ("origin", "https://oauth2:tok@GitLab.Example.com/acme/ui-kit-web.git")]);
        let plain = checkout(&root, "scratch", &[]);
        lab.write("s-clone", &[typed_in(now - 2_000, "hi", &clone), reply(now - 1_000, "hello")]);
        lab.write("s-plain", &[typed_in(now - 2_000, "hi", &plain), reply(now - 1_000, "hello")]);
        let rows = discover_in(&lab.roots, &registry(vec![], now), &HashMap::new(), now);
        let row = |id: &str| rows.iter().find(|r| r.session_id == id).unwrap();
        assert_eq!(row("s-clone").repository.as_deref(), Some("gitlab.example.com/acme/ui-kit-web"), "origin's, spelt one way");
        assert_eq!(row("s-plain").repository, None);
        let v = serde_json::to_value(row("s-plain")).unwrap();
        assert!(v.get("repository").is_none(), "absent rather than null, for the phones that decode strictly: {v}");
        assert_legacy_row(&serde_json::to_value(row("s-clone")).unwrap());
    }

    #[test]
    fn project_totals_are_a_repositorys() {
        let lab = Lab::new("totals");
        let now = now_ms();
        let root = lab.roots.projects.parent().unwrap().join("code");
        let url = [("origin", "https://gitlab.example.com/acme/ui-kit-web.git")];
        let clone = checkout(&root, "ui-kit-web", &url);
        let lane = checkout(&root, "ui-kit-web-lane-2", &url);
        let other = checkout(&root, "other", &[("origin", "https://gitlab.example.com/acme/other.git")]);
        let plain = checkout(&root, "scratch", &[]);
        for (id, cwd) in [("s-clone", &clone), ("s-lane", &lane), ("s-other", &other), ("s-plain", &plain)] {
            lab.write(id, &[typed_in(now - 2_000, "hi", cwd), reply(now - 1_000, "hello")]);
        }
        let reg = registry(vec![], now);
        let sessions = |key: &str| project_stats_in(&lab.roots, &reg, &HashMap::new(), key, now)["sessions"].as_i64().unwrap();
        assert_eq!(sessions("gitlab.example.com/acme/ui-kit-web"), 2, "the clone and the worktree are one project");
        assert_eq!(sessions(&clone), 0, "a directory with a repository is not its own project");
        assert_eq!(sessions(&plain), 1, "a directory with none is");
    }

    /// What an installed iPhone requires of every row, or its whole list goes blank.
    fn assert_legacy_row(v: &Value) {
        assert!(v["session_id"].is_string() && v["project"].is_string(), "{v}");
        assert!(v["last_activity_ms"].is_i64(), "an integer, always: {v}");
        for k in ["active", "archived", "wrapped"] {
            assert!(v[k].is_boolean(), "{k}: {v}");
        }
        assert!(v["bytes"].is_u64(), "{v}");
        let state = v["state"].as_str().unwrap();
        assert!(["working", "ask", "idle", "stalled", "failed"].contains(&state), "{state}");
        for k in ["cwd", "git_branch", "title"] {
            assert!(v[k].is_null() || v[k].is_string(), "{k}: {v}");
        }
        assert!(v["created_ms"].is_null() || v["created_ms"].is_i64(), "{v}");
        // New keys are left out rather than sent as null.
        for k in ["open", "since_ms", "ended_ms", "ended_how", "mid_turn", "waiting_on", "background"] {
            assert!(v.get(k).is_none_or(|x| !x.is_null()), "{k} null: {v}");
        }
    }

    #[test]
    fn every_status_speaks_the_old_vocabulary_too() {
        let lab = Lab::new("legacy");
        let now = now_ms();
        lab.write("s-lg", &[typed(now - 5_000, "go"), call(now - 4_000, "t1", "Bash", json!({"command":"make"}))]);
        for reg in [no_registry(), registry(vec![], now),
                    registry(vec![record("s-lg", Some(RegStatus::Busy), Some(now - 5_050), now - 60_000)], now),
                    registry(vec![record("s-lg", Some(RegStatus::Unknown("compacting".into())), None, now - 60_000)], now)] {
            let o = lab.observe(&reg, "s-lg", now);
            let st = serde_json::to_value(StreamStatus::of(&o)).unwrap();
            let state = st["state"].as_str().unwrap();
            assert!(["ask", "tool", "idle", "working", "failed"].contains(&state), "{state}");
            assert!(st["detail"].is_string() && st["quiet_ms"].is_i64() && st["quiet_ms"].as_i64() >= Some(0), "{st}");
            assert!(st["queued"].is_u64(), "{st}");
            assert_legacy_row(&serde_json::to_value(list_row(&o, "s-lg", "-work".into(), &HashMap::new())).unwrap());
        }
    }

    #[test]
    fn the_status_is_sent_on_a_change_and_otherwise_once_a_second() {
        let lab = Lab::new("beat");
        let now = now_ms();
        lab.write("s-b", &[typed(now - 5_000, "go")]);
        let o = lab.observe(&no_registry(), "s-b", now);
        let st = StreamStatus::of(&o);
        assert!(status_due(None, &st, Duration::ZERO), "the first one always goes");
        // The same reading a moment later: only the clock moved, so it waits.
        let later = StreamStatus { quiet_ms: st.quiet_ms + 200, as_of_ms: st.as_of_ms + 200, ..st.clone() };
        assert!(!status_due(Some(&st), &later, Duration::from_millis(200)));
        assert!(status_due(Some(&st), &later, HEARTBEAT), "a heartbeat once a second, not five");
        // A change goes at once.
        let changed = StreamStatus { queued: 1, ..st.clone() };
        assert!(status_due(Some(&st), &changed, Duration::from_millis(200)));
        let answered = StreamStatus { waiting_on: None, ..st.clone() };
        let asked = StreamStatus {
            waiting_on: Some(WaitingOn { kind: "permission", tool: None, request_id: Some("r".into()), tool_use_id: None, summary: None }),
            ..st.clone()
        };
        assert!(status_due(Some(&answered), &asked, Duration::ZERO));
    }

    #[test]
    fn a_stop_is_refused_only_where_there_is_no_turn_to_stop() {
        for kind in Kind::ALL {
            let refused = stop_refusal(kind).is_some();
            assert_eq!(refused, matches!(kind, Kind::Closed | Kind::Background), "{kind:?}");
        }
    }

    #[test]
    fn an_answer_to_a_question_never_approves_a_permission_prompt() {
        // The old phones' question form sends answers and no `allow`.
        assert!(approval_refusal(None, "Bash").is_some(), "a question's answer is not an approval");
        assert!(approval_refusal(None, "ExitPlanMode").is_some());
        assert!(approval_refusal(None, ASK_TOOL).is_none(), "and still answers a question");
        // Every client that shows a permission prompt says which it wants.
        assert!(approval_refusal(Some(true), "Bash").is_none());
        assert!(approval_refusal(Some(true), ASK_TOOL).is_none());
    }

    #[test]
    fn a_question_takes_only_answers_to_itself() {
        let input = json!({"questions":[{"question":"Which database?"},{"question":"Scope?"}]});
        let answers = |pairs: &[(&str, &str)]| -> HashMap<String, String> {
            pairs.iter().map(|(q, a)| ((*q).to_owned(), (*a).to_owned())).collect()
        };
        let refused = |pairs: &[(&str, &str)]| answers_refusal(ASK_TOOL, &input, &answers(pairs));
        // Another question's form, submitted onto this one.
        assert_eq!(refused(&[("Which framework?", "React")]), Some("That question is no longer waiting"));
        // One stray answer among right ones is still the wrong form.
        assert!(refused(&[("Which database?", "Postgres"), ("Which framework?", "React")]).is_some());
        // Nothing at all would dismiss it unanswered.
        assert!(refused(&[]).is_some());
        // Its own, in part or whole.
        assert!(refused(&[("Which database?", "Postgres")]).is_none());
        assert!(refused(&[("Which database?", "Postgres"), ("Scope?", "All of it")]).is_none());
        // A permission request carries no answers to check.
        assert!(answers_refusal("Bash", &json!({"command":"ls"}), &answers(&[])).is_none());
        // A question with no questions in it cannot be answered at all.
        assert!(answers_refusal(ASK_TOOL, &json!({}), &answers(&[("x", "y")])).is_some());
    }

    #[test]
    fn the_side_questions_fork_is_left_out_but_never_its_parent() {
        let now = now_ms();
        let mut reg = registry(vec![record("s-open", Some(RegStatus::Idle), None, now)], now);
        reg.spawned.insert("s-fork".into(), Some(now));
        reg.spawned.insert("s-open".into(), Some(now));
        reg.spawned.insert("s-parent".into(), Some(now));
        assert!(is_router_fork(&reg, "s-fork"));
        assert!(!is_router_fork(&reg, "s-open"), "a live session is never a fork");
        let asking = Asking::new("s-parent");
        assert!(!is_router_fork(&reg, "s-parent"), "the session being asked about stays");
        drop(asking);
        assert!(is_router_fork(&reg, "s-parent"));
        assert!(!is_router_fork(&reg, "s-other"));
    }

    #[test]
    fn last_activity_falls_back_to_the_start_then_the_mtime() {
        let lab = Lab::new("fallback");
        let now = now_ms();
        // Only bookkeeping in the window: no content line to go by.
        let p = lab.write("s-fb", &[json!({"type":"ai-title","aiTitle":"x"}),
                                    json!({"type":"last-prompt","lastPrompt":"y"})]);
        let o = lab.observe(&no_registry(), "s-fb", now);
        assert_eq!(o.last_activity_ms(), fs::metadata(&p).map(|m| mtime_ms(&m)).unwrap());
        let started = Observed {
            seen: Seen { head: Head { created_ms: Some(1_000), ..Head::default() }, ..o.seen.clone() },
            ..o.clone()
        };
        assert_eq!(started.last_activity_ms(), 1_000);
    }

    #[test]
    fn a_settled_call_is_found_in_what_was_appended() {
        let lab = Lab::new("settle");
        let now = now_ms();
        let p = lab.write("s-st", &[call(now - 3_000, "toolu_s", "Bash", json!({}))]);
        assert!(!tool_settled(&p, "toolu_s"));
        let line = result(now - 2_000, "toolu_s", "ok").to_string();
        let (a, b) = line.split_at(line.len() / 2);
        {
            use std::io::Write;
            let mut f = fs::OpenOptions::new().append(true).open(&p).unwrap();
            f.write_all(a.as_bytes()).unwrap();
        }
        assert!(!tool_settled(&p, "toolu_s"), "half a result is not a result");
        {
            use std::io::Write;
            let mut f = fs::OpenOptions::new().append(true).open(&p).unwrap();
            writeln!(f, "{b}").unwrap();
        }
        assert!(tool_settled(&p, "toolu_s"), "the line is read whole once it is finished");
    }

    /// A wrapper's control socket, as far as the router can tell: it answers the bare
    /// probe, records every frame written to it — what would reach the session — and
    /// acknowledges each one the way a wrapper does. Stops when dropped.
    #[cfg(unix)]
    struct FakeWrapper {
        frames: Arc<Mutex<Vec<String>>>,
        stop: Arc<std::sync::atomic::AtomicBool>,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    #[cfg(unix)]
    impl FakeWrapper {
        fn bind(path: &Path) -> FakeWrapper {
            use std::io::Write;
            let listener = std::os::unix::net::UnixListener::bind(path).unwrap();
            listener.set_nonblocking(true).unwrap();
            let frames = Arc::new(Mutex::new(Vec::new()));
            let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let (seen, done) = (frames.clone(), stop.clone());
            let thread = std::thread::spawn(move || {
                while !done.load(Ordering::Relaxed) {
                    let mut conn = match listener.accept() {
                        Ok((conn, _)) => conn,
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(5));
                            continue;
                        }
                        Err(_) => return,
                    };
                    let _ = conn.set_nonblocking(false);
                    let _ = conn.set_read_timeout(Some(Duration::from_secs(5)));
                    let mut body = Vec::new();
                    let mut buf = [0u8; 64 * 1024];
                    while let Ok(n @ 1..) = conn.read(&mut buf) {
                        body.extend_from_slice(&buf[..n]);
                        if body.contains(&b'\n') {
                            break;
                        }
                    }
                    // The router's liveness probe sends nothing and hangs up.
                    if !body.is_empty() {
                        seen.lock().unwrap().push(String::from_utf8_lossy(&body).into_owned());
                        let _ = conn.write_all(b"{\"ok\":true}\n");
                    }
                }
            });
            FakeWrapper { frames, stop, thread: Some(thread) }
        }

        fn frames(&self) -> Vec<String> {
            self.frames.lock().unwrap().clone()
        }
    }

    #[cfg(unix)]
    impl Drop for FakeWrapper {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
            if let Some(t) = self.thread.take() {
                let _ = t.join();
            }
        }
    }

    /// Q1 was answered in the editor and the model asked Q2. A phone still showing Q1's
    /// form names the prompt live now — Q2's — and sends Q1's answers.
    #[test]
    #[cfg(unix)]
    fn answers_for_a_question_that_is_not_the_one_waiting_never_reach_it() {
        let lab = Lab::new("ans");
        let now = now_ms();
        let q2 = json!({"questions":[{"question":"Which database?","options":[{"label":"Postgres"}]}]});
        lab.write("s-a", &[
            typed(now - 60_000, "set it up"),
            call(now - 50_000, "toolu_q1", ASK_TOOL, json!({"questions":[{"question":"Which framework?"}]})),
            result(now - 40_000, "toolu_q1", "User has answered your questions: \"Which framework?\"=\"React\""),
            call(now - 30_000, "toolu_q2", ASK_TOOL, q2.clone()),
        ]);
        let wrapper = FakeWrapper::bind(&lab.roots.wrapper.join("s-a.sock"));
        fs::write(lab.roots.wrapper.join("s-a.pending"), pending_line("req-Q2", ASK_TOOL, "toolu_q2", q2) + "\n").unwrap();
        let reg = registry(vec![record("s-a", Some(RegStatus::Waiting), Some(now - 30_000), now - 120_000)], now);
        let body = |answers: &[(&str, &str)]| AnswerBody {
            session: "s-a".into(),
            answers: answers.iter().map(|(q, a)| ((*q).to_owned(), (*a).to_owned())).collect(),
            allow: None,
            suggestion: None,
            message: None,
            request_id: Some("req-Q2".into()),
        };

        let stale = answer_in(&lab.roots, &reg, &body(&[("Which framework?", "React")]), now);
        assert_eq!(stale, Err((StatusCode::CONFLICT, "That question is no longer waiting".to_owned())));
        // Nothing at all would dismiss the question unanswered.
        assert_eq!(answer_in(&lab.roots, &reg, &body(&[]), now).unwrap_err().0, StatusCode::CONFLICT);
        assert!(wrapper.frames().is_empty(), "nothing reached the session: {:?}", wrapper.frames());

        // Its own answers go through, beside the questions they answer.
        let done = answer_in(&lab.roots, &reg, &body(&[("Which database?", "Postgres")]), now);
        assert_eq!(done, Ok("req-Q2".to_owned()));
        let frames = wrapper.frames();
        assert_eq!(frames.len(), 1);
        let sent: Value = serde_json::from_str(frames[0].trim()).unwrap();
        assert_eq!(sent["response"]["request_id"], "req-Q2");
        assert_eq!(sent["response"]["response"]["updatedInput"]["answers"], json!({"Which database?":"Postgres"}));
    }

    /// A frame the wrapper would cut short and write into the CLI half-finished — which the
    /// CLI exits on — is refused as too large, whichever path it would have gone by.
    #[test]
    #[cfg(unix)]
    fn what_the_wrapper_would_cut_short_is_refused_as_too_large() {
        let lab = Lab::new("toolong");
        let now = now_ms();
        let ask = json!({"questions":[{"question":"Paste the log?"}]});
        lab.write("s-l", &[typed(now - 20_000, "look at this"), call(now - 10_000, "toolu_l", ASK_TOOL, ask.clone())]);
        let wrapper = FakeWrapper::bind(&lab.roots.wrapper.join("s-l.sock"));
        fs::write(lab.roots.wrapper.join("s-l.pending"), pending_line("req-L", ASK_TOOL, "toolu_l", ask) + "\n").unwrap();
        let reg = registry(vec![record("s-l", Some(RegStatus::Waiting), Some(now - 10_000), now - 60_000)], now);
        let log = "x".repeat(1_100_000);

        // The send box, as `send` delivers a message.
        let e = crate::peer::send_via_wrapper_in(&lab.roots.wrapper, "s-l", &log).unwrap_err();
        let (code, why) = delivery_failure(e);
        assert_eq!(code, StatusCode::PAYLOAD_TOO_LARGE);
        // Both sizes in one unit, so the refused size can never read as the limit itself.
        assert!(why.starts_with("That is too long to send: 1075 KiB"), "{why}");
        assert!(why.contains("at most 1024 KiB"), "{why}");
        // An answer, as `answer` delivers it.
        let body = AnswerBody {
            session: "s-l".into(),
            answers: HashMap::from([("Paste the log?".to_owned(), log.clone())]),
            allow: None,
            suggestion: None,
            message: None,
            request_id: Some("req-L".into()),
        };
        assert_eq!(answer_in(&lab.roots, &reg, &body, now).unwrap_err().0, StatusCode::PAYLOAD_TOO_LARGE);
        assert!(wrapper.frames().is_empty(), "nothing reached the session");
        // Anything else is the session out of reach, as before.
        assert_eq!(delivery_failure(anyhow::anyhow!("Broken pipe (os error 32)")).0, StatusCode::SERVICE_UNAVAILABLE);
    }

    /// Busy after the turn is over, with nothing in the background to show for it, reads
    /// by when the registry went busy: before the end (a Stop hook, or a task the
    /// transcript cannot see) is not stopped; after it (a /compact) is the main loop.
    #[test]
    fn busy_after_the_turn_is_stoppable_only_when_it_began_after_the_end() {
        let lab = Lab::new("afterturn");
        let now = now_ms();
        let started = now - 600_000;
        lab.write("s-h", &[typed(now - 30_000, "go"), reply(now - 10_000, "Done.")]);
        lab.write("s-c", &[typed(now - 90_000, "go"), reply(now - 60_000, "Done.")]);
        let reg = registry(vec![
            // Busy stamped with the prompt and held through a Stop hook.
            record("s-h", Some(RegStatus::Busy), Some(now - 30_050), started),
            // Busy stamped by a /compact typed a minute after the reply.
            record("s-c", Some(RegStatus::Busy), Some(now - 6_000), started),
        ], now);

        let o = lab.observe(&reg, "s-h", now);
        assert_eq!((o.activity.kind, o.activity.detail.as_str(), o.activity.background), (Kind::Background, "Working", None));
        assert!(stop_refusal(o.activity.kind).is_some(), "it may be background work");
        let row = list_row(&o, "s-h", "-work".into(), &HashMap::new());
        assert_eq!((row.state.as_str(), row.active, row.activity, row.since_ms), ("working", true, "background", Some(now - 10_000)));

        let o = lab.observe(&reg, "s-c", now);
        assert_eq!((o.activity.kind, o.activity.detail.as_str()), (Kind::Generating, "Working"));
        assert_eq!(o.activity.since_ms, Some(now - 6_000), "since the compaction began, not the reply before");
        assert!(o.activity.stoppable() && stop_refusal(o.activity.kind).is_none());
        assert_eq!(StreamStatus::of(&o).state, "working");
    }

    /// a2ae08ce 2022–2024: a thinking-only `end_turn`, then 3 minutes of the CLI carrying
    /// the turn on by itself before its next line. Nothing at the time can tell that from a
    /// hook or unseen background work holding busy past the end, so it reads the same —
    /// working, no counts claimed, no Stop — until the next line reopens the turn.
    #[test]
    fn a_turn_the_cli_carries_on_by_itself_reads_working_until_its_next_line() {
        let lab = Lab::new("carry");
        let now = now_ms();
        lab.write("s-k", &[
            typed(now - 200_000, "go on"),
            json!({"type":"assistant","timestamp":iso(now - 186_000),"message":{"id":"m-think",
                   "role":"assistant","stop_reason":"end_turn","content":[{"type":"thinking","thinking":""}]}}),
        ]);
        let reg = registry(vec![record("s-k", Some(RegStatus::Busy), Some(now - 200_050), now - 600_000)], now);
        let o = lab.observe(&reg, "s-k", now);
        assert_eq!((o.activity.kind, o.activity.detail.as_str(), o.activity.background), (Kind::Background, "Working", None));
        assert!(stop_refusal(o.activity.kind).is_some());
        lab.write("s-k", &[call(now - 1_000, "toolu_k", "Bash", json!({"command":"make"}))]);
        let o = lab.observe(&reg, "s-k", now);
        assert_eq!((o.activity.kind, o.activity.detail.as_str()), (Kind::Tool, "Running Bash"));
        assert!(stop_refusal(o.activity.kind).is_none());
    }

    /// A router with a Claude account and, unless `claude_only`, a Kimi Code one.
    async fn router_with(tag: &str, claude_only: bool) -> crate::api_refusals::Lab {
        let lab = crate::api_refusals::Lab::local(tag);
        let mut adds = vec![serde_json::json!({ "label": format!("{tag} work"), "source": "token", "token": "sk-ant-oat01-not-a-credential" })];
        if !claude_only {
            adds.push(serde_json::json!({ "label": format!("{tag} kimi"), "source": "token", "provider": "kimi", "token": "sk-kimi-not-a-credential" }));
        }
        for add in adds {
            assert_eq!(lab.post("/api/accounts", &add.to_string()).await.0, StatusCode::OK);
        }
        lab
    }

    async fn options_of(lab: &crate::api_refusals::Lab) -> Value {
        let res = options(axum::extract::State(lab.state.clone())).await;
        serde_json::from_slice(&axum::body::to_bytes(res.into_body(), 1 << 20).await.unwrap()).unwrap()
    }

    #[tokio::test]
    async fn live_options_offer_each_providers_models() {
        let lab = router_with("options-providers", false).await;
        let got = options_of(&lab).await;
        let groups = got["by_provider"].as_array().unwrap();
        assert_eq!(groups.len(), 2, "{got}");
        assert_eq!((&groups[0]["provider"], &groups[0]["name"]), (&serde_json::json!("anthropic"), &serde_json::json!("Anthropic")));
        let claude: Vec<&str> = groups[0]["models"].as_array().unwrap().iter().map(|m| m["id"].as_str().unwrap()).collect();
        assert_eq!(&claude[..4], ["opus", "opus[1m]", "sonnet", "sonnet[1m]"], "Claude's aliases, as the CLI's picker has them");
        assert_eq!((&groups[1]["provider"], &groups[1]["name"]), (&serde_json::json!("kimi"), &serde_json::json!("Kimi Code")));
        assert_eq!(groups[1]["models"][0], serde_json::json!({ "id": "k3", "display_name": "Kimi K3" }));
        // Every client that reads the flat list can move a session to a Kimi Code model too.
        let flat: Vec<&str> = got["models"].as_array().unwrap().iter().map(|m| m.as_str().unwrap()).collect();
        assert!(flat.contains(&"opus") && flat.contains(&"k3") && flat.contains(&"kimi-for-coding"), "{flat:?}");

        // With no Kimi Code account there is nothing to offer under it.
        let lab = router_with("options-claude-only", true).await;
        let got = options_of(&lab).await;
        assert_eq!(got["by_provider"].as_array().unwrap().len(), 1, "{got}");
        assert!(!got["models"].as_array().unwrap().contains(&serde_json::json!("k3")));
    }

    #[tokio::test]
    async fn set_model_refuses_a_model_no_source_serves() {
        let lab = router_with("set-model-unserved", true).await;
        let session = "6f708192-0000-4000-8000-0000000000aa";
        let ask = |model: &str| serde_json::json!({ "session": session, "model": model }).to_string();
        // No session by that id is under a wrapper here, so anything that got past the router's
        // own check would be answered "only a session under the wrapper", 503.
        for (model, code) in [("gpt-9", StatusCode::CONFLICT), ("unknown-model", StatusCode::BAD_REQUEST), ("k3", StatusCode::CONFLICT)] {
            let (status, body) = lab.post("/api/live/model", &ask(model)).await;
            assert_eq!(status, code, "{model}: {body}");
            assert_eq!(body["reason"], format!("no source serves the model \"{model}\""), "{model}");
        }
        // Claude's aliases and ids are still taken, and reach the session check.
        for model in ["opus", "sonnet[1m]", "claude-opus-5-5"] {
            assert_eq!(lab.post("/api/live/model", &ask(model)).await.0, StatusCode::SERVICE_UNAVAILABLE, "{model}");
        }

        // A hosted router refuses it before the request goes down any machine's tunnel.
        let hosted = crate::api_refusals::Lab::hosted("set-model-unserved-hosted").await;
        let (status, body) = hosted.post("/api/live/model", &ask("k3")).await;
        assert_eq!((status, body["reason"].as_str()), (StatusCode::CONFLICT, Some(r#"no source serves the model "k3""#)), "{body}");
    }
}
