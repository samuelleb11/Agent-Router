//! Telling a phone that a session is waiting on a person.
//!
//! Two places a prompt shows up. A session under the wrapper writes every unanswered
//! `can_use_tool` request — a Bash approval, an edit, an AskUserQuestion — to
//! `/tmp/agent-router-sessions/<id>.pending`, the moment it appears. A session with no
//! wrapper reveals only an AskUserQuestion, and only through its transcript. Both are
//! watched once a second; each prompt is announced once, keyed by its request id.
//!
//! Nothing here can unblock the session: that is the phone's job, and what the app
//! opens to when the notification is tapped. What *does* happen here is the other half
//! of that: a prompt answered anywhere — the terminal, the editor, another phone —
//! leaves the waiting set, and the banner it raised is taken down again.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rusqlite::{params, Connection, OpenFlags};
use serde_json::{json, Value};

use crate::apns::{Apns, Delivery, Key, SendError};

const PENDING_DIR: &str = "/tmp/agent-router-sessions";
const POLL: Duration = Duration::from_secs(1);
/// Unwrapped sessions are only worth reading if they moved recently.
const RECENT_MS: i64 = 10 * 60_000;
/// Prompt ids already announced; bounded, oldest forgotten first.
const REMEMBER: usize = 2000;
pub const DEFAULT_TOPIC: &str = "ca.whnet.agent-router";
/// What the app said about itself when it registered: its team, which completes a
/// dropped-in key file, and its bundle id, which is the push topic.
const TEAM_KEY: &str = "push_team_id";
const BUNDLE_KEY: &str = "push_bundle_id";

#[derive(serde::Serialize, Clone)]
pub struct Device {
    pub token: String,
    pub name: Option<String>,
    pub environment: String,
    pub router_ref: Option<String>,
    pub registered_at: i64,
    pub last_seen_at: i64,
    pub last_result: Option<String>,
}

/// One unanswered request, as the wrapper recorded it.
#[derive(Debug, Clone, PartialEq)]
pub struct Prompt {
    pub request_id: String,
    pub tool: String,
    pub display: String,
    pub input: Value,
}

pub fn parse_pending(text: &str) -> Vec<Prompt> {
    text.lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(|v| v["request"]["subtype"] == "can_use_tool")
        .filter_map(|v| {
            let request_id = v["request_id"].as_str()?.to_owned();
            let tool = v["request"]["tool_name"].as_str().unwrap_or("tool").to_owned();
            let display = v["request"]["display_name"].as_str().unwrap_or(&tool).to_owned();
            Some(Prompt { request_id, tool, display, input: v["request"]["input"].clone() })
        })
        .collect()
}

fn clip(s: &str, n: usize) -> String {
    if s.chars().count() <= n { s.to_owned() } else { format!("{}…", s.chars().take(n).collect::<String>()) }
}

/// What the notification says under the session's name: what is being asked.
pub fn describe(p: &Prompt) -> (String, String) {
    let s = |k: &str| p.input.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
    match p.tool.as_str() {
        "AskUserQuestion" => {
            let q = p.input["questions"]
                .as_array()
                .and_then(|qs| qs.first())
                .and_then(|q| q.get("question"))
                .and_then(Value::as_str)
                .unwrap_or("A question is waiting for you");
            let n = p.input["questions"].as_array().map(Vec::len).unwrap_or(1);
            (if n > 1 { format!("{n} questions") } else { "Question".into() }, clip(q, 140))
        }
        "Bash" => ("Bash wants to run".into(), clip(&s("command"), 140)),
        "Edit" | "Write" | "MultiEdit" | "NotebookEdit" => {
            let file = s("file_path").or_else(|| s("notebook_path"));
            let name = file.rsplit('/').next().unwrap_or("").to_owned();
            (format!("{} wants to change", p.display), if name.is_empty() { "a file".into() } else { name })
        }
        "WebFetch" => ("WebFetch wants to fetch".into(), clip(&s("url"), 140)),
        _ => (format!("{} needs permission", p.display), String::new()),
    }
}

trait OrElse {
    fn or_else(self, f: impl FnOnce() -> String) -> String;
}
impl OrElse for String {
    fn or_else(self, f: impl FnOnce() -> String) -> String {
        if self.is_empty() { f() } else { self }
    }
}

