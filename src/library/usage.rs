//! What the Library is used for, counted from the traffic a hosted router already carries
//! (docs/library.md, "Usage"). Three things count, each against the item of that name:
//!
//! - a skill the model calls: a `Skill` tool call, by its `skill`;
//! - a subagent it launches: an `Agent` call (`Task`, in older Claude Codes), by its
//!   `subagent_type`;
//! - a command or a skill a person types: a turn that begins `<command-name>/x</command-name>`.
//!
//! A conversation sends its whole history with every request, so a use is taken where the
//! search index first places it (`db::write_blocks`): a block at one spot in a session, which
//! the requests repeating it later do not place again. Counting as requests are recorded and
//! rebuilding from the index therefore find the same uses, once each.
//!
//! Every name that could be an item's is kept, in the Library or not, so an item's history
//! starts before it joined. What counts against which item is decided when usage is read.
//!
//! **The listing.** Claude Code tells the model, on every turn, which skills it may call: a
//! line per skill, its name and its description. What the Library's skills add to that line
//! is a cost every turn pays, measured here from the newest listing the router has seen.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use axum::extract::{Query, State};
use axum::response::{IntoResponse, Response};
use axum::Json;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::api::{read, Refusal};
use super::Kind;
use crate::analyze::Block;
use crate::proxy::AppState;

/// How a name was used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum How {
    /// The model called a skill.
    Called,
    /// The model launched a subagent.
    Launched,
    /// A person typed a command, or a skill, by its slash name.
    Typed,
}

impl How {
    fn as_str(self) -> &'static str {
        match self {
            How::Called => "called",
            How::Launched => "launched",
            How::Typed => "typed",
        }
    }

    fn parse(s: &str) -> Option<How> {
        [How::Called, How::Launched, How::Typed].into_iter().find(|h| h.as_str() == s)
    }

    /// Whether a use counts against an item of this kind. A typed name is the skill's when the
    /// Library holds a skill of that name, and the command's otherwise: Claude Code offers both
    /// under one slash.
    fn counts_for(self, kind: Kind, a_skill_has_it: bool) -> bool {
        match (kind, self) {
            (Kind::Skill, How::Called | How::Typed) => true,
            (Kind::Command, How::Typed) => !a_skill_has_it,
            (Kind::Subagent, How::Launched) => true,
            _ => false,
        }
    }
}

/// The use a block records, if it records one: what was used and how. `kind` and `text` are
/// the block's as `analyze` cut it — a tool call is its name, a space and its input — and
/// `role` is the message's it sat in.
pub fn use_in(kind: &str, role: &str, text: &str) -> Option<(How, String)> {
    match (kind, role) {
        ("tool_use", "assistant") => {
            let (tool, input) = text.split_once(' ')?;
            let (how, field) = match tool {
                "Skill" => (How::Called, "skill"),
                "Agent" | "Task" => (How::Launched, "subagent_type"),
                _ => return None,
            };
            let input: Value = serde_json::from_str(input).ok()?;
            named(input.get(field)?.as_str()?).map(|name| (how, name))
        }
        // Only a turn a person began with a command. A summary, a pasted file or an answer
        // that mentions the tag further in is not one.
        ("text", "user") => {
            let turn = text.trim_start();
            if !turn.starts_with("<command-") {
                return None;
            }
            let from = turn.find("<command-name>")? + "<command-name>".len();
            let to = from + turn[from..].find("</command-name>")?;
            named(turn[from..to].trim()).map(|name| (How::Typed, name))
        }
        _ => None,
    }
}

/// A used name, when it is one an item could have: `/x` and `x` alike.
fn named(raw: &str) -> Option<String> {
    let name = raw.strip_prefix('/').unwrap_or(raw);
    super::check_name(name).is_ok().then(|| name.to_string())
}

/// Record the uses among the blocks a request placed for the first time. `placed` is what
/// `db::write_blocks` answers: each newly placed block's index in `blocks`, and its id.
pub fn note(tx: &Connection, blocks: &[Block], placed: &[(usize, i64)], session: &str, at: i64) -> Result<()> {
    for &(index, block_id) in placed {
        let block = &blocks[index];
        if let Some((how, name)) = use_in(block.kind, &block.role, &block.text) {
            keep(tx, block_id, session, block.ordinal, how, &name, at)?;
        }
    }
    Ok(())
}

