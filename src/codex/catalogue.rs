//! Native model metadata and ownership of its local file. Inference credentials
//! stay in the provider config; this catalogue never contains credentials.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use toml_edit::{value, DocumentMut, Item};

use crate::models::ClientCatalogue;

pub const QUALIFIED_VERSION: &str = "0.162.0-alpha.17.2";
pub const FILE: &str = "agent-router-models.json";
const LEDGER: &str = "agent-router-models.state.json";
pub(crate) static WRITES: Mutex<()> = Mutex::new(());

// Our own portable coding instructions, rather than another model's cached or
// private prompt. Both the legacy field and the canonical V2 template are set.
const INSTRUCTIONS: &str = "You are a coding assistant working in the user's workspace. Follow the user's instructions and the workspace instructions. Inspect relevant files before editing. Use the available local tools to make and verify changes. Preserve unrelated user changes. Explain the result and any remaining limitations clearly.";

/// Contract inspected at the qualified Codex tag and exercised by the native
/// app-server. No bundled models, hosted tools, upgrade notices or service tiers
/// are inherited. Unknown limits use conservative input/output headroom.
pub fn native(catalogue: &ClientCatalogue) -> Result<Value> {
    if catalogue.schema_version != 1 {
        bail!("unsupported router model metadata version");
    }
    let mut rows = Vec::new();
    let mut ids = std::collections::HashSet::new();
    for (priority, model) in catalogue.models.iter().enumerate() {
        if model.id.trim().is_empty()
            || !ids.insert(&model.id)
            || model.id.chars().any(char::is_control)
        {
            bail!("invalid model ID in router metadata");
        }
        let caps = &model.capabilities;
        let context = caps.context_tokens.unwrap_or(128_000);
        let output = caps.output_tokens.unwrap_or(4_096.min((context / 4).max(1)));
        if context == 0 || context > 100_000_000 || output == 0 || output > 100_000_000 {
            bail!("invalid model token limits in router metadata");
        }
        if output >= context {
            bail!("Codex model {} needs a context limit larger than its output budget",model.id);
        }
        let efforts = if caps.reasoning {
            json!([{"effort":"low","description":"Lighter reasoning"},
                {"effort":"medium","description":"Balanced reasoning"},
                {"effort":"high","description":"Deeper reasoning"}])
        } else {
            json!([])
        };
        // Codex has no per-model max-output field. Reserve that budget in its
        // input window; the gateway enforces explicitly configured output limits.
        let usable = (context.saturating_sub(output) * 100 / context).clamp(1, 95);
        rows.push(json!({
            "slug":model.id,"display_name":model.display_name,
            "description":format!("{} through Agent Router",model.provider.name()),
            "default_reasoning_level":if caps.reasoning {json!("medium")} else {Value::Null},
            "supported_reasoning_levels":efforts,
            "shell_type":if caps.tools {"unified_exec"} else {"disabled"},
            "visibility":if caps.tools {"list"} else {"hide"},"supported_in_api":true,"priority":priority,
            "availability_nux":null,"upgrade":null,
            "base_instructions":INSTRUCTIONS,"model_messages":{"instructions_template":INSTRUCTIONS},
            "include_skills_usage_instructions":caps.tools,"include_plugin_usage_instructions":caps.tools,
            "include_apps_usage_instructions":false,"support_verbosity":false,"default_verbosity":null,
            "supports_reasoning_summary_parameter":caps.reasoning,"default_reasoning_summary":"none",
            "apply_patch_tool_type":null,"truncation_policy":{"mode":"tokens","limit":output.min(10_000)},
            "context_window":context,"max_context_window":context,"effective_context_window_percent":usable,
            "experimental_supported_tools":[],"input_modalities":if caps.images {json!(["text","image"])} else {json!(["text"])},
            "supports_search_tool":false,"use_responses_lite":false,"supports_reasoning_effort_updates":false,
            "tool_mode":"direct"
        }));
    }
    Ok(json!({"models":rows}))
}

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn snapshot(path: &Path) -> Result<Option<Vec<u8>>> {
    if std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
        bail!("managed Codex catalogue files cannot be symlinks");
    }
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).context("reading managed Codex catalogue"),
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Saved {
    config: PathBuf,
    previous_path: Option<String>,
    installed_path: String,
    digest: String,
    qualified_version: String,
}

