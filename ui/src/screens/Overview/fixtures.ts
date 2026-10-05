// New for agent-router: what the Overview's specs answer for the router, in its own shapes (src/api.rs,
// src/quota.rs, src/auth.rs). Invented, every one; small enough that a spec can say which figure it
// expects where. Read only by specs.
import { vi } from 'vitest';
import type { ProjectRow } from '~/data/router';
import type { LiveQuota, RequestRecord, SessionRow, Summary, Tools, Totals } from '~/data/overview';
import type { Forecast, ForecastAccount } from '~/model/forecast';
import type { Account, Auth } from '~/model/settings';

export const MIN = 60_000;
export const HOUR = 60 * MIN;
export const DAY = 24 * HOUR;

/// The moment the fixtures are built around: the spec's own now, so "ago" reads as it would.
export const NOW = Date.now();

export const SERVING = { id: 'secondary', label: 'secondary', email: 'secondary@example.test', org_name: null, subscription_type: 'max' } as const;

export const S1 = 'aaaaaaaa-1111-4111-8111-111111111111';
export const S2 = 'bbbbbbbb-2222-4222-8222-222222222222';

export function totals(over: Partial<Totals> = {}): Totals {
  return {
    requests: 412,
    prompt_tokens: 12_400_000,
    cost: 41.2,
    input_tokens: 600_000,
    output_tokens: 310_000,
    thinking_tokens: 42_000,
    cache_read_tokens: 10_400_000,
    cache_write_5m_tokens: 1_100_000,
    cache_write_1h_tokens: 300_000,
    cache_write_tokens: 1_400_000,
    errors: 3,
    subagent_requests: 27,
    sessions: 2,
    projects: 2,
    first_seen: NOW - 3 * HOUR,
    last_seen: NOW - 2 * MIN,
    ...over,
  };
}

export function summary(over: Partial<Summary> = {}): Summary {
  const t0 = Math.floor((NOW - 3 * HOUR) / HOUR) * HOUR;
  return {
    totals: totals(),
    unattributed: 4,
    quota: { rl_status: 'allowed', rl_5h_utilization: 0.41, rl_7d_utilization: 0.58, rl_5h_reset: Math.floor((NOW + 2 * HOUR) / 1000), rl_7d_reset: Math.floor((NOW + 3 * DAY) / 1000), started_at: NOW - 4 * MIN },
    account: SERVING,
    context: [
      { component: 'history', est_tokens: 6_000_000, chars: 24_000_000, est_cost_once: 90, requests: 400 },
      { component: 'tool_results', est_tokens: 4_000_000, chars: 16_000_000, est_cost_once: 60, requests: 200 },
      { component: 'tools', est_tokens: 1_600_000, chars: 6_400_000, est_cost_once: 24, requests: 400 },
      { component: 'system', est_tokens: 800_000, chars: 3_200_000, est_cost_once: 12, requests: 400 },
    ],
    models: [
      { model: 'claude-opus-5-20260115', requests: 300, prompt_tokens: 9_000_000, cost: 38, output_tokens: 220_000, thinking_tokens: 30_000, cache_read_tokens: 7_600_000, errors: 2, subagent_requests: 20 },
      { model: 'claude-sonnet-4-5-20250929', requests: 112, prompt_tokens: 3_400_000, cost: 3.2, output_tokens: 90_000, thinking_tokens: 12_000, cache_read_tokens: 2_800_000, errors: 1, subagent_requests: 7 },
    ],
    threads: [
      { thread: 'main', requests: 385, prompt_tokens: 11_000_000, cost: 37, output_tokens: 280_000, cache_read_tokens: 9_300_000, agents: 0 },
      { thread: 'subagent', requests: 27, prompt_tokens: 1_400_000, cost: 4.2, output_tokens: 30_000, cache_read_tokens: 1_100_000, agents: 6 },
    ],
    latency: { ttfb_p50: 812, ttfb_p95: 3_200, duration_p50: 9_400, duration_p95: 31_000, duration_max: 64_000 },
    stop_reasons: [
      { stop_reason: 'tool_use', requests: 300 },
      { stop_reason: 'end_turn', requests: 112 },
    ],
    by_account: [
      { id: 'secondary', label: 'secondary@example.test', active: 1, requests: 300, prompt_tokens: 9_000_000, cost: 30, output_tokens: 200_000, sessions: 2, last_seen: NOW - 2 * MIN },
      { id: 'primary', label: 'primary@example.test', active: 0, requests: 112, prompt_tokens: 3_400_000, cost: 11.2, output_tokens: 110_000, sessions: 1, last_seen: NOW - HOUR },
    ],
    timeline: {
      start: t0,
      end: t0 + 3 * HOUR,
      bucket_ms: HOUR,
      rows: [
        { t: t0, model: 'claude-opus-5-20260115', requests: 100, prompt_tokens: 3_000_000, cost: 12, output_tokens: 70_000, cache_read_tokens: 2_500_000, subagent_tokens: 300_000, errors: 1 },
        { t: t0 + HOUR, model: 'claude-sonnet-4-5-20250929', requests: 112, prompt_tokens: 3_400_000, cost: 3.2, output_tokens: 90_000, cache_read_tokens: 2_800_000, subagent_tokens: 0, errors: 0 },
        { t: t0 + 2 * HOUR, model: 'claude-opus-5-20260115', requests: 200, prompt_tokens: 6_000_000, cost: 26, output_tokens: 150_000, cache_read_tokens: 5_100_000, subagent_tokens: 1_100_000, errors: 2 },
      ],
    },
    quota_history: [
      { t: t0, rl_5h: 0.12, rl_7d: 0.5 },
      { t: t0 + 2 * HOUR, rl_5h: 0.41, rl_7d: 0.58 },
    ],
    ...over,
  };
}