pub struct Push {
    apns: Apns,
    db_path: PathBuf,
    seen: Mutex<(HashSet<String>, VecDeque<String>)>,
    pub last_error: Mutex<Option<String>>,
    pub last_sent: Mutex<Option<(i64, String)>>,
    /// The key file dropped beside the database: what was found, and what it waits on.
    pub key_file: Mutex<Option<String>>,
    /// The (file name, mtime) whose import failed outright, so it is not retried every second.
    tried: Mutex<Option<(String, std::time::SystemTime)>>,
    /// Prompts announced to a phone that are still waiting, as key -> session: what a
    /// banner may be up for, and so what a dismissal is owed for.
    announced: Mutex<HashMap<String, String>>,
}

impl Push {
    pub fn new(db_path: &Path) -> Self {
        Self {
            apns: Apns::new(),
            db_path: db_path.to_path_buf(),
            seen: Mutex::new((HashSet::new(), VecDeque::new())),
            last_error: Mutex::new(None),
            last_sent: Mutex::new(None),
            key_file: Mutex::new(None),
            tried: Mutex::new(None),
            announced: Mutex::new(HashMap::new()),
        }
    }

    pub fn key(&self) -> Option<Key> {
        crate::accounts::push_key().map(|k| Key { team_id: k.team_id, key_id: k.key_id, p8: k.p8 })
    }

    pub fn topic(&self) -> String {
        crate::accounts::push_key()
            .and_then(|k| k.bundle_id)
            .filter(|b| !b.trim().is_empty())
            .or_else(|| crate::db::setting(&self.db_path, BUNDLE_KEY))
            .unwrap_or_else(|| DEFAULT_TOPIC.into())
    }

    pub fn configured(&self) -> bool {
        self.key().is_some()
    }

    /// Keep what the app says about itself at registration.
    pub fn remember_app(&self, team_id: Option<&str>, bundle_id: Option<&str>) {
        for (key, value) in [(TEAM_KEY, team_id), (BUNDLE_KEY, bundle_id)] {
            if let Some(v) = value.map(str::trim).filter(|v| !v.is_empty()) {
                if let Err(e) = crate::db::set_setting(&self.db_path, key, v) {
                    tracing::warn!(%key, "could not keep the app's {key}: {e}");
                }
            }
        }
    }

    fn note_key_file(&self, note: String) -> Option<String> {
        *self.key_file.lock().unwrap() = Some(note.clone());
        Some(note)
    }

