# Agent Router — Architecture

Agent Router is a local proxy that sits between Claude Code and Anthropic. It exists to do
two things well:

1. **Separate accounts on one machine.** Several Claude subscriptions live side by side; you
   pick the active one in a web UI and the router attaches its credentials. Claude Code never
   holds a token and never needs `/login` again.
2. **Show where the tokens go.** Every request is attributed — per project, per session, per
   model, per tool — and kept in a single SQLite file you can mine later.

It is **not** a load balancer for evading rate limits, not an optimiser, and not a research
profiler. It never rewrites a request's semantics; the only thing it changes on the wire is
the `Authorization` header.

## Locked decisions

| Area | Decision | Why |
| --- | --- | --- |
| Language | Rust, single static binary | No runtime, no infra, one artefact to run |
| Server | `axum` + `hyper` on one port | Proxy, read API, OAuth callback and UI all co-hosted |
| Interception | **One endpoint**, `http://127.0.0.1:8787` | Every project points at the same URL. No path prefixes, no per-project routing, no CA cert, no MITM |
| Opt-in | Per project, via `.claude/settings.local.json` | A project routes through the router only if it carries the file. Nothing global in `~/.claude` |
| Identity | **`x-claude-code-session-id` request header** | Claude Code labels every request with its session UUID. Exact, synchronous, no inference and no CLI wrapper |
| Project | The repository the session's directory is a checkout of, else the directory | One repository is one project on every machine and in every worktree: its `origin` remote, spelt as Northstar's ADR 0028 spells it (`src/repository.rs`), read by the machine itself (`src/places.rs`) |
| Control surface | **Web UI only** | Accounts, login, the active-account switch and config are managed in the browser. No CLI beyond `agent-router serve` |
| Credentials | Claude Code's own login flows; tokens in the macOS Keychain | Measured: `claude setup-token` and `claude auth login` emit nothing headlessly, even given a pty — they need a real terminal. So the router orchestrates around that one command rather than reimplementing an OAuth client against Anthropic's endpoints |
| Account switch | Cached credential, dropped on switch | Takes effect on the next request, mid-session, with nothing restarted |
| Restarts | `SO_REUSEPORT` + pid handover + drain | An upgrade never refuses a connection or cuts a stream |
| Storage | One SQLite file (WAL), structure kept in full, content capped | Every aggregate stays derivable without gigabytes of file dumps on disk |
| Cost/pricing | Config table, never hardcoded | Prices change |

## Two sources of truth

**The wire** (what the proxy sees) carries more than expected. Verified against live traffic
from Claude Code 2.1.266:

```
x-claude-code-session-id: 92807c6a-c3fd-46d6-a9a7-860f20d727c9
x-claude-code-agent-id:   a9f0d533f6…            (subagent traffic only; absent on the main thread)
authorization:            sk-ant-oat01-…            (subscription OAuth)
anthropic-beta:           oauth-2025-04-20,interleaved-thinking-2025-05-14,…
user-agent:               claude-cli/2.1.266 (external, claude-vscode, agent-sdk/0.3.267)
x-app:                    cli
→ response headers
   request-id:                                 req_011Cetg5MiwC75RoJRMWEZa9
   content-encoding:                           gzip
   anthropic-ratelimit-unified-status:         allowed
   anthropic-ratelimit-unified-5h-utilization: 0.04     5h-reset: 1789005000
   anthropic-ratelimit-unified-7d-utilization: 0.71     7d-reset: 1789333200
```

Two consequences. Responses arrive **gzipped**, so capture decompresses a copy while the
original is relayed downstream untouched. And the rate-limit headers give per-account quota
utilization for free — on a subscription that is the scarce resource actually being spent, and
it matters more than a synthetic dollar figure, which is why `usage.cost` stays nullable until
a pricing table exists.

So the proxy alone knows: the session, the account it served with, latency, status,
rate-limit headers, errors, the entrypoint (VSCode vs CLI), and the exact usage in the SSE
stream. Session attribution needs no correlation at all.

**Project** is the repository a session works in, else its working directory
(`sessions.project`). The working directory comes from the transcript where there is one
(the session UUID names `~/.claude/projects/<cwd-slug>/<uuid>.jsonl`), else from the
environment the request itself carries. The repository is read on the machine the checkout
is on: every minute a machine reports the sessions it lists, their directories and each
directory's git remotes (`src/places.rs`; on a hosted router the machine agent posts it to
`/api/machines/places`). A repository is spelt one way, as Northstar's ADR 0028 spells it:
the host lowercased, then the path, without scheme, user, port, `.git` or trailing slashes
(`src/repository.rs`). A clone on one machine, a clone on another and the worktrees beside
either are then one project.

Subagent traffic is separable from the wire too: Claude Code stamps `x-claude-code-agent-id`
on Task-tool requests and omits it on the main thread. Measured on a single ordinary turn, a
subagent accounted for **31% of the run's prompt tokens**, so this is not a rounding error.

