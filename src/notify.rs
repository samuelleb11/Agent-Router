//! Telling a phone that a session is waiting on a person.
//!
//! Whether a session is waiting is `live::session_activity`'s answer, the same one the
//! list and the open view give, and what it is waiting on decides the key:
//!
//!   * `req:<request_id>` — a prompt a wrapped session's wrapper recorded (a Bash
//!     approval, an edit, an AskUserQuestion, a plan), answerable from the phone;
//!   * `ask:<tool_use_id>` — a question, a plan or a tool call the transcript shows open
//!     while the session waits;
//!   * `ask:wait-<session>-<statusUpdatedAt>` — only the CLI's registry knows: an
//!     unwrapped session, answerable only where it runs.
//!
//! The installed apps reconcile and take down only `req:` and `ask:` banners, which is
//! why every key wears one of those two prefixes. Swept once a second; each key is
//! announced once, after it has waited two seconds without a break (auto mode and hooks
//! resolve many prompts within one).
//!
//! Nothing here can unblock the session: that is the phone's job, and what the app
//! opens to when the notification is tapped. What *does* happen here is the other half
//! of that: a prompt answered anywhere — the terminal, the editor, another phone —
//! leaves the waiting set, and the banner it raised is taken down again.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use rusqlite::{params, Connection, OpenFlags};
use serde_json::{json, Value};

use crate::activity::Kind;
use crate::apns::{Apns, Delivery, Key, SendError};
use crate::fcm::Fcm;
use crate::live::{Observed, Roots};
use crate::registry::Snapshot;

const POLL: Duration = Duration::from_secs(1);
/// Without a registry, unwrapped sessions are only worth reading if they moved recently.
const RECENT_MS: i64 = 10 * 60_000;
/// A prompt is announced once it has waited this long without a break. Auto mode and
/// hooks resolve some prompts within about a second, and a banner for one of those only
/// ever arrives already stale.
const ANNOUNCE_AFTER_MS: i64 = 2_000;
/// How often wrapper sockets no live session holds are checked for a listener, and what
/// dead wrappers left behind is cleared.
const JANITOR_EVERY: Duration = Duration::from_secs(60);
/// A wrapper record with no socket beside it for this long is a leftover. A live wrapper
/// is without its socket only for moments: before it binds, while it rebinds one that
/// vanished (within 3 s), and between the renames of a `/clear`.
const ORPHAN_AFTER: Duration = Duration::from_secs(10 * 60);
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

/// One line of a wrapper's `.pending`, if it is an open `can_use_tool` request.
pub fn prompt_of(v: &Value) -> Option<Prompt> {
    if v["request"]["subtype"] != "can_use_tool" {
        return None;
    }
    let request_id = v["request_id"].as_str()?.to_owned();
    let tool = v["request"]["tool_name"].as_str().unwrap_or("tool").to_owned();
    let display = v["request"]["display_name"].as_str().unwrap_or(&tool).to_owned();
    Some(Prompt { request_id, tool, display, input: v["request"]["input"].clone() })
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
            let name = crate::live::basename(&file).to_owned();
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
    /// A hosted router's push keys: a Secret mounted read-only, looked in after the
    /// store's own directory.
    keys_dir: Option<PathBuf>,
    /// A hosted router's machines, whose agents say what is waiting — it has no
    /// transcripts or registry of its own to read.
    machines: std::sync::OnceLock<Arc<crate::machines::Machines>>,
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
    /// Every key in the last sweep, with when it began waiting without a break.
    waiting_since: Mutex<HashMap<String, i64>>,
    /// Announced `req:` keys missing from the sweep, with how many sweeps in a row.
    missing: Mutex<HashMap<String, u32>>,
}

impl Push {
    pub fn new(db_path: &Path) -> Self {
        Self {
            apns: Apns::new(),
            fcm: Fcm::new(),
            db_path: db_path.to_path_buf(),
            keys_dir: None,
            machines: std::sync::OnceLock::new(),
            seen: Mutex::new((HashSet::new(), VecDeque::new())),
            last_error: Mutex::new(None),
            fcm_last_error: Mutex::new(None),
            last_sent: Mutex::new(None),
            key_file: Mutex::new(None),
            fcm_key_file: Mutex::new(None),
            tried: Mutex::new(None),
            fcm_tried: Mutex::new(None),
            announced: Mutex::new(HashMap::new()),
            waiting_since: Mutex::new(HashMap::new()),
            missing: Mutex::new(HashMap::new()),
        }
    }

    /// Also take key files from `dir` — the Secret a deployment mounts for a hosted
    /// router, which has no one to drop a file beside its store.
    pub fn with_keys_dir(mut self, dir: Option<PathBuf>) -> Self {
        self.keys_dir = dir;
        self
    }

    /// Take what is waiting from these machines' agents (a hosted router).
    pub fn use_machines(&self, machines: Arc<crate::machines::Machines>) {
        let _ = self.machines.set(machines);
    }

