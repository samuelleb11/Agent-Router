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
use crate::fcm::Fcm;

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
    /// Whose push service this token belongs to: `apns` or `fcm`.
    pub platform: String,
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
    fcm: Fcm,
    db_path: PathBuf,
    seen: Mutex<(HashSet<String>, VecDeque<String>)>,
    /// What Apple last said, and what Google last said. Two slots, not one: a working
    /// iPhone clearing the field would quietly erase a standing Android failure.
    pub last_error: Mutex<Option<String>>,
    pub fcm_last_error: Mutex<Option<String>>,
    pub last_sent: Mutex<Option<(i64, String)>>,
    /// The key file dropped beside the database: what was found, and what it waits on.
    pub key_file: Mutex<Option<String>>,
    pub fcm_key_file: Mutex<Option<String>>,
    /// The (file name, mtime) whose import failed outright, so it is not retried every
    /// second. One per file, because they are two different files.
    tried: Mutex<Option<(String, std::time::SystemTime)>>,
    fcm_tried: Mutex<Option<(String, std::time::SystemTime)>>,
    /// Prompts announced to a phone that are still waiting, as key -> session: what a
    /// banner may be up for, and so what a dismissal is owed for.
    announced: Mutex<HashMap<String, String>>,
}

impl Push {
    pub fn new(db_path: &Path) -> Self {
        Self {
            apns: Apns::new(),
            fcm: Fcm::new(),
            db_path: db_path.to_path_buf(),
            seen: Mutex::new((HashSet::new(), VecDeque::new())),
            last_error: Mutex::new(None),
            fcm_last_error: Mutex::new(None),
            last_sent: Mutex::new(None),
            key_file: Mutex::new(None),
            fcm_key_file: Mutex::new(None),
            tried: Mutex::new(None),
            fcm_tried: Mutex::new(None),
            announced: Mutex::new(HashMap::new()),
        }
    }

    pub fn apns_key(&self) -> Option<Key> {
        crate::accounts::push_key().map(|k| Key { team_id: k.team_id, key_id: k.key_id, p8: k.p8 })
    }

    pub fn fcm_key(&self) -> Option<crate::fcm::Key> {
        crate::accounts::fcm_key().map(|k| crate::fcm::Key {
            project_id: k.project_id,
            client_email: k.client_email,
            private_key: k.private_key,
            private_key_id: k.private_key_id,
            token_uri: k.token_uri,
        })
    }

    pub fn topic(&self) -> String {
        crate::accounts::push_key()
            .and_then(|k| k.bundle_id)
            .filter(|b| !b.trim().is_empty())
            .or_else(|| crate::db::setting(&self.db_path, BUNDLE_KEY))
            .unwrap_or_else(|| DEFAULT_TOPIC.into())
    }

    pub fn apns_configured(&self) -> bool {
        self.apns_key().is_some()
    }

    pub fn fcm_configured(&self) -> bool {
        self.fcm_key().is_some()
    }