### Quota is forecast against hours of work, not hours on the clock

The rate-limit headers hand us the reset time, so the question worth asking is not *when does
the window reset* but *does the way you are working get you to 100% first*. Answering it needs
a burn rate, and the obvious one — utilisation against wall-clock time — is wrong in a way the
store itself demonstrates.

In the 7-day window that reset on 2026-09-11, one account sat between `u=0.97` and
`u=0.98` for **31.4 hours with no requests at all**. That is sleep, not slow burn. Fitted
against the clock, that window predicted exhaustion 15–33 hours early at every point before
the gap, then flipped to "idle, never" once the gap was inside the lookback — a wall-clock
rate is really measuring what fraction of the lookback you happened to be awake.

Summing only the gaps that look like work (five minutes or less — Claude Code's turns are
seconds apart) fixes it. The same window, refitted at eight points across 34 hours, held a
steady 1.17%/active-hour and landed within ~45 minutes of work of the truth:

```
forecast at    used   %/active-h   work left   actual   err
09-09 23:47    0.95        2.15%       2.33h    5.51h  -3.18h
09-10 05:47    0.97        1.17%       2.55h    1.80h  +0.75h
09-10 17:47    0.97        1.17%       2.55h    1.80h  +0.75h
09-11 09:47    0.97        1.17%       2.55h    1.80h  +0.75h
```

So the unit is **hours of work left**, and a calendar date is derived from it through a duty
cycle — which is measured over a horizon matched to the window, because the two are nothing
alike. A 5-hour window is forecast against the session you are in (71–100% duty, measured);
a 7-day window against the habit, sleep included (2.5–8.2 active hours a day here). Reading a
week's duty off the session inside it projected four days of solid work nobody was going to do.

Burn is a measurement; duty is a forecast, and holding one number for it constant from now to
the reset extrapolates whatever you happened to be doing this week. On 2026-09-14 that put the
7-day window at 49% used, 1.68%/active-hour and 32.8% duty — a crossing 2.2 hours before a
reset 94.7 hours away, rendered in red, from a record whose complete days ran 2.1 to 11.6 hours
of work. A 2.4% change in duty flipped the verdict.

So duty is a band, not a number: the trailing week's rate, shrunk toward the mean of every
complete day on record by how many days each speaks for, with the standard deviation of those
days either side — divided by the square root of the days left, because the forecast averages
over them and the band should tighten as the reset approaches. The projection runs three times,
and the verdict is only `crosses` when even a quiet week exhausts the window; when the band
straddles the reset it says `tight` rather than picking a side of a coin flip, and only
`crosses` is red. With less than a week of history there is nothing older to shrink toward and
the central estimate is the trailing rate to the digit; the shrinkage starts to bite as history
accumulates. A 5-hour window's band has no width — that duty is a measurement of the session
you are in, and it must not learn to hedge because the weekly one did.

Utilisation arrives rounded to whole percent, so a window younger than two steps has no slope
worth trusting; it borrows the account's own recent average and `basis` says so. The rate also
depends on the model — Opus eats a window far faster than Haiku — and is not weighted for it.
Per `Token estimates are estimates`, the UI discloses what each projection was fitted on.

Quota is per account, so the cross-account rollup adds spend, tokens and requests but never
percentages; combined headroom is totalled in hours of work, and two credentials onto one
Anthropic account (same 7-day reset) count once. It is a reporting figure — the router still
does not route around a spent limit.

**The transcript** is therefore *enrichment only*, and nothing load-bearing depends on it. It
remains the only source for the reconstructed message tree and per-message branch history, and
it is joined on `requestId`, which equals the `request-id` response header the proxy records.

## Component overview

```
Claude Code (VSCode ext / CLI)
  |  ANTHROPIC_BASE_URL=http://127.0.0.1:8787
  v
+------------------------------------------------+
| agent-router (one binary, one port)            |
|                                                |
|  proxy/     listener, auth swap, byte-faithful |
|             streaming + async tee              |
|  auth/      OAuth PKCE, refresh, Keychain      |
|  accounts/  registry, active selection         |
|  capture/   parse SSE usage -> SQLite          |
|  transcript/ fs watcher over ~/.claude/projects|
|  join/      request-id correlation + backfill  |
|  api/       read endpoints                     |
|  console/   the web console (ui/), embedded    |
|  guard/     loopback, peer and Host checks     |
+------------------------------------------------+
  |  Authorization: Bearer <active account token>
  v
api.anthropic.com
```

## Request lifecycle

1. **Ingress.** A request arrives at `/v1/messages`. The path is forwarded verbatim; there is
   nothing to strip. Requests outside `/v1/` are refused locally, as are requests that present
   no credential of their own — the router must never lend its token to a stray browser tab
   that finds the port.
