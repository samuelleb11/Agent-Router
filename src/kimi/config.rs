//! Kimi Code's `config.toml` as the router keeps it (AGEN-0108): its provider, a model for
//! each of the catalogue's Kimi Code models, the default model, and the hooks that tell the
//! agent which sessions are live. The CLI and the VS Code extension both read this file.
//!
//! Everything else in it is the owner's and stays as it was, comments and order included:
//! the file is edited, never rewritten (`toml_edit`). The router's entries are known by
//! their names: the provider `agent-router`, the models it serves (`agent-router/<id>`), and
//! the hooks that run `agent-router hook kimi`.
//!
//! Written at install, put back at every sync, since Kimi Code updates itself and may
//! rewrite the file, and taken out on uninstall, which puts back the owner's default model.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use toml_edit::{value, ArrayOfTables, DocumentMut, Item, Table};

use crate::accounts::Provider;

/// The router's provider, and the prefix of its models' names.
pub const PROVIDER: &str = "agent-router";

/// The events whose hooks tell the agent about a session.
pub const HOOK_EVENTS: &[&str] =
    &["SessionStart", "SessionEnd", "SessionHeartbeat", "Stop", "PermissionRequest", "PermissionResult"];

/// What each of the router's hooks runs, after the program.
const HOOK_ARGS: &str = "hook kimi";

/// The model made the default at install, when the catalogue has it.
const DEFAULT: &str = "k3";

/// A model's context window when Kimi Code has not described it: the smallest of its
/// models' (K3's 1M window needs a Pro plan).
const CONTEXT: i64 = 262_144;

/// What a model can do when Kimi Code has not described it.
const CAPABILITIES: &[&str] = &["thinking", "image_in", "tool_use"];

/// The router's entries, as they should be.
pub struct Entries {
    /// The router, as Kimi Code is to reach it.
    pub base_url: String,
    /// What Kimi Code presents to it: the router's password, or anything for a local one.
    pub api_key: String,
    /// The catalogue's Kimi Code models: (id, display name).
    pub models: Vec<(String, String)>,
    /// The program each hook runs, with `hook kimi`.
    pub program: PathBuf,
}

/// The catalogue's Kimi Code models, as `GET /api/models` answers it.
pub fn models_of(catalogue: &[Value]) -> Vec<(String, String)> {
    catalogue
        .iter()
        .filter_map(|m| {
            let id = m.get("id")?.as_str()?;
            let label = m.get("display_name").and_then(Value::as_str).unwrap_or(id);
            (m.get("provider")?.as_str()? == Provider::Kimi.as_str()).then(|| (id.to_string(), label.to_string()))
        })
        .collect()
}

/// This machine's Kimi Code pointed at the router at `base_url`, if it has Kimi Code: at
/// install (`install`), and at every sync while the machine's clients are this router's
/// (`consented`: Claude Code's settings name it, as the install wrote them). A router the
/// install did not choose, such as one started by hand to try a build, leaves Kimi Code
/// alone. A sync that finds Kimi Code for the first time sets it up as an install does.
/// None when there is nothing to set up; else whether the file changed.
pub fn here(base_url: &str, api_key: &str, catalogue: &[Value], install: bool, consented: bool) -> Result<Option<bool>> {
    let Some(home) = super::home() else { return Ok(None) };
    if !install && !consented {
        return Ok(None);
    }
    let kept = kept_path()?;
    // The installed binary when there is one, so a hook outlives the build that wrote it.
    let installed = crate::setup::installed_binary()?;
    let program = if installed.is_file() { installed } else { std::env::current_exe()? };
    let entries = Entries {
        base_url: base_url.trim_end_matches('/').to_string(),
        api_key: api_key.to_string(),
        models: models_of(catalogue),
        program,
    };
    let first = !kept.exists();
    apply(&home.join("config.toml"), &kept, &entries, install || first).map(Some)
}

/// The router's entries taken out of this machine's Kimi Code, if it has Kimi Code. None
/// when there is no Kimi Code here; else whether the file changed.
pub fn take_out() -> Result<Option<bool>> {
    let Some(home) = super::home() else { return Ok(None) };
    remove(&home.join("config.toml"), &kept_path()?).map(Some)
}