pub struct Plan {
    pub after: String,
    file: PathBuf,
    ledger: PathBuf,
    before_file: Option<Vec<u8>>,
    before_ledger: Option<Vec<u8>>,
    body: Vec<u8>,
    saved: Vec<u8>,
}

impl Plan {
    pub fn changed(&self) -> bool {
        self.before_file.as_deref() != Some(&self.body)
    }
    pub fn apply(&self) -> Result<()> {
        if snapshot(&self.file)? != self.before_file
            || snapshot(&self.ledger)? != self.before_ledger
        {
            bail!("Codex catalogue changed after preview; refresh the preview");
        }
        let result = (|| {
            super::write_private(&self.ledger, &self.saved)?;
            super::write_private(&self.file, &self.body)
        })();
        if result.is_err() {
            self.rollback()?;
        }
        result
    }
    pub fn rollback(&self) -> Result<()> {
        for (path, previous) in [
            (&self.file, &self.before_file),
            (&self.ledger, &self.before_ledger),
        ] {
            match previous {
                Some(bytes) => super::write_private(path, bytes)?,
                None if path.exists() => std::fs::remove_file(path)?,
                None => {}
            }
        }
        Ok(())
    }
}

/// Prepare all changes before writing. The caller holds WRITES during application
/// and rolls these two artifacts back if its final config replacement fails.
pub fn prepare(
    config: &Path,
    before: &str,
    after: &str,
    metadata: &ClientCatalogue,
) -> Result<Plan> {
    let home = config.parent().context("Codex config has no parent")?;
    let file = home.join(FILE);
    let ledger = home.join(LEDGER);
    let before_file = snapshot(&file)?;
    let before_ledger = snapshot(&ledger)?;
    let original = super::parse_config(before)?;
    let previous_path = original
        .get("model_catalog_json")
        .map(|i| {
            i.as_str()
                .map(str::to_owned)
                .context("model_catalog_json must be a path")
        })
        .transpose()?;
    let installed_path = file
        .to_str()
        .context("Codex catalogue path must be UTF-8")?
        .to_owned();
    let previous_path = if let Some(bytes) = &before_ledger {
        let saved: Saved =
            serde_json::from_slice(bytes).context("saved Codex catalogue ownership is invalid")?;
        if saved.config != config || saved.installed_path != installed_path {
            bail!("saved Codex catalogue belongs to another configuration");
        }
        if previous_path.as_deref() != Some(&installed_path) {
            bail!("Codex model_catalog_json was edited; existing settings were preserved");
        }
        if before_file
            .as_ref()
            .is_some_and(|body| digest(body) != saved.digest)
        {
            bail!("Codex catalogue was edited; preserve it before refreshing");
        }
        saved.previous_path
    } else {
        if before_file.is_some() {
            bail!("an unmanaged Codex catalogue file already exists; preserve it before setup");
        }
        previous_path
    };
    let body = serde_json::to_vec_pretty(&native(metadata)?)?;
    let mut doc = super::parse_config(after)?;
    doc["model_catalog_json"] = value(&installed_path);
    let saved = serde_json::to_vec_pretty(&Saved {
        config: config.to_owned(),
        previous_path,
        installed_path,
        digest: digest(&body),
        qualified_version: QUALIFIED_VERSION.into(),
    })?;
    Ok(Plan {
        after: doc.to_string(),
        file,
        ledger,
        before_file,
        before_ledger,
        body,
        saved,
    })
}

/// Maintain an owned catalogue without selecting a provider or restarting a
/// running client. None means this config is not currently using Agent Router.
pub fn sync(config: &Path, metadata: &ClientCatalogue, install: bool) -> Result<Option<bool>> {
    let _lock = WRITES
        .lock()
        .map_err(|_| anyhow::anyhow!("Codex catalogue lock unavailable"))?;
    let before = super::read(config)?;
    let doc = super::parse_config(&before)?;
    if doc.get("model_provider").and_then(Item::as_str) != Some(super::PROVIDER) {
        return Ok(None);
    }
    if !install && !config.parent().unwrap().join(LEDGER).exists() {
        return Ok(None);
    }
    let plan = prepare(config, &before, &before, metadata)?;
    let changed = plan.changed() || plan.after != before;
    if changed {
        plan.apply()?;
        if plan.after != before {
            if let Err(error) = super::write_private(config, &plan.after) {
                plan.rollback()?;
                return Err(error);
            }
        }
    }
    Ok(Some(changed))
}