2. **Auth swap.** The router discards whatever `Authorization` / `x-api-key` header arrived and
   attaches the active account's access token, refreshing first if it is near expiry.
3. **Passthrough.** Forwarded upstream over HTTPS byte-for-byte. The response — including the
   SSE stream — is piped straight back to the agent **unbuffered**. Nothing on the hot path
   waits on capture, parsing or disk.
4. **Tee (async).** Request and response bytes are cloned into a bounded `tokio::mpsc` channel
   consumed by a capture task. If the channel fills, capture drops events; it never applies
   backpressure to the agent.
5. **Capture.** The task records the session id, the response's `request-id`, the account used,
   latency, status, rate-limit headers, and the usage totals from the SSE `message_start` /
   `message_delta` events. Project is resolved from the session id by filename lookup.
6. **Enrich.** A watcher tails `~/.claude/projects/**/*.jsonl` and joins `requestId` to the
   captured row, adding git branch, the sidechain flag and the message/tool structure.

## Account switching

One account is active at a time, chosen in the UI. Because the credential is attached at proxy
time rather than held by the agent, switching takes effect on the **next request** — mid-session,
with no restart of Claude Code and no `/login`.

**Refresh-token ownership.** OAuth access tokens are short-lived and Anthropic rotates the
refresh token on every use. Once the router owns an account it must be the sole refresher: if
Claude Code refreshes the same account independently, whichever refreshes second invalidates
the other. Routed accounts are therefore managed only in the router's UI.

## Storage model

Two SQLite files. The rule is **keep the structure, cap the content**: every message, block
type, tool name, byte count and token count is retained forever, while large text bodies (tool
results, file contents) are truncated to a configurable cap with the true size recorded.

The structure lives in `agent-router.db` and is what every query reads. The capped bodies
live beside it in `agent-router-bodies.db`, attached as `bodies_db`. They were split
because the bodies had grown to 95% of a 1.87 GB store while nothing read them back, which
made every check-point, backup and vacuum pay for data no query touched; the structure on
its own is ~17 MB. Both files travel together — a copy or a backup that takes only the
first silently leaves the bodies behind, and a foreign key cannot span the two, so the
cascade from `requests` no longer reaches them.

```
accounts   (id, label, subject, expires_at, active, created_at)   -- tokens live in Keychain
projects   (cwd, label, first_seen_at)                            -- discovered, not configured
sessions   (id, cwd, git_branch, started_at, last_seen_at, label,
            machine, repository, project = coalesce(repository, cwd))
directories (machine, cwd, remotes, repository, changed_at)       -- reported by each machine
machines   (id, label, platform, last_seen_at)
requests   (id, request_id, message_id, session_id, account_id, model, path,
            started_at, ended_at, status, latency_ms, streaming, stop_reason,
            is_sidechain, service_tier, error, rate_limit_remaining)
usage      (request_id, input_tokens, output_tokens, thinking_tokens,
            cache_read_tokens, cache_creation_5m_tokens, cache_creation_1h_tokens, cost)
context    (request_id, component, char_count, est_tokens)        -- system | tools | history | tool_results
tool_calls (id, request_id, ordinal, name, input_chars,
            result_chars_true, result_chars_stored, est_tokens)
bodies     (request_id, direction, content, truncated_at, true_bytes)  -- in bodies_db
blocks     (id, hash, kind, chars, text)                          -- in bodies_db; the search corpus
block_sites(block_id, session_id, ordinal, role, request_id, at)  -- in bodies_db; where each copy sat
blocks_fts (text)                                                 -- in bodies_db; FTS5 over blocks
model_rate (model, key)                                           -- wire name -> pricing key
```

`requests.session_id` is nullable: a request whose transcript does not exist yet lands
unattributed and is resolved on a later request in the same session.

## Search

`GET /api/search?q=…` finds every session and message whose text matches, over a corpus
that is extracted rather than captured.

The captured bodies cannot serve this. They are cut at `body_cap`, and because `messages`
is the first field of the request JSON that cap keeps the *oldest* part of a conversation
and drops the newest — on a real store, 98.7% of request bodies were truncated, so the
recent turns of every long session were never stored at all. So the corpus is taken in
`analyze_request`, from the full body, in the same parse that already measures the context
split: one parse, two outputs.

What makes indexing the uncapped text affordable is that it is mostly the same text.
Every request replays the whole conversation, so a session's requests restate each other
end to end: measured over one store, 51.7 GB of request bytes crossed the wire carrying
about 0.2 GB of distinct content. `blocks` therefore holds each piece of text once, keyed
by a hash of its kind and content, and `block_sites` — cheap rows of integers — carries
the repetition. The index costs less than the truncated copies already do.

