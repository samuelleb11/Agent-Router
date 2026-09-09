//! Phase 1 configuration. Everything here is env-driven; from Phase 3 onward the
//! account list moves into SQLite and the web UI, and only the bind address stays.

use std::net::SocketAddr;

pub struct Config {
    pub bind: SocketAddr,
    pub upstream: String,
    /// How many requests get a full header dump at INFO. Phase 1 diagnostics.
    pub dump_headers: usize,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let port: u16 = std::env::var("AGENT_ROUTER_PORT")
            .unwrap_or_else(|_| "8787".into())
            .parse()?;
        Ok(Self {
            bind: SocketAddr::from(([127, 0, 0, 1], port)),
            upstream: std::env::var("AGENT_ROUTER_UPSTREAM")
                .unwrap_or_else(|_| "https://api.anthropic.com".into()),
            dump_headers: std::env::var("AGENT_ROUTER_DUMP_HEADERS")
                .unwrap_or_else(|_| "3".into())
                .parse()
                .unwrap_or(3),
        })
    }
}
