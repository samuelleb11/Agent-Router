//! The machine agent: what makes a developer machine's sessions visible to a hosted
//! router. See `docs/hosting-machine-agent.md`; the frames are `tunnel`'s.
//!
//! A hosted router cannot reach into a laptop, so the agent reaches out: it holds one
//! WebSocket open to the router and answers the requests that come down it — list the
//! sessions here, follow this one, answer that prompt — by running them through the very
//! `live` handlers a local router serves, in process. It listens on nothing. Only the
//! routes in `routes` can be asked for: nothing about setting the machine up, installing
//! anything, or reading files the live view does not — and one sync of the Library, which
//! places only what the router's Library lists, through the agent's own checks.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use axum::body::Body;
use axum::routing::{get, post};
use axum::{Json, Router};
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::Message;
use tower::ServiceExt;

use crate::tunnel::{self, Head, Hello};

const PING_EVERY: Duration = Duration::from_secs(20);
const IDLE_LIMIT: Duration = Duration::from_secs(60);
const CONNECT_WAIT: Duration = Duration::from_secs(15);
/// Reconnection backs off to this, so a router that is down for a while is asked twice
/// a minute rather than hammered.
const MAX_BACKOFF: Duration = Duration::from_secs(30);

pub const USAGE: &str = "\
agent-router agent — connect this machine's Claude Code sessions to a hosted router

USAGE:
  agent-router agent install --server <url> [--project <dir>] [--label <name>]
                                     [--no-service] [--no-config] [--no-library]
                                     [--no-auto-update]
                                     check the password, save it, point Claude Code at
                                     the server, and start the agent at login.
                                     --no-library: never place the hosted router's
                                     Library in ~/.claude (remembered; --library undoes it)
                                     --no-auto-update: never fetch the router's agent
                                     build (remembered; --auto-update undoes it)
  agent-router agent uninstall       undo exactly what install changed
  agent-router agent status          what is installed, and whether it runs
  agent-router agent [--server <url>] [--label <name>] [--data-dir <dir>]
                                     run the agent: a keeper that runs its worker, and
                                     hands over to a new build without going offline
  agent-router agent --ensure        run only if no agent's keeper runs here

The password comes from AGENT_ROUTER_PASSWORD, from standard input, or from a prompt;
`install` saves it, owner-only, where the agent reads it from then on.
";

#[derive(Debug, Default)]
struct RunArgs {
    server: Option<String>,
    label: Option<String>,
    data_dir: Option<PathBuf>,
    ensure: bool,
    /// The keeper's worker (`--worker`): the tunnel, with no lock of its own.
    worker: bool,
}

fn parse(args: &[String]) -> Result<RunArgs, String> {
    let mut out = RunArgs::default();
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        i += 1;
        let (name, inline) = match arg.split_once('=') {
            Some((n, v)) if n.starts_with("--") => (n, Some(v.to_string())),
            _ => (arg.as_str(), None),
        };
        let mut value = || -> Result<String, String> {
            if let Some(v) = &inline {
                return Ok(v.clone());
            }
            let v = args.get(i).cloned().ok_or_else(|| format!("{name} needs a value"))?;
            i += 1;
            Ok(v)
        };
        match name {
            "--server" => out.server = Some(value()?),
            "--label" => out.label = Some(value()?),
            "--data-dir" => out.data_dir = Some(PathBuf::from(value()?)),
            "--ensure" if inline.is_none() => out.ensure = true,
            "--worker" if inline.is_none() => out.worker = true,
            other => return Err(format!("unknown option {other:?}")),
        }
    }
    Ok(out)
}

/// `agent-router agent …`. A failure is one line on standard error and exit status 1,
/// which is what the editor extension shows the person who clicked.
pub async fn main(args: &[String]) -> Result<()> {
    if let Err(e) = dispatch(args).await {
        eprintln!("Error: {e:#}");
        std::process::exit(1);
    }
    Ok(())
}

async fn dispatch(args: &[String]) -> Result<()> {
    match args.first().map(String::as_str) {
        Some("install") => install(&args[1..]).await,
        Some("uninstall") => uninstall(&args[1..]),
        Some("status") => status(&args[1..]),
        Some("-h" | "--help" | "help") => {
            print!("{USAGE}");
            Ok(())
        }
        _ => match parse(args) {
            Ok(run_args) => run(run_args).await,
            Err(why) => {
                eprintln!("{why}\n\n{USAGE}");
                std::process::exit(2);
            }
        },
    }
}

/// What `agent install` saves, owner-only, in the data directory.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Saved {
    #[serde(default)]
    server: Option<String>,
    #[serde(default)]
    password: Option<String>,
    /// This machine's id, made on the first run and kept: it survives a rename.
    #[serde(default)]
    machine: Option<String>,
    /// What to call the machine, when `install` was told.
    #[serde(default)]
    label: Option<String>,
    /// `false` when installed with `--no-library`: the agent never places the hosted
    /// router's Library in `~/.claude`. Absent, it does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    library: Option<bool>,
    /// `false` when installed with `--no-auto-update`: the keeper never fetches its router's
    /// agent build. Absent, it does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    update: Option<bool>,
    /// The router build the keeper last checked its own binary against, so each router
    /// build costs one download at most (`update_from`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    checked_build: Option<String>,
}

/// What `agent install` saved that the Library's commands need (`library::cli`).
pub(crate) struct Connection {
    pub server: String,
    pub password: String,
    pub library: Option<bool>,
    /// This machine's id and name, which a publish from here signs with.
    pub machine: Option<String>,
    pub label: String,
}

pub(crate) fn connection() -> Result<Connection> {
    let saved = load_saved();
    let server = saved.server.ok_or_else(|| anyhow!("this machine has no agent: run `agent-router agent install --server <url>`"))?;
    let password = std::env::var("AGENT_ROUTER_PASSWORD")
        .ok()
        .filter(|p| !p.is_empty())
        .or(saved.password)
        .ok_or_else(|| anyhow!("no password saved: run `agent-router agent install --server <url>` again"))?;
    let label = saved.label.unwrap_or_else(host_name);
    Ok(Connection { server, password, library: saved.library, machine: saved.machine, label })
}

fn saved_path() -> Result<PathBuf> {
    Ok(crate::config::data_dir()?.join("agent.json"))
}

fn load_saved() -> Saved {
    saved_path().map(|p| load_saved_at(&p)).unwrap_or_default()
}

fn load_saved_at(path: &Path) -> Saved {
    std::fs::read_to_string(path).ok().and_then(|raw| serde_json::from_str(&raw).ok()).unwrap_or_default()
}

fn store_saved(saved: &Saved) -> Result<()> {
    store_saved_at(&saved_path()?, saved)
}

/// Written whole and owner-only: it holds the router password.
fn store_saved_at(path: &Path, saved: &Saved) -> Result<()> {
    let path = path.to_path_buf();
    let tmp = path.with_extension("json.tmp");
    let json = serde_json::to_vec_pretty(saved)?;
    {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        use std::io::Write;
        let mut file = options.open(&tmp).with_context(|| format!("writing {}", tmp.display()))?;
        file.write_all(&json)?;
        file.sync_all()?;
    }
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

/// A random version-4 UUID.
fn new_machine_id() -> String {
    use ring::rand::SecureRandom;
    let mut b = [0u8; 16];
    ring::rand::SystemRandom::new().fill(&mut b).expect("system randomness is available");
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}

/// What the machine calls itself, without the `.local` mDNS adds.
fn host_name() -> String {
    #[cfg(unix)]
    let name = {
        let mut buf = [0u8; 256];
        // SAFETY: the buffer is valid for its length; gethostname NUL-terminates within it.
        let ok = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) } == 0;
        let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        ok.then(|| String::from_utf8_lossy(&buf[..end]).into_owned()).unwrap_or_default()
    };
    #[cfg(not(unix))]
    let name = std::env::var("COMPUTERNAME").unwrap_or_default();
    let name = name.trim().trim_end_matches(".local").to_string();
    if name.is_empty() {
        "this machine".to_string()
    } else {
        name
    }
}

/// Where a router is, as a WebSocket address and the pieces needed to dial it.
#[derive(Debug, PartialEq)]
struct Target {
    tls: bool,
    host: String,
    port: u16,
    /// Any path the router is served under, without a trailing slash.
    base: String,
}

impl Target {
    fn parse(server: &str) -> Result<Self> {
        let server = server.trim().trim_end_matches('/');
        let (tls, rest) = if let Some(r) = server.strip_prefix("https://") {
            (true, r)
        } else if let Some(r) = server.strip_prefix("http://") {
            (false, r)
        } else {
            bail!("the server must be an https:// (or http://) URL, got {server:?}");
        };
        let (authority, base) = match rest.find('/') {
            Some(i) => (&rest[..i], rest[i..].to_string()),
            None => (rest, String::new()),
        };
        let default_port = if tls { 443 } else { 80 };
        let (host, port) = if let Some(inner) = authority.strip_prefix('[') {
            let (h, after) = inner.split_once(']').ok_or_else(|| anyhow!("an unterminated IPv6 address in {server:?}"))?;
            let port = match after.strip_prefix(':') {
                Some(p) => p.parse().map_err(|_| anyhow!("a bad port in {server:?}"))?,
                None => default_port,
            };
            (h.to_string(), port)
        } else {
            match authority.rsplit_once(':') {
                Some((h, p)) => (h.to_string(), p.parse().map_err(|_| anyhow!("a bad port in {server:?}"))?),
                None => (authority.to_string(), default_port),
            }
        };
        if host.is_empty() {
            bail!("no host in {server:?}");
        }
        Ok(Self { tls, host, port, base })
    }

    fn ws_url(&self) -> String {
        let host = if self.host.contains(':') { format!("[{}]", self.host) } else { self.host.clone() };
        let default_port = if self.tls { 443 } else { 80 };
        let port = if self.port == default_port { String::new() } else { format!(":{}", self.port) };
        format!("{}://{host}{port}{}{}", if self.tls { "wss" } else { "ws" }, self.base, tunnel::PATH)
    }
}

/// The routes a router may ask an agent for: the live view, what the notifier needs, and
/// one sync of the Library (`library::push`). That one is the only route that writes here,
/// and it carries nothing: the agent pulls the manifest and the blobs itself and checks every
/// digest, so all a router can have a machine place in `~/.claude` is what its Library
/// lists. Nothing else is served — no setup, no shim, no files. `library` is `None` on a
/// machine installed with `--no-library`, whose answer to a sync says so.
fn routes(library: Option<Library>) -> Router {
    use crate::live;
    Router::new()
        .route("/api/live/sessions", get(live::sessions))
        .route("/api/live/stream", get(live::stream))
        .route("/api/live/stats", get(live::stats))
        .route("/api/live/prompt", get(live::prompt))
        .route("/api/live/project", get(live::project_stats))
        .route("/api/live/send", post(live::send))
        .route("/api/live/interrupt", post(live::interrupt))
        .route("/api/live/answer", post(live::answer))
        .route("/api/live/model", post(live::set_model))
        .route("/api/live/effort", post(live::set_effort))
        .route("/api/live/mode", post(live::set_mode))
        .route("/api/live/aside", post(live::aside))
        .route("/api/live/start", post(crate::codex::live::start_route))
        .route("/api/live/resume", post(crate::codex::live::resume_route))
        .route("/api/live/compact", post(crate::codex::live::compact_route))
        .route("/api/live/archive", post(live::archive))
        .route("/agent/waiting", get(waiting))
        .route("/agent/still-waiting", get(still_waiting))
        .route(crate::library::push::PATH, post(move || library_sync(library.clone())))
}

