# Multiple providers and generic clients

2026-10-11. Implemented provider/client support, configuration and measured validation.

## Requirement

Agent Router must support multiple providers independently of the selected client.
Claude Code and Codex are the original primary clients. A generic client such as OpenCode
or Cline is also acceptable. The client owns its agent loop and tools; Agent Router owns
provider accounts, routing, protocol adaptation and usage capture.

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
