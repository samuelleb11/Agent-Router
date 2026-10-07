//! The machine agent's half of the Library: keep this machine's `~/.claude` in step with the
//! live versions its hosted router lists. docs/library.md, "The sync".
//!
//! For each item in the router's manifest:
//! - What is at its place already matches the digest: nothing to do.
//! - Otherwise its blob is downloaded and its digest checked, then unpacked into a staging
//!   directory in `~/.claude` but outside `skills/` — so Claude Code never lists a
//!   half-written skill — and renamed into place whole.
//! - What was at the place goes. The agent's own unchanged copy simply goes. Anything else —
//!   a copy edited here, a skill of that name the Library did not place, a link — is moved
//!   aside to `<data-dir>/library/replaced/`, the last one kept, and named in the report: the
//!   Library wins, and nothing an agent wrote is lost without trace. A link is moved as a
//!   link, never written through.
//!
//! An item the manifest no longer lists is removed only where the ledger
//! (`<data-dir>/library/ledger.json`) says this agent placed it.

use std::collections::{BTreeMap, HashSet};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use axum::http::{header, HeaderMap, HeaderValue, Method, Request, StatusCode};
use bytes::Bytes;
use serde::{Deserialize, Serialize};

use super::bundle::{self, Bundle};
use super::Kind;

/// How often the agent syncs between connections.
pub const EVERY: Duration = Duration::from_secs(15 * 60);

/// Where a sync reads and writes.
#[derive(Clone, Debug)]
pub struct Places {
    /// Claude Code's directory: `$CLAUDE_CONFIG_DIR`, or `~/.claude`.
    pub claude: PathBuf,
    /// The agent's data directory, `~/.agent-router`.
    pub data: PathBuf,
}

impl Places {
    pub fn system(data: PathBuf) -> Places {
        Places { claude: crate::registry::claude_dir(), data }
    }

    /// Where Claude Code reads an item of this kind.
    pub fn target(&self, kind: Kind, name: &str) -> PathBuf {
        match kind {
            Kind::Skill => self.claude.join("skills").join(name),
            Kind::Command => self.claude.join("commands").join(format!("{name}.md")),
            Kind::Subagent => self.claude.join("agents").join(format!("{name}.md")),
        }
    }

    /// Where the last copy a sync took off an item's place is kept.
    pub fn replaced(&self, kind: Kind, name: &str) -> PathBuf {
        let target = self.target(kind, name);
        let file = target.file_name().expect("a target has a name");
        self.library().join("replaced").join(kind.as_str()).join(file)
    }

    fn library(&self) -> PathBuf {
        self.data.join("library")
    }

    fn ledger(&self) -> PathBuf {
        self.library().join("ledger.json")
    }

    fn last_sync(&self) -> PathBuf {
        self.library().join("last-sync.json")
    }

    /// Beside `skills/`, `commands/` and `agents/`, on their filesystem, so a staged item
    /// renames into place, and outside them, so Claude Code never reads one half-written.
    fn staging(&self) -> PathBuf {
        self.claude.join(".agent-router-staging")
    }
}

/// One row of the router's manifest: an item's live version.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Wanted {
    pub kind: Kind,
    pub name: String,
    pub version: i64,
    pub digest: String,
    pub size: i64,
}

/// Where the Library comes from: the router, or a test's shelf. Asked from a blocking thread.
pub trait Source {
    fn manifest(&self) -> Result<Vec<Wanted>>;
    fn blob(&self, digest: &str) -> Result<Vec<u8>>;
}

/// What this agent placed, and so what it may take away again.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Ledger {
    #[serde(default)]
    items: BTreeMap<String, Placed>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Placed {
    kind: Kind,
    name: String,
    version: i64,
    digest: String,
    at_ms: i64,
}

fn key(kind: Kind, name: &str) -> String {
    format!("{kind}/{name}")
}

impl Ledger {
    fn load(places: &Places) -> Ledger {
        std::fs::read_to_string(places.ledger()).ok().and_then(|raw| serde_json::from_str(&raw).ok()).unwrap_or_default()
    }

    fn save(&self, places: &Places) -> Result<()> {
        write_json(&places.ledger(), self)
    }

    fn record(&mut self, item: &Wanted, now: i64) {
        let placed = Placed { kind: item.kind, name: item.name.clone(), version: item.version, digest: item.digest.clone(), at_ms: now };
        self.items.insert(key(item.kind, &item.name), placed);
    }
}

/// What a sync did with one item.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    InStep,
    Placed,
    /// Placed over something the Library had not placed as it was, which is kept aside.
    Replaced,
    /// Taken off this machine: the Library no longer lists it.
    Removed,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Line {
    pub kind: Kind,
    pub name: String,
    pub version: i64,
    pub outcome: Outcome,
    /// Where the copy this sync took off the item's place is kept.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kept: Option<String>,
    /// Why it failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
}

impl Line {
    fn of(kind: Kind, name: &str, version: i64, outcome: Outcome) -> Line {
        Line { kind, name: name.to_string(), version, outcome, kept: None, why: None }
    }
}

/// What one sync did, item by item. Kept as `<data-dir>/library/last-sync.json`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Report {
    pub at_ms: i64,
    pub items: Vec<Line>,
}

/// What is at an item's place now.
enum Here {
    Nothing,
    Link,
    /// A file or a directory, and its digest when it reads as a bundle of the item's kind.
    Copy(Option<String>),
}

