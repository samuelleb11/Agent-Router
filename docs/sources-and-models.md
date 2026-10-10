# Sources: models from more than one provider, picked by model

2026-10-09. A plan for routing each request to an account at any Anthropic-compatible
provider, chosen by the model the request asks for. Kimi Code is the first provider after
Anthropic. Switching provider is switching model in the client — `/model k3` in Claude
Code — with no account to flip and nothing restarted.

Written for the maintainer, and as the contract the items are built against. The work is
Northstar's milestone `sources` ([Tracked in Northstar](#tracked-in-northstar)). The router's
part is built: accounts at a provider, the catalogue, and routing by model (AGEN-0082 to
AGEN-0085). Kimi Code's quota, limits and prices, and the clients, are not yet.

## Owner's decisions (2026-10-09)

| Question | Answer |
| --- | --- |
| What Agent Router is | **A router.** It can have different sources and potentially different clients. Claude Code is the favourite client, and stays first-class. |
| How a request picks its account | **By model.** Every source's models are presented as one catalogue, each model name unique, and each model is assigned an account. To switch, pick another model in the client. |
| What happens when a source's limit is spent | **Nothing automatic.** The router shows the account as spent, and the owner switches by hand. ARCHITECTURE.md's "not a load balancer for evading rate limits" stands. |

## Words

- **Provider** — the service an account belongs to: `anthropic` or `kimi` (Kimi Code). A
  provider fixes the base URL, the auth header, the quota reader and how a spent limit looks.
- **Account** — one credential at one provider. Where an Anthropic credential is kept is its
  existing `source` (`keychain`, `token`, `profile`). A Kimi Code key is kept like a
  setup-token: `source = token`, in the router's 0600 `credentials.json`.
- **Source** — what the console and the phones call an account now that it can be at more
  than one provider. Not a new thing in the store.
- **Catalogue** — every model the router can serve, each with its provider and the account
  it is assigned to.

## The shape

```
Claude Code ──POST /v1/messages {model: "k3", …}──▶ router
                                                     │ 1. read `model` from the body it already buffers
                                                     │ 2. catalogue: k3 → provider kimi → its account
                                                     │ 3. that provider's base URL and auth header
                                                     ▼
                         https://api.kimi.ai/coding/v1/messages   x-api-key: <the account's key>
```

- **The body is forwarded as it was sent.** The client asked for `k3` and Kimi Code serves
  `k3`: no name is rewritten in either direction. The request row records what the
  response named and which account served it.
- **What changes on the wire** is the credential and the destination. ARCHITECTURE.md's
  "the only thing it changes on the wire is the `Authorization` header" becomes "the
  credential and the host, chosen by the request's model".
- **Paths that carry a model** — `/v1/messages` and `/v1/messages/count_tokens` — are routed
  by it. Every other `/v1/` path goes to the active Anthropic account, as today, except
  `GET /v1/models`, which the router answers itself from the catalogue.
- **A model no provider serves** is refused by the router before anything leaves it, with
  an Anthropic-shaped error the client shows verbatim:
  `404 {"type":"error","error":{"type":"not_found_error","message":"agent-router: no source serves the model \"…\""}}`.

## Providers

