// New for agent-router: what the fixture harness answers for the Overview's reads, in the router's own
// shapes (src/api.rs `summary_of`, `projects_inner`, `sessions_of`, `tools_inner`, `requests_inner`,
// `search_inner`, `forecast_inner`; src/quota.rs `quota`; src/auth.rs `status`). Invented, every
// one: a request log generated once per page load, around the moment it loads, and every answer
// worked out from it the way the router's SQL works it out — so a scope chosen in the harness
// narrows every figure, the way it would against a router.
//
// It holds what the Overview has to survive: an account removed from the router that still has
// history, requests no session holds yet, a model no family claims, errors, subagents, a slow tail,
// and a path, a branch, a model id and an error long enough to widen a phone if anything let them.
import type { ProjectRow } from '~/data/router';
import type {
  AccountSplitRow,
  BlockHistoryRow,
  ContextRow,
  LiveQuota,
  ModelRow,
  ProviderQuota,
  RequestRecord,
  SessionRow,
  Summary,
  ThreadRow,
  TimelineRow,
  ToolCallRow,
  ToolOutputRow,
  Tools,
} from '~/data/overview';
import type { Forecast, ForecastAccount, Window } from '~/model/forecast';
import type { SearchReply } from '~/model/search';
import type { Account, Auth } from '~/model/settings';

const MIN = 60_000;
const HOUR = 60 * MIN;
const DAY = 24 * HOUR;

/// The moment the log is built around: now, to the minute.
const NOW = Math.floor(Date.now() / MIN) * MIN;

/// A seeded generator, so every load draws the same log.
function seeded(seed: number) {
  let s = seed >>> 0;
  return () => {
    s = (s * 1664525 + 1013904223) >>> 0;
    return s / 2 ** 32;
  };
}
const rnd = seeded(20261005);
const pick = <T>(xs: readonly T[]): T => xs[Math.floor(rnd() * xs.length)]!;

const LONG_PATH = '/Users/sample/code/a-directory-name-long-enough-to-wrap-in-a-phones-rail';
const PROJECT_PATHS = ['/Users/sample/code/agent-router', '/Users/sample/code/northstar', LONG_PATH] as const;
const MODELS = ['claude-opus-5-20260115', 'claude-sonnet-4-5-20250929', 'claude-haiku-4-5-20251001', 'gpt-oss-120b-with-a-name-long-enough-to-wrap-on-a-phone'] as const;
const TOOLS = ['Bash', 'Read', 'Edit', 'Grep', 'Glob', 'WebFetch', 'TodoWrite'] as const;
const LONG_ERROR =
  'upstream returned 529 overloaded_error: {"type":"error","error":{"type":"overloaded_error","message":"Overloaded"},"request_id":"req_011CTlongunbrokenidentifierthatkeepsgoingandgoingwithoutanyspacesatall"}';

interface SessionSeed {
  readonly id: string;
  readonly project: string | null;
  readonly branch: string | null;
  readonly entrypoint: string | null;
  readonly started: number;
  readonly models: readonly string[];
}

const hex = (n: number) => Math.floor(rnd() * 16 ** n).toString(16).padStart(n, '0');
const uuid = () => `${hex(8)}-${hex(4)}-${hex(4)}-${hex(4)}-${hex(12)}`;

/// Fourteen sessions over the last thirty days, the newest still running.
const SESSIONS: readonly SessionSeed[] = Array.from({ length: 14 }, (_, i) => {
  const project = i === 13 ? null : PROJECT_PATHS[i % 3]!;
  const age = i === 0 ? 40 * MIN : i < 5 ? i * 5 * HOUR : i * 2 * DAY;
  return {
    id: uuid(),
    project,
    branch: i === 1 ? 'a-branch-name-that-goes-on-for-quite-a-while-without-a-break' : i % 4 === 3 ? null : pick(['dev', 'main', 'agen-0010-overview']),
    entrypoint: i % 3 === 0 ? 'claude-vscode, claude-cli' : i % 3 === 1 ? 'claude-cli' : 'sdk-ts',
    started: NOW - age,
    models: i % 5 === 4 ? [MODELS[0], MODELS[3]] : i % 2 ? [MODELS[0], MODELS[2]] : [MODELS[1], MODELS[0]],
  };
});

interface Logged extends RequestRecord {
  readonly account_id: string;
  readonly cache_write_5m: number;
  readonly cache_write_1h: number;
  readonly rl_5h: number | null;
  readonly rl_7d: number | null;
  readonly ctx: Readonly<Record<string, number>>;
  readonly called: readonly string[];
}