fn keep(conn: &Connection, block_id: i64, session: &str, ordinal: i64, how: How, name: &str, at: i64) -> Result<()> {
    conn.execute(
        "INSERT INTO library_uses (block_id, session_id, ordinal, how, name, at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6) ON CONFLICT DO NOTHING",
        params![block_id, session, ordinal, how.as_str(), name, at],
    )?;
    Ok(())
}

/// How far the rebuild has read, by block id: kept in `settings` so a restart resumes.
const REBUILT_KEY: &str = "library.uses_after";
const REBUILD_BATCH: i64 = 5_000;

/// Find the uses in what the router stored before it counted them, once. A hosted router
/// runs it at start, once the router before it has let go of the store.
///
/// It reads the blocks written before it began, in batches of ids, and every place each one
/// sat; the blocks written since were counted as they were placed. Resumable, and idempotent:
/// a use found twice is kept once.
pub fn rebuild(path: &Path) -> Result<()> {
    let conn = super::store::writer(path)?;
    crate::db::attach_bodies(&conn, path)?;
    let mut after: i64 = conn
        .query_row("SELECT value FROM settings WHERE key = ?1", [REBUILT_KEY], |r| r.get::<_, String>(0))
        .optional()?
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let ceiling: i64 = conn.query_row("SELECT coalesce(max(id), 0) FROM bodies_db.blocks", [], |r| r.get(0))?;
    if after >= ceiling {
        return Ok(());
    }
    tracing::info!(from = after, to = ceiling, "finding the Library's uses in the conversations already stored");
    let mut found = 0;
    while after < ceiling {
        let upto = (after + REBUILD_BATCH).min(ceiling);
        let tx = conn.unchecked_transaction()?;
        {
            // The text is read only for a block that could hold a use: a tool call by one of
            // the three names, or a text that begins with a command's tag.
            let mut blocks = tx.prepare(
                "SELECT id, kind, text FROM bodies_db.blocks
                  WHERE id > ?1 AND id <= ?2
                    AND ((kind = 'tool_use' AND (text LIKE 'Skill %' OR text LIKE 'Agent %' OR text LIKE 'Task %'))
                      OR (kind = 'text' AND ltrim(substr(text, 1, 64), char(32, 9, 10, 13)) LIKE '<command-%'))",
            )?;
            let mut sites = tx.prepare("SELECT session_id, ordinal, role, at FROM bodies_db.block_sites WHERE block_id = ?1")?;
            let candidates: Vec<(i64, String, String)> = blocks
                .query_map(params![after, upto], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                .collect::<rusqlite::Result<_>>()?;
            for (id, kind, text) in candidates {
                let placed: Vec<(String, i64, String, i64)> = sites
                    .query_map([id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
                    .collect::<rusqlite::Result<_>>()?;
                for (session, ordinal, role, at) in placed {
                    if let Some((how, name)) = use_in(&kind, &role, &text) {
                        keep(&tx, id, &session, ordinal, how, &name, at)?;
                        found += 1;
                    }
                }
            }
        }
        tx.execute(
            "INSERT INTO settings (key, value, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
            params![REBUILT_KEY, upto.to_string(), crate::db::now_ms()],
        )?;
        tx.commit()?;
        after = upto;
        // Live traffic writes to the same store, and must not wait behind this.
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    tracing::info!(found, "the Library's uses in the stored conversations are counted");
    Ok(())
}

const DAY: i64 = 86_400_000;
/// How many days usage is told over: the 30 that end today.
pub const DAYS: i64 = 30;

/// An item's uses over the last [`DAYS`] days, and when it was last used at all.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Totals {
    pub uses_30d: i64,
    pub last_used_at: Option<i64>,
}

/// Every item the Library holds, as (kind, name).
fn held(conn: &Connection) -> Result<Vec<(Kind, String)>> {
    let mut stmt = conn.prepare("SELECT kind, name FROM library_items ORDER BY kind, name")?;
    let rows: Vec<(String, String)> = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
    Ok(rows.into_iter().filter_map(|(kind, name)| Some((Kind::parse(&kind)?, name))).collect())
}

/// The first moment of the [`DAYS`] that end today, in the viewer's days: `tz` is their UTC
/// offset in minutes.
fn window(now: i64, tz: i64) -> (i64, i64) {
    let tz_ms = tz.clamp(-14 * 60, 14 * 60) * 60_000;
    let first_day = (now + tz_ms).div_euclid(DAY) - (DAYS - 1);
    (first_day, first_day * DAY - tz_ms)
}

/// Each item's totals, for `GET /api/library`.
pub fn totals(conn: &Connection, now: i64) -> Result<HashMap<(Kind, String), Totals>> {
    let held = held(conn)?;
    let skills: HashSet<&str> = held.iter().filter(|(k, _)| *k == Kind::Skill).map(|(_, n)| n.as_str()).collect();
    let mut stmt = conn.prepare(
        "SELECT name, how, sum(at >= ?1), max(at) FROM library_uses
          WHERE name IN (SELECT name FROM library_items) GROUP BY name, how",
    )?;
    let since = window(now, 0).1;
    let rows: Vec<(String, String, i64, i64)> =
        stmt.query_map([since], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?.collect::<rusqlite::Result<_>>()?;
    let mut out: HashMap<(Kind, String), Totals> = held.iter().map(|k| (k.clone(), Totals::default())).collect();
    for (name, how, recent, last) in rows {
        let Some(how) = How::parse(&how) else { continue };
        for kind in Kind::ALL {
            if !how.counts_for(kind, skills.contains(name.as_str())) {
                continue;
            }
            if let Some(t) = out.get_mut(&(kind, name.clone())) {
                t.uses_30d += recent;
                t.last_used_at = t.last_used_at.max(Some(last));
            }
        }
    }
    Ok(out)
}

/// One machine's share of an item's uses. `id` is absent for the uses of sessions no machine
/// has listed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ByMachine {
    pub id: Option<String>,
    pub label: Option<String>,
    pub uses: i64,
}

/// One project's share: the session's working directory, absent while its transcript is not
/// known.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ByProject {
    pub project: Option<String>,
    pub uses: i64,
}

/// One item's use, as `GET /api/library/usage` tells it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ItemUsage {
    pub kind: Kind,
    pub name: String,
    #[serde(flatten)]
    pub totals: Totals,
    /// Uses on each of the [`DAYS`] days, oldest first.
    pub daily: Vec<i64>,
    pub machines: Vec<ByMachine>,
    pub projects: Vec<ByProject>,
}

/// What one Library skill's line in the listing adds to every turn.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Entry {
    pub name: String,
    pub chars: i64,
    pub tokens: i64,
}

