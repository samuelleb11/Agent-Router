//! Read-only projection of Codex's append-only rollout files. Session identity is
//! namespaced at the router boundary because Codex and Claude both use bare UUIDs.

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

use serde_json::Value;

use crate::activity::{Activity, Kind};
use crate::live::{Item, LiveSession, Stats};

pub const CLIENT: &str = "codex";
const LISTED_WITHIN_MS: i64 = 7 * 24 * 60 * 60 * 1000;
const MAX_LISTED: usize = 60;

pub fn router_id(native_id: &str) -> String { format!("codex:{native_id}") }

pub fn native_id(id: &str) -> Option<&str> {
    let id = id.strip_prefix("codex:")?;
    (!id.is_empty() && id.len() <= 128 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')).then_some(id)
}

/// Rollouts are normally under sessions/YYYY/MM/DD. Walk only ordinary directories;
/// a symlink into another tree never becomes a transcript source.
pub fn files_in(home: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
        if depth > 5 { return; }
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let Ok(kind) = entry.file_type() else { continue };
            if kind.is_dir() { walk(&entry.path(), depth + 1, out); }
            else if kind.is_file() && entry.file_name().to_string_lossy().starts_with("rollout-")
                && entry.path().extension().and_then(|s| s.to_str()) == Some("jsonl")
            { out.push(entry.path()); }
        }
    }
    let mut out = Vec::new();
    walk(&home.join("sessions"), 0, &mut out);
    walk(&home.join("archived_sessions"), 0, &mut out);
    out
}

fn mtime(meta: &std::fs::Metadata) -> i64 {
    meta.modified().ok().and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok()).map_or(0, |d| d.as_millis() as i64)
}

pub fn discover(archived: &HashMap<String, i64>, now: i64) -> Vec<LiveSession> {
    let mut rows=crate::accounts::codex_home().ok().map(|home| discover_in(&home, archived, now)).unwrap_or_default();
    if let Some(host)=super::runtime::host() {
        for id in host.known() {
            if !rows.iter().any(|r|r.session_id==id) {
                if let Some(session)=find(&id) {rows.push(session.row(archived,now));}
            }
        }
    }
    rows
}

pub fn discover_in(home: &Path, archived: &HashMap<String, i64>, now: i64) -> Vec<LiveSession> {
    let mut files: Vec<_> = files_in(home).into_iter().filter_map(|path| {
        let meta = std::fs::metadata(&path).ok()?;
        (now.saturating_sub(mtime(&meta)) <= LISTED_WITHIN_MS).then_some((mtime(&meta), path))
    }).collect();
    files.sort_by_key(|(at, _)| std::cmp::Reverse(*at));
    files.into_iter().take(MAX_LISTED).filter_map(|(_, path)| cached(&path))
        .filter(|session| !session.subagent)
        .map(|session| session.row(archived, now)).collect()
}

pub fn find(id: &str) -> Option<Arc<Session>> {
    let native = native_id(id)?;
    let home = crate::accounts::codex_home().ok()?;
    files_in(&home).into_iter().filter(|path| {
        path.file_stem().and_then(|s| s.to_str()).is_some_and(|name| name.ends_with(native))
    }).find_map(|path| cached(&path).filter(|session| session.id == id)).or_else(|| {
        let (thread,_)=super::runtime::host()?.snapshot(id)?;
        Some(Arc::new(Session {id:id.into(),cwd:thread["cwd"].as_str().map(str::to_owned),title:thread["preview"].as_str().filter(|s|!s.is_empty()).map(str::to_owned),created_ms:thread["createdAt"].as_i64().map(|n|n*1000),last_ms:thread["updatedAt"].as_i64().map(|n|n*1000),..Default::default()}))
    })
}