export const project = (path: string | null, prompt_tokens: number, over: Partial<ProjectRow> = {}): ProjectRow => ({
  project: path,
  sessions: 1,
  requests: 200,
  prompt_tokens,
  cost: prompt_tokens / 300_000,
  output_tokens: Math.round(prompt_tokens / 40),
  cache_read_tokens: Math.round(prompt_tokens * 0.8),
  subagent_tokens: Math.round(prompt_tokens * 0.1),
  errors: 1,
  models: 'claude-opus-5-20260115,claude-sonnet-4-5-20250929',
  first_seen: NOW - DAY,
  last_seen: NOW - 2 * MIN,
  ...over,
});

export const PROJECTS: ProjectRow[] = [project('/code/agent-router', 9_000_000), project('/code/northstar', 3_400_000, { errors: 0 })];

export const session = (id: string, over: Partial<SessionRow> = {}): SessionRow => ({
  id,
  project: '/code/agent-router',
  git_branch: 'dev',
  entrypoint: 'claude-vscode, claude-cli',
  started_at: NOW - 3 * HOUR,
  last_seen_at: NOW - 2 * MIN,
  requests: 200,
  prompt_tokens: 6_000_000,
  cost: 20,
  output_tokens: 150_000,
  cache_read_tokens: 5_000_000,
  subagent_tokens: 600_000,
  errors: 1,
  subagents: 3,
  models: 'claude-opus-5-20260115',
  first_request_at: NOW - 3 * HOUR,
  last_request_at: NOW - 2 * MIN,
  ...over,
});

export const SESSIONS: SessionRow[] = [session(S1), session(S2, { project: '/code/northstar', git_branch: 'main', entrypoint: 'claude-cli', prompt_tokens: 3_400_000 })];

export const TOOLS: Tools = {
  produced: [
    { name: 'Read', results: 120, chars: 4_800_000, est_tokens: 1_200_000, max_tokens: 40_000, est_cost_once: 18, errors: 2 },
    { name: 'Bash', results: 80, chars: 1_600_000, est_tokens: 400_000, max_tokens: 9_000, est_cost_once: 6, errors: 0 },
  ],
  called: [
    { name: 'Read', calls: 1_234, requests: 300, subagent_calls: 200 },
    { name: 'Bash', calls: 80, requests: 70, subagent_calls: 0 },
  ],
};

