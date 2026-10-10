//! Kimi Code's sessions read from its files (AGEN-0110): a row of the live list in the shape
//! Claude Code's have (`live::LiveSession`), the feed of what was said and done
//! (`live::Item`), and its usage (`live::Stats`).
//!
//! Each session is a directory, `sessions/<workspace>/<session>/` under Kimi Code's home,
//! with a `state.json` (id, directory, title, `lastTurnReason`) and an append-only
//! `agents/main/wire.jsonl` (wire protocol 1.5), its subagents' beside it in
//! `agents/agent-N/`. Every record carries its `time` in epoch ms. The records read here,
//! measured on Kimi Code 2.1.1 (fixtures/kimi/session/):
//! - `turn.prompt`: what was asked; `turn.ended`: the turn's end and its `reason`;
//! - `context.append_loop_event`: `tool.call`, `tool.result`, `content.part` (text,
//!   thinking);
//! - `interaction.request` and `interaction.resolved`: an approval asked and answered;
//! - `llm.request`, `usage.record`: the model and the tokens of each call.
//!
//! The files cannot say whether a process holds the session: a turn in progress, an approval
//! waiting and a killed process all leave a `turn.prompt` with no `turn.ended`. Kimi Code's
//! hooks say (`registry`), and decide when they have; without them the files are read alone.

use std::collections::HashMap;
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use serde_json::Value;

use super::registry::{self, Record};
use crate::activity::{Activity, Kind, WaitingOn};
use crate::live::{Item, LiveSession, Stats};

/// The client a Kimi Code session's row names.
pub const CLIENT: &str = "kimi-code";

/// Every session directory under Kimi Code's home `home`.
pub fn sessions_in(home: &Path) -> Vec<PathBuf> {
    let Ok(workspaces) = std::fs::read_dir(home.join("sessions")) else { return Vec::new() };
    workspaces
        .flatten()
        .filter(|w| w.file_name().to_string_lossy().starts_with("wd_"))
        .filter_map(|w| std::fs::read_dir(w.path()).ok())
        .flat_map(|sessions| sessions.flatten().map(|s| s.path()))
        .filter(|dir| dir.join("state.json").is_file())
        .collect()
}

/// How long a session nobody holds stays listed, as Claude Code's do.
const LISTED_WITHIN_MS: i64 = 7 * 24 * 60 * 60 * 1000;

/// Every Kimi Code session on this machine worth showing, as rows of the live list: each one
/// its hooks say is live, and every other written to in the last week. A session is read
/// again only when its wire has grown or its state changed.
pub fn discover(archived: &HashMap<String, i64>, now: i64) -> Vec<LiveSession> {
    let Some(home) = super::home() else { return Vec::new() };
    let hooked: HashMap<String, Record> =
        registry::dir().map(|d| registry::read(&d, now)).unwrap_or_default().into_iter().map(|r| (r.session.clone(), r)).collect();
    sessions_in(&home)
        .into_iter()
        .filter_map(|dir| {
            let session = cached(&dir)?;
            let hook = hooked.get(&session.id);
            let live = hook.is_some_and(|r| r.status != registry::Status::Ended);
            let last = session.fold.last_ms.or(session.created_ms).unwrap_or(0);
            (live || now - last <= LISTED_WITHIN_MS).then(|| session.row(hook, archived, now))
        })
        .collect()
}

/// The session `id` on this machine, if Kimi Code has it: its directory is named by its id.
pub fn find(id: &str) -> Option<Arc<Session>> {
    let home = super::home()?;
    sessions_in(&home).into_iter().find(|dir| dir.file_name().is_some_and(|n| n == id)).and_then(|dir| cached(&dir))
}

/// What a session's files were when it was last read: the wire's length and time, and the
/// state's time.
type Seen = (u64, Option<SystemTime>, Option<SystemTime>);

static READ: Mutex<Option<HashMap<PathBuf, (Seen, Arc<Session>)>>> = Mutex::new(None);

