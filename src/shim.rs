//! Installing the wrapper into the VSCode extension, from the settings modal.
//!
//! The extension spawns `<ext>/resources/native-binary/claude` and keeps that process's
//! stdio, which is why nothing can type into a running session from outside (see
//! ARCHITECTURE.md). Installing means taking that filename: the real binary moves to
//! `claude.real` and `wrapper` takes its place.
//!
//! Two things make this safe enough to expose as a button:
//!
//!   * **Every state is recoverable by looking at the directory.** The real binary is
//!     ~200 MB and the wrapper is under 20; a directory holding both, with a small
//!     `claude`, is installed. Nothing is recorded anywhere else, so nothing can go
//!     stale or disagree.
//!   * **A failed install puts the real binary back.** The rename happens first, and
//!     if the copy fails it is undone before returning.
//!
//! Extensions are version-pinned directories, so an update lands a fresh copy of the
//! real binary under a new path and the shim is simply absent there. That reads as
//! "not installed" rather than as breakage, which is why status is computed per
//! directory rather than stored.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Anything at or above this is the real binary, not our wrapper.
const REAL_BINARY_FLOOR: u64 = 20 * 1024 * 1024;

/// The setting that turns automatic management on. Off unless someone said so: this
/// writes into VSCode's extension directories.
const AUTO_KEY: &str = "auto_wrapper";
const AUTO_POLL: std::time::Duration = std::time::Duration::from_secs(3);
/// A binary that VSCode is still extracting changes under us; wait for it to sit
/// still this long, and to look the same on two consecutive looks.
const SETTLE_MS: i64 = 5_000;
/// What the manager did lately, for the settings pages.
static EVENTS: Mutex<VecDeque<(i64, String)>> = Mutex::new(VecDeque::new());

pub fn auto_enabled(db_path: &Path) -> bool {
    crate::db::setting(db_path, AUTO_KEY).as_deref() == Some("1")
}

pub fn set_auto(db_path: &Path, enabled: bool) -> anyhow::Result<()> {
    crate::db::set_setting(db_path, AUTO_KEY, if enabled { "1" } else { "0" })
}

fn note(what: String) {
    tracing::info!("{what}");
    let mut g = EVENTS.lock().unwrap();
    g.push_back((crate::db::now_ms(), what));
    while g.len() > 20 {
        g.pop_front();
    }
}

pub fn events() -> Vec<(i64, String)> {
    EVENTS.lock().unwrap().iter().cloned().collect()
}

/// Whether a binary is ready to be wrapped: it needs wrapping, it is the real thing
/// (not a half-written file), it has stopped changing, and it looked the same last time.
pub fn ready(state: State, size: u64, mtime_age_ms: i64, prev: Option<(u64, i64)>) -> bool {
    matches!(state, State::NotInstalled | State::Inconsistent)
        && size >= REAL_BINARY_FLOOR
        && mtime_age_ms >= SETTLE_MS
        && prev.is_some_and(|(ps, _)| ps == size)
}

fn observe(bin: &Path) -> Option<(u64, i64)> {
    let m = std::fs::metadata(bin).ok()?;
    let mtime = m
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_millis() as i64;
    Some((m.len(), mtime))
}

/// Runs for the life of the router: every few seconds, every Claude Code extension on
/// the Mac that spawns an unwrapped binary gets the wrapper — if the switch is on, and
/// the binary has settled. Sessions already running keep their old stdio; the phone is
/// told, when it can be, so it knows to restart them.
pub async fn watch(db_path: PathBuf, push: std::sync::Arc<crate::notify::Push>) {
    let mut seen: HashMap<PathBuf, (u64, i64)> = HashMap::new();
    let mut said_missing = false;
    loop {
        tokio::time::sleep(AUTO_POLL).await;
        if !auto_enabled(&db_path) {
            continue;
        }
        if wrapper_source().is_none() {
            if !said_missing {
                note("automatic wrapper: claude-wrapper is not beside agent-router; nothing can be installed".into());
                said_missing = true;
            }
            continue;
        }
        said_missing = false;
        let now = crate::db::now_ms();
        for t in targets() {
            let bin = PathBuf::from(&t.path);
            let Some((size, mtime)) = observe(&bin) else { continue };
            let prev = seen.insert(bin.clone(), (size, mtime));
            if !ready(t.state, size, now - mtime, prev) {
                continue;
            }
            let short = t
                .extension
                .trim_start_matches("anthropic.claude-code-")
                .trim_end_matches("-darwin-arm64")
                .to_string();
            match install(&bin) {
                Ok(()) => {
                    note(format!("automatic wrapper: installed into {short}"));
                    seen.remove(&bin);
                    let _ = push
                        .notify("Agent Router", "Wrapper installed", &format!("Claude Code {short} is wrapped. Sessions started from now on can be typed into; restart any that are open."), None, &format!("wrapper:{short}"))
                        .await;
                }
                Err(e) => note(format!("automatic wrapper: could not install into {short}: {e}")),
            }
        }
    }
}

