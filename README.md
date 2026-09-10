# Agent Router

A local proxy that sits between Claude Code and Anthropic, so you can

* **keep several Claude accounts separate on one machine** and pick which one is
  serving, without `/login` and without restarting your agent; and
* **see where your tokens actually go** — per project, per session, per model, per
  tool, and what filled the context window.

Everything is local: one Rust binary, one SQLite file, no cloud, no telemetry, and a
socket that only ever listens on loopback.

> **Status:** proxy, capture, dashboard and multi-account switching are all working.
> See [ARCHITECTURE.md](ARCHITECTURE.md).

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

**5. Open the dashboard** at <http://127.0.0.1:8787>.

Quota headroom for the active account with its history, the totals for whatever you
have scoped (prompt and output tokens, cache hit rate, time to first token, errors),
activity over time stacked by model, what the model read from cache versus paid in
full, what filled the context window, the main-thread / subagent split, tokens per
project and per session, which tools flood the context, and every request as an
expandable row. Click a project or a session and the whole page rescopes to it; the
breadcrumb and the time range (1h / 24h / 7d / 30d / all) sit in a sticky bar at the
top and are mirrored in the URL, so a link keeps its scope. It refreshes every five
seconds without flicker and is served from the same binary — no build step, no node
toolchain, no network, and it works down to phone width.

For anything the dashboard does not answer, the store is plain SQLite:

```sh
sqlite3 ~/.agent-router/agent-router.db < queries.sql
```

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

## Accounts

The account Claude Code is already logged into is picked up automatically — nothing to
configure. To add another, open the dashboard and use **+ add an account**:

* **Long-lived token (recommended).** Log into the other account in a terminal and run
  `claude setup-token`; paste what it prints. The token does not expire, so the router
  never has to refresh it and can never collide with a running Claude Code.
* **Separate Claude Code profile.** The router creates an isolated `CLAUDE_CONFIG_DIR`
  and hands you a `claude auth login` command to run in a terminal. It polls for
  completion and fills in the email, org and plan itself. The token is short-lived and
  only Claude Code can renew it, so the dashboard shows its expiry.

Switching is a click, takes effect on the **next request**, and needs no restart of the
router or of Claude Code. Tokens are held in the macOS Keychain, never in the database.

An isolated profile cannot disturb your main login: a fresh `CLAUDE_CONFIG_DIR` reports
"not logged in" even with the default profile's `.claude.json` copied into it.

## Upgrading without breaking sessions

Build the new binary and just start it — no need to stop the old one:

```sh
cargo build --release && ./target/release/agent-router
```

The new instance binds the port alongside the old one, tells it to stand down, and the
old one finishes the requests and streams it is already holding before exiting. Nothing
is refused and no stream is cut, so a Claude Code session running through the router
does not notice. Measured across a live handover: 575 consecutive probes with zero
failures, and a 58-second streaming response begun on the old instance completed intact.

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
They are read and written through `/usr/bin/security`, and the items the router creates
trust it (`-T /usr/bin/security`). That is a deliberate trade. A locally built binary is
ad-hoc signed, so every `cargo build` produces a new code identity, "Always Allow"
whitelists only the build that asked, and the prompt returns on the next rebuild — with
no button that makes it stop. The cost is that any process running as you can read those
items through `security` without a prompt, which is the same footing as Claude Code's own
credential entry. If you would rather have a per-binary ACL, sign the binary with a
stable identity (`codesign -s <your-cert> target/release/agent-router`) and the Security
framework path becomes practical again.
Inbound `authorization` and `x-api-key` headers are discarded, never stored. Captured
bodies may contain your source code — the database is local and gitignored.

## Current limitations

* **Adding an account needs a terminal once.** Claude Code's login is a terminal
  program and produces no output when driven headlessly, so the router cannot run it
  inside the page. It sets everything up either side of that one command and detects
  when you finish.
* **No cost figures.** `usage.cost` is nullable and unset — there is no pricing table
  yet, and on a subscription the quota utilization the router already records is the
  more meaningful number.
* **Context composition is an estimate.** Totals are exact; the split across system /
  tools / history / tool results apportions that exact total by character share,
  because there is no local tokenizer for current Claude models.
