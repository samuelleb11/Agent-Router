//! What a request cost, from a rate table that lives in the database.
//!
//! Prices change, so they are a config table rather than constants (ARCHITECTURE.md):
//! the defaults below seed an empty table and are never written over an existing row,
//! so an edited rate survives every upgrade.
//!
//! The five columns are the ones Anthropic bills on, and they are not interchangeable —
//! a cache write costs more than base input (1.25x for the 5-minute duration, 2x for the
//! hour) while a cache read costs a tenth. On a long agentic session cache reads are
//! almost all the traffic, so collapsing them into one "input" number would overstate
//! the bill by an order of magnitude.

use std::collections::HashMap;

use rusqlite::Connection;

/// US dollars per million tokens.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub struct Rates {
    pub input: f64,
    pub cache_write_5m: f64,
    pub cache_write_1h: f64,
    pub cache_read: f64,
    pub output: f64,
}

/// Published rates, current as of 2026-09-10. Seed values only.
const DEFAULTS: &[(&str, Rates)] = &[
    // model id,               input, write5m, write1h,  read,  output
    ("claude-fable-5-1",  Rates { input: 10.0, cache_write_5m: 12.50, cache_write_1h: 20.0, cache_read: 0.25, output: 50.0 }),
    ("claude-mythos-5-1", Rates { input: 10.0, cache_write_5m: 12.50, cache_write_1h: 20.0, cache_read: 0.25, output: 50.0 }),
    ("claude-fable-5",    Rates { input: 10.0, cache_write_5m: 12.50, cache_write_1h: 20.0, cache_read: 1.00, output: 50.0 }),
    ("claude-mythos-5",   Rates { input: 10.0, cache_write_5m: 12.50, cache_write_1h: 20.0, cache_read: 1.00, output: 50.0 }),
    ("claude-opus-5",     Rates { input: 5.0,  cache_write_5m: 6.25,  cache_write_1h: 10.0, cache_read: 0.50, output: 25.0 }),
    ("claude-opus-4-8",   Rates { input: 5.0,  cache_write_5m: 6.25,  cache_write_1h: 10.0, cache_read: 0.50, output: 25.0 }),
    ("claude-opus-4-7",   Rates { input: 5.0,  cache_write_5m: 6.25,  cache_write_1h: 10.0, cache_read: 0.50, output: 25.0 }),
    ("claude-opus-4-6",   Rates { input: 5.0,  cache_write_5m: 6.25,  cache_write_1h: 10.0, cache_read: 0.50, output: 25.0 }),
    ("claude-opus-4-5",   Rates { input: 5.0,  cache_write_5m: 6.25,  cache_write_1h: 10.0, cache_read: 0.50, output: 25.0 }),
    ("claude-opus-4-1",   Rates { input: 15.0, cache_write_5m: 18.75, cache_write_1h: 30.0, cache_read: 1.50, output: 75.0 }),
    ("claude-opus-4",     Rates { input: 15.0, cache_write_5m: 18.75, cache_write_1h: 30.0, cache_read: 1.50, output: 75.0 }),
    ("claude-sonnet-5",   Rates { input: 2.0,  cache_write_5m: 2.50,  cache_write_1h: 4.0,  cache_read: 0.20, output: 10.0 }),
    ("claude-sonnet-4-6", Rates { input: 3.0,  cache_write_5m: 3.75,  cache_write_1h: 6.0,  cache_read: 0.30, output: 15.0 }),
    ("claude-sonnet-4-5", Rates { input: 3.0,  cache_write_5m: 3.75,  cache_write_1h: 6.0,  cache_read: 0.30, output: 15.0 }),
    ("claude-sonnet-4",   Rates { input: 3.0,  cache_write_5m: 3.75,  cache_write_1h: 6.0,  cache_read: 0.30, output: 15.0 }),
    ("claude-haiku-4-5",  Rates { input: 1.0,  cache_write_5m: 1.25,  cache_write_1h: 2.0,  cache_read: 0.10, output: 5.0 }),
    ("claude-haiku-3-5",  Rates { input: 0.80, cache_write_5m: 1.00,  cache_write_1h: 1.60, cache_read: 0.08, output: 4.0 }),
];