`blocks_fts` is an external-content FTS5 index: it stores the terms and reads the text back
out of `blocks`, so the corpus is never held twice. Blocks are insert-only, so a single
`AFTER INSERT` trigger is the whole of the synchronisation. The tokenizer keeps `_` inside
a token, so `body_cap` survives as one term while `analyze.rs` is found by either half.

Typed text is offered to FTS5 verbatim, so `"exact phrase"`, `term*`, `NEAR(a b)` and
`a OR b` work; anything FTS5 rejects — `don't`, a bare `AND`, an unclosed bracket — is
retried once as a plain AND of quoted words, which is always valid, so a search box never
answers a stray apostrophe with an error.

**Why `tool_calls` is keyed by `tool_use_id`.** Every request in a session replays the entire
conversation, so the same tool result appears in request N, N+1, N+2 and so on. Keying the
table by the request would count one 200 KB file read once per subsequent turn and report tool
costs several times larger than reality. `tool_use_id` is stable and unique, so the result is
recorded once, on the request that first carried it. `tool_invocations` is the separate,
per-request record of calls the assistant actually *made* in that response.

### Token estimates are estimates

Totals in `usage` are exact — reported by the API. The per-component split in `context` is not:
there is no local tokenizer for current Claude models, so the exact total is apportioned by
character share. Ratios are reliable; per-component absolutes are approximate, and the UI must
label them as such.

## Project onboarding

A project opts in by carrying a gitignored `.claude/settings.local.json`:

```json
{ "env": { "ANTHROPIC_BASE_URL": "http://127.0.0.1:8787" } }
```

That is the whole configuration — the same three lines in every project. Deleting the file puts
the project straight back to talking to Anthropic directly.

**Test project.** A small project elsewhere on disk is the designated guinea pig. It has no
`.claude/` directory yet; the settings file goes in during Phase 1, along with a
`.claude/settings.local.json` entry in its `.gitignore`.

## Restarting without dropping traffic

Upgrading the router must not break sessions that are talking through it. Three pieces,
in this order:

1. **`SO_REUSEPORT`.** The incoming binary binds the port while the outgoing one is
   still serving it, so the port is never unbound and no `connect()` is ever refused.
2. **Handover.** The incoming binary reads the incumbent's pid from `/_router/health`
   and sends it `SIGTERM`. A pid served by the live process beats a pidfile, which can
   outlive whatever wrote it.
3. **Drain.** On `SIGTERM` the outgoing binary stops accepting, lets its open requests
   and SSE streams finish, waits for the capture queue to flush, then exits.

**Ordering is the whole trick.** The replacement must reach its accept loop *before*
signalling the incumbent. `SO_REUSEPORT` makes the kernel spread new connections across
every socket bound to the port, so a process that has joined the group but is not yet
accepting silently collects connections nobody reads. Signalling first and serving
second — the obvious order — produced timeouts on ~6% of connections during a handover.
Reversed, a probe every 20 ms across a live handover saw 575 successes and 0 failures,
while a 58-second streaming response begun on the old instance completed intact and was
recorded in full by it on the way out.

## Telling the phone

Whether a session is waiting on a person is the same answer the list gives
(`live::session_activity`, see "What each session is doing" below), asked once a second of
every open session. What it waits on decides the key: `req:<request_id>` for a prompt the
wrapper recorded in `/tmp/agent-router-sessions/<id>.pending` (answerable from the phone);
`ask:<tool_use_id>` for a question, plan or call the transcript shows open;
`ask:wait-<session>-<statusUpdatedAt>` when only Claude Code's own registry knows — an
unwrapped session at a permission prompt, answerable only where it runs. The installed apps
take down only `req:`/`ask:` banners, which is why every key wears one of those prefixes.
A key is announced once, after two seconds of waiting without a break (auto mode and hooks
resolve many prompts within one); what is already waiting when the router starts is not
announced, because a restart is not news. A `req:` key has to be missing from two sweeps in
a row before its banner comes down (an older wrapper's file can read empty mid-rewrite), and
a closed session's banners come down. The same loop clears, once a minute, wrapper sockets
nothing listens on that no open session holds. Delivery is
Apple's push service (`apns`): an ES256 provider token minted from the team's `.p8`
(kept in the secrets file beside the account credentials), reused for fifty minutes, over
HTTP/2, to every phone that registered through `POST /api/push/register`. A device Apple
reports gone is forgotten. The payload names the session and the phone's own id for the
router, so a tap lands in the right chat — the store is attached to the registrar before
any view exists and a tap that arrives earlier still is kept until it is, because a cold
launch from the notification delivers the tap before the first view's task runs; and the
notification delegate uses the completion-handler forms, completed on the main queue,
because the `async` forms have their completion invoked from a background executor and
UIKit's state-restoration work inside it asserts main-thread and aborts (a simulator UI
test taps a local notification with the same payload to keep it so). Apple's `.p8`
omits the public half of the key
that `ring` insists on; when that bites, the key is passed once through the system
`openssl` to fill it in.