/// The Library's share of the newest skill listing the router has seen.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Listing {
    /// When that listing was sent.
    pub at: i64,
    pub chars: i64,
    pub tokens: i64,
    pub items: Vec<Entry>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Report {
    /// Where each of the [`DAYS`] days begins, oldest first.
    pub days: Vec<i64>,
    /// Absent until a listing carrying the Library's skills has been seen.
    pub listing: Option<Listing>,
    pub items: Vec<ItemUsage>,
}

/// Every item's use over the last [`DAYS`] days, split by day, machine and project, and what
/// the listing of its skills costs. `tz` is the viewer's UTC offset in minutes, so a day
/// starts at their midnight.
pub fn report(conn: &Connection, now: i64, tz: i64) -> Result<Report> {
    let held = held(conn)?;
    let skills: HashSet<&str> = held.iter().filter(|(k, _)| *k == Kind::Skill).map(|(_, n)| n.as_str()).collect();
    let (first_day, since) = window(now, tz);
    let tz_ms = since - first_day * DAY;
    let totals = totals(conn, now)?;
    let places = places(conn)?;

    struct Tally {
        daily: Vec<i64>,
        machines: BTreeMap<Option<String>, i64>,
        projects: BTreeMap<Option<String>, i64>,
    }
    let mut tallies: HashMap<(Kind, String), Tally> = held
        .iter()
        .map(|k| (k.clone(), Tally { daily: vec![0; DAYS as usize], machines: BTreeMap::new(), projects: BTreeMap::new() }))
        .collect();
    let mut stmt = conn.prepare(
        "SELECT u.name, u.how, u.session_id, u.at, s.project FROM library_uses u
           LEFT JOIN sessions s ON s.id = u.session_id
          WHERE u.at >= ?1 AND u.name IN (SELECT name FROM library_items)",
    )?;
    let rows = stmt.query_map([since], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, i64>(3)?, r.get::<_, Option<String>>(4)?))
    })?;
    for row in rows {
        let (name, how, session, at, cwd) = row?;
        let Some(how) = How::parse(&how) else { continue };
        let day = (at + tz_ms).div_euclid(DAY) - first_day;
        if !(0..DAYS).contains(&day) {
            continue;
        }
        for kind in Kind::ALL {
            if !how.counts_for(kind, skills.contains(name.as_str())) {
                continue;
            }
            if let Some(t) = tallies.get_mut(&(kind, name.clone())) {
                t.daily[day as usize] += 1;
                *t.machines.entry(places.get(&session).map(|(m, _)| m.clone())).or_default() += 1;
                *t.projects.entry(cwd.clone()).or_default() += 1;
            }
        }
    }

    let labels: HashMap<&str, Option<&str>> = places.values().map(|(m, l)| (m.as_str(), l.as_deref())).collect();
    let items = held
        .iter()
        .filter_map(|key| Some((tallies.remove(key)?, key.clone())))
        .map(|(t, (kind, name))| {
            let mut machines: Vec<ByMachine> = t
                .machines
                .into_iter()
                .map(|(id, uses)| {
                    let label = id.as_deref().and_then(|m| labels.get(m).copied().flatten()).map(str::to_string);
                    ByMachine { id, label, uses }
                })
                .collect();
            machines.sort_by_key(|m| std::cmp::Reverse(m.uses));
            let mut projects: Vec<ByProject> = t.projects.into_iter().map(|(project, uses)| ByProject { project, uses }).collect();
            projects.sort_by_key(|p| std::cmp::Reverse(p.uses));
            let totals = totals.get(&(kind, name.clone())).copied().unwrap_or_default();
            ItemUsage { kind, name, totals, daily: t.daily, machines, projects }
        })
        .collect();
    let days = (0..DAYS).map(|d| since + d * DAY).collect();
    Ok(Report { days, listing: listing(conn, &skills)?, items })
}