    /// The key as the file Apple hands out, `AuthKey_<key id>.p8`, dropped beside the
    /// database. Taken in once the team is known — the app says which when it registers
    /// — so nothing is typed. The file stays where it is, made private: Apple never
    /// hands the same key out twice. Returns what was done, or what is still waited on.
    pub fn import_key_file(&self) -> Option<String> {
        if self.configured() {
            return None;
        }
        let dir = self.db_path.parent()?;
        let (path, key_id) = key_file_in(dir)?;
        let name = path.file_name()?.to_string_lossy().into_owned();
        let mtime = std::fs::metadata(&path).and_then(|m| m.modified()).ok()?;
        if self.tried.lock().unwrap().as_ref() == Some(&(name.clone(), mtime)) {
            return self.key_file.lock().unwrap().clone();
        }
        let Some(team_id) = crate::db::setting(&self.db_path, TEAM_KEY) else {
            return self.note_key_file(format!(
                "{name} found · waiting for the phone to register once, which tells the router the team id"
            ));
        };
        let failed = |why: String| {
            *self.tried.lock().unwrap() = Some((name.clone(), mtime));
            tracing::warn!(file = %name, "APNs key file not taken: {why}");
            self.note_key_file(format!("{name}: {why}"))
        };
        let p8 = match std::fs::read_to_string(&path) {
            Ok(s) => s.trim().to_string(),
            Err(e) => return failed(e.to_string()),
        };
        let key = Key { team_id: team_id.clone(), key_id: key_id.clone(), p8: p8.clone() };
        if let Err(e) = crate::apns::mint_jwt(&key, crate::db::now_ms() / 1000) {
            return failed(format!("does not sign: {e}"));
        }
        let bundle_id = crate::db::setting(&self.db_path, BUNDLE_KEY);
        if let Err(e) = crate::accounts::store_push_key(Some(crate::accounts::PushKey { team_id, key_id, p8, bundle_id })) {
            return failed(format!("could not store: {e}"));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
        self.apns_forget();
        tracing::info!(file = %path.display(), "APNs key imported; phones can be told");
        self.note_key_file(format!("imported from {name}"))
    }

    fn open(&self) -> rusqlite::Result<Connection> {
        Connection::open(&self.db_path)
    }

    pub fn devices(&self) -> Vec<Device> {
        let Ok(conn) = Connection::open_with_flags(&self.db_path, OpenFlags::SQLITE_OPEN_READ_ONLY) else {
            return Vec::new();
        };
        let Ok(mut stmt) = conn.prepare(
            "SELECT token, name, environment, router_ref, registered_at, last_seen_at, last_result
             FROM push_devices ORDER BY registered_at",
        ) else {
            return Vec::new();
        };
        stmt.query_map([], |r| {
            Ok(Device {
                token: r.get(0)?,
                name: r.get(1)?,
                environment: r.get(2)?,
                router_ref: r.get(3)?,
                registered_at: r.get(4)?,
                last_seen_at: r.get(5)?,
                last_result: r.get(6)?,
            })
        })
        .map(|rows| rows.filter_map(Result::ok).collect())
        .unwrap_or_default()
    }

    pub fn register(&self, token: &str, name: Option<&str>, environment: &str, router_ref: Option<&str>) -> anyhow::Result<()> {
        let now = crate::db::now_ms();
        self.open()?.execute(
            "INSERT INTO push_devices (token, name, environment, router_ref, registered_at, last_seen_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?5)
             ON CONFLICT(token) DO UPDATE SET name = ?2, environment = ?3, router_ref = ?4, last_seen_at = ?5",
            params![token, name, environment, router_ref, now],
        )?;
        Ok(())
    }

    pub fn unregister(&self, token: &str) -> anyhow::Result<()> {
        self.open()?.execute("DELETE FROM push_devices WHERE token = ?1", [token])?;
        Ok(())
    }

    fn record(&self, token: &str, result: &str) {
        if let Ok(conn) = self.open() {
            let _ = conn.execute(
                "UPDATE push_devices SET last_result = ?2, last_seen_at = ?3 WHERE token = ?1",
                params![token, result, crate::db::now_ms()],
            );
        }
    }

    /// Send one message to every registered phone, and keep what Apple said about each.
    /// The payload is built per device, because each is told which router this came from.
    /// Returns one line per device.
    async fn fan_out(
        &self,
        delivery: Delivery<'_>,
        payload: impl Fn(&Device) -> Value,
    ) -> Vec<(String, Result<(), String>)> {
        let Some(key) = self.key() else {
            return Vec::new();
        };
        let topic = self.topic();
        let mut out = Vec::new();
        for d in self.devices() {
            let body = payload(&d);
            let tail = d.token.chars().rev().take(6).collect::<String>().chars().rev().collect::<String>();
            match self.apns.send(&key, &d.token, &d.environment, &topic, delivery, &body).await {
                Ok(()) => {
                    self.record(&d.token, "delivered");
                    *self.last_error.lock().unwrap() = None;
                    out.push((tail, Ok(())));
                }
                Err(SendError::Unregistered) => {
                    tracing::info!(token = %tail, "phone unregistered by Apple; forgetting it");
                    let _ = self.unregister(&d.token);
                    out.push((tail, Err("no longer registered".into())));
                }
                Err(e) => {
                    let why = e.to_string();
                    tracing::warn!(token = %tail, error = %why, "push failed");
                    self.record(&d.token, &why);
                    *self.last_error.lock().unwrap() = Some(why.clone());
                    out.push((tail, Err(why)));
                }
            }
        }
        out
    }

    /// Tell every registered phone. Returns one line per device.
    pub async fn notify(&self, title: &str, subtitle: &str, body: &str, session: Option<&str>, collapse: &str) -> Vec<(String, Result<(), String>)> {
        let out = self
            .fan_out(Delivery::Alert(collapse), |d| {
                let mut alert = json!({ "title": title });
                if !subtitle.is_empty() { alert["subtitle"] = json!(subtitle); }
                if !body.is_empty() { alert["body"] = json!(body); }
                json!({
                    "aps": { "alert": alert, "sound": "default", "thread-id": session.unwrap_or("router"), "category": "PROMPT" },
                    "session": session,
                    "router_ref": d.router_ref,
                    "request_id": collapse,
                })
            })
            .await;
        if out.iter().any(|(_, r)| r.is_ok()) {
            *self.last_sent.lock().unwrap() = Some((crate::db::now_ms(), format!("{title}: {body}")));
        }
        out
    }

    /// These prompts are answered; the banners they raised can come down. Nothing is
    /// shown — the phone is woken to take them out of its notification centre.
    ///
    /// Apple may drop a message like this one, and does whenever the app has been force
    /// quit. That is survivable and not worked around here: the app asks
    /// `/api/push/pending` what is still waiting whenever it comes to the front, and
    /// clears up anything this missed.
    pub async fn dismiss(&self, answered: &[String]) -> Vec<(String, Result<(), String>)> {
        if answered.is_empty() {
            return Vec::new();
        }
        self.fan_out(Delivery::Background, |d| {
            json!({
                "aps": { "content-available": 1 },
                "dismiss": answered,
                "router_ref": d.router_ref,
            })
        })
        .await
    }

    fn seen(&self, id: &str) -> bool {
        self.seen.lock().unwrap().0.contains(id)
    }

    fn mark(&self, id: &str) {
        let mut g = self.seen.lock().unwrap();
        if g.0.insert(id.to_owned()) {
            g.1.push_back(id.to_owned());
            if g.1.len() > REMEMBER {
                if let Some(old) = g.1.pop_front() { g.0.remove(&old); }
            }
        }
    }

    /// The key of everything waiting for a person right now — what a phone checks its
    /// own notification centre against.
    ///
    /// Everything `waiting()` finds, plus anything already announced that is still
    /// unanswered. The second half matters: a session that has sat on a question for
    /// longer than `RECENT_MS` is no longer read, and a phone told only the first half
    /// would take down a banner for a question nobody has answered.
    pub fn waiting_keys(&self) -> Vec<String> {
        let mut keys: HashSet<String> = self.waiting().into_iter().map(|(key, ..)| key).collect();
        for (key, session) in self.announced.lock().unwrap().iter() {
            if !keys.contains(key) && still_waiting(key, session) {
                keys.insert(key.clone());
            }
        }
        let mut out: Vec<String> = keys.into_iter().collect();
        out.sort();
        out
    }

    /// Everything currently waiting, as (dedupe key, session id, headline, detail).
    fn waiting(&self) -> Vec<(String, String, String, String)> {
        let mut out = Vec::new();
        if let Ok(dir) = std::fs::read_dir(PENDING_DIR) {
            for e in dir.flatten() {
                let path = e.path();
                let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
                let Some(session) = name.strip_suffix(".pending") else { continue };
                let Ok(text) = std::fs::read_to_string(&path) else { continue };
                for p in parse_pending(&text) {
                    let (head, detail) = describe(&p);
                    out.push((format!("req:{}", p.request_id), session.to_owned(), head, detail));
                }
            }
        }
        for (session, path) in crate::live::recent_transcripts(RECENT_MS) {
            if crate::peer::wrapper_alive(&session) {
                continue; // its prompts arrive through the file above
            }
            if let Some((id, question)) = crate::live::pending_ask(&path) {
                out.push((format!("ask:{id}"), session, "Question".into(), clip(&question, 140)));
            }
        }
        out
    }

    /// Runs for the life of the router. What is already waiting at startup is not
    /// announced — a restart is not news.
    pub async fn watch(self: Arc<Self>) {
        for (key, ..) in self.waiting() {
            self.mark(&key);
        }
        self.import_key_file();
        loop {
            tokio::time::sleep(POLL).await;
            if !self.configured() {
                self.import_key_file();
                continue;
            }
            let waiting = self.waiting();
            let still: HashSet<&str> = waiting.iter().map(|(k, ..)| k.as_str()).collect();
            // Gone from the sweep is not the same as answered — see `still_waiting`.
            let answered: Vec<String> = {
                let mut announced = self.announced.lock().unwrap();
                let gone: Vec<String> = announced
                    .iter()
                    .filter(|(key, session)| !still.contains(key.as_str()) && !still_waiting(key, session))
                    .map(|(key, _)| key.clone())
                    .collect();
                for key in &gone {
                    announced.remove(key);
                }
                gone
            };
            if !answered.is_empty() {
                tracing::info!(prompts = answered.len(), "answered elsewhere; taking the banners down");
                self.dismiss(&answered).await;
            }
            let fresh: Vec<_> = waiting.into_iter().filter(|(k, ..)| !self.seen(k)).collect();
            for (key, session, head, detail) in fresh {
                self.mark(&key);
                self.announced.lock().unwrap().insert(key.clone(), session.clone());
                let (project, title) = crate::live::session_label(&session).unwrap_or_else(|| ("session".into(), None));
                let name = match title {
                    Some(t) => format!("{project} · {}", clip(&t, 40)),
                    None => project,
                };
                tracing::info!(session = %session, %head, "prompt waiting; telling the phone");
                self.notify(&name, &head, &detail, Some(&session), &key).await;
            }
        }
    }
}

/// Whether a prompt that has left the sweep is nonetheless still waiting for someone.
///
/// A wrapper's `.pending` file is authoritative and never filtered by time, so a `req:`
/// key that is gone really is answered — or its wrapper died, and nothing can answer it
/// either way. An `ask:` key is read off a transcript that is only looked at while the
/// session is moving, and a session waiting on a question does not move; that one has to
/// be asked about directly.
fn still_waiting(key: &str, session: &str) -> bool {
    match key.strip_prefix("ask:") {
        Some(ask_id) => crate::live::still_asking(session, ask_id),
        None => false,
    }
}

/// The key id Apple puts in the file name: `AuthKey_<ten alphanumerics>.p8`.
pub fn key_id_of(name: &str) -> Option<String> {
    let id = name.strip_prefix("AuthKey_")?.strip_suffix(".p8")?;
    (id.len() == 10 && id.chars().all(|c| c.is_ascii_alphanumeric())).then(|| id.to_string())
}

/// The newest key file in a directory, with its key id.
fn key_file_in(dir: &Path) -> Option<(PathBuf, String)> {
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            let id = key_id_of(&path.file_name()?.to_string_lossy())?;
            let modified = e.metadata().and_then(|m| m.modified()).ok()?;
            Some((modified, path, id))
        })
        .max_by_key(|(modified, ..)| *modified)
        .map(|(_, path, id)| (path, id))
}