/// The request log: each session's requests from its start onward, two accounts serving, a third
/// that was removed from the router but still has history, and a few requests no session holds.
const LOG: readonly Logged[] = (() => {
  const out: Logged[] = [];
  let id = 1;
  const add = (s: SessionSeed | null, at: number, account: string) => {
    const model = s ? pick(s.models) : MODELS[1];
    const prompt = Math.round(20_000 + rnd() * 160_000);
    const read = Math.round(prompt * (0.6 + rnd() * 0.35));
    const w5 = Math.round((prompt - read) * 0.6);
    const w1 = Math.round((prompt - read) * 0.1);
    const input = Math.max(0, prompt - read - w5 - w1);
    const output = Math.round(300 + rnd() * 6_000);
    const thinking = Math.round(output * rnd() * 0.4);
    const r = rnd();
    const status = r < 0.03 ? 529 : r < 0.05 ? 429 : r < 0.06 ? 500 : 200;
    const sidechain = rnd() < 0.18 ? 1 : 0;
    const ttfb = Math.round(400 + rnd() ** 3 * 9_000);
    const priced = !model.startsWith('gpt');
    const cost = priced ? (input * 15 + (w5 * 18.75 + w1 * 30) + read * 1.5 + output * 75) / 1e6 / (model.includes('haiku') ? 15 : model.includes('sonnet') ? 5 : 1) : null;
    const called = Array.from({ length: Math.floor(rnd() * 4) }, () => pick(TOOLS));
    const tools = Math.round(prompt * 0.12);
    const system = Math.round(prompt * 0.08);
    const results = Math.round(prompt * (0.2 + rnd() * 0.2));
    out.push({
      id: id++,
      request_id: `req_${hex(12)}${hex(12)}`,
      session_id: s?.id ?? null,
      project: s?.project ?? null,
      model,
      status,
      ttfb_ms: ttfb,
      duration_ms: ttfb + Math.round(1_000 + rnd() * 40_000),
      stop_reason: status >= 400 ? null : pick(['end_turn', 'tool_use', 'tool_use', 'max_tokens']),
      is_sidechain: sidechain,
      agent_id: sidechain ? `agent-${hex(6)}` : null,
      started_at: at,
      error: status === 529 ? LONG_ERROR : status >= 400 ? `upstream returned ${status}` : null,
      prompt_tokens: prompt,
      input_tokens: input,
      output_tokens: output,
      thinking_tokens: thinking,
      cost,
      cache_read_tokens: read,
      cache_write_tokens: w5 + w1,
      tools: called.join(',') || null,
      context: `tools:${tools},system:${system},history:${prompt - tools - system - results},tool_results:${results}`,
      account_id: account,
      cache_write_5m: w5,
      cache_write_1h: w1,
      rl_5h: account === 'secondary' ? Math.min(1, 0.05 + ((NOW - at) < 5 * HOUR ? (5 * HOUR - (NOW - at)) / (5 * HOUR) : 0) * 0.5) : null,
      rl_7d: account === 'secondary' ? Math.min(1, 0.2 + rnd() * 0.3) : null,
      ctx: { tools, system, history: prompt - tools - system - results, tool_results: results },
      called,
    });
  };
  SESSIONS.forEach((s, i) => {
    const count = i === 0 ? 70 : i < 5 ? 40 : 18;
    const span = Math.min(NOW - s.started, i === 0 ? 40 * MIN : 3 * HOUR);
    for (let k = 0; k < count; k++) {
      const at = s.started + Math.round((span * k) / count);
      add(s, at, i > 9 ? 'retired' : i % 4 === 2 ? 'primary' : 'secondary');
    }
  });
  for (let k = 0; k < 4; k++) add(null, NOW - (k + 1) * 3 * MIN, 'secondary');
  return out.sort((a, b) => b.started_at - a.started_at);
})();

/// A Kimi Code account with nothing in the request log: its usage API reads it all the same, so it
/// has a quota block of its own beside Anthropic's (OV-C224).
const KIMI = { id: 'kimi-code', label: 'kimi@example.test' } as const;

const ACCOUNTS: readonly (Account & { readonly subscription_type: string | null; readonly org_name: string | null })[] = [
  { id: 'primary', label: 'primary@example.test', email: 'primary@example.test', source: 'keychain', active: 0, subscription_type: 'pro', org_name: null },
  { id: 'secondary', label: 'secondary@example.test', email: 'secondary@example.test', source: 'token', active: 1, subscription_type: 'max', org_name: null },
];

// ---------------------------------------------------------------- the router's arithmetic

interface Filter {
  /// Every request in the log is Anthropic's: Kimi Code's account has none (OV-C227).
  readonly provider: string | null;
  readonly account: string | null;
  readonly project: string | null;
  readonly session: string | null;
  readonly since: number | null;
  readonly tz: number;
}

const filterOf = (q: URLSearchParams): Filter => ({
  provider: q.get('provider'),
  account: q.get('account'),
  project: q.get('project'),
  session: q.get('session'),
  since: q.has('since') ? Math.min(NOW, Math.max(0, Number(q.get('since')))) : null,
  tz: Number(q.get('tz') ?? 0),
});

/// A request's provider, Anthropic's when it names none, as the router reads one (`provider_test`).
const ofProvider = (f: Filter) => (r: Logged) => f.provider === null || (r.provider || 'anthropic') === f.provider;