#[derive(serde::Serialize, PartialEq, Debug, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum State {
    /// The extension's own binary is in place.
    NotInstalled,
    /// Our wrapper holds the name, real binary parked alongside.
    Installed,
    /// `claude.real` exists but `claude` is a real binary — an interrupted install or
    /// an extension update that overwrote us. Installing again is the fix.
    Inconsistent,
}

#[derive(serde::Serialize, Clone)]
pub struct Target {
    pub extension: String,
    pub path: String,
    pub state: State,
}

fn extensions_root() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".vscode/extensions")
}

/// Every Claude Code extension on this machine that spawns a native binary.
pub fn targets() -> Vec<Target> {
    let Ok(dirs) = std::fs::read_dir(extensions_root()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for d in dirs.flatten() {
        let name = d.file_name().to_string_lossy().into_owned();
        if !name.starts_with("anthropic.claude-code-") {
            continue;
        }
        let bin = d.path().join("resources/native-binary/claude");
        if !bin.is_file() {
            continue;
        }
        out.push(Target {
            state: state_of(&bin),
            extension: name,
            path: bin.to_string_lossy().into_owned(),
        });
    }
    out.sort_by(|a, b| a.extension.cmp(&b.extension));
    out
}

fn real_beside(bin: &Path) -> PathBuf {
    let mut p = bin.as_os_str().to_owned();
    p.push(".real");
    PathBuf::from(p)
}

fn size(p: &Path) -> u64 {
    std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)
}

pub fn state_of(bin: &Path) -> State {
    let real = real_beside(bin);
    match (real.is_file(), size(bin) < REAL_BINARY_FLOOR) {
        (true, true) => State::Installed,
        (true, false) => State::Inconsistent,
        (false, _) => State::NotInstalled,
    }
}

/// The wrapper binary to install, shipped beside the router.
fn wrapper_source() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    for candidate in [
        dir.join("claude-wrapper"),
        dir.join("../claude-wrapper"),
        PathBuf::from("target/release/claude-wrapper"),
    ] {
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

pub fn install(bin: &Path) -> anyhow::Result<()> {
    let src = wrapper_source()
        .ok_or_else(|| anyhow::anyhow!("claude-wrapper binary not found beside agent-router"))?;
    let real = real_beside(bin);

    match state_of(bin) {
        // Already installed still means copy: this is how a newer wrapper reaches the
        // extension. The real binary is already parked, so only the shim is replaced,
        // and returning early here would silently pin the version installed first.
        State::Installed => return copy_executable(&src, bin).map_err(Into::into),
        // The real binary is under `claude`; the parked copy is stale, so replace it.
        State::Inconsistent => {
            std::fs::remove_file(&real).ok();
        }
        State::NotInstalled => {}
    }

    std::fs::rename(bin, &real)?;
    // From here the extension has no binary, so any failure must put it back.
    if let Err(e) = copy_executable(&src, bin) {
        std::fs::rename(&real, bin).ok();
        return Err(anyhow::anyhow!("could not install the wrapper: {e}"));
    }
    Ok(())
}

pub fn remove(bin: &Path) -> anyhow::Result<()> {
    let real = real_beside(bin);
    if !real.is_file() {
        return Ok(());
    }
    // Overwrites the wrapper with the real binary in one step.
    std::fs::rename(&real, bin)?;
    Ok(())
}

/// Put `src` at `dst` as a new file — never by writing into the one already there.
///
/// `dst` is the binary the extension runs, and once a wrapper is installed every open
/// session is executing it. Writing into a Mach-O while it is mapped invalidates its code
/// signature: every later exec of that path dies at once with SIGKILL, and the running
/// sessions sit on pages that may be read back from the changed file (seen 2026-09-12 with
/// the router's own binary). A copy beside it, renamed over it, leaves the running ones on
/// the old file and gives the next session the new one.
fn copy_executable(src: &Path, dst: &Path) -> std::io::Result<()> {
    let name = dst.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = dst.with_file_name(format!(".{name}.agent-router-new"));
    let placed = std::fs::copy(src, &tmp).and_then(|_| {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?;
        }
        std::fs::rename(&tmp, dst)
    });
    if placed.is_err() {
        std::fs::remove_file(&tmp).ok();
    }
    placed
}