pub mod api {
    use std::sync::Arc;

    use axum::extract::State;
    use axum::Json;
    use serde::Deserialize;
    use serde_json::{json, Value};

    use crate::proxy::AppState;

    #[derive(Deserialize)]
    pub struct ConfigBody {
        #[serde(default)]
        pub team_id: String,
        #[serde(default)]
        pub key_id: String,
        #[serde(default)]
        pub p8: String,
        #[serde(default)]
        pub bundle_id: Option<String>,
        #[serde(default)]
        pub clear: bool,
    }

    #[derive(Deserialize)]
    pub struct RegisterBody {
        pub token: String,
        #[serde(default)]
        pub name: Option<String>,
        #[serde(default)]
        pub environment: Option<String>,
        #[serde(default)]
        pub router_ref: Option<String>,
        /// The team the app is signed under, and its bundle id.
        #[serde(default)]
        pub team_id: Option<String>,
        #[serde(default)]
        pub bundle_id: Option<String>,
    }

    #[derive(Deserialize)]
    pub struct TokenBody {
        pub token: String,
    }

    pub async fn status(State(state): State<Arc<AppState>>) -> Json<Value> {
        let push = &state.push;
        let key = crate::accounts::push_key();
        let devices: Vec<Value> = push
            .devices()
            .into_iter()
            .map(|d| {
                json!({
                    "token_tail": d.token.chars().rev().take(6).collect::<String>().chars().rev().collect::<String>(),
                    "name": d.name, "environment": d.environment, "router_ref": d.router_ref,
                    "registered_at": d.registered_at, "last_seen_at": d.last_seen_at, "last_result": d.last_result,
                })
            })
            .collect();
        Json(json!({
            "configured": key.is_some(),
            "team_id": key.as_ref().map(|k| k.team_id.clone()),
            "key_id": key.as_ref().map(|k| k.key_id.clone()),
            "bundle_id": push.topic(),
            "key_file": push.key_file.lock().unwrap().clone(),
            "devices": devices,
            "last_error": push.last_error.lock().unwrap().clone(),
            "last_sent": push.last_sent.lock().unwrap().clone().map(|(at, what)| json!({ "at": at, "what": what })),
        }))
    }