/// src/api.rs `SCOPE`: "(unresolved)" selects the requests whose session has no project.
const inScope = (f: Filter) => (r: Logged) =>
  ofProvider(f)(r) &&
  (f.project === null || r.project === f.project || (f.project === '(unresolved)' && r.session_id !== null && r.project === null)) &&
  (f.session === null || r.session_id === f.session) &&
  (f.since === null || r.started_at >= f.since) &&
  (f.account === null || r.account_id === f.account);

const sum = (rows: readonly Logged[], get: (r: Logged) => number | null) => rows.reduce((a, r) => a + (get(r) ?? 0), 0);
const isErr = (r: Logged) => (r.status ?? 0) >= 400 || !!r.error;

/// src/api.rs `pctile`: ORDER BY and OFFSET count × pct / 100.
function pctile(rows: readonly Logged[], get: (r: Logged) => number | null, pct: number): number | null {
  const xs = rows.map(get).filter((v): v is number => v !== null).sort((a, b) => a - b);
  return xs.length ? (xs[Math.floor((xs.length * pct) / 100)] ?? null) : null;
}

/// src/api.rs `pick_bucket`.
function pickBucket(span: number): number {
  for (const b of [MIN, 5 * MIN, 15 * MIN, 30 * MIN, HOUR, 3 * HOUR, 6 * HOUR, DAY]) if (span / b <= 72) return b;
  return DAY;
}

