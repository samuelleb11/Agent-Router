# Prompt caching and Claude Code settings, as they affect the router

Research, 2026-09-14. Collected while measuring where the subscription quota goes, see
[Where the subscription quota goes](quota-token-economics.md). Three parts: what Claude
Code actually puts on the wire through the router, the prompt caching rules that decide
what a turn costs, and the Claude Code settings and flags that control the levers. The
API facts were read from the API reference bundled with Claude Code (cached 2026-06-24)
and should be re-checked against the live pages in the sources before anything is built
on them.

## What Claude Code sends through the router

Observed on Claude Code 2.1.266 to 2.1.270 through the VS Code extension.

- **Betas on every messages request:** `claude-code-20250219`, `oauth-2025-04-20`,
  `context-1m-2025-08-07`, `interleaved-thinking-2025-05-14`,
  `thinking-token-count-2026-05-13`, `context-management-2025-06-27`,
  `prompt-caching-scope-2026-01-05`, `mid-conversation-system-2026-04-07`,
  `mid-conversation-tool-changes-2026-07-01`, `advisor-tool-2026-03-01`,
  `effort-2025-11-24`, `fallback-credit-2026-06-01`, `extended-cache-ttl-2025-04-11`.
  Token-count requests carry `token-counting-2024-11-01` instead. The path is
  `/v1/messages?beta=true`.
- **Body key order:** `model`, `messages`, `system`, `tools`, `metadata`, `max_tokens`,
  `thinking`, `context_management`, `output_config`, `stream`. Messages come first, so a
  body slice capped at 64KB never reaches `system` or `tools` on a long session.
- **The prefix:** `system[0]` is a short billing block,
  `x-anthropic-billing-header: cc_version=2.1.270.c53; cc_entrypoint=claude-vscode;`,
  with `cc_is_subagent=true` on subagent requests. The version suffix changes with CLI
  updates. Then the agent identity line, a short "Reporting outcomes" block, and the
  main interactive-agent prompt of about 11.7K characters.
- **Breakpoints:** `cache_control: {type: ephemeral, ttl: 1h}` on `system[1]`, on the
  last system block, and on the last content block of the last message. No explicit
  marker on tools; the system marker covers them, since tools render first.
- **Operator instructions mid-session** go in as `{"role": "system", ...}` entries in
  `messages[]` (the environment block, environment updates), not as edits to the
  top-level prompt. This is what keeps the prefix stable turn over turn.
- **Other parameters:** `thinking: {type: adaptive, display: omitted}` on Opus 5,
  `output_config: {effort: ...}`, `max_tokens: 64000`,
  `context_management: {edits: [{type: clear_thinking_20251015, keep: all}]}`,
  `stream: true`, and `metadata.user_id` carrying the device id, account uuid and
  session id as JSON.
- **Tools:** every VS Code request carries the full built-in tool list, 130K to 150K
  characters, plus the MCP tools. One captured session in `/private/tmp` carried only
  the 19 MCP tools (Google Drive and primeng, 19K characters); why that session differed
  was not established.
- **Subagents** carry `x-claude-code-agent-id`; the main thread does not. Their system
  prompt is not the main thread's, so they share no cache with it.
- **Web search** runs as sub-requests on the session's model: a 2KB body, "Perform a web
  search for the query: ...", no `cache_control`, and the results come back as tens of
  thousands of uncached input tokens per call. They appear in the stream as
  `server_tool_use` blocks, which the router's SSE scanner does not record.
- **Rate-limit headers** on every response: `anthropic-ratelimit-unified-status`,
  `-5h-utilization`, `-5h-reset`, `-7d-utilization`, `-7d-reset`, utilization rounded to
  a whole percent.

## Prompt caching rules that matter here

- **Prefix match.** The cache key is the exact bytes up to each breakpoint. Any change
  earlier in the prefix invalidates everything after it. Render order is tools, then
  system, then messages. Up to 4 breakpoints per request; a top-level automatic
  `cache_control` places one on the last cacheable block and moves it forward.
- **Minimum cacheable prefix:** 512 tokens on Opus 5 and Fable 5.1, 1,024 on Opus 4.8
  and Sonnet 5, 2,048 on Opus 4.7, 4,096 on Opus 4.6 and Haiku 4.5. Shorter prefixes
  silently do not cache.
- **Economics at API list price:** reads cost 0.1 times input, 0.025 on Fable 5.1;
  writes 1.25 times for the 5-minute TTL, 2 times for the 1-hour TTL. The router's
  `pricing.rs` table carries these multipliers. The subscription meter has its own
  weights; the finding document gives the measured per-turn costs.
