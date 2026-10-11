//! Reversible setup for clients that speak Messages or Responses.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, bail, Context, Result};
use axum::extract::{Path as Params, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use ring::rand::SecureRandom;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use toml_edit::{value, DocumentMut, Item, Table};

use crate::accounts::Provider;
use crate::proxy::AppState;
use crate::refusal::{failed, refuse, JsonBody};

const PLACEHOLDER: &str = "<AGENT_ROUTER_CLIENT_TOKEN>";
const IDS: [&str; 3] = ["claude-code", "codex", "opencode"];
static WRITES: Mutex<()> = Mutex::new(());

/// Internal dependency injection for fixture homes. No HTTP parameter selects a home.
#[derive(Clone)]
pub(crate) struct Roots {
    home: PathBuf,
    codex: PathBuf,
    opencode: PathBuf,
}

impl Roots {
    fn here() -> Result<Self> {
        let home = crate::setup::home_dir()?;
        let codex = crate::accounts::codex_home()?;
        let opencode = std::env::var_os("XDG_CONFIG_HOME")
            .filter(|p| !p.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".config"))
            .join("opencode");
        Ok(Self {
            home,
            codex,
            opencode,
        })
    }

    fn path(&self, client: &str, project: Option<&str>) -> Result<PathBuf> {
        if let Some(project) = project {
            if client == "codex" {
                bail!("Codex setup uses its global config; omit project");
            }
            let path = PathBuf::from(project);
            if !path.is_absolute() || !path.is_dir() {
                bail!("project must be an existing absolute directory on this machine");
            }
            let path = path.canonicalize()?;
            let target = match client {
                "claude-code" => path.join(".claude/settings.local.json"),
                "opencode" => path.join("opencode.json"),
                _ => bail!("unknown client"),
            };
            return canonical_target(&target);
        }
        let path = match client {
            "claude-code" => self.home.join(".claude/settings.json"),
            "codex" => self.codex.join("config.toml"),
            "opencode" => self.opencode.join("opencode.json"),
            _ => bail!("unknown client"),
        };
        canonical_target(&path)
    }
}

fn canonical_target(path: &Path) -> Result<PathBuf> {
    // Native Codex canonicalizes its home before maintenance. Resolve the same
    // ancestor here so symlinked homes still share one restoration record.
    if path.exists() || std::fs::symlink_metadata(path).is_ok() {
        return path
            .canonicalize()
            .context("client configuration symlink cannot be resolved");
    }
    let mut ancestor = path
        .parent()
        .context("client configuration has no parent")?;
    while !ancestor.exists() {
        ancestor = ancestor
            .parent()
            .context("client configuration has no existing ancestor")?;
    }
    Ok(ancestor.canonicalize()?.join(path.strip_prefix(ancestor)?))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Setup {
    model: String,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    token: Option<String>,
    #[serde(default)]
    project: Option<String>,
    #[serde(default)]
    revision: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Restore {
    #[serde(default)]
    project: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct Saved {
    client: String,
    path: PathBuf,
    existed: bool,
    before: String,
    installed: String,
    owned: Vec<Vec<String>>,
    #[serde(default)]
    credential_path: Option<PathBuf>,
    #[serde(default)]
    credential_digest: Option<String>,
}

fn digest(bytes: impl AsRef<[u8]>) -> String {
    Sha256::digest(bytes.as_ref())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn kept(state: &AppState, client: &str, path: &Path) -> PathBuf {
    kept_at(&state.config.db_path, client, path)
}

fn kept_at(db: &Path, client: &str, path: &Path) -> PathBuf {
    // Paths and their backups never come from a writable request field.
    let id = digest(format!("{client}\0{}", path.display()));
    db.parent()
        .unwrap_or(Path::new("."))
        .join("client-configurations")
        .join(format!("{id}.json"))
}

pub(crate) fn manages(client: &str, path: &Path) -> bool {
    crate::config::db_path()
        .ok()
        .is_some_and(|db| kept_at(&db, client, path).exists())
}

fn read(path: &Path) -> Result<String> {
    match std::fs::read_to_string(path) {
        Ok(body) => Ok(body),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(e).context("could not read client configuration"),
    }
}

/// Write the replacement privately before renaming: no public temporary secret.
fn write_private(path: &Path, body: impl AsRef<[u8]>) -> Result<()> {
    use std::io::Write;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut nonce = [0u8; 16];
    ring::rand::SystemRandom::new()
        .fill(&mut nonce)
        .map_err(|_| anyhow!("could not generate a private configuration replacement"))?;
    let tmp = path.with_extension(format!("{}.tmp", digest(nonce)));
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
        let _ = std::fs::remove_file(tmp);
    }
    result
}

fn valid_client(client: &str) -> Result<()> {
    if !IDS.contains(&client) {
        bail!("unknown client; choose claude-code, codex or opencode");
    }
    Ok(())
}

fn router_url(state: &AppState) -> String {
    state
        .config
        .public_url
        .clone()
        .unwrap_or_else(|| format!("http://127.0.0.1:{}", state.config.bind.port()))
}

fn selected_url(state: &AppState, supplied: Option<&str>) -> Result<String> {
    let expected = router_url(state);
    let url = supplied.unwrap_or(&expected).trim_end_matches('/');
    let uri: axum::http::Uri = url.parse().map_err(|_| anyhow!("router URL is invalid"))?;
    if uri.authority().is_none()
        || uri.authority().is_some_and(|a| a.as_str().contains('@'))
        || !matches!(uri.scheme_str(), Some("http" | "https"))
        || uri.path() != "/"
        || uri.query().is_some()
        || url.contains('#')
    {
        bail!("router URL must be an HTTP(S) origin without credentials, path or query");
    }
    if state.config.server && uri.scheme_str() != Some("https") {
        bail!("hosted client configuration needs an HTTPS router URL");
    }
    // Credentials checked against this router must never be exported to another server.
    if state.config.public_url.is_some() || !state.config.server {
        if url != expected.trim_end_matches('/') {
            bail!("router URL must match this router's advertised address");
        }
    } else if !uri.host().is_some_and(|host| {
        state
            .config
            .allow_hosts
            .iter()
            .any(|allowed| allowed == host)
    }) {
        bail!("configure the hosted router's public URL before exporting client settings");
    }
    Ok(url.to_string())
}

fn json_config(body: &str) -> Result<Value> {
    if body.trim().is_empty() {
        return Ok(json!({}));
    }
    // Parse errors can quote source secrets; expose only a fixed diagnosis.
    let parsed: Value = serde_json::from_str(body).map_err(|_| {
        anyhow!("client configuration is invalid JSON; fix it before applying changes")
    })?;
    if !parsed.is_object() {
        bail!("client configuration must be a JSON object");
    }
    Ok(parsed)
}

fn toml_config(body: &str) -> Result<DocumentMut> {
    body.parse()
        .map_err(|_| anyhow!("Codex configuration is invalid TOML; fix it before applying changes"))
}

fn json_at<'a>(root: &'a Value, path: &[String]) -> Option<&'a Value> {
    let mut at = root;
    for key in path {
        at = at.as_object()?.get(key)?;
    }
    Some(at)
}

fn json_set(root: &mut Value, path: &[String], item: Option<Value>) -> Result<()> {
    let (key, parents) = path.split_last().context("empty configuration key")?;
    let mut at = root;
    for key in parents {
        at = at
            .as_object_mut()
            .context("managed client configuration parent must be an object")?
            .entry(key.clone())
            .or_insert_with(|| json!({}));
    }
    let at = at
        .as_object_mut()
        .context("managed client configuration parent must be an object")?;
    match item {
        Some(item) => {
            at.insert(key.clone(), item);
        }
        None => {
            at.remove(key);
        }
    }
    Ok(())
}

fn toml_at<'a>(root: &'a DocumentMut, path: &[String]) -> Option<&'a Item> {
    let (first, rest) = path.split_first()?;
    let mut at = root.get(first)?;
    for key in rest {
        at = at.get(key)?;
    }
    Some(at)
}

