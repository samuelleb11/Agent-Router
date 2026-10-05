// New for agent-router: how the console reads the router, as the Shell section of
// docs/web-console-contracts.md states it for every screen — one fetch layer and what it calls a
// refusal, a timeout on every request, the optional reads that never fail a cycle, and the one clock
// every re-read keeps. Driven through the modules every screen reads with, against a fetch stub.
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';
import type { ReactNode } from 'react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { act, renderHook, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { createHealthPoll, useLive } from '~/app/live';
import { TIMEOUT_MS, getJson, postJson } from '~/data/http';
import { SETTINGS_EVERY_MS, createPoll, readSnapshot, useAccounts, useAuth, useOverview } from '~/data/overview';
import { readHealth } from '~/data/router';
import { Refused, Unreachable, refusalText } from '~/lib/errors';
import { POLL_MS } from '~/lib/freshness';
import { UNSCOPED } from '~/model/scope';
import { ACCOUNTS, LOCAL, refused, router, summary } from '~/screens/Overview/fixtures';

/// Waits and limits generous enough for a machine busy with other work: a timeout here is a slow
/// machine, never a pass.
vi.setConfig({ testTimeout: 30_000 });

const src = join(__dirname, '..');

/// Every source file under src/, specs and test helpers left out.
function sources(dir = src): string[] {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) return sources(path);
    if (!/\.(ts|tsx)$/.test(name) || /\.spec\.(ts|tsx)$/.test(name) || /^test-(utils|setup)\./.test(name)) return [];
    return [path];
  });
}

/// A fetch that answers `respond` for every request, and a request the router never answers when
/// `respond` gives nothing: it waits until its caller gives up on it.
function stubAnswer(respond: (path: string) => Response | undefined) {
  const fetch = vi.fn((input: RequestInfo | URL, init?: RequestInit) => {
    const path = new URL(String(input), 'http://console.test').pathname;
    const answer = respond(path);
    if (answer) return Promise.resolve(answer);
    return new Promise<Response>((_, reject) => {
      init?.signal?.addEventListener('abort', () => reject(new DOMException('The operation was aborted.', 'AbortError')));
    });
  });
  vi.stubGlobal('fetch', fetch);
  return fetch;
}

const json = (data: unknown, status = 200) => new Response(JSON.stringify(data), { status, headers: { 'Content-Type': 'application/json' } });

/// What a promise came to, without letting a rejection go unhandled while the clock is moved.
function outcome<T>(p: Promise<T>) {
  const seen: { done: boolean; value?: T; error?: unknown } = { done: false };
  p.then(
    (value) => Object.assign(seen, { done: true, value }),
    (error: unknown) => Object.assign(seen, { done: true, error }),
  );
  return seen;
}

