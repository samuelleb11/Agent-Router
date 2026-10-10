//! The Library's items, their versions and the blobs, in the main store beside the router's
//! own tables. `db::Db::open` creates the tables, on a new store and an existing one alike.
//!
//! A version never changes once stored. Its number, digest, description and provenance are
//! fixed, and so is the blob behind a digest: triggers refuse an UPDATE that would change
//! either. What moves is a version's state — `pending` to `live` or `rejected`, `live` to
//! `superseded` — and at most one version of an item is live, which a partial unique index
//! holds even against a mistake here.

use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension, Row, Transaction, TransactionBehavior};
use serde::{Deserialize, Serialize};

use super::bundle::{self, Bundle};
use super::Kind;

const SCHEMA: &str = r#"
-- One row per item. `source` says where its versions may come from: NULL for uploads and
-- publishes, or the one place an item imported from a forge, or built into the router, takes
-- them from (docs/library.md, "An item has one source of truth").
CREATE TABLE IF NOT EXISTS library_items (
  kind        TEXT NOT NULL CHECK (kind IN ('skill', 'command', 'subagent')),
  name        TEXT NOT NULL,
  source      TEXT,
  created_at  INTEGER NOT NULL,
  archived_at INTEGER,
  PRIMARY KEY (kind, name)
);

-- Packed bundles by the sha256 of their bytes. Versions with the same content share one.
CREATE TABLE IF NOT EXISTS library_blobs (
  digest     TEXT PRIMARY KEY,
  bytes      BLOB NOT NULL,
  created_at INTEGER NOT NULL
);

-- `size` is the packed bundle's, which is what a machine downloads. `provenance` is JSON:
-- see `Provenance`.
CREATE TABLE IF NOT EXISTS library_versions (
  kind        TEXT NOT NULL,
  name        TEXT NOT NULL,
  number      INTEGER NOT NULL,
  digest      TEXT NOT NULL REFERENCES library_blobs(digest),
  size        INTEGER NOT NULL,
  description TEXT NOT NULL,
  provenance  TEXT NOT NULL,
  state       TEXT NOT NULL CHECK (state IN ('live', 'pending', 'rejected', 'superseded')),
  created_at  INTEGER NOT NULL,
  decided_at  INTEGER,
  PRIMARY KEY (kind, name, number),
  FOREIGN KEY (kind, name) REFERENCES library_items(kind, name)
);
CREATE UNIQUE INDEX IF NOT EXISTS library_versions_one_live
  ON library_versions(kind, name) WHERE state = 'live';

CREATE TRIGGER IF NOT EXISTS library_versions_fixed
  BEFORE UPDATE OF kind, name, number, digest, size, description, provenance, created_at
  ON library_versions
BEGIN
  SELECT RAISE(ABORT, 'a Library version never changes: publish a new one');
END;

CREATE TRIGGER IF NOT EXISTS library_blobs_fixed
  BEFORE UPDATE ON library_blobs
BEGIN
  SELECT RAISE(ABORT, 'a Library blob never changes');
END;