fn toml_set(root: &mut DocumentMut, path: &[String], item: Option<Item>) -> Result<()> {
    let (key, parents) = path.split_last().context("empty configuration key")?;
    let mut table = root.as_table_mut();
    for key in parents {
        if !table.contains_key(key) {
            table.insert(key, Item::Table(Table::new()));
        }
        table = table
            .get_mut(key)
            .and_then(Item::as_table_mut)
            .context("managed Codex configuration parent must be a table")?;
    }
    match item {
        Some(mut item) => {
            if let (Some(previous), Some(next)) =
                (table.get(key).and_then(Item::as_value), item.as_value_mut())
            {
                *next.decor_mut() = previous.decor().clone();
            }
            if let Some(current) = table.get_mut(key) {
                *current = item;
            } else {
                table.insert(key, item);
            }
        }
        None => {
            table.remove(key);
        }
    }
    Ok(())
}

fn key(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| s.to_string()).collect()
}

fn owned_parents(owned: &[Vec<String>]) -> Vec<Vec<String>> {
    let mut paths = owned
        .iter()
        .flat_map(|path| (1..path.len()).map(|n| path[..n].to_vec()))
        .collect::<Vec<_>>();
    paths.sort_by(|a, b| b.len().cmp(&a.len()).then(a.cmp(b)));
    paths.dedup();
    paths
}

fn changes(
    client: &str,
    model: &str,
    url: &str,
    token: &str,
    capabilities: &crate::provider_registry::Capabilities,
) -> Vec<(Vec<String>, Value)> {
    match client {
        "claude-code" => vec![
            (key(&["env", "ANTHROPIC_BASE_URL"]), json!(url)),
            (key(&["env", "ANTHROPIC_AUTH_TOKEN"]), json!(token)),
            (key(&["env", "ANTHROPIC_API_KEY"]), json!("")),
            (key(&["env", "ANTHROPIC_MODEL"]), json!(model)),
        ],
        "opencode" => vec![
            (key(&["model"]), json!(format!("agent-router/{model}"))),
            (
                key(&["small_model"]),
                json!(format!("agent-router/{model}")),
            ),
            (
                key(&["provider", "agent-router", "npm"]),
                json!("@ai-sdk/anthropic"),
            ),
            (
                key(&["provider", "agent-router", "name"]),
                json!("Agent Router"),
            ),
            (
                key(&["provider", "agent-router", "options", "baseURL"]),
                json!(format!("{url}/v1")),
            ),
            (
                key(&["provider", "agent-router", "options", "apiKey"]),
                json!(token),
            ),
            (
                key(&["provider", "agent-router", "models", model]),
                json!({"name":model,"tool_call":capabilities.tools,"limit":{"context":capabilities.context_tokens.unwrap_or(128000).min(128000),"output":capabilities.output_tokens.unwrap_or(4096).min(4096)}}),
            ),
        ],
        _ => Vec::new(),
    }
}

fn render(
    client: &str,
    before: &str,
    model: &str,
    url: &str,
    token: &str,
    capabilities: &crate::provider_registry::Capabilities,
) -> Result<(String, Vec<Vec<String>>)> {
    if client == "codex" {
        let mut doc = toml_config(before)?;
        // Legacy passwords use Authorization, as the dedicated token header fails closed.
        let token_header = token.starts_with(crate::auth::tokens::PREFIX) || token == PLACEHOLDER;
        let mut changes = vec![
            (key(&["model"]), value(model)),
            (key(&["model_provider"]), value("agent-router")),
            (key(&["web_search"]), value("disabled")),
            (
                key(&["model_providers", "agent-router", "name"]),
                value("Agent Router"),
            ),
            (
                key(&["model_providers", "agent-router", "base_url"]),
                value(format!("{url}/v1")),
            ),
            (
                key(&["model_providers", "agent-router", "wire_api"]),
                value("responses"),
            ),
            (
                key(&["model_providers", "agent-router", "requires_openai_auth"]),
                value(false),
            ),
            (
                key(&["model_providers", "agent-router", "supports_websockets"]),
                value(false),
            ),
        ];
        changes.push((
            key(&[
                "model_providers",
                "agent-router",
                "http_headers",
                if token_header {
                    "x-agent-router-token"
                } else {
                    "Authorization"
                },
            ]),
            value(if token_header {
                token.to_string()
            } else {
                format!("Bearer {token}")
            }),
        ));
        changes.push((
            key(&[
                "model_providers",
                "agent-router",
                "http_headers",
                if token_header {
                    "Authorization"
                } else {
                    "x-agent-router-token"
                },
            ]),
            Item::None,
        ));
        for (path, item) in &changes {
            toml_set(&mut doc, path, (!item.is_none()).then(|| item.clone()))?;
        }
        Ok((
            doc.to_string(),
            changes.into_iter().map(|(k, _)| k).collect(),
        ))
    } else {
        let mut doc = json_config(before)?;
        let changes = changes(client, model, url, token, capabilities);
        for (path, item) in &changes {
            json_set(&mut doc, path, Some(item.clone()))?;
        }
        Ok((
            format!("{}\n", serde_json::to_string_pretty(&doc)?),
            changes.into_iter().map(|(k, _)| k).collect(),
        ))
    }
}

fn configured(client: &str, body: &str, url: &str) -> bool {
    if client == "codex" {
        return toml_config(body).ok().is_some_and(|doc| {
            doc.get("model_provider").and_then(Item::as_str) == Some("agent-router")
                && toml_at(&doc, &key(&["model_providers", "agent-router", "base_url"]))
                    .and_then(Item::as_str)
                    == Some(format!("{url}/v1").as_str())
        });
    }
    let Ok(doc) = json_config(body) else {
        return false;
    };
    match client {
        "claude-code" => json_at(&doc, &key(&["env", "ANTHROPIC_BASE_URL"]))
            .and_then(Value::as_str)
            .is_some_and(|base| base.trim_end_matches('/') == url),
        "opencode" => {
            doc.get("model")
                .and_then(Value::as_str)
                .is_some_and(|m| m.starts_with("agent-router/"))
                && json_at(
                    &doc,
                    &key(&["provider", "agent-router", "options", "baseURL"]),
                )
                .and_then(Value::as_str)
                    == Some(format!("{url}/v1").as_str())
        }
        _ => false,
    }
}