/// What the agent syncs the Library with: where on this machine, from which router, and
/// who it says it is when it reports.
#[derive(Clone)]
pub(crate) struct Library {
    pub places: crate::library::sync::Places,
    pub router: Arc<crate::library::sync::Router>,
    pub machine: String,
    pub label: String,
}

impl Library {
    /// Sync once, and log what changed. A sync the agent ran on its own is reported to the
    /// router; one the router asked for is answered instead.
    async fn sync(&self, report_it: bool) -> Result<crate::library::sync::Report> {
        let report = crate::library::sync::run_once(self.places.clone(), self.router.clone(), None)
            .await?
            .ok_or_else(|| anyhow!("this machine takes no part in the Library"))?;
        for line in report.items.iter().filter(|l| l.outcome != crate::library::sync::Outcome::InStep) {
            tracing::info!(kind = %line.kind, name = %line.name, version = line.version, outcome = ?line.outcome, kept = ?line.kept, why = ?line.why, "the Library");
        }
        if report_it {
            if let Err(e) = self.router.report(&self.machine, &self.label, &report).await {
                tracing::warn!("the Library's report did not reach the router: {e:#}");
            }
        }
        Ok(report)
    }
}

/// `POST /agent/library/sync`, from the router: sync now, and answer with the report.
async fn library_sync(library: Option<Library>) -> axum::response::Response {
    use axum::response::IntoResponse;
    let Some(library) = library else {
        return crate::refusal::refuse(
            axum::http::StatusCode::CONFLICT,
            "this machine takes no part in the Library: it was installed with --no-library",
        );
    };
    match library.sync(false).await {
        Ok(report) => Json(report).into_response(),
        Err(e) => crate::refusal::failed(format!("the Library could not be synced: {e:#}")),
    }
}

/// The rows of Claude Code's `/model` picker the router keeps in the user's `settings`: one
/// for each non-Claude model of `catalogue` (`GET /api/models`) while that Claude Code talks
/// to `server`, which its settings say as `ANTHROPIC_BASE_URL`; none once it does not. The
/// owner's own rows are never touched (`setup::sync_picker_rows`). Says whether the file
/// changed.
fn picker_rows_for(settings: &Path, server: &str, catalogue: &[Value]) -> Result<bool> {
    let ours = crate::setup::base_url_in(settings)?
        .is_some_and(|url| url.trim_end_matches('/') == server.trim_end_matches('/'));
    let rows = if ours { crate::setup::picker_models(catalogue) } else { Vec::new() };
    crate::setup::sync_picker_rows(settings, &rows)
}

/// The router's catalogue, `GET /api/models`. None from a router from before it, which
/// answers 404.
async fn catalogue_of(router: &crate::library::sync::Router, server: &str) -> Result<Option<Vec<Value>>> {
    let (status, _, body) = router.send(axum::http::Method::GET, "/api/models", Vec::new(), Bytes::new()).await?;
    if status == axum::http::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !status.is_success() {
        bail!("{server} answered {status} for its models");
    }
    Ok(Some(serde_json::from_slice(&body).context("the router's models were not a list")?))
}

/// What keeping this machine's clients in step changed.
struct ClientsKept {
    /// Kimi Code's `config.toml`: None when there was nothing to set up.
    kimi: Option<bool>,
    codex: Option<bool>,
}

/// Keep this machine's clients in step with the router's catalogue: Claude Code's `/model`
/// rows (`picker_rows_for`), and Kimi Code's `config.toml` (`kimi::config`), set up the first
/// time on `install` and put back at every sync after, since Kimi Code updates itself.
async fn keep_clients(router: &crate::library::sync::Router, server: &str, password: &str, install: bool) -> Result<ClientsKept> {
    let Some(catalogue) = catalogue_of(router, server).await? else { return Ok(ClientsKept { kimi: None, codex: None }) };
    let (status,_,body)=router.send(axum::http::Method::GET,"/_router/models",Vec::new(),Bytes::new()).await?;
    let native:Option<crate::models::ClientCatalogue>=if status==axum::http::StatusCode::NOT_FOUND {None}
        else if status.is_success() {Some(serde_json::from_slice(&body).context("invalid router client model metadata")?)}
        else {bail!("{server} answered {status} for its client model metadata")};
    let settings = crate::setup::settings_path(None)?;
    let (server, password) = (server.to_string(), password.to_string());
    tokio::task::spawn_blocking(move || -> Result<ClientsKept> {
        picker_rows_for(&settings, &server, &catalogue)?;
        let consented = crate::setup::base_url_in(&settings)?
            .is_some_and(|url| url.trim_end_matches('/') == server.trim_end_matches('/'));
        let kimi = crate::kimi::config::here(&server, &password, &catalogue, install, consented);
        let mut codex = crate::codex::here(&server, &password, install, consented)?;
        if let Some(native)=native {
            let path=crate::accounts::codex_home()?.join("config.toml");
            if path.exists() {
                let path=path.canonicalize()?;
                if let Some(changed)=crate::codex::catalogue::sync(&path,&native,install)? {
                    codex=Some(codex.unwrap_or(false)||changed);
                }
            }
        }
        Ok(ClientsKept { kimi: kimi?, codex })
    })
    .await?
}

/// What is waiting on a person here, named the way a banner names it.
async fn waiting() -> Json<Value> {
    let rows = tokio::task::spawn_blocking(|| {
        let reg = crate::registry::snapshot();
        crate::notify::waiting_in(&crate::live::Roots::system(), &reg, crate::db::now_ms())
            .into_iter()
            .map(|(key, session, head, detail)| {
                let (project, title) = crate::live::session_label(&session).unwrap_or_else(|| ("session".into(), None));
                json!({ "key": key, "session": session, "head": head, "detail": detail, "project": project, "title": title })
            })
            .collect::<Vec<_>>()
    })
    .await
    .unwrap_or_default();
    Json(Value::Array(rows))
}

#[derive(Deserialize)]
struct StillQuery {
    key: String,
    session: String,
}

async fn still_waiting(axum::extract::Query(q): axum::extract::Query<StillQuery>) -> Json<Value> {
    let waiting = tokio::task::spawn_blocking(move || crate::notify::still_waiting(&q.key, &q.session))
        .await
        .unwrap_or(true);
    Json(json!({ "waiting": waiting }))
}

/// The one agent a machine runs: `agent.lock`, held for the life of its keeper (`keep`). Who
/// holds it is in `agent.pid` beside it, since Windows refuses to read a file another process
/// holds locked. An agent from before keepers wrote itself into `agent.lock` instead, and a
/// keeper leaves that file empty, which is how the two are told apart (`holder`).
enum Lock {
    Held(#[allow(dead_code)] std::fs::File),
    Busy(Holder),
}

/// Who holds `agent.lock`.
#[derive(Debug, PartialEq)]
struct Holder {
    pid: i32,
    build: String,
    /// A keeper hands over to a new build by itself; an agent from before keepers has to be
    /// asked to make way.
    keeper: bool,
}

fn open_lock(dir: &Path) -> Result<std::fs::File> {
    let path = dir.join("agent.lock");
    std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&path)
        .with_context(|| format!("opening {}", path.display()))
}

/// Take the lock as this machine's keeper.
fn lock(dir: &Path) -> Result<Lock> {
    let file = open_lock(dir)?;
    match file.try_lock() {
        Ok(()) => {
            file.set_len(0)?;
            let pid = dir.join("agent.pid");
            let tmp = dir.join("agent.pid.tmp");
            std::fs::write(&tmp, format!("{} {} keeper\n", std::process::id(), env!("AGENT_ROUTER_BUILD")))?;
            std::fs::rename(&tmp, &pid)?;
            Ok(Lock::Held(file))
        }
        Err(std::fs::TryLockError::WouldBlock) => Ok(Lock::Busy(holder(dir))),
        Err(std::fs::TryLockError::Error(e)) => Err(e.into()),
    }
}

/// Who holds the lock, if anyone, without taking it.
fn peek(dir: &Path) -> Result<Option<Holder>> {
    let file = open_lock(dir)?;
    match file.try_lock() {
        Ok(()) => Ok(None),
        Err(std::fs::TryLockError::WouldBlock) => Ok(Some(holder(dir))),
        Err(std::fs::TryLockError::Error(e)) => Err(e.into()),
    }
}

/// Who holds `agent.lock`, which is busy. An agent from before keepers wrote itself into the
/// lock file, which a keeper leaves empty; on Windows the lock file cannot be read while it
/// is held, and such an agent wrote no `agent.pid`, so it is named by nothing.
fn holder(dir: &Path) -> Holder {
    let parse = |raw: &str| {
        let mut parts = raw.split_whitespace();
        let pid = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
        let build = parts.next().unwrap_or_default().to_string();
        let keeper = parts.next() == Some("keeper");
        Holder { pid, build, keeper }
    };
    let legacy = std::fs::read_to_string(dir.join("agent.lock")).unwrap_or_default();
    if !legacy.trim().is_empty() {
        return Holder { keeper: false, ..parse(&legacy) };
    }
    match std::fs::read_to_string(dir.join("agent.pid")) {
        Ok(raw) => parse(&raw),
        Err(_) => Holder { pid: 0, build: String::new(), keeper: false },
    }
}

/// What `agent --ensure` does when the lock is busy.
#[derive(Debug, PartialEq)]
enum Busy {
    /// A keeper runs: it hands over to a new build by itself.
    LeaveAlone,
    /// An agent from before keepers, named: asked to stop, and this keeper takes its place.
    Replace(i32),
    /// One from before keepers that cannot be named (Windows): replaced at the next logon,
    /// when the Startup script starts a keeper first.
    AtNextLogon,
}

fn on_busy(holder: &Holder) -> Busy {
    match holder {
        Holder { keeper: true, .. } => Busy::LeaveAlone,
        Holder { pid, .. } if *pid > 0 => Busy::Replace(*pid),
        _ => Busy::AtNextLogon,
    }
}

/// Ask a running agent to stop, so this build can take its place.
fn ask_to_stop(pid: i32) -> Result<()> {
    if pid <= 0 {
        bail!("the running agent did not say which process it is");
    }
    #[cfg(unix)]
    {
        // SAFETY: kill(2) with a valid signal number; a stale pid just returns ESRCH.
        if unsafe { libc::kill(pid, libc::SIGTERM) } != 0 {
            bail!("could not ask the running agent (pid {pid}) to stop");
        }
        Ok(())
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE};
        // SAFETY: a handle this block opens, uses once and closes.
        unsafe {
            let handle = OpenProcess(PROCESS_TERMINATE, 0, pid as u32);
            if handle.is_null() {
                bail!("could not ask the running agent (pid {pid}) to stop");
            }
            let stopped = TerminateProcess(handle, 0) != 0;
            CloseHandle(handle);
            if !stopped {
                bail!("could not stop the running agent (pid {pid})");
            }
        }
        Ok(())
    }
}

