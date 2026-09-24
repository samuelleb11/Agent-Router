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

        let db_path = db_path()?;

        // The bind address defaults to loopback and should stay there unless you mean
        // otherwise. Agent Router holds a live subscription credential and signs
        // whatever it forwards, so a routable bind hands that credential to everyone
        // on the network: there is no authentication on any route, and `proxy` will
        // sign for any caller that presents any credential at all. It is widened
        // deliberately, for following sessions from another device on a network you
        // trust, and then the password in `auth` is what stands in the way.
        //
        // Two ways to say so, in this order: AGENT_ROUTER_BIND, an explicit override
        // for a hand-launch; then the stored setting the dashboard writes, which is
        // the one that survives a reboot — launchd hands a login item an almost empty
        // environment, so an answer that lives only in a variable is an answer that
        // lasts until the next restart. See `network`.
        let bind = match std::env::var("AGENT_ROUTER_BIND") {
            Ok(addr) if !addr.trim().is_empty() => {
                let ip: IpAddr = addr.trim().parse().map_err(|_| {
                    anyhow::anyhow!("AGENT_ROUTER_BIND must be an IP address, got {addr:?}")
                })?;
                SocketAddr::new(ip, port)
            }
            _ => match crate::network::stored(&db_path) {
                Some(ip) => SocketAddr::new(ip, port),
                None => SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port),
            },
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

/// Where the store lives, without building a whole `Config` around it: `install` needs
/// to write a setting into it before there is a router to ask.
pub fn db_path() -> anyhow::Result<PathBuf> {
    match std::env::var("AGENT_ROUTER_DB") {
        Ok(p) => Ok(PathBuf::from(p)),
        Err(_) => Ok(default_data_dir()?.join("agent-router.db")),
    }
}

fn default_data_dir() -> anyhow::Result<PathBuf> {
    let home = std::env::var("HOME").map_err(|_| anyhow::anyhow!("HOME is not set"))?;
    let dir = PathBuf::from(home).join(".agent-router");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}
