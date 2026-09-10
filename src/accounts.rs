//! The account registry: what the router can route as, and which one is live.
//!
//! Three sources, because Claude Code's login is a terminal program and the router
//! cannot drive it from a browser:
//!
//! * `keychain` — the default Claude Code profile. Zero setup, and Claude Code keeps
//!   it refreshed; the router just reads it. This is the account you are already
//!   logged into.
//! * `token` — a long-lived token from `claude setup-token`, held in the router's own
//!   Keychain entry. The robust choice for additional accounts: nothing to refresh,
//!   and no contention with any running Claude Code.
//! * `profile` — an isolated `CLAUDE_CONFIG_DIR` you ran `claude auth login` in. The
//!   router reads that profile's credentials. Convenient, but the token is short-lived
//!   and only Claude Code can renew it, so the UI shows its expiry.
//!
//! Isolation is real: a fresh `CLAUDE_CONFIG_DIR` reports "not logged in" even with
//! the default profile's `.claude.json` copied in, so adding an account here cannot
//! disturb the account you use day to day.

use anyhow::{anyhow, Context, Result};
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Keychain service the router stores its *own* secrets under.
const ROUTER_SERVICE: &str = "agent-router";
/// Keychain service for the optional claude.ai web session key an account may carry
/// for reading usage. Kept separate from the inference credential: different scope,
/// different lifetime, and losing one should not invalidate the other.
const USAGE_SERVICE: &str = "agent-router-usage";
/// Keychain service Claude Code stores the default profile's credentials under.
const CLAUDE_SERVICE: &str = "Claude Code-credentials";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Keychain,
    Token,
    Profile,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Source::Keychain => "keychain",
            Source::Token => "token",
            Source::Profile => "profile",
        }
    }
    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "keychain" => Ok(Source::Keychain),
            "token" => Ok(Source::Token),
            "profile" => Ok(Source::Profile),
            other => Err(anyhow!("unknown account source {other:?}")),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Account {
    pub id: String,
    pub label: String,
    pub source: Source,
    pub profile_dir: Option<String>,
    /// The identity this account was recorded as. For a Keychain account this is
    /// checked before signing: `/login` rewrites that Keychain entry in place, so the
    /// credential there is only this account's while the logged-in identity still
    /// matches.
    pub email: Option<String>,
}

/// A credential ready to sign with.
pub struct Resolved {
    pub token: String,
    /// Who the credential actually belongs to, where that is knowable.
    pub identity: Option<String>,
    /// Milliseconds since the Unix epoch, or 0 when the token has no known expiry.
    pub expires_at_ms: i64,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// The shape Claude Code stores, in the Keychain or in a profile's credentials file.
#[derive(Debug, Deserialize)]
struct StoredCredentials {
    #[serde(rename = "claudeAiOauth")]
    oauth: OauthEntry,
}

#[derive(Debug, Deserialize)]
struct OauthEntry {
    #[serde(rename = "accessToken")]
    access_token: String,
    #[serde(rename = "expiresAt", default)]
    expires_at: i64,
    #[serde(default)]
    scopes: Vec<String>,
    #[serde(rename = "subscriptionType", default)]
    subscription_type: String,
}

/// What `claude auth status --json` reports for a profile.
#[derive(Debug, Deserialize, Serialize, Default)]
pub struct AuthStatus {
    #[serde(rename = "loggedIn", default)]
    pub logged_in: bool,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(rename = "orgName", default)]
    pub org_name: Option<String>,
    #[serde(rename = "subscriptionType", default)]
    pub subscription_type: Option<String>,
    #[serde(rename = "authMethod", default)]
    pub auth_method: Option<String>,
}

