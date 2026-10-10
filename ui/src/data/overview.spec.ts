// New for agent-router: the Overview's reads (docs/web-console-contracts.md, "Scope", "All accounts",
// "Lists" and "Refresh"): what each cycle asks the router, what it keeps, and how its failures count.
import { afterEach, describe, expect, it, vi } from 'vitest';
import { Unreachable } from '~/lib/errors';
import { TABS, type OverviewTab, type Scope } from '~/model/scope';
import { createPoll, freshnessOf, keepReading, readAccounts, readSnapshot, snapshotPaths, UNREACHABLE_AFTER, type Snapshot } from './overview';
import { dropped, held, kimiBlock, ok, partsOf, QUOTA, refused, router, forecast, summary } from '~/screens/Overview/fixtures';
import { TIMEOUT_MS } from './http';

const NOW = Date.UTC(2026, 9, 5, 15, 0, 0);
const SCOPED: Scope = { provider: null, account: 'secondary', project: '/code/agent-router', session: 'aaaaaaaa-1', range: '24h' };
const query = (path: string) => Object.fromEntries(new URL(path, 'http://x').searchParams);

afterEach(() => {
  vi.unstubAllGlobals();
  vi.useRealTimers();
});

describe('what a cycle asks', () => {
  const p = snapshotPaths(SCOPED, 'projects', NOW - 86_400_000, -240);

  it('OV-C27: the range narrows every read but the quota, measured back from the moment of the read', () => {
    const since = String(NOW - 86_400_000);
    for (const path of [p.summary, p.projects, p.sessions, p.tools, p.requests, p.forecast]) expect(query(path).since, path).toBe(since);
    expect(p.quota).toBe('/api/quota');
    // "all" asks for no lower bound at all.
    expect(Object.keys(query(snapshotPaths({ ...SCOPED, range: 'all' }, 'activity', null, 0).summary))).not.toContain('since');
  });

  it("OV-C111: the summary goes with the viewer's UTC offset, so the router buckets on the viewer's clock", () => {
    expect(query(p.summary).tz).toBe('-240');
  });

  it('OV-C131: a navigator is never narrowed by the level it picks: projects by the account and range, sessions by the account, project and range', () => {
    expect(query(p.projects)).toEqual({ account: 'secondary', since: String(NOW - 86_400_000) });
    expect(query(p.sessions)).toEqual({ account: 'secondary', project: '/code/agent-router', since: String(NOW - 86_400_000) });
    expect(query(p.summary)).toMatchObject({ account: 'secondary', project: '/code/agent-router', session: 'aaaaaaaa-1' });
  });

  it('OV-C54, OV-C87: the quota is the serving account’s whatever the scope, and all accounts covers every account, only its usage following the range', () => {
    expect(query(p.quota)).toEqual({});
    expect(query(p.forecast)).toEqual({ since: String(NOW - 86_400_000) });
  });

  it('OV-C227: the provider narrows every read the scope does — the summary, projects, sessions, tools and requests — and never the quota or the forecast', () => {
    const kimi = snapshotPaths({ ...SCOPED, provider: 'kimi' }, 'projects', NOW - 86_400_000, -240);
    for (const path of [kimi.summary, kimi.projects, kimi.sessions, kimi.tools, kimi.requests]) expect(query(path).provider, path).toBe('kimi');
    expect(query(kimi.projects)).toEqual({ provider: 'kimi', account: 'secondary', since: String(NOW - 86_400_000) });
    expect(kimi.quota).toBe('/api/quota');
    expect(query(kimi.forecast)).toEqual({ since: String(NOW - 86_400_000) });
    // Every provider sends none.
    for (const path of Object.values(p)) expect(query(path), path).not.toHaveProperty('provider');
  });

  it('OV-C167: the accounts are read without a credential check, which only Settings asks for', async () => {
    const r = router();
    await readAccounts();
    expect(r.seen.map((s) => `${s.path}?${s.query}`)).toEqual(['/api/accounts?']);
  });
});