function summary(f: Filter): Summary {
  const rows = LOG.filter(inScope(f));
  const totals = {
    requests: rows.length,
    prompt_tokens: sum(rows, (r) => r.prompt_tokens),
    cost: sum(rows, (r) => r.cost),
    input_tokens: sum(rows, (r) => r.input_tokens),
    output_tokens: sum(rows, (r) => r.output_tokens),
    thinking_tokens: sum(rows, (r) => r.thinking_tokens),
    cache_read_tokens: sum(rows, (r) => r.cache_read_tokens),
    cache_write_5m_tokens: sum(rows, (r) => r.cache_write_5m),
    cache_write_1h_tokens: sum(rows, (r) => r.cache_write_1h),
    cache_write_tokens: sum(rows, (r) => r.cache_write_tokens),
    errors: rows.filter(isErr).length,
    subagent_requests: rows.filter((r) => r.is_sidechain === 1).length,
    sessions: new Set(rows.map((r) => r.session_id).filter(Boolean)).size,
    projects: new Set(rows.filter((r) => r.session_id).map((r) => r.project ?? '')).size,
    first_seen: rows.length ? Math.min(...rows.map((r) => r.started_at)) : null,
    last_seen: rows.length ? Math.max(...rows.map((r) => r.started_at)) : null,
    // Each figure's split by provider, Anthropic's first (OV-C226).
    by_provider: [...new Set(rows.map((r) => r.provider || 'anthropic'))]
      .sort((a, b) => Number(a !== 'anthropic') - Number(b !== 'anthropic') || a.localeCompare(b))
      .map((provider) => {
        const rs = rows.filter((r) => (r.provider || 'anthropic') === provider);
        return {
          provider,
          requests: rs.length,
          prompt_tokens: sum(rs, (r) => r.prompt_tokens),
          cost: sum(rs, (r) => r.cost),
          unpriced: rs.filter((r) => r.cost === null).length,
          estimated: 0,
          output_tokens: sum(rs, (r) => r.output_tokens),
          cache_read_tokens: sum(rs, (r) => r.cache_read_tokens),
          cache_write_tokens: sum(rs, (r) => r.cache_write_tokens),
        };
      }),
  };
  const active = ACCOUNTS.find((a) => a.active)!;
  const latest = LOG.find((r) => r.account_id === active.id && r.rl_5h !== null);
  const context: ContextRow[] = ['tools', 'system', 'history', 'tool_results']
    .map((c) => {
      const est = sum(rows, (r) => r.ctx[c] ?? 0);
      const priced = rows.filter((r) => r.cost !== null);
      return { component: c, est_tokens: est, chars: est * 4, est_cost_once: (sum(priced, (r) => r.ctx[c] ?? 0) * 15) / 1e6, requests: rows.length };
    })
    .sort((a, b) => (b.est_tokens ?? 0) - (a.est_tokens ?? 0));
  const byModel = new Map<string, Logged[]>();
  for (const r of rows) if (r.model) byModel.set(r.model, [...(byModel.get(r.model) ?? []), r]);
  const models: ModelRow[] = [...byModel]
    .map(([model, rs]) => ({
      model,
      requests: rs.length,
      prompt_tokens: sum(rs, (r) => r.prompt_tokens),
      cost: sum(rs, (r) => r.cost),
      output_tokens: sum(rs, (r) => r.output_tokens),
      thinking_tokens: sum(rs, (r) => r.thinking_tokens),
      cache_read_tokens: sum(rs, (r) => r.cache_read_tokens),
      errors: rs.filter(isErr).length,
      subagent_requests: rs.filter((r) => r.is_sidechain === 1).length,
    }))
    .sort((a, b) => (b.prompt_tokens ?? 0) - (a.prompt_tokens ?? 0));
  const thread = (side: 0 | 1): ThreadRow | null => {
    const rs = rows.filter((r) => (r.is_sidechain ?? 0) === side);
    if (!rs.length) return null;
    return {
      thread: side ? 'subagent' : 'main',
      requests: rs.length,
      prompt_tokens: sum(rs, (r) => r.prompt_tokens),
      cost: sum(rs, (r) => r.cost),
      output_tokens: sum(rs, (r) => r.output_tokens),
      cache_read_tokens: sum(rs, (r) => r.cache_read_tokens),
      agents: new Set(rs.map((r) => r.agent_id).filter(Boolean)).size,
    };
  };
  const ranged = LOG.filter((r) => ofProvider(f)(r) && (f.since === null || r.started_at >= f.since));
  const byAccount: AccountSplitRow[] = [...new Set(ranged.map((r) => r.account_id))]
    .map((id) => {
      const rs = ranged.filter((r) => r.account_id === id);
      const a = ACCOUNTS.find((x) => x.id === id);
      return {
        id,
        label: a?.email ?? a?.label ?? id,
        active: a?.active ?? 0,
        requests: rs.length,
        prompt_tokens: sum(rs, (r) => r.prompt_tokens),
        cost: sum(rs, (r) => r.cost),
        output_tokens: sum(rs, (r) => r.output_tokens),
        sessions: new Set(rs.map((r) => r.session_id)).size,
        last_seen: Math.max(...rs.map((r) => r.started_at)),
      };
    })
    .sort((a, b) => (b.prompt_tokens ?? 0) - (a.prompt_tokens ?? 0));
  const stops = new Map<string, number>();
  for (const r of rows) stops.set(r.stop_reason ?? '(none)', (stops.get(r.stop_reason ?? '(none)') ?? 0) + 1);

  // The timeline spans the range when one was asked for, the data's own span otherwise.
  const lo = totals.first_seen;
  const hi = totals.last_seen;
  const range = lo !== null && hi !== null ? [f.since !== null ? Math.min(f.since, lo) : lo, f.since !== null ? Math.max(NOW, hi) : hi] : f.since !== null ? [f.since, Math.max(NOW, f.since + 1)] : null;
  const tzMs = Math.max(-14 * 60, Math.min(14 * 60, f.tz)) * MIN;
  let timeline: TimelineRow[] = [];
  let history: BlockHistoryRow[] = [];
  let kimiHistory: BlockHistoryRow[] = [];
  let bucket = 0;
  if (range) {
    bucket = pickBucket(range[1]! - range[0]!);
    const at = (t: number) => Math.floor((t + tzMs) / bucket) * bucket - tzMs;
    const cells = new Map<string, Logged[]>();
    for (const r of rows) {
      const key = `${at(r.started_at)}|${r.model}`;
      cells.set(key, [...(cells.get(key) ?? []), r]);
    }
    timeline = [...cells]
      .map(([key, rs]) => ({
        t: Number(key.split('|')[0]),
        model: key.split('|')[1] ?? null,
        requests: rs.length,
        prompt_tokens: sum(rs, (r) => r.prompt_tokens),
        cost: sum(rs, (r) => r.cost),
        output_tokens: sum(rs, (r) => r.output_tokens),
        cache_read_tokens: sum(rs, (r) => r.cache_read_tokens),
        subagent_tokens: sum(rs, (r) => (r.is_sidechain ? r.prompt_tokens : 0)),
        errors: rs.filter(isErr).length,
      }))
      .sort((a, b) => a.t - b.t || String(a.model).localeCompare(String(b.model)));
    const quota = new Map<number, { h: number; d: number }>();
    for (const r of LOG) {
      if (r.account_id !== active.id || r.rl_5h === null || r.started_at < range[0]! || r.started_at > range[1]!) continue;
      const t = at(r.started_at);
      const was = quota.get(t) ?? { h: 0, d: 0 };
      quota.set(t, { h: Math.max(was.h, r.rl_5h ?? 0), d: Math.max(was.d, r.rl_7d ?? 0) });
    }
    history = [...quota].map(([t, v]) => ({ t, five_hour: v.h, seven_day: v.d })).sort((a, b) => a.t - b.t);
    // Kimi Code's usage, read every quarter hour over the last day: a 5-hour window that climbs
    // through the session and a month that creeps.
    const kimi = new Map<number, BlockHistoryRow>();
    for (let read = NOW - DAY; read <= NOW; read += 15 * MIN) {
      if (read < range[0]! || read > range[1]!) continue;
      const t = at(read);
      const five = Math.min(1, 0.05 + ((read - (NOW - DAY)) % (5 * HOUR)) / (5 * HOUR) * 0.6);
      const month = 0.62 + ((read - (NOW - DAY)) / DAY) * 0.04;
      const was = kimi.get(t);
      kimi.set(t, { t, five_hour: Math.max(was?.five_hour ?? 0, five), month: Math.max(was?.month ?? 0, month) });
    }
    kimiHistory = [...kimi.values()].sort((a, b) => a.t - b.t);
  }
  // A block per provider with an account, Anthropic's first (src/api.rs `providers_quota`).
  const providers: ProviderQuota[] = [
    {
      provider: 'anthropic',
      account: { id: active.id, label: active.label ?? null, email: active.email ?? null, org_name: null, plan: active.subscription_type },
      windows: latest
        ? { five_hour: { utilization: latest.rl_5h, resets_at: Math.floor((NOW + 2 * HOUR) / 1000) }, seven_day: { utilization: latest.rl_7d, resets_at: Math.floor((NOW + 3 * DAY) / 1000) } }
        : { five_hour: null, seven_day: null },
      status: latest ? 'allowed' : null,
      at: latest?.started_at ?? null,
      history,
    },
    {
      provider: 'kimi',
      account: { id: KIMI.id, label: KIMI.label, email: null, org_name: null, plan: null },
      windows: { five_hour: { utilization: 0.38, resets_at: Math.floor((NOW + 3 * HOUR + 40 * MIN) / 1000) }, month: { utilization: 0.66, resets_at: Math.floor((NOW + 17 * DAY) / 1000) } },
      status: 'allowed',
      at: NOW - 4 * MIN,
      history: kimiHistory,
    },
  ];
  return {
    totals,
    unattributed: rows.filter((r) => r.session_id === null).length,
    context,
    models,
    threads: [thread(0), thread(1)].filter((t): t is ThreadRow => t !== null),
    latency: {
      ttfb_p50: pctile(rows, (r) => r.ttfb_ms, 50),
      ttfb_p95: pctile(rows, (r) => r.ttfb_ms, 95),
      duration_p50: pctile(rows, (r) => r.duration_ms, 50),
      duration_p95: pctile(rows, (r) => r.duration_ms, 95),
      duration_max: rows.length ? Math.max(...rows.map((r) => r.duration_ms ?? 0)) : null,
    },
    stop_reasons: [...stops].map(([stop_reason, requests]) => ({ stop_reason, requests })).sort((a, b) => b.requests - a.requests),
    by_account: byAccount,
    timeline: { start: range?.[0] ?? null, end: range?.[1] ?? null, bucket_ms: bucket, rows: timeline },
    providers,
  };
}

