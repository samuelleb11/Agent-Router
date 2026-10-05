// New for agent-router: what the Sessions specs share, and the screen never imports. The router's
// answers in its own shapes, a fetch that answers by method, path and query, and the screen rendered
// at an address. The router's clock is set seven minutes behind the spec's, so a duration measured
// on the wrong clock shows (SE-C212).
import { Fragment, createElement, type ReactNode } from 'react';
import { act, configure } from '@testing-library/react';
import { vi } from 'vitest';
import { renderScreen } from '~/test-utils';
import type { SessionRow } from '~/model/session';
import { FakeEventSource } from './fakes';
import Sessions from './index';

/// A screen spec waits for its conditions, and on a machine busy with other work a condition can take
/// longer than the defaults allow: a timeout here is a slow machine, never a pass. So the waits are
/// long, and a test's own limit longer still, so that a slow one fails by its own assertion rather
/// than leaving its work running into the next test.
configure({ asyncUtilTimeout: 10_000 });
vi.setConfig({ testTimeout: 60_000 });

export const NOW = Date.UTC(2026, 9, 5, 12, 0, 0);
/// The router's clock.
export const AT = NOW - 7 * 60e3;

/// A session as a current router lists it.
export function session(id: string, extra: Partial<SessionRow> = {}): SessionRow {
  return {
    session_id: id,
    cwd: '/Users/sample/code/agent-router',
    project: 'agent-router',
    git_branch: 'main',
    title: `Session ${id}`,
    created_ms: AT - 3600e3,
    last_activity_ms: AT - 12e3,
    active: true,
    state: 'working',
    archived: false,
    wrapped: true,
    bytes: 1,
    activity: 'generating',
    open: true,
    detail: 'Thinking',
    since_ms: AT - 12e3,
    tone: 'busy',
    source: 'registry',
    as_of_ms: AT,
    ...extra,
  };
}

/// Two sessions in one project and one in another.
export const A = 'aaaaaaaa-1111-2222-3333-444444444444';
export const B = 'bbbbbbbb-1111-2222-3333-444444444444';
export const C = 'cccccccc-1111-2222-3333-444444444444';

export const SESSIONS = (): SessionRow[] => [
  session(A, { title: 'Port the live view', created_ms: AT - 7200e3 }),
  session(B, { title: 'Write the specs', activity: 'waiting', tone: 'attention', state: 'ask', detail: 'Waiting for you', created_ms: AT - 3600e3 }),
  session(C, { cwd: '/Users/sample/code/northstar', project: 'northstar', title: 'Elsewhere', activity: 'idle', tone: 'idle', state: 'idle', active: false, detail: 'Waiting for you' }),
];

export const OPTIONS = {
  models: ['opus', 'sonnet', 'haiku'],
  efforts: ['low', 'medium', 'high'],
  modes: [
    { value: 'default', label: 'Ask' },
    { value: 'acceptEdits', label: 'Accept edits' },
    { value: 'bypassPermissions', label: 'Bypass' },
  ],
};

export const STATS: Readonly<Record<string, object>> = {
  [A]: { model: 'claude-opus-4-1', effort: 'high', mode: 'default', total: 1_234_000, output: 56_000, cache_read: 900_000, turns: 12, cost: 4.56 },
  [B]: { model: 'claude-sonnet-4-5', effort: 'low', mode: 'acceptEdits', total: 3_400, output: 512, cache_read: 0, turns: 1, cost: 0.1234 },
  [C]: { model: null, total: 0, output: 0, cache_read: 0, turns: 0, cost: null },
};

export const json = (data: unknown, status = 200) => new Response(JSON.stringify(data), { status, headers: { 'Content-Type': 'application/json' } });

export interface Asked {
  readonly method: string;
  readonly path: string;
  readonly query: URLSearchParams;
  readonly body: Record<string, unknown> | undefined;
  readonly signal: AbortSignal | undefined;
}

/// A reply that never comes, until the caller gives up on it, as a router that does not answer.
export const hang = (req: Asked) =>
  new Promise<Response>((_, reject) => {
    req.signal?.addEventListener('abort', () => reject(new DOMException('The operation was aborted.', 'AbortError')));
  });