/// Restore only our unchanged path and file. Return whether user edits were
/// preserved, independently of the provider installer's own restoration record.
pub fn restore(config: &Path) -> Result<bool> {
    let _lock = WRITES
        .lock()
        .map_err(|_| anyhow::anyhow!("Codex catalogue lock unavailable"))?;
    let home = config.parent().context("Codex config has no parent")?;
    let ledger = home.join(LEDGER);
    let Some(bytes) = snapshot(&ledger)? else {
        return Ok(false);
    };
    let saved: Saved =
        serde_json::from_slice(&bytes).context("saved Codex catalogue ownership is invalid")?;
    let file = home.join(FILE);
    if saved.config != config || saved.installed_path != file.to_string_lossy() {
        bail!("saved Codex catalogue belongs to another configuration");
    }
    let mut doc: DocumentMut = super::parse_config(&super::read(config)?)?;
    let mut preserved = false;
    if doc.get("model_catalog_json").and_then(Item::as_str) == Some(saved.installed_path.as_str()) {
        match saved.previous_path {
            Some(path) => doc["model_catalog_json"] = value(path),
            None => {
                doc.remove("model_catalog_json");
            }
        }
        super::write_private(config, doc.to_string())?;
    } else {
        preserved = true;
    }
    match snapshot(&file)? {
        Some(bytes) if digest(&bytes) == saved.digest => std::fs::remove_file(file)?,
        Some(_) => preserved = true,
        None => {}
    }
    std::fs::remove_file(ledger)?;
    Ok(preserved)
}

