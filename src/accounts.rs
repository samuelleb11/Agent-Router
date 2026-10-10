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

use anyhow::{anyhow, bail, Context, Result};
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Keychain service the router stores its *own* secrets under.
/// Keychain service Claude Code stores the default profile's credentials under (macOS).
#[cfg(target_os = "macos")]
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

/// The service an account belongs to. It fixes where the account's requests go, how they
/// are signed, how its quota is read and what a spent limit looks like
/// (docs/sources-and-models.md). Where the credential is kept is `Source`, which says
/// nothing about this: a Kimi Code key is kept as a setup-token is, `Source::Token`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Anthropic,
    /// Kimi Code, Moonshot's coding subscription, through its Anthropic-compatible endpoint.
    Kimi,
}

impl Provider {
    pub fn as_str(self) -> &'static str {
        match self {
            Provider::Anthropic => "anthropic",
            Provider::Kimi => "kimi",
        }
    }

    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "anthropic" => Ok(Provider::Anthropic),
            "kimi" => Ok(Provider::Kimi),
            other => Err(anyhow!("unknown provider {other:?}")),
        }
    }

    /// The provider's name as the dashboard and the phones write it.
    pub fn name(self) -> &'static str {
        match self {
            Provider::Anthropic => "Anthropic",
            Provider::Kimi => "Kimi Code",
        }
    }

    /// The article its name takes: "an Anthropic account", "a Kimi Code account".
    pub fn article(self) -> &'static str {
        match self {
            Provider::Anthropic => "an",
            Provider::Kimi => "a",
        }
    }

    /// Where an account of this provider sends its requests when it names no base URL of
    /// its own. An Anthropic account has none here: it goes to the router's `upstream`.
    pub fn default_base_url(self) -> Option<&'static str> {
        match self {
            Provider::Anthropic => None,
            Provider::Kimi => Some("https://api.kimi.ai/coding"),
        }
    }

    /// Why `key` cannot be this provider's credential, if it cannot. A key pasted into the
    /// wrong provider's field would otherwise be stored, and refused upstream at every
    /// request it signed.
    pub fn key_problem(self, key: &str) -> Option<&'static str> {
        match self {
            Provider::Anthropic if key.starts_with("sk-ant-sid") => Some(
                "that is a claude.ai session key — it belongs in the session-key field, \
                 and cannot sign API requests",
            ),
            Provider::Anthropic if !key.starts_with("sk-ant-") => {
                Some("expected a token starting sk-ant- (from `claude setup-token`)")
            }
            Provider::Kimi if key.starts_with("sk-ant-") => Some(
                "that is an Anthropic token — a Kimi Code account takes a key from the \
                 Kimi Code console",
            ),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Account {
    pub id: String,
    pub label: String,
    pub source: Source,
    pub provider: Provider,
    /// Where this account's requests go, when not its provider's default. Always `None`
    /// for an Anthropic account.
    pub base_url: Option<String>,
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
    // A hosted router has no Claude Code to ask, and spawning a binary that is not
    // there costs a failed exec on every account listing for nothing.
    if crate::config::is_server() {
        return Ok(AuthStatus::default());
    }
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

/// Why a Claude Code login or a profile cannot sign on a hosted router: both are files on
/// a developer's machine, and the router is not on one.
pub const NOT_ON_A_SERVER: &str = "this account is a Claude Code login on a developer's machine, which a hosted \
    router cannot read — add an account with a `claude setup-token` value in the dashboard";

/// Resolve an account to a token to sign with. Blocking; call from a blocking task.
pub fn resolve(account: &Account) -> Result<Resolved> {
    match account.source {
        Source::Keychain => {
            if let Ok(token) = std::env::var("AGENT_ROUTER_ACCESS_TOKEN") {
                if !token.is_empty() {
                    return Ok(Resolved { token, identity: None, expires_at_ms: 0 });
                }
            }
            if crate::config::is_server() {
                bail!("{}", NOT_ON_A_SERVER);
            }
            // `claude /login` overwrites this one Keychain entry in place, so this
            // account means "whoever Claude Code is logged into now" — deliberately
            // dynamic. The identity is therefore resolved per request and stamped on
            // the stored row, so a later /login cannot retroactively relabel traffic
            // that a different account actually served.
            let live = probe_profile(None).ok().and_then(|s| s.email);
            let raw = read_default_profile_credentials()?;
            let mut resolved = parse_stored(&raw)?;
            resolved.identity = live;
            Ok(resolved)
        }
        Source::Token => {
            let token = secrets::get(&account.id).token.ok_or_else(|| {
                anyhow!("no token stored for account {} — add one in the dashboard", account.id)
            })?;
            // setup-token issues long-lived tokens; there is no expiry to track.
            Ok(Resolved {
                token: token.trim().to_string(),
                identity: account.email.clone(),
                expires_at_ms: 0,
            })
        }
        Source::Profile => {
            if crate::config::is_server() {
                bail!("{}", NOT_ON_A_SERVER);
            }
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

/// Router-owned secrets live in a 0600 file, not the Keychain.
///
/// The Keychain identifies an application by its code signature, and a locally built
/// binary is ad-hoc signed with a per-build hash, so every rebuild is a new
/// application to the ACL and "Always Allow" never sticks. The workaround is to trust
/// `/usr/bin/security` on the item — but that lets *any* process running as this user
/// read it without a prompt, which is the same exposure as a 0600 file in the user's
/// home directory. Given identical real-world exposure, the file avoids the prompts,
/// avoids the `-U` update path that cannot set an ACL without a GUI prompt, and keeps
/// rotation a plain atomic write.
///
/// What it gives up: encryption at rest while the Keychain is locked. FileVault
/// covers the disk-at-rest case, which is the one that matters here.
///
/// Claude Code's own credential entry is still read from the Keychain — that store
/// belongs to Claude Code, and reading it never prompts.
mod secrets {
    use super::*;
    use std::collections::BTreeMap;
    use std::io::Write;
    #[cfg(unix)]
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    #[derive(Debug, Default, Serialize, Deserialize)]
    pub struct Entry {
        #[serde(skip_serializing_if = "Option::is_none", default)]
        pub token: Option<String>,
        /// A claude.ai session key, used only to read usage.
        #[serde(skip_serializing_if = "Option::is_none", default)]
        pub usage_key: Option<String>,
    }

    /// The APNs signing key and its ids — what lets the router tell a phone a session
    /// is waiting. The ids are not secret; they live here so they cannot drift from
    /// the key they belong to.
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct PushKey {
        pub team_id: String,
        pub key_id: String,
        /// The `.p8` file's contents, PEM.
        pub p8: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub bundle_id: Option<String>,
    }

    /// The Firebase service account — what lets the router tell an Android phone a
    /// session is waiting. Kept beside the APNs key rather than folded into it: the two
    /// are set, replaced and cleared independently, and widening `PushKey` would make
    /// "an Apple key with no key in it" a thing the type allows.
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct FcmKey {
        pub project_id: String,
        pub client_email: String,
        /// The service account's `private_key`, PEM.
        pub private_key: String,
        pub private_key_id: String,
        pub token_uri: String,
    }

    #[derive(Debug, Default, Serialize, Deserialize)]
    struct File {
        version: u32,
        #[serde(default)]
        accounts: BTreeMap<String, Entry>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        push: Option<PushKey>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fcm: Option<FcmKey>,
    }

    pub fn push_key() -> Option<PushKey> {
        load().ok().and_then(|f| f.push)
    }

    pub fn store_push_key(key: Option<PushKey>) -> Result<()> {
        change(|file| file.push = key)
    }

    pub fn fcm_key() -> Option<FcmKey> {
        load().ok().and_then(|f| f.fcm)
    }

    pub fn store_fcm_key(key: Option<FcmKey>) -> Result<()> {
        change(|file| file.fcm = key)
    }

    /// One change at a time. Each is a read of the whole file, an edit and a rename into
    /// place, so two at once would each write back what the other had not seen, and the
    /// later rename would drop the earlier change: an account's token, or a push key. A
    /// read needs no turn, since the rename is atomic and a reader sees one whole file or
    /// the other.
    static CHANGING: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn change(edit: impl FnOnce(&mut File)) -> Result<()> {
        let _turn = CHANGING.lock().unwrap_or_else(|e| e.into_inner());
        let mut file = load()?;
        file.version = 1;
        edit(&mut file);
        store(&file)
    }

    fn path() -> Result<PathBuf> {
        let dir = dir()?;
        // The directory holds secrets; keep it owner-only too.
        #[cfg(unix)]
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
        Ok(dir.join("credentials.json"))
    }

    #[cfg(not(test))]
    fn dir() -> Result<PathBuf> {
        crate::config::data_dir()
    }

    /// Under test, a directory of the test process's own. The real file holds the live
    /// router's tokens and push keys, and the data directory a test resolves is the
    /// person's own unless something has moved `HOME`: a test that adds an account would
    /// otherwise write its token there, beside theirs.
    #[cfg(test)]
    fn dir() -> Result<PathBuf> {
        static DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
        let dir = DIR.get_or_init(|| {
            let dir = std::env::temp_dir().join(format!("ar-secrets-{}", std::process::id()));
            // An earlier test process may have had the same id, and left its file here.
            let _ = std::fs::remove_dir_all(&dir);
            dir
        });
        std::fs::create_dir_all(dir)?;
        Ok(dir.clone())
    }

    fn load() -> Result<File> {
        let path = path()?;
        match std::fs::read_to_string(&path) {
            Ok(text) => Ok(serde_json::from_str(&text).unwrap_or_default()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(File::default()),
            Err(e) => Err(e.into()),
        }
    }

    /// Write 0600 from the moment the file exists — never create it readable and
    /// tighten afterwards — and rename into place so a reader never sees a partial file.
    fn store(file: &File) -> Result<()> {
        let path = path()?;
        let tmp = path.with_extension("json.tmp");
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut f = options.open(&tmp)?;
        f.write_all(serde_json::to_string_pretty(file)?.as_bytes())?;
        f.write_all(b"\n")?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }

    pub fn get(account_id: &str) -> Entry {
        load()
            .ok()
            .and_then(|f| f.accounts.get(account_id).map(|e| Entry {
                token: e.token.clone(),
                usage_key: e.usage_key.clone(),
            }))
            .unwrap_or_default()
    }

    pub fn update(account_id: &str, f: impl FnOnce(&mut Entry)) -> Result<()> {
        change(|file| {
            f(file.accounts.entry(account_id.to_string()).or_default());
            // Drop the account entirely once it holds nothing.
            if file
                .accounts
                .get(account_id)
                .map(|e| e.token.is_none() && e.usage_key.is_none())
                .unwrap_or(false)
            {
                file.accounts.remove(account_id);
            }
        })
    }
}

pub use secrets::{FcmKey, PushKey};

pub fn push_key() -> Option<PushKey> {
    secrets::push_key()
}

pub fn store_push_key(key: Option<PushKey>) -> Result<()> {
    secrets::store_push_key(key)
}

pub fn fcm_key() -> Option<FcmKey> {
    secrets::fcm_key()
}

pub fn store_fcm_key(key: Option<FcmKey>) -> Result<()> {
    secrets::store_fcm_key(key)
}

pub fn store_token(account_id: &str, token: &str) -> Result<()> {
    let token = token.trim().to_string();
    secrets::update(account_id, |e| e.token = Some(token))
}

pub fn forget_token(account_id: &str) -> Result<()> {
    secrets::update(account_id, |e| e.token = None)
}

/// A claude.ai session key (`sk-ant-sid…`), used only to read usage. It is never
/// sent to api.anthropic.com and never used to sign a proxied request.
pub fn store_usage_token(account_id: &str, token: &str) -> Result<()> {
    let token = token.trim().to_string();
    secrets::update(account_id, |e| e.usage_key = Some(token))
}

/// Whether a signing credential is stored for this account. The value never leaves
/// the router; this is only so the UI can show that the field is already filled.
pub fn has_token(account_id: &str) -> bool {
    secrets::get(account_id).token.is_some()
}

pub fn usage_token(account_id: &str) -> Option<String> {
    secrets::get(account_id).usage_key
}

pub fn forget_usage_token(account_id: &str) -> Result<()> {
    secrets::update(account_id, |e| e.usage_key = None)
}

/// Run `security`, optionally feeding an interactive-mode command line on stdin so a
/// secret never appears in the process list. Only used to *read* Claude Code's entry
/// now that the router keeps its own secrets in a file.
#[cfg(target_os = "macos")]
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

/// Read Claude Code's own stored credential for the default profile.
///
/// macOS keeps it in the login Keychain; elsewhere Claude Code writes
/// `~/.claude/.credentials.json`, the same shape an isolated profile directory uses.
/// Either way this is Claude Code's store, not ours, and we only ever read it.
fn read_default_profile_credentials() -> Result<String> {
    #[cfg(target_os = "macos")]
    {
        keychain_read(CLAUDE_SERVICE, &current_user())
            .context("reading Claude Code's Keychain entry — log in with `claude` first")
    }
    #[cfg(not(target_os = "macos"))]
    {
        let path = home_dir()?.join(".claude").join(".credentials.json");
        std::fs::read_to_string(&path).with_context(|| {
            format!("reading {} — log in with `claude` first", path.display())
        })
    }
}

#[cfg(not(target_os = "macos"))]
fn home_dir() -> Result<PathBuf> {
    crate::setup::home_dir()
}

#[cfg(target_os = "macos")]
fn keychain_read(service: &str, account: &str) -> Result<String> {
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

#[cfg(target_os = "macos")]
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

    /// The active Anthropic account: the one every request is signed with until requests
    /// are routed by their model (docs/sources-and-models.md).
    pub fn active(&self) -> Result<Account> {
        self.active_for(Provider::Anthropic)
    }

    /// The account `provider`'s requests go to when nothing else is chosen. Each provider
    /// has its own, so making a Kimi Code account active leaves the Claude account active.
    pub fn active_for(&self, provider: Provider) -> Result<Account> {
        let account = self
            .conn()?
            .query_row(
                &format!(
                    "SELECT {ACCOUNT_COLUMNS} FROM accounts
                     WHERE active = 1 AND provider = ?1 ORDER BY created_at LIMIT 1"
                ),
                [provider.as_str()],
                account_row,
            )
            .optional()?;
        Account::from_row(
            account.ok_or_else(|| anyhow!("no account to sign with — add one in the dashboard"))?,
        )
    }

    /// The account with this id, if there is one.
    pub fn get(&self, id: &str) -> Result<Option<Account>> {
        self.conn()?
            .query_row(
                &format!("SELECT {ACCOUNT_COLUMNS} FROM accounts WHERE id = ?1"),
                [id],
                account_row,
            )
            .optional()?
            .map(Account::from_row)
            .transpose()
    }

    /// The name of the account that has this id, if one does.
    fn holder(&self, id: &str) -> Result<Option<String>> {
        Ok(self
            .conn()?
            .query_row("SELECT label FROM accounts WHERE id = ?1", [id], |r| r.get(0))
            .optional()?)
    }

    /// Refuse an id another account has ([`Conflict::NameTaken`]) before anything is made
    /// under it. A profile account's directory and a sign-in's are both named by the id, so
    /// for a taken name the directory is that account's own; `add` refuses the name too,
    /// but only once that directory had been made and used.
    pub fn check_free(&self, id: &str) -> Result<()> {
        match self.holder(id)? {
            Some(holder) => Err(Conflict::NameTaken { id: id.to_string(), holder }.into()),
            None => Ok(()),
        }
    }

    /// Add an account. Never one whose id another account has: that is refused with
    /// [`Conflict::NameTaken`], and the other account is left exactly as it was.
    ///
    /// `credential` stores what the new account signs with — a token goes in the secrets
    /// file, under the same id — and runs only once the row is in. The insert is what
    /// decides, in one statement, whether the id was free, so a refused add never reaches
    /// the holder's credential, even when two adds race for one name. Storing the token
    /// first, as this once did, wrote it over the holder's before anything had looked.
    /// When the credential cannot be stored, the row comes out again: an account with
    /// nothing behind it would only fail every request it signed.
    #[allow(clippy::too_many_arguments)]
    pub fn add(
        &self,
        id: &str,
        label: &str,
        source: Source,
        provider: Provider,
        base_url: Option<&str>,
        profile_dir: Option<&str>,
        status: &AuthStatus,
        credential: impl FnOnce() -> Result<()>,
    ) -> Result<()> {
        let conn = self.conn()?;
        let added = conn.execute(
            "INSERT INTO accounts (id, label, source, provider, base_url, profile_dir, email,
                                   org_name, subscription_type, active, created_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,0,?10)
             ON CONFLICT(id) DO NOTHING",
            rusqlite::params![
                id,
                label,
                source.as_str(),
                provider.as_str(),
                base_url,
                profile_dir,
                status.email,
                status.org_name,
                status.subscription_type,
                now_ms()
            ],
        )?;
        if added == 0 {
            let holder = self.holder(id)?.unwrap_or_else(|| id.to_string());
            return Err(Conflict::NameTaken { id: id.to_string(), holder }.into());
        }
        if let Err(e) = credential() {
            let _ = conn.execute("DELETE FROM accounts WHERE id = ?1", [id]);
            return Err(e);
        }
        // Never leave the router with nothing to sign with: the first account a store
        // gets of a provider is the one that provider's requests go to. Locally the
        // discovered default always holds Anthropic's place, so this matters to a hosted
        // router, whose store starts empty, and to every provider's first account. Only
        // now, with the credential stored, so it never signs from an empty entry.
        conn_activate_if_none(&conn, id)?;
        Ok(())
    }

    /// Rename an account, or record who it is: only what is given changes, and never its
    /// id, its source or its credential.
    pub fn edit(&self, id: &str, label: Option<&str>, email: Option<&str>) -> Result<()> {
        let hit = self.conn()?.execute(
            "UPDATE accounts SET label = COALESCE(?2, label), email = COALESCE(?3, email)
             WHERE id = ?1",
            rusqlite::params![id, label, email],
        )?;
        if hit == 0 {
            return Err(Conflict::NoAccount(id.to_string()).into());
        }
        Ok(())
    }

    /// What a profile account's own login says, once `claude auth login` has run in its
    /// directory: who it is, and its name becomes that. Only ever an update of the profile
    /// account in `dir`, so a probe can neither add an account nor reach one that has since
    /// been removed and its name taken by another.
    pub fn record_login(&self, id: &str, dir: &str, label: &str, status: &AuthStatus) -> Result<()> {
        self.conn()?.execute(
            "UPDATE accounts SET label = ?3,
                    email = COALESCE(?4, email), org_name = COALESCE(?5, org_name),
                    subscription_type = COALESCE(?6, subscription_type)
             WHERE id = ?1 AND profile_dir = ?2",
            rusqlite::params![id, dir, label, status.email, status.org_name, status.subscription_type],
        )?;
        Ok(())
    }

    /// Exactly one account of each provider is active; switching is a single transaction,
    /// and touches only the provider of the account switched to.
    pub fn activate(&self, id: &str) -> Result<()> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let hit = tx.execute(
            "UPDATE accounts SET active = (id = ?1)
              WHERE provider = (SELECT provider FROM accounts WHERE id = ?1)",
            [id],
        )?;
        if hit == 0 {
            return Err(Conflict::NoAccount(id.to_string()).into());
        }
        tx.commit()?;
        Ok(())
    }

    pub fn remove(&self, id: &str) -> Result<()> {
        let conn = self.conn()?;
        let was_active: Option<String> = conn
            .query_row("SELECT provider FROM accounts WHERE id = ?1 AND active = 1", [id], |r| {
                r.get(0)
            })
            .optional()?;
        conn.execute("DELETE FROM accounts WHERE id = ?1", [id])?;
        // Its models go back to their provider's active account.
        conn.execute("DELETE FROM model_assignments WHERE account_id = ?1", [id])?;
        let _ = forget_token(id);
        let _ = forget_usage_token(id);
        if let Some(provider) = was_active {
            // Never leave a provider with nothing to sign with while it has an account.
            conn.execute(
                "UPDATE accounts SET active = 1 WHERE id =
                   (SELECT id FROM accounts WHERE provider = ?1 ORDER BY created_at LIMIT 1)",
                [provider],
            )?;
        }
        Ok(())
    }
}

fn conn_activate_if_none(conn: &rusqlite::Connection, id: &str) -> Result<()> {
    conn.execute(
        "UPDATE accounts SET active = 1
          WHERE id = ?1 AND NOT EXISTS (
            SELECT 1 FROM accounts o
             WHERE o.active = 1 AND o.provider = (SELECT provider FROM accounts WHERE id = ?1))",
        [id],
    )?;
    Ok(())
}

/// What every read of an account selects, in the order `account_row` takes it.
const ACCOUNT_COLUMNS: &str = "id, label, source, provider, base_url, profile_dir, email";

/// One row as `SELECT {ACCOUNT_COLUMNS}` reads it.
type AccountRow = (String, String, Option<String>, Option<String>, Option<String>, Option<String>, Option<String>);

fn account_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<AccountRow> {
    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?))
}

impl Account {
    fn from_row((id, label, source, provider, base_url, profile_dir, email): AccountRow) -> Result<Self> {
        Ok(Account {
            id,
            label,
            source: Source::parse(source.as_deref().unwrap_or("keychain"))?,
            provider: Provider::parse(provider.as_deref().unwrap_or("anthropic"))?,
            base_url,
            profile_dir,
            email,
        })
    }
}

/// A change to the accounts that what the store holds stands in the way of, as opposed
/// to one the store failed at. The API answers these 409 (see `refusal`): the request
/// was well made, and it is the store's state that has to change for it to go through.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Conflict {
    /// The name given is, made safe ([`slugify`]), the id of another account. Everything
    /// an account is hangs off its id — its row, its token in the secrets file, its
    /// profile directory, the history recorded against it — so adding under that id
    /// would replace the account: a new "Work" took an existing "work"'s credential and
    /// source, and a new "Default" turned the built-in account into a token account.
    /// Adding never replaces; this is said instead, naming the account that has the id.
    NameTaken {
        id: String,
        /// That account's name, as the lists show it.
        holder: String,
    },
    /// No account has this id: removed since it was listed, or never added.
    NoAccount(String),
}

impl std::fmt::Display for Conflict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Conflict::NameTaken { id, holder } => write!(
                f,
                "that name is the id {id:?}, which the account {holder:?} already has — choose another name"
            ),
            Conflict::NoAccount(id) => write!(f, "no account with id {id:?}"),
        }
    }
}

