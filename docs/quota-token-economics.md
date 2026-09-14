# Where the subscription quota goes, and what would stretch it

Finding, 2026-09-14. Measured on the router's own capture for the 14 days ending
2026-09-14: 11,352 successful message requests, 97% of them Opus 5 with the 1M context
window, across the project-a, agent-router and project-b projects, all through the VS Code
extension. The question was what the router could do to save tokens now that it sits
between Claude Code and Anthropic. The answer is not caching. Claude Code's caching is
already at its ceiling, and cache rewrites are 6% of the quota. The 5-hour meter is spent
on ordinary turns, and what an ordinary turn costs is decided by how much context it
re-reads and how much it writes back. Stretching the quota means smaller contexts, fewer
turns and less output.

## The short version

- **Ordinary turns are 87% of the meter.** Two thirds of it went to turns whose context
  was above 200K tokens, 44% to turns above 400K.
- **A turn at 700K costs 2.4 times a turn at 150K.** Roughly half of a turn's cost
  tracks the context it re-reads; the rest tracks the output it produces, thinking
  included.
- **Caching is done.** 98.9% of prompt tokens on Opus 5 were served from cache. Claude
  Code places 1-hour breakpoints itself and never edits the prefix mid-session.
- **Cache rewrites are small.** Resuming after an hour idle, account switches, forks and
  new sessions together took 6%.
- **Web search on Fable 5.1 is the one expensive tool.** 70 searches moved the meter by
  21 points in 14 minutes, about 11 ordinary turns per search.
- **Every VS Code request carries 130K to 150K characters of tool definitions**, about
  12% of the context. Nearly all of it is the harness's built-in tool set.
- **The levers, largest first:** compact earlier and keep exploration out of the main
  thread; run at a lower effort than `max`; make fewer turns; deny WebSearch where it is
  not needed; drop the Google Drive connector per project; fix the router's own fork
  flags and make account switches session-sticky.

## How the meter was read

Every response carries `anthropic-ratelimit-unified-5h-utilization`, the share of the
5-hour window used so far, rounded to a whole percent. The router stores it per request.
The cost of a request is the difference between its reading and the previous reading on
the same account in the same window. One reading is worthless, since most differences
are 0 and a few are 1. Averaged over hundreds of turns of the same kind the rounding
cancels, and that is how every number below was produced. Two caveats: several sessions
run at once, so a long turn can absorb a neighbour's step; and a regression of the
per-request steps on token counts was unstable and is not used, only grouped averages
are.

"Points" below are percentage points of the 5-hour window. 100 points is one full window.

## Where 14 days of the meter went

| Spend | Requests | Points |
| --- | --- | --- |
| Ordinary turns, context under 200K | 3,293 | 50 |
| Ordinary turns, 200K to 400K | 3,558 | 75 |
| Ordinary turns, 400K to 600K | 2,552 | 67 |
| Ordinary turns, over 600K | 1,744 | 65 |
| One burst of 70 web searches on Fable 5.1 | 124 sub-requests | 19 |
| Turns after a 5 to 60 minute pause | 213 | 7 |
| Resuming after more than an hour idle | 32 | 6 |
| First request of a fork or resumed session | 8 | 2 |
| First request of a fresh session | 34 | 2 |
| Account switched mid-session | 8 | 1 |

About 300 points in all, three full windows. Subagent requests were 139 in total, 124 of
them the web search burst.

## What one turn costs

Ordinary turns only: less than five minutes after the previous turn, same account, under
20K tokens written to cache.

| Context re-read | Turns | Average output tokens | Points per turn |
| --- | --- | --- | --- |
| Under 100K | 708 | 679 | 0.016 |
| 100K to 200K | 2,585 | 1,000 | 0.015 |
| 200K to 400K | 3,558 | 988 | 0.021 |
| 400K to 600K | 2,552 | 1,028 | 0.026 |
| Over 600K | 1,744 | 1,080 | 0.037 |