/// Explicit refresh loads the current router metadata using the existing client
/// credential. It never restarts a CLI, editor or app-server during an active turn.
pub async fn cli(args: &[String]) -> Result<()> {
    if args.len() != 2 || args[0] != "models" || !matches!(args[1].as_str(), "refresh" | "restore")
    {
        bail!("usage: agent-router codex models refresh|restore");
    }
    let config = crate::accounts::codex_home()?
        .join("config.toml")
        .canonicalize()
        .context("configure Codex with Agent Router before refreshing its models")?;
    if args[1] == "restore" {
        let edited = restore(&config)?;
        println!(
            "Codex's previous model catalogue setting restored{}.",
            if edited {
                "; edited files and settings were preserved"
            } else {
                ""
            }
        );
        return Ok(());
    }
    let doc = super::parse_config(&super::read(&config)?)?;
    if doc.get("model_provider").and_then(Item::as_str) != Some(super::PROVIDER) {
        bail!("Codex must select the agent-router provider before refreshing models");
    }
    let provider = &doc["model_providers"][super::PROVIDER];
    let base = provider
        .get("base_url")
        .and_then(Item::as_str)
        .context("Codex's router base_url is missing")?;
    let server = base
        .trim_end_matches('/')
        .strip_suffix("/v1")
        .context("Codex's router base_url must end in /v1")?;
    let token = std::env::var("AGENT_ROUTER_CLIENT_TOKEN")
        .ok()
        .filter(|t| !t.is_empty())
        .or_else(|| {
            provider
                .get("http_headers")
                .and_then(|h| h.get("x-agent-router-token"))
                .and_then(Item::as_str)
                .map(str::to_owned)
                .or_else(|| {
                    provider
                        .get("http_headers")
                        .and_then(|h| h.get("Authorization"))
                        .and_then(Item::as_str)
                        .and_then(|h| h.strip_prefix("Bearer "))
                        .map(str::to_owned)
                })
        })
        .context(
            "Codex's router token is missing; use Client setup or AGENT_ROUTER_CLIENT_TOKEN",
        )?;
    let router = crate::library::sync::Router::new(server, &token)?;
    let (status, _, body) = router
        .send(
            axum::http::Method::GET,
            "/_router/models",
            vec![],
            bytes::Bytes::new(),
        )
        .await?;
    if !status.is_success() {
        bail!(
            "router model discovery returned {status}; check the Client token and router version"
        );
    }
    let metadata: ClientCatalogue =
        serde_json::from_slice(&body).context("router model metadata is invalid")?;
    let changed = sync(&config, &metadata, true)?.unwrap_or(false);
    println!(
        "Codex model catalogue {} ({} models).",
        if changed {
            "updated"
        } else {
            "already current"
        },
        metadata.models.len()
    );
    println!("Running clients keep their current list. Finish active turns, then use `codex resume <SESSION_ID>` or reload the Codex editor window and reopen the same conversation.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::Provider;
    use crate::models::ClientModel;
    use crate::provider_registry::Capabilities;

    #[test]
    fn catalogue_ownership_and_capabilities() {
        let home = std::env::temp_dir().join(format!("ar-codex-catalogue-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        let config = home.join("config.toml");
        let original="model_provider = 'agent-router'\nmodel_catalog_json = '/previous/catalogue.json'\n# keep this comment\n";
        std::fs::write(&config, original).unwrap();
        let mut metadata = ClientCatalogue {
            schema_version: 1,
            models: vec![ClientModel {
                id: "team-fast".into(),
                display_name: "Team fast".into(),
                provider: Provider::Anthropic,
                capabilities: Capabilities {
                    images: false,
                    reasoning: false,
                    context_tokens: Some(32000),
                    output_tokens: Some(2000),
                    ..Default::default()
                },
            }],
        };
        assert_eq!(sync(&config, &metadata, false).unwrap(), None);
        assert_eq!(sync(&config, &metadata, true).unwrap(), Some(true));
        let first = std::fs::read(home.join(FILE)).unwrap();
        let value: Value = serde_json::from_slice(&first).unwrap();
        assert_eq!(value["models"][0]["slug"], "team-fast");
        assert_eq!(value["models"][0]["context_window"], 32000);
        assert_eq!(value["models"][0]["input_modalities"], json!(["text"]));
        assert_eq!(value["models"][0]["supported_reasoning_levels"], json!([]));
        assert!(!value["models"][0]["base_instructions"]
            .as_str()
            .unwrap()
            .is_empty());
        assert_eq!(sync(&config, &metadata, false).unwrap(), Some(false));
        metadata.models[0].id = "team-next".into();
        assert_eq!(sync(&config, &metadata, false).unwrap(), Some(true));
        let installed = std::fs::read_to_string(&config).unwrap();
        assert!(installed.contains("# keep this comment"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for path in [&config, &home.join(FILE), &home.join(LEDGER)] {
                assert_eq!(
                    std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
                    0o600
                );
            }
        }
        std::fs::write(home.join(FILE), b"user edits").unwrap();
        assert!(sync(&config, &metadata, false)
            .unwrap_err()
            .to_string()
            .contains("edited"));
        assert!(restore(&config).unwrap());
        assert_eq!(std::fs::read(home.join(FILE)).unwrap(), b"user edits");
        assert!(std::fs::read_to_string(&config)
            .unwrap()
            .contains("/previous/catalogue.json"));
        assert!(
            sync(&config, &metadata, true).is_err(),
            "unmanaged file must survive"
        );
        std::fs::remove_file(home.join(FILE)).unwrap();
        sync(&config, &metadata, true).unwrap();
        std::fs::write(
            &config,
            "model_provider='agent-router'\nmodel_catalog_json='/user/new.json'\n",
        )
        .unwrap();
        assert!(sync(&config, &metadata, false).is_err());
        assert!(restore(&config).unwrap());
        assert!(std::fs::read_to_string(&config)
            .unwrap()
            .contains("/user/new.json"));
        // Failed final config writes can restore both artifacts to their preview.
        let before = std::fs::read_to_string(&config).unwrap();
        let plan = prepare(&config, &before, &before, &metadata).unwrap();
        plan.apply().unwrap();
        plan.rollback().unwrap();
        assert!(!home.join(FILE).exists() && !home.join(LEDGER).exists());
        metadata.schema_version = 99;
        assert!(native(&metadata).is_err());
        std::fs::remove_dir_all(home).unwrap();
    }
}