fn success(body: Value) -> Response {
    let mut response = Json(body).into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    response
}

async fn legacy_credential(state: &AppState, credential: &str) -> bool {
    let mut headers = axum::http::HeaderMap::new();
    let Ok(value) = format!("Bearer {credential}").parse() else {
        return false;
    };
    headers.insert(header::AUTHORIZATION, value);
    matches!(state.auth.check(&headers).await, crate::auth::Verdict::Ok)
}

pub async fn status(
    State(state): State<Arc<AppState>>,
    roots: Option<Extension<Roots>>,
) -> Response {
    let result = tokio::task::spawn_blocking(move || -> Result<Value> {
        let conn=state.reads.get()?;
        let mut models=Vec::new();
        for m in crate::models::catalogue(&conn)? {
            let (service,capabilities)=crate::provider_registry::metadata(&conn,&m.id,m.provider)?;
            models.push(json!({"id":m.id,"provider":m.provider.as_str(),"display_name":m.display_name,"account_id":m.serving,"available":m.serving.is_some(),"service_id":service.id,"native_protocol":service.protocol,"capabilities":capabilities}));
        }
        let roots = if state.config.server { None } else { Some(roots.map(|r| r.0).map(Ok).unwrap_or_else(Roots::here)?) };
        let found = if state.config.server || roots.as_ref().is_some_and(|r| r.home != crate::setup::home_dir().unwrap_or_default()) { Vec::new() } else { crate::clients::here() };
        let clients = IDS.into_iter().map(|id| {
            let path = roots.as_ref().and_then(|r| r.path(id, None).ok());
            let found = found.iter().find(|c| c.client == id);
            let configured = path.as_deref().and_then(|p| read(p).ok()).is_some_and(|body| configured(id, &body, &router_url(&state)));
            json!({"id":id,"name":match id {"claude-code"=>"Claude Code","codex"=>"Codex",_=>"OpenCode"},
                "protocol":if id=="codex"{"responses"}else{"messages"},
                "installed":found.is_some() || path.as_ref().is_some_and(|p| p.exists()),
                "version":found.and_then(|c| c.cli.as_ref()).and_then(|c| c.version.as_ref()),
                "configured":configured,"managed":path.as_deref().is_some_and(|p|kept(&state,id,p).exists()),
                "config_path":path.as_ref().map(|p|p.display().to_string()),"can_apply":!state.config.server})
        }).collect::<Vec<_>>();
        Ok(json!({"mode":if state.config.server{"server"}else{"local"},"url":router_url(&state),"clients":clients,"models":models}))
    }).await;
    match result {
        Ok(Ok(body)) => success(body),
        Ok(Err(e)) => failed(e),
        Err(e) => failed(e),
    }
}

struct Plan {
    body: Value,
    path: Option<PathBuf>,
    before: String,
    after: String,
    owned: Vec<Vec<String>>,
    credential: Option<(PathBuf, String)>,
}

async fn plan(
    state: &AppState,
    roots: Option<Roots>,
    client: &str,
    req: &Setup,
    applying: bool,
) -> Result<Plan> {
    valid_client(client)?;
    let url = selected_url(state, req.url.as_deref())?;
    let model = {
        let conn = state.reads.get()?;
        crate::models::catalogue(&conn)?
            .into_iter()
            .find(|m| m.id == req.model)
    }
    .context("choose an exact model ID from this router's catalogue")?;
    let account = model
        .serving
        .clone()
        .context("the selected model has no serving provider account")?;
    let protocol = if client == "codex" {
        "responses"
    } else {
        "messages"
    };
    let (service, capabilities) = {
        let conn = state.reads.get()?;
        crate::provider_registry::metadata(&conn, &model.id, model.provider)?
    };
    let translated = (protocol == "responses")
        != (service.protocol == crate::provider_registry::WireProtocol::Responses);
    let mut warnings = Vec::new();
    let mut checks = vec![
        json!({"id":"model","label":"Model and route","status":"pass","detail":format!("{} routes to {} account {}",model.id,model.provider.as_str(),account)}),
    ];
    let credential = req.token.as_deref().filter(|s| !s.is_empty());
    if let Some(credential) = credential {
        if credential.starts_with(crate::auth::tokens::PREFIX) {
            let token = crate::auth::tokens::check(&state.config.db_path, credential)
                .await
                .map_err(|_| anyhow!("router access token is invalid, expired or revoked"))?;
            if token.scope != crate::auth::tokens::Scope::Client {
                warnings.push(format!("This {} token has broader permissions than model requests; create a Client token for this client.",token.scope.as_str()));
            }
        } else if !legacy_credential(state, credential).await {
            bail!("router credential is invalid; use a Client access token or the legacy router password");
        } else {
            warnings.push("The legacy password grants administration access; use a Client access token for this client.".into());
        }
        checks.push(json!({"id":"router_auth","label":"Router credential","status":"pass","detail":"Credential is active on this router"}));
    } else {
        if applying && state.config.server {
            bail!("a hosted client needs a router access token");
        }
        checks.push(json!({"id":"router_auth","label":"Router credential","status":"warning","detail":if state.config.server{"Replace the exported placeholder with a Client access token"}else{"Loopback requests are trusted; create a Client token for access from other machines"}}));
    }
    let mut subscription = false;
    let selected_signer = match crate::provider_registry::resolve(state, &model.id).await {
        Ok(Some(route)) => Ok(route.signer),
        Ok(None) => state.signer(&account).await,
        Err(error) => Err(error),
    };
    match selected_signer {
        Ok(signer) => {
            subscription = signer.openai_chatgpt
                || signer.provider == Provider::Kimi
                || signer.token.starts_with("sk-ant-oat");
            checks.push(json!({"id":"provider_auth","label":"Provider credential","status":"pass","detail":"The router can resolve the selected account's credential; inference and entitlement were not tested"}));
        }
        Err(_) => {
            if applying {
                bail!("the selected provider account's credential is unavailable; repair it in Accounts first");
            }
            checks.push(json!({"id":"provider_auth","label":"Provider credential","status":"fail","detail":"The selected account's credential is unavailable; repair it in Accounts"}));
        }
    }
    if translated {
        warnings.push("Translation supports portable text, images and tools. Provider-hosted tools, encrypted reasoning and stored history references require the provider's native protocol.".into());
        if client == "codex" {
            warnings.push("Cross-provider Codex compaction is unavailable. Start a fresh session before the context fills; provider-hosted web search is disabled.".into());
        } else {
            warnings.push("Anthropic token counting and stop sequences are unavailable when translating to OpenAI Responses.".into());
        }
    }
    if client == "codex" && !translated {
        warnings.push("Web search is disabled in this portable setup; enable it separately for native OpenAI sessions if supported by the selected model.".into());
    }
    if client == "opencode" {
        warnings.push("This configuration targets OpenCode V1 with the Anthropic SDK. Conservative context/output limits are 128,000/4,096 tokens; provider entitlements still apply.".into());
    }
    if !capabilities.tools {
        warnings.push("This route declares tools unavailable; a coding client may require a model with tool support.".into());
    }
    if !capabilities.images {
        warnings.push("This route declares image inputs unavailable.".into());
    }
    if !capabilities.reasoning {
        warnings.push(
            "This route declares reasoning unavailable; disable reasoning requests in the client."
                .into(),
        );
    }
    if client == "codex" && !translated && !capabilities.compaction {
        warnings.push("This route declares Responses compaction unavailable. Start a fresh session before the context fills.".into());
    }
    if subscription {
        warnings.push("This account uses a native subscription login. Protocol support does not establish that the provider permits this client; validate its entitlement separately.".into());
    }
    let path = if state.config.server {
        if req.project.is_some() {
            bail!("hosted exports do not accept a server project path");
        }
        None
    } else {
        Some(
            roots
                .map(Ok)
                .unwrap_or_else(Roots::here)?
                .path(client, req.project.as_deref())?,
        )
    };
    let before = path.as_deref().map(read).transpose()?.unwrap_or_default();
    let supplied = credential.unwrap_or(if applying {
        "agent-router"
    } else {
        PLACEHOLDER
    });
    // OpenCode project config may be committed. Its V1 variable expansion reads
    // this private file without embedding the credential in the project itself.
    let private_credential = if client == "opencode" && req.project.is_some() {
        credential
            .map(|token| -> Result<(PathBuf, String)> {
                let config_path = path.as_ref().unwrap();
                let id = digest(format!("{client}\0{}", config_path.display()));
                let private_path = state
                    .config
                    .db_path
                    .parent()
                    .unwrap_or(Path::new("."))
                    .join("client-credentials")
                    .join(format!("{id}.token"));
                Ok((canonical_target(&private_path)?, token.to_string()))
            })
            .transpose()?
    } else {
        None
    };
    let reference = private_credential
        .as_ref()
        .map(|(path, _)| format!("{{file:{}}}", path.display()));
    if reference.is_some() {
        warnings.push("Apply creates the private credential file outside the project. Copying or downloading this configuration alone does not create that file; create it privately on the client machine before using the file reference.".into());
    }
    let supplied = reference.as_deref().unwrap_or(supplied);
    let (after, owned) = render(client, &before, &model.id, &url, supplied, &capabilities)?;
    // Export only our fragment. An existing config may contain other providers' keys.
    let content = render(client, "", &model.id, &url, supplied, &capabilities)?.0;
    let body = json!({"ok":true,"client":client,"model":model.id,"provider":model.provider.as_str(),"account_id":account,
        "protocol":protocol,"translated":translated,"can_apply":!state.config.server,
        "path":path.as_ref().map(|p|p.display().to_string()),"format":if client=="codex"{"toml"}else{"json"},
        "content":content,"revision":digest(&before),"warnings":warnings,"checks":checks,"connection_tested":false,"service_id":service.id,"native_protocol":service.protocol,"capabilities":capabilities,
        "credential_path":private_credential.as_ref().map(|(p,_)|p.display().to_string())});
    Ok(Plan {
        body,
        path,
        before,
        after,
        owned,
        credential: private_credential,
    })
}

