// @vitest-environment node
// New for agent-router: the scope's contracts (docs/web-console-contracts.md, "Scope"). No old
// check drove the dashboard's scope, so these are new specs, not ported ones.
import { describe, expect, it } from 'vitest';
import {
  choose,
  chooseProvider,
  clearScope,
  crumbs,
  isScoped,
  parseProvider,
  parseRange,
  pickerMatches,
  pickerPlaceholder,
  PROVIDERS,
  RANGE_ORDER,
  readQuery,
  sessionsHeld,
  setRange,
  sinceMs,
  toggle,
  UNSCOPED,
  DEFAULT_RANGE,
  DEFAULT_TAB,
  TABS,
  writeQuery,
  type OverviewAddress,
  type Scope,
} from './scope';

const NOW = Date.UTC(2026, 9, 5, 12, 0, 0);
const S: Scope = { provider: null, account: 'acct', project: '/code/a', session: 'sess-1', range: '7d' };

describe('scope', () => {
  it('OV-C22: choosing a level clears the levels below it and keeps those above; a session sets its own project', () => {
    expect(choose(S, 'account', 'other')).toEqual({ provider: null, account: 'other', project: null, session: null, range: '7d' });
    expect(choose(S, 'project', '/code/b')).toEqual({ provider: null, account: 'acct', project: '/code/b', session: null, range: '7d' });
    expect(choose(UNSCOPED, 'session', 'sess-2', '/code/c')).toEqual({ provider: null, account: null, project: '/code/c', session: 'sess-2', range: 'all' });
    // A level chosen under a provider keeps it.
    expect(choose({ ...S, provider: 'kimi' }, 'account', 'other').provider).toBe('kimi');
    // A session whose project was never resolved keeps the project in force.
    expect(choose({ ...UNSCOPED, project: '/code/a' }, 'session', 'sess-3', null).project).toBe('/code/a');
  });

  it('OV-C23: leaving a level does the same from its picker as from its list, where "All accounts" also cleared the project and session', () => {
    expect(choose(S, 'account', null)).toEqual({ ...S, account: null });
    expect(toggle(S, 'account', 'acct')).toEqual(choose(S, 'account', null));
    expect(toggle(S, 'project', '/code/a')).toEqual({ ...S, project: null, session: null });
    expect(toggle(S, 'session', 'sess-1')).toEqual({ ...S, session: null });
    expect(toggle(S, 'account', 'other')).toEqual({ provider: null, account: 'other', project: null, session: null, range: '7d' });
  });

  it('OV-C25: "Clear scope" shows exactly when a level is set, and clears all three keeping the range', () => {
    expect([isScoped(UNSCOPED), isScoped({ ...UNSCOPED, range: '1h' }), isScoped(S)]).toEqual([false, false, true]);
    expect(clearScope(S)).toEqual({ provider: null, account: null, project: null, session: null, range: '7d' });
  });

  it('OV-C26, OV-C28: five ranges, all by default, measured back from each read; anything else is all, never a property of every object', () => {
    expect(RANGE_ORDER).toEqual(['all', '1h', '24h', '7d', '30d']);
    expect([sinceMs('all', NOW), sinceMs('24h', NOW)]).toEqual([null, NOW - 86_400_000]);
    for (const bad of ['constructor', '__proto__', 'toString', '2d', '', null, 7]) expect(parseRange(bad), String(bad)).toBe('all');
    expect(setRange(S, 'hasOwnProperty').range).toBe('all');
  });

  it('OV-C29, OV-C31, OV-C215: the scope, the range, the open tab, the open rows and the request filters live in the address and read back as themselves', () => {
    const address: OverviewAddress = { scope: S, tab: 'requests', open: ['12', '13'], requests: { kind: 'errors', model: 'claude-opus-5', stop: 'end_turn' } };
    expect(readQuery(writeQuery(address))).toEqual(address);
    // The range is left out when it is the default the page starts at, and named otherwise.
    expect(writeQuery({ scope: { ...UNSCOPED, range: DEFAULT_RANGE } })).toBe('');
    expect(writeQuery({ scope: UNSCOPED })).toBe('?range=all');
    expect(readQuery('').scope.range).toBe(DEFAULT_RANGE);
    expect(DEFAULT_RANGE).toBe('1h');
    expect(readQuery('?kind=__proto__&range=__proto__')).toMatchObject({ scope: { range: 'all' }, requests: { kind: 'all' } });
  });

  it('OV-C227: the provider is in the address with the rest of the scope and reads back as itself; one the router does not know reads as every provider, never as a property of every object', () => {
    const address: OverviewAddress = { scope: { ...S, provider: 'kimi' }, tab: 'search', open: [], requests: { kind: 'all', model: null, stop: null } };
    expect(writeQuery(address)).toBe('?provider=kimi&account=acct&project=%2Fcode%2Fa&session=sess-1&range=7d&tab=search');
    expect(readQuery(writeQuery(address))).toEqual(address);
    expect(readQuery('?provider=anthropic&range=all').scope).toEqual({ ...UNSCOPED, provider: 'anthropic' });
    // Every provider is the default, and left out of the address.
    expect(readQuery('').scope.provider).toBeNull();
    expect(writeQuery({ scope: { ...UNSCOPED, provider: null } })).toBe('?range=all');
    expect(PROVIDERS).toEqual(['anthropic', 'kimi']);
    for (const bad of ['openai', 'Kimi', 'constructor', '__proto__', 'toString', '']) expect(readQuery(`?provider=${bad}`).scope.provider, bad).toBeNull();
    expect([parseProvider(null), parseProvider(7), parseProvider('kimi')]).toEqual([null, null, 'kimi']);
  });

  it('OV-C227: choosing a provider lets go of an account known to be another provider’s and keeps the project and the session; every provider keeps the account; Clear scope clears the provider too', () => {
    expect(chooseProvider(S, 'kimi', 'anthropic')).toEqual({ ...S, provider: 'kimi', account: null });
    expect(chooseProvider(S, 'kimi', 'kimi')).toEqual({ ...S, provider: 'kimi' });
    // An account whose provider neither the router's accounts nor the rows know is not let go.
    expect(chooseProvider(S, 'kimi', null)).toEqual({ ...S, provider: 'kimi' });
    expect(chooseProvider({ ...S, provider: 'kimi' }, null, 'kimi')).toEqual(S);
    expect(chooseProvider({ ...UNSCOPED, range: '24h' }, 'anthropic')).toEqual({ ...UNSCOPED, provider: 'anthropic', range: '24h' });
    // The provider narrows the page, so Clear scope shows for it, and clears it, keeping the range.
    expect(isScoped({ ...UNSCOPED, provider: 'kimi' })).toBe(true);
    expect(clearScope({ ...S, provider: 'kimi' })).toEqual({ ...UNSCOPED, range: '7d' });
  });

  it('OV-C215: the open tab is in the address as tab=, Activity by default and left out, and a tab the page does not know opens Activity', () => {
    expect(TABS).toEqual(['activity', 'context', 'projects', 'tools', 'requests', 'search']);
    expect(readQuery('').tab).toBe(DEFAULT_TAB);
    expect(DEFAULT_TAB).toBe('activity');
    expect(writeQuery({ scope: UNSCOPED, tab: 'activity' })).toBe('?range=all');
    expect(writeQuery({ scope: UNSCOPED, tab: 'tools', requests: { kind: 'errors', model: null, stop: null } })).toBe('?range=all&tab=tools&kind=errors');
    for (const bad of ['constructor', '__proto__', 'Tools', '']) expect(readQuery(`?tab=${bad}`).tab, bad).toBe('activity');
  });

  it('OV-C12, OV-C13: each crumb names its level as the lists do', () => {
    expect(crumbs(UNSCOPED)).toEqual({
      account: { label: 'All accounts', full: null, set: false },
      project: { label: 'All projects', full: null, set: false },
      session: { label: 'All sessions', full: null, set: false },
    });
    expect(crumbs(S, { accountLabel: 'work@example.test', branch: 'dev' })).toEqual({
      account: { label: 'work@example.test', full: 'acct', set: true },
      project: { label: 'a', full: '/code/a', set: true },
      session: { label: 'sess-1 (dev)', full: 'sess-1', set: true },
    });
    expect(crumbs(S).account.label, 'the router id until the summary knows the label').toBe('acct');
  });

  it("OV-C18: a picker's filter names what it filters, which the account picker's did not, and ignores case", () => {
    expect((['account', 'project', 'session'] as const).map(pickerPlaceholder)).toEqual(['Filter accounts…', 'Filter projects…', 'Filter sessions…']);
    const rows = [
      { name: 'agent-router', sub: '/code/agent-router' },
      { name: 'northstar', sub: '/code/NorthStar' },
    ];
    expect(pickerMatches(rows, 'NORTH').map((r) => r.name)).toEqual(['northstar']);
    expect(pickerMatches(rows, '  ')).toEqual(rows);
  });

  it('OV-C34: the session picker says when its list stopped at the 200 most recent, which it did not', () => {
    expect([sessionsHeld(200), sessionsHeld(1), sessionsHeld(12), sessionsHeld(null)]).toEqual(['200 most recent', '1 session', '12 sessions', '—']);
  });
});
