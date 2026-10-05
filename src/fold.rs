//! What a session is doing, as far as its own transcript can tell: one incremental fold
//! per transcript.
//!
//! The main transcript is the one source every session has, whatever hosts it, but it is
//! a record of what *happened*, written on its own schedule, and reading state off it
//! takes more care than it looks:
//!
//!   * An assistant message is written only once the whole message is complete, every
//!     line of it already carrying the message's final `stop_reason`. Nothing at all is
//!     written while the model thinks — measured silences of 110 s — so "no new line"
//!     never means "idle". A turn is open from its prompt until something closes it.
//!   * File order is not event order. A fast tool's result can land before its own call
//!     (an async Agent launch, a Skill), a compaction re-appends a preserved stretch of
//!     old history with the same uuids and timestamps, and slash-command echoes arrive
//!     late. So calls and results are paired by id in either order, re-appended lines are
//!     recognised and skipped, and only content lines move the clock.
//!   * Most of what gets appended is bookkeeping: titles, modes, ledgers, file history,
//!     attachments, the exit flush. None of it is activity, and none of it may count as
//!     such — that is how a closed session used to jump to the top of the list.
//!   * It says nothing about whether the process is alive, and nothing about permission
//!     prompts. Those come from the registry and the wrapper; see `activity`.
//!
//! This module is PURE: it is handed complete lines and never touches the filesystem.
//! The byte offset, the partial-line carry and the reset on shrink live with the caller,
//! which folds only what was appended since it last looked.

use std::collections::{HashSet, VecDeque};

use serde::Serialize;
use serde_json::Value;

/// How much of a transcript is folded when it is first seen. Enough to hold the current
/// turn in practice — a turn start further back than this is re-derived from the next
/// assistant line — while keeping a cold start over sixty transcripts well under a second.
#[allow(dead_code)]
pub const INITIAL_WINDOW: u64 = 2 * 1024 * 1024;
/// A user or assistant line this far behind the newest content already folded is history
/// being re-appended (compaction keeps the original timestamps), not something new.
const REAPPEND_SLACK_MS: i64 = 5 * 60_000;
/// Uuids remembered for de-duplication. Compaction re-appends from anywhere in the file,
/// so this is sized to cover a whole initial window with room to spare.
const UUIDS_KEPT: usize = 20_000;
/// Results seen before their calls, remembered until the call turns up.
const SETTLED_KEPT: usize = 512;
/// Background tasks tracked at once; the oldest is forgotten first.
const LEDGER_KEPT: usize = 256;
/// Calls outstanding at once. A turn never legitimately has this many in flight; the
/// bound only stops a pathological file from growing the list without end.
const PENDING_KEPT: usize = 256;
/// The meta line a resumed session writes when the process that ran its last turn died.
const RESUME_MARKER: &str = "Continue from where you left off.";
/// Content stamped this soon after the exit time is the shutdown's own last write, not a
/// resume: a background task that ends as the process goes down still gets its
/// notification turn written, 7–18 ms after the exit (c0b3be1f). A resume takes seconds:
/// a new process has to start and register before it can write anything.
const EXIT_SLACK_MS: i64 = 2_000;
/// Task statuses that mean a background task is over.
const FINAL_STATUSES: [&str; 4] = ["completed", "failed", "killed", "stopped"];

/// How the last turn ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LastEnd {
    /// The model said it was done: `end_turn`, `stop_sequence`, `max_tokens` or `refusal`.
    EndTurn,
    /// Someone pressed stop.
    Interrupt,
    /// The request failed for good, after the CLI's own retries.
    ApiError,
}

/// The two tool calls that are the session asking *you* something rather than doing work.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AskKind {
    Question,
    Plan,
}

impl AskKind {
    #[allow(dead_code)]
    pub fn as_str(self) -> &'static str {
        match self {
            AskKind::Question => "question",
            AskKind::Plan => "plan",
        }
    }
}

/// A tool call written and not yet answered.
#[derive(Clone, Debug, PartialEq)]
pub struct PendingTool {
    pub id: String,
    pub name: String,
    /// When the call's block completed, which is also when the tool started.
    pub ts: Option<i64>,
    /// Its most identifying field, so a row reads `Bash: git status`.
    pub summary: String,
}

/// An AskUserQuestion or ExitPlanMode still waiting for its answer.
#[derive(Clone, Debug, PartialEq)]
pub struct AskPending {
    pub id: String,
    pub kind: AskKind,
    pub summary: String,
    pub ts: Option<i64>,
}

/// Background work launched by this session and not yet reported finished.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Bg {
    pub agents: usize,
    pub workflows: usize,
    pub shells: usize,
    pub monitors: usize,
}

/// One background task still outstanding, as the ledger has it.
#[derive(Clone, Debug, PartialEq)]
pub struct BgTask {
    pub id: String,
    /// agent | workflow | shell | monitor
    pub kind: &'static str,
    /// When its launch result was written.
    pub launched_ts: Option<i64>,
}

/// Everything the fold knows, as of the last line it was given.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FoldSummary {
    /// A turn is running: prompted and not yet ended.
    pub turn_open: bool,
    pub turn_start_ts: Option<i64>,
    pub turn_end_ts: Option<i64>,
    pub last_end: Option<LastEnd>,
    /// The last turn ended on an API error and nothing has been asked since.
    pub failed: bool,
    pub error_text: Option<String>,
    /// Calls outstanding in the current turn, newest last.
    pub pending: Vec<PendingTool>,
    pub ask_pending: Option<AskPending>,
    /// The CLI is retrying a failed request: (attempt, of how many).
    pub retrying: Option<(u32, u32)>,
    /// The newest tool result in the current turn: after it, the model is thinking again.
    pub last_result_ts: Option<i64>,
    /// Every outstanding task, counted.
    pub bg: Bg,
    /// The same tasks one by one, oldest first, so a caller that knows when the current
    /// process started can discount those an earlier process launched (`bg_since`).
    pub bg_tasks: Vec<BgTask>,
    /// Messages typed (or sent by a peer) while a turn ran, still waiting their turn.
    pub queued: usize,
    /// The newest content line — the only honest "last activity" a transcript has.
    pub last_content_ts: Option<i64>,
    /// The process exited cleanly after the last content line.
    pub exited: bool,
    pub exit_ms: Option<i64>,
    /// Claude Code's own name for the session, the newest it wrote.
    pub title: Option<String>,
}

impl FoldSummary {
    /// The outstanding tasks a process that registered at `started_ms` could still be
    /// running. A task ends without a word in the transcript when it is stopped from the
    /// editor's task list, or when the process that ran it dies without a clean exit; one
    /// launched before the current process started is certainly gone. With no start time
    /// to go by, everything outstanding counts.
    #[allow(dead_code)]
    pub fn bg_since(&self, started_ms: Option<i64>, slack_ms: i64) -> Bg {
        let mut bg = self.bg;
        let Some(started) = started_ms else { return bg };
        for t in &self.bg_tasks {
            if !t.launched_ts.is_some_and(|l| l < started - slack_ms) {
                continue;
            }
            let n = match t.kind {
                "agent" => &mut bg.agents,
                "workflow" => &mut bg.workflows,
                "shell" => &mut bg.shells,
                _ => &mut bg.monitors,
            };
            *n = n.saturating_sub(1);
        }
        bg
    }
}

/// Counters for checking the fold against real transcripts. Not state.
#[derive(Clone, Copy, Debug, Default)]
#[allow(dead_code)]
pub struct FoldStats {
    /// Lines handed to the fold that parsed.
    pub lines: u64,
    /// Lines that did not parse — a torn tail, or junk.
    pub unparsed: u64,
    /// Lines skipped because their uuid was already folded (compaction re-appends).
    pub reappended: u64,
    /// User and assistant lines skipped for being far behind the newest content.
    pub stale: u64,
    /// Tool calls whose result had already been seen.
    pub settled_early_hits: u64,
    /// Thinking-only closing lines held open because the CLI had asked to continue.
    pub continuation_holds: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BgKind {
    Agent,
    Workflow,
    Shell,
    Monitor,
}

impl BgKind {
    fn as_str(self) -> &'static str {
        match self {
            BgKind::Agent => "agent",
            BgKind::Workflow => "workflow",
            BgKind::Shell => "shell",
            BgKind::Monitor => "monitor",
        }
    }
}

/// A pending call plus the message it belongs to, which is what decides whether a
/// closing `stop_reason` can end the turn.
#[derive(Clone, Debug)]
struct Pending {
    tool: PendingTool,
    msg: Option<String>,
}

/// A set that forgets its oldest member once full.
#[derive(Default)]
struct Bounded {
    set: HashSet<String>,
    order: VecDeque<String>,
}

impl Bounded {
    /// True when the id was not already there.
    fn insert(&mut self, id: &str, cap: usize) -> bool {
        if self.set.contains(id) {
            return false;
        }
        self.set.insert(id.to_owned());
        self.order.push_back(id.to_owned());
        while self.order.len() > cap {
            if let Some(old) = self.order.pop_front() {
                self.set.remove(&old);
            }
        }
        true
    }

    fn remove(&mut self, id: &str) -> bool {
        if self.set.remove(id) {
            self.order.retain(|x| x != id);
            true
        } else {
            false
        }
    }

    fn clear(&mut self) {
        self.set.clear();
        self.order.clear();
    }
}

/// The running fold. Feed it lines in file order; ask it for a `summary` at any point.
#[derive(Default)]
pub struct Fold {
    turn_open: bool,
    turn_start_ts: Option<i64>,
    turn_end_ts: Option<i64>,
    last_end: Option<LastEnd>,
    failed: bool,
    error_text: Option<String>,
    pending: Vec<Pending>,
    ask_pending: Option<AskPending>,
    retrying: Option<(u32, u32)>,
    last_result_ts: Option<i64>,
    /// The CLI asked the model to carry on by itself (a `turnCompanion` line: "your
    /// previous response had no visible output", or the note after a refusal), so a
    /// thinking-only closing line is the prelude to more, not the end.
    continuation_asked: bool,
    /// The message whose closing line ended the turn. A message's blocks arrive as
    /// separate lines, each with the final `stop_reason`, so its first line can close the
    /// turn before a later one shows it calling a tool — which undoes the close.
    closed_by: Option<String>,
    settled_early: Bounded,
    uuids: Bounded,
    high_water: Option<i64>,
    last_content_ts: Option<i64>,
    /// (task id, kind, when launched), oldest first.
    ledger: VecDeque<(String, BgKind, Option<i64>)>,
    queued_human: usize,
    queued_notes: usize,
    exited: bool,
    exit_ms: Option<i64>,
    title: Option<String>,
    stats: FoldStats,
}