/// Replies held until the spec lets them go, every one held so far with the same answer.
export function held(): { readonly reply: (req: Asked) => Promise<Response>; readonly release: (r: Response) => void; readonly waiting: () => number } {
  let waiting: ((r: Response) => void)[] = [];
  const reply = () =>
    new Promise<Response>((resolve) => {
      waiting.push(resolve);
    });
  const release = (r: Response) => {
    const now = waiting;
    waiting = [];
    // A fresh Response each, since a body is read once.
    for (const resolve of now) resolve(r.clone());
  };
  return { reply, release, waiting: () => waiting.length };
}

export type Handler = (req: Asked) => Response | Promise<Response> | undefined;

/// The router's answers by default: the list, a local router with every extension wrapped, the
/// options, each session's totals, nothing waiting.
export function defaults(rows: () => readonly SessionRow[] = SESSIONS): Handler {
  return (req) => {
    if (req.method !== 'GET') return undefined;
    switch (req.path) {
      case '/api/live/sessions':
        return json(rows());
      case '/api/auth':
        return json({ mode: 'local', configured: false });
      case '/api/setup/shim':
        return json({ targets: [{ extension: 'anthropic.claude-code-2.1.0', state: 'installed' }] });
      case '/api/live/options':
        return json(OPTIONS);
      case '/api/live/stats':
        return json(STATS[req.query.get('session') ?? ''] ?? { total: 0, turns: 0 });
      case '/api/live/project':
        return json({ sessions: 2, total: 1_237_400, turns: 13, cost: 4.68 });
      case '/api/live/prompt':
        return json({ pending: false, count: 0 });
      default:
        return undefined;
    }
  };
}

/// Replace `fetch` with one that asks `handler` first and the defaults after, keeps every request,
/// and fails loudly for one neither answers. Undo it with `vi.unstubAllGlobals()`.
export function stubRouter(handler: Handler = () => undefined, base: Handler = defaults()) {
  const asked: Asked[] = [];
  const fetch = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const href = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
    const url = new URL(href, 'http://console.test');
    const method = (init?.method ?? 'GET').toUpperCase();
    const body = init?.body ? (JSON.parse(String(init.body)) as Record<string, unknown>) : undefined;
    const req: Asked = { method, path: url.pathname, query: url.searchParams, body, signal: init?.signal ?? undefined };
    asked.push(req);
    const res = (await handler(req)) ?? (await base(req));
    if (!res) throw new Error(`test fetch: no answer for ${method} ${url.pathname}${url.search}`);
    return res;
  });
  vi.stubGlobal('fetch', fetch);
  return {
    fetch,
    asked,
    posts: (path: string) => asked.filter((a) => a.method === 'POST' && a.path === path),
    gets: (path: string) => asked.filter((a) => a.method === 'GET' && a.path === path),
  };
}

/// The Sessions screen at an address, its streams through the fake, with `beside` drawn next to it
/// inside the same router: a spec's way to press the browser's Back.
export function renderSessions(at = '/live', beside: ReactNode = null) {
  vi.stubGlobal('EventSource', FakeEventSource);
  return renderScreen(createElement(Fragment, null, createElement(Sessions), beside), { path: '/live', at });
}

/// Let every reply already given be read and drawn, before asserting that something did NOT
/// happen: an absence has no condition to wait for, so the queue is run dry instead (SE-C209). A
/// presence is waited for with `waitFor` or `findBy…`, never with this.
export async function drain(): Promise<void> {
  await act(async () => {
    for (let i = 0; i < 5; i++) await new Promise((r) => setTimeout(r, 0));
  });
}

/// The stream opened for a session, the newest one.
export function streamOf(id: string): FakeEventSource {
  const all = FakeEventSource.made.filter((s) => s.url === `/api/live/stream?session=${encodeURIComponent(id)}`);
  const s = all[all.length - 1];
  if (!s) throw new Error(`no stream was opened for ${id}`);
  return s;
}

/// What the router's stream says while a session works.
export const working = (extra: object = {}) => ({
  state: 'working',
  detail: 'Running Bash',
  activity: 'tool',
  tone: 'busy',
  as_of_ms: AT,
  since_ms: AT - 12e3,
  quiet_ms: 12e3,
  queued: 0,
  ...extra,
});