    pub async fn config(State(state): State<Arc<AppState>>, Json(req): Json<ConfigBody>) -> Json<Value> {
        if req.clear {
            return match crate::accounts::store_push_key(None) {
                Ok(()) => { state.push.apns_forget(); Json(json!({ "ok": true, "configured": false })) }
                Err(e) => Json(json!({ "error": e.to_string() })),
            };
        }
        let key = crate::apns::Key { team_id: req.team_id.trim().to_string(), key_id: req.key_id.trim().to_string(), p8: req.p8.trim().to_string() };
        if key.team_id.is_empty() || key.key_id.is_empty() || key.p8.is_empty() {
            return Json(json!({ "error": "team id, key id and the .p8 key are all needed" }));
        }
        // Prove the key signs before keeping it.
        if let Err(e) = crate::apns::mint_jwt(&key, crate::db::now_ms() / 1000) {
            return Json(json!({ "error": format!("that key does not sign: {e}") }));
        }
        let stored = crate::accounts::PushKey {
            team_id: key.team_id, key_id: key.key_id, p8: key.p8,
            bundle_id: req.bundle_id.map(|b| b.trim().to_string()).filter(|b| !b.is_empty()),
        };
        match crate::accounts::store_push_key(Some(stored)) {
            Ok(()) => { state.push.apns_forget(); Json(json!({ "ok": true, "configured": true })) }
            Err(e) => Json(json!({ "error": e.to_string() })),
        }
    }