    /// Whether any phone at all can be told — what `watch` turns on. Either service
    /// being set up is enough; a router may well only ever see one kind of phone.
    pub fn configured(&self) -> bool {
        self.apns_configured() || self.fcm_configured()
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

    fn note_fcm_key_file(&self, note: String) -> Option<String> {
        *self.fcm_key_file.lock().unwrap() = Some(note.clone());
        Some(note)
    }

    /// The service account as the Firebase console hands it out, dropped beside the
    /// database. Any `*.json` there that says it is one is taken in — the project read
    /// out of the file itself, so nothing is typed — and the file then made private.
    ///
    /// Simpler than its Apple counterpart in one way: a service account is self-contained,
    /// so there is nothing to wait for a phone to tell us, and this can run at startup.
    /// Unlike Apple's key it can also be re-issued at will, so losing one costs a click.
    pub fn import_service_account_file(&self) -> Option<String> {
        if self.fcm_configured() {
            return None;
        }
        let dir = self.db_path.parent()?;
        let (path, key) = service_account_in(dir)?;
        let name = path.file_name()?.to_string_lossy().into_owned();
        let mtime = std::fs::metadata(&path).and_then(|m| m.modified()).ok()?;
        if self.fcm_tried.lock().unwrap().as_ref() == Some(&(name.clone(), mtime)) {
            return self.fcm_key_file.lock().unwrap().clone();
        }
        let failed = |why: String| {
            *self.fcm_tried.lock().unwrap() = Some((name.clone(), mtime));
            tracing::warn!(file = %name, "service account not taken: {why}");
            self.note_fcm_key_file(format!("{name}: {why}"))
        };
        if let Err(e) = crate::fcm::mint_jwt(&key, crate::db::now_ms() / 1000) {
            return failed(format!("does not sign: {e}"));
        }
        let stored = crate::accounts::FcmKey {
            project_id: key.project_id.clone(),
            client_email: key.client_email.clone(),
            private_key: key.private_key.clone(),
            private_key_id: key.private_key_id.clone(),
            token_uri: key.token_uri.clone(),
        };
        if let Err(e) = crate::accounts::store_fcm_key(Some(stored)) {
            return failed(format!("could not store: {e}"));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
        self.fcm_forget();
        tracing::info!(file = %path.display(), project = %key.project_id, "service account imported; Android phones can be told");
        self.note_fcm_key_file(format!("imported from {name} · project {}", key.project_id))
    }

    /// The key as the file Apple hands out, `AuthKey_<key id>.p8`, dropped beside the
    /// database. Taken in once the team is known — the app says which when it registers
    /// — so nothing is typed. The file stays where it is, made private: Apple never
    /// hands the same key out twice. Returns what was done, or what is still waited on.
    pub fn import_key_file(&self) -> Option<String> {
        if self.apns_configured() {
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
            "SELECT token, platform, name, environment, router_ref, registered_at, last_seen_at, last_result
             FROM push_devices ORDER BY registered_at",
        ) else {
            return Vec::new();
        };
        stmt.query_map([], |r| {
            Ok(Device {
                token: r.get(0)?,
                platform: r.get(1)?,
                name: r.get(2)?,
                environment: r.get(3)?,
                router_ref: r.get(4)?,
                registered_at: r.get(5)?,
                last_seen_at: r.get(6)?,
                last_result: r.get(7)?,
            })
        })
        .map(|rows| rows.filter_map(Result::ok).collect())
        .unwrap_or_default()
    }

    pub fn register(
        &self,
        token: &str,
        platform: &str,
        name: Option<&str>,
        environment: &str,
        router_ref: Option<&str>,
    ) -> anyhow::Result<()> {
        let now = crate::db::now_ms();
        self.open()?.execute(
            "INSERT INTO push_devices (token, platform, name, environment, router_ref, registered_at, last_seen_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)
             ON CONFLICT(token) DO UPDATE SET platform = ?2, name = ?3, environment = ?4, router_ref = ?5, last_seen_at = ?6",
            params![token, platform, name, environment, router_ref, now],
        )?;
        Ok(())
    }

    /// Forget a phone. Either spelling of the token: Apple's is hex and was stored
    /// lowercased from the start, Google's is case-sensitive and is stored as given, and
    /// a caller holding one cannot be expected to know which it has.
    pub fn unregister(&self, token: &str) -> anyhow::Result<()> {
        self.open()?.execute(
            "DELETE FROM push_devices WHERE token = ?1 OR token = ?2",
            params![token, token.to_ascii_lowercase()],
        )?;
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

    /// Send one message to every registered phone, by whichever service that phone
    /// belongs to, and keep what that service said about each. Both payloads are built
    /// per device, because each is told which router this came from — and the two
    /// services want different words for the same message, so each is written where it
    /// is read rather than translated out of the other. Returns one line per device.
    async fn fan_out(
        &self,
        delivery: Delivery<'_>,
        apns_payload: impl Fn(&Device) -> Value,
        fcm_data: impl Fn(&Device) -> Value,
    ) -> Vec<(String, Result<(), String>)> {
        let apns_key = self.apns_key();
        let fcm_key = self.fcm_key();
        if apns_key.is_none() && fcm_key.is_none() {
            return Vec::new();
        }
        let topic = self.topic();
        let mut out = Vec::new();
        for d in self.devices() {
            let tail = d.token.chars().rev().take(6).collect::<String>().chars().rev().collect::<String>();
            let android = d.platform == "fcm";
            // Flattened to (forget this phone, why) so the bookkeeping below is written
            // once, for two services that know nothing about each other. A phone whose
            // service has no credential is skipped rather than reported: a router that
            // only ever had iPhones should not start announcing Android failures.
            let outcome: Result<(), (bool, String)> = if android {
                let Some(key) = fcm_key.as_ref() else { continue };
                self.fcm
                    .send(key, &d.token, delivery, &fcm_data(&d))
                    .await
                    .map_err(|e| (matches!(e, SendError::Unregistered), e.to_string()))
            } else {
                let Some(key) = apns_key.as_ref() else { continue };
                self.apns
                    .send(key, &d.token, &d.environment, &topic, delivery, &apns_payload(&d))
                    .await
                    .map_err(|e| (matches!(e, SendError::Unregistered), e.to_string()))
            };
            let slot = if android { &self.fcm_last_error } else { &self.last_error };
            match outcome {
                Ok(()) => {
                    self.record(&d.token, "delivered");
                    *slot.lock().unwrap() = None;
                    out.push((tail, Ok(())));
                }
                Err((true, _)) => {
                    tracing::info!(token = %tail, platform = %d.platform, "phone unregistered upstream; forgetting it");
                    let _ = self.unregister(&d.token);
                    out.push((tail, Err("no longer registered".into())));
                }
                Err((false, why)) => {
                    tracing::warn!(token = %tail, platform = %d.platform, error = %why, "push failed");
                    self.record(&d.token, &why);
                    *slot.lock().unwrap() = Some(why.clone());
                    out.push((tail, Err(why)));
                }
            }
        }
        out
    }

    /// Tell every registered phone. Returns one line per device.
    pub async fn notify(&self, title: &str, subtitle: &str, body: &str, session: Option<&str>, collapse: &str) -> Vec<(String, Result<(), String>)> {
        let out = self
            .fan_out(
                Delivery::Alert(collapse),
                |d| {
                    let mut alert = json!({ "title": title });
                    if !subtitle.is_empty() { alert["subtitle"] = json!(subtitle); }
                    if !body.is_empty() { alert["body"] = json!(body); }
                    json!({
                        "aps": { "alert": alert, "sound": "default", "thread-id": session.unwrap_or("router"), "category": "PROMPT" },
                        "session": session,
                        "router_ref": d.router_ref,
                        "request_id": collapse,
                    })
                },
                // Android draws its own notification, so it is handed the parts rather
                // than a rendered alert: whose session it is, what is being asked, and
                // the detail that goes under it. Empty ones are dropped on the way out.
                |d| json!({
                    "title": title,
                    "head": subtitle,
                    "detail": body,
                    "session": session,
                    "router_ref": d.router_ref,
                    "request_id": collapse,
                }),
            )
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
        self.fan_out(
            Delivery::Background,
            |d| json!({
                "aps": { "content-available": 1 },
                "dismiss": answered,
                "router_ref": d.router_ref,
            }),
            // FCM carries strings and nothing else, so the list crosses as JSON in one
            // value; the app parses it back out.
            |d| json!({
                "dismiss": answered,
                "router_ref": d.router_ref,
            }),
        )
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
        self.import_service_account_file();
        loop {
            tokio::time::sleep(POLL).await;
            // Each importer is a no-op once its own service is set up, so a router that
            // already has one credential still notices the other being dropped in.
            self.import_key_file();
            self.import_service_account_file();
            if !self.configured() {
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

/// Which push service a token belongs to, or `None` if it is not a device token at all.
///
/// The app says which it is, and is believed when it does — the shape is only how an
/// older build that says nothing is still placed. Apple's is checked first, because a
/// 64-character hex string satisfies Google's character set too.
pub fn platform_of(token: &str, declared: Option<&str>) -> Option<&'static str> {
    match declared.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
        Some("apns") => is_apns_token(token).then_some("apns"),
        Some("fcm") => is_fcm_token(token).then_some("fcm"),
        // A service we do not speak is an error, not something to guess around.
        Some(other) if !other.is_empty() => None,
        _ if is_apns_token(token) => Some("apns"),
        _ if is_fcm_token(token) => Some("fcm"),
        _ => None,
    }
}

/// Apple's: hex, and an even number of digits. Sixty-four today, but Apple asks that no
/// exact length be assumed and has lengthened it before, so only the shape is checked.
fn is_apns_token(t: &str) -> bool {
    t.len() >= 64 && t.len().is_multiple_of(2) && t.chars().all(|c| c.is_ascii_hexdigit())
}

/// Google's: long, case-sensitive, and drawn from the URL-safe alphabet plus the colon
/// that separates the instance id from the token proper. The colon is not required —
/// the `<iid>:APA91b…` form is usual but not guaranteed across SDK versions — and the
/// length is bounded only to keep a paste accident out of the database.
fn is_fcm_token(t: &str) -> bool {
    (64..=4096).contains(&t.len())
        && t.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, ':' | '_' | '-' | '.' | '%'))
}

/// The key id Apple puts in the file name: `AuthKey_<ten alphanumerics>.p8`.
pub fn key_id_of(name: &str) -> Option<String> {
    let id = name.strip_prefix("AuthKey_")?.strip_suffix(".p8")?;
    (id.len() == 10 && id.chars().all(|c| c.is_ascii_alphanumeric())).then(|| id.to_string())
}

/// The newest service account in a directory, and the key read out of it.
///
/// Chosen by what is inside rather than by name: Apple's file name carries the key id,
/// where Firebase's (`<project>-firebase-adminsdk-<hash>-<hash>.json`) carries nothing
/// worth matching on.
fn service_account_in(dir: &Path) -> Option<(PathBuf, crate::fcm::Key)> {
    /// Ample for a service account, and small enough that a stray JSON left in the
    /// directory is not read off the disk once a second.
    const CAP: u64 = 64 * 1024;
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            let name = path.file_name()?.to_string_lossy().into_owned();
            // The router's own secrets file lives in this directory. Never read it back in.
            if !name.ends_with(".json") || name.starts_with("credentials.json") {
                return None;
            }
            let meta = e.metadata().ok()?;
            if meta.len() > CAP {
                return None;
            }
            let text = std::fs::read_to_string(&path).ok()?;
            let key = crate::fcm::parse_service_account(&text).ok()?;
            Some((meta.modified().ok()?, path, key))
        })
        .max_by_key(|(modified, ..)| *modified)
        .map(|(_, path, key)| (path, key))
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
        /// Which credential this is about. Absent means Apple's, so the existing form
        /// posts exactly as it did.
        #[serde(default)]
        pub platform: Option<String>,
        /// The service-account JSON, whole, as the Firebase console downloads it.
        #[serde(default)]
        pub service_account: Option<String>,
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
        /// Which service the token belongs to: `apns` or `fcm`. Absent means the shape
        /// of the token decides, so an iPhone build that predates Android still registers.
        #[serde(default)]
        pub platform: Option<String>,
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
                    "platform": d.platform,
                    "name": d.name, "environment": d.environment, "router_ref": d.router_ref,
                    "registered_at": d.registered_at, "last_seen_at": d.last_seen_at, "last_result": d.last_result,
                })
            })
            .collect();
        let fcm = crate::accounts::fcm_key();
        Json(json!({
            // These keep meaning Apple, so the dashboard's key form needs no changes.
            "configured": key.is_some(),
            "team_id": key.as_ref().map(|k| k.team_id.clone()),
            "key_id": key.as_ref().map(|k| k.key_id.clone()),
            "bundle_id": push.topic(),
            "key_file": push.key_file.lock().unwrap().clone(),
            // The other service. `private_key_id` is an id, not a key, and is the only
            // way to tell two service accounts apart at a glance.
            "fcm": {
                "configured": fcm.is_some(),
                "project_id": fcm.as_ref().map(|k| k.project_id.clone()),
                "client_email": fcm.as_ref().map(|k| k.client_email.clone()),
                "private_key_id": fcm.as_ref().map(|k| k.private_key_id.clone()),
                "key_file": push.fcm_key_file.lock().unwrap().clone(),
                "last_error": push.fcm_last_error.lock().unwrap().clone(),
            },
            // What the UI should ask before offering a test: either service will do.
            "any_configured": push.configured(),
            "devices": devices,
            "last_error": push.last_error.lock().unwrap().clone(),
            "last_sent": push.last_sent.lock().unwrap().clone().map(|(at, what)| json!({ "at": at, "what": what })),
        }))
    }

    pub async fn config(State(state): State<Arc<AppState>>, Json(req): Json<ConfigBody>) -> Json<Value> {
        if req.platform.as_deref() == Some("fcm") {
            return fcm_config(state, req);
        }
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

    /// The Firebase half of `/api/push/config`. Takes the console's file whole rather
    /// than four fields: it is what the user actually has in hand, it cannot drift
    /// between fields, and one parse gives one precise complaint instead of four.
    fn fcm_config(state: Arc<AppState>, req: ConfigBody) -> Json<Value> {
        if req.clear {
            return match crate::accounts::store_fcm_key(None) {
                Ok(()) => {
                    state.push.fcm_forget();
                    Json(json!({ "ok": true, "configured": false }))
                }
                Err(e) => Json(json!({ "error": e.to_string() })),
            };
        }
        let text = req.service_account.unwrap_or_default();
        let text = text.trim();
        if text.is_empty() {
            return Json(json!({ "error": "paste the service-account JSON the Firebase console downloads" }));
        }
        let key = match crate::fcm::parse_service_account(text) {
            Ok(k) => k,
            Err(e) => return Json(json!({ "error": format!("that is not a service account: {e}") })),
        };
        // Prove the key signs before keeping it — the same gate the `.p8` gets.
        if let Err(e) = crate::fcm::mint_jwt(&key, crate::db::now_ms() / 1000) {
            return Json(json!({ "error": format!("that key does not sign: {e}") }));
        }
        let stored = crate::accounts::FcmKey {
            project_id: key.project_id.clone(),
            client_email: key.client_email,
            private_key: key.private_key,
            private_key_id: key.private_key_id,
            token_uri: key.token_uri,
        };
        match crate::accounts::store_fcm_key(Some(stored)) {
            Ok(()) => {
                state.push.fcm_forget();
                Json(json!({ "ok": true, "configured": true, "project_id": key.project_id }))
            }
            Err(e) => Json(json!({ "error": e.to_string() })),
        }
    }

    pub async fn register(State(state): State<Arc<AppState>>, Json(req): Json<RegisterBody>) -> Json<Value> {
        let token = req.token.trim();
        let Some(platform) = super::platform_of(token, req.platform.as_deref()) else {
            return Json(json!({ "error": "that is not a device token" }));
        };
        // Apple's token is hex and case-insensitive, and has been stored lowercased from
        // the start; Google's is neither, and has to be kept exactly as the phone gave it.
        let stored = if platform == "apns" { token.to_ascii_lowercase() } else { token.to_string() };
        let env = req.environment.as_deref().unwrap_or("development");
        match state.push.register(&stored, platform, req.name.as_deref(), env, req.router_ref.as_deref()) {
            Ok(()) => {
                let (key_file, configured) = if platform == "apns" {
                    // The team and bundle id are Apple's, and naming a topic after an
                    // Android package would poison every push to every iPhone.
                    state.push.remember_app(req.team_id.as_deref(), req.bundle_id.as_deref());
                    (state.push.import_key_file(), state.push.apns_configured())
                } else {
                    (state.push.import_service_account_file(), state.push.fcm_configured())
                };
                Json(json!({ "ok": true, "platform": platform, "configured": configured, "key_file": key_file }))
            }
            Err(e) => Json(json!({ "error": e.to_string() })),
        }
    }

    pub async fn unregister(State(state): State<Arc<AppState>>, Json(req): Json<TokenBody>) -> Json<Value> {
        match state.push.unregister(req.token.trim()) {
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
            return Json(json!({ "error": "no push credentials are set" }));
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

    /// Likewise: a new service account means the held access token belongs to nobody.
    pub fn fcm_forget(&self) {
        self.fcm.forget_token();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real one of each, so the two are told apart on the shapes that actually occur.
    const APNS: &str = "740f4707bebcf74f9b7c25d48e3358945f6aa01da5ddb387462c7eaf61bb78ad";
    const FCM: &str = "cjE9vT2zRk6mQq1sLp0aBc:APA91bH-x_9Kd3fQz7YwVn2pRt5uXsA0bCdEfGhIjKlMnOpQrStUvWxYz01234567890abcdefghijklmnopqrstuvwxyzAB";

    #[test]
    fn a_phone_is_placed_by_what_it_says_it_is_or_by_the_shape_of_its_token() {
        // Said outright, and believed.
        assert_eq!(platform_of(APNS, Some("apns")), Some("apns"));
        assert_eq!(platform_of(FCM, Some("fcm")), Some("fcm"));
        assert_eq!(platform_of(FCM, Some("FCM")), Some("fcm"), "however it is spelt");

        // Unsaid — an iPhone build that predates Android says nothing, and still lands.
        assert_eq!(platform_of(APNS, None), Some("apns"));
        assert_eq!(platform_of(FCM, None), Some("fcm"));
        assert_eq!(platform_of(APNS, Some("")), Some("apns"), "an empty claim is no claim");

        // Hex is checked first: Apple's token also satisfies Google's character set,
        // and guessing the wrong way round would send it to the wrong service forever.
        assert_eq!(platform_of(APNS, None), Some("apns"));

        // Claimed as something it is not.
        assert_eq!(platform_of(FCM, Some("apns")), None, "not hex, so not Apple's");
        assert_eq!(platform_of("short", Some("fcm")), None);
        assert_eq!(platform_of(APNS, Some("webpush")), None, "a service we do not speak");
    }

    #[test]
    fn what_is_not_a_device_token_is_refused() {
        for junk in ["", "   ", "hello", "req:not-a-token", &"a".repeat(63), &"0".repeat(4097)] {
            assert_eq!(platform_of(junk, None), None, "{junk:?} is not a token");
        }
        // Hex but an odd number of digits is not Apple's, and an app that claims Apple
        // is told so.
        assert_eq!(platform_of(&"a".repeat(65), Some("apns")), None);

        // What this check cannot do, stated plainly: an FCM token is opaque, so any long
        // enough run of the right characters passes — including that odd-length hex, and
        // including a run of z's. Nothing local can tell a real one from a plausible one;
        // Google's UNREGISTERED is what settles it, and the device is forgotten then. The
        // point here is to keep a stray paste out of the table, not to authenticate a phone.
        assert_eq!(platform_of(&"z".repeat(64), None), Some("fcm"));
        assert_eq!(platform_of(&"a".repeat(65), None), Some("fcm"));
    }

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