pub async fn preview(
    State(state): State<Arc<AppState>>,
    Params(client): Params<String>,
    roots: Option<Extension<Roots>>,
    JsonBody(req): JsonBody<Setup>,
) -> Response {
    match plan(&state, roots.map(|r| r.0), &client, &req, false).await {
        Ok(plan) => success(plan.body),
        Err(e) => refuse(StatusCode::BAD_REQUEST, e.to_string()),
    }
}

/// A precise readiness check: no inference request is sent and no spend is incurred.
pub async fn check(
    State(state): State<Arc<AppState>>,
    Params(client): Params<String>,
    roots: Option<Extension<Roots>>,
    JsonBody(req): JsonBody<Setup>,
) -> Response {
    match plan(&state, roots.map(|r| r.0), &client, &req, false).await {
        Ok(mut plan) => {
            plan.body.as_object_mut().unwrap().remove("content");
            success(plan.body)
        }
        Err(e) => refuse(StatusCode::BAD_REQUEST, e.to_string()),
    }
}

fn load_saved(path: &Path) -> Result<Saved> {
    serde_json::from_str(&read(path)?).map_err(|_| {
        anyhow!("saved client restoration state is invalid; settings were left unchanged")
    })
}

fn ensure_provider_owned(
    client: &str,
    path: &Path,
    before: &str,
    url: &str,
    saved: Option<&Saved>,
) -> Result<()> {
    if saved.is_some() || client == "claude-code" {
        return Ok(());
    }
    let exists = if client == "codex" {
        let doc = toml_config(before)?;
        if crate::codex::tracked_config(path)
            && toml_at(&doc, &key(&["model_providers", "agent-router", "base_url"]))
                .and_then(Item::as_str)
                == Some(format!("{url}/v1").as_str())
            && toml_at(&doc, &key(&["model_providers", "agent-router", "wire_api"]))
                .and_then(Item::as_str)
                == Some("responses")
        {
            return Ok(());
        }
        toml_at(&doc, &key(&["model_providers", "agent-router"])).is_some()
    } else {
        json_at(&json_config(before)?, &key(&["provider", "agent-router"])).is_some()
    };
    if exists {
        bail!("a provider named agent-router already exists; rename it before applying a new managed configuration");
    }
    Ok(())
}