| | Anthropic | Kimi Code |
| --- | --- | --- |
| Base URL | `AGENT_ROUTER_UPSTREAM`, default `https://api.anthropic.com` | per account, default `https://api.kimi.ai/coding` (China: `https://api.kimi.com/coding`) |
| Auth header | `Authorization: Bearer` | `x-api-key` (Kimi's Claude Code guide sets `ANTHROPIC_API_KEY`) |
| Key format | `sk-ant-` | Kimi Code console key; never `sk-ant-` |
| Models | every `claude-*` id | `k3`, `k3-256k`, `kimi-for-coding`, `kimi-for-coding-highspeed` (by plan: K3 needs Plus, HighSpeed and K3 1M need Pro) |
| Quota | `/api/oauth/usage`, then the `anthropic-ratelimit-unified-*` headers | `GET /coding/v1/usages` with the same key as `Bearer` |
| Spent limit | `429` with the unified headers | `403` with a message naming the window (5-hour, weekly, monthly) |
| Identity | `claude auth status` | none |

**Nothing Anthropic-only is ever sent with another provider's key.** Today `/api/quota`
signs `/api/oauth/usage` with whatever the active credential is. Once a Kimi Code account
can be active, that would hand its key to Anthropic. Each Anthropic-only call checks the
provider first.

## The catalogue

- **Which provider a model belongs to.** A model whose id contains `claude` belongs to
  Anthropic, so a new Claude model works the day it ships. Every other model belongs to the
  provider whose list names it. Kimi Code's list comes from its own `GET /v1/models` if it
  answers one ([still to measure](#still-to-measure)), else from the table above.
- **Which account serves it.** Each model may be assigned an account of its own provider.
  A model with no assignment goes to its provider's **active** account. `active` becomes one
  per provider: activating a Claude account leaves the Kimi Code account active, and the
  palette's "Use …", the phones' switcher and `POST /api/accounts/activate` keep working
  within a provider.
- **The API.**
  - `GET /api/models` — each model: name, provider, assigned account (or none), the account
    that serves it now, and the provider's display name for it.
  - `POST /api/models/assign {model, account}` — `account: null` clears the assignment. An
    account of another provider is refused.
  - `GET /v1/models` — the catalogue in the Anthropic Models API shape (`data[]` of `{type:
    "model", id, display_name, created_at}`), for any client that lists models.

## Kimi Code

From Kimi's documentation (kimi.com/code/docs, read 2026-10-09):
- The subscription's Anthropic-compatible endpoint is the base URL above. The same key and
  quota are shared by Kimi's own CLI, its VS Code extension and every third-party tool.
- **Keep the client's real User-Agent.** Changing it "is considered a violation and may
  result in suspension of membership benefits". The router forwards the client's
  `User-Agent` untouched, and its own calls (the quota reader) say they are Agent Router.
  It never presents itself as Kimi's CLI.
- Quota is a rolling 5-hour window plus a monthly total (legacy plans: a weekly window).
- Errors: `401` for a bad key, an unknown model id and a model above the plan; `402` for a
  membership that cannot be verified; `403` for a spent 5-hour, weekly and monthly window
  and for the concurrency limit; `429` for too many requests and an overloaded engine.

**The quota reader.** `GET /coding/v1/usages` is not documented by Kimi, but its CLI's
`/usage` reads it, and so do several third-party meters. Two shapes are in the wild:
- older: `usage` (the weekly quota: `limit`, `used`, `remaining` as strings, `resetTime`)
  and `limits[]` (a window of `{duration: 300, timeUnit: "TIME_UNIT_MINUTE"}` with the same
  `detail`);
- newer: `limits[]` as above, and `usages` — a map of pools `limit_5h`, `limit_7d`,
  `limit_month_total`, `limit_month_code`, each `{used_ratio, reset_time}`.
The router reads both. It maps the 5-hour pool onto the 5-hour window the console already
draws, and the weekly pool, else the monthly total, onto the second window, with the
window's real name beside it. One community note says the endpoint answered 404 without
Kimi's CLI User-Agent; if the router's own User-Agent is refused, the reader is off and the
account's windows come from its `403`s alone.

**A spent limit.** A `403` whose message names a window marks the account spent until that
window's reset, the same state a Claude account's `rejected` status shows. The concurrency
`403` is not a spent quota and is counted as an error.

**Prices.** A subscription is flat-rate, like a Claude subscription. The router prices both
at the provider's metered rates, so "what this would have cost" stays comparable: Kimi's
models at Moonshot's Open Platform rates, cited with their date in the pricing seed.

### Kimi Code rates (read 2026-10-10)

From Moonshot's Open Platform pricing page, platform.kimi.ai/docs/pricing/chat, which shows
no date of its own. US dollars per million tokens, as the seed in `src/pricing.rs` holds them:

| Model | Input | Cache write 5 min | Cache write 1 h | Cache read | Output | Priced as |
| --- | --- | --- | --- | --- | --- | --- |
| `k3` | 3.00 | 3.00 | 6.00 | 0.30 | 15.00 | `kimi-k3` |
| `k3-256k` | 3.00 | 3.00 | 6.00 | 0.30 | 15.00 | `kimi-k3`, in a smaller window |
| `kimi-for-coding` | 0.95 | 0.95 | 0.95 | 0.19 | 4.00 | `kimi-k2.7-code` |
| `kimi-for-coding-highspeed` | 1.90 | 1.90 | 1.90 | 0.38 | 8.00 | `kimi-k2.7-code-highspeed` |

- `kimi-for-coding` is K2.8 Preview today, upgraded in place, and the page does not list it.
  K2.7 Code is the nearest model it does list, so its figures are an estimate.
- `kimi-for-coding-highspeed` is K2.7 Code Highspeed, by the Kimi CLI's own name for it
  (its `config.toml`, 2026-10-10), so its figures are that model's own.
- The K2 series charges nothing for writing to the cache, so a write is priced as an
  uncached input.
- A rate is edited in the `pricing` table and survives upgrades. Every request is re-priced
  from it at the next start.

### Measured without a key (2026-10-10)

Asked of `https://api.kimi.ai/coding` with an invalid key and Agent Router's own User-Agent,
and read from the Kimi CLI's configuration (Kimi Code CLI 2.1.1, signed in to the owner's
plan), nothing of either credential used:

- **The plan's models** are the four the catalogue lists: `k3` (1M context), `k3-256k`,
  `kimi-for-coding` ("K2.8 Preview", 1M) and `kimi-for-coding-highspeed` ("K2.7 Code
  Highspeed", 256K). K3 at 1M and HighSpeed mean a Pro plan or above. The CLI itself talks
  to `https://api.kimi.ai/coding/v1`, signed in with a 15-minute OAuth token only it renews,
  which the router cannot sign with: a router account takes a console key.
- **Routes.** `POST /v1/messages`, `POST /v1/messages/count_tokens`, `GET /v1/models` and
  `GET /v1/usages` all exist: each refuses the invalid key with 401, where a route that
  does not exist answers 404 `resource_not_found_error`. `/v1/usages` answers the router's
  own User-Agent, so the note that it answered 404 to anything but Kimi's CLI no longer
  holds, at least for reaching it.
- **Both auth headers are read**: `x-api-key` and `Authorization: Bearer` each get the same
  invalid-key refusal. Which one a valid console key needs is still to measure.
- **Error bodies are not Anthropic's shape.** `{"error":{"message":"The API Key appears to
  be invalid or may have expired. …","type":"invalid_authentication_error"}}` has no
  top-level `"type":"error"`, which is what capture looks for before it records an error's
  message (`SseScanner::absorb_json_body`). AGEN-0088 reads the message without it.
- **No `request-id`.** A response carries `x-trace-id` instead, and none of Anthropic's
  `anthropic-ratelimit-*` headers.

### Still to measure

Measured since: see [What Kimi Code accepts](#what-kimi-code-accepts-measured-2026-10-10).
What these were, for the record. Each answer went in a section of this document, dated:
- whether the endpoint takes `x-api-key`, `Authorization: Bearer`, and both;
- whether it refuses Claude Code's `anthropic-beta` values, and which;
- whether it answers `/v1/messages/count_tokens` and `GET /v1/models`;
- whether `GET /coding/v1/usages` answers the router's own User-Agent;
- what its error bodies look like, and whether they carry `request-id`;
- what Claude Code sends for `k3[1m]`, and whether it sends `effort` for an id it does not
  know;
- whether a conversation survives moving from `k3` to a Claude model and back. Claude's
  thinking blocks are signed; Kimi's are not, so Anthropic may refuse the history, and
  Claude Code's own recovery (drop the thinking blocks, retry) should then take over. If it
  does not, the router drops another provider's thinking blocks from the history it sends
  to Anthropic, and that is filed as its own item.

## What Kimi Code accepts (measured 2026-10-10)

With a Kimi Code console key on the owner's plan (Pro: K3 at 1M and HighSpeed included), from
the vault item "Kimi Code probe key (agent-router)", against `https://api.kimi.ai/coding`,
every request under Agent Router's own User-Agent. The bodies are kept in `fixtures/kimi/`.

**Auth header.** `x-api-key`, `Authorization: Bearer`, and both together are each taken with
200. The router sends `x-api-key`, as Kimi's Claude Code guide sets it; `/v1/usages` takes
either.

**Routes.**
- `POST /v1/messages/count_tokens` answers `{"input_tokens": 14}`.
- `GET /v1/models` lists the plan's four models in the Models API's shape, with Kimi's own
  names and more fields than Anthropic's (`context_length`, `supports_reasoning`,
  `think_efforts`): `kimi-for-coding` "K2.8 Preview" 1M, `kimi-for-coding-highspeed` "K2.7
  Code Highspeed" 256K, `k3` "K3" 1M, `k3-256k` "K3-256k" 256K (`models.json`).
- `GET /v1/usages` answers the router's User-Agent, in the newer shape: a count window
  `limits[0]` (300 minutes, `"limit": "100"`) and the pools `limit_5h`,
  `limit_month_total` and `limit_month_code`, each `{used_ratio, reset_time}`
  (`usages.json`). After a dozen requests it still read `used_ratio: 0` and
  `remaining: "100"`: its counters lag, or zero, as others have reported of it. A reading
  of 0 is not proof of an unspent window.

**Claude Code's request, unchanged.** Kimi Code takes exactly what Claude Code 2.1.267 sends
for `k3` (read off the wire by a stand-in server on loopback): `POST /v1/messages?beta=true`
with `anthropic-beta` `claude-code-20250219, context-1m-2025-08-07,
interleaved-thinking-2025-05-14, thinking-token-count-2026-05-13,
context-management-2025-06-27, prompt-caching-scope-2026-01-05,
mid-conversation-system-2026-04-07, effort-2025-11-24`, `thinking: {type: "adaptive",
display: "omitted"}`, `output_config: {effort: "high"}`, `context_management: {edits:
[{type: "clear_thinking_20251015", keep: "all"}]}`, `cache_control` on the system prompt
and `max_tokens: 32000`. Nothing is refused, and nothing needs stripping.
- `k3[1m]` goes on the wire as `k3` with the `context-1m-2025-08-07` beta.
- For an id it does not know, Claude Code still sends adaptive thinking and an effort, and
  says it keeps the session within 200K unless a `modelPicker` row maps the model with
  `behavesAs`, or `[1m]` is added.

**What comes back.**
- Anthropic's shape, streamed as Anthropic's events: `message_start`, `content_block_*`,
  `thinking_delta`, `signature_delta`, `text_delta`, `message_delta`, `message_stop`
  (`stream-k3.sse`). Its `data:` lines have no space after the colon, which capture's
  scanner reads.
- Kimi Code streams its thinking even when asked for `display: "omitted"`, and **signs its
  thinking blocks** (`signature`).
- Usage is Anthropic's flat form: `input_tokens`, `cache_creation_input_tokens`,
  `cache_read_input_tokens`, `output_tokens`, `output_tokens_details.thinking_tokens`, and
  `service_tier`. The final `message_delta` carries all of it.
- No `request-id`: `x-trace-id` instead, and no rate-limit headers.
- Errors are `{"error": {"message", "type"}}` with no top-level `"type": "error"`
  (`401-invalid-key.json`).

### Moving a conversation between providers

Measured with a real Claude Code session (2.1.267, a scratch `CLAUDE_CONFIG_DIR`) through
this router's build on loopback, with the probe key as its Kimi Code account and this Mac's
Claude Code login as its Anthropic one. One conversation, resumed turn by turn:
`k3` → `claude-haiku-5-5` → `k3` → `claude-sonnet-5-5` → `k3`.

- **Every request was served on its first try**, each by its own provider, all 200. No
  refusal, so Claude Code's recovery (drop the thinking blocks, retry) was never needed.
- **Kimi's signed thinking blocks went to Anthropic and were taken.** The first `k3` turn's
  thinking block, with Kimi's signature, is in the transcript Claude Code resent to Haiku.
- **Not exercised: Claude's signed thinking going to Kimi Code.** Neither Claude turn chose
  to think (adaptive thinking, `thinking_tokens: 0`), so no Anthropic-signed block reached
  Kimi Code. If one is ever refused, the router can drop another provider's thinking blocks
  from the history it forwards. That is filed only if it is seen to happen.
- A Claude subscription credential is used only by Claude Code's own requests. A request
  made by hand through the router was answered 429 `rate_limit_error`, with the window at
  49%, so the cross-provider check needs a real session, as here.

### Not measured

- **A spent window's 403.** It cannot be had without using up the owner's 5-hour window, which
  Kimi Code's own apps share. The owner chose to accept Kimi's documented bodies instead
  (2026-10-10): `fixtures/kimi/*.documented.json`, the measured error envelope with the
  message from Kimi's error reference. AGEN-0088 classifies those until the router records a
  real one.
- **The concurrency 403.** Kimi applies it by risk control, and appeals are by email, so it
  is not provoked on purpose.

## Clients

**Claude Code** (2.1.267, read from its binary and its gateway documentation on 2026-10-09):
- Anthropic's gateway guide says Claude Code is not supported against non-Claude models
  through a gateway. Kimi's own guide points Claude Code straight at Kimi Code, so it works
  in practice. It is outside Anthropic's documented envelope, and a Claude Code upgrade can
  change it.
- **Picking a model.** `/model k3` takes any id behind a custom base URL, and confirms it
  with one small request, which the router routes like any other. `--model`, `ANTHROPIC_MODEL`
  and the settings' `model` are not checked until the first request.
- **The `/model` picker.** Gateway discovery (`CLAUDE_CODE_ENABLE_GATEWAY_MODEL_DISCOVERY`)
  reads the router's `GET /v1/models` but **keeps only ids containing `claude` or
  `anthropic`**, so Kimi's models never reach the picker that way, and the router does not
  disguise them to get in. What does reach it: `modelPicker.options` in the user's
  `~/.claude/settings.json` (ignored in project settings), rows of `{model, label,
  description, behavesAs}` added after the built-in lineup. The installer and the machine
  agent write one row for each non-Claude model in the catalogue, and leave the owner's own
  rows alone.
- **Context window.** An id Claude Code does not know is assumed to hold 200K tokens, or 1M
  with `[1m]`. That is safe for `k3` and `k3-256k` (262,144). The 1M `k3` on a Pro plan is
  picked as `k3[1m]`; what that sends on the wire is still to measure.
- **Background work follows the main model.** On a gateway, titles and summaries use the
  main model, so a session on `k3` spends Kimi quota on them. `ANTHROPIC_DEFAULT_HAIKU_MODEL`
  pins them to a Claude id. A subagent with `model: inherit` runs where its session does.
- **Moving between providers mid-conversation.** When an upstream refuses a thinking
  block's signature, Claude Code drops the earlier thinking blocks and retries, matching on
  the upstream's error wording. The router forwards error bodies unchanged, so that recovery
  should work; it is measured before anything is built for it.
- **`count_tokens`** is optional for a gateway: if it is absent, Claude Code estimates from
  characters, and says so in `/context`.

**The console and the phones:**
- Settings adds a Kimi Code account from its key, shows each account's provider, and has a
  Models page: the catalogue with each model's account, assignable.
- The session model picker (`/api/live/options`) offers each provider's models beside
  Claude's aliases, grouped by provider, so a phone can move a session to `k3`.
- The Overview's All accounts card shows a Kimi Code account's windows. It never says
  "never seen" for an account that served requests, and never `$0` for one the router could
  not price.
- Model families gain `kimi`, with its own colour, beside opus, sonnet and haiku.

## Later

Not in this milestone, and not filed:
- other providers (Moonshot's metered Open Platform, GLM, DeepSeek): a generic
  "Anthropic-compatible" provider with a base URL, an auth style and a model list;
- other clients (Codex, OpenCode), which speak the OpenAI shape;
- pinning a session to an account;
- automatic overflow, which the owner declined.

## Tracked in Northstar

Milestone `sources` in project `agent-router`, filed 2026-10-09. Each task is a child of
its epic, and "waits on" is a `blocks` edge drawn from the item that waits.

| Item | What | Waits on |
| --- | --- | --- |
| **AGEN-0078** | Epic: sources, and routing by model | |
| AGEN-0081 | This plan, and ARCHITECTURE.md's opening | |
| AGEN-0082 | An account names its provider; Kimi Code accounts; one active account per provider | |
| AGEN-0083 | The catalogue: `/api/models`, `/api/models/assign`, `/v1/models` | 0082 |
| AGEN-0084 | Routing `/v1/messages` and `count_tokens` by model, signed the provider's way | 0082, 0083 |
| AGEN-0085 | No Anthropic-only call with another provider's key | 0082 |
| **AGEN-0079** | Epic: Kimi Code as a source | |
| AGEN-0086 | Measure what Kimi Code's endpoint accepts, with a real key | 0084 |
| AGEN-0087 | Kimi Code's quota windows from `/coding/v1/usages` | 0082, 0085, 0086 |
| AGEN-0088 | A Kimi Code 403 marks the account spent until its window resets | 0084, 0086 |
| AGEN-0089 | Kimi Code's models priced at Moonshot's Open Platform rates | |
| **AGEN-0080** | Epic: every client picks a source by picking a model | |
| AGEN-0090 | The session model picker offers every provider's models | 0083 |
| AGEN-0091 | Claude Code's `/model` gets a row for each non-Claude model | 0083 |
| AGEN-0092 | Console: Kimi Code accounts, the Models page, the picker, the cards | 0082, 0083, 0087, 0090 |
| AGEN-0093 | iOS: the same | 0082, 0087, 0090 |
| AGEN-0094 | Android: the same | 0082, 0087, 0090 |
| AGEN-0095 | End to end on the hosted router: `/model k3`, then back to Claude | 0084, 0085, 0086, 0091, 0092 |

AGEN-0083 and AGEN-0084 also relate to AGEN-0086: its findings settle Kimi Code's model list
and which headers pass. The measuring itself starts as soon as there is a Kimi Code key in the
vault. Only its last part, a conversation moving between providers, needs AGEN-0084.
