//! The store. Two SQLite files, one writer (the capture task), WAL mode: the structure
//! in `agent-router.db`, and the captured bodies attached beside it as `bodies_db` —
//! see `attach_bodies` for why they were separated.
//!
//! Design rule from ARCHITECTURE.md: keep the structure, cap the content. Every
//! count, name and size is kept forever; only large text bodies are truncated, and
//! their true size is recorded alongside the truncated copy.

use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use std::path::{Path, PathBuf};

use crate::analyze::RequestShape;

pub struct Db {
    conn: Connection,
}

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
fn bodies_path(main: &Path) -> PathBuf {
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
fn attach_bodies(conn: &Connection, main: &Path) -> Result<()> {
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
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.execute_batch(SCHEMA)?;
        // Lightweight forward migration so an existing store upgrades in place.
        ensure_column(&conn, "requests", "agent_id", "TEXT")?;
        ensure_column(&conn, "requests", "account_email", "TEXT")?;
        // Every phone registered before there was a second push service was an iPhone,
        // which is exactly what the default backfills.
        ensure_column(&conn, "push_devices", "platform", "TEXT NOT NULL DEFAULT 'apns'")?;
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
        ] {
            ensure_column(&conn, "accounts", col, ty)?;
        }
        Ok(Self { conn })
    }

    /// Record the account Claude Code is already logged into. Called once at startup;
    /// it never steals `active` from an account the user has chosen.
    pub fn upsert_default_account(
        &self,
        id: &str,
        label: &str,
        status: &crate::accounts::AuthStatus,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO accounts (id, label, source, email, org_name, subscription_type,
                                   active, created_at)
             VALUES (?1, ?2, 'keychain', ?3, ?4, ?5,
                     CASE WHEN (SELECT count(*) FROM accounts WHERE active = 1) = 0 THEN 1 ELSE 0 END,
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
               agent_id, is_sidechain, account_email)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24,?25)
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
            ],
        )?;
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

        for (direction, body) in [("request", &ev.request_body), ("response", &ev.response_body)] {
            tx.execute(
                "INSERT INTO bodies_db.bodies
                     (request_id, direction, content, truncated_at, true_bytes)
                 VALUES (?1,?2,?3,?4,?5)",
                params![
                    row_id,
                    direction,
                    body.stored.as_slice(),
                    if body.truncated { Some(body.stored.len() as i64) } else { None },
                    body.true_bytes,
                ],
            )?;
        }

        tx.commit()?;
        Ok(())
    }
}

/// Add a column if the table predates it. Cheap, idempotent, no migration table.
fn ensure_column(conn: &Connection, table: &str, column: &str, ty: &str) -> Result<()> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
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
            request_body: crate::capture::StoredBody {
                stored: b"{\"hello\":1}".to_vec(),
                truncated: false,
                true_bytes: 11,
            },
            response_body: Default::default(),
            request_bytes: 11,
            response_bytes: 0,
        }
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
                "SELECT content, true_bytes FROM bodies_db.bodies WHERE direction = 'request'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(content, b"{\"hello\":1}".to_vec());
        assert_eq!(bytes, 11);

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
}