export const request = (id: number, over: Partial<RequestRecord> = {}): RequestRecord => ({
  id,
  request_id: `req_${String(id).padStart(6, '0')}`,
  session_id: S1,
  project: '/code/agent-router',
  model: 'claude-opus-5-20260115',
  status: 200,
  ttfb_ms: 900,
  duration_ms: 9_000,
  stop_reason: 'tool_use',
  is_sidechain: 0,
  agent_id: null,
  started_at: NOW - id * MIN,
  error: null,
  prompt_tokens: 30_000,
  input_tokens: 1_000,
  output_tokens: 800,
  thinking_tokens: 100,
  cost: 0.05,
  cache_read_tokens: 27_000,
  cache_write_tokens: 2_000,
  tools: 'Read,Read,Bash',
  context: 'tools:4000,system:2000,history:16000,tool_results:8000',
  ...over,
});

/// Thirty requests: a slow one, an error, a subagent, two models and two stop reasons.
export const REQUESTS: RequestRecord[] = Array.from({ length: 30 }, (_, i) =>
  request(i + 1, {
    ...(i === 1 ? { status: 500, error: 'upstream returned 500\nretry later', stop_reason: null } : {}),
    ...(i === 2 ? { ttfb_ms: 7_400 } : {}),
    ...(i === 3 ? { is_sidechain: 1, agent_id: 'agent-abc123' } : {}),
    ...(i % 4 === 0 ? { model: 'claude-sonnet-4-5-20250929', stop_reason: 'end_turn' } : {}),
  }),
);

export const QUOTA: LiveQuota = {
  five_hour: { utilization: 41, resets_at: new Date(NOW + 2 * HOUR).toISOString() },
  seven_day: { utilization: 58, resets_at: new Date(NOW + 3 * DAY).toISOString() },
  severity: 'normal',
  account_id: 'secondary',
  source: 'usage-api',
  age_ms: 60_000,
};

const secs = (ms: number) => Math.floor(ms / 1000);

export function forecastAccount(id: string, over: Partial<ForecastAccount> = {}): ForecastAccount {
  return {
    id,
    label: `${id}@example.test`,
    email: `${id}@example.test`,
    plan: 'max',
    active: id === 'secondary' ? 1 : 0,
    usage: { requests: 200, prompt_tokens: 6_000_000, output_tokens: 150_000, cost: 20, last_seen: NOW - 2 * MIN },
    seen_at: NOW - 2 * MIN,
    status: 'allowed',
    pool: secs(NOW + 3 * DAY),
    five_hour: {
      used: 0.41, reset_at: secs(NOW + 2 * HOUR), window_start: secs(NOW - 3 * HOUR),
      burn_per_active_hour: 0.12, active_hours_left: 4.9, duty_fraction: 0.95, duty: { low: 0.95, mid: 0.95, high: 0.95 }, verdict: 'resets-first',
      exhausts_at: null, exhausts_at_early: null, exhausts_at_late: null, resets_first: true,
      projected_end_util: 0.66, projected_end_util_low: 0.66, projected_end_util_high: 0.66,
      basis: 'this-window', measured_active_hours: 2.8, readings: 400, expired: false, seen_at: NOW - 2 * MIN, series: [{ t: NOW - 2 * HOUR, u: 0.2 }],
    },
    seven_day: {
      used: 0.58, reset_at: secs(NOW + 3 * DAY), window_start: secs(NOW - 4 * DAY),
      burn_per_active_hour: 0.017, active_hours_left: 24.7, duty_fraction: 0.33, duty: { low: 0.25, mid: 0.33, high: 0.41 }, verdict: 'tight',
      exhausts_at: NOW + 2 * DAY + 20 * HOUR, exhausts_at_early: NOW + 2 * DAY, exhausts_at_late: null, resets_first: false,
      projected_end_util: 1, projected_end_util_low: 0.88, projected_end_util_high: 1,
      basis: 'this-window', measured_active_hours: 29, readings: 7000, expired: false, seen_at: NOW - 2 * MIN, series: [{ t: NOW - 3 * DAY, u: 0.2 }],
    },
    ...over,
  };
}