fn cached(dir: &Path) -> Option<Arc<Session>> {
    let wire = std::fs::metadata(dir.join("agents/main/wire.jsonl")).ok();
    let state = std::fs::metadata(dir.join("state.json")).ok();
    let seen: Seen =
        (wire.as_ref().map_or(0, |m| m.len()), wire.and_then(|m| m.modified().ok()), state.and_then(|m| m.modified().ok()));
    let mut memo = READ.lock().unwrap_or_else(|p| p.into_inner());
    let memo = memo.get_or_insert_with(HashMap::new);
    if let Some((was, session)) = memo.get(dir) {
        if *was == seen {
            return Some(session.clone());
        }
    }
    let session = Arc::new(read(dir)?);
    memo.insert(dir.to_path_buf(), (seen, session.clone()));
    Some(session)
}

/// One session, as its files say.
pub struct Session {
    pub id: String,
    pub cwd: Option<String>,
    pub title: Option<String>,
    pub created_ms: Option<i64>,
    /// How its last turn ended: completed, cancelled, failed.
    pub last_turn_reason: Option<String>,
    fold: Fold,
}

/// What a pass over the main agent's wire says.
#[derive(Default)]
struct Fold {
    items: Vec<Item>,
    /// The newest record of content: a prompt, a step, an answer, a turn's end.
    last_ms: Option<i64>,
    turn_open: bool,
    turn_started_ms: Option<i64>,
    /// Approvals asked and not yet answered, by id: (tool, what it would do, its call).
    pending: Vec<(String, Pending)>,
    /// Tool calls without their result yet, by call id: (tool, since).
    running: Vec<(String, String, i64)>,
    stats: Stats,
}

#[derive(Clone)]
struct Pending {
    tool: Option<String>,
    action: Option<String>,
    call: Option<String>,
    since_ms: i64,
}

/// The session in directory `dir`, if it has a `state.json`.
pub fn read(dir: &Path) -> Option<Session> {
    let state: Value = serde_json::from_slice(&std::fs::read(dir.join("state.json")).ok()?).ok()?;
    let text = |k: &str| state.get(k).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_owned);
    Some(Session {
        id: text("id")?,
        cwd: text("cwd"),
        title: text("title"),
        created_ms: state.get("createdAt").and_then(Value::as_i64),
        last_turn_reason: text("lastTurnReason"),
        fold: fold(&dir.join("agents/main/wire.jsonl")),
    })
}