A straight line through the bands is about 0.010 points per turn plus 0.035 per million
tokens of context. At the average turn, 366K of context and 1,000 output tokens, the
context part is about half. The remainder is what a turn costs regardless of size, and
the only thing that varies there is output. Within one context band, turns with 1K to 3K
of output cost more than turns with 300 to 1K, but that comparison is noisy, so the
output weight is inferred rather than measured.

The other kinds of turn, for comparison:

| Kind of turn | Turns | Written to cache | Points per turn |
| --- | --- | --- | --- |
| Ordinary | 11,202 | 2K | 0.024 |
| After a 5 to 60 minute pause | 213 | 4K | 0.033 |
| Resume after more than an hour | 32 | 405K | 0.188 |
| First request of a fork or resume | 8 | 344K | 0.25 |
| Account switched mid-session | 8 | 433K | 0.125 |

A full rewrite of a 400K context costs about eight ordinary turns. The rewrite levers
are small because there are few rewrites, not because a rewrite is cheap.

## What the context is made of

Average composition of a main-thread request, tokens apportioned by character share:

| Component | Characters | Tokens | Share |
| --- | --- | --- | --- |
| Conversation history | 727K | 222K | 61% |
| Tool results | 311K | 91K | 25% |
| Tool definitions | 130K | 44K | 12% |
| System prompt | 12K | 4K | 1% |

**Tool definitions.** The list is the same in every VS Code session: 130,598 characters
on average in project-a, 127,585 in agent-router, up to 150,268. It starts with `Agent`
(8,589 characters) and `Artifact`, and 89 of the 95 captured slices that show the start
of the list contain `Artifact`. The MCP part is small: the Google Drive connector, 11
tools and about 15K characters, and primeng, 8 tools and about 4K characters. Because
`ANTHROPIC_BASE_URL` is set, Claude Code switches off its MCP tool search and loads every
MCP schema upfront; see the companion document.

**Tool results**, one row per tool call, 14 days:

| Tool | Calls | Tokens carried | Average | Largest |
| --- | --- | --- | --- | --- |
| Bash | 11,514 | 4.47M | 389 | 17K |
| Read | 102 | 3.76M | 36.8K | 150K |
| WebSearch | 51 | 104K | 2.0K | 4.3K |
| SendMessage | 718 | 66K | 92 | 158 |
| WebFetch | 68 | 61K | 892 | 9.8K |

**Tool inputs**, what the assistant wrote into tool calls, from the transcripts of every
session in the period:

| Tool | Calls | Tokens | Average |
| --- | --- | --- | --- |
| Bash | 51,343 | 12.9M | 252 |
| SendMessage | 4,504 | 3.84M | 853 |
| Write | 457 | 820K | 1.8K |
| Workflow | 212 | 680K | 3.2K |
| Edit | 1,353 | 589K | 435 |

Once in the history, every one of these is re-read on every later turn of the session.
A 50K result that stays for 200 turns costs about 0.35 points on its own.

**Sessions.** 68% of turns happened past turn 150 of their session, at 427K of context on
average. The longest session, in project-a, ran 2,603 turns, peaked at 999K, and spent 1,227
turns above 400K; those turns took 37 of its 69 points. Contexts dropped by more than 40%
between consecutive turns 44 times in the period, from 324K to 80K on average, which is a
compaction or a clear.

## Caching is already done right

| Model | Requests | Read from cache | Written to cache | Uncached | Hit rate |
| --- | --- | --- | --- | --- | --- |
| Opus 5 | 10,964 | 3,957M | 44.7M | 0.4M | 98.9% |
| Fable 5.1 | 330 | 96.3M | 5.2M | 2.9M | 92.2% |
| Haiku 4.5 | 50 | 0.9M | 0.3M | 0.01M | 75.7% |

Claude Code puts 1-hour breakpoints on the system prompt and on the last message block,
sends operator instructions as `role: system` messages inside `messages[]` rather than
editing the prefix, and clears thinking server-side. Turn over turn, reads cover the
whole prior prefix and writes are one turn's delta: 0.6% of prompt tokens written on
turns under five minutes apart, 1.1% on turns 5 to 60 minutes apart, 96% on turns more
than an hour apart. The Fable uncached tokens are the web search results, which arrive as
input and are never cached.

