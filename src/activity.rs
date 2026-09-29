//! What a session is doing — one answer, for every surface that asks.
//!
//! The list, the open view's status line, the prompt endpoints, the stop guard and the
//! phone notifier used to each work this out for themselves, from different inputs, and
//! so disagreed: a session generating read `idle` in the list and `Working` in its own
//! view; one that had exited read exactly like one waiting for you. This is the one
//! derivation they all call now, and nothing else derives state.
//!
//! The order of trust, from most to least:
//!
//!   1. The CLI's registry record, verified against the live process (`registry`): it
//!      alone knows whether anything is running, and its `status` is written by the
//!      CLI's own state machine — busy through silent generation, waiting on a prompt.
//!   2. The wrapper, for wrapped sessions: it sees permission prompts as they open.
//!   3. The transcript (`fold`): what the turn is doing in detail, and the whole story
//!      when the registry has nothing to say.
//!
//! File modification times are not evidence of anything here. They move on bookkeeping
//! and stand still through real work.
//!
//! PURE: no I/O. The caller gathers the evidence; `derive` only decides.

use serde::Serialize;

use crate::fold::{Bg, FoldSummary, LastEnd};
use crate::registry::{ProcRecord, RegStatus};

/// A wrapper prompt newer than this still counts while the registry says idle: the two
/// are read at different moments, and a prompt can land between them.
const PROMPT_GRACE_MS: i64 = 10_000;
/// Subagent transcripts written this recently are background work in progress.
const SUBAGENT_FRESH_MS: i64 = 120_000;
/// `busy` stamped this recently, after the last turn ended, is a new turn whose prompt
/// line has not landed yet (the CLI writes busy ~50 ms before the prompt line).
const NEW_TURN_WINDOW_MS: i64 = 3_000;
/// The registry writes idle 9–29 ms after the closing line; with our own 250 ms snapshot
/// cache on top, a busy reading this soon after a turn ended is lag, not work.
const REGISTRY_LAG_MS: i64 = 2_000;
/// Generation this long without any sign of progress is reported as stalled. Shared with
/// the caller, which has to know when to look for subagent writes that would hold it off.
pub(crate) const STALLED_AFTER_MS: i64 = 600_000;
/// A turn that started this long before the process registered belongs to an earlier
/// process (registration trails the process start by up to ~6 s).
const TURN_SLACK_MS: i64 = 5_000;

/// An open `can_use_tool` request from a wrapped session, already filtered (live
/// wrapper, not settled in the transcript) by the caller.
#[allow(dead_code)]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WrapperPrompt {
    pub request_id: String,
    /// The tool asking: `Bash`, `Edit`, `AskUserQuestion`, `ExitPlanMode`, …
    pub tool: String,
    pub tool_use_id: Option<String>,
    /// `notify::describe`'s headline: "Bash wants to run", "Question", …
    pub headline: String,
    /// `notify::describe`'s detail: the command, the question, the file.
    pub summary: String,
    /// When the router first saw it — the order prompts arrived in, which the wrapper's
    /// file does not keep.
    pub first_seen_ms: i64,
}

/// Everything known about one session at one moment.
#[allow(dead_code)]
pub struct Evidence<'a> {
    pub now_ms: i64,
    /// The registry directory could be read at all.
    pub registry_available: bool,
    /// The verified-live record naming this session, if any.
    pub record: Option<&'a ProcRecord>,
    pub wrapper_alive: bool,
    /// Live wrapper prompts, newest last.
    pub prompts: &'a [WrapperPrompt],
    pub fold: &'a FoldSummary,
    /// The newest write under this session's subagent transcripts, when it was looked at.
    pub subagent_write_ms: Option<i64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// No process holds this conversation.
    Closed,
    /// Open, and the turn is over: nothing will happen until someone asks.
    Idle,
    /// Thinking or writing — the phase the transcript cannot see.
    Generating,
    /// Running a tool call.
    Tool,
    /// Blocked on a person: a permission prompt, a question, a plan, a dialog.
    Waiting,
    /// The turn is over and the process is still busy: background agents or workflows at
    /// work, which will report back by themselves — or, with detail "Working" and no
    /// counts, something no transcript shows (a Stop hook, a task the CLI never records).
    Background,
    /// The last turn ended on an API error.
    Failed,
    /// Generating, with no sign of progress for ten minutes.
    Stalled,
}

impl Kind {
    #[allow(dead_code)]
    pub const ALL: [Kind; 8] = [
        Kind::Closed,
        Kind::Idle,
        Kind::Generating,
        Kind::Tool,
        Kind::Waiting,
        Kind::Background,
        Kind::Failed,
        Kind::Stalled,
    ];

    #[allow(dead_code)]
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Closed => "closed",
            Kind::Idle => "idle",
            Kind::Generating => "generating",
            Kind::Tool => "tool",
            Kind::Waiting => "waiting",
            Kind::Background => "background",
            Kind::Failed => "failed",
            Kind::Stalled => "stalled",
        }
    }
}

/// What the session is waiting on.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct WaitingOn {
    /// permission | question | plan | dialog
    pub kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    /// Present for a wrapper prompt: what an answer must name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_use_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

/// Background work still running.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Background {
    pub agents: usize,
    pub workflows: usize,
    pub shells: usize,
}

/// The one answer.
#[derive(Clone, Debug, PartialEq)]
pub struct Activity {
    pub kind: Kind,
    /// Some(true) open, Some(false) closed, None unknown (no registry, no wrapper).
    pub open: Option<bool>,
    /// When the current activity began. Clients show the elapsed time; `detail` never
    /// carries a relative time, since it would change every tick.
    pub since_ms: Option<i64>,
    /// The header line.
    pub detail: String,
    /// generating: "thinking" | "retrying"
    pub phase: Option<&'static str>,
    pub tool: Option<String>,
    pub waiting_on: Option<WaitingOn>,
    pub background: Option<Background>,
    pub ended_ms: Option<i64>,
    /// closed: "exit" (clean exit marker) | "unknown"
    pub ended_how: Option<&'static str>,
    pub mid_turn: Option<bool>,
    /// "registry" when the CLI's own status decided it, "transcript" when inferred.
    pub source: &'static str,
}

impl Activity {
    /// Work in flight that will produce something without anyone asking.
    #[allow(dead_code)]
    pub fn busy(&self) -> bool {
        matches!(
            self.kind,
            Kind::Generating | Kind::Tool | Kind::Background | Kind::Stalled
        )
    }

    #[allow(dead_code)]
    pub fn needs_you(&self) -> bool {
        self.kind == Kind::Waiting
    }

    /// The legacy `active` field: busy, or waiting on you.
    #[allow(dead_code)]
    pub fn active(&self) -> bool {
        self.busy() || self.needs_you()
    }

    /// Whether a stop makes sense. Not for background work: what an interrupt does to
    /// running agents and workflows is unmeasured, and it may discard their results.
    #[allow(dead_code)]
    pub fn stoppable(&self) -> bool {
        matches!(self.kind, Kind::Generating | Kind::Tool | Kind::Stalled)
    }

    /// The list's `state`, in the vocabulary installed clients know. Never "thinking",
    /// "generating", "background", "closed" or "waiting": an old phone would render an
    /// unknown state gray and drop its Stop button while the model works.
    #[allow(dead_code)]
    pub fn legacy_list_state(&self) -> &'static str {
        match self.kind {
            Kind::Closed | Kind::Idle => "idle",
            Kind::Generating | Kind::Tool | Kind::Background => "working",
            Kind::Waiting => "ask",
            Kind::Failed => "failed",
            Kind::Stalled => "stalled",
        }
    }

    /// The stream status event's `state`, in the vocabulary installed clients know.
    #[allow(dead_code)]
    pub fn legacy_stream_state(&self) -> &'static str {
        match self.kind {
            Kind::Closed | Kind::Idle => "idle",
            Kind::Generating | Kind::Background | Kind::Stalled => "working",
            Kind::Tool => "tool",
            Kind::Waiting => "ask",
            Kind::Failed => "failed",
        }
    }

    /// The colour class, chosen here so every client colours a state alike.
    #[allow(dead_code)]
    pub fn tone(&self) -> &'static str {
        match self.kind {
            Kind::Generating | Kind::Tool | Kind::Background => "busy",
            Kind::Waiting => "attention",
            Kind::Stalled => "warn",
            Kind::Failed => "error",
            Kind::Idle => "idle",
            Kind::Closed => "closed",
        }
    }

    fn new(
        kind: Kind,
        open: Option<bool>,
        source: &'static str,
        detail: impl Into<String>,
    ) -> Self {
        Activity {
            kind,
            open,
            since_ms: None,
            detail: detail.into(),
            phase: None,
            tool: None,
            waiting_on: None,
            background: None,
            ended_ms: None,
            ended_how: None,
            mid_turn: None,
            source,
        }
    }
}