/// Ask Claude Code who a profile is logged in as; `None` probes the default profile.
/// Blocking; call from a blocking task.
pub fn probe_profile(dir: Option<&Path>) -> Result<AuthStatus> {
    let mut cmd = std::process::Command::new("claude");
    cmd.args(["auth", "status", "--json"]);
    if let Some(dir) = dir {
        cmd.env("CLAUDE_CONFIG_DIR", dir);
    }
    let out = cmd
        .output()
        .context("running `claude auth status` (is the claude CLI on PATH?)")?;
    let text = String::from_utf8_lossy(&out.stdout);
    // The CLI prints advisory lines before the JSON when a config file is missing.
    let start = text.find('{').ok_or_else(|| anyhow!("no JSON in `claude auth status` output"))?;
    Ok(serde_json::from_str(&text[start..])?)
}

/// Resolve an account to a token to sign with. Blocking; call from a blocking task.
pub fn resolve(account: &Account) -> Result<Resolved> {
    match account.source {
        Source::Keychain => {
            if let Ok(token) = std::env::var("AGENT_ROUTER_ACCESS_TOKEN") {
                if !token.is_empty() {
                    return Ok(Resolved { token, identity: None, expires_at_ms: 0 });
                }
            }
            // `claude /login` overwrites this one Keychain entry in place, so this
            // account means "whoever Claude Code is logged into now" — deliberately
            // dynamic. The identity is therefore resolved per request and stamped on
            // the stored row, so a later /login cannot retroactively relabel traffic
            // that a different account actually served.
            let live = probe_profile(None).ok().and_then(|s| s.email);
            let raw = keychain_read(CLAUDE_SERVICE, &current_user())
                .context("reading Claude Code's Keychain entry — log in with `claude` first")?;
            let mut resolved = parse_stored(&raw)?;
            resolved.identity = live;
            Ok(resolved)
        }
        Source::Token => {
            let token = keychain_read(ROUTER_SERVICE, &account.id)
                .with_context(|| format!("reading the stored token for account {}", account.id))?;
            // setup-token issues long-lived tokens; there is no expiry to track.
            Ok(Resolved {
                token: token.trim().to_string(),
                identity: account.email.clone(),
                expires_at_ms: 0,
            })
        }
        Source::Profile => {
            let dir = account
                .profile_dir
                .as_ref()
                .ok_or_else(|| anyhow!("account {} has no profile directory", account.id))?;
            let path = PathBuf::from(dir).join(".credentials.json");
            let raw = std::fs::read_to_string(&path).with_context(|| {
                format!(
                    "reading {} — run `CLAUDE_CONFIG_DIR={dir} claude auth login` first",
                    path.display()
                )
            })?;
            let mut resolved = parse_stored(&raw)?;
            resolved.identity = probe_profile(Some(std::path::Path::new(dir)))
                .ok()
                .and_then(|s| s.email)
                .or_else(|| account.email.clone());
            Ok(resolved)
        }
    }
}

fn parse_stored(raw: &str) -> Result<Resolved> {
    let stored: StoredCredentials =
        serde_json::from_str(raw.trim()).context("credentials were not in the expected shape")?;
    let _ = (&stored.oauth.scopes, &stored.oauth.subscription_type);
    Ok(Resolved {
        token: stored.oauth.access_token,
        identity: None,
        expires_at_ms: stored.oauth.expires_at,
    })
}

pub fn store_token(account_id: &str, token: &str) -> Result<()> {
    keychain_write(ROUTER_SERVICE, account_id, token.trim())
}

/// A claude.ai session key (`sk-ant-sid…`), used only to read usage. It is never
/// sent to api.anthropic.com and never used to sign a proxied request.
pub fn store_usage_token(account_id: &str, token: &str) -> Result<()> {
    keychain_write(USAGE_SERVICE, account_id, token.trim())
}

pub fn usage_token(account_id: &str) -> Option<String> {
    keychain_read(USAGE_SERVICE, account_id).ok().map(|t| t.trim().to_string())
}

pub fn forget_usage_token(account_id: &str) -> Result<()> {
    let out = security(&["delete-generic-password", "-s", USAGE_SERVICE, "-a", account_id], None)?;
    if out.status.success() || out.status.code() == Some(44) {
        return Ok(());
    }
    Ok(())
}