Where the 49.7M cache-write tokens of the period came from, counting only rewrites of
more than half the prompt on prompts above 20K:

| Cause | Rewrites | Tokens |
| --- | --- | --- |
| Resuming after more than an hour idle | 40 | 18.0M |
| Mid-session, same account and model, cause unknown | 59 | 5.3M |
| Account switched mid-session | 10 | 4.7M |
| First request of a session | 7 | 2.7M |
| Model switched mid-session | 2 | 0.3M |

The remaining 18.7M is ordinary turn-over-turn growth. Of the 59 unexplained rewrites,
34 kept the tools-and-system hit and lost the messages cache, which points at compaction,
an effort change or an early history edit; 25 lost everything, which points at a tool
list change. The stored body slice cannot tell which: Claude Code serialises `messages`
before `system` and `tools`, so the router's 64KB cap never reaches them on a long
session.

## The levers, largest first

1. **Carry less context per turn.** Two thirds of the meter is turns above 200K. The
   knobs are Claude Code's: `autoCompactWindow` sets how full the context gets before it
   compacts, and with `opus[1m]` as the model auto-compaction otherwise waits until near
   a million; `/compact` takes focus instructions. Exploration belongs in subagents: only
   7 Agent calls ran in the period, while 102 Read calls averaging 37K tokens stayed in
   their sessions' context to the end. A subagent that returns a paragraph costs a few
   small turns; the result it keeps out of the main thread would have cost 0.035 points
   per million tokens on every later turn.
2. **Spend less on output.** Thinking is 33% of Opus 5 output and 47% of Fable's. The
   user settings pin `effortLevel` to `max`; the documented default for Opus 5 and Fable
   5.1 is `high`. Anthropic's Opus 5 coding runs gave up about 2 points at `medium` for
   half the cost per task. The transcripts show flat output per response across `high`,
   `xhigh` and `max` on Opus 5 (915, 994 and 942 tokens, about one tool call each), so a
   saving would come from fewer turns per task, which the capture cannot measure. A week
   at `high` on project-a, set per project with `modelSettings`, would.
3. **Make fewer turns.** 93.5% of turns end in a tool call, and each is a full re-read.
   Chained commands and parallel tool calls in one response cut turns directly. Agent
   team traffic is a multiplier: 4,504 SendMessage calls in the period, each at least one
   full-context turn on both ends.
4. **No web search on Fable 5.1.** 0.27 points per search, about 11 ordinary turns. There
   is no per-tool model setting; `permissions.deny` can remove WebSearch from projects
   that never need it, and WebFetch on a known URL is far cheaper.
5. **Trim tool definitions.** The built-in set is not configurable.
   `disableClaudeAiConnectors` in a project's settings removes the Drive connector where
   it is unused, about 5K tokens per turn. Whether `ENABLE_TOOL_SEARCH` can force
   deferral back on behind a custom base URL is worth one test, and if it can, the
   router's setup can write it next to the base URL it already writes. Worth about 2%
   today, more as MCP servers grow.
6. **Rewrites.** About 6% together, so cheap fixes only: a sticky account per session,
   and the aside fork. `live::aside` passes `--tools ""` and `--append-system-prompt`,
   both of which change the cached prefix, so every aside rewrites the whole context at
   about ten turns' cost. Putting the instruction in the user message and capping with
   `--max-turns` would let the fork read the parent's cache. Keep-alive pings against the
   1-hour cache would address 2% and are not worth synthetic traffic.

## What the router should build

Measurement and control, no byte rewriting. Rewriting bodies, whether injecting context
editing or stripping tools, does not beat compaction on this meter, and it would break
the one invariant that makes the router safe.

- **Cost per turn in the live view and on the phone**, split into context and output,
  with a compact button through the wrapper and a nudge when a session crosses 300K.
- **A session report**: turns above 400K and the share of the window they took, visible
  while the session is still running.
- **Cost per tool, including server tools.** The SSE scanner in `analyze.rs` records only
  `tool_use` blocks, so web search never appears on the dashboard.