#[derive(Clone, Default)]
pub struct Session {
    pub id: String,
    pub cwd: Option<String>,
    pub title: Option<String>,
    pub git_branch: Option<String>,
    pub created_ms: Option<i64>,
    pub last_ms: Option<i64>,
    pub subagent: bool,
    bytes: u64,
    items: Vec<Item>,
    stats: Stats,
    turn_open: bool,
    turn_started: Option<i64>,
    failed: bool,
    running: Vec<(String, String, i64)>,
    names: HashMap<String, String>,
    last_usage_total: i64,
    /// Events and response items can both carry the same completed assistant message.
    last_said: Option<(String, String, String, i64)>,
}

struct Tracked {
    identity: (u64, u64),
    length: u64,
    modified: Option<SystemTime>,
    offset: u64,
    carry: Vec<u8>,
    session: Arc<Session>,
    used_ms: i64,
}

fn identity(meta: &std::fs::Metadata) -> (u64, u64) {
    #[cfg(unix)]
    { use std::os::unix::fs::MetadataExt; (meta.dev(), meta.ino()) }
    #[cfg(not(unix))]
    { (0, meta.created().ok().and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok()).map_or(0, |d| d.as_nanos() as u64)) }
}

fn cached(path: &Path) -> Option<Arc<Session>> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, Tracked>>> = OnceLock::new();
    let mut file = std::fs::File::open(path).ok()?;
    let meta = file.metadata().ok()?;
    let now = crate::db::now_ms();
    let mut all = CACHE.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
    if all.len() > 256 { all.retain(|_, read| now - read.used_ms < 30 * 60 * 1000); }
    let read = all.entry(path.to_path_buf()).or_insert_with(|| Tracked {
        identity: identity(&meta), length: 0, modified: None, offset: 0, carry: Vec::new(), session: Arc::new(Session::default()), used_ms: now,
    });
    read.used_ms = now;
    if identity(&meta) != read.identity || meta.len() < read.offset || (meta.len() == read.length && meta.modified().ok() != read.modified) {
        read.offset = 0; read.carry.clear(); read.session = Arc::new(Session::default());
    }
    read.identity = identity(&meta);
    file.seek(SeekFrom::Start(read.offset)).ok()?;
    let mut fresh = Vec::new();
    file.take(meta.len().saturating_sub(read.offset)).read_to_end(&mut fresh).ok()?;
    if !fresh.is_empty() {
        read.offset += fresh.len() as u64;
        read.carry.extend_from_slice(&fresh);
        let used = read.carry.iter().rposition(|b| *b == b'\n').map_or(0, |pos| pos + 1);
        let session = Arc::make_mut(&mut read.session);
        for line in read.carry[..used].split(|b| *b == b'\n') {
            if let Ok(v) = serde_json::from_slice::<Value>(line) { session.absorb(&v); }
        }
        read.carry.drain(..used);
        // A partial record should be tiny. Bound an interrupted or malformed giant line.
        if read.carry.len() > 8 * 1024 * 1024 { read.carry.clear(); }
    }
    read.length = meta.len(); read.modified = meta.modified().ok();
    Arc::make_mut(&mut read.session).bytes = meta.len();
    (!read.session.id.is_empty()).then(|| read.session.clone())
}

/// A complete read with no global environment or memo, also used by fixture tests.
#[cfg(test)]
fn read(path: &Path) -> Option<Session> {
    let raw = std::fs::read(path).ok()?;
    let mut session = Session::default();
    for line in raw.split(|b| *b == b'\n') {
        if let Ok(v) = serde_json::from_slice::<Value>(line) { session.absorb(&v); }
    }
    (!session.id.is_empty()).then_some(session)
}

fn string(v: &Value, key: &str) -> Option<String> { v.get(key).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_owned) }

fn content(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts.iter().filter_map(|p| p.get("text").and_then(Value::as_str)).collect::<Vec<_>>().join("\n"),
        Value::Null => String::new(),
        _ => v.to_string(),
    }
}

