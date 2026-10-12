//! Kimi Code as a client of the router, beside Claude Code (docs/kimi-client.md): the router's
//! entries in its `config.toml` ([`config`]), and what its hooks run (`agent-router hook
//! kimi`), which keeps the [`registry`] of its live sessions.

pub mod config;
pub mod launch;
pub mod live;
pub mod registry;
pub mod session;
pub mod web;

use std::path::{Path, PathBuf};

/// Kimi Code's home: `KIMI_CODE_HOME` when it is set, else `~/.kimi-code`. The CLI and the
/// VS Code extension share it.
pub fn home_in(home: &Path, set: Option<PathBuf>) -> PathBuf {
    set.unwrap_or_else(|| home.join(".kimi-code"))
}

/// This machine's Kimi Code home, if Kimi Code has made one.
pub fn home() -> Option<PathBuf> {
    let set = std::env::var_os("KIMI_CODE_HOME").filter(|h| !h.is_empty()).map(PathBuf::from);
    Some(home_in(&crate::setup::home_dir().ok()?, set)).filter(|h| h.is_dir())
}

/// `agent-router hook kimi`: what each of the router's hooks in Kimi Code's `config.toml`
/// runs, with the event's JSON on stdin, kept in the [`registry`]. It never fails and never
/// answers: a hook that exits non-zero or prints may block or decide what it hooks, and these
/// only report.
pub fn hook(args: &[String]) -> anyhow::Result<()> {
    if args.first().map(String::as_str) != Some("kimi") {
        return Ok(());
    }
    let mut payload = Vec::new();
    let _ = std::io::Read::read_to_end(&mut std::io::stdin(), &mut payload);
    if let (Ok(payload), Ok(dir)) = (serde_json::from_slice(&payload), registry::dir()) {
        let _ = registry::heard(&dir, &payload, crate::db::now_ms());
    }
    Ok(())
}
