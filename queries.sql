-- Ready-made queries for the Agent Router store, until the web UI lands.
--   sqlite3 ~/.agent-router/agent-router.db < queries.sql
-- or open it and run them one at a time:
--   sqlite3 -header -column ~/.agent-router/agent-router.db

.headers on
.mode column

SELECT '── tokens per project ──' AS '';
SELECT s.cwd                                   AS project,
       count(*)                                AS requests,
       sum(u.prompt_tokens)                    AS prompt_tokens,
       sum(u.output_tokens)                    AS output_tokens,
       sum(u.cache_read_tokens)                AS cache_read,
       round(100.0 * sum(u.cache_read_tokens) / NULLIF(sum(u.prompt_tokens), 0), 1) AS cache_hit_pct
FROM requests r
JOIN usage u    ON u.request_id = r.id
JOIN sessions s ON s.id = r.session_id
GROUP BY s.cwd
ORDER BY prompt_tokens DESC;

SELECT '── tokens per session ──' AS '';
SELECT substr(s.id, 1, 8)                      AS session,
       s.cwd                                   AS project,
       s.git_branch                            AS branch,
       datetime(s.started_at / 1000, 'unixepoch', 'localtime') AS started,
       count(*)                                AS requests,
       sum(u.prompt_tokens)                    AS prompt_tokens,
       sum(u.output_tokens)                    AS output_tokens
FROM requests r
JOIN usage u    ON u.request_id = r.id
JOIN sessions s ON s.id = r.session_id
GROUP BY s.id
ORDER BY s.started_at DESC;

SELECT '── where the context actually goes ──' AS '';
SELECT c.component,
       sum(c.est_tokens)                       AS est_tokens,
       round(100.0 * sum(c.est_tokens) / (SELECT sum(est_tokens) FROM context), 1) AS pct
FROM context c
GROUP BY c.component
ORDER BY est_tokens DESC;

SELECT '── per model ──' AS '';
SELECT r.model,
       count(*)                                AS requests,
       sum(u.prompt_tokens)                    AS prompt_tokens,
       sum(u.output_tokens)                    AS output_tokens,
       sum(u.thinking_tokens)                  AS thinking,
       sum(u.cache_creation_1h_tokens)         AS cache_write_1h,
       sum(u.cache_creation_5m_tokens)         AS cache_write_5m
FROM requests r JOIN usage u ON u.request_id = r.id
GROUP BY r.model ORDER BY prompt_tokens DESC;

SELECT '── which tools produce the most output ──' AS '';
SELECT name,
       count(*)                                AS results,
       sum(result_chars)                       AS chars,
       sum(est_tokens)                         AS est_tokens,
       sum(is_error)                           AS errors
FROM tool_calls
GROUP BY name ORDER BY est_tokens DESC;

SELECT '── which tools the model calls most ──' AS '';
SELECT name, count(*) AS calls FROM tool_invocations GROUP BY name ORDER BY calls DESC;

SELECT '── quota headroom, most recent request ──' AS '';
SELECT rl_status                               AS status,
       round(100.0 * rl_5h_utilization, 1)     AS pct_5h,
       datetime(rl_5h_reset, 'unixepoch', 'localtime') AS resets_5h,
       round(100.0 * rl_7d_utilization, 1)     AS pct_7d,
       datetime(rl_7d_reset, 'unixepoch', 'localtime') AS resets_7d
FROM requests WHERE rl_status IS NOT NULL ORDER BY started_at DESC LIMIT 1;

SELECT '── slowest requests ──' AS '';
SELECT substr(r.request_id, 1, 20) AS request, r.model, r.status,
       r.ttfb_ms, r.duration_ms, u.output_tokens AS out_tok
FROM requests r LEFT JOIN usage u ON u.request_id = r.id
ORDER BY r.duration_ms DESC LIMIT 10;

SELECT '── errors ──' AS '';
SELECT datetime(started_at / 1000, 'unixepoch', 'localtime') AS at,
       status, substr(coalesce(error, ''), 1, 60) AS error, path
FROM requests WHERE status >= 400 OR error IS NOT NULL
ORDER BY started_at DESC LIMIT 20;
