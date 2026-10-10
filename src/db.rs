//! The store. Two SQLite files, one writer (the capture task), WAL mode: the structure
//! in `agent-router.db`, and the captured bodies attached beside it as `bodies_db` —
//! see `attach_bodies` for why they were separated.
//!
//! Design rule from ARCHITECTURE.md: keep the structure, cap the content. Every
//! count, name and size is kept forever; only large text bodies are truncated, and
//! their true size is recorded alongside the truncated copy.

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::{Path, PathBuf};

use crate::analyze::RequestShape;

pub struct Db {
    conn: Connection,
    /// Conversation blocks already written by this process, keyed by where they sat.
    ///
    /// Every request replays the whole history, so without this each of a session's
    /// several hundred requests would re-offer every block that came before it — a few
    /// hundred thousand no-op statements a day. Correctness never rests on it: the
    /// unique hash and the site primary key reject a duplicate either way, and a miss
    /// only costs the statement it would have skipped. Bounded, and cleared wholesale
    /// rather than evicted, because a cold cache is merely slower for a moment.
    seen_blocks: std::collections::HashSet<u64>,
}

/// How many block sites to remember before starting over. At roughly forty bytes a
/// entry this is a few tens of megabytes at worst, against a store measured in gigabytes.
const SEEN_BLOCKS_MAX: usize = 1_000_000;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS accounts (
  id                TEXT PRIMARY KEY,
  label             TEXT NOT NULL,
  source            TEXT NOT NULL DEFAULT 'keychain',
  profile_dir       TEXT,
  email             TEXT,
  org_name          TEXT,
  subscription_type TEXT,
  active            INTEGER NOT NULL DEFAULT 0,
  created_at        INTEGER NOT NULL
);