impl Session {
    fn item(&mut self, time: &Option<String>, at: i64, role: &str, kind: &str, name: Option<String>, text: Option<String>) {
        // Codex emits a completed message as both an event and a response item. Adjacent
        // duplicates belong to one message; a repeated reply on a later turn remains.
        if let Some(text) = &text {
            if self.last_said.as_ref().is_some_and(|(was_role, was_kind, was_text, was_at)| was_role == role && was_kind == kind && was_text == text && at >= *was_at && at - *was_at <= 1000) {
                return;
            }
            self.last_said = Some((role.to_owned(), kind.to_owned(), text.clone(), at));
        } else { self.last_said = None; }
        let dur_ms = self.last_ms.filter(|last| at >= *last).map(|last| at - last);
        self.items.push(Item { t: time.clone(), role: role.into(), kind: kind.into(), name,
            text: text.map(|s| s.chars().take(6000).collect()), ask: None, dur_ms, sidechain: false });
        self.last_ms = Some(at);
    }

    fn absorb(&mut self, v: &Value) {
        let time = string(v, "timestamp");
        let at = crate::fold::ts_ms(v).unwrap_or_else(|| self.last_ms.or(self.created_ms).unwrap_or(0));
        let p = v.get("payload").unwrap_or(&Value::Null);
        match v.get("type").and_then(Value::as_str) {
            Some("session_meta") => {
                if let Some(id) = string(p, "id") { self.id = router_id(&id); }
                self.cwd = string(p, "cwd");
                self.git_branch = p.get("git").or_else(|| p.get("git_info")).and_then(|git| string(git, "branch"));
                self.created_ms = Some(at);
                self.subagent = p.get("parent_thread_id").and_then(Value::as_str).is_some()
                    || p.get("source").is_some_and(|s| s.to_string().contains("subagent"));
            }
            Some("turn_context") => {
                self.stats.model = string(p, "model").or_else(|| self.stats.model.take());
                self.stats.effort = string(p, "effort").or_else(|| string(p, "reasoning_effort")).or_else(|| self.stats.effort.take());
                self.stats.mode = string(p, "approval_policy").or_else(|| self.stats.mode.take());
                if self.cwd.is_none() { self.cwd = string(p, "cwd"); }
            }
            Some("event_msg") => match p.get("type").and_then(Value::as_str) {
                Some("task_started" | "turn_started") => {
                    self.turn_open = true; self.turn_started = Some(at); self.failed = false;
                    self.last_ms = Some(at); self.last_said = None;
                }
                Some("user_message") => {
                    let text = string(p, "message").unwrap_or_default();
                    if self.title.is_none() { self.title = Some(text.chars().take(100).collect()); }
                    self.turn_open = true; self.turn_started = Some(at); self.failed = false;
                    self.last_said = None;
                    self.item(&time, at, "user", "text", None, Some(text));
                }
                Some("agent_message") => self.item(&time, at, "assistant", "text", None, string(p, "message")),
                Some("agent_reasoning" | "agent_reasoning_raw_content") => self.item(&time, at, "assistant", "thinking", None, string(p, "text")),
                Some("task_complete" | "task_completed" | "turn_completed") => {
                    self.turn_open = false; self.running.clear(); self.last_ms = Some(at);
                }
                Some("turn_aborted") => {
                    self.item(&time, at, "user", "interrupt", None, None);
                    self.turn_open = false; self.running.clear();
                }
                Some("error") => {
                    self.failed = true;
                    self.item(&time, at, "system", "other", Some("Error".into()), string(p, "message"));
                }
                Some("token_count") => {
                    if let Some(u) = p.get("info").and_then(|info| info.get("total_token_usage")) {
                        let n = |key: &str| u.get(key).and_then(Value::as_i64).unwrap_or(0).max(0);
                        let input = n("input_tokens");
                        self.stats.cache_read = n("cached_input_tokens").min(input);
                        self.stats.cache_write = n("cache_write_input_tokens").min(input - self.stats.cache_read);
                        self.stats.cache_write_5m = self.stats.cache_write;
                        self.stats.input = input - self.stats.cache_read - self.stats.cache_write;
                        self.stats.output = n("output_tokens");
                        self.stats.total = input + self.stats.output;
                        if self.stats.total > self.last_usage_total { self.stats.turns += 1; }
                        self.last_usage_total = self.stats.total;
                    }
                }
                _ => {}
            },
            Some("response_item") => match p.get("type").and_then(Value::as_str) {
                Some("message") if p.get("role").and_then(Value::as_str) == Some("assistant") => {
                    self.item(&time, at, "assistant", "text", None, p.get("content").map(content));
                }
                Some("reasoning") => {
                    if let Some(summary) = p.get("summary").map(content).filter(|s| !s.is_empty()) {
                        self.item(&time, at, "assistant", "thinking", None, Some(summary));
                    }
                }
                Some("function_call" | "custom_tool_call" | "local_shell_call") => {
                    let name = string(p, "name").unwrap_or_else(|| "shell".into());
                    let id = string(p, "call_id").or_else(|| string(p, "id"));
                    let args = p.get("arguments").or_else(|| p.get("input")).or_else(|| p.get("action")).map(content);
                    self.item(&time, at, "assistant", "tool_use", Some(name.clone()), args);
                    if let Some(id) = id { self.names.insert(id.clone(), name.clone()); self.running.push((id, name, at)); }
                    self.turn_open = true;
                }
                Some("function_call_output" | "custom_tool_call_output") => {
                    let id = string(p, "call_id").unwrap_or_default();
                    self.item(&time, at, "user", "tool_result", self.names.get(&id).cloned(), p.get("output").map(content));
                    self.running.retain(|(call, _, _)| call != &id);
                }
                _ => {}
            },
            _ => {}
        }
    }

