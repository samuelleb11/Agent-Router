//! Codex as a native Responses client. Configuration is edited in place; its
//! login remains owned by Codex and the installer can restore the previous provider.

pub mod live;
pub mod session;
pub mod catalogue;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use toml_edit::{value, DocumentMut, Item, Table};

const PROVIDER: &str = "agent-router";
pub const ACCOUNT_ID: &str = "codex-default";

/// Use the native Codex backend's portable request shape for a ChatGPT account.
/// API-only sampling controls and metadata are not part of that backend's wire API.
pub fn backend_request(body: &[u8]) -> Result<Vec<u8>> {
    use serde_json::{json, Value};
    let mut value: Value = serde_json::from_slice(body)?;
    let object = value
        .as_object_mut()
        .context("Responses request must be an object")?;
    for field in ["max_output_tokens", "metadata", "temperature", "top_p"] {
        object.remove(field);
    }
    if let Some(instructions) = object.remove("instructions").filter(|v| !v.is_null()) {
        let instructions = instructions
            .as_str()
            .context("Responses instructions must be text")?;
        if !instructions.is_empty() {
            let mut input = match object.remove("input").unwrap_or(json!([])) {
                Value::Array(input) => input,
                Value::String(text) => vec![
                    json!({"type":"message","role":"user","content":[{"type":"input_text","text":text}]}),
                ],
                _ => bail!("Responses input must be text or an array"),
            };
            input.insert(0, json!({"type":"message","role":"developer","content":[{"type":"input_text","text":instructions}]}));
            object.insert("input".into(), Value::Array(input));
        }
    }
    object.insert("store".into(), json!(false));
    object.insert("stream".into(), json!(true));
    Ok(serde_json::to_vec(&value)?)
}

/// Discover only a usable login. Logged-out Codex installations still appear as
/// clients, without introducing an account that cannot sign.
pub fn discover_account(db_path: &Path) -> Result<()> {
    let account = crate::accounts::Account {
        id: ACCOUNT_ID.into(),
        label: "Codex login".into(),
        source: crate::accounts::Source::Codex,
        provider: crate::accounts::Provider::OpenAI,
        base_url: None,
        profile_dir: None,
        email: None,
    };
    let Ok(resolved) = crate::accounts::resolve(&account) else {
        return Ok(());
    };
    let conn = rusqlite::Connection::open(db_path)?;
    conn.execute(
        "INSERT INTO accounts (id, label, source, provider, email, active, created_at)
         VALUES (?1, 'Codex login', 'codex', 'openai', ?2,
                 CASE WHEN EXISTS (SELECT 1 FROM accounts WHERE provider = 'openai' AND active = 1) THEN 0 ELSE 1 END, ?3)
         ON CONFLICT(id) DO UPDATE SET email = excluded.email WHERE accounts.source = 'codex' AND accounts.provider = 'openai'",
        rusqlite::params![ACCOUNT_ID, resolved.identity, crate::db::now_ms()],
    )?;
    Ok(())
}

/// Refresh an API-key account's model catalogue. A local ChatGPT account's
/// catalogue is read from Codex's own model cache instead.
pub async fn refresh_models(
    state: &std::sync::Arc<crate::proxy::AppState>,
    account: &str,
) -> Result<()> {
    let signer = state.signer(account).await?;
    if signer.provider != crate::accounts::Provider::OpenAI || signer.openai_chatgpt {
        return Ok(());
    }
    let request = axum::http::Request::builder()
        .uri(signer.upstream(&state.config.upstream, "/v1/models"))
        .header("authorization", format!("Bearer {}", signer.token))
        .header("accept", "application/json")
        .body(axum::body::Body::empty())?;
    let body = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        let response = state.client.request(request).await?;
        if !response.status().is_success() {
            bail!("OpenAI model listing returned {}", response.status());
        }
        let body =
            axum::body::to_bytes(axum::body::Body::new(response.into_body()), 4 * 1024 * 1024)
                .await?;
        Ok::<_, anyhow::Error>(body)
    })
    .await??;
    let models: serde_json::Value = serde_json::from_slice(&body)?;
    let db = state.config.db_path.clone();
    tokio::task::spawn_blocking(move || {
        let conn = rusqlite::Connection::open(db)?;
        crate::models::record_openai_models(&conn, &models)
    })
    .await??;
    Ok(())
}

