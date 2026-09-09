//! Phase 1–2 configuration. Everything here is env-driven; from Phase 3 onward the
//! account list moves into SQLite and the web UI.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;

pub struct Config {
    pub bind: SocketAddr,
    pub upstream: String,
    pub db_path: PathBuf,
    /// Largest body slice persisted to the `bodies` table, in bytes.
    pub body_cap: usize,
    /// How many requests get a full header dump at INFO. Diagnostics only.
    pub dump_headers: usize,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let port: u16 = std::env::var("AGENT_ROUTER_PORT")
            .unwrap_or_else(|_| "8787".into())
            .parse()?;

        // The bind address is deliberately NOT configurable. Agent Router holds a live
        // subscription credential and signs whatever it forwards, so exposing it on a
        // routable interface would hand that credential to the network. Loopback only,
        // with no env var and no flag that can widen it.
        let bind = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port);

        let db_path = match std::env::var("AGENT_ROUTER_DB") {
            Ok(p) => PathBuf::from(p),
            Err(_) => default_data_dir()?.join("agent-router.db"),
        };

        Ok(Self {
            bind,
            upstream: std::env::var("AGENT_ROUTER_UPSTREAM")
                .unwrap_or_else(|_| "https://api.anthropic.com".into()),
            db_path,
            body_cap: std::env::var("AGENT_ROUTER_BODY_CAP")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(64 * 1024),
            dump_headers: std::env::var("AGENT_ROUTER_DUMP_HEADERS")
                .unwrap_or_else(|_| "1".into())
                .parse()
                .unwrap_or(1),
        })
    }
}

fn default_data_dir() -> anyhow::Result<PathBuf> {
    let home = std::env::var("HOME").map_err(|_| anyhow::anyhow!("HOME is not set"))?;
    let dir = PathBuf::from(home).join(".agent-router");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}
