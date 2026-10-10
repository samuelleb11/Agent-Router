# Session liveness: what a running Claude Code session tells you, and how the router reads it

2026-09-29. Measured on one Mac running Claude Code 2.1.267 (terminal) and the VS Code
extension at 2.1.280, 2.1.283 and 2.1.284, against the 154–157 main transcripts written in
the previous 28 days (1.27 GB). The investigation behind it had six lenses (the registry,
the transcript, the wrapper, the clients, a live audit, the history), a synthesis and two
adversarial verifications; its raw material lived in a scratch directory that is not kept.
This is what it established, with the numbers. The code that applies it is
`src/registry.rs`, `src/fold.rs`, `src/activity.rs` and `live::session_activity`.

## In one paragraph

A session's transcript is a record of what *happened*, written on its own schedule. It is
silent while the model generates (an assistant message lands only when complete: silences
of 110 s measured, p99 116 s from prompt to first reply), silent while background agents
and workflows run (they write elsewhere), never records a permission prompt, says nothing
about whether the process is alive, and its mtime is moved by bookkeeping. Claude Code
already publishes the process-level truth in `~/.claude/sessions/<pid>.json`: which
conversation each live process holds, and a `status` of `busy`, `waiting`, `idle` or
`shell` written by the CLI's own state machine. The router reads that first, verified
against the live process, and uses the transcript for detail and as the fallback. One
function (`activity::derive`) turns the evidence into one answer that every surface uses.

## What was wrong before, so it is not rebuilt

Four derivations, each reading the main transcript on its own: `tail_state` for the list,
`status_of` for the open view, `live_prompt` for prompts, and `notify::waiting`. Measured
consequences:

| Symptom | Numbers |
|---|---|
| A generating session read `idle` in the list | 6,827 generation phases in 7 days, 27.6 h shown as idle; the orchestrator's own 397 s turn read idle for 385 s (97%) |
| A session running a background workflow read `idle`, `active=false` | 272 of 272 samples, while its registry said `busy` and seven agent transcripts grew every ~2 s |
| A closed session read exactly like an open idle one | 19 of 24 listed sessions had no process; all showed `idle · Waiting for you` |
| `active` and `last_activity_ms` followed mtime | an exit flush five days after the last message moved a session to the top; an idle session gained a ledger line every 1–4 h (46 h "newer" than its last message); subagents' queue lines flipped `active` with no change underneath |
| The list and the open view disagreed during every turn | generating: `idle` vs `Working`; a Bash prompt: `ask` vs `Running Bash`; an API error: `failed` vs `Waiting for you` |
| The stream re-sent an unchanged status 5 times a second | 1,016 events in 205 s per open view |
| Permission prompts of unwrapped sessions were invisible | only AskUserQuestion was detected, and only after its line landed |

## The signals

| Signal | Tells | Latency | Coverage | Failure modes | Used for |
|---|---|---|---|---|---|
| Registry record `~/.claude/sessions/<pid>.json`, verified by pid and start time | a live process holds this conversation (open/closed) | written 0.8–6.2 s after process start; unlinked on any graceful exit | every registered CLI on this machine and config dir | stale after SIGKILL/crash until the next CLI start sweeps it; a reused pid is never swept | open/closed, regime A |
| Registry `status`, `waitingFor`, `statusUpdatedAt` | busy / waiting / idle / shell | busy at prompt dequeue (+0 ms from enqueue, ~50 ms before the user line); idle 9–21 ms after `end_turn` | TUI writer 2.1.266+, headless writer 2.1.278+; absent on older headless CLIs | no heartbeat; a hung process stays busy; the record is first written without a status | the class in regime A |
| Main transcript content lines | turns, tool calls and results, errors, background launches and completions, titles | user/queue lines within ~50 ms; assistant messages only when complete | every session | file order is not event order; compaction re-appends; a crash leaves a turn open | detail; the whole story in regime B |
| Main transcript mtime and size | something was appended | immediate | every session | moved by the exit flush, ledgers, titles, subagents' queue lines; not moved by generation | only "re-read this file", and a last-resort `last_activity_ms` |
| Subagent transcripts `<session>/subagents/*.jsonl`, `<session>/subagents/workflows/<wf>/*.jsonl` | background agents are making progress | written per content block (~every 2 s observed) | Agent and Workflow subagents | thousands of files; not liveness of the parent | the Background class, `last_activity_ms`, holding off "stalled" |
| Wrapper socket `/tmp/agent-router-sessions/<id>.sock` (bare connect) | a message, a stop or an answer can be delivered | 3–162 ms to reply | wrapped sessions only | leaked on every editor close by old wrappers (14 of 19 dead); the deployed Sep 12 wrapper keeps the first session id through `/clear` | `wrapped`, open when there is no registry |
| Wrapper `.pending` | the open `can_use_tool` requests, with their payload | immediate | wrapped sessions | torn reads (0.02–1.5 %), request-id order in old wrappers, cancels not cleared in old wrappers | Waiting, prompt detail, answers, `req:` notifications |