- **TTL:** a read refreshes the timer at no cost, on either TTL. The lifetime runs from
  the start of the request that writes or reads. Requests less than five minutes apart
  keep a 5-minute entry warm indefinitely; the 1-hour TTL pays only for gaps of 5 to 60
  minutes. Claude Code uses the 1-hour TTL everywhere.
- **Keep-alive:** a request with `max_tokens: 0` runs prefill, writes or refreshes the
  cache, returns `content: []` with `stop_reason: max_tokens`, and bills no output. It is
  rejected with `stream: true`, structured outputs, forced `tool_choice`,
  `thinking: {type: enabled}`, and inside batches; adaptive thinking is accepted.
  Anthropic recommends it over the 1-hour TTL on Fable 5.1 for 5 to 60 minute gaps. Not
  tested through the subscription.
- **Concurrency:** an entry becomes readable only after the first response begins
  streaming, so N parallel requests with the same prefix all pay the write. Fan-outs
  should send one, wait for the first token, then send the rest.
- **Isolation:** caches are per workspace on the Claude API and never shared across
  organizations. Switching the router's active account mid-session is therefore a full
  cache miss on the next request; measured at 95% of the prompt rewritten.
- **Invalidation hierarchy:** three tiers, tools, system, messages. Changing tool
  definitions or the model rebuilds all three. Changing the system prompt keeps the
  tools cache. Changing `thinking` or `effort` always invalidates the messages cache, and
  on some models the tiers above. Message content changes touch only messages. Escape
  hatches: `role: system` messages in `messages[]` for operator instructions (no beta;
  Opus 5, Opus 4.8, Fable 5 and 5.1, not Sonnet 5); `tool_addition` and `tool_removal`
  blocks behind `mid-conversation-tool-changes-2026-07-01`; a per-message effort change
  behind `mid-conversation-output-config-2026-07-01` on Fable 5.1 and Opus 5;
  turn-scoped reminders with `clear_at: next_user_message` behind
  `mid-conversation-system-clear-at-2026-08-21`.
- **Thinking blocks** are preserved across turns on Opus 4.5 and later, Sonnet 4.6 and
  later, and Fable, so a plain user turn after tool use does not break the cache. On
  Fable 5.1, editing earlier turns invalidates later thinking blocks; harnesses must be
  append-only.
- **20-block lookback:** a breakpoint looks back at most 20 positions for a prior entry.
  A run of consecutive `tool_use` blocks counts as one position, as does a run of
  `tool_result` blocks, so parallel calls are safe; a turn that appends more than 20
  positions of other content makes the next request miss.
- **Verifying:** `usage.cache_read_input_tokens` should cover the whole prior prefix and
  `cache_creation_input_tokens` should be one turn's delta. `usage.cache_creation` splits
  writes by TTL. To localise a break, diff consecutive request bodies with the moving
  marker stripped, or send `cache-diagnosis-2026-04-07` on every request with
  `diagnostics.previous_message_id` and read the response's `diagnostics`.
- **Context editing** (`clear_tool_uses_20250919`) clears old tool results server-side
  but rewrites the cached conversation on every pass; in Anthropic's own measurement it
  cost more than it saved. **Compaction** (`compact-2026-01-12`) summarises server-side
  and cut a long triage run's bill by a further 38% where it fired.

## Anthropic's cost levers, in the order they recommend

Free wins first, tradeoffs last, judged in cost per completed task.

1. Prompt caching, permanently: it cut agent-loop cost by 2.5 to 3.7 times at 81% to 90%
   hit rates in their runs.
2. Input hygiene: reference material behind tools, tool search with `defer_loading` once
   schemas pass about 10K tokens, narrow accessors, programmatic tool calling (24% fewer
   input tokens on agentic search), and an audit of prompts written for older models
   (prompts written for Opus 4.8 cost 36% more per ticket on Opus 5 for no accuracy
   change).
3. Loop hygiene: compaction for long sessions, client-side pruning at natural
   boundaries, subagents for self-contained bulky steps.
4. Output: `max_tokens` as a backstop only; shorten visible output by specifying the
   shape; shorten reasoning with effort, not with `max_tokens`.
5. Batch: 50% off everything for work no one waits on. Not applicable to interactive
   sessions.
6. Effort, then budgets: research work is nearly flat (`medium` matched the default at
   70% to 85% of its cost); long-horizon coding on Opus 5 gave up about 2 points at
   `medium` for half the cost and 8 points at `low` for a quarter; re-running failures at
   higher effort held the pass rate for about half the cost. Task budgets are advisory
   and invalidate the cache if changed mid-task.
7. Model choice last: price the tail, not the median; the frontier model at lower effort
   often beats a cheaper model at high effort per completed task.

## Claude Code settings and flags found