async fn run(args: RunArgs) -> Result<()> {
    // Logs go to standard error: a worker's standard output is what it tells its keeper.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("AGENT_ROUTER_LOG").unwrap_or_else(|_| "agent_router=info".into()),
        )
        .with_target(false)
        .with_writer(std::io::stderr)
        .init();
    rustls::crypto::ring::default_provider().install_default().ok();
    let stopping = crate::lifecycle::listen_for_shutdown();

    let dir = crate::config::settle_data_dir(args.data_dir.as_deref())?;
    if args.worker {
        return work(args, dir, stopping).await;
    }
    let _held = match lock(&dir)? {
        Lock::Held(file) => file,
        Lock::Busy(holder) if !args.ensure => bail!("an agent is already running here (pid {})", holder.pid),
        Lock::Busy(holder) => match on_busy(&holder) {
            Busy::LeaveAlone => {
                println!("an agent keeper is already running here (pid {}); it keeps the agent at its build itself", holder.pid);
                return Ok(());
            }
            Busy::AtNextLogon => {
                println!("an agent from an older build is running here; a keeper takes its place at the next logon");
                return Ok(());
            }
            Busy::Replace(pid) => {
                tracing::info!(pid, "an agent from before keepers is running; asking it to make way");
                ask_to_stop(pid)?;
                let deadline = Instant::now() + Duration::from_secs(15);
                loop {
                    if let Lock::Held(file) = lock(&dir)? {
                        break file;
                    }
                    if Instant::now() > deadline {
                        bail!("the running agent (pid {pid}) did not stop");
                    }
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
            }
        },
    };
    keep(&dir, &args, stopping).await
}

/// How long a new worker has to say its tunnel is up before a handover is given up.
const READY_WAIT: Duration = Duration::from_secs(30);
/// How long a worker told to stop has before it is killed.
const STOP_GRACE: Duration = Duration::from_secs(5);
/// How often the keeper looks at its binary for a new build.
const WATCH_EVERY: Duration = Duration::from_secs(5);
/// A build that would not take over is tried again after this, in case what stopped it was
/// the router being away rather than the build.
const RETRY_REFUSED: Duration = Duration::from_secs(10 * 60);
/// The fallback for checking the router's build, besides at start and at every reconnect.
const UPDATE_EVERY: Duration = Duration::from_secs(60 * 60);

/// What a worker tells its keeper, one line on its standard output each.
#[derive(Debug, PartialEq)]
enum Said {
    /// Its tunnel is up for the first time, as this build.
    Ready(String),
    /// Its tunnel is up again.
    Connected,
}

/// One worker the keeper started.
struct Worker {
    child: tokio::process::Child,
    /// Never written to: closing it tells the worker to stop, and it closes by itself when
    /// the keeper goes away, whatever stops it.
    stdin: Option<tokio::process::ChildStdin>,
    said: mpsc::UnboundedReceiver<Said>,
    /// The sha256 of the binary it was started from.
    digest: [u8; 32],
    build: Option<String>,
}

impl Worker {
    fn start(program: &Path, args: &[String], digest: [u8; 32]) -> Result<Worker> {
        let mut command = tokio::process::Command::new(program);
        command
            .args(args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::inherit())
            .kill_on_drop(true);
        #[cfg(windows)]
        {
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        let mut child = command.spawn().with_context(|| format!("starting {}", program.display()))?;
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().ok_or_else(|| anyhow!("the worker has no standard output"))?;
        let (tx, said) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            use tokio::io::AsyncBufReadExt;
            let mut lines = tokio::io::BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let said = match line.trim().split_once(' ') {
                    Some(("ready", build)) => Said::Ready(build.to_string()),
                    None if line.trim() == "connected" => Said::Connected,
                    _ => continue,
                };
                if tx.send(said).is_err() {
                    break;
                }
            }
        });
        Ok(Worker { child, stdin, said, digest, build: None })
    }

    /// Wait for it to say its tunnel is up.
    async fn ready(&mut self, within: Duration) -> Result<String> {
        let up = async {
            loop {
                tokio::select! {
                    said = self.said.recv() => match said {
                        Some(Said::Ready(build)) => return Ok(build),
                        Some(Said::Connected) => continue,
                        None => bail!("it exited ({}) before its tunnel was up", self.child.wait().await?),
                    },
                    status = self.child.wait() => bail!("it exited ({}) before its tunnel was up", status?),
                }
            }
        };
        let build = tokio::time::timeout(within, up)
            .await
            .map_err(|_| anyhow!("its tunnel was not up within {} s", within.as_secs()))??;
        self.build = Some(build.clone());
        Ok(build)
    }

    /// Close its pipe and give it `grace` to go; then kill it.
    async fn stop(mut self, grace: Duration) {
        drop(self.stdin.take());
        if tokio::time::timeout(grace, self.child.wait()).await.is_err() {
            let _ = self.child.kill().await;
        }
    }
}

/// Start a worker from `program`, wait until its tunnel is up, and only then stop `current`.
/// The router routes to a machine's newest tunnel the moment it connects, and the old one's
/// closing neither unregisters it nor marks the machine offline (`machines::Machines::serve`),
/// so the machine never goes offline; what the old one was relaying at that moment fails and
/// is asked again. A worker that is not up in time is stopped, and `current` carries on.
/// The worker running afterwards, and whether the new one took over.
async fn hand_over(current: Worker, program: &Path, args: &[String], digest: [u8; 32], within: Duration) -> (Worker, Result<()>) {
    let mut next = match Worker::start(program, args, digest) {
        Ok(w) => w,
        Err(e) => return (current, Err(e)),
    };
    match next.ready(within).await {
        Ok(_) => {
            current.stop(STOP_GRACE).await;
            (next, Ok(()))
        }
        Err(e) => {
            next.stop(Duration::ZERO).await;
            (current, Err(e))
        }
    }
}

/// The binary the keeper starts its workers from, as last looked at: its size and time, and
/// its digest, read again only when those change.
struct Seen {
    stamp: Option<(u64, std::time::SystemTime)>,
    digest: [u8; 32],
}

impl Seen {
    fn of(path: &Path) -> Seen {
        Seen { stamp: stamp(path), digest: digest_of(path).unwrap_or_default() }
    }

    /// Its digest, when the file is not what it was.
    fn changed(&mut self, path: &Path) -> Option<[u8; 32]> {
        let now = stamp(path);
        if now == self.stamp {
            return None;
        }
        self.stamp = now;
        let digest = digest_of(path).ok()?;
        (digest != self.digest).then(|| {
            self.digest = digest;
            digest
        })
    }
}

fn stamp(path: &Path) -> Option<(u64, std::time::SystemTime)> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.len(), meta.modified().ok()?))
}

fn digest_of(path: &Path) -> Result<[u8; 32]> {
    use sha2::Digest;
    Ok(sha2::Sha256::digest(std::fs::read(path)?).into())
}

/// The keeper: what the login item and the editor start. It holds `agent.lock`, runs no
/// tunnel itself, and keeps one worker running (`work`), started from the installed binary as
/// it is now. When that binary changes — the installer, or `update_from` — it hands over to a
/// worker of the new build before the old one stops (`hand_over`), so neither the router nor
/// the login item sees the agent go. The keeper itself stays on its build until it is next
/// started; it does nothing but this.
async fn keep(dir: &Path, args: &RunArgs, mut stopping: crate::lifecycle::Stopping) -> Result<()> {
    let saved_at = dir.join("agent.json");
    let mut saved = load_saved_at(&saved_at);
    if saved.machine.is_none() {
        saved.machine = Some(new_machine_id());
        store_saved_at(&saved_at, &saved)?;
    }
    let installed = crate::setup::installed_binary()?;
    let own = std::env::current_exe()?;
    // Only the installed agent updates itself: one run from a build directory runs its own
    // binary and is left alone.
    let from_bin = same_file(&own, &installed);
    let program = if from_bin { installed.clone() } else { own };
    let worker_args = worker_args(args);
    tracing::info!(build = env!("AGENT_ROUTER_BUILD"), program = %program.display(), "agent keeper starting");

    let updates = may_update(&saved, from_bin)
        .then(|| {
            let server = args.server.clone().or_else(|| saved.server.clone())?;
            let password = std::env::var("AGENT_ROUTER_PASSWORD").ok().filter(|p| !p.is_empty()).or_else(|| saved.password.clone())?;
            let router = crate::library::sync::Router::new(&server, &password).ok()?;
            Some(Updater { router: Arc::new(router), dir: dir.to_path_buf(), installed: installed.clone(), busy: Default::default() })
        })
        .flatten();
    if let Some(u) = &updates {
        u.check_soon();
    }

    let mut seen = Seen::of(&program);
    let mut current = Worker::start(&program, &worker_args, seen.digest)?;
    let mut refused: Option<([u8; 32], Instant)> = None;
    let mut backoff = Duration::from_secs(1);
    let mut watch = tokio::time::interval(WATCH_EVERY);
    let mut hourly = tokio::time::interval_at(tokio::time::Instant::now() + UPDATE_EVERY, UPDATE_EVERY);
    loop {
        tokio::select! {
            why = stopping.wait() => {
                tracing::info!("{why} received; agent keeper stopping");
                current.stop(STOP_GRACE).await;
                return Ok(());
            }
            status = current.child.wait() => {
                let how = status.map(|s| s.to_string()).unwrap_or_else(|e| e.to_string());
                tracing::warn!("the agent's worker exited ({how}); starting it again in {} s", backoff.as_secs());
                tokio::select! {
                    _ = tokio::time::sleep(backoff) => {}
                    why = stopping.wait() => {
                        tracing::info!("{why} received; agent keeper stopping");
                        return Ok(());
                    }
                }
                backoff = (backoff * 2).min(Duration::from_secs(60));
                seen = Seen::of(&program);
                match Worker::start(&program, &worker_args, seen.digest) {
                    Ok(w) => current = w,
                    Err(e) => tracing::warn!("{e:#}"),
                }
            }
            Some(said) = current.said.recv() => match said {
                Said::Ready(build) => {
                    backoff = Duration::from_secs(1);
                    current.build = Some(build);
                }
                Said::Connected => {
                    if let Some(u) = &updates {
                        u.check_soon();
                    }
                }
            },
            _ = watch.tick() => {
                let Some(digest) = seen.changed(&program) else { continue };
                if digest == current.digest {
                    continue;
                }
                if refused.is_some_and(|(d, at)| d == digest && at.elapsed() < RETRY_REFUSED) {
                    continue;
                }
                let from = current.build.clone().unwrap_or_else(|| "an unknown build".into());
                let (running, took) = hand_over(current, &program, &worker_args, digest, READY_WAIT).await;
                current = running;
                match took {
                    Ok(()) => {
                        tracing::info!("handed over from {from} to {}", current.build.as_deref().unwrap_or("an unknown build"));
                        refused = None;
                    }
                    Err(e) => {
                        tracing::warn!("a new agent build did not take over ({e:#}); {from} carries on");
                        refused = Some((digest, Instant::now()));
                        // Looked at again once the wait is over, as if it had just changed.
                        seen.stamp = None;
                        seen.digest = current.digest;
                    }
                }
            }
            _ = hourly.tick() => {
                if let Some(u) = &updates {
                    u.check_soon();
                }
            }
        }
    }
}

