//! Reversible setup for clients that speak Messages or Responses.
//!
//! A local router sets up the clients on its own machine. A hosted router exports their
//! configuration, and sets up Claude Code and Codex on a connected machine through its agent
//! (AGEN-0200): the router decides the model, the URL, the credential and the catalogue — an
//! `Intent` — and the machine's agent reads, merges and writes its own file against the
//! revision the reader reviewed, keeping the same ownership ledger a local Apply keeps.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use axum::extract::{Path as Params, Query, State};
use axum::http::{header, Method, StatusCode};
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
/// The clients a hosted router may set up on a machine, in their global configuration.
const ON_A_MACHINE: [&str; 2] = ["claude-code", "codex"];
/// How long a machine's agent is given to read or write its client settings.
const MACHINE_WAIT: Duration = Duration::from_secs(30);
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

/// What a router decided for one client: everything a plan needs but the file itself, which is
/// what a hosted router sends a machine's agent to render against the file it holds.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Intent {
    client: String,
    model: String,
    url: String,
    token: String,
    capabilities: crate::provider_registry::Capabilities,
    catalogue: Option<crate::models::ClientCatalogue>,
}

/// A machine's client file as its agent read it: where it is, and its revision.
struct Remote {
    path: PathBuf,
    revision: String,
}

/// Which machine a hosted router's Clients page is about, when it is about one.
#[derive(Deserialize)]
pub struct Which {
    #[serde(default)]
    machine: Option<String>,
}

/// What a machine's agent sets its clients up against: the router it connects to, and where
/// its clients and its ledger are — its own home and data directory, or a test's.
#[derive(Clone)]
pub(crate) struct OnMachine {
    server: String,
    roots: Option<Roots>,
    ledger: Option<PathBuf>,
}

impl OnMachine {
    pub(crate) fn here(server: &str) -> Self {
        Self { server: server.trim_end_matches('/').to_string(), roots: None, ledger: None }
    }

    fn roots(&self) -> Result<Roots> {
        self.roots.clone().map(Ok).unwrap_or_else(Roots::here)
    }

