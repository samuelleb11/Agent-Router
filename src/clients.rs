//! Which coding clients a machine has: Claude Code and Kimi Code, each as a command, an
//! editor extension and a home folder, with their versions. The agent reports them with its
//! places (`places::Report::clients`), at every connection and every minute, and
//! `/api/machines` gives each machine's.
//!
//! Found by looking, never by asking either client to do anything but print its version: a
//! command on `PATH` or in the folders its installers use, an extension in each editor's
//! extensions folder, and the home folder (`~/.claude`, `KIMI_CODE_HOME`).

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

use serde::{Deserialize, Serialize};

pub const CLAUDE_CODE: &str = "claude-code";
pub const KIMI_CODE: &str = "kimi-code";

/// One client on a machine: what of it is there, with versions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Client {
    /// `claude-code` or `kimi-code`.
    pub client: String,
    /// Its command line, when one is installed.
    #[serde(default)]
    pub cli: Option<Cli>,
    /// Its extension, in each editor that has it.
    #[serde(default)]
    pub extensions: Vec<Extension>,
    /// Its home folder, when there is one.
    #[serde(default)]
    pub home: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Cli {
    pub path: String,
    /// What `--version` printed, its first word that starts with a digit; None when it did
    /// not answer in time.
    #[serde(default)]
    pub version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Extension {
    /// The editor, as its folder names it: `vscode`, `cursor`, ...
    pub editor: String,
    pub version: String,
}

/// Where a machine's clients are looked for: what the agent's environment says.
pub struct Places {
    pub home: PathBuf,
    /// `PATH`, as the agent got it.
    pub path: Option<OsString>,
    /// `KIMI_CODE_HOME`, when set.
    pub kimi_home: Option<PathBuf>,
    /// Where each client's command may be, after `PATH`: a service's `PATH` is short, and
    /// the installers put the command in these.
    pub installed: Vec<PathBuf>,
}

impl Places {
    pub fn here() -> Option<Self> {
        let home = crate::setup::home_dir().ok()?;
        Some(Self {
            installed: installed(&home),
            path: std::env::var_os("PATH"),
            kimi_home: std::env::var_os("KIMI_CODE_HOME").filter(|h| !h.is_empty()).map(PathBuf::from),
            home,
        })
    }
}

/// The folders npm, pnpm, yarn, bun, Homebrew and the native installers put a command in.
fn installed(h: &Path) -> Vec<PathBuf> {
    let mut dirs = vec![
        h.join(".local/bin"),
        h.join(".npm-global/bin"),
        h.join(".bun/bin"),
        h.join(".yarn/bin"),
        h.join("Library/pnpm"),
        h.join(".local/share/pnpm"),
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
    ];
    if cfg!(windows) {
        dirs.push(h.join("AppData/Roaming/npm"));
    }
    dirs
}

/// The editors whose extensions folders are read, by the folder under the home.
const EDITORS: &[(&str, &str)] = &[
    ("vscode", ".vscode/extensions"),
    ("vscode-insiders", ".vscode-insiders/extensions"),
    ("vscode-server", ".vscode-server/extensions"),
    ("cursor", ".cursor/extensions"),
    ("windsurf", ".windsurf/extensions"),
    ("vscodium", ".vscode-oss/extensions"),
];

/// This machine's clients. Blocking: it reads folders and may run each command once.
pub fn here() -> Vec<Client> {
    Places::here().map(|at| find(&at)).unwrap_or_default()
}

/// The clients found at `at`, Claude Code first; a client with none of its parts is left out.
pub fn find(at: &Places) -> Vec<Client> {
    let claude_home = at.home.join(".claude");
    let kimi_home = at.kimi_home.clone().unwrap_or_else(|| at.home.join(".kimi-code"));
    let claude = Client {
        client: CLAUDE_CODE.into(),
        cli: command(at, "claude", &[at.home.join(".claude/local")]),
        extensions: extensions(&at.home, "anthropic.claude-code-"),
        home: folder(&claude_home),
    };
    let kimi = Client {
        client: KIMI_CODE.into(),
        // Its own installer's first: the command it updates in place.
        cli: command(at, "kimi", &[kimi_home.join("bin")]),
        extensions: extensions(&at.home, "moonshot-ai.kimi-code-"),
        home: folder(&kimi_home),
    };
    [claude, kimi].into_iter().filter(|c| c.cli.is_some() || !c.extensions.is_empty() || c.home.is_some()).collect()
}

fn folder(dir: &Path) -> Option<String> {
    dir.is_dir().then(|| dir.display().to_string())
}

/// The command `name`, from `first`, then `PATH`, then where installers put it.
fn command(at: &Places, name: &str, first: &[PathBuf]) -> Option<Cli> {
    let on_path = at.path.as_ref().map(|p| std::env::split_paths(p).collect::<Vec<_>>()).unwrap_or_default();
    let found = first.iter().chain(&on_path).chain(&at.installed).find_map(|dir| runnable(dir, name))?;
    let version = version_of(&found);
    Some(Cli { path: found.display().to_string(), version })
}

fn runnable(dir: &Path, name: &str) -> Option<PathBuf> {
    let names: &[&str] = if cfg!(windows) { &["exe", "cmd"] } else { &[""] };
    names.iter().map(|ext| dir.join(name).with_extension(ext)).find(|p| is_runnable(p))
}

#[cfg(unix)]
fn is_runnable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_runnable(p: &Path) -> bool {
    p.is_file()
}

/// How long a command has to say its version.
const VERSION_WAIT: Duration = Duration::from_secs(5);

/// Versions already read, by the command's path, size and time: a command is asked once per
/// install, not every minute.
static VERSIONS: Mutex<Option<HashMap<(PathBuf, u64, Option<SystemTime>), Option<String>>>> = Mutex::new(None);

fn version_of(cli: &Path) -> Option<String> {
    let meta = std::fs::metadata(cli).ok()?;
    let key = (cli.to_path_buf(), meta.len(), meta.modified().ok());
    if let Some(known) = VERSIONS.lock().unwrap_or_else(|p| p.into_inner()).get_or_insert_with(HashMap::new).get(&key) {
        return known.clone();
    }
    let read = ask_version(cli);
    VERSIONS.lock().unwrap_or_else(|p| p.into_inner()).get_or_insert_with(HashMap::new).insert(key, read.clone());
    read
}

fn ask_version(cli: &Path) -> Option<String> {
    let mut cmd = std::process::Command::new(cli);
    cmd.arg("--version");
    // Kimi Code checks for an update when it starts; its version needs no such thing.
    cmd.env("KIMI_CODE_NO_AUTO_UPDATE", "1");
    cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = cmd.spawn().ok()?;
    let deadline = Instant::now() + VERSION_WAIT;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let out = child.wait_with_output().ok()?;
    version_in(&String::from_utf8_lossy(&out.stdout))
}

/// `2.1.267 (Claude Code)` → `2.1.267`; `kimi, version 2.1.1` → `2.1.1`.
fn version_in(said: &str) -> Option<String> {
    said.split(|c: char| c.is_whitespace() || c == ',')
        .map(|w| w.trim_start_matches('v'))
        .find(|w| w.starts_with(|c: char| c.is_ascii_digit()) && w.contains('.'))
        .map(str::to_owned)
}

/// The newest extension named `prefix…` in each editor.
fn extensions(home: &Path, prefix: &str) -> Vec<Extension> {
    EDITORS
        .iter()
        .filter_map(|(editor, dir)| {
            let newest = std::fs::read_dir(home.join(dir))
                .ok()?
                .flatten()
                .filter(|e| e.path().is_dir())
                .filter_map(|e| {
                    let name = e.file_name().to_string_lossy().into_owned();
                    let rest = name.strip_prefix(prefix)?;
                    extension_version(&e.path()).or_else(|| rest.split('-').next().map(str::to_owned))
                })
                .max_by(|a, b| numbers(a).cmp(&numbers(b)))?;
            Some(Extension { editor: (*editor).into(), version: newest })
        })
        .collect()
}

fn extension_version(dir: &Path) -> Option<String> {
    let manifest: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.join("package.json")).ok()?).ok()?;
    manifest.get("version")?.as_str().map(str::to_owned)
}

