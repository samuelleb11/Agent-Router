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
| Project | Session UUID → `~/.claude/projects/<cwd-slug>/<uuid>.jsonl` | A filename lookup resolves session to working directory. No content parsing |
| Control surface | **Web UI only** | Accounts, login, the active-account switch and config are managed in the browser. No CLI beyond `agent-router serve` |
| Credentials | Claude Code's own login flows; tokens in the macOS Keychain | Measured: `claude setup-token` and `claude auth login` emit nothing headlessly, even given a pty — they need a real terminal. So the router orchestrates around that one command rather than reimplementing an OAuth client against Anthropic's endpoints |
| Account switch | Cached credential, dropped on switch | Takes effect on the next request, mid-session, with nothing restarted |
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

**Project** comes from one filename lookup: the session UUID names a file under
`~/.claude/projects/<cwd-slug>/<uuid>.jsonl`, and its directory is the working directory.
Resolved once per session and cached.

Subagent traffic is separable from the wire too: Claude Code stamps `x-claude-code-agent-id`
on Task-tool requests and omits it on the main thread. Measured on a single ordinary turn, a
subagent accounted for **31% of the run's prompt tokens**, so this is not a rounding error.

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
|  api/       read endpoints + the embedded UI   |
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

A single SQLite file. The rule is **keep the structure, cap the content**: every message, block
type, tool name, byte count and token count is retained forever, while large text bodies (tool
results, file contents) are truncated to a configurable cap with the true size recorded.

```
accounts   (id, label, subject, expires_at, active, created_at)   -- tokens live in Keychain
projects   (cwd, label, first_seen_at)                            -- discovered, not configured
sessions   (id, cwd, git_branch, started_at, last_seen_at, label)
requests   (id, request_id, message_id, session_id, account_id, model, path,
            started_at, ended_at, status, latency_ms, streaming, stop_reason,
            is_sidechain, service_tier, error, rate_limit_remaining)
usage      (request_id, input_tokens, output_tokens, thinking_tokens,
            cache_read_tokens, cache_creation_5m_tokens, cache_creation_1h_tokens, cost)
context    (request_id, component, char_count, est_tokens)        -- system | tools | history | tool_results
tool_calls (id, request_id, ordinal, name, input_chars,
            result_chars_true, result_chars_stored, est_tokens)
bodies     (request_id, direction, content, truncated_at, true_bytes)
```

`requests.session_id` is nullable: a request whose transcript does not exist yet lands
unattributed and is resolved on a later request in the same session.

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

**Test project.** `/path/to/test-project` is the designated guinea pig. It has no
`.claude/` directory yet; the settings file goes in during Phase 1, along with a
`.claude/settings.local.json` entry in its `.gitignore`.

## Security

- Binds to `127.0.0.1` only. No telemetry, no cloud dependency.
- OAuth tokens live in the macOS Keychain, never in the SQLite file or in config.
- Inbound `Authorization` / `x-api-key` headers are discarded, never persisted.
- The credential is attached only to `/v1/` requests that already presented one of their own.
  Anything else is refused before it reaches Anthropic.
- The transcript watcher is strictly read-only; it never writes to Claude Code's files.
- Captured bodies may contain source code. The database is local and gitignored.

## Build order

| Phase | Deliverable |
| --- | --- |
| 1 | ✅ **Done.** Proxy listener, credential swap, byte-faithful streaming, path/credential guards. Verified end to end from `test-project`: subscription OAuth works over a custom base URL, and `x-claude-code-session-id` gives exact session attribution for free. |
| 2 | ✅ **Done.** Capture tee, SQLite schema, SSE usage parsing through gzip, context composition, tool attribution, quota headroom, session→project resolution. Verified against a live tool-using session. |
| 3 | ✅ **Done.** Account registry with three sources, Keychain storage, live switching, and the accounts page. Verified by switching to a deliberately invalid credential mid-flight (200 → 401 → 200 with nothing restarted). |
| 4 | ✅ **Done.** Embedded read-only dashboard: quota, projects, sessions, context composition, thread split, tools, requests. A single HTML file compiled into the binary — no build step and no node toolchain, which keeps the "one static binary" promise intact. Account management joins it in Phase 3. |

Each phase after the first is useful on its own.