/// Whether the transcript's open turn is this process's. A conversation resumed after a
/// crash still ends in the turn that crashed; the process now holding it never ran that
/// turn, and it must not read as work.
#[allow(dead_code)]
pub fn turn_counts(fold: &FoldSummary, record: Option<&ProcRecord>) -> bool {
    if !fold.turn_open {
        return false;
    }
    match record.and_then(|r| r.started_at_ms) {
        None => true,
        Some(started) => fold
            .turn_start_ts
            .is_some_and(|t| t >= started - TURN_SLACK_MS),
    }
}

/// The wrapper prompts that count, oldest first (ties keep the caller's order): none
/// while the session is closed, and none first seen more than `PROMPT_GRACE_MS` ago
/// while the registry calls the session idle — an idle session cannot be blocked on a
/// prompt, so that one is a prompt the wrapper never saw resolved. `derive` waits on the
/// newest of these; the prompt endpoints answer from, and the notifier announces,
/// exactly this set, so what a row says is waiting and what can be answered agree.
pub fn live_prompts<'a>(e: &Evidence<'a>) -> Vec<&'a WrapperPrompt> {
    if e.registry_available && e.record.is_none() {
        return Vec::new();
    }
    let status = e
        .record
        .and_then(|r| r.status.as_ref())
        .filter(|s| s.is_known());
    let mut out: Vec<&WrapperPrompt> = e
        .prompts
        .iter()
        .filter(|p| match status {
            Some(RegStatus::Idle | RegStatus::Shell) => {
                e.now_ms - p.first_seen_ms <= PROMPT_GRACE_MS
            }
            _ => true,
        })
        .collect();
    out.sort_by_key(|p| p.first_seen_ms);
    out
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Class {
    Waiting,
    Busy,
    Background,
    Idle,
}

/// The activity, from the evidence. Nothing else in the router derives state.
#[allow(dead_code)]
pub fn derive(e: &Evidence) -> Activity {
    let f = e.fold;
    let r = e.record;
    let now = e.now_ms;
    let rt = r.and_then(|r| r.status_updated_ms);
    let counts = turn_counts(f, r);
    // Background tasks an earlier process launched died with it, reported or not.
    let bg = f.bg_since(r.and_then(|r| r.started_at_ms), TURN_SLACK_MS);

    // Step 1 — open? The registry decides when it can be read at all: a record exists
    // exactly while its process lives. Without it, an answering wrapper is proof of life
    // and nothing else is.
    let open = if e.registry_available {
        Some(r.is_some())
    } else if e.wrapper_alive {
        Some(true)
    } else {
        None
    };

    // Step 2 — closed.
    if open == Some(false) {
        return closed(f);
    }

    // Step 3 — class. A known status selects the registry (regime A); a missing or
    // unfamiliar one is no evidence of anything, so the transcript decides (regime B).
    let status = r.and_then(|r| r.status.as_ref()).filter(|s| s.is_known());
    let source = if status.is_some() {
        "registry"
    } else {
        "transcript"
    };
    // A session the registry calls idle cannot be blocked on a prompt; one that has
    // stayed idle past the grace is a prompt the wrapper never saw resolved.
    let prompt = live_prompts(e).into_iter().max_by_key(|p| p.first_seen_ms);
    let agents_out = bg.agents + bg.workflows > 0;
    let subagents_writing = e
        .subagent_write_ms
        .is_some_and(|a| now - a <= SUBAGENT_FRESH_MS);
    let bg_working = agents_out && subagents_writing;
    let class = match status {
        Some(s) => {
            if *s == RegStatus::Waiting || prompt.is_some() {
                Class::Waiting
            } else if *s == RegStatus::Busy {
                Class::Busy
            } else if counts && matches!((f.turn_start_ts, rt), (Some(start), Some(t)) if start > t)
            {
                // Idle stamped before a turn that has since started: busy is written
                // ~50 ms before the prompt line, so the snapshot, or the writer, is behind.
                Class::Busy
            } else if bg_working {
                // Older CLIs may not hold busy for background agents.
                Class::Background
            } else {
                Class::Idle
            }
        }
        None => {
            if prompt.is_some() || (counts && f.ask_pending.is_some()) {
                Class::Waiting
            } else if counts {
                Class::Busy
            } else if bg_working {
                Class::Background
            } else {
                Class::Idle
            }
        }
    };

    // Step 4 — activity, first match wins.
    match class {
        Class::Waiting => waiting(e, prompt, counts, status.is_some(), open, source),
        Class::Busy if counts && !f.pending.is_empty() => {
            let p = f.pending.last().expect("non-empty");
            let mut a = Activity::new(Kind::Tool, open, source, format!("Running {}", p.name));
            a.tool = Some(p.name.clone());
            a.since_ms = p.ts;
            a
        }
        Class::Busy if counts => {
            let since = max_opt(f.turn_start_ts, f.last_result_ts);
            stall_or(generating(f, open, source, since), e)
        }
        // Row 4: a new turn whose prompt line has not landed yet. Never with agents or
        // workflows out: the CLI holds busy for them and does not rewrite busy→busy, so a
        // fresh stamp then is one of theirs — a permission prompt answered, busy→waiting→
        // busy — and a Stop now would land on nothing but their work.
        Class::Busy
            if r.is_some()
                && !agents_out
                && rt.is_some_and(|t| {
                    f.turn_end_ts.is_none_or(|end| t > end) && now - t < NEW_TURN_WINDOW_MS
                }) =>
        {
            stall_or(generating(f, open, source, rt), e)
        }
        // Row 5: the turn just ended and the registry has not caught up. It reads what it
        // is about to read, failed included: not "Waiting for you" before "API error".
        Class::Busy
            if !agents_out && f.turn_end_ts.is_some_and(|end| now - end < REGISTRY_LAG_MS) =>
        {
            if f.last_end == Some(LastEnd::ApiError) && f.failed {
                failed(f, open, source)
            } else {
                idle(e, bg, open, source)
            }
        }
        // Row 6a: busy stamped after the turn ended, and nothing in the background — the
        // main loop working with no turn open: a /compact, or the CLI carrying on after it
        // went idle. It can be stopped. Busy stamped *before* the end is 6c, not this: that
        // is busy held for something the transcript cannot see.
        Class::Busy
            if !agents_out
                && !subagents_writing
                && rt.is_some_and(|t| f.turn_end_ts.is_none_or(|end| t > end)) =>
        {
            let mut a = Activity::new(Kind::Generating, open, source, "Working");
            a.since_ms = rt;
            stall_or(a, e)
        }
        // Row 6b: the turn is over and background work shows — agents or workflows on the
        // ledger, or subagent transcripts being written.
        Class::Busy | Class::Background if agents_out || subagents_writing => {
            let bg = Background {
                agents: bg.agents,
                workflows: bg.workflows,
                shells: bg.shells,
            };
            let detail = match (bg.agents, bg.workflows) {
                (1, 0) => "1 background agent running".to_owned(),
                (n, 0) if n > 1 => format!("{n} background agents running"),
                (0, 1) => "Background workflow running".to_owned(),
                (0, n) if n > 1 => format!("{n} background workflows running"),
                _ => "Background work running".to_owned(),
            };
            let mut a = Activity::new(Kind::Background, open, source, detail);
            a.background = Some(bg);
            a.since_ms = f.turn_end_ts.or(rt).or(f.last_content_ts);
            a
        }
        // Row 6c: busy held since before the turn ended, with nothing to see — a Stop hook
        // running, or background work no transcript records (remote_agent, mcp_task,
        // auto_mode_scan, a teammate); a turn the CLI carries on by itself reads this way
        // too until its next line. Nothing is claimed about what runs, and since it may be
        // background work, Stop stays off (`Activity::stoppable`); nor does it ever turn
        // into a stall, which would offer Stop on a guess.
        Class::Busy | Class::Background => {
            let mut a = Activity::new(Kind::Background, open, source, "Working");
            a.since_ms = f.turn_end_ts.or(rt).or(f.last_content_ts);
            a
        }
        // Row 7.
        Class::Idle if f.last_end == Some(LastEnd::ApiError) && f.failed => failed(f, open, source),
        // Row 8.
        Class::Idle => idle(e, bg, open, source),
    }
}

