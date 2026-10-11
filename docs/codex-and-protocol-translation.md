# Codex setup and protocol translation

2026-10-11. Implemented behavior and compatibility limits for connecting Claude Code,
Codex and OpenCode to accounts at Anthropic, Kimi Code and OpenAI.

The client and provider are independent. A registered model alias explicitly chooses
its service, upstream model and account. Other catalogue models use their assigned
account, falling back to that provider's active account. The router replaces the
client's credential with the selected account's credential and adapts the protocol if
needed. Capture retains the requested alias, service and upstream model separately.
The following table describes the built-in services; a registered service declares its
Messages or Responses protocol explicitly.

| Client protocol | Anthropic / Kimi Code account | OpenAI account |
| --- | --- | --- |
| Anthropic Messages (`POST /v1/messages`) | Native forwarding | Translate to Responses |
| OpenAI Responses (`POST /v1/responses`) | Translate to Messages | Native forwarding |
| OpenAI Chat Completions (`POST /v1/chat/completions`) | Translate through Responses to Messages | Translate to Responses |

Native catalogue inference bodies and streams are forwarded unchanged, except for the
settings required by the ChatGPT Codex backend: `store: false`, `stream: true`, omitting
API-only `max_output_tokens`, `metadata`, `temperature` and `top_p`, and moving top-level
instructions into leading developer input. Sampling follows the native Codex backend's
settings; use an API-key account for API sampling controls. Translated streams are
emitted incrementally. If a client asks for JSON and its upstream only streams, the
router collects the terminal response.
Capture parses the upstream response so its usage and request ID retain the provider's
meaning. OpenAI input usage is split into uncached input and cache reads; reasoning tokens
are part of output, rather than extra tokens added to its total. Registered routes also
replace the upstream model ID and enforce their configured output budget.

## Accounts and models

- **OpenAI API key:** add an account in Settings and choose OpenAI. The key stays in the
  router's credential store. After adding it, the router fetches its model listing.
  A custom HTTPS base URL (without a trailing `/v1`) can target an OpenAI
  Responses-compatible service for supported model IDs.
- **Local Codex login:** a local router discovers a usable native login under
  `CODEX_HOME` or `~/.codex`. File credentials and Codex's direct macOS Keychain store are
  supported. API keys and ChatGPT subscription credentials use their respective upstreams.
  Codex owns login and refresh; the router rereads its credentials for each request.
  Run Codex's own login flow if its access token expires. Encrypted, ephemeral and
  non-macOS keyring-only credential stores are currently unavailable to this reader.
- **Hosted router:** add an OpenAI API key to the server. A machine agent configures the
  client and reports watched sessions; it does not upload the machine's Codex login.

The catalogue combines OpenAI's model listing, Codex's visible model cache, observed
models and explicit assignments. Prices are configurable; a model with no rate stays
unpriced. OpenAI subscription quota is currently unavailable in the dashboard.

Claude Code's picker gets the router's catalogue through the existing installer sync.
In any client, select the exact model ID from Settings → Models. Model assignments
remain within a provider; changing the client does not change an account assignment.

Protocol compatibility does not establish subscription entitlement. The router preserves
the client's identity rather than impersonating a different client. Use a provider
credential that permits the chosen client; the measured live results below do not
establish entitlement for combinations that were quota-blocked or skipped.

Settings → Models configures explicit services, aliases and capability overrides. Model
overrides restrict service capabilities with AND and take the lower supplied numeric
limit. Unknown context/output limits remain unknown. Context limits guide client
configuration and are not checked by tokenizing every request. A configured output cap
rejects an explicit larger budget and bounds omitted or translation-generated budgets
before forwarding. A bounded route refuses native ChatGPT Codex subscription inference
because that backend removes output budgets; use an API-key account for such a route.