fn here(places: &Places, kind: Kind, name: &str) -> Here {
    let target = places.target(kind, name);
    match std::fs::symlink_metadata(&target) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Here::Nothing,
        Ok(meta) if meta.file_type().is_symlink() => Here::Link,
        _ => Here::Copy(Bundle::from_path(kind, name, &target).ok().map(|b| bundle::digest(&b.pack()))),
    }
}

/// Bring this machine in step with `source`, and say what was done. One sync runs at a time
/// per data directory: the agent's and `agent-router library sync`'s wait for each other.
pub fn sync(places: &Places, source: &dyn Source, now: i64) -> Result<Report> {
    let _one = one_at_a_time(places)?;
    let wanted = source.manifest().context("reading the router's manifest")?;
    let mut ledger = Ledger::load(places);
    let mut items = Vec::new();
    for item in &wanted {
        let line = place(places, &mut ledger, source, item, now).unwrap_or_else(|e| Line {
            why: Some(format!("{e:#}")),
            ..Line::of(item.kind, &item.name, item.version, Outcome::Failed)
        });
        items.push(line);
        ledger.save(places)?;
    }
    let listed: HashSet<String> = wanted.iter().map(|w| key(w.kind, &w.name)).collect();
    let gone: Vec<Placed> = ledger.items.values().filter(|p| !listed.contains(&key(p.kind, &p.name))).cloned().collect();
    for placed in gone {
        let line = withdraw(places, &mut ledger, &placed).unwrap_or_else(|e| Line {
            why: Some(format!("{e:#}")),
            ..Line::of(placed.kind, &placed.name, placed.version, Outcome::Failed)
        });
        items.push(line);
        ledger.save(places)?;
    }
    let report = Report { at_ms: now, items };
    write_json(&places.last_sync(), &report)?;
    Ok(report)
}

fn place(places: &Places, ledger: &mut Ledger, source: &dyn Source, item: &Wanted, now: i64) -> Result<Line> {
    let line = |outcome| Line::of(item.kind, &item.name, item.version, outcome);
    let found = here(places, item.kind, &item.name);
    if matches!(&found, Here::Copy(Some(digest)) if *digest == item.digest) {
        ledger.record(item, now);
        return Ok(line(Outcome::InStep));
    }
    let blob = source.blob(&item.digest).context("downloading it")?;
    if bundle::digest(&blob) != item.digest {
        bail!("what the router sent does not match its digest, so it was not placed");
    }
    let bundle = Bundle::unpack(item.kind, &item.name, &blob).map_err(|why| anyhow!("{why}"))?;
    let staged = stage(places, &bundle)?;
    let target = places.target(item.kind, &item.name);
    let ours = ledger.items.get(&key(item.kind, &item.name)).map(|p| p.digest.clone());
    let swapped = (|| {
        let kept = match found {
            Here::Nothing => None,
            Here::Copy(Some(digest)) if Some(&digest) == ours.as_ref() => {
                remove(&target)?;
                None
            }
            Here::Link | Here::Copy(_) => Some(set_aside(places, item.kind, &item.name)?),
        };
        rename_in(&staged, &target).with_context(|| format!("moving it into {}", target.display()))?;
        anyhow::Ok(kept)
    })();
    let kept = swapped.inspect_err(|_| {
        let _ = remove(&staged);
    })?;
    ledger.record(item, now);
    Ok(match kept {
        Some(kept) => Line { kept: Some(kept), ..line(Outcome::Replaced) },
        None => line(Outcome::Placed),
    })
}

/// Take away an item the Library no longer lists. Only the agent's own unchanged copy
/// simply goes; anything else at its place is kept aside, as a replaced copy is.
fn withdraw(places: &Places, ledger: &mut Ledger, placed: &Placed) -> Result<Line> {
    let target = places.target(placed.kind, &placed.name);
    let kept = match here(places, placed.kind, &placed.name) {
        Here::Nothing => None,
        Here::Copy(Some(digest)) if digest == placed.digest => {
            remove(&target)?;
            None
        }
        Here::Link | Here::Copy(_) => Some(set_aside(places, placed.kind, &placed.name)?),
    };
    ledger.items.remove(&key(placed.kind, &placed.name));
    Ok(Line { kept, ..Line::of(placed.kind, &placed.name, placed.version, Outcome::Removed) })
}

/// Write the bundle's files where nothing reads them yet.
fn stage(places: &Places, bundle: &Bundle) -> Result<PathBuf> {
    let root = places.staging();
    std::fs::create_dir_all(&root).with_context(|| format!("creating {}", root.display()))?;
    let tag = format!("{}-{}-{}", bundle.name(), std::process::id(), crate::db::now_ms());
    let staged = match bundle.kind() {
        Kind::Skill => root.join(&tag),
        Kind::Command | Kind::Subagent => root.join(format!("{tag}.md")),
    };
    let written = (|| {
        for file in bundle.files() {
            let path = match bundle.kind() {
                Kind::Skill => file.path.split('/').fold(staged.clone(), |at, part| at.join(part)),
                Kind::Command | Kind::Subagent => staged.clone(),
            };
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&path, &file.bytes)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = if file.executable { 0o755 } else { 0o644 };
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode))?;
            }
        }
        std::io::Result::Ok(())
    })();
    if let Err(e) = written {
        let _ = remove(&staged);
        return Err(e).with_context(|| format!("writing it into {}", root.display()));
    }
    Ok(staged)
}

