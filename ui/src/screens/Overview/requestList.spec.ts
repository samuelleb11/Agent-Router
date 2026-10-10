// @vitest-environment node
// New for agent-router: the request list's contracts (docs/web-console-contracts.md, "Requests"), on the
// rows it is handed, with no DOM.
import { describe, expect, it } from 'vitest';
import type { RequestRecord } from '~/data/overview';
import { UNSCOPED } from '~/model/scope';
import { badges, detail, figures, filtering, isError, isSubagent, moreOrFewer, parseContext, REQUEST_CAP, REQUEST_PAGE, requestsCaption, requestTime, toolsCalled } from './requestList';
import { request, S1 } from './fixtures';

const ALL = { kind: 'all', model: null, stop: null } as const;
const P95 = 3_200;

const LIST: RequestRecord[] = [
  request(1),
  request(2, { status: 500, error: 'upstream returned 500', ttfb_ms: 9_000 }),
  request(3, { is_sidechain: 1, agent_id: 'agent-1', model: 'claude-sonnet-4-5', stop_reason: 'end_turn' }),
  request(4, { ttfb_ms: 4_000, stop_reason: null }),
];

describe('the request list', () => {
  it('OV-C146: the filters narrow what is loaded, and the caption says "X of Y shown", what they were filtered from, and the 300 newest at the cap', () => {
    const f = filtering(LIST, { ...ALL, kind: 'errors' }, P95);
    expect(f.rows.map((r) => r.id)).toEqual([2]);
    expect(requestsCaption(LIST.length, f.rows.length, REQUEST_PAGE, UNSCOPED)).toBe('1 of 1 shown · filtered from 4');
    expect(requestsCaption(REQUEST_CAP, REQUEST_CAP, REQUEST_PAGE, UNSCOPED)).toBe('20 of 300 shown · 300 newest loaded — scope to a session for the rest');
  });

  it('OV-C147: the cap’s note offers "scope to a session" only where no session is scoped', () => {
    expect(requestsCaption(REQUEST_CAP, REQUEST_CAP, REQUEST_PAGE, { ...UNSCOPED, session: S1 })).toBe('20 of 300 shown · 300 newest loaded');
  });

  it('OV-C148: All always, and Errors, Subagents and Slow each with its count, only when some loaded row is of it', () => {
    expect(filtering(LIST, ALL, P95).kinds).toEqual([
      { kind: 'all', label: 'All', count: 4 },
      { kind: 'errors', label: 'Errors', count: 1 },
      { kind: 'subagents', label: 'Subagents', count: 1 },
      { kind: 'slow', label: 'Slow', count: 2 },
    ]);
    expect(filtering([request(1)], ALL, P95).kinds.map((k) => k.kind)).toEqual(['all']);
  });

  it('OV-C149: an error is a status of 400 or more or an error recorded, and a subagent request one marked as a sidechain', () => {
    expect([isError(request(1)), isError(request(1, { status: 429 })), isError(request(1, { status: 200, error: 'stream cut' }))]).toEqual([false, true, true]);
    expect([isSubagent(request(1)), isSubagent(request(1, { is_sidechain: 1 })), isSubagent(request(1, { is_sidechain: true }))]).toEqual([false, true, true]);
  });

  it('OV-C150: Slow is a first token slower than the p95, its threshold said in visible text, and without a p95 there is no Slow', () => {
    expect(filtering(LIST, ALL, P95).slowRule).toBe('first token slower than the p95 of 3.2 s');
    const none = filtering(LIST, { ...ALL, kind: 'slow' }, null);
    expect(none.slowRule).toBeNull();
    expect(none.kinds.map((k) => k.kind)).not.toContain('slow');
    expect(none.effective.kind).toBe('all');
  });

  it('OV-C151: every row the Slow filter keeps carries the slow badge, an error row included', () => {
    const slow = filtering(LIST, { ...ALL, kind: 'slow' }, P95).rows;
    expect(slow.map((r) => r.id)).toEqual([2, 4]);
    for (const r of slow) expect(badges(r, P95).slow, String(r.id)).toBe(true);
    expect(badges(LIST[1]!, P95)).toEqual({ subagent: false, slow: true, error: true });
  });

  it('OV-C152: a chip per model when more than one is loaded, and per stop reason, "(none)" for none; one value filters nothing', () => {
    const f = filtering(LIST, ALL, P95);
    expect(f.models.map((m) => [m.model, m.short, m.family, m.count])).toEqual([
      ['claude-opus-5-20260115', 'opus-5-20260115', 'opus', 3],
      ['claude-sonnet-4-5', 'sonnet-4-5', 'sonnet', 1],
    ]);
    expect(f.stops).toEqual([
      { stop: 'tool_use', count: 2 },
      { stop: 'end_turn', count: 1 },
      { stop: '(none)', count: 1 },
    ]);
    expect(filtering(LIST, { ...ALL, stop: '(none)' }, P95).rows.map((r) => r.id)).toEqual([4]);
    const one = filtering([request(1), request(2)], ALL, P95);
    expect([one.models, one.stops]).toEqual([[], []]);
  });

  it('OV-C153: a model or stop filter whose value has left the loaded rows is cleared, and a kind filter that is on stays offered and pressed', () => {
    const gone = filtering([request(1)], { kind: 'errors', model: 'claude-haiku-4-5', stop: 'max_tokens' }, P95);
    expect(gone.effective).toEqual({ kind: 'errors', model: null, stop: null });
    expect(gone.kinds).toContainEqual({ kind: 'errors', label: 'Errors', count: 0 });
    expect(gone.rows).toEqual([]);
  });

  it('OV-C154: any filter change returns to twenty rows; "Show N more" adds twenty at a time and "Show fewer" returns to twenty', () => {
    expect(REQUEST_PAGE).toBe(20);
    expect(moreOrFewer(300, 20)).toEqual({ more: 20 });
    expect(moreOrFewer(35, 20)).toEqual({ more: 15 });
    expect(moreOrFewer(35, 40)).toEqual({ fewer: true });
    expect(moreOrFewer(12, 20)).toBeNull();
  });

  it('OV-C156: a row reads its time with seconds today and with the date otherwise, and its figures, "—" cached without a prompt', () => {
    const now = Date.UTC(2026, 9, 5, 15, 0, 0);
    expect(requestTime(now - 60_000, now)).toMatch(/\d{1,2}:\d{2}:\d{2}/);
    expect(requestTime(now - 3 * 86_400_000, now)).not.toMatch(/:\d{2}:\d{2}/);
    expect(figures(request(1))).toEqual({ prompt: '30k', cached: '90% cached', output: '800 out', cost: '$0.050', firstToken: '900 ms' });
    expect(figures(request(1, { prompt_tokens: null, cost: null })).cached).toBe('—');
    // No usage to price: no cost, rather than $0.
    expect(figures(request(1, { prompt_tokens: null, output_tokens: null, cost: null })).cost).toBe('—');
  });

  it('OV-C229: a request with usage the router could not price reads "not priced", never "$0", and its detail says why', () => {
    const unpriced = request(5, { model: 'mystery-1', cost: null });
    expect(figures(unpriced).cost).toBe('not priced');
    expect(detail(unpriced, UNSCOPED).facts.filter((f) => f.label === 'cost').map((f) => f.value)).toEqual(['not priced — no rate for its model in the pricing table']);
    // A priced request's detail adds nothing: the card already says what it cost.
    expect(detail(request(5), UNSCOPED).facts.map((f) => f.label)).not.toContain('cost');
  });

  it('OV-C158: the detail gives every fact recorded, the tools in the order first called with ×N, the error, and the context window', () => {
    const d = detail(request(7, { is_sidechain: 1, agent_id: 'agent-7', error: 'boom\nagain', tools: 'Read,Bash,Read' }), UNSCOPED);
    expect(d.facts.map((f) => `${f.label}: ${String(f.value)}`)).toEqual([
      'request id: req_000007',
      'session: aaaaaaaa · agent-router',
      'whole reply: 9.0 s · first token 900 ms',
      'stopped: tool_use',
      'thinking: 100 tokens',
      'cache: 27k read · 2.0k written · 1.0k uncached',
      'agent id: agent-7',
      'tools called: Read ×2, Bash',
    ]);
    expect(d.error).toBe('boom\nagain');
    expect(d.context?.map((p) => [p.key, p.value])).toEqual([
      ['tools', 4_000],
      ['system', 2_000],
      ['history', 16_000],
      ['tool_results', 8_000],
    ]);
    // Scoped to a session, its session is not repeated on every row.
    expect(detail(request(7), { ...UNSCOPED, session: S1 }).facts.map((f) => f.label)).not.toContain('session');
    expect(parseContext('tools:5,system:x')).toEqual({ tools: 5, system: 0 });
    expect(toolsCalled(null)).toEqual([]);
  });
});
