//! agent-router — a local proxy between Claude Code and Anthropic.
// On Windows the binary runs only as the machine agent, so the router's own machinery is
// compiled there but never reached.
#![cfg_attr(windows, allow(dead_code, unused_imports))]
//!
//! Phase 1: one endpoint, one credential, byte-faithful streaming passthrough.
//! Capture, the transcript join, multi-account switching and the web UI follow.
//! See ARCHITECTURE.md.

mod accounts;
mod activity;
mod agent;
mod analyze;
mod api;
#[cfg(test)]
mod api_refusals;
mod apns;
mod auth;
mod capture;
mod chat_completions;
mod chatgpt;
mod clients;
mod client_setup;
mod config;
mod console;
mod db;
mod downloads;
mod fcm;
mod fold;
mod forecast;
mod guard;
mod hosted;
mod kimi;
mod library;
mod lifecycle;
mod live;
mod machines;
mod network;
mod peer;
mod places;
mod pricing;
mod login;
mod models;
mod provider_registry;
mod notify;
mod proxy;
mod quota;
mod refusal;
mod registry;
mod repository;
mod setup;
mod shim;
mod transcript;
mod responses;
mod translate;
mod codex;
mod tunnel;

use std::sync::Arc;
use std::time::Duration;

use axum::routing::{any, get, post};
use axum::Router;

const USAGE: &str = "\
agent-router — a local proxy between Claude Code and Anthropic

USAGE:
  agent-router                       run the router (default)
  agent-router serve [OPTIONS]       the same, with options
  agent-router --ensure              run only if no router is serving, or an older
                                     build is — then take the port over without a gap
  agent-router --if-not-running      run only if no router is serving
  agent-router --version             print the version
  agent-router install [OPTIONS]     point Claude Code at it, and start it at login
  agent-router uninstall             undo exactly what install changed
  agent-router status                show what is installed and what is managed
  agent-router agent [OPTIONS]       connect this machine's sessions to a hosted router
  agent-router library status|sync   the hosted router's Library on this machine
  agent-router codex models refresh|restore
                                     refresh Codex's server model list or restore it
  agent-router kimi [--model <id>] [prompt]
                                     a hosted Kimi Code session in this directory,
                                     worked with here and from the console

SERVE OPTIONS:
  --bind <ip>            also listen here, not only on loopback (like AGENT_ROUTER_BIND)
  --port <n>             router port (default 8787)
  --data-dir <dir>       where everything the router writes lives (default
                         ~/.agent-router; AGENT_ROUTER_DATA_DIR)
  --db <path>            the store (default <data-dir>/agent-router.db; AGENT_ROUTER_DB)
  --server               run as a hosted server, behind an HTTPS ingress: --bind is
                         the only listener, every caller presents the password, which
                         is pinned from AGENT_ROUTER_SERVER_PASSWORD, and nothing of a
                         developer's machine is read. These need --server:
  --allow-host <name>    a name clients reach the router by; repeatable
  --public-url <url>     the https URL clients use; its host is allowed too
  --push-keys-dir <dir>  a read-only directory of push keys (AuthKey_<id>.p8, or a
                         Firebase service-account JSON)
  --agents-dir <dir>     the agent's files its installer hands out, one directory per
                         platform (default <binary>/../share/agent-router/agents)
  --drain-seconds <n>    how long a stop waits for open requests (default 20)
  --stop-delay-seconds <n>
                         how long a stop keeps accepting first, while the load
                         balancer lets go of this router (default 0)

INSTALL OPTIONS:
  --project <dir>   configure one project instead of every project
  --no-service      only edit the config; do not register a login item
  --no-config       only register the login item; do not edit the config
  --port <n>        router port (default 8787)
  --bind <addr>     also listen here, not only on loopback, from now on and after
                    every reboot (0.0.0.0 lets the phone app connect — set a
                    password first; 127.0.0.1 puts it back). The dashboard's
                    Listen-on-your-network switch changes the same setting.

Only one settings key is ever touched: env.ANTHROPIC_BASE_URL. Everything else in
your settings file is left exactly as it is, and uninstall restores whatever was
there before — unless you have since changed it yourself, in which case it is left
alone.
";