/// What the keeper starts its worker with: `agent --worker`, and what it was itself told.
fn worker_args(args: &RunArgs) -> Vec<String> {
    let mut out = vec!["agent".to_string(), "--worker".to_string()];
    if let Some(server) = &args.server {
        out.extend(["--server".to_string(), server.clone()]);
    }
    if let Some(label) = &args.label {
        out.extend(["--label".to_string(), label.clone()]);
    }
    if let Some(dir) = &args.data_dir {
        out.extend(["--data-dir".to_string(), dir.display().to_string()]);
    }
    out
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// Whether this keeper fetches its router's build: not on a machine installed with
/// `--no-auto-update`, and not when it runs from anywhere but the installed binary.
fn may_update(saved: &Saved, from_bin: bool) -> bool {
    from_bin && saved.update != Some(false)
}

/// The platform the router's installer names, whose agent it hands out
/// (`downloads::PLATFORMS`): Windows on Arm runs the x64 build.
fn platform() -> &'static str {
    if cfg!(windows) {
        "win32-x64"
    } else if cfg!(target_os = "macos") {
        if cfg!(target_arch = "aarch64") { "darwin-arm64" } else { "darwin-x64" }
    } else if cfg!(target_arch = "aarch64") {
        "linux-arm64"
    } else {
        "linux-x64"
    }
}

/// The keeper's self-update, one check at a time, off its loop.
struct Updater {
    router: Arc<crate::library::sync::Router>,
    dir: PathBuf,
    installed: PathBuf,
    busy: Arc<std::sync::atomic::AtomicBool>,
}

impl Updater {
    fn check_soon(&self) {
        use std::sync::atomic::Ordering;
        if self.busy.swap(true, Ordering::SeqCst) {
            return;
        }
        let (router, dir, installed, busy) = (self.router.clone(), self.dir.clone(), self.installed.clone(), self.busy.clone());
        tokio::spawn(async move {
            match update_from(&router, &dir, &installed, try_run).await {
                Ok(Some(build)) => tracing::info!("fetched the router's agent, {build}, into {}; handing over to it", installed.display()),
                Ok(None) => {}
                Err(e) => tracing::warn!("could not check the router's agent build: {e:#}"),
            }
            busy.store(false, Ordering::SeqCst);
        });
    }
}

/// Once per router build: fetch the router's agent for this platform, and when it differs
/// from the installed one, put it in place (`install_if_different`). Only what every router
/// already serves is asked for: its build (`/_router/health`) and the agent its installer
/// hands out (`/agent/<platform>/agent-router`). The new build, when there is one.
async fn update_from(
    router: &crate::library::sync::Router,
    dir: &Path,
    installed: &Path,
    try_run: fn(&Path) -> Result<String>,
) -> Result<Option<String>> {
    let (status, _, body) = router.send(axum::http::Method::GET, "/_router/health", Vec::new(), Bytes::new()).await?;
    if !status.is_success() {
        bail!("the router answered {status} for its health");
    }
    let health: Value = serde_json::from_slice(&body).context("the router's health is not JSON")?;
    let build = health.get("build").and_then(Value::as_str).ok_or_else(|| anyhow!("the router did not say its build"))?.to_string();
    let saved_at = dir.join("agent.json");
    if load_saved_at(&saved_at).checked_build.as_deref() == Some(build.as_str()) {
        return Ok(None);
    }
    let name = if cfg!(windows) { "agent-router.exe" } else { "agent-router" };
    let path = format!("/agent/{}/{name}", platform());
    let (status, _, binary) = router.send(axum::http::Method::GET, &path, Vec::new(), Bytes::new()).await?;
    if !status.is_success() {
        bail!("the router answered {status} for {path}");
    }
    let installed = installed.to_path_buf();
    let outcome = tokio::task::spawn_blocking(move || install_if_different(&installed, &binary, try_run)).await??;
    let mut saved = load_saved_at(&saved_at);
    saved.checked_build = Some(build);
    store_saved_at(&saved_at, &saved)?;
    Ok(outcome)
}

/// Put `bytes` in place of the installed binary, unless they are the same: written beside it,
/// run once (`try_run`), then renamed over it (`setup::replace`), never written over a binary
/// in place. A download that will not run here is never installed. The new build, if any.
fn install_if_different(installed: &Path, bytes: &[u8], try_run: fn(&Path) -> Result<String>) -> Result<Option<String>> {
    use sha2::Digest;
    let new: [u8; 32] = sha2::Sha256::digest(bytes).into();
    if digest_of(installed).ok() == Some(new) {
        return Ok(None);
    }
    let name = installed.file_name().and_then(|n| n.to_str()).unwrap_or("agent-router");
    let fresh = installed.with_file_name(format!("{name}.new-{}", std::process::id()));
    std::fs::write(&fresh, bytes).with_context(|| format!("writing {}", fresh.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&fresh, std::fs::Permissions::from_mode(0o755))?;
    }
    let build = match try_run(&fresh) {
        Ok(build) => build,
        Err(e) => {
            let _ = std::fs::remove_file(&fresh);
            return Err(e.context("the router's agent does not run here"));
        }
    };
    crate::setup::replace(&fresh, installed)?;
    Ok(Some(build))
}

/// Run a binary with `--build`: it must answer within ten seconds, exit 0, and say a build.
/// This catches a binary for another architecture, or one macOS will not start.
fn try_run(path: &Path) -> Result<String> {
    let mut run = std::process::Command::new(path);
    run.arg("--build").stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null());
    // Just written: busy for as long as a fork elsewhere still holds it open.
    let mut child = crate::setup::spawn_when_free(&mut run).with_context(|| format!("starting {}", path.display()))?;
    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            bail!("{} did not answer within 10 s", path.display());
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let mut out = String::new();
    if let Some(mut stdout) = child.stdout.take() {
        use std::io::Read;
        stdout.read_to_string(&mut out)?;
    }
    let build = out.trim().to_string();
    if !status.success() || build.is_empty() {
        bail!("{} answered {status} with no build", path.display());
    }
    Ok(build)
}

/// The keeper's worker: today's agent, the tunnel and everything it serves, with no lock of
/// its own. It stops when asked, and when its standard input closes: that is how its keeper
/// tells it to make way, and what happens when the keeper goes away, whatever stopped it. It
/// says `ready <build>` on its standard output once its tunnel is first up, and `connected`
/// at each reconnect after (`Said`).
async fn work(args: RunArgs, dir: PathBuf, stopping: crate::lifecycle::Stopping) -> Result<()> {
    // Read on a thread of its own, which nothing waits for at exit.
    let (closed, eof) = tokio::sync::oneshot::channel::<()>();
    std::thread::spawn(move || {
        use std::io::Read;
        let mut buf = [0u8; 64];
        let mut stdin = std::io::stdin();
        while matches!(stdin.read(&mut buf), Ok(n) if n > 0) {}
        let _ = closed.send(());
    });
    let mut stopping = stopping.or("EOF on the keeper's pipe", async move {
        let _ = eof.await;
    });
    let mut saved = load_saved();
    if saved.machine.is_none() {
        saved.machine = Some(new_machine_id());
        store_saved(&saved)?;
    }
    let server = args
        .server
        .or_else(|| saved.server.clone())
        .ok_or_else(|| anyhow!("no server: pass --server <url>, or run `agent-router agent install --server <url>`"))?;
    let password = std::env::var("AGENT_ROUTER_PASSWORD")
        .ok()
        .filter(|p| !p.is_empty())
        .or_else(|| saved.password.clone())
        .ok_or_else(|| anyhow!("no password: set AGENT_ROUTER_PASSWORD, or run `agent-router agent install --server <url>`"))?;
    let target = Target::parse(&server)?;
    let hello = Hello {
        machine: saved.machine.clone().unwrap_or_default(),
        label: args.label.or_else(|| saved.label.clone()).unwrap_or_else(host_name),
        platform: std::env::consts::OS.to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        build: env!("AGENT_ROUTER_BUILD").to_string(),
    };
    tracing::info!(server = %target.ws_url(), machine = %hello.machine, label = %hello.label, "agent-router agent starting");

    // The housekeeping a local router does for the wrappers on its machine: here, the
    // agent is what is on the machine.
    tokio::spawn(async {
        loop {
            tokio::time::sleep(Duration::from_secs(60)).await;
            let cleared = tokio::task::spawn_blocking(|| {
                crate::notify::janitor(&crate::peer::wrapper_dir(), &crate::registry::snapshot())
            })
            .await
            .unwrap_or(0);
            if cleared > 0 {
                tracing::info!(cleared, "removed what dead wrappers left behind");
            }
        }
    });

    // The Library: in step at every connection and every fifteen minutes between, unless this
    // machine opted out (docs/library.md, "The sync").
    let (connected, on_connect) = tokio::sync::watch::channel(0u64);
    let library = match saved.library {
        Some(false) => None,
        _ => Some(Library {
            places: crate::library::sync::Places::system(dir.clone()),
            router: Arc::new(crate::library::sync::Router::new(&server, &password)?),
            machine: hello.machine.clone(),
            label: hello.label.clone(),
        }),
    };
    if let Some(library) = library.clone() {
        tokio::spawn(crate::library::sync::keep_in_step(on_connect, crate::library::sync::EVERY, move || {
            let library = library.clone();
            async move {
                if let Err(e) = library.sync(true).await {
                    tracing::warn!("the Library could not be synced: {e:#}");
                }
            }
        }));
    }

    // Where this machine's sessions run, and the repositories its directories are checkouts
    // of, at every connection and every minute between (`places`).
    {
        let router = Arc::new(crate::library::sync::Router::new(&server, &password)?);
        let (machine, label) = (hello.machine.clone(), hello.label.clone());
        tokio::spawn(crate::library::sync::keep_in_step(connected.subscribe(), crate::places::EVERY, move || {
            let (router, machine, label) = (router.clone(), machine.clone(), label.clone());
            async move {
                if let Err(e) = crate::places::send(&router, &machine, &label).await {
                    tracing::warn!("this machine's places could not be reported: {e:#}");
                }
            }
        }));
    }

    // Kimi Code's server for the sessions this machine hosts, stopped once unused.
    tokio::spawn(crate::kimi::live::keep_host());
    tokio::spawn(crate::codex::runtime::keep_host());

    // The clients, at every connection and every fifteen minutes between (`keep_clients`):
    // Claude Code's /model picker gets a row for each of the router's non-Claude models, so a
    // model the router gains or loses is offered or taken away without a reinstall, and Kimi
    // Code's config.toml gets the router's entries back after Kimi Code updates itself.
    {
        let router = Arc::new(crate::library::sync::Router::new(&server, &password)?);
        let (server, password) = (server.clone(), password.clone());
        tokio::spawn(crate::library::sync::keep_in_step(connected.subscribe(), crate::library::sync::EVERY, move || {
            let (router, server, password) = (router.clone(), server.clone(), password.clone());
            async move {
                match keep_clients(&router, &server, &password, false).await {
                    Ok(ClientsKept { kimi, codex }) => {
                        if kimi == Some(true) { tracing::info!("Kimi Code's config.toml: the router's entries put back"); }
                        if codex == Some(true) { tracing::info!("Codex's config.toml: the router's entries put back"); }
                    },
                    Err(e) => tracing::warn!("the clients could not be kept in step: {e:#}"),
                }
            }
        }));
    }

    // Said to the keeper: the first time the tunnel is up, so it can stop the worker this one
    // replaces; every time after, so it can check its router's build.
    {
        let mut up = connected.subscribe();
        tokio::spawn(async move {
            let mut first = true;
            while up.changed().await.is_ok() {
                if std::mem::take(&mut first) {
                    println!("ready {}", env!("AGENT_ROUTER_BUILD"));
                } else {
                    println!("connected");
                }
            }
        });
    }

    let router = routes(library);
    let mut backoff = Duration::from_secs(1);
    loop {
        let started = Instant::now();
        let outcome = tokio::select! {
            outcome = connect_and_serve(&target, &password, &hello, router.clone(), &connected) => outcome,
            why = stopping.wait() => {
                tracing::info!("{why} received; agent stopping");
                return Ok(());
            }
        };
        match outcome {
            Ok(()) => tracing::info!("the router closed the connection"),
            Err(e) => tracing::warn!("{e:#}"),
        }
        if started.elapsed() > Duration::from_secs(60) {
            backoff = Duration::from_secs(1);
        }
        let wait = backoff + jitter();
        tokio::select! {
            _ = tokio::time::sleep(wait) => {}
            why = stopping.wait() => {
                tracing::info!("{why} received; agent stopping");
                return Ok(());
            }
        }
        backoff = (backoff * 2).min(MAX_BACKOFF);
    }
}