/// A version's numbers, for ordering: `2.1.296` after `2.1.30`.
fn numbers(v: &str) -> Vec<u64> {
    v.split(['.', '-']).map_while(|n| n.parse().ok()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ar-clients-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[cfg(unix)]
    fn script(path: &Path, says: &str) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, format!("#!/bin/sh\necho '{says}'\n")).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn extension(home: &Path, editor_dir: &str, folder: &str, version: Option<&str>) {
        let dir = home.join(editor_dir).join(folder);
        std::fs::create_dir_all(&dir).unwrap();
        if let Some(v) = version {
            std::fs::write(dir.join("package.json"), format!(r#"{{"name":"x","version":"{v}"}}"#)).unwrap();
        }
    }

    /// A home with both clients, each as a command, an extension and a home folder, gives
    /// both with their versions; an empty home gives neither.
    #[cfg(unix)]
    #[test]
    fn the_agent_finds_claude_code_and_kimi_code() {
        let h = home("both");
        let bin = h.join("tools/bin");
        script(&bin.join("claude"), "2.1.267 (Claude Code)");
        script(&h.join(".kimi-code/bin/kimi"), "2.1.1");
        std::fs::create_dir_all(h.join(".claude")).unwrap();
        extension(&h, ".vscode/extensions", "anthropic.claude-code-2.1.30-darwin-arm64", Some("2.1.30"));
        extension(&h, ".vscode/extensions", "anthropic.claude-code-2.1.296-darwin-arm64", Some("2.1.296"));
        extension(&h, ".cursor/extensions", "anthropic.claude-code-2.1.290-linux-x64", None);
        extension(&h, ".vscode/extensions", "moonshot-ai.kimi-code-0.8.1-darwin-arm64", Some("0.8.1"));
        extension(&h, ".vscode/extensions", "ms-python.python-2026.1.0", Some("2026.1.0"));

        // The folders a machine shares with every home are not this test's.
        let at = Places { home: h.clone(), path: Some(bin.clone().into_os_string()), kimi_home: None, installed: Vec::new() };
        let found = find(&at);
        assert_eq!(found.len(), 2, "{found:?}");
        let (claude, kimi) = (&found[0], &found[1]);

        assert_eq!(claude.client, CLAUDE_CODE);
        let cli = claude.cli.as_ref().expect("claude on PATH");
        assert_eq!(cli.path, bin.join("claude").display().to_string());
        assert_eq!(cli.version.as_deref(), Some("2.1.267"));
        assert_eq!(claude.home, Some(h.join(".claude").display().to_string()));
        assert_eq!(
            claude.extensions,
            vec![
                Extension { editor: "vscode".into(), version: "2.1.296".into() },
                Extension { editor: "cursor".into(), version: "2.1.290".into() },
            ],
            "the newest in each editor, from its manifest or else its folder"
        );

        assert_eq!(kimi.client, KIMI_CODE);
        let cli = kimi.cli.as_ref().expect("Kimi Code's own install, off PATH");
        assert_eq!(cli.path, h.join(".kimi-code/bin/kimi").display().to_string());
        assert_eq!(cli.version.as_deref(), Some("2.1.1"));
        assert_eq!(kimi.home, Some(h.join(".kimi-code").display().to_string()));
        assert_eq!(kimi.extensions, vec![Extension { editor: "vscode".into(), version: "0.8.1".into() }]);

        // KIMI_CODE_HOME moves Kimi Code's home, and its command with it.
        let moved = h.join("elsewhere");
        std::fs::rename(h.join(".kimi-code"), &moved).unwrap();
        let at = Places { home: h.clone(), path: Some(bin.into_os_string()), kimi_home: Some(moved.clone()), installed: Vec::new() };
        let kimi = find(&at).into_iter().find(|c| c.client == KIMI_CODE).unwrap();
        assert_eq!(kimi.home, Some(moved.display().to_string()));
        assert_eq!(kimi.cli.unwrap().path, moved.join("bin/kimi").display().to_string());

        let empty = home("neither");
        // Every folder under the home an installer uses, none of the machine's own.
        let under_home = installed(&empty).into_iter().filter(|d| d.starts_with(&empty)).collect();
        let at = Places { home: empty.clone(), path: Some(empty.join("bin").into_os_string()), kimi_home: None, installed: under_home };
        let none = find(&at);
        assert!(none.is_empty(), "a home with neither finds neither: {none:?}");

        let _ = std::fs::remove_dir_all(h);
        let _ = std::fs::remove_dir_all(empty);
    }

    #[test]
    fn a_version_is_read_from_what_the_command_prints() {
        assert_eq!(version_in("2.1.267 (Claude Code)\n").as_deref(), Some("2.1.267"));
        assert_eq!(version_in("kimi, version 2.1.1").as_deref(), Some("2.1.1"));
        assert_eq!(version_in("v1.2.3").as_deref(), Some("1.2.3"));
        assert_eq!(version_in("usage: kimi [options]"), None);
    }
}