/// Move what is at an item's place to where the last replaced copy is kept, and say where.
fn set_aside(places: &Places, kind: Kind, name: &str) -> Result<String> {
    let target = places.target(kind, name);
    let keep = places.replaced(kind, name);
    remove(&keep)?;
    if let Some(parent) = keep.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if std::fs::rename(&target, &keep).is_err() {
        // Another filesystem: copy it over, then take it away.
        copy(&target, &keep).with_context(|| format!("keeping {} aside", target.display()))?;
        remove(&target)?;
    }
    Ok(keep.display().to_string())
}

/// Remove whatever is at `path` — a link as a link, never what it points to.
fn remove(path: &Path) -> Result<()> {
    match std::fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
        Ok(meta) if meta.is_dir() => Ok(std::fs::remove_dir_all(path)?),
        Ok(_) => Ok(std::fs::remove_file(path)?),
    }
}

fn copy(from: &Path, to: &Path) -> Result<()> {
    let meta = std::fs::symlink_metadata(from)?;
    if meta.file_type().is_symlink() {
        #[cfg(unix)]
        return Ok(std::os::unix::fs::symlink(std::fs::read_link(from)?, to)?);
        #[cfg(not(unix))]
        bail!("{} is a link on another disk, which cannot be moved", from.display());
    }
    if meta.is_dir() {
        std::fs::create_dir_all(to)?;
        for entry in std::fs::read_dir(from)? {
            let entry = entry?;
            copy(&entry.path(), &to.join(entry.file_name()))?;
        }
        return Ok(());
    }
    std::fs::copy(from, to)?;
    Ok(())
}

/// Rename into place. Windows refuses a rename while a reader holds the file open for a
/// moment, so it is tried again for a few seconds there.
fn rename_in(from: &Path, to: &Path) -> std::io::Result<()> {
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut tries = 0;
    loop {
        match std::fs::rename(from, to) {
            Err(e) if cfg!(windows) && e.kind() == std::io::ErrorKind::PermissionDenied && tries < 30 => {
                tries += 1;
                std::thread::sleep(Duration::from_millis(100));
            }
            done => return done,
        }
    }
}

/// Hold `<data-dir>/library/sync.lock` for one sync, waiting a minute for another to finish.
fn one_at_a_time(places: &Places) -> Result<std::fs::File> {
    std::fs::create_dir_all(places.library())?;
    let path = places.library().join("sync.lock");
    let file = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(&path)?;
    for _ in 0..600 {
        match file.try_lock() {
            Ok(()) => return Ok(file),
            Err(std::fs::TryLockError::WouldBlock) => std::thread::sleep(Duration::from_millis(100)),
            Err(std::fs::TryLockError::Error(e)) => return Err(e).context("locking the Library's sync"),
        }
    }
    bail!("another sync of the Library has been running here for a minute")
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(value)?)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Where an item stands on this machine, for `agent-router library status`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Standing {
    InStep,
    /// The live version is not here yet: nothing is, or the agent's copy of an older one.
    Behind,
    /// The agent's copy was edited here. The next sync puts the Library's back.
    ChangedHere,
    /// Something the Library did not place is at its place. The next sync replaces it.
    NotTheLibrarys,
    /// The last sync could not place it.
    Failed,
    /// Placed here, and no longer listed. The next sync takes it away.
    Withdrawn,
}

