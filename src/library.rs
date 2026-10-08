//! The Library: reusable agent components a hosted router keeps, and every connected
//! machine's agent places where Claude Code reads them — skills, slash commands and
//! subagents to begin with. The plan, and the owner's decisions behind it, are
//! docs/library.md (Northstar's AGEN-0042).
//!
//! - [`bundle`] is what one version is: its files, checked against its kind, packed in one
//!   canonical form whose sha256 is the version's digest.
//! - [`store`] keeps the items, their versions and the blobs, in the main store.
//! - [`api`] is how a hosted router serves them, to the console and to the machines' agents,
//!   and [`push`] how it tells those agents to sync now, through their own tunnels.
//! - [`sources`] imports from repositories on [`forge`]s, by a poller and on demand.
//! - [`sync`] is the agent's half: keeping a machine's `~/.claude` in step with them, and
//!   [`cli`] is `agent-router library` on that machine.

pub mod api;
pub mod bundle;
pub mod cli;
pub mod forge;
pub mod push;
pub mod sources;
pub mod store;
pub mod sync;

use std::fmt;

use serde::{Deserialize, Serialize};

/// What an item is. It decides what a version must hold and where a machine puts it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// A directory with a `SKILL.md`, placed as `~/.claude/skills/<name>/`.
    Skill,
    /// One Markdown file, placed as `~/.claude/commands/<name>.md`.
    Command,
    /// One Markdown file with its frontmatter, placed as `~/.claude/agents/<name>.md`.
    Subagent,
}

impl Kind {
    pub const ALL: [Kind; 3] = [Kind::Skill, Kind::Command, Kind::Subagent];

    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Skill => "skill",
            Kind::Command => "command",
            Kind::Subagent => "subagent",
        }
    }

    pub fn parse(s: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.as_str() == s)
    }
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Why something cannot go into the Library, in words the person who sent it can act on.
/// The API answers it as a 422 (`refusal.rs`): it is never the router failing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refused(pub String);

impl fmt::Display for Refused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Refused {}

/// The longest name an item may have.
pub const NAME_MAX: usize = 64;

/// An item's name: lowercase letters and digits, with single hyphens between them, at most
/// [`NAME_MAX`] characters. It becomes a directory or a file name on every machine, Windows
/// included, and the name Claude Code lists a skill under, so nothing else is let through.
pub fn check_name(name: &str) -> Result<(), Refused> {
    let ok = !name.is_empty()
        && name.len() <= NAME_MAX
        && name
            .split('-')
            .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit()));
    if ok {
        Ok(())
    } else {
        Err(Refused(format!(
            "\"{name}\" cannot name an item: use lowercase letters and digits, with single hyphens \
             between them, at most {NAME_MAX} characters"
        )))
    }
}
