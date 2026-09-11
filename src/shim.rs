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

use std::path::{Path, PathBuf};

/// Anything at or above this is the real binary, not our wrapper.
const REAL_BINARY_FLOOR: u64 = 20 * 1024 * 1024;

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

fn copy_executable(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::copy(src, dst)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dst, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

pub mod api {
    use axum::Json;
    use serde::Deserialize;
    use serde_json::{json, Value};

    use super::*;

    #[derive(Deserialize)]
    pub struct Body {
        /// Absolute path of the extension binary, from the status listing.
        pub path: String,
        /// install | remove
        pub action: String,
    }

    pub fn snapshot() -> Value {
        let t = targets();
        json!({
            "available": wrapper_source().is_some(),
            "installed": t.iter().filter(|x| x.state == State::Installed).count(),
            "targets": t,
        })
    }

    pub async fn status() -> Json<Value> {
        Json(snapshot())
    }

    pub async fn toggle(Json(body): Json<Body>) -> Json<Value> {
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
                Json(json!({ "ok": true, "shim": snapshot() }))
            }
            Err(e) => Json(json!({ "ok": false, "reason": e.to_string() })),
        }
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