pub fn forget_token(account_id: &str) -> Result<()> {
    let out = security(&["delete-generic-password", "-s", ROUTER_SERVICE, "-a", account_id], None)?;
    // Exit status 44 is errSecItemNotFound: already gone is the outcome we wanted.
    if out.status.success() || out.status.code() == Some(44) {
        return Ok(());
    }
    // A leftover entry written by an earlier build through the Security framework.
    let entry = keyring::Entry::new(ROUTER_SERVICE, account_id)?;
    match entry.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e.into()),
    }
}

// Why the Keychain is driven through /usr/bin/security rather than the Security
// framework: the Keychain identifies an application by its code signature, and a
// locally built binary is ad-hoc signed with a per-build hash. Every `cargo build`
// is therefore a brand-new application as far as the ACL is concerned, "Always
// Allow" whitelists only the build that asked, and the prompt comes back after the
// next rebuild — and a running process whose binary was rebuilt underneath it cannot
// be validated at all, so no button makes that prompt go away. `security` is
// Apple-signed, so its identity never changes, and it is already trusted on Claude
// Code's own entry because Claude Code reads and writes it the same way. Going
// through it means the router never prompts, on any build.

/// Run `security` with the given arguments, optionally feeding an interactive-mode
/// command line on stdin so a secret never appears in the process list.
fn security(args: &[&str], stdin_line: Option<&str>) -> Result<std::process::Output> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let mut cmd = Command::new("/usr/bin/security");
    cmd.args(args)
        .stdin(if stdin_line.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().context("running /usr/bin/security")?;
    if let Some(line) = stdin_line {
        let mut stdin = child.stdin.take().context("opening security's stdin")?;
        stdin.write_all(line.as_bytes())?;
        stdin.write_all(b"\n")?;
        drop(stdin);
    }
    Ok(child.wait_with_output()?)
}

fn keychain_read(service: &str, account: &str) -> Result<String> {
    // `-w` prints only the secret, to stdout, followed by a newline.
    let out = security(&["find-generic-password", "-s", service, "-a", account, "-w"], None)?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        anyhow::bail!(
            "no Keychain entry for {service} / {account}: {}",
            err.trim().trim_start_matches("security: ")
        );
    }
    let text = String::from_utf8(out.stdout).context("Keychain entry was not UTF-8 text")?;
    Ok(text.trim_end_matches(['\n', '\r']).to_string())
}

fn keychain_write(service: &str, account: &str, secret: &str) -> Result<()> {
    // Interactive mode reads the command line from stdin, so the secret is never a
    // process argument. The parser splits on whitespace and honours double quotes;
    // a token that could confuse it goes through the Security framework instead,
    // where "Always Allow" is per build — but the item is at least written.
    let simple = !secret.is_empty()
        && secret
            .chars()
            .all(|c| c.is_ascii_graphic() && c != '"' && c != '\\');
    if !simple {
        return Ok(keyring::Entry::new(service, account)?.set_password(secret)?);
    }
    let line = format!(
        "add-generic-password -U -s \"{service}\" -a \"{account}\" -w \"{secret}\" -T /usr/bin/security"
    );
    let out = security(&["-i"], Some(&line))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        anyhow::bail!("storing the token in the Keychain failed: {}", err.trim());
    }
    Ok(())
}

fn current_user() -> String {
    std::env::var("USER").unwrap_or_else(|_| "unknown".into())
}

/// Account rows, read and written through their own short-lived connection. The
/// capture task owns the writer for traffic; account edits are rare and go direct.
pub struct Store {
    path: PathBuf,
}

impl Store {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    fn conn(&self) -> Result<rusqlite::Connection> {
        Ok(rusqlite::Connection::open(&self.path)?)
    }