    /// Where a dropped key file is looked for: beside the store, then the deployment's
    /// Secret when there is one.
    fn key_dirs(&self) -> Vec<&Path> {
        self.db_path.parent().into_iter().chain(self.keys_dir.as_deref()).collect()
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
        let (path, key) = self.key_dirs().into_iter().find_map(service_account_in)?;
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
        let (path, key_id) = self.key_dirs().into_iter().find_map(key_file_in)?;
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
    /// unanswered. The second half matters without a registry: a session that has sat on
    /// a question for longer than `RECENT_MS` is no longer swept, and a phone told only
    /// the first half would take down a banner for a question nobody has answered.
    ///
    /// Blocking: it reads the registry and the transcripts.
    pub fn waiting_keys(&self) -> Vec<String> {
        let mut keys: HashSet<String> = self.waiting().into_iter().map(|(key, ..)| key).collect();
        let announced: Vec<(String, String)> = self
            .announced
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .map(|(k, s)| (k.clone(), s.clone()))
            .collect();
        for (key, session) in announced {
            if !keys.contains(&key) && still_waiting(&key, &session) {
                keys.insert(key);
            }
        }
        let mut out: Vec<String> = keys.into_iter().collect();
        out.sort();
        out
    }

    /// Everything currently waiting, as (dedupe key, session id, headline, detail).
    fn waiting(&self) -> Vec<(String, String, String, String)> {
        let reg = crate::registry::snapshot();
        waiting_in(&Roots::system(), &reg, crate::db::now_ms())
    }

    /// One pass of the watcher, run off the async workers because all of it reads files:
    /// the prompts answered since the last pass, and the ones now due to be announced,
    /// each with the name to announce it under.
    fn sweep(&self, now: i64) -> (Vec<String>, Vec<Announce>) {
        let waiting = self.waiting();
        // Gone from the sweep is not the same as answered — see `still_waiting`, asked
        // without holding the lock, since it reads the session.
        let elsewhere: HashSet<String> = self
            .missing_from(&waiting)
            .into_iter()
            .filter(|(key, session)| still_waiting(key, session))
            .map(|(key, _)| key)
            .collect();
        self.sweep_with(now, waiting, elsewhere, crate::live::session_label)
    }

    /// The announced prompts that a sweep no longer shows, as (key, session): answered,
    /// or only gone from view (see `still_waiting`).
    pub fn missing_from(&self, waiting: &[(String, String, String, String)]) -> Vec<(String, String)> {
        let still: HashSet<&str> = waiting.iter().map(|(k, ..)| k.as_str()).collect();
        self.announced
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .filter(|(key, _)| !still.contains(key.as_str()))
            .map(|(k, s)| (k.clone(), s.clone()))
            .collect()
    }

    /// A sweep over evidence gathered elsewhere: what is waiting, which of the announced
    /// prompts missing from it are still waiting, and how to name a session. The local
    /// router reads all three from this machine; a hosted router hears them from agents.
    fn sweep_with(
        &self,
        now: i64,
        waiting: Vec<(String, String, String, String)>,
        elsewhere: HashSet<String>,
        label: impl Fn(&str) -> Option<(String, Option<String>)>,
    ) -> (Vec<String>, Vec<Announce>) {
        let still: HashSet<&str> = waiting.iter().map(|(k, ..)| k.as_str()).collect();
        let due: HashSet<String> = {
            let mut since = self.waiting_since.lock().unwrap_or_else(|e| e.into_inner());
            waited(&mut since, &still, now)
        };
        let answered = {
            let mut announced = self.announced.lock().unwrap_or_else(|e| e.into_inner());
            let mut missing = self.missing.lock().unwrap_or_else(|e| e.into_inner());
            settle(&mut announced, &mut missing, &still, &elsewhere)
        };

        let fresh = waiting
            .into_iter()
            .filter(|(key, ..)| due.contains(key) && !self.seen(key))
            .map(|(key, session, head, detail)| {
                let (project, title) = label(&session).unwrap_or_else(|| ("session".into(), None));
                let name = match title {
                    Some(t) => format!("{project} · {}", clip(&t, 40)),
                    None => project,
                };
                Announce { key, session, name, head, detail }
            })
            .collect();
        (answered, fresh)
    }

    /// Runs for the life of the router. What is already waiting at startup is not
    /// announced — a restart is not news.
    pub async fn watch(self: Arc<Self>) {
        // A hosted router has none of what the sweep and the janitor read — no
        // transcripts, no session registry, no wrapper sockets — so it only takes keys
        // in, and delivers. What is waiting will come from the machine agents.
        let local = !crate::config::is_server();
        if local {
            let me = self.clone();
            if let Ok(already) = tokio::task::spawn_blocking(move || me.waiting()).await {
                for (key, ..) in already {
                    self.mark(&key);
                }
            }
        }
        self.import_key_file();
        self.import_service_account_file();
        // First pass straight away: leftovers from before a restart are the likeliest.
        let mut swept_sockets: Option<Instant> = None;
        loop {
            tokio::time::sleep(POLL).await;
            // Each importer is a no-op once its own service is set up, so a router that
            // already has one credential still notices the other being dropped in.
            self.import_key_file();
            self.import_service_account_file();
            // Housekeeping, not notification: it runs whether or not a phone can be told.
            if local && swept_sockets.is_none_or(|at| at.elapsed() >= JANITOR_EVERY) {
                swept_sockets = Some(Instant::now());
                let cleared = tokio::task::spawn_blocking(|| {
                    janitor(&crate::peer::wrapper_dir(), &crate::registry::snapshot())
                })
                .await
                .unwrap_or(0);
                if cleared > 0 {
                    tracing::info!(cleared, "removed what dead wrappers left behind");
                }
            }
            if !self.configured() {
                continue;
            }
            let swept = if local {
                let me = self.clone();
                tokio::task::spawn_blocking(move || me.sweep(crate::db::now_ms())).await
            } else {
                let Some(machines) = self.machines.get().cloned() else {
                    continue;
                };
                let evidence = machines.evidence(true).await;
                // What a machine had waiting when it connected is not news, for the same
                // reason a restart is not.
                for key in &evidence.first {
                    self.mark(key);
                }
                let elsewhere = machines.still_waiting(self.missing_from(&evidence.waiting)).await;
                let me = self.clone();
                let labels = evidence.labels;
                let waiting = evidence.waiting;
                tokio::task::spawn_blocking(move || {
                    me.sweep_with(crate::db::now_ms(), waiting, elsewhere, |s| labels.get(s).cloned())
                })
                .await
            };
            let Ok((answered, fresh)) = swept else {
                continue;
            };
            if !answered.is_empty() {
                tracing::info!(prompts = answered.len(), "answered elsewhere; taking the banners down");
                self.dismiss(&answered).await;
            }
            for a in fresh {
                self.mark(&a.key);
                self.announced
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(a.key.clone(), a.session.clone());
                tracing::info!(session = %a.session, head = %a.head, "prompt waiting; telling the phone");
                self.notify(&a.name, &a.head, &a.detail, Some(&a.session), &a.key).await;
            }
        }
    }
}

/// A prompt due to be announced, with the name of the session it is in.
struct Announce {
    key: String,
    session: String,
    name: String,
    head: String,
    detail: String,
}

/// Keep each key's clock of how long it has waited without a break, and say which have
/// waited long enough to announce. A key that leaves the sweep, however briefly, starts
/// over: a prompt resolved and re-raised is a new wait.
fn waited(since: &mut HashMap<String, i64>, still: &HashSet<&str>, now: i64) -> HashSet<String> {
    since.retain(|key, _| still.contains(key.as_str()));
    for key in still {
        since.entry((*key).to_owned()).or_insert(now);
    }
    since
        .iter()
        .filter(|(_, at)| now - **at >= ANNOUNCE_AFTER_MS)
        .map(|(key, _)| key.clone())
        .collect()
}

/// Which announced prompts are over: gone from the sweep and not still waiting somewhere
/// it cannot see (`elsewhere`). A `req:` key has to be missing from two sweeps in a row
/// first — an older wrapper rewrites its `.pending` by truncating it, so one read now and
/// then finds it empty, and a banner taken down for that is never put back up. Removes
/// what it returns from `announced`.
fn settle(
    announced: &mut HashMap<String, String>,
    missing: &mut HashMap<String, u32>,
    still: &HashSet<&str>,
    elsewhere: &HashSet<String>,
) -> Vec<String> {
    let mut gone = Vec::new();
    for key in announced.keys() {
        if still.contains(key.as_str()) || elsewhere.contains(key) {
            missing.remove(key);
            continue;
        }
        if key.starts_with("req:") {
            let n = missing.entry(key.clone()).or_insert(0);
            *n += 1;
            if *n < 2 {
                continue;
            }
        }
        gone.push(key.clone());
    }
    missing.retain(|key, _| announced.contains_key(key));
    for key in &gone {
        announced.remove(key);
        missing.remove(key);
    }
    gone.sort();
    gone
}

/// Everything waiting on a person, as (key, session, headline, detail): every open
/// session whose activity is Waiting when the registry says which are open; without it,
/// as before there was one, the transcripts that moved lately — plus every wrapped
/// session, since one sitting on a prompt does not move.
pub(crate) fn waiting_in(roots: &Roots, reg: &Snapshot, now: i64) -> Vec<(String, String, String, String)> {
    let mut sessions: HashMap<String, Option<PathBuf>> = HashMap::new();
    if reg.available {
        for id in reg.by_session.keys() {
            // A process with no conversation yet — a new tab, the editor's config loader —
            // is not listed anywhere, so a notification would open onto nothing.
            if let Some(path) = crate::live::transcript_path_in(&roots.projects, id) {
                sessions.insert(id.clone(), Some(path));
            }
        }
    } else {
        for (id, path) in crate::live::recent_transcripts_in(&roots.projects, RECENT_MS) {
            sessions.insert(id, Some(path));
        }
        for id in socket_ids(&roots.wrapper) {
            if let std::collections::hash_map::Entry::Vacant(slot) = sessions.entry(id) {
                let path = crate::live::transcript_path_in(&roots.projects, slot.key());
                slot.insert(path);
            }
        }
    }
    let mut out = Vec::new();
    for (session, path) in sessions {
        let o = crate::live::observe(roots, reg, &session, path.as_deref(), now);
        for (key, head, detail) in keys_of(&session, &o) {
            out.push((key, session.clone(), head, detail));
        }
    }
    out.sort();
    out
}

/// What one session is waiting on, as notification keys with a headline and a detail —
/// nothing unless its activity is Waiting. The wrapper's prompts when there are any (they
/// name the request an answer must echo); otherwise the call the transcript shows open;
/// otherwise the registry's word alone, keyed by when it started waiting.
pub(crate) fn keys_of(session: &str, o: &Observed) -> Vec<(String, String, String)> {
    if o.activity.kind != Kind::Waiting {
        return Vec::new();
    }
    if !o.prompts.is_empty() {
        return o
            .prompts
            .iter()
            .map(|p| {
                let w = &p.prompt;
                (format!("req:{}", w.request_id), w.headline.clone(), w.summary.clone())
            })
            .collect();
    }
    let Some(on) = o.activity.waiting_on.as_ref() else {
        return Vec::new();
    };
    let summary = clip(on.summary.as_deref().unwrap_or(""), 140);
    if let Some(id) = &on.tool_use_id {
        let head = match on.kind {
            "question" => "Question".to_owned(),
            "plan" => "Plan waiting for approval".to_owned(),
            "dialog" => "Waiting on a dialog".to_owned(),
            _ => format!("{} wants to run", on.tool.as_deref().unwrap_or("A tool")),
        };
        return vec![(format!("ask:{id}"), head, summary)];
    }
    // Only the registry knows, so nothing here can answer it: say where it can be.
    let record = o.record.as_ref();
    let at = record.and_then(|r| r.status_updated_ms).unwrap_or(0);
    let place = match record.and_then(|r| r.entrypoint.as_deref()) {
        Some("cli") => "Waiting in the terminal",
        Some("claude-vscode") => "Waiting in the editor",
        _ => "Waiting where the session runs",
    };
    vec![(format!("ask:wait-{session}-{at}"), o.activity.detail.clone(), place.to_owned())]
}

/// The session ids of the sockets in the wrapper directory.
fn socket_ids(dir: &Path) -> Vec<String> {
    // A pipe has no file, so on Windows the records a wrapper keeps stand for its channel.
    #[cfg(unix)]
    const MARKS: &[&str] = &[".sock"];
    #[cfg(windows)]
    const MARKS: &[&str] = &[".ctl", ".pending"];
    let mut ids: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name();
            let name = name.to_str()?;
            MARKS.iter().find_map(|m| name.strip_suffix(m)).map(str::to_owned)
        })
        .collect();
    ids.sort();
    ids.dedup();
    ids
}

