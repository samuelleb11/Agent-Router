//! Self-install: register the service, and point Claude Code at the router.
//!
//! The rule this module exists to honour: **only ever touch our own key.** A user's
//! settings file is theirs. We add exactly one entry, remember precisely what was
//! there before, and on removal put that back — but only if the value is still the
//! one we wrote. If the user has since changed it, we leave it alone and say so.
//! Nothing else in the file is rewritten, reordered or reformatted.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// The one settings key the router manages.
const ENV_KEY: &str = "ANTHROPIC_BASE_URL";

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

/// `$HOME` on Unix, `%USERPROFILE%` on Windows.
pub fn home_dir() -> Result<PathBuf> {
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
    write_atomic(&path, &serde_json::to_string_pretty(state)?)
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

fn write_atomic(path: &Path, text: &str) -> Result<()> {
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
    let mut settings = read_settings(path)?;

    let previous = settings
        .get("env")
        .and_then(Value::as_object)
        .and_then(|e| e.get(ENV_KEY))
        .and_then(Value::as_str)
        .map(str::to_owned);

    let already = previous.as_deref() == Some(url);
    if already && load_state().managed.iter().any(|m| m.settings_path == path.to_string_lossy()) {
        return Ok(Applied::AlreadySet);
    }

    // Back up the whole file once, before the first change we make to it.
    let backup_path = if path.exists() && !already {
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
    env.insert(ENV_KEY.to_string(), Value::String(url.to_string()));

    if !already {
        write_atomic(path, &(serde_json::to_string_pretty(&Value::Object(settings))? + "\n"))?;
    }

    let mut state = load_state();
    state.managed.retain(|m| m.settings_path != path.to_string_lossy());
    state.managed.push(Managed {
        settings_path: path.to_string_lossy().into_owned(),
        scope: scope.to_string(),
        value: url.to_string(),
        // When adopting, there is no knowable prior value: the key already held ours.
        previous: if already { None } else { previous.clone() },
        applied_at: now_ms(),
        backup_path,
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

pub enum Reverted {
    Restored { to: String },
    Removed,
    NotOurs { found: String },
    NotManaged,
}

/// Undo what `apply` did — and nothing else.
pub fn revert(path: &Path) -> Result<Reverted> {
    let mut state = load_state();
    let key = path.to_string_lossy().into_owned();
    let Some(idx) = state.managed.iter().position(|m| m.settings_path == key) else {
        return Ok(Reverted::NotManaged);
    };
    let managed = state.managed[idx].clone();

    let mut settings = read_settings(path)?;
    let current = settings
        .get("env")
        .and_then(Value::as_object)
        .and_then(|e| e.get(ENV_KEY))
        .and_then(Value::as_str)
        .map(str::to_owned);

    // If it is no longer the value we wrote, the user changed it deliberately.
    // Reverting would undo their edit, so stop and leave the file alone.
    match current.as_deref() {
        Some(found) if found != managed.value => {
            return Ok(Reverted::NotOurs { found: found.to_string() })
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
                env.insert(ENV_KEY.to_string(), Value::String(previous.clone()));
                Reverted::Restored { to: previous.clone() }
            }
            None => {
                env.remove(ENV_KEY);
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

/// Copy the running binary somewhere stable, so rebuilding the repo never swaps the
/// file out from under the installed service.
pub fn install_binary() -> Result<PathBuf> {
    let dir = data_dir()?.join("bin");
    std::fs::create_dir_all(&dir)?;
    let dest = dir.join(if cfg!(windows) { "agent-router.exe" } else { "agent-router" });
    let current = std::env::current_exe()?;
    if current != dest {
        let tmp = dest.with_extension("new");
        std::fs::copy(&current, &tmp)?;
        std::fs::rename(&tmp, &dest)?;
    }
    Ok(dest)
}

// ---------------------------------------------------------------------------
// Web API — the same operations the subcommands perform, so setup can be done
// from the dashboard the binary is already serving.
// ---------------------------------------------------------------------------

pub mod api {
    use std::sync::Arc;

    use axum::extract::State;
    use axum::Json;
    use serde::Deserialize;
    use serde_json::{json, Value};

    use super::*;
    use crate::proxy::AppState;

    fn snapshot(state: &AppState) -> Value {
        let managed = load_state()
            .managed
            .into_iter()
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

    pub async fn service_toggle(
        State(state): State<Arc<AppState>>,
        Json(req): Json<Toggle>,
    ) -> Json<Value> {
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
                Json(out)
            }
            Ok(Err(e)) => Json(json!({ "error": e.to_string() })),
            Err(e) => Json(json!({ "error": e.to_string() })),
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
        Json(req): Json<ConfigReq>,
    ) -> Json<Value> {
        let url = format!("http://127.0.0.1:{}", state.config.bind.port());
        let project = req.project.clone().map(PathBuf::from);
        if let Some(dir) = &project {
            if !dir.is_dir() {
                return Json(json!({ "error": format!("{} is not a directory", dir.display()) }));
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
                Json(out)
            }
            Ok(Err(e)) => Json(json!({ "error": e.to_string() })),
            Err(e) => Json(json!({ "error": e.to_string() })),
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
        let _ = run("launchctl", &["bootout", &format!("{domain}/{SERVICE_LABEL}")]);
        run("launchctl", &["bootstrap", &domain, &path.to_string_lossy()])?;
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

    fn run(cmd: &str, args: &[&str]) -> Result<()> {
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

    pub fn path() -> Result<PathBuf> {
        Err(anyhow!("no service file on this platform"))
    }
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