fn fold(wire: &Path) -> Fold {
    let mut f = Fold::default();
    let Ok(file) = std::fs::File::open(wire) else { return f };
    let mut names: HashMap<String, String> = HashMap::new();
    let mut previous: Option<i64> = None;
    for line in std::io::BufReader::new(file).lines().map_while(Result::ok) {
        let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
        let Some(t) = v.get("time").and_then(Value::as_i64) else { continue };
        let ty = v.get("type").and_then(Value::as_str).unwrap_or_default();
        let s = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).map(str::to_owned);
        let mut item = |role: &str, kind: &str, name: Option<String>, text: Option<String>| {
            let dur_ms = previous.filter(|p| t >= *p).map(|p| t - p);
            f.items.push(Item { t: Some(iso_ms(t)), role: role.into(), kind: kind.into(), name, text, ask: None, dur_ms, sidechain: false });
        };
        match ty {
            "turn.prompt" => {
                let said: Vec<&str> = v
                    .get("input")
                    .and_then(Value::as_array)
                    .map(|parts| parts.iter().filter_map(|p| p.get("text").and_then(Value::as_str)).collect())
                    .unwrap_or_default();
                item("user", "text", None, Some(said.join("\n")));
                f.turn_open = true;
                f.turn_started_ms = Some(t);
            }
            "turn.ended" => {
                if s(&v, "reason").as_deref() == Some("cancelled") {
                    item("user", "interrupt", None, None);
                }
                f.turn_open = false;
                f.pending.clear();
                f.running.clear();
            }
            "interaction.request" => {
                let request = v.get("request").unwrap_or(&Value::Null);
                let pending = Pending {
                    tool: s(request, "toolName"),
                    action: s(request, "action"),
                    call: s(&v, "toolCallId").or_else(|| s(request, "toolCallId")),
                    since_ms: t,
                };
                if let Some(id) = s(&v, "id") {
                    f.pending.push((id, pending));
                }
            }
            "interaction.resolved" => {
                let id = s(&v, "id");
                f.pending.retain(|(p, _)| Some(p) != id.as_ref());
            }
            "context.append_loop_event" => {
                let e = v.get("event").unwrap_or(&Value::Null);
                match e.get("type").and_then(Value::as_str).unwrap_or_default() {
                    "tool.call" => {
                        let name = s(e, "name").unwrap_or_default();
                        let args = e.get("args").unwrap_or(&Value::Null);
                        let shown = args.get("command").and_then(Value::as_str).map(str::to_owned).unwrap_or_else(|| args.to_string());
                        item("assistant", "tool_use", Some(name.clone()), Some(shown));
                        if let Some(call) = s(e, "toolCallId") {
                            names.insert(call.clone(), name.clone());
                            f.running.push((call, name, t));
                        }
                    }
                    "tool.result" => {
                        let call = s(e, "toolCallId");
                        let result = e.get("result").unwrap_or(&Value::Null);
                        let output = result.get("output").map(|o| o.as_str().map(str::to_owned).unwrap_or_else(|| o.to_string()));
                        item("user", "tool_result", call.as_ref().and_then(|c| names.get(c).cloned()), output);
                        f.running.retain(|(c, _, _)| Some(c) != call.as_ref());
                    }
                    "content.part" => {
                        let part = e.get("part").unwrap_or(&Value::Null);
                        match part.get("type").and_then(Value::as_str) {
                            Some("text") => item("assistant", "text", None, s(part, "text")),
                            Some("thinking") => item("assistant", "thinking", None, s(part, "thinking").or_else(|| s(part, "text"))),
                            _ => {}
                        }
                    }
                    _ => {}
                }
            }
            "llm.request" => {
                f.stats.model = s(&v, "model");
                continue;
            }
            "profile.bind" => {
                f.stats.effort = s(&v, "thinkingEffort");
                continue;
            }
            "permission.set_mode" => {
                f.stats.mode = s(&v, "mode");
                continue;
            }
            "usage.record" => {
                let u = v.get("usage").unwrap_or(&Value::Null);
                let get = |k: &str| u.get(k).and_then(Value::as_i64).unwrap_or(0);
                f.stats.input += get("inputOther");
                f.stats.output += get("output");
                f.stats.cache_read += get("inputCacheRead");
                f.stats.cache_write += get("inputCacheCreation");
                f.stats.cache_write_5m += get("inputCacheCreation");
                f.stats.turns += 1;
                continue;
            }
            _ => continue,
        }
        f.last_ms = Some(t);
        previous = Some(t);
    }
    let st = &mut f.stats;
    st.total = st.input + st.output + st.cache_read + st.cache_write;
    f
}

/// Epoch ms as the live feed carries a time: `2026-10-10T17:43:18.831Z`.
fn iso_ms(ms: i64) -> String {
    let secs = crate::quota::iso(ms.div_euclid(1000));
    format!("{}.{:03}Z", secs.trim_end_matches('Z'), ms.rem_euclid(1000))
}

impl Session {
    /// What it said and did, in order.
    #[cfg_attr(not(test), expect(dead_code, reason = "the live stream serves it, AGEN-0112"))]
    pub fn feed(&self) -> &[Item] {
        &self.fold.items
    }

    /// Its tokens, as its files count them; not yet priced (`live::price`).
    pub fn stats(&self) -> Stats {
        self.fold.stats.clone()
    }