/// src/api.rs `projects_inner`: narrowed by the account and the range only (OV-C131).
function projects(f: Filter): ProjectRow[] {
  const rows = LOG.filter((r) => ofProvider(f)(r) && r.session_id !== null && (f.since === null || r.started_at >= f.since) && (f.account === null || r.account_id === f.account));
  const by = new Map<string | null, Logged[]>();
  for (const r of rows) by.set(r.project, [...(by.get(r.project) ?? []), r]);
  return [...by]
    .map(([project, rs]) => ({
      project,
      repository: null,
      directories: project ? [{ cwd: project, machine: null, remotes: [] }] : [],
      sessions: new Set(rs.map((r) => r.session_id)).size,
      requests: rs.length,
      prompt_tokens: sum(rs, (r) => r.prompt_tokens),
      cost: sum(rs, (r) => r.cost),
      output_tokens: sum(rs, (r) => r.output_tokens),
      cache_read_tokens: sum(rs, (r) => r.cache_read_tokens),
      subagent_tokens: sum(rs, (r) => (r.is_sidechain ? r.prompt_tokens : 0)),
      errors: rs.filter(isErr).length,
      models: [...new Set(rs.map((r) => r.model))].join(','),
      first_seen: Math.min(...rs.map((r) => r.started_at)),
      last_seen: Math.max(...rs.map((r) => r.started_at)),
    }))
    .sort((a, b) => b.prompt_tokens - a.prompt_tokens);
}

/// src/api.rs `sessions_of`: narrowed by the account, the project and the range, never the session.
function sessions(f: Filter): SessionRow[] {
  return SESSIONS.map((s): SessionRow | null => {
    const rs = LOG.filter((r) => ofProvider(f)(r) && r.session_id === s.id && (f.since === null || r.started_at >= f.since) && (f.account === null || r.account_id === f.account));
    if (!rs.length || (f.project !== null && s.project !== f.project && !(f.project === '(unresolved)' && s.project === null))) return null;
    const last = Math.max(...rs.map((r) => r.started_at));
    return {
      id: s.id,
      project: s.project,
      cwd: s.project,
      repository: null,
      machine: null,
      git_branch: s.branch,
      entrypoint: s.entrypoint,
      started_at: s.started,
      last_seen_at: last,
      requests: rs.length,
      prompt_tokens: sum(rs, (r) => r.prompt_tokens),
      cost: sum(rs, (r) => r.cost),
      output_tokens: sum(rs, (r) => r.output_tokens),
      cache_read_tokens: sum(rs, (r) => r.cache_read_tokens),
      subagent_tokens: sum(rs, (r) => (r.is_sidechain ? r.prompt_tokens : 0)),
      errors: rs.filter(isErr).length,
      subagents: new Set(rs.map((r) => r.agent_id).filter(Boolean)).size,
      models: [...new Set(rs.map((r) => r.model))].join(','),
      first_request_at: Math.min(...rs.map((r) => r.started_at)),
      last_request_at: last,
    };
  })
    .filter((s): s is SessionRow => s !== null)
    .sort((a, b) => (b.last_seen_at ?? 0) - (a.last_seen_at ?? 0))
    .slice(0, 200);
}