Not used, and why: the process table beyond `proc_pidinfo` (costly, cannot tell a sleep
from a hang); `.key` files and `/tmp/cc-socks` (they outlive their process, and keys are
secrets); `updatedAt`/`statusUpdatedAt` as liveness (no heartbeat: idle records sit
untouched for days); the proxy's in-flight requests (not tracked, blind to unproxied
sessions, and needs a classifier for auxiliary requests); the wrapper's stdout frames
(`session_state_changed` exists only for 2.1.284 hosts and needs a wrapper change).

## The session registry

### Files

`$CLAUDE_CONFIG_DIR/sessions`, else `~/.claude/sessions` (mode 0700). Four kinds of file:

- `<pid>.json` (0644), one per registered CLI process — the only ones to read. The CLI's own
  reader deletes names that are not canonical; the router only ever parses `^\d+\.json$`.
- `<pid>.<sha256(socket path)>.key` (0600): `{peerToken, procStart, pidDomain}`. The token is
  a secret. Never opened for status (the registry reader provably never opens one).
- `<pid>.<hash>.key.tmp.<hex>` during a key write, and `.fleetview-heartbeat`.

### Fields

Carried today: `pid` (the file name wins), `sessionId`, `cwd`, `startedAt` (ms, the
registration time), `procStart`, `version`, `kind`, `entrypoint`, `name`, `nameSource`,
`nameSince`, `peerProtocol`, `peerFeatures`, `pidDomain`, `messagingSocketPath`,
`updatedAt`, `status`, `statusUpdatedAt`, and `waitingFor` while waiting. The CLI's reader
also knows `jobId`, `parkedJobId`, `spare`, `bridgeSessionId`, `hostSessionId`, `tempo`,
`needs` and others.

- `procStart` is `ps -o lstart= -p <pid>` run with `TZ=UTC LC_ALL=C`: a UTC ctime string
  with one-second resolution, e.g. `Tue Sep 29 09:34:36 2026`. Plain `ps` prints local time,
  which is how a naive check marks every session stale. Two processes can share a second.
- `kind` is `interactive` unless `CLAUDE_CODE_SESSION_KIND` says `bg`, `daemon` or
  `daemon-worker`; VS Code sessions and `claude -p` are both `interactive`. `entrypoint`
  tells them apart (`claude-vscode`, `cli`, `sdk-cli`, `mcp`).
- `sessionId` is rewritten in place when the process switches conversation (`/clear`,
  resume, fork, `cd`, remote attach). Re-map on every read; never cache pid → session.

### Status values

Only `busy`, `waiting`, `idle` and `shell` are accepted by the CLI; anything else, or no
status at all, is not evidence of anything and selects the transcript (regime B).

- **Headless writer** (VS Code, `-p`, SDK; CLI 2.1.278 and later): a direct projection of the
  SDK session state — `running` → busy, `requires_action` → waiting, `idle` → idle.
  `waitingFor` is `input needed` for AskUserQuestion and any `dialog:*`, otherwise
  `permission prompt`. Writes only when (status, waitingFor) changes; each write stamps
  `statusUpdatedAt`.
- **TUI writer** (terminal; 2.1.266 and later): waiting > busy > shell > idle. Waiting labels
  add `dialog open`, `goal proposal`, `sandbox request`, `worker request`. `shell` is idle
  with a background Bash running. The TUI holds busy for background agents too.
- **No status**: headless CLIs 2.1.267 and older (`~/.local/bin/claude -p` here), and every
  record between registration and its first status change.

### The idle gate

At a headless turn's end the status goes idle only when nothing else holds it: background
`local_agent`, `local_workflow`, `remote_agent`, `mcp_task` or `auto_mode_scan` tasks,
active teammates, or an agent completion not yet delivered keep it **busy after
`end_turn`** (verified: the orchestrator stayed busy 20+ minutes past its turn while a
workflow ran). A background **Bash does not**: a session whose 12-hour loop still runs read
idle for 6.5 h — and when that loop ends, its notification starts a turn by itself.

### Lifecycle

