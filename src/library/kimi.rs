//! The Library on Kimi Code (AGEN-0114): what of it Kimi Code takes, and in what form. Its
//! skills are Claude Code's shape, a `SKILL.md` and what is beside it, and are placed as they
//! are, in `$KIMI_CODE_HOME/skills/`. Its subagents are converted into Kimi Code's own agent
//! files, in `$KIMI_CODE_HOME/agents/`:
//! - `name` kept: the Library's names are already the kebab-case Kimi Code wants;
//! - `description` kept, and `whenToUse` said from it, which is what Kimi Code shows the
//!   model when it picks one;
//! - `tools` mapped to Kimi Code's names, `*` (every tool) when Claude Code's file names none;
//! - the body, the prompt, as it is.
//!
//! Kimi Code has no folder for commands: they reach it in a plugin of the router's,
//! `agent-router` (AGEN-0115). The sync places each command as it is, a Markdown file whose
//! `$ARGUMENTS` Kimi Code expands as Claude Code does, in the plugin's `commands/`; then
//! [`keep_plugin`] writes the plugin's manifest and registers it in
//! `$KIMI_CODE_HOME/plugins/installed.json`, beside the owner's own plugins.
//!
//! The forms were read from Kimi Code 2.1.1's own loaders:
//! - an agent's `name` must be kebab-case, `description` is required, `tools` and
//!   `disallowedTools` are a comma-separated string or a list, and the prompt must not be
//!   empty;
//! - a plugin is a directory with a `kimi.plugin.json` (`name`, `version`, `description`,
//!   `commands`); a path in it starts with `./`, and a directory of commands makes each `.md`
//!   in it a command named by its file;
//! - `installed.json` is `{"version": 1, "plugins": [{id, root, source, enabled, installedAt,
//!   updatedAt}]}`; Kimi Code keeps the plugins it installs in `plugins/managed/<id>`.

use std::path::{Path, PathBuf};

use anyhow::{bail, Result};
use serde_json::{json, Value};

use super::bundle::{Bundle, File};
use super::Kind;

/// The router's plugin, which carries the Library's commands.
pub const PLUGIN: &str = "agent-router";

/// Where the router's plugin is, in Kimi Code's home `home`.
pub fn plugin_root(home: &Path) -> PathBuf {
    home.join("plugins").join("managed").join(PLUGIN)
}

/// Where Kimi Code reads the Library's command `name`: in the router's plugin.
pub fn command_path(home: &Path, name: &str) -> PathBuf {
    plugin_root(home).join("commands").join(format!("{name}.md"))
}

/// The router's plugin made and registered while it has commands, and taken out once it has
/// none. The owner's own plugins in `installed.json` are left as they are.
pub fn keep_plugin(home: &Path) -> Result<()> {
    let root = plugin_root(home);
    let has_commands = std::fs::read_dir(root.join("commands"))
        .map(|d| d.flatten().any(|e| e.path().extension().is_some_and(|x| x == "md")))
        .unwrap_or(false);
    let manifest = root.join("kimi.plugin.json");
    if has_commands {
        let wanted = json!({
            "name": PLUGIN,
            "version": "1",
            "description": "Agent Router's Library: its commands, kept by the machine agent",
            "commands": "./commands",
        });
        let current: Option<Value> = std::fs::read(&manifest).ok().and_then(|b| serde_json::from_slice(&b).ok());
        if current.as_ref() != Some(&wanted) {
            crate::setup::write_atomic(&manifest, serde_json::to_vec_pretty(&wanted)?)?;
        }
    } else if root.exists() {
        std::fs::remove_dir_all(&root)?;
    }
    register(home, &root, has_commands)
}

/// The router's plugin listed in `installed.json`, or not.
fn register(home: &Path, root: &Path, listed: bool) -> Result<()> {
    let path = home.join("plugins").join("installed.json");
    let mut installed: Value = match std::fs::read(&path) {
        Ok(b) => serde_json::from_slice(&b)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => json!({ "version": 1, "plugins": [] }),
        Err(e) => return Err(e.into()),
    };
    let Some(plugins) = installed.get_mut("plugins").and_then(Value::as_array_mut) else {
        bail!("{} has no list of plugins; it was left as it is", path.display());
    };
    let at = plugins.iter().position(|p| p.get("id").and_then(Value::as_str) == Some(PLUGIN));
    let before = plugins.clone();
    match (at, listed) {
        (Some(i), false) => {
            plugins.remove(i);
        }
        (None, true) => {
            let now = crate::quota::iso(crate::db::now_ms() / 1000);
            plugins.push(json!({
                "id": PLUGIN,
                "root": root.display().to_string(),
                "source": "local-path",
                "enabled": true,
                "installedAt": now,
                "updatedAt": now,
            }));
        }
        // Listed already: its root kept right, and whether the owner turned it off left alone.
        (Some(i), true) => {
            plugins[i]["root"] = json!(root.display().to_string());
        }
        (None, false) => {}
    }
    if *plugins != before {
        crate::setup::write_atomic(&path, serde_json::to_vec_pretty(&installed)?)?;
    }
    Ok(())
}

