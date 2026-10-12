// @vitest-environment node
// New for agent-router: the two searches' contracts beyond what the old checks pinned, among them
// the ones the pages broke (docs/web-console-contracts.md, Overview "Search" and Sessions "Find").
import { describe, expect, it } from 'vitest';
import { search as dashboardReply } from './fixtures/dashboard';
import { search as liveReply } from './fixtures/live';
import { UNSCOPED } from './scope';
import { findParams, findView, searchFailed, searchIdle, searchParams, searchView, type SearchReply } from './search';

const NOW = Date.UTC(2026, 9, 5, 12, 0, 0);

describe('the Overview search', () => {
  it("OV-C24: a search's session row and hit scope the page to that session and its project, where the page set no project", () => {
    const view = searchView(dashboardReply(NOW), 'kestrel', UNSCOPED, 5, NOW);
    expect(view.sessions[0]?.scope).toEqual({ provider: null, account: null, project: '/code/agent-router', session: 'aaaaaaaa-1111-2222-3333-444444444444', range: 'all' });
    expect(view.hits[0]?.scope.project).toBe('/code/agent-router');
    // Pressed again, the session row leaves the session level (OV-C23), and the row is marked.
    const scoped = view.sessions[0]?.scope ?? UNSCOPED;
    const again = searchView(dashboardReply(NOW), 'kestrel', scoped, 5, NOW).sessions[0];
    expect(again?.selected).toBe(true);
    expect(again?.scope).toEqual({ ...scoped, session: null });
  });

  it('OV-C43, OV-C44, OV-C227: the text goes verbatim, trimmed, with the page’s whole scope — its provider, account, project, session and range', () => {
    expect(searchParams('  "exact phrase" OR pre*  ', 'tool_use', { provider: 'kimi', account: 'acct', project: '/code/a', session: 's', range: '1h' }, NOW)).toEqual({
      q: '"exact phrase" OR pre*',
      kind: 'tool_use',
      provider: 'kimi',
      account: 'acct',
      project: '/code/a',
      session: 's',
      since: String(NOW - 3600e3),
    });
    // Every provider, every account and every range send nothing; the range has no end, so no until.
    expect(searchParams('x', '', UNSCOPED, NOW)).toEqual({ q: 'x' });
  });

  it('OV-C44, OV-C227: the idle caption says when the scope narrows the search — its range, its provider or its account — which the page never did', () => {
    expect(searchIdle(UNSCOPED)).toBe('every message, tool call and result ever captured');
    expect(searchIdle({ ...UNSCOPED, range: '7d' })).toBe('every message, tool call and result captured in the current scope');
    expect(searchIdle({ ...UNSCOPED, account: 'acct' })).toBe('every message, tool call and result captured in the current scope');
    expect(searchIdle({ ...UNSCOPED, provider: 'kimi' })).toBe('every message, tool call and result captured in the current scope');
    expect(searchView({ hits: [] }, 'x', { ...UNSCOPED, provider: 'kimi' }, 3, NOW).empty).toBe('Nothing matched “x” in the current scope.');
  });

  it("OV-C45: a failed search reads in the router's own words, not its reply's JSON", () => {
    expect(searchFailed('database is locked')).toBe('Search failed: database is locked');
  });

  it("OV-C47: the caption says what came back and how long it took, and when the router's cap was reached, which it did not", () => {
    const many: SearchReply = { sessions: Array.from({ length: 200 }, (_, n) => ({ session_id: `s-${n}`, hits: 1, blocks: 1 })), hits: [] };
    expect(searchView(many, 'x', UNSCOPED, 12.4, NOW).caption).toBe(`${(200).toLocaleString()} sessions, the most the router lists · 12 ms`);
    expect(searchView({ sessions: [], hits: [] }, 'x', UNSCOPED, 3, NOW)).toMatchObject({ caption: 'nothing matched · 3 ms', empty: 'Nothing matched “x”.' });
    expect(searchView({ hits: [] }, 'x', { ...UNSCOPED, project: '/p' }, 3, NOW).empty).toBe('Nothing matched “x” in the current scope.');
  });

  it('OV-C51, OV-C52: a hit is its session, block and position together, and an unknown project reads "(unresolved)"', () => {
    const reply = dashboardReply(NOW);
    const first = reply.hits?.[0];
    if (!first) throw new Error('the fixture lost its hits');
    // The same block at another place is another hit.
    const view = searchView({ ...reply, hits: [...(reply.hits ?? []), { ...first, ordinal: 9 }] }, 'kestrel', UNSCOPED, 5, NOW);
    expect(view.hits.map((h) => h.key)).toEqual(['aaaaaaaa-1111-2222-3333-444444444444:1:4', 'bbbbbbbb-1111-2222-3333-444444444444:2:0', 'aaaaaaaa-1111-2222-3333-444444444444:1:9']);
    expect(view.hits[1]?.project).toBe('(unresolved)');
  });
});

describe("the live view's Find", () => {
  it('SE-C164: Find asks nothing for an empty query, and names the open session only when scoped to it', () => {
    expect(findParams('  ', 'session', 'aaa-1')).toBeNull();
    expect(findParams(' x ', 'session', 'aaa-1')).toEqual({ q: 'x', limit: '30', session: 'aaa-1' });
    expect(findParams('x', 'all', 'aaa-1')).toEqual({ q: 'x', limit: '30' });
    expect(findParams('x', 'session', null)).toEqual({ q: 'x', limit: '30' });
  });

  it('SE-C165: Find says when the router counts more matches than came back, which it never did', () => {
    expect(findView({ reply: liveReply(NOW) }, 'kestrel', 'all', null, NOW).more).toBe('2 best of 3 matches shown');
  });

  it('SE-C167: a Find that did not reach the router, or that the router failed, never reads "Nothing matched", where a 500 did', () => {
    expect(findView({ unreachable: true }, 'x', 'all', null, NOW).note).toBe('Could not reach the router.');
    expect(findView({ failed: 'database is locked' }, 'x', 'all', null, NOW).note).toBe('database is locked');
  });

  it('SE-C168: Find says what the router ran only when it differs from what was typed', () => {
    expect(findView({ reply: { ...liveReply(NOW), expression: '"kestrel" "roost"' } }, 'kestrel roost(', 'all', null, NOW).searched).toBe('Searched for "kestrel" "roost"');
    expect(findView({ reply: liveReply(NOW) }, 'kestrel', 'all', null, NOW).searched).toBeNull();
  });

  it('SE-C169, SE-C171: a hit from another session names its project, "—" when unknown, and its session; its key is session, block and ordinal', () => {
    const hit = liveReply(NOW).hits?.[1];
    if (!hit) throw new Error('the fixture lost its hits');
    const view = findView({ reply: { hits: [{ ...hit, project: null }] } }, 'x', 'all', null, NOW);
    expect(view.hits[0]).toMatchObject({ key: 'bbbbbbbb-1111-2222-3333-444444444444:2:0', where: '— · bbbbbbbb', age: '10m', role: 'user', kind: 'message' });
  });
});
