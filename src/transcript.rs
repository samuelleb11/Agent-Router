//! Session → project resolution.
//!
//! Claude Code labels every API request with `x-claude-code-session-id`, and stores
//! that session's log at `~/.claude/projects/<cwd-slug>/<session-uuid>.jsonl`. So the
//! session id alone resolves to a working directory: find the file, read the first
//! event that carries `cwd`.
//!
//! The directory name is a slug of the path with `/` and `.` flattened to `-`, which
//! is lossy — `/a/b.c` and `/a-b-c` slug identically. So the cwd is read from the
//! file's contents rather than un-slugged from the directory name.
//!
//! This module is READ-ONLY. It never writes to Claude Code's files.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::Deserialize;

#[derive(Debug, Clone, Default)]
pub struct SessionInfo {
    pub cwd: Option<String>,
    pub git_branch: Option<String>,
}

#[derive(Deserialize)]
struct Event {
    #[serde(default)]
    cwd: Option<String>,
    #[serde(rename = "gitBranch", default)]
    git_branch: Option<String>,
}

pub struct Resolver {
    root: PathBuf,
    cache: HashMap<String, SessionInfo>,
}

impl Resolver {
    pub fn new() -> Self {
        let root = crate::registry::claude_dir().join("projects");
        Self { root, cache: HashMap::new() }
    }

    /// Resolve a session id to its project. Cached: a hit costs nothing, and a miss
    /// is retried on the next request, since the transcript may not exist yet when
    /// the very first request of a session goes out.
    pub fn resolve(&mut self, session_id: &str) -> SessionInfo {
        if let Some(hit) = self.cache.get(session_id) {
            if hit.cwd.is_some() {
                return hit.clone();
            }
        }
        let info = self.read(session_id).unwrap_or_default();
        if info.cwd.is_some() {
            self.cache.insert(session_id.to_string(), info.clone());
        }
        info
    }

    fn read(&self, session_id: &str) -> Option<SessionInfo> {
        let path = self.find_transcript(session_id)?;
        let content = std::fs::read_to_string(&path).ok()?;
        let mut info = SessionInfo::default();
        for line in content.lines() {
            let Ok(ev) = serde_json::from_str::<Event>(line) else {
                continue;
            };
            if info.cwd.is_none() {
                info.cwd = ev.cwd;
            }
            if info.git_branch.is_none() {
                info.git_branch = ev.git_branch;
            }
            if info.cwd.is_some() && info.git_branch.is_some() {
                break;
            }
        }
        Some(info)
    }

    fn find_transcript(&self, session_id: &str) -> Option<PathBuf> {
        // Reject anything that is not a plain UUID before it reaches the filesystem:
        // the session id arrives in an HTTP header, so it is attacker-controlled and
        // must never be able to escape the projects directory.
        if !is_uuid_like(session_id) {
            return None;
        }
        let name = format!("{session_id}.jsonl");
        for entry in std::fs::read_dir(&self.root).ok()? {
            let dir = entry.ok()?.path();
            let candidate = dir.join(&name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
        None
    }
}

fn is_uuid_like(s: &str) -> bool {
    s.len() == 36
        && s.bytes()
            .enumerate()
            .all(|(i, b)| match i {
                8 | 13 | 18 | 23 => b == b'-',
                _ => b.is_ascii_hexdigit(),
            })
}

#[cfg(test)]
mod tests {
    use super::is_uuid_like;

    #[test]
    fn accepts_real_session_ids() {
        assert!(is_uuid_like("6f708192-0000-4000-8000-00000000000f"));
    }

    #[test]
    fn rejects_path_traversal_and_junk() {
        for s in [
            "../../../../etc/passwd",
            "..%2f..%2fetc",
            "6f708192-0000-4000-8000-00000000000/",
            "",
            "not-a-uuid",
            "6f70819200004000800000000000000f",
        ] {
            assert!(!is_uuid_like(s), "should reject {s:?}");
        }
    }
}