-- A provider's own reading of an account's windows, from its usage API rather than from
-- response headers (Kimi Code's `/v1/usages`; `quota`). Utilisation is 0..1 and a reset is
-- epoch seconds, as the `rl_*` columns of `requests` hold them.
CREATE TABLE IF NOT EXISTS quota_readings (
  account_id      TEXT NOT NULL,
  at              INTEGER NOT NULL,
  five_hour_used  REAL,
  five_hour_reset INTEGER,
  second_window   TEXT,
  second_used     REAL,
  second_reset    INTEGER
);
CREATE INDEX IF NOT EXISTS quota_readings_account ON quota_readings(account_id, at);

-- A window a provider said is spent, and until when (`quota::kimi_spent_window`): Kimi Code
-- answers a spent window with a 403 naming it, and sends no header that says so.
CREATE TABLE IF NOT EXISTS spent_windows (
  account_id TEXT NOT NULL,
  window     TEXT NOT NULL,
  at         INTEGER NOT NULL,
  until      INTEGER NOT NULL,
  message    TEXT,
  PRIMARY KEY (account_id, window)
);

-- Which account a model's requests go to, when not its provider's active one (`models`).
CREATE TABLE IF NOT EXISTS model_assignments (
  model      TEXT PRIMARY KEY,
  account_id TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS projects (
  cwd           TEXT PRIMARY KEY,
  label         TEXT,
  first_seen_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS sessions (
  id           TEXT PRIMARY KEY,
  cwd          TEXT,
  git_branch   TEXT,
  entrypoint   TEXT,
  started_at   INTEGER NOT NULL,
  last_seen_at INTEGER NOT NULL,
  label        TEXT
);

-- Where a machine has a checkout, and the repository its remotes name (`places`,
-- `repository`). `machine` is '' for a local router's own machine; `remotes` is JSON
-- [{name, url, repository}], every URL without its credentials.
CREATE TABLE IF NOT EXISTS directories (
  machine    TEXT NOT NULL,
  cwd        TEXT NOT NULL,
  remotes    TEXT NOT NULL,
  repository TEXT,
  changed_at INTEGER NOT NULL,
  PRIMARY KEY (machine, cwd)
);

-- The machines that have reported their places, by the id their agent keeps.
CREATE TABLE IF NOT EXISTS machines (
  id           TEXT PRIMARY KEY,
  label        TEXT,
  platform     TEXT,
  last_seen_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS requests (
  id                INTEGER PRIMARY KEY AUTOINCREMENT,
  request_id        TEXT UNIQUE,
  message_id        TEXT,
  session_id        TEXT,
  account_id        TEXT,
  model             TEXT,
  method            TEXT NOT NULL,
  path              TEXT NOT NULL,
  started_at        INTEGER NOT NULL,
  ttfb_ms           INTEGER,
  duration_ms       INTEGER,
  status            INTEGER,
  streaming         INTEGER,
  stop_reason       TEXT,
  service_tier      TEXT,
  is_sidechain      INTEGER,
  agent_id          TEXT,
  error             TEXT,
  rl_status         TEXT,
  rl_5h_utilization REAL,
  rl_7d_utilization REAL,
  rl_5h_reset       INTEGER,
  rl_7d_reset       INTEGER,
  request_bytes     INTEGER,
  response_bytes    INTEGER
);
CREATE INDEX IF NOT EXISTS requests_session ON requests(session_id);
CREATE INDEX IF NOT EXISTS requests_started ON requests(started_at);
-- Everything the forecast and the quota panel ask for is scoped to one account: its
-- spend over a range, and its most recent rate-limit reading. Without these, both fall
-- back to walking `requests_started` across every account's history on every poll.
CREATE INDEX IF NOT EXISTS requests_account_started ON requests(account_id, started_at);
-- Partial, because a rate-limit reading rides on a small minority of requests and the
-- lookup only ever wants the newest one.
CREATE INDEX IF NOT EXISTS requests_rl ON requests(account_id, started_at DESC)
  WHERE rl_status IS NOT NULL;

CREATE TABLE IF NOT EXISTS usage (
  request_id               INTEGER PRIMARY KEY REFERENCES requests(id) ON DELETE CASCADE,
  input_tokens             INTEGER NOT NULL DEFAULT 0,
  output_tokens            INTEGER NOT NULL DEFAULT 0,
  thinking_tokens          INTEGER NOT NULL DEFAULT 0,
  cache_read_tokens        INTEGER NOT NULL DEFAULT 0,
  cache_creation_5m_tokens INTEGER NOT NULL DEFAULT 0,
  cache_creation_1h_tokens INTEGER NOT NULL DEFAULT 0,
  prompt_tokens            INTEGER NOT NULL DEFAULT 0,
  cost                     REAL
);

-- Per-request split of the context window. char_count is exact; est_tokens is the
-- API's exact prompt total apportioned by character share. See ARCHITECTURE.md.
CREATE TABLE IF NOT EXISTS context (
  request_id INTEGER NOT NULL REFERENCES requests(id) ON DELETE CASCADE,
  component  TEXT NOT NULL,
  char_count INTEGER NOT NULL,
  est_tokens INTEGER NOT NULL,
  PRIMARY KEY (request_id, component)
);

-- Tool results, deduplicated by tool_use_id. Every request in a session replays the
-- whole history, so without this key a single tool result would be counted once per
-- subsequent request and a long session would report wildly inflated tool costs.
CREATE TABLE IF NOT EXISTS tool_calls (
  tool_use_id      TEXT PRIMARY KEY,
  session_id       TEXT,
  first_request_id INTEGER,
  name             TEXT,
  result_chars     INTEGER NOT NULL DEFAULT 0,
  est_tokens       INTEGER NOT NULL DEFAULT 0,
  is_error         INTEGER NOT NULL DEFAULT 0,
  seen_at          INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS tool_calls_session ON tool_calls(session_id);

-- Tool calls the assistant *made* in this response, in order.
CREATE TABLE IF NOT EXISTS tool_invocations (
  id         INTEGER PRIMARY KEY AUTOINCREMENT,
  request_id INTEGER NOT NULL REFERENCES requests(id) ON DELETE CASCADE,
  ordinal    INTEGER NOT NULL,
  name       TEXT NOT NULL
);

-- Router-level settings that must outlive a restart and have no better home.
-- Today: the password hash (see auth.rs). Never a credential.
CREATE TABLE IF NOT EXISTS settings (
  key        TEXT PRIMARY KEY,
  value      TEXT NOT NULL,
  updated_at INTEGER NOT NULL
);

-- Phones that asked to be told when a session is waiting on a person. `platform` says
-- whose push service the token belongs to, and so which of the two the router speaks to
-- for this row; `router_ref` is what the phone calls this router, echoed back so a tap
-- opens the right session in the right place.
CREATE TABLE IF NOT EXISTS push_devices (
  token         TEXT PRIMARY KEY,
  platform      TEXT NOT NULL DEFAULT 'apns',
  name          TEXT,
  environment   TEXT NOT NULL DEFAULT 'development',
  router_ref    TEXT,
  registered_at INTEGER NOT NULL,
  last_seen_at  INTEGER NOT NULL,
  last_result   TEXT
);

"#;

/// Where the bodies live: beside the main store, under its own name.
pub fn bodies_path(main: &Path) -> PathBuf {
    let stem = main.file_stem().and_then(|s| s.to_str()).unwrap_or("agent-router");
    main.with_file_name(format!("{stem}-bodies.db"))
}

/// Captured request and response bodies, kept in their own file.
///
/// They are by far the biggest thing here — on a real store, nineteen parts in twenty —
/// and nothing reads them back: they are captured against a future need, and every query
/// the dashboard runs is over the structure around them. Holding them in the main file
/// bought nothing and cost everything, since a store measured in gigabytes is slower to
/// check-point, back up and vacuum than one measured in tens of megabytes.
///
/// The row loses its `REFERENCES requests(id) ON DELETE CASCADE`: a foreign key cannot
/// span two database files. Nothing deletes a request today, so there is no cascade to
/// lose — but anything that starts deleting them has to clear both files.
pub(crate) fn attach_bodies(conn: &Connection, main: &Path) -> Result<()> {
    let side = bodies_path(main);
    conn.execute(
        "ATTACH DATABASE ?1 AS bodies_db",
        [side.to_string_lossy().as_ref()],
    )
    .with_context(|| format!("attaching {}", side.display()))?;
    conn.pragma_update(Some("bodies_db"), "journal_mode", "WAL")?;
    conn.pragma_update(Some("bodies_db"), "synchronous", "NORMAL")?;
    create_bodies_table(conn)?;
    Ok(())
}

/// The sidecar's one table. Created by whoever opens the file first — the router at
/// startup, or the migration on its own connection — so neither depends on the other.
fn create_bodies_table(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS bodies_db.bodies (
           request_id   INTEGER NOT NULL,
           direction    TEXT NOT NULL,
           content      BLOB,
           truncated_at INTEGER,
           true_bytes   INTEGER NOT NULL,
           PRIMARY KEY (request_id, direction)
         );",
    )?;
    create_search_tables(conn)?;
    Ok(())
}

/// The search index, in the sidecar because it is content, not structure.
///
/// `blocks` holds each distinct piece of conversation text exactly once; `block_sites`
/// says where every copy of it appeared. That split is the whole design: a session's
/// requests replay the same history end to end, so the text is written once and the
/// sites — cheap rows of integers — carry the repetition.
///
/// `blocks_fts` is an external-content index: it stores the terms and reads the text
/// back out of `blocks`, so the corpus is not held twice. Blocks are only ever inserted,
/// never updated or deleted, which is why one `AFTER INSERT` trigger is the whole of the
/// synchronisation — there is no `_bd`/`_bu` pair to keep honest.
///
/// The tokenizer keeps `_` inside a token so `body_cap` and `seen_blocks` survive as
/// single terms; `.` and `-` stay separators, so `analyze.rs` is found by the phrase
/// "analyze rs" and by either half on its own.
fn create_search_tables(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        r#"CREATE TABLE IF NOT EXISTS bodies_db.blocks (
             id    INTEGER PRIMARY KEY,
             hash  BLOB NOT NULL UNIQUE,
             kind  TEXT NOT NULL,
             chars INTEGER NOT NULL,
             text  TEXT NOT NULL
           );

           CREATE TABLE IF NOT EXISTS bodies_db.block_sites (
             block_id   INTEGER NOT NULL,
             session_id TEXT NOT NULL,
             ordinal    INTEGER NOT NULL,
             role       TEXT NOT NULL,
             request_id INTEGER NOT NULL,
             at         INTEGER NOT NULL,
             PRIMARY KEY (block_id, session_id, ordinal)
           );
           CREATE INDEX IF NOT EXISTS bodies_db.block_sites_session
             ON block_sites(session_id, ordinal);

           CREATE VIRTUAL TABLE IF NOT EXISTS bodies_db.blocks_fts USING fts5(
             text,
             content='blocks',
             content_rowid='id',
             tokenize="unicode61 remove_diacritics 2 tokenchars '_'"
           );

           CREATE TRIGGER IF NOT EXISTS bodies_db.blocks_ai AFTER INSERT ON blocks BEGIN
             INSERT INTO blocks_fts(rowid, text) VALUES (new.id, new.text);
           END;"#,
    )?;
    Ok(())
}

/// Move whatever the old single-file store already captured, once.
///
/// **Only safe when this process is the only one on the store.** A deploy hands over by
/// letting the outgoing router keep serving until the incoming one is accepting, so for
/// a few seconds two processes share this file — and the outgoing one is still inserting
/// into `main.bodies`. Dropping that table, or vacuuming around it, while it is being
/// written loses those rows and fails its writes. So this runs from a background task
/// that waits for the incumbent to exit first; until it does, new bodies simply go to the
/// sidecar and the old ones sit where they are, which nothing reads either way.
///
/// Idempotent: if it cannot finish — the lock is held, the process dies — `main.bodies`
/// is still there and the next start tries again.
pub fn migrate_bodies_if_needed(path: &Path) -> Result<()> {
    let conn = Connection::open(path)?;
    // The capture task and the dashboard's readers are live on this file; wait for them
    // rather than giving up the moment one of them holds the write lock.
    conn.busy_timeout(std::time::Duration::from_secs(30))?;
    conn.execute(
        "ATTACH DATABASE ?1 AS bodies_db",
        [bodies_path(path).to_string_lossy().as_ref()],
    )?;
    create_bodies_table(&conn)?;
    migrate_bodies(&conn, path)
}

/// Copied in batches so the write-ahead log stays a sensible size, then dropped and
/// vacuumed, which is what actually hands the disk space back.
fn migrate_bodies(conn: &Connection, path: &Path) -> Result<()> {
    let present: bool = conn.query_row(
        "SELECT count(*) FROM main.sqlite_master WHERE type = 'table' AND name = 'bodies'",
        [],
        |r| r.get::<_, i64>(0),
    )? > 0;
    if !present {
        return Ok(());
    }
    let total: i64 = conn.query_row("SELECT count(*) FROM main.bodies", [], |r| r.get(0))?;
    if total > 0 {
        tracing::info!(rows = total, "moving captured bodies into their own file");
        let mut after: i64 = 0;
        loop {
            let moved = conn.execute(
                "INSERT OR IGNORE INTO bodies_db.bodies
                     (request_id, direction, content, truncated_at, true_bytes)
                 SELECT request_id, direction, content, truncated_at, true_bytes
                 FROM main.bodies WHERE rowid > ?1 ORDER BY rowid LIMIT 200",
                [after],
            )?;
            if moved == 0 {
                break;
            }
            after = conn.query_row(
                "SELECT max(rowid) FROM (SELECT rowid FROM main.bodies
                 WHERE rowid > ?1 ORDER BY rowid LIMIT 200)",
                [after],
                |r| r.get(0),
            )?;
        }
    }
    conn.execute_batch("DROP TABLE main.bodies")?;
    // Reclaiming a gigabyte or more, so worth the one-off pause. VACUUM cannot run
    // inside a transaction, which is why this sits outside any of them.
    tracing::info!("vacuuming the main store now the bodies have left it");
    conn.execute_batch("VACUUM main")?;
    // In WAL mode the vacuum lands in the log, and the database file keeps its old size
    // until a checkpoint truncates it — so without this the store reads as small through
    // SQLite while still holding gigabytes on disk. Blocks behind live readers up to the
    // busy timeout; if it loses that race the pages are still free and the next
    // checkpoint gets them, so a busy result is not a failure.
    let before = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    if let Err(err) = conn.execute_batch("PRAGMA main.wal_checkpoint(TRUNCATE)") {
        tracing::warn!(%err, "could not truncate the log; the space comes back at the next checkpoint");
    }
    let after = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    tracing::info!(
        from_mb = before / 1_048_576,
        to_mb = after / 1_048_576,
        "the main store has given the space back"
    );
    Ok(())
}

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)
            .with_context(|| format!("opening {}", path.display()))?;
        // A router starting beside the one it replaces — a handover on a laptop, a rolling
        // update in a pod — opens and migrates the store while the other is writing to it.
        // Wait as the other connections on this file do, not rusqlite's 5 s.
        conn.busy_timeout(std::time::Duration::from_secs(30))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.execute_batch(SCHEMA)?;
        crate::library::store::ensure(&conn)?;
        // Lightweight forward migration so an existing store upgrades in place.
        ensure_column(&conn, "requests", "agent_id", "TEXT")?;
        ensure_column(&conn, "requests", "account_email", "TEXT")?;
        // The machine that listed a session and the repository it worked in (`places`), and
        // the key every read takes a project by: its repository, else its directory.
        ensure_column(&conn, "sessions", "machine", "TEXT")?;
        ensure_column(&conn, "sessions", "repository", "TEXT")?;
        ensure_column(&conn, "sessions", "project", "TEXT GENERATED ALWAYS AS (coalesce(repository, cwd)) VIRTUAL")?;
        conn.execute_batch("CREATE INDEX IF NOT EXISTS sessions_project ON sessions(project)")?;
        // Every phone registered before there was a second push service was an iPhone,
        // which is exactly what the default backfills.
        ensure_column(&conn, "push_devices", "platform", "TEXT NOT NULL DEFAULT 'apns'")?;
        // The provider of the account that served a request, kept on the request so it
        // outlives the account. Every request recorded before there was a second provider
        // was Anthropic's; the API reads a missing one as that.
        ensure_column(&conn, "requests", "provider", "TEXT")?;
        // The coding clients a machine said it has (`clients`), as JSON.
        ensure_column(&conn, "machines", "clients", "TEXT")?;
        attach_bodies(&conn, path)?;
        for (col, ty) in [
            ("source", "TEXT NOT NULL DEFAULT 'keychain'"),
            ("profile_dir", "TEXT"),
            ("email", "TEXT"),
            ("org_name", "TEXT"),
            // Whether a claude.ai session key is stored for this account. The key
            // itself stays in the Keychain; this is only so the UI can say whether
            // one is set without a Keychain read per account per poll.
            ("usage_key", "INTEGER NOT NULL DEFAULT 0"),
            // Which service the account belongs to (`accounts::Provider`). Every account
            // stored before there was a second one was Anthropic's, which is exactly what
            // the default backfills.
            ("provider", "TEXT NOT NULL DEFAULT 'anthropic'"),
            // Where its requests go, when not its provider's default.
            ("base_url", "TEXT"),
        ] {
            ensure_column(&conn, "accounts", col, ty)?;
        }
        Ok(Self { conn, seen_blocks: Default::default() })
    }

    /// Record the account Claude Code is already logged into. Called once at startup;
    /// it never steals `active` from an account the user has chosen.
    ///
    /// It is always Claude Code's own login. Adding an account under its name once wrote
    /// over it — a new "Default" made it a token account, signing with the token that add
    /// stored under its id — and adding no longer can (`accounts::Store::add`), but a store
    /// an add had reached stayed that way: this put back only its name and who it is. Its
    /// source and profile directory are put back as well, and a token stored under its id
    /// is forgotten, its usage key kept. Nothing signs with that token once the account is
    /// Claude Code's again, and nothing could remove it: the built-in account cannot be
    /// removed, nor its token replaced. Every other account is left as it is.
    pub fn upsert_default_account(
        &self,
        id: &str,
        label: &str,
        status: &crate::accounts::AuthStatus,
    ) -> Result<()> {
        let restored = self.conn.execute(
            "UPDATE accounts SET source = 'keychain', provider = 'anthropic', base_url = NULL,
                                 profile_dir = NULL
              WHERE id = ?1 AND (source != 'keychain' OR provider != 'anthropic'
                                 OR base_url IS NOT NULL OR profile_dir IS NOT NULL)",
            [id],
        )?;
        if restored > 0 {
            tracing::warn!(account = id, "the built-in account had been turned into another kind by an add under its name; it is Claude Code's login again");
        }
        if crate::accounts::has_token(id) {
            // Not a reason to stop: the router signs as it should without this.
            match crate::accounts::forget_token(id) {
                Ok(()) => tracing::warn!(account = id, "forgot the token an add under the built-in account's name stored; add it again under another name if it is still wanted"),
                Err(e) => tracing::warn!(account = id, "could not forget the token an add under the built-in account's name stored: {e:#}"),
            }
        }
        self.conn.execute(
            "INSERT INTO accounts (id, label, source, email, org_name, subscription_type,
                                   active, created_at)
             VALUES (?1, ?2, 'keychain', ?3, ?4, ?5,
                     CASE WHEN (SELECT count(*) FROM accounts
                                 WHERE active = 1 AND provider = 'anthropic') = 0 THEN 1 ELSE 0 END,
                     ?6)
             ON CONFLICT(id) DO UPDATE SET
               label = ?2, email = ?3, org_name = ?4, subscription_type = ?5",
            params![
                id,
                label,
                status.email,
                status.org_name,
                status.subscription_type,
                now_ms()
            ],
        )?;
        Ok(())
    }

    pub fn touch_session(
        &self,
        id: &str,
        cwd: Option<&str>,
        branch: Option<&str>,
        entrypoint: Option<&str>,
    ) -> Result<()> {
        let ts = now_ms();
        if let Some(cwd) = cwd {
            self.conn.execute(
                "INSERT INTO projects (cwd, first_seen_at) VALUES (?1, ?2)
                 ON CONFLICT(cwd) DO NOTHING",
                params![cwd, ts],
            )?;
        }
        self.conn.execute(
            "INSERT INTO sessions (id, cwd, git_branch, entrypoint, started_at, last_seen_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?5)
             ON CONFLICT(id) DO UPDATE SET
               last_seen_at = ?5,
               cwd          = COALESCE(?2, cwd),
               git_branch   = COALESCE(?3, git_branch),
               entrypoint   = COALESCE(?4, entrypoint)",
            params![id, cwd, branch, entrypoint, ts],
        )?;
        Ok(())
    }

    /// Write one completed request and everything derived from it, atomically.
    pub fn record(&mut self, ev: &crate::capture::CaptureEvent) -> Result<()> {
        let tx = self.conn.transaction()?;

        tx.execute(
            "INSERT INTO requests (
               request_id, message_id, session_id, account_id, model, method, path,
               started_at, ttfb_ms, duration_ms, status, streaming, stop_reason,
               service_tier, error, rl_status, rl_5h_utilization, rl_7d_utilization,
               rl_5h_reset, rl_7d_reset, request_bytes, response_bytes,
               agent_id, is_sidechain, account_email, provider)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24,?25,?26)
             ON CONFLICT(request_id) DO NOTHING",
            params![
                ev.request_id,
                ev.message_id,
                ev.session_id,
                ev.account_id,
                ev.model,
                ev.method,
                ev.path,
                ev.started_at_ms,
                ev.ttfb_ms,
                ev.duration_ms,
                ev.status,
                ev.streaming as i64,
                ev.stop_reason,
                ev.service_tier,
                ev.error,
                ev.rl.status,
                ev.rl.utilization_5h,
                ev.rl.utilization_7d,
                ev.rl.reset_5h,
                ev.rl.reset_7d,
                ev.request_bytes,
                ev.response_bytes,
                ev.agent_id,
                ev.agent_id.is_some() as i64,
                ev.account_email,
                ev.provider.as_str(),
            ],
        )?;
        // A window Kimi Code said is spent: the account is spent until that window resets,
        // as the quota reader last gave it, else the window's length from now.
        if ev.provider == crate::accounts::Provider::Kimi && ev.status == 403 {
            if let Some(window) = ev.error.as_deref().and_then(crate::quota::kimi_spent_window) {
                let at = ev.started_at_ms;
                let (reset_col, name_check) = match window {
                    "five_hour" => ("five_hour_reset", None),
                    other => ("second_reset", Some(other)),
                };
                let kept: Option<i64> = tx
                    .query_row(
                        &format!(
                            "SELECT {reset_col} FROM quota_readings
                             WHERE account_id = ?1 AND (?2 IS NULL OR second_window = ?2) AND {reset_col} > ?3
                             ORDER BY at DESC LIMIT 1"
                        ),
                        params![ev.account_id, name_check, at / 1000],
                        |r| r.get(0),
                    )
                    .optional()?
                    .flatten();
                let span = match window {
                    "five_hour" => crate::forecast::FIVE_HOUR.span_ms,
                    "seven_day" => crate::forecast::SEVEN_DAY.span_ms,
                    _ => crate::forecast::MONTH.span_ms,
                };
                let until = kept.unwrap_or((at + span) / 1000);
                tx.execute(
                    "INSERT INTO spent_windows (account_id, window, at, until, message) VALUES (?1, ?2, ?3, ?4, ?5)
                     ON CONFLICT(account_id, window) DO UPDATE SET at = excluded.at, until = excluded.until, message = excluded.message",
                    params![ev.account_id, window, at, until, ev.error],
                )?;
            }
        }
        let row_id = tx.last_insert_rowid();
        if row_id == 0 {
            tx.commit()?;
            return Ok(()); // duplicate request-id; already recorded
        }

        // Price a model the first time it is seen, so today's new model is costed on the
        // next query rather than after the next restart. See `pricing::resolve_one`.
        if let Some(model) = ev.model.as_deref() {
            crate::pricing::resolve_one(&tx, model)?;
        }

        let u = &ev.usage;
        tx.execute(
            "INSERT INTO usage (request_id, input_tokens, output_tokens, thinking_tokens,
               cache_read_tokens, cache_creation_5m_tokens, cache_creation_1h_tokens, prompt_tokens)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            params![
                row_id,
                u.input_tokens,
                u.output_tokens,
                u.thinking_tokens,
                u.cache_read_tokens,
                u.cache_creation_5m_tokens,
                u.cache_creation_1h_tokens,
                u.prompt_tokens(),
            ],
        )?;

        if let Some(shape) = &ev.shape {
            for (component, chars) in context_components(shape) {
                let est = apportion(chars, shape_total_chars(shape), u.prompt_tokens());
                tx.execute(
                    "INSERT INTO context (request_id, component, char_count, est_tokens)
                     VALUES (?1,?2,?3,?4)",
                    params![row_id, component, chars, est],
                )?;
            }
            for tr in &shape.tool_results {
                if tr.tool_use_id.is_empty() {
                    continue;
                }
                tx.execute(
                    "INSERT INTO tool_calls (tool_use_id, session_id, first_request_id, name,
                       result_chars, est_tokens, is_error, seen_at)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8)
                     ON CONFLICT(tool_use_id) DO NOTHING",
                    params![
                        tr.tool_use_id,
                        ev.session_id,
                        row_id,
                        tr.name,
                        tr.result_chars,
                        tr.result_chars / 4, // rough; see ARCHITECTURE.md on estimates
                        tr.is_error as i64,
                        now_ms(),
                    ],
                )?;
            }
        }

        for (i, name) in ev.tool_calls.iter().enumerate() {
            tx.execute(
                "INSERT INTO tool_invocations (request_id, ordinal, name) VALUES (?1,?2,?3)",
                params![row_id, i as i64, name],
            )?;
        }

        // The conversation, split into blocks and stored once each. Skipped when the
        // request carries no session: a block with nowhere to have been is not findable
        // and not worth a row.
        // The uses of the Library's items are taken from the same placing: a block placed
        // here for the first time is a use the requests repeating it will not count again.
        if let (Some(shape), Some(session_id)) = (&ev.shape, ev.session_id.as_deref()) {
            let placed = write_blocks(&tx, &mut self.seen_blocks, &shape.blocks, session_id, row_id, ev.started_at_ms)?;
            crate::library::usage::note(&tx, &shape.blocks, &placed, session_id, ev.started_at_ms)?;
        }

        // Only the response. The request body used to be kept here capped at
        // `body_cap`, which — because `messages` is its first field — preserved the
        // oldest part of a conversation and dropped the newest, and which nothing ever
        // read back. What was worth keeping in it is now in `blocks`, uncapped and
        // stored once instead of once per replay, and `requests.request_bytes` still
        // records the size. Rows written by older builds stay where they are.
        let body = &ev.response_body;
        tx.execute(
            "INSERT INTO bodies_db.bodies
                 (request_id, direction, content, truncated_at, true_bytes)
             VALUES (?1,'response',?2,?3,?4)",
            params![
                row_id,
                body.stored.as_slice(),
                if body.truncated { Some(body.stored.len() as i64) } else { None },
                body.true_bytes,
            ],
        )?;

        tx.commit()?;
        Ok(())
    }
}