/// The last turn ended on an API error, and nothing has been asked since.
fn failed(f: &FoldSummary, open: Option<bool>, source: &'static str) -> Activity {
    let text = f.error_text.as_deref().unwrap_or("").trim();
    // The CLI's own text often starts "API Error: …"; say it once.
    let text = text
        .strip_prefix("API Error:")
        .map(str::trim_start)
        .unwrap_or(text);
    let detail = if text.is_empty() {
        "API error".to_owned()
    } else {
        format!("API error: {text}")
    };
    let mut a = Activity::new(Kind::Failed, open, source, detail);
    a.since_ms = f.turn_end_ts;
    a
}

fn closed(f: &FoldSummary) -> Activity {
    let mid_turn = f.turn_open && !f.exited;
    let detail = if mid_turn {
        "Session ended mid-turn"
    } else {
        "Session ended"
    };
    let mut a = Activity::new(Kind::Closed, Some(false), "registry", detail);
    a.ended_ms = if f.exited {
        f.exit_ms.or(f.last_content_ts)
    } else {
        f.last_content_ts
    };
    a.ended_how = Some(if f.exited { "exit" } else { "unknown" });
    a.mid_turn = Some(mid_turn);
    a.since_ms = a.ended_ms;
    a
}

fn generating(
    f: &FoldSummary,
    open: Option<bool>,
    source: &'static str,
    since: Option<i64>,
) -> Activity {
    let mut a = match f.retrying {
        Some((n, of)) if of > 0 => Activity::new(
            Kind::Generating,
            open,
            source,
            format!("Retrying ({n}/{of})"),
        ),
        Some((n, _)) => Activity::new(Kind::Generating, open, source, format!("Retrying ({n})")),
        None => Activity::new(Kind::Generating, open, source, "Thinking"),
    };
    a.phase = Some(if f.retrying.is_some() {
        "retrying"
    } else {
        "thinking"
    });
    a.since_ms = since;
    a
}

/// Generation with nothing to show for ten minutes — no transcript line, no registry
/// write, no subagent write — is reported as stalled. Tools never are: legitimate ones run
/// for hours, so they show their duration instead.
fn stall_or(a: Activity, e: &Evidence) -> Activity {
    let last = max_opt(
        max_opt(
            e.fold.last_content_ts,
            e.record.and_then(|r| r.status_updated_ms),
        ),
        e.subagent_write_ms,
    );
    match last {
        Some(t) if e.now_ms - t > STALLED_AFTER_MS => {
            let mut s = Activity::new(Kind::Stalled, a.open, a.source, "No progress");
            s.since_ms = Some(t);
            s
        }
        _ => a,
    }
}

fn idle(e: &Evidence, bg: Bg, open: Option<bool>, source: &'static str) -> Activity {
    let f = e.fold;
    let base = if f.last_end == Some(LastEnd::Interrupt) {
        "Interrupted"
    } else {
        "Waiting for you"
    };
    // A background command finishing will start a turn by itself: worth a word, and a
    // count clients can key on rather than parse the words for. The registry's `shell`
    // says one runs without saying how many; the ledger's count is used when it has one.
    let shell =
        e.record.and_then(|r| r.status.as_ref()) == Some(&RegStatus::Shell) || bg.shells > 0;
    let detail = if shell {
        format!("{base} · background shell running")
    } else {
        base.to_owned()
    };
    let mut a = Activity::new(Kind::Idle, open, source, detail);
    a.since_ms = f.turn_end_ts.or(f.last_content_ts);
    if shell {
        a.background = Some(Background {
            agents: 0,
            workflows: 0,
            shells: bg.shells.max(1),
        });
    }
    a
}

fn waiting(
    e: &Evidence,
    prompt: Option<&WrapperPrompt>,
    counts: bool,
    regime_a: bool,
    open: Option<bool>,
    source: &'static str,
) -> Activity {
    let f = e.fold;
    let r = e.record;
    let registry_waiting =
        regime_a && r.and_then(|r| r.status.as_ref()) == Some(&RegStatus::Waiting);
    let rt = r
        .and_then(|r| r.status_updated_ms)
        .filter(|_| registry_waiting);

    // The most specific account wins: the wrapper's prompt names the request itself; the
    // transcript names the call; the registry only says what sort of thing is open.
    let (on, since, detail) = if let Some(p) = prompt {
        let kind = kind_of_tool(&p.tool);
        let detail = match kind {
            "permission" if !p.headline.trim().is_empty() => p.headline.clone(),
            "permission" => format!("{} wants to run", p.tool),
            k => waiting_detail(k).to_owned(),
        };
        let on = WaitingOn {
            kind,
            tool: Some(p.tool.clone()),
            request_id: Some(p.request_id.clone()),
            tool_use_id: p.tool_use_id.clone(),
            summary: Some(p.summary.clone()).filter(|s| !s.trim().is_empty()),
        };
        (on, Some(p.first_seen_ms), detail)
    } else if let Some(ask) = f.ask_pending.as_ref().filter(|_| counts) {
        let kind = ask.kind.as_str();
        let tool = if kind == "plan" {
            "ExitPlanMode"
        } else {
            "AskUserQuestion"
        };
        let on = WaitingOn {
            kind,
            tool: Some(tool.to_owned()),
            request_id: None,
            tool_use_id: Some(ask.id.clone()),
            summary: Some(ask.summary.clone()).filter(|s| !s.trim().is_empty()),
        };
        (on, rt.or(ask.ts), waiting_detail(kind).to_owned())
    } else if let Some(p) = f.pending.last().filter(|_| counts) {
        let kind = kind_of_tool(&p.name);
        let detail = match kind {
            "permission" => format!("{} wants to run", p.name),
            k => waiting_detail(k).to_owned(),
        };
        let on = WaitingOn {
            kind,
            tool: Some(p.name.clone()),
            request_id: None,
            tool_use_id: Some(p.id.clone()),
            summary: Some(p.summary.clone()).filter(|s| !s.trim().is_empty()),
        };
        (on, rt.or(p.ts), detail)
    } else {
        let label = r.and_then(|r| r.waiting_for.as_deref());
        let kind = match label {
            Some("input needed") => "question",
            Some("permission prompt") => "permission",
            _ => "dialog",
        };
        let detail = match kind {
            "permission" => "Waiting for permission",
            k => waiting_detail(k),
        };
        let on = WaitingOn {
            kind,
            tool: None,
            request_id: None,
            tool_use_id: None,
            summary: label.filter(|_| kind == "dialog").map(str::to_owned),
        };
        (on, r.and_then(|r| r.status_updated_ms), detail.to_owned())
    };

    let mut a = Activity::new(Kind::Waiting, open, source, detail);
    a.tool = on.tool.clone();
    a.waiting_on = Some(on);
    a.since_ms = since;
    a
}

fn kind_of_tool(tool: &str) -> &'static str {
    match tool {
        "AskUserQuestion" => "question",
        "ExitPlanMode" => "plan",
        _ => "permission",
    }
}

fn waiting_detail(kind: &str) -> &'static str {
    match kind {
        "question" => "Waiting for your answer",
        "plan" => "Plan waiting for approval",
        "dialog" => "Waiting on a dialog",
        _ => "Waiting for permission",
    }
}