- **Rewrite attribution.** Hash `model`, `thinking`, `effort`, `system`, `tools` and each
  message in `analyze_request`, store the fingerprints, and diff consecutive requests to
  name the cause of a rewrite. The router's own effort and model controls are triggers
  and should say what they will re-read before sending.
- **The fork flags and a sticky account per session**, both small changes in the router's
  own code.

## Unverified

- The output-token weight of the meter is inferred by subtraction, not measured
  directly; the within-band comparison is too noisy to pin it.
- Whether the meter charges web searches per search or per uncached input token; the
  burst fits either.
- Whether the meter weights models differently; 97% of requests were Opus 5, so the fit
  is an Opus 5 fit.
- The cause of the 59 unexplained rewrites; prefix fingerprints would settle it.
- Whether `ENABLE_TOOL_SEARCH` overrides the custom-base-URL rule.
- The unit of `autoCompactWindow`; the settings page was truncated before its entry.
- A `max_tokens: 0` keep-alive was not tested on the subscription's OAuth path.

## Appendix: reproducing the numbers

All queries run read-only against the router's database:
`sqlite3 "file:$HOME/.agent-router/agent-router.db?mode=ro"`. Paths carry `?beta=true`,
so match with `LIKE`. Timestamps are milliseconds.

```sql
-- Main-thread turns with their predecessor in the same session.
CREATE TEMP VIEW seq AS
  SELECT r.id, r.session_id, r.started_at, r.account_id, r.model,
         lag(r.started_at) OVER w prev_start, lag(r.account_id) OVER w prev_acct,
         lag(r.model) OVER w prev_model,
         u.prompt_tokens, u.cache_read_tokens cread,
         u.cache_creation_5m_tokens + u.cache_creation_1h_tokens cc,
         u.input_tokens inp, u.output_tokens out
  FROM requests r JOIN usage u ON u.request_id = r.id
  WHERE r.path LIKE '/v1/messages%' AND r.path NOT LIKE '%count_tokens%'
    AND r.status = 200 AND r.agent_id IS NULL AND r.session_id IS NOT NULL
    AND r.started_at > (strftime('%s','now') - 14*86400) * 1000
  WINDOW w AS (PARTITION BY r.session_id ORDER BY r.started_at);

-- Each request's step on the 5h meter: its reading minus the previous reading on the
-- same account in the same window, over every request including subagents.
CREATE TEMP VIEW meter AS
  SELECT r.id, r.rl_5h_utilization u,
         lag(r.rl_5h_utilization) OVER (PARTITION BY r.account_id, r.rl_5h_reset
                                        ORDER BY r.started_at) pu
  FROM requests r
  WHERE r.path LIKE '/v1/messages%' AND r.path NOT LIKE '%count_tokens%'
    AND r.status = 200 AND r.rl_5h_utilization IS NOT NULL
    AND r.started_at > (strftime('%s','now') - 14*86400) * 1000;

-- Points per turn by kind of turn.
SELECT CASE WHEN s.prev_start IS NULL AND s.prompt_tokens > 50000 THEN 'fork or resume'
            WHEN s.prev_start IS NULL THEN 'fresh session'
            WHEN s.prev_acct <> s.account_id THEN 'account switch'
            WHEN s.prev_model <> s.model THEN 'model switch'
            WHEN s.started_at - s.prev_start >= 3600000 THEN 'resume after >60m'
            WHEN s.started_at - s.prev_start >= 300000 THEN 'after 5-60m pause'
            ELSE 'ordinary' END kind,
       count(*) n, round(avg((m.u - m.pu) * 100), 3) avg_pp,
       round(sum((m.u - m.pu) * 100), 1) total_pp, round(avg(s.cc)) avg_written
FROM seq s JOIN meter m ON m.id = s.id
WHERE m.pu IS NOT NULL AND m.u >= m.pu
GROUP BY 1 ORDER BY total_pp DESC;

-- Points per ordinary turn by context band.
SELECT CASE WHEN s.prompt_tokens < 100000 THEN '<100k'
            WHEN s.prompt_tokens < 200000 THEN '100-200k'
            WHEN s.prompt_tokens < 400000 THEN '200-400k'
            WHEN s.prompt_tokens < 600000 THEN '400-600k'
            ELSE '600k+' END band,
       count(*) n, round(avg(s.prompt_tokens)) avg_prompt, round(avg(s.out)) avg_out,
       round(avg((m.u - m.pu) * 100), 3) avg_pp, round(sum((m.u - m.pu) * 100), 1) total_pp
FROM seq s JOIN meter m ON m.id = s.id
WHERE m.pu IS NOT NULL AND m.u >= m.pu AND s.prev_start IS NOT NULL
  AND s.started_at - s.prev_start < 300000 AND s.prev_acct = s.account_id AND s.cc < 20000
GROUP BY 1 ORDER BY min(s.prompt_tokens);

-- Big rewrites and their cause.
SELECT CASE WHEN prev_start IS NULL THEN 'first request'
            WHEN prev_acct <> account_id THEN 'account switch'
            WHEN prev_model <> model THEN 'model switch'
            WHEN started_at - prev_start >= 3600000 THEN 'gap > 60m'
            WHEN started_at - prev_start >= 300000 THEN 'gap 5-60m'
            ELSE 'other' END why, count(*) n, sum(cc) tokens
FROM seq WHERE cc > 0.5 * prompt_tokens AND prompt_tokens > 20000
GROUP BY 1 ORDER BY tokens DESC;

-- What the context is made of.
SELECT c.component, round(avg(c.char_count)) chars, round(avg(c.est_tokens)) tokens
FROM context c JOIN requests r ON r.id = c.request_id
WHERE r.agent_id IS NULL AND r.status = 200
  AND r.started_at > (strftime('%s','now') - 14*86400) * 1000
GROUP BY 1 ORDER BY chars DESC;

-- Tool results by tool, one row per call.
SELECT name, count(*) calls, round(sum(result_chars) / 4.0) tokens,
       round(avg(result_chars) / 4.0) avg
FROM tool_calls WHERE seen_at > (strftime('%s','now') - 14*86400) * 1000
GROUP BY 1 ORDER BY tokens DESC;

-- The web search burst, local time.
SELECT min(rl_5h_utilization) before, max(rl_5h_utilization) after, count(*) requests
FROM requests WHERE status = 200 AND path LIKE '/v1/messages%'
  AND started_at BETWEEN strftime('%s','2026-09-13 21:09:00','utc') * 1000
                     AND strftime('%s','2026-09-13 21:23:00','utc') * 1000;
```