    /// The agent's ledger sits beside its store, as a local router's does.
    fn db(&self) -> Result<PathBuf> {
        self.ledger.clone().map(Ok).unwrap_or_else(crate::config::db_path)
    }
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

/// The file a plan writes: what the router decided, merged into the file as it is now, with
/// Codex's catalogue beside it when there is a file to put it beside.
fn rendered(path: Option<&Path>, before: &str, intent: &Intent) -> Result<(String, Vec<Vec<String>>)> {
    let (mut after, owned) = render(&intent.client, before, &intent.model, &intent.url, &intent.token, &intent.capabilities)?;
    if let (Some(path), Some(metadata)) = (path, &intent.catalogue) {
        after = crate::codex::catalogue::prepare(path, before, &after, metadata)?.after;
    }
    Ok((after, owned))
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
    Query(which): Query<Which>,
    roots: Option<Extension<Roots>>,
) -> Response {
    let asking = state.clone();
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
    let body = match result {
        Ok(Ok(body)) => body,
        Ok(Err(e)) => return failed(e),
        Err(e) => return failed(e),
    };
    let Some(id) = which.machine else { return success(body) };
    match ask_machine(&asking, &id, "/agent/clients", json!({})).await {
        Ok(found) => success(overlaid(body, &id, &found)),
        Err((status, why)) => refuse(status, why),
    }
}

/// The Clients page for one machine: this router's catalogue, and that machine's Claude Code and
/// Codex as its agent read them. OpenCode stays an export.
fn overlaid(mut body: Value, id: &str, found: &Value) -> Value {
    if let Some(rows) = body["clients"].as_array_mut() {
        for row in rows {
            let Some(theirs) = found["clients"].as_array().and_then(|f| f.iter().find(|c| c["id"] == row["id"])) else { continue };
            for key in ["installed", "version", "configured", "managed", "config_path"] {
                row[key] = theirs[key].clone();
            }
            row["can_apply"] = json!(true);
        }
    }
    body["machine"] = json!(id);
    body
}

fn on_a_machine(client: &str) -> Result<()> {
    valid_client(client)?;
    if !ON_A_MACHINE.contains(&client) {
        bail!("a router sets up Claude Code and Codex on a machine; export OpenCode's configuration instead");
    }
    Ok(())
}

/// A refusal in this router's words, with its status.
type Refused = (StatusCode, String);

fn offline(label: &str) -> String {
    format!("{label} is offline: its clients can be set up once its agent connects again")
}

/// Ask a machine's agent about its clients: its answer, or this router's words for why there
/// is none — not hosted, no such machine, offline, an agent that predates these routes, or the
/// agent's own refusal.
async fn ask_machine(state: &AppState, id: &str, path: &str, body: Value) -> Result<Value, Refused> {
    use crate::machines::Relay;
    if !state.config.server {
        return Err((StatusCode::BAD_REQUEST, "a local router sets up the clients on its own machine; omit machine".into()));
    }
    let Some(machine) = state.machines.online(id) else {
        return Err(match state.machines.seen().into_iter().find(|s| s.id == id) {
            Some(seen) => (StatusCode::SERVICE_UNAVAILABLE, offline(&seen.label)),
            None => (StatusCode::NOT_FOUND, format!("no machine {id} has connected to this router")),
        });
    };
    let label = machine.hello.label.clone();
    let headers = vec![("content-type".to_string(), "application/json".to_string())];
    let response = match machine.request(&Method::POST, path, headers, body.to_string().as_bytes(), MACHINE_WAIT).await {
        Ok(response) => response,
        Err(Relay::Offline(_)) => return Err((StatusCode::SERVICE_UNAVAILABLE, offline(&label))),
        Err(Relay::Failed(why)) => return Err((StatusCode::BAD_GATEWAY, why)),
        Err(Relay::Unknown) => return Err((StatusCode::BAD_GATEWAY, format!("{label} did not answer"))),
    };
    let status = response.status();
    let Ok(Ok(bytes)) = tokio::time::timeout(MACHINE_WAIT, axum::body::to_bytes(response.into_body(), 1 << 20)).await else {
        return Err((StatusCode::BAD_GATEWAY, format!("{label} did not finish answering")));
    };
    let said: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    if status.is_success() {
        return Ok(said);
    }
    // An agent installed before these routes has none of them.
    if matches!(status, StatusCode::NOT_FOUND | StatusCode::METHOD_NOT_ALLOWED) {
        return Err((StatusCode::CONFLICT, format!("{label}'s agent predates setting up its clients from here: run the installer on it again")));
    }
    let words = said["error"].as_str().map(str::to_owned).unwrap_or_else(|| String::from_utf8_lossy(&bytes).trim().to_string());
    Err((if status.is_client_error() { status } else { StatusCode::BAD_GATEWAY }, format!("{label} refused: {words}")))
}

/// Where a machine keeps a client's configuration, and its revision now, as its agent reads it.
async fn machine_file(state: &AppState, id: &str, client: &str) -> Result<Remote, Refused> {
    on_a_machine(client).map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    let found = ask_machine(state, id, "/agent/clients", json!({})).await?;
    let row = found["clients"].as_array().and_then(|rows| rows.iter().find(|c| c["id"] == client));
    match row.and_then(|r| Some((r["config_path"].as_str()?, r["revision"].as_str()?))) {
        Some((path, revision)) => Ok(Remote { path: path.into(), revision: revision.into() }),
        None => Err((StatusCode::BAD_GATEWAY, format!("the machine did not say where {client}'s configuration is"))),
    }
}

/// Apply on a machine: this router decides the plan again, as for its own Apply, and the
/// machine's agent writes it only if its file is still at the revision that was reviewed.
async fn apply_on_machine(state: &AppState, id: &str, client: &str, req: &Setup) -> Response {
    let remote = match machine_file(state, id, client).await {
        Ok(remote) => remote,
        Err((status, why)) => return refuse(status, why),
    };
    let Some(revision) = req.revision.clone() else {
        return refuse(StatusCode::CONFLICT, "client settings changed or have not been previewed; preview again before applying");
    };
    let plan = match plan(state, None, client, req, true, Some(&remote)).await {
        Ok(plan) => plan,
        Err(e) => return refuse(StatusCode::BAD_REQUEST, e.to_string()),
    };
    let reviewed = json!({ "intent": plan.intent, "revision": revision });
    match ask_machine(state, id, &format!("/agent/clients/{client}/apply"), reviewed).await {
        Ok(mut applied) => {
            applied["warnings"] = plan.body["warnings"].clone();
            success(applied)
        }
        Err((status, why)) => refuse(status, why),
    }
}

/// The routes a machine's agent sets its clients up with, for the router it connects to: its
/// Claude Code and Codex, read, applied from a reviewed plan, and restored. Nothing else of a
/// machine's files is served.
pub(crate) fn machine_routes() -> axum::Router {
    use axum::routing::post;
    axum::Router::new()
        .route("/agent/clients", post(machine_status))
        .route("/agent/clients/{client}/apply", post(machine_apply))
        .route("/agent/clients/{client}/restore", post(machine_restore))
}

/// What a router sends a machine to apply: its decision, and the revision the reader reviewed.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Reviewed {
    intent: Intent,
    revision: String,
}