    pub fn feed(&self) -> &[Item] { &self.items }
    pub fn stats(&self) -> Stats {
        let mut stats=self.stats.clone();
        if let Some((thread,_))=super::runtime::host().and_then(|h|h.snapshot(&self.id)) {
            stats.model=thread["model"].as_str().map(str::to_owned).or(stats.model);
            stats.effort=thread["reasoningEffort"].as_str().map(str::to_owned).or(stats.effort);
        }
        stats
    }

    /// A rollout records turn progress, never whether its client process is alive.
    /// Keep open unknown, and mark a silent unfinished turn stalled rather than claiming
    /// that an old approval or an abandoned turn is still running.
    pub fn activity(&self, now: i64) -> Activity {
        let mut a = Activity {
            kind: Kind::Idle, open: None, since_ms: self.last_ms, detail: "Waiting for you".into(),
            phase: None, tool: None, waiting_on: None, background: None,
            ended_ms: None, ended_how: None, mid_turn: Some(self.turn_open), source: "transcript",
        };
        if self.failed {
            a.kind = Kind::Failed; a.detail = "The last turn failed".into();
        } else if self.turn_open && self.last_ms.is_some_and(|at| now - at > crate::activity::STALLED_AFTER_MS) {
            a.kind = Kind::Stalled; a.detail = "No recent progress".into();
        } else if self.turn_open {
            if let Some((_, tool, since)) = self.running.last() {
                a.kind = Kind::Tool; a.tool = Some(tool.clone()); a.since_ms = Some(*since); a.detail = format!("Running {tool}");
            } else {
                a.kind = Kind::Generating; a.phase = Some("thinking"); a.since_ms = self.turn_started.or(self.last_ms); a.detail = "Thinking".into();
            }
        }
        super::runtime::host().map_or(a.clone(),|host|host.activity(&self.id,a))
    }