fn apply_plan(state: &AppState, client: &str, req: &Setup, plan: Plan) -> Result<Value> {
    let _lock = WRITES
        .lock()
        .map_err(|_| anyhow!("client configuration lock is unavailable"))?;
    let path = plan
        .path
        .context("hosted configuration can only be exported")?;
    if req.revision.as_deref() != Some(digest(&plan.before).as_str()) || read(&path)? != plan.before
    {
        bail!("client settings changed or have not been previewed; preview again before applying");
    }
    let kept = kept(state, client, &path);
    let previous = if kept.exists() {
        Some(load_saved(&kept)?)
    } else {
        None
    };
    if previous
        .as_ref()
        .is_some_and(|s| s.client != client || s.path != path)
    {
        bail!("saved client settings belong to another configuration");
    }
    let url = selected_url(state, req.url.as_deref())?;
    ensure_provider_owned(client, &path, &plan.before, &url, previous.as_ref())?;
    let mut owned = previous
        .as_ref()
        .map(|saved| saved.owned.clone())
        .unwrap_or_default();
    for path in plan.owned {
        if !owned.contains(&path) {
            owned.push(path);
        }
    }
    let mut credential_path = previous.as_ref().and_then(|s| s.credential_path.clone());
    let mut credential_digest = previous.as_ref().and_then(|s| s.credential_digest.clone());
    let mut previous_credential = None;
    if let Some((path, token)) = &plan.credential {
        previous_credential = match std::fs::read(path) {
            Ok(bytes) => Some(bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.into()),
        };
        if previous_credential.as_ref().is_some_and(|bytes| {
            credential_path.as_ref() != Some(path)
                || credential_digest.as_deref() != Some(digest(bytes).as_str())
        }) {
            bail!("private client credential file was edited; preserve or remove it before applying again");
        }
        credential_path = Some(path.clone());
        credential_digest = Some(digest(token));
    }
    let saved = Saved {
        client: client.into(),
        path: path.clone(),
        existed: previous
            .as_ref()
            .map(|s| s.existed)
            .unwrap_or_else(|| path.exists()),
        before: previous.map(|s| s.before).unwrap_or(plan.before.clone()),
        installed: plan.after.clone(),
        owned,
        credential_path,
        credential_digest,
    };
    // Save recovery material before replacing the client file. Both contain secrets.
    let previous_backup = if kept.exists() {
        Some(std::fs::read(&kept)?)
    } else {
        None
    };
    write_private(&kept, serde_json::to_vec_pretty(&saved)?)?;
    let changed = plan.before != plan.after;
    let write_result = (|| -> Result<()> {
        if let Some((credential_path, token)) = &plan.credential {
            write_private(credential_path, token)?;
        }
        write_private(&path, &plan.after)
    })();
    if let Err(error) = write_result {
        if let Some((credential_path, _)) = &plan.credential {
            match previous_credential {
                Some(bytes) => write_private(credential_path, bytes)?,
                None if credential_path.exists() => std::fs::remove_file(credential_path)?,
                None => {}
            }
        }
        match previous_backup {
            Some(bytes) => {
                write_private(&kept, bytes).context(
                    "client write failed and previous restoration state could not be restored",
                )?;
            }
            None => {
                std::fs::remove_file(&kept).context(
                    "client write failed and unused restoration state could not be removed",
                )?;
            }
        }
        return Err(error);
    }
    Ok(
        json!({"ok":true,"client":client,"path":path.display().to_string(),"changed":changed,"warnings":plan.body["warnings"]}),
    )
}

pub async fn apply(
    State(state): State<Arc<AppState>>,
    Params(client): Params<String>,
    roots: Option<Extension<Roots>>,
    JsonBody(req): JsonBody<Setup>,
) -> Response {
    if state.config.server {
        return refuse(
            StatusCode::CONFLICT,
            "hosted routers export client configuration; apply it on the client machine",
        );
    }
    let plan = match plan(&state, roots.map(|r| r.0), &client, &req, true).await {
        Ok(plan) => plan,
        Err(e) => return refuse(StatusCode::BAD_REQUEST, e.to_string()),
    };
    let result = tokio::task::spawn_blocking(move || apply_plan(&state, &client, &req, plan)).await;
    match result {
        Ok(Ok(body)) => success(body),
        Ok(Err(e)) => refuse(StatusCode::CONFLICT, e.to_string()),
        Err(e) => failed(e),
    }
}