    /// What it is doing now: what its hooks last said (`hooked`), when they have said
    /// anything, else what its files show.
    pub fn activity(&self, hooked: Option<&Record>, now: i64) -> Activity {
        let f = &self.fold;
        let source = if hooked.is_some() { "registry" } else { "transcript" };
        let hooked = hooked.map(|r| r.status);
        let mut a = Activity {
            kind: Kind::Idle,
            open: hooked.map(|s| s != registry::Status::Ended),
            since_ms: f.last_ms,
            detail: "Waiting for you".into(),
            phase: None,
            tool: None,
            waiting_on: None,
            background: None,
            ended_ms: None,
            ended_how: None,
            mid_turn: Some(f.turn_open),
            source,
        };
        let ended = match hooked {
            Some(registry::Status::Ended) => true,
            // Nothing heard from the hooks: a turn left open and long quiet is a process
            // that went away.
            None => f.turn_open && f.last_ms.is_none_or(|t| now - t > registry::SILENT_MS),
            Some(_) => false,
        };
        let waiting = f.pending.last().filter(|_| f.turn_open && !ended && hooked != Some(registry::Status::Idle));
        if ended {
            a.kind = Kind::Closed;
            a.open = Some(false);
            a.detail = "Session ended".into();
            a.ended_ms = f.last_ms;
            a.ended_how = Some("unknown");
        } else if let Some((id, p)) = waiting {
            a.kind = Kind::Waiting;
            a.since_ms = Some(p.since_ms);
            a.detail = match &p.tool {
                Some(tool) => format!("{tool} wants to run"),
                None => "Waiting for permission".into(),
            };
            a.tool = p.tool.clone();
            a.waiting_on = Some(WaitingOn {
                kind: "permission",
                tool: p.tool.clone(),
                request_id: Some(id.clone()),
                tool_use_id: p.call.clone(),
                summary: p.action.clone(),
            });
        } else if f.turn_open || hooked == Some(registry::Status::Busy) {
            match f.running.last() {
                Some((_, tool, since)) => {
                    a.kind = Kind::Tool;
                    a.since_ms = Some(*since);
                    a.detail = format!("Running {tool}");
                    a.tool = Some(tool.clone());
                }
                None => {
                    a.kind = Kind::Generating;
                    a.since_ms = f.turn_started_ms.or(f.last_ms);
                    a.detail = "Thinking".into();
                    a.phase = Some("thinking");
                }
            }
        } else if self.last_turn_reason.as_deref() == Some("failed") {
            a.kind = Kind::Failed;
            a.detail = "The last turn failed".into();
        }
        a
    }