/// Where each session ran, as the machines listed it (`places`): session -> (machine, its
/// label).
fn places(conn: &Connection) -> Result<HashMap<String, (String, Option<String>)>> {
    let mut stmt = conn.prepare(
        "SELECT s.id, s.machine, m.label FROM sessions s LEFT JOIN machines m ON m.id = s.machine
          WHERE s.machine IS NOT NULL AND s.machine <> ''",
    )?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, (r.get::<_, String>(1)?, r.get::<_, Option<String>>(2)?))))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// How Claude Code heads its skill listing, within a `<system-reminder>`.
pub const LISTING_HEAD: &str = "The following skills are available for use with the Skill tool:";

/// The entries of a skill listing: each skill's name and the characters of its line, with
/// the lines its description runs on to. `None` when the text is not a listing: it must begin
/// with the head, inside its reminder or not, so a text that quotes one is not taken for one.
pub fn entries(text: &str) -> Option<Vec<(String, i64)>> {
    let text = text.trim_start();
    let text = text.strip_prefix("<system-reminder>").map(str::trim_start).unwrap_or(text);
    let body = text.strip_prefix(LISTING_HEAD)?;
    let mut out: Vec<(String, i64)> = Vec::new();
    for line in body.lines() {
        if line.starts_with("</system-reminder>") {
            break;
        }
        let chars = line.chars().count() as i64 + 1;
        match line.strip_prefix("- ") {
            Some(entry) => {
                let name = entry.split_once(": ").map_or(entry, |(name, _)| name).trim();
                out.push((name.to_string(), chars));
            }
            None => {
                // A description that runs on: its lines are the entry's. Blank lines before
                // the first entry are the listing's own.
                if let Some(last) = out.last_mut() {
                    last.1 += chars;
                }
            }
        }
    }
    Some(out)
}

/// Roughly four characters to a token, as the router estimates elsewhere (ARCHITECTURE.md).
fn tokens(chars: i64) -> i64 {
    (chars + 3) / 4
}