-- The forges the Library imports from (`library::forge`), and each one's token, kept apart
-- so no read of a forge ever carries it. A forge's token is what the router presents there;
-- a public GitHub repository needs none.
CREATE TABLE IF NOT EXISTS library_forges (
  key        TEXT PRIMARY KEY,
  kind       TEXT NOT NULL CHECK (kind IN ('gitlab', 'github')),
  base_url   TEXT NOT NULL UNIQUE,
  created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS library_forge_tokens (
  forge  TEXT PRIMARY KEY REFERENCES library_forges(key) ON DELETE CASCADE,
  token  TEXT NOT NULL,
  set_at INTEGER NOT NULL
);

-- A repository, a ref and a path in it that items are imported from (`library::sources`).
-- `live` is the owner's trust: 1, its new versions go live; 0, they wait for approval.
-- `head` is the commit last imported, which the poller compares the ref's head with.
-- `hook_secret` is what its forge's webhook proves itself with, kept as the forge needs
-- it checked (`library::hooks`); no read ever carries it.
CREATE TABLE IF NOT EXISTS library_sources (
  key         TEXT PRIMARY KEY,
  forge       TEXT NOT NULL REFERENCES library_forges(key),
  repository  TEXT NOT NULL,
  ref         TEXT NOT NULL,
  path        TEXT NOT NULL DEFAULT '',
  live        INTEGER NOT NULL DEFAULT 0,
  head        TEXT,
  checked_at  INTEGER,
  imported_at INTEGER,
  last_result TEXT,
  hook_secret TEXT,
  created_at  INTEGER NOT NULL
);

-- The webhook deliveries already taken (`library::hooks`), a week of them, so a forge
-- sending one again imports nothing.
CREATE TABLE IF NOT EXISTS library_hook_deliveries (
  id     TEXT PRIMARY KEY,
  source TEXT NOT NULL,
  at_ms  INTEGER NOT NULL
);

-- Publish tokens (`library::tokens`): kept as the sha256 of the value, which is shown once,
-- when made. A revoked token keeps its row, so what it published still names it.
CREATE TABLE IF NOT EXISTS library_tokens (
  id           TEXT PRIMARY KEY,
  name         TEXT NOT NULL UNIQUE,
  hash         TEXT NOT NULL UNIQUE,
  kinds        TEXT NOT NULL,
  items        TEXT NOT NULL,
  live         INTEGER NOT NULL DEFAULT 0,
  created_at   INTEGER NOT NULL,
  last_used_at INTEGER,
  revoked_at   INTEGER
);

-- What each machine last said about the Library: its last sync's report, or that it takes
-- no part, or that its agent predates the Library. Kept here because machines themselves
-- are known only in memory, and a router restarted would otherwise know nothing of them.
-- `heard` is JSON: see `Heard`.
CREATE TABLE IF NOT EXISTS library_machines (
  machine TEXT PRIMARY KEY,
  label   TEXT,
  at_ms   INTEGER NOT NULL,
  heard   TEXT NOT NULL
);

-- Each use of a skill, a command or a subagent the router has carried (`library::usage`), by
-- name, whether the Library holds an item of that name yet or not: an item's history starts
-- before it joined. Keyed as the search index places a block, so the requests that repeat a
-- conversation's history add no use; `block_id` is a block of `bodies_db`.
CREATE TABLE IF NOT EXISTS library_uses (
  block_id   INTEGER NOT NULL,
  session_id TEXT NOT NULL,
  ordinal    INTEGER NOT NULL,
  how        TEXT NOT NULL CHECK (how IN ('called', 'launched', 'typed')),
  name       TEXT NOT NULL,
  at         INTEGER NOT NULL,
  PRIMARY KEY (block_id, session_id, ordinal)
);
CREATE INDEX IF NOT EXISTS library_uses_name ON library_uses(name, at);

-- Which machine each session ran on is the store's own now (`sessions.machine`, `places`).
DROP TABLE IF EXISTS library_session_machines;
"#;

/// A connection that may write: one of its own, waiting on other writers as the store's do.
pub fn writer(path: &std::path::Path) -> Result<Connection> {
    let conn = Connection::open(path)?;
    conn.busy_timeout(std::time::Duration::from_secs(30))?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    Ok(conn)
}

/// Create the Library's tables where they are missing. Called by `db::Db::open`.
pub fn ensure(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(SCHEMA)
}

/// Where a version came from. Stored as JSON, its origin under `from`, which the console
/// and the phones read.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "from", rename_all = "snake_case")]
pub enum Provenance {
    /// Sent from the console, or through the API with the password.
    Upload,
    /// Imported by a source (`library::sources`): its key, its forge's, the repository, the
    /// commit and the item's path in it.
    Forge { source: String, forge: String, repository: String, commit: String, path: String },
    /// Published with a publish token, named here by the token's name.
    Token { token: String },
    /// Published from a connected machine, by the Claude Code session that ran the publish
    /// when there was one.
    Machine {
        machine: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session: Option<String>,
    },
    /// Built into the router: its build.
    Router { build: String },
}

/// Where a version stands. Machines get only the `live` one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Live,
    /// Waiting for the owner's approval.
    Pending,
    Rejected,
    /// Was live, until another version was made live.
    Superseded,
}