/// Whether a wrapper still holds the session's channel, without asking it anything: its
/// socket file, or on Windows, where a pipe has no file, its pipe name.
fn channel_held(dir: &Path, id: &str) -> bool {
    #[cfg(unix)]
    return dir.join(format!("{id}.sock")).symlink_metadata().is_ok();
    #[cfg(windows)]
    return crate::peer::wrapper_socket_in(dir, id).is_some();
}

/// Ask every wrapper socket no live session holds whether anything listens on it, and
/// clear away what a dead wrapper left: a wrapper killed before it could clean up, or one
/// of the editor's throwaway processes that never wrote a transcript and so is never
/// listed. `peer::wrapper_alive` removes a refused socket and its `.pending`; the control
/// recording and a `.pending.tmp` caught mid-write go here. Here and not there, because
/// `wrapper_alive` also probes open sessions, where a refused connect can be a full
/// backlog and the recording is what `set_model` reads its answer back from. Sockets of
/// open sessions are left alone — everything else probes those often enough. Then the
/// orphans (`sweep_orphans`). Returns how many sockets and orphans were removed.
pub(crate) fn janitor(dir: &Path, reg: &Snapshot) -> usize {
    let mut cleared = 0;
    for id in socket_ids(dir) {
        if reg.by_session.contains_key(&id) {
            continue;
        }
        if !crate::peer::wrapper_alive_in(dir, &id) && !dir.join(format!("{id}.sock")).exists() {
            for ext in ["ctl", "pending.tmp"] {
                let _ = std::fs::remove_file(dir.join(format!("{id}.{ext}")));
            }
            cleared += 1;
        }
    }
    cleared + sweep_orphans(dir, reg, SystemTime::now())
}

