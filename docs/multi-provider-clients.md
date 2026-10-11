# Multiple providers and generic clients

2026-10-11. Implemented provider/client support, configuration and measured validation.

## Requirement

Agent Router must support multiple providers independently of the selected client.
Claude Code and Codex were the original primary clients. On 2026-10-11 the owner
directed the next phase toward one fully integrated client: it must have a CLI and
its own native VS Code chat/sidebar extension, use a server-provided model catalogue,
support plugins or skills and MCP servers, and expose complete session control through
Agent Router for sessions started from either client interface. The owner explicitly
confirmed that Claude Code and Codex are eligible; choosing a generic client is optional.
The owner prefers Codex because its integration also advances a reusable
OpenAI-compatible gateway for other clients.
The client owns its agent loop and tools; Agent Router owns provider accounts, routing,
protocol adaptation, usage capture and the managed runtime's integration.

The first version supports Anthropic, OpenAI and Kimi Code, as confirmed by the user.
Compatible services can be registered explicitly. API keys provide the baseline for
this scope; native subscription credentials retain their provider's
entitlement and refresh limits.

| Layer | Responsibility |
| --- | --- |
| Client integration | Configuration, session identity, feed and supported session controls |
| Protocol adapter | Messages, Responses and Chat Completions ingress |
| Provider registration | Service identity, endpoint, authentication and capability profile |
| Account | Credential storage and refresh ownership for one provider |
| Model route | Client-visible ID, provider, upstream model, selected account and capabilities |

## Primary client direction

**Codex is the primary integration target**, following the owner's preference for a
native CLI/VS Code client and a reusable OpenAI-compatible gateway. Claude Code offered
the shortest path using existing controls; Codex aligns the next phase with Responses
compatibility and an explicit app-server control protocol. Anthropic, OpenAI and Kimi
remain the required upstream providers.