#[derive(Serialize, Deserialize)]
struct Kept {
    model_provider: Option<String>,
    #[serde(default)]
    web_search: Option<String>,
    #[serde(default)]
    config_path: Option<PathBuf>,
}

fn kept_path() -> Result<PathBuf> {
    Ok(crate::setup::data_dir()?.join("codex.json"))
}

/// Ownership check for the unified setup API. Its own snapshot can adopt a native
/// installer configuration only when the installer records this exact config file.
pub(crate) fn tracked_config(path: &Path) -> bool {
    kept_path().ok().and_then(|kept| read_kept(&kept).ok())
        .is_some_and(|saved| saved.config_path.as_deref() == Some(path))
}

pub fn here(url: &str, password: &str, install: bool, consented: bool) -> Result<Option<bool>> {
    let home = crate::accounts::codex_home()?;
    if !install && !consented {
        return Ok(None);
    }
    if !home.is_dir() {
        if !install
            || !crate::clients::here()
                .iter()
                .any(|client| client.client == crate::clients::CODEX)
        {
            return Ok(None);
        }
        std::fs::create_dir_all(&home)
            .context("creating the installed Codex client's config directory")?;
    }
    let kept = kept_path()?;
    // A sync only maintains an installation, and does not choose a new provider.
    if !install && !kept.exists() {
        return Ok(None);
    }
    let path = home.canonicalize()?.join("config.toml");
    if !install && crate::client_setup::manages("codex", &path) {
        // The dashboard manages its chosen credential and model. A background
        // legacy installer sync must not replace that reviewed configuration.
        return Ok(Some(false));
    }
    if !install
        && read_kept(&kept)?
            .config_path
            .is_some_and(|saved| saved != path)
    {
        return Ok(None);
    }
    apply(&path, &kept, url, password, install).map(Some)
}

pub fn take_out() -> Result<Option<bool>> {
    let kept = kept_path()?;
    if !kept.exists() {
        return Ok(None);
    }
    let path = match read_kept(&kept)?.config_path {
        Some(path) => path,
        None => crate::accounts::codex_home()?.join("config.toml"),
    };
    remove(&path, &kept).map(Some)
}