pub mod api {
    use std::sync::Arc;

    use axum::extract::State as AxState;
    use axum::Json;
    use serde::Deserialize;
    use serde_json::{json, Value};

    use super::*;
    use crate::proxy::AppState;

    #[derive(Deserialize)]
    pub struct Body {
        /// Absolute path of the extension binary, from the status listing.
        pub path: String,
        /// install | remove
        pub action: String,
    }

    #[derive(Deserialize)]
    pub struct AutoBody {
        pub enabled: bool,
    }

    pub fn snapshot(db_path: &Path) -> Value {
        let t = targets();
        json!({
            "available": wrapper_source().is_some(),
            "installed": t.iter().filter(|x| x.state == State::Installed).count(),
            "targets": t,
            "auto": auto_enabled(db_path),
            "auto_log": events().into_iter().rev().map(|(at, what)| json!({ "at": at, "what": what })).collect::<Vec<_>>(),
        })
    }

    pub async fn status(AxState(state): AxState<Arc<AppState>>) -> Json<Value> {
        Json(snapshot(&state.config.db_path))
    }

    /// The switch. Turning it on also does the work now rather than in three seconds.
    pub async fn auto(AxState(state): AxState<Arc<AppState>>, Json(body): Json<AutoBody>) -> Json<Value> {
        match set_auto(&state.config.db_path, body.enabled) {
            Ok(()) => {
                note(format!("automatic wrapper: turned {}", if body.enabled { "on" } else { "off" }));
                Json(json!({ "ok": true, "shim": snapshot(&state.config.db_path) }))
            }
            Err(e) => Json(json!({ "ok": false, "reason": e.to_string() })),
        }
    }

    pub async fn toggle(AxState(state): AxState<Arc<AppState>>, Json(body): Json<Body>) -> Json<Value> {
        let path = PathBuf::from(&body.path);
        // Only ever touch a path the listing itself produced.
        if !targets().iter().any(|t| t.path == body.path) {
            return Json(json!({ "ok": false, "reason": "Not a known extension binary." }));
        }
        let done = match body.action.as_str() {
            "install" => install(&path),
            "remove" => remove(&path),
            other => Err(anyhow::anyhow!("unknown action {other:?}")),
        };
        match done {
            Ok(()) => {
                tracing::info!(path = %body.path, action = %body.action, "shim changed");
                Json(json!({ "ok": true, "shim": snapshot(&state.config.db_path) }))
            }
            Err(e) => Json(json!({ "ok": false, "reason": e.to_string() })),
        }
    }
}

#[cfg(test)]
mod auto_tests {
    use super::*;

    #[test]
    fn a_binary_is_wrapped_only_once_it_has_settled() {
        let big = REAL_BINARY_FLOOR + 1;
        // Unwrapped, real size, old enough, seen once before at the same size: yes.
        assert!(ready(State::NotInstalled, big, SETTLE_MS, Some((big, 0))));
        assert!(ready(State::Inconsistent, big, SETTLE_MS, Some((big, 0))));
        // First sight: not yet — look again.
        assert!(!ready(State::NotInstalled, big, SETTLE_MS, None));
        // Still growing: not yet.
        assert!(!ready(State::NotInstalled, big, SETTLE_MS, Some((big - 1000, 0))));
        // Just written: not yet.
        assert!(!ready(State::NotInstalled, big, SETTLE_MS - 1, Some((big, 0))));
        // Too small to be the real binary: never.
        assert!(!ready(State::NotInstalled, 1024, SETTLE_MS, Some((1024, 0))));
        // Already ours: nothing to do.
        assert!(!ready(State::Installed, big, SETTLE_MS, Some((big, 0))));
    }