The key gets into the router two ways. The dashboard and the phone take a paste. Or the
file Apple hands out, `AuthKey_<key id>.p8`, is dropped beside the database: while no
key is stored, `notify` looks for it once a second, reads the key id off the name, takes
the team id from what the app said about itself when it registered (`AppIdentifierPrefix`
from its Info.plist, sent with `POST /api/push/register` along with its bundle id — which
is also where the push topic comes from), proves the key signs, and stores it. The file
is left where it is, made private, since Apple never hands the same key out twice; a file
that fails is not retried until it changes. Nothing is provisioned after install from the
router's side. What remains — a push-enabled App ID and the key itself — is issued only
by Apple's portal to the account holder, and there is no API for the key.

## The editor extension

`vscode/` packages the router as a VS Code extension, one VSIX per platform with the
router and the wrapper in `bin/`. The design follows one fact about VS Code: every
window runs its own extension host and nothing is shared between them, so an extension
that *was* the router would be several routers. Instead the extension is an installer, a
supervisor and a UI for a machine-wide daemon that is alone on its port by construction.

On activation — after start-up, not blocking it — each window asks `/_router/health`.
A same-or-newer version answering means nothing to do. Otherwise the bundled binaries
are copied beside their targets in `~/.agent-router/bin` and renamed over them (never
written in place: the kernel kills a running binary whose file changes), and
`agent-router --ensure` is spawned detached, so it outlives the window. `--ensure` is
`--if-not-running` with a version check: it exits when the incumbent is as new as it
is, and otherwise serves, which is the ordinary `SO_REUSEPORT` handover. Two windows
racing at start-up both try; the loser sees a healthy peer and exits. Launchd (or a
systemd user unit) owns boot-time and crash recovery, registered by `agent-router
install --bind …`, which now carries the listen address into the service so the phone
keeps working after a reboot; the extension's periodic health check covers the gap a
handover leaves, since the instance that took over is not launchd's until next login.

Claude Code is wired through its own settings and nothing else: `ANTHROPIC_BASE_URL`
in its settings file, written by `install`, and `claudeCode.claudeProcessWrapper`
pointing at `claude-wrapper`, which then receives the real binary's path as its first
argument (the wrapper still works as the in-place shim when run under the name
`claude`). That retires the need to modify Anthropic's extension folder, and with it
the one behaviour a Marketplace listing could not describe as expected.

The status bar item reads `/api/quota`, the menubar app's job inside the editor; the
dashboard and live pages open through VS Code's Simple Browser, which defers to the
Integrated Browser where it exists, with a webview frame as the fallback. The
extension never stops the router: closing every window leaves the phone and the
forecast working. Removal is an explicit command that runs `agent-router uninstall`
and clears the wrapper setting.

## Security

- Always binds `127.0.0.1`, and adds a second listener on a wider address only when it is
  widened deliberately — the dashboard switch, which stores the answer, or
  `AGENT_ROUTER_BIND`, which overrides it for one run (`network.rs`). `SO_REUSEPORT` lets
  the two share the port and the kernel steers by most-specific match, so the switch can
  be thrown mid-session without unbinding anything. Widened, every
  caller that is not on this machine must present the router's password (`auth.rs`): as
  HTTP Basic from a browser, or as `Authorization: Bearer <password>` from an agent, which
  is what a Claude Code elsewhere sends when `ANTHROPIC_AUTH_TOKEN` is set to it. With no
  password set, off-host callers are refused outright, so the first one can only be set
  from the machine itself — which is also how a forgotten one is replaced. Loopback is
  never asked: whoever sits here already holds the Keychain. Stored as PBKDF2-HMAC-SHA256
  (600k iterations) in the `settings` table, verified once per credential and remembered
  by digest, one verification at a time with a 400 ms penalty per miss. It travels as
  plain HTTP: the bind already assumes a network you trust. No telemetry, no cloud
  dependency.
- OAuth tokens live in the macOS Keychain, never in the SQLite file or in config,
  and never appear in a process argument: reads use `find-generic-password -w`, which
  writes only to stdout, and writes go through `security -i` with the command supplied
  on stdin.
- The Keychain is driven through `/usr/bin/security` rather than the Security framework,
  because a locally built binary is ad-hoc signed and every build is a different code
  identity to the ACL. The router's own items therefore carry `-T /usr/bin/security`,
  which any process running as the user can also invoke — the same footing as Claude
  Code's credential entry, and reversible by code-signing the binary with a stable
  identity.
- Inbound `Authorization` / `x-api-key` headers are discarded, never persisted.
- The credential is attached only to `/v1/` requests that already presented one of their own.
  Anything else is refused before it reaches Anthropic.