/// A dead wrapper's records with no socket beside them: a probe found the socket refused
/// and took only it and the prompts — every router before this one did that, and the
/// per-session probe still does. Nothing reads them again, and this Mac never reaps `/tmp`,
/// so they would stay for good (eight here, up to 342 KB each). Removed once untouched for
/// `ORPHAN_AFTER` and not held by an open session — only `<id>.ctl`, `<id>.pending` and
/// `<id>.pending.tmp`, with an id a session could have, and only plain files. Returns how
/// many.
fn sweep_orphans(dir: &Path, reg: &Snapshot, now: SystemTime) -> usize {
    let mut removed = 0;
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let name = entry.file_name();
        let Some(id) = name.to_str().and_then(|n| {
            [".pending.tmp", ".pending", ".ctl"]
                .iter()
                .find_map(|ext| n.strip_suffix(ext))
        }) else {
            continue;
        };
        let is_id = !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
        if !is_id
            || !entry.file_type().is_ok_and(|t| t.is_file())
            || channel_held(dir, id)
            || reg.by_session.contains_key(id)
        {
            continue;
        }
        let untouched = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|at| now.duration_since(at).ok())
            .is_some_and(|age| age > ORPHAN_AFTER);
        if untouched && std::fs::remove_file(entry.path()).is_ok() {
            removed += 1;
        }
    }
    removed
}