impl State {
    pub fn as_str(self) -> &'static str {
        match self {
            State::Live => "live",
            State::Pending => "pending",
            State::Rejected => "rejected",
            State::Superseded => "superseded",
        }
    }

    fn parse(s: &str) -> Option<State> {
        [State::Live, State::Pending, State::Rejected, State::Superseded].into_iter().find(|v| v.as_str() == s)
    }
}

/// One stored version of an item.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Version {
    pub kind: Kind,
    pub name: String,
    /// From 1, in the order the item's versions were stored.
    pub number: i64,
    pub digest: String,
    pub size: i64,
    pub description: String,
    pub provenance: Provenance,
    pub state: State,
    pub created_at: i64,
    /// When it last went live, was superseded or was rejected.
    pub decided_at: Option<i64>,
}

/// An item and every version of it, newest first. An item never exists without a version:
/// the first is stored with it.
#[derive(Clone, Debug, Serialize)]
pub struct Item {
    pub kind: Kind,
    pub name: String,
    pub source: Option<String>,
    pub created_at: i64,
    /// Taken off every machine; going live again brings it back.
    pub archived_at: Option<i64>,
    pub versions: Vec<Version>,
}

impl Item {
    pub fn live(&self) -> Option<&Version> {
        self.versions.iter().find(|v| v.state == State::Live)
    }

    /// The live version's description, or the newest's while none is live.
    pub fn description(&self) -> &str {
        self.live().or(self.versions.first()).map_or("", |v| v.description.as_str())
    }
}

/// Why a version's state, or an item's, was not changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unmade {
    /// There is no such item, or no such version of it.
    Missing,
    /// It cannot move that way, and why, in words.
    Refused(String),
}

/// What a machine last said about the Library.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Heard {
    /// Its last sync, item by item.
    Report(super::sync::Report),
    /// Installed with `--no-library`.
    OptedOut,
    /// Its agent has no `/agent/library/sync`: it was installed before the Library.
    Predates,
}

/// Keep what a machine said, over what it said before.
pub fn hear(conn: &Connection, machine: &str, label: Option<&str>, heard: &Heard, now: i64) -> Result<()> {
    conn.execute(
        "INSERT INTO library_machines (machine, label, at_ms, heard) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(machine) DO UPDATE SET label = coalesce(excluded.label, label), at_ms = excluded.at_ms, heard = excluded.heard",
        params![machine, label, now, serde_json::to_string(heard)?],
    )?;
    Ok(())
}

/// What every machine last said, by machine id, with when.
pub fn heard(conn: &Connection) -> Result<std::collections::HashMap<String, (i64, Heard)>> {
    let mut stmt = conn.prepare("SELECT machine, at_ms, heard FROM library_machines")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?)))?;
    let mut out = std::collections::HashMap::new();
    for row in rows {
        let (machine, at_ms, heard) = row?;
        if let Ok(heard) = serde_json::from_str(&heard) {
            out.insert(machine, (at_ms, heard));
        }
    }
    Ok(out)
}

/// What publishing did.
#[derive(Debug)]
pub enum Published {
    New(Version),
    /// Nothing: the content is the item's newest version already, which is answered.
    Unchanged(Version),
    /// Nothing: the item takes versions from one place only, and this is not it. Why, in words.
    Refused(String),
}

impl Published {
    #[cfg(test)]
    pub fn version(&self) -> &Version {
        match self {
            Published::New(v) | Published::Unchanged(v) => v,
            Published::Refused(why) => panic!("refused: {why}"),
        }
    }
}

const COLUMNS: &str = "kind, name, number, digest, size, description, provenance, state, created_at, decided_at";