impl Fold {
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self::default()
    }

    /// Fold every complete line in `bytes`; returns how many bytes that consumed, so the
    /// caller can carry a partial last line over to the next read.
    #[allow(dead_code)]
    pub fn feed(&mut self, bytes: &[u8]) -> usize {
        let Some(last_nl) = bytes.iter().rposition(|&b| b == b'\n') else {
            return 0;
        };
        for raw in bytes[..last_nl].split(|&b| b == b'\n') {
            self.feed_line(raw);
        }
        last_nl + 1
    }

    /// Fold one line, as raw bytes without its newline. Blank and unparseable lines are
    /// counted and otherwise ignored: a torn write or a stray byte must not stop the fold.
    pub fn feed_line(&mut self, raw: &[u8]) {
        let raw = raw.strip_suffix(b"\r").unwrap_or(raw);
        if raw.iter().all(u8::is_ascii_whitespace) {
            return;
        }
        match serde_json::from_slice::<Value>(raw) {
            Ok(v) => self.line(&v),
            Err(_) => self.stats.unparsed += 1,
        }
    }

    /// Fold one parsed line.
    pub fn line(&mut self, v: &Value) {
        self.stats.lines += 1;
        let ty = v.get("type").and_then(Value::as_str).unwrap_or("");
        match ty {
            // Written whenever the CLI refines its name for the session; the newest wins.
            "ai-title" => {
                if let Some(t) = v
                    .get("aiTitle")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|t| !t.is_empty())
                {
                    self.title = Some(clip(t, 90));
                }
                return;
            }
            "cost-state" => {
                self.exit(v);
                return;
            }
            "queue-operation" => {
                self.queue(v);
                return;
            }
            "user" | "assistant" => {}
            // Of the system lines only a retry and a compaction are about the conversation;
            // the rest (local command output, notices, turn durations) are bookkeeping.
            "system" => {
                let sub = v.get("subtype").and_then(Value::as_str).unwrap_or("");
                if !matches!(sub, "api_error" | "compact_boundary") {
                    return;
                }
            }
            // Attachments, titles, modes, ledgers, file history, frame links, the rest.
            _ => return,
        }

        // Compaction re-appends a preserved stretch with the same uuids as the originals.
        if let Some(uuid) = v.get("uuid").and_then(Value::as_str) {
            if !self.uuids.insert(uuid, UUIDS_KEPT) {
                self.stats.reappended += 1;
                return;
            }
        }
        let ts = ts_ms(v);
        // ...and when the originals are older than the window, the timestamps still give
        // it away: nothing the session does *now* is stamped minutes behind its last line.
        if ty != "system" {
            if let (Some(t), Some(hw)) = (ts, self.high_water) {
                if t < hw - REAPPEND_SLACK_MS {
                    self.stats.stale += 1;
                    return;
                }
            }
        }
        if let Some(t) = ts {
            self.high_water = Some(self.high_water.map_or(t, |h| h.max(t)));
            self.last_content_ts = Some(self.last_content_ts.map_or(t, |h| h.max(t)));
        }
        // Content after an exit marker: the session was resumed — unless it is stamped
        // within the shutdown itself.
        if self.exited {
            let shutdown =
                matches!((ts, self.exit_ms), (Some(t), Some(exit)) if t <= exit + EXIT_SLACK_MS);
            if !shutdown {
                self.exited = false;
            }
        }

        match ty {
            "user" => self.user(v, ts),
            "assistant" => self.assistant(v, ts),
            _ => self.system(v, ts),
        }
    }

    #[allow(dead_code)]
    pub fn summary(&self) -> FoldSummary {
        let mut bg = Bg::default();
        for (_, kind, _) in &self.ledger {
            match kind {
                BgKind::Agent => bg.agents += 1,
                BgKind::Workflow => bg.workflows += 1,
                BgKind::Shell => bg.shells += 1,
                BgKind::Monitor => bg.monitors += 1,
            }
        }
        let bg_tasks = self
            .ledger
            .iter()
            .map(|(id, kind, ts)| BgTask {
                id: id.clone(),
                kind: kind.as_str(),
                launched_ts: *ts,
            })
            .collect();
        FoldSummary {
            turn_open: self.turn_open,
            turn_start_ts: self.turn_start_ts,
            turn_end_ts: self.turn_end_ts,
            last_end: self.last_end,
            failed: self.failed,
            error_text: self.error_text.clone(),
            pending: self.pending.iter().map(|p| p.tool.clone()).collect(),
            ask_pending: self.ask_pending.clone(),
            retrying: self.retrying,
            last_result_ts: self.last_result_ts,
            bg,
            bg_tasks,
            queued: self.queued_human,
            last_content_ts: self.last_content_ts,
            exited: self.exited,
            exit_ms: self.exit_ms,
            title: self.title.clone(),
        }
    }

    #[allow(dead_code)]
    pub fn stats(&self) -> FoldStats {
        self.stats
    }

    /// The ids of the background tasks still outstanding, for diagnostics.
    #[allow(dead_code)]
    pub fn outstanding(&self) -> Vec<String> {
        self.ledger.iter().map(|(id, _, _)| id.clone()).collect()
    }

    /// A graceful exit: `cost-state` is the last thing the CLI writes on its way out, and
    /// its start plus its duration is the moment it left.
    fn exit(&mut self, v: &Value) {
        self.exited = true;
        self.exit_ms = match (num(v.get("startTime")), num(v.get("totalDuration"))) {
            (Some(start), Some(dur)) => Some(start + dur),
            _ => None,
        };
        // The queue and every background task lived in that process and died with it. A
        // resumed session starts with neither, and nothing will ever report them done.
        self.queued_human = 0;
        self.queued_notes = 0;
        self.ledger.clear();
    }

    /// The CLI's own input queue: typed while a turn ran, taken up when it ends.
    ///
    /// Only messages from a person or a peer are counted. Task notifications travel the
    /// same queue — including every one a *subagent's* background task sends, which
    /// passes through the parent's queue while the parent sits idle — and counting them
    /// shows "1 queued" when nobody typed anything. A `dequeue` names nothing, so it is
    /// credited to a person's message when one is waiting: the CLI takes those first
    /// (1,119 of 1,125 dequeues with both kinds queued, over 28 days of transcripts).
    fn queue(&mut self, v: &Value) {
        let content = v.get("content").and_then(Value::as_str);
        let note = content.is_some_and(is_task_notification);
        if let (Some(c), true) = (content, note) {
            self.notifications(c);
        }
        match v.get("operation").and_then(Value::as_str) {
            Some("enqueue") if note => self.queued_notes += 1,
            Some("enqueue") => self.queued_human += 1,
            Some("dequeue") => {
                if self.queued_human > 0 {
                    self.queued_human -= 1;
                } else {
                    self.queued_notes = self.queued_notes.saturating_sub(1);
                }
            }
            // A contentless remove pairs with a contentless enqueue: a prompt from the
            // editor, absorbed into the running turn.
            Some("remove") if note => self.queued_notes = self.queued_notes.saturating_sub(1),
            Some("remove") => self.queued_human = self.queued_human.saturating_sub(1),
            _ => {}
        }
    }

    fn system(&mut self, v: &Value, ts: Option<i64>) {
        // A compaction boundary is activity (it moved the clock above) and nothing more.
        if v.get("subtype").and_then(Value::as_str) != Some("api_error") {
            return;
        }
        // Retry lines are sometimes appended late; one from before this turn is not news.
        let in_turn = self.turn_open
            && match (ts, self.turn_start_ts) {
                (Some(t), Some(start)) => t >= start,
                _ => true,
            };
        if in_turn {
            let attempt = num(v.get("retryAttempt"))
                .unwrap_or(1)
                .clamp(0, u32::MAX as i64) as u32;
            let max = num(v.get("maxRetries"))
                .unwrap_or(0)
                .clamp(0, u32::MAX as i64) as u32;
            self.retrying = Some((attempt, max));
        }
    }

    fn user(&mut self, v: &Value, ts: Option<i64>) {
        let content = v.get("message").and_then(|m| m.get("content"));
        let mut texts: Vec<&str> = Vec::new();
        let mut has_result = false;
        match content {
            Some(Value::String(s)) => texts.push(s),
            Some(Value::Array(blocks)) => {
                for b in blocks {
                    match b.get("type").and_then(Value::as_str) {
                        Some("tool_result") => {
                            has_result = true;
                            self.tool_result(b, ts);
                        }
                        Some("text") => {
                            if let Some(s) = b.get("text").and_then(Value::as_str) {
                                texts.push(s);
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
        // The turn a task notification starts is also the task's own report of how it
        // ended, so it closes the ledger entry either way.
        for t in &texts {
            if t.contains("<task-notification>") {
                self.notifications(t);
            }
        }

        // An interruption ends the turn outright: the tool it cut short never gets a
        // result, and a fold that waits for one says "working" forever.
        if texts.iter().any(|t| is_marker(t)) {
            self.end_turn(LastEnd::Interrupt, ts);
            return;
        }
        if is_turn_start(v, &texts, has_result) {
            self.start_turn(ts);
            return;
        }
        if flag(v, "turnCompanion") {
            // The CLI carrying the same turn on by itself: after a refusal, or after a
            // response with nothing visible in it. The turn is not over.
            self.continuation_asked = true;
            if !self.turn_open {
                self.turn_open = true;
                self.turn_start_ts = ts;
            }
            return;
        }
        if flag(v, "isMeta") && texts.iter().any(|t| t.trim() == RESUME_MARKER) {
            // A new process resuming a turn whose process died: whatever that process was
            // running in the background, whatever calls it left open, and whatever was
            // queued in it (31044fb2: two peer messages, never delivered) died with it, as
            // at an exit. The marker is sometimes written after the new process's first
            // queue lines, so a message queued in those few seconds can go uncounted until
            // it is taken up — where a dead one stayed counted until some other dequeue
            // happened to absorb it (21 minutes of "1 queued" there).
            self.ledger.clear();
            self.pending.clear();
            self.ask_pending = None;
            self.queued_human = 0;
            self.queued_notes = 0;
        }
    }

    fn assistant(&mut self, v: &Value, ts: Option<i64>) {
        let msg = v.get("message");
        let blocks = msg
            .and_then(|m| m.get("content"))
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default();

        if flag(v, "isApiErrorMessage") {
            let text = blocks
                .iter()
                .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
                .find_map(|b| b.get("text").and_then(Value::as_str))
                .unwrap_or("API error");
            self.failed = true;
            self.error_text = Some(clip(text.trim(), 200));
            self.end_turn(LastEnd::ApiError, ts);
            return;
        }

        // A response arrived, so whatever was being retried went through.
        self.retrying = None;
        let msg_id = msg.and_then(|m| m.get("id")).and_then(Value::as_str);
        let mut visible = false;
        let mut calls = false;
        for b in blocks {
            match b.get("type").and_then(Value::as_str) {
                Some("tool_use") => {
                    visible = true;
                    calls = true;
                    self.tool_use(b, ts, msg_id);
                }
                Some("text") => {
                    if b.get("text")
                        .and_then(Value::as_str)
                        .is_some_and(|s| !s.trim().is_empty())
                    {
                        visible = true;
                    }
                }
                _ => {}
            }
        }
        if visible {
            self.continuation_asked = false;
        }

        match msg
            .and_then(|m| m.get("stop_reason"))
            .and_then(Value::as_str)
        {
            Some("tool_use") => {
                // Mid-turn by definition, even when the prompt that opened the turn lies
                // outside what was read — or the CLI carried on after a thinking-only end.
                if !self.turn_open {
                    self.turn_open = true;
                    self.turn_start_ts = ts;
                }
            }
            Some("end_turn" | "stop_sequence" | "max_tokens" | "refusal") => {
                // A message that calls a tool ends nothing, whatever its stop_reason: the
                // CLI runs the call and asks the model again (0b2e2351 2371, a SendMessage
                // in an `end_turn` message, answered 15 s after its result). An earlier
                // line of the same message — its thinking — may have closed the turn with
                // nothing on it to say otherwise; that close is undone. Decided by the call
                // being on this line, not by it being pending: a fast call's result can be
                // written before the call itself.
                if calls {
                    if !self.turn_open && msg_id.is_some() && self.closed_by.as_deref() == msg_id {
                        self.turn_open = true;
                        self.last_end = None;
                        self.closed_by = None;
                    }
                    return;
                }
                // A tool an earlier line of this very message called is still to run.
                let own_call_pending =
                    self.pending
                        .iter()
                        .any(|p| match (msg_id, p.msg.as_deref()) {
                            (Some(m), Some(pm)) => m == pm,
                            _ => false,
                        });
                if own_call_pending {
                    return;
                }
                if !visible && self.continuation_asked {
                    self.stats.continuation_holds += 1;
                    return;
                }
                self.end_turn(LastEnd::EndTurn, ts);
                self.closed_by = msg_id.map(str::to_owned);
            }
            // null: a partial or aborted message; nothing changes until the next line.
            _ => {}
        }
    }

    fn tool_use(&mut self, b: &Value, ts: Option<i64>, msg_id: Option<&str>) {
        let Some(id) = b.get("id").and_then(Value::as_str) else {
            return;
        };
        // Its result was written first — a fast async launch finishes before the message
        // that asked for it is flushed. It is settled; it was never pending.
        if self.settled_early.remove(id) {
            self.stats.settled_early_hits += 1;
            return;
        }
        if self.pending.iter().any(|p| p.tool.id == id) {
            return;
        }
        let name = b
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("tool")
            .to_owned();
        let summary = tool_summary(&name, b.get("input"));
        let kind = match name.as_str() {
            "AskUserQuestion" => Some(AskKind::Question),
            "ExitPlanMode" => Some(AskKind::Plan),
            _ => None,
        };
        if let Some(kind) = kind {
            self.ask_pending = Some(AskPending {
                id: id.to_owned(),
                kind,
                summary: summary.clone(),
                ts,
            });
        }
        self.pending.push(Pending {
            tool: PendingTool {
                id: id.to_owned(),
                name,
                ts,
                summary,
            },
            msg: msg_id.map(str::to_owned),
        });
        if self.pending.len() > PENDING_KEPT {
            self.pending.remove(0);
        }
    }

    fn tool_result(&mut self, b: &Value, ts: Option<i64>) {
        if let Some(t) = ts {
            self.last_result_ts = Some(self.last_result_ts.map_or(t, |h| h.max(t)));
        }
        let head = result_head(b.get("content"), 2048);
        self.ledger_from_result(&head, ts);
        let Some(id) = b.get("tool_use_id").and_then(Value::as_str) else {
            return;
        };
        if let Some(i) = self.pending.iter().position(|p| p.tool.id == id) {
            self.pending.remove(i);
            if self.ask_pending.as_ref().is_some_and(|a| a.id == id) {
                self.ask_pending = None;
            }
        } else {
            self.settled_early.insert(id, SETTLED_KEPT);
        }
    }

    /// What a tool result says about background work. Launches are recognised only at
    /// the very start of a result: a Read of a file that quotes one is not a launch.
    fn ledger_from_result(&mut self, head: &str, ts: Option<i64>) {
        let t = head.trim_start();
        if let Some(rest) = t.strip_prefix("Async agent launched") {
            // "…successfully. (internal metadata…)\nagentId: <id> (internal ID…)"
            if let Some(i) = rest.find("agentId: ") {
                if let Some(id) = task_id(&rest[i + "agentId: ".len()..]) {
                    self.ledger_add(id, BgKind::Agent, ts);
                }
            }
        } else if let Some(rest) = t.strip_prefix("Workflow launched in background. Task ID: ") {
            if let Some(id) = task_id(rest) {
                self.ledger_add(id, BgKind::Workflow, ts);
            }
        } else if let Some(rest) = t.strip_prefix("Command running in background with ID: ") {
            if let Some(id) = task_id(rest) {
                self.ledger_add(id, BgKind::Shell, ts);
            }
        } else if let Some(rest) = t.strip_prefix("Monitor started (task ") {
            if let Some(id) = task_id(rest) {
                self.ledger_add(id, BgKind::Monitor, ts);
            }
        } else if let Some(rest) = t
            .strip_prefix("{\"message\":\"Successfully stopped task: ")
            .or_else(|| t.strip_prefix("Successfully stopped task: "))
        {
            // TaskStop: the model stopped it itself, and no notification follows.
            if let Some(id) = task_id(rest) {
                self.ledger_remove(id);
            }
        } else if let Some(i) = t.find("No task found with ID: ") {
            // Asked after a task the CLI no longer knows: whatever it was, it is not running.
            if let Some(id) = task_id(&t[i + "No task found with ID: ".len()..]) {
                self.ledger_remove(id);
            }
        } else if t.contains("<retrieval_status>") {
            // TaskOutput handing back a finished task's output.
            if let (Some(id), Some(status)) = (tag(t, "task_id"), tag(t, "status")) {
                if FINAL_STATUSES.contains(&status) {
                    self.ledger_remove(id);
                }
            }
        }
    }

    /// `<task-notification>` blocks: a background task reporting how it ended. One
    /// notification can name several tasks — a new process sweeping up the previous
    /// one's orphaned shells does — and a monitor's end is an expiry event, not a status.
    /// Notifications for tasks this session never launched (a subagent's own) find
    /// nothing to remove.
    fn notifications(&mut self, text: &str) {
        for chunk in text.split("<task-notification>").skip(1) {
            let chunk = chunk.split("</task-notification>").next().unwrap_or(chunk);
            let ended = tag(chunk, "status").is_some_and(|s| FINAL_STATUSES.contains(&s))
                || chunk.contains("[Monitor expired")
                || chunk.contains("[Monitor timed out");
            if !ended {
                continue;
            }
            for id in tags(chunk, "task-id") {
                self.ledger_remove(id);
            }
        }
    }

    fn ledger_add(&mut self, id: &str, kind: BgKind, ts: Option<i64>) {
        if self.ledger.iter().any(|(x, _, _)| x == id) {
            return;
        }
        self.ledger.push_back((id.to_owned(), kind, ts));
        while self.ledger.len() > LEDGER_KEPT {
            self.ledger.pop_front();
        }
    }

    fn ledger_remove(&mut self, id: &str) {
        self.ledger.retain(|(x, _, _)| x != id);
    }

    fn start_turn(&mut self, ts: Option<i64>) {
        self.turn_open = true;
        self.turn_start_ts = ts;
        self.pending.clear();
        self.settled_early.clear();
        self.ask_pending = None;
        self.retrying = None;
        self.last_result_ts = None;
        self.last_end = None;
        self.failed = false;
        // `exited` is not touched here: a turn start is content, and `line` has already
        // decided whether this content ends the exit.
        self.continuation_asked = false;
        self.closed_by = None;
    }

    /// The turn is over. Nothing it called is still running: the CLI waits for every
    /// result before it asks the model again, so a call still listed here is a leftover
    /// (a crash, a result that went missing) and would otherwise read as work forever.
    fn end_turn(&mut self, how: LastEnd, ts: Option<i64>) {
        self.turn_open = false;
        self.last_end = Some(how);
        self.turn_end_ts = ts.or(self.turn_end_ts);
        self.pending.clear();
        self.ask_pending = None;
        self.retrying = None;
        self.continuation_asked = false;
        // Only a closing line may be undone by a later line of its message (set by the
        // caller); an interrupt or an API error is final.
        self.closed_by = None;
    }
}

/// A user line that starts a turn. Current CLIs mark every one — typed, sent by a peer,
/// or a task notification — with `promptSource`, `origin` or `turnOrigin`. Older files
/// are read by what the line is: real typed text, not the CLI's own bookkeeping.
fn is_turn_start(v: &Value, texts: &[&str], has_result: bool) -> bool {
    if v.get("promptSource").is_some() || v.get("origin").is_some() || v.get("turnOrigin").is_some()
    {
        return true;
    }
    if has_result || texts.is_empty() {
        return false;
    }
    if flag(v, "isMeta") || flag(v, "isCompactSummary") || flag(v, "turnCompanion") {
        return false;
    }
    texts.iter().any(|t| !t.trim().is_empty()) && texts.iter().all(|t| is_typed(t))
}

/// Text a person typed, as opposed to what the CLI stores under the user role: slash
/// command echoes and their output, the caveat addressed to the model, interruption
/// markers, and the resume note.
fn is_typed(s: &str) -> bool {
    let t = s.trim();
    if t.is_empty() {
        return true;
    }
    !(t.starts_with("<local-command-caveat>")
        || t.contains("<command-name>")
        || t.contains("<local-command-stdout>")
        || is_marker(t)
        || t == RESUME_MARKER)
}

/// Bracketed system markers Claude Code stores under the user role — the same test the
/// live view uses to render them as interruptions rather than as the user's words.
pub(crate) fn is_marker(s: &str) -> bool {
    let t = s.trim();
    t.starts_with('[')
        && t.ends_with(']')
        && t.len() < 200
        && (t.contains("interrupted")
            || t.contains("Request interrupted")
            || t.contains("No response"))
}

fn is_task_notification(s: &str) -> bool {
    s.trim_start().starts_with("<task-notification>")
}

fn flag(v: &Value, key: &str) -> bool {
    v.get(key).and_then(Value::as_bool) == Some(true)
}

/// A number the CLI wrote, whether it came out as an integer or a float.
fn num(v: Option<&Value>) -> Option<i64> {
    let v = v?;
    v.as_i64()
        .or_else(|| v.as_f64().filter(|f| f.is_finite()).map(|f| f as i64))
}

/// A task id at the start of `s`: the CLI's ids are short runs of letters and digits.
fn task_id(s: &str) -> Option<&str> {
    let end = s
        .char_indices()
        .find(|(_, c)| !(c.is_ascii_alphanumeric() || *c == '_' || *c == '-'))
        .map(|(i, _)| i)
        .unwrap_or(s.len());
    let id = &s[..end];
    (!id.is_empty() && id.len() <= 64).then_some(id)
}

/// The first `<name>…</name>` in `s`, trimmed.
fn tag<'a>(s: &'a str, name: &str) -> Option<&'a str> {
    tags(s, name).into_iter().next()
}

/// Every `<name>…</name>` in `s`, trimmed, in order.
fn tags<'a>(s: &'a str, name: &str) -> Vec<&'a str> {
    let open = format!("<{name}>");
    let close = format!("</{name}>");
    let mut out = Vec::new();
    let mut rest = s;
    while let Some(i) = rest.find(&open) {
        let after = &rest[i + open.len()..];
        let Some(j) = after.find(&close) else { break };
        out.push(after[..j].trim());
        rest = &after[j + close.len()..];
    }
    out
}

/// The start of a tool result's text: all a launch or a stop ever needs, without copying
/// a multi-megabyte result to find it.
fn result_head(content: Option<&Value>, cap: usize) -> String {
    let text = match content {
        Some(Value::String(s)) => s.as_str(),
        Some(Value::Array(blocks)) => blocks
            .iter()
            .find_map(|b| b.get("text").and_then(Value::as_str))
            .unwrap_or(""),
        _ => "",
    };
    let mut end = text.len().min(cap);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

/// What to show for a pending call: the question for a question, the plan's first line
/// for a plan, and otherwise the call's most identifying field.
fn tool_summary(name: &str, input: Option<&Value>) -> String {
    match name {
        "AskUserQuestion" => input
            .and_then(|i| i.get("questions"))
            .and_then(Value::as_array)
            .and_then(|qs| qs.first())
            .and_then(|q| q.get("question"))
            .and_then(Value::as_str)
            .map(|q| clip(q.trim(), 300))
            .unwrap_or_default(),
        "ExitPlanMode" => input
            .and_then(|i| i.get("plan"))
            .and_then(Value::as_str)
            .and_then(|p| p.lines().map(str::trim).find(|l| !l.is_empty()))
            .map(|l| clip(l.trim_start_matches('#').trim(), 300))
            .unwrap_or_default(),
        _ => summarise_input(input),
    }
}

/// The most identifying field of a tool call, so a row reads like `Bash: git status`
/// rather than a wall of JSON.
pub fn summarise_input(input: Option<&Value>) -> String {
    let Some(v) = input else {
        return String::new();
    };
    for key in [
        "command",
        "file_path",
        "path",
        "pattern",
        "query",
        "url",
        "prompt",
        "description",
        "skill",
        "message",
    ] {
        if let Some(s) = v.get(key).and_then(|x| x.as_str()) {
            if !s.trim().is_empty() {
                return clip(s.trim(), 300);
            }
        }
    }
    clip(&v.to_string(), 300)
}

/// At most `cap` characters, with an ellipsis when cut. Characters, not bytes: a byte
/// slice would panic on a multi-byte boundary.
pub(crate) fn clip(s: &str, cap: usize) -> String {
    if s.chars().count() <= cap {
        return s.to_owned();
    }
    let kept: String = s.chars().take(cap).collect();
    format!("{kept}…")
}

/// A line's `timestamp` in epoch milliseconds. The transcript writes UTC RFC 3339
/// (`2026-09-29T09:41:53.044Z`); an explicit offset is honoured in case that ever changes.
pub fn ts_ms(v: &Value) -> Option<i64> {
    parse_rfc3339_ms(v.get("timestamp")?.as_str()?)
}

pub(crate) fn parse_rfc3339_ms(s: &str) -> Option<i64> {
    let (date, rest) = s.split_once('T')?;
    let mut d = date.splitn(3, '-');
    let y: i64 = d.next()?.parse().ok()?;
    let mo: i64 = d.next()?.parse().ok()?;
    let day: i64 = d.next()?.parse().ok()?;
    // Split the clock from its zone: `Z`, `+hh:mm`, `-hh:mm`, or nothing (taken as UTC).
    let (clock, offset_min) =
        if let Some(c) = rest.strip_suffix('Z').or_else(|| rest.strip_suffix('z')) {
            (c, 0)
        } else if let Some(i) = rest.rfind(['+', '-']).filter(|&i| i >= 5) {
            let (c, z) = rest.split_at(i);
            let sign = if z.starts_with('-') { -1 } else { 1 };
            let (h, m) = z[1..].split_once(':').unwrap_or((&z[1..], "0"));
            (
                c,
                sign * (h.parse::<i64>().ok()? * 60 + m.parse::<i64>().ok()?),
            )
        } else {
            (rest, 0)
        };
    let mut t = clock.splitn(3, ':');
    let h: i64 = t.next()?.parse().ok()?;
    let mi: i64 = t.next()?.parse().ok()?;
    let sec = t.next()?;
    let (whole, frac) = sec.split_once('.').unwrap_or((sec, ""));
    let whole: i64 = whole.parse().ok()?;
    let digits: String = frac.chars().take(3).collect();
    let ms: i64 = if digits.is_empty() {
        0
    } else {
        format!("{digits:0<3}").parse().ok()?
    };
    if !(1..=12).contains(&mo) || !(1..=31).contains(&day) || h > 23 || mi > 59 || whole > 60 {
        return None;
    }
    let days = crate::registry::days_from_civil(y, mo, day);
    Some(((days * 86_400 + h * 3600 + mi * 60 + whole) - offset_min * 60) * 1000 + ms)
}

/// Where to start reading a transcript seen for the first time: its last
/// `INITIAL_WINDOW` bytes.
#[allow(dead_code)]
pub fn initial_offset(len: u64) -> u64 {
    len.saturating_sub(INITIAL_WINDOW)
}

/// Where the first whole line of a window begins. A window that starts mid-file almost
/// always starts mid-line, and half a line would only fail to parse; dropping the first
/// line whole costs at most one line of history.
#[allow(dead_code)]
pub fn skip_partial_line(chunk: &[u8], from_mid_file: bool) -> usize {
    if !from_mid_file {
        return 0;
    }
    chunk
        .iter()
        .position(|&b| b == b'\n')
        .map_or(chunk.len(), |i| i + 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fold(lines: &[Value]) -> Fold {
        let mut f = Fold::new();
        for l in lines {
            f.line(l);
        }
        f
    }

    const T0: &str = "2026-09-29T09:35:15.572Z";

    fn at(secs: i64) -> String {
        // T0 plus `secs`, formatted the way the transcript writes it.
        let base = parse_rfc3339_ms(T0).unwrap() + secs * 1000;
        let (days, rem) = (base.div_euclid(86_400_000), base.rem_euclid(86_400_000));
        let (y, m, d) = crate::registry::civil_from_days(days);
        format!(
            "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{:03}Z",
            rem / 3_600_000,
            rem / 60_000 % 60,
            rem / 1000 % 60,
            rem % 1000
        )
    }

    fn ms(secs: i64) -> i64 {
        parse_rfc3339_ms(T0).unwrap() + secs * 1000
    }

    /// A prompt as current CLIs write it.
    fn prompt(secs: i64, uuid: &str, text: &str) -> Value {
        json!({"type":"user","uuid":uuid,"timestamp":at(secs),"promptSource":"sdk","origin":{"kind":"human"},
               "message":{"role":"user","content":text}})
    }

    fn assistant(secs: i64, uuid: &str, msg: &str, stop: Option<&str>, blocks: Value) -> Value {
        json!({"type":"assistant","uuid":uuid,"timestamp":at(secs),
               "message":{"id":msg,"role":"assistant","stop_reason":stop,"content":blocks}})
    }

    fn call(id: &str, name: &str, input: Value) -> Value {
        json!([{"type":"tool_use","id":id,"name":name,"input":input}])
    }

    fn result(secs: i64, uuid: &str, id: &str, text: &str) -> Value {
        json!({"type":"user","uuid":uuid,"timestamp":at(secs),
               "message":{"role":"user","content":[{"type":"tool_result","tool_use_id":id,"content":text}]}})
    }

    fn said(text: &str) -> Value {
        json!([{"type":"text","text":text}])
    }

    fn queue(secs: i64, op: &str, content: Option<&str>) -> Value {
        let mut v = json!({"type":"queue-operation","operation":op,"timestamp":at(secs)});
        if let Some(c) = content {
            v["content"] = json!(c);
        }
        v
    }

    #[test]
    fn timestamps_parse_to_epoch_milliseconds() {
        assert_eq!(parse_rfc3339_ms("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(
            parse_rfc3339_ms("2026-09-10T07:35:03Z"),
            Some(1_789_025_703_000)
        );
        assert_eq!(
            parse_rfc3339_ms("2026-09-10T07:35:03.5Z"),
            Some(1_789_025_703_500)
        );
        assert_eq!(
            parse_rfc3339_ms("2026-09-10T09:35:03.044+02:00"),
            Some(1_789_025_703_044)
        );
        assert_eq!(parse_rfc3339_ms("not a time"), None);
        assert_eq!(parse_rfc3339_ms("2026-13-10T07:35:03Z"), None);
    }

    #[test]
    fn a_prompt_opens_a_turn_and_a_text_end_closes_it() {
        let f = fold(&[
            queue(0, "enqueue", None),
            queue(0, "dequeue", None),
            prompt(0, "u1", "make it work"),
        ]);
        let s = f.summary();
        assert!(
            s.turn_open,
            "the model is thinking: silence is not idleness"
        );
        assert_eq!(s.turn_start_ts, Some(ms(0)));
        assert_eq!(s.queued, 0);

        let mut f = f;
        f.line(&assistant(
            110,
            "a1",
            "m1",
            Some("end_turn"),
            json!([{"type":"thinking","thinking":"…"}]),
        ));
        f.line(&assistant(110, "a2", "m1", Some("end_turn"), said("Done.")));
        let s = f.summary();
        assert!(!s.turn_open);
        assert_eq!(s.last_end, Some(LastEnd::EndTurn));
        assert_eq!(s.turn_end_ts, Some(ms(110)));
        assert_eq!(s.last_content_ts, Some(ms(110)));
    }

    #[test]
    fn a_call_is_pending_until_its_result_and_the_summary_names_it() {
        let mut f = fold(&[
            prompt(0, "u1", "status?"),
            assistant(
                3,
                "a1",
                "m1",
                Some("tool_use"),
                call("t1", "Bash", json!({"command":"git status"})),
            ),
        ]);
        let s = f.summary();
        assert_eq!(s.pending.len(), 1);
        assert_eq!(
            (s.pending[0].name.as_str(), s.pending[0].summary.as_str()),
            ("Bash", "git status")
        );
        assert_eq!(s.pending[0].ts, Some(ms(3)));
        f.line(&result(4, "u2", "t1", "On branch dev"));
        let s = f.summary();
        assert!(s.pending.is_empty());
        assert!(s.turn_open, "a result hands the turn back to the model");
        assert_eq!(s.last_result_ts, Some(ms(4)));
    }

    #[test]
    fn a_result_written_before_its_call_settles_it() {
        // f0ae6c64 lines 162/168: the async Agent launch's result (16:19:16.994) is written
        // before its own tool_use line (16:19:16.957), between other calls of the turn.
        let mut f = fold(&[
            prompt(0, "u0", "look into it"),
            assistant(10, "a1", "m1", Some("tool_use"), call("toolu_kGCH754N", "Bash", json!({"command":"sqlite3 …"}))),
            result(43, "u1", "toolu_fQQHkYu3",
                "Async agent launched successfully. (This tool result is internal metadata — never quote it.)\nagentId: ab8f512a3ecfdd1cc (internal ID - do not mention to user.)\nThe agent is working in the background."),
            result(11, "u2", "toolu_kGCH754N", "=== ordinary turns ==="),
            assistant(42, "a2", "m2", Some("tool_use"), call("toolu_fQQHkYu3", "Agent", json!({"description":"audit","run_in_background":true}))),
        ]);
        let s = f.summary();
        assert!(s.pending.is_empty(), "no phantom 'Running Agent'");
        assert_eq!(f.stats().settled_early_hits, 1);
        assert_eq!(s.bg.agents, 1, "and the launch is on the ledger");
        f.line(&assistant(
            50,
            "a3",
            "m3",
            Some("end_turn"),
            said("It runs in the background."),
        ));
        assert!(!f.summary().turn_open);
    }

    #[test]
    fn a_compaction_reappend_is_not_folded_again() {
        // 19a7bd89 2788–2927: the preserved segment comes back with its original uuids and
        // timestamps, ending in an old end_turn, while the real turn is mid-compaction.
        let old = [
            prompt(0, "u-old", "earlier request"),
            assistant(
                5,
                "a-old",
                "m-old",
                Some("end_turn"),
                said("Earlier answer."),
            ),
        ];
        let mut f = fold(&old);
        f.line(&prompt(3600, "u-now", "the current request"));
        f.line(&assistant(
            3605,
            "a-now",
            "m-now",
            Some("tool_use"),
            call("t9", "Bash", json!({"command":"make"})),
        ));
        f.line(&result(3606, "u-res", "t9", "ok"));
        // The re-append: same uuids, same (old) timestamps.
        for l in &old {
            f.line(l);
        }
        f.line(&json!({"type":"system","subtype":"compact_boundary","uuid":"b1","timestamp":at(3790),"content":"Conversation compacted"}));
        f.line(&json!({"type":"user","uuid":"u-sum","timestamp":at(3790),"isCompactSummary":true,
                       "message":{"role":"user","content":"This session is being continued from a previous conversation…"}}));
        let s = f.summary();
        assert!(
            s.turn_open,
            "the old end_turn inside the re-append must not end today's turn"
        );
        assert_eq!(s.turn_start_ts, Some(ms(3600)));
        assert_eq!(
            s.last_content_ts,
            Some(ms(3790)),
            "the boundary is activity"
        );
        assert_eq!(f.stats().reappended, 2);

        // The same stretch re-appended when its originals were never folded (outside the
        // window): the timestamps give it away.
        let mut g = fold(&[
            prompt(3600, "u-now", "the current request"),
            assistant(
                3605,
                "a-now",
                "m-now",
                Some("tool_use"),
                call("t9", "Bash", json!({"command":"make"})),
            ),
            result(3606, "u-res", "t9", "ok"),
        ]);
        for l in &old {
            g.line(l);
        }
        assert!(g.summary().turn_open);
        assert_eq!(g.stats().stale, 2);
    }

    #[test]
    fn a_failure_holds_until_the_next_prompt_and_not_after() {
        // ce90e636 2831/2834: the limit error, then "Please resume" 42 minutes later.
        let mut f = fold(&[
            prompt(0, "u1", "go"),
            assistant(
                2,
                "a1",
                "m1",
                Some("tool_use"),
                call(
                    "q1",
                    "AskUserQuestion",
                    json!({"questions":[{"question":"Which one?"}]}),
                ),
            ),
            result(10, "u2", "q1", "The user answered: \"Which one?\"=\"B\""),
            json!({"type":"assistant","uuid":"a2","timestamp":at(11),"isApiErrorMessage":true,
                   "message":{"id":"m2","model":"<synthetic>","stop_reason":"stop_sequence",
                              "content":[{"type":"text","text":"You've hit your weekly limit · resets 1pm (America/Toronto)"}]}}),
        ]);
        let s = f.summary();
        assert!(!s.turn_open && s.failed);
        assert_eq!(s.last_end, Some(LastEnd::ApiError));
        assert_eq!(
            s.error_text.as_deref(),
            Some("You've hit your weekly limit · resets 1pm (America/Toronto)")
        );
        f.line(&queue(2530, "enqueue", None));
        f.line(&queue(2530, "dequeue", None));
        f.line(&prompt(2530, "u3", "Please resume"));
        let s = f.summary();
        assert!(s.turn_open && !s.failed, "the retry is work, not a failure");
        assert_eq!(s.last_end, None);
    }

    #[test]
    fn a_retry_is_reported_for_the_turn_it_belongs_to() {
        let mut f = fold(&[prompt(0, "u1", "go")]);
        f.line(
            &json!({"type":"system","subtype":"api_error","uuid":"s1","timestamp":at(3),
                       "retryAttempt":3,"maxRetries":10,"retryInMs":2400}),
        );
        assert_eq!(f.summary().retrying, Some((3, 10)));
        f.line(&assistant(
            9,
            "a1",
            "m1",
            Some("tool_use"),
            call("t1", "Read", json!({"file_path":"/x"})),
        ));
        assert_eq!(
            f.summary().retrying,
            None,
            "a response means the retry went through"
        );
    }

    #[test]
    fn a_thinking_only_end_closes_the_turn_unless_the_cli_asked_for_more() {
        // a2ae08ce 2023: nothing between the thinking-only end_turn and the continuation.
        let mut f = fold(&[prompt(0, "u1", "go"), result(5, "u2", "t0", "x")]);
        f.line(&assistant(
            15,
            "a1",
            "m1",
            Some("end_turn"),
            json!([{"type":"thinking","thinking":"…"}]),
        ));
        assert!(!f.summary().turn_open, "a text-less end_turn ends the turn");
        // ...and the automatic continuation re-opens it.
        f.line(&assistant(
            200,
            "a2",
            "m2",
            Some("tool_use"),
            call("t1", "Bash", json!({"command":"ls"})),
        ));
        let s = f.summary();
        assert!(s.turn_open);
        assert_eq!(s.turn_start_ts, Some(ms(200)));

        // 8c69ff30 6610–6612: the CLI's "no visible output, continue" lands first, then the
        // thinking-only message it refers to, then the real answer.
        let mut g = fold(&[prompt(0, "u1", "go")]);
        g.line(&json!({"type":"user","uuid":"c1","timestamp":at(20),"isMeta":true,"turnCompanion":true,
                       "message":{"role":"user","content":"[Your previous response had no visible output. Please continue and produce a user-visible response.]"}}));
        g.line(&assistant(
            20,
            "a1",
            "m1",
            Some("end_turn"),
            json!([{"type":"thinking","thinking":"…"}]),
        ));
        assert!(g.summary().turn_open, "the continuation is under way");
        assert_eq!(g.stats().continuation_holds, 1);
        g.line(&assistant(
            33,
            "a2",
            "m2",
            Some("end_turn"),
            said("Here it is."),
        ));
        assert!(!g.summary().turn_open);

        // 5ce27314 86–90: a refusal ends the message, then the CLI continues once.
        let mut h = fold(&[prompt(0, "u1", "go")]);
        h.line(&assistant(
            34,
            "a1",
            "m1",
            Some("refusal"),
            json!([{"type":"thinking","thinking":"…"}]),
        ));
        assert!(!h.summary().turn_open);
        h.line(&json!({"type":"user","uuid":"c1","timestamp":at(34),"isMeta":true,"turnCompanion":true,
                       "message":{"role":"user","content":"Your response above was stopped by a safety classifier…"}}));
        assert!(h.summary().turn_open, "continuing once is the same turn");
        h.line(&assistant(
            83,
            "a2",
            "m2",
            Some("end_turn"),
            json!([{"type":"thinking","thinking":"…"}]),
        ));
        h.line(&assistant(
            83,
            "a3",
            "m2",
            Some("end_turn"),
            said("I won't help with this one."),
        ));
        assert!(!h.summary().turn_open);
    }

    /// 0b2e2351 2365–2378, as written. Message `U7AGi4cT` closes with `end_turn` on both of
    /// its lines: the thinking first, then a SendMessage call. The CLI runs the call anyway
    /// and asks the model again, which answers 15 s after the result.
    fn the_two_line_closing_message() -> [Value; 10] {
        [
            json!({"type":"user","uuid":"u0","timestamp":"2026-09-03T07:14:02.000Z","promptSource":"sdk","origin":{"kind":"human"},
                   "message":{"role":"user","content":"sweep the workspace and report"}}),
            json!({"type":"assistant","uuid":"a0","timestamp":"2026-09-03T07:15:43.930Z",
                   "message":{"id":"msg_011Ceg6Qd8FNRSb","role":"assistant","stop_reason":"tool_use",
                              "content":[{"type":"tool_use","id":"toolu_01mJZv1fjN","name":"Bash","input":{"command":"cargo test --workspace"}}]}}),
            json!({"type":"user","uuid":"u1","timestamp":"2026-09-03T07:15:44.122Z",
                   "message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_01mJZv1fjN","content":"=== does the device REPORT a version over capabilities? ==="}]}}),
            // 2370: the first line of the closing message, thinking only.
            json!({"type":"assistant","uuid":"a1","timestamp":"2026-09-03T07:15:57.076Z",
                   "message":{"id":"msg_011Ceg6TmYWPa78EU7AGi4cT","role":"assistant","stop_reason":"end_turn",
                              "content":[{"type":"thinking","thinking":"","signature":"CAISxBEK"}]}}),
            // 2371: the same message's second line, the call.
            json!({"type":"assistant","uuid":"a2","timestamp":"2026-09-03T07:16:23.638Z",
                   "message":{"id":"msg_011Ceg6TmYWPa78EU7AGi4cT","role":"assistant","stop_reason":"end_turn",
                              "content":[{"type":"tool_use","id":"toolu_01XJGjGkU5FyLyBDL2KJT8wG","name":"SendMessage",
                                          "input":{"to":"nami-56","message":"**Sweep done.**"}}]}}),
            json!({"type":"last-prompt","lastPrompt":"sweep the workspace and report","sessionId":"s"}),
            json!({"type":"ai-title","aiTitle":"Workspace sweep","sessionId":"s"}),
            // 2376: the call's result.
            json!({"type":"user","uuid":"u2","timestamp":"2026-09-03T07:16:23.973Z",
                   "message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_01XJGjGkU5FyLyBDL2KJT8wG","content":"{\"success\":true}"}]}}),
            json!({"type":"attachment","timestamp":"2026-09-03T07:16:23.984Z","attachment":{"type":"queued_command"}}),
            // 2378: the model's answer to it, a new message.
            json!({"type":"assistant","uuid":"a3","timestamp":"2026-09-03T07:16:38.885Z",
                   "message":{"id":"msg_011Ceg6WgUzAnQuVquNHjVic","role":"assistant","stop_reason":"end_turn",
                              "content":[{"type":"text","text":"Sweep delivered. Workspace **352 passed / 0 failed**."}]}}),
        ]
    }

    #[test]
    fn a_tool_this_message_called_keeps_the_turn_open_through_its_end_turn() {
        let lines = the_two_line_closing_message();
        let mut f = fold(&lines[..4]);
        // Nothing on the thinking line says a call follows: as far as it goes, it is the end.
        assert!(!f.summary().turn_open);
        f.line(&lines[4]);
        let s = f.summary();
        assert!(
            s.turn_open,
            "a call on a later line of the closing message: not over after all"
        );
        assert_eq!(s.last_end, None);
        assert_eq!(
            s.turn_start_ts,
            parse_rfc3339_ms("2026-09-03T07:14:02.000Z"),
            "the same turn"
        );
        assert_eq!(
            s.pending
                .iter()
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>(),
            ["SendMessage"]
        );
        for l in &lines[5..9] {
            f.line(l);
        }
        let s = f.summary();
        assert!(s.turn_open, "the model answers the result next");
        assert!(s.pending.is_empty());
        f.line(&lines[9]);
        let s = f.summary();
        assert!(!s.turn_open, "and then it is over");
        assert_eq!(s.last_end, Some(LastEnd::EndTurn));
        assert_eq!(s.turn_end_ts, parse_rfc3339_ms("2026-09-03T07:16:38.885Z"));
    }

    #[test]
    fn a_call_whose_result_came_first_still_reopens_its_closing_message() {
        // The same message, its call's result written before the call's own line (as a
        // fast async launch's is): the call was never pending, and the turn goes on anyway.
        let lines = the_two_line_closing_message();
        let mut f = fold(&lines[..4]);
        f.line(&lines[7]);
        assert!(!f.summary().turn_open);
        f.line(&lines[4]);
        let s = f.summary();
        assert!(
            s.turn_open,
            "decided by the call being on the line, not by it being pending"
        );
        assert!(s.pending.is_empty());
        assert_eq!(f.stats().settled_early_hits, 1);
        f.line(&lines[9]);
        assert!(!f.summary().turn_open);
        // A call in a message that is still open settles nothing on its own either: a
        // one-line closing message whose call already has its result is not the end.
        let mut g = fold(&[
            prompt(0, "u1", "go"),
            result(
                5,
                "u2",
                "t1",
                "Async agent launched successfully.\nagentId: a1b2c3 (internal)",
            ),
            assistant(
                4,
                "a1",
                "m1",
                Some("end_turn"),
                call("t1", "Agent", json!({"description":"audit"})),
            ),
        ]);
        assert!(g.summary().turn_open);
        g.line(&assistant(
            9,
            "a2",
            "m2",
            Some("end_turn"),
            said("Launched."),
        ));
        assert!(!g.summary().turn_open);
    }

    #[test]
    fn a_bookkeeping_only_tail_changes_nothing() {
        let mut f = fold(&[
            prompt(0, "u1", "go"),
            assistant(5, "a1", "m1", Some("end_turn"), said("Done.")),
        ]);
        let before = f.summary();
        for l in [
            json!({"type":"last-prompt","lastPrompt":"go","sessionId":"s"}),
            json!({"type":"atis-latch","atis":1,"sessionId":"s"}),
            json!({"type":"mode","mode":"default","sessionId":"s"}),
            json!({"type":"artifact-autoreact-ledger","sessionId":"s"}),
            json!({"type":"file-history-snapshot","messageId":"x","snapshot":{}}),
            json!({"type":"frame-link","timestamp":at(9000)}),
            json!({"type":"attachment","timestamp":at(9000),"attachment":{"type":"queued_command","prompt":"<task-notification>…"}}),
            json!({"type":"system","subtype":"local_command","uuid":"s9","timestamp":at(9000),"content":"<local-command-stdout>Set effort</local-command-stdout>"}),
        ] {
            f.line(&l);
        }
        let mut after = f.summary();
        assert_eq!(
            after.last_content_ts,
            Some(ms(5)),
            "bookkeeping is never activity"
        );
        after.title = before.title.clone();
        assert_eq!(after, before);

        f.line(&json!({"type":"ai-title","aiTitle":"  Fix the live view  ","sessionId":"s"}));
        f.line(&json!({"type":"ai-title","aiTitle":"Session liveness","sessionId":"s"}));
        assert_eq!(
            f.summary().title.as_deref(),
            Some("Session liveness"),
            "the newest title wins"
        );
    }

    #[test]
    fn a_plan_or_a_question_is_the_session_asking() {
        // d3f90113 857–859: ExitPlanMode waited 11.5 min for approval, then was rejected.
        let mut f = fold(&[
            prompt(0, "u1", "plan it"),
            assistant(
                30,
                "a1",
                "m1",
                Some("tool_use"),
                call(
                    "p1",
                    "ExitPlanMode",
                    json!({"plan":"# Rework the fold\n\n1. Registry\n2. Fold"}),
                ),
            ),
        ]);
        let s = f.summary();
        let ask = s.ask_pending.expect("a plan is waiting");
        assert_eq!(
            (ask.kind, ask.id.as_str(), ask.summary.as_str()),
            (AskKind::Plan, "p1", "Rework the fold")
        );
        f.line(&result(720, "u2", "p1", "User rejected the plan"));
        assert!(f.summary().ask_pending.is_none());

        let f = fold(&[
            prompt(0, "u1", "ask me"),
            assistant(
                3,
                "a1",
                "m1",
                Some("tool_use"),
                call(
                    "q1",
                    "AskUserQuestion",
                    json!({"questions":[{"question":"Which delivery?","options":[]}]}),
                ),
            ),
        ]);
        let ask = f.summary().ask_pending.unwrap();
        assert_eq!(
            (ask.kind, ask.summary.as_str()),
            (AskKind::Question, "Which delivery?")
        );
    }

    #[test]
    fn background_work_is_tracked_from_launch_to_notification() {
        let mut f = fold(&[
            prompt(0, "u1", "start everything"),
            assistant(1, "a1", "m1", Some("tool_use"), call("t1", "Bash", json!({"command":"sleep 99","run_in_background":true}))),
            result(1, "u2", "t1", "Command running in background with ID: by69sv3il. Output is being written to: /tmp/x/tasks/by69sv3il.output. You will be notified when it completes."),
            assistant(2, "a2", "m2", Some("tool_use"), call("t2", "Workflow", json!({"description":"diagnose"}))),
            result(2, "u3", "t2", "Workflow launched in background. Task ID: w83ctrk6l\nSummary: Diagnose\nTranscript dir: /x"),
            assistant(3, "a3", "m3", Some("tool_use"), call("t3", "Agent", json!({"description":"audit"}))),
            result(3, "u4", "t3", "Async agent launched successfully. (This tool result is internal metadata.)\nagentId: ab8f512a3ecfdd1cc (internal ID - do not mention to user.)"),
            assistant(4, "a4", "m4", Some("tool_use"), call("t4", "Monitor", json!({"command":"tail -f x"}))),
            result(4, "u5", "t4", "Monitor started (task bud8kaugk, expires in 3m unless the source ends first). You will be notified on each event."),
            assistant(5, "a5", "m5", Some("end_turn"), said("All four are running.")),
        ]);
        let s = f.summary();
        assert!(!s.turn_open);
        assert_eq!(
            s.bg,
            Bg {
                agents: 1,
                workflows: 1,
                shells: 1,
                monitors: 1
            }
        );

        // The shell finishes: the notification is enqueued, then starts a turn of its own.
        let note = "<task-notification>\n<task-id>by69sv3il</task-id>\n<tool-use-id>toolu_x</tool-use-id>\n<status>completed</status>\n<summary>Background command completed (exit code 0)</summary>\n</task-notification>";
        f.line(&queue(60, "enqueue", Some(note)));
        assert_eq!(f.summary().bg.shells, 0);
        f.line(&queue(60, "dequeue", None));
        f.line(&json!({"type":"user","uuid":"u6","timestamp":at(60),"promptSource":"system","origin":{"kind":"task-notification"},
                       "message":{"role":"user","content":note}}));
        // A monitor event is not its end; its expiry is.
        f.line(&queue(70, "enqueue", Some("<task-notification>\n<task-id>bud8kaugk</task-id>\n<summary>Monitor event: \"x\"</summary>\n<event>line 1</event>\n</task-notification>")));
        assert_eq!(f.summary().bg.monitors, 1);
        f.line(&queue(250, "enqueue", Some("<task-notification>\n<task-id>bud8kaugk</task-id>\n<summary>Monitor event: \"x\"</summary>\n<event>[Monitor expired after 3m with no events delivered. Re-arm it if you still need the watch.]</event>\n</task-notification>")));
        assert_eq!(f.summary().bg.monitors, 0);
        // The agent and the workflow report in a user line of their own turn.
        f.line(&json!({"type":"user","uuid":"u7","timestamp":at(300),"promptSource":"system","origin":{"kind":"task-notification"},
                       "message":{"role":"user","content":[{"type":"text","text":"<task-notification>\n<task-id>ab8f512a3ecfdd1cc</task-id>\n<status>completed</status>\n</task-notification>"}]}}));
        f.line(&queue(400, "enqueue", Some("<task-notification>\n<task-id>w83ctrk6l</task-id>\n<status>failed</status>\n</task-notification>")));
        assert_eq!(f.summary().bg, Bg::default());
    }

    #[test]
    fn a_stopped_or_vanished_task_leaves_the_ledger() {
        let mut f = fold(&[
            prompt(0, "u1", "go"),
            result(1, "u2", "t1", "Command running in background with ID: bbac9anu4. Output is being written to: /tmp/o"),
            result(2, "u3", "t2", "Workflow launched in background. Task ID: wky6aa8c9\nSummary: x"),
            result(3, "u4", "t3", "Command running in background with ID: bed0e4xb5. Output is being written to: /tmp/o"),
            result(4, "u5", "t4", "Workflow launched in background. Task ID: wj5cqcckz\nSummary: y"),
        ]);
        assert_eq!(
            f.summary().bg,
            Bg {
                agents: 0,
                workflows: 2,
                shells: 2,
                monitors: 0
            }
        );
        // TaskStop answers with JSON and no notification ever follows (9fda36f0 line 389).
        f.line(&result(5, "u6", "t5", "{\"message\":\"Successfully stopped task: bbac9anu4 (SC=/tmp/x && ./run)\",\"task_id\":\"bbac9anu4\"}"));
        // TaskOutput on a task the CLI no longer knows (16a89637 line 555).
        f.line(&result(
            6,
            "u7",
            "t6",
            "<tool_use_error>No task found with ID: wky6aa8c9</tool_use_error>",
        ));
        // TaskOutput handing back a finished task.
        f.line(&result(7, "u8", "t7", "<retrieval_status>success</retrieval_status>\n<task_id>wj5cqcckz</task_id>\n<task_type>local_workflow</task_type>\n<status>completed</status>"));
        // A new process sweeping up the previous one's shells, several ids at once (62da2cfe).
        f.line(&queue(8, "enqueue", Some("<task-notification>\n<task-id>bvd5mtxva</task-id>\n<task-id>bed0e4xb5</task-id>\n<task-id>__orphan_summary__:shell</task-id>\n<status>stopped</status>\n<summary>2 background shell command task(s) from the previous session have no completion record.</summary>\n</task-notification>")));
        assert_eq!(f.summary().bg, Bg::default());
        // A result that merely quotes a launch is not a launch.
        f.line(&result(
            9,
            "u9",
            "t8",
            "1\t\n2\tCommand running in background with ID: bquoted01",
        ));
        assert_eq!(f.summary().bg, Bg::default());
    }

    #[test]
    fn subagent_notifications_through_the_parents_queue_are_not_queued_messages() {
        // d6e4d7da lines 147–152: the parent's turn is over; its subagents' own background
        // tasks report through the parent's queue and are absorbed by the subagents.
        let mut f = fold(&[
            prompt(0, "u1", "go"),
            assistant(5, "a1", "m1", Some("end_turn"), said("Launched.")),
        ]);
        let note = "<task-notification>\n<task-id>bcjvvd1mj</task-id>\n<status>completed</status>\n</task-notification>";
        f.line(&queue(60, "enqueue", Some(note)));
        assert_eq!(f.summary().queued, 0, "nobody typed anything");
        f.line(&queue(70, "remove", Some(note)));
        assert_eq!(f.summary().queued, 0);
        assert_eq!(
            f.summary().last_content_ts,
            Some(ms(5)),
            "queue lines are not activity"
        );

        // A person typing while a turn runs is queued until the turn takes it up.
        f.line(&prompt(100, "u2", "next thing"));
        f.line(&queue(
            101,
            "enqueue",
            Some("<task-notification>\n<task-id>b1</task-id>\n</task-notification>"),
        ));
        f.line(&queue(102, "enqueue", None));
        f.line(&queue(103, "enqueue", Some("and also this")));
        assert_eq!(f.summary().queued, 2);
        // A dequeue takes a person's message first.
        f.line(&queue(104, "dequeue", None));
        assert_eq!(f.summary().queued, 1);
        f.line(&queue(105, "remove", Some("and also this")));
        assert_eq!(f.summary().queued, 0);
        f.line(&queue(106, "remove", None));
        f.line(&queue(107, "dequeue", None));
        assert_eq!(f.summary().queued, 0, "never below zero");
    }

    #[test]
    fn an_exit_marker_holds_until_the_session_is_resumed() {
        let mut f = fold(&[
            prompt(0, "u1", "go"),
            queue(1, "enqueue", Some("typed while busy")),
            result(2, "u2", "t1", "Command running in background with ID: bxff3dvzv. Output is being written to: /tmp/o"),
            assistant(5, "a1", "m1", Some("end_turn"), said("Done.")),
            json!({"type":"last-prompt","lastPrompt":"go","sessionId":"s"}),
            json!({"type":"cost-state","sessionId":"s","startTime":ms(-100),"totalDuration":120_000,"totalCostUSD":0.5}),
        ]);
        let s = f.summary();
        assert!(s.exited);
        assert_eq!(s.exit_ms, Some(ms(-100) + 120_000));
        assert_eq!(
            s.last_content_ts,
            Some(ms(5)),
            "the exit flush is not activity"
        );
        assert_eq!(
            (s.queued, s.bg.shells),
            (0, 0),
            "the queue and the tasks died with the process"
        );
        // Resumed: the next prompt is content, and the session is not exited any more.
        f.line(&queue(900, "enqueue", None));
        f.line(&queue(900, "dequeue", None));
        f.line(&prompt(900, "u3", "back again"));
        let s = f.summary();
        assert!(!s.exited && s.turn_open);
    }

    #[test]
    fn what_the_shutdown_itself_writes_is_not_a_resume() {
        // c0b3be1f 311–317: the exit marker says 00:31:02.948; a background task ending as
        // the process goes down still gets its notification turn written, 18 ms later.
        let exit = ms(300);
        let mut f = fold(&[
            prompt(0, "u1", "go"),
            assistant(5, "a1", "m1", Some("end_turn"), said("Done.")),
            json!({"type":"cost-state","sessionId":"s","startTime":ms(-600),"totalDuration":exit - ms(-600)}),
            queue(300, "enqueue", Some("<task-notification>\n<task-id>btz67rmk2</task-id>\n<status>killed</status>\n</task-notification>")),
            queue(300, "dequeue", None),
        ]);
        let mut late = prompt(300, "u2", "<task-notification>…</task-notification>");
        late["timestamp"] = json!("2026-09-29T09:40:15.590Z"); // exit + 18 ms
        f.line(&late);
        let s = f.summary();
        assert!(s.exited, "still a clean exit");
        assert_eq!(s.exit_ms, Some(exit));
        // A real resume, minutes later, is content like any other.
        f.line(&prompt(900, "u3", "back"));
        assert!(!f.summary().exited);
    }

    #[test]
    fn background_launched_by_an_earlier_process_is_discounted() {
        let f = fold(&[
            prompt(0, "u1", "go"),
            result(10, "u2", "t1", "Workflow launched in background. Task ID: wold00001\nSummary: x"),
            result(11, "u3", "t2", "Command running in background with ID: bold00001. Output is being written to: /tmp/o"),
            prompt(4000, "u4", "after a restart"),
            result(4010, "u5", "t3", "Command running in background with ID: bnew00001. Output is being written to: /tmp/o"),
        ]);
        let s = f.summary();
        assert_eq!(
            s.bg,
            Bg {
                agents: 0,
                workflows: 1,
                shells: 2,
                monitors: 0
            }
        );
        assert_eq!(
            s.bg_tasks
                .iter()
                .map(|t| (t.id.as_str(), t.kind))
                .collect::<Vec<_>>(),
            vec![
                ("wold00001", "workflow"),
                ("bold00001", "shell"),
                ("bnew00001", "shell")
            ]
        );
        // The process holding the session now registered at 3990 s.
        assert_eq!(
            s.bg_since(Some(ms(3990)), 5_000),
            Bg {
                agents: 0,
                workflows: 0,
                shells: 1,
                monitors: 0
            }
        );
        assert_eq!(
            s.bg_since(None, 5_000),
            s.bg,
            "with no start time, everything counts"
        );
    }

    #[test]
    fn a_resume_after_a_crash_closes_what_the_dead_process_left() {
        // 1cea2166 1271–1280: a task-notification turn, the process dies, a new one writes
        // the resume pair (stamped when it resumed) and then takes the next message.
        let mut f = fold(&[
            json!({"type":"user","uuid":"u1","timestamp":at(0),"promptSource":"sdk","origin":{"kind":"task-notification"},
                   "message":{"role":"user","content":"<task-notification>…</task-notification>"}}),
            result(
                1,
                "u2",
                "t1",
                "Workflow launched in background. Task ID: wdead0001\nSummary: x",
            ),
            assistant(
                2,
                "a1",
                "m1",
                Some("tool_use"),
                call("t2", "Bash", json!({"command":"sleep 600"})),
            ),
        ]);
        assert!(f.summary().turn_open);
        f.line(&json!({"type":"user","uuid":"u3","timestamp":at(300),"isMeta":true,
                       "message":{"role":"user","content":[{"type":"text","text":"Continue from where you left off."}]}}));
        f.line(&json!({"type":"assistant","uuid":"a2","timestamp":at(300),"isApiErrorMessage":false,
                       "message":{"id":"m2","model":"<synthetic>","stop_reason":"stop_sequence","content":[{"type":"text","text":"No response requested."}]}}));
        let s = f.summary();
        assert!(!s.turn_open && !s.failed, "resumed, not failed");
        assert!(s.pending.is_empty());
        assert_eq!(s.bg.workflows, 0);
    }

    #[test]
    fn a_resume_after_a_crash_empties_the_queue_the_dead_process_held() {
        // 31044fb2 1062–1076: two peer messages were queued while a question waited, and the
        // Mac went down. The new process enqueues its sweep of the old one's shells and takes
        // it up at once — its first dequeue — and only then writes the resume pair, stamped
        // earlier. Neither peer message was ever delivered: nothing is queued any more.
        let sweep = "<task-notification>\n<task-id>bl5v0z294</task-id>\n<task-id>b7dmq7h3r</task-id>\n<task-id>bl5ro1shd</task-id>\n<task-id>__orphan_summary__:shell</task-id>\n<status>stopped</status>\n<summary>3 background shell command tasks didn't finish before the previous session ended.</summary>\n</task-notification>";
        let mut f = fold(&[
            json!({"type":"user","uuid":"u0","timestamp":"2026-09-15T20:50:02.000Z","promptSource":"sdk","origin":{"kind":"human"},
                   "message":{"role":"user","content":"ssh to the AP and check the build"}}),
            json!({"type":"assistant","uuid":"a0","timestamp":"2026-09-15T20:53:39.622Z",
                   "message":{"id":"msg_011Cf5tXUthte8xYmWQwbWma","role":"assistant","stop_reason":"tool_use",
                              "content":[{"type":"tool_use","id":"toolu_01RziTSNF17ZtT46yySjiXc5","name":"AskUserQuestion",
                                          "input":{"questions":[{"question":"Your Mac's user database has stopped resolving uid 501. Reboot?"}]}}]}}),
            json!({"type":"queue-operation","operation":"enqueue","timestamp":"2026-09-15T20:55:20.355Z",
                   "content":"<task-notification>\n<task-id>bl5ro1shd</task-id>\n<status>failed</status>\n</task-notification>"}),
            json!({"type":"queue-operation","operation":"enqueue","timestamp":"2026-09-15T20:55:36.018Z",
                   "content":"<cross-session-message from=\"uds:/tmp/cc-socks/34905.sock\" from-name=\"mamoru-d0\">\nConfirmed independently: `whoami` returns `501` here too.</cross-session-message>"}),
            json!({"type":"queue-operation","operation":"enqueue","timestamp":"2026-09-15T20:58:58.012Z",
                   "content":"<cross-session-message from=\"uds:/tmp/cc-socks/34905.sock\" from-name=\"mamoru-d0\">\none line (relay to nami-06)</cross-session-message>"}),
            // The crash; a new process.
            json!({"type":"queue-operation","operation":"enqueue","timestamp":"2026-09-15T21:14:04.594Z","content":sweep}),
            json!({"type":"queue-operation","operation":"dequeue","timestamp":"2026-09-15T21:14:06.609Z"}),
            json!({"type":"user","uuid":"u1","timestamp":"2026-09-15T21:14:02.552Z","toolDenialKind":"interrupted",
                   "message":{"role":"user","content":[{"type":"tool_result","content":"[Request interrupted by user for tool use]","is_error":true,"tool_use_id":"toolu_01RziTSNF17ZtT46yySjiXc5"}]}}),
            json!({"type":"user","uuid":"u2","timestamp":"2026-09-15T21:14:02.552Z","isMeta":true,
                   "message":{"role":"user","content":[{"type":"text","text":"Continue from where you left off."}]}}),
        ]);
        assert_eq!(
            f.summary().queued,
            0,
            "the queue died with the process that held it"
        );
        f.line(&json!({"type":"assistant","uuid":"a1","timestamp":"2026-09-15T21:14:02.567Z","isApiErrorMessage":false,
                       "message":{"id":"c8d0fb1f","role":"assistant","stop_reason":"stop_sequence","content":[{"type":"text","text":"No response requested."}]}}));
        f.line(&json!({"type":"user","uuid":"u3","timestamp":"2026-09-15T21:14:12.746Z","promptSource":"sdk","origin":{"kind":"task-notification"},
                       "message":{"role":"user","content":sweep}}));
        let s = f.summary();
        assert!(s.turn_open);
        assert_eq!(s.queued, 0, "not '1 queued' through the next turns");
        // What the new process queues is counted as ever.
        f.line(&json!({"type":"queue-operation","operation":"enqueue","timestamp":"2026-09-15T21:25:40.777Z",
                       "content":"<cross-session-message from=\"uds:/tmp/cc-socks/2050.sock\" from-name=\"mamoru-42\">\nYou inherit the lane.</cross-session-message>"}));
        assert_eq!(f.summary().queued, 1);
        f.line(&json!({"type":"queue-operation","operation":"dequeue","timestamp":"2026-09-15T21:25:40.779Z"}));
        assert_eq!(f.summary().queued, 0);
    }

    #[test]
    fn an_interrupt_ends_the_turn_in_either_form() {
        let mut f = fold(&[
            prompt(0, "u1", "go"),
            assistant(
                2,
                "a1",
                "m1",
                Some("tool_use"),
                call("t1", "Bash", json!({"command":"cargo test"})),
            ),
            result(
                9,
                "u2",
                "t1",
                "The user doesn't want to proceed with this tool use. The tool use was rejected.",
            ),
            json!({"type":"user","uuid":"u3","timestamp":at(9),
                   "message":{"role":"user","content":[{"type":"text","text":"[Request interrupted by user for tool use]"}]}}),
        ]);
        let s = f.summary();
        assert!(!s.turn_open && s.pending.is_empty());
        assert_eq!(s.last_end, Some(LastEnd::Interrupt));
        assert_eq!(s.turn_end_ts, Some(ms(9)));

        f.line(&prompt(20, "u4", "again"));
        f.line(&assistant(
            22,
            "a2",
            "m2",
            Some("tool_use"),
            call("t2", "Bash", json!({"command":"ls"})),
        ));
        f.line(&json!({"type":"user","uuid":"u5","timestamp":at(23),"message":{"role":"user","content":"[Request interrupted by user]"}}));
        assert_eq!(f.summary().last_end, Some(LastEnd::Interrupt));
        assert!(f.summary().pending.is_empty());
    }

    #[test]
    fn older_files_start_turns_on_typed_text_only() {
        let typed = json!({"type":"user","uuid":"u1","timestamp":at(0),"message":{"role":"user","content":"do the thing"}});
        assert!(fold(&[typed]).summary().turn_open);
        for not_a_prompt in [
            json!({"type":"user","uuid":"u2","timestamp":at(0),"message":{"role":"user","content":"<command-name>/effort</command-name>\n<command-args>max</command-args>"}}),
            json!({"type":"user","uuid":"u3","timestamp":at(0),"message":{"role":"user","content":"<local-command-caveat>Caveat: generated by local commands.</local-command-caveat>"}}),
            json!({"type":"user","uuid":"u4","timestamp":at(0),"message":{"role":"user","content":[{"type":"text","text":"<local-command-stdout>Set effort</local-command-stdout>"}]}}),
            json!({"type":"user","uuid":"u5","timestamp":at(0),"isMeta":true,"message":{"role":"user","content":"skill body"}}),
            json!({"type":"user","uuid":"u6","timestamp":at(0),"isCompactSummary":true,"message":{"role":"user","content":"This session is being continued"}}),
            json!({"type":"user","uuid":"u7","timestamp":at(0),"message":{"role":"user","content":"Continue from where you left off."}}),
        ] {
            assert!(
                !fold(std::slice::from_ref(&not_a_prompt)).summary().turn_open,
                "{not_a_prompt}"
            );
        }
    }

    #[test]
    fn a_turn_whose_prompt_is_outside_the_window_is_still_a_turn() {
        let f = fold(&[
            result(0, "u1", "t0", "earlier output"),
            assistant(
                4,
                "a1",
                "m1",
                Some("tool_use"),
                call("t1", "Grep", json!({"pattern":"fold"})),
            ),
        ]);
        let s = f.summary();
        assert!(s.turn_open);
        assert_eq!(
            s.turn_start_ts,
            Some(ms(4)),
            "dated by the first line that proves it"
        );
        assert_eq!(s.pending[0].summary, "fold");
    }

    #[test]
    fn feed_folds_whole_lines_and_leaves_the_partial_one() {
        let a = serde_json::to_string(&prompt(0, "u1", "hi")).unwrap();
        let b = serde_json::to_string(&assistant(1, "a1", "m1", Some("end_turn"), said("hello")))
            .unwrap();
        let bytes = format!("{a}\n{b}\n{{\"type\":\"user\",\"timest");
        let mut f = Fold::new();
        let used = f.feed(bytes.as_bytes());
        assert_eq!(used, a.len() + b.len() + 2);
        assert!(!f.summary().turn_open);
        assert_eq!(f.stats().unparsed, 0, "the partial tail was not attempted");

        // A window that starts mid-file drops its first (partial) line.
        let window = format!("tail of something\"}}\n{a}\n");
        let start = skip_partial_line(window.as_bytes(), true);
        let mut g = Fold::new();
        g.feed(&window.as_bytes()[start..]);
        assert!(g.summary().turn_open);
        assert_eq!(skip_partial_line(window.as_bytes(), false), 0);
        assert_eq!(initial_offset(5), 0);
        assert_eq!(initial_offset(INITIAL_WINDOW + 7), 7);
    }

    #[test]
    fn junk_and_torn_lines_are_counted_not_fatal() {
        let mut f = Fold::new();
        f.feed(b"not json\n{\"type\":\"user\"\n\n");
        assert_eq!(f.stats().unparsed, 2);
        assert_eq!(f.summary(), FoldSummary::default());
    }

    #[test]
    fn the_identifying_field_of_a_call_is_its_summary() {
        assert_eq!(
            summarise_input(Some(&json!({"command":"git status"}))),
            "git status"
        );
        assert_eq!(
            summarise_input(Some(&json!({"file_path":"/a/b.rs","old_string":"x"}))),
            "/a/b.rs"
        );
        assert_eq!(
            summarise_input(Some(&json!({"command":"  "}))),
            "{\"command\":\"  \"}"
        );
        assert_eq!(summarise_input(None), "");
        assert_eq!(clip(&"é".repeat(10), 4), "éééé…");
    }

    /// Fold every main transcript written in the last 28 days and report how they end.
    ///
    /// Read-only, and slow in a debug build (about 1.3 GB of JSON): run it with
    /// `cargo test --release -p agent-router fold::tests::corpus_replay -- --ignored --nocapture`.
    /// Each transcript is folded twice — from its first byte, and from its last
    /// `INITIAL_WINDOW` bytes as the router does when it first sees a file — and the two
    /// end states are compared: a rule that depends on where reading started is a bug.
    #[test]
    #[ignore]
    fn corpus_replay() {
        use std::collections::{BTreeMap, HashSet};
        use std::io::Read;

        let root = crate::registry::claude_dir().join("projects");
        let now = std::time::SystemTime::now();
        let mut files: Vec<std::path::PathBuf> = Vec::new();
        for dir in std::fs::read_dir(&root).expect("projects dir").flatten() {
            let Ok(entries) = std::fs::read_dir(dir.path()) else {
                continue;
            };
            for e in entries.flatten() {
                let p = e.path();
                if p.extension().and_then(|x| x.to_str()) != Some("jsonl") {
                    continue;
                }
                let fresh = e
                    .metadata()
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|m| now.duration_since(m).ok())
                    .is_some_and(|age| age.as_secs() < 28 * 86_400);
                if fresh {
                    files.push(p);
                }
            }
        }
        files.sort();

        #[derive(Default)]
        struct Tally {
            files: usize,
            bytes: u64,
            turn_open: usize,
            pending: usize,
            exited: usize,
            exited_mid_turn: usize,
            queued: usize,
            old_rule_queued: usize,
            bg: [usize; 4],
            bg_files: usize,
            bg_with_note: usize,
            reappended: u64,
            stale: u64,
            settled_early: u64,
            continuation_holds: u64,
            thinking_only_msgs: usize,
            thinking_only_left_open: usize,
            failed: usize,
            window_disagree: usize,
            unparsed: u64,
        }
        let mut t = Tally::default();
        let mut notes: BTreeMap<&'static str, Vec<String>> = BTreeMap::new();
        // With FOLD_DUMP_DIR set, also write what `activity::derive` says after every line,
        // with no registry and no subagent evidence (regime B), for the replay harness to
        // integrate over time against its truth model.
        let dump_dir = std::env::var_os("FOLD_DUMP_DIR").map(std::path::PathBuf::from);
        if let Some(d) = &dump_dir {
            std::fs::create_dir_all(d).unwrap();
            let list: Vec<String> = files
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect();
            std::fs::write(d.join("files.json"), serde_json::to_string(&list).unwrap()).unwrap();
        }
        let short = |p: &std::path::Path| p.file_stem().unwrap().to_string_lossy()[..8].to_string();

        for path in &files {
            let mut bytes = Vec::new();
            std::fs::File::open(path)
                .unwrap()
                .read_to_end(&mut bytes)
                .unwrap();
            t.files += 1;
            t.bytes += bytes.len() as u64;

            // The whole file, with an independent look at the raw lines alongside.
            let mut f = Fold::new();
            let mut old_queue = 0usize;
            let mut noted_ids: HashSet<String> = HashSet::new();
            // message id -> (closing stop seen, visible block seen)
            let mut msgs: BTreeMap<String, (bool, bool)> = BTreeMap::new();
            let mut dump = String::new();
            for raw in bytes.split(|&b| b == b'\n') {
                if raw.iter().all(u8::is_ascii_whitespace) {
                    continue;
                }
                let parsed = serde_json::from_slice::<Value>(raw);
                match &parsed {
                    Ok(v) => f.line(v),
                    Err(_) => f.feed_line(raw),
                }
                if dump_dir.is_some() {
                    use std::fmt::Write as _;
                    let s = f.summary();
                    let x = crate::activity::derive(&crate::activity::Evidence {
                        now_ms: s.last_content_ts.unwrap_or(0),
                        registry_available: false,
                        record: None,
                        wrapper_alive: true,
                        prompts: &[],
                        fold: &s,
                        subagent_write_ms: None,
                    });
                    let _ = writeln!(
                        dump,
                        "{}\t{}",
                        x.kind.as_str(),
                        s.last_content_ts.unwrap_or(0)
                    );
                }
                let Ok(v) = parsed else { continue };
                match v.get("type").and_then(Value::as_str) {
                    Some("queue-operation") => match v.get("operation").and_then(Value::as_str) {
                        Some("enqueue") => old_queue += 1,
                        Some("dequeue" | "remove") => old_queue = old_queue.saturating_sub(1),
                        _ => {}
                    },
                    Some("assistant") if !flag(&v, "isApiErrorMessage") => {
                        let m = &v["message"];
                        if let Some(id) = m["id"].as_str() {
                            let e = msgs.entry(id.to_owned()).or_default();
                            if matches!(m["stop_reason"].as_str(), Some("end_turn")) {
                                e.0 = true;
                            }
                            for b in m["content"]
                                .as_array()
                                .map(Vec::as_slice)
                                .unwrap_or_default()
                            {
                                match b["type"].as_str() {
                                    Some("tool_use") => e.1 = true,
                                    Some("text")
                                        if b["text"]
                                            .as_str()
                                            .is_some_and(|s| !s.trim().is_empty()) =>
                                    {
                                        e.1 = true
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                    _ => {}
                }
                let text = String::from_utf8_lossy(raw);
                if text.contains("<task-notification>") {
                    for chunk in text.split("<task-id>").skip(1) {
                        if let Some(id) = chunk.split("</task-id>").next() {
                            noted_ids.insert(id.to_owned());
                        }
                    }
                }
            }
            if let Some(d) = &dump_dir {
                std::fs::write(
                    d.join(format!(
                        "{}.tsv",
                        path.file_stem().unwrap().to_string_lossy()
                    )),
                    &dump,
                )
                .unwrap();
            }
            let s = f.summary();
            let st = f.stats();
            let name = short(path);
            t.reappended += st.reappended;
            t.stale += st.stale;
            t.settled_early += st.settled_early_hits;
            t.continuation_holds += st.continuation_holds;
            t.unparsed += st.unparsed;
            let thinking_only = msgs.values().filter(|(end, vis)| *end && !*vis).count();
            t.thinking_only_msgs += thinking_only;
            if s.turn_open {
                t.turn_open += 1;
                if thinking_only > 0 {
                    t.thinking_only_left_open += 1;
                }
                notes
                    .entry("ends with a turn open")
                    .or_default()
                    .push(format!(
                        "{name} pending={} exited={} last_end={:?} start={:?}",
                        s.pending.len(),
                        s.exited,
                        s.last_end,
                        s.turn_start_ts
                    ));
            }
            if !s.pending.is_empty() {
                t.pending += 1;
                notes
                    .entry("ends with calls pending")
                    .or_default()
                    .push(format!(
                        "{name} {}",
                        s.pending
                            .iter()
                            .map(|p| format!(
                                "{}({})",
                                p.name,
                                &p.id[p.id.len().saturating_sub(8)..]
                            ))
                            .collect::<Vec<_>>()
                            .join(",")
                    ));
            }
            if s.exited {
                t.exited += 1;
                if s.turn_open {
                    t.exited_mid_turn += 1;
                    notes
                        .entry("exited with a turn open")
                        .or_default()
                        .push(name.clone());
                }
            }
            if s.failed {
                t.failed += 1;
            }
            if s.queued != 0 {
                t.queued += 1;
                notes
                    .entry("ends with messages queued")
                    .or_default()
                    .push(format!("{name} queued={}", s.queued));
            }
            if old_queue != 0 {
                t.old_rule_queued += 1;
            }
            let bg = [s.bg.agents, s.bg.workflows, s.bg.shells, s.bg.monitors];
            if bg.iter().any(|&n| n > 0) {
                t.bg_files += 1;
                for (acc, n) in t.bg.iter_mut().zip(bg) {
                    *acc += n;
                }
                let out = f.outstanding();
                let noted: Vec<&String> = out.iter().filter(|id| noted_ids.contains(*id)).collect();
                if !noted.is_empty() {
                    t.bg_with_note += 1;
                }
                notes.entry("ends with background work outstanding").or_default().push(format!(
                    "{name} agents={} workflows={} shells={} monitors={} ids={:?} notified-but-open={:?}",
                    bg[0], bg[1], bg[2], bg[3], out, noted
                ));
            }

            // The same file from its last INITIAL_WINDOW bytes, as the router first sees it.
            let from = initial_offset(bytes.len() as u64) as usize;
            let skip = skip_partial_line(&bytes[from..], from > 0);
            let mut w = Fold::new();
            w.feed(&bytes[from + skip..]);
            let ws = w.summary();
            let key = |x: &FoldSummary| {
                (
                    x.turn_open,
                    x.pending.iter().map(|p| p.id.clone()).collect::<Vec<_>>(),
                    x.exited,
                    x.queued,
                    x.bg,
                    x.last_end,
                    x.failed,
                    x.ask_pending.as_ref().map(|a| a.id.clone()),
                )
            };
            if from > 0 && key(&ws) != key(&s) {
                t.window_disagree += 1;
                notes.entry("window and whole file disagree").or_default().push(format!(
                    "{name} whole: open={} pend={} exited={} q={} bg={:?} end={:?} | window: open={} pend={} exited={} q={} bg={:?} end={:?}",
                    s.turn_open, s.pending.len(), s.exited, s.queued, s.bg, s.last_end,
                    ws.turn_open, ws.pending.len(), ws.exited, ws.queued, ws.bg, ws.last_end
                ));
            }
        }

        println!(
            "\n== corpus: {} main transcripts, {:.1} MB, modified in the last 28 days",
            t.files,
            t.bytes as f64 / 1e6
        );
        println!("ending with a turn open .............. {}", t.turn_open);
        println!(
            "  of which hold a thinking-only end ... {}",
            t.thinking_only_left_open
        );
        println!("ending with calls pending ............ {}", t.pending);
        println!("ending exited (cost-state) ........... {}", t.exited);
        println!(
            "  of which with a turn open .......... {}",
            t.exited_mid_turn
        );
        println!("ending failed ........................ {}", t.failed);
        println!(
            "queued != 0 at the end ............... {} (old rule: {})",
            t.queued, t.old_rule_queued
        );
        println!("background outstanding at the end .... {} transcripts: agents={} workflows={} shells={} monitors={}", t.bg_files, t.bg[0], t.bg[1], t.bg[2], t.bg[3]);
        println!(
            "  with a notification seen for an open id: {}",
            t.bg_with_note
        );
        println!("re-appended lines skipped (uuid) ..... {}", t.reappended);
        println!("stale lines skipped (5 min) .......... {}", t.stale);
        println!("settled_early hits ................... {}", t.settled_early);
        println!(
            "thinking-only end_turn messages ...... {}",
            t.thinking_only_msgs
        );
        println!(
            "continuation holds ................... {}",
            t.continuation_holds
        );
        println!("unparsed lines ....................... {}", t.unparsed);
        println!(
            "2 MiB window vs whole file disagree .. {}",
            t.window_disagree
        );
        for (k, v) in &notes {
            println!("\n-- {k} ({})", v.len());
            for line in v {
                println!("   {line}");
            }
        }
    }
}