/// Where the owner's default model is kept while the router's entries are in; that it is
/// there says the router set this machine's Kimi Code up.
fn kept_path() -> Result<PathBuf> {
    Ok(crate::setup::data_dir()?.join("kimi-code.json"))
}

/// The owner's default model before the router's became it, kept beside the router's store.
#[derive(Default, Serialize, Deserialize)]
struct Kept {
    default_model: Option<String>,
}

fn name(id: &str) -> String {
    format!("{PROVIDER}/{id}")
}

fn is_ours(model: &str) -> bool {
    model.starts_with(&format!("{PROVIDER}/"))
}

/// The command a hook runs: the program, quoted when its path needs it, and `hook kimi`.
pub fn hook_command(program: &Path) -> String {
    let p = program.display().to_string();
    let quoted = if !p.contains([' ', '\'', '"']) {
        p
    } else if cfg!(windows) {
        format!("\"{p}\"")
    } else {
        format!("'{}'", p.replace('\'', r"'\''"))
    };
    format!("{quoted} {HOOK_ARGS}")
}

fn our_hook(hook: &Table) -> bool {
    hook.get("command")
        .and_then(Item::as_str)
        .is_some_and(|c| c.trim_end().ends_with(HOOK_ARGS) && c.contains("agent-router"))
}

fn our_model(model: &Item) -> bool {
    model.get("provider").and_then(Item::as_str) == Some(PROVIDER)
}

/// Put the router's entries in the file at `path`, after the owner's. On `install`, the
/// router's K3 also becomes the default model, and the owner's is kept at `kept` to put
/// back. A sync leaves the default alone, unless it names a model of the router's that the
/// router no longer serves. Says whether the file changed.
pub fn apply(path: &Path, kept: &Path, entries: &Entries, install: bool) -> Result<bool> {
    let before = read(path)?;
    let mut doc = parse(path, &before)?;
    put(&mut doc, entries)?;

    let default = doc.get("default_model").and_then(Item::as_str).map(str::to_owned);
    let ours: Vec<String> = entries.models.iter().map(|(id, _)| name(id)).collect();
    let preferred =
        entries.models.iter().find(|(id, _)| id == DEFAULT).or(entries.models.first()).map(|(id, _)| name(id));
    if install {
        if !default.as_deref().is_some_and(is_ours) {
            write_kept(kept, &Kept { default_model: default })?;
        }
        if let Some(preferred) = preferred {
            doc["default_model"] = value(preferred);
        }
    } else if let Some(gone) = default.filter(|d| is_ours(d) && !ours.contains(d)) {
        match preferred.or_else(|| restorable(&doc, kept)) {
            Some(model) => doc["default_model"] = value(model),
            None => {
                doc.remove("default_model");
            }
        }
        tracing::info!("Kimi Code's default model {gone} is no longer served; changed");
    }
    write_if_changed(path, &before, &doc)
}

/// Take the router's entries out of the file at `path`, and put back the owner's default
/// model if the router's was the default. Says whether the file changed.
pub fn remove(path: &Path, kept: &Path) -> Result<bool> {
    let before = read(path)?;
    let mut doc = parse(path, &before)?;
    if let Some(providers) = doc.get_mut("providers").and_then(Item::as_table_mut) {
        providers.remove(PROVIDER);
    }
    if let Some(models) = doc.get_mut("models").and_then(Item::as_table_mut) {
        models.retain(|_, item| !our_model(item));
    }
    for key in ["providers", "models"] {
        if doc.get(key).and_then(Item::as_table).is_some_and(Table::is_empty) {
            doc.remove(key);
        }
    }
    if let Some(hooks) = doc.get_mut("hooks").and_then(Item::as_array_of_tables_mut) {
        hooks.retain(|h| !our_hook(h));
        if hooks.is_empty() {
            doc.remove("hooks");
        }
    }
    if doc.get("default_model").and_then(Item::as_str).is_some_and(is_ours) {
        match restorable(&doc, kept) {
            Some(model) => doc["default_model"] = value(model),
            None => {
                doc.remove("default_model");
            }
        }
    }
    let changed = write_if_changed(path, &before, &doc)?;
    match std::fs::remove_file(kept) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
        _ => {}
    }
    Ok(changed)
}

