//! The store. One SQLite file, one writer (the capture task), WAL mode.
//!
//! Design rule from ARCHITECTURE.md: keep the structure, cap the content. Every
//! count, name and size is kept forever; only large text bodies are truncated, and
//! their true size is recorded alongside the truncated copy.

use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use std::path::Path;

use crate::analyze::RequestShape;

pub struct Db {
    conn: Connection,
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS accounts (
  id                TEXT PRIMARY KEY,
  label             TEXT NOT NULL,
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

CREATE TABLE IF NOT EXISTS bodies (
  request_id   INTEGER NOT NULL REFERENCES requests(id) ON DELETE CASCADE,
  direction    TEXT NOT NULL,
  content      BLOB,
  truncated_at INTEGER,
  true_bytes   INTEGER NOT NULL,
  PRIMARY KEY (request_id, direction)
);
"#;

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
        Ok(Self { conn })
    }

    pub fn upsert_account(&self, id: &str, label: &str, subscription: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO accounts (id, label, subscription_type, active, created_at)
             VALUES (?1, ?2, ?3, 1, ?4)
             ON CONFLICT(id) DO UPDATE SET label = ?2, subscription_type = ?3, active = 1",
            params![id, label, subscription, now_ms()],
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
               agent_id, is_sidechain)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24)
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
            ],
        )?;
        let row_id = tx.last_insert_rowid();
        if row_id == 0 {
            tx.commit()?;
            return Ok(()); // duplicate request-id; already recorded
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
                "INSERT INTO bodies (request_id, direction, content, truncated_at, true_bytes)
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