- The transcript watcher is strictly read-only; it never writes to Claude Code's files.
- Captured bodies may contain source code. The database is local and gitignored.
- The web console (`console.rs`) is served under a content security policy that admits this
  origin alone and no inline script, and that only VS Code's own schemes may frame: it approves
  prompts and can switch a session to bypass permissions. Every response carries
  `X-Content-Type-Options: nosniff`.

## Running as a hosted server

`serve --server` is the same binary built for a different premise: it runs in a
container, and every caller is on another machine. The full design, and the map of
everything that assumed otherwise, is in
[docs/hosting-on-kubernetes.md](docs/hosting-on-kubernetes.md).

**Configuration.**
- Configuration comes from flags (`config::ServeArgs`), so a binary too old to know one
  refuses to start rather than ignoring it.
- `config::data_dir()` is the one root for everything the router writes. Before, five
  places each derived `~/.agent-router` on their own.

**Trust (`guard`, `auth`).**
- The Host check also admits the names the server was given.
- Loopback is asked for the password.
- The password is pinned from the deployment at every start (`Auth::pin`), the pattern
  of Northstar's ADR 0022.

**Routes and tasks.**
- The routes that act on the router's own machine are answered by `hosted`, in the
  shapes the installed phone apps decode, instead of being removed.
- The background tasks that read the developer's machine are not started: the shim
  watcher, the wrapper janitor and the prompt sweep.

**Projects.** Attribution falls back to what the request says. Claude Code's environment
block names the working directory and the branch (`analyze::environment_of`). That also
covers sessions on other machines and SDK sessions with no transcript on this one.

**Live sessions through the machine agent (`agent`, `tunnel`, `machines`).** What a pod
cannot do alone is everything under "Reaching a running session" below, so each developer
machine runs the same binary as `agent-router agent`:
- The agent dials out to `/_router/agent` over one WebSocket, with the router password,
  and listens on nothing.
- The router sends each live request down that socket as framed HTTP (`tunnel`). The
  agent answers it with the same `live` handlers a local router serves, in-process, and
  the response comes back the same way, event streams included.
- `machines` merges every machine's session list and labels each row with its machine.
  It routes a request to the machine holding that session, keeps the archive shelf, and
  feeds the prompt notifier with each agent's evidence. A machine that drops reads as
  offline (503), never as closed.
- On Windows only the agent runs. The wrapper's control channel there is a named pipe,
  written to only after the agent has checked that the pipe is served by its own user
  (`peer::pipe`).

The protocol, the decisions and the Windows port are in
[docs/hosting-machine-agent.md](docs/hosting-machine-agent.md).

## Build order

| Phase | Deliverable |
| --- | --- |
| 1 | ✅ **Done.** Proxy listener, credential swap, byte-faithful streaming, path/credential guards. Verified end to end from a real project: subscription OAuth works over a custom base URL, and `x-claude-code-session-id` gives exact session attribution for free. |
| 2 | ✅ **Done.** Capture tee, SQLite schema, SSE usage parsing through gzip, context composition, tool attribution, quota headroom, session→project resolution. Verified against a live tool-using session. |
| 3 | ✅ **Done.** Account registry with three sources, Keychain storage, live switching, and the accounts page. Verified by switching to a deliberately invalid credential mid-flight (200 → 401 → 200 with nothing restarted). |
| 4 | ✅ **Done.** Embedded read-only dashboard: quota, projects, sessions, context composition, thread split, tools, requests. Account management joins it in Phase 3. At first a single HTML file compiled into the binary; since the clean cut of AGEN-0001, the web console: a React and IBM Carbon app in `ui/`, built with pnpm and Vite and compiled in with `rust-embed` (`console.rs`). The "one static binary" promise holds, and the console loads nothing from the network; building from source now needs Node for the console. |

Each phase after the first is useful on its own.

## Reaching a running session

Three ways in, and the difference between them is decided by the operating system,
not by Claude Code.

**Transcripts (read).** `~/.claude/projects/<cwd-slug>/<uuid>.jsonl` is appended live
by every session, including ones the router never proxied and ones the VSCode
extension hosts. `live` tails these. It cannot break on a CLI update, because it reads
a file the CLI has written the same way for its whole life. This is the whole basis of
the live view's *content*, and it is why following works everywhere. What a session is
*doing* needs more than the transcript can say; see "What each session is doing" below.

**The session registry (read).** Every CLI process registers itself in
`~/.claude/sessions/<pid>.json` (under `$CLAUDE_CONFIG_DIR` when set): the conversation it
holds now, and on current builds a `status` of busy, waiting, idle or shell written by the
CLI's own state machine. `registry` reads the `<pid>.json` files only — never the `.key`
files beside them, which hold secrets — and counts a record only while its pid is alive and
is the same process (`proc_pidinfo` start time against the record's `procStart`, read as
UTC). It never writes there.