/// The owner's default model as it was kept, if the file still has that model.
fn restorable(doc: &DocumentMut, kept: &Path) -> Option<String> {
    let model = read_kept(kept).default_model?;
    doc.get("models").and_then(Item::as_table).is_some_and(|m| m.contains_key(&model)).then_some(model)
}

fn put(doc: &mut DocumentMut, e: &Entries) -> Result<()> {
    let root = doc.as_table_mut();

    let provider = table_in(table_in(root, "providers", true)?, PROVIDER, false)?;
    set(provider, "type", "anthropic");
    set(provider, "base_url", e.base_url.as_str());
    set(provider, "api_key", e.api_key.as_str());

    let models = table_in(root, "models", true)?;
    let wanted: HashSet<String> = e.models.iter().map(|(id, _)| name(id)).collect();
    models.retain(|key, item| !our_model(item) || wanted.contains(key));
    for (id, label) in &e.models {
        // Kimi Code's own description of the model, from its own sign-in's entries: what it
        // can do, and its window on the owner's plan.
        let like = described(models, id);
        let model = table_in(models, &name(id), false)?;
        set(model, "provider", PROVIDER);
        set(model, "model", id.as_str());
        let label = like.as_ref().and_then(|l| l.get("display_name")?.as_str().map(str::to_owned)).unwrap_or(label.clone());
        set(model, "display_name", format!("{label} via Agent Router"));
        set(model, "max_context_size", like.as_ref().and_then(|l| l.get("max_context_size")?.as_integer()).unwrap_or(CONTEXT));
        let capabilities = like
            .as_ref()
            .and_then(|l| l.get("capabilities")?.as_array().cloned())
            .unwrap_or_else(|| CAPABILITIES.iter().copied().collect());
        set(model, "capabilities", capabilities);
        for key in ["support_efforts", "default_effort"] {
            if let Some(v) = like.as_ref().and_then(|l| l.get(key)?.as_value().cloned()) {
                set(model, key, v);
            }
        }
    }

    let command = hook_command(&e.program);
    let hooks = match root.entry("hooks").or_insert_with(|| Item::ArrayOfTables(ArrayOfTables::new())) {
        Item::ArrayOfTables(hooks) => hooks,
        _ => bail!("`hooks` in config.toml is not a list of [[hooks]]"),
    };
    let present: Vec<(String, String)> = hooks
        .iter()
        .filter(|h| our_hook(h))
        .map(|h| {
            let field = |k: &str| h.get(k).and_then(Item::as_str).unwrap_or_default().to_string();
            (field("event"), field("command"))
        })
        .collect();
    let wanted: Vec<(String, String)> = HOOK_EVENTS.iter().map(|ev| ((*ev).to_string(), command.clone())).collect();
    if present != wanted {
        hooks.retain(|h| !our_hook(h));
        for (event, command) in wanted {
            let mut hook = Table::new();
            hook["event"] = value(event);
            hook["command"] = value(command);
            hooks.push(hook);
        }
    }
    Ok(())
}

/// The owner's entry for model `id`, Kimi Code's own sign-in's first.
fn described(models: &Table, id: &str) -> Option<Table> {
    let theirs = || {
        models.iter().filter_map(|(_, item)| item.as_table()).filter(|t| {
            t.get("model").and_then(Item::as_str) == Some(id) && t.get("provider").and_then(Item::as_str) != Some(PROVIDER)
        })
    };
    let managed = theirs().find(|t| t.get("provider").and_then(Item::as_str).is_some_and(|p| p.starts_with("managed:")));
    managed.or_else(|| theirs().next()).cloned()
}