/// Store `bundle` as its item's next version, live or pending — unless it is exactly the
/// item's newest version already, when nothing changes. Content that goes back to an older
/// version's is a new version all the same: it is the newest change, and a forge reverting a
/// skill has to reach machines like any other change. Going live supersedes the live one and
/// brings an archived item back.
///
/// The one exception to "unchanged": the newest version was rejected and this publish goes
/// live. Only the owner, or a source the owner trusts, publishes live, and sending again what
/// was rejected is a new decision, which a rejected version cannot take (`make_live`).
/// Anything published to wait — a re-import, an untrusted publish — is unchanged, so what was
/// rejected is not asked about again.
pub fn publish(conn: &Connection, bundle: &Bundle, provenance: &Provenance, live: bool, now: i64) -> Result<Published> {
    let packed = bundle.pack();
    let digest = bundle::digest(&packed);
    let (kind, name) = (bundle.kind(), bundle.name());
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let created = tx.execute(
        "INSERT OR IGNORE INTO library_items (kind, name, source, created_at) VALUES (?1, ?2, ?3, ?4)",
        params![kind.as_str(), name, owner(provenance), now],
    )? == 1;
    if !created {
        if let Some(why) = not_from_here(&tx, kind, name, provenance)? {
            return Ok(Published::Refused(why));
        }
    }
    let newest = tx
        .query_row(
            &format!("SELECT {COLUMNS} FROM library_versions WHERE kind = ?1 AND name = ?2 ORDER BY number DESC LIMIT 1"),
            params![kind.as_str(), name],
            version,
        )
        .optional()?;
    if let Some(newest) = newest.as_ref().filter(|v| v.digest == digest && !(live && v.state == State::Rejected)) {
        let unchanged = newest.clone();
        tx.commit()?;
        return Ok(Published::Unchanged(unchanged));
    }
    tx.execute(
        "INSERT OR IGNORE INTO library_blobs (digest, bytes, created_at) VALUES (?1, ?2, ?3)",
        params![digest, packed, now],
    )?;
    if live {
        supersede(&tx, kind, name, now)?;
        unarchive(&tx, kind, name)?;
    }
    let number = newest.map_or(1, |v| v.number + 1);
    let state = if live { State::Live } else { State::Pending };
    tx.execute(
        &format!("INSERT INTO library_versions ({COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)"),
        params![
            kind.as_str(),
            name,
            number,
            digest,
            packed.len() as i64,
            bundle.description(),
            serde_json::to_string(provenance)?,
            state.as_str(),
            now,
            live.then_some(now),
        ],
    )?;
    let stored = get(&tx, kind, name, number)?.expect("the version just written");
    tx.commit()?;
    Ok(Published::New(stored))
}

/// Make version `number` the item's live one — approving a pending version, or rolling back
/// to an older one — and supersede the version that was. A version already live is left as
/// it is, and a rejected one never goes live.
pub fn make_live(conn: &Connection, kind: Kind, name: &str, number: i64, now: i64) -> Result<Result<Version, Unmade>> {
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let Some(found) = get(&tx, kind, name, number)? else { return Ok(Err(Unmade::Missing)) };
    match found.state {
        State::Live => {}
        State::Rejected => {
            return Ok(Err(Unmade::Refused(format!(
                "version {number} of the {kind} {name} was rejected, and a rejected version never goes live: \
                 upload its content again to make it a new version"
            ))))
        }
        State::Pending | State::Superseded => {
            supersede(&tx, kind, name, now)?;
            unarchive(&tx, kind, name)?;
            tx.execute(
                "UPDATE library_versions SET state = 'live', decided_at = ?4 WHERE kind = ?1 AND name = ?2 AND number = ?3",
                params![kind.as_str(), name, number, now],
            )?;
        }
    }
    let live = get(&tx, kind, name, number)?.expect("the version just read");
    tx.commit()?;
    Ok(Ok(live))
}

/// Turn down a pending version. Only a version waiting for approval can be rejected: one
/// that is or was live is rolled back from instead.
pub fn reject(conn: &Connection, kind: Kind, name: &str, number: i64, now: i64) -> Result<Result<Version, Unmade>> {
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let Some(found) = get(&tx, kind, name, number)? else { return Ok(Err(Unmade::Missing)) };
    if found.state != State::Pending {
        return Ok(Err(Unmade::Refused(format!(
            "version {number} of the {kind} {name} is {}, not waiting for approval: only a pending version can be \
             rejected",
            found.state.as_str()
        ))));
    }
    tx.execute(
        "UPDATE library_versions SET state = 'rejected', decided_at = ?4 WHERE kind = ?1 AND name = ?2 AND number = ?3",
        params![kind.as_str(), name, number, now],
    )?;
    let rejected = get(&tx, kind, name, number)?.expect("the version just read");
    tx.commit()?;
    Ok(Ok(rejected))
}