/// Write a request's blocks and the places they sat, and answer the blocks placed for the
/// first time: each one's index in `blocks`, and its id.
///
/// Shared by the live capture path and the backfill so the two cannot drift on what a
/// block is or how a repeat is refused. Neither correctness nor completeness rests on
/// `seen`: the unique hash and the site primary key reject a duplicate anyway, and a
/// cache miss costs only the statement it would have saved.
fn write_blocks(
    tx: &Connection,
    seen: &mut std::collections::HashSet<u64>,
    blocks: &[crate::analyze::Block],
    session_id: &str,
    request_row_id: i64,
    at: i64,
) -> Result<Vec<(usize, i64)>> {
    if seen.len() > SEEN_BLOCKS_MAX {
        seen.clear();
    }
    let mut placed = Vec::new();
    for (index, b) in blocks.iter().enumerate() {
        if !seen.insert(site_key(session_id, b.ordinal, &b.hash)) {
            continue; // this exact block, at this spot, is already in
        }
        let fresh = tx.execute(
            "INSERT INTO bodies_db.blocks (hash, kind, chars, text)
             VALUES (?1,?2,?3,?4) ON CONFLICT(hash) DO NOTHING",
            params![&b.hash[..], b.kind, b.chars, b.text],
        )?;
        // `last_insert_rowid` is only this block's when the insert took; on a conflict
        // it still holds whatever was written before it.
        let block_id: i64 = if fresh == 1 {
            tx.last_insert_rowid()
        } else {
            tx.query_row(
                "SELECT id FROM bodies_db.blocks WHERE hash = ?1",
                params![&b.hash[..]],
                |r| r.get(0),
            )?
        };
        let sited = tx.execute(
            "INSERT INTO bodies_db.block_sites
                 (block_id, session_id, ordinal, role, request_id, at)
             VALUES (?1,?2,?3,?4,?5,?6) ON CONFLICT DO NOTHING",
            params![block_id, session_id, b.ordinal, b.role, request_row_id, at],
        )?;
        if sited == 1 {
            placed.push((index, block_id));
        }
    }
    Ok(placed)
}