export function forecast(accounts: ForecastAccount[] = [forecastAccount('secondary'), forecastAccount('primary', { five_hour: null, seven_day: null, seen_at: null })]): Forecast {
  return {
    generated_at: NOW,
    accounts,
    global: {
      requests: 412, prompt_tokens: 12_400_000, output_tokens: 310_000, cost: 41.2, accounts: accounts.length,
      combined_headroom: { active_hours_left: 4.9, pools: 2, first_to_exhaust: { account_id: 'secondary', window: 'seven_day', at: NOW + 2 * DAY + 20 * HOUR, in_ms: 2 * DAY + 20 * HOUR } },
      pace_30d: { cost: 180, prompt_tokens: 50_000_000, basis: 'the last 7 days of recorded spend' },
    },
  };
}

export const LOCAL: Auth = { configured: true, updated_at: NOW - DAY, from_loopback: true, remote_bind: true, listening_on: '0.0.0.0:8787', lan_url: 'http://192.0.2.10:8787', env_override: false, mode: 'local', password_pinned: false };
export const HOSTED: Auth = { ...LOCAL, from_loopback: false, mode: 'server', password_pinned: true, public_url: 'https://router.example' };

export const ACCOUNTS: Account[] = [
  { id: 'secondary', label: 'secondary', email: 'secondary@example.test', source: 'token', active: 1 },
  { id: 'primary', label: 'primary', email: 'primary@example.test', source: 'keychain', active: 0 },
];

export type Reply = () => Response | Promise<Response>;

export const ok = (data: unknown): Response => new Response(JSON.stringify(data), { status: 200, headers: { 'Content-Type': 'application/json' } });
export const refused = (message: string, status = 500): Response => new Response(JSON.stringify({ error: message }), { status, headers: { 'Content-Type': 'application/json' } });
export const dropped = (): Promise<Response> => Promise.reject(new TypeError('fetch failed'));

/// A promise held open until the spec lets it go.
export function held(): { readonly promise: Promise<Response>; readonly release: (r: Response) => void } {
  let release: (r: Response) => void = () => {};
  const promise = new Promise<Response>((r) => (release = r));
  return { promise, release };
}

export interface Seen {
  readonly path: string;
  readonly query: URLSearchParams;
  readonly signal: AbortSignal | null;
}

/// A router for the Overview: every read answered by `answer(path, query)`, or from the fixtures
/// above; every request recorded, in order, with its signal.
export function router(answer: (path: string, query: URLSearchParams, n: number) => Response | Promise<Response> | undefined = () => undefined) {
  const seen: Seen[] = [];
  const count = new Map<string, number>();
  const fetch = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const href = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
    const url = new URL(href, 'http://console.test');
    const n = (count.get(url.pathname) ?? 0) + 1;
    count.set(url.pathname, n);
    seen.push({ path: url.pathname, query: url.searchParams, signal: init?.signal ?? null });
    const custom = answer(url.pathname, url.searchParams, n);
    // As a browser's fetch does, an answer still on its way is given up when its signal aborts.
    const signal = init?.signal;
    if (custom !== undefined) {
      if (!signal) return custom;
      return Promise.race([
        Promise.resolve(custom),
        new Promise<Response>((_, reject) => {
          const gone = () => reject(new DOMException('The operation was aborted.', 'AbortError'));
          if (signal.aborted) gone();
          else signal.addEventListener('abort', gone, { once: true });
        }),
      ]);
    }
    switch (url.pathname) {
      case '/api/summary':
        return ok(summary());
      case '/api/projects':
        return ok(PROJECTS);
      case '/api/sessions':
        return ok(SESSIONS);
      case '/api/tools':
        return ok(TOOLS);
      case '/api/requests':
        return ok(REQUESTS);
      case '/api/quota':
        return ok(QUOTA);
      case '/api/forecast':
        return ok(forecast());
      case '/api/auth':
        return ok(LOCAL);
      case '/api/accounts':
        return ok(ACCOUNTS);
      case '/api/search':
        return ok({ query: url.searchParams.get('q'), expression: url.searchParams.get('q'), sessions: [], hits: [] });
      default:
        throw new Error(`test router: no answer for GET ${url.pathname}${url.search}`);
    }
  });
  vi.stubGlobal('fetch', fetch);
  const calls = (path: string) => seen.filter((s) => s.path === path);
  return { fetch, seen, calls };
}