Codex has a native CLI and VS Code chat/sidebar, shared provider configuration, skills,
plugins and MCP. Official OpenAI documentation covers its
[CLI/IDE settings](https://learn.chatgpt.com/docs/ide/settings),
[gateway configuration](https://learn.chatgpt.com/docs/enterprise/connect-to-a-gateway)
and [app-server API](https://learn.chatgpt.com/docs/app-server).
The installed test build is **0.162.0-alpha.17.2**; its earlier fixture result does not
select an alpha build for production. Pin and accept a compatible CLI/extension pair.

There are two contracts to complete:

| Contract | Responsibility | Current state |
| --- | --- | --- |
| OpenAI-compatible model gateway | Model discovery, authenticated inference, streaming, tool history, usage and errors for any compatible client | Standard discovery and the official SDK portable contract across three providers are implemented |
| Codex runtime integration | Native session lifecycle, editor/CLI context, approvals, cancellation, model selection and remote controls | Setup and watched session feeds are implemented; managed controls remain open |

Codex uses **Responses**. Strengthening that endpoint benefits other Responses clients;
Chat Completions clients use the separate adapter. Agent Router already exposes
`POST /v1/responses` and `POST /v1/chat/completions`, routing OpenAI natively and
translating Anthropic/Kimi Messages. Native Codex passed the earlier three-provider
fixture tool matrix. That result establishes portable inference/tool routing, not the
full OpenAI API or complete native editor/control support.
[Gateway compatibility requirements](https://learn.chatgpt.com/docs/enterprise/gateway-compatibility)
cover the Responses conversation and tool contract.

**Model discovery has two forms.** Other OpenAI-compatible clients need the standard
OpenAI list shape; `GET /v1/models` now provides it, preserving Anthropic discovery when
the request carries `anthropic-version`. Codex also needs richer, version-matched model
metadata. Agent Router generates and syncs `model_catalog_json` from the router's
routes, including aliases, labels, context limits, reasoning/tool/image capabilities and
valid instructions. The local catalog replaces the bundled catalog and loads at startup;
`agent-router codex models refresh` updates it without stopping running clients; finish
active turns and resume the same session after restarting. A `/v1/models` list alone does
not populate every Codex picker field or prove automatic refresh.
[Gateway rollout documentation](https://learn.chatgpt.com/docs/enterprise/roll-out-a-gateway)
explains custom aliases, version matching and startup loading.

**Remote control requires owning the runtime connection.** Bridge Agent Router's controls
to local app-server JSON-RPC and relay remote actions through the existing outbound machine
tunnel. CLI remote attachment is documented; the installed build also lists local
`app-server daemon` and `app-server proxy --sock` commands. Those are integration points,
not evidence that the actual native VS Code extension already attaches to the managed
backend. Prove the editor path with the pinned extension. Current app-server WebSocket
transport is experimental and unsupported for production; prefer local socket/stdio
integration where the selected release supports it and qualify any experimental path.
[App-server transports and CLI attachment](https://learn.chatgpt.com/docs/app-server)
document this boundary. Unmanaged sessions retain their safe watched-only behavior.

The remaining work is:

- Server-driven native model catalogues and refresh in both CLI and sidebar, with scoped
  credentials, correct capabilities/limits, atomic sync and reversible configuration.
- Managed app-server controls for start/resume, prompt/steer, cancellation,
  permissions/questions, model/reasoning/mode changes, compaction and archive; include
  liveness, acknowledgement, offline errors and recovery.
- Deliberate Codex Library targets and ownership ledgers for skills/plugins and MCP;
  invoke real skills/tools while keeping tool and provider credentials separate.
- Actual native VS Code sidebar integration with file/selection context and the same
  remotely controlled thread identity; verify CLI entrypoints and frontend changes too.
- Portable native compaction and subsequent turns across all three providers. Current
  cross-protocol Responses compaction and opaque encrypted input are refused. Select a
  valid native fallback/bridge; never fabricate an encrypted compaction item. Disabling
  compaction and requiring fresh sessions does not meet full primary-client support.
- Standard OpenAI-shaped model discovery plus official SDK acceptance for Responses and
  Chat Completions independently of Codex, preserving current Anthropic discovery.
  Define stateless full-history continuation and unsupported provider-hosted features
  explicitly; this phase does not claim every OpenAI API endpoint or transport.
- A pinned CLI/extension three-provider lifecycle matrix, including catalogue changes,
  tools, skills/MCP, approvals, compaction/resume, capture and failure recovery. Full UI
  suites, production builds and extension-host automation remain in CI.

| Candidate | CLI and native VS Code chat/sidebar | Role |
| --- | --- | --- |
| Codex | Yes | Selected primary; inference works, managed integration is pending |
| Claude Code | Yes | Existing proxy, picker sync, Library and wrapped editor controls remain available |
| OpenCode 1.19.0 | CLI and terminal-based editor extension | Tested routing reference; fails the native sidebar requirement |
| Cline | CLI and native editor UI | Not accepted against the router's complete integration contract |

Claude Code's non-Claude routing remains an Agent Router compatibility responsibility:
[Anthropic does not officially support it](https://code.claude.com/docs/en/llm-gateway).
OpenCode's [official IDE extension](https://opencode.ai/docs/ide/) is terminal-based;
Cline documents its [IDE](https://docs.cline.bot/usage/ide) and
[CLI](https://docs.cline.bot/cli/cli-reference) interfaces.

Northstar **AGEN-0157** now tracks the Codex-primary direction. Its open tasks are
catalogue **AGEN-0168**, runtime **AGEN-0169**, Library **AGEN-0170**, native frontends
**AGEN-0171**, compaction **AGEN-0172**, reusable gateway **AGEN-0173** and lifecycle
acceptance **AGEN-0174**. The unimplemented OpenCode plan **AGEN-0158–AGEN-0162** and
preliminary Claude-primary acceptance plan **AGEN-0163–AGEN-0167** were withdrawn as
superseded. New acceptance commands are planned and have not been run. The earlier
three-client transport work remains completed under **AGEN-0148**.

## Current implementation

Anthropic and Kimi Code use Messages upstreams; OpenAI uses Responses. The router
translates in either direction, so a client speaking a supported protocol can select
models from those three providers. JSON, streaming and portable tool history are covered.
[Current compatibility and limits](codex-and-protocol-translation.md) apply to generic
clients too.

Settings → Clients configures Claude Code, Codex and OpenCode with an exact model and
router credential. Review the managed settings before applying them locally; the router
merges them with existing client configuration and refuses a stale preview. Restore
reverts managed values that still match the installed configuration and preserves later
edits, including edited routing URLs or tokens. Review any retained values manually.
Hosted routers offer copy and download without writing client configuration on the
server. The Clients readiness check validates the model route, router credential and
availability of the selected provider credential. It does not send an inference request
or prove remote connectivity, provider entitlement or quota availability.

For a local OpenCode project, a supplied router credential is kept outside the project
in a private file with owner-only permissions (`0600`). The project configuration uses
`{file:/absolute/path/to/credential.token}`. Preview only shows that reference; **Apply**
creates the credential file. Copying or downloading a preview does not create it.
Global configuration and hosted exports retain their inline credential or placeholder
format and should be treated as private files.

Settings → Models also exposes explicit service registrations and model aliases. A
registered alias resolves before the built-in model-name fallback. The service chooses
its Messages or Responses protocol and authentication profile, and the route chooses
the exact upstream model and account. Distinct aliases can expose the same upstream
model at different services. Native Gemini/Bedrock adapters and Chat Completions
upstreams are outside this implementation.

## Generic client assessment

**OpenCode V1 is the tested generic reference client.** Its documented custom
provider configuration separates the SDK/protocol from model IDs and permits a shared
router endpoint with a list of models. The stable V1 SDK choices are:

| OpenCode package | Protocol | Router status |
| --- | --- | --- |
| `@ai-sdk/anthropic` | Messages | Supported |
| `@ai-sdk/openai` | Responses | Supported |
| `@ai-sdk/openai-compatible` | Chat Completions | Supported ingress |

An OpenCode custom provider can use the Messages SDK for all three providers, or
override the SDK per model. The same configuration keeps the client-facing protocol
constant while Agent Router selects and adapts the provider upstream.

OpenCode model routing, tools and capture are supported. Its server session feeds and
remote controls are not integrated. Cline has not been tested; its OpenAI-compatible
Chat Completions configuration can target the router, subject to the adapter's portable
feature limits. Native CLI validation uses temporary configuration and does not change
the user's installed clients or home settings.

## Configure OpenCode V1

Add a custom provider to `opencode.json` in the project, using the router's URL and
Client access token from Settings → Access tokens. This example uses the model IDs
from the smoke test. Choose exact IDs available in Agent Router's Settings → Models
and set client context/output budgets
within each selected model's limits; the example budgets do not describe provider limits.

```json
{
  "$schema": "https://opencode.ai/config.json",
  "model": "agent-router/claude-sonnet-4-6",
  "small_model": "agent-router/claude-sonnet-4-6",
  "enabled_providers": ["agent-router"],
  "provider": {
    "agent-router": {
      "npm": "@ai-sdk/anthropic",
      "name": "Agent Router",
      "options": {
        "baseURL": "http://127.0.0.1:8787/v1",
        "apiKey": "{env:AGENT_ROUTER_API_KEY}"
      },
      "models": {
        "claude-sonnet-4-6": {
          "name": "Anthropic", "tool_call": true,
          "limit": {"context": 128000, "output": 4096}
        },
        "gpt-5.4": {
          "name": "OpenAI", "tool_call": true,
          "limit": {"context": 128000, "output": 4096}
        },
        "kimi-for-coding": {
          "name": "Kimi", "tool_call": true,
          "limit": {"context": 128000, "output": 4096}
        }
      }
    }
  }
}
```

Set `AGENT_ROUTER_API_KEY` to the Client access token, then select a model with
`opencode --model agent-router/MODEL_ID`. For a hosted router, use its HTTPS URL with
`/v1` appended. OpenCode presents the router credential; Agent Router signs with the
selected provider account. `small_model` keeps auxiliary requests on the same router.

The router records OpenCode's `x-opencode-session-id` as `opencode:<id>` and names its
requests OpenCode in web and phone usage history.

## Provider and model registry

Register services explicitly with a stable ID, label, HTTPS base URL, wire protocol and
authentication profile. Built-in provider registrations can supply discovery, quota and
subscription behavior; compatible custom endpoints should reuse a protocol adapter and
configure their own authentication. Credential storage remains separate from the protocol.

Register each model route with a client-visible alias, service ID and exact upstream
model ID. The alias is globally unique; the upstream ID is scoped to its service and
retains its spelling. For example, two aliases could point to the same Claude model at
direct Anthropic and at another compatible service. The administrative API is
`GET /api/provider-registry`, `POST /api/provider-registry/services` and
`POST /api/provider-registry/models`. A model body contains `alias`, `service`,
`upstream_model` and `account_id`; display name and capability overrides are optional.
Delete routes before deleting a service they use. Built-in services cannot be deleted.

The selected account must belong to the service's provider. The router translates and
forwards with the upstream model ID, while capture keeps the requested alias, service
ID and upstream model separately. Those recorded values survive account and registry
changes. Existing built-in model names remain compatible fallbacks.

Capability metadata describes tools, images, reasoning, compaction and configured
context/output limits. Model overrides can restrict a service: feature flags combine
with AND, and supplied numeric limits use the lower service/model value. Null means
unknown; these settings do not establish a provider's limits or entitlement. Context
limits are metadata for client configuration, without tokenizer-based validation of
each request. Generated OpenCode budgets use known limits when supplied, capped at
128,000 context and 4,096 output tokens; those defaults are client budgets.

Registered routes reject unavailable tools, images and active reasoning before calling
the provider. An explicit output budget above a configured limit is refused. Omitted
or translation-generated output budgets are bounded by that limit before forwarding.
The native ChatGPT Codex subscription backend does not support output budgets, so a
registered route with a configured output limit refuses inference through that backend;
use an API-key account for a bounded route. Opaque reasoning state, provider-hosted
tools and cross-protocol compaction retain the translation limits documented separately.

Treat API keys and native subscription logins as different credential profiles.
Subscription entitlement and refresh ownership depend on the provider, and protocol
translation does not establish permission to use a subscription with another client.
Native login and OAuth credentials cannot be redirected to custom services.

## Validation

The native CLIs **Claude Code 2.1.267**, **Codex 0.162.0-alpha.17.2** and
**OpenCode 1.19.0** each completed tool execution, tool-result replay and final response
through Agent Router against local mock upstreams for Anthropic, OpenAI and Kimi.
Assertions cover model/account selection, provider credential replacement, scoped
router authentication and revocation, session/client identity and captured usage.
The HTTP proxy regression matrix separately covers JSON/SSE, cache normalization and
native passthrough across the supported protocols.

Authenticated live inference produced this measured matrix:

| Client | Anthropic | OpenAI | Kimi |
| --- | --- | --- | --- |
| Claude Code | Passed | Passed | HTTP 403: exhausted five-hour quota |
| Codex | HTTP 429: did not complete | Passed | Not run after quota refusal |
| OpenCode | Not run | Passed | Not run after quota refusal |

All three live OpenAI runs completed a tool cycle, replayed its result and captured
usage. Claude Code also completed the live Anthropic case. The remaining live cases
are unverified; quota failures and skipped runs are not passes. Implementation is
complete for the agreed scope without waiting for provider quotas to recover.

Run all three isolated native mock checks with installed client executables:

```sh
CLAUDE_BIN=/path/to/claude CODEX_BIN=/path/to/codex OPENCODE_BIN=/path/to/opencode \
  bash scripts/check-client-routing.sh
```

The scripts do not install packages or use real provider credentials. Clients receive
temporary settings and only local fake inference endpoints. The HTTP connector is
enabled solely in the test harness; production upstreams require HTTPS.
Live inference is a separate opt-in command, `bash scripts/check-live-client-routing.sh`.
It reads existing provider credentials and uses temporary client settings. The optional
`AR_LIVE_ANTHROPIC_MODEL`, `AR_LIVE_OPENAI_MODEL` and `AR_LIVE_KIMI_MODEL` variables
select exact models. Its output reports the measured client/provider matrix.

## Sources

- [OpenCode provider configuration](https://opencode.ai/docs/providers/)
- [OpenCode configuration schema](https://opencode.ai/config.json)
- [OpenCode V1.19.0 environment substitution](https://github.com/anomalyco/opencode/blob/v1.19.0/packages/opencode/src/config/variable.ts)
- [OpenCode V1.19.0 model selection](https://github.com/anomalyco/opencode/blob/v1.19.0/packages/opencode/src/cli/cmd/tui.ts)
- [Cline OpenAI-compatible configuration](https://docs.cline.bot/provider-config/openai-compatible)