impl Standing {
    pub fn words(self) -> &'static str {
        match self {
            Standing::InStep => "in step",
            Standing::Behind => "behind",
            Standing::ChangedHere => "changed here",
            Standing::NotTheLibrarys => "not the Library's",
            Standing::Failed => "failed",
            Standing::Withdrawn => "withdrawn",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Status {
    pub kind: Kind,
    pub name: String,
    pub version: i64,
    pub standing: Standing,
    pub why: Option<String>,
}

/// Where each item the router lists stands here, and each it no longer lists that is still
/// here, as the next sync would find them.
pub fn status(places: &Places, wanted: &[Wanted]) -> Vec<Status> {
    let ledger = Ledger::load(places);
    let last: Report =
        std::fs::read_to_string(places.last_sync()).ok().and_then(|raw| serde_json::from_str(&raw).ok()).unwrap_or_default();
    let mut out = Vec::new();
    for item in wanted {
        let ours = ledger.items.get(&key(item.kind, &item.name)).map(|p| p.digest.as_str());
        let mut standing = match here(places, item.kind, &item.name) {
            Here::Copy(Some(digest)) if digest == item.digest => Standing::InStep,
            Here::Nothing => Standing::Behind,
            Here::Copy(Some(digest)) if Some(digest.as_str()) == ours => Standing::Behind,
            _ if ours.is_some() => Standing::ChangedHere,
            _ => Standing::NotTheLibrarys,
        };
        let failed = last.items.iter().find(|l| l.kind == item.kind && l.name == item.name && l.outcome == Outcome::Failed);
        let why = failed.and_then(|l| l.why.clone()).filter(|_| standing != Standing::InStep);
        if why.is_some() {
            standing = Standing::Failed;
        }
        out.push(Status { kind: item.kind, name: item.name.clone(), version: item.version, standing, why });
    }
    let listed: HashSet<String> = wanted.iter().map(|w| key(w.kind, &w.name)).collect();
    for placed in ledger.items.values().filter(|p| !listed.contains(&key(p.kind, &p.name))) {
        out.push(Status { kind: placed.kind, name: placed.name.clone(), version: placed.version, standing: Standing::Withdrawn, why: None });
    }
    out
}

fn row(kind: Kind, name: &str, version: i64, what: &str) -> String {
    format!("{name:<24} {kind:<9} v{version:<4} {what}")
}

/// `agent-router library status`, one line per item.
pub fn status_lines(status: &[Status]) -> Vec<String> {
    if status.is_empty() {
        return vec!["the Library is empty".into()];
    }
    status
        .iter()
        .map(|s| {
            let what = match &s.why {
                Some(why) => format!("{} ({why})", s.standing.words()),
                None => s.standing.words().to_string(),
            };
            row(s.kind, &s.name, s.version, &what)
        })
        .collect()
}

/// `agent-router library sync`, one line per item.
pub fn report_lines(report: &Report) -> Vec<String> {
    if report.items.is_empty() {
        return vec!["the Library is empty".into()];
    }
    report
        .items
        .iter()
        .map(|l| {
            let kept = l.kept.as_deref().map(|k| format!(": the copy that was here is kept in {k}")).unwrap_or_default();
            let what = match l.outcome {
                Outcome::InStep => "in step".to_string(),
                Outcome::Placed => "placed".to_string(),
                Outcome::Replaced => format!("replaced{kept}"),
                Outcome::Removed => format!("removed{kept}"),
                Outcome::Failed => format!("failed: {}", l.why.as_deref().unwrap_or("for no reason given")),
            };
            row(l.kind, &l.name, l.version, &what)
        })
        .collect()
}

/// Sync each time the agent connects to its router, and every `every` in between — never
/// before the first connection, and never two at once.
pub async fn keep_in_step<F, Fut>(mut connected: tokio::sync::watch::Receiver<u64>, every: Duration, mut sync: F)
where
    F: FnMut() -> Fut,
    Fut: Future<Output = ()>,
{
    if connected.changed().await.is_err() {
        return;
    }
    loop {
        sync().await;
        tokio::select! {
            changed = connected.changed() => if changed.is_err() { return },
            _ = tokio::time::sleep(every) => {}
        }
    }
}

/// One sync, unless this machine opted out of the Library (`agent install --no-library`):
/// then nothing is read from the router, nothing under `~/.claude` is touched, and there is
/// no report.
pub async fn run_once(places: Places, router: Arc<Router>, library: Option<bool>) -> Result<Option<Report>> {
    if library == Some(false) {
        return Ok(None);
    }
    let handle = tokio::runtime::Handle::current();
    let report =
        tokio::task::spawn_blocking(move || sync(&places, &Blocking { router: &router, handle }, crate::db::now_ms())).await??;
    Ok(Some(report))
}

/// The hosted router, as the agent reaches it for the Library: over HTTPS with the password
/// it already holds, apart from the tunnel.
pub struct Router {
    server: String,
    auth: HeaderValue,
    client: hyper_util::client::legacy::Client<
        hyper_rustls::HttpsConnector<hyper_util::client::legacy::connect::HttpConnector>,
        http_body_util::Full<Bytes>,
    >,
    /// The last manifest and its ETag, so an unchanged Library costs a 304.
    last: Mutex<Option<(String, Vec<Wanted>)>>,
}

impl Router {
    pub fn new(server: &str, password: &str) -> Result<Router> {
        let tls = hyper_rustls::HttpsConnectorBuilder::new().with_webpki_roots().https_or_http().enable_http1().build();
        let client = hyper_util::client::legacy::Client::builder(hyper_util::rt::TokioExecutor::new()).build(tls);
        let mut auth = HeaderValue::from_str(&format!("Bearer {password}")).context("the password cannot be sent in a header")?;
        auth.set_sensitive(true);
        Ok(Router { server: server.trim_end_matches('/').to_string(), auth, client, last: Mutex::new(None) })
    }

    async fn get(&self, path: &str, etag: Option<&str>) -> Result<(StatusCode, HeaderMap, Bytes)> {
        use http_body_util::BodyExt;
        let mut request = Request::builder()
            .method(Method::GET)
            .uri(format!("{}{path}", self.server))
            .header(header::AUTHORIZATION, self.auth.clone());
        if let Some(etag) = etag {
            request = request.header(header::IF_NONE_MATCH, etag);
        }
        let request = request.body(http_body_util::Full::new(Bytes::new()))?;
        let response = tokio::time::timeout(Duration::from_secs(60), self.client.request(request))
            .await
            .map_err(|_| anyhow!("{} did not answer", self.server))?
            .with_context(|| format!("reaching {}", self.server))?;
        let (parts, body) = response.into_parts();
        Ok((parts.status, parts.headers, body.collect().await?.to_bytes()))
    }

    pub async fn manifest(&self) -> Result<Vec<Wanted>> {
        #[derive(Deserialize)]
        struct Manifest {
            items: Vec<Wanted>,
        }
        let last = self.last.lock().unwrap().clone();
        let (status, headers, body) = self.get("/api/library/manifest", last.as_ref().map(|(tag, _)| tag.as_str())).await?;
        match (status, last) {
            (StatusCode::NOT_MODIFIED, Some((_, items))) => Ok(items),
            (StatusCode::OK, _) => {
                let manifest: Manifest = serde_json::from_slice(&body).context("the router's manifest is not one")?;
                if let Some(tag) = headers.get(header::ETAG).and_then(|t| t.to_str().ok()) {
                    *self.last.lock().unwrap() = Some((tag.to_string(), manifest.items.clone()));
                }
                Ok(manifest.items)
            }
            (status, _) => bail!("{} answered {status} for its Library: {}", self.server, words(&body)),
        }
    }

    pub async fn blob(&self, digest: &str) -> Result<Vec<u8>> {
        let (status, _, body) = self.get(&format!("/api/library/blobs/{digest}"), None).await?;
        if status != StatusCode::OK {
            bail!("{} answered {status} for {digest}: {}", self.server, words(&body));
        }
        Ok(body.to_vec())
    }
}

/// A refusal's words, from the router's JSON or its guard's plain line.
fn words(body: &[u8]) -> String {
    serde_json::from_slice::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_string))
        .unwrap_or_else(|| String::from_utf8_lossy(body).trim().to_string())
}

