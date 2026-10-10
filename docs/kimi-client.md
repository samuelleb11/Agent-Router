# Kimi Code as a client: the same features as Claude Code, on one router

2026-10-10. A plan for Kimi Code (Moonshot's coding CLI and its VS Code extension) as a
second client of Agent Router, beside Claude Code, with the features the router gives
Claude Code: its requests recorded and priced by session and project, its sessions live in
the console and on the phones, driven remotely, the Library on its machines, and the
machine agent finding and setting up both clients.

Written for the maintainer, and as the contract the items are built against. The work is
Northstar's milestone `kimi-client` ([Tracked in Northstar](#tracked-in-northstar)). It
builds on [docs/sources-and-models.md](sources-and-models.md): Kimi Code's account, its
models, quota and prices are already the router's. Nothing in this document is built yet.

## Owner's decisions (2026-10-10)

Sam: "I'd like it if the next release would get us working. Automatic Kimi Code updates,
remote sessions, Library sync. Auto detection of Kimi Code AND Claude Code on the machine. I
expect to use the Kimi Code client and get all the same features as our current
implementation. I also expect the data recorded and shown in Agent Router to fit in our
current model. API pricing included."

| Question | Answer |
| --- | --- |
| What "automatic Kimi Code updates" means | **Surviving Kimi's own updates.** When Kimi Code's CLI and its VS Code extension update themselves, the machine agent puts the router's entries back: its provider, its hooks, the Library's items. The router does not install or upgrade Kimi Code. |
| How Kimi sessions are controlled remotely | **Watch all, host to control.** Every Kimi Code session is shown live. Full control (send, answer, interrupt, model, mode) is for the sessions the router hosts: started from the console, a phone, or `agent-router kimi` in a terminal. |
| Which models the Kimi Code client uses | Kimi Code's, through the router's Kimi Code account. A Claude subscription serves only Claude Code's own requests (a hand-made request was answered 429, AGEN-0097), so Claude's models stay with Claude Code. |

## What was measured

Kimi Code CLI 2.1.1 (a Node single-executable app at `~/.kimi-code/bin/kimi`) and its VS Code
extension 0.8.1, on 2026-10-10, against a scratch home and a stand-in model server: no
tokens spent, and nothing of the owner's Kimi Code state touched.

### Its home

`KIMI_CODE_HOME`, default `~/.kimi-code`, is shared by the CLI and the extension:

- `config.toml`: providers, models, `default_model`, `[services.*]`, `[[hooks]]`;
- `tui.toml`: the terminal UI's own preferences, `[upgrade] auto_install` among them;
- `mcp.json`, `skills/`, `agents/`, `plugins/installed.json`;
- `sessions/<wd>/<sid>/`, `session_index.jsonl`, `workspaces.json`. A workspace id is
  `wd_<basename>_<sha256 of the absolute path, 12 hex>`;
- `updates/`, `logs/`, `telemetry/`, `device_id`, `region`.

### Its requests through an `anthropic` provider

- `POST {base_url}/v1/messages?beta=true`, always streamed.
- The key is sent as `x-api-key`. `Authorization` is sent only when `custom_headers` add it.
- `metadata.user_id` is `session_<uuid>`, the session's id, on every request, subagents
  included. No header names the session.
- The working directory appears only in the system prompt: "The current working directory
  is `…`".
- The User-Agent names the client: `kimi-code-cli/2.1.1` in the terminal, with `(web)` under
  `kimi web`; `kimi-code-vscode/<v>` in VS Code; `Anthropic/JS <v>` under `kimi acp`.
- Kimi Code retries a 529 up to ten times, each retry a request like the first.
- No `/v1/models` and no `count_tokens`: Kimi counts tokens itself.
- Per-provider `custom_headers` and `KIMI_CODE_CUSTOM_HEADERS` are sent.

### Its sessions on disk

- Each session has `state.json` (id, cwd, title, last prompt, `lastTurnReason`) and an
  append-only `agents/main/wire.jsonl` (wire protocol 1.5).
- Each subagent has its own `agents/agent-N/wire.jsonl`.
- The records:
  - `turn.prompt`;
  - `context.append_message`;
  - `context.append_loop_event`, carrying `step.begin`, `tool.call`, `tool.result`,
    `content.part` and `step.end`;
  - `usage.record`;
  - `interaction.request` and `interaction.resolved`;
  - `turn.ended`.
- No file says whether a session is running: a turn in progress, an approval waiting, and a
  killed process all leave a `turn.prompt` with no `turn.ended`.

### Its hooks

- `[[hooks]] event, matcher, command, timeout` in `config.toml`.
- The events include SessionStart, SessionEnd, SessionHeartbeat (every 60 s), Stop,
  PermissionRequest, PermissionResult, PreToolUse and UserPromptSubmit.
- The command gets a JSON payload on stdin: `hook_event_name`, `session_id`, `cwd`,
  `client_type`.
- Killing the terminal UI fires no SessionEnd.

### Its control surfaces

- **`kimi web`.** A loopback server, its bearer in `server.token`, with an HTTP API:
  - `GET /api/v1/sessions` (busy, pending interaction, last turn);
  - `POST …/{id}/prompts`;
  - `GET`/`POST …/{id}/approvals`;
  - `GET …/{id}/messages`;
  - an abort;
  - `POST …/profile` for model and mode;
  - a WebSocket.
- **`kimi acp`.** The Agent Client Protocol over stdio: `session/new`, `session/load`,
  `session/prompt`, `session/cancel`, `session/set_mode`, `session/set_model`, and
  `session/request_permission` back to the client.
- **The terminal UI and the VS Code extension** keep each session in their own process (the
  extension runs Kimi's runtime in-process), with no socket. A second process resuming the
  same session interleaves its `wire.jsonl`: watched only.

### Its Library places

- **Skills:** `$KIMI_CODE_HOME/skills/<name>/SKILL.md`, the same shape as Claude Code's.
- **Agents:** `$KIMI_CODE_HOME/agents/<name>.md`, frontmatter `name` (kebab-case),
  `description`, `whenToUse`, `tools`, the body the prompt.
- **Commands:** no user commands folder. Commands arrive only in a plugin (`kimi.plugin.json`
  with `commands`, `$ARGUMENTS` expanded).
- **MCP:** `$KIMI_CODE_HOME/mcp.json`, and a repository's `.mcp.json`, Claude Code's shape.

### Its updates and its detection

- The CLI updates itself from `code.kimi.com`, staged beside the binary and swapped at the
  next launch, unless `[upgrade] auto_install = false` or `KIMI_CODE_NO_AUTO_UPDATE` is set.
- The extension updates through the Marketplace, with its own runtime.
- A machine has Kimi Code when it has `KIMI_CODE_HOME`, a `kimi` binary (native, npm, pnpm,
  yarn, bun, Homebrew) and the `moonshot-ai.kimi-code-*` extension. It has Claude Code when
  it has `~/.claude`, `claude` on PATH, and the `anthropic.claude-code-*` extension.

## Feature by feature

| Claude Code has | Kimi Code gets | How |
| --- | --- | --- |
| Requests routed and signed by the router | the same | an `anthropic` provider at the router; the router routes `k3` and the rest to the Kimi Code account (AGEN-0084) |
| Requests by session, project and client | the same | the session from `metadata.user_id`, the directory from the system prompt, the client from the User-Agent (AGEN-0106) |
| Cost and quota | the same | Kimi Code's prices (AGEN-0089), cached tokens counted once (AGEN-0098), its windows (AGEN-0087, 0088) |
| The router's password from a remote machine | the same | taken as `x-api-key` (AGEN-0105) |
| `agent-router install` and the machine agent set it up | the same | the provider, the catalogue's models, the default model and hooks in `config.toml`, put back after every update (AGEN-0108) |
| The machine knows it has Claude Code | knows it has both | detection, reported to Machines (AGEN-0107) |
| Live sessions, from `~/.claude/sessions` and transcripts | every session, from its files and hooks | AGEN-0109, 0110 |
| Driven from the console and the phones, under the wrapper | the sessions the router hosts | `kimi web` run by the agent; `agent-router kimi` in a terminal (AGEN-0111, 0112, 0113) |
| The `/model` picker's rows | Kimi's own models list | the provider's model aliases (AGEN-0108) |
| The Library's skills, subagents and commands | the same, in Kimi's places | skills as they are, subagents converted, commands in a plugin (AGEN-0114, 0115) |
| The console and the phones | the same, with each session's client | AGEN-0116, 0117, 0118 |

Kimi Code's own extras stay its own: its web search and fetch services, its telemetry and
its updates go to Kimi with its own sign-in.

## Later

- Driving a Kimi session started in the terminal UI and in VS Code: not possible from outside
  while Kimi keeps them in-process.
- Claude's models from the Kimi Code client: needs a metered Anthropic API key, not a
  subscription.

## Tracked in Northstar

Milestone `kimi-client` in project `agent-router`, filed 2026-10-10. Each task is a child of
its epic, and "waits on" is a `blocks` edge drawn from the item that waits.

| Item | What | Waits on |
| --- | --- | --- |
| **AGEN-0099** | Epic: requests recorded as Claude Code's are | |
| AGEN-0104 | This plan | |
| AGEN-0105 | A hosted router takes its password as `x-api-key` | |
| AGEN-0106 | A Kimi request filed under its session, project and client | |
| **AGEN-0100** | Epic: detection and setup, through Kimi's updates | |
| AGEN-0107 | Each machine finds Claude Code and Kimi Code | |
| AGEN-0108 | Kimi's `config.toml` gets the router's entries, kept through updates | 0107 |
| AGEN-0109 | Kimi's hooks tell the agent which sessions are live | 0108 |
| **AGEN-0101** | Epic: live sessions, watched and hosted | |
| AGEN-0110 | Every Kimi session read from its files | 0109 |
| AGEN-0111 | Hosted sessions through `kimi web`, driven | 0107 |
| AGEN-0112 | The live routes: watched, hosted, started | 0110, 0111 |
| AGEN-0113 | `agent-router kimi` in a terminal | 0111 |
| **AGEN-0102** | Epic: the Library on Kimi Code | |
| AGEN-0114 | Skills, and subagents converted | 0107 |
| AGEN-0115 | Commands as a plugin, state per client | 0114 |
| **AGEN-0103** | Epic: the console and the phones | |
| AGEN-0116 | Console | 0106, 0107, 0112 |
| AGEN-0117 | iOS | 0107, 0112 |
| AGEN-0118 | Android | 0107, 0112 |
| AGEN-0119 | End to end on the hosted router | 0105, 0106, 0108, 0112, 0116 |