pub fn ensure_table(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS pricing (
             model           TEXT PRIMARY KEY,
             input           REAL NOT NULL,
             cache_write_5m  REAL NOT NULL,
             cache_write_1h  REAL NOT NULL,
             cache_read      REAL NOT NULL,
             output          REAL NOT NULL,
             source          TEXT
         );",
    )?;
    // OR IGNORE, never REPLACE: a rate someone corrected by hand must outlive an upgrade.
    let mut stmt = db.prepare(
        "INSERT OR IGNORE INTO pricing
             (model, input, cache_write_5m, cache_write_1h, cache_read, output, source)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'seed 2026-09-10')",
    )?;
    for (model, r) in DEFAULTS {
        stmt.execute(rusqlite::params![
            model,
            r.input,
            r.cache_write_5m,
            r.cache_write_1h,
            r.cache_read,
            r.output
        ])?;
    }
    drop(stmt);

    // Cost is derived, never stored. A rate corrected today should restate every
    // request ever captured, which a column written at capture time cannot do.
    //
    // The model a request carries is a wire name — dated, suffixed, sometimes prefixed
    // — so it is matched against the longest table key it contains, the same rule the
    // Rust lookup uses, because `claude-opus-4` is a substring of `claude-opus-4-1`.
    db.execute_batch(
        "DROP VIEW IF EXISTS request_cost;
         CREATE VIEW request_cost AS
         SELECT r.id AS request_id,
                ( u.input_tokens             * p.input
                + u.cache_creation_5m_tokens * p.cache_write_5m
                + u.cache_creation_1h_tokens * p.cache_write_1h
                + u.cache_read_tokens        * p.cache_read
                + u.output_tokens            * p.output ) / 1000000.0 AS cost
         FROM requests r
         JOIN usage u ON u.request_id = r.id
         JOIN pricing p ON p.model = (
             SELECT p2.model FROM pricing p2
             WHERE instr(lower(r.model), p2.model) > 0
             ORDER BY length(p2.model) DESC LIMIT 1
         );",
    )?;
    Ok(())
}

pub fn load(db: &Connection) -> rusqlite::Result<HashMap<String, Rates>> {
    let mut stmt = db.prepare(
        "SELECT model, input, cache_write_5m, cache_write_1h, cache_read, output FROM pricing",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            Rates {
                input: r.get(1)?,
                cache_write_5m: r.get(2)?,
                cache_write_1h: r.get(3)?,
                cache_read: r.get(4)?,
                output: r.get(5)?,
            },
        ))
    })?;
    rows.collect()
}

/// Find the rate for a model string as it appears on the wire.
///
/// Wire names carry things the table does not: a date suffix, a `[1m]` marker, a
/// provider prefix. So the longest table key the name contains wins — longest because
/// `claude-opus-4-1` and `claude-opus-4` both match an Opus 4.1 request, and the
/// specific one is the right answer.
pub fn rate_for<'a>(table: &'a HashMap<String, Rates>, model: &str) -> Option<&'a Rates> {
    let m = model.to_ascii_lowercase();
    table
        .iter()
        .filter(|(k, _)| m.contains(k.as_str()))
        .max_by_key(|(k, _)| k.len())
        .map(|(_, v)| v)
}

/// Tokens as billed, split the way the rates are.
#[derive(Debug, Default, Clone, Copy)]
pub struct Tokens {
    pub input: i64,
    pub cache_write_5m: i64,
    pub cache_write_1h: i64,
    pub cache_read: i64,
    pub output: i64,
}