/// On a machine's agent: its Claude Code and Codex, read for the router it connects to, with
/// each file's revision for a plan to be reviewed against.
async fn machine_status(Extension(on): Extension<OnMachine>) -> Response {
    let read_all = tokio::task::spawn_blocking(move || -> Result<Value> {
        let roots = on.roots()?;
        let db = on.db()?;
        let found = if roots.home == crate::setup::home_dir().unwrap_or_default() { crate::clients::here() } else { Vec::new() };
        let clients = ON_A_MACHINE
            .into_iter()
            .map(|id| -> Result<Value> {
                let path = roots.path(id, None)?;
                let body = read(&path)?;
                let found = found.iter().find(|c| c.client == id);
                Ok(json!({"id":id,"installed":found.is_some() || path.exists(),
                    "version":found.and_then(|c| c.cli.as_ref()).and_then(|c| c.version.as_ref()),
                    "configured":configured(id, &body, &on.server),"managed":kept_at(&db, id, &path).exists(),
                    "config_path":path.display().to_string(),"revision":digest(&body)}))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(json!({ "clients": clients }))
    })
    .await;
    match read_all {
        Ok(Ok(body)) => success(body),
        Ok(Err(e)) => refuse(StatusCode::CONFLICT, e.to_string()),
        Err(e) => failed(e),
    }
}

/// On a machine's agent: the router's plan merged into this machine's file, with the same
/// revision check and ledger as a local Apply, and only for the router this agent connects to.
async fn machine_apply(Extension(on): Extension<OnMachine>, Params(client): Params<String>, JsonBody(req): JsonBody<Reviewed>) -> Response {
    let applied = tokio::task::spawn_blocking(move || -> Result<Value> {
        on_a_machine(&client)?;
        if req.intent.client != client {
            bail!("that plan is for another client");
        }
        if req.intent.url.trim_end_matches('/') != on.server {
            bail!("this machine's agent connects to {}; it sets its clients up for that router alone", on.server);
        }
        let path = on.roots()?.path(&client, None)?;
        let before = read(&path)?;
        let (after, owned) = rendered(Some(&path), &before, &req.intent)?;
        let plan = Plan { body: json!({ "warnings": [] }), path: Some(path), before, after, owned, credential: None, catalogue: req.intent.catalogue.clone(), intent: req.intent };
        apply_plan(&on.db()?, &client, Some(req.revision.as_str()), plan)
    })
    .await;
    match applied {
        Ok(Ok(body)) => success(body),
        Ok(Err(e)) => refuse(StatusCode::CONFLICT, e.to_string()),
        Err(e) => failed(e),
    }
}

/// On a machine's agent: put back what an Apply from the router replaced, as a local Restore does.
async fn machine_restore(Extension(on): Extension<OnMachine>, Params(client): Params<String>) -> Response {
    let restored = tokio::task::spawn_blocking(move || -> Result<Value> {
        on_a_machine(&client)?;
        restore_saved(&on.db()?, on.roots()?, &client, None)
    })
    .await;
    match restored {
        Ok(Ok(body)) => success(body),
        Ok(Err(e)) => refuse(StatusCode::CONFLICT, e.to_string()),
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
    catalogue: Option<crate::models::ClientCatalogue>,
    intent: Intent,
}

/// A client's plan on this router: its own machine's file locally, an export when hosted, and
/// with `remote` a connected machine's file, which its agent renders and writes.
async fn plan(
    state: &AppState,
    roots: Option<Roots>,
    client: &str,
    req: &Setup,
    applying: bool,
    remote: Option<&Remote>,
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
        checks.push(json!({"id":"router_auth","label":"Router credential","status":"warning","detail":if remote.is_some(){"Give a Client access token: Apply writes it into this machine's configuration"}else if state.config.server{"Replace the exported placeholder with a Client access token"}else{"Loopback requests are trusted; create a Client token for access from other machines"}}));
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
    let path = if let Some(remote) = remote {
        if req.project.is_some() {
            bail!("a machine's clients are set up in their global configuration; omit project");
        }
        Some(remote.path.clone())
    } else if state.config.server {
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
    // A machine's file is read, and rendered, by its own agent.
    let before = if remote.is_some() { String::new() } else { path.as_deref().map(read).transpose()?.unwrap_or_default() };
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
    // Export only our fragment. An existing config may contain other providers' keys.
    let mut content = render(client, "", &model.id, &url, supplied, &capabilities)?.0;
    let catalogue = if client == "codex" {
        if !capabilities.tools {bail!("Codex requires a model route with tools enabled");}
        let metadata = {let conn=state.reads.get()?;crate::models::client_catalogue(&conn)?};
        if let Some(path) = &path {
            let mut fragment = toml_config(&content)?;
            fragment["model_catalog_json"] = value(path.parent().unwrap().join(crate::codex::catalogue::FILE).to_string_lossy().as_ref());
            content = fragment.to_string();
        }
        warnings.push("Model changes apply when Codex starts. Refresh with `agent-router codex models refresh`; finish active turns, then resume the same CLI session or reload the Codex editor window. Refresh does not stop running clients.".into());
        Some(metadata)
    } else {None};
    let intent = Intent { client: client.into(), model: model.id.clone(), url: url.clone(), token: supplied.into(), capabilities: capabilities.clone(), catalogue: catalogue.clone() };
    let (after, owned) = if remote.is_some() { (String::new(), Vec::new()) } else { rendered(path.as_deref(), &before, &intent)? };
    let body = json!({"ok":true,"client":client,"model":model.id,"provider":model.provider.as_str(),"account_id":account,
        "protocol":protocol,"translated":translated,"can_apply":!state.config.server || remote.is_some(),
        "path":path.as_ref().map(|p|p.display().to_string()),"format":if client=="codex"{"toml"}else{"json"},
        "content":content,"revision":remote.map_or_else(|| digest(&before), |r| r.revision.clone()),"warnings":warnings,"checks":checks,"connection_tested":false,"service_id":service.id,"native_protocol":service.protocol,"capabilities":capabilities,
        "credential_path":private_credential.as_ref().map(|(p,_)|p.display().to_string()),
        "catalogue_content":catalogue.as_ref().map(crate::codex::catalogue::native).transpose()?.map(|v|serde_json::to_string_pretty(&v)).transpose()?,
        "catalogue_path":if client=="codex" {path.as_ref().map(|p|p.parent().unwrap().join(crate::codex::catalogue::FILE).display().to_string())} else {None},
        "catalogue_qualified_version":if client=="codex" {Some(crate::codex::catalogue::QUALIFIED_VERSION)} else {None}});
    Ok(Plan {
        body,
        path,
        before,
        after,
        owned,
        credential: private_credential,
        catalogue,
        intent,
    })
}

pub async fn preview(
    State(state): State<Arc<AppState>>,
    Params(client): Params<String>,
    Query(which): Query<Which>,
    roots: Option<Extension<Roots>>,
    JsonBody(req): JsonBody<Setup>,
) -> Response {
    let remote = match which.machine.as_deref().map(|id| machine_file(&state, id, &client)) {
        Some(asked) => match asked.await {
            Ok(remote) => Some(remote),
            Err((status, why)) => return refuse(status, why),
        },
        None => None,
    };
    match plan(&state, roots.map(|r| r.0), &client, &req, false, remote.as_ref()).await {
        Ok(plan) => success(plan.body),
        Err(e) => refuse(StatusCode::BAD_REQUEST, e.to_string()),
    }
}

/// A precise readiness check: no inference request is sent and no spend is incurred.
pub async fn check(
    State(state): State<Arc<AppState>>,
    Params(client): Params<String>,
    Query(which): Query<Which>,
    roots: Option<Extension<Roots>>,
    JsonBody(req): JsonBody<Setup>,
) -> Response {
    let remote = match which.machine.as_deref().map(|id| machine_file(&state, id, &client)) {
        Some(asked) => match asked.await {
            Ok(remote) => Some(remote),
            Err((status, why)) => return refuse(status, why),
        },
        None => None,
    };
    match plan(&state, roots.map(|r| r.0), &client, &req, false, remote.as_ref()).await {
        Ok(mut plan) => {
            plan.body.as_object_mut().unwrap().remove("content");
            plan.body.as_object_mut().unwrap().remove("catalogue_content");
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

/// Write a reviewed plan, keeping in the ledger beside `db` what it owns and what was there
/// before: a local router's own Apply, and a machine's agent's.
fn apply_plan(db: &Path, client: &str, revision: Option<&str>, plan: Plan) -> Result<Value> {
    let _lock = WRITES
        .lock()
        .map_err(|_| anyhow!("client configuration lock is unavailable"))?;
    let path = plan
        .path
        .context("hosted configuration can only be exported")?;
    let _catalogue_lock = if client=="codex" {Some(crate::codex::catalogue::WRITES.lock().map_err(|_|anyhow!("Codex catalogue lock unavailable"))?)} else {None};
    if revision != Some(digest(&plan.before).as_str()) || read(&path)? != plan.before
    {
        bail!("client settings changed or have not been previewed; preview again before applying");
    }
    let kept = kept_at(db, client, &path);
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
    ensure_provider_owned(client, &path, &plan.before, &plan.intent.url, previous.as_ref())?;
    let catalogue = plan.catalogue.as_ref().map(|metadata|crate::codex::catalogue::prepare(&path,&plan.before,&plan.after,metadata)).transpose()?;
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
    let changed = plan.before != plan.after || catalogue.as_ref().is_some_and(|p|p.changed());
    let mut catalogue_applied = false;
    let write_result = (|| -> Result<()> {
        if let Some(catalogue)=&catalogue {catalogue.apply()?;catalogue_applied=true;}
        if let Some((credential_path, token)) = &plan.credential {
            write_private(credential_path, token)?;
        }
        write_private(&path, &plan.after)
    })();
    if let Err(error) = write_result {
        if catalogue_applied {catalogue.as_ref().unwrap().rollback()?;}
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
    Query(which): Query<Which>,
    roots: Option<Extension<Roots>>,
    JsonBody(req): JsonBody<Setup>,
) -> Response {
    if let Some(id) = which.machine.as_deref() {
        return apply_on_machine(&state, id, &client, &req).await;
    }
    if state.config.server {
        return refuse(
            StatusCode::CONFLICT,
            "hosted routers export client configuration; apply it on the client machine, or choose a connected machine",
        );
    }
    let plan = match plan(&state, roots.map(|r| r.0), &client, &req, true, None).await {
        Ok(plan) => plan,
        Err(e) => return refuse(StatusCode::BAD_REQUEST, e.to_string()),
    };
    let db = state.config.db_path.clone();
    let result = tokio::task::spawn_blocking(move || apply_plan(&db, &client, req.revision.as_deref(), plan)).await;
    match result {
        Ok(Ok(body)) => success(body),
        Ok(Err(e)) => refuse(StatusCode::CONFLICT, e.to_string()),
        Err(e) => failed(e),
    }
}

fn restore_saved(
    db: &Path,
    roots: Roots,
    client: &str,
    project: Option<&str>,
) -> Result<Value> {
    valid_client(client)?;
    let _lock = WRITES
        .lock()
        .map_err(|_| anyhow!("client configuration lock is unavailable"))?;
    let path = roots.path(client, project)?;
    let kept = kept_at(db, client, &path);
    if !kept.exists() {
        return Ok(
            json!({"ok":true,"client":client,"path":path.display().to_string(),"changed":false,"preserved_edits":false}),
        );
    }
    let saved = load_saved(&kept)?;
    if saved.client != client || saved.path != path {
        bail!("saved client settings belong to another configuration");
    }
    let original = read(&path)?;
    let mut preserved_edits = if client=="codex" {crate::codex::catalogue::restore(&path)?} else {false};
    let before = read(&path)?;
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
    let changed = original != after;
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
    Query(which): Query<Which>,
    roots: Option<Extension<Roots>>,
    JsonBody(req): JsonBody<Restore>,
) -> Response {
    if let Some(id) = which.machine.as_deref() {
        if let Err(e) = on_a_machine(&client) {
            return refuse(StatusCode::BAD_REQUEST, e.to_string());
        }
        if req.project.is_some() {
            return refuse(StatusCode::BAD_REQUEST, "a machine's clients are set up in their global configuration; omit project");
        }
        return match ask_machine(&state, id, &format!("/agent/clients/{client}/restore"), json!({})).await {
            Ok(restored) => success(restored),
            Err((status, why)) => refuse(status, why),
        };
    }
    if state.config.server {
        return refuse(
            StatusCode::CONFLICT,
            "hosted routers do not modify client files; restore them on the client machine, or choose a connected machine",
        );
    }
    let roots = match roots.map(|r| r.0).map(Ok).unwrap_or_else(Roots::here) {
        Ok(roots) => roots,
        Err(e) => return failed(e),
    };
    let db = state.config.db_path.clone();
    let result = tokio::task::spawn_blocking(move || {
        restore_saved(&db, roots, &client, req.project.as_deref())
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

    /// A machine connected to this hosted router as `agent run` connects one, serving its client
    /// setup over a fixture home and a ledger of its own — or, `predates`, an agent installed
    /// before those routes, which serves none of them. Answers the task serving its connection.
    async fn connect(lab: &Lab, roots: Option<Roots>, id: &str) -> tokio::task::JoinHandle<()> {
        use tokio_tungstenite::tungstenite::client::IntoClientRequest;
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = lab.app.clone().into_make_service_with_connect_info::<std::net::SocketAddr>();
        tokio::spawn(async move { axum::serve(listener, app).await });
        let machine = match roots {
            Some(roots) => machine_routes().layer(Extension(OnMachine {
                server: "https://router.invalid".into(),
                roots: Some(roots),
                ledger: Some(machine_db(lab)),
            })),
            None => axum::Router::new(),
        };
        let mut request = format!("ws://{addr}{}", crate::tunnel::PATH).into_client_request().unwrap();
        request.headers_mut().insert(header::AUTHORIZATION, format!("Bearer {}", lab.password.unwrap()).parse().unwrap());
        let tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
        let (socket, _) = tokio_tungstenite::client_async(request, tcp).await.unwrap();
        let hello = crate::tunnel::Hello {
            machine: id.into(),
            label: format!("the test's {id}"),
            platform: std::env::consts::OS.into(),
            version: "test".into(),
            build: "test".into(),
        };
        let serving = tokio::spawn(async move {
            let _ = crate::agent::serve(socket, &hello, machine).await;
        });
        let online = async {
            while lab.state.machines.online(id).is_none() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        };
        tokio::time::timeout(Duration::from_secs(10), online).await.expect("the machine comes online");
        serving
    }

    /// Where the stand-in machine's agent keeps its store, and so its ledger.
    fn machine_db(lab: &Lab) -> PathBuf {
        lab.state.config.db_path.parent().unwrap().join("machine-store").join("agent-router.db")
    }

    fn row<'a>(status: &'a Value, client: &str) -> &'a Value {
        status["clients"].as_array().unwrap().iter().find(|c| c["id"] == client).unwrap()
    }

    #[tokio::test]
    async fn remote_client_setup_reads_applies_and_restores_a_machines_claude_code_and_codex() {
        let mut lab = Lab::hosted("remote-client-setup").await;
        let roots = fixtures(&mut lab).await;
        let credential = token(&lab, "client").await;
        let _machine = connect(&lab, Some(roots.clone()), "m-new").await;
        for client in ON_A_MACHINE {
            let path = roots.path(client, None).unwrap();
            let original = if client == "codex" { "# Mine\nmodel = \"before\"\n".to_string() } else { json!({"theme": "dark"}).to_string() };
            write_private(&path, &original).unwrap();

            // The page reads the machine's file, not the router's.
            let (status, page) = lab.get("/api/clients?machine=m-new").await;
            assert_eq!(status, StatusCode::OK, "{page}");
            assert_eq!(page["mode"], "server");
            assert_eq!(page["machine"], "m-new");
            assert_eq!(row(&page, client)["config_path"], path.display().to_string());
            assert_eq!((row(&page, client)["installed"].clone(), row(&page, client)["configured"].clone()), (json!(true), json!(false)));
            assert_eq!(row(&page, client)["can_apply"], true);
            assert_eq!(row(&page, "opencode")["can_apply"], false, "OpenCode stays an export");

            let ask = |revision: Option<&Value>| {
                let mut body = json!({"model": "k3", "token": credential});
                if let Some(r) = revision {
                    body["revision"] = r.clone();
                }
                body.to_string()
            };
            let (status, preview) = lab.post(&format!("/api/clients/{client}/preview?machine=m-new"), &ask(None)).await;
            assert_eq!(status, StatusCode::OK, "{preview}");
            assert_eq!(preview["path"], path.display().to_string());
            assert_eq!(preview["revision"], digest(&original), "the machine's file is what was reviewed");
            assert_eq!(preview["can_apply"], true);

            // A file edited after the review is not written over.
            write_private(&path, format!("{original}\n")).unwrap();
            let (status, stale) = lab.post(&format!("/api/clients/{client}/apply?machine=m-new"), &ask(Some(&preview["revision"]))).await;
            assert_eq!(status, StatusCode::CONFLICT, "{stale}");
            assert!(stale["error"].as_str().unwrap().contains("preview again"), "{stale}");
            assert_eq!(read(&path).unwrap(), format!("{original}\n"));

            let (_, preview) = lab.post(&format!("/api/clients/{client}/preview?machine=m-new"), &ask(None)).await;
            let (status, applied) = lab.post(&format!("/api/clients/{client}/apply?machine=m-new"), &ask(Some(&preview["revision"]))).await;
            assert_eq!(status, StatusCode::OK, "{applied}");
            assert_eq!(applied["changed"], true);
            assert!(applied["warnings"].as_array().is_some_and(|w| !w.is_empty()), "the router's warnings: {applied}");
            let installed = read(&path).unwrap();
            assert!(installed.contains(&credential), "the Client token given is written");
            assert!(configured(client, &installed, "https://router.invalid"), "{installed}");
            if client == "codex" {
                assert!(installed.contains("# Mine"), "the reader's own lines are kept: {installed}");
            } else {
                assert_eq!(json_config(&installed).unwrap()["theme"], "dark");
            }
            // The machine keeps the ledger; the router keeps nothing of the machine's file.
            assert!(kept_at(&machine_db(&lab), client, &path).exists());
            assert!(!kept(&lab.state, client, &path).exists());
            let (_, page) = lab.get("/api/clients?machine=m-new").await;
            assert_eq!((row(&page, client)["configured"].clone(), row(&page, client)["managed"].clone()), (json!(true), json!(true)));

            let (status, restored) = lab.post(&format!("/api/clients/{client}/restore?machine=m-new"), "{}").await;
            assert_eq!(status, StatusCode::OK, "{restored}");
            assert_eq!(restored["changed"], true);
            let back = read(&path).unwrap();
            assert!(!back.contains(&credential), "{back}");
            assert!(!kept_at(&machine_db(&lab), client, &path).exists());
            if client == "codex" {
                assert_eq!(toml_config(&back).unwrap()["model"].as_str(), Some("before"));
            } else {
                assert_eq!(json_config(&back).unwrap(), json!({"theme": "dark"}));
            }
        }
    }

    #[tokio::test]
    async fn remote_client_setup_refuses_offline_machines_old_agents_and_other_clients_in_words() {
        let mut lab = Lab::hosted("remote-client-setup-refusals").await;
        let roots = fixtures(&mut lab).await;
        let credential = token(&lab, "client").await;
        let setup = json!({"model": "k3", "token": credential}).to_string();

        let (status, body) = lab.get("/api/clients?machine=nobody").await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
        assert!(body["error"].as_str().unwrap().contains("no machine nobody"), "{body}");

        let _old = connect(&lab, None, "m-old").await;
        let (status, body) = lab.get("/api/clients?machine=m-old").await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert!(body["error"].as_str().unwrap().contains("predates"), "{body}");
        let (status, body) = lab.post("/api/clients/codex/restore?machine=m-old", "{}").await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert!(body["error"].as_str().unwrap().contains("run the installer on it again"), "{body}");

        let gone = connect(&lab, Some(roots.clone()), "m-gone").await;
        let (status, body) = lab.post("/api/clients/opencode/preview?machine=m-gone", &setup).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert!(body["error"].as_str().unwrap().contains("Claude Code and Codex"), "{body}");
        let (status, body) = lab.post("/api/clients/codex/apply?machine=m-gone", &json!({"model": "k3", "revision": digest("")}).to_string()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert!(body["error"].as_str().unwrap().contains("access token"), "a machine is given a Client token: {body}");
        let (status, body) = lab.post("/api/clients/codex/apply?machine=m-gone", &setup).await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert!(body["error"].as_str().unwrap().contains("preview again"), "no review, no write: {body}");
        assert!(!roots.path("codex", None).unwrap().exists());

        gone.abort();
        let offline = async {
            while lab.state.machines.online("m-gone").is_some() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        };
        tokio::time::timeout(Duration::from_secs(10), offline).await.expect("the machine goes offline");
        let (status, body) = lab.post("/api/clients/codex/preview?machine=m-gone", &setup).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
        assert!(body["error"].as_str().unwrap().contains("the test's m-gone is offline"), "{body}");

        let local = Lab::local("remote-client-setup-local");
        let (status, body) = local.get("/api/clients?machine=m-new").await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert!(body["error"].as_str().unwrap().contains("omit machine"), "{body}");
    }

    /// A machine's agent writes for the router it connects to alone, whatever a router sends.
    #[tokio::test]
    async fn remote_client_setup_is_refused_by_an_agent_for_another_router() {
        use tower::ServiceExt;
        let routes = machine_routes().layer(Extension(OnMachine::here("https://router.invalid/")));
        let intent = json!({"client": "codex", "model": "k3", "url": "https://elsewhere.invalid", "token": "arr_x", "capabilities": {}, "catalogue": null});
        let request = axum::http::Request::post("/agent/clients/codex/apply")
            .header(header::CONTENT_TYPE, "application/json")
            .body(axum::body::Body::from(json!({"intent": intent, "revision": digest("")}).to_string()))
            .unwrap();
        let answer = routes.oneshot(request).await.unwrap();
        assert_eq!(answer.status(), StatusCode::CONFLICT);
        let words: Value = serde_json::from_slice(&axum::body::to_bytes(answer.into_body(), 1 << 16).await.unwrap()).unwrap();
        assert!(words["error"].as_str().unwrap().contains("connects to https://router.invalid"), "{words}");
    }
}
