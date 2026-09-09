//! Reads Claude Code's own OAuth credentials from the macOS Keychain.
//!
//! Phase 1 borrows the credential Claude Code already stored so the proxy has
//! something to inject. Phase 3 replaces this with router-owned accounts: its own
//! PKCE flow, its own Keychain entries, and its own refresh. See ARCHITECTURE.md —
//! once the router owns an account it must be the sole refresher.
//!
//! This module is READ-ONLY. It never writes to Claude Code's Keychain entry.

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;

const SERVICE: &str = "Claude Code-credentials";

#[derive(Debug, Deserialize)]
struct Stored {
    #[serde(rename = "claudeAiOauth")]
    oauth: OauthEntry,
}

#[derive(Debug, Deserialize)]
struct OauthEntry {
    #[serde(rename = "accessToken")]
    access_token: String,
    /// Milliseconds since the Unix epoch.
    #[serde(rename = "expiresAt")]
    expires_at: i64,
    #[serde(default)]
    scopes: Vec<String>,
    #[serde(rename = "subscriptionType", default)]
    subscription_type: String,
}

pub struct Credential {
    pub access_token: String,
    pub expires_at_ms: i64,
    pub scopes: Vec<String>,
    pub subscription_type: String,
}

impl Credential {
    pub fn expires_in_secs(&self) -> i64 {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        (self.expires_at_ms - now) / 1000
    }

    /// The token's family prefix (`sk-ant-oat01`), safe to log. Never log the token.
    pub fn kind(&self) -> &str {
        token_kind(&self.access_token)
    }
}

/// Classify a bearer token by its prefix without revealing it.
pub fn token_kind(token: &str) -> &str {
    let t = token.strip_prefix("Bearer ").unwrap_or(token).trim();
    match t.split('-').take(3).collect::<Vec<_>>().join("-").as_str() {
        "sk-ant-oat01" => "sk-ant-oat01 (subscription OAuth)",
        "sk-ant-ort01" => "sk-ant-ort01 (OAuth refresh)",
        "sk-ant-api03" => "sk-ant-api03 (API key)",
        _ if t.is_empty() => "<empty>",
        _ => "<unrecognised>",
    }
}

/// Prefer an explicitly supplied token, else fall back to Claude Code's Keychain entry.
pub fn load() -> Result<Credential> {
    if let Ok(token) = std::env::var("AGENT_ROUTER_ACCESS_TOKEN") {
        if !token.is_empty() {
            return Ok(Credential {
                access_token: token,
                expires_at_ms: 0,
                scopes: vec![],
                subscription_type: "env".into(),
            });
        }
    }
    from_keychain()
}

fn from_keychain() -> Result<Credential> {
    let out = std::process::Command::new("security")
        .args(["find-generic-password", "-s", SERVICE, "-w"])
        .output()
        .context("running `security find-generic-password`")?;

    if !out.status.success() {
        return Err(anyhow!(
            "no Keychain entry {SERVICE:?} — run `claude` and log in, or set AGENT_ROUTER_ACCESS_TOKEN"
        ));
    }

    let raw = String::from_utf8(out.stdout).context("Keychain payload was not UTF-8")?;
    let stored: Stored =
        serde_json::from_str(raw.trim()).context("Keychain payload was not the expected JSON")?;

    Ok(Credential {
        access_token: stored.oauth.access_token,
        expires_at_ms: stored.oauth.expires_at,
        scopes: stored.oauth.scopes,
        subscription_type: stored.oauth.subscription_type,
    })
}