Tool inputs and the effort comparison come from the transcripts under
`~/.claude/projects/*/*.jsonl`: every `assistant` line carries `message.usage`,
`message.model`, the per-turn effort, and the `tool_use` blocks with their `input`.
Claude Code writes one line per content block, so deduplicate by `message.id` before
counting output tokens, and by the `tool_use` id before summing inputs.

## Sources

- The router's own database, `~/.agent-router/agent-router.db`, requests from
  2026-08-31 to 2026-09-14, and the session transcripts under `~/.claude/projects`.
- Captured request bodies for the wire shape and breakpoint placement; the beta list
  from the router's header dump at startup.
- User settings in `~/.claude/settings.json` (`effortLevel`, `model`) and MCP
  configuration in `~/.claude.json`.
- Anthropic's cost guidance and measured effort curves:
  https://platform.claude.com/docs/en/about-claude/models/optimizing-for-cost-and-intelligence.md
- Prompt caching rules and economics:
  https://platform.claude.com/docs/en/build-with-claude/prompt-caching.md
- Claude Code settings: https://code.claude.com/docs/en/settings-reference.md; MCP and
  tool search: https://code.claude.com/docs/en/mcp.md; model and effort:
  https://code.claude.com/docs/en/model-config.md; CLI flags:
  https://code.claude.com/docs/en/cli-reference.md
- The companion reference in this folder:
  [Prompt caching and Claude Code settings](prompt-caching-and-claude-code-settings.md).
