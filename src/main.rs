//! agent-router — a local proxy between Claude Code and Anthropic.
//!
//! Phase 1: one endpoint, one credential, byte-faithful streaming passthrough.
//! Capture, the transcript join, multi-account switching and the web UI follow.
//! See ARCHITECTURE.md.

mod analyze;
mod capture;
mod config;
mod db;
mod credentials;
mod guard;
mod proxy;
mod transcript;

use anyhow::Context;
use axum::routing::{any, get};
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
    let credential = credentials::load().context("loading a credential to route with")?;

    let bind = config.bind;
    let upstream = config.upstream.clone();
    tracing::info!("agent-router listening on http://{bind} (loopback only)");
    tracing::info!("  upstream:     {upstream}");
    tracing::info!("  credential:   {}", credential.kind());
    if credential.expires_at_ms > 0 {
        let mins = credential.expires_in_secs() / 60;
        tracing::info!("  subscription: {}", credential.subscription_type);
        tracing::info!("  scopes:       {}", credential.scopes.join(", "));
        if mins <= 0 {
            tracing::warn!("  expires:      EXPIRED {} min ago — expect 401s", -mins);
        } else {
            tracing::info!("  expires:      in {mins} min");
        }
    }
    tracing::info!("point a project at it with .claude/settings.local.json:");
    tracing::info!("  {{ \"env\": {{ \"ANTHROPIC_BASE_URL\": \"http://{bind}\" }} }}");

    let capture = capture::spawn(
        config.db_path.clone(),
        proxy::ACCOUNT_ID.to_string(),
        "default (Claude Code Keychain)".to_string(),
        credential.subscription_type.clone(),
    );
    let state = proxy::AppState::new(config, credential, capture)?;
    let app = Router::new()
        .route("/_router/health", get(|| async { "ok" }))
        .fallback(any(proxy::handle))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .with_context(|| format!("binding {bind} (is another agent-router running?)"))?;

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
        .with_graceful_shutdown(async {
            tokio::signal::ctrl_c().await.ok();
            tracing::info!("shutting down");
        })
        .await?;
    Ok(())
}