function wrapper() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return {
    client,
    wrapper: ({ children }: { children: ReactNode }) => <QueryClientProvider client={client}>{children}</QueryClientProvider>,
  };
}

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe('one fetch layer', () => {
  it('SH-C23: serves every screen: no module but ~/data/http calls fetch', () => {
    const callers = sources()
      .filter((path) => /(?<![\w.])fetch\(/.test(readFileSync(path, 'utf8')))
      .map((path) => relative(src, path));
    expect(callers).toEqual(['data/http.ts']);
  });

  it('SH-C23: a non-2xx, a 200 that carries a non-empty error, and any ok: false are each a refusal, read and written alike, and what is shown is the router’s own sentence, never a page from something in front of it', async () => {
    const cases: [string, () => Response, number, string][] = [
      ['a plain-text 403 from the guard', () => new Response('agent-router: Host header is not an address we serve', { status: 403 }), 403, 'agent-router: Host header is not an address we serve'],
      [
        'a 502 page from the ingress in front of the router',
        () =>
          new Response('<html>\r\n<head><title>502 Bad Gateway</title></head>\r\n<body>\r\n<center><h1>502 Bad Gateway</h1></center>\r\n<hr><center>nginx</center>\r\n</body>\r\n</html>\r\n', {
            status: 502,
            statusText: 'Bad Gateway',
            headers: { 'Content-Type': 'text/html' },
          }),
        502,
        'The router answered 502 Bad Gateway and gave no reason.',
      ],
      ['a 500 carrying error', () => json({ error: 'database is locked' }, 500), 500, 'database is locked'],
      ['a 200 carrying error', () => json({ error: 'the usage token was refused' }), 200, 'the usage token was refused'],
      ['a 200 with ok: false and a reason', () => json({ ok: false, reason: 'Cannot set permission mode to plan: the session is mid-turn' }), 200, 'Cannot set permission mode to plan: the session is mid-turn'],
      ['a 422 with ok: false and a reason', () => json({ ok: false, reason: 'the wrapper is not installed' }, 422), 422, 'the wrapper is not installed'],
      ['a 200 with ok: false and nothing said', () => json({ ok: false }), 200, 'The router refused this.'],
      ['a 503 with no body', () => new Response('', { status: 503 }), 503, 'The router answered 503 and gave no reason.'],
    ];
    for (const [what, reply, status, words] of cases) {
      for (const [verb, call] of [
        ['read', () => getJson('/api/quota')],
        ['write', () => postJson('/api/password', { password: 'x' })],
      ] as const) {
        stubAnswer(reply);
        const error = await call().then(
          () => null,
          (e: unknown) => e,
        );
        expect(error, `${what}, as a ${verb}`).toBeInstanceOf(Refused);
        expect((error as Refused).status, `${what}, as a ${verb}`).toBe(status);
        expect(refusalText(error), `${what}, as a ${verb}`).toBe(words);
      }
    }
    // An empty error is no refusal, and a body that refuses nothing is the answer.
    stubAnswer(() => json({ error: '', ok: true, five_hour: null }));
    await expect(getJson('/api/quota')).resolves.toEqual({ error: '', ok: true, five_hour: null });
    stubAnswer(() => json({ ok: true, active: 'work' }));
    await expect(postJson('/api/accounts/activate', { id: 'work' })).resolves.toEqual({ ok: true, active: 'work' });
  });
});

describe('a timeout on every request', () => {
  it('SH-C13: a read or a write the router never answers fails as unreachable after ten seconds, never sooner and never later', async () => {
    vi.useFakeTimers();
    for (const call of [() => getJson('/api/summary'), () => postJson('/api/accounts/activate', { id: 'work' })]) {
      stubAnswer(() => undefined);
      const seen = outcome(call());
      await vi.advanceTimersByTimeAsync(TIMEOUT_MS - 1);
      expect(seen.done).toBe(false);
      await vi.advanceTimersByTimeAsync(1);
      expect(seen.done).toBe(true);
      expect(seen.error).toBeInstanceOf(Unreachable);
      expect(refusalText(seen.error)).toBe('The router did not answer within 10 s.');
    }
  });

  it('SH-C13: a read that times out fails its cycle like any other failure, the Overview’s and the shell’s alike', async () => {
    vi.useFakeTimers();
    // Every read the Overview's cycle makes answers but one, which never does.
    stubAnswer((path) => (path === '/api/summary' ? undefined : path === '/api/quota' || path === '/api/forecast' ? json({}) : json([])));
    const overview = createPoll();
    const cycle = outcome(overview.cycle(() => readSnapshot(UNSCOPED)));
    await vi.advanceTimersByTimeAsync(TIMEOUT_MS);
    expect(cycle.done).toBe(true);
    expect(cycle.error).toBeInstanceOf(Unreachable);
    expect(overview.snapshot().strikes).toBe(1);

    stubAnswer(() => undefined);
    const shell = createHealthPoll();
    const health = outcome(shell.cycle(() => readHealth()));
    await vi.advanceTimersByTimeAsync(TIMEOUT_MS);
    expect(health.error).toBeInstanceOf(Unreachable);
    expect(shell.snapshot()).toEqual({ strikes: 1, lastOk: null });
  });
});

describe('the optional reads', () => {
  it('SH-C12: the quota and the forecast failing on their own fail no cycle and count no strike, and keep their last good value with its age; a core read failing is a strike', async () => {
    let failing: 'optional' | 'core' | null = null;
    router((path) => {
      if (failing === 'optional' && path === '/api/quota') return refused('the usage token was refused', 200);
      if (failing === 'optional' && path === '/api/forecast') return refused('forecast: no history yet', 500);
      if (failing === 'core' && path === '/api/summary') return refused('database is locked');
      return undefined;
    });
    const poll = createPoll();
    const read = () => poll.cycle(() => readSnapshot(UNSCOPED, { kept: poll.kept() }));
    const first = await read();
    expect(first.quota?.kept).toBe(false);
    expect(first.forecast?.kept).toBe(false);

    failing = 'optional';
    await new Promise((r) => setTimeout(r, 5));
    const second = await read();
    expect(poll.snapshot().strikes).toBe(0);
    expect(second.quota).toMatchObject({ kept: true, at: first.quota!.at, value: first.quota!.value });
    expect(second.forecast).toMatchObject({ kept: true, at: first.forecast!.at });
    expect(second.at).toBeGreaterThan(first.quota!.at);

    failing = 'core';
    await expect(read()).rejects.toBeInstanceOf(Refused);
    expect(poll.snapshot().strikes).toBe(1);
  });

  it('SH-C12: the router’s mode and its accounts, read beside the snapshot, failing keep their last good value and leave the Overview’s count alone', async () => {
    let failing = false;
    router((path) => (failing && (path === '/api/auth' || path === '/api/accounts') ? refused('keychain locked', 500) : undefined));
    const { client, wrapper: Wrapper } = wrapper();
    const { result } = renderHook(() => ({ overview: useOverview(UNSCOPED), auth: useAuth(), accounts: useAccounts() }), { wrapper: Wrapper });
    await waitFor(() => expect(result.current.overview.freshness).toBe('live'));
    await waitFor(() => expect(result.current.auth.value).toEqual(LOCAL));
    await waitFor(() => expect(result.current.accounts.value).toEqual(ACCOUNTS));

    failing = true;
    await act(() => client.refetchQueries({ queryKey: ['auth'] }));
    await act(() => client.refetchQueries({ queryKey: ['accounts'], exact: true }));
    await waitFor(() => expect(result.current.auth.stale).toBe(true));
    expect(result.current.auth.value).toEqual(LOCAL);
    expect(result.current.accounts.stale).toBe(true);
    expect(result.current.accounts.value).toEqual(ACCOUNTS);
    expect(result.current.overview.freshness).toBe('live');
    expect(result.current.overview.strikes).toBe(0);
  });
});

describe('one clock', () => {
  /// Requests to `path`, each with when it was asked and when it was answered on the fake clock.
  function timed(path: string, takesMs: number, fail = () => false) {
    const asked: { at: number; answered: number | null }[] = [];
    vi.stubGlobal(
      'fetch',
      vi.fn((input: RequestInfo | URL) => {
        const p = new URL(String(input), 'http://console.test').pathname;
        if (p !== path) return Promise.resolve(p === '/api/quota' || p === '/api/forecast' ? json({}) : p === '/api/summary' ? json(summary()) : p === '/_router/health' ? json({ ok: true }) : json([]));
        const call = { at: Date.now(), answered: null as number | null };
        asked.push(call);
        return new Promise<Response>((resolve) =>
          setTimeout(() => {
            call.answered = Date.now();
            resolve(fail() ? json({ error: 'database is locked' }, 500) : p === '/api/summary' ? json(summary()) : json({ ok: true }));
          }, takesMs),
        );
      }),
    );
    return asked;
  }

  for (const [who, path, use] of [
    ['the Overview', '/api/summary', () => useOverview(UNSCOPED)],
    ['the shell’s health check', '/_router/health', () => useLive()],
  ] as const) {
    it(`SH-C15: ${who} reads again five seconds after its last read finished, never while one is in flight, and never retries a failure quietly`, async () => {
      vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'setInterval', 'clearInterval', 'Date'] });
      let failing = false;
      const asked = timed(path, 3000, () => failing);
      const { wrapper: Wrapper } = wrapper();
      renderHook(() => use(), { wrapper: Wrapper });
      await act(() => vi.advanceTimersByTimeAsync(10));
      expect(asked).toHaveLength(1);
      // Answered at 3 s; nothing is asked at 5 s, the start plus a beat, while…
      await act(() => vi.advanceTimersByTimeAsync(POLL_MS - 10));
      expect(asked).toHaveLength(1);
      // …the next read is asked a beat after the answer, at 8 s.
      await act(() => vi.advanceTimersByTimeAsync(3000 - 100));
      expect(asked).toHaveLength(1);
      await act(() => vi.advanceTimersByTimeAsync(200));
      expect(asked).toHaveLength(2);
      expect(asked[1]!.at - asked[0]!.answered!).toBeGreaterThanOrEqual(POLL_MS);
      // Never two at once.
      for (let i = 1; i < asked.length; i++) expect(asked[i]!.at).toBeGreaterThanOrEqual(asked[i - 1]!.answered ?? Infinity);

      // A failed read is not asked again until the next beat: no quiet retry. The second read was
      // asked at 8 s and fails at 11 s; the third is asked at 16 s and not before.
      failing = true;
      await act(() => vi.advanceTimersByTimeAsync(3000));
      expect(asked).toHaveLength(2);
      expect(asked[1]!.answered).not.toBeNull();
      await act(() => vi.advanceTimersByTimeAsync(POLL_MS - 200));
      expect(asked).toHaveLength(2);
      await act(() => vi.advanceTimersByTimeAsync(200));
      expect(asked).toHaveLength(3);
      expect(asked[2]!.at - asked[1]!.answered!).toBeGreaterThanOrEqual(POLL_MS);
    });
  }

  it('SH-C15: the router’s mode and its accounts are read again every 30 s', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'setInterval', 'clearInterval', 'Date'] });
    const r = router();
    const { wrapper: Wrapper } = wrapper();
    renderHook(() => ({ auth: useAuth(), accounts: useAccounts() }), { wrapper: Wrapper });
    await act(() => vi.advanceTimersByTimeAsync(10));
    expect(r.calls('/api/auth')).toHaveLength(1);
    expect(r.calls('/api/accounts')).toHaveLength(1);
    await act(() => vi.advanceTimersByTimeAsync(SETTINGS_EVERY_MS - 100));
    expect(r.calls('/api/auth')).toHaveLength(1);
    await act(() => vi.advanceTimersByTimeAsync(200));
    expect(r.calls('/api/auth')).toHaveLength(2);
    expect(r.calls('/api/accounts')).toHaveLength(2);
  });

  it('SH-C15: a view reads again at once when its tab is shown again', async () => {
    const r = router();
    let health = 0;
    const base = globalThis.fetch;
    vi.stubGlobal(
      'fetch',
      vi.fn((input: RequestInfo | URL, init?: RequestInit) => {
        if (new URL(String(input), 'http://console.test').pathname === '/_router/health') {
          health += 1;
          return Promise.resolve(json({ ok: true }));
        }
        return base(input, init);
      }),
    );
    const { wrapper: Wrapper } = wrapper();
    const { result } = renderHook(() => ({ overview: useOverview(UNSCOPED), live: useLive() }), { wrapper: Wrapper });
    await waitFor(() => expect(result.current.overview.freshness).toBe('live'));
    await waitFor(() => expect(result.current.live.mode).toBe('live'));
    const before = { summary: r.calls('/api/summary').length, health };
    // At once: by the time the event has been handled, never at the next tick.
    await act(async () => {
      window.dispatchEvent(new Event('visibilitychange'));
    });
    expect(r.calls('/api/summary').length).toBe(before.summary + 1);
    expect(health).toBe(before.health + 1);
  });
});