#[derive(Debug, Default)]
struct InstallArgs {
    server: Option<String>,
    project: Option<PathBuf>,
    label: Option<String>,
    data_dir: Option<PathBuf>,
    no_service: bool,
    no_config: bool,
    /// `Some(false)` from `--no-library`, `Some(true)` from `--library`: otherwise left as
    /// it was saved.
    library: Option<bool>,
    /// `Some(false)` from `--no-auto-update`, `Some(true)` from `--auto-update`: otherwise
    /// left as it was saved.
    auto_update: Option<bool>,
}

fn parse_install(args: &[String]) -> Result<InstallArgs, String> {
    let mut out = InstallArgs::default();
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        i += 1;
        let (name, inline) = match arg.split_once('=') {
            Some((n, v)) if n.starts_with("--") => (n, Some(v.to_string())),
            _ => (arg.as_str(), None),
        };
        let mut value = || -> Result<String, String> {
            if let Some(v) = &inline {
                return Ok(v.clone());
            }
            let v = args.get(i).cloned().ok_or_else(|| format!("{name} needs a value"))?;
            i += 1;
            Ok(v)
        };
        match name {
            "--server" => out.server = Some(value()?),
            "--project" => out.project = Some(PathBuf::from(value()?)),
            "--label" => out.label = Some(value()?),
            "--data-dir" => out.data_dir = Some(PathBuf::from(value()?)),
            "--no-service" if inline.is_none() => out.no_service = true,
            "--no-config" if inline.is_none() => out.no_config = true,
            "--no-library" if inline.is_none() => out.library = Some(false),
            "--library" if inline.is_none() => out.library = Some(true),
            "--no-auto-update" if inline.is_none() => out.auto_update = Some(false),
            "--auto-update" if inline.is_none() => out.auto_update = Some(true),
            other => return Err(format!("unknown option {other:?}")),
        }
    }
    Ok(out)
}

/// The password, from AGENT_ROUTER_PASSWORD, a line on standard input (an editor or a
/// script handing it over), or a prompt that does not echo it.
fn read_password() -> Result<String> {
    if let Some(p) = std::env::var("AGENT_ROUTER_PASSWORD").ok().filter(|p| !p.is_empty()) {
        return Ok(p);
    }
    use std::io::IsTerminal;
    let password = if std::io::stdin().is_terminal() {
        eprint!("Router password: ");
        let p = read_hidden()?;
        eprintln!();
        p
    } else {
        let mut line = String::new();
        std::io::stdin().read_line(&mut line)?;
        line.trim_end_matches(['\r', '\n']).to_string()
    };
    if password.is_empty() {
        bail!("no password given");
    }
    Ok(password)
}

#[cfg(unix)]
fn read_hidden() -> Result<String> {
    // SAFETY: termios is plain data; fd 0 is a terminal (checked by the caller), and the
    // saved settings are put back whatever the read does.
    let mut saved: libc::termios = unsafe { std::mem::zeroed() };
    if unsafe { libc::tcgetattr(0, &mut saved) } != 0 {
        bail!("cannot read the terminal's settings");
    }
    let mut quiet = saved;
    quiet.c_lflag &= !libc::ECHO;
    unsafe { libc::tcsetattr(0, libc::TCSANOW, &quiet) };
    let mut line = String::new();
    let read = std::io::stdin().read_line(&mut line);
    unsafe { libc::tcsetattr(0, libc::TCSANOW, &saved) };
    read?;
    Ok(line.trim_end_matches(['\r', '\n']).to_string())
}

#[cfg(not(unix))]
fn read_hidden() -> Result<String> {
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    Ok(line.trim_end_matches(['\r', '\n']).to_string())
}

/// Ask the router who it is, with the password: a wrong password, a router that is not
/// hosted, or a name it does not answer to, all say so here rather than later in a log.
async fn verify(server: &str, password: &str) -> Result<()> {
    use http_body_util::BodyExt;
    let tls = hyper_rustls::HttpsConnectorBuilder::new()
        .with_webpki_roots()
        .https_or_http()
        .enable_http1()
        .build();
    let client: hyper_util::client::legacy::Client<_, http_body_util::Empty<Bytes>> =
        hyper_util::client::legacy::Client::builder(hyper_util::rt::TokioExecutor::new()).build(tls);
    let mut auth = axum::http::HeaderValue::from_str(&format!("Bearer {password}"))
        .context("the password cannot be sent in a header")?;
    auth.set_sensitive(true);
    let request = axum::http::Request::get(format!("{server}/api/auth"))
        .header(axum::http::header::AUTHORIZATION, auth)
        .body(http_body_util::Empty::new())?;
    let response = tokio::time::timeout(CONNECT_WAIT, client.request(request))
        .await
        .map_err(|_| anyhow!("{server} did not answer"))?
        .with_context(|| format!("reaching {server}"))?;
    let status = response.status();
    let body = response.into_body().collect().await.map(|b| b.to_bytes()).unwrap_or_default();
    match status.as_u16() {
        200 => {}
        401 => bail!("the router refused the password"),
        403 => bail!("the router refused this request: {}", String::from_utf8_lossy(&body).trim()),
        _ => bail!("{server} answered {status}"),
    }
    let auth: Value = serde_json::from_slice(&body).context("that does not look like Agent Router")?;
    match auth.get("mode").and_then(Value::as_str) {
        Some("server") => Ok(()),
        Some(mode) => bail!("{server} is not a hosted router (it runs in {mode} mode); the agent connects to one started with `serve --server`"),
        None => bail!("{server} is an Agent Router too old to take agents"),
    }
}

async fn install(args: &[String]) -> Result<()> {
    rustls::crypto::ring::default_provider().install_default().ok();
    let args = match parse_install(args) {
        Ok(a) => a,
        Err(why) => {
            eprintln!("{why}\n\n{USAGE}");
            std::process::exit(2);
        }
    };
    let server = args
        .server
        .as_deref()
        .map(|s| s.trim().trim_end_matches('/').to_string())
        .ok_or_else(|| anyhow!("--server <url> is required"))?;
    Target::parse(&server)?;
    crate::config::settle_data_dir(args.data_dir.as_deref())?;
    let password = read_password()?;
    verify(&server, &password).await?;
    println!("server    {server} (password accepted)");

    let mut saved = load_saved();
    saved.server = Some(server.clone());
    saved.password = Some(password.clone());
    saved.machine.get_or_insert_with(new_machine_id);
    if args.label.is_some() {
        saved.label = args.label.clone();
    }
    if let Some(on) = args.library {
        saved.library = (!on).then_some(false);
    }
    if let Some(on) = args.auto_update {
        saved.update = (!on).then_some(false);
    }
    store_saved(&saved)?;
    println!("saved     {} (owner-only)", saved_path()?.display());
    println!("library   {}", library_words(saved.library));
    println!("updates   {}", update_words(saved.update));

    // A machine that ran the router itself still has its login item, which starts
    // `bin/agent-router` — the path `install_binary` is about to give the agent — so at the
    // next login it would start this build as an empty local router. The agent replaces it.
    match crate::setup::service::retire() {
        Ok(done) if done.is_empty() => {}
        Ok(done) => {
            println!("router    the local router's login item is retired: this machine runs the agent instead");
            for line in done {
                println!("          {line}");
            }
        }
        Err(e) => println!("router    the local router's login item could not be retired ({e:#})"),
    }

    if args.no_config {
        println!("config    left alone (--no-config)");
    } else {
        let scope = if args.project.is_some() { "project" } else { "global" };
        let path = crate::setup::settings_path(args.project.as_deref())?;
        crate::setup::apply(&path, &server, scope)?;
        crate::setup::apply_key(&path, crate::setup::TOKEN_KEY, &password, scope)?;
        println!("config    ANTHROPIC_BASE_URL={server} and ANTHROPIC_AUTH_TOKEN");
        println!("          {} ({scope}); uninstall puts back what was there", path.display());
        let offered = match crate::library::sync::Router::new(&server, &password) {
            Ok(router) => keep_clients(&router, &server, &password, true).await,
            Err(e) => Err(e),
        };
        match offered {
            Ok(ClientsKept { kimi, codex }) => {
                if kimi.is_some() {
                    println!("kimi      Kimi Code's config.toml: the router's provider, its Kimi models and hooks");
                    println!("          uninstall takes them out and puts back the default model");
                }
                if codex.is_some() { println!("codex     Codex's config.toml: Responses through Agent Router; uninstall restores its provider"); }
            },
            Err(e) => {
                println!("clients   some client settings could not be updated ({e:#}); the agent tries again when it connects")
            }
        }
    }

    if args.no_service {
        println!("service   left alone (--no-service); start it with `agent-router agent`");
    } else {
        // The settings are already written: a login item that cannot be registered here
        // is said, not a failure of the whole install.
        // A keeper already running hands over to the binary just put in place by itself, with
        // the machine never offline; anything older is restarted as a keeper.
        let keeper = crate::config::data_dir().ok().and_then(|dir| peek(&dir).ok().flatten()).filter(|h| h.keeper);
        let registered = crate::setup::install_binary().and_then(|exe| crate::setup::agent_service::install(&exe, keeper.is_some()));
        match registered {
            Ok(path) => {
                println!("service   registered: {}", path.display());
                match &keeper {
                    Some(k) => println!("          the running agent (pid {}) hands over to this build without going offline", k.pid),
                    None => println!("          the agent starts now, and at every login"),
                }
                // A user's systemd stops with their last session unless it lingers, and a
                // machine reached over SSH is often left with none.
                #[cfg(target_os = "linux")]
                println!("          to keep it running between logins: loginctl enable-linger");
            }
            Err(e) => println!("service   not registered ({e:#}); start it with `agent-router agent`"),
        }
    }
    Ok(())
}

