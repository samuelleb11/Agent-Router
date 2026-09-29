//! agent-router — a local proxy between Claude Code and Anthropic.
//!
//! Phase 1: one endpoint, one credential, byte-faithful streaming passthrough.
//! Capture, the transcript join, multi-account switching and the web UI follow.
//! See ARCHITECTURE.md.

mod accounts;
mod activity;
mod analyze;
mod api;
mod apns;
mod auth;
mod capture;
mod config;
mod db;
mod fcm;
mod fold;
mod forecast;
mod guard;
mod lifecycle;
mod live;
mod network;
mod peer;
mod pricing;
mod login;
mod notify;
mod proxy;
mod quota;
mod registry;
mod setup;
mod shim;
mod transcript;

use std::time::Duration;

use axum::routing::{any, get, post};
use axum::Router;

const USAGE: &str = "\
agent-router — a local proxy between Claude Code and Anthropic

USAGE:
  agent-router                       run the router (default)
  agent-router --ensure              run only if no router is serving, or an older
                                     build is — then take the port over without a gap
  agent-router --if-not-running      run only if no router is serving
  agent-router --version             print the version
  agent-router install [OPTIONS]     point Claude Code at it, and start it at login
  agent-router uninstall             undo exactly what install changed
  agent-router status                show what is installed and what is managed

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
    match args.first().map(String::as_str) {
        Some("install") => return cmd_install(&args[1..]),
        Some("uninstall") => return cmd_uninstall(),
        Some("status") => return cmd_status(),
        Some("-h") | Some("--help") | Some("help") => {
            print!("{USAGE}");
            return Ok(());
        }
        Some("--version") | Some("-V") => {
            println!("agent-router {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        Some("--if-not-running") => {
            // Something already serving means our job is done. Used by the shell
            // profile autostart, which runs on every new shell.
            if lifecycle::incumbent(loopback_addr()).is_some() {
                return Ok(());
            }
            return serve().await;
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
            return serve().await;
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
    serve().await
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
    println!("\ndashboard {url}");
    Ok(())
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

async fn serve() -> anyhow::Result<()> {
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

    let config = config::Config::from_env()?;
    // Loopback is always served, and a wider bind adds a second listener beside it
    // rather than moving this one — see `network`. It keeps every local tool working
    // whatever the switch is set to, and it keeps the handover probe honest: it has to
    // reach the incumbent on *this* machine.
    let port = config.bind.port();
    let bind = loopback_addr_on(port);
    let wide = (!config.bind.ip().is_loopback()).then(|| config.bind.ip());
    // Built before the app it will serve, because the app is built from the state that
    // holds it. The supervisor takes the other half once the app exists.
    let (network, pending) = network::switch();
    let db_path = config.db_path.clone();

    // Create the schema and make sure there is always something to route as: the
    // account Claude Code is already logged into, discovered rather than configured.
    {
        let db = db::Db::open(&db_path)?;
        {
            // Rates live in the database so they can be corrected without a rebuild.
            let conn = rusqlite::Connection::open(&db_path)?;
            pricing::ensure_table(&conn)?;
        }
        let status = accounts::probe_profile(None).unwrap_or_default();
        let label = status
            .email
            .clone()
            .unwrap_or_else(|| "Claude Code (default profile)".to_string());
        db.upsert_default_account(proxy::ACCOUNT_ID, &label, &status)?;
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

    // Ask before binding: after our own bind, a health probe could be answered by
    // this process and we would signal ourselves.
    let incumbent = lifecycle::incumbent(bind);

    let (capture, capture_task) = capture::spawn(db_path.clone());
    let state = proxy::AppState::new(config, capture, network.clone())?;
    // Watches for prompts for the life of the process; a restart announces nothing.
    tokio::spawn(state.push.clone().watch());
    // Wraps every Claude Code extension VSCode adds, when the switch is on.
    tokio::spawn(shim::watch(db_path.clone(), state.push.clone()));
    if wide.is_some() {
        if state.auth.configured().await {
            tracing::info!("  callers from other machines must present the router password");
        } else {
            tracing::warn!("  no password is set: callers from other machines are refused until one is set from the dashboard on this machine");
        }
    }

    let app = Router::new()
        .route("/", get(api::index))
        .route("/favicon.svg", get(api::favicon))
        .route("/api/summary", get(api::summary))
        .route("/api/projects", get(api::projects))
        .route("/api/sessions", get(api::sessions))
        .route("/api/requests", get(api::requests))
        .route("/api/search", get(api::search))
        .route("/api/tools", get(api::tools))
        .route("/api/accounts", get(api::accounts_list).post(api::accounts_add))
        .route("/api/accounts/activate", post(api::accounts_activate))
        .route("/api/accounts/probe", post(api::accounts_probe))
        .route("/api/accounts/remove", post(api::accounts_remove))
        .route("/api/accounts/edit", post(api::accounts_edit))
        .route("/api/forecast", get(api::forecast))
        .route("/api/auth", get(auth::api::status).post(auth::api::set))
        .route("/api/push", get(notify::api::status))
        .route("/api/push/config", post(notify::api::config))
        .route("/api/push/register", post(notify::api::register))
        .route("/api/push/unregister", post(notify::api::unregister))
        .route("/api/push/test", post(notify::api::test))
        .route("/api/push/pending", get(notify::api::pending))
        .route("/api/quota", get(quota::quota))
        .route("/api/quota/raw", get(quota::raw_usage))
        .route("/api/usage-token", post(quota::set_usage_token))
        .route("/api/login/start", post(login::api::start_login))
        .route("/api/login/status", get(login::api::login_status))
        .route("/api/setup", get(setup::api::status))
        .route("/api/setup/service", post(setup::api::service_toggle))
        .route("/api/setup/config", post(setup::api::config_toggle))
        .route("/api/setup/shim", get(shim::api::status).post(shim::api::toggle))
        .route("/api/setup/shim/auto", post(shim::api::auto))
        .route("/api/network", post(network::api::set))
        .route("/live", get(live::page))
        .route("/api/live/sessions", get(live::sessions))
        .route("/api/live/stream", get(live::stream))
        .route("/api/live/stats", get(live::stats))
        .route("/api/live/send", post(live::send))
        .route("/api/live/interrupt", post(live::interrupt))
        .route("/api/live/prompt", get(live::prompt))
        .route("/api/live/answer", post(live::answer))
        .route("/api/live/archive", post(live::archive))
        .route("/api/live/project", get(live::project_stats))
        .route("/api/live/options", get(live::options))
        .route("/api/live/model", post(live::set_model))
        .route("/api/live/effort", post(live::set_effort))
        .route("/api/live/mode", post(live::set_mode))
        .route("/api/live/aside", post(live::aside))
        .route("/_router/health", get(api::health))
        .fallback(any(proxy::handle))
        .layer(axum::middleware::from_fn_with_state(state.clone(), guard::layer))
        .with_state(state.clone());

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

    // Captured bodies moved out of the main store into their own file. The move waits
    // for the outgoing router to finish draining, because until it exits it is still
    // writing the very table this drops — see `db::migrate_bodies_if_needed`. Off the
    // startup path either way: it is housekeeping, and the port is already being served.
    {
        let db_path = db_path.clone();
        tokio::task::spawn_blocking(move || {
            if let Some(pid) = departing {
                if !lifecycle::has_exited_within(pid, Duration::from_secs(60)) {
                    tracing::warn!(pid, "previous router still running; leaving the bodies where they are");
                    return;
                }
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
        });
    }

    // The wider listener, when there is to be one, serves this same app from a second
    // socket. Started last, once loopback is about to be accepted on and the incumbent
    // has been signalled: a socket that has joined a `SO_REUSEPORT` group without
    // reaching its accept loop collects connections nobody reads.
    network::supervise(app.clone(), port, pending);
    if let Err(err) = network.set(wide).await {
        tracing::warn!("{err}; staying on loopback only");
    }

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(async {
        let why = lifecycle::shutdown_signal().await;
        tracing::info!("{why} received; draining in-flight requests");
    })
    .await?;

    // The listener on the network drains on the same signal, in parallel with this
    // one. Wait for it before going any further, or a phone's open stream would be
    // cut by our exit rather than finished. `drain` bounds its own wait.
    network.drain().await;

    // Serving has stopped and every connection has finished. Dropping the last
    // AppState closes the capture channel, which is what lets the capture task know
    // it can stop; wait for it so nothing already queued is lost on the way out.
    drop(state);
    match tokio::time::timeout(Duration::from_secs(5), capture_task).await {
        Ok(_) => tracing::info!("shutdown complete"),
        Err(_) => tracing::warn!("capture did not drain within 5s; exiting anyway"),
    }
    Ok(())
}
