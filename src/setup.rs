//! Self-install: register the service, and point Claude Code at the router.
//!
//! The rule this module exists to honour: **only ever touch our own keys.** A user's
//! settings file is theirs. We add exactly the entries we need — the router's address,
//! and for a hosted router the password it asks for — remember precisely what was
//! there before, and on removal put that back, but only if the value is still the one
//! we wrote. If the user has since changed it, we leave it alone and say so. Nothing
//! else in the file is rewritten, reordered or reformatted.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Where Claude Code sends its requests: the key every install manages.
const ENV_KEY: &str = "ANTHROPIC_BASE_URL";
/// What a Claude Code pointed at a hosted router presents: its password. Managed only by
/// `agent install`, and never recorded in the clear (see `fingerprint`).
pub const TOKEN_KEY: &str = "ANTHROPIC_AUTH_TOKEN";

fn base_url_key() -> String {
    ENV_KEY.to_string()
}

/// What `managed.json` keeps of a value we wrote: the value itself, except a password,
/// which is kept as a fingerprint — enough to tell it from a later edit, and nothing
/// worth stealing from a file that lists what was changed.
fn fingerprint(key: &str, value: &str) -> String {
    if key == TOKEN_KEY {
        use sha2::Digest;
        let digest = sha2::Sha256::digest(value.as_bytes());
        format!("sha256:{}", digest.iter().map(|b| format!("{b:02x}")).collect::<String>())
    } else {
        value.to_string()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Managed {
    /// Absolute path of the settings file we edited.
    pub settings_path: String,
    pub scope: String,
    /// What we wrote, so we can tell our value from a later user edit.
    pub value: String,
    /// The value that was there before us; `None` means the key was absent.
    pub previous: Option<String>,
    pub applied_at: i64,
    /// A full copy of the file as it was, for a manual restore.
    pub backup_path: Option<String>,
    /// Which `env` key. Records from before there was more than one are the address.
    #[serde(default = "base_url_key")]
    pub key: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct State {
    #[serde(default)]
    pub managed: Vec<Managed>,
}

pub fn state_path() -> Result<PathBuf> {
    let dir = data_dir()?;
    Ok(dir.join("managed.json"))
}

pub fn data_dir() -> Result<PathBuf> {
    crate::config::data_dir()
}

/// `$HOME` on Unix, `%USERPROFILE%` on Windows — first there, because Git Bash and MSYS
/// set a `HOME` of their own, and the editor extension and Claude Code use the profile.
pub fn home_dir() -> Result<PathBuf> {
    #[cfg(windows)]
    if let Ok(h) = std::env::var("USERPROFILE") {
        if !h.is_empty() {
            return Ok(PathBuf::from(h));
        }
    }
    if let Ok(h) = std::env::var("HOME") {
        if !h.is_empty() {
            return Ok(PathBuf::from(h));
        }
    }
    if let Ok(h) = std::env::var("USERPROFILE") {
        if !h.is_empty() {
            return Ok(PathBuf::from(h));
        }
    }
    Err(anyhow!("neither HOME nor USERPROFILE is set"))
}

pub fn load_state() -> State {
    state_path()
        .ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

pub fn save_state(state: &State) -> Result<()> {
    let path = state_path()?;
    write_atomic(&path, &serde_json::to_string_pretty(state)?)?;
    // A value it must restore can be a password someone had there before us.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

/// Global settings live at `~/.claude/settings.json`; a project's machine-local
/// settings at `<project>/.claude/settings.local.json`, which is gitignored.
pub fn settings_path(project: Option<&Path>) -> Result<PathBuf> {
    Ok(match project {
        Some(dir) => dir.join(".claude").join("settings.local.json"),
        None => home_dir()?.join(".claude").join("settings.json"),
    })
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// How many times a busy program is tried again, 20 ms apart: a second in all.
const BUSY_TRIES: u32 = 50;

/// Start a program, trying again while it is busy. On Linux a program cannot be started while
/// any process holds it open for writing, and a fork made by another thread between a write
/// and its close holds it until that child execs (AGEN-0122). Any other failure is said at
/// once.
pub(crate) fn spawn_when_free(cmd: &mut std::process::Command) -> std::io::Result<std::process::Child> {
    when_free(|| cmd.spawn())
}

fn when_free<T>(mut start: impl FnMut() -> std::io::Result<T>) -> std::io::Result<T> {
    let mut tries = 0;
    loop {
        match start() {
            Err(e) if e.kind() == std::io::ErrorKind::ExecutableFileBusy && tries < BUSY_TRIES => {
                tries += 1;
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            done => return done,
        }
    }
}

pub(crate) fn write_atomic(path: &Path, text: impl AsRef<[u8]>) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Read a settings file, preserving key order. A missing file is an empty object; a
/// malformed one is an error, because silently replacing a file we cannot parse
/// would destroy settings we were asked to preserve.
fn read_settings(path: &Path) -> Result<Map<String, Value>> {
    match std::fs::read_to_string(path) {
        Ok(text) if text.trim().is_empty() => Ok(Map::new()),
        Ok(text) => match serde_json::from_str::<Value>(&text) {
            Ok(Value::Object(m)) => Ok(m),
            Ok(_) => Err(anyhow!("{} is not a JSON object", path.display())),
            Err(e) => Err(anyhow!("{} is not valid JSON ({e}) — fix or move it first", path.display())),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Map::new()),
        Err(e) => Err(e).context(format!("reading {}", path.display())),
    }
}

pub enum Applied {
    Added,
    Replaced { previous: String },
    /// Already pointed at us, but untracked — now recorded so it can be undone.
    Adopted,
    AlreadySet,
}

/// Point a settings file at the router, recording what was there before.
pub fn apply(path: &Path, url: &str, scope: &str) -> Result<Applied> {
    apply_key(path, ENV_KEY, url, scope)
}

/// Set one of our `env` keys in a settings file, recording what was there before.
pub fn apply_key(path: &Path, key: &str, value: &str, scope: &str) -> Result<Applied> {
    let mut settings = read_settings(path)?;
    let file = path.to_string_lossy().into_owned();

    let previous = settings
        .get("env")
        .and_then(Value::as_object)
        .and_then(|e| e.get(key))
        .and_then(Value::as_str)
        .map(str::to_owned);

    let already = previous.as_deref() == Some(value);
    let state = load_state();
    if already && state.managed.iter().any(|m| m.settings_path == file && m.key == key) {
        return Ok(Applied::AlreadySet);
    }
    let backed_up = state.managed.iter().any(|m| m.settings_path == file);

    // Back up the whole file once, before the first change we make to it.
    let backup_path = if path.exists() && !already && !backed_up {
        let backup = path.with_extension(format!("json.before-agent-router.{}", now_ms()));
        std::fs::copy(path, &backup)?;
        Some(backup.to_string_lossy().into_owned())
    } else {
        None
    };

    // Touch nothing but our key: `env` keeps every other variable, and every
    // sibling key keeps its position because the map preserves insertion order.
    let env = settings
        .entry("env".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    let Some(env) = env.as_object_mut() else {
        return Err(anyhow!("\"env\" in {} is not an object", path.display()));
    };
    env.insert(key.to_string(), Value::String(value.to_string()));

    if !already {
        write_atomic(path, &(serde_json::to_string_pretty(&Value::Object(settings))? + "\n"))?;
    }

    let mut state = load_state();
    state.managed.retain(|m| !(m.settings_path == file && m.key == key));
    state.managed.push(Managed {
        settings_path: file,
        scope: scope.to_string(),
        value: fingerprint(key, value),
        // When adopting, there is no knowable prior value: the key already held ours.
        previous: if already { None } else { previous.clone() },
        applied_at: now_ms(),
        backup_path,
        key: key.to_string(),
    });
    save_state(&state)?;

    Ok(match (already, previous) {
        // The value was already ours but nothing was tracking it — a hand-edited
        // file. Adopt it so uninstall can undo it.
        (true, _) => Applied::Adopted,
        (false, Some(previous)) => Applied::Replaced { previous },
        (false, None) => Applied::Added,
    })
}

/// The router a settings file points Claude Code at, if it names one.
pub fn base_url_in(path: &Path) -> Result<Option<String>> {
    Ok(read_settings(path)?
        .get("env")
        .and_then(|e| e.get(ENV_KEY))
        .and_then(Value::as_str)
        .map(str::to_owned))
}

/// What marks a row of Claude Code's `/model` picker as the router's: the end of its
/// description. Every other row is the owner's, and left exactly as it is.
const PICKER_MARK: &str = "via Agent Router";

/// One model to offer in Claude Code's `/model` picker.
#[derive(Debug, Clone, PartialEq)]
pub struct PickerModel {
    pub id: String,
    pub label: String,
    /// The provider's name, as the row's description says it.
    pub provider: String,
}

/// The models of a catalogue, as `GET /api/models` answers it, that Claude Code's picker
/// would never list by itself: every one that is not Claude's. Its gateway discovery keeps
/// only ids containing `claude` (docs/sources-and-models.md, "Clients"), so these are
/// offered as rows of `modelPicker.options` instead.
pub fn picker_models(catalogue: &[Value]) -> Vec<PickerModel> {
    catalogue
        .iter()
        .filter_map(|m| {
            let id = m.get("id")?.as_str()?;
            let provider = crate::accounts::Provider::parse(m.get("provider")?.as_str()?).ok()?;
            (provider != crate::accounts::Provider::Anthropic).then(|| PickerModel {
                id: id.to_string(),
                label: m.get("display_name").and_then(Value::as_str).unwrap_or(id).to_string(),
                provider: provider.name().to_string(),
            })
        })
        .collect()
}

fn ours(row: &Value) -> bool {
    row.get("description").and_then(Value::as_str).is_some_and(|d| d.ends_with(PICKER_MARK))
}

/// Keep the router's rows of `modelPicker.options` in the settings file at `path` in step
/// with `models`: a row for each, after the owner's rows, and none for a model no longer
/// given. A row the router did not write is never touched, nor is a model the owner already
/// has a row for given a second, and `replaceBuiltInOptions` is never set. Claude Code reads
/// `modelPicker` from the user's settings only. Says whether the file changed.
pub fn sync_picker_rows(path: &Path, models: &[PickerModel]) -> Result<bool> {
    let mut settings = read_settings(path)?;
    let before = settings.get("modelPicker").cloned();
    let options: Vec<Value> = before
        .as_ref()
        .and_then(|p| p.get("options"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut rows: Vec<Value> = options.into_iter().filter(|r| !ours(r)).collect();
    let theirs: Vec<String> =
        rows.iter().filter_map(|r| r.get("model").and_then(Value::as_str).map(str::to_owned)).collect();
    for m in models.iter().filter(|m| !theirs.contains(&m.id)) {
        rows.push(serde_json::json!({
            "model": m.id,
            "label": m.label,
            "description": format!("{} · {PICKER_MARK}", m.provider),
        }));
    }
    let after = match before.clone() {
        // Nothing of the router's to write, and no picker of the owner's to keep.
        None if rows.is_empty() => None,
        None => Some(serde_json::json!({ "options": rows })),
        Some(Value::Object(mut picker)) => {
            picker.insert("options".into(), Value::Array(rows));
            Some(Value::Object(picker))
        }
        Some(_) => return Err(anyhow!("\"modelPicker\" in {} is not an object", path.display())),
    };
    if after == before {
        return Ok(false);
    }
    match after {
        Some(picker) => {
            settings.insert("modelPicker".into(), picker);
        }
        None => {
            settings.remove("modelPicker");
        }
    }
    write_atomic(path, &(serde_json::to_string_pretty(&Value::Object(settings))? + "\n"))?;
    Ok(true)
}

pub enum Reverted {
    Restored { to: String },
    Removed,
    NotOurs { found: String },
    NotManaged,
}

/// Undo what `apply` and `apply_key` did to one file — and nothing else. Says what
/// became of the address; a password's fate is never echoed.
pub fn revert(path: &Path) -> Result<Reverted> {
    let file = path.to_string_lossy().into_owned();
    let keys: Vec<String> = load_state()
        .managed
        .iter()
        .filter(|m| m.settings_path == file)
        .map(|m| m.key.clone())
        .collect();
    let mut address = Reverted::NotManaged;
    for key in keys {
        let outcome = revert_key(path, &key)?;
        if key == ENV_KEY {
            address = outcome;
        }
    }
    Ok(address)
}

fn revert_key(path: &Path, key: &str) -> Result<Reverted> {
    let mut state = load_state();
    let file = path.to_string_lossy().into_owned();
    let Some(idx) = state.managed.iter().position(|m| m.settings_path == file && m.key == key) else {
        return Ok(Reverted::NotManaged);
    };
    let managed = state.managed[idx].clone();

    let mut settings = read_settings(path)?;
    let current = settings
        .get("env")
        .and_then(Value::as_object)
        .and_then(|e| e.get(key))
        .and_then(Value::as_str)
        .map(str::to_owned);

    // If it is no longer the value we wrote, the user changed it deliberately.
    // Reverting would undo their edit, so stop and leave the file alone.
    match current.as_deref() {
        Some(found) if fingerprint(key, found) != managed.value => {
            let found = if key == TOKEN_KEY { "a different password".to_string() } else { found.to_string() };
            return Ok(Reverted::NotOurs { found })
        }
        None => {
            state.managed.remove(idx);
            save_state(&state)?;
            return Ok(Reverted::Removed);
        }
        _ => {}
    }

    let outcome = {
        let env = settings
            .get_mut("env")
            .and_then(Value::as_object_mut)
            .ok_or_else(|| anyhow!("\"env\" vanished from {}", path.display()))?;
        match &managed.previous {
            Some(previous) => {
                env.insert(key.to_string(), Value::String(previous.clone()));
                Reverted::Restored { to: previous.clone() }
            }
            None => {
                env.remove(key);
                // Leave no debris: an `env` we created and then emptied goes too.
                if env.is_empty() {
                    settings.remove("env");
                }
                Reverted::Removed
            }
        }
    };

    write_atomic(path, &(serde_json::to_string_pretty(&Value::Object(settings))? + "\n"))?;
    state.managed.remove(idx);
    save_state(&state)?;
    Ok(outcome)
}

/// Where the installed binary lives: what the login items run, and what a machine agent's
/// keeper starts its worker from (`agent::keep`).
pub fn installed_binary() -> Result<PathBuf> {
    Ok(data_dir()?.join("bin").join(if cfg!(windows) { "agent-router.exe" } else { "agent-router" }))
}

/// Copy the running binary somewhere stable, so rebuilding the repo never swaps the
/// file out from under the installed service.
pub fn install_binary() -> Result<PathBuf> {
    let dest = installed_binary()?;
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let current = std::env::current_exe()?;
    if current != dest {
        let tmp = dest.with_extension("new");
        std::fs::copy(&current, &tmp)?;
        replace(&tmp, &dest)?;
    }
    Ok(dest)
}

/// Rename `fresh` over `to`. Windows will not let a running program's file be replaced,
/// but it will let it be renamed: the running copy moves aside, under the name the
/// extension gives it (`vscode/src/router.ts`, `replace`), and goes at a later install
/// once nothing runs it. Elsewhere this is a plain rename.
pub(crate) fn replace(fresh: &Path, to: &Path) -> Result<()> {
    if let (true, Some(dir)) = (cfg!(windows), to.parent()) {
        sweep_aside(dir);
    }
    let Err(e) = std::fs::rename(fresh, to) else {
        return Ok(());
    };
    if !cfg!(windows) {
        let _ = std::fs::remove_file(fresh);
        return Err(e.into());
    }
    let aside = PathBuf::from(format!("{}.old-{}-{}", to.display(), std::process::id(), crate::db::now_ms()));
    let moved = std::fs::rename(to, &aside).is_ok();
    if let Err(again) = std::fs::rename(fresh, to) {
        // Never leave the place empty: put back the copy that was there.
        if moved {
            let _ = std::fs::rename(&aside, to);
        }
        let _ = std::fs::remove_file(fresh);
        return Err(again).with_context(|| format!("replacing {}", to.display()));
    }
    Ok(())
}

/// The copies `replace` moved aside, removed once nothing runs them any more.
fn sweep_aside(dir: &Path) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let name = entry.file_name();
        let aside = name.to_str().and_then(|n| n.rsplit_once(".old-")).is_some_and(|(_, tail)| {
            tail.split_once('-').is_some_and(|(pid, ms)| {
                [pid, ms].iter().all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
            })
        });
        if aside {
            // Still running is still locked, and stays for the next time.
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

// ---------------------------------------------------------------------------
// Web API — the same operations the subcommands perform, so setup can be done
// from the dashboard the binary is already serving.
// ---------------------------------------------------------------------------

pub mod api {
    use std::sync::Arc;

    use axum::extract::State;
    use axum::http::StatusCode;
    use axum::response::{IntoResponse, Response};
    use axum::Json;
    use serde::Deserialize;
    use serde_json::{json, Value};

    use super::*;
    use crate::proxy::AppState;
    use crate::refusal::{failed, refuse, JsonBody};

    fn snapshot(state: &AppState) -> Value {
        // The address entries only: a password `agent install` manages is no business of
        // a page that lists where Claude Code is pointed.
        let managed = load_state()
            .managed
            .into_iter()
            .filter(|m| m.key == ENV_KEY)
            .map(|m| {
                json!({
                    "path": m.settings_path,
                    "scope": m.scope,
                    "value": m.value,
                    "previous": m.previous,
                    "backup_path": m.backup_path,
                    "applied_at": m.applied_at,
                })
            })
            .collect::<Vec<_>>();
        json!({
            "service": {
                "registered": service::is_registered(),
                "supported": cfg!(any(target_os = "macos", target_os = "linux")),
                "platform": std::env::consts::OS,
            },
            "url": format!("http://127.0.0.1:{}", state.config.bind.port()),
            "global_settings_path": settings_path(None)
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default(),
            "managed": managed,
        })
    }

    pub async fn status(State(state): State<Arc<AppState>>) -> Json<Value> {
        Json(snapshot(&state))
    }

    #[derive(Deserialize)]
    pub struct Toggle {
        pub enable: bool,
    }

    /// Refusals answer with an error status (see `refusal`): until they did, the dashboard
    /// read a switch that could not be thrown as "done".
    pub async fn service_toggle(
        State(state): State<Arc<AppState>>,
        JsonBody(req): JsonBody<Toggle>,
    ) -> Response {
        let result = tokio::task::spawn_blocking(move || -> Result<()> {
            if req.enable {
                let exe = install_binary()?;
                // Nothing about the bind here: it is a stored setting, so the login
                // item comes back up listening wherever this router does, whether or
                // not the switch was thrown before the service was registered.
                service::install(&exe)?;
            } else {
                service::uninstall()?;
            }
            Ok(())
        })
        .await;
        match result {
            Ok(Ok(())) => {
                let mut out = snapshot(&state);
                out["ok"] = json!(true);
                Json(out).into_response()
            }
            // 500 either way: a request that says only on or off cannot be wrong, so a
            // failure here is launchctl, systemd or the file system not doing what the
            // router asked. (A platform with no login item to register says so in GET
            // /api/setup's `supported`, and no client offers the switch there.)
            Ok(Err(e)) => failed(e),
            Err(e) => failed(e),
        }
    }

    #[derive(Deserialize)]
    pub struct ConfigReq {
        pub enable: bool,
        /// Absent means the global settings file.
        #[serde(default)]
        pub project: Option<String>,
    }

    pub async fn config_toggle(
        State(state): State<Arc<AppState>>,
        JsonBody(req): JsonBody<ConfigReq>,
    ) -> Response {
        let url = format!("http://127.0.0.1:{}", state.config.bind.port());
        let project = req.project.clone().map(PathBuf::from);
        if let Some(dir) = &project {
            if !dir.is_dir() {
                // 400: the path sent is not a directory on this machine, and the project's
                // own directory is the fix.
                return refuse(StatusCode::BAD_REQUEST, format!("{} is not a directory", dir.display()));
            }
        }
        let scope = if project.is_some() { "project" } else { "global" };

        let result = tokio::task::spawn_blocking(move || -> Result<String> {
            let path = settings_path(project.as_deref())?;
            if req.enable {
                Ok(match apply(&path, &url, scope)? {
                    Applied::Added => format!("added ANTHROPIC_BASE_URL to {}", path.display()),
                    Applied::Replaced { previous } => {
                        format!("replaced {previous} in {}; it will be restored", path.display())
                    }
                    Applied::Adopted => {
                        format!("adopted the existing setting in {}", path.display())
                    }
                    Applied::AlreadySet => format!("{} already points here", path.display()),
                })
            } else {
                Ok(match revert(&path)? {
                    Reverted::Restored { to } => format!("restored {to}"),
                    Reverted::Removed => "removed ANTHROPIC_BASE_URL".to_string(),
                    Reverted::NotOurs { found } => {
                        format!("left alone — it now reads {found}, which we did not write")
                    }
                    Reverted::NotManaged => "was not managed by agent-router".to_string(),
                })
            }
        })
        .await;

        match result {
            Ok(Ok(detail)) => {
                let mut out = snapshot(&state);
                out["ok"] = json!(true);
                out["detail"] = json!(detail);
                Json(out).into_response()
            }
            Ok(Err(e)) => refuse(settings_status(&e), e.to_string()),
            Err(e) => failed(e),
        }
    }

    /// What `apply` or `revert` failing means. A file the router could not read or write
    /// is the router failing: 500. Anything else they refuse is the settings file as it
    /// stands — not JSON, not an object, an `env` that is not one — which they will not
    /// rewrite, because replacing a file they cannot parse would lose what the person put
    /// there: 409, fixed by fixing the file, never by sending the request again.
    fn settings_status(e: &anyhow::Error) -> StatusCode {
        if e.downcast_ref::<std::io::Error>().is_some() {
            StatusCode::INTERNAL_SERVER_ERROR
        } else {
            StatusCode::CONFLICT
        }
    }
}

/// Autostart from a shell profile, for systems with no service manager — which in
/// practice means WSL, where systemd only runs if `/etc/wsl.conf` opts in.
///
/// The block is delimited by markers so it can be removed exactly, leaving the rest
/// of the user's profile untouched. The same principle as the settings file: add one
/// thing, remember precisely what it was, take back only that.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub mod profile_autostart {
    use super::*;

    const BEGIN: &str = "# >>> agent-router >>>";
    const END: &str = "# <<< agent-router <<<";

    /// Every profile file that exists, so the router starts whichever shell is used.
    fn candidates() -> Vec<PathBuf> {
        let Ok(home) = home_dir() else { return vec![] };
        let mut out: Vec<PathBuf> = [".bashrc", ".zshrc"]
            .iter()
            .map(|f| home.join(f))
            .filter(|p| p.exists())
            .collect();
        if out.is_empty() {
            out.push(home.join(".profile"));
        }
        out
    }

    fn block(exe: &Path) -> String {
        format!(
            "{BEGIN}\n\
             # Starts Agent Router if it is not already running. `--if-not-running`\n\
             # exits immediately when something already answers on the port, so this\n\
             # is safe to run from every new shell.\n\
             [ -x \"{exe}\" ] && \"{exe}\" --if-not-running >>\"$HOME/.agent-router/logs/router.log\" 2>&1 &\n\
             {END}\n",
            exe = exe.display()
        )
    }

    #[cfg(test)]
    pub fn block_for_test(exe: &Path) -> String {
        block(exe)
    }

    #[cfg(test)]
    pub fn strip_for_test(text: &str) -> String {
        strip(text)
    }

    fn strip(text: &str) -> String {
        let mut out = String::new();
        let mut skipping = false;
        for line in text.lines() {
            if line.trim() == BEGIN {
                skipping = true;
                continue;
            }
            if line.trim() == END {
                skipping = false;
                continue;
            }
            if !skipping {
                out.push_str(line);
                out.push('\n');
            }
        }
        out
    }

    pub fn install(exe: &Path) -> Result<Vec<PathBuf>> {
        std::fs::create_dir_all(data_dir()?.join("logs"))?;
        let mut touched = vec![];
        for path in candidates() {
            let existing = std::fs::read_to_string(&path).unwrap_or_default();
            let mut cleaned = strip(&existing);
            if !cleaned.is_empty() && !cleaned.ends_with('\n') {
                cleaned.push('\n');
            }
            write_atomic(&path, &format!("{cleaned}{}", block(exe)))?;
            touched.push(path);
        }
        Ok(touched)
    }

    pub fn uninstall() -> Result<()> {
        for path in candidates() {
            let Ok(existing) = std::fs::read_to_string(&path) else { continue };
            if existing.contains(BEGIN) {
                write_atomic(&path, &strip(&existing))?;
            }
        }
        Ok(())
    }

    pub fn is_registered() -> bool {
        candidates().iter().any(|p| {
            std::fs::read_to_string(p)
                .map(|t| t.contains(BEGIN))
                .unwrap_or(false)
        })
    }
}

// ---------------------------------------------------------------------------
// Service registration
// ---------------------------------------------------------------------------

#[cfg(target_os = "macos")]
pub const SERVICE_LABEL: &str = "ca.whnet.agent-router";

/// Load a login item into launchd from its plist, taking out first the job it replaces.
///
/// `bootout` answers before launchd has let go of a job that is still stopping, and a
/// `bootstrap` of its label in that window is refused with "5: Input/output error". On
/// 2026-10-09 that left a Mac's agent stopped and not loaded again: an agent that took 3 s to
/// stop kept its label for 3.2 s after `bootout` returned. So the label is waited out first,
/// for at most 30 s: launchd kills a job still running `ExitTimeOut` (20 s by default) after
/// its SIGTERM.
#[cfg(any(target_os = "macos", test))]
fn launchd_reload(domain: &str, label: &str, plist: &Path, launchctl: impl Fn(&[&str]) -> Result<()>) -> Result<()> {
    let target = format!("{domain}/{label}");
    let _ = launchctl(&["bootout", &target]);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while launchctl(&["print", &target]).is_ok() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    launchctl(&["bootstrap", domain, &plist.to_string_lossy()])
}

#[cfg(test)]
mod launchd_tests {
    use std::cell::{Cell, RefCell};

    use super::*;

    /// launchd as measured on 2026-10-09: `bootout` answers at once, the job's label stays
    /// while it stops (here, for `stops_in` more looks at it), and a `bootstrap` of a label
    /// still there is refused.
    struct Launchd {
        there: Cell<bool>,
        stopping: Cell<Option<u32>>,
        stops_in: u32,
        calls: RefCell<Vec<String>>,
    }

    impl Launchd {
        fn new(there: bool, stops_in: u32) -> Launchd {
            Launchd { there: Cell::new(there), stopping: Cell::new(None), stops_in, calls: RefCell::new(Vec::new()) }
        }

        fn launchctl(&self, args: &[&str]) -> Result<()> {
            self.calls.borrow_mut().push(args[0].to_string());
            match (args[0], self.there.get(), self.stopping.get()) {
                ("bootout", true, _) => self.stopping.set(Some(self.stops_in)),
                ("bootout", false, _) => return Err(anyhow!("Boot-out failed: 3: No such process")),
                ("print", true, Some(0)) => {
                    self.there.set(false);
                    self.stopping.set(None);
                    return Err(anyhow!("Could not find service"));
                }
                ("print", true, Some(n)) => self.stopping.set(Some(n - 1)),
                ("print", true, None) => {}
                ("print", false, _) => return Err(anyhow!("Could not find service")),
                ("bootstrap", true, _) => return Err(anyhow!("Bootstrap failed: 5: Input/output error")),
                ("bootstrap", false, _) => self.there.set(true),
                (other, ..) => panic!("launchctl {other} was not expected"),
            }
            Ok(())
        }

        fn calls(&self) -> Vec<String> {
            self.calls.borrow().clone()
        }
    }

    #[test]
    fn launchd_reload_waits_until_the_old_job_is_gone() {
        let launchd = Launchd::new(true, 3);
        launchd_reload("gui/501", "ca.whnet.agent-router.agent", Path::new("/p.plist"), |a| launchd.launchctl(a)).unwrap();
        assert_eq!(launchd.calls(), ["bootout", "print", "print", "print", "print", "bootstrap"]);
        assert!(launchd.there.get(), "the new job is loaded");
    }

    #[test]
    fn launchd_reload_loads_at_once_what_was_not_loaded() {
        let launchd = Launchd::new(false, 3);
        launchd_reload("gui/501", "ca.whnet.agent-router.agent", Path::new("/p.plist"), |a| launchd.launchctl(a)).unwrap();
        assert_eq!(launchd.calls(), ["bootout", "print", "bootstrap"]);
    }
}

#[cfg(target_os = "macos")]
pub mod service {
    use super::*;

    /// The file launchd reads at boot. Public so `forget_bind_env` can rewrite it.
    pub fn path() -> Result<PathBuf> {
        plist_path()
    }

    fn plist_path() -> Result<PathBuf> {
        let dir = home_dir()?.join("Library").join("LaunchAgents");
        std::fs::create_dir_all(&dir)?;
        Ok(dir.join(format!("{SERVICE_LABEL}.plist")))
    }

    fn uid() -> String {
        // SAFETY: getuid() cannot fail and touches no memory we own.
        unsafe { libc::getuid() }.to_string()
    }

    pub fn install(exe: &Path) -> Result<PathBuf> {
        let dir = data_dir()?;
        std::fs::create_dir_all(dir.join("logs"))?;
        let logs = dir.join("logs").join("router.log");

        // launchd hands a process almost no PATH, and the router shells out to
        // `claude` to identify accounts.
        let claude_dir = which("claude").unwrap_or_else(|| PathBuf::from("/usr/local/bin"));
        let path_env = format!(
            "{}:/usr/bin:/bin:/usr/sbin:/sbin",
            claude_dir.parent().unwrap_or(Path::new("/usr/local/bin")).display()
        );

        let plist = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>{SERVICE_LABEL}</string>
  <key>ProgramArguments</key><array><string>{exe}</string></array>
  <key>RunAtLoad</key><true/>
  <!-- Restart on a crash only. A clean exit is deliberate: it is what happens when a
       newer build takes the port over, and restarting then would fight the handover. -->
  <key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>
  <key>ThrottleInterval</key><integer>5</integer>
  <key>ProcessType</key><string>Background</string>
  <!-- Where to listen is not here: it is a setting in the store, so the dashboard can
       change it without rewriting this file. See src/network.rs. -->
  <key>EnvironmentVariables</key><dict>
    <key>PATH</key><string>{path_env}</string>
    <key>HOME</key><string>{home}</string>
  </dict>
  <key>StandardOutPath</key><string>{logs}</string>
  <key>StandardErrorPath</key><string>{logs}</string>
</dict>
</plist>
"#,
            exe = exe.display(),
            home = home_dir()?.display(),
            logs = logs.display(),
        );

        let path = plist_path()?;
        write_atomic(&path, &plist)?;
        let domain = format!("gui/{}", uid());
        launchd_reload(&domain, SERVICE_LABEL, &path, |args| run("launchctl", args))?;
        let _ = run("launchctl", &["enable", &format!("{domain}/{SERVICE_LABEL}")]);
        Ok(path)
    }

    pub fn uninstall() -> Result<()> {
        let domain = format!("gui/{}", uid());
        let _ = run("launchctl", &["bootout", &format!("{domain}/{SERVICE_LABEL}")]);
        if let Ok(p) = plist_path() {
            let _ = std::fs::remove_file(p);
        }
        Ok(())
    }

    pub fn is_registered() -> bool {
        plist_path().map(|p| p.exists()).unwrap_or(false)
    }

    /// For a machine that runs the agent instead (`agent install`): the login item out of
    /// launchd, which stops a router it started, and its plist kept aside. Says what it did;
    /// nothing when there was no login item.
    pub fn retire() -> Result<Vec<String>> {
        let path = plist_path()?;
        if !path.exists() {
            return Ok(Vec::new());
        }
        let _ = run("launchctl", &["bootout", &format!("gui/{}/{SERVICE_LABEL}", uid())]);
        let kept = super::set_aside(&path, &data_dir()?)?;
        Ok(vec![
            "out of launchd, and a router it started has stopped".to_string(),
            format!("its plist is kept as {}", kept.display()),
        ])
    }

    fn which(bin: &str) -> Option<PathBuf> {
        let out = std::process::Command::new("/usr/bin/which").arg(bin).output().ok()?;
        out.status.success().then(|| {
            PathBuf::from(String::from_utf8_lossy(&out.stdout).trim().to_string())
        })
    }

    fn run(cmd: &str, args: &[&str]) -> Result<()> {
        let out = std::process::Command::new(cmd).args(args).output()?;
        if !out.status.success() {
            return Err(anyhow!(
                "{cmd} {}: {}",
                args.join(" "),
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        Ok(())
    }
}

/// The machine agent's login item, beside the router's own: a machine pointed at a
/// hosted router runs the agent and no router, but the two never share a label, so
/// installing one can never remove the other.
#[cfg(target_os = "macos")]
pub const AGENT_LABEL: &str = "ca.whnet.agent-router.agent";

#[cfg(target_os = "macos")]
pub mod agent_service {
    use super::*;

    fn plist_path() -> Result<PathBuf> {
        let dir = home_dir()?.join("Library").join("LaunchAgents");
        std::fs::create_dir_all(&dir)?;
        Ok(dir.join(format!("{AGENT_LABEL}.plist")))
    }

    fn domain() -> String {
        // SAFETY: getuid() cannot fail and touches no memory we own.
        format!("gui/{}", unsafe { libc::getuid() })
    }

    /// Write the login item and load it. With a keeper running it is not restarted: the
    /// keeper hands over to the new build itself (`agent::keep`), and a restart would take the
    /// machine offline for nothing. An agent from before keepers is restarted once.
    pub fn install(exe: &Path, keeper_running: bool) -> Result<PathBuf> {
        let dir = data_dir()?;
        std::fs::create_dir_all(dir.join("logs"))?;
        let logs = dir.join("logs").join("agent.log");
        let plist = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>{AGENT_LABEL}</string>
  <!-- `--ensure`: when the editor has already started an agent this one exits cleanly,
       and a clean exit is not restarted. -->
  <key>ProgramArguments</key><array><string>{exe}</string><string>agent</string><string>--ensure</string></array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>
  <key>ThrottleInterval</key><integer>5</integer>
  <key>ProcessType</key><string>Background</string>
  <!-- Where the router is, and its password, are in agent.json beside the store, not
       here: this file is world-readable. -->
  <key>EnvironmentVariables</key><dict>
    <key>PATH</key><string>/usr/bin:/bin:/usr/sbin:/sbin</string>
    <key>HOME</key><string>{home}</string>
  </dict>
  <key>StandardOutPath</key><string>{logs}</string>
  <key>StandardErrorPath</key><string>{logs}</string>
</dict>
</plist>
"#,
            exe = exe.display(),
            home = home_dir()?.display(),
            logs = logs.display(),
        );
        let path = plist_path()?;
        write_atomic(&path, &plist)?;
        let target = format!("{}/{AGENT_LABEL}", domain());
        if keeper_running {
            // Loaded if it is not; launchd refuses a second load of a loaded job, harmlessly.
            let _ = launchctl(&["bootstrap", &domain(), &path.to_string_lossy()]);
        } else {
            launchd_reload(&domain(), AGENT_LABEL, &path, launchctl)?;
        }
        let _ = launchctl(&["enable", &target]);
        Ok(path)
    }

    pub fn uninstall() -> Result<()> {
        let _ = launchctl(&["bootout", &format!("{}/{AGENT_LABEL}", domain())]);
        if let Ok(p) = plist_path() {
            let _ = std::fs::remove_file(p);
        }
        Ok(())
    }

    pub fn is_registered() -> bool {
        plist_path().map(|p| p.exists()).unwrap_or(false)
    }

    fn launchctl(args: &[&str]) -> Result<()> {
        let out = std::process::Command::new("launchctl").args(args).output()?;
        if !out.status.success() {
            return Err(anyhow!("launchctl {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim()));
        }
        Ok(())
    }
}

/// On Windows the agent starts at logon from the user's Startup folder, through a small
/// script that runs it hidden: it is a console program, and started directly it would
/// keep a console window open for as long as the user is logged on. Nothing here needs
/// administrator rights, and nothing touches the registry.
#[cfg(windows)]
pub mod agent_service {
    use super::*;

    fn script_path() -> Result<PathBuf> {
        let appdata = std::env::var_os("APPDATA").ok_or_else(|| anyhow!("APPDATA is not set"))?;
        let dir = PathBuf::from(appdata).join("Microsoft").join("Windows").join("Start Menu").join("Programs").join("Startup");
        std::fs::create_dir_all(&dir)?;
        Ok(dir.join("Agent Router agent.vbs"))
    }

    /// Write the Startup script, and run it now unless a keeper already runs: a keeper hands
    /// over to the new build itself (`agent::keep`).
    pub fn install(exe: &Path, keeper_running: bool) -> Result<PathBuf> {
        let exe = exe.display().to_string().replace('"', "");
        // Run(command, 0 = hidden, False = do not wait). A doubled quote is a quote in VBScript.
        let script = format!(
            "' Starts the Agent Router agent at logon, without a window.\r\n\
             CreateObject(\"WScript.Shell\").Run \"\"\"{exe}\"\" agent --ensure\", 0, False\r\n"
        );
        let path = script_path()?;
        // UTF-16 with its mark: without one, Windows Script Host reads a script in the ANSI
        // code page, and a profile folder named outside it would name the wrong program.
        let utf16: Vec<u8> = format!("\u{feff}{script}")
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        write_atomic(&path, utf16)?;
        // Started now as well, as launchd does on macOS.
        if !keeper_running {
            std::process::Command::new("wscript.exe")
                .arg(&path)
                .spawn()
                .with_context(|| format!("starting {}", path.display()))?;
        }
        Ok(path)
    }

    pub fn uninstall() -> Result<()> {
        if let Ok(p) = script_path() {
            let _ = std::fs::remove_file(p);
        }
        // The running agent is asked to stop through its lock, by `agent uninstall`.
        Ok(())
    }

    pub fn is_registered() -> bool {
        script_path().map(|p| p.exists()).unwrap_or(false)
    }
}

/// The machine agent's systemd user unit (see `agent_service` on Linux). Built outside that
/// module so its test runs everywhere. `%` is systemd's specifier character, so a path is
/// written with it doubled.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn agent_unit(exe: &Path, log: &Path) -> String {
    let esc = |p: &Path| p.display().to_string().replace('%', "%%");
    format!(
        "[Unit]\n\
         Description=Agent Router agent: this machine's sessions, for a hosted router\n\
         Wants=network-online.target\n\
         After=network-online.target\n\n\
         [Service]\n\
         # `--ensure`: when the editor has already started an agent this one exits\n\
         # cleanly, and a clean exit is not restarted.\n\
         ExecStart=\"{exe}\" agent --ensure\n\
         Restart=on-failure\n\
         RestartSec=5\n\
         StandardOutput=append:{log}\n\
         StandardError=append:{log}\n\
         # Where the router is, and its password, are in agent.json beside the store,\n\
         # not here.\n\n\
         [Install]\n\
         WantedBy=default.target\n",
        exe = esc(exe),
        log = esc(log),
    )
}

/// On Linux the agent is a systemd user unit beside the router's own, under its own name,
/// so installing one can never remove the other. Without a user systemd (WSL without it, a
/// container) there is no login item, and the editor starts the agent when a window opens.
#[cfg(target_os = "linux")]
pub mod agent_service {
    use super::*;

    pub const UNIT: &str = "agent-router-agent.service";

    fn unit_file() -> Result<PathBuf> {
        Ok(home_dir()?.join(".config").join("systemd").join("user").join(UNIT))
    }

    /// Write the unit and enable it. With a keeper running it is not restarted: the keeper
    /// hands over to the new build itself (`agent::keep`). Otherwise it is restarted, which
    /// starts a stopped unit and replaces an agent from before keepers; `enable --now` did
    /// nothing to a unit already running, which kept the old build until the next login.
    pub fn install(exe: &Path, keeper_running: bool) -> Result<PathBuf> {
        let logs = data_dir()?.join("logs");
        std::fs::create_dir_all(&logs)?;
        let path = unit_file()?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        write_atomic(&path, super::agent_unit(exe, &logs.join("agent.log")))?;
        if super::service::run("systemctl", &["--user", "daemon-reload"]).is_err() {
            let _ = std::fs::remove_file(&path);
            return Err(anyhow!(
                "systemd user services are not running here; start the agent with `agent-router agent`"
            ));
        }
        super::service::run("systemctl", &["--user", "enable", UNIT])?;
        if !keeper_running {
            super::service::run("systemctl", &["--user", "restart", UNIT])?;
        }
        Ok(path)
    }

    pub fn uninstall() -> Result<()> {
        let _ = super::service::run("systemctl", &["--user", "disable", "--now", UNIT]);
        if let Ok(p) = unit_file() {
            let _ = std::fs::remove_file(p);
        }
        let _ = super::service::run("systemctl", &["--user", "daemon-reload"]);
        Ok(())
    }

    pub fn is_registered() -> bool {
        unit_file().map(|p| p.exists()).unwrap_or(false)
    }
}

#[cfg(test)]
mod agent_unit_tests {
    use super::*;

    #[test]
    fn the_unit_runs_the_agent_once_and_restarts_it_only_after_a_crash() {
        let u = agent_unit(Path::new("/home/a b/.agent-router/bin/agent-router"), Path::new("/home/a b/.agent-router/logs/agent.log"));
        assert!(u.contains("ExecStart=\"/home/a b/.agent-router/bin/agent-router\" agent --ensure\n"), "{u}");
        assert!(u.contains("Restart=on-failure\n"), "a clean exit, another agent already running, is not restarted");
        assert!(u.contains("StandardOutput=append:/home/a b/.agent-router/logs/agent.log\n"), "the log the editor reads");
        assert!(u.contains("WantedBy=default.target\n"));
        assert!(
            !u.contains("Environment") && !u.contains("AGENT_ROUTER_PASSWORD"),
            "nothing secret in a unit file: the password stays in agent.json"
        );
        assert!(agent_unit(Path::new("/opt/100%/agent-router"), Path::new("/l")).contains("/opt/100%%/agent-router"));
    }
}

/// Everywhere else the agent has no login item: it is started by hand or by the editor.
#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
pub mod agent_service {
    use super::*;

    pub fn install(_exe: &Path, _keeper_running: bool) -> Result<PathBuf> {
        Err(anyhow!("the agent's login item is built for macOS, Linux and Windows; start it with `agent-router agent`"))
    }

    pub fn uninstall() -> Result<()> {
        Ok(())
    }

    pub fn is_registered() -> bool {
        false
    }
}

#[cfg(target_os = "linux")]
pub mod service {
    use super::*;

    pub const UNIT: &str = "agent-router.service";

    /// The file systemd reads at boot. Public so `forget_bind_env` can rewrite it.
    pub fn path() -> Result<PathBuf> {
        unit_path()
    }

    fn unit_path() -> Result<PathBuf> {
        let dir = home_dir()?.join(".config").join("systemd").join("user");
        std::fs::create_dir_all(&dir)?;
        Ok(dir.join(UNIT))
    }

    pub fn install(exe: &Path) -> Result<PathBuf> {
        let dir = data_dir()?;
        std::fs::create_dir_all(dir.join("logs"))?;
        let path_env = std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".into());
        let unit = format!(
            "[Unit]\n\
             Description=Agent Router — local proxy between Claude Code and Anthropic\n\
             After=network.target\n\n\
             [Service]\n\
             ExecStart={exe}\n\
             # Restart on a crash only. A clean exit is deliberate: it is what happens\n\
             # when a newer build takes the port over, and restarting would fight that.\n\
             Restart=on-failure\n\
             RestartSec=5\n\
             Environment=PATH={path_env}\n\
             # Where to listen is not here: it is a setting in the store, so the\n\
             # dashboard can change it without rewriting this file. See src/network.rs.\n\
             \n\
             [Install]\n\
             WantedBy=default.target\n",
            exe = exe.display(),
        );
        let path = unit_path()?;
        write_atomic(&path, &unit)?;

        // WSL only runs systemd when /etc/wsl.conf opts in, so a failure here is
        // expected rather than exceptional — say what to do instead.
        if run("systemctl", &["--user", "daemon-reload"]).is_err() {
            // Expected under WSL rather than exceptional: fall back to the shell
            // profile so the router still comes up on its own.
            let _ = std::fs::remove_file(&path);
            let touched = super::profile_autostart::install(exe)?;
            return touched.into_iter().next().ok_or_else(|| {
                anyhow!("no shell profile found to add an autostart line to")
            });
        }
        run("systemctl", &["--user", "enable", "--now", UNIT])?;
        Ok(path)
    }

    pub fn uninstall() -> Result<()> {
        let _ = super::profile_autostart::uninstall();
        let _ = run("systemctl", &["--user", "disable", "--now", UNIT]);
        if let Ok(p) = unit_path() {
            let _ = std::fs::remove_file(p);
        }
        let _ = run("systemctl", &["--user", "daemon-reload"]);
        Ok(())
    }

    pub fn is_registered() -> bool {
        unit_path().map(|p| p.exists()).unwrap_or(false)
            || super::profile_autostart::is_registered()
    }

    /// For a machine that runs the agent instead (`agent install`): the unit stopped and
    /// disabled and its file kept aside, and the shell-profile line taken out. Says what it
    /// did; nothing when there was neither.
    pub fn retire() -> Result<Vec<String>> {
        let mut done = Vec::new();
        if super::profile_autostart::is_registered() {
            super::profile_autostart::uninstall()?;
            done.push("its line in the shell profile is removed".to_string());
        }
        let path = unit_path()?;
        if path.exists() {
            let _ = run("systemctl", &["--user", "disable", "--now", UNIT]);
            let kept = super::set_aside(&path, &data_dir()?)?;
            let _ = run("systemctl", &["--user", "daemon-reload"]);
            done.push(format!("{UNIT} is stopped and disabled, and kept as {}", kept.display()));
        }
        Ok(done)
    }

    pub(super) fn run(cmd: &str, args: &[&str]) -> Result<()> {
        let out = std::process::Command::new(cmd)
            .args(args)
            .output()
            .with_context(|| format!("running {cmd}"))?;
        if !out.status.success() {
            return Err(anyhow!(
                "{cmd} {}: {}",
                args.join(" "),
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        Ok(())
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub mod service {
    use super::*;

    /// Windows would register a per-user logon task (Task Scheduler supports
    /// restart-on-failure, which the Run registry key does not). Deliberately not
    /// written blind: it cannot be tested from here, and an untested installer that
    /// edits the registry is worse than none.
    pub fn install(_exe: &Path) -> Result<PathBuf> {
        Err(anyhow!(
            "automatic service registration is not implemented on this platform yet — \
             the router still runs in the foreground, and `install --no-service` will \
             set up the Claude Code config for you"
        ))
    }

    pub fn uninstall() -> Result<()> {
        Ok(())
    }

    pub fn is_registered() -> bool {
        false
    }

    /// A local router never ran here, so it left no login item.
    pub fn retire() -> Result<Vec<String>> {
        Ok(Vec::new())
    }

    pub fn path() -> Result<PathBuf> {
        Err(anyhow!("no service file on this platform"))
    }
}

/// Move a retired login item's file into `dir`, named for what it was: kept, so the step
/// can be undone by hand, but no longer anywhere the system reads at login. A rename where
/// it can be, a copy where the two are on different volumes.
#[cfg_attr(not(any(target_os = "macos", target_os = "linux")), allow(dead_code))]
fn set_aside(file: &Path, dir: &Path) -> Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let name = file.file_name().ok_or_else(|| anyhow!("{} names no file", file.display()))?;
    let kept = dir.join(format!("{}.retired", name.to_string_lossy()));
    if std::fs::rename(file, &kept).is_err() {
        std::fs::copy(file, &kept).with_context(|| format!("keeping {}", file.display()))?;
        std::fs::remove_file(file).with_context(|| format!("removing {}", file.display()))?;
    }
    Ok(kept)
}

/// Take `AGENT_ROUTER_BIND` out of the login item, if one registered by an older build
/// still carries it.
///
/// Where to listen is a stored setting now, and the environment outranks it — so a
/// variable left behind in the service file would quietly outvote the switch on the
/// next boot, which is the exact trap this whole change exists to close.
///
/// A rewrite and nothing more: the file is only read at boot, and `launchctl bootout`
/// would kill the very router making the change.
pub fn forget_bind_env() -> bool {
    let Ok(path) = service::path() else { return false };
    let Ok(text) = std::fs::read_to_string(&path) else { return false };
    if !text.contains("AGENT_ROUTER_BIND") {
        return false;
    }
    match write_atomic(&path, &without_bind_env(&text)) {
        Ok(()) => {
            tracing::info!(
                path = %path.display(),
                "removed AGENT_ROUTER_BIND from the login item; the stored setting decides now"
            );
            true
        }
        Err(err) => {
            tracing::warn!(%err, "could not rewrite the login item to drop AGENT_ROUTER_BIND");
            false
        }
    }
}

/// Every line mentioning the variable, gone. One line each in both formats — a plist
/// `<key>`/`<string>` pair on one line, a systemd `Environment=` line — and nothing
/// else in either file mentions it.
fn without_bind_env(text: &str) -> String {
    text.lines()
        .filter(|line| !line.contains("AGENT_ROUTER_BIND"))
        .map(|line| format!("{line}\n"))
        .collect()
}

#[cfg(test)]
mod set_aside_tests {
    use super::set_aside;

    /// A retired login item leaves the folder the system reads at login, and is kept whole,
    /// under a name that says what it was.
    #[test]
    fn a_retired_login_item_is_kept_where_nothing_starts_it() {
        let root = std::env::temp_dir().join(format!("ar-retire-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let agents = root.join("LaunchAgents");
        std::fs::create_dir_all(&agents).unwrap();
        let plist = agents.join("ca.whnet.agent-router.plist");
        std::fs::write(&plist, "<plist/>").unwrap();
        let kept = set_aside(&plist, &root.join("data")).unwrap();
        assert_eq!(kept, root.join("data").join("ca.whnet.agent-router.plist.retired"));
        assert!(!plist.exists());
        assert_eq!(std::fs::read_to_string(&kept).unwrap(), "<plist/>");
        let _ = std::fs::remove_dir_all(&root);
    }
}

#[cfg(test)]
mod bind_env_tests {
    use super::without_bind_env;

    #[test]
    fn the_variable_goes_and_the_rest_of_the_file_stays() {
        let plist = "<dict>\n  <key>PATH</key><string>/usr/bin</string>\n    \
                     <key>AGENT_ROUTER_BIND</key><string>0.0.0.0</string>\n  \
                     <key>HOME</key><string>/Users/u</string>\n</dict>\n";
        let cleaned = without_bind_env(plist);
        assert!(!cleaned.contains("AGENT_ROUTER_BIND"));
        assert!(cleaned.contains("<key>PATH</key>"));
        assert!(cleaned.contains("<key>HOME</key>"));

        let unit = "[Service]\nExecStart=/x\nEnvironment=PATH=/usr/bin\n\
                    Environment=AGENT_ROUTER_BIND=0.0.0.0\n\n[Install]\n";
        let cleaned = without_bind_env(unit);
        assert!(!cleaned.contains("AGENT_ROUTER_BIND"));
        assert!(cleaned.contains("Environment=PATH=/usr/bin"));
        assert!(cleaned.contains("[Install]"));
    }

    #[test]
    fn a_file_without_it_is_left_byte_for_byte() {
        let unit = "[Service]\nExecStart=/x\nEnvironment=PATH=/usr/bin\n";
        assert_eq!(without_bind_env(unit), unit);
    }
}

#[cfg(test)]
mod autostart_tests {
    use super::profile_autostart::*;
    use std::path::Path;

    // These exercise the logic that edits a user's shell profile — the part with the
    // most potential to do damage on a machine none of us can test on.

    fn rc(body: &str) -> String {
        format!("export EDITOR=vim\n{body}alias ll='ls -la'\n")
    }

    #[test]
    fn removing_our_block_leaves_the_rest_of_the_profile_intact() {
        let exe = Path::new("/home/u/.agent-router/bin/agent-router");
        let original = rc("");
        let with_block = format!("{original}{}", block_for_test(exe));
        assert_eq!(strip_for_test(&with_block), original);
    }

    #[test]
    fn installing_twice_does_not_duplicate_the_block() {
        let exe = Path::new("/home/u/.agent-router/bin/agent-router");
        let once = format!("{}{}", rc(""), block_for_test(exe));
        // A second install strips first, then appends — never two copies.
        let twice = format!("{}{}", strip_for_test(&once), block_for_test(exe));
        assert_eq!(once, twice);
        // One marker pair is what "not duplicated" actually means.
        assert_eq!(twice.matches("# >>> agent-router >>>").count(), 1);
        assert_eq!(twice.matches("# <<< agent-router <<<").count(), 1);
    }

    #[test]
    fn a_profile_without_our_block_is_untouched() {
        let untouched = rc("");
        assert_eq!(strip_for_test(&untouched), untouched);
    }

    #[test]
    fn the_block_starts_the_router_without_replacing_a_running_one() {
        let exe = Path::new("/home/u/.agent-router/bin/agent-router");
        let b = block_for_test(exe);
        // --if-not-running is what makes this safe to run from every new shell.
        assert!(b.contains("--if-not-running"), "{b}");
        assert!(b.contains(&exe.display().to_string()));
        assert!(b.trim_end().ends_with("# <<< agent-router <<<"), "{b}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_busy_program_is_started_again_once_free() {
        use std::io::{Error, ErrorKind};
        let mut calls = 0;
        let started = when_free(|| {
            calls += 1;
            if calls <= 2 { Err(Error::from(ErrorKind::ExecutableFileBusy)) } else { Ok("started") }
        });
        assert_eq!((started.unwrap(), calls), ("started", 3));

        let mut calls = 0;
        let missing = when_free(|| -> std::io::Result<()> {
            calls += 1;
            Err(Error::from(ErrorKind::NotFound))
        });
        assert_eq!((missing.unwrap_err().kind(), calls), (ErrorKind::NotFound, 1), "any other failure at once");

        let mut calls = 0;
        let stuck = when_free(|| -> std::io::Result<()> {
            calls += 1;
            Err(Error::from(ErrorKind::ExecutableFileBusy))
        });
        assert_eq!((stuck.unwrap_err().kind(), calls), (ErrorKind::ExecutableFileBusy, BUSY_TRIES + 1), "within a bound");
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ar-setup-{name}-{}", now_ms()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("settings.json")
    }

    /// `HOME` is process-global, so tests that redirect it must not run in
    /// parallel with each other — without this they interleave and each sees
    /// another's managed state.
    static HOME_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn with_home<T>(f: impl FnOnce() -> T) -> T {
        let _guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // Keep managed-state writes out of the real home directory.
        let tmp = std::env::temp_dir().join(format!("ar-home-{}", now_ms()));
        std::fs::create_dir_all(&tmp).unwrap();
        let old = std::env::var("HOME").ok();
        std::env::set_var("HOME", &tmp);
        let out = f();
        match old {
            Some(h) => std::env::set_var("HOME", h),
            None => std::env::remove_var("HOME"),
        }
        out
    }

    #[test]
    fn adds_our_key_and_leaves_every_other_setting_untouched() {
        with_home(|| {
            let path = scratch("add");
            std::fs::write(
                &path,
                r#"{"effortLevel":"max","env":{"FOO":"bar"},"other":{"deep":[1,2]}}"#,
            )
            .unwrap();
            apply(&path, "http://127.0.0.1:8787", "global").unwrap();

            let v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
            assert_eq!(v["effortLevel"], "max");
            assert_eq!(v["env"]["FOO"], "bar");
            assert_eq!(v["other"]["deep"][1], 2);
            assert_eq!(v["env"]["ANTHROPIC_BASE_URL"], "http://127.0.0.1:8787");
        });
    }

    /// `agent install` manages the password beside the address: recorded only as a
    /// fingerprint, backed up once, and both put back together.
    #[test]
    fn a_password_is_kept_as_a_fingerprint_and_put_back_with_the_address() {
        with_home(|| {
            let path = scratch("token");
            std::fs::write(&path, r#"{"env":{"ANTHROPIC_AUTH_TOKEN":"old-token","FOO":"bar"}}"#).unwrap();
            apply(&path, "https://router.example.com", "global").unwrap();
            apply_key(&path, TOKEN_KEY, "s3cret-password", "global").unwrap();

            let v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
            assert_eq!(v["env"]["ANTHROPIC_BASE_URL"], "https://router.example.com");
            assert_eq!(v["env"]["ANTHROPIC_AUTH_TOKEN"], "s3cret-password");
            let recorded = std::fs::read_to_string(state_path().unwrap()).unwrap();
            assert!(!recorded.contains("s3cret-password"), "the password is never recorded");
            assert!(recorded.contains("sha256:"));
            let backups = load_state().managed.iter().filter(|m| m.backup_path.is_some()).count();
            assert_eq!(backups, 1, "one backup per file, not one per key");

            assert!(matches!(revert(&path).unwrap(), Reverted::Removed));
            let v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
            assert!(v["env"].get("ANTHROPIC_BASE_URL").is_none());
            assert_eq!(v["env"]["ANTHROPIC_AUTH_TOKEN"], "old-token", "the password that was there before is back");
            assert_eq!(v["env"]["FOO"], "bar");
            assert!(load_state().managed.is_empty());
        });
    }

    #[test]
    fn revert_removes_a_key_we_added_and_the_env_we_created() {
        with_home(|| {
            let path = scratch("revert");
            std::fs::write(&path, r#"{"effortLevel":"max"}"#).unwrap();
            apply(&path, "http://127.0.0.1:8787", "global").unwrap();
            matches!(revert(&path).unwrap(), Reverted::Removed);

            let v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
            assert_eq!(v["effortLevel"], "max");
            assert!(v.get("env").is_none(), "an env we created should not be left behind");
        });
    }

    #[test]
    fn revert_restores_a_value_that_was_there_before_us() {
        with_home(|| {
            let path = scratch("restore");
            std::fs::write(&path, r#"{"env":{"ANTHROPIC_BASE_URL":"https://upstream.example"}}"#)
                .unwrap();
            apply(&path, "http://127.0.0.1:8787", "global").unwrap();
            match revert(&path).unwrap() {
                Reverted::Restored { to } => assert_eq!(to, "https://upstream.example"),
                _ => panic!("should have restored the prior value"),
            }
            let v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
            assert_eq!(v["env"]["ANTHROPIC_BASE_URL"], "https://upstream.example");
        });
    }

    #[test]
    fn revert_refuses_to_undo_an_edit_the_user_made_themselves() {
        with_home(|| {
            let path = scratch("useredit");
            std::fs::write(&path, "{}").unwrap();
            apply(&path, "http://127.0.0.1:8787", "global").unwrap();
            // The user re-points it somewhere else afterwards.
            std::fs::write(&path, r#"{"env":{"ANTHROPIC_BASE_URL":"http://127.0.0.1:9999"}}"#)
                .unwrap();
            match revert(&path).unwrap() {
                Reverted::NotOurs { found } => assert_eq!(found, "http://127.0.0.1:9999"),
                _ => panic!("must not clobber a value the user changed"),
            }
            let v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
            assert_eq!(v["env"]["ANTHROPIC_BASE_URL"], "http://127.0.0.1:9999");
        });
    }

    #[test]
    fn malformed_settings_are_refused_rather_than_replaced() {
        with_home(|| {
            let path = scratch("broken");
            std::fs::write(&path, "{ this is not json").unwrap();
            assert!(apply(&path, "http://127.0.0.1:8787", "global").is_err());
            assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ this is not json");
        });
    }
}

#[cfg(test)]
mod picker_tests {
    use super::*;

    /// A user settings file of its own, holding `json`.
    fn settings(tag: &str, json: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ar-picker-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        std::fs::write(&path, json).unwrap();
        path
    }

    fn read(path: &Path) -> Value {
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    /// The catalogue as `GET /api/models` answers it, with a Claude model among Kimi Code's.
    fn catalogue(ids: &[(&str, &str, &str)]) -> Vec<Value> {
        ids.iter()
            .map(|(id, provider, name)| serde_json::json!({ "id": id, "provider": provider, "display_name": name, "assigned": null, "serving": "a" }))
            .collect()
    }

    #[test]
    fn install_writes_a_picker_row_for_each_non_claude_model() {
        let path = settings("rows", r#"{"env":{"ANTHROPIC_BASE_URL":"http://127.0.0.1:8787"},"theme":"dark"}"#);
        let models = picker_models(&catalogue(&[
            ("claude-opus-5-5", "anthropic", "claude-opus-5-5"),
            ("k3", "kimi", "Kimi K3"),
            ("kimi-for-coding", "kimi", "Kimi for Coding"),
        ]));
        assert!(sync_picker_rows(&path, &models).unwrap());
        let got = read(&path);
        assert_eq!(
            got["modelPicker"],
            serde_json::json!({ "options": [
                { "model": "k3", "label": "Kimi K3", "description": "Kimi Code · via Agent Router" },
                { "model": "kimi-for-coding", "label": "Kimi for Coding", "description": "Kimi Code · via Agent Router" },
            ] }),
            "Claude's models are the picker's own; only the others get a row"
        );
        assert_eq!(got["env"]["ANTHROPIC_BASE_URL"], "http://127.0.0.1:8787");
        assert_eq!(got["theme"], "dark", "nothing else in the file changes");
        assert!(got["modelPicker"].get("replaceBuiltInOptions").is_none());
        assert!(!sync_picker_rows(&path, &models).unwrap(), "in step already: the file is not written again");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn the_owners_own_picker_rows_are_left_alone() {
        let path = settings(
            "owner",
            r#"{"modelPicker":{"replaceBuiltInOptions":false,"options":[
                {"model":"opus","label":"Mine"},
                {"model":"k3","label":"My K3","description":"set by hand"},
                {"model":"k3-256k","label":"Kimi K3 (256K)","description":"Kimi Code · via Agent Router"}
            ]}}"#,
        );
        let models = picker_models(&catalogue(&[("k3", "kimi", "Kimi K3"), ("kimi-for-coding", "kimi", "Kimi for Coding")]));
        assert!(sync_picker_rows(&path, &models).unwrap());
        assert_eq!(
            read(&path)["modelPicker"],
            serde_json::json!({ "replaceBuiltInOptions": false, "options": [
                { "model": "opus", "label": "Mine" },
                { "model": "k3", "label": "My K3", "description": "set by hand" },
                { "model": "kimi-for-coding", "label": "Kimi for Coding", "description": "Kimi Code · via Agent Router" },
            ] }),
            "the owner's rows stay as they were, k3 gets no second row, and the router's stale row goes"
        );

        // Nothing left to offer takes out the router's rows and nothing else.
        assert!(sync_picker_rows(&path, &[]).unwrap());
        let rows = read(&path)["modelPicker"]["options"].as_array().unwrap().len();
        assert_eq!(rows, 2);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