/// How far the backfill has read. Kept in `settings` so it survives a restart and can
/// pick up where it stopped rather than starting the whole store again.
const BACKFILL_KEY: &str = "blocks_backfill_after";

/// Index the request bodies that were captured before there was an index.
///
/// **Must run while those bodies still exist.** They are the only record of the older
/// conversations; once request bodies stop being stored there is nothing left to read.
///
/// Most of what it reads is truncated — on a real store, 98.7% of request bodies were
/// cut at `body_cap` — so it goes through `blocks_from_stored_body`, which recovers the
/// messages that closed before the cut and drops the partial tail. That makes this a
/// partial recovery by nature: the opening of every session comes back, and with it
/// nearly all assistant text, since each reply is replayed in the following request.
/// The middles of long sessions were never captured and cannot be conjured.
///
/// Resumable and idempotent. It runs in small transactions on its own connection with a
/// busy timeout and a pause between batches, because the capture task is writing to the
/// same file for live traffic and must not be made to wait behind housekeeping.
pub fn backfill_blocks(path: &Path) -> Result<()> {
    let conn = Connection::open(path)?;
    conn.busy_timeout(std::time::Duration::from_secs(30))?;
    attach_bodies(&conn, path)?;

    let mut after: i64 = conn
        .query_row("SELECT value FROM settings WHERE key = ?1", [BACKFILL_KEY], |r| {
            r.get::<_, String>(0)
        })
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);

    let ceiling: i64 = conn.query_row(
        "SELECT coalesce(max(request_id), 0) FROM bodies_db.bodies WHERE direction = 'request'",
        [],
        |r| r.get(0),
    )?;
    if after >= ceiling {
        return Ok(());
    }
    tracing::info!(from = after, to = ceiling, "indexing the bodies captured before the index");

    let started = std::time::Instant::now();
    let mut seen = std::collections::HashSet::new();
    let (mut done, blocks_before) = (0i64, current_block_count(&conn)?);
    loop {
        let batch: Vec<(i64, Option<String>, i64, Vec<u8>)> = {
            let mut stmt = conn.prepare(
                "SELECT b.request_id, r.session_id, r.started_at, b.content
                   FROM bodies_db.bodies b
                   JOIN main.requests r ON r.id = b.request_id
                  WHERE b.direction = 'request' AND b.request_id > ?1 AND b.request_id <= ?2
                  ORDER BY b.request_id LIMIT 200",
            )?;
            let rows = stmt
                .query_map(params![after, ceiling], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get::<_, Vec<u8>>(3)?))
                })?
                .collect::<std::result::Result<_, _>>()?;
            rows
        };
        if batch.is_empty() {
            break;
        }

        // Unchecked because the read above borrows `conn` to prepare its statement;
        // there is no nesting here, only one transaction open at a time.
        let tx = conn.unchecked_transaction()?;
        for (request_id, session_id, at, content) in &batch {
            after = (*request_id).max(after);
            // A request with no session has nowhere to be found, and an empty body has
            // nothing in it; both still move the watermark past themselves.
            let Some(session_id) = session_id.as_deref() else { continue };
            if content.is_empty() {
                continue;
            }
            let blocks = crate::analyze::blocks_from_stored_body(content);
            write_blocks(&tx, &mut seen, &blocks, session_id, *request_id, *at)?;
        }
        tx.execute(
            "INSERT INTO settings (key, value, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
            params![BACKFILL_KEY, after.to_string(), now_ms()],
        )?;
        tx.commit()?;

        done += batch.len() as i64;
        if done % 5000 == 0 {
            tracing::info!(done, at = after, "backfill progress");
        }
        // Housekeeping yields to live traffic: the capture task writes to this same
        // file, and a writer that never pauses makes every capture wait behind it.
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    tracing::info!(
        requests = done,
        blocks_added = current_block_count(&conn)? - blocks_before,
        secs = started.elapsed().as_secs(),
        "backfill complete"
    );
    Ok(())
}