| What | Setting or flag | Where | Note |
| --- | --- | --- | --- |
| Compact earlier | `autoCompactWindow` | any settings file, or `--autocompact` | "how full the context gets before Claude Code compacts"; the unit was not visible on the truncated page |
| Auto-compaction off or on | `autoCompactEnabled` | any settings file | |
| Compact now, with focus | `/compact [instructions]` | in session | summarises, then re-reads recently modified files and invoked skills; preserving specific state is not documented |
| Effort default | `effortLevel` | any settings file | the user setting is `max` today; the documented default is `high` for Opus 5, Fable 5.1, Opus 4.8 and Sonnet 5, `xhigh` for Opus 4.7 |
| Effort per model, per project | `modelSettings` | any settings file, project or local | the per-project override |
| Cap effort | `maxEffortLevel` | any settings file | |
| Project `.mcp.json` servers | `enabledMcpjsonServers`, `disabledMcpjsonServers`, `enableAllProjectMcpServers` | any settings file | approval and blocking of project-defined servers |
| claude.ai connectors | `disableClaudeAiConnectors: true`, or `ENABLE_CLAUDEAI_MCP_SERVERS=false` | any settings file, env | true in any scope wins; `deniedMcpServers` blocks single connectors by name or URL |
| Per-project toggle of any server | the `/mcp` panel, recorded in `disabledMcpServers` and `enabledMcpServers` in `~/.claude.json` | in session | unrelated to the `.mcp.json` keys |
| Only explicitly passed servers | `--strict-mcp-config` with `--mcp-config` | CLI | |
| MCP tool search | `ENABLE_TOOL_SEARCH` | env | on by default; off whenever `ANTHROPIC_BASE_URL` is set, so every routed session loads all MCP schemas upfront; `false` also disables it |
| MCP discovery cache | `MCP_DISCOVERY_CACHE=1` | env | caches HTTP and SSE tool lists |
| Limit a headless run | `--max-turns` | CLI | exits with an error at the limit |
| Extra system prompt | `--append-system-prompt` | CLI | appends to the system prompt sent to the API, so it changes the cached prefix |
| Restrict tools | `--tools` | CLI | "which tools are available"; whether `""` removes built-ins is not documented |
| Block WebSearch | `permissions.deny` with `WebSearch` | any settings file | no setting runs it on another model |
| 1M context window | none found | | no documented way to force a 200K window; `autoCompactWindow` is the substitute |

The user's settings at the time: `effortLevel: max`, `model: opus[1m]`, `env` carrying
`ANTHROPIC_BASE_URL`. MCP at user scope is primeng over stdio; the Google Drive connector
comes from claude.ai; there is no project-scope MCP in project-a, project-b or agent-router.

## Unverified

- `--tools ""` semantics; the values of `ENABLE_TOOL_SEARCH` and its 10% threshold rule
  (the documentation section was truncated); the unit of `autoCompactWindow`.
- Whether `ENABLE_TOOL_SEARCH=true` can override the custom-base-URL rule.
- Availability of `cache-diagnosis-2026-04-07` and of `max_tokens: 0` on the
  subscription's OAuth path.
- Why one captured session carried only MCP tools while every VS Code session carried
  the full built-in list.
- Whether the subscription meter follows API price ratios per token type; the finding
  document gives measured per-turn costs instead.

## Sources

- Prompt caching: https://platform.claude.com/docs/en/build-with-claude/prompt-caching.md
- Pricing: https://platform.claude.com/docs/en/pricing.md
- Cost optimization and measured effort curves:
  https://platform.claude.com/docs/en/about-claude/models/optimizing-for-cost-and-intelligence.md
- Effort: https://platform.claude.com/docs/en/build-with-claude/effort.md
- Context editing: https://platform.claude.com/docs/en/build-with-claude/context-editing.md
- Compaction: https://platform.claude.com/docs/en/build-with-claude/compaction.md
- Tool search: https://platform.claude.com/docs/en/agents-and-tools/tool-use/tool-search-tool.md
- Rate limits: https://platform.claude.com/docs/en/api/rate-limits.md
- Claude Code settings reference: https://code.claude.com/docs/en/settings-reference.md
- Claude Code MCP, tool search, connectors: https://code.claude.com/docs/en/mcp.md
- Claude Code sessions and `/compact`: https://code.claude.com/docs/en/sessions.md
- Claude Code CLI flags: https://code.claude.com/docs/en/cli-reference.md
- Claude Code model and effort configuration: https://code.claude.com/docs/en/model-config.md
- Claude Code context window: https://code.claude.com/docs/en/context-window.md
- Claude Code headless use: https://code.claude.com/docs/en/headless.md
- Wire observations: request bodies and headers captured by the router, 2026-08-31 to
  2026-09-14.