- Registration: written once at startup, in place (truncate then write, not atomic), 0.8–6.2 s
  after the process starts. Not registered: parked spares (until claimed), swarm teammates,
  interactive TTY children. Non-interactive children *do* register — including `claude -p`
  run from a session's Bash tool (inheriting `entrypoint=claude-vscode`), and the router's
  own `/btw` fork.
- Updates: read-merge-write, serialized per process. If the file is missing the update fails
  and the record is never recreated: deleting a live process's record makes it invisible
  for life. The router never writes, renames or deletes anything there.
- Removal: unlinked synchronously on exit, including SIGINT/SIGTERM/SIGHUP. SIGKILL, a crash
  or power loss leave it; the next CLI start deletes records whose pid returns ESRCH. A pid
  reused by another process is never swept.
- VS Code starts short-lived CLIs that register and deregister within seconds (the config
  loader, pre-spawned panels); their `sessionId` has no transcript.

### Verifying a record (`registry.rs`)

1. Parse only `^\d+\.json$`; on a parse failure re-read after 25 ms (the CLI's own retry),
   then fall back to the last good parse of that file.
2. `proc_pidinfo(PROC_PIDTBSDINFO)`: dead on ESRCH or a zombie. Compare `pbi_start_tvsec`
   with `procStart` parsed as UTC, within 1 s. One syscall, no `ps`, no time zone. Other
   operating systems: `kill(pid, 0)` liveness only.
3. Cache the verdict per (pid, procStart, observed start).
4. Parser guard: if at least two live pids all fail the start check, suspect the parser or
   the clock, not the processes — verify by liveness only and warn once.
5. Drop records whose parent or grandparent is the router (its `/btw` fork). Their session
   ids are kept aside so the list can leave the fork's transcript out while it runs.
6. Several live records naming one session: the newest `statusUpdatedAt`, then `updatedAt`.

A snapshot is reused for 250 ms. The directory unreadable means regime C: nothing can be
said about open or closed from the registry.

## The transcript

`<claude dir>/projects/<cwd-slug>/<session>.jsonl`, appended as things happen.

### Write timing

- A **main** transcript writes an assistant message only when it is complete; every line of
  it carries the message's final `stop_reason`, and each `tool_use` block flushes as it
  completes, so a tool can start — even finish — before its own call line is visible
  (3,616 of 88,570 messages). **Subagent** transcripts write block by block with
  `stop_reason: null` until the last block.
- A line's `timestamp` is when its block completed (when a tool started), not when it was
  written. Writes follow within ~0.1 s.
- From a prompt to the first visible assistant line: p50 16.8 s, p90 40.3 s, p99 115.7 s.
  Any input to the next visible line: p50 11 s, p99 72 s, 251 gaps over two minutes in 28
  days. Auto-compaction is silent for p50 126 s, max 198 s. Generating is the largest share
  of busy time (42 %), ahead of tools (37 %) and waiting on a person (21 %).

### Turns

- **Start**: a user line carrying `promptSource`, `origin` or `turnOrigin` (typed, peer and
  task-notification turns alike), written just after its queue enqueue/dequeue pair. Older
  files: a user line of real typed text — not `isMeta`, not `isCompactSummary`, not a
  slash-command echo (`<command-name>`, `<local-command-stdout>`,
  `<local-command-caveat>`), not an interrupt marker, not `Continue from where you left
  off.`, not tool results only.
- **End**: `end_turn` with a text block (95.0 % of 6,548 turns), an interrupt marker (3.2 %),
  an `isApiErrorMessage` line (1.1 %), or `stop_sequence` / `max_tokens` / `refusal`. A
  thinking-only `end_turn` happened 4 times in 28 days, 3 of them followed by an automatic
  continuation (a `turnCompanion` line). There is no explicit turn-end record in VS Code
  sessions (`system/turn_duration` exists only for `entrypoint=cli`).
- **A message that calls a tool ends nothing**, whatever its `stop_reason`: the CLI runs the
  call and asks the model again. Its blocks arrive as separate lines, each carrying the
  final `stop_reason`, so the line that shows the call can come after one that already
  looked like the end. 0b2e2351 2370–2378: message `U7AGi4cT` is a thinking line then a
  SendMessage line, both `end_turn`; the model answered the result 15 s later (1 such
  message in 28 days). The fold closes the turn on the thinking line, remembers which
  message closed it, and reopens the turn when a later line of that same message carries a
  `tool_use` block — decided by the call being on the line, not by it being pending, since a
  fast call's result can be written before the call. Only a closing line is undone this way:
  an interrupt or an API error is final.
- `stop_reason` counts on assistant lines: tool_use 194,255; end_turn 7,845; null 14,948
  (subagent per-block lines and aborted partials); stop_sequence 106; refusal 2;
  max_tokens 1; pause_turn never.

### Bookkeeping lines

No `timestamp`, no effect on state, never activity: `ai-title`, `atis-latch` and
`last-prompt` (a triplet after model responses), `mode`, `permission-mode`, `custom-title`,
`agent-name`, `bridge-session`, `worktree-state`, `file-history-*`, `artifact-*`, and
`cost-state` except as the exit marker. `attachment` lines (70,940 of them, prompt
snapshots up to 310 KB) are written while the next request is built. The newest `ai-title`
is the session's name.

### Out of order, and twice

- A tool result can precede its own call (four real cases: fast async Agent launches, a
  Skill): pair calls and results by id in either order.
- Compaction re-appends a preserved segment with the **same uuid and timestamp** as the
  originals (139 of 139 lines in one case): skip a uuid already seen, and skip user and
  assistant lines stamped more than 5 minutes behind the newest content.
- `system/api_error` retry lines and slash-command echoes are sometimes written late.

### Errors, retries, interruptions

- Each retry writes `system/api_error` (`retryAttempt` 1–10, `retryInMs` p50 2.4 s). A final
  failure is an assistant line with `isApiErrorMessage: true`, model `<synthetic>`,
  `stop_sequence`, and text `API Error: …` or `You've hit your … limit`.
- An interrupted generation leaves partial lines (`stop_reason: null`, the last flagged
  `isAbortedMidStream`) then `[Request interrupted by user]`. An interrupted tool leaves a
  synthetic `is_error` result then `[Request interrupted by user for tool use]`.

### Exit and resume

- A graceful exit appends `last-prompt` then `cost-state`; `startTime + totalDuration` is the
  exit time to the second (7 of 7 checked). Older builds write only a trailing `last-prompt`;
  print-mode runs write nothing; a crash writes nothing. A `cost-state` in the middle of a
  file is an earlier exit that was later resumed. Content stamped within 2 s after the exit
  is the shutdown's own last write (a task-notification turn 18 ms after exit was seen).
- Resuming a turn whose process died writes `Continue from where you left off.` (`isMeta`)
  and a synthetic `No response requested.`. Whatever that process was running died with it:
  its background tasks, its open calls, and its queue. 31044fb2: two peer messages queued
  before a crash were never delivered, and a fold that kept them read `1 queued` for 21
  minutes after the resume, so the marker empties the count, as an exit does. The marker is
  sometimes written after the new process's first queue lines (its sweep of the old
  process's shells, enqueued and taken up at once), so a message queued in those seconds
  can go uncounted until it is taken up.

### The queue

`queue-operation` lines: `enqueue`, then `dequeue` (it becomes a turn) or `remove`
(`absorbed_mid_turn`, `delivered_to_agent`, `cleared_on_cancel`). Subagents' background
tasks report through the **parent's** queue as `<task-notification>` entries while the
parent's own turn is over — they are not messages anyone typed, and they are not activity.
A `dequeue` names nothing; with both kinds queued the CLI took the person's message first in
1,119 of 1,125 cases.

### Background work

Launch results, recognised only at the start of a tool result:

| Kind | Result text | Registry |
|---|---|---|
| agent | `Async agent launched … agentId: <id>` | holds busy |
| workflow | `Workflow launched in background. Task ID: <id>` | holds busy |
| shell | `Command running in background with ID: <id>` | does not (headless); `shell` in the TUI |
| monitor | `Monitor started (task <id> …` | holds busy only with a timeout |

A task is over on a `<task-notification>` with `<task-id>` and a `<status>` of completed,
failed, killed or stopped (in a user line or a queue entry; one notification can list
several ids), on a monitor's `[Monitor expired…]`/`[Monitor timed out…]` event, on a TaskStop
result `Successfully stopped task: <id>`, on `No task found with ID: <id>`, and on a
TaskOutput result with a final status. Every task dies with its process at an exit or a
resume. A task stopped from the editor's task list, or killed with a process older CLIs did
not mark, is never reported: 10 of 155 transcripts end with such ids. So only tasks launched
by the *current* process count (launch ≥ registration − 5 s), and background agents count as
running only while their transcripts are written.

Subagents are never recorded in the parent file. The Agent tool writes
`<session>/subagents/agent-<id>.jsonl` (+ `.meta.json`: agentType, toolUseId, requestShape);
Workflow writes `<session>/subagents/workflows/<wf>/agent-*.jsonl` beside a `journal.jsonl`.

## The wrapper

For sessions the editor starts through `claude-wrapper`:

- `<id>.sock` answers a bare connect (`{"ok":false}`, nothing reaches the session) while the
  wrapper lives. A refused connect means a leftover; the probe deletes it and its
  `.pending`. The router probes an id at most once a second, only for open sessions (or all
  when there is no registry), and a janitor in the notifier probes, once a minute, every
  socket no open session holds — the editor's throwaway processes never write a transcript,
  so nothing else would ever look at theirs.
- The janitor clears the rest of what a dead wrapper leaves. When a socket it probes is
  refused, the `<id>.ctl` control recording and a `<id>.pending.tmp` caught mid-write go
  with it — there, and not in the per-session probe, which also runs for open sessions,
  where a refused connect can be a full backlog and the recording is what `set_model` and
  `set_mode` read their answers back from. And on every pass it removes `<id>.ctl`,
  `<id>.pending` and `<id>.pending.tmp` files with no `<id>.sock` beside them, whose id could
  be a session's (`[A-Za-z0-9-]+`), that no open session holds, untouched for 10 minutes:
  what is left once a probe has taken a refused socket and its prompts — every router
  before this one did only that, and the per-session probe still does — which nothing reads
  again (eight here, up to 342 KB each, and this Mac never reaps `/tmp`). A live wrapper is
  without its socket only for moments — before it binds, while it rebinds one that
  vanished, between the renames of a `/clear`.
- `<id>.pending` holds the open `can_use_tool` requests, one JSON line each, with
  `request.tool_use_id`. It clears when the CLI echoes the answer (sessions run with
  `--replay-user-messages`). Old wrappers kept it in request-id order and rewrote it
  truncate-then-write, so the router orders prompts by when it first saw each and keeps a
  prompt missing from one read for one more read. A prompt whose `tool_use_id` already has
  a result in the transcript is over, whatever the file says.
- One frame into the socket is one line of at most 1 MiB, newline included. The wrappers
  deployed inside long-running sessions read a frame in 8 KiB pieces, stop once they hold
  more than a mebibyte, newline or not, and write what they have into the CLI anyway —
  and the CLI exits on a line it cannot parse, taking the session with it (a 1.1 MB
  message did exactly that to 2.1.284 in a lab run). The current wrapper refuses such a
  frame whole, and only injects at a line boundary of the editor's own stream.
  The router measures each frame as it would send it, escaping included, and refuses a
  longer message or answer with 413 before connecting, which protects both.
- **Session id.** The wrapper names its files after the session the CLI announces. After
  `/clear` the CLI names the new session in the `system/init` it emits at once — 5–6 ms
  after its `conversation_reset`, whose own `new_conversation_id` is not the session id —
  and again at the start of every turn, and later frames carry the new id (2.1.278, 2.1.280,
  2.1.284). The current wrapper follows that init: its socket, `.ctl` and `.pending` move to
  the new id within milliseconds; the `hook_callback` path is only a fallback. The deployed
  Sep 12 wrapper does not follow at all: its files keep the first id, so after `/clear` the
  new conversation reads open but not wrapped, and the old one reads closed — its socket,
  still answering, is left alone (the janitor removes only refused ones).

## The derivation (`activity::derive`)

Evidence: the time, whether the registry is readable, the verified record naming the
session (R), whether the wrapper answers, the live wrapper prompts (W), the transcript fold
(F), and the newest subagent write (A). R.t is `statusUpdatedAt`. A turn **counts** when F
has it open and it started no earlier than 5 s before R registered — a turn left open by a
crashed earlier process does not.

1. **Open?** Registry readable: open exactly when R exists. Otherwise open when the wrapper
   answers, else unknown.
2. **Closed** when not open: `ended_ms` is the exit time if F saw a clean exit, else the last
   content; `ended_how` exit/unknown; `mid_turn` when F's turn is open and it did not exit.
3. **Class.**
   - Regime A (R has a known status; source `registry`). W prompts first seen more than 10 s
     ago are dropped while R is idle/shell. Waiting if R says waiting or a W prompt
     remains; Busy if R says busy; otherwise Busy if the turn counts and started after R.t
     (the registry lagged), Background if agents/workflows are outstanding and A is within
     120 s, else Idle.
   - Regime B (no known status, or no registry; source `transcript`). Waiting if a W prompt
     or F's question/plan is open (turn counts); Busy if the turn counts; Background as
     above; else Idle.
4. **Activity**, first match wins:

| # | Condition | Kind | Detail |
|---|---|---|---|
| 1 | Waiting | Waiting | from the newest W prompt, else F's question/plan, else F's newest call, else R.waitingFor: `Waiting for your answer`, `<Tool> wants to run`, `Plan waiting for approval`, `Waiting on a dialog` |
| 2 | Busy, turn counts, a call pending | Tool | `Running <name>`, since the call |
| 3 | Busy, turn counts | Generating | `Thinking`, or `Retrying (n/N)`; since the later of the turn start and the newest result |
| 4 | Busy, turn does not count, no agents/workflows out, R.t after the last turn end and under 3 s ago | Generating | `Thinking`: a new turn whose prompt line has not landed |
| 5 | Busy, turn does not count, no agents/workflows out, turn ended under 2 s ago | Failed after an API error, else Idle | what rows 7–8 are about to say: registry lag plus the 250 ms cache |
| 6a | Busy, turn does not count, R.t after the last turn end, no agents/workflows out, no subagent write within 120 s | Generating | `Working`, since R.t: the main loop with no turn open — stoppable |
| 6b | Busy or Background, with agents/workflows out or a subagent write within 120 s | Background | `N background agent(s) running` / `Background workflow running` / `Background work running`, with the counts |
| 6c | Busy (the rest): held since before the turn ended, nothing observable | Background | `Working`, no counts, since the turn end — not stoppable |
| 7 | Idle after an API error | Failed | `API error: …` |
| 8 | otherwise | Idle | `Waiting for you`, `Interrupted`; `· background shell running`, with `background.shells`, when one is |

Rows 3, 4 and 6a become **Stalled** (`No progress`) when nothing — a content line, a
registry write, a subagent write — has happened for 10 minutes. Tools never stall on silence
(they run for hours legitimately), and neither does 6c (a stall offers Stop); a dead process
is Closed, never Stalled. Details never carry a relative time; clients add it from
`since_ms`.

Derived: busy = Generating, Tool, Background or Stalled; needs you = Waiting;
`active` = either; stoppable = Generating, Tool or Stalled — **not** Background, because what
an interrupt (`cancel_queued: true`) does to running agents is unmeasured and may discard
their results.

**Busy after the turn is over** (rows 4–6) is read by when the registry went busy, because
busy is stamped when the main loop starts and never rewritten while it stays busy:

- **Stamped after the last end** — the loop started again with no prompt line. Within 3 s
  that is a prompt about to land (row 4; the CLI writes busy ~50 ms before the prompt
  line). Later it is a `/compact`, whose echo and summary are written only when it ends
  (16.7 s for a tiny conversation, 113–198 s for real ones), or a continuation the CLI
  began after going idle: the main loop at work, row 6a, stoppable — an interrupt aborts a
  running compaction within 2 ms and discards nothing. A background agent's permission
  prompt, answered, restamps busy after the end too (busy→waiting→busy), which is why rows
  4 and 6a need no agents or workflows out: while they run the CLI holds busy and never
  rewrites it, so a fresh stamp then is always theirs, never a new prompt — and the agents
  writing their transcripts put it in 6b.
- **Stamped before the end** — busy *held* past it: a Stop hook (12 and 15 s in the two
  probes); background work no transcript records (`remote_agent`, `mcp_task`,
  `auto_mode_scan` with `skipTranscript`, teammates); or a turn the CLI carries on by itself
  after a closing message — a thinking-only `end_turn`, `max_tokens`, an API error
  mid-response: 17 s to 3 min with nothing in the background, about 6 minutes in 28 days,
  all on CLIs that wrote no status (a 6-minute one had a workflow running, and read 6b,
  truthfully). Nothing the registry
  or the transcript carries tells these apart at the time. Row 6c claims nothing — `Working`,
  no counts — and keeps Stop off, since an interrupt could discard background results. The
  price: such a continuation, runaway or not, cannot be stopped from a phone or the web
  until its next line reopens the turn (the editor's own Stop still works). On a CLI without
  a status (regime B) it reads Idle, `Failed` after an API error, for as long.
- Row 6b needs positive evidence of background work. The residuals come from an agent
  launched before the 2 MiB window, which is not on the ledger: for the 3 s after its
  answered prompt, row 4 takes it for a landing prompt; and once it has written nothing for
  120 s, it reads by its stamp, 6a or 6c.

Legacy vocabulary (never `thinking`, `generating`, `background`, `closed` or `waiting` in a
legacy field; installed phones render an unknown state gray and drop Stop):

| Kind | list `state` | stream `state` | `active` | `tone` |
|---|---|---|---|---|
| Closed | idle | idle | false | closed |
| Idle | idle | idle | false | idle |
| Generating | working | working | true | busy |
| Tool | working | tool | true | busy |
| Waiting | ask | ask | true | attention |
| Background | working | working | true | busy |
| Failed | failed | failed | false | error |
| Stalled | stalled | working | true | warn |

## How the router applies it

- **One cached view** (`live::tracked`): per transcript, the inode, the byte offset, a
  partial-line carry, the fold and the head (cwd, branch, first prompt, start time, read
  once). The first look folds the last 2 MiB; after that only appended bytes. A file that
  shrank or was replaced, or grew by more than 8 MiB unseen, starts over. `session_activity`
  gathers the evidence (the registry snapshot, the fold, the wrapper probe, the W prompts, A
  when the rules need it) and calls `derive`. Nothing else derives state.
- **The list** (`GET /api/live/sessions`): every open session however old, then the most
  recently active of the rest up to 60, by `last_activity_ms` = the newest content line or
  subagent write (then the start time, then mtime — always an integer). `active`, `state`,
  `wrapped` and `archived` keep their names and types; `activity`, `open`, `detail`,
  `since_ms`, `ended_ms`, `ended_how`, `mid_turn`, `waiting_on`, `background`, `tone`,
  `source` and `as_of_ms` are new and omitted when empty.
- **The stream** (`GET /api/live/stream`): items exactly as before; a `status` event when the
  state, detail, activity, queue, openness, waited-on request or background counts change,
  and once a second otherwise. `quiet_ms` is the time in the current activity.
- **Prompts** (`/api/live/prompt`, `/api/live/answer`): the newest counted W prompt plus how
  many there are; an answer names its `request_id` (409 if it is no longer waiting), and a
  client that names none is answered only when exactly one prompt is open. A question
  takes only answers to itself: at least one, each keyed by one of its own
  `input.questions[*].question`, else 409 `That question is no longer waiting`. The tool
  looks each of its questions up in `answers` and drops the rest, so a phone's form for a
  question answered in the editor since, sent under the id of the question that took its
  place, would settle that one as "The user did not answer the questions." while the phone
  said "Answered." A refusal (`allow: false`) carries no answers and is not checked.
- **Sending** (`/api/live/send`, and the answers): a frame longer than the wrapper's 1 MiB
  line is refused with 413 and says so, before anything connects.
- **Stop** (`/api/live/interrupt`): refused (409) for a closed session and for Background
  (rows 6b and 6c); allowed for the main loop working with no turn open (row 6a).
- **Notifications**: `req:<request_id>` for a W prompt, `ask:<tool_use_id>` for a question,
  plan or call the transcript shows, `ask:wait-<session>-<R.t>` when only the registry
  knows. Installed phones reconcile only `req:`/`ask:` keys. A key is announced after two
  seconds of waiting without a break; a `req:` key must be missing from two sweeps before
  its banner comes down; a closed session's banners come down.

Cost, measured against 24 listed sessions (38.8 MB of transcript windows): the first list
call after a start folds every window — 0.11 s on a release build, 0.71 s on a debug one —
and after that a call is `stat`s and the appended bytes, 1.6–4.7 ms on release (debug: median
2.8 ms, max 34 ms over 60 calls with two streams open).

## Client presentation policy

The router decides what a session is doing; how it looks is the clients' — the web page,
the iPhone and Android apps. They drifted apart once already, so this is the one policy all
three follow. The web page's behaviour is the reference, with three exceptions where the
phones were right and the web console that replaced the page (2026-10-05) follows them:
numbered Markdown lists are numbered, 1., 2., 3. and again from 1 after a blank line; a
hosted router's sessions are headed per machine, each with its own projects, older fold and
shelf; and model names and bucket sizes are spelled the phones' way. The cases all three
must agree on are pinned in `fixtures/live/`.

- **Row line** under every session: the router's `detail` plus the time elapsed since
  `since_ms`, measured against `as_of_ms` — never the device's clock — for every activity,
  idle included (`Waiting for you · 1h`, `Waiting for you · background shell running · 10h`).
  A closed row reads `Session ended · 3h ago`, from `ended_ms` against `as_of_ms`. A failed
  row carries its elapsed time too. With an older router (no `activity` key), rows read
  exactly as they did before.
- **Project heading dot**, one ranking on all three: attention (any session waiting) >
  error (any failed) > warn (any stalled) > busy (any generating, tool or background) >
  idle (any open and idle) > a hollow gray ring, only when every session in the project is
  closed. With an older router, each client keeps its previous behaviour.
- **Stream silence**: a client that has heard nothing on the stream — no item, no status —
  for 10 s marks the header stale (`reconnecting…`), hides Stop, and reconnects. The feed
  is cleared when the new connection delivers its first event, just before the backfill
  lands, so the rows stay readable during an outage and are never appended twice. The same 10 s on all three; the
  router sends a status at least once a second, so silence is a reliable signal.
- **The open tab's dot** follows the open session's stream status, not the list row, which
  can be 5 s old.
- **Header counts**: `N working · M waiting`, where working is generating, tool, background
  or stalled, and waiting is waiting; `N active` only when no row carries `activity`.
- **Palette**, the same on all three (the phones' colours, chosen 2026-09-29): `busy`
  green, `attention` orange, `warn` yellow/amber, `error` red, `idle` gray, `closed` a
  hollow gray ring. On the web these are the `--st-*` variables; the page's accent colours
  (links, tabs, the question card) are separate and unchanged.

## Validation

- Corpus replay (155 transcripts, whole file and last 2 MiB): no transcript ends with a
  message counted as queued (the old rule left 13), none with calls pending; the only
  window/whole-file differences are unreported background ids launched outside the window.
  The reopen rule for a closing message's later call, and the queue reset at a resume,
  change no transcript's end state — they act mid-file, where the replay of 0b2e2351 and
  31044fb2 shows them (`fold` tests carry both shapes).
- Busy after the turn, observed live against CLI 2.1.284 in an isolated config, under the
  single row 6 this replaced: a 12 s Stop hook read Idle for about 2 s, then `Background
  work running` with zero counts and Stop refused, until the registry went idle; a manual
  `/compact` read the same from 3 s in until its boundary landed at 16.7 s, dated from the
  reply before it. Stop would have worked there: an interrupt written to the CLI 5 s into
  the compaction aborted it within 2 ms and discarded nothing. Today the hook's shape reads
  row 6c (`Working`, no counts, no Stop; a unit test), and the recorded `/compact` timeline,
  replayed through the derivation, reads row 6a (`Working` from the busy stamp, stoppable).
- An API error read `Waiting for you` for up to 404 ms before `failed` while the registry
  snapshot still held the turn's busy (1 of 3 live runs); replayed, it reads `failed` at
  once.
- Against a truth model built from write timing: of 22,424 truly generating minutes the
  derivation labels 100.0 % busy, where `tail_state` labelled 0.1 %.
- Live, on this Mac (a test router on another port, sampled every 2 s for two minutes against
  `ps`-verified registry records): 1,440 rows, 0 mismatches. The orchestrator running a
  background workflow read `background` (legacy `working`, `active` true) in 60 of 60
  samples; four open idle sessions read idle; 19 closed sessions read closed (legacy `idle`,
  `active` false, `wrapped` false). The open view's status arrived at 1.00 per second.

## Unverified

- The registry's `waiting` state and its `waitingFor` strings were never observed live; their
  timing comes from the CLI's code (synchronous with the request and its answer).
- Which CLI version between 2.1.268 and 2.1.278 introduced the headless status writer.
- Whether an older idle gate ignores background agents (only 2.1.284 was observed holding
  busy for them); the Background class looks for subagent writes either way.
- What an interrupt does to running background agents and workflows.
- Whether a message that calls a tool always goes on whatever its `stop_reason`: the one
  closing message with a call in 28 days did.
- How often busy is held past a turn's end on current CLIs: every continuation case in the
  corpus ran on 2.1.247–2.1.270, which wrote no status, so row 6c's timing on 2.1.278 and
  later rests on the probes above.
- The day padding `procStart` uses for days 1–9 (compared as parsed seconds, so either works).
- Whether a future storage backend (flag `tengu_hover_rest`) keeps `<pid>.json` where it is;
  an unreadable directory falls back to regime C.

## Sources

Claude Code 2.1.284 (`claude.real`), by byte offset: registry directory and file patterns
@178976841; record reader @184079992; status enum and kinds @178765414; headless status
projection `rP` @203567711; busy on loop work @203658286; status write stamping
`statusUpdatedAt` @180191676; idle gate `Bs`/`Ia`/`Cf` @203333969 and @203651789; TUI writer
@206637808 and @206644503; registration `ZD` @180187004; update `Xt` @180189660;
conversation switch `Ph` @177192704; exit sweep `Wft` @180194119; `procStart` via `ps` with
`TZ=UTC LC_ALL=C` @178753426. VS Code extension 2.1.284 `extension.js` (session state env,
close sequence) and `webview/index.js` (the editor's own running/waiting logic). The router's
own code: `src/registry.rs`, `src/fold.rs`, `src/activity.rs`, `src/live.rs`,
`src/notify.rs`, `src/peer.rs`, `wrapper/src/main.rs`.