/// Where a router on this machine answers, for the flags that ask before serving.
fn loopback_addr() -> std::net::SocketAddr {
    let port: u16 = std::env::var("AGENT_ROUTER_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8787);
    loopback_addr_on(port)
}

/// The same, for a port already resolved. Every router serves this address, whatever
/// else it may also be listening on.
fn loopback_addr_on(port: u16) -> std::net::SocketAddr {
    std::net::SocketAddr::from(([127, 0, 0, 1], port))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if std::env::current_exe()?.file_stem().is_some_and(|s|s=="agent-router-codex") {
        return codex::runtime::adapter(&args).await;
    }
    match args.first().map(String::as_str) {
        Some("serve") => {
            return match config::ServeArgs::parse(&args[1..]) {
                Ok(serve_args) => serve(serve_args).await,
                Err(why) => {
                    eprintln!("{why}\n\n{USAGE}");
                    std::process::exit(2);
                }
            };
        }
        Some("agent") => return agent::main(&args[1..]).await,
        Some("library") => return library::cli::main(&args[1..]).await,
        Some("install") => return cmd_install(&args[1..]),
        Some("uninstall") => return cmd_uninstall(),
        Some("status") => return cmd_status(),
        Some("hook") => return kimi::hook(&args[1..]),
        Some("kimi") => return kimi::launch::main(&args[1..]).await,
        Some("codex") => return codex::runtime::cli(&args[1..]).await,
        Some("-h") | Some("--help") | Some("help") => {
            print!("{USAGE}");
            return Ok(());
        }
        Some("--version") | Some("-V") => {
            println!("agent-router {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        // The build alone, `tree:` and the compiled tree: what a machine agent runs a new
        // binary with before it trusts it (`agent::try_run`).
        Some("--build") => {
            println!("{}", env!("AGENT_ROUTER_BUILD"));
            return Ok(());
        }
        Some("--if-not-running") => {
            // Something already serving means our job is done. Used by the shell
            // profile autostart, which runs on every new shell.
            if lifecycle::incumbent(loopback_addr()).is_some() {
                return Ok(());
            }
            return serve(config::ServeArgs::default()).await;
        }
        Some("--ensure") => {
            // Serve only if nobody does, or if whoever does is an older build — then
            // the handover replaces them without dropping a request. The editor
            // extension runs this on every window it opens, so "nothing to do" has to
            // be quick and quiet.
            let ours = env!("CARGO_PKG_VERSION");
            match lifecycle::incumbent_version(loopback_addr()) {
                Some((pid, theirs)) if !lifecycle::is_newer(ours, &theirs) => {
                    println!("agent-router {theirs} already serving (pid {pid}); nothing to do");
                    return Ok(());
                }
                Some((pid, theirs)) => {
                    println!("agent-router {theirs} (pid {pid}) is older than {ours}; taking over");
                }
                None => {}
            }
            return serve(config::ServeArgs::default()).await;
        }
        Some(other) if other.starts_with('-') => {
            eprintln!("unknown option {other:?}\n\n{USAGE}");
            std::process::exit(2);
        }
        Some(other) => {
            eprintln!("unknown command {other:?}\n\n{USAGE}");
            std::process::exit(2);
        }
        None => {}
    }
    serve(config::ServeArgs::default()).await
}

fn flag_value(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn cmd_install(args: &[String]) -> anyhow::Result<()> {
    let port: u16 = flag_value(args, "--port")
        .and_then(|v| v.parse().ok())
        .unwrap_or(8787);
    let url = format!("http://127.0.0.1:{port}");
    let project = flag_value(args, "--project").map(std::path::PathBuf::from);
    let scope = if project.is_some() { "project" } else { "global" };

    // Where to listen is a stored setting rather than something baked into the login
    // item, so it holds however the router is next started and the dashboard can change
    // it later without a reinstall.
    if let Some(addr) = flag_value(args, "--bind") {
        let ip: std::net::IpAddr = addr
            .trim()
            .parse()
            .map_err(|_| anyhow::anyhow!("--bind wants an IP address, got {addr:?}"))?;
        let db_path = config::db_path()?;
        // A first install has no store yet, and the setting needs a table to live in.
        db::Db::open(&db_path)?;
        let wide = (!ip.is_loopback()).then_some(ip);
        network::store(&db_path, wide)?;
        setup::forget_bind_env();
        match wide {
            Some(ip) => println!("listen    on {ip} as well as loopback — set a password before using it"),
            None => println!("listen    loopback only"),
        }
    }

    if !args.iter().any(|a| a == "--no-service") {
        let exe = setup::install_binary()?;
        match setup::service::install(&exe) {
            Ok(path) => println!("service   registered: {}", path.display()),
            Err(err) => println!("service   skipped: {err}"),
        }
    }
    if args.iter().any(|a| a == "--no-config") {
        println!("config    left alone (--no-config)");
        println!("\ndashboard {url}");
        return Ok(());
    }

    let path = setup::settings_path(project.as_deref())?;
    match setup::apply(&path, &url, scope)? {
        setup::Applied::Added => println!("config    added ANTHROPIC_BASE_URL={url}"),
        setup::Applied::Replaced { previous } => {
            println!("config    ANTHROPIC_BASE_URL={url}");
            println!("          was {previous} — uninstall will put that back");
        }
        setup::Applied::Adopted => {
            println!("config    already pointed at {url}; now tracked so uninstall can undo it")
        }
        setup::Applied::AlreadySet => println!("config    already pointing at {url}"),
    }
    println!("          {} ({scope})", path.display());
    match local_picker_rows() {
        Ok(0) => {}
        Ok(n) => println!("picker    {n} model(s) of other providers offered in Claude Code's /model"),
        Err(e) => println!("picker    left alone ({e:#})"),
    }
    match local_kimi_config(&url, true) {
        Ok(Some(_)) => {
            println!("kimi      Kimi Code's config.toml: the router's provider, its Kimi models and hooks");
            println!("          uninstall takes them out and puts back the default model")
        }
        Ok(None) => {}
        Err(e) => println!("kimi      Kimi Code's config.toml left alone ({e:#})"),
    }
    match codex::here(&url, "agent-router", true, true) {
        Ok(Some(_)) => {
            println!("codex     Codex's config.toml: Responses through Agent Router; uninstall restores its provider");
            if let Err(error)=local_codex_catalogue(true) {println!("codex     model catalogue left alone ({error:#})");}
        },
        Ok(None) => {},
        Err(e) => println!("codex     Codex's config.toml left alone ({e:#})"),
    }
    println!("\ndashboard {url}");
    Ok(())
}

/// The router's catalogue, from this machine's own store; empty for a store not made yet.
fn local_catalogue() -> anyhow::Result<Vec<serde_json::Value>> {
    let db = config::db_path()?;
    if !db.exists() {
        return Ok(Vec::new());
    }
    let conn = rusqlite::Connection::open(&db)?;
    Ok(models::catalogue(&conn)?.iter().map(models::row).collect())
}

fn local_codex_catalogue(install: bool) -> anyhow::Result<()> {
    let db=config::db_path()?;
    let path=accounts::codex_home()?.join("config.toml");
    if !db.exists() || !path.exists() {return Ok(());}
    let metadata=models::client_catalogue(&rusqlite::Connection::open(db)?)?;
    if install && metadata.models.is_empty() {return Ok(());}
    codex::catalogue::sync(&path.canonicalize()?,&metadata,install)?;
    Ok(())
}

/// This machine's Kimi Code pointed at the local router at `url` (`kimi::config::here`). A
/// local router takes requests from loopback without a password, so the key is a name.
fn local_kimi_config(url: &str, install: bool) -> anyhow::Result<Option<bool>> {
    let consented = setup::base_url_in(&setup::settings_path(None)?)?
        .is_some_and(|set| set.trim_end_matches('/') == url.trim_end_matches('/'));
    let db = config::db_path()?;
    if db.exists() {
        if let Err(e) = codex::discover_account(&db) { tracing::warn!("Codex account discovery: {e:#}"); }
    }
    if !install {
        if let Err(e) = codex::here(url, "agent-router", false, consented) { tracing::warn!("Codex config could not be kept in step: {e:#}"); }
        if let Err(e) = local_codex_catalogue(false) {tracing::warn!("Codex model catalogue could not be kept in step: {e:#}");}
    }
    kimi::config::here(url, "agent-router", &local_catalogue()?, install, consented)
}

/// Kimi Code's entries put back after it updates itself, every fifteen minutes, as an agent
/// does for a hosted router's (`agent::keep_clients`).
async fn keep_local_kimi_config(url: String) {
    loop {
        let url = url.clone();
        match tokio::task::spawn_blocking(move || local_kimi_config(&url, false)).await {
            Ok(Ok(Some(true))) => tracing::info!("Kimi Code's config.toml: the router's entries put back"),
            Ok(Ok(_)) => {}
            Ok(Err(e)) => tracing::warn!("Kimi Code's config.toml could not be kept: {e:#}"),
            Err(e) => tracing::warn!("Kimi Code's config.toml could not be kept: {e}"),
        }
        tokio::time::sleep(library::sync::EVERY).await;
    }
}

/// Offer the router's non-Claude models in Claude Code's `/model` picker, from this
/// machine's own store: rows of `modelPicker.options` in the user's settings
/// (`setup::sync_picker_rows`). A store not made yet has no other provider to offer.
fn local_picker_rows() -> anyhow::Result<usize> {
    let rows = setup::picker_models(&local_catalogue()?);
    setup::sync_picker_rows(&setup::settings_path(None)?, &rows)?;
    Ok(rows.len())
}

fn cmd_uninstall() -> anyhow::Result<()> {
    let state = setup::load_state();
    if state.managed.is_empty() {
        println!("config    nothing was managed by agent-router");
    }
    for managed in &state.managed {
        let path = std::path::PathBuf::from(&managed.settings_path);
        match setup::revert(&path)? {
            setup::Reverted::Restored { to } => {
                println!("config    restored ANTHROPIC_BASE_URL={to}\n          {}", path.display())
            }
            setup::Reverted::Removed => {
                println!("config    removed ANTHROPIC_BASE_URL\n          {}", path.display())
            }
            setup::Reverted::NotOurs { found } => {
                println!("config    left alone: now set to {found}, which is not what we wrote");
                println!("          {}", path.display());
            }
            setup::Reverted::NotManaged => {}
        }
    }
    match kimi::config::take_out() {
        Ok(Some(true)) => println!("kimi      the router's entries taken out of Kimi Code's config.toml"),
        Ok(_) => {}
        Err(e) => println!("kimi      Kimi Code's config.toml left alone ({e:#})"),
    }
    match codex::take_out() {
        Ok(Some(true)) => println!("codex     the previous Codex provider restored"),
        Ok(_) => {},
        Err(e) => println!("codex     Codex's config.toml left alone ({e:#})"),
    }
    setup::service::uninstall()?;
    println!("service   unregistered");
    println!("\ndata kept at {}", setup::data_dir()?.display());
    Ok(())
}

fn cmd_status() -> anyhow::Result<()> {
    println!(
        "service   {}",
        if setup::service::is_registered() { "registered" } else { "not registered" }
    );
    let state = setup::load_state();
    if state.managed.is_empty() {
        println!("config    no settings files are managed");
    }
    for m in &state.managed {
        println!("config    {} ({})", m.settings_path, m.scope);
        println!("          set to {}", m.value);
        match &m.previous {
            Some(p) => println!("          restores to {p} on uninstall"),
            None => println!("          key did not exist before; uninstall removes it"),
        }
        if let Some(b) = &m.backup_path {
            println!("          backup {b}");
        }
    }
    Ok(())
}

async fn serve(args: config::ServeArgs) -> anyhow::Result<()> {
    // The router holds its port with SO_REUSEPORT and hands it over with signals, and
    // neither exists here; a hosted router runs in a Linux container. What a Windows
    // machine runs is the agent that connects its sessions to one.
    #[cfg(windows)]
    {
        let _ = args;
        anyhow::bail!("on Windows, Agent Router runs as a machine agent: agent-router agent install --server <url>");
    }
    #[cfg(not(windows))]
    serve_here(args).await
}

#[cfg(not(windows))]
async fn serve_here(args: config::ServeArgs) -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("AGENT_ROUTER_LOG")
                .unwrap_or_else(|_| "agent_router=info".into()),
        )
        .with_target(false)
        .init();

    rustls::crypto::ring::default_provider()
        .install_default()
        .ok();

    // Listen for SIGTERM before anything slow. In a container the router is PID 1, and
    // the kernel drops a signal PID 1 has no handler for: a rollout landing during
    // startup would otherwise wait out the whole grace period for its SIGKILL.
    let stopping = lifecycle::listen_for_shutdown();

    // Read first, so a hosted router with no password stops with a sentence before it
    // has opened (and perhaps migrated) the store.
    let server_password = if args.server { Some(config::server_password()?) } else { None };

    let config = config::Config::load(&args)?;
    let server = config.server;
    // Loopback is always served, and a wider bind adds a second listener beside it
    // rather than moving this one — see `network`. It keeps every local tool working
    // whatever the switch is set to, and it keeps the handover probe honest: it has to
    // reach the incumbent on *this* machine. A hosted router has neither: it serves its
    // one bind address, which covers loopback as well when it is 0.0.0.0.
    let port = config.bind.port();
    let bind = loopback_addr_on(port);
    let wide = (!server && !config.bind.ip().is_loopback()).then(|| config.bind.ip());
    // Built before the app it will serve, because the app is built from the state that
    // holds it. The supervisor takes the other half once the app exists.
    let (network, pending) = network::switch();
    let db_path = config.db_path.clone();

    // Create the schema and make sure there is always something to route as: the
    // account Claude Code is already logged into, discovered rather than configured.
    // Not on a server: that login lives on the developer's machine, and looking for it
    // would mean running a `claude` the container does not have.
    {
        let db = db::Db::open(&db_path)?;
        {
            // Rates live in the database so they can be corrected without a rebuild.
            let conn = rusqlite::Connection::open(&db_path)?;
            pricing::ensure_table(&conn)?;
        }
        if server {
            log_hosted_start(&config);
        } else {
            let status = accounts::probe_profile(None).unwrap_or_default();
            let label = status
                .email
                .clone()
                .unwrap_or_else(|| "Claude Code (default profile)".to_string());
            db.upsert_default_account(proxy::ACCOUNT_ID, &label, &status)?;
            codex::discover_account(&db_path)?;
            match wide {
                // The wide listener announces itself as it comes up, in `network`.
                Some(ip) => tracing::info!("agent-router listening on http://{bind} and on {ip}"),
                None => tracing::info!("agent-router listening on http://{bind} (loopback only)"),
            }
            tracing::info!("  dashboard:    http://{bind}");
            tracing::info!("  upstream:     {}", config.upstream);
            tracing::info!("  database:     {}", db_path.display());
            if status.logged_in {
                tracing::info!(
                    "  default acct: {label}{}",
                    status
                        .subscription_type
                        .as_deref()
                        .map(|s| format!(" · {s}"))
                        .unwrap_or_default()
                );
            } else {
                tracing::warn!("  default acct: not logged in — run `claude` and log in, or add an account in the UI");
            }
        }
    }

    // Ask before binding: after our own bind, a health probe could be answered by
    // this process and we would signal ourselves.
    let incumbent = if server { None } else { lifecycle::incumbent(bind) };

    let (capture, capture_task) = capture::spawn(db_path.clone());
    let state = proxy::AppState::new(config, capture, network.clone())?;
    if let Some(password) = server_password {
        state.auth.pin(&db_path, &password).await?;
        // What is waiting on a person is on the developer machines; their agents say so.
        state.push.use_machines(state.machines.clone());
    }
    // Watches for prompts for the life of the process; a restart announces nothing.
    tokio::spawn(state.push.clone().watch());
    // Kimi Code accounts' windows, read from Kimi Code's usage API every five minutes.
    quota::keep_kimi_readings(state.clone());
    // Wraps every Claude Code extension VSCode adds, when the switch is on. A hosted
    // router has no editor to watch.
    if !server {
        tokio::spawn(shim::watch(db_path.clone(), state.push.clone()));
    }
    // The Library, which a hosted router alone keeps: its first item, the router's own skill,
    // as this build has it, and its sources, polled for a moved ref.
    if server {
        match library::builtin::publish(&db_path) {
            Ok(library::store::Published::New(v)) => tracing::info!(version = v.number, "the Library holds this build's agent-router skill"),
            Ok(library::store::Published::Refused(why)) => tracing::warn!("the router's own skill was not published: {why}"),
            Ok(library::store::Published::Unchanged(_)) => {}
            Err(e) => tracing::warn!("the router's own skill was not published: {e:#}"),
        }
        tokio::spawn(library::sources::poll(state.clone()));
    }
    if wide.is_some() {
        if state.auth.configured().await {
            tracing::info!("  callers from other machines must present the router password");
        } else {
            tracing::warn!("  no password is set: callers from other machines are refused until one is set from the dashboard on this machine");
        }
    }

    let app = app(state.clone());

    if server {
        return serve_hosted(app, state, capture_task, stopping).await;
    }

    // SO_REUSEPORT: the incumbent keeps serving until it has drained, so the port is
    // never unbound and no agent sees a refused connection.
    let listener = lifecycle::bind_reuseport(bind)?;
    let mut departing = None;
    if let Some(pid) = incumbent {
        // Signalled here, but only watched in the background: we must reach the accept
        // loop below immediately, or connections the kernel steers to our socket sit
        // unread while we wait.
        if lifecycle::signal_incumbent(pid) {
            lifecycle::watch_incumbent(pid);
            departing = Some(pid);
        }
    }
    spawn_store_housekeeping(db_path.clone(), false, move || {
        departing.is_none_or(|pid| lifecycle::has_exited_within(pid, Duration::from_secs(60)))
    });
    // This machine's places, every minute, as an agent reports its own to a hosted router:
    // which repository each session works in.
    tokio::spawn(places::keep_here(db_path.clone()));
    tokio::spawn(kimi::live::keep_host());
    tokio::spawn(codex::runtime::keep_host());
    tokio::spawn(keep_local_kimi_config(format!("http://127.0.0.1:{}", loopback_addr().port())));

    // The wider listener, when there is to be one, serves this same app from a second
    // socket. Started last, once loopback is about to be accepted on and the incumbent
    // has been signalled: a socket that has joined a `SO_REUSEPORT` group without
    // reaching its accept loop collects connections nobody reads.
    network::supervise(app.clone(), port, pending);
    if let Err(err) = network.set(wide).await {
        tracing::warn!("{err}; staying on loopback only");
    }

    let mut told = stopping;
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(async move {
        let why = told.wait().await;
        tracing::info!("{why} received; draining in-flight requests");
    })
    .await?;

    // The listener on the network drains on the same signal, in parallel with this
    // one. Wait for it before going any further, or a phone's open stream would be
    // cut by our exit rather than finished. `drain` bounds its own wait.
    network.drain().await;
    finish_capture(state, capture_task).await;
    Ok(())
}

/// Everything a router serves: its routes, the proxy behind them for whatever else
/// arrives, and the guard in front of all of it. One function, so the tests that hold the
/// API to what its clients rely on (`api_refusals`) drive exactly what `serve` does.
fn app(state: Arc<proxy::AppState>) -> Router {
    app_with(state, console::Files::embedded())
}

/// `app`, serving the console's files given here rather than the ones compiled in, so the
/// console's tests drive a build of their own through the very routes and guard `serve` uses.
fn app_with(state: Arc<proxy::AppState>, console: console::Files) -> Router {
    // The console's routes first, on the router both route sets grow from: like every route
    // here they are added before the guard layer, which wraps only what was added before it.
    let common = console::routes(Router::new(), console)
        .route("/api/summary", get(api::summary))
        .route("/api/projects", get(api::projects))
        .route("/api/sessions", get(api::sessions))
        .route("/api/requests", get(api::requests))
        .route("/api/search", get(api::search))
        .route("/api/tools", get(api::tools))
        .route("/api/accounts/activate", post(api::accounts_activate))
        .route("/api/accounts/remove", post(api::accounts_remove))
        .route("/api/accounts/edit", post(api::accounts_edit))
        .route("/api/forecast", get(api::forecast))
        .route("/api/auth", get(auth::api::status).post(auth::api::set))
        .route("/api/auth/tokens", get(auth::tokens::list).post(auth::tokens::create))
        .route("/api/auth/tokens/{id}", axum::routing::delete(auth::tokens::revoke))
        .route("/api/push", get(notify::api::status))
        .route("/api/push/config", post(notify::api::config))
        .route("/api/push/register", post(notify::api::register))
        .route("/api/push/unregister", post(notify::api::unregister))
        .route("/api/push/test", post(notify::api::test))
        .route("/api/quota", get(quota::quota_route))
        .route("/api/quota/raw", get(quota::raw_usage))
        .route("/api/usage-token", post(quota::set_usage_token))
        .route("/api/live/options", get(live::options))
        .route("/api/models", get(models::list))
        .route("/api/clients", get(client_setup::status))
        .route("/api/clients/{client}/preview", post(client_setup::preview))
        .route("/api/clients/{client}/apply", post(client_setup::apply))
        .route("/api/clients/{client}/restore", post(client_setup::restore))
        .route("/api/clients/{client}/check", post(client_setup::check))
        .route("/api/provider-registry", get(provider_registry::status))
        .route("/api/provider-registry/services", post(provider_registry::add_service))
        .route("/api/provider-registry/services/{id}", axum::routing::delete(provider_registry::remove_service))
        .route("/api/provider-registry/models", post(provider_registry::add_model))
        .route("/api/provider-registry/models/{alias}", axum::routing::delete(provider_registry::remove_model))
        .route("/api/models/assign", post(models::assign_route))
        // Answered from the catalogue, never forwarded (`models::v1_models`).
        .route("/v1/models", get(models::v1_models))
        .route("/_router/models", get(models::client_metadata))
        .route("/v1/chat/completions", post(chat_completions::handle))
        .route("/_router/health", get(api::health));
    let routes = if state.config.server { hosted::routes(common) } else { machine_routes(common) };
    routes
        .fallback(any(proxy::handle))
        .layer(axum::middleware::from_fn_with_state(state.clone(), guard::layer))
        // Outside the guard, so its refusals carry it as well.
        .layer(axum::middleware::map_response(console::nosniff))
        .with_state(state)
}

/// The routes that act on the machine the router runs on: its Claude Code settings, its
/// login item, its editor extensions, its listening socket, the `claude` it can run to
/// sign in, and the sessions whose transcripts and sockets are on its disk. A hosted
/// router answers the same paths from `hosted` instead.
fn machine_routes(router: Router<Arc<proxy::AppState>>) -> Router<Arc<proxy::AppState>> {
    router
        .route("/api/accounts", get(api::accounts_list).post(api::accounts_add))
        .route("/api/accounts/probe", post(api::accounts_probe))
        .route("/api/push/pending", get(notify::api::pending))
        .route("/api/login/start", post(login::api::start_login))
        .route("/api/login/status", get(login::api::login_status))
        .route("/api/setup", get(setup::api::status))
        .route("/api/setup/service", post(setup::api::service_toggle))
        .route("/api/setup/config", post(setup::api::config_toggle))
        .route("/api/setup/shim", get(shim::api::status).post(shim::api::toggle))
        .route("/api/setup/shim/auto", post(shim::api::auto))
        .route("/api/network", post(network::api::set))
        .route("/api/live/sessions", get(live::sessions))
        .route("/api/live/stream", get(live::stream))
        .route("/api/live/stats", get(live::stats))
        .route("/api/live/send", post(live::send))
        .route("/api/live/interrupt", post(live::interrupt))
        .route("/api/live/prompt", get(live::prompt))
        .route("/api/live/answer", post(live::answer))
        .route("/api/live/archive", post(live::archive))
        .route("/api/live/project", get(live::project_stats))
        .route("/api/live/model", post(live::set_model_checked))
        .route("/api/live/effort", post(live::set_effort))
        .route("/api/live/mode", post(live::set_mode))
        .route("/api/live/aside", post(live::aside))
        .route("/api/live/start", post(codex::live::start_route))
        .route("/api/live/resume", post(codex::live::resume_route))
        .route("/api/live/compact", post(codex::live::compact_route))
        // A router on a developer's machine keeps no Library, and says so at its addresses
        // rather than passing them to the proxy.
        .route("/api/library", any(library::api::not_offered))
        .route("/api/library/{*rest}", any(library::api::not_offered))
}

/// A hosted router: one listener on its bind address, and the handover a rolling update
/// makes of it. The next pod serves before this one is told to stop; then this one lets
/// its agents go at once (`Machines::stop`), keeps accepting for the stop delay while the
/// load balancer lets go of it, and drains open requests only so long. The kubelet sends
/// SIGKILL when its grace period runs out, and capture still needs a moment after the
/// last connection.
async fn serve_hosted(
    app: Router,
    state: Arc<proxy::AppState>,
    capture_task: tokio::task::JoinHandle<()>,
    stopping: lifecycle::Stopping,
) -> anyhow::Result<()> {
    use std::future::IntoFuture;

    let addr = state.config.bind;
    let drain = state.config.drain;
    let stop_delay = state.config.stop_delay;
    // Every caller is on another machine — behind the ingress, or a probe by pod IP —
    // so the peer and Host checks work from names and the password from the first
    // request, not from a switch thrown later.
    guard::allow_off_host(true);
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| anyhow::anyhow!("binding {addr}: {e}"))?;
    // The previous pod may still be draining on this same store: what must not run beside
    // it waits for the store's lock, which that pod holds until it exits.
    let data_dir = config::data_dir()?;
    spawn_store_housekeeping(state.config.db_path.clone(), true, move || {
        lifecycle::hold_store(&data_dir);
        true
    });

    let mut told = stopping.clone();
    let machines = state.machines.clone();
    let serving = axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(async move {
        let why = told.wait().await;
        machines.stop();
        if !stop_delay.is_zero() {
            tracing::info!(
                "{why} received; the agents are let go, and new requests are still taken for {}s",
                stop_delay.as_secs()
            );
            tokio::time::sleep(stop_delay).await;
        }
        tracing::info!("{why} received; draining in-flight requests for at most {}s", drain.as_secs());
    })
    .into_future();
    let mut limit = stopping;
    let limit = async move {
        limit.wait().await;
        tokio::time::sleep(stop_delay + drain).await;
    };
    tokio::select! {
        served = serving => served?,
        () = limit => tracing::warn!("drain limit reached; closing the connections still open"),
    }
    finish_capture(state, capture_task).await;
    Ok(())
}

/// What a hosted router says about itself at startup: where it listens, the names it
/// answers to, and where its data lives — and that TLS is someone else's job.
fn log_hosted_start(config: &config::Config) {
    tracing::info!("agent-router serving on http://{} as a hosted router", config.bind);
    if let Some(url) = &config.public_url {
        tracing::info!("  public url:   {url}");
    }
    if config.allow_hosts.is_empty() {
        tracing::warn!("  answers to:   IP addresses and localhost only; pass --public-url or --allow-host for the names clients use");
    } else {
        tracing::info!("  answers to:   {} (and IP addresses)", config.allow_hosts.join(", "));
    }
    tracing::info!("  upstream:     {}", config.upstream);
    tracing::info!("  database:     {}", config.db_path.display());
    if let Ok(dir) = config::data_dir() {
        tracing::info!("  data dir:     {}", dir.display());
    }
    tracing::info!(
        "  every caller presents the password from {}; whatever presents these names to clients must terminate TLS",
        config::SERVER_PASSWORD_ENV
    );
}

/// Captured bodies moved out of the main store into their own file. The move waits for
/// the outgoing router to finish draining, because until it exits it is still writing
/// the very table this drops — see `db::migrate_bodies_if_needed`. `alone` is that wait:
/// false when the outgoing router outlasted it. Off the startup path either way: it is
/// housekeeping, and the port is already being served.
///
/// A hosted router then finds the Library's uses in what it stored before it counted them
/// (`library::usage::rebuild`), from the same index.
fn spawn_store_housekeeping(db_path: std::path::PathBuf, hosted: bool, alone: impl FnOnce() -> bool + Send + 'static) {
    tokio::task::spawn_blocking(move || {
        if !alone() {
            tracing::warn!("previous router still running; leaving the bodies where they are");
            return;
        }
        match db::migrate_bodies_if_needed(&db_path) {
            Ok(()) => {}
            // Nothing is lost by failing: the rows stay in the main store and the
            // next start tries again. New bodies already go to the sidecar.
            Err(err) => {
                tracing::warn!(%err, "could not move the captured bodies yet");
                return;
            }
        }
        // Only once every body is in the sidecar, because the backfill reads from
        // there and remembers how far it got: run it over a half-moved store and it
        // would mark the rows still sitting in the main file as already seen.
        if let Err(err) = db::backfill_blocks(&db_path) {
            tracing::warn!(%err, "could not finish indexing the bodies captured before the index");
        }
        if let Err(err) = db::backfill_session_cwds(&db_path) {
            tracing::warn!(%err, "could not give a directory to the sessions recorded without one");
        }
        if hosted {
            if let Err(err) = library::usage::rebuild(&db_path) {
                tracing::warn!(%err, "could not finish finding the Library's uses in the stored conversations");
            }
        }
    });
}

/// Serving has stopped and every connection has finished. Dropping the last AppState
/// closes the capture channel, which is what lets the capture task know it can stop;
/// wait for it so nothing already queued is lost on the way out.
async fn finish_capture(state: Arc<proxy::AppState>, capture_task: tokio::task::JoinHandle<()>) {
    drop(state);
    match tokio::time::timeout(Duration::from_secs(5), capture_task).await {
        Ok(_) => tracing::info!("shutdown complete"),
        Err(_) => tracing::warn!("capture did not drain within 5s; exiting anyway"),
    }
}