/// Claude Code's tool names, and Kimi Code's for each; None for one it has no match for.
const TOOLS: &[(&str, Option<&str>)] = &[
    ("Read", Some("Read")),
    ("Write", Some("Write")),
    ("Edit", Some("Edit")),
    ("MultiEdit", Some("Edit")),
    ("Grep", Some("Grep")),
    ("Glob", Some("Glob")),
    ("LS", Some("Glob")),
    ("Bash", Some("Bash")),
    ("BashOutput", Some("TaskOutput")),
    ("KillShell", Some("TaskStop")),
    ("KillBash", Some("TaskStop")),
    ("WebFetch", Some("FetchURL")),
    ("WebSearch", Some("WebSearch")),
    ("Task", Some("Agent")),
    ("Agent", Some("Agent")),
    ("TodoWrite", Some("TodoList")),
    ("NotebookRead", Some("Read")),
    ("NotebookEdit", None),
    ("AskUserQuestion", Some("AskUserQuestion")),
    ("Skill", Some("Skill")),
];

/// A Library item as Kimi Code reads it.
pub fn shape(bundle: Bundle) -> Result<Bundle, String> {
    match bundle.kind() {
        Kind::Subagent => {
            let file = bundle.files().first().ok_or("a subagent with no file")?;
            let converted = agent(bundle.name(), file)?;
            Bundle::check(Kind::Subagent, bundle.name(), vec![converted]).map_err(|r| r.to_string())
        }
        _ => Ok(bundle),
    }
}

/// The tools Kimi Code is given for Claude Code's `tools`: each mapped, an MCP tool as it is,
/// one it has no match for left out.
fn tools(listed: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for tool in listed.split(',').map(str::trim).filter(|t| !t.is_empty()) {
        let mapped = if tool.starts_with("mcp__") || tool == "*" {
            Some(tool)
        } else {
            TOOLS.iter().find(|(claude, _)| *claude == tool).and_then(|(_, kimi)| *kimi)
        };
        if let Some(kimi) = mapped.filter(|k| !out.iter().any(|o| o == k)) {
            out.push(kimi.to_string());
        }
    }
    out
}

/// Claude Code's subagent file as Kimi Code's agent file.
fn agent(name: &str, file: &File) -> Result<File, String> {
    let fields = super::bundle::frontmatter_fields(file).map_err(|r| r.to_string())?;
    let field = |k: &str| fields.iter().find(|(key, _)| key == k).map(|(_, v)| v.as_str());
    let description = field("description").filter(|d| !d.trim().is_empty()).ok_or("a subagent with no description")?;
    let text = std::str::from_utf8(&file.bytes).map_err(|_| "a subagent that is not UTF-8")?;
    // The body: everything after the frontmatter's closing line.
    let mut lines = text.strip_prefix('\u{feff}').unwrap_or(text).split_inclusive('\n');
    lines.next();
    let body: String = lines.skip_while(|l| l.trim_end() != "---").skip(1).collect();
    if body.trim().is_empty() {
        return Err("a subagent with no prompt".into());
    }
    let tools = match field("tools").filter(|t| !t.trim().is_empty()) {
        Some(listed) => tools(listed).join(", "),
        None => "*".to_string(),
    };
    // Double-quoted, as JSON writes a string: YAML reads it as it is.
    let quoted = |s: &str| serde_json::to_string(s).unwrap_or_default();
    let converted = format!(
        "---\nname: {name}\ndescription: {}\nwhenToUse: {}\ntools: {}\n---\n{}",
        quoted(description),
        quoted(description),
        quoted(&tools),
        body.trim_start_matches('\n'),
    );
    Ok(File { path: file.path.clone(), bytes: converted.into_bytes(), executable: false })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn subagent(text: &str) -> Bundle {
        Bundle::check(Kind::Subagent, "code-reviewer", vec![File { path: "code-reviewer.md".into(), bytes: text.into(), executable: false }])
            .unwrap()
    }

    fn text(b: &Bundle) -> String {
        String::from_utf8(b.files()[0].bytes.clone()).unwrap()
    }

    #[test]
    fn a_library_subagent_is_converted_for_kimi_code() {
        let claude = "---\nname: code-reviewer\ndescription: Reviews a diff for bugs, \"carefully\".\ntools: Read, Grep, Glob, Bash, WebFetch, TodoWrite, NotebookEdit, MultiEdit, Edit, mcp__northstar__get_item\nmodel: opus\ncolor: red\n---\n\nYou review the diff.\n\nBe brief.\n";
        let kimi = shape(subagent(claude)).unwrap();
        assert_eq!(
            text(&kimi),
            "---\nname: code-reviewer\n\
             description: \"Reviews a diff for bugs, \\\"carefully\\\".\"\n\
             whenToUse: \"Reviews a diff for bugs, \\\"carefully\\\".\"\n\
             tools: \"Read, Grep, Glob, Bash, FetchURL, TodoList, Edit, mcp__northstar__get_item\"\n\
             ---\nYou review the diff.\n\nBe brief.\n"
        );
        assert_eq!(kimi.description(), "Reviews a diff for bugs, \"carefully\".", "Kimi Code's file reads back as a Library subagent");

        // No tools named: every tool, as Claude Code gives one.
        let open = shape(subagent("---\nname: code-reviewer\ndescription: Reviews.\n---\nYou review.\n")).unwrap();
        assert!(text(&open).contains("\ntools: \"*\"\n"), "{}", text(&open));

        // A skill is Claude Code's shape already; a command is not Kimi Code's to take here.
        let skill = Bundle::check(
            Kind::Skill,
            "release",
            vec![File { path: "SKILL.md".into(), bytes: b"---\nname: release\ndescription: Ships.\n---\nShip.\n".to_vec(), executable: false }],
        )
        .unwrap();
        assert_eq!(shape(skill.clone()).unwrap().pack(), skill.pack());

        // Kimi Code refuses an agent with no prompt; so is it refused here.
        assert!(shape(subagent("---\nname: code-reviewer\ndescription: Reviews.\n---\n\n")).is_err());
    }
}