    pub fn active(&self) -> Result<Account> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, label, source, profile_dir, email FROM accounts
             WHERE active = 1 ORDER BY created_at LIMIT 1",
        )?;
        let account = stmt
            .query_row([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, Option<String>>(4)?,
                ))
            })
            .optional()?;
        let (id, label, source, profile_dir, email) =
            account.ok_or_else(|| anyhow!("no active account"))?;
        Ok(Account {
            id,
            label,
            source: Source::parse(source.as_deref().unwrap_or("keychain"))?,
            profile_dir,
            email,
        })
    }

    pub fn add(
        &self,
        id: &str,
        label: &str,
        source: Source,
        profile_dir: Option<&str>,
        status: &AuthStatus,
    ) -> Result<()> {
        self.conn()?.execute(
            "INSERT INTO accounts (id, label, source, profile_dir, email, org_name,
                                   subscription_type, active, created_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,0,?8)
             ON CONFLICT(id) DO UPDATE SET
               label = ?2, source = ?3, profile_dir = ?4,
               email = COALESCE(?5, email), org_name = COALESCE(?6, org_name),
               subscription_type = COALESCE(?7, subscription_type)",
            rusqlite::params![
                id,
                label,
                source.as_str(),
                profile_dir,
                status.email,
                status.org_name,
                status.subscription_type,
                now_ms()
            ],
        )?;
        Ok(())
    }

    /// Exactly one account is active; switching is a single transaction.
    pub fn activate(&self, id: &str) -> Result<()> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let n = tx.execute("UPDATE accounts SET active = 0 WHERE active = 1", [])?;
        let _ = n;
        let hit = tx.execute("UPDATE accounts SET active = 1 WHERE id = ?1", [id])?;
        if hit == 0 {
            return Err(anyhow!("no account with id {id:?}"));
        }
        tx.commit()?;
        Ok(())
    }

    pub fn remove(&self, id: &str) -> Result<()> {
        let conn = self.conn()?;
        let was_active: bool = conn
            .query_row("SELECT active FROM accounts WHERE id = ?1", [id], |r| {
                r.get::<_, i64>(0)
            })
            .optional()?
            .map(|v| v == 1)
            .unwrap_or(false);
        conn.execute("DELETE FROM accounts WHERE id = ?1", [id])?;
        let _ = forget_token(id);
        let _ = forget_usage_token(id);
        if was_active {
            // Never leave the router with nothing to sign with.
            conn.execute(
                "UPDATE accounts SET active = 1 WHERE id =
                   (SELECT id FROM accounts ORDER BY created_at LIMIT 1)",
                [],
            )?;
        }
        Ok(())
    }
}

/// Where the router keeps the isolated profile directories it creates.
pub fn profiles_root() -> Result<PathBuf> {
    let home = std::env::var("HOME").map_err(|_| anyhow!("HOME is not set"))?;
    let dir = PathBuf::from(home).join(".agent-router").join("accounts");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Turn a user-supplied label into a safe single path segment.
pub fn slugify(label: &str) -> String {
    let s: String = label
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect();
    let s = s.trim_matches('-').to_string();
    let s: String = s
        .split('-')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if s.is_empty() {
        format!("account-{}", now_ms())
    } else {
        s.chars().take(48).collect()
    }
}

/// The token's family prefix, safe to log. Never log the token itself.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_are_safe_single_path_segments() {
        assert_eq!(slugify("Work Account"), "work-account");
        assert_eq!(slugify("../../etc/passwd"), "etc-passwd");
        assert!(slugify("  ~!@#  ").starts_with("account-"));
        assert!(!slugify("a/b").contains('/'));
        assert!(slugify("").starts_with("account-"));
    }

    #[test]
    fn token_kinds_never_echo_the_secret() {
        let t = "sk-ant-oat01-SECRETSECRET";
        assert_eq!(token_kind(t), "sk-ant-oat01 (subscription OAuth)");
        assert!(!token_kind(t).contains("SECRET"));
        assert_eq!(token_kind("Bearer sk-ant-api03-x"), "sk-ant-api03 (API key)");
    }
}
