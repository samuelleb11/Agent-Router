//! The machine agent: what makes a developer machine's sessions visible to a hosted
//! router. See `docs/hosting-machine-agent.md`; the frames are `tunnel`'s.
//!
//! A hosted router cannot reach into a laptop, so the agent reaches out: it holds one
//! WebSocket open to the router and answers the requests that come down it — list the
//! sessions here, follow this one, answer that prompt — by running them through the very
//! `live` handlers a local router serves, in process. It listens on nothing. Only the
//! routes in `routes` can be asked for: nothing about setting the machine up, installing
//! anything, or reading files the live view does not.

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
                                     [--no-service] [--no-config]
                                     check the password, save it, point Claude Code at
                                     the server, and start the agent at login
  agent-router agent uninstall       undo exactly what install changed
  agent-router agent status          what is installed, and whether it runs
  agent-router agent [--server <url>] [--label <name>] [--data-dir <dir>]
                                     run the agent
  agent-router agent --ensure        run only if no agent from this build runs here

The password comes from AGENT_ROUTER_PASSWORD, from standard input, or from a prompt;
`install` saves it, owner-only, where the agent reads it from then on.
";

#[derive(Debug, Default)]
struct RunArgs {
    server: Option<String>,
    label: Option<String>,
    data_dir: Option<PathBuf>,
    ensure: bool,
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
}

fn saved_path() -> Result<PathBuf> {
    Ok(crate::config::data_dir()?.join("agent.json"))
}

fn load_saved() -> Saved {
    saved_path()
        .ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

/// Written whole and owner-only: it holds the router password.
fn store_saved(saved: &Saved) -> Result<()> {
    let path = saved_path()?;
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

/// The routes a router may ask an agent for: the live view, and what the notifier needs.
/// Nothing else is served — no setup, no shim, no files — so the router can never ask
/// a machine for more than its sessions.
fn routes() -> Router {
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
        .route("/agent/waiting", get(waiting))
        .route("/agent/still-waiting", get(still_waiting))
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

/// The one agent a machine runs: a lock held for the life of the process, naming the
/// holder so a newer build can ask it to make way.
enum Lock {
    Held(#[allow(dead_code)] std::fs::File),
    Busy { pid: i32, build: String },
}

fn lock(dir: &Path) -> Result<Lock> {
    let path = dir.join("agent.lock");
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&path)
        .with_context(|| format!("opening {}", path.display()))?;
    match file.try_lock() {
        Ok(()) => {
            use std::io::{Seek, Write};
            file.set_len(0)?;
            file.seek(std::io::SeekFrom::Start(0))?;
            writeln!(file, "{} {}", std::process::id(), env!("AGENT_ROUTER_BUILD"))?;
            file.flush()?;
            Ok(Lock::Held(file))
        }
        Err(std::fs::TryLockError::WouldBlock) => {
            let raw = std::fs::read_to_string(&path).unwrap_or_default();
            let mut parts = raw.split_whitespace();
            let pid = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
            let build = parts.next().unwrap_or_default().to_string();
            Ok(Lock::Busy { pid, build })
        }
        Err(std::fs::TryLockError::Error(e)) => Err(e.into()),
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
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("AGENT_ROUTER_LOG").unwrap_or_else(|_| "agent_router=info".into()),
        )
        .with_target(false)
        .init();
    rustls::crypto::ring::default_provider().install_default().ok();
    let mut stopping = crate::lifecycle::listen_for_shutdown();

    let dir = crate::config::settle_data_dir(args.data_dir.as_deref())?;
    let _held = match lock(&dir)? {
        Lock::Held(file) => file,
        Lock::Busy { pid, build } if args.ensure && build == env!("AGENT_ROUTER_BUILD") => {
            println!("agent-router agent already running (pid {pid}); nothing to do");
            return Ok(());
        }
        Lock::Busy { pid, .. } if args.ensure => {
            tracing::info!(pid, "an agent from another build is running; asking it to make way");
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
        Lock::Busy { pid, .. } => bail!("an agent is already running here (pid {pid})"),
    };

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

    let router = routes();
    let mut backoff = Duration::from_secs(1);
    loop {
        let started = Instant::now();
        let outcome = tokio::select! {
            outcome = connect_and_serve(&target, &password, &hello, router.clone()) => outcome,
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
    store_saved(&saved)?;
    println!("saved     {} (owner-only)", saved_path()?.display());

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
    }

    if args.no_service {
        println!("service   left alone (--no-service); start it with `agent-router agent`");
    } else {
        // The settings are already written: a login item that cannot be registered here
        // is said, not a failure of the whole install.
        let registered = crate::setup::install_binary().and_then(|exe| crate::setup::agent_service::install(&exe));
        match registered {
            Ok(path) => {
                println!("service   registered: {}", path.display());
                println!("          the agent starts now, and at every login");
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
    crate::setup::agent_service::uninstall()?;
    // launchd stops what it started; anything else still running (the editor's, or one
    // started by hand) is asked to stop through its lock.
    let dir = crate::config::data_dir()?;
    if let Lock::Busy { pid, .. } = lock(&dir)? {
        let _ = ask_to_stop(pid);
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
    match lock(&dir)? {
        Lock::Busy { pid, build } => println!("running   yes (pid {pid}, {build})"),
        Lock::Held(_) => println!("running   no"),
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

async fn connect_and_serve(target: &Target, password: &str, hello: &Hello, router: Router) -> Result<()> {
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

    /// The agent's routes answer what the router may ask, and nothing else.
    #[tokio::test]
    async fn only_the_live_routes_are_served() {
        let options = axum::http::Request::builder().uri("/api/setup").body(Body::empty()).unwrap();
        assert_eq!(routes().oneshot(options).await.unwrap().status(), 404);
        let setup = axum::http::Request::builder().method("POST").uri("/api/setup/shim").body(Body::empty()).unwrap();
        assert_eq!(routes().oneshot(setup).await.unwrap().status(), 404);
    }
}
