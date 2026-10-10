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