fn read(path: &Path) -> Result<String> {
    match std::fs::read_to_string(path) {
        Ok(s) => Ok(s),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

fn read_kept(path: &Path) -> Result<Kept> {
    serde_json::from_str(&read(path)?).context("reading saved Codex settings")
}

fn parse_config(text: &str) -> Result<DocumentMut> {
    // TOML diagnostics include source excerpts, which can contain a password.
    text.parse().map_err(|_| {
        anyhow::anyhow!("Codex config.toml is invalid; existing settings were left unchanged")
    })
}

/// The provider config contains the router password. Create the replacement
/// privately before writing it, including when the original config was public.
pub(crate) fn write_private(path: &Path, body: impl AsRef<[u8]>) -> Result<()> {
    use std::io::Write;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut nonce = [0u8; 16];
    ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut nonce)
        .map_err(|_| anyhow::anyhow!("could not create a private config replacement"))?;
    let suffix: String = nonce.iter().map(|b| format!("{b:02x}")).collect();
    let tmp = path.with_extension(format!("{suffix}.tmp"));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = (|| -> Result<()> {
        let mut file = options.open(&tmp)?;
        file.write_all(body.as_ref())?;
        file.sync_all()?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

pub fn apply(path: &Path, kept: &Path, url: &str, password: &str, install: bool) -> Result<bool> {
    let before = read(path)?;
    let mut doc = parse_config(&before)?;
    if let Some(providers) = doc.get("model_providers") {
        let table = providers.clone().into_table().map_err(|_| {
            anyhow::anyhow!(
                "Codex model_providers must be a table; existing settings were left unchanged"
            )
        })?;
        doc["model_providers"] = Item::Table(table);
    }
    if kept.exists()
        && read_kept(kept)?
            .config_path
            .is_some_and(|saved| saved != path)
    {
        bail!("Agent Router is installed in another Codex home; uninstall it before changing CODEX_HOME");
    }
    if doc
        .get("model_providers")
        .and_then(|p| p.get(PROVIDER))
        .is_some()
        && !kept.exists()
    {
        bail!("Codex already has a model provider named agent-router; rename it before installing");
    }
    if install && !kept.exists() {
        let model_provider = doc
            .get("model_provider")
            .and_then(Item::as_str)
            .map(str::to_owned);
        let web_search = doc
            .get("web_search")
            .and_then(Item::as_str)
            .map(str::to_owned);
        write_private(
            kept,
            serde_json::to_vec_pretty(&Kept {
                model_provider,
                web_search,
                config_path: Some(path.to_path_buf()),
            })?,
        )?;
    }
    if doc.get("model_providers").is_none() {
        doc["model_providers"] = Item::Table(Table::new());
    }
    let mut provider = Table::new();
    provider["name"] = value("Agent Router");
    provider["base_url"] = value(format!("{}/v1", url.trim_end_matches('/')));
    provider["wire_api"] = value("responses");
    provider["requires_openai_auth"] = value(false);
    // HTTP/SSE can be translated incrementally. WebSockets need their own adapter.
    provider["supports_websockets"] = value(false);
    let mut headers = Table::new();
    headers["Authorization"] = value(format!("Bearer {password}"));
    provider["http_headers"] = Item::Table(headers);
    doc["model_providers"][PROVIDER] = Item::Table(provider);
    if install {
        doc["model_provider"] = value(PROVIDER);
        // Codex includes a provider-hosted web_search tool even in cached mode.
        // Portable function/custom tools are supported across either protocol.
        doc["web_search"] = value("disabled");
    }
    let after = doc.to_string();
    if after == before {
        return Ok(false);
    }
    write_private(path, after)?;
    Ok(true)
}

pub fn remove(path: &Path, kept: &Path) -> Result<bool> {
    if !kept.exists() {
        return Ok(false);
    }
    let previous = read_kept(kept)?;
    if previous
        .config_path
        .as_deref()
        .is_some_and(|saved| saved != path)
    {
        bail!("saved Codex settings belong to another config file");
    }
    catalogue::restore(path)?;
    let before = read(path)?;
    let mut doc = parse_config(&before)?;
    if doc.get("model_provider").and_then(Item::as_str) == Some(PROVIDER) {
        match previous.model_provider {
            Some(p) => doc["model_provider"] = value(p),
            None => {
                doc.remove("model_provider");
            }
        }
    }
    if doc.get("web_search").and_then(Item::as_str) == Some("disabled") {
        match previous.web_search {
            Some(search) => doc["web_search"] = value(search),
            None => {
                doc.remove("web_search");
            }
        }
    }
    if let Some(providers) = doc
        .get_mut("model_providers")
        .and_then(Item::as_table_like_mut)
    {
        providers.remove(PROVIDER);
        if providers.is_empty() {
            doc.remove("model_providers");
        }
    }
    let after = doc.to_string();
    let changed = after != before;
    if changed {
        write_private(path, after)?;
    }
    std::fs::remove_file(kept)?;
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct ConfigFiles {
        root: PathBuf,
        path: PathBuf,
        kept: PathBuf,
    }

    impl ConfigFiles {
        fn new(tag: &str, config: &str) -> Self {
            let root = std::env::temp_dir().join(format!("ar-codex-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).unwrap();
            let path = root.join("config.toml");
            let kept = root.join("kept.json");
            std::fs::write(&path, config).unwrap();
            Self { root, path, kept }
        }
    }

    impl Drop for ConfigFiles {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn malformed_secret_config_is_refused_without_leaking_or_changing_files() {
        let secret = "SYNTHETIC-ROUTER-PASSWORD-DO-NOT-PRINT";
        let malformed =
            format!("[model_providers.other.http_headers]\nAuthorization = \"Bearer {secret}\n");
        let files = ConfigFiles::new("malformed-secret", &malformed);
        let error = apply(
            &files.path,
            &files.kept,
            "https://router.example",
            "new password",
            true,
        )
        .unwrap_err();
        assert!(!format!("{error:#}").contains(secret));
        assert_eq!(read(&files.path).unwrap(), malformed);
        assert!(
            !files.kept.exists(),
            "refusal must not create restoration state"
        );

        std::fs::write(&files.path, "model_provider = \"other\"\n").unwrap();
        apply(
            &files.path,
            &files.kept,
            "https://router.example",
            "new password",
            true,
        )
        .unwrap();
        let saved = std::fs::read(&files.kept).unwrap();
        std::fs::write(&files.path, &malformed).unwrap();
        let error = remove(&files.path, &files.kept).unwrap_err();
        assert!(!format!("{error:#}").contains(secret));
        assert_eq!(read(&files.path).unwrap(), malformed);
        assert_eq!(
            std::fs::read(&files.kept).unwrap(),
            saved,
            "failed uninstall must retain restoration state"
        );
    }

    #[test]
    fn non_table_provider_settings_are_refused_before_saving_or_writing() {
        for (i, invalid) in ["\"scalar\"", "42", "true", "[]"].iter().enumerate() {
            let original = format!("model_provider = \"other\"\nmodel_providers = {invalid}\n");
            let files = ConfigFiles::new(&format!("providers-not-table-{i}"), &original);
            let error = apply(
                &files.path,
                &files.kept,
                "https://router.example",
                "router password",
                true,
            )
            .unwrap_err();
            assert!(error
                .to_string()
                .contains("model_providers must be a table"));
            assert_eq!(read(&files.path).unwrap(), original);
            assert!(!files.kept.exists());
        }
    }

    #[test]
    fn a_different_codex_home_cannot_overwrite_or_discard_restoration_state() {
        let original = "model_provider = \"original\"\nweb_search = \"cached\"\n";
        let files = ConfigFiles::new("home-mismatch", original);
        apply(
            &files.path,
            &files.kept,
            "https://router.example",
            "router password",
            true,
        )
        .unwrap();
        assert_eq!(
            read_kept(&files.kept).unwrap().config_path.as_deref(),
            Some(files.path.as_path())
        );
        let installed = read(&files.path).unwrap();
        let saved = std::fs::read(&files.kept).unwrap();
        let other = files.root.join("another-home").join("config.toml");
        std::fs::create_dir_all(other.parent().unwrap()).unwrap();
        let other_config = "model_provider = \"unrelated\"\nweb_search = \"live\"\n";
        std::fs::write(&other, other_config).unwrap();

        for install in [false, true] {
            assert!(apply(
                &other,
                &files.kept,
                "https://second.example",
                "second password",
                install
            )
            .is_err());
            assert_eq!(read(&other).unwrap(), other_config);
            assert_eq!(read(&files.path).unwrap(), installed);
            assert_eq!(std::fs::read(&files.kept).unwrap(), saved);
        }
        assert!(remove(&other, &files.kept).is_err());
        assert_eq!(read(&other).unwrap(), other_config);
        assert_eq!(std::fs::read(&files.kept).unwrap(), saved);
        remove(&files.path, &files.kept).unwrap();
        let restored = parse_config(&read(&files.path).unwrap()).unwrap();
        assert_eq!(restored["model_provider"].as_str(), Some("original"));
        assert_eq!(restored["web_search"].as_str(), Some("cached"));
        assert!(!files.kept.exists());
    }

    #[test]
    fn uninstall_preserves_settings_the_user_changed_after_install() {
        let files = ConfigFiles::new("user-edits", "# Keep this comment\nmodel = \"gpt-before\"\nmodel_provider = \"before\"\nweb_search = \"cached\"\n[model_providers.before]\nname = \"Before\"\nbase_url = \"https://before.example\"\n");
        apply(
            &files.path,
            &files.kept,
            "https://router.example",
            "router password",
            true,
        )
        .unwrap();
        let mut changed = parse_config(&read(&files.path).unwrap()).unwrap();
        changed["model_provider"] = value("user-selected");
        changed["model"] = value("gpt-user-selected");
        changed["web_search"] = value("live");
        changed["model_providers"]["before"]["name"] = value("Renamed by user");
        let mut added = Table::new();
        added["name"] = value("User selected");
        added["base_url"] = value("https://user.example");
        changed["model_providers"]["user-selected"] = Item::Table(added);
        std::fs::write(&files.path, changed.to_string()).unwrap();

        assert!(remove(&files.path, &files.kept).unwrap());
        let after = read(&files.path).unwrap();
        assert!(after.contains("# Keep this comment"));
        assert!(!after.contains("router password"));
        let config = parse_config(&after).unwrap();
        assert_eq!(config["model_provider"].as_str(), Some("user-selected"));
        assert_eq!(config["model"].as_str(), Some("gpt-user-selected"));
        assert_eq!(config["web_search"].as_str(), Some("live"));
        assert_eq!(
            config["model_providers"]["before"]["name"].as_str(),
            Some("Renamed by user")
        );
        assert_eq!(
            config["model_providers"]["user-selected"]["base_url"].as_str(),
            Some("https://user.example")
        );
        assert!(config["model_providers"].get(PROVIDER).is_none());
        assert!(!files.kept.exists());
    }

    #[test]
    fn install_sync_and_uninstall_preserve_the_users_model_and_settings() {
        let root = std::env::temp_dir().join(format!("ar-codex-config-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("config.toml");
        let kept = root.join("kept.json");
        let _ = std::fs::remove_file(&kept);
        std::fs::write(&path, "# My settings\nmodel = \"gpt-test\"\nmodel_provider = \"other\"\nweb_search = \"cached\"\n[model_providers.other]\nname = \"Other\"\nbase_url = \"https://example.com\"\n").unwrap();
        assert!(apply(
            &path,
            &kept,
            "https://router.example",
            "router password",
            true
        )
        .unwrap());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
            assert_eq!(
                std::fs::metadata(&kept).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        assert!(!apply(
            &path,
            &kept,
            "https://router.example",
            "router password",
            false
        )
        .unwrap());
        let config: DocumentMut = read(&path).unwrap().parse().unwrap();
        assert_eq!(config["model"].as_str(), Some("gpt-test"));
        assert_eq!(config["web_search"].as_str(), Some("disabled"));
        assert_eq!(
            config["model_providers"][PROVIDER]["wire_api"].as_str(),
            Some("responses")
        );
        assert_eq!(
            config["model_providers"][PROVIDER]["supports_websockets"].as_bool(),
            Some(false)
        );
        assert!(remove(&path, &kept).unwrap());
        let after = read(&path).unwrap();
        assert!(after.contains("# My settings"));
        let config: DocumentMut = after.parse().unwrap();
        assert_eq!(config["model_provider"].as_str(), Some("other"));
        assert_eq!(config["web_search"].as_str(), Some("cached"));
        assert!(config["model_providers"].get(PROVIDER).is_none());
        let _ = std::fs::remove_dir_all(root);
    }
}