/// Whether a prompt that has left the sweep is nonetheless still waiting for someone.
///
/// With the registry, the sweep sees every open session, so this is the session's own
/// answer: still open, and still waiting on this very key. A closed session waits on
/// nothing — whoever could have answered it is gone — so its banners come down.
///
/// Without the registry, a `req:` key that is gone really is answered (the wrapper's
/// record is authoritative, and dead wrappers are swept too). An `ask:` key is read off a
/// transcript that is only swept while the session is moving, and a session waiting on a
/// question does not move; that one has to be asked about directly, and quiet is never
/// taken for an answer.
pub(crate) fn still_waiting(key: &str, session: &str) -> bool {
    let reg = crate::registry::snapshot();
    still_waiting_in(&Roots::system(), &reg, key, session, crate::db::now_ms())
}

fn still_waiting_in(roots: &Roots, reg: &Snapshot, key: &str, session: &str, now: i64) -> bool {
    if reg.available {
        if !reg.by_session.contains_key(session) {
            return false;
        }
        let path = crate::live::transcript_path_in(&roots.projects, session);
        let o = crate::live::observe(roots, reg, session, path.as_deref(), now);
        return keys_of(session, &o).iter().any(|(k, ..)| k == key);
    }
    match key.strip_prefix("ask:") {
        Some(ask_id) => crate::live::still_asking_in(&roots.projects, session, ask_id),
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
    use axum::http::StatusCode;
    use axum::response::{IntoResponse, Response};
    use axum::Json;
    use serde::Deserialize;
    use serde_json::{json, Value};

    use crate::proxy::AppState;
    use crate::refusal::{failed, refuse, JsonBody};

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

    /// Refusals answer with an error status (see `refusal`): until they did, the dashboard
    /// read a key that does not sign as "saved".
    pub async fn config(State(state): State<Arc<AppState>>, JsonBody(req): JsonBody<ConfigBody>) -> Response {
        if req.platform.as_deref() == Some("fcm") {
            return fcm_config(state, req);
        }
        if req.clear {
            return match crate::accounts::store_push_key(None) {
                Ok(()) => { state.push.apns_forget(); Json(json!({ "ok": true, "configured": false })).into_response() }
                // The secrets file would not take the write: the router failing.
                Err(e) => failed(e),
            };
        }
        let key = crate::apns::Key { team_id: req.team_id.trim().to_string(), key_id: req.key_id.trim().to_string(), p8: req.p8.trim().to_string() };
        if key.team_id.is_empty() || key.key_id.is_empty() || key.p8.is_empty() {
            // 400: part of the key is missing from the request.
            return refuse(StatusCode::BAD_REQUEST, "team id, key id and the .p8 key are all needed");
        }
        // Prove the key signs before keeping it. 422: everything a key needs was sent, but
        // this one does not work — a different key is the fix, not a different request.
        if let Err(e) = crate::apns::mint_jwt(&key, crate::db::now_ms() / 1000) {
            return refuse(StatusCode::UNPROCESSABLE_ENTITY, format!("that key does not sign: {e}"));
        }
        let stored = crate::accounts::PushKey {
            team_id: key.team_id, key_id: key.key_id, p8: key.p8,
            bundle_id: req.bundle_id.map(|b| b.trim().to_string()).filter(|b| !b.is_empty()),
        };
        match crate::accounts::store_push_key(Some(stored)) {
            Ok(()) => { state.push.apns_forget(); Json(json!({ "ok": true, "configured": true })).into_response() }
            Err(e) => failed(e),
        }
    }

    /// The Firebase half of `/api/push/config`. Takes the console's file whole rather
    /// than four fields: it is what the user actually has in hand, it cannot drift
    /// between fields, and one parse gives one precise complaint instead of four. Its
    /// statuses follow the Apple half's.
    fn fcm_config(state: Arc<AppState>, req: ConfigBody) -> Response {
        if req.clear {
            return match crate::accounts::store_fcm_key(None) {
                Ok(()) => {
                    state.push.fcm_forget();
                    Json(json!({ "ok": true, "configured": false })).into_response()
                }
                Err(e) => failed(e),
            };
        }
        let text = req.service_account.unwrap_or_default();
        let text = text.trim();
        if text.is_empty() {
            return refuse(StatusCode::BAD_REQUEST, "paste the service-account JSON the Firebase console downloads");
        }
        // 400: what was pasted is not the file the console hands out, so a key in it was
        // never reached — unlike the refusal below, where it was, and did not work.
        let key = match crate::fcm::parse_service_account(text) {
            Ok(k) => k,
            Err(e) => return refuse(StatusCode::BAD_REQUEST, format!("that is not a service account: {e}")),
        };
        // Prove the key signs before keeping it — the same gate the `.p8` gets.
        if let Err(e) = crate::fcm::mint_jwt(&key, crate::db::now_ms() / 1000) {
            return refuse(StatusCode::UNPROCESSABLE_ENTITY, format!("that key does not sign: {e}"));
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
                Json(json!({ "ok": true, "configured": true, "project_id": key.project_id })).into_response()
            }
            Err(e) => failed(e),
        }
    }

    /// Refusals answer with an error status (see `refusal`). Only the phones call this and
    /// `unregister`, and both read a refusal's words from one; a 200 carrying `error` was
    /// the one shape left that they had to look inside a success for.
    pub async fn register(State(state): State<Arc<AppState>>, JsonBody(req): JsonBody<RegisterBody>) -> Response {
        let token = req.token.trim();
        let Some(platform) = super::platform_of(token, req.platform.as_deref()) else {
            // 400: no push service would take it, whatever the router holds.
            return refuse(StatusCode::BAD_REQUEST, "that is not a device token");
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
                    .into_response()
            }
            // The store would not take the row: the router failing, not a refusal.
            Err(e) => failed(e),
        }
    }

    pub async fn unregister(State(state): State<Arc<AppState>>, JsonBody(req): JsonBody<TokenBody>) -> Response {
        let token = req.token.trim();
        if token.is_empty() {
            // 400: no token names no phone, so there is nothing it could forget.
            return refuse(StatusCode::BAD_REQUEST, "no device token supplied");
        }
        match state.push.unregister(token) {
            Ok(()) => Json(json!({ "ok": true })).into_response(),
            Err(e) => failed(e),
        }
    }

    /// What is waiting for a person right now, keyed as the notifications are. A phone
    /// compares this against the banners it has up and clears the rest — which is what
    /// makes a dropped background push harmless.
    ///
    /// A failure is an error status, never an empty list: a phone takes an empty list
    /// literally (Android defaults a missing `waiting` to one) and would clear every
    /// banner, where an error makes it keep them.
    pub async fn pending(State(state): State<Arc<AppState>>) -> axum::response::Response {
        use axum::response::IntoResponse;
        let push = state.push.clone();
        match tokio::task::spawn_blocking(move || push.waiting_keys()).await {
            Ok(waiting) => Json(json!({ "waiting": waiting })).into_response(),
            Err(e) => (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": e.to_string() })),
            )
                .into_response(),
        }
    }

    /// A notification that says nothing but "it works", to every phone.
    #[derive(Deserialize, Default)]
    pub struct TestBody {
        /// A session for the test to name, so its tap can be checked to land there.
        #[serde(default)]
        pub session: Option<String>,
    }

    pub async fn test(State(state): State<Arc<AppState>>, body: Option<JsonBody<TestBody>>) -> Response {
        if !state.push.configured() {
            // 409: asking is fine, but nothing is set to sign a push with yet, and setting
            // a key is the fix.
            return refuse(StatusCode::CONFLICT, "no push credentials are set");
        }
        let session = body.and_then(|JsonBody(b)| b.session).filter(|s| !s.trim().is_empty());
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
        // A 200 whatever each phone's service said: the test was run, and what came back
        // for each phone — delivered, or Apple's or Google's reason — is the answer.
        Json(json!({
            "ok": results.iter().any(|(_, r)| r.is_ok()),
            "results": results.into_iter().map(|(tail, r)| json!({ "device": tail, "ok": r.is_ok(), "error": r.err() })).collect::<Vec<_>>(),
        }))
        .into_response()
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
    use crate::registry::RegStatus;

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

    fn parse_pending(text: &str) -> Vec<Prompt> {
        text.lines()
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .filter_map(|v| prompt_of(&v))
            .collect()
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
        use crate::live::testkit::*;
        let lab = Lab::new("banner");
        let now = crate::db::now_ms();
        // Sitting on a question for an hour, no wrapper, the transcript long quiet.
        lab.write("s-q", &[
            typed(now - 3_700_000, "pick one"),
            call(now - 3_600_000, "toolu_q", "AskUserQuestion", json!({"questions":[{"question":"Which?"}]})),
        ]);
        let none = no_registry();
        // Without a registry: a wrapper's prompt is answered the moment it leaves its
        // file, and a question is asked of the session itself — quiet is not an answer.
        assert!(!still_waiting_in(&lab.roots, &none, "req:r-1", "s-q", now));
        assert!(still_waiting_in(&lab.roots, &none, "ask:toolu_q", "s-q", now));
        assert!(!still_waiting_in(&lab.roots, &none, "ask:toolu_other", "s-q", now));
        assert!(!still_waiting_in(&lab.roots, &none, "ask:toolu_1", "no-such-session", now));
        assert!(!still_waiting_in(&lab.roots, &none, "ask:toolu_1", "../etc/passwd", now), "an id that is not one reads as gone");

        // With one: open and waiting on this very key...
        let open = registry(vec![record("s-q", Some(RegStatus::Waiting), Some(now - 3_600_000), now - 4_000_000)], now);
        assert!(still_waiting_in(&lab.roots, &open, "ask:toolu_q", "s-q", now));
        assert!(!still_waiting_in(&lab.roots, &open, "ask:toolu_other", "s-q", now));
        // ...and closed is over: nobody is left to answer it, so its banner comes down.
        let closed = registry(vec![], now);
        assert!(!still_waiting_in(&lab.roots, &closed, "ask:toolu_q", "s-q", now));
    }

    #[test]
    #[cfg(unix)]
    fn each_kind_of_wait_gets_a_key_the_phones_can_dismiss() {
        use crate::live::testkit::*;
        let lab = Lab::new("keys");
        let now = crate::db::now_ms();
        let started = now - 600_000;

        // Only the registry knows: an unwrapped terminal session at a permission prompt.
        lab.write("s-reg", &[typed(now - 60_000, "hi"), reply(now - 59_000, "hello")]);
        let mut r = record("s-reg", Some(RegStatus::Waiting), Some(now - 5_000), started);
        r.waiting_for = Some("permission prompt".into());
        r.entrypoint = Some("cli".into());
        // The transcript shows the question: an unwrapped editor session.
        lab.write("s-ask", &[
            typed(now - 30_000, "choose"),
            call(now - 29_000, "toolu_a", "AskUserQuestion", json!({"questions":[{"question":"Which delivery?"}]})),
        ]);
        let q = record("s-ask", Some(RegStatus::Waiting), Some(now - 29_000), started);
        // The wrapper has the requests: two at once.
        lab.write("s-wr", &[typed(now - 10_000, "go"), call(now - 9_000, "toolu_b", "Bash", json!({"command":"make"}))]);
        let _l = std::os::unix::net::UnixListener::bind(lab.roots.wrapper.join("s-wr.sock")).unwrap();
        std::fs::write(lab.roots.wrapper.join("s-wr.pending"), format!("{}\n{}\n",
            pending_line("r-2", "Bash", "toolu_b", json!({"command":"make"})),
            pending_line("r-3", "Edit", "toolu_c", json!({"file_path":"/w/main.rs"})))).unwrap();
        let w = record("s-wr", Some(RegStatus::Waiting), Some(now - 9_000), started);
        // Busy is not waiting.
        lab.write("s-busy", &[typed(now - 5_000, "work")]);
        let b = record("s-busy", Some(RegStatus::Busy), Some(now - 5_050), started);
        let reg = registry(vec![r, q, w, b], now);

        let all = waiting_in(&lab.roots, &reg, now);
        let keys: Vec<(&str, &str, &str, &str)> =
            all.iter().map(|(k, s, h, d)| (k.as_str(), s.as_str(), h.as_str(), d.as_str())).collect();
        // Keyed by when the registry started waiting: a new wait is a new banner.
        let registry_only = format!("ask:wait-s-reg-{}", now - 5_000);
        assert_eq!(keys, vec![
            ("ask:toolu_a", "s-ask", "Question", "Which delivery?"),
            (registry_only.as_str(), "s-reg", "Waiting for permission", "Waiting in the terminal"),
            ("req:r-2", "s-wr", "Bash wants to run", "make"),
            ("req:r-3", "s-wr", "Edit wants to change", "main.rs"),
        ]);
        for (key, ..) in &all {
            assert!(key.starts_with("req:") || key.starts_with("ask:"), "the phones dismiss only these: {key}");
        }
    }

    #[test]
    fn a_wait_is_announced_only_after_two_seconds_without_a_break() {
        let mut since = HashMap::new();
        let still = |keys: &[&'static str]| keys.iter().copied().collect::<HashSet<&str>>();
        assert!(waited(&mut since, &still(&["req:a"]), 10_000).is_empty(), "just seen");
        assert!(waited(&mut since, &still(&["req:a"]), 11_000).is_empty());
        assert_eq!(waited(&mut since, &still(&["req:a"]), 12_000), HashSet::from(["req:a".to_owned()]));
        // Resolved by auto mode in a second and raised again: a new wait, a new clock.
        assert!(waited(&mut since, &still(&[]), 13_000).is_empty());
        assert!(waited(&mut since, &still(&["req:a"]), 14_000).is_empty());
        assert!(since.len() == 1 && since["req:a"] == 14_000);
    }

    #[test]
    fn a_prompt_missing_from_one_sweep_keeps_its_banner_until_the_next() {
        let mut announced = HashMap::from([
            ("req:r-1".to_owned(), "s-1".to_owned()),
            ("ask:toolu_1".to_owned(), "s-2".to_owned()),
            ("ask:toolu_2".to_owned(), "s-3".to_owned()),
        ]);
        let mut missing = HashMap::new();
        let none: HashSet<&str> = HashSet::new();
        let elsewhere = HashSet::from(["ask:toolu_2".to_owned()]);
        // First sweep without them: the question is over, the request gets one more.
        assert_eq!(settle(&mut announced, &mut missing, &none, &elsewhere), ["ask:toolu_1"]);
        assert!(announced.contains_key("req:r-1") && announced.contains_key("ask:toolu_2"));
        // Back in the next sweep (a torn read of an old wrapper's file): the count resets.
        let back: HashSet<&str> = HashSet::from(["req:r-1"]);
        assert!(settle(&mut announced, &mut missing, &back, &elsewhere).is_empty());
        assert!(settle(&mut announced, &mut missing, &none, &elsewhere).is_empty());
        // Missing from two in a row: answered.
        assert_eq!(settle(&mut announced, &mut missing, &none, &elsewhere), ["req:r-1"]);
        assert!(missing.is_empty());
        assert_eq!(announced.keys().collect::<Vec<_>>(), ["ask:toolu_2"], "still waiting where the sweep cannot see");
    }

    #[test]
    #[cfg(unix)]
    fn dead_wrapper_sockets_are_cleared_but_open_sessions_are_left_alone() {
        use crate::live::testkit::*;
        let lab = Lab::new("janitor");
        let dir = &lab.roots.wrapper;
        let now = crate::db::now_ms();
        // Dead: the file of a wrapper that was killed, with its unanswerable prompts.
        drop(std::os::unix::net::UnixListener::bind(dir.join("dead.sock")).unwrap());
        std::fs::write(dir.join("dead.pending"), "{}\n").unwrap();
        // Alive, and nothing holds its session in the registry: asked, and kept.
        let _live = std::os::unix::net::UnixListener::bind(dir.join("live.sock")).unwrap();
        // Dead too, but an open session's: not this sweep's business.
        drop(std::os::unix::net::UnixListener::bind(dir.join("open.sock")).unwrap());
        std::fs::write(dir.join("open.pending.tmp"), "x").unwrap();
        let reg = registry(vec![record("open", Some(RegStatus::Idle), None, now)], now);

        assert_eq!(janitor(dir, &reg), 1);
        assert!(!dir.join("dead.sock").exists() && !dir.join("dead.pending").exists());
        assert!(dir.join("live.sock").exists());
        assert!(dir.join("open.sock").exists() && dir.join("open.pending.tmp").exists());
    }

    #[test]
    #[cfg(unix)]
    fn what_a_dead_wrapper_leaves_goes_with_it_and_orphans_go_once_ten_minutes_old() {
        use crate::live::testkit::*;
        let lab = Lab::new("orphans");
        let dir = &lab.roots.wrapper;
        let now = crate::db::now_ms();
        let aged = |name: &str| {
            let p = dir.join(name);
            std::fs::write(&p, "{}\n").unwrap();
            let t = std::time::SystemTime::now() - Duration::from_secs(11 * 60);
            std::fs::File::options().write(true).open(&p).unwrap().set_modified(t).unwrap();
        };
        // A wrapper killed mid-write: its socket, its prompts, its control recording and
        // the temp file of the write it died in all go with it, however recent.
        drop(std::os::unix::net::UnixListener::bind(dir.join("dead.sock")).unwrap());
        for ext in ["pending", "ctl", "pending.tmp"] {
            std::fs::write(dir.join(format!("dead.{ext}")), "{}\n").unwrap();
        }
        // Records whose socket went long ago (removed by an older router's probe, or by a
        // wrapper that never cleaned up the rest): gone once ten minutes old...
        for ext in ["ctl", "pending", "pending.tmp"] {
            aged(&format!("0fa9a5c6-gone.{ext}"));
        }
        // ...and not before: a wrapper may be between binding and writing.
        std::fs::write(dir.join("fresh.ctl"), "{}\n").unwrap();
        // An open session's are never touched, socket or not; nor a live wrapper's.
        aged("open.ctl");
        let _live = std::os::unix::net::UnixListener::bind(dir.join("live.sock")).unwrap();
        aged("live.ctl");
        // Nothing that is not a session's record.
        aged("not an id.ctl");
        aged("notes.txt");
        std::fs::create_dir(dir.join("sub.ctl")).unwrap();
        let reg = registry(vec![record("open", Some(RegStatus::Idle), None, now)], now);

        janitor(dir, &reg);
        for ext in ["sock", "pending", "ctl", "pending.tmp"] {
            assert!(!dir.join(format!("dead.{ext}")).exists(), "dead.{ext}");
        }
        for ext in ["ctl", "pending", "pending.tmp"] {
            assert!(!dir.join(format!("0fa9a5c6-gone.{ext}")).exists(), "0fa9a5c6-gone.{ext}");
        }
        for kept in ["fresh.ctl", "open.ctl", "live.sock", "live.ctl", "not an id.ctl", "notes.txt", "sub.ctl"] {
            assert!(dir.join(kept).exists(), "{kept} was removed");
        }
        // Eleven minutes on, the fresh one has become a leftover too — and only it.
        let later = std::time::SystemTime::now() + Duration::from_secs(11 * 60);
        assert_eq!(sweep_orphans(dir, &reg, later), 1);
        assert!(!dir.join("fresh.ctl").exists());
        for kept in ["open.ctl", "live.sock", "live.ctl", "not an id.ctl", "notes.txt", "sub.ctl"] {
            assert!(dir.join(kept).exists(), "{kept} was removed");
        }
    }

    #[test]
    #[cfg(unix)]
    fn without_a_registry_wrapped_sessions_are_still_swept_however_quiet() {
        use crate::live::testkit::*;
        let lab = Lab::new("noreg");
        let now = crate::db::now_ms();
        // A wrapped session blocked on a prompt for an hour: its transcript is not recent.
        let p = lab.write("s-old", &[typed(now - 3_700_000, "go"), call(now - 3_600_000, "toolu_o", "Bash", json!({"command":"deploy"}))]);
        std::fs::File::options().write(true).open(&p).unwrap()
            .set_modified(std::time::UNIX_EPOCH + Duration::from_millis((now - 3_600_000) as u64)).unwrap();
        let _l = std::os::unix::net::UnixListener::bind(lab.roots.wrapper.join("s-old.sock")).unwrap();
        std::fs::write(lab.roots.wrapper.join("s-old.pending"),
            pending_line("r-old", "Bash", "toolu_o", json!({"command":"deploy"})) + "\n").unwrap();
        let keys: Vec<String> = waiting_in(&lab.roots, &no_registry(), now).into_iter().map(|(k, ..)| k).collect();
        assert_eq!(keys, ["req:r-old"]);
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