const SESSION_CWDS_KEY: &str = "session_cwds_from_opening";

/// Put right the directory of the sessions recorded before the router read Claude Code
/// 2.1.292's environment, which arrives in a `system` message of its own
/// (`analyze::environment_of`). Until then such a session had none, or whatever a request
/// that is not one of its turns quoted (`capture::speaks_for_its_session`). Their opening
/// blocks are in the index, uncapped, so the directory is read from there with the same
/// rules: from the main thread's requests, before the first reply. Once: from this build
/// on, every turn writes what its opening says. Runs after `backfill_blocks`, whose blocks
/// these are.
pub fn backfill_session_cwds(path: &Path) -> Result<usize> {
    let conn = Connection::open(path)?;
    conn.busy_timeout(std::time::Duration::from_secs(30))?;
    attach_bodies(&conn, path)?;
    let done = conn.query_row("SELECT 1 FROM settings WHERE key = ?1", [SESSION_CWDS_KEY], |_| Ok(())).is_ok();
    if done {
        return Ok(0);
    }

    // The `system` blocks of each session's opening turn, in order: in the messages
    // (ordinal 0 and up), placed by the main thread, and before its first reply.
    let rows: Vec<(String, String)> = {
        let mut stmt = conn.prepare(
            "SELECT s.id, b.text
               FROM main.sessions s
               JOIN bodies_db.block_sites bs ON bs.session_id = s.id AND bs.role = 'system' AND bs.ordinal >= 0
               JOIN main.requests r ON r.id = bs.request_id AND r.agent_id IS NULL
               JOIN bodies_db.blocks b ON b.id = bs.block_id
              WHERE bs.ordinal < coalesce((SELECT min(a.ordinal)
                                             FROM bodies_db.block_sites a
                                             JOIN main.requests ra ON ra.id = a.request_id AND ra.agent_id IS NULL
                                            WHERE a.session_id = s.id AND a.role = 'assistant'), 9223372036854775807)
              ORDER BY s.id, bs.ordinal",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<std::result::Result<_, _>>()?;
        rows
    };
    let mut changed = 0;
    for opening in rows.chunk_by(|a, b| a.0 == b.0) {
        let session = &opening[0].0;
        let texts: Vec<&str> = opening.iter().map(|(_, t)| t.as_str()).collect();
        let Some(cwd) = crate::analyze::environment_in(&texts).0 else { continue };
        conn.execute(
            "INSERT INTO projects (cwd, first_seen_at)
             SELECT ?1, started_at FROM sessions WHERE id = ?2
             ON CONFLICT(cwd) DO NOTHING",
            params![cwd, session],
        )?;
        changed += conn.execute("UPDATE sessions SET cwd = ?1 WHERE id = ?2 AND cwd IS NOT ?1", params![cwd, session])?;
    }
    conn.execute(
        "INSERT INTO settings (key, value, updated_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        params![SESSION_CWDS_KEY, changed.to_string(), now_ms()],
    )?;
    if changed > 0 {
        tracing::info!(sessions = changed, "put right the directory of the sessions recorded before their environment was read");
    }
    Ok(changed)
}

fn current_block_count(conn: &Connection) -> Result<i64> {
    Ok(conn.query_row("SELECT count(*) FROM bodies_db.blocks", [], |r| r.get(0))?)
}

/// Identity of one block *at one spot in one session*, for the replay cache. A hash of
/// a hash: collisions cost a redundant `INSERT OR IGNORE`, never a lost or wrong row.
fn site_key(session_id: &str, ordinal: i64, hash: &[u8; 32]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    session_id.hash(&mut h);
    ordinal.hash(&mut h);
    hash.hash(&mut h);
    h.finish()
}

/// Add a column if the table predates it. Cheap, idempotent, no migration table.
/// `table_xinfo`, because `table_info` leaves out a generated column.
fn ensure_column(conn: &Connection, table: &str, column: &str, ty: &str) -> Result<()> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_xinfo({table})"))?;
    let existing: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<std::result::Result<_, _>>()?;
    if !existing.iter().any(|c| c == column) {
        conn.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {column} {ty}"))?;
    }
    Ok(())
}