function tools(f: Filter): Tools {
  const rows = LOG.filter(inScope(f));
  const names = [...new Set(rows.flatMap((r) => r.called))];
  const produced: ToolOutputRow[] = names
    .map((name) => {
      const rs = rows.filter((r) => r.called.includes(name));
      const est = sum(rs, (r) => Math.round((r.ctx['tool_results'] ?? 0) / Math.max(1, r.called.length)));
      return { name, results: rs.length, chars: est * 4, est_tokens: est, max_tokens: Math.round(est / Math.max(1, rs.length) * 2.5), est_cost_once: (est * 15) / 1e6, errors: rs.filter(isErr).length };
    })
    .sort((a, b) => (b.est_tokens ?? 0) - (a.est_tokens ?? 0));
  const called: ToolCallRow[] = names
    .map((name) => {
      const rs = rows.filter((r) => r.called.includes(name));
      return { name, calls: sum(rs, (r) => r.called.filter((c) => c === name).length), requests: rs.length, subagent_calls: sum(rs.filter((r) => r.is_sidechain), (r) => r.called.filter((c) => c === name).length) };
    })
    .sort((a, b) => (b.calls ?? 0) - (a.calls ?? 0));
  return { produced, called };
}

/// src/api.rs `requests_inner`: the 300 newest in scope.
function requests(f: Filter): RequestRecord[] {
  return LOG.filter(inScope(f))
    .slice(0, 300)
    .map(({ account_id: _a, cache_write_5m: _w5, cache_write_1h: _w1, rl_5h: _h, rl_7d: _d, ctx: _c, called: _t, ...r }) => r);
}

/// src/quota.rs: the serving account's usage, read from Anthropic a moment ago.
const quota = (): LiveQuota => ({
  five_hour: { utilization: 51, resets_at: new Date(NOW + 2 * HOUR + 14 * MIN).toISOString() },
  seven_day: { utilization: 63, resets_at: new Date(NOW + 3 * DAY + 5 * HOUR).toISOString() },
  severity: 'normal',
  account_id: 'secondary',
  source: 'usage-api',
  age_ms: 12_000,
});

const secs = (ms: number) => Math.floor(ms / 1000);