describe('one snapshot', () => {
  it('OV-C160: the top and the open tab come from one cycle of reads, scoped alike, drawn whole or not at all', async () => {
    const r = router();
    const s = await readSnapshot(SCOPED, 'requests', { now: NOW, tz: 0 });
    // The scoped session's crumb asks for the sessions on any tab.
    expect(r.seen.map((x) => x.path).sort()).toEqual(['/api/forecast', '/api/quota', '/api/requests', '/api/sessions', '/api/summary']);
    for (const path of ['/api/summary', '/api/requests', '/api/sessions']) expect(r.calls(path)[0]!.query.get('project'), path).toBe('/code/agent-router');
    expect(s).toMatchObject({ scope: SCOPED, since: NOW - 86_400_000, part: { tab: 'requests' } });
    // One core read failing fails the whole cycle: nothing of it is drawn.
    router((path) => (path === '/api/requests' ? refused('database is locked') : undefined));
    await expect(readSnapshot(SCOPED, 'requests', { now: NOW })).rejects.toThrow('database is locked');
  });

  it('OV-C216, OV-C224: each cycle reads the top’s summary parts, a quota block per provider among them, the quota and the forecast, plus the open tab’s own reads, and nothing for a tab that is not open', async () => {
    // `providers` holds every provider's block; Anthropic's `quota`, `account` and `quota_history` are
    // the phones' to read.
    const top = ['totals', 'models', 'latency', 'timeline', 'providers'];
    const own: Record<OverviewTab, { readonly paths: readonly string[]; readonly parts: readonly string[] }> = {
      activity: { paths: [], parts: [] },
      context: { paths: [], parts: ['context', 'threads'] },
      projects: { paths: ['/api/projects', '/api/sessions'], parts: ['by_account'] },
      tools: { paths: ['/api/tools'], parts: [] },
      requests: { paths: ['/api/requests'], parts: [] },
      search: { paths: [], parts: [] },
    };
    const unscoped: Scope = { provider: null, account: null, project: null, session: null, range: '24h' };
    for (const tab of TABS) {
      const r = router();
      const s = await readSnapshot(unscoped, tab, { now: NOW });
      expect(r.seen.map((x) => x.path).sort(), tab).toEqual(['/api/forecast', '/api/quota', '/api/summary', ...own[tab].paths].sort());
      expect(r.calls('/api/summary')[0]!.query.get('parts')!.split(',').sort(), tab).toEqual([...top, ...own[tab].parts].sort());
      expect(s.part.tab).toBe(tab);
    }
    // The crumbs of a scoped account and a scoped session ask for what names them, on any tab (OV-C13).
    const r = router();
    const s = await readSnapshot({ ...unscoped, account: 'secondary', session: 'aaaaaaaa-1111-4111-8111-111111111111' }, 'activity', { now: NOW });
    expect(r.calls('/api/summary')[0]!.query.get('parts')!.split(',')).toContain('by_account');
    expect(r.calls('/api/sessions')).toHaveLength(1);
    expect(s.session?.git_branch).toBe('dev');
  });

  it('OV-C64: a quota or forecast read that fails never fails the cycle, and keeps its last good reading with its age', async () => {
    router();
    const first = await readSnapshot(SCOPED, 'activity', { now: NOW });
    expect(first.quota).toMatchObject({ value: QUOTA, kept: false, for: 'secondary' });
    router((path) => (path === '/api/quota' || path === '/api/forecast' ? dropped() : undefined));
    const second = await readSnapshot(SCOPED, 'activity', { now: NOW, kept: { quota: first.quota, forecast: first.forecast } });
    expect(second.quota).toEqual({ ...first.quota, kept: true });
    expect(second.forecast).toEqual({ ...first.forecast, kept: true });
    expect(second.summary.totals).toEqual(summary().totals);
  });

  it('OV-C65, OV-C224: a quota reading is kept for Anthropic’s serving account, named by its block, and on a router with no Anthropic account for none', async () => {
    router();
    const both = await readSnapshot(SCOPED, 'activity', { now: NOW });
    expect(both.quota?.for).toBe('secondary');
    // Kimi Code's accounts alone: `/api/quota` refuses, and a kept reading is for no account.
    router((path, query) =>
      path === '/api/summary'
        ? ok(partsOf(summary({ providers: [kimiBlock()] }), query.get('parts')))
        : path === '/api/quota'
          ? refused('no Anthropic account')
          : undefined,
    );
    const kimiOnly = await readSnapshot(SCOPED, 'activity', { now: NOW, kept: { quota: both.quota, forecast: null } });
    expect(kimiOnly.quota).toBeNull();
    expect(kimiOnly.summary.providers.map((b) => b.provider)).toEqual(['kimi']);
  });

  it('OV-C65: a kept reading is shown only for the account and the range it was read for', () => {
    const quota = { value: QUOTA, at: NOW - 60_000, kept: false, for: 'secondary' };
    expect(keepReading({ ok: false }, quota, 'secondary', NOW)).toEqual({ ...quota, kept: true });
    // The serving account changed: the old account's quota is not this one's.
    expect(keepReading({ ok: false }, quota, 'primary', NOW)).toBeNull();
    const fc = { value: forecast(), at: NOW - 60_000, kept: false, for: '24h' };
    // The range changed: the previous range's spend is not this range's.
    expect(keepReading({ ok: false }, fc, '7d', NOW)).toBeNull();
    expect(keepReading({ ok: true, value: forecast() }, fc, '7d', NOW)).toMatchObject({ kept: false, for: '7d', at: NOW });
  });

  it('OV-C165: every read has a timeout, and one that runs out fails its cycle', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
    router((path) => (path === '/api/sessions' ? new Promise<Response>(() => {}) : undefined));
    const cycle = readSnapshot(SCOPED, 'activity', { now: NOW });
    const failed = expect(cycle).rejects.toBeInstanceOf(Unreachable);
    await vi.advanceTimersByTimeAsync(TIMEOUT_MS + 1);
    await failed;
  });
});