**The peer socket (write, best-effort).** Every live session binds
`/tmp/cc-socks/<pid>.sock` and publishes a token at
`~/.claude/sessions/<pid>.<sha256(socket path)>.key`. Two newline-delimited JSON
frames — `{"type":"auth","token":…}` then a `{"msgV":1,…,"type":"user",…}` — push a
message into the session's input queue. `peer` implements this.

Measured, it delivers into some sessions and silently not into others, with no
observable difference between them: same extension host process, same launch flags,
same CLI version, same socket ownership, verified from inside the receiving session.
So it is exposed as best-effort and the UI says when it cannot confirm delivery. Do
not build anything load-bearing on it.

**Being the parent (write, reliable).** The VSCode extension does not attach to
sessions; it spawns them:

```
claude --input-format stream-json --output-format stream-json \
       --permission-prompt-tool stdio --include-partial-messages --replay-user-messages
```

and keeps their stdio. Measured: `fd 0` of a running session is an **anonymous unix
socketpair**, not a pipe and not a pty. A socketpair has no name in the filesystem, so
the only way to hold an endpoint is to have been given it at `exec`, or to be passed it
over `SCM_RIGHTS` by someone who already has one. No permission, path or API grants it
afterwards.

That single fact settles the design. Typing into a session — and answering the
`AskUserQuestion` and permission prompts that `--permission-prompt-tool stdio` routes
over that same channel — requires having been there when it was spawned.

### The wrapper

So the router interposes at spawn time instead, by shimming the binary the extension
executes:

```
VSCode extension
   |  spawns, speaks stream-json          (unchanged)
   v
claude-wrapper  ------- control socket -------  Agent Router  ---- phone
   |  spawns the real binary, owns its stdin
   v
claude (real)
```

The wrapper relays both directions byte-for-byte, sniffs only the session id out of the
`init` message, and publishes a control socket named by that session id. The CLI emits
that message at startup, again at once after a `/clear` — naming the new session — and at
the start of every turn, so the wrapper moves its socket when the id changes (the Sep 12
build still deployed inside long-running sessions keeps the first). A write on that
socket becomes the *identical bytes the extension writes when you type* — an ordinary
user turn, not a peer message. Because the wrapper also sees the downstream side of the
permission channel, prompts become answerable remotely too.

One write is one line, and a wrapper takes at most 1 MiB of it, newline included. The
builds deployed inside long-running sessions read in 8 KiB pieces, stop once they hold
more, and write what they have into the CLI anyway — which exits on the unparseable line,
and the session with it; the current wrapper refuses such a frame whole. So `peer`
measures every frame as it would send it and refuses a longer one before connecting; a
message or an answer that long gets a 413 that says so.

A session is reported as `wrapped` only when its control socket *answers*, not when the
file exists: the wrapper unlinks its socket on a clean exit but a killed one leaves it
behind, and a phone showing that session as live is where the mistake costs the most.
The router connects to the socket (the wrapper reads EOF, replies `{"ok":false}`, and
nothing reaches the session) and removes the leftovers of a refused one — at most once a
second per session, and only for sessions a live process holds, since every probe costs
the wrapper a thread; the notifier's janitor asks the rest once a minute. `wrapper_alive`
in `peer.rs` is the one place that rule lives. The janitor also takes what a dead wrapper
leaves beside its socket — the `.ctl` recording, a `.pending.tmp` — and, once ten minutes
old, records whose socket is already gone and whose session no process holds; the
per-session probe never touches `.ctl`, since for an open session a refused connect can be
a full backlog, and the recording is where `set_model` reads its answer.

The same socket changes what the session runs with. `set_model` is a control request
the CLI's own SDK sends (`{subtype:"set_model", model}`); the CLI answers on its control
channel with a success and its notices, or an error, and the wrapper's recording is
where the router reads that answer back, so a refused name is reported rather than
silently ignored. Effort has no control request; `/effort <level>` is a slash command
flagged non-interactive in the CLI, so it goes in as a typed line and is executed rather
than sent to the model. Both take effect on the next turn, which the transcript then
records as `model` and `effort` on the assistant message.

Two costs, both accepted deliberately:

  * It shims a binary inside a version-pinned extension directory, so an extension
    update lands a fresh, unwrapped binary and every session it starts is view-only —
    silently, unless something says so. Setup reports per-directory state (a `.real`
    beside a small `claude` is installed; beside a large one is an interrupted install
    or an update that overwrote us) and reinstalls from `claude-wrapper` beside the
    router binary; the live page and the phone app both warn when the newest extension
    is unwrapped.
  * If the wrapper dies the session dies with it. The relay path must therefore be as
    dumb as `proxy`'s tee: no parse that can throw, no buffer that can stall, and a
    fail-open to plain pass-through when anything unexpected happens.

### Keeping the wrapper installed across extension updates