fn restore_saved(
    state: &AppState,
    roots: Roots,
    client: &str,
    project: Option<&str>,
) -> Result<Value> {
    valid_client(client)?;
    let _lock = WRITES
        .lock()
        .map_err(|_| anyhow!("client configuration lock is unavailable"))?;
    let path = roots.path(client, project)?;
    let kept = kept(state, client, &path);
    if !kept.exists() {
        return Ok(
            json!({"ok":true,"client":client,"path":path.display().to_string(),"changed":false,"preserved_edits":false}),
        );
    }
    let saved = load_saved(&kept)?;
    if saved.client != client || saved.path != path {
        bail!("saved client settings belong to another configuration");
    }
    let before = read(&path)?;
    let mut preserved_edits = false;
    let after = if client == "codex" {
        let mut current = toml_config(&before)?;
        let installed = toml_config(&saved.installed)?;
        let previous = toml_config(&saved.before)?;
        for key in &saved.owned {
            if toml_at(&current, key).map(Item::to_string)
                == toml_at(&installed, key).map(Item::to_string)
            {
                toml_set(&mut current, key, toml_at(&previous, key).cloned())?;
            } else {
                preserved_edits = true;
            }
        }
        for parent in owned_parents(&saved.owned) {
            if toml_at(&previous, &parent).is_none()
                && toml_at(&current, &parent)
                    .and_then(Item::as_table)
                    .is_some_and(Table::is_empty)
            {
                toml_set(&mut current, &parent, None)?;
            }
        }
        current.to_string()
    } else {
        let mut current = json_config(&before)?;
        let installed = json_config(&saved.installed)?;
        let previous = json_config(&saved.before)?;
        for key in &saved.owned {
            if json_at(&current, key) == json_at(&installed, key) {
                json_set(&mut current, key, json_at(&previous, key).cloned())?;
            } else {
                preserved_edits = true;
            }
        }
        for parent in owned_parents(&saved.owned) {
            if json_at(&previous, &parent).is_none()
                && json_at(&current, &parent)
                    .and_then(Value::as_object)
                    .is_some_and(serde_json::Map::is_empty)
            {
                json_set(&mut current, &parent, None)?;
            }
        }
        format!("{}\n", serde_json::to_string_pretty(&current)?)
    };
    let changed = before != after;
    if !saved.existed && !preserved_edits && (after.trim().is_empty() || after.trim() == "{}") {
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
    } else if changed {
        write_private(&path, &after)?;
    }
    if let Some(credential_path) = saved.credential_path {
        match std::fs::read(&credential_path) {
            Ok(bytes) if saved.credential_digest.as_deref() == Some(digest(&bytes).as_str()) => {
                std::fs::remove_file(credential_path)?
            }
            Ok(_) => preserved_edits = true,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    std::fs::remove_file(kept)?;
    Ok(
        json!({"ok":true,"client":client,"path":path.display().to_string(),"changed":changed,"preserved_edits":preserved_edits}),
    )
}

pub async fn restore(
    State(state): State<Arc<AppState>>,
    Params(client): Params<String>,
    roots: Option<Extension<Roots>>,
    JsonBody(req): JsonBody<Restore>,
) -> Response {
    if state.config.server {
        return refuse(
            StatusCode::CONFLICT,
            "hosted routers do not modify client files; restore them on the client machine",
        );
    }
    let roots = match roots.map(|r| r.0).map(Ok).unwrap_or_else(Roots::here) {
        Ok(roots) => roots,
        Err(e) => return failed(e),
    };
    let result = tokio::task::spawn_blocking(move || {
        restore_saved(&state, roots, &client, req.project.as_deref())
    })
    .await;
    match result {
        Ok(Ok(body)) => success(body),
        Ok(Err(e)) => refuse(StatusCode::CONFLICT, e.to_string()),
        Err(e) => failed(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api_refusals::Lab;

    async fn fixtures(lab: &mut Lab) -> Roots {
        let root = lab
            .state
            .config
            .db_path
            .parent()
            .unwrap()
            .join("fixture-home");
        std::fs::create_dir_all(&root).unwrap();
        let roots = Roots {
            home: root.clone(),
            codex: root.join(".codex"),
            opencode: root.join(".config/opencode"),
        };
        lab.app = lab.app.clone().layer(Extension(roots.clone()));
        let conn = rusqlite::Connection::open(&lab.state.config.db_path).unwrap();
        crate::pricing::ensure_table(&conn).unwrap();
        for (id, provider, model) in [
            ("fixture-anthropic", Provider::Anthropic, "claude-fixture"),
            ("fixture-openai", Provider::OpenAI, "gpt-fixture"),
            ("fixture-kimi", Provider::Kimi, "k3"),
        ] {
            conn.execute("INSERT INTO accounts(id,label,source,provider,active,created_at) VALUES(?1,?1,'token',?2,1,1)", rusqlite::params![id,provider.as_str()]).unwrap();
            crate::pricing::resolve_one(&conn, model).unwrap();
            lab.state.credentials.write().await.insert(
                id.into(),
                crate::proxy::Active {
                    account_id: id.into(),
                    label: id.into(),
                    identity: None,
                    token: format!("PROVIDER-SECRET-{id}"),
                    expires_at_ms: i64::MAX,
                    provider,
                    base_url: None,
                    openai_chatgpt: false,
                    chatgpt_account_id: None,
                },
            );
        }
        roots
    }

    async fn token(lab: &Lab, scope: &str) -> String {
        let (status, body) = lab
            .post(
                "/api/auth/tokens",
                &json!({"name":"fixture","scope":scope}).to_string(),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        body["token"].as_str().unwrap().into()
    }

    async fn preview_and_apply(lab: &Lab, client: &str, mut request: Value) -> Value {
        let (status, preview) = lab
            .post(
                &format!("/api/clients/{client}/preview"),
                &request.to_string(),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{preview}");
        request["revision"] = preview["revision"].clone();
        let (status, applied) = lab
            .post(
                &format!("/api/clients/{client}/apply"),
                &request.to_string(),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{applied}");
        applied
    }

    #[tokio::test]
    async fn hosted_catalogue_offers_exports_without_machine_paths_or_apply() {
        let lab = Lab::hosted("client-setup-hosted-contract").await;
        let (status, body) = lab.get("/api/clients").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["mode"], "server");
        for id in ["claude-code", "codex", "opencode"] {
            let client = body["clients"]
                .as_array()
                .unwrap()
                .iter()
                .find(|c| c["id"] == id)
                .unwrap();
            assert_eq!(client["can_apply"], false);
            assert!(client["config_path"].is_null());
        }
        let (status, body) = lab
            .post(
                "/api/clients/codex/apply",
                &json!({"model":"k3"}).to_string(),
            )
            .await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(body["ok"], false);
    }

    #[tokio::test]
    async fn preview_refuses_unknown_models_and_clients_with_a_json_error() {
        let lab = Lab::hosted("client-setup-invalid-contract").await;
        for path in ["/api/clients/unknown/preview", "/api/clients/codex/preview"] {
            let (status, body) = lab
                .post(path, &json!({"model":"unlisted-model"}).to_string())
                .await;
            assert_eq!(status, StatusCode::BAD_REQUEST);
            assert_eq!(body["ok"], false);
            assert!(body["error"].as_str().is_some());
        }
    }

    #[tokio::test]
    async fn local_configuration_and_restore_preserve_user_edits_for_all_clients() {
        for client in IDS {
            let mut lab = Lab::local(&format!("client-setup-roundtrip-{client}"));
            let roots = fixtures(&mut lab).await;
            let path = roots.path(client, None).unwrap();
            let original = if client == "codex" {
                "# User comment\nmodel = \"before\"\nmodel_provider = \"previous\"\nweb_search = \"cached\"\n[sandbox_workspace_write]\nnetwork_access = false\n".into()
            } else {
                json!({"model":"before","theme":"original","env":{"PERSONAL_SECRET":"never-export-me","ANTHROPIC_AUTH_TOKEN":"old-secret"}}).to_string()
            };
            write_private(&path, &original).unwrap();
            let credential = token(&lab, "client").await;
            preview_and_apply(&lab, client, json!({"model":"k3","token":credential})).await;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                assert_eq!(
                    std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                    0o600
                );
                assert_eq!(
                    std::fs::metadata(kept(&lab.state, client, &path))
                        .unwrap()
                        .permissions()
                        .mode()
                        & 0o777,
                    0o600
                );
            }
            let installed = read(&path).unwrap();
            assert!(installed.contains(&credential));
            let changed = if client == "codex" {
                let mut doc =
                    toml_config(&installed.replace("model = \"k3\"", "model = \"user-selected\""))
                        .unwrap();
                doc["user_added"] = value("keep");
                doc["model_providers"]["agent-router"]["name"] = value("My router name");
                doc.to_string()
            } else {
                let mut doc = json_config(&installed).unwrap();
                doc["theme"] = json!("user-selected");
                doc["user_added"] = json!("keep");
                if client == "opencode" {
                    doc["provider"]["agent-router"]["name"] = json!("My router name");
                }
                serde_json::to_string_pretty(&doc).unwrap()
            };
            write_private(&path, &changed).unwrap();
            let (status, body) = lab
                .post(&format!("/api/clients/{client}/restore"), "{}")
                .await;
            assert_eq!(status, StatusCode::OK, "{body}");
            assert!(!kept(&lab.state, client, &path).exists());
            let restored = read(&path).unwrap();
            assert!(!restored.contains(&credential));
            if client == "codex" {
                assert!(restored.contains("# User comment"));
                let doc = toml_config(&restored).unwrap();
                assert_eq!(doc["model"].as_str(), Some("user-selected"));
                assert_eq!(doc["model_provider"].as_str(), Some("previous"));
                assert_eq!(doc["web_search"].as_str(), Some("cached"));
                assert_eq!(doc["user_added"].as_str(), Some("keep"));
                assert_eq!(
                    doc["model_providers"]["agent-router"]["name"].as_str(),
                    Some("My router name")
                );
                assert_eq!(body["preserved_edits"], true);
            } else {
                let doc = json_config(&restored).unwrap();
                assert_eq!(doc["theme"], "user-selected");
                assert_eq!(doc["user_added"], "keep");
                assert_eq!(doc["env"]["PERSONAL_SECRET"], "never-export-me");
                assert_eq!(doc["env"]["ANTHROPIC_AUTH_TOKEN"], "old-secret");
                if client == "opencode" {
                    assert_eq!(doc["provider"]["agent-router"]["name"], "My router name");
                }
            }
        }
    }

    #[tokio::test]
    async fn previews_and_status_never_export_other_credentials_and_checks_are_precise() {
        let mut lab = Lab::local("client-setup-redaction");
        let roots = fixtures(&mut lab).await;
        write_private(&roots.path("claude-code",None).unwrap(),json!({"env":{"ANTHROPIC_AUTH_TOKEN":"UNRELATED-SECRET","ANTHROPIC_API_KEY":"ANOTHER-SECRET"}}).to_string()).unwrap();
        let credential = token(&lab, "client").await;
        for client in IDS {
            let request = json!({"model":"k3","token":credential});
            let (status, preview) = lab
                .post(
                    &format!("/api/clients/{client}/preview"),
                    &request.to_string(),
                )
                .await;
            assert_eq!(status, StatusCode::OK, "{preview}");
            assert!(preview["content"].as_str().unwrap().contains(&credential));
            assert!(!preview.to_string().contains("UNRELATED-SECRET"));
            assert!(!preview.to_string().contains("ANOTHER-SECRET"));
            assert!(!preview.to_string().contains("PROVIDER-SECRET"));
            let (status, checked) = lab
                .post(
                    &format!("/api/clients/{client}/check"),
                    &request.to_string(),
                )
                .await;
            assert_eq!(status, StatusCode::OK, "{checked}");
            assert_eq!(checked["connection_tested"], false);
            assert!(checked.get("content").is_none());
            assert!(!checked.to_string().contains(&credential));
        }
        let (status, body) = lab.get("/api/clients").await;
        assert_eq!(status, StatusCode::OK);
        assert!(!body.to_string().contains("SECRET"));
        assert!(!body.to_string().contains(&credential));
    }

    #[tokio::test]
    async fn hosted_exports_carry_explicit_capability_limits_and_never_touch_home_files() {
        let mut lab = Lab::hosted("client-setup-hosted-export").await;
        let roots = fixtures(&mut lab).await;
        for client in IDS {
            let path = roots.path(client, None).unwrap();
            assert!(!path.exists());
            for model in ["claude-fixture", "gpt-fixture", "k3"] {
                let (status, body) = lab
                    .post(
                        &format!("/api/clients/{client}/preview"),
                        &json!({"model":model}).to_string(),
                    )
                    .await;
                assert_eq!(status, StatusCode::OK, "{body}");
                assert!(body["path"].is_null());
                assert!(body["content"].as_str().unwrap().contains(PLACEHOLDER));
                assert_eq!(
                    body["translated"],
                    (client == "codex") != (model == "gpt-fixture")
                );
                assert_eq!(body["can_apply"], false);
            }
            for action in ["apply", "restore"] {
                let (status, body) = lab
                    .post(
                        &format!("/api/clients/{client}/{action}"),
                        &if action == "apply" {
                            json!({"model":"k3"}).to_string()
                        } else {
                            "{}".into()
                        },
                    )
                    .await;
                assert_eq!(status, StatusCode::CONFLICT, "{body}");
            }
            assert!(!path.exists());
        }
    }

    #[tokio::test]
    async fn applying_requires_review_and_refuses_stale_configuration() {
        let mut lab = Lab::local("client-setup-stale");
        let roots = fixtures(&mut lab).await;
        let client = "claude-code";
        let path = roots.path(client, None).unwrap();
        let request = json!({"model":"k3"});
        let (status, body) = lab
            .post("/api/clients/claude-code/apply", &request.to_string())
            .await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert!(!path.exists());
        let (_, preview) = lab
            .post("/api/clients/claude-code/preview", &request.to_string())
            .await;
        write_private(&path, "{\"editor\":\"changed\"}").unwrap();
        let (status, body) = lab
            .post(
                "/api/clients/claude-code/apply",
                &json!({"model":"k3","revision":preview["revision"]}).to_string(),
            )
            .await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert_eq!(read(&path).unwrap(), "{\"editor\":\"changed\"}");
        assert!(!kept(&lab.state, client, &path).exists());
    }

    #[tokio::test]
    async fn invalid_or_revoked_tokens_are_refused_and_broader_credentials_have_warnings() {
        let mut lab = Lab::local("client-setup-token-checks");
        fixtures(&mut lab).await;
        for scope in ["client", "machine", "admin"] {
            let credential = token(&lab, scope).await;
            let request = json!({"model":"gpt-fixture","token":credential});
            let (status, body) = lab
                .post("/api/clients/codex/preview", &request.to_string())
                .await;
            assert_eq!(status, StatusCode::OK, "{body}");
            assert_eq!(
                body["warnings"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|w| w.as_str().unwrap().contains("broader permissions")),
                scope != "client"
            );
            let metadata = crate::auth::tokens::check(&lab.state.config.db_path, &credential)
                .await
                .unwrap();
            let (_, body) = lab
                .post(
                    "/api/auth/tokens",
                    &json!({"name":"another","scope":"client"}).to_string(),
                )
                .await;
            assert!(body["ok"].as_bool().unwrap());
            rusqlite::Connection::open(&lab.state.config.db_path)
                .unwrap()
                .execute(
                    "UPDATE auth_tokens SET revoked_at=1 WHERE id=?1",
                    [metadata.id],
                )
                .unwrap();
            let (status, body) = lab
                .post("/api/clients/codex/preview", &request.to_string())
                .await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
            assert!(!body.to_string().contains(&credential));
        }
        lab.state
            .auth
            .set(&lab.state.config.db_path, "legacy-router-password")
            .await
            .unwrap();
        let (status, body) = lab
            .post(
                "/api/clients/codex/preview",
                &json!({"model":"k3","token":"legacy-router-password"}).to_string(),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(body["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w.as_str().unwrap().contains("legacy password")));
        let (status, body) = lab
            .post(
                "/api/clients/codex/preview",
                &json!({"model":"k3","token":"arr_bad"}).to_string(),
            )
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    }

    #[tokio::test]
    async fn project_configuration_uses_fixed_paths_and_never_accepts_codex_project_files() {
        let mut lab = Lab::local("client-setup-projects");
        let roots = fixtures(&mut lab).await;
        let project = roots.home.join("project");
        std::fs::create_dir_all(&project).unwrap();
        for client in ["claude-code", "opencode"] {
            let path = roots.path(client, project.to_str()).unwrap();
            preview_and_apply(&lab, client, json!({"model":"k3","project":project})).await;
            assert!(path.exists());
            assert!(!roots.path(client, None).unwrap().exists());
            let (status, body) = lab
                .post(
                    &format!("/api/clients/{client}/restore"),
                    &json!({"project":project}).to_string(),
                )
                .await;
            assert_eq!(status, StatusCode::OK, "{body}");
            assert!(!path.exists());
        }
        let (status, body) = lab
            .post(
                "/api/clients/codex/preview",
                &json!({"model":"k3","project":project}).to_string(),
            )
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert!(!roots.path("codex", None).unwrap().exists());
    }

    #[tokio::test]
    async fn reapplying_models_and_credentials_restores_the_original_file_for_all_clients() {
        for client in IDS {
            let mut lab = Lab::local(&format!("client-setup-reapply-{client}"));
            let roots = fixtures(&mut lab).await;
            let path = roots.path(client, None).unwrap();
            let first = token(&lab, "client").await;
            let second = token(&lab, "client").await;
            preview_and_apply(&lab, client, json!({"model":"k3","token":first})).await;
            preview_and_apply(&lab, client, json!({"model":"k3-256k","token":second})).await;
            let installed = read(&path).unwrap();
            assert!(installed.contains(&second));
            assert!(!installed.contains(&first));
            let (status, body) = lab
                .post(&format!("/api/clients/{client}/restore"), "{}")
                .await;
            assert_eq!(status, StatusCode::OK, "{body}");
            assert!(!path.exists());
        }
    }

    #[tokio::test]
    async fn codex_rotation_removes_competing_header_and_restore_returns_original_headers() {
        let mut lab = Lab::local("client-setup-codex-auth-rotation");
        let roots = fixtures(&mut lab).await;
        let path = roots.path("codex", None).unwrap();
        write_private(&path,"model = \"before\"\nmodel_provider = \"previous\"\n[model_providers.agent-router]\nbase_url = \"https://previous.invalid/v1\"\nwire_api = \"responses\"\n[model_providers.agent-router.http_headers]\nAuthorization = \"Bearer ORIGINAL-USER-HEADER\"\nx-personal = \"preserve\"\n").unwrap();
        // An exact managed snapshot permits reapplying this existing provider.
        let original = read(&path).unwrap();
        let saved = Saved {
            client: "codex".into(),
            path: path.clone(),
            existed: true,
            before: original.clone(),
            installed: original.clone(),
            owned: Vec::new(),
            credential_path: None,
            credential_digest: None,
        };
        write_private(
            &kept(&lab.state, "codex", &path),
            serde_json::to_vec(&saved).unwrap(),
        )
        .unwrap();
        let credential = token(&lab, "client").await;
        preview_and_apply(&lab, "codex", json!({"model":"k3","token":credential})).await;
        let first = toml_config(&read(&path).unwrap()).unwrap();
        let prefix = ["model_providers", "agent-router", "http_headers"];
        assert!(toml_at(
            &first,
            &key(&[prefix[0], prefix[1], prefix[2], "Authorization"])
        )
        .is_none());
        lab.state
            .auth
            .set(&lab.state.config.db_path, "legacy-router-password")
            .await
            .unwrap();
        preview_and_apply(
            &lab,
            "codex",
            json!({"model":"k3","token":"legacy-router-password"}),
        )
        .await;
        let current = read(&path).unwrap();
        assert!(!current.contains(&credential));
        assert!(!current.contains("x-agent-router-token"));
        assert!(current.contains("Bearer legacy-router-password"));
        let (status, body) = lab.post("/api/clients/codex/restore", "{}").await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let restored = read(&path).unwrap();
        assert!(restored.contains("Bearer ORIGINAL-USER-HEADER"));
        assert!(restored.contains("x-personal"));
        assert!(!restored.contains("legacy-router-password"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn symlinked_config_files_and_project_directories_survive_apply_and_restore() {
        use std::os::unix::fs::symlink;
        let mut lab = Lab::local("client-setup-symlinks");
        let roots = fixtures(&mut lab).await;
        let real = roots.home.join("real-codex.toml");
        write_private(&real, "model = \"original\"\n").unwrap();
        std::fs::create_dir_all(&roots.codex).unwrap();
        let link = roots.codex.join("config.toml");
        symlink(&real, &link).unwrap();
        preview_and_apply(&lab, "codex", json!({"model":"k3"})).await;
        assert!(std::fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
        assert!(read(&real).unwrap().contains("k3"));
        assert_eq!(
            lab.post("/api/clients/codex/restore", "{}").await.0,
            StatusCode::OK
        );
        assert!(read(&real).unwrap().contains("original"));
        assert!(std::fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
        let project = roots.home.join("project");
        let real_dir = roots.home.join("private-claude");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::create_dir_all(&real_dir).unwrap();
        let directory_link = project.join(".claude");
        symlink(&real_dir, &directory_link).unwrap();
        preview_and_apply(&lab, "claude-code", json!({"model":"k3","project":project})).await;
        assert!(real_dir.join("settings.local.json").exists());
        assert_eq!(
            lab.post(
                "/api/clients/claude-code/restore",
                &json!({"project":project}).to_string()
            )
            .await
            .0,
            StatusCode::OK
        );
        assert!(!real_dir.join("settings.local.json").exists());
        assert!(std::fs::symlink_metadata(directory_link)
            .unwrap()
            .file_type()
            .is_symlink());
    }

    #[tokio::test]
    async fn opencode_project_keeps_rotatable_credentials_outside_the_project_and_preserves_edits()
    {
        let mut lab = Lab::local("client-setup-opencode-private");
        let roots = fixtures(&mut lab).await;
        let project = roots.home.join("project");
        std::fs::create_dir_all(&project).unwrap();
        let first = token(&lab, "client").await;
        let request = json!({"model":"k3","token":first,"project":project});
        let (status, preview) = lab
            .post("/api/clients/opencode/preview", &request.to_string())
            .await;
        assert_eq!(status, StatusCode::OK, "{preview}");
        assert!(!preview["content"].as_str().unwrap().contains(&first));
        let credential_path = PathBuf::from(preview["credential_path"].as_str().unwrap());
        assert!(!credential_path.starts_with(&project));
        assert!(!credential_path.exists());
        preview_and_apply(&lab, "opencode", request).await;
        assert_eq!(read(&credential_path).unwrap(), first);
        let config = read(&project.join("opencode.json")).unwrap();
        assert!(!config.contains(&first));
        assert!(config.contains(&format!("{{file:{}}}", credential_path.display())));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&credential_path)
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        let second = token(&lab, "client").await;
        preview_and_apply(
            &lab,
            "opencode",
            json!({"model":"k3","token":second,"project":project}),
        )
        .await;
        assert_eq!(read(&credential_path).unwrap(), second);
        write_private(&credential_path, "USER-EDITED-PRIVATE-TOKEN").unwrap();
        let (status, body) = lab
            .post(
                "/api/clients/opencode/restore",
                &json!({"project":project}).to_string(),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["preserved_edits"], true);
        assert!(!project.join("opencode.json").exists());
        assert_eq!(read(&credential_path).unwrap(), "USER-EDITED-PRIVATE-TOKEN");
        std::fs::remove_file(&credential_path).unwrap();
        preview_and_apply(
            &lab,
            "opencode",
            json!({"model":"k3","token":second,"project":project}),
        )
        .await;
        assert_eq!(
            lab.post(
                "/api/clients/opencode/restore",
                &json!({"project":project}).to_string()
            )
            .await
            .0,
            StatusCode::OK
        );
        assert!(!credential_path.exists());
    }

    #[tokio::test]
    async fn malformed_configs_do_not_leak_source_secrets_or_create_restore_state() {
        let mut lab = Lab::local("client-setup-malformed");
        let roots = fixtures(&mut lab).await;
        for client in IDS {
            let path = roots.path(client, None).unwrap();
            let malformed = "TOKEN-SECRET-THAT-MUST-NOT-APPEAR = \"unterminated";
            write_private(&path, malformed).unwrap();
            let (status, body) = lab
                .post(
                    &format!("/api/clients/{client}/preview"),
                    &json!({"model":"k3"}).to_string(),
                )
                .await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
            assert!(!body.to_string().contains("TOKEN-SECRET"));
            assert_eq!(read(&path).unwrap(), malformed);
            assert!(!kept(&lab.state, client, &path).exists());
        }
    }
}