/// Take an item off every machine: its live version leaves the manifest, and the agents
/// remove what they placed. Its versions stay, and going live again brings it back.
pub fn archive(conn: &Connection, kind: Kind, name: &str, now: i64) -> Result<Result<(), Unmade>> {
    let found = conn.execute(
        "UPDATE library_items SET archived_at = coalesce(archived_at, ?3) WHERE kind = ?1 AND name = ?2",
        params![kind.as_str(), name, now],
    )?;
    Ok(if found == 1 { Ok(()) } else { Err(Unmade::Missing) })
}

/// Where an item a publish creates takes its versions from from then on: the source that
/// imported it, or the router's build. `None` for uploads and publishes, which share.
fn owner(provenance: &Provenance) -> Option<String> {
    match provenance {
        Provenance::Forge { source, .. } => Some(format!("source:{source}")),
        Provenance::Router { .. } => Some("router".to_string()),
        Provenance::Upload | Provenance::Token { .. } | Provenance::Machine { .. } => None,
    }
}

/// Why this publish may not add a version to an item that exists, if it may not. An item has
/// one source of truth: one a repository imported changes in that repository, and the
/// router's own changes with the router. One uploaded or published can be taken over by an
/// import only once it is archived.
fn not_from_here(tx: &Transaction, kind: Kind, name: &str, provenance: &Provenance) -> Result<Option<String>> {
    let (has, archived): (Option<String>, bool) = tx.query_row(
        "SELECT source, archived_at IS NOT NULL FROM library_items WHERE kind = ?1 AND name = ?2",
        params![kind.as_str(), name],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let wants = owner(provenance);
    if has == wants {
        return Ok(None);
    }
    if has.is_none() && archived {
        tx.execute("UPDATE library_items SET source = ?3 WHERE kind = ?1 AND name = ?2", params![kind.as_str(), name, wants])?;
        return Ok(None);
    }
    Ok(Some(match has.as_deref() {
        Some("router") => format!(
            "the {kind} {name} comes with the router's build: change skills/{name} in Agent Router's repository"
        ),
        Some(source) => {
            let key = source.trim_start_matches("source:");
            let from: Option<(String, String)> = tx
                .query_row("SELECT repository, ref FROM library_sources WHERE key = ?1", [key], |r| Ok((r.get(0)?, r.get(1)?)))
                .optional()?;
            match from {
                Some((repository, reference)) => {
                    format!("the {kind} {name} comes from {repository} ({reference}): change it there, and the Library follows")
                }
                None => format!("the {kind} {name} comes from the Library source {key}: change it there"),
            }
        }
        None => format!(
            "the {kind} {name} exists already, uploaded or published here: archive it first for an import to take it over"
        ),
    }))
}

fn supersede(tx: &Transaction, kind: Kind, name: &str, now: i64) -> rusqlite::Result<usize> {
    tx.execute(
        "UPDATE library_versions SET state = 'superseded', decided_at = ?3 WHERE kind = ?1 AND name = ?2 AND state = 'live'",
        params![kind.as_str(), name, now],
    )
}

fn unarchive(tx: &Transaction, kind: Kind, name: &str) -> rusqlite::Result<usize> {
    tx.execute(
        "UPDATE library_items SET archived_at = NULL WHERE kind = ?1 AND name = ?2",
        params![kind.as_str(), name],
    )
}

/// One item with all its versions, if there is one.
pub fn item(conn: &Connection, kind: Kind, name: &str) -> Result<Option<Item>> {
    let row = conn
        .query_row(
            "SELECT source, created_at, archived_at FROM library_items WHERE kind = ?1 AND name = ?2",
            params![kind.as_str(), name],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let Some((source, created_at, archived_at)) = row else { return Ok(None) };
    let versions = versions(conn, kind, name)?;
    Ok(Some(Item { kind, name: name.to_string(), source, created_at, archived_at, versions }))
}

/// Every item, archived ones included, by kind and name.
pub fn items(conn: &Connection) -> Result<Vec<Item>> {
    let mut stmt = conn.prepare("SELECT kind, name FROM library_items ORDER BY kind, name")?;
    let keys: Vec<(String, String)> = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
    let mut items = Vec::with_capacity(keys.len());
    for (kind, name) in keys {
        let Some(kind) = Kind::parse(&kind) else { continue };
        items.extend(item(conn, kind, &name)?);
    }
    Ok(items)
}

/// One version, if there is one.
pub fn get(conn: &Connection, kind: Kind, name: &str, number: i64) -> Result<Option<Version>> {
    Ok(conn
        .query_row(
            &format!("SELECT {COLUMNS} FROM library_versions WHERE kind = ?1 AND name = ?2 AND number = ?3"),
            params![kind.as_str(), name, number],
            version,
        )
        .optional()?)
}

/// Every version of an item, newest first.
pub fn versions(conn: &Connection, kind: Kind, name: &str) -> Result<Vec<Version>> {
    let mut stmt =
        conn.prepare(&format!("SELECT {COLUMNS} FROM library_versions WHERE kind = ?1 AND name = ?2 ORDER BY number DESC"))?;
    let rows = stmt.query_map(params![kind.as_str(), name], version)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// The live version of every item that is not archived: what machines hold.
pub fn live(conn: &Connection) -> Result<Vec<Version>> {
    let columns = COLUMNS.split(", ").map(|c| format!("v.{c}")).collect::<Vec<_>>().join(", ");
    let mut stmt = conn.prepare(&format!(
        "SELECT {columns} FROM library_versions v JOIN library_items i USING (kind, name)
         WHERE v.state = 'live' AND i.archived_at IS NULL ORDER BY v.kind, v.name"
    ))?;
    let rows = stmt.query_map([], version)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// The packed bundle a digest names.
pub fn blob(conn: &Connection, digest: &str) -> Result<Option<Vec<u8>>> {
    Ok(conn.query_row("SELECT bytes FROM library_blobs WHERE digest = ?1", [digest], |r| r.get(0)).optional()?)
}

fn version(row: &Row) -> rusqlite::Result<Version> {
    let unreadable = |column: usize, what: String| {
        rusqlite::Error::FromSqlConversionFailure(column, rusqlite::types::Type::Text, what.into())
    };
    let kind: String = row.get(0)?;
    let provenance: String = row.get(6)?;
    let state: String = row.get(7)?;
    Ok(Version {
        kind: Kind::parse(&kind).ok_or_else(|| unreadable(0, format!("no kind is called {kind}")))?,
        name: row.get(1)?,
        number: row.get(2)?,
        digest: row.get(3)?,
        size: row.get(4)?,
        description: row.get(5)?,
        provenance: serde_json::from_str(&provenance).map_err(|e| unreadable(6, e.to_string()))?,
        state: State::parse(&state).ok_or_else(|| unreadable(7, format!("no state is called {state}")))?,
        created_at: row.get(8)?,
        decided_at: row.get(9)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::bundle::File;

    fn store() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        ensure(&conn).unwrap();
        conn
    }

    fn skill(name: &str, description: &str) -> Bundle {
        let text = format!("---\nname: {name}\ndescription: {description}\n---\n");
        Bundle::check(Kind::Skill, name, vec![File { path: "SKILL.md".into(), bytes: text.into_bytes(), executable: false }])
            .unwrap()
    }

    fn live_numbers(conn: &Connection) -> Vec<i64> {
        versions(conn, Kind::Skill, "bench").unwrap().iter().filter(|v| v.state == State::Live).map(|v| v.number).collect()
    }

    #[test]
    fn a_stored_version_is_never_rewritten() {
        let db = store();
        let first = publish(&db, &skill("bench", "One."), &Provenance::Upload, true, 1).unwrap().version().clone();
        let packed = blob(&db, &first.digest).unwrap().unwrap();
        assert_eq!(bundle::digest(&packed), first.digest, "a blob is kept under its own digest");

        for sql in [
            "UPDATE library_versions SET digest = 'x'",
            "UPDATE library_versions SET provenance = '{\"from\":\"upload\"}'",
            "UPDATE library_versions SET number = 9",
            "UPDATE library_versions SET description = 'Another.'",
            "UPDATE library_blobs SET bytes = x'00'",
        ] {
            let err = db.execute(sql, []).expect_err(sql).to_string();
            assert!(err.contains("never changes"), "{sql}: {err}");
        }

        // A change is a second version beside the first, which stays as it was.
        let second = publish(&db, &skill("bench", "Two."), &Provenance::Upload, true, 2).unwrap().version().clone();
        assert_eq!(second.number, 2);
        let all = versions(&db, Kind::Skill, "bench").unwrap();
        assert_eq!((all[1].number, all[1].digest.as_str(), all[1].description.as_str()), (1, first.digest.as_str(), "One."));
        assert_eq!(blob(&db, &first.digest).unwrap().unwrap(), packed);
    }

    #[test]
    fn identical_content_makes_no_new_version() {
        let db = store();
        let (one, two) = (skill("bench", "One."), skill("bench", "Two."));
        let first = publish(&db, &one, &Provenance::Upload, true, 1).unwrap();
        assert!(matches!(&first, Published::New(v) if v.number == 1), "{first:?}");

        let again = publish(&db, &one, &Provenance::Token { token: "ci".into() }, true, 2).unwrap();
        assert!(
            matches!(&again, Published::Unchanged(v) if v.number == 1 && v.provenance == Provenance::Upload),
            "the newest version is answered as it was: {again:?}"
        );
        assert_eq!(versions(&db, Kind::Skill, "bench").unwrap().len(), 1);

        assert!(matches!(publish(&db, &two, &Provenance::Upload, true, 3).unwrap(), Published::New(v) if v.number == 2));
        // Going back to the first content is a change, and so a version, sharing its blob.
        let back = publish(&db, &one, &Provenance::Upload, true, 4).unwrap();
        assert!(matches!(&back, Published::New(v) if v.number == 3 && v.digest == first.version().digest), "{back:?}");
        let blobs: i64 = db.query_row("SELECT count(*) FROM library_blobs", [], |r| r.get(0)).unwrap();
        assert_eq!(blobs, 2);
    }

    #[test]
    fn a_version_records_its_provenance() {
        let db = store();
        let all = [
            Provenance::Upload,
            Provenance::Forge {
                source: "skills".into(),
                forge: "gitlab".into(),
                repository: "tools/skills".into(),
                commit: "0123abcd".into(),
                path: "skills/bench".into(),
            },
            Provenance::Token { token: "ci-publisher".into() },
            Provenance::Machine {
                machine: "5d0c9a1e-0000-4000-8000-000000000001".into(),
                label: Some("bench-mac".into()),
                session: Some("4d5e6f70-0000-4000-8000-000000000002".into()),
            },
            Provenance::Router { build: "tree:0000000000000000000000000000000000000000".into() },
        ];
        // Uploads, tokens and machines share an item; a source and the router each keep one
        // of their own (`not_from_here`).
        for (n, from) in all.iter().enumerate() {
            let name = match from {
                Provenance::Forge { .. } => "imported",
                Provenance::Router { .. } => "built-in",
                _ => "bench",
            };
            let live = n % 2 == 0;
            let stored = publish(&db, &skill(name, &format!("Version {n}.")), from, live, n as i64).unwrap();
            assert_eq!(&stored.version().provenance, from);
            assert_eq!(stored.version().state, if live { State::Live } else { State::Pending });
        }

        let read = versions(&db, Kind::Skill, "bench").unwrap();
        let provenances: Vec<_> = read.iter().rev().map(|v| v.provenance.clone()).collect();
        assert_eq!(provenances, [all[0].clone(), all[2].clone(), all[3].clone()]);
        let states: Vec<_> = read.iter().map(|v| v.state).collect();
        assert_eq!(states, [State::Pending, State::Live, State::Superseded]);
        assert_eq!(versions(&db, Kind::Skill, "imported").unwrap()[0].provenance, all[1]);
        assert_eq!(versions(&db, Kind::Skill, "built-in").unwrap()[0].provenance, all[4]);

        let json = serde_json::to_value(&all[3]).unwrap();
        assert_eq!((json["from"].as_str(), json["session"].as_str()), (Some("machine"), Some("4d5e6f70-0000-4000-8000-000000000002")));
        let bare = serde_json::to_value(Provenance::Machine { machine: "m".into(), label: None, session: None }).unwrap();
        assert_eq!(bare, serde_json::json!({ "from": "machine", "machine": "m" }));
    }

    #[test]
    fn at_most_one_version_of_an_item_is_live() {
        let db = store();
        publish(&db, &skill("bench", "One."), &Provenance::Upload, true, 1).unwrap();
        assert_eq!(live_numbers(&db), [1]);
        publish(&db, &skill("bench", "Two."), &Provenance::Upload, true, 2).unwrap();
        assert_eq!(live_numbers(&db), [2]);
        publish(&db, &skill("bench", "Three."), &Provenance::Upload, false, 3).unwrap();
        assert_eq!(live_numbers(&db), [2], "a pending version leaves the live one live");

        assert_eq!(make_live(&db, Kind::Skill, "bench", 3, 4).unwrap().unwrap().state, State::Live);
        assert_eq!(live_numbers(&db), [3]);
        make_live(&db, Kind::Skill, "bench", 1, 5).unwrap().unwrap();
        assert_eq!(live_numbers(&db), [1], "rolled back");
        make_live(&db, Kind::Skill, "bench", 1, 6).unwrap().unwrap();
        assert_eq!(live_numbers(&db), [1]);
        assert_eq!(make_live(&db, Kind::Skill, "bench", 9, 7).unwrap(), Err(Unmade::Missing));

        let states: Vec<_> = versions(&db, Kind::Skill, "bench").unwrap().iter().map(|v| v.state).collect();
        assert_eq!(states, [State::Superseded, State::Superseded, State::Live]);

        // Not even a write around this module can make a second.
        let err = db.execute("UPDATE library_versions SET state = 'live' WHERE number = 2", []).expect_err("a second live version");
        assert!(err.to_string().contains("UNIQUE"), "{err}");

        // What machines hold: one version per item.
        publish(&db, &skill("vault", "Keeps secrets."), &Provenance::Upload, true, 8).unwrap();
        let held: Vec<_> = live(&db).unwrap().into_iter().map(|v| (v.name, v.number)).collect();
        assert_eq!(held, [("bench".to_string(), 1), ("vault".to_string(), 1)]);
    }

    #[test]
    fn the_library_tables_appear_on_an_existing_store() {
        let dir = std::env::temp_dir().join(format!("ar-library-store-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("agent-router.db");
        {
            // A store from before the Library, with something in it.
            let old = Connection::open(&path).unwrap();
            old.execute_batch(
                "CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL, updated_at INTEGER NOT NULL);
                 INSERT INTO settings VALUES ('bind', '0.0.0.0', 1);",
            )
            .unwrap();
        }
        // Twice: the second start finds them there already.
        for _ in 0..2 {
            drop(crate::db::Db::open(&path).unwrap());
        }

        let conn = Connection::open(&path).unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        let mut names: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE name LIKE 'library_%' ORDER BY name")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        names.retain(|n| !n.starts_with("sqlite_autoindex"));
        assert_eq!(
            names,
            [
                "library_blobs",
                "library_blobs_fixed",
                "library_forge_tokens",
                "library_forges",
                "library_hook_deliveries",
                "library_items",
                "library_machines",
                "library_sources",
                "library_tokens",
                "library_uses",
                "library_uses_name",
                "library_versions",
                "library_versions_fixed",
                "library_versions_one_live"
            ]
        );
        let bind: String = conn.query_row("SELECT value FROM settings WHERE key = 'bind'", [], |r| r.get(0)).unwrap();
        assert_eq!(bind, "0.0.0.0", "what was in the store is still there");
        let stored = publish(&conn, &skill("bench", "One."), &Provenance::Upload, true, 1).unwrap();
        assert_eq!(stored.version().number, 1);
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