describe('the poll', () => {
  const snap = (at: number) => ({ at, quota: null, forecast: null }) as unknown as Snapshot;

  it('OV-C164: failed cycles, not HTTP attempts, mark it stale; five in a row unreachable; a good cycle clears both; a cancelled one is neither', async () => {
    const poll = createPoll();
    expect(freshnessOf(poll.snapshot(), false)).toBe('connecting');
    await poll.cycle(async () => snap(NOW));
    expect(poll.snapshot()).toEqual({ strikes: 0, lastOk: NOW });
    expect(freshnessOf(poll.snapshot(), true)).toBe('live');
    for (let i = 1; i <= UNREACHABLE_AFTER; i++) {
      await expect(poll.cycle(async () => Promise.reject(new Unreachable()))).rejects.toThrow();
      expect(poll.snapshot().strikes).toBe(i);
      expect(freshnessOf(poll.snapshot(), true)).toBe(i < UNREACHABLE_AFTER ? 'stale' : 'unreachable');
    }
    // The time of the last good read is kept through every failure.
    expect(poll.snapshot().lastOk).toBe(NOW);
    const cancelled = new AbortController();
    cancelled.abort();
    await expect(poll.cycle(async () => Promise.reject(new DOMException('gone', 'AbortError')), cancelled.signal)).rejects.toThrow();
    expect(poll.snapshot().strikes).toBe(UNREACHABLE_AFTER);
    await poll.cycle(async () => snap(NOW + 1));
    expect(poll.snapshot()).toEqual({ strikes: 0, lastOk: NOW + 1 });
  });

  it('OV-C162: a read whose scope was superseded while it ran is never counted as drawn', async () => {
    const poll = createPoll();
    const gate = held();
    const r = router((path) => (path === '/api/summary' ? gate.promise : undefined));
    const superseded = new AbortController();
    const cycle = poll.cycle(() => readSnapshot(SCOPED, 'activity', { now: NOW, signal: superseded.signal }), superseded.signal);
    const settled = expect(cycle).rejects.toThrow();
    superseded.abort();
    gate.release(ok(summary()));
    await settled;
    expect(poll.snapshot()).toEqual({ strikes: 0, lastOk: null });
    expect(r.calls('/api/summary')[0]!.signal?.aborted).toBe(true);
  });
});