fn context_components(s: &RequestShape) -> [(&'static str, i64); 4] {
    [
        ("system", s.system_chars),
        ("tools", s.tools_chars),
        ("history", s.history_chars),
        ("tool_results", s.tool_result_chars),
    ]
}

fn shape_total_chars(s: &RequestShape) -> i64 {
    s.system_chars + s.tools_chars + s.history_chars + s.tool_result_chars
}

/// Apportion an exact token total across components by character share.
fn apportion(chars: i64, total_chars: i64, total_tokens: i64) -> i64 {
    if total_chars <= 0 || total_tokens <= 0 {
        return 0;
    }
    ((chars as i128 * total_tokens as i128) / total_chars as i128) as i64
}

/// A router-level setting, by key. Absent is `None`.
pub fn setting(db_path: &Path, key: &str) -> Option<String> {
    let conn = Connection::open_with_flags(
        db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
    )
    .ok()?;
    conn.query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| r.get(0))
        .ok()
}

pub fn set_setting(db_path: &Path, key: &str, value: &str) -> Result<()> {
    let conn = Connection::open(db_path)?;
    conn.execute(
        "INSERT INTO settings (key, value, updated_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        params![key, value, now_ms()],
    )?;
    Ok(())
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyze::Usage;

    /// A capture carrying just enough to be recorded.
    fn an_event(model: &str) -> crate::capture::CaptureEvent {
        crate::capture::CaptureEvent {
            request_id: Some("req-1".into()),
            message_id: None,
            session_id: Some("sess-1".into()),
            account_email: None,
            agent_id: None,
            account_id: "acct-1".into(),
            provider: crate::accounts::Provider::Anthropic,
            entrypoint: None,
            model: Some(model.into()),
            method: "POST".into(),
            path: "/v1/messages".into(),
            started_at_ms: 1_700_000_000_000,
            ttfb_ms: 10,
            duration_ms: 20,
            status: 200,
            streaming: false,
            stop_reason: None,
            service_tier: None,
            error: None,
            rl: Default::default(),
            usage: Default::default(),
            tool_calls: Vec::new(),
            shape: None,
            response_body: crate::capture::StoredBody {
                stored: b"{\"hello\":1}".to_vec(),
                truncated: false,
                true_bytes: 11,
            },
            request_bytes: 11,
            response_bytes: 0,
        }
    }

    /// The account that served a request, and its provider, are kept on the request, so
    /// the request list says where each one went; a request kept from before there was a
    /// second provider reads as Anthropic's, which it was.
    #[test]
    fn the_request_row_names_the_account_and_provider_that_served_it() {
        let dir = std::env::temp_dir().join(format!("ar-provider-row-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut db = Db::open(&dir.join("store.db")).unwrap();
        crate::pricing::ensure_table(&db.conn).unwrap();

        let claude = an_event("claude-opus-5-5");
        let mut kimi = an_event("k3");
        kimi.request_id = Some("req-kimi".into());
        kimi.account_id = "kimi".into();
        kimi.provider = crate::accounts::Provider::Kimi;
        kimi.started_at_ms += 1_000;
        let mut before = an_event("claude-opus-4-1");
        before.request_id = Some("req-before".into());
        before.started_at_ms -= 1_000;
        for ev in [&claude, &kimi, &before] {
            db.record(ev).unwrap();
        }
        db.conn.execute("UPDATE requests SET provider = NULL WHERE request_id = 'req-before'", []).unwrap();

        let everything = crate::api::Filter { project: None, session: None, account: None, since: None, until: None, tz: None };
        let list = crate::api::requests_of(&db.conn, &everything).unwrap();
        let served: Vec<(&str, &str, &str)> = list
            .as_array()
            .unwrap()
            .iter()
            .map(|r| (r["request_id"].as_str().unwrap(), r["account_id"].as_str().unwrap(), r["provider"].as_str().unwrap()))
            .collect();
        assert_eq!(
            served,
            [("req-kimi", "kimi", "kimi"), ("req-1", "acct-1", "anthropic"), ("req-before", "acct-1", "anthropic")]
        );
        drop(db);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The replay problem in miniature. Two requests one turn apart: the second resends
    /// everything the first did. The text must be written once, the positions twice, and
    /// the whole thing must come back out of the index.
    #[test]
    fn a_replayed_conversation_is_stored_once_and_stays_searchable() {
        let dir = std::env::temp_dir().join(format!("ar-search-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut db = Db::open(&dir.join("store.db")).unwrap();
        crate::pricing::ensure_table(&db.conn).unwrap();

        // The third message repeats the first word for word — same block, second site.
        let first = br#"{"messages":[
            {"role":"user","content":[{"type":"text","text":"where does the kestrel roost"}]}]}"#;
        let again = br#"{"messages":[
            {"role":"user","content":[{"type":"text","text":"where does the kestrel roost"}]},
            {"role":"assistant","content":[{"type":"text","text":"on the cliff ledge, near body_cap"}]},
            {"role":"user","content":[{"type":"text","text":"where does the kestrel roost"}]}]}"#;

        for (n, body) in [(1, &first[..]), (2, &again[..])] {
            let mut ev = an_event("claude-opus-5");
            ev.request_id = Some(format!("req-{n}"));
            ev.shape = crate::analyze::analyze_request(body);
            db.record(&ev).unwrap();
        }

        fn count(db: &Db, sql: &str) -> i64 { db.conn.query_row(sql, [], |r| r.get(0)).unwrap() }
        assert_eq!(count(&db, "SELECT count(*) FROM bodies_db.blocks"), 2, "two distinct texts");
        assert_eq!(
            count(&db, "SELECT count(*) FROM bodies_db.block_sites"),
            3,
            "three places they sat, across both requests"
        );

        // The index must see the block, and find it by a word from the middle of it.
        assert_eq!(
            count(&db, "SELECT count(*) FROM bodies_db.blocks_fts WHERE blocks_fts MATCH 'kestrel'"),
            1
        );
        // An identifier survives the tokenizer whole rather than splitting at the `_`.
        assert_eq!(
            count(&db, "SELECT count(*) FROM bodies_db.blocks_fts WHERE blocks_fts MATCH 'body_cap'"),
            1
        );
        // And the text really is readable back through the external-content index.
        let snip: String = db
            .conn
            .query_row(
                "SELECT snippet(blocks_fts, 0, '[', ']', '…', 8) FROM bodies_db.blocks_fts
                 WHERE blocks_fts MATCH 'cliff'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(snip.contains("[cliff]"), "snippet should mark the hit, got {snip:?}");

        // A third request replaying the same two turns adds nothing at all.
        let mut ev = an_event("claude-opus-5");
        ev.request_id = Some("req-3".into());
        ev.shape = crate::analyze::analyze_request(again);
        db.record(&ev).unwrap();
        assert_eq!(count(&db, "SELECT count(*) FROM bodies_db.blocks"), 2);
        assert_eq!(count(&db, "SELECT count(*) FROM bodies_db.block_sites"), 3);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A session recorded before its `system`-message environment was read gets the
    /// directory its main thread's opening names: in place of none, and in place of a
    /// quoted one. One told only after the first reply, a subagent's own opening, and a
    /// session from an older build are no evidence, and it is done once.
    #[test]
    fn sessions_recorded_without_a_directory_get_it_from_their_opening() {
        let dir = std::env::temp_dir().join(format!("ar-cwds-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("store.db");
        let mut db = Db::open(&path).unwrap();
        crate::pricing::ensure_table(&db.conn).unwrap();

        let env = |p: &str| format!("# Environment\nYou have been invoked in the following environment: \n - Primary working directory: {p}\n");
        let opening = |p: &str| serde_json::json!([
            { "role": "user", "content": "fix it" },
            { "role": "system", "content": [{ "type": "text", "text": env(p) }] },
            { "role": "assistant", "content": "On it." },
        ]);
        let quoted = "/Users/u/code/project\\n - Is a git repository: true\\n\";";
        // (session, stored directory, subagent, messages)
        let requests = [
            ("missed", None, None, opening("/home/u/code/p")),
            ("quoted", Some(quoted), None, opening("/home/u/code/q")),
            ("too-late", None, None, serde_json::json!([
                { "role": "user", "content": "hello" },
                { "role": "assistant", "content": "Hi." },
                { "role": "system", "content": env("/home/u/elsewhere") },
            ])),
            ("forked", None, None, opening("/home/u/code/r")),
            ("forked", None, Some("a1"), opening("/home/u/code/r/.claude/worktrees/a1")),
            ("older", Some("/kept/as/is"), None, serde_json::json!([{ "role": "user", "content": "hi" }])),
        ];
        for (n, (session, cwd, agent, messages)) in requests.iter().enumerate() {
            db.touch_session(session, *cwd, None, None).unwrap();
            let mut ev = an_event("claude-opus-5");
            ev.request_id = Some(format!("req-{n}"));
            ev.session_id = Some(session.to_string());
            ev.agent_id = agent.map(str::to_owned);
            ev.shape = crate::analyze::analyze_request(serde_json::json!({ "messages": messages }).to_string().as_bytes());
            db.record(&ev).unwrap();
        }

        assert_eq!(backfill_session_cwds(&path).unwrap(), 3);
        let cwd = |id: &str| -> Option<String> {
            db.conn.query_row("SELECT cwd FROM sessions WHERE id = ?1", [id], |r| r.get(0)).unwrap()
        };
        assert_eq!(cwd("missed").as_deref(), Some("/home/u/code/p"));
        assert_eq!(cwd("quoted").as_deref(), Some("/home/u/code/q"));
        assert_eq!(cwd("too-late"), None);
        assert_eq!(cwd("forked").as_deref(), Some("/home/u/code/r"), "the main thread's, not the subagent's");
        assert_eq!(cwd("older").as_deref(), Some("/kept/as/is"));
        let listed: i64 = db
            .conn
            .query_row("SELECT count(*) FROM projects WHERE cwd = '/home/u/code/p'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(listed, 1, "and its project is listed");

        db.conn.execute("UPDATE sessions SET cwd = NULL WHERE id = 'missed'", []).unwrap();
        assert_eq!(backfill_session_cwds(&path).unwrap(), 0, "once");
        assert_eq!(cwd("missed"), None);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A store written before bodies had their own file still has them in the main one.
    /// They have to arrive intact, and the main store has to actually give the space back.
    #[test]
    fn bodies_captured_by_an_older_build_move_across_intact() {
        let dir = std::env::temp_dir().join(format!("ar-migrate-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let main = dir.join("store.db");

        // A store as an older build left it: bodies in the main file.
        {
            let conn = Connection::open(&main).unwrap();
            conn.execute_batch(
                "CREATE TABLE bodies (
                   request_id   INTEGER NOT NULL,
                   direction    TEXT NOT NULL,
                   content      BLOB,
                   truncated_at INTEGER,
                   true_bytes   INTEGER NOT NULL,
                   PRIMARY KEY (request_id, direction)
                 );
                 INSERT INTO bodies VALUES (1, 'request',  x'414243', NULL, 3);
                 INSERT INTO bodies VALUES (1, 'response', x'4445',   NULL, 2);
                 INSERT INTO bodies VALUES (2, 'request',  x'46',     7,    99);",
            )
            .unwrap();
            // Enough bulk that the file has real space to give back, which is the whole
            // point of the move and does not happen on its own in WAL mode.
            let blob = vec![b'x'; 64 * 1024];
            for i in 10..210 {
                conn.execute(
                    "INSERT INTO bodies VALUES (?1, 'request', ?2, NULL, ?3)",
                    params![i, blob.as_slice(), blob.len() as i64],
                )
                .unwrap();
            }
        }
        let fat = std::fs::metadata(&main).unwrap().len();
        assert!(fat > 8 * 1024 * 1024, "the store should start large, was {fat} bytes");

        migrate_bodies_if_needed(&main).unwrap();

        let side = Connection::open(dir.join("store-bodies.db")).unwrap();
        let n: i64 = side.query_row("SELECT count(*) FROM bodies", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 203, "every row should have moved");
        let (content, trunc, bytes): (Vec<u8>, Option<i64>, i64) = side
            .query_row(
                "SELECT content, truncated_at, true_bytes FROM bodies
                 WHERE request_id = 2 AND direction = 'request'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(content, vec![0x46]);
        assert_eq!(trunc, Some(7), "the truncation mark must survive the move");
        assert_eq!(bytes, 99, "the true size must survive the move");

        let conn = Connection::open(&main).unwrap();
        let left: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='bodies'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(left, 0, "the main store should have let the table go");

        // And the space is actually back on disk, not merely free inside a file that is
        // still gigabytes wide.
        let slim = std::fs::metadata(&main).unwrap().len();
        assert!(slim < fat / 4, "the store should have shrunk: {fat} -> {slim} bytes");

        // Running again is a no-op rather than an error, which is what makes a deploy
        // that could not finish the move safe to simply repeat.
        migrate_bodies_if_needed(&main).unwrap();
        let still: i64 = side.query_row("SELECT count(*) FROM bodies", [], |r| r.get(0)).unwrap();
        assert_eq!(still, 203);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Bodies live in their own file now, so a capture has to reach across the attached
    /// database to land. Nothing reads them back, which is exactly why a broken write
    /// here would go unnoticed until someone went looking for a body and found none.
    #[test]
    fn a_captured_body_lands_in_the_sidecar_file() {
        let dir = std::env::temp_dir().join(format!("ar-bodies-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let main = dir.join("store.db");

        let mut db = Db::open(&main).unwrap();
        crate::pricing::ensure_table(&db.conn).unwrap();
        db.record(&an_event("claude-opus-5-20260115")).unwrap();

        // The main file must not have grown a bodies table back.
        let in_main: i64 = db
            .conn
            .query_row(
                "SELECT count(*) FROM main.sqlite_master WHERE type='table' AND name='bodies'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(in_main, 0, "bodies must not be recreated in the main store");

        let (content, bytes): (Vec<u8>, i64) = db
            .conn
            .query_row(
                "SELECT content, true_bytes FROM bodies_db.bodies WHERE direction = 'response'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(content, b"{\"hello\":1}".to_vec());
        assert_eq!(bytes, 11);

        // The request body is no longer kept: what was worth reading in it is indexed
        // as blocks, and the capped copy this used to store was never read back.
        let requests: i64 = db
            .conn
            .query_row(
                "SELECT count(*) FROM bodies_db.bodies WHERE direction = 'request'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(requests, 0, "no request body should be written");

        // And it is genuinely a separate file on disk, not a table under another name.
        assert!(dir.join("store-bodies.db").exists(), "sidecar file should exist");

        // The model met for the first time here must have been priced on the way in,
        // or every cost this request contributes to would silently read as zero.
        let cost: f64 = db
            .conn
            .query_row("SELECT cost FROM request_cost", [], |r| r.get(0))
            .unwrap();
        assert!(cost.is_finite(), "the request should be priced, got {cost}");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn apportions_exact_total_by_share() {
        // 1000 exact prompt tokens split across a 4:1 character ratio.
        assert_eq!(apportion(800, 1000, 1000), 800);
        assert_eq!(apportion(200, 1000, 1000), 200);
        // Degenerate inputs never divide by zero.
        assert_eq!(apportion(5, 0, 100), 0);
        assert_eq!(apportion(5, 100, 0), 0);
    }

    #[test]
    fn usage_prompt_total_is_the_sum_of_all_read_tokens() {
        let u = Usage {
            input_tokens: 2,
            cache_read_tokens: 100,
            cache_creation_5m_tokens: 10,
            cache_creation_1h_tokens: 20,
            ..Default::default()
        };
        assert_eq!(u.prompt_tokens(), 132);
    }

    /// A store reached by an add under the built-in account's name, from before adds stopped
    /// replacing accounts: a new "Default" made it a token account, its token stored under
    /// the account's id, and a profile add made it a profile account. The next start makes it
    /// Claude Code's login again, exactly as it was, and forgets the token that add stored,
    /// keeping its usage key, which is its own. No other account changes.
    ///
    /// The built-in account's id is this test's own rather than "default": the secrets file,
    /// which every test in the process shares (`accounts::secrets`), never holds a token
    /// under "default" for another test to find.
    #[test]
    fn built_in_account_restored_at_startup_and_no_other_account_changes() {
        use crate::accounts::{self, Account, AuthStatus, Provider, Source, Store};
        use rusqlite::types::Value;

        // None of these is a credential.
        const STRAY: &str = "sk-ant-oat01-stray-QZ9JX8QZ9JX8QZ9JX8QZ9JX8QZ9JX8QZ9JX8QZ9JX8QZ9JX8QZ9JX8QZ9JX8QZ9JX8QZ9JX8QZ9JX8QZ9";
        const WORK: &str = "sk-ant-oat01-work-JX8QZ9JX8QZ9JX8QZ9JX8QZ9JX8QZ9JX8QZ9JX8QZ9JX8QZ9JX8QZ9JX8QZ9JX8QZ9JX8QZ9JX8QZ9JX8";
        const BUILT_IN_USAGE: &str = "sk-ant-sid01-built-in-QZ9JX8";
        const WORK_USAGE: &str = "sk-ant-sid01-work-JX8QZ9";

        let dir = std::env::temp_dir().join(format!("ar-built-in-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("agent-router.db");
        let db = Db::open(&path).unwrap();
        let store = Store::new(path.clone());
        let id = "built-in-restored-default";
        let me = AuthStatus {
            logged_in: true,
            email: Some("me@example.com".into()),
            subscription_type: Some("max".into()),
            ..Default::default()
        };

        // Every column of the accounts this filter picks, in id order.
        let rows = |filter: &str| -> Vec<Vec<Value>> {
            let mut stmt = db.conn.prepare(&format!("SELECT * FROM accounts WHERE {filter} ORDER BY id")).unwrap();
            let n = stmt.column_count();
            stmt.query_map([id], |r| (0..n).map(|i| r.get::<_, Value>(i)).collect())
                .unwrap()
                .map(Result::unwrap)
                .collect()
        };
        // The token the router would sign with for a token account with this id, and the
        // account's usage key.
        let secrets = |of: &str| {
            let as_token = Account {
                id: of.into(),
                label: String::new(),
                source: Source::Token,
                provider: Provider::Anthropic,
                base_url: None,
                profile_dir: None,
                email: None,
            };
            (accounts::resolve(&as_token).ok().map(|r| r.token), accounts::usage_token(of))
        };

        // The first start, and an account of each other kind beside the built-in one, which
        // is no longer the active one. Its usage key, as `quota::set_usage_token` records it.
        db.upsert_default_account(id, "me@example.com", &me).unwrap();
        store
            .add("built-in-restored-work", "Work", Source::Token, Provider::Anthropic, None, None, &AuthStatus::default(), || {
                accounts::store_token("built-in-restored-work", WORK)
            })
            .unwrap();
        accounts::store_usage_token("built-in-restored-work", WORK_USAGE).unwrap();
        let home = "/nowhere/accounts/built-in-restored-home";
        store.add("built-in-restored-home", "Home", Source::Profile, Provider::Anthropic, None, Some(home), &AuthStatus::default(), || Ok(())).unwrap();
        store.activate("built-in-restored-work").unwrap();
        accounts::store_usage_token(id, BUILT_IN_USAGE).unwrap();
        db.conn.execute("UPDATE accounts SET usage_key = 1 WHERE id = ?1", [id]).unwrap();

        let built_in = rows("id = ?1");
        let others = rows("id != ?1");
        assert_eq!(others.len(), 2);
        let theirs = [secrets("built-in-restored-work"), secrets("built-in-restored-home")];
        assert_eq!(theirs[0], (Some(WORK.to_string()), Some(WORK_USAGE.to_string())));

        let profile = "/nowhere/accounts/built-in-restored-default";
        for (source, profile_dir) in [("token", None), ("profile", Some(profile))] {
            // The add from before, as it wrote over an account whose id it took: the token
            // stored first, then the row upserted over the one there.
            if source == "token" {
                accounts::store_token(id, STRAY).unwrap();
            }
            db.conn
                .execute(
                    "INSERT INTO accounts (id, label, source, profile_dir, email, org_name,
                                           subscription_type, active, created_at)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,0,?8)
                     ON CONFLICT(id) DO UPDATE SET
                       label = ?2, source = ?3, profile_dir = ?4,
                       email = COALESCE(?5, email), org_name = COALESCE(?6, org_name),
                       subscription_type = COALESCE(?7, subscription_type)",
                    params![id, "Default", source, profile_dir, None::<String>, None::<String>, None::<String>, now_ms()],
                )
                .unwrap();
            assert_eq!(store.get(id).unwrap().unwrap().source.as_str(), source, "the add wrote over it");

            // The next start.
            db.upsert_default_account(id, "me@example.com", &me).unwrap();
            assert_eq!(rows("id = ?1"), built_in, "{source}: Claude Code's login again, as it was");
            assert_eq!(secrets(id), (None, Some(BUILT_IN_USAGE.to_string())), "{source}: no token of the add's, its own usage key");
            assert_eq!(rows("id != ?1"), others, "{source}: no other account changed");
            assert_eq!([secrets("built-in-restored-work"), secrets("built-in-restored-home")], theirs, "{source}");
        }

        // A start with nothing to put back changes nothing.
        db.upsert_default_account(id, "me@example.com", &me).unwrap();
        assert_eq!(rows("id = ?1"), built_in);
        assert_eq!(secrets(id), (None, Some(BUILT_IN_USAGE.to_string())));
        assert_eq!(rows("id != ?1"), others);
        accounts::forget_usage_token(id).unwrap();
        accounts::forget_token("built-in-restored-work").unwrap();
        accounts::forget_usage_token("built-in-restored-work").unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