    /// Its row of the live list.
    pub fn row(&self, hooked: Option<&Record>, archived: &HashMap<String, i64>, now: i64) -> LiveSession {
        let a = self.activity(hooked, now);
        let last = self.fold.last_ms.or(self.created_ms).unwrap_or(0);
        let active = a.active();
        let repository = self.cwd.as_deref().and_then(|cwd| crate::repository::key_of(&crate::repository::remotes_cached(cwd)));
        LiveSession {
            session_id: self.id.clone(),
            cwd: self.cwd.clone(),
            // Named as Claude Code names a project's folder: its directory, every other
            // character a dash.
            project: self
                .cwd
                .as_deref()
                .unwrap_or_default()
                .chars()
                .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
                .collect(),
            git_branch: None,
            repository,
            title: self.title.clone(),
            created_ms: self.created_ms,
            last_activity_ms: last,
            active,
            state: a.legacy_list_state().to_owned(),
            archived: archived.get(&self.id).is_some_and(|at| *at >= last) && !active,
            wrapped: false,
            bytes: 0,
            activity: a.kind.as_str(),
            open: a.open,
            detail: a.detail.clone(),
            since_ms: a.since_ms,
            ended_ms: a.ended_ms,
            ended_how: a.ended_how,
            mid_turn: a.mid_turn,
            waiting_on: a.waiting_on.clone(),
            background: None,
            tone: a.tone(),
            source: a.source,
            client: Some(CLIENT),
            as_of_ms: now,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DONE_AT: i64 = 1_791_654_198_855;

    /// A Kimi Code home holding the recorded session, its wire as `wire`.
    struct Home(PathBuf);

    impl Home {
        fn new(name: &str, wire: &str, state: &str) -> Self {
            let root = std::env::temp_dir().join(format!("ar-kimi-session-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            let dir = root.join("sessions/wd_demo_b034f63fc2a8/session_fd31bc41-dc2f-432b-8be3-2ae7f81240a6");
            std::fs::create_dir_all(dir.join("agents/main")).unwrap();
            let fixture = |f: &str| std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/kimi/session").join(f)).unwrap();
            std::fs::write(dir.join("state.json"), fixture(state)).unwrap();
            std::fs::write(dir.join("agents/main/wire.jsonl"), fixture(wire)).unwrap();
            Self(root)
        }
        fn session(&self) -> Session {
            let dirs = sessions_in(&self.0);
            assert_eq!(dirs.len(), 1, "{dirs:?}");
            read(&dirs[0]).unwrap()
        }
    }

    impl Drop for Home {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn hooked(status: registry::Status, at: i64) -> Record {
        Record {
            session: "session_fd31bc41-dc2f-432b-8be3-2ae7f81240a6".into(),
            cwd: None,
            client: Some("kimi_code_cli".into()),
            status,
            started_ms: at,
            heard_ms: at,
        }
    }

    #[test]
    fn a_kimi_session_on_disk_is_a_live_row_with_its_messages_and_tool_calls() {
        let home = Home::new("row", "wire.jsonl", "state.json");
        let s = home.session();
        assert_eq!(s.id, "session_fd31bc41-dc2f-432b-8be3-2ae7f81240a6");

        let said: Vec<(&str, &str, Option<&str>, Option<&str>)> =
            s.feed().iter().map(|i| (i.role.as_str(), i.kind.as_str(), i.name.as_deref(), i.text.as_deref())).collect();
        assert_eq!(
            said,
            [
                ("user", "text", None, Some("Run echo, please.")),
                ("assistant", "tool_use", Some("Bash"), Some("echo hello-from-standin")),
                ("user", "tool_result", Some("Bash"), Some("hello-from-standin\n")),
                ("assistant", "text", None, Some("The command said hello.")),
            ]
        );
        assert_eq!(s.feed()[0].t.as_deref(), Some("2026-10-10T17:43:10.213Z"));

        // Its hooks said it is open and the turn is over.
        let row = s.row(Some(&hooked(registry::Status::Idle, DONE_AT)), &HashMap::new(), DONE_AT + 1_000);
        assert_eq!(row.session_id, s.id);
        assert_eq!(row.cwd.as_deref(), Some("/Users/u/code/demo"));
        assert_eq!(row.project, "-Users-u-code-demo");
        assert_eq!(row.title.as_deref(), Some("Run echo, please."));
        assert_eq!(row.client, Some(CLIENT));
        assert_eq!((row.activity, row.state.as_str(), row.open, row.active), ("idle", "idle", Some(true), false));
        assert_eq!(row.last_activity_ms, DONE_AT);
        assert_eq!(row.source, "registry");

        // The same row, as installed clients read it: the legacy keys, with their types.
        let json = serde_json::to_value(&row).unwrap();
        for key in ["session_id", "project", "state", "detail", "activity", "tone"] {
            assert!(json[key].is_string(), "{key}: {json}");
        }
        assert_eq!(json["client"], "kimi-code");

        // Ended by its hooks; and, with no hooks at all, a finished turn is idle.
        let ended = s.row(Some(&hooked(registry::Status::Ended, DONE_AT)), &HashMap::new(), DONE_AT + 1_000);
        assert_eq!((ended.activity, ended.open), ("closed", Some(false)));
        assert_eq!(s.row(None, &HashMap::new(), DONE_AT + 1_000).activity, "idle");

        // Its usage, priced as K3.
        let st = s.stats();
        assert_eq!(st.model.as_deref(), Some("k3"));
        assert_eq!((st.input, st.output, st.cache_read, st.turns), (2_000, 40, 1_000, 2));
    }

    #[test]
    fn a_kimi_session_waiting_on_an_approval_reads_as_waiting() {
        let home = Home::new("waiting", "wire-waiting.jsonl", "state-waiting.json");
        let s = home.session();
        let asked_at = 1_791_654_190_304;

        for hooks in [None, Some(hooked(registry::Status::Waiting, asked_at)), Some(hooked(registry::Status::Busy, asked_at))] {
            let row = s.row(hooks.as_ref(), &HashMap::new(), asked_at + 5_000);
            assert_eq!((row.activity, row.state.as_str(), row.tone, row.active), ("waiting", "ask", "attention", true), "{hooks:?}");
            assert_eq!(row.detail, "Bash wants to run");
            let on = row.waiting_on.unwrap();
            assert_eq!(on.kind, "permission");
            assert_eq!(on.tool.as_deref(), Some("Bash"));
            assert_eq!(on.summary.as_deref(), Some("Running: echo hello-from-standin"));
            assert_eq!(on.request_id.as_deref(), Some("approval_349187f6-af98-4286-a79c-94eeff2d1320"));
            assert_eq!(on.tool_use_id.as_deref(), Some("toolu_standin_7640a4c48c"));
        }

        // Its process went away with the approval open: no longer waiting on anyone.
        let gone = s.row(Some(&hooked(registry::Status::Ended, asked_at)), &HashMap::new(), asked_at + 5_000);
        assert_eq!(gone.activity, "closed");
        let silent = s.row(None, &HashMap::new(), asked_at + registry::SILENT_MS + 1);
        assert_eq!(silent.activity, "closed", "no hooks, and nothing written for three minutes");
    }
}