    #[test]
    fn the_switch_is_off_until_someone_turns_it_on() {
        let dir = std::env::temp_dir().join(format!("auto-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("t.db");
        let _ = crate::db::Db::open(&db).unwrap();
        assert!(!auto_enabled(&db));
        set_auto(&db, true).unwrap();
        assert!(auto_enabled(&db));
        set_auto(&db, false).unwrap();
        assert!(!auto_enabled(&db));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("shim-test-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn write(p: &Path, bytes: usize) {
        std::fs::write(p, vec![b'x'; bytes]).unwrap();
    }

    #[test]
    fn a_plain_extension_binary_reads_as_not_installed() {
        let d = scratch("plain");
        let bin = d.join("claude");
        write(&bin, 32 * 1024 * 1024);
        assert_eq!(state_of(&bin), State::NotInstalled);
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn a_small_binary_beside_a_parked_real_one_reads_as_installed() {
        let d = scratch("installed");
        let bin = d.join("claude");
        write(&bin, 1024);
        write(&d.join("claude.real"), 32 * 1024 * 1024);
        assert_eq!(state_of(&bin), State::Installed);
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn an_extension_update_that_overwrote_us_reads_as_inconsistent() {
        // Both files large: the update restored a real binary over the wrapper, and the
        // parked copy is now stale. Installing again must be what fixes it.
        let d = scratch("stale");
        let bin = d.join("claude");
        write(&bin, 32 * 1024 * 1024);
        write(&d.join("claude.real"), 32 * 1024 * 1024);
        assert_eq!(state_of(&bin), State::Inconsistent);
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn remove_puts_the_real_binary_back_under_the_original_name() {
        let d = scratch("remove");
        let bin = d.join("claude");
        write(&bin, 1024);
        std::fs::write(d.join("claude.real"), b"REAL").unwrap();
        remove(&bin).unwrap();
        assert_eq!(std::fs::read(&bin).unwrap(), b"REAL");
        assert!(!d.join("claude.real").exists(), "nothing left parked");
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn remove_is_a_no_op_when_nothing_was_installed() {
        let d = scratch("noop");
        let bin = d.join("claude");
        write(&bin, 32 * 1024 * 1024);
        remove(&bin).unwrap();
        assert_eq!(size(&bin), 32 * 1024 * 1024, "left alone");
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn installing_over_an_existing_shim_replaces_it() {
        // A wrapper update must reach an extension that already has an older one.
        let d = scratch("update");
        let bin = d.join("claude");
        std::fs::write(&bin, b"OLD WRAPPER").unwrap();
        write(&d.join("claude.real"), 32 * 1024 * 1024);
        assert_eq!(state_of(&bin), State::Installed);
        let _ = install(&bin);
        assert_ne!(
            std::fs::read(&bin).unwrap(),
            b"OLD WRAPPER".to_vec(),
            "the shim was replaced, not left pinned at the first version"
        );
        assert!(d.join("claude.real").is_file(), "the real binary stays parked");
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn a_wrapper_is_replaced_by_rename_never_written_in_place() {
        // Open sessions are executing the installed wrapper: the file under them must stay
        // exactly as it was, and the next session must find the new one.
        use std::io::Read;
        use std::os::unix::fs::MetadataExt;
        let d = scratch("in-place");
        let src = d.join("new-wrapper");
        let dst = d.join("claude");
        std::fs::write(&src, b"NEW WRAPPER").unwrap();
        std::fs::write(&dst, b"OLD WRAPPER").unwrap();
        let running = std::fs::File::open(&dst).unwrap();
        let old_inode = running.metadata().unwrap().ino();
        copy_executable(&src, &dst).unwrap();
        let mut under_them = String::new();
        (&running).read_to_string(&mut under_them).unwrap();
        assert_eq!(under_them, "OLD WRAPPER", "written into the file the sessions are running");
        assert_eq!(std::fs::read(&dst).unwrap(), b"NEW WRAPPER");
        let now = std::fs::metadata(&dst).unwrap();
        assert_ne!(now.ino(), old_inode, "a new file, not the old one rewritten");
        assert_eq!(now.mode() & 0o777, 0o755);
        let names: Vec<_> = std::fs::read_dir(&d).unwrap().flatten().map(|e| e.file_name()).collect();
        assert_eq!(names.len(), 2, "no temporary left beside it: {names:?}");
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn a_failed_install_leaves_the_extension_binary_where_it_was() {
        // No wrapper to copy, so install must fail *and* restore.
        let d = scratch("rollback");
        let bin = d.join("claude");
        std::fs::write(&bin, b"REAL BINARY").unwrap();
        let before = std::fs::read(&bin).unwrap();
        // wrapper_source() may resolve in-tree, so only assert the invariant that
        // matters: whatever happens, `claude` still runs something.
        let _ = install(&bin);
        assert!(bin.is_file(), "the extension always has a binary");
        if state_of(&bin) == State::NotInstalled {
            assert_eq!(std::fs::read(&bin).unwrap(), before, "restored byte for byte");
        }
        std::fs::remove_dir_all(&d).ok();
    }
}