/// src/api.rs `forecast_inner`: the serving account with a 5-hour window that resets first and a
/// 7-day one too close to call; the other with both windows rolled over; one removed from the router
/// that still has history (OV-C96); and a Kimi Code account with a 5-hour window and a month.
function forecast(f: Filter): Forecast {
  const usage = (id: string) => {
    const rs = LOG.filter((r) => r.account_id === id && (f.since === null || r.started_at >= f.since));
    return { requests: rs.length, prompt_tokens: sum(rs, (r) => r.prompt_tokens), output_tokens: sum(rs, (r) => r.output_tokens), cost: sum(rs, (r) => r.cost), last_seen: rs.length ? Math.max(...rs.map((r) => r.started_at)) : null };
  };
  const series = (start: number, used: number, n: number) => Array.from({ length: n }, (_, i) => ({ t: start + ((NOW - start) * i) / n, u: (used * i) / n }));
  const fiveStart = NOW - 2 * HOUR - 46 * MIN;
  const five: Window = {
    used: 0.51, reset_at: secs(NOW + 2 * HOUR + 14 * MIN), window_start: secs(fiveStart),
    burn_per_active_hour: 0.15, active_hours_left: 3.2, duty_fraction: 0.99, duty: { low: 0.99, mid: 0.99, high: 0.99 },
    exhausts_at: null, exhausts_at_early: null, exhausts_at_late: null, resets_first: true, verdict: 'resets-first',
    projected_end_util: 0.73, projected_end_util_low: 0.73, projected_end_util_high: 0.73,
    basis: 'this-window', measured_active_hours: 2.7, readings: 900, expired: false, seen_at: NOW - 2 * MIN, series: series(fiveStart, 0.51, 24),
  };
  const sevenStart = NOW - 3 * DAY - 19 * HOUR;
  const seven: Window = {
    used: 0.63, reset_at: secs(NOW + 3 * DAY + 5 * HOUR), window_start: secs(sevenStart),
    burn_per_active_hour: 0.0167, active_hours_left: 22.1, duty_fraction: 0.33, duty: { low: 0.25, mid: 0.33, high: 0.41 },
    exhausts_at: NOW + 3 * DAY + 2 * HOUR, exhausts_at_early: NOW + 2 * DAY + 9 * HOUR, exhausts_at_late: null, resets_first: false, verdict: 'tight',
    projected_end_util: 1, projected_end_util_low: 0.88, projected_end_util_high: 1,
    basis: 'this-window', measured_active_hours: 29.1, readings: 7935, expired: false, seen_at: NOW - 2 * MIN, series: series(sevenStart, 0.63, 48),
  };
  const accounts: ForecastAccount[] = [
    { id: 'primary', label: 'primary@example.test', email: 'primary@example.test', plan: 'pro', active: 0, usage: usage('primary'), seen_at: NOW - 30 * HOUR, status: 'allowed', pool: secs(NOW - DAY),
      five_hour: { expired: true, reset_at: secs(NOW - 26 * HOUR), seen_at: NOW - 30 * HOUR }, seven_day: { expired: true, reset_at: secs(NOW - DAY), seen_at: NOW - 30 * HOUR } },
    { id: 'secondary', label: 'secondary@example.test', email: 'secondary@example.test', plan: 'max', active: 1, usage: usage('secondary'), seen_at: NOW - 2 * MIN, status: 'allowed', pool: secs(NOW + 3 * DAY + 5 * HOUR), five_hour: five, seven_day: seven },
    { id: 'retired', label: 'retired', email: null, plan: null, active: 0, usage: usage('retired'), seen_at: null, status: null, pool: null, five_hour: null, seven_day: null },
    {
      id: KIMI.id, label: KIMI.label, email: null, plan: null, active: 1, provider: 'kimi', usage: usage(KIMI.id), seen_at: NOW - 4 * MIN, status: 'allowed', pool: null,
      five_hour: {
        used: 0.38, reset_at: secs(NOW + 3 * HOUR + 40 * MIN), window_start: secs(NOW - HOUR - 20 * MIN),
        burn_per_active_hour: 0.2, active_hours_left: 3.1, duty_fraction: 0.9, duty: { low: 0.9, mid: 0.9, high: 0.9 },
        exhausts_at: null, exhausts_at_early: null, exhausts_at_late: null, resets_first: true, verdict: 'resets-first',
        projected_end_util: 0.86, projected_end_util_low: 0.86, projected_end_util_high: 0.86,
        basis: 'this-window', measured_active_hours: 1.2, readings: 6, expired: false, spent: false, seen_at: NOW - 4 * MIN, series: series(NOW - HOUR - 20 * MIN, 0.38, 6),
      },
      seven_day: null,
      month: {
        used: 0.66, reset_at: secs(NOW + 17 * DAY), window_start: secs(NOW - 13 * DAY),
        burn_per_active_hour: 0.012, active_hours_left: 28, duty_fraction: 0.3, duty: { low: 0.22, mid: 0.3, high: 0.38 },
        exhausts_at: NOW + 11 * DAY, exhausts_at_early: NOW + 8 * DAY, exhausts_at_late: null, resets_first: false, verdict: 'tight',
        projected_end_util: 1, projected_end_util_low: 0.92, projected_end_util_high: 1,
        basis: 'this-window', measured_active_hours: 55, readings: 1240, expired: false, spent: false, seen_at: NOW - 4 * MIN, series: series(NOW - 13 * DAY, 0.66, 48),
      },
    },
  ];
  const all = accounts.map((a) => a.usage!);
  return {
    generated_at: NOW,
    accounts,
    global: {
      requests: all.reduce((x, u) => x + (u.requests ?? 0), 0),
      prompt_tokens: all.reduce((x, u) => x + (u.prompt_tokens ?? 0), 0),
      output_tokens: all.reduce((x, u) => x + (u.output_tokens ?? 0), 0),
      cost: all.reduce((x, u) => x + (u.cost ?? 0), 0),
      accounts: accounts.length,
      // Each provider's own headroom, never totalled across them (OV-C225).
      providers: {
        anthropic: { active_hours_left: 3.2, pools: 2, first_to_exhaust: { account_id: 'secondary', window: 'seven_day', at: NOW + 3 * DAY + 2 * HOUR, in_ms: 3 * DAY + 2 * HOUR } },
        kimi: { active_hours_left: 3.1, pools: 1, first_to_exhaust: { account_id: KIMI.id, window: 'month', at: NOW + 11 * DAY, in_ms: 11 * DAY } },
      },
      pace_30d: { cost: 1840.5, prompt_tokens: 2_100_000_000, basis: 'the last 7 days of recorded spend' },
    },
  };
}

/// src/auth.rs `status`: a router on this Mac, listening on the network with a password set.
const auth = (): Auth => ({
  configured: true, updated_at: NOW - 6 * DAY, from_loopback: true, remote_bind: true,
  listening_on: '0.0.0.0:8787', lan_url: 'http://192.0.2.10:8787', env_override: false, mode: 'local', password_pinned: false, public_url: null,
});