The first cost above used to be paid by hand: notice the sessions went view-only, open
Setup, click install, reload the editor. `shim::watch` pays it instead when the
server-side switch is on (`settings.auto_wrapper`, `POST /api/setup/shim/auto`;
off until turned on). Every 3 s it looks at each extension directory and installs into
one whose `claude` is a real binary the wrapper is not in — but only once the file has
*settled*: the same size on two consecutive looks and a modification time at least 5 s
old. VS Code writes an update file by file, and wrapping a half-written binary would be
worse than leaving it alone. The install is the same routine the Setup button calls;
each one is noted in a short in-memory log the Setup section shows, and pushed to the
phone. The switch is server-side on purpose: it is the router that does the work, so it
is the router that must be told to stop, from any client, without shipping one.

### What each session is doing

One derivation answers it for every surface — the list, the open view's status line, the
prompt endpoints, the stop guard and the notifier — and nothing else derives state. It
used to be four, each reading the transcript on its own, and they disagreed: a session
generating read `idle` in the list and `Working` in its own view, one running a background
workflow read idle everywhere, and a closed one read exactly like one waiting for you. The
evidence behind every rule, with the numbers, is in
[docs/session-liveness.md](docs/session-liveness.md).

The order of trust is process first, transcript for detail:

  1. **The registry record** (`registry`), verified against the live process, decides
     whether the session is open at all, and its `status` decides the class when it has a
     known one: `busy` covers the silent generation the transcript cannot see and the
     background agents that outlive a turn; `waiting` covers permission prompts the
     transcript never records.
  2. **The wrapper's open prompts**, for wrapped sessions: what exactly is being asked,
     ordered by when the router first saw each, and answerable by request id.
  3. **The transcript** (`fold`, one incremental fold per file): the turn, the call being
     run, the question or plan open, retries, API errors, the background tasks launched
     and not yet reported done, the queue, the exit marker, the title. With no usable
     registry status it decides the class alone.
  4. **Subagent transcripts** under `<session>/subagents/`: the heartbeat of background
     agents, whose parent transcript sits silent after its turn.

`activity::derive` turns that into one of closed, idle, generating, tool, waiting,
background, failed or stalled, with a header line, a start time, and — for old clients —
the legacy `state` and `active`. A file's mtime is never evidence of activity: it moves on
bookkeeping (the exit flush, titles, ledgers, subagents' queue lines) and stands still
through real work, so `last_activity_ms` is the newest *content*. `live::tracked` keeps
each transcript's fold, offset and head in memory, so a list call costs a registry
snapshot (reused for 250 ms), `stat`s, and the bytes appended since the last look.

Three lines the fold has to read carefully. A user-role line reading `[Request interrupted
by user]` (or `… for tool use`) — which is what Stop produces, stored as a plain string or
a text block depending on the CLI's mood — ends the turn outright: the tool it cut short
will never get a result. An assistant message's blocks arrive as separate lines, each
carrying the message's final `stop_reason`, and a message that calls a tool ends nothing
whatever that reason says: so when a later line of a closing message turns out to carry a
call, the close its first line made is undone. And `queue-operation` lines are the CLI's
own bookkeeping of messages typed while a turn ran (`enqueue` / `dequeue` / `remove`);
their count is the status's `queued`, less the task notifications subagents route through
the same queue, and it empties when a crashed session is resumed, as at an exit.
Because the session queues on its own, Send is never replaced by Stop: the two sit side
by side while a turn runs. Stop is offered only while the turn itself runs (generating, a
tool, stalled) — not for background work, where an interrupt could discard what the agents
report back; the server refuses it there for clients too old to know. Busy after a turn
is over is told apart by when the registry went busy: stamped after the end, with nothing
in the background, it is the main loop at work (a `/compact`) and can be stopped; held
from before the end with nothing to see — a Stop hook, a task no transcript records, a
turn the CLI carries on by itself — it reads `Working`, claims no background counts, and
is not.

### Side questions

`/btw` is a `local-jsx` command: interactive only, and through stream-json the CLI
answers "/btw isn't available in this environment". What it does is fork the session
for a side conversation, and the router does that itself
(`live::aside`, `POST /api/live/aside`):

```
claude -p --resume <id> --fork-session --tools "" \
       --append-system-prompt "…answer briefly, no tools…" -- "<question>"
```

The real CLI binary is used (the newest extension's `claude.real`; the shim adds nothing
a fork needs), run from the session's own directory so the fork lands in the same
project, with `--output-format json` for one result object. A fork is a new session id
with the original untouched — the question and answer never enter the session's
transcript, nothing is interrupted, nothing is queued — and the id it leaves behind is
shelved on the spot so the list does not fill with one-question sessions. While it runs
the fork registers like any session, but its parent is the router, so the registry reader
sets it aside: it is never counted as open, and its transcript is left out of the list. The price is a
replay of the session's context through the proxy, mostly served from cache; three
minutes is the ceiling before the router gives up.