/// Dollars for one request, or for a summed set of them.
pub fn cost(r: &Rates, t: &Tokens) -> f64 {
    const PER: f64 = 1_000_000.0;
    (t.input as f64 * r.input
        + t.cache_write_5m as f64 * r.cache_write_5m
        + t.cache_write_1h as f64 * r.cache_write_1h
        + t.cache_read as f64 * r.cache_read
        + t.output as f64 * r.output)
        / PER
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> HashMap<String, Rates> {
        let db = Connection::open_in_memory().unwrap();
        ensure_table(&db).unwrap();
        load(&db).unwrap()
    }

    #[test]
    fn the_view_prices_a_request_the_same_way_the_code_does() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE requests (id INTEGER PRIMARY KEY, model TEXT);
             CREATE TABLE usage (request_id INTEGER, input_tokens INT, output_tokens INT,
                 cache_read_tokens INT, cache_creation_5m_tokens INT, cache_creation_1h_tokens INT);
             INSERT INTO requests VALUES (1, 'claude-opus-5-20260115');
             INSERT INTO usage VALUES (1, 1000, 2000, 500000, 10000, 0);",
        )
        .unwrap();
        ensure_table(&db).unwrap();

        let from_view: f64 = db
            .query_row("SELECT cost FROM request_cost WHERE request_id = 1", [], |r| r.get(0))
            .unwrap();
        let t = load(&db).unwrap();
        let from_code = cost(
            rate_for(&t, "claude-opus-5-20260115").unwrap(),
            &Tokens { input: 1000, output: 2000, cache_read: 500_000, cache_write_5m: 10_000, cache_write_1h: 0 },
        );
        assert!((from_view - from_code).abs() < 1e-9, "view {from_view} vs code {from_code}");
    }

    #[test]
    fn an_unknown_model_is_left_out_rather_than_priced_at_zero() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE requests (id INTEGER PRIMARY KEY, model TEXT);
             CREATE TABLE usage (request_id INTEGER, input_tokens INT, output_tokens INT,
                 cache_read_tokens INT, cache_creation_5m_tokens INT, cache_creation_1h_tokens INT);
             INSERT INTO requests VALUES (1, 'some-other-vendor/model-x');
             INSERT INTO usage VALUES (1, 1000, 2000, 0, 0, 0);",
        )
        .unwrap();
        ensure_table(&db).unwrap();
        let n: i64 = db
            .query_row("SELECT count(*) FROM request_cost WHERE request_id = 1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0, "an unpriced model must be absent, not counted as free");
    }

    #[test]
    fn seeds_are_not_written_over_an_edited_rate() {
        let db = Connection::open_in_memory().unwrap();
        ensure_table(&db).unwrap();
        db.execute("UPDATE pricing SET input = 999.0 WHERE model = 'claude-opus-5'", [])
            .unwrap();
        ensure_table(&db).unwrap(); // an upgrade re-runs this
        let t = load(&db).unwrap();
        assert_eq!(t["claude-opus-5"].input, 999.0, "a hand-corrected rate must survive");
    }

    #[test]
    fn the_most_specific_model_wins() {
        let t = table();
        // Both claude-opus-4 and claude-opus-4-1 are substrings of this name.
        let r = rate_for(&t, "claude-opus-4-1-20250805").expect("known");
        assert_eq!(r.input, 15.0);
        // Opus 5 must not be mistaken for Opus 4's older, dearer rate.
        assert_eq!(rate_for(&t, "claude-opus-5").unwrap().input, 5.0);
    }

    #[test]
    fn wire_decorations_do_not_defeat_the_lookup() {
        let t = table();
        for name in ["claude-opus-5", "claude-opus-5-20260115", "opus[1m] claude-opus-5"] {
            assert!(rate_for(&t, name).is_some(), "{name} should resolve");
        }
        assert!(rate_for(&t, "some-other-vendor-model").is_none());
    }

    #[test]
    fn cache_reads_are_priced_a_tenth_of_input() {
        let t = table();
        let r = rate_for(&t, "claude-opus-5").unwrap();
        let read_only = cost(r, &Tokens { cache_read: 1_000_000, ..Default::default() });
        let input_only = cost(r, &Tokens { input: 1_000_000, ..Default::default() });
        assert!((read_only - 0.50).abs() < 1e-9, "got {read_only}");
        assert!((input_only - 5.00).abs() < 1e-9, "got {input_only}");
    }

    #[test]
    fn a_realistic_session_costs_what_the_rates_say() {
        // This conversation, roughly: almost all of it cache reads.
        let t = table();
        let r = rate_for(&t, "claude-opus-5").unwrap();
        let c = cost(
            r,
            &Tokens { input: 848, cache_write_5m: 3_089_991, cache_read: 89_974_795, output: 543_911, ..Default::default() },
        );
        // 3.09M writes at $6.25 + 90M reads at $0.50 + 544k out at $25 ~= $77.
        assert!((c - 77.3).abs() < 1.0, "expected about $77, got ${c:.2}");
    }
}
