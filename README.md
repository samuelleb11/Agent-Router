# Agent Router

A local proxy that sits between Claude Code and Anthropic, so you can

* **keep several Claude accounts separate on one machine** and pick which one is
  serving, without `/login` and without restarting your agent; and
* **see where your tokens actually go** — per project, per session, per model, per
  tool, and what filled the context window.

Everything is local: one Rust binary, one SQLite file, no cloud, no telemetry, and a
socket that only ever listens on loopback.

> **Status:** Phases 1–2 are done — the proxy captures and attributes real traffic.
> Multi-account switching (Phase 3) and the web UI (Phase 4) are not built yet, so
> today it routes through the account Claude Code is already logged into and you read
> the data with SQL. See [ARCHITECTURE.md](ARCHITECTURE.md).

## Use it

**1. Build.**

```sh
cargo build --release
```

**2. Run it.** Leave it running in a terminal, or background it.

```sh
./target/release/agent-router
```

It prints where it is listening, which credential it picked up and when that expires:

```
agent-router listening on http://127.0.0.1:8787 (loopback only)
  upstream:     https://api.anthropic.com
  credential:   sk-ant-oat01 (subscription OAuth)
  subscription: max
  expires:      in 75 min
capture started db=/Users/you/.agent-router/agent-router.db
```

**3. Point a project at it.** Create `.claude/settings.local.json` in the project:

```json
{
  "env": {
    "ANTHROPIC_BASE_URL": "http://127.0.0.1:8787"
  }
}
```

Add that file to the project's `.gitignore` — it is machine-local. The same three
lines work in every project; there is nothing per-project to configure. Both the CLI
and the VSCode extension read it.

**4. Use Claude Code normally.** Nothing else changes. Delete the settings file and
the project goes straight back to talking to Anthropic directly.

**5. Read the data.**

```sh
sqlite3 ~/.agent-router/agent-router.db < queries.sql
```

[`queries.sql`](queries.sql) answers the usual questions: tokens per project, tokens
per session, cache hit rate, which tools produce the most output, what filled the
context window, and how much of your 5-hour and weekly quota is gone.

## What it records

Per request: the session and project it belongs to, the model, latency, status,
exact token usage (input, output, thinking, cache reads, and cache writes split by
5-minute and 1-hour tier), the tools the model called, the tool results in its
context, quota headroom, and a capped copy of both bodies with their true sizes.

Session and project attribution are exact, not inferred: Claude Code labels every
request with `x-claude-code-session-id`, and that id names its own transcript file
under `~/.claude/projects/<cwd-slug>/`, whose contents give the working directory
and git branch.

### One number worth looking at first

On a normal Claude Code turn, **tool definitions are around three quarters of the
context window** — far more than the conversation. Run the "where the context
actually goes" query and see what your own split looks like.

## Configuration

| Variable | Default | Purpose |
| --- | --- | --- |
| `AGENT_ROUTER_PORT` | `8787` | Port to listen on |
| `AGENT_ROUTER_DB` | `~/.agent-router/agent-router.db` | Store location |
| `AGENT_ROUTER_BODY_CAP` | `65536` | Bytes of each body kept; true size is always recorded |
| `AGENT_ROUTER_UPSTREAM` | `https://api.anthropic.com` | Upstream API |
| `AGENT_ROUTER_ACCESS_TOKEN` | *(unset)* | Use this token instead of the Keychain one |
| `AGENT_ROUTER_LOG` | `agent_router=info` | `tracing` filter |
| `AGENT_ROUTER_DUMP_HEADERS` | `1` | Requests to dump full headers for, at startup |

There is deliberately **no setting for the bind address**. See below.

## Security

Agent Router holds a live subscription credential and signs everything it forwards,
so reaching it is kept as hard as it can be without a login of its own:

1. The socket binds to `127.0.0.1` only, and no flag or variable can widen that.
2. The connecting peer must itself be on loopback.
3. The `Host` header must parse as a loopback address. This is what defeats DNS
   rebinding — a web page that resolves its own domain to `127.0.0.1` so your browser
   will send credentialed requests here. Layers 1 and 2 both *pass* in that attack.
4. Only `/v1/` paths are proxied, and only for callers that already presented a
   credential of their own, so nothing can borrow the token by simply finding the port.

Tokens live in the macOS Keychain and are never written to the database or to config.
Inbound `authorization` and `x-api-key` headers are discarded, never stored. Captured
bodies may contain your source code — the database is local and gitignored.

## Current limitations

* **One account.** It borrows whichever account Claude Code is logged into, re-reading
  the Keychain as the token nears expiry. Router-owned accounts and the switch are
  Phase 3.
* **No UI.** Phase 4. Use `queries.sql` in the meantime.
* **No cost figures.** `usage.cost` is nullable and unset — there is no pricing table
  yet, and on a subscription the quota utilization the router already records is the
  more meaningful number.
* **Context composition is an estimate.** Totals are exact; the split across system /
  tools / history / tool results apportions that exact total by character share,
  because there is no local tokenizer for current Claude models.
