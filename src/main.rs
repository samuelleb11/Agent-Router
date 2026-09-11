//! agent-router — a local proxy between Claude Code and Anthropic.
//!
//! Phase 1: one endpoint, one credential, byte-faithful streaming passthrough.
//! Capture, the transcript join, multi-account switching and the web UI follow.
//! See ARCHITECTURE.md.

mod accounts;
mod analyze;
mod api;
mod capture;
mod config;
mod db;
mod guard;
mod lifecycle;
mod live;
mod peer;
mod login;
mod proxy;
mod quota;
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
  agent-router install [OPTIONS]     point Claude Code at it, and start it at login
  agent-router uninstall             undo exactly what install changed
  agent-router status                show what is installed and what is managed

INSTALL OPTIONS:
  --project <dir>   configure one project instead of every project
  --no-service      only edit the config; do not register a login item
  --port <n>        router port (default 8787)

Only one settings key is ever touched: env.ANTHROPIC_BASE_URL. Everything else in
your settings file is left exactly as it is, and uninstall restores whatever was
there before — unless you have since changed it yourself, in which case it is left
alone.
";

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
        Some("--if-not-running") => {
            // Something already serving means our job is done. Used by the shell
            // profile autostart, which runs on every new shell.
            let port: u16 = std::env::var("AGENT_ROUTER_PORT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(8787);
            let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
            if lifecycle::incumbent(addr).is_some() {
                return Ok(());
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

    if !args.iter().any(|a| a == "--no-service") {
        let exe = setup::install_binary()?;
        match setup::service::install(&exe) {
            Ok(path) => println!("service   registered: {}", path.display()),
            Err(err) => println!("service   skipped: {err}"),
        }
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
    let bind = config.bind;
    // Layers 2 and 3 must know before the first request arrives.
    guard::allow_off_host(!bind.ip().is_loopback());
    let db_path = config.db_path.clone();

    // Create the schema and make sure there is always something to route as: the
    // account Claude Code is already logged into, discovered rather than configured.
    {
        let db = db::Db::open(&db_path)?;
        let status = accounts::probe_profile(None).unwrap_or_default();
        let label = status
            .email
            .clone()
            .unwrap_or_else(|| "Claude Code (default profile)".to_string());
        db.upsert_default_account(proxy::ACCOUNT_ID, &label, &status)?;
        if bind.ip().is_loopback() {
            tracing::info!("agent-router listening on http://{bind} (loopback only)");
        } else {
            tracing::warn!("agent-router listening on http://{bind} — REACHABLE FROM THE NETWORK");
            tracing::warn!("  no route requires authentication, and the proxy signs with your credential");
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
    let state = proxy::AppState::new(config, capture)?;

    let app = Router::new()
        .route("/", get(api::index))
        .route("/favicon.svg", get(api::favicon))
        .route("/api/summary", get(api::summary))
        .route("/api/projects", get(api::projects))
        .route("/api/sessions", get(api::sessions))
        .route("/api/requests", get(api::requests))
        .route("/api/tools", get(api::tools))
        .route("/api/accounts", get(api::accounts_list).post(api::accounts_add))
        .route("/api/accounts/activate", post(api::accounts_activate))
        .route("/api/accounts/probe", post(api::accounts_probe))
        .route("/api/accounts/remove", post(api::accounts_remove))
        .route("/api/accounts/edit", post(api::accounts_edit))
        .route("/api/quota", get(quota::quota))
        .route("/api/quota/raw", get(quota::raw_usage))
        .route("/api/usage-token", post(quota::set_usage_token))
        .route("/api/login/start", post(login::api::start_login))
        .route("/api/login/status", get(login::api::login_status))
        .route("/api/setup", get(setup::api::status))
        .route("/api/setup/service", post(setup::api::service_toggle))
        .route("/api/setup/config", post(setup::api::config_toggle))
        .route("/api/setup/shim", get(shim::api::status).post(shim::api::toggle))
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
        .route("/_router/health", get(api::health))
        .fallback(any(proxy::handle))
        .layer(axum::middleware::from_fn(guard::layer))
        .with_state(state.clone());

    // SO_REUSEPORT: the incumbent keeps serving until it has drained, so the port is
    // never unbound and no agent sees a refused connection.
    let listener = lifecycle::bind_reuseport(bind)?;
    if let Some(pid) = incumbent {
        // Signalled here, but only watched in the background: we must reach the accept
        // loop below immediately, or connections the kernel steers to our socket sit
        // unread while we wait.
        if lifecycle::signal_incumbent(pid) {
            lifecycle::watch_incumbent(pid);
        }
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