fn max_opt(a: Option<i64>, b: Option<i64>) -> Option<i64> {
    match (a, b) {
        (Some(x), Some(y)) => Some(x.max(y)),
        (x, None) => x,
        (None, y) => y,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fold::{AskKind, AskPending, BgTask, PendingTool};

    /// Now, for every case: a fixed instant well after everything below.
    const NOW: i64 = 1_790_680_000_000;
    /// When the process holding the session registered.
    const STARTED: i64 = NOW - 3_600_000;

    fn rec(status: Option<&str>, t: Option<i64>) -> ProcRecord {
        ProcRecord {
            pid: 4352,
            session_id: "d6e4d7da".into(),
            started_at_ms: Some(STARTED),
            status: status.map(|s| match s {
                "busy" => RegStatus::Busy,
                "waiting" => RegStatus::Waiting,
                "idle" => RegStatus::Idle,
                "shell" => RegStatus::Shell,
                other => RegStatus::Unknown(other.into()),
            }),
            status_updated_ms: t,
            updated_ms: t,
            ..Default::default()
        }
    }

    /// A turn opened `secs` ago by this process, nothing else happening.
    fn open_turn(secs: i64) -> FoldSummary {
        FoldSummary {
            turn_open: true,
            turn_start_ts: Some(NOW - secs * 1000),
            last_content_ts: Some(NOW - secs * 1000),
            ..Default::default()
        }
    }

    /// A turn that ended `secs` ago with a text end_turn.
    fn ended(secs: i64) -> FoldSummary {
        FoldSummary {
            turn_open: false,
            turn_start_ts: Some(NOW - secs * 1000 - 60_000),
            turn_end_ts: Some(NOW - secs * 1000),
            last_end: Some(LastEnd::EndTurn),
            last_content_ts: Some(NOW - secs * 1000),
            ..Default::default()
        }
    }

    fn pending(name: &str, secs: i64) -> PendingTool {
        PendingTool {
            id: format!("toolu_{name}"),
            name: name.into(),
            ts: Some(NOW - secs * 1000),
            summary: "git status".into(),
        }
    }

    fn prompt(tool: &str, seen_secs: i64) -> WrapperPrompt {
        WrapperPrompt {
            request_id: format!("req-{tool}"),
            tool: tool.into(),
            tool_use_id: Some(format!("toolu_{tool}")),
            headline: format!("{tool} wants to run"),
            summary: "cargo test".into(),
            first_seen_ms: NOW - seen_secs * 1000,
        }
    }

    struct Case<'a> {
        registry: bool,
        record: Option<&'a ProcRecord>,
        wrapper: bool,
        prompts: &'a [WrapperPrompt],
        fold: &'a FoldSummary,
        subagent: Option<i64>,
    }

    impl Case<'_> {
        fn derive(&self) -> Activity {
            derive(&Evidence {
                now_ms: NOW,
                registry_available: self.registry,
                record: self.record,
                wrapper_alive: self.wrapper,
                prompts: self.prompts,
                fold: self.fold,
                subagent_write_ms: self.subagent,
            })
        }
    }

    fn a(record: Option<&ProcRecord>, fold: &FoldSummary) -> Activity {
        Case {
            registry: true,
            record,
            wrapper: true,
            prompts: &[],
            fold,
            subagent: None,
        }
        .derive()
    }

    #[test]
    fn regime_a_rows() {
        // Row 1: the registry says waiting, the wrapper names the prompt.
        let r = rec(Some("waiting"), Some(NOW - 4_000));
        let f = FoldSummary {
            pending: vec![pending("Bash", 5)],
            ..open_turn(30)
        };
        let p = [prompt("Bash", 4)];
        let x = Case {
            registry: true,
            record: Some(&r),
            wrapper: true,
            prompts: &p,
            fold: &f,
            subagent: None,
        }
        .derive();
        assert_eq!(
            (x.kind, x.detail.as_str(), x.source),
            (Kind::Waiting, "Bash wants to run", "registry")
        );
        let on = x.waiting_on.unwrap();
        assert_eq!(
            (on.kind, on.request_id.as_deref(), on.tool_use_id.as_deref()),
            ("permission", Some("req-Bash"), Some("toolu_Bash"))
        );
        assert_eq!(x.since_ms, Some(NOW - 4_000));

        // Row 1, no wrapper: the transcript's question.
        let f = FoldSummary {
            ask_pending: Some(AskPending {
                id: "q1".into(),
                kind: AskKind::Question,
                summary: "Which delivery?".into(),
                ts: Some(NOW - 9_000),
            }),
            pending: vec![PendingTool {
                id: "q1".into(),
                name: "AskUserQuestion".into(),
                ts: Some(NOW - 9_000),
                summary: "Which delivery?".into(),
            }],
            ..open_turn(30)
        };
        let x = a(Some(&r), &f);
        assert_eq!(
            (x.kind, x.detail.as_str()),
            (Kind::Waiting, "Waiting for your answer")
        );
        assert_eq!(x.waiting_on.as_ref().unwrap().kind, "question");
        assert_eq!(x.since_ms, Some(NOW - 4_000), "the registry dates the wait");

        // Row 1, nothing but the registry's label: a terminal session's prompt.
        for (label, kind, detail) in [
            ("permission prompt", "permission", "Waiting for permission"),
            ("input needed", "question", "Waiting for your answer"),
            ("dialog open", "dialog", "Waiting on a dialog"),
        ] {
            let r = ProcRecord {
                waiting_for: Some(label.into()),
                ..rec(Some("waiting"), Some(NOW - 2_000))
            };
            let x = a(Some(&r), &open_turn(30));
            assert_eq!(
                (
                    x.kind,
                    x.waiting_on.as_ref().unwrap().kind,
                    x.detail.as_str()
                ),
                (Kind::Waiting, kind, detail)
            );
        }

        // Row 2: busy, a call outstanding.
        let r = rec(Some("busy"), Some(NOW - 30_000));
        let f = FoldSummary {
            pending: vec![pending("Bash", 3)],
            ..open_turn(30)
        };
        let x = a(Some(&r), &f);
        assert_eq!(
            (x.kind, x.detail.as_str(), x.tool.as_deref()),
            (Kind::Tool, "Running Bash", Some("Bash"))
        );
        assert_eq!(x.since_ms, Some(NOW - 3_000));

        // Row 3: busy, nothing outstanding — the silent phase.
        let f = FoldSummary {
            last_result_ts: Some(NOW - 12_000),
            ..open_turn(30)
        };
        let x = a(Some(&r), &f);
        assert_eq!(
            (x.kind, x.detail.as_str(), x.phase),
            (Kind::Generating, "Thinking", Some("thinking"))
        );
        assert_eq!(
            x.since_ms,
            Some(NOW - 12_000),
            "since the model got its last result"
        );
        let f = FoldSummary {
            retrying: Some((2, 10)),
            ..open_turn(30)
        };
        let x = a(Some(&r), &f);
        assert_eq!(
            (x.kind, x.detail.as_str(), x.phase),
            (Kind::Generating, "Retrying (2/10)", Some("retrying"))
        );

        // Row 4: busy stamped after the last end, and fresh: the next prompt is landing.
        let r4 = rec(Some("busy"), Some(NOW - 1_000));
        let x = a(Some(&r4), &ended(20));
        assert_eq!((x.kind, x.since_ms), (Kind::Generating, Some(NOW - 1_000)));

        // Row 5: busy from before the end, 20 ms after it: registry lag.
        let r5 = rec(Some("busy"), Some(NOW - 90_000));
        let f = FoldSummary {
            turn_end_ts: Some(NOW - 20),
            last_content_ts: Some(NOW - 20),
            ..ended(0)
        };
        let x = a(Some(&r5), &f);
        assert_eq!((x.kind, x.detail.as_str()), (Kind::Idle, "Waiting for you"));

        // Row 6b: busy long after the turn ended, with a workflow out.
        let f = FoldSummary {
            bg: Bg {
                workflows: 1,
                ..Bg::default()
            },
            ..ended(600)
        };
        let x = a(Some(&r5), &f);
        assert_eq!(
            (x.kind, x.detail.as_str()),
            (Kind::Background, "Background workflow running")
        );
        assert_eq!(
            x.background,
            Some(Background {
                agents: 0,
                workflows: 1,
                shells: 0
            })
        );
        assert_eq!(x.since_ms, Some(NOW - 600_000));

        // Row 7: idle after an API error.
        let r7 = rec(Some("idle"), Some(NOW - 5_000));
        let f = FoldSummary {
            failed: true,
            last_end: Some(LastEnd::ApiError),
            error_text: Some("API Error: 529 Overloaded".into()),
            ..ended(5)
        };
        let x = a(Some(&r7), &f);
        assert_eq!(
            (x.kind, x.detail.as_str()),
            (Kind::Failed, "API error: 529 Overloaded")
        );

        // Row 8: idle, plain, interrupted, and with a background shell.
        assert_eq!(a(Some(&r7), &ended(5)).detail, "Waiting for you");
        let f = FoldSummary {
            last_end: Some(LastEnd::Interrupt),
            ..ended(5)
        };
        assert_eq!(a(Some(&r7), &f).detail, "Interrupted");
        let shell = rec(Some("shell"), Some(NOW - 5_000));
        assert_eq!(
            a(Some(&shell), &ended(5)).detail,
            "Waiting for you · background shell running"
        );
        let f = FoldSummary {
            bg: Bg {
                shells: 1,
                ..Bg::default()
            },
            ..ended(5)
        };
        let x = a(Some(&r7), &f);
        assert_eq!(
            (x.kind, x.detail.as_str()),
            (Kind::Idle, "Waiting for you · background shell running")
        );
    }

    /// One line per row of the derivation table, in both regimes, so a change to any rule
    /// names the row it broke.
    #[test]
    fn every_row_of_the_derivation_table() {
        let busy = rec(Some("busy"), Some(NOW - 30_000));
        let busy_fresh = rec(Some("busy"), Some(NOW - 1_000));
        let busy_old = rec(Some("busy"), Some(NOW - 900_000));
        let waiting = ProcRecord {
            waiting_for: Some("permission prompt".into()),
            ..rec(Some("waiting"), Some(NOW - 2_000))
        };
        let idle_r = rec(Some("idle"), Some(NOW - 60_000));
        let shell = rec(Some("shell"), Some(NOW - 60_000));
        let nostatus = rec(None, None);
        let tool = FoldSummary {
            pending: vec![pending("Bash", 3)],
            ..open_turn(30)
        };
        let thinking = open_turn(30);
        let long_think = open_turn(20 * 60);
        let just_ended = FoldSummary {
            turn_end_ts: Some(NOW - 20),
            last_content_ts: Some(NOW - 20),
            ..ended(0)
        };
        let workflow_out = FoldSummary {
            bg: Bg {
                workflows: 1,
                ..Bg::default()
            },
            ..ended(600)
        };
        let failed = FoldSummary {
            failed: true,
            last_end: Some(LastEnd::ApiError),
            error_text: Some("Overloaded".into()),
            ..ended(30)
        };
        let interrupted = FoldSummary {
            last_end: Some(LastEnd::Interrupt),
            ..ended(30)
        };
        let asked = FoldSummary {
            ask_pending: Some(AskPending {
                id: "q".into(),
                kind: AskKind::Question,
                summary: "Which?".into(),
                ts: Some(NOW - 5_000),
            }),
            ..open_turn(30)
        };
        let gone = FoldSummary {
            exited: true,
            exit_ms: Some(NOW - 50_000),
            ..ended(60)
        };
        let agent_out = FoldSummary {
            bg: Bg {
                agents: 1,
                ..Bg::default()
            },
            ..ended(20)
        };
        let failed_just_now = FoldSummary {
            turn_end_ts: Some(NOW - 20),
            last_content_ts: Some(NOW - 20),
            ..failed.clone()
        };
        let busy_after_end = rec(Some("busy"), Some(NOW - 10_000));
        let (ended20, ended30) = (ended(20), ended(30));
        let p = [prompt("Bash", 2)];
        let none: [WrapperPrompt; 0] = [];

        #[rustfmt::skip]
        let table: Vec<(&str, bool, Option<&ProcRecord>, &[WrapperPrompt], &FoldSummary, Option<i64>, Kind, &str)> = vec![
            ("step 2  closed, clean exit",          true,  None,              &none, &gone,         None,                Kind::Closed,     "Session ended"),
            ("A row 1 waiting (wrapper prompt)",    true,  Some(&waiting),    &p,    &tool,         None,                Kind::Waiting,    "Bash wants to run"),
            ("A row 1 waiting (registry only)",     true,  Some(&waiting),    &none, &thinking,     None,                Kind::Waiting,    "Waiting for permission"),
            ("A row 2 tool",                        true,  Some(&busy),       &none, &tool,         None,                Kind::Tool,       "Running Bash"),
            ("A row 3 generating",                  true,  Some(&busy),       &none, &thinking,     None,                Kind::Generating, "Thinking"),
            ("A row 3 stalled overlay",             true,  Some(&busy_old),   &none, &long_think,   None,                Kind::Stalled,    "No progress"),
            ("A row 4 prompt about to land",        true,  Some(&busy_fresh), &none, &ended20,      None,                Kind::Generating, "Thinking"),
            ("A row 4 never with agents out",       true,  Some(&busy_fresh), &none, &agent_out,    None,                Kind::Background, "1 background agent running"),
            ("A row 5 registry lag",                true,  Some(&busy_old),   &none, &just_ended,   None,                Kind::Idle,       "Waiting for you"),
            ("A row 5 registry lag, API error",     true,  Some(&busy_old),   &none, &failed_just_now, None,             Kind::Failed,     "API error: Overloaded"),
            ("A row 6a busy after the end",         true,  Some(&busy_after_end), &none, &ended20,  None,                Kind::Generating, "Working"),
            ("A row 6b background",                 true,  Some(&busy_old),   &none, &workflow_out, None,                Kind::Background, "Background workflow running"),
            ("A row 6b background (subagents)",     true,  Some(&busy_after_end), &none, &ended20,  Some(NOW - 5_000),   Kind::Background, "Background work running"),
            ("A row 6b background (idle, agents)",  true,  Some(&idle_r),     &none, &workflow_out, Some(NOW - 5_000),   Kind::Background, "Background workflow running"),
            ("A row 6c busy held past the end",     true,  Some(&busy),       &none, &ended20,      None,                Kind::Background, "Working"),
            ("A row 7 failed",                      true,  Some(&idle_r),     &none, &failed,       None,                Kind::Failed,     "API error: Overloaded"),
            ("A row 8 idle",                        true,  Some(&idle_r),     &none, &ended30,      None,                Kind::Idle,       "Waiting for you"),
            ("A row 8 idle, shell",                 true,  Some(&shell),      &none, &ended30,      None,                Kind::Idle,       "Waiting for you · background shell running"),
            ("A row 8 interrupted",                 true,  Some(&idle_r),     &none, &interrupted,  None,                Kind::Idle,       "Interrupted"),
            ("B row 1 waiting (transcript ask)",    true,  Some(&nostatus),   &none, &asked,        None,                Kind::Waiting,    "Waiting for your answer"),
            ("B row 1 waiting (wrapper prompt)",    false, None,              &p,    &thinking,     None,                Kind::Waiting,    "Bash wants to run"),
            ("B row 2 tool",                        true,  Some(&nostatus),   &none, &tool,         None,                Kind::Tool,       "Running Bash"),
            ("B row 3 generating",                  false, None,              &none, &thinking,     None,                Kind::Generating, "Thinking"),
            ("B row 3 stalled overlay",             false, None,              &none, &long_think,   None,                Kind::Stalled,    "No progress"),
            ("B row 6b background",                 true,  Some(&nostatus),   &none, &workflow_out, Some(NOW - 5_000),   Kind::Background, "Background workflow running"),
            ("B row 7 failed",                      false, None,              &none, &failed,       None,                Kind::Failed,     "API error: Overloaded"),
            ("B row 8 idle",                        true,  Some(&nostatus),   &none, &ended30,      None,                Kind::Idle,       "Waiting for you"),
        ];
        for (name, registry, record, prompts, fold, subagent, kind, detail) in table {
            let x = Case {
                registry,
                record,
                wrapper: true,
                prompts,
                fold,
                subagent,
            }
            .derive();
            assert_eq!((x.kind, x.detail.as_str()), (kind, detail), "{name}");
            let want_source = if name.starts_with("A ") || name.starts_with("step 2") {
                "registry"
            } else {
                "transcript"
            };
            assert_eq!(x.source, want_source, "{name}");
        }
    }

    #[test]
    fn regime_a_corrections() {
        // Idle stamped before a turn that has started since: the registry is behind.
        let r = rec(Some("idle"), Some(NOW - 10_000));
        let x = a(Some(&r), &open_turn(2));
        assert_eq!(x.kind, Kind::Generating);

        // Idle, but agents are out and their transcripts are moving.
        let f = FoldSummary {
            bg: Bg {
                agents: 3,
                ..Bg::default()
            },
            ..ended(300)
        };
        let x = Case {
            registry: true,
            record: Some(&r),
            wrapper: true,
            prompts: &[],
            fold: &f,
            subagent: Some(NOW - 30_000),
        }
        .derive();
        assert_eq!(
            (x.kind, x.detail.as_str()),
            (Kind::Background, "3 background agents running")
        );
        // ...and when those transcripts went quiet long ago, the ledger alone is not enough.
        let x = Case {
            registry: true,
            record: Some(&r),
            wrapper: true,
            prompts: &[],
            fold: &f,
            subagent: Some(NOW - 600_000),
        }
        .derive();
        assert_eq!(x.kind, Kind::Idle);
    }

    #[test]
    fn regime_b_rows() {
        // A record without a status (a fresh registration, an older headless CLI).
        let r = rec(None, None);
        let case = |f: &FoldSummary, p: &[WrapperPrompt], sub: Option<i64>| {
            Case {
                registry: true,
                record: Some(&r),
                wrapper: false,
                prompts: p,
                fold: f,
                subagent: sub,
            }
            .derive()
        };
        // Waiting: a wrapper prompt, or a question in the transcript.
        let x = case(&open_turn(10), &[prompt("Edit", 3)], None);
        assert_eq!((x.kind, x.source), (Kind::Waiting, "transcript"));
        let f = FoldSummary {
            ask_pending: Some(AskPending {
                id: "p1".into(),
                kind: AskKind::Plan,
                summary: "Rework".into(),
                ts: Some(NOW - 60_000),
            }),
            ..open_turn(90)
        };
        let x = case(&f, &[], None);
        assert_eq!(
            (x.kind, x.detail.as_str(), x.since_ms),
            (
                Kind::Waiting,
                "Plan waiting for approval",
                Some(NOW - 60_000)
            )
        );
        // Busy: the turn counts.
        let f = FoldSummary {
            pending: vec![pending("Read", 1)],
            ..open_turn(10)
        };
        assert_eq!(case(&f, &[], None).kind, Kind::Tool);
        assert_eq!(case(&open_turn(10), &[], None).kind, Kind::Generating);
        // Background: agents out and subagents writing.
        let f = FoldSummary {
            bg: Bg {
                agents: 1,
                ..Bg::default()
            },
            ..ended(60)
        };
        let x = case(&f, &[], Some(NOW - 5_000));
        assert_eq!(
            (x.kind, x.detail.as_str()),
            (Kind::Background, "1 background agent running")
        );
        // Idle and failed.
        assert_eq!(case(&ended(60), &[], None).kind, Kind::Idle);
        let f = FoldSummary {
            failed: true,
            last_end: Some(LastEnd::ApiError),
            error_text: Some("You've hit your weekly limit".into()),
            ..ended(60)
        };
        assert_eq!(
            case(&f, &[], None).detail,
            "API error: You've hit your weekly limit"
        );
    }

    #[test]
    fn an_unknown_status_is_read_from_the_transcript() {
        let r = rec(Some("compacting"), Some(NOW - 100_000));
        let x = a(Some(&r), &open_turn(150));
        assert_eq!(
            (x.kind, x.source),
            (Kind::Generating, "transcript"),
            "not idle"
        );
    }

    #[test]
    fn the_110_second_think_is_generating() {
        // The prompt landed 110 s ago and nothing has been written since.
        let r = rec(Some("busy"), Some(NOW - 110_050));
        let x = a(Some(&r), &open_turn(110));
        assert_eq!(
            (x.kind, x.legacy_list_state(), x.active()),
            (Kind::Generating, "working", true)
        );
        // The same with no registry at all.
        let x = Case {
            registry: false,
            record: None,
            wrapper: false,
            prompts: &[],
            fold: &open_turn(110),
            subagent: None,
        }
        .derive();
        assert_eq!((x.kind, x.open), (Kind::Generating, None));
    }

    #[test]
    fn the_orchestrator_after_its_end_turn_is_background() {
        // d6e4d7da: busy stamped at 09:35:15.572, end_turn at 09:41:53.044, a workflow out.
        let end = NOW - 20 * 60_000;
        let r = rec(Some("busy"), Some(end - 397_472));
        let f = FoldSummary {
            turn_open: false,
            turn_start_ts: Some(end - 397_472),
            turn_end_ts: Some(end),
            last_end: Some(LastEnd::EndTurn),
            last_content_ts: Some(end),
            bg: Bg {
                workflows: 1,
                shells: 0,
                ..Bg::default()
            },
            ..Default::default()
        };
        let x = Case {
            registry: true,
            record: Some(&r),
            wrapper: true,
            prompts: &[],
            fold: &f,
            subagent: Some(NOW - 2_000),
        }
        .derive();
        assert_eq!(
            (x.kind, x.legacy_list_state(), x.stoppable()),
            (Kind::Background, "working", false)
        );
    }

    #[test]
    fn a_restamp_after_the_end_with_subagents_writing_is_background() {
        // A background agent asks permission and is answered 10 s after end_turn:
        // busy→waiting→busy restamps R.t, while the agent's transcript keeps growing. 5 s
        // later it is neither fresh enough to be a landing prompt nor the main loop at work.
        let f = ended(15);
        let r = rec(Some("busy"), Some(NOW - 5_000));
        let x = Case {
            registry: true,
            record: Some(&r),
            wrapper: true,
            prompts: &[],
            fold: &f,
            subagent: Some(NOW - 2_000),
        }
        .derive();
        assert_eq!(
            (x.kind, x.detail.as_str()),
            (Kind::Background, "Background work running")
        );
        assert!(!x.stoppable());
    }

    #[test]
    fn an_answered_prompt_of_a_background_agent_is_not_a_new_turn() {
        // The turn ended with an agent out; the agent asked permission and was answered a
        // second ago, restamping busy after the end. With agents out the CLI already holds
        // busy and never rewrites busy→busy, so a fresh stamp then is never a new prompt —
        // and a Stop now would land on nothing but the agent's work.
        let f = FoldSummary {
            bg: Bg {
                agents: 1,
                ..Bg::default()
            },
            ..ended(60)
        };
        let r = rec(Some("busy"), Some(NOW - 1_000));
        let x = a(Some(&r), &f);
        assert_eq!(
            (x.kind, x.detail.as_str()),
            (Kind::Background, "1 background agent running")
        );
        assert!(!x.stoppable());
    }

    #[test]
    fn busy_stamped_after_the_turn_with_nothing_out_is_the_main_loop_at_work() {
        // A manual /compact after the turn: the registry goes busy, and the transcript says
        // nothing until the compaction is over (16.7 s for a tiny conversation, minutes for
        // a real one). Nothing runs in the background; stopping it aborts the compaction
        // and discards nothing.
        let r = rec(Some("busy"), Some(NOW - 5_000));
        let x = a(Some(&r), &ended(60));
        assert_eq!(
            (x.kind, x.detail.as_str(), x.phase),
            (Kind::Generating, "Working", None)
        );
        assert_eq!(
            x.since_ms,
            Some(NOW - 5_000),
            "since the registry went busy, not since the reply before"
        );
        assert_eq!(x.background, None);
        assert!(x.stoppable());
        assert_eq!((x.legacy_list_state(), x.tone()), ("working", "busy"));
        // Ten minutes of it with nothing to show is a stall like any other generation.
        let r = rec(Some("busy"), Some(NOW - 11 * 60_000));
        let x = a(Some(&r), &ended(12 * 60));
        assert_eq!((x.kind, x.detail.as_str()), (Kind::Stalled, "No progress"));
    }

    #[test]
    fn busy_held_past_the_end_with_nothing_to_see_is_working_but_not_stoppable() {
        // Busy stamped when the turn began and held after its end_turn: a Stop hook (15 s in
        // the probe), or a task the transcript cannot see — remote_agent, mcp_task,
        // auto_mode_scan, a teammate. No counts it cannot see are claimed; and since it may
        // be background work, Stop is not offered.
        let r = rec(Some("busy"), Some(NOW - 68_000));
        let x = a(Some(&r), &ended(8));
        assert_eq!((x.kind, x.detail.as_str()), (Kind::Background, "Working"));
        assert_eq!(x.background, None, "no counts it cannot see");
        assert_eq!(x.since_ms, Some(NOW - 8_000), "since the turn ended");
        assert!(!x.stoppable());
        assert_eq!(
            (x.legacy_list_state(), x.active(), x.tone()),
            ("working", true, "busy")
        );
        // However long it lasts: never "No progress" with Stop on offer.
        let r = rec(Some("busy"), Some(NOW - 21 * 60_000));
        let x = a(Some(&r), &ended(20 * 60));
        assert_eq!((x.kind, x.detail.as_str()), (Kind::Background, "Working"));
    }

    #[test]
    fn an_api_error_reads_failed_through_the_registry_lag_too() {
        // The error line landed 50 ms ago; the snapshot still holds the busy stamped at the
        // turn's start. What it says must be what it will say once the registry catches up.
        let f = FoldSummary {
            failed: true,
            last_end: Some(LastEnd::ApiError),
            error_text: Some("API Error: Connection refused".into()),
            turn_end_ts: Some(NOW - 50),
            last_content_ts: Some(NOW - 50),
            ..ended(0)
        };
        let lagging = a(Some(&rec(Some("busy"), Some(NOW - 60_000))), &f);
        assert_eq!(
            (lagging.kind, lagging.detail.as_str(), lagging.since_ms),
            (
                Kind::Failed,
                "API error: Connection refused",
                Some(NOW - 50)
            )
        );
        assert_eq!(lagging, a(Some(&rec(Some("idle"), Some(NOW - 45))), &f));
        // A plain end during the same lag is still the idle it is about to be.
        let f = FoldSummary {
            turn_end_ts: Some(NOW - 50),
            last_content_ts: Some(NOW - 50),
            ..ended(0)
        };
        let x = a(Some(&rec(Some("busy"), Some(NOW - 60_000))), &f);
        assert_eq!((x.kind, x.detail.as_str()), (Kind::Idle, "Waiting for you"));
    }

    #[test]
    fn an_idle_session_with_a_shell_running_carries_its_count() {
        // The row line a client shows for it keys on the structured count, not the words.
        let x = a(Some(&rec(Some("shell"), Some(NOW - 5_000))), &ended(5));
        assert_eq!(
            (x.kind, x.background),
            (
                Kind::Idle,
                Some(Background {
                    agents: 0,
                    workflows: 0,
                    shells: 1
                })
            )
        );
        let f = FoldSummary {
            bg: Bg {
                shells: 2,
                ..Bg::default()
            },
            ..ended(5)
        };
        let x = a(Some(&rec(Some("idle"), Some(NOW - 5_000))), &f);
        assert_eq!(
            x.background,
            Some(Background {
                agents: 0,
                workflows: 0,
                shells: 2
            })
        );
        assert!(!x.busy() && !x.active(), "a shell does not make it busy");
        let x = a(Some(&rec(Some("idle"), Some(NOW - 5_000))), &ended(5));
        assert_eq!(x.background, None, "plain idle carries none");
    }

    #[test]
    fn closed_sessions() {
        // A clean exit, with its exit marker.
        let f = FoldSummary {
            exited: true,
            exit_ms: Some(NOW - 7_200_000),
            ..ended(10_000)
        };
        let x = a(None, &f);
        assert_eq!(
            (x.kind, x.open, x.ended_how, x.mid_turn),
            (Kind::Closed, Some(false), Some("exit"), Some(false))
        );
        assert_eq!(
            (x.ended_ms, x.detail.as_str()),
            (Some(NOW - 7_200_000), "Session ended")
        );
        assert_eq!(
            (
                x.legacy_list_state(),
                x.legacy_stream_state(),
                x.active(),
                x.tone()
            ),
            ("idle", "idle", false, "closed")
        );

        // 738b7b63: a task-notification turn, then nothing — the process died in it.
        let f = FoldSummary {
            last_content_ts: Some(NOW - 13 * 86_400_000),
            ..open_turn(13 * 86_400)
        };
        let x = a(None, &f);
        assert_eq!(
            (x.kind, x.ended_how, x.mid_turn),
            (Kind::Closed, Some("unknown"), Some(true))
        );
        assert_eq!(
            (x.detail.as_str(), x.ended_ms),
            ("Session ended mid-turn", Some(NOW - 13 * 86_400_000))
        );
        assert!(!x.stoppable(), "nothing to stop");
    }

    #[test]
    fn a_turn_the_previous_process_left_open_does_not_count() {
        // Resumed after a crash: the transcript still ends in the crashed turn.
        let f = FoldSummary {
            turn_start_ts: Some(STARTED - 86_400_000),
            ..open_turn(90_000)
        };
        let r = rec(Some("idle"), Some(STARTED + 1_000));
        assert!(!turn_counts(&f, Some(&r)));
        assert_eq!(a(Some(&r), &f).kind, Kind::Idle);
        // Without a registry there is nothing to date the process by, and it counts.
        assert!(turn_counts(&f, None));
        // A turn begun during the registration lag is this process's.
        let f = FoldSummary {
            turn_start_ts: Some(STARTED - 4_000),
            ..open_turn(10)
        };
        assert!(turn_counts(&f, Some(&r)));
    }

    #[test]
    fn background_an_earlier_process_launched_is_not_this_ones() {
        // The ledger still lists a workflow and a shell launched before this process
        // registered: stopped from the editor, or killed with the old process.
        let f = FoldSummary {
            bg: Bg {
                workflows: 1,
                shells: 1,
                ..Bg::default()
            },
            bg_tasks: vec![
                BgTask {
                    id: "wold".into(),
                    kind: "workflow",
                    launched_ts: Some(STARTED - 600_000),
                },
                BgTask {
                    id: "bold".into(),
                    kind: "shell",
                    launched_ts: Some(STARTED - 600_000),
                },
            ],
            ..ended(60)
        };
        let r = rec(Some("idle"), Some(NOW - 60_000));
        let x = Case {
            registry: true,
            record: Some(&r),
            wrapper: true,
            prompts: &[],
            fold: &f,
            subagent: Some(NOW - 5_000),
        }
        .derive();
        assert_eq!((x.kind, x.detail.as_str()), (Kind::Idle, "Waiting for you"));
        // The same tasks launched by this process are live work.
        let f = FoldSummary {
            bg_tasks: vec![
                BgTask {
                    id: "wnew".into(),
                    kind: "workflow",
                    launched_ts: Some(NOW - 120_000),
                },
                BgTask {
                    id: "bnew".into(),
                    kind: "shell",
                    launched_ts: Some(NOW - 120_000),
                },
            ],
            ..f
        };
        let x = Case {
            registry: true,
            record: Some(&r),
            wrapper: true,
            prompts: &[],
            fold: &f,
            subagent: Some(NOW - 5_000),
        }
        .derive();
        assert_eq!(
            (x.kind, x.detail.as_str()),
            (Kind::Background, "Background workflow running")
        );
        assert_eq!(
            x.background,
            Some(Background {
                agents: 0,
                workflows: 1,
                shells: 1
            })
        );
    }

    #[test]
    fn a_wrapper_prompt_on_an_idle_session_expires() {
        let r = rec(Some("idle"), Some(NOW - 60_000));
        let p = [prompt("Bash", 11)];
        let x = Case {
            registry: true,
            record: Some(&r),
            wrapper: true,
            prompts: &p,
            fold: &ended(60),
            subagent: None,
        }
        .derive();
        assert_eq!(x.kind, Kind::Idle, "idle cannot be blocked on a prompt");
        let p = [prompt("Bash", 3)];
        let x = Case {
            registry: true,
            record: Some(&r),
            wrapper: true,
            prompts: &p,
            fold: &ended(60),
            subagent: None,
        }
        .derive();
        assert_eq!(
            x.kind,
            Kind::Waiting,
            "a fresh one may simply be ahead of the registry"
        );
        // The newest of several is the one shown, by arrival.
        let p = [prompt("Edit", 2), prompt("Bash", 5)];
        let r = rec(Some("waiting"), Some(NOW - 2_000));
        let x = Case {
            registry: true,
            record: Some(&r),
            wrapper: true,
            prompts: &p,
            fold: &open_turn(30),
            subagent: None,
        }
        .derive();
        assert_eq!(
            x.waiting_on.unwrap().request_id.as_deref(),
            Some("req-Edit")
        );
        // A question and a plan read as themselves.
        let r = rec(Some("waiting"), Some(NOW - 2_000));
        for (tool, detail) in [
            ("AskUserQuestion", "Waiting for your answer"),
            ("ExitPlanMode", "Plan waiting for approval"),
        ] {
            let p = [prompt(tool, 1)];
            let x = Case {
                registry: true,
                record: Some(&r),
                wrapper: true,
                prompts: &p,
                fold: &open_turn(30),
                subagent: None,
            }
            .derive();
            assert_eq!(x.detail, detail);
        }
    }

    #[test]
    fn stalled_is_generation_without_progress_and_never_a_tool() {
        let r = rec(Some("busy"), Some(NOW - 20 * 60_000));
        let x = a(Some(&r), &open_turn(20 * 60));
        assert_eq!(
            (
                x.kind,
                x.detail.as_str(),
                x.legacy_list_state(),
                x.legacy_stream_state()
            ),
            (Kind::Stalled, "No progress", "stalled", "working")
        );
        assert_eq!(x.since_ms, Some(NOW - 20 * 60_000));
        assert!(x.stoppable());
        // Subagents writing is progress.
        let x = Case {
            registry: true,
            record: Some(&r),
            wrapper: true,
            prompts: &[],
            fold: &open_turn(20 * 60),
            subagent: Some(NOW - 10_000),
        }
        .derive();
        assert_eq!(x.kind, Kind::Generating);
        // A two-hour tool is a two-hour tool (in a process older than the turn).
        let r = ProcRecord {
            started_at_ms: Some(NOW - 8_000_000),
            ..r
        };
        let f = FoldSummary {
            pending: vec![pending("Bash", 7200)],
            ..open_turn(7300)
        };
        assert_eq!(a(Some(&r), &f).kind, Kind::Tool);
    }

    #[test]
    fn without_a_registry_openness_comes_from_the_wrapper_or_is_unknown() {
        let x = Case {
            registry: false,
            record: None,
            wrapper: true,
            prompts: &[],
            fold: &ended(30),
            subagent: None,
        }
        .derive();
        assert_eq!((x.kind, x.open), (Kind::Idle, Some(true)));
        let x = Case {
            registry: false,
            record: None,
            wrapper: false,
            prompts: &[],
            fold: &ended(30),
            subagent: None,
        }
        .derive();
        assert_eq!(
            (x.kind, x.open),
            (Kind::Idle, None),
            "never closed on a guess"
        );
    }

    /// What the three modules make of this machine right now: the registry, each open
    /// session's transcript (its last `INITIAL_WINDOW`, as the router first reads it) and
    /// its subagent writes. Read-only; never opens a key file, never touches a socket.
    /// `cargo test -p agent-router activity::tests::live_sessions_now -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_sessions_now() {
        use std::io::{Read, Seek, SeekFrom};
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        let snap = crate::registry::snapshot_in(&crate::registry::sessions_dir(), now);
        println!(
            "registry available={} live sessions={}",
            snap.available,
            snap.by_session.len()
        );
        let projects = crate::registry::claude_dir().join("projects");
        let mtime = |p: &std::path::Path| {
            std::fs::metadata(p)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as i64)
        };
        let mut ids: Vec<&String> = snap.by_session.keys().collect();
        ids.sort();
        for id in ids {
            let r = &snap.by_session[id];
            let path = std::fs::read_dir(&projects)
                .unwrap()
                .flatten()
                .map(|d| d.path().join(format!("{id}.jsonl")))
                .find(|p| p.is_file());
            let Some(path) = path else {
                println!(
                    "{} pid={} status={:?}: no transcript (not listed)",
                    &id[..8],
                    r.pid,
                    r.status
                );
                continue;
            };
            let mut file = std::fs::File::open(&path).unwrap();
            let len = file.metadata().unwrap().len();
            let from = crate::fold::initial_offset(len);
            file.seek(SeekFrom::Start(from)).unwrap();
            let mut buf = Vec::new();
            file.read_to_end(&mut buf).unwrap();
            let mut f = crate::fold::Fold::new();
            let skip = crate::fold::skip_partial_line(&buf, from > 0);
            f.feed(&buf[skip..]);
            let s = f.summary();
            // A: the newest subagent or workflow-agent transcript write.
            let sub = path.with_extension("").join("subagents");
            let mut newest: Option<i64> = None;
            let mut look = |dir: &std::path::Path| {
                for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
                    if e.path().extension().and_then(|x| x.to_str()) == Some("jsonl") {
                        newest = newest.max(mtime(&e.path()));
                    }
                }
            };
            look(&sub);
            for wf in std::fs::read_dir(sub.join("workflows"))
                .into_iter()
                .flatten()
                .flatten()
            {
                look(&wf.path());
            }
            let x = derive(&Evidence {
                now_ms: now,
                registry_available: snap.available,
                record: Some(r),
                wrapper_alive: false,
                prompts: &[],
                fold: &s,
                subagent_write_ms: newest,
            });
            println!(
                "{} pid={} status={:?} since_status={}s | fold: turn_open={} pending={} bg={:?} last_content={}s ago | subagents {} | => {:?} {:?} list={} stream={} active={} since={}s source={}",
                &id[..8], r.pid, r.status.as_ref().map(|s| s.as_str()),
                r.status_updated_ms.map_or(-1, |t| (now - t) / 1000),
                s.turn_open, s.pending.len(), s.bg_since(r.started_at_ms, TURN_SLACK_MS),
                s.last_content_ts.map_or(-1, |t| (now - t) / 1000),
                newest.map_or("never".to_owned(), |t| format!("{}s ago", (now - t) / 1000)),
                x.kind, x.detail, x.legacy_list_state(), x.legacy_stream_state(), x.active(),
                x.since_ms.map_or(-1, |t| (now - t) / 1000), x.source,
            );
        }
    }

    #[test]
    fn every_kind_speaks_the_legacy_vocabulary_consistently() {
        const NEVER: [&str; 5] = ["thinking", "generating", "background", "closed", "waiting"];
        for kind in Kind::ALL {
            let x = Activity::new(kind, Some(true), "registry", "");
            let (list, stream) = (x.legacy_list_state(), x.legacy_stream_state());
            assert!(
                !NEVER.contains(&list) && !NEVER.contains(&stream),
                "{kind:?}: {list}/{stream}"
            );
            // Installed phones: busy is stream tool|working; the list says working|stalled.
            let list_busy = matches!(list, "working" | "stalled");
            let stream_busy = matches!(stream, "working" | "tool");
            assert_eq!(list_busy, stream_busy, "{kind:?}");
            assert_eq!(list_busy, x.busy(), "{kind:?}");
            assert_eq!(list == "ask", stream == "ask", "{kind:?}");
            assert_eq!(list == "ask", x.needs_you(), "{kind:?}");
            assert_eq!(x.active(), list_busy || list == "ask", "{kind:?}");
            assert!(
                !x.stoppable() || x.busy(),
                "{kind:?}: only busy work can be stopped"
            );
            assert!(!x.tone().is_empty());
        }
        let bg = Activity::new(Kind::Background, Some(true), "registry", "");
        assert!(
            bg.busy() && !bg.stoppable(),
            "background is busy but not stoppable"
        );
        assert_eq!(
            serde_json::to_string(&Kind::Generating).unwrap(),
            "\"generating\""
        );
    }
}