    pub async fn register(State(state): State<Arc<AppState>>, Json(req): Json<RegisterBody>) -> Json<Value> {
        let token = req.token.trim();
        if token.is_empty() || !token.chars().all(|c| c.is_ascii_hexdigit()) {
            return Json(json!({ "error": "that is not a device token" }));
        }
        let env = req.environment.as_deref().unwrap_or("development");
        match state.push.register(&token.to_ascii_lowercase(), req.name.as_deref(), env, req.router_ref.as_deref()) {
            Ok(()) => {
                state.push.remember_app(req.team_id.as_deref(), req.bundle_id.as_deref());
                let key_file = state.push.import_key_file();
                Json(json!({ "ok": true, "configured": state.push.configured(), "key_file": key_file }))
            }
            Err(e) => Json(json!({ "error": e.to_string() })),
        }
    }

    pub async fn unregister(State(state): State<Arc<AppState>>, Json(req): Json<TokenBody>) -> Json<Value> {
        match state.push.unregister(&req.token.trim().to_ascii_lowercase()) {
            Ok(()) => Json(json!({ "ok": true })),
            Err(e) => Json(json!({ "error": e.to_string() })),
        }
    }

    /// What is waiting for a person right now, keyed as the notifications are. A phone
    /// compares this against the banners it has up and clears the rest — which is what
    /// makes a dropped background push harmless.
    pub async fn pending(State(state): State<Arc<AppState>>) -> Json<Value> {
        Json(json!({ "waiting": state.push.waiting_keys() }))
    }

    /// A notification that says nothing but "it works", to every phone.
    #[derive(Deserialize, Default)]
    pub struct TestBody {
        /// A session for the test to name, so its tap can be checked to land there.
        #[serde(default)]
        pub session: Option<String>,
    }

    pub async fn test(State(state): State<Arc<AppState>>, body: Option<Json<TestBody>>) -> Json<Value> {
        if !state.push.configured() {
            return Json(json!({ "error": "no APNs key is set" }));
        }
        let session = body.and_then(|Json(b)| b.session).filter(|s| !s.trim().is_empty());
        let results = match &session {
            Some(id) => {
                let (project, title) = crate::live::session_label(id).unwrap_or_else(|| ("session".into(), None));
                let name = match title {
                    Some(t) => format!("{project} · {}", super::clip(&t, 40)),
                    None => project,
                };
                state.push.notify(&name, "Test", "Tap to open this session.", Some(id), "test").await
            }
            None => state.push.notify("Agent Router", "Test", "Notifications reach this phone.", None, "test").await,
        };
        Json(json!({
            "ok": results.iter().any(|(_, r)| r.is_ok()),
            "results": results.into_iter().map(|(tail, r)| json!({ "device": tail, "ok": r.is_ok(), "error": r.err() })).collect::<Vec<_>>(),
        }))
    }
}