/// The table at `key` under `parent`, made if it is not there.
fn table_in<'a>(parent: &'a mut Table, key: &str, implicit: bool) -> Result<&'a mut Table> {
    parent
        .entry(key)
        .or_insert_with(|| {
            let mut t = Table::new();
            t.set_implicit(implicit);
            Item::Table(t)
        })
        .as_table_mut()
        .ok_or_else(|| anyhow!("`{key}` in config.toml is not a table"))
}

/// Set `key` to `v`, unless it already is: an unchanged value keeps its own spacing.
fn set(table: &mut Table, key: &str, v: impl Into<toml_edit::Value>) {
    let v = v.into();
    if table.get(key).and_then(Item::as_value).is_some_and(|old| plain(old) == plain(&v)) {
        return;
    }
    table[key] = Item::Value(v);
}

fn plain(v: &toml_edit::Value) -> String {
    let mut v = v.clone().decorated("", "");
    if let Some(a) = v.as_array_mut() {
        a.fmt();
    }
    v.to_string()
}

fn read(path: &Path) -> Result<String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(e.into()),
    }
}

/// A file that does not parse is refused rather than replaced: replacing it would lose the
/// owner's settings.
fn parse(path: &Path, text: &str) -> Result<DocumentMut> {
    text.parse::<DocumentMut>()
        .map_err(|e| anyhow!("{} is not valid TOML ({e}); fix or move it first", path.display()))
}

fn write_if_changed(path: &Path, before: &str, doc: &DocumentMut) -> Result<bool> {
    let after = doc.to_string();
    if after == before {
        return Ok(false);
    }
    crate::setup::write_atomic(path, after)?;
    Ok(true)
}