    pub fn row(&self, archived: &HashMap<String, i64>, now: i64) -> LiveSession {
        let a = self.activity(now);
        let last = self.last_ms.or(self.created_ms).unwrap_or(0);
        let active = a.active();
        let repository = self.cwd.as_deref().and_then(|cwd| crate::repository::key_of(&crate::repository::remotes_cached(cwd)));
        LiveSession {
            session_id: self.id.clone(), cwd: self.cwd.clone(),
            project: self.cwd.as_deref().unwrap_or_default().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect(),
            git_branch: self.git_branch.clone(), repository, title: self.title.clone(), created_ms: self.created_ms,
            last_activity_ms: last, active, state: a.legacy_list_state().into(),
            archived: crate::live::is_shelved(archived, &self.id, last) && !active,
            wrapped: super::runtime::host().is_some_and(|h|h.hosts(&self.id)) && a.open==Some(true), bytes: self.bytes, activity: a.kind.as_str(), open: a.open,
            detail: a.detail.clone(), since_ms: a.since_ms, ended_ms: None, ended_how: None,
            mid_turn: a.mid_turn, waiting_on: None, background: None, tone: a.tone(), source: a.source,
            client: Some(CLIENT), hosted: Some(super::runtime::host().is_some_and(|h|h.hosts(&self.id))), as_of_ms: now,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_codex_rollout_has_shared_feed_and_normalized_cumulative_usage() {
        let root = std::env::temp_dir().join(format!("ar-codex-session-{}", std::process::id()));
        let path = root.join("sessions/2026/10/10/rollout-test-thread-1.jsonl");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let record = |kind: &str, payload: Value| json!({ "timestamp": "2026-10-10T12:00:00.000Z", "type": kind, "payload": payload });
        let records = [
            record("session_meta", json!({ "id": "thread-1", "cwd": "/work/project", "git": { "branch": "main" } })),
            record("turn_context", json!({ "model": "gpt-test", "effort": "high", "approval_policy": "on-request" })),
            record("event_msg", json!({ "type": "user_message", "message": "Find the file" })),
            record("response_item", json!({ "type": "function_call", "name": "shell", "call_id": "call1", "arguments": "{\"cmd\":\"ls\"}" })),
            record("response_item", json!({ "type": "function_call_output", "call_id": "call1", "output": "a.txt" })),
            record("event_msg", json!({ "type": "agent_message", "message": "Found it" })),
            record("response_item", json!({ "type": "message", "role": "assistant", "content": [{ "type": "output_text", "text": "Found it" }] })),
            record("event_msg", json!({ "type": "token_count", "info": { "total_token_usage": { "input_tokens": 100, "cached_input_tokens": 60, "output_tokens": 20, "reasoning_output_tokens": 10 } } })),
            record("event_msg", json!({ "type": "token_count", "info": { "total_token_usage": { "input_tokens": 100, "cached_input_tokens": 60, "output_tokens": 20 } } })),
            record("event_msg", json!({ "type": "task_complete" })),
        ];
        let fixture = records.iter().map(Value::to_string).collect::<Vec<_>>().join("\n") + "\n";
        std::fs::write(&path, &fixture).unwrap();
        let session = read(&path).unwrap();
        assert_eq!(session.id, "codex:thread-1");
        assert_eq!(session.feed().len(), 4, "duplicate native message representations counted once");
        assert_eq!(session.feed()[2].name.as_deref(), Some("shell"));
        let st = session.stats();
        assert_eq!((st.input, st.cache_read, st.output, st.total, st.turns), (40, 60, 20, 120, 1));
        assert_eq!(st.model.as_deref(), Some("gpt-test"));
        assert_eq!(session.activity(1791633600000).kind, Kind::Idle);
        assert_eq!(session.activity(1791633600000).open, None);
        assert_eq!(native_id("codex:thread-1"), Some("thread-1"));
        assert_eq!(native_id("codex:../thread"), None);

        // A partly appended record is offered only once it is complete.
        let before = cached(&path).unwrap().feed().len();
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
        let next = record("event_msg", json!({ "type": "user_message", "message": "Another turn" })).to_string();
        file.write_all(&next.as_bytes()[..20]).unwrap();
        assert_eq!(cached(&path).unwrap().feed().len(), before);
        file.write_all(&next.as_bytes()[20..]).unwrap(); file.write_all(b"\n").unwrap();
        assert_eq!(cached(&path).unwrap().feed().len(), before + 1);
        std::fs::remove_dir_all(root).unwrap();
    }
}