impl Push {
    /// After the key changes, the cached provider token is for the old one.
    pub fn apns_forget(&self) {
        self.apns.forget_token();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pending_file_becomes_prompts_and_prompts_become_words() {
        let text = concat!(
            r#"{"type":"control_request","request_id":"r-1","request":{"subtype":"can_use_tool","tool_name":"Bash","display_name":"Bash","input":{"command":"cargo test --release"}}}"#, "\n",
            r#"{"type":"control_request","request_id":"r-2","request":{"subtype":"can_use_tool","tool_name":"AskUserQuestion","display_name":"AskUserQuestion","input":{"questions":[{"question":"Which delivery?","header":"Delivery"},{"question":"Scope?"}]}}}"#, "\n",
            r#"{"type":"control_request","request_id":"r-3","request":{"subtype":"can_use_tool","tool_name":"Edit","display_name":"Edit","input":{"file_path":"/code/x/src/main.rs"}}}"#, "\n",
            r#"{"type":"control_response","request_id":"r-9","response":{}}"#, "\n",
            "not json at all\n",
        );
        let prompts = parse_pending(text);
        assert_eq!(prompts.iter().map(|p| p.request_id.as_str()).collect::<Vec<_>>(), ["r-1", "r-2", "r-3"]);
        assert_eq!(describe(&prompts[0]), ("Bash wants to run".into(), "cargo test --release".into()));
        assert_eq!(describe(&prompts[1]), ("2 questions".into(), "Which delivery?".into()));
        assert_eq!(describe(&prompts[2]), ("Edit wants to change".into(), "main.rs".into()));
    }

    #[test]
    fn an_announced_prompt_is_not_announced_again_and_memory_is_bounded() {
        let push = Push::new(Path::new("/nonexistent/for-this-test.db"));
        assert!(!push.seen("req:a"));
        push.mark("req:a");
        assert!(push.seen("req:a"));
        for i in 0..(REMEMBER + 5) {
            push.mark(&format!("req:{i}"));
        }
        assert!(!push.seen("req:a"), "the oldest are forgotten first");
        assert!(push.seen(&format!("req:{}", REMEMBER + 4)));
        // `configured()` reads the Mac's own secrets file, which may well hold a key;
        // what is checkable here is that nothing is imported from beside a database
        // that does not exist.
        assert!(push.import_key_file().is_none());
    }

    #[test]
    fn a_question_nobody_answered_keeps_its_banner() {
        // A wrapper's prompt is answered the moment it leaves the pending file.
        assert!(!still_waiting("req:r-1", "s-1"));
        // A question is asked of the session itself, and a session with no transcript
        // is waiting on nothing.
        assert!(!still_waiting("ask:toolu_1", "no-such-session"));
        assert!(!still_waiting("ask:toolu_1", "../etc/passwd"), "an id that is not one reads as gone");
    }

    #[test]
    fn long_details_are_clipped_for_a_lock_screen() {
        let long = "x".repeat(300);
        assert_eq!(clip(&long, 140).chars().count(), 141);
        assert_eq!(clip("short", 140), "short");
    }
}

#[cfg(test)]
mod key_file_tests {
    use super::*;

    #[test]
    fn the_key_id_is_read_off_apples_file_name() {
        assert_eq!(key_id_of("AuthKey_ABC123DEF4.p8").as_deref(), Some("ABC123DEF4"));
        assert_eq!(key_id_of("AuthKey_ABC123DEF4.p8.txt"), None);
        assert_eq!(key_id_of("AuthKey_.p8"), None);
        assert_eq!(key_id_of("apns.p8"), None);
        assert_eq!(key_id_of("AuthKey_ABC-123DEF.p8"), None);
    }

    #[test]
    fn the_newest_key_file_beside_the_database_is_the_one_taken() {
        let dir = std::env::temp_dir().join(format!("ar-keyfile-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(key_file_in(&dir).is_none(), "nothing there yet");
        std::fs::write(dir.join("AuthKey_AAAAAAAAAA.p8"), "old").unwrap();
        std::fs::write(dir.join("notes.p8.txt"), "not a key").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(dir.join("AuthKey_BBBBBBBBBB.p8"), "new").unwrap();
        let (path, id) = key_file_in(&dir).unwrap();
        assert_eq!(id, "BBBBBBBBBB");
        assert!(path.ends_with("AuthKey_BBBBBBBBBB.p8"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