fn uninstall(args: &[String]) -> Result<()> {
    let data_dir = match parse(args) {
        Ok(a) => a.data_dir,
        Err(why) => bail!(why),
    };
    crate::config::settle_data_dir(data_dir.as_deref())?;
    let state = crate::setup::load_state();
    let files: std::collections::BTreeSet<String> = state
        .managed
        .iter()
        .filter(|m| m.key == crate::setup::TOKEN_KEY)
        .map(|m| m.settings_path.clone())
        .collect();
    if files.is_empty() {
        println!("config    nothing was pointed at a hosted router by the agent");
    }
    for file in files {
        let path = PathBuf::from(&file);
        match crate::setup::revert(&path)? {
            crate::setup::Reverted::Restored { to } => {
                println!("config    restored ANTHROPIC_BASE_URL={to}; ANTHROPIC_AUTH_TOKEN is back to what it was")
            }
            crate::setup::Reverted::Removed => {
                println!("config    removed ANTHROPIC_BASE_URL; ANTHROPIC_AUTH_TOKEN is back to what it was")
            }
            crate::setup::Reverted::NotOurs { found } => {
                println!("config    left alone: ANTHROPIC_BASE_URL is now {found}, which is not what we wrote")
            }
            crate::setup::Reverted::NotManaged => {}
        }
        println!("          {file}");
    }
    match crate::kimi::config::take_out() {
        Ok(Some(true)) => println!("kimi      the router's entries taken out of Kimi Code's config.toml"),
        Ok(_) => {}
        Err(e) => println!("kimi      Kimi Code's config.toml left alone ({e:#})"),
    }
    match crate::codex::take_out() {
        Ok(Some(true)) => println!("codex     the previous Codex provider restored"),
        Ok(_) => {},
        Err(e) => println!("codex     Codex's config.toml left alone ({e:#})"),
    }
    crate::setup::agent_service::uninstall()?;
    // launchd stops what it started; anything else still running (the editor's, or one
    // started by hand) is asked to stop through its lock.
    let dir = crate::config::data_dir()?;
    // A keeper stops its worker on its way out; on Windows, where it is ended outright, the
    // worker sees its pipe close and goes too.
    if let Some(holder) = peek(&dir)? {
        let _ = ask_to_stop(holder.pid);
    }
    println!("service   unregistered; the agent has stopped");
    let mut saved = load_saved();
    saved.password = None;
    saved.server = None;
    store_saved(&saved)?;
    println!("saved     the password is forgotten; this machine keeps its id");
    Ok(())
}

fn status(args: &[String]) -> Result<()> {
    let data_dir = match parse(args) {
        Ok(a) => a.data_dir,
        Err(why) => bail!(why),
    };
    let dir = crate::config::settle_data_dir(data_dir.as_deref())?;
    let saved = load_saved();
    println!("server    {}", saved.server.as_deref().unwrap_or("not set"));
    println!("password  {}", if saved.password.is_some() { "saved" } else { "not saved" });
    println!("machine   {}", saved.machine.as_deref().unwrap_or("no id yet"));
    println!("label     {}", saved.label.clone().unwrap_or_else(host_name));
    println!("library   {}", library_words(saved.library));
    println!("updates   {}", update_words(saved.update));
    match peek(&dir)? {
        Some(Holder { pid, build, keeper: true }) => println!("running   yes: a keeper (pid {pid}, {build}) and its worker"),
        Some(Holder { pid, build, .. }) => println!("running   yes (pid {pid}, {build}), from before keepers"),
        None => println!("running   no"),
    }
    println!(
        "service   {}",
        if crate::setup::agent_service::is_registered() { "registered" } else { "not registered" }
    );
    for m in crate::setup::load_state().managed.iter().filter(|m| m.key == crate::setup::TOKEN_KEY) {
        println!("config    {} ({})", m.settings_path, m.scope);
    }
    Ok(())
}

fn update_words(update: Option<bool>) -> &'static str {
    match update {
        Some(false) => "not fetched (--no-auto-update): the agent keeps its build until the installer runs again",
        _ => "the router's build, fetched once per router build and handed over to without going offline",
    }
}

fn library_words(library: Option<bool>) -> &'static str {
    match library {
        Some(false) => "not taken (--no-library): nothing is placed in ~/.claude",
        _ => "kept in step in ~/.claude at every connection, and every 15 minutes",
    }
}

/// Up to a second, so machines that lost the router together do not return together.
fn jitter() -> Duration {
    use ring::rand::SecureRandom;
    let mut b = [0u8; 2];
    let _ = ring::rand::SystemRandom::new().fill(&mut b);
    Duration::from_millis(u64::from(u16::from_be_bytes(b)) % 1000)
}

trait Io: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send {}
impl<T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send> Io for T {}

/// Certificates as the proxy checks them: the bundled web PKI roots, ring, and HTTP/1.1
/// only — a WebSocket upgrade is an HTTP/1.1 request, and an ingress offered h2 would
/// take it.
fn tls_config() -> Arc<rustls::ClientConfig> {
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let mut config = rustls::ClientConfig::builder().with_root_certificates(roots).with_no_client_auth();
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Arc::new(config)
}

async fn connect_and_serve(
    target: &Target,
    password: &str,
    hello: &Hello,
    router: Router,
    connected: &tokio::sync::watch::Sender<u64>,
) -> Result<()> {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    use tokio_tungstenite::tungstenite::http::{header::AUTHORIZATION, HeaderValue};

    let mut request = target.ws_url().into_client_request()?;
    let mut auth = HeaderValue::from_str(&format!("Bearer {password}")).context("the password cannot be sent in a header")?;
    auth.set_sensitive(true);
    request.headers_mut().insert(AUTHORIZATION, auth);

    let tcp = tokio::time::timeout(CONNECT_WAIT, tokio::net::TcpStream::connect((target.host.as_str(), target.port)))
        .await
        .map_err(|_| anyhow!("{}:{} did not answer", target.host, target.port))?
        .with_context(|| format!("connecting to {}:{}", target.host, target.port))?;
    tcp.set_nodelay(true)?;
    let io: Box<dyn Io> = if target.tls {
        let name = rustls::pki_types::ServerName::try_from(target.host.clone()).context("not a name a certificate can be for")?;
        let tls = tokio::time::timeout(CONNECT_WAIT, tokio_rustls::TlsConnector::from(tls_config()).connect(name, tcp))
            .await
            .map_err(|_| anyhow!("TLS with {} did not finish", target.host))??;
        Box::new(tls)
    } else {
        Box::new(tcp)
    };
    let config = WebSocketConfig::default()
        .max_message_size(Some(tunnel::MAX_MESSAGE))
        .max_frame_size(Some(tunnel::MAX_MESSAGE));
    let (socket, _) = match tokio::time::timeout(
        CONNECT_WAIT,
        tokio_tungstenite::client_async_with_config(request, io, Some(config)),
    )
    .await
    {
        Err(_) => bail!("the router did not accept the connection in time"),
        Ok(Ok(pair)) => pair,
        Ok(Err(tokio_tungstenite::tungstenite::Error::Http(response))) => {
            let status = response.status();
            match status.as_u16() {
                401 => bail!("the router refused the password"),
                403 => bail!("the router refused this connection ({status}): is this its name, and is it a hosted router?"),
                404 => bail!("the router has no agent endpoint: is it running with --server?"),
                _ => bail!("the router answered {status}"),
            }
        }
        Ok(Err(e)) => return Err(e.into()),
    };
    tracing::info!(server = %target.ws_url(), "connected to the router");
    connected.send_modify(|n| *n += 1);
    serve(socket, hello, router).await
}

/// One connection to the router, for as long as it lasts: say who this is, then run each
/// request it sends through `router`. Also how a test connects a machine of its own
/// (`api_refusals`).
pub(crate) async fn serve<S>(socket: tokio_tungstenite::WebSocketStream<S>, hello: &Hello, router: Router) -> Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let (mut sink, mut stream) = socket.split();
    let (out, mut outgoing) = mpsc::unbounded_channel::<Vec<u8>>();
    out.send(tunnel::encode(tunnel::HELLO, 0, &serde_json::to_vec(hello)?))
        .map_err(|_| anyhow!("the connection closed at once"))?;
    let writer = tokio::spawn(async move {
        let mut ping = tokio::time::interval(PING_EVERY);
        loop {
            tokio::select! {
                frame = outgoing.recv() => match frame {
                    Some(frame) => if sink.send(Message::Binary(frame.into())).await.is_err() { break },
                    None => break,
                },
                _ = ping.tick() => if sink.send(Message::Ping(Bytes::new())).await.is_err() { break },
            }
        }
        let _ = sink.close().await;
    });

    let running: Arc<Mutex<HashMap<u32, tokio::task::AbortHandle>>> = Arc::default();
    let outcome = loop {
        match tokio::time::timeout(IDLE_LIMIT, stream.next()).await {
            Ok(Some(Ok(Message::Binary(data)))) => {
                let Some((kind, id, payload)) = tunnel::decode(&data) else {
                    continue;
                };
                match kind {
                    tunnel::REQUEST => match serde_json::from_slice::<tunnel::Request>(payload) {
                        Ok(request) => {
                            // Registered under the lock it removes itself under, so a request
                            // that finishes at once cannot leave its entry behind.
                            let mut held = running.lock().unwrap_or_else(|e| e.into_inner());
                            let task = tokio::spawn(answer(router.clone(), id, request, out.clone(), running.clone()));
                            held.insert(id, task.abort_handle());
                        }
                        Err(e) => {
                            let _ = out.send(tunnel::encode(tunnel::FAIL, id, e.to_string().as_bytes()));
                        }
                    },
                    tunnel::CANCEL => {
                        if let Some(task) = running.lock().unwrap_or_else(|e| e.into_inner()).remove(&id) {
                            task.abort();
                        }
                    }
                    _ => {}
                }
            }
            Ok(Some(Ok(Message::Close(_)))) | Ok(None) => break Ok(()),
            Ok(Some(Err(e))) => break Err(anyhow::Error::from(e).context("the connection to the router failed")),
            Err(_) => break Err(anyhow!("the router went quiet")),
            Ok(Some(Ok(_))) => {}
        }
    };
    for (_, task) in running.lock().unwrap_or_else(|e| e.into_inner()).drain() {
        task.abort();
    }
    writer.abort();
    outcome
}