/// What a search finds: the blocks of the log's newest sessions that mention it.
const BLOCKS = SESSIONS.slice(0, 6).flatMap((s, i) => [
  { kind: 'text', role: 'user', text: `Run the tests in ${s.project ?? 'the scratch directory'} and tell me what fails.`, s, i, at: s.started + 60_000 },
  { kind: 'tool_use', role: 'assistant', text: `pnpm exec vitest run --reporter=dot # session ${i}`, s, i, at: s.started + 120_000 },
  { kind: 'tool_result', role: 'user', text: `Tests  ${12 + i} passed (${12 + i}) and the overview renders every panel`, s, i, at: s.started + 180_000 },
  { kind: 'thinking', role: 'assistant', text: 'The overview reads one snapshot per scope, so the tiles and the lists agree.', s, i, at: s.started + 240_000 },
]);

function search(q: URLSearchParams): SearchReply {
  const words = (q.get('q') ?? '').trim();
  const kind = q.get('kind');
  const term = words.toLowerCase().replace(/"/g, '');
  const found = BLOCKS.filter(
    (b) =>
      term !== '' &&
      b.text.toLowerCase().includes(term) &&
      (!kind || b.kind === kind) &&
      (!q.get('project') || b.s.project === q.get('project')) &&
      (!q.get('session') || b.s.id === q.get('session')) &&
      (!q.get('since') || b.at >= Number(q.get('since'))) &&
      // The account and the provider of the requests its session made (OV-C227).
      (!q.get('account') || LOG.some((r) => r.session_id === b.s.id && r.account_id === q.get('account'))) &&
      (!q.get('provider') || LOG.some((r) => r.session_id === b.s.id && (r.provider || 'anthropic') === q.get('provider'))),
  );
  const mark = (text: string) => {
    const at = text.toLowerCase().indexOf(term);
    return at < 0 ? text : `${text.slice(0, at)}‹${text.slice(at, at + term.length)}›${text.slice(at + term.length)}`;
  };
  const bySession = new Map<string, typeof found>();
  for (const b of found) bySession.set(b.s.id, [...(bySession.get(b.s.id) ?? []), b]);
  return {
    query: words,
    expression: words,
    sessions: [...bySession].map(([id, bs]) => ({
      session_id: id,
      project: bs[0]!.s.project,
      session_label: null,
      git_branch: bs[0]!.s.branch,
      blocks: bs.length,
      hits: bs.length,
      first_at: Math.min(...bs.map((b) => b.at)),
      last_at: Math.max(...bs.map((b) => b.at)),
    })),
    hits: found.map((b, k) => ({
      block_id: 100 + k,
      kind: b.kind,
      chars: b.text.length,
      role: b.role,
      ordinal: k,
      request_id: k + 1,
      at: b.at,
      session_id: b.s.id,
      project: b.s.project,
      session_label: null,
      snippet: mark(b.text),
      rank: -10 + k,
    })),
  };
}

/// The summary as the router answers `parts=` (SV-C45): the parts named and nothing else, totals
/// bringing unattributed; the whole of it without `parts=`.
function partsOf(s: Summary, parts: string | null): Partial<Summary> {
  if (parts === null) return s;
  const keep = new Set(parts.split(','));
  if (keep.has('totals')) keep.add('unattributed');
  return Object.fromEntries(Object.entries(s).filter(([k]) => keep.has(k)));
}

const json = (payload: unknown): Response => new Response(JSON.stringify(payload), { status: 200, headers: { 'Content-Type': 'application/json' } });

/// The Overview's reads, answered; null for a read this file does not hold, which the harness then
/// answers as unanswered. Only the parameters each endpoint takes are honoured: anything else is a
/// read the console should not be making.
export function overviewRead(path: string, query: URLSearchParams): Response | null {
  const only = (...allowed: string[]) => [...query.keys()].every((p) => allowed.includes(p));
  const f = filterOf(query);
  if (path === '/api/summary' && only('provider', 'account', 'project', 'session', 'since', 'tz', 'parts')) return json(partsOf(summary(f), query.get('parts')));
  if (path === '/api/projects' && only('provider', 'account', 'since')) return json(projects(f));
  if (path === '/api/sessions' && only('provider', 'account', 'project', 'since')) return json(sessions(f));
  if (path === '/api/tools' && only('provider', 'account', 'project', 'session', 'since')) return json(tools(f));
  if (path === '/api/requests' && only('provider', 'account', 'project', 'session', 'since')) return json(requests(f));
  if (path === '/api/quota' && only()) return json(quota());
  if (path === '/api/forecast' && only('since')) return json(forecast(f));
  if (path === '/api/auth' && only()) return json(auth());
  if (path === '/api/accounts' && only()) return json(ACCOUNTS);
  if (path === '/api/search' && only('q', 'kind', 'provider', 'account', 'project', 'session', 'since', 'limit')) return json(search(query));
  return null;
}