Codex is now the primary deep-integration target. The
[primary-client design](multi-provider-clients.md#primary-client-direction) separates the
reusable OpenAI-compatible gateway from managed native session controls. This document
describes the implemented transport scope. Standard discovery, generated Codex metadata
and explicit refresh are implemented; native sidebar qualification, managed runtime and
lifecycle acceptance are tracked under Northstar AGEN-0157.

## Configure Codex

Settings → Clients offers model selection, a readiness check, a configuration preview
and reversible local apply/restore. A hosted router exports the managed settings for
the machine running Codex. Use a Client token for inference; a Machine token also works
when the same machine runs the router agent. The readiness check resolves the model
route and provider credential and validates a supplied router credential. It sends no
inference request and does not verify remote connectivity, entitlement or remaining
quota. Preview exports only router-managed settings, keeping unrelated saved secrets
out of the browser. Apply merges the reviewed settings and refuses a stale file;
restore preserves subsequently edited managed values, which may retain routing URLs
or tokens and should be reviewed manually.

Codex Apply also writes a private, atomic `agent-router-models.json` and selects it with
`model_catalog_json`. A separate ownership ledger restores the previous catalogue path
and preserves edited files. It contains only serving model aliases, labels, effective
capabilities and portable coding instructions; provider accounts, endpoints, credentials
and cached private instructions are excluded. The native metadata is qualified against
Codex **0.162.0-alpha.17.2**, bundled with extension **26.1007.21434**. It replaces bundled
models. Routes without tools are hidden from the coding picker. Unknown token limits use
128,000 context and 4,096 output headroom; configured output caps are enforced by the
gateway, since Codex has no per-model max-output catalogue field.

For hosted exports, merge the generated config on the client machine, then run:

```sh
agent-router codex models refresh
```

The command reads the router URL and Client token from the existing provider config;
`AGENT_ROUTER_CLIENT_TOKEN` can supply a replacement credential for this request. It
installs or refreshes the catalogue without stopping a running client. The machine
agent maintains owned catalogues every fifteen minutes. Local installer maintenance
does the same using the local store. **Codex loads metadata at startup.** Finish active
turns before restarting; use `codex resume <SESSION_ID>` to retain the CLI conversation,
or reopen the same conversation after reloading the editor window. The command does
not claim to refresh an already running picker. `agent-router codex models restore`
restores only the catalogue; Clients → Restore also restores managed routing settings.

`GET /v1/models` returns standard OpenAI `object: list` entries with `id`, `object`,
Unix-second `created` and `owned_by`. Requests carrying `anthropic-version` retain the
Anthropic shape used by Claude Code. `GET /_router/models` exposes versioned capability
metadata to Client and Machine tokens; both endpoints are answered locally.

`bash scripts/check-primary-codex.sh gateway` runs the official Python OpenAI SDK
**3.28.0** against three local providers: both APIs, JSON and SSE, terminal events,
function-call/result replay, stateless Responses history, JSON schemas, usage, useful
provider errors and Client-token replacement/revocation. It also checks refusals for
unsupported cross-provider stored history, encrypted reasoning and hosted tools.
This qualifies the portable gateway contract rather than every OpenAI API feature.
`catalogue-core` checks native `model/list` additions/removals and private reversible
files; `catalogue` additionally drives the published native sidebar in Linux CI. The
editor job records failing baseline and current results as importable Northstar artifacts.
History retention compares complete records and session identity while accounting for
the pinned app-server's legacy-to-paginated display-event migration. User/assistant
content, tool calls/results, timestamps, record order and existing metadata stay intact.

OpenCode project configuration uses a `{file:/absolute/path/to/credential.token}`
reference for a supplied router credential. Only Apply creates the private `0600` file
outside the project; preview, copy and download do not. Global configurations and
hosted exports use their existing inline credential or placeholder format.

Run `agent-router install` after installing Codex. The installer saves the previous
`model_provider` and `web_search`, adds its own provider and selects it. It preserves the
chosen `model` and other settings, and writes the config privately because it contains
the router password. Codex sends a provider-hosted `web_search` tool by default; the
installer sets `web_search = "disabled"` so the request can work across providers.

Uninstall removes the managed provider and restores the old provider and web-search
setting if they still have the installed values. The saved state records the config's
path, so restoring it does not depend on the current `CODEX_HOME`. A preexisting provider
called `agent-router` requires a different name before installation, so the installer
does not overwrite an unrelated provider.

For manual setup, add this to `$CODEX_HOME/config.toml` or `~/.codex/config.toml`:

```toml
model_provider = "agent-router"
web_search = "disabled"

[model_providers.agent-router]
name = "Agent Router"
base_url = "http://127.0.0.1:8787/v1"
wire_api = "responses"
requires_openai_auth = false
supports_websockets = false

[model_providers.agent-router.http_headers]
Authorization = "Bearer <CLIENT_ACCESS_TOKEN>"
```

Replace `<CLIENT_ACCESS_TOKEN>` with a Client access token created in Settings → Access
tokens. For a hosted router, use its HTTPS URL. The legacy router password also works
as the Bearer value. Restrict the config's file permissions to its owner.
The client authenticates to the router; the router selects and signs for the actual
provider. Clients retaining their own Authorization header can use
`x-agent-router-token` for the router token. The legacy `x-agent-router-password` header
also works; both headers are stripped before forwarding.

Select a model with `codex --model MODEL_ID`. Start a fresh session when switching
providers if its saved history includes provider-specific reasoning or compaction items.
When using an OpenAI model natively, web search can be enabled explicitly with
`codex -c 'web_search="cached"' --model MODEL_ID`; that tool cannot be translated to
a Messages provider.

## Translation compatibility

| Feature | Cross-protocol behavior |
| --- | --- |
| Text, privileged instructions and full portable history | Translated; Responses system/developer text is collected in the Messages system field in input order |
| Images in user messages and tool results | Translated for supported image blocks |
| Function tools and tool results | Translated, including parallel calls |
| Namespaced tools | Names are flattened for Messages and restored in Responses output |
| Custom tools | Raw input is wrapped as a JSON string for Messages and unwrapped in output |
| Custom-tool grammars | Passed as instructions to the model; the Messages provider cannot enforce the raw grammar |
| Visible reasoning | Translated; signed Messages thinking history becomes portable text |
| Cached input and reasoning usage | Normalized without counting either twice |
| JSON replies and fragmented SSE | Translated |
| JSON-schema output format | Translated |
| Provider-hosted tools | Refused with a clear error |
| Encrypted/redacted reasoning history | Refused when it cannot be represented |
| Stored Responses references (`previous_response_id`, `conversation`, `item_reference`) | Refused; send full portable history |
| Messages `clear_thinking_20251015` edits | Supports omitted/`all` retention or a positive `thinking_turns` count; retained visible thinking becomes text |
| Responses compaction and other provider-specific context edits | Refused across protocols; native OpenAI compaction is forwarded |
| Nonempty Messages stop sequences | Refused across protocols |
| Messages token-counting endpoint for an OpenAI model | Refused |

Native requests retain their own provider features. Requesting encrypted reasoning in
Responses output does not prevent an otherwise portable request from being translated;
opaque encrypted items in its input do.

Codex's sidebar inserts developer context after initial user/environment messages.
For Messages targets, the router keeps that privileged text in the top-level system
field, including updates after tool history. User messages and tool results keep their
original conversation roles. This portable mapping changes the prompt-prefix placement
of later instructions and can invalidate a provider's prefix cache; it does not use
model-specific inline-system features.

## Managed remote control and local stdio attachment

Configure inference first with Settings → Clients using a Client token. Then select
the version-matched native executable:

```sh
agent-router codex configure /absolute/path/to/codex
agent-router codex run
```

Configure prints the private launcher path. The Agent Router VS Code extension's
**Connect Managed Codex** command configures the published Codex sidebar to use it;
**Restore Codex Launch Configuration** restores the previous executable setting while
preserving subsequent user edits. Finish active turns and reload the editor after
changing the launcher. This adapter is qualified against extension 26.1007.21434 and
Codex 0.162.0-alpha.17.2; the extension's custom executable setting is labelled for
development use upstream, so other versions are refused pending qualification.

The local supervisor owns one native app-server listening on a private Unix socket.
Codex's socket transport uses WebSocket framing. Its native `app-server proxy` relays
raw socket bytes; Agent Router adds the WebSocket handshake and converts JSONL for
`agent-router codex proxy` and the sidebar launcher. The actual terminal UI uses native
`--remote unix://...` attachment. No app-server listener is exposed to the network.
Remote commands use the existing authenticated outbound machine tunnel.

Managed threads support acknowledged prompts/steering, interruption, model/effort/mode
changes, approvals/questions, explicit resume and native archive/unarchive. The console
marks only managed reachable threads as controllable. Approval replies name their
thread and connection and wait for native resolution; disconnects clear pending ownership.
Unknown or disconnected actions fail without automatic replay. Unmanaged sessions keep
their watched-only behavior. Client tokens authorize inference, not remote controls;
the machine tunnel and Administrator access authorize those separately.

The backend survives frontend and router-agent exits. To load changed catalogue metadata,
finish active turns and run `agent-router codex restart`, then reconnect the sidebar or
resume the saved CLI thread. Restart/stop refuse an observed active turn. Explicit
resume retains native thread identity and history after backend recovery. Cross-provider
compaction and opaque encrypted OpenAI history remain separate work under AGEN-0172;
side questions and native authentication ceremonies are not driven through this bridge.

`bash scripts/check-primary-codex.sh runtime` exercises the real pinned backend and
machine tunnel, including actual native terminal attachment. `frontends` also runs the
published native sidebar, editor context and remote controls in Linux CI. Northstar
AGEN-0169/AGEN-0171 hold the measurements; a passing work-tree check alone does not close them.

## Sessions and validation

Codex CLI and VS Code installations are detected and displayed as clients. Native
`thread-id` identifies captured requests, with `session-id` and older header forms as
fallbacks. Local rollout files supply watched session feeds, tools and usage. Session
IDs are prefixed `codex:` to
keep them distinct from Claude Code IDs. The web console, iOS and Android display them
with controls only for threads owned by the managed backend. Process liveness and native
paginated history are not established from these rollout files.

Regression coverage includes credential-store decoding, model routing, session parsing,
tool/history translation, fragmented SSE and usage. HTTP proxy tests use local mock
providers to exercise both directions, JSON and streaming, credential replacement,
response headers and capture. Native **Claude Code 2.1.267**,
**Codex 0.162.0-alpha.17.2** and **OpenCode 1.19.0** each completed tool execution,
tool-result replay and final responses against mock Anthropic, OpenAI and Kimi upstreams.

Real OpenAI inference passed with all three clients, including tool execution, replay
and captured usage. Claude Code also passed real Anthropic inference. Codex's Anthropic
run received HTTP 429 and did not complete; OpenCode's Anthropic run was not performed.
Kimi returned HTTP 403 for an exhausted five-hour quota, so the remaining Kimi client
runs were not performed. These cases remain unverified, rather than passing. The
implementation is complete for the agreed scope without waiting for quota recovery.
See the [full validation matrix and commands](multi-provider-clients.md#validation).

## Sources

- [Official OpenAI documentation: advanced Codex configuration](https://learn.chatgpt.com/docs/config-file/config-advanced)
- [Official OpenAI documentation: Codex configuration reference](https://learn.chatgpt.com/docs/config-file/config-reference)
- [Official OpenAI documentation: Responses API](https://developers.openai.com/api/docs/guides/text?api-mode=responses)
- [Anthropic context editing](https://platform.claude.com/docs/en/build-with-claude/context-editing)