/// The Library's skills in the newest listing the router has seen. `None` while there is
/// none, or while no Library skill is in it.
fn listing(conn: &Connection, skills: &HashSet<&str>) -> Result<Option<Listing>> {
    if skills.is_empty() {
        return Ok(None);
    }
    // A reader whose search index could not be attached has no listing to measure. The
    // listing comes in a user message; a `system` one is taken too, where Claude Code 2.1.292
    // moved the environment, should the listing follow it.
    let found = conn.prepare(
        "SELECT b.text, max(s.at) AS at FROM bodies_db.blocks_fts f
           JOIN bodies_db.blocks b ON b.id = f.rowid
           JOIN bodies_db.block_sites s ON s.block_id = b.id
          WHERE blocks_fts MATCH ?1 AND b.kind = 'text' AND s.role IN ('user', 'system')
          GROUP BY b.id ORDER BY at DESC LIMIT 20",
    );
    let Ok(mut stmt) = found else { return Ok(None) };
    let phrase = format!("\"{}\"", LISTING_HEAD.trim_end_matches(':'));
    let candidates: Vec<(String, i64)> = stmt.query_map([phrase], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
    for (text, at) in candidates {
        let Some(entries) = entries(&text) else { continue };
        let items: Vec<Entry> = entries
            .into_iter()
            .filter(|(name, _)| skills.contains(name.as_str()))
            .map(|(name, chars)| Entry { name, chars, tokens: tokens(chars) })
            .collect();
        if items.is_empty() {
            return Ok(None);
        }
        let chars = items.iter().map(|e| e.chars).sum();
        return Ok(Some(Listing { at, chars, tokens: tokens(chars), items }));
    }
    Ok(None)
}

#[derive(Deserialize)]
pub struct Asked {
    /// The viewer's UTC offset in minutes, as the dashboard's other reads take it.
    tz: Option<i64>,
}

/// `GET /api/library/usage`: every item's use over the last 30 days, by day, machine and
/// project, and what the Library's skills add to every turn.
pub async fn usage(State(state): State<Arc<AppState>>, Query(asked): Query<Asked>) -> Result<Response, Refusal> {
    let tz = asked.tz.unwrap_or(0);
    let report = read(&state, move |conn| report(conn, crate::db::now_ms(), tz)).await?;
    Ok(Json(report).into_response())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::{CaptureEvent, StoredBody};
    use crate::db::Db;
    use crate::library::bundle::{Bundle, File};
    use crate::library::store::{self, Provenance};
    use serde_json::json;

    /// 2023-11-14, late in the evening UTC: every test's conversation happens then.
    const AT: i64 = 1_700_000_000_000;

    struct Lab {
        dir: std::path::PathBuf,
        db: Db,
    }

    impl Lab {
        fn new(name: &str) -> Lab {
            let dir = std::env::temp_dir().join(format!("ar-usage-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let db = Db::open(&dir.join("agent-router.db")).unwrap();
            Lab { dir, db }
        }

        fn path(&self) -> std::path::PathBuf {
            self.dir.join("agent-router.db")
        }

        /// A reader as the API's are: the main store, with the search index beside it.
        fn reader(&self) -> Connection {
            let conn = Connection::open(self.path()).unwrap();
            conn.execute("ATTACH DATABASE ?1 AS bodies_db", [crate::db::bodies_path(&self.path()).to_string_lossy().as_ref()])
                .unwrap();
            conn
        }

        /// One request of a session, `minutes` after [`AT`], carrying `messages`.
        fn request(&mut self, session: &str, id: &str, minutes: i64, messages: &[Value]) {
            let body = serde_json::to_vec(&json!({ "messages": messages })).unwrap();
            let ev = CaptureEvent {
                request_id: Some(id.into()),
                message_id: None,
                session_id: Some(session.into()),
                account_email: None,
                agent_id: None,
                account_id: "acct-1".into(),
                provider: crate::accounts::Provider::Anthropic,
                entrypoint: None,
                model: None,
                method: "POST".into(),
                path: "/v1/messages".into(),
                started_at_ms: AT + minutes * 60_000,
                ttfb_ms: 1,
                duration_ms: 2,
                status: 200,
                streaming: false,
                stop_reason: None,
                service_tier: None,
                error: None,
                rl: Default::default(),
                usage: Default::default(),
                tool_calls: Vec::new(),
                shape: crate::analyze::analyze_request(&body),
                response_body: StoredBody { stored: Vec::new(), truncated: false, true_bytes: 0 },
                request_bytes: body.len() as i64,
                response_bytes: 0,
            };
            self.db.record(&ev).unwrap();
        }

        /// A session's working directory, and the machine that listed it, as its report says.
        fn place(&self, session: &str, cwd: &str, machine: &str, label: &str) {
            let conn = self.reader();
            conn.execute("INSERT INTO sessions (id, cwd, started_at, last_seen_at) VALUES (?1, ?2, ?3, ?3)", params![session, cwd, AT])
                .unwrap();
            let report = crate::places::Report {
                machine: machine.into(),
                label: Some(label.into()),
                sessions: vec![crate::places::Listed { session: session.into(), cwd: Some(cwd.into()) }],
                ..Default::default()
            };
            crate::places::record(&conn, &report, AT).unwrap();
        }

        fn hold(&self, kind: Kind, name: &str, at: i64) {
            let conn = self.reader();
            let bundle = match kind {
                Kind::Skill => Bundle::check(kind, name, vec![file("SKILL.md", &format!("---\nname: {name}\ndescription: One.\n---\n"))]),
                Kind::Command => Bundle::check(kind, name, vec![file(&format!("{name}.md"), "---\ndescription: One.\n---\nGo.\n")]),
                Kind::Subagent => Bundle::check(
                    kind,
                    name,
                    vec![file(&format!("{name}.md"), &format!("---\nname: {name}\ndescription: One.\n---\nGo.\n"))],
                ),
            }
            .unwrap();
            store::publish(&conn, &bundle, &Provenance::Upload, true, at).unwrap();
        }

        /// The report a viewer in UTC reads the next day.
        fn report(&self) -> Report {
            report(&self.reader(), AT + DAY, 0).unwrap()
        }

        fn item(&self, kind: Kind, name: &str) -> ItemUsage {
            self.report().items.into_iter().find(|i| i.kind == kind && i.name == name).unwrap()
        }
    }

    impl Drop for Lab {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn file(path: &str, text: &str) -> File {
        File { path: path.into(), bytes: text.as_bytes().to_vec(), executable: false }
    }

    fn user(text: &str) -> Value {
        json!({ "role": "user", "content": [{ "type": "text", "text": text }] })
    }

    fn assistant_calls(tool: &str, input: Value) -> Value {
        json!({ "role": "assistant", "content": [{ "type": "tool_use", "id": "toolu_1", "name": tool, "input": input }] })
    }

    fn result(text: &str) -> Value {
        json!({ "role": "user", "content": [{ "type": "tool_result", "tool_use_id": "toolu_1", "content": text }] })
    }

    fn assistant(text: &str) -> Value {
        json!({ "role": "assistant", "content": [{ "type": "text", "text": text }] })
    }

    /// The day [`AT`] falls on, of the 30 a report a day later tells.
    const TODAY_BUT_ONE: usize = DAYS as usize - 2;

    #[test]
    fn a_skill_call_is_counted_once() {
        let mut lab = Lab::new("skill");
        lab.hold(Kind::Skill, "release-notes", AT - DAY);
        lab.place("s-1", "/work/app", "m-1", "studio");
        let asked = [user("write the release notes"), assistant_calls("Skill", json!({ "skill": "release-notes" }))];
        let mut turn = asked.to_vec();
        turn.push(result("Launching skill: release-notes"));
        lab.request("s-1", "r-1", 0, &turn);
        // Every later request repeats the call; none of them is another use.
        turn.extend([assistant("Done."), user("thanks")]);
        lab.request("s-1", "r-2", 1, &turn);
        turn.extend([assistant("You're welcome."), user("and the changelog?")]);
        lab.request("s-1", "r-3", 2, &turn);

        let item = lab.item(Kind::Skill, "release-notes");
        assert_eq!(item.totals, Totals { uses_30d: 1, last_used_at: Some(AT) });
        assert_eq!(item.daily.iter().sum::<i64>(), 1);
        assert_eq!(item.daily[TODAY_BUT_ONE], 1, "counted on the day it was made");
        assert_eq!(item.machines, [ByMachine { id: Some("m-1".into()), label: Some("studio".into()), uses: 1 }]);
        assert_eq!(item.projects, [ByProject { project: Some("/work/app".into()), uses: 1 }]);

        // A second session's call is a second use, on its own machine and project; one no
        // machine has listed is counted all the same, placed nowhere.
        lab.place("s-2", "/work/site", "m-2", "laptop");
        lab.request("s-2", "r-4", 5, &[user("notes please"), assistant_calls("Skill", json!({ "skill": "release-notes" })), result("ok")]);
        lab.request("s-3", "r-5", 6, &[user("notes"), assistant_calls("Skill", json!({ "skill": "release-notes" })), result("ok")]);
        let item = lab.item(Kind::Skill, "release-notes");
        assert_eq!(item.totals.uses_30d, 3);
        let mut machines: Vec<_> = item.machines.iter().map(|m| (m.id.as_deref(), m.uses)).collect();
        machines.sort();
        assert_eq!(machines, [(None, 1), (Some("m-1"), 1), (Some("m-2"), 1)]);
        // As `GET /api/library` tells it on the item's row.
        let listed = totals(&lab.reader(), AT + DAY).unwrap();
        assert_eq!(listed[&(Kind::Skill, "release-notes".to_string())], Totals { uses_30d: 3, last_used_at: Some(AT + 6 * 60_000) });
    }

    #[test]
    fn a_subagent_launch_is_counted_once() {
        let mut lab = Lab::new("subagent");
        lab.hold(Kind::Subagent, "reviewer", AT - DAY);
        lab.place("s-1", "/work/app", "m-1", "studio");
        let launch = json!({ "description": "Review the diff", "prompt": "Review it.", "subagent_type": "reviewer" });
        let mut turn = vec![user("review this"), assistant_calls("Agent", launch), result("Looks fine.")];
        lab.request("s-1", "r-1", 0, &turn);
        turn.extend([assistant("It looks fine."), user("ok")]);
        lab.request("s-1", "r-2", 1, &turn);
        // An older Claude Code names the tool Task.
        let older = json!({ "description": "Again", "prompt": "Review again.", "subagent_type": "reviewer" });
        turn.extend([assistant_calls("Task", older), result("Still fine.")]);
        lab.request("s-1", "r-3", 2, &turn);

        let item = lab.item(Kind::Subagent, "reviewer");
        assert_eq!(item.totals.uses_30d, 2, "two launches, each once");
        assert_eq!(item.daily[TODAY_BUT_ONE], 2);
        assert_eq!(item.machines, [ByMachine { id: Some("m-1".into()), label: Some("studio".into()), uses: 2 }]);
        assert_eq!(item.projects, [ByProject { project: Some("/work/app".into()), uses: 2 }]);
    }

    #[test]
    fn a_typed_command_is_counted_once() {
        let mut lab = Lab::new("typed");
        lab.hold(Kind::Command, "standup", AT - DAY);
        lab.hold(Kind::Skill, "triage", AT - DAY);
        lab.hold(Kind::Command, "triage", AT - DAY);
        lab.place("s-1", "/work/app", "m-1", "studio");
        // As Claude Code sends a typed command: the tags open the turn.
        let typed = "<command-message>standup</command-message>\n<command-name>/standup</command-name>\n<command-args></command-args>";
        let mut turn = vec![user(typed), assistant("Yesterday you…")];
        lab.request("s-1", "r-1", 0, &turn);
        turn.push(user("thanks"));
        lab.request("s-1", "r-2", 1, &turn);
        // A skill typed by its slash name is the skill's, not the command's of that name.
        turn.extend([assistant("Sure."), json!({ "role": "user", "content": "<command-name>/triage</command-name>\n<command-message>triage</command-message>" })]);
        lab.request("s-1", "r-3", 2, &turn);

        let standup = lab.item(Kind::Command, "standup");
        assert_eq!(standup.totals.uses_30d, 1);
        assert_eq!(standup.daily[TODAY_BUT_ONE], 1);
        assert_eq!(standup.machines, [ByMachine { id: Some("m-1".into()), label: Some("studio".into()), uses: 1 }]);
        assert_eq!(standup.projects, [ByProject { project: Some("/work/app".into()), uses: 1 }]);
        assert_eq!(lab.item(Kind::Skill, "triage").totals.uses_30d, 1);
        assert_eq!(lab.item(Kind::Command, "triage").totals.uses_30d, 0);
    }

    #[test]
    fn a_name_outside_the_library_is_not_counted() {
        let mut lab = Lab::new("outside");
        lab.hold(Kind::Skill, "release-notes", AT - DAY);
        lab.hold(Kind::Command, "standup", AT - DAY);
        lab.request(
            "s-1",
            "r-1",
            0,
            &[
                user("go"),
                assistant_calls("Skill", json!({ "skill": "elsewhere" })),
                result("ok"),
                assistant_calls("Agent", json!({ "description": "Look", "prompt": "Look.", "subagent_type": "general-purpose" })),
                result("ok"),
                // Neither a tool the Library counts, nor a turn that begins with a command.
                assistant_calls("Bash", json!({ "command": "echo Skill release-notes" })),
                result("Skill release-notes"),
                user("The summary said <command-name>/standup</command-name> was typed earlier."),
            ],
        );
        let report = lab.report();
        let names: Vec<&str> = report.items.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names, ["standup", "release-notes"], "only what the Library holds");
        assert!(report.items.iter().all(|i| i.totals.uses_30d == 0 && i.totals.last_used_at.is_none()));
        assert!(report.items.iter().all(|i| i.daily.iter().all(|d| *d == 0)));
        // The names were kept all the same: an item that joins later has its history.
        let kept: i64 = lab.reader().query_row("SELECT count(*) FROM library_uses", [], |r| r.get(0)).unwrap();
        assert_eq!(kept, 2, "elsewhere and general-purpose");
    }

    #[test]
    fn the_listing_cost_of_each_item_is_measured() {
        // Copied from a request Claude Code sent, its other skills left out: the reminder, its
        // head, a blank line, and a line per skill, claude-api's running on to two more.
        let listing = include_str!("fixtures/skill-listing.txt");
        let mut lab = Lab::new("listing");
        lab.hold(Kind::Skill, "simplify", AT - DAY);
        lab.hold(Kind::Skill, "claude-api", AT - DAY);
        lab.hold(Kind::Command, "init", AT - DAY);
        lab.request("s-1", "r-1", 0, &[json!({ "role": "user", "content": [{ "type": "text", "text": listing }, { "type": "text", "text": "hello" }] })]);

        let line = |name: &str| -> i64 {
            let lines: Vec<&str> = listing.lines().collect();
            let at = lines.iter().position(|l| l.starts_with(&format!("- {name}: "))).unwrap();
            let runs_on = lines[at + 1..].iter().take_while(|l| !l.starts_with("- ") && !l.starts_with("</")).count();
            lines[at..=at + runs_on].iter().map(|l| l.chars().count() as i64 + 1).sum()
        };
        let report = lab.report();
        let measured = report.listing.expect("a listing was seen");
        assert_eq!(measured.at, AT);
        let mut items: Vec<(&str, i64, i64)> = measured.items.iter().map(|e| (e.name.as_str(), e.chars, e.tokens)).collect();
        items.sort();
        let (api, simplify) = (line("claude-api"), line("simplify"));
        assert!(api > 3 * simplify, "claude-api's entry runs on: {api} against {simplify}");
        assert_eq!(items, [("claude-api", api, (api + 3) / 4), ("simplify", simplify, (simplify + 3) / 4)]);
        assert_eq!(measured.chars, api + simplify, "only the Library's skills: init is a command here, the rest are not held");
        assert_eq!(measured.tokens, (api + simplify + 3) / 4);

        // A text that quotes the head is not a listing.
        assert!(entries(&format!("As the docs say, {LISTING_HEAD}\n\n- simplify: x")).is_none());
        assert_eq!(entries(&format!("{LISTING_HEAD}\n\n- plugin:deploy: Ship it.\n- bare")).unwrap(), [
            ("plugin:deploy".to_string(), 26),
            ("bare".to_string(), 7)
        ]);
    }

    #[test]
    fn usage_is_rebuilt_from_stored_bodies() {
        let mut lab = Lab::new("rebuild");
        lab.place("s-1", "/work/app", "m-1", "studio");
        let call = [user("notes"), assistant_calls("Skill", json!({ "skill": "release-notes" })), result("ok")];
        lab.request("s-1", "r-1", 0, &call);
        let mut later = call.to_vec();
        later.extend([assistant("Done."), user("<command-message>standup</command-message>\n<command-name>/standup</command-name>")]);
        lab.request("s-1", "r-2", 1, &later);
        // A store from before uses were counted: the conversations, and no uses.
        let conn = lab.reader();
        conn.execute_batch("DELETE FROM library_uses; DELETE FROM settings WHERE key = 'library.uses_after';").unwrap();

        // The items join the Library a week after they were used.
        lab.hold(Kind::Skill, "release-notes", AT + 7 * DAY);
        lab.hold(Kind::Command, "standup", AT + 7 * DAY);
        let path = lab.path();
        rebuild(&path).unwrap();
        // Run again, as every start does: nothing is counted twice.
        conn.execute("DELETE FROM settings WHERE key = 'library.uses_after'", []).unwrap();
        rebuild(&path).unwrap();
        rebuild(&path).unwrap();

        let now = AT + 8 * DAY;
        let report = report(&conn, now, 0).unwrap();
        let item = |kind: Kind, name: &str| report.items.iter().find(|i| i.kind == kind && i.name == name).unwrap().clone();
        let notes = item(Kind::Skill, "release-notes");
        assert_eq!(notes.totals, Totals { uses_30d: 1, last_used_at: Some(AT) }, "used before it joined");
        assert_eq!(notes.daily[DAYS as usize - 9], 1, "eight days before the report's day");
        assert_eq!(notes.machines, [ByMachine { id: Some("m-1".into()), label: Some("studio".into()), uses: 1 }]);
        assert_eq!(notes.projects, [ByProject { project: Some("/work/app".into()), uses: 1 }]);
        assert_eq!(item(Kind::Command, "standup").totals, Totals { uses_30d: 1, last_used_at: Some(AT + 60_000) });
    }
}