impl std::error::Error for Conflict {}

/// Where the router keeps the isolated profile directories it creates.
#[cfg(not(test))]
pub fn profiles_root() -> Result<PathBuf> {
    let dir = crate::config::data_dir()?.join("accounts");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Under test, a directory of the test process's own, as the secrets file's is: the real
/// one holds the profiles the live router signs with, and a sign-in or a profile add a test
/// makes would otherwise make its directory there.
#[cfg(test)]
pub fn profiles_root() -> Result<PathBuf> {
    static DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    let dir = DIR.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("ar-profiles-{}", std::process::id()));
        // An earlier test process may have had the same id, and left its directories here.
        let _ = std::fs::remove_dir_all(&dir);
        dir
    });
    std::fs::create_dir_all(dir)?;
    Ok(dir.clone())
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

/// A credential's kind by its account's provider, safe to show and to log: an Anthropic
/// token's by its prefix (`token_kind`), a Kimi Code account's as its key, whose form Kimi Code
/// does not promise. Never any part of the secret.
pub fn credential_kind(provider: Provider, token: &str) -> &str {
    match provider {
        Provider::Kimi if token.trim().is_empty() => "<empty>",
        Provider::Kimi => "Kimi Code key",
        Provider::Anthropic => token_kind(token),
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

    /// A Kimi Code key was "<unrecognised>" on its card, as an Anthropic token of no known
    /// family would be (AGEN-0123).
    #[test]
    fn a_kimi_code_key_is_described_as_one() {
        let key = "sk-kimi-SECRETSECRET";
        assert_eq!(credential_kind(Provider::Kimi, key), "Kimi Code key");
        assert!(!credential_kind(Provider::Kimi, key).contains("SECRET"));
        assert_eq!(credential_kind(Provider::Kimi, " "), "<empty>");
        assert_eq!(credential_kind(Provider::Anthropic, "sk-ant-oat01-SECRET"), "sk-ant-oat01 (subscription OAuth)");
        assert_eq!(credential_kind(Provider::Anthropic, "sk-kimi-SECRET"), "<unrecognised>", "an Anthropic account's is still its token's family");
    }

    /// A store of its own under the temp directory, opened as the router opens one at start.
    struct Scratch {
        dir: PathBuf,
        path: PathBuf,
    }

    impl Scratch {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("ar-accounts-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self { path: dir.join("agent-router.db"), dir }
        }

        /// Open the store as a start of the router does, and hand back its accounts.
        fn start(&self) -> Store {
            crate::db::Db::open(&self.path).unwrap();
            Store::new(self.path.clone())
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn add(store: &Store, id: &str, provider: Provider, base_url: Option<&str>) {
        store
            .add(id, id, Source::Token, provider, base_url, None, &AuthStatus::default(), || Ok(()))
            .unwrap();
    }

    #[test]
    fn an_account_carries_its_provider_and_base_url() {
        let scratch = Scratch::new("provider");
        let store = scratch.start();
        add(&store, "work", Provider::Anthropic, None);
        add(&store, "kimi", Provider::Kimi, Some("https://api.kimi.com/coding"));

        // Read again by a router started over the same store.
        let store = scratch.start();
        let work = store.get("work").unwrap().unwrap();
        assert_eq!((work.provider, work.base_url), (Provider::Anthropic, None));
        let kimi = store.get("kimi").unwrap().unwrap();
        assert_eq!(kimi.provider, Provider::Kimi);
        assert_eq!(kimi.base_url.as_deref(), Some("https://api.kimi.com/coding"));
        assert_eq!(kimi.source, Source::Token, "a Kimi Code key is kept as a setup-token is");
    }

    #[test]
    fn an_account_stored_before_providers_reads_as_anthropic() {
        let scratch = Scratch::new("before-providers");
        // The accounts table as it was before there was a second provider.
        let conn = rusqlite::Connection::open(&scratch.path).unwrap();
        conn.execute_batch(
            "CREATE TABLE accounts (
                id TEXT PRIMARY KEY, label TEXT NOT NULL, source TEXT NOT NULL DEFAULT 'keychain',
                profile_dir TEXT, email TEXT, org_name TEXT, subscription_type TEXT,
                active INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL);
             INSERT INTO accounts (id, label, source, active, created_at)
             VALUES ('default', 'Claude Code', 'keychain', 1, 1), ('work', 'Work', 'token', 0, 2);",
        )
        .unwrap();
        drop(conn);

        let store = scratch.start();
        for id in ["default", "work"] {
            let account = store.get(id).unwrap().unwrap();
            assert_eq!((account.provider, account.base_url), (Provider::Anthropic, None), "{id}");
        }
        assert_eq!(store.active().unwrap().id, "default", "it still signs");
    }

    #[test]
    fn each_provider_keeps_its_own_active_account() {
        let scratch = Scratch::new("active-per-provider");
        let store = scratch.start();
        // The first account of each provider is the one its requests go to.
        add(&store, "work", Provider::Anthropic, None);
        add(&store, "kimi", Provider::Kimi, Some("https://api.kimi.ai/coding"));
        add(&store, "home", Provider::Anthropic, None);
        add(&store, "kimi-two", Provider::Kimi, Some("https://api.kimi.ai/coding"));
        let active = |p| store.active_for(p).unwrap().id;
        assert_eq!((active(Provider::Anthropic), active(Provider::Kimi)), ("work".into(), "kimi".into()));

        store.activate("home").unwrap();
        assert_eq!(active(Provider::Anthropic), "home");
        assert_eq!(active(Provider::Kimi), "kimi", "activating a Claude account leaves Kimi Code's");

        store.activate("kimi-two").unwrap();
        assert_eq!(active(Provider::Kimi), "kimi-two");
        assert_eq!(active(Provider::Anthropic), "home", "activating a Kimi Code account leaves Claude's");
        assert_eq!(store.active().unwrap().id, "home", "what signs is the active Anthropic account");

        // Removing a provider's active account hands its place to that provider's next one.
        store.remove("kimi-two").unwrap();
        assert_eq!((active(Provider::Anthropic), active(Provider::Kimi)), ("home".into(), "kimi".into()));
        assert!(store.activate("nobody").is_err());
    }
}
