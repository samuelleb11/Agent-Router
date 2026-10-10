# Kimi Code response bodies

What Kimi Code's Anthropic-compatible endpoint (`https://api.kimi.ai/coding`) answers, for the
router's tests. The measuring is in docs/sources-and-models.md, "What Kimi Code accepts".

**Recorded** on 2026-10-10 with a console key on the owner's plan, nothing changed but
indentation. No key is in any of them.

| File | What |
| --- | --- |
| `usages.json` | `GET /v1/usages`, the newer shape: a 5-hour count window and the `limit_5h`, `limit_month_total` and `limit_month_code` pools |
| `models.json` | `GET /v1/models`: the plan's four models |
| `message.json` | `POST /v1/messages` to `kimi-for-coding`, not streamed: a signed thinking block, the text, and flat usage |
| `stream-k3.sse` | `POST /v1/messages?beta=true` to `k3` with Claude Code's own request shape, streamed |
| `401-invalid-key.json` | an invalid key's refusal |

**Documented, not recorded.** A spent window's 403 cannot be recorded without using up the
owner's window, which Kimi Code's own apps share, and the concurrency 403 is applied by risk
control. Each body is the error envelope measured on the 401 with the message from Kimi's
error reference (kimi.com/code/docs, read 2026-10-09). The reference gives no `type` for them,
so none is written. Replace one with the real body when the router captures it.

| File | Message for |
| --- | --- |
| `403-five-hour.documented.json` | a spent 5-hour window |
| `403-weekly.documented.json` | a spent weekly window (legacy plans) |
| `403-monthly.documented.json` | a spent monthly total |
| `403-concurrency.documented.json` | the concurrent request limit, which is not a spent quota |

## Kimi Code's own client

**Recorded** on 2026-10-10 from Kimi Code 2.1.1's own server (`kimi web`) on a scratch
`KIMI_CODE_HOME`, its model a stand-in (`scripts/kimi-standin.py`), so no tokens were spent
and nothing of the owner's Kimi Code was touched. The scratch paths are replaced: the
session's directory reads `/Users/u/code/demo` and the home `/Users/u/.kimi-code`. Nothing
else was changed.

One session, started in a directory, its model set to `agent-router/k3`, asked "Run echo,
please.": the stand-in calls Bash with `echo hello-from-standin`, Kimi Code asks for
approval, the approval is given, the command runs, and the stand-in answers in text.

| File | What |
| --- | --- |
| `session/wire.jsonl` | the session's `agents/main/wire.jsonl` once the turn completed: the prompt, the tool call and its result, the approval asked and resolved, usage, `turn.ended` |
| `session/state.json` | its `state.json` then: id, directory, title, last prompt, `lastTurnReason` |
| `session/wire-waiting.jsonl` | the same file while the approval was waiting: an `interaction.request` and no `interaction.resolved` |
| `session/state-waiting.json` | its `state.json` then |
| `web/created.json` | `POST /api/v1/sessions` with the directory: the session, with no model yet |
| `web/session.json` | `GET /api/v1/sessions/{id}` after the turn: `busy`, `pending_interaction`, its model |
| `web/sessions.json` | `GET /api/v1/sessions`, while a second prompt waited on an approval |
| `web/status.json` | `GET /api/v1/sessions/{id}/status` |
| `web/approvals-pending.json` | `GET /api/v1/sessions/{id}/approvals?status=pending`: the approval, with its tool and command |

What the server was measured to do, beyond its OpenAPI schema (`GET /openapi.json`):
- a model given to `POST /api/v1/sessions` is not kept: the first prompt then fails with
  `model.not_configured`. The model is set with `POST …/profile {"agent_config": {"model"}}`,
  which also takes `permission_mode` (`manual`, `auto`, `yolo`);
- a running prompt is interrupted with `POST …/prompts/<prompt_id>:abort`, which answers
  `{"aborted": true}`; the turn ends `cancelled` and a waiting approval is cancelled with it;
- every answer is wrapped as `{"code": 0, "msg", "data", "request_id"}`;
- the bearer is `$KIMI_CODE_HOME/server.token`, and the server registers itself in
  `$KIMI_CODE_HOME/server/instances/<id>.json` with its pid, port and heartbeat.