/// One request, run through `routes` and streamed back.
async fn answer(
    router: Router,
    id: u32,
    request: tunnel::Request,
    out: mpsc::UnboundedSender<Vec<u8>>,
    running: Arc<Mutex<HashMap<u32, tokio::task::AbortHandle>>>,
) {
    let send = |kind: u8, payload: &[u8]| {
        let _ = out.send(tunnel::encode(kind, id, payload));
    };
    let mut builder = axum::http::Request::builder().method(request.method.as_str()).uri(request.path.as_str());
    for (name, value) in &request.headers {
        if tunnel::carried(&name.to_ascii_lowercase()) {
            builder = builder.header(name.as_str(), value.as_str());
        }
    }
    let body = crate::auth::base64_decode(&request.body).unwrap_or_default();
    match builder.body(Body::from(body)) {
        Err(e) => send(tunnel::FAIL, e.to_string().as_bytes()),
        Ok(req) => {
            let response = match router.oneshot(req).await {
                Ok(r) => r,
                Err(never) => match never {},
            };
            let head = Head {
                status: response.status().as_u16(),
                headers: response
                    .headers()
                    .iter()
                    .filter(|(n, _)| tunnel::carried(n.as_str()))
                    .filter_map(|(n, v)| Some((n.as_str().to_string(), v.to_str().ok()?.to_string())))
                    .collect(),
            };
            match serde_json::to_vec(&head) {
                Ok(head) => send(tunnel::HEAD, &head),
                Err(e) => {
                    send(tunnel::FAIL, e.to_string().as_bytes());
                    running.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
                    return;
                }
            }
            let mut body = response.into_body().into_data_stream();
            while let Some(chunk) = body.next().await {
                let Ok(bytes) = chunk else { break };
                for piece in bytes.chunks(tunnel::CHUNK) {
                    send(tunnel::BODY, piece);
                }
            }
            send(tunnel::END, &[]);
        }
    }
    running.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ar-agent-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A stand-in worker: says it is up, writes when it starts and when it goes, and goes
    /// when its pipe closes, as `work` does.
    #[cfg(unix)]
    fn stand_in(n: u32, log: &Path) -> Vec<String> {
        let log = log.display();
        vec!["-c".into(), format!("echo up-{n} >> '{log}'; echo ready tree:{n}; cat >/dev/null; echo down-{n} >> '{log}'")]
    }

    #[cfg(unix)]
    fn lines(log: &Path) -> Vec<String> {
        std::fs::read_to_string(log).unwrap_or_default().lines().map(str::to_owned).collect()
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn the_keeper_hands_over_before_the_old_worker_stops() {
        let dir = scratch("handover");
        let log = dir.join("log");
        let sh = Path::new("/bin/sh");
        let mut one = Worker::start(sh, &stand_in(1, &log), [1; 32]).unwrap();
        assert_eq!(one.ready(Duration::from_secs(10)).await.unwrap(), "tree:1");
        let (two, took) = hand_over(one, sh, &stand_in(2, &log), [2; 32], Duration::from_secs(10)).await;
        took.unwrap();
        assert_eq!((two.build.as_deref(), two.digest), (Some("tree:2"), [2; 32]));
        assert_eq!(lines(&log), ["up-1", "up-2", "down-1"], "the old worker went only once the new one was up");
        two.stop(Duration::from_secs(5)).await;
        assert_eq!(lines(&log).last().map(String::as_str), Some("down-2"), "closing its pipe is enough");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_worker_that_never_becomes_ready_changes_nothing() {
        let dir = scratch("refused");
        let log = dir.join("log");
        let sh = Path::new("/bin/sh");
        let mut one = Worker::start(sh, &stand_in(1, &log), [1; 32]).unwrap();
        one.ready(Duration::from_secs(10)).await.unwrap();
        let never = vec!["-c".to_string(), format!("echo up-2 >> '{}'; exec sleep 30", log.display())];
        let (mut one, took) = hand_over(one, sh, &never, [2; 32], Duration::from_millis(500)).await;
        let why = took.expect_err("a worker that never said ready took over");
        assert!(why.to_string().contains("not up within"), "{why:#}");
        assert_eq!(one.digest, [1; 32]);
        assert!(one.child.try_wait().unwrap().is_none(), "the old worker carries on");
        assert!(!lines(&log).iter().any(|l| l == "down-1"), "and was never told to go: {:?}", lines(&log));
        one.stop(Duration::from_secs(5)).await;
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    fn wait_for_legacy_probe(file: &std::fs::File, budget: Duration) -> std::result::Result<(), std::fs::TryLockError> {
        let deadline = Instant::now() + budget;
        loop {
            match file.try_lock() {
                Err(std::fs::TryLockError::WouldBlock) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                result => return result,
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn legacy_lock_waits_for_an_inherited_probe_descriptor() {
        let dir = scratch("inherited-probe");
        std::fs::write(dir.join("agent.lock"), "4242 tree:old\n").unwrap();
        let probe = open_lock(&dir).unwrap();
        probe.try_lock().unwrap();
        let legacy = open_lock(&dir).unwrap();
        assert!(matches!(legacy.try_lock(), Err(std::fs::TryLockError::WouldBlock)));
        // The descriptor outlives the parent probe, as it can between fork/exec.
        let release = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(250));
            drop(probe);
        });
        let acquired = wait_for_legacy_probe(&legacy, Duration::from_secs(5));
        release.join().unwrap();
        if acquired.is_ok() {
            assert_eq!(peek(&dir).unwrap().unwrap(), Holder { pid: 4242, build: "tree:old".into(), keeper: false });
        }
        drop(legacy);
        let _ = std::fs::remove_dir_all(&dir);
        acquired.expect("legacy acquisition must wait for the inherited probe descriptor");
    }

    #[cfg(unix)]
    #[test]
    fn a_keeper_is_left_alone_and_a_legacy_agent_is_named() {
        let dir = scratch("lock");
        let held = lock(&dir).unwrap();
        assert!(matches!(held, Lock::Held(_)));
        let me = Holder { pid: std::process::id() as i32, build: env!("AGENT_ROUTER_BUILD").into(), keeper: true };
        match lock(&dir).unwrap() {
            Lock::Busy(h) => assert_eq!(h, me),
            Lock::Held(_) => panic!("the lock was taken twice"),
        }
        assert_eq!(peek(&dir).unwrap(), Some(me));
        assert_eq!(on_busy(&peek(&dir).unwrap().unwrap()), Busy::LeaveAlone, "a keeper hands over by itself");
        drop(held);
        // Free once dropped. Other tests in this process start children, and one between its
        // fork and its exec still holds a copy of the descriptor for that moment.
        let deadline = Instant::now() + Duration::from_secs(2);
        while peek(&dir).unwrap().is_some() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(peek(&dir).unwrap(), None);

        // An agent from before keepers wrote itself into the lock file; a stale agent.pid
        // beside it does not name it a keeper.
        std::fs::write(dir.join("agent.lock"), "4242 tree:old\n").unwrap();
        let legacy = open_lock(&dir).unwrap();
        // The last peek can itself be inherited by a concurrent fork. Wait on
        // the acquisition we need, instead of relying on a separate free probe.
        wait_for_legacy_probe(&legacy, Duration::from_secs(5)).unwrap();
        let h = peek(&dir).unwrap().unwrap();
        assert_eq!(h, Holder { pid: 4242, build: "tree:old".into(), keeper: false });
        assert_eq!(on_busy(&h), Busy::Replace(4242));
        assert_eq!(on_busy(&Holder { pid: 0, build: String::new(), keeper: false }), Busy::AtNextLogon, "Windows cannot name it");
        drop(legacy);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The router side of a handover, as the router already behaves: the newer tunnel for a
    /// machine is the one it routes to, and the older one's closing neither unregisters it nor
    /// marks the machine offline.
    #[tokio::test]
    async fn the_router_sees_no_offline_across_a_handover() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use tokio_tungstenite::tungstenite::client::IntoClientRequest;

        let lab = crate::api_refusals::Lab::hosted("handover").await;
        let password = lab.password.unwrap();
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = lab.app.clone().into_make_service_with_connect_info::<std::net::SocketAddr>();
        tokio::spawn(async move { axum::serve(listener, app).await });

        let tunnel = |answer: &'static str| async move {
            let routes = Router::new().route("/api/live/sessions", get(move || async move { Json(json!([{ "session_id": answer }])) }));
            let mut request = format!("ws://{addr}{}", tunnel::PATH).into_client_request().unwrap();
            request.headers_mut().insert(axum::http::header::AUTHORIZATION, format!("Bearer {password}").parse().unwrap());
            let tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
            let (socket, _) = tokio_tungstenite::client_async(request, tcp).await.unwrap();
            let hello = Hello {
                machine: "one-machine".into(),
                label: "handover".into(),
                platform: std::env::consts::OS.into(),
                version: "test".into(),
                build: answer.into(),
            };
            tokio::spawn(async move {
                let _ = serve(socket, &hello, routes).await;
            })
            .abort_handle()
        };
        let listed = || async {
            let mut req = axum::http::Request::get("/api/live/sessions")
                .header(axum::http::header::HOST, "127.0.0.1")
                .header(axum::http::header::AUTHORIZATION, format!("Bearer {password}"))
                .body(Body::empty())
                .unwrap();
            req.extensions_mut().insert(axum::extract::ConnectInfo(std::net::SocketAddr::from(([127, 0, 0, 1], 50_000))));
            let res = lab.app.clone().oneshot(req).await.unwrap();
            let body = axum::body::to_bytes(res.into_body(), 1 << 20).await.unwrap();
            let rows: Value = serde_json::from_slice(&body).unwrap();
            rows.as_array().unwrap().iter().filter_map(|r| r["session_id"].as_str().map(str::to_owned)).collect::<Vec<_>>()
        };
        let online = || lab.state.machines.seen().iter().any(|m| m.online);

        let old = tunnel("old").await;
        tokio::time::timeout(Duration::from_secs(10), async {
            while !online() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the old worker comes online");
        assert_eq!(listed().await, ["old"]);

        // From the moment the new worker connects until after the old one has gone, the
        // machine is never seen offline.
        let dropped = Arc::new(AtomicBool::new(false));
        let watching = Arc::new(AtomicBool::new(true));
        let watcher = {
            let (dropped, watching, state) = (dropped.clone(), watching.clone(), lab.state.clone());
            tokio::spawn(async move {
                while watching.load(Ordering::SeqCst) {
                    if !state.machines.seen().iter().any(|m| m.online) {
                        dropped.store(true, Ordering::SeqCst);
                    }
                    tokio::time::sleep(Duration::from_millis(2)).await;
                }
            })
        };
        let new = tunnel("new").await;
        tokio::time::timeout(Duration::from_secs(10), async {
            while listed().await != ["new"] {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the router routes to the newer tunnel");
        old.abort();
        tokio::time::sleep(Duration::from_millis(300)).await;
        watching.store(false, Ordering::SeqCst);
        watcher.await.unwrap();
        assert!(!dropped.load(Ordering::SeqCst), "the machine was seen offline during the handover");
        assert!(online());
        assert_eq!(listed().await, ["new"], "still the newer tunnel once the older one has gone");
        new.abort();
    }

    #[tokio::test]
    async fn a_router_build_already_checked_downloads_nothing() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        rustls::crypto::ring::default_provider().install_default().ok();
        let dir = scratch("checked");
        let installed = dir.join("agent-router");
        std::fs::write(&installed, b"the same build").unwrap();
        let downloads = Arc::new(AtomicUsize::new(0));
        let name = if cfg!(windows) { "agent-router.exe" } else { "agent-router" };
        let app = {
            let downloads = downloads.clone();
            Router::new()
                .route("/_router/health", get(|| async { Json(json!({ "ok": true, "build": "tree:r1" })) }))
                .route(
                    &format!("/agent/{}/{name}", platform()),
                    get(move || {
                        let downloads = downloads.clone();
                        async move {
                            downloads.fetch_add(1, Ordering::SeqCst);
                            b"the same build".to_vec()
                        }
                    }),
                )
        };
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await });
        let router = crate::library::sync::Router::new(&format!("http://{addr}"), "pw").unwrap();
        fn never(_: &Path) -> Result<String> {
            panic!("an unchanged binary is never run")
        }

        assert_eq!(update_from(&router, &dir, &installed, never).await.unwrap(), None);
        assert_eq!(downloads.load(Ordering::SeqCst), 1, "a router build not checked yet is fetched once");
        assert_eq!(load_saved_at(&dir.join("agent.json")).checked_build.as_deref(), Some("tree:r1"));
        assert_eq!(update_from(&router, &dir, &installed, never).await.unwrap(), None);
        assert_eq!(downloads.load(Ordering::SeqCst), 1, "and never again for the same router build");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_download_that_will_not_run_is_never_installed() {
        let dir = scratch("broken");
        let installed = dir.join("agent-router");
        std::fs::write(&installed, b"the running build").unwrap();
        let err = install_if_different(&installed, b"#!/bin/sh\nexit 3\n", try_run).unwrap_err();
        assert!(format!("{err:#}").contains("does not run here"), "{err:#}");
        assert_eq!(std::fs::read(&installed).unwrap(), b"the running build");
        let left: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(left, ["agent-router"], "nothing is left beside it");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_new_build_is_installed_by_rename() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let dir = scratch("renamed");
        let installed = dir.join("agent-router");
        std::fs::write(&installed, b"the running build").unwrap();
        let before = std::fs::metadata(&installed).unwrap().ino();
        let new = b"#!/bin/sh\necho tree:new\n";
        assert_eq!(install_if_different(&installed, new, try_run).unwrap().as_deref(), Some("tree:new"));
        assert_eq!(std::fs::read(&installed).unwrap(), new);
        let meta = std::fs::metadata(&installed).unwrap();
        assert_ne!(meta.ino(), before, "renamed into place, never written over a running binary");
        assert_eq!(meta.permissions().mode() & 0o777, 0o755);
        assert_eq!(install_if_different(&installed, new, try_run).unwrap(), None, "the same bytes again change nothing");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_machine_that_opted_out_never_updates() {
        let out = Saved { update: Some(false), ..Default::default() };
        assert!(!may_update(&out, true));
        assert!(may_update(&Saved::default(), true));
        assert!(!may_update(&Saved::default(), false), "only the installed agent updates itself");
        assert_eq!(parse_install(&["--server".into(), "https://r.example.com".into(), "--no-auto-update".into()]).unwrap().auto_update, Some(false));
        let json = serde_json::to_value(&out).unwrap();
        assert_eq!(json["update"], false, "remembered in agent.json");
    }

    #[test]
    fn a_server_url_gives_the_socket_to_dial() {
        let t = Target::parse("https://agent-router.example.com/").unwrap();
        assert_eq!(t, Target { tls: true, host: "agent-router.example.com".into(), port: 443, base: String::new() });
        assert_eq!(t.ws_url(), "wss://agent-router.example.com/_router/agent");
        let t = Target::parse("http://192.0.2.10:8787").unwrap();
        assert_eq!((t.tls, t.port), (false, 8787));
        assert_eq!(t.ws_url(), "ws://192.0.2.10:8787/_router/agent");
        let t = Target::parse("https://example.com:8443/router").unwrap();
        assert_eq!(t.ws_url(), "wss://example.com:8443/router/_router/agent");
        let t = Target::parse("http://[::1]:8799").unwrap();
        assert_eq!(t.ws_url(), "ws://[::1]:8799/_router/agent");
        for bad in ["agent-router.example.com", "ftp://x", "https://", "https://x:port"] {
            assert!(Target::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn options_are_read_and_unknown_ones_refused() {
        let a = parse(&["--server=https://r.example.com".into(), "--label".into(), "mac".into(), "--ensure".into()]).unwrap();
        assert_eq!(a.server.as_deref(), Some("https://r.example.com"));
        assert_eq!(a.label.as_deref(), Some("mac"));
        assert!(a.ensure);
        assert!(parse(&["--sever".into()]).is_err());
    }

    #[test]
    fn a_machine_id_is_a_version_4_uuid() {
        let id = new_machine_id();
        assert_eq!(id.len(), 36);
        assert_eq!(id.as_bytes()[14], b'4');
        assert!(matches!(id.as_bytes()[19], b'8' | b'9' | b'a' | b'b'));
        assert_ne!(id, new_machine_id());
    }

    /// `install --no-library` is remembered in agent.json, and a machine that has it takes
    /// no part: nothing is asked of the router and nothing is written under ~/.claude.
    #[tokio::test]
    async fn a_machine_that_opted_out_never_writes_into_claude() {
        rustls::crypto::ring::default_provider().install_default().ok();
        let args = |rest: &[&str]| parse_install(&rest.iter().map(|s| s.to_string()).collect::<Vec<_>>()).unwrap();
        assert_eq!(args(&["--server", "https://r.example.com", "--no-library"]).library, Some(false));
        assert_eq!(args(&["--server", "https://r.example.com", "--library"]).library, Some(true));
        assert_eq!(args(&["--server", "https://r.example.com"]).library, None, "left as it was saved");
        let saved: Saved = serde_json::from_str(&serde_json::to_string(&Saved { library: Some(false), ..Default::default() }).unwrap()).unwrap();
        assert_eq!(saved.library, Some(false));
        let before: Saved = serde_json::from_str(r#"{"server":"https://r.example.com","password":"p"}"#).unwrap();
        assert_eq!(before.library, None, "a machine installed before the Library takes it");

        let root = std::env::temp_dir().join(format!("ar-opted-out-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let places = crate::library::sync::Places { claude: root.join("claude"), data: root.join("data"), form: Default::default(), kimi: None };
        // No router answers there: a machine that asked anything would fail.
        let router = Arc::new(crate::library::sync::Router::new("http://127.0.0.1:9", "unused").unwrap());
        let report = crate::library::sync::run_once(places.clone(), router.clone(), Some(false)).await.unwrap();
        assert!(report.is_none());
        assert!(!root.exists(), "nothing was written anywhere");
        assert!(crate::library::sync::run_once(places, router, None).await.is_err(), "a machine that takes it does ask");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The tunnel's one new route syncs the Library and answers its report; a machine that
    /// opted out says so; and everything else the router might try is still not there.
    #[tokio::test]
    async fn the_tunnel_serves_library_sync_and_nothing_else_new() {
        use crate::library::store::{self, Provenance};
        use crate::library::{bundle::Bundle, bundle::File, Kind};

        let lab = crate::api_refusals::Lab::hosted("agent-library-route").await;
        let conn = store::writer(&lab.state.config.db_path).unwrap();
        let text = "---\nname: bench\ndescription: Drives the bench.\n---\n";
        let bench = Bundle::check(Kind::Skill, "bench", vec![File { path: "SKILL.md".into(), bytes: text.into(), executable: false }]).unwrap();
        store::publish(&conn, &bench, &Provenance::Upload, true, 1).unwrap();
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = lab.app.clone().into_make_service_with_connect_info::<std::net::SocketAddr>();
        tokio::spawn(async move { axum::serve(listener, app).await });

        let root = std::env::temp_dir().join(format!("ar-agent-library-route-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let library = Library {
            places: crate::library::sync::Places { claude: root.join("claude"), data: root.join("data"), form: Default::default(), kimi: None },
            router: Arc::new(crate::library::sync::Router::new(&format!("http://{addr}"), lab.password.unwrap()).unwrap()),
            machine: "m".into(),
            label: "the test's machine".into(),
        };
        let ask = |method: &str, path: &str| {
            axum::http::Request::builder().method(method).uri(path).body(Body::empty()).unwrap()
        };

        let synced = routes(Some(library.clone())).oneshot(ask("POST", "/agent/library/sync")).await.unwrap();
        assert_eq!(synced.status(), 200);
        let report: crate::library::sync::Report =
            serde_json::from_slice(&axum::body::to_bytes(synced.into_body(), 1 << 20).await.unwrap()).unwrap();
        assert_eq!(report.items.len(), 1);
        assert_eq!(report.items[0].outcome, crate::library::sync::Outcome::Placed);
        assert_eq!(std::fs::read_to_string(root.join("claude/skills/bench/SKILL.md")).unwrap(), text);

        let refused = routes(None).oneshot(ask("POST", "/agent/library/sync")).await.unwrap();
        assert_eq!(refused.status(), 409);
        let words: Value = serde_json::from_slice(&axum::body::to_bytes(refused.into_body(), 1 << 20).await.unwrap()).unwrap();
        assert!(words["error"].as_str().unwrap().contains("--no-library"), "{words}");

        assert_eq!(routes(Some(library.clone())).oneshot(ask("GET", "/agent/library/sync")).await.unwrap().status(), 405);
        for (method, path) in [
            ("POST", "/agent/library/publish"),
            ("GET", "/api/library/manifest"),
            ("POST", "/api/setup/shim"),
            ("POST", "/api/setup/config"),
            ("GET", "/agent/install.sh"),
        ] {
            let answer = routes(Some(library.clone())).oneshot(ask(method, path)).await.unwrap();
            assert_eq!(answer.status(), 404, "{method} {path}");
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The agent's routes answer what the router may ask, and nothing else.
    #[tokio::test]
    async fn only_the_live_routes_are_served() {
        let options = axum::http::Request::builder().uri("/api/setup").body(Body::empty()).unwrap();
        assert_eq!(routes(None).oneshot(options).await.unwrap().status(), 404);
        let setup = axum::http::Request::builder().method("POST").uri("/api/setup/shim").body(Body::empty()).unwrap();
        assert_eq!(routes(None).oneshot(setup).await.unwrap().status(), 404);
    }

    #[test]
    fn the_agent_keeps_picker_rows_in_step_with_the_catalogue() {
        let dir = std::env::temp_dir().join(format!("ar-agent-picker-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let settings = dir.join("settings.json");
        let server = "https://router.example.invalid";
        std::fs::write(
            &settings,
            format!(r#"{{"env":{{"ANTHROPIC_BASE_URL":"{server}"}},"modelPicker":{{"options":[{{"model":"opus","label":"Mine"}}]}}}}"#),
        )
        .unwrap();
        let catalogue = |ids: &[&str]| -> Vec<Value> {
            ids.iter().map(|id| json!({ "id": id, "provider": if id.starts_with("claude") { "anthropic" } else { "kimi" }, "display_name": id })).collect()
        };
        let rows = |path: &Path| -> Vec<String> {
            let got: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
            got["modelPicker"]["options"].as_array().unwrap().iter().map(|r| r["model"].as_str().unwrap().to_string()).collect()
        };

        assert!(picker_rows_for(&settings, server, &catalogue(&["claude-opus-5-5", "k3", "k3-256k"])).unwrap());
        assert_eq!(rows(&settings), ["opus", "k3", "k3-256k"]);
        // A model that joined the catalogue gets a row, and one that left loses its own.
        assert!(picker_rows_for(&settings, server, &catalogue(&["k3", "kimi-for-coding"])).unwrap());
        assert_eq!(rows(&settings), ["opus", "k3", "kimi-for-coding"]);
        assert!(!picker_rows_for(&settings, &format!("{server}/"), &catalogue(&["k3", "kimi-for-coding"])).unwrap(), "in step");

        // A Claude Code pointed at another router is offered none of this one's models.
        let elsewhere = dir.join("elsewhere.json");
        std::fs::write(&elsewhere, std::fs::read_to_string(&settings).unwrap().replace(server, "http://127.0.0.1:8787")).unwrap();
        assert!(picker_rows_for(&elsewhere, server, &catalogue(&["k3"])).unwrap());
        assert_eq!(rows(&elsewhere), ["opus"], "its rows go; the owner's stays");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