fn read_kept(kept: &Path) -> Kept {
    std::fs::read(kept).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn write_kept(kept: &Path, k: &Kept) -> Result<()> {
    crate::setup::write_atomic(kept, serde_json::to_vec_pretty(k)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A config.toml as Kimi Code 2.1.1 writes it after its own sign-in, with the owner's
    /// own provider, search service, hook and settings around it.
    const OWNERS: &str = r#"# my Kimi settings
default_model = "kimi-code/k3"

[providers."managed:kimi-code"]
type = "kimi"
base_url = "https://api.kimi.ai/coding/v1"

[providers.mine]
type = "openai_legacy"
base_url = "https://llm.example.com/v1"

[models."kimi-code/k3"]
provider = "managed:kimi-code"
model = "k3"
max_context_size = 1048576
capabilities = [ "thinking", "always_thinking", "image_in", "tool_use" ]
display_name = "K3"
support_efforts = [ "low", "high", "max" ]
default_effort = "high"

[models."kimi-code/kimi-for-coding"]
provider = "managed:kimi-code"
model = "kimi-for-coding"
max_context_size = 1048576
capabilities = [ "thinking", "tool_use" ]
display_name = "K2.8 Preview"

[thinking]
enabled = true

[services.moonshot_search]
base_url = "https://api.kimi.ai/coding/v1/search"

[[hooks]]
event = "Stop"
command = "say done"
"#;

    struct Home {
        dir: PathBuf,
    }

    impl Home {
        fn new(name: &str, config: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("ar-kimi-config-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("config.toml"), config).unwrap();
            Self { dir }
        }
        fn config(&self) -> PathBuf {
            self.dir.join("config.toml")
        }
        fn kept(&self) -> PathBuf {
            self.dir.join("kept.json")
        }
        fn doc(&self) -> DocumentMut {
            std::fs::read_to_string(self.config()).unwrap().parse().unwrap()
        }
    }

    impl Drop for Home {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn entries() -> Entries {
        Entries {
            base_url: "https://router.example.com".into(),
            api_key: "the router's password".into(),
            models: vec![("k3".into(), "Kimi K3".into()), ("k3-256k".into(), "Kimi K3 (256K)".into())],
            program: PathBuf::from("/Users/u/.agent-router/bin/agent-router"),
        }
    }

    fn strings(item: &Item) -> Vec<String> {
        item.as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect()
    }

    #[test]
    fn kimi_config_gets_the_routers_provider_models_and_hooks() {
        let home = Home::new("gets", OWNERS);
        assert!(apply(&home.config(), &home.kept(), &entries(), true).unwrap());
        let doc = home.doc();

        let provider = &doc["providers"][PROVIDER];
        assert_eq!(provider["type"].as_str(), Some("anthropic"));
        assert_eq!(provider["base_url"].as_str(), Some("https://router.example.com"));
        assert_eq!(provider["api_key"].as_str(), Some("the router's password"));

        // K3 as Kimi Code's own sign-in describes it, served by the router.
        let k3 = &doc["models"]["agent-router/k3"];
        assert_eq!(k3["provider"].as_str(), Some(PROVIDER));
        assert_eq!(k3["model"].as_str(), Some("k3"));
        assert_eq!(k3["display_name"].as_str(), Some("K3 via Agent Router"));
        assert_eq!(k3["max_context_size"].as_integer(), Some(1_048_576));
        assert_eq!(strings(&k3["capabilities"]), ["thinking", "always_thinking", "image_in", "tool_use"]);
        assert_eq!(strings(&k3["support_efforts"]), ["low", "high", "max"]);
        assert_eq!(k3["default_effort"].as_str(), Some("high"));
        // A model Kimi Code has not described: the catalogue's name, and the smallest window.
        let small = &doc["models"]["agent-router/k3-256k"];
        assert_eq!(small["display_name"].as_str(), Some("Kimi K3 (256K) via Agent Router"));
        assert_eq!(small["max_context_size"].as_integer(), Some(CONTEXT));
        assert_eq!(strings(&small["capabilities"]), CAPABILITIES);

        assert_eq!(doc["default_model"].as_str(), Some("agent-router/k3"));
        assert_eq!(read_kept(&home.kept()).default_model.as_deref(), Some("kimi-code/k3"));

        let ours: Vec<(&str, &str)> = doc["hooks"]
            .as_array_of_tables()
            .unwrap()
            .iter()
            .filter(|h| our_hook(h))
            .map(|h| (h["event"].as_str().unwrap(), h["command"].as_str().unwrap()))
            .collect();
        let command = "/Users/u/.agent-router/bin/agent-router hook kimi";
        assert_eq!(ours, HOOK_EVENTS.iter().map(|e| (*e, command)).collect::<Vec<_>>());

        // Once written, a second pass changes nothing.
        assert!(!apply(&home.config(), &home.kept(), &entries(), false).unwrap());
        assert!(!apply(&home.config(), &home.kept(), &entries(), true).unwrap(), "a reinstall changes nothing either");
        assert_eq!(read_kept(&home.kept()).default_model.as_deref(), Some("kimi-code/k3"), "and keeps the owner's default");
    }

    #[test]
    fn kimi_config_keeps_the_owners_own_entries() {
        let home = Home::new("keeps", OWNERS);
        apply(&home.config(), &home.kept(), &entries(), true).unwrap();
        let after = std::fs::read_to_string(home.config()).unwrap();
        let doc = home.doc();

        // Every line of the owner's but the default model is still there, as it was.
        for line in OWNERS.lines().filter(|l| !l.starts_with("default_model")) {
            assert!(after.lines().any(|a| a == line), "lost {line:?} from:\n{after}");
        }
        assert_eq!(doc["providers"]["mine"]["base_url"].as_str(), Some("https://llm.example.com/v1"));
        assert_eq!(doc["providers"]["managed:kimi-code"]["type"].as_str(), Some("kimi"));
        assert_eq!(doc["models"]["kimi-code/k3"]["provider"].as_str(), Some("managed:kimi-code"));
        assert_eq!(doc["services"]["moonshot_search"]["base_url"].as_str(), Some("https://api.kimi.ai/coding/v1/search"));
        assert_eq!(doc["thinking"]["enabled"].as_bool(), Some(true));
        let hooks = doc["hooks"].as_array_of_tables().unwrap();
        assert_eq!(hooks.get(0).unwrap()["command"].as_str(), Some("say done"), "the owner's hook, first");
        assert_eq!(hooks.len(), 1 + HOOK_EVENTS.len());

        // A file it cannot read as TOML is refused, never replaced.
        let broken = Home::new("broken", "default_model = \n[[");
        assert!(apply(&broken.config(), &broken.kept(), &entries(), true).is_err());
        assert_eq!(std::fs::read_to_string(broken.config()).unwrap(), "default_model = \n[[");

        // No file yet: the router's entries make one.
        let fresh = Home::new("fresh", "");
        assert!(apply(&fresh.config(), &fresh.kept(), &entries(), true).unwrap());
        assert_eq!(fresh.doc()["default_model"].as_str(), Some("agent-router/k3"));
    }

    #[test]
    fn kimi_config_is_put_back_after_an_update_and_taken_out_on_uninstall() {
        let home = Home::new("update", OWNERS);
        apply(&home.config(), &home.kept(), &entries(), true).unwrap();

        // Kimi Code updates and writes the file again with its own entries only.
        std::fs::write(home.config(), OWNERS.replace("\"kimi-code/k3\"\n\n[providers", "\"agent-router/k3\"\n\n[providers"))
            .unwrap();
        assert!(home.doc().get("providers").unwrap().get(PROVIDER).is_none());
        assert!(apply(&home.config(), &home.kept(), &entries(), false).unwrap(), "the sync put them back");
        let doc = home.doc();
        assert_eq!(doc["providers"][PROVIDER]["base_url"].as_str(), Some("https://router.example.com"));
        assert_eq!(doc["models"]["agent-router/k3"]["model"].as_str(), Some("k3"));
        assert_eq!(doc["hooks"].as_array_of_tables().unwrap().iter().filter(|h| our_hook(h)).count(), HOOK_EVENTS.len());
        assert_eq!(doc["default_model"].as_str(), Some("agent-router/k3"));

        // The owner picks another default: a sync leaves it.
        let mine = std::fs::read_to_string(home.config()).unwrap().replace(
            "default_model = \"agent-router/k3\"",
            "default_model = \"kimi-code/kimi-for-coding\"",
        );
        std::fs::write(home.config(), mine).unwrap();
        assert!(!apply(&home.config(), &home.kept(), &entries(), false).unwrap());

        // The router stops serving a model: its entry goes, and a default that named it
        // becomes the router's K3.
        let mut fewer = entries();
        fewer.models.retain(|(id, _)| id == "k3");
        std::fs::write(
            home.config(),
            std::fs::read_to_string(home.config()).unwrap().replace("\"kimi-code/kimi-for-coding\"\n", "\"agent-router/k3-256k\"\n"),
        )
        .unwrap();
        assert!(apply(&home.config(), &home.kept(), &fewer, false).unwrap());
        let doc = home.doc();
        assert!(doc["models"].get("agent-router/k3-256k").is_none());
        assert_eq!(doc["default_model"].as_str(), Some("agent-router/k3"));

        // Uninstall: the router's entries out, the owner's default back, the owner's own as
        // they were.
        assert!(remove(&home.config(), &home.kept()).unwrap());
        let after = std::fs::read_to_string(home.config()).unwrap();
        let doc = home.doc();
        assert!(doc["providers"].get(PROVIDER).is_none());
        assert!(doc["models"].as_table().unwrap().iter().all(|(k, _)| !is_ours(k)), "{after}");
        assert!(doc["hooks"].as_array_of_tables().unwrap().iter().all(|h| !our_hook(h)));
        assert_eq!(doc["default_model"].as_str(), Some("kimi-code/k3"));
        assert_eq!(after, OWNERS, "the file is the owner's again, as it was");
        assert!(!home.kept().exists());
        assert!(!remove(&home.config(), &home.kept()).unwrap(), "a second uninstall has nothing to take out");
    }

    #[test]
    fn a_hook_command_quotes_a_path_with_spaces() {
        assert_eq!(hook_command(Path::new("/opt/agent-router")), "/opt/agent-router hook kimi");
        if !cfg!(windows) {
            assert_eq!(hook_command(Path::new("/Users/A B/agent-router")), "'/Users/A B/agent-router' hook kimi");
        }
    }
}
