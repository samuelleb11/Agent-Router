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
mod proxy;
mod quota;
mod transcript;

use std::time::Duration;

use axum::routing::{any, get, post};
use axum::Router;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
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
        tracing::info!("agent-router listening on http://{bind} (loopback only)");
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