/// The router, asked from a blocking thread.
struct Blocking<'a> {
    router: &'a Router,
    handle: tokio::runtime::Handle,
}

impl Source for Blocking<'_> {
    fn manifest(&self) -> Result<Vec<Wanted>> {
        self.handle.block_on(self.router.manifest())
    }

    fn blob(&self, digest: &str) -> Result<Vec<u8>> {
        self.handle.block_on(self.router.blob(digest))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::library::bundle::File;

    fn scratch(label: &str) -> Places {
        let root = std::env::temp_dir().join(format!("ar-sync-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        Places { claude: root.join("claude"), data: root.join("data") }
    }

    fn done(places: &Places) {
        let _ = std::fs::remove_dir_all(places.claude.parent().unwrap());
    }

    fn file(path: &str, text: &str) -> File {
        File { path: path.into(), bytes: text.as_bytes().to_vec(), executable: false }
    }

    fn skill_md(name: &str, description: &str) -> String {
        format!("---\nname: {name}\ndescription: {description}\n---\n")
    }

    fn skill(name: &str, description: &str, more: Vec<File>) -> Bundle {
        let mut files = vec![file("SKILL.md", &skill_md(name, description))];
        files.extend(more);
        Bundle::check(Kind::Skill, name, files).unwrap()
    }

    fn command(name: &str, text: &str) -> Bundle {
        Bundle::check(Kind::Command, name, vec![file(&format!("{name}.md"), &format!("---\ndescription: {text}\n---\n{text}\n"))])
            .unwrap()
    }

    fn subagent(name: &str) -> Bundle {
        let text = format!("---\nname: {name}\ndescription: Reviews a diff.\n---\nYou review.\n");
        Bundle::check(Kind::Subagent, name, vec![file(&format!("{name}.md"), &text)]).unwrap()
    }

    /// The Library as a test holds it: bundles at their versions, served as the router would.
    #[derive(Default)]
    struct Shelf {
        items: Vec<(Bundle, i64)>,
        /// Serve each blob with a byte changed, as a damaged transfer would.
        tampered: bool,
        fetched: Mutex<Vec<String>>,
        /// Looks at the machine while a sync is under way, between the manifest and the swap.
        meanwhile: Option<Box<dyn Fn() + Send + Sync>>,
    }

    impl Shelf {
        fn of(items: Vec<(Bundle, i64)>) -> Shelf {
            Shelf { items, ..Default::default() }
        }
    }

    impl Source for Shelf {
        fn manifest(&self) -> Result<Vec<Wanted>> {
            Ok(self
                .items
                .iter()
                .map(|(b, version)| {
                    let packed = b.pack();
                    Wanted { kind: b.kind(), name: b.name().into(), version: *version, digest: bundle::digest(&packed), size: packed.len() as i64 }
                })
                .collect())
        }

        fn blob(&self, digest: &str) -> Result<Vec<u8>> {
            if let Some(look) = &self.meanwhile {
                look();
            }
            self.fetched.lock().unwrap().push(digest.to_string());
            let mut packed = self.items.iter().map(|(b, _)| b.pack()).find(|p| bundle::digest(p) == digest).expect("a listed blob");
            if self.tampered {
                packed[600] ^= 1;
            }
            Ok(packed)
        }
    }

    fn read(path: &Path) -> String {
        std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    }

    /// Every file under `dir`, relative, sorted.
    fn tree(dir: &Path) -> Vec<String> {
        fn walk(dir: &Path, prefix: &str, out: &mut Vec<String>) {
            for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
                let name = format!("{prefix}{}", entry.file_name().to_string_lossy());
                if entry.file_type().unwrap().is_dir() {
                    walk(&entry.path(), &format!("{name}/"), out);
                } else {
                    out.push(name);
                }
            }
        }
        let mut out = Vec::new();
        walk(dir, "", &mut out);
        out.sort();
        out
    }

    fn outcomes(report: &Report) -> Vec<(String, Outcome)> {
        report.items.iter().map(|l| (l.name.clone(), l.outcome)).collect()
    }

    #[test]
    fn a_sync_places_each_kind_where_claude_code_reads_it() {
        let places = scratch("kinds");
        let run = File { path: "scripts/run.sh".into(), bytes: b"#!/bin/sh\necho hi\n".to_vec(), executable: true };
        let shelf = Shelf::of(vec![
            (skill("bench", "Drives the bench.", vec![run]), 1),
            (command("ship", "Ship the branch."), 2),
            (subagent("reviewer"), 3),
        ]);
        let report = sync(&places, &shelf, 1).unwrap();
        assert_eq!(
            outcomes(&report),
            [("bench".into(), Outcome::Placed), ("ship".into(), Outcome::Placed), ("reviewer".into(), Outcome::Placed)]
        );
        let claude = &places.claude;
        assert_eq!(read(&claude.join("skills/bench/SKILL.md")), skill_md("bench", "Drives the bench."));
        assert_eq!(read(&claude.join("skills/bench/scripts/run.sh")), "#!/bin/sh\necho hi\n");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(claude.join("skills/bench/scripts/run.sh")).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o755, "the executable bit arrives");
        }
        assert!(read(&claude.join("commands/ship.md")).contains("Ship the branch."));
        assert!(read(&claude.join("agents/reviewer.md")).contains("name: reviewer"));

        // Again: everything is in step, and nothing is downloaded.
        shelf.fetched.lock().unwrap().clear();
        let again = sync(&places, &shelf, 2).unwrap();
        assert!(again.items.iter().all(|l| l.outcome == Outcome::InStep), "{again:?}");
        assert!(shelf.fetched.lock().unwrap().is_empty());
        assert!(tree(&places.claude.join(".agent-router-staging")).is_empty(), "nothing left half-made");
        done(&places);
    }

    #[test]
    fn an_item_whose_digest_differs_is_never_placed() {
        let places = scratch("digest");
        let mut shelf = Shelf::of(vec![(skill("bench", "One.", vec![]), 1)]);
        shelf.tampered = true;
        let report = sync(&places, &shelf, 1).unwrap();
        assert_eq!(report.items[0].outcome, Outcome::Failed);
        assert!(report.items[0].why.as_deref().unwrap().contains("does not match its digest"), "{report:?}");
        assert!(!places.target(Kind::Skill, "bench").exists(), "nothing was placed");

        // With a version in place, a damaged next one leaves it as it is.
        sync(&places, &Shelf::of(vec![(skill("bench", "One.", vec![]), 1)]), 2).unwrap();
        let mut next = Shelf::of(vec![(skill("bench", "Two.", vec![]), 2)]);
        next.tampered = true;
        assert_eq!(sync(&places, &next, 3).unwrap().items[0].outcome, Outcome::Failed);
        assert_eq!(read(&places.claude.join("skills/bench/SKILL.md")), skill_md("bench", "One."));
        assert!(tree(&places.claude.join(".agent-router-staging")).is_empty());
        done(&places);
    }

    #[test]
    fn an_item_is_swapped_in_whole() {
        let places = scratch("whole");
        let first = skill("bench", "One.", vec![file("notes.md", "old notes\n")]);
        sync(&places, &Shelf::of(vec![(first, 1)]), 1).unwrap();

        // While the next version downloads, the place still holds the first, whole, and
        // nothing else has appeared where Claude Code lists skills.
        let watched = places.clone();
        let mut next = Shelf::of(vec![(skill("bench", "Two.", vec![file("scripts/new.sh", "new\n")]), 2)]);
        next.meanwhile = Some(Box::new(move || {
            assert_eq!(tree(&watched.claude.join("skills")), ["bench/SKILL.md", "bench/notes.md"]);
            assert_eq!(read(&watched.claude.join("skills/bench/SKILL.md")), skill_md("bench", "One."));
        }));
        let report = sync(&places, &next, 2).unwrap();
        assert_eq!((report.items[0].outcome, report.items[0].kept.as_deref()), (Outcome::Placed, None), "its own copy is simply replaced");

        // Afterwards it is the second, whole: what the first had and the second has not is gone.
        assert_eq!(tree(&places.claude.join("skills")), ["bench/SKILL.md", "bench/scripts/new.sh"]);
        assert_eq!(read(&places.claude.join("skills/bench/SKILL.md")), skill_md("bench", "Two."));
        assert!(tree(&places.claude.join(".agent-router-staging")).is_empty());
        done(&places);
    }

    #[test]
    fn a_changed_copy_is_replaced_and_kept_aside() {
        let places = scratch("changed");
        let shelf = Shelf::of(vec![(skill("bench", "One.", vec![]), 1)]);
        sync(&places, &shelf, 1).unwrap();
        let placed = places.claude.join("skills/bench/SKILL.md");
        let kept = places.replaced(Kind::Skill, "bench");

        for edit in ["an agent's improvement\n", "a second one\n"] {
            std::fs::write(&placed, format!("{}{edit}", skill_md("bench", "One."))).unwrap();
            assert_eq!(status(&places, &shelf.manifest().unwrap())[0].standing, Standing::ChangedHere);
            let report = sync(&places, &shelf, 2).unwrap();
            let line = &report.items[0];
            assert_eq!((line.outcome, line.kept.as_deref()), (Outcome::Replaced, Some(kept.display().to_string().as_str())));
            assert_eq!(read(&placed), skill_md("bench", "One."), "the Library wins");
            assert!(read(&kept.join("SKILL.md")).ends_with(edit), "the last copy replaced is kept");
            let printed = report_lines(&report).join("\n");
            assert!(printed.contains(&format!("replaced: the copy that was here is kept in {}", kept.display())), "{printed}");
        }
        done(&places);
    }

    #[test]
    fn a_copy_the_library_did_not_place_is_replaced_and_kept_aside() {
        let places = scratch("foreign");
        let shelf = Shelf::of(vec![(skill("bench", "One.", vec![]), 1), (command("ship", "Ship the branch."), 1), (skill("same", "Same.", vec![]), 1)]);
        std::fs::create_dir_all(places.claude.join("skills/bench")).unwrap();
        std::fs::write(places.claude.join("skills/bench/SKILL.md"), "my own bench skill, written by hand\n").unwrap();
        std::fs::create_dir_all(places.claude.join("commands")).unwrap();
        std::fs::write(places.claude.join("commands/ship.md"), "my own ship command\n").unwrap();
        // A hand-made copy identical to the Library's is the Library's from now on.
        std::fs::create_dir_all(places.claude.join("skills/same")).unwrap();
        std::fs::write(places.claude.join("skills/same/SKILL.md"), skill_md("same", "Same.")).unwrap();
        let before: Vec<Standing> = status(&places, &shelf.manifest().unwrap()).iter().map(|s| s.standing).collect();
        assert_eq!(before, [Standing::NotTheLibrarys, Standing::NotTheLibrarys, Standing::InStep]);

        let report = sync(&places, &shelf, 1).unwrap();
        assert_eq!(
            outcomes(&report),
            [("bench".into(), Outcome::Replaced), ("ship".into(), Outcome::Replaced), ("same".into(), Outcome::InStep)]
        );
        assert_eq!(read(&places.replaced(Kind::Skill, "bench").join("SKILL.md")), "my own bench skill, written by hand\n");
        assert_eq!(read(&places.replaced(Kind::Command, "ship")), "my own ship command\n");
        assert_eq!(read(&places.claude.join("skills/bench/SKILL.md")), skill_md("bench", "One."));
        assert!(read(&places.claude.join("commands/ship.md")).contains("Ship the branch."));
        assert_eq!(report.items[2].kept, None);
        done(&places);
    }

    #[test]
    fn a_link_at_an_items_place_is_never_written_through() {
        #[cfg(unix)]
        {
            let places = scratch("link");
            // A link into a repository, as `make install` leaves Northstar's skill: the sync
            // replaces the link itself, and never writes into the repository behind it.
            let repo = places.claude.parent().unwrap().join("repo/skills/bench");
            std::fs::create_dir_all(&repo).unwrap();
            std::fs::write(repo.join("SKILL.md"), skill_md("bench", "One.")).unwrap();
            std::fs::create_dir_all(places.claude.join("skills")).unwrap();
            std::os::unix::fs::symlink(&repo, places.claude.join("skills/bench")).unwrap();

            let report = sync(&places, &Shelf::of(vec![(skill("bench", "Two.", vec![]), 2)]), 1).unwrap();
            assert_eq!(report.items[0].outcome, Outcome::Replaced);
            let kept = places.replaced(Kind::Skill, "bench");
            assert_eq!(std::fs::read_link(&kept).unwrap(), repo, "the link is kept as a link");
            let target = places.claude.join("skills/bench");
            assert!(!std::fs::symlink_metadata(&target).unwrap().file_type().is_symlink(), "a real copy now");
            assert_eq!(read(&target.join("SKILL.md")), skill_md("bench", "Two."));
            assert_eq!(read(&repo.join("SKILL.md")), skill_md("bench", "One."), "the repository is untouched");
            done(&places);
        }
    }

    #[test]
    fn a_withdrawn_item_is_removed_only_where_the_agent_placed_it() {
        let places = scratch("withdrawn");
        sync(&places, &Shelf::of(vec![(skill("bench", "One.", vec![]), 1), (command("ship", "Ship."), 1), (skill("vault", "Vault.", vec![]), 1)]), 1)
            .unwrap();
        // A skill the Library never placed, and one of its copies edited here.
        std::fs::create_dir_all(places.claude.join("skills/mine")).unwrap();
        std::fs::write(places.claude.join("skills/mine/SKILL.md"), "mine\n").unwrap();
        std::fs::write(places.claude.join("skills/vault/SKILL.md"), "vault, edited here\n").unwrap();

        let report = sync(&places, &Shelf::of(vec![(command("ship", "Ship."), 1)]), 2).unwrap();
        assert_eq!(
            outcomes(&report),
            [("ship".into(), Outcome::InStep), ("bench".into(), Outcome::Removed), ("vault".into(), Outcome::Removed)]
        );
        assert!(!places.claude.join("skills/bench").exists());
        assert!(!places.claude.join("skills/vault").exists());
        assert_eq!(report.items[2].kept.as_deref(), Some(places.replaced(Kind::Skill, "vault").display().to_string().as_str()));
        assert_eq!(read(&places.replaced(Kind::Skill, "vault").join("SKILL.md")), "vault, edited here\n");
        assert_eq!(read(&places.claude.join("skills/mine/SKILL.md")), "mine\n", "never the Library's, never touched");

        // Gone from the ledger too: a later sync has nothing more to take away.
        let later = sync(&places, &Shelf::of(vec![(command("ship", "Ship."), 1)]), 3).unwrap();
        assert_eq!(outcomes(&later), [("ship".into(), Outcome::InStep)]);
        done(&places);
    }

    #[tokio::test(start_paused = true)]
    async fn the_agent_syncs_on_connect_and_every_fifteen_minutes() {
        assert_eq!(EVERY, Duration::from_secs(15 * 60));
        let (connected, on_connect) = tokio::sync::watch::channel(0u64);
        let syncs = Arc::new(AtomicUsize::new(0));
        let count = syncs.clone();
        tokio::spawn(keep_in_step(on_connect, EVERY, move || {
            count.fetch_add(1, Ordering::SeqCst);
            async {}
        }));
        let after = |wait: Duration| async move {
            tokio::time::advance(wait).await;
            for _ in 0..10 {
                tokio::task::yield_now().await;
            }
        };
        let minutes = |m: u64| Duration::from_secs(m * 60);
        let seen = || syncs.load(Ordering::SeqCst);

        after(minutes(60)).await;
        assert_eq!(seen(), 0, "nothing before the first connection");
        connected.send_modify(|n| *n += 1);
        after(Duration::ZERO).await;
        assert_eq!(seen(), 1, "at once on connecting");
        after(minutes(14)).await;
        assert_eq!(seen(), 1);
        after(minutes(1) + Duration::from_secs(1)).await;
        assert_eq!(seen(), 2, "fifteen minutes on");
        after(minutes(15)).await;
        assert_eq!(seen(), 3);

        // A reconnection syncs at once, and the fifteen minutes start again from it.
        after(minutes(10)).await;
        connected.send_modify(|n| *n += 1);
        after(Duration::ZERO).await;
        assert_eq!(seen(), 4);
        after(minutes(14)).await;
        assert_eq!(seen(), 4);
        after(minutes(1) + Duration::from_secs(1)).await;
        assert_eq!(seen(), 5);
    }

    #[test]
    fn library_status_prints_one_line_per_item() {
        let places = scratch("status");
        sync(
            &places,
            &Shelf::of(vec![(skill("bench", "One.", vec![]), 3), (command("ship", "Ship."), 1), (skill("old", "Old.", vec![]), 1)]),
            1,
        )
        .unwrap();
        std::fs::write(places.claude.join("commands/ship.md"), "edited here\n").unwrap();
        std::fs::create_dir_all(places.claude.join("skills/vault")).unwrap();
        std::fs::write(places.claude.join("skills/vault/SKILL.md"), "mine\n").unwrap();
        // The last sync could not place `broken`.
        let last = Report {
            at_ms: 4,
            items: vec![Line { why: Some("what the router sent does not match its digest".into()), ..Line::of(Kind::Skill, "broken", 4, Outcome::Failed) }],
        };
        write_json(&places.last_sync(), &last).unwrap();

        let shelf = Shelf::of(vec![
            (skill("bench", "One.", vec![]), 3),
            (command("ship", "Ship."), 1),
            (skill("vault", "Vault.", vec![]), 2),
            (subagent("reviewer"), 1),
            (skill("broken", "Broken.", vec![]), 4),
        ]);
        let lines = status_lines(&status(&places, &shelf.manifest().unwrap()));
        let words: Vec<Vec<&str>> = lines.iter().map(|l| l.split_whitespace().collect()).collect();
        assert_eq!(
            words,
            [
                vec!["bench", "skill", "v3", "in", "step"],
                vec!["ship", "command", "v1", "changed", "here"],
                vec!["vault", "skill", "v2", "not", "the", "Library's"],
                vec!["reviewer", "subagent", "v1", "behind"],
                vec!["broken", "skill", "v4", "failed", "(what", "the", "router", "sent", "does", "not", "match", "its", "digest)"],
                vec!["old", "skill", "v1", "withdrawn"],
            ]
        );
        // One column each, so the lines read as a table: the kind always starts in one place.
        assert!(lines.iter().all(|l| l.as_bytes()[24] == b' ' && l.as_bytes()[25] != b' '), "{lines:#?}");
        assert!(lines[0].starts_with("bench ") && lines[0].ends_with(" in step"));
        assert_eq!(status_lines(&[]), ["the Library is empty"]);
        done(&places);
    }

    #[tokio::test]
    async fn library_sync_prints_its_report() {
        use std::net::SocketAddr;

        use crate::library::store::{self, Provenance};

        // A hosted router with two skills in its Library, served on a socket as the agent
        // reaches it.
        let lab = crate::api_refusals::Lab::hosted("library-sync").await;
        let conn = rusqlite::Connection::open(&lab.state.config.db_path).unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        for bundle in [skill("bench", "Drives the bench.", vec![]), skill("vault", "Keeps secrets.", vec![])] {
            store::publish(&conn, &bundle, &Provenance::Upload, true, 1).unwrap();
        }
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = lab.app.clone().into_make_service_with_connect_info::<SocketAddr>();
        tokio::spawn(async move { axum::serve(listener, app).await });

        let places = scratch("report");
        std::fs::create_dir_all(places.claude.join("skills/vault")).unwrap();
        std::fs::write(places.claude.join("skills/vault/SKILL.md"), "my own vault skill\n").unwrap();
        let router = Arc::new(Router::new(&format!("http://{addr}"), lab.password.unwrap()).unwrap());

        let report = run_once(places.clone(), router.clone(), None).await.unwrap().expect("a machine that takes the Library");
        let lines = report_lines(&report);
        assert_eq!(lines.len(), 2, "{lines:#?}");
        assert!(lines[0].starts_with("bench ") && lines[0].ends_with(" v1    placed"), "{lines:#?}");
        let kept = places.replaced(Kind::Skill, "vault").display().to_string();
        assert!(lines[1].starts_with("vault ") && lines[1].ends_with(&format!(" v1    replaced: the copy that was here is kept in {kept}")), "{lines:#?}");
        assert_eq!(read(&places.claude.join("skills/bench/SKILL.md")), skill_md("bench", "Drives the bench."));

        // Again: the router answers the manifest with a 304, and everything is in step.
        let again = run_once(places.clone(), router, None).await.unwrap().unwrap();
        assert!(report_lines(&again).iter().all(|l| l.ends_with(" in step")), "{again:?}");
        done(&places);
    }
}
