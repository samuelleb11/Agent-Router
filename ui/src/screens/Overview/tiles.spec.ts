// @vitest-environment node
// New for agent-router: Totals in scope's contracts (docs/web-console-contracts.md, "Headline" and
// "Formatting"), on the totals each tile is handed, with no DOM.
import { describe, expect, it } from 'vitest';
import { ESTIMATED_WHY, UNPRICED_WHY, tod } from '~/model/format';
import { UNSCOPED } from '~/model/scope';
import { cacheRates, headline, providerSplit, spendSplit, spendValue, totalsCaption, type TileKey } from './tiles';
import { listErrors } from './requestList';
import { HOUR, MIN, providerTotals, totals, TWO_PROVIDERS } from './fixtures';

const NOW = Date.UTC(2026, 9, 5, 15, 0, 0);
const LATENCY = { ttfb_p50: 812, ttfb_p95: 3_200, duration_p50: 9_400, duration_p95: 31_000, duration_max: 64_000 };
const tiles = (t = totals({ first_seen: NOW - 3 * HOUR, last_seen: NOW - 2 * MIN }), scope = UNSCOPED) => headline(t, LATENCY, scope, NOW);
const tile = (key: string, list = tiles()) => list.find((t) => t.key === key)!;

describe('Totals in scope', () => {
  it('OV-C98: eight tiles, the largest first, each with its meaning as visible text under its value', () => {
    const list = tiles();
    expect(list.map((t) => t.label)).toEqual(['Prompt tokens read', 'Spend', 'Requests', 'Output tokens', 'Served from cache', 'Cache writes', 'Time to first token', 'Errors']);
    for (const t of list) expect(t.meaning, t.label).toMatch(/\w.+\./);
  });

  it('OV-C99, OV-C230: the largest says what it counts and over what span, and with nothing recorded how to start, naming each client', () => {
    expect(tile('prompt')).toMatchObject({ value: '12.40M', detail: `${tod(NOW - 3 * HOUR, NOW)} → ${tod(NOW - 2 * MIN, NOW)} · everything the model read, cached or not` });
    expect(tile('prompt', tiles(totals({ first_seen: null, last_seen: null, prompt_tokens: 0 }))).detail).toBe('nothing recorded yet — point Claude Code or Kimi Code at this router');
  });

  it('OV-C100: Spend is subtitled with cached reads and output, and with nothing priced says so', () => {
    expect(tile('spend')).toMatchObject({ value: '$41.20', detail: '10.40M cached reads · 310k out', marks: [] });
    expect(tile('spend', tiles(totals({ cost: 0 })))).toMatchObject({ value: null, detail: 'nothing priced in scope' });
  });

  it('OV-C101, OV-C228: Spend opens into its full split, the 04efe6b one, its cache rows saying what happened and each provider’s rates after them', () => {
    expect(spendSplit(totals()).map((s) => `${s.value} ${s.label}`)).toEqual([
      '$41.20 total in scope',
      '600k uncached input',
      '1.10M written to cache, 5 minutes',
      '300k written to cache, 1 hour',
      '10.40M read from cache',
      '310k output — dearest per token',
      '84% of prompt served from cache',
      '412 requests',
      // Anthropic's alone in scope, so its rates alone.
      '1.25× input Anthropic · written to cache, 5 minutes',
      '2× input Anthropic · written to cache, 1 hour',
      '0.1× input Anthropic · read from cache',
      'editable in the pricing table rates',
    ]);
  });

  it('OV-C228: the cache figures’ labels say what happened, and each provider’s own rates are in the tooltip, keyed in its colour', () => {
    // No label quotes a provider's multiplier: the rates are in the tip, each provider's its own.
    expect(tile('writes').meaning).toBe('Prompt tokens written to the cache, to be read back for less; each provider prices them at its own rates.');
    for (const row of spendSplit(totals()).slice(0, 8)) expect(row.label, row.label).not.toMatch(/×/);
    expect(cacheRates(totals({ by_provider: TWO_PROVIDERS })).map((r) => [r.value, r.label, r.colour])).toEqual([
      ['1.25× input', 'Anthropic · written to cache, 5 minutes', '--ar-tok'],
      ['2× input', 'Anthropic · written to cache, 1 hour', '--ar-tok'],
      ['0.1× input', 'Anthropic · read from cache', '--ar-tok'],
      // Kimi Code's writes cost what an uncached input does, and its reads differ by model (src/pricing.rs).
      ['1× input', 'Kimi Code · written to cache', '--ar-model-kimi'],
      ['0.1–0.2× input', 'Kimi Code · read from cache, by model', '--ar-model-kimi'],
    ]);
    // A scope narrowed to Kimi Code gives Kimi Code's rates alone; a provider the console has no rates
    // for says where they are kept.
    expect(cacheRates(totals({ by_provider: [providerTotals('kimi')] })).map((r) => r.label)).toEqual(['Kimi Code · written to cache', 'Kimi Code · read from cache, by model']);
    expect(cacheRates(totals({ by_provider: [providerTotals('mistral')] }))).toEqual([{ value: 'its own rates', label: 'mistral · in the pricing table', colour: '--ar-model-other' }]);
  });

  it('OV-C102: Requests says how many sessions and projects, and how many by subagents; scoped to a session, which project', () => {
    expect(tile('requests').detail).toBe('2 sessions · 2 projects · 27 by subagents');
    expect(tile('requests', tiles(totals({ sessions: 1, projects: 1, subagent_requests: 0 }))).detail).toBe('1 session · 1 project');
    expect(tile('requests', tiles(totals(), { ...UNSCOPED, session: 'aaaaaaaa-1', project: '/code/agent-router' })).detail).toBe('1 session · agent-router');
  });

  it('OV-C103: Output tokens says how much of it was thinking', () => {
    expect(tile('output')).toMatchObject({ value: '310k', detail: '42k of it thinking' });
  });

  it('OV-C104: Served from cache is cache reads ÷ every prompt token read, "X of Y came from cache", and says when there are no prompt tokens', () => {
    expect(tile('cache')).toMatchObject({ value: '84%', detail: '10.40M of 12.40M came from cache' });
    expect(tile('cache', tiles(totals({ prompt_tokens: 0, cache_read_tokens: 0 })))).toMatchObject({ value: '—', detail: 'no prompt tokens yet' });
  });

  it('OV-C105: Cache writes gives the total with its 5-minute and 1-hour tiers', () => {
    expect(tile('writes')).toMatchObject({ value: '1.40M', detail: '5-minute 1.10M · 1-hour 300k' });
  });

  it('OV-C106: Time to first token gives the median, and every latency figure as text: p95, the whole reply’s median and p95, and the slowest', () => {
    expect(tile('latency')).toMatchObject({ value: '812 ms', detail: 'p95 3.2 s · whole reply 9.4 s, p95 31.0 s · slowest 64.0 s' });
    expect(headline(totals(), null, UNSCOPED, NOW).find((t) => t.key === 'latency')).toMatchObject({ value: null, detail: 'median · no requests yet' });
  });

  it('OV-C107: Errors gives the count, red when above zero, of how many requests, and its button lists them', () => {
    expect(tile('errors')).toMatchObject({ value: '3', detail: 'of 412 requests', tone: 'bad' });
    expect(tile('errors', tiles(totals({ errors: 0 }))).tone).toBeUndefined();
    expect([listErrors(3), listErrors(1)]).toEqual(['List the 3 errors', 'List the 1 error']);
  });

  it('OV-C108: the heading’s caption reads "N requests · <last seen>" once anything is recorded', () => {
    expect(totalsCaption(totals({ last_seen: NOW - 2 * MIN }), NOW)).toBe('412 requests · 2 min ago');
    expect(totalsCaption(totals({ first_seen: null }), NOW)).toBe('');
  });

  it('OV-C178, OV-C229: a total with requests the router could not price reads its priced part marked with how many, the reason in its tooltip; one with nothing priced is never "$0"', () => {
    const spend = tile('spend', tiles(totals({ unpriced: 22 })));
    expect(spend).toMatchObject({ value: '$41.20 + 22 unpriced', detail: '10.40M cached reads · 310k out', marks: [{ kind: 'unpriced', value: '22 unpriced', label: UNPRICED_WHY }] });
    expect(spendSplit(totals({ unpriced: 22 }))[0]).toEqual({ value: '$41.20 + 22 unpriced', label: 'total in scope' });
    // Nothing priced beside requests the router could not price: "not priced"; nothing at all: "—".
    expect(tile('spend', tiles(totals({ cost: 0, unpriced: 22 })))).toMatchObject({ value: 'not priced', detail: 'nothing priced in scope' });
    expect(spendValue(totals({ cost: 0 }))).toBeNull();
    expect(spendSplit(totals({ cost: 0 }))[0]).toEqual({ value: '—', label: 'total in scope' });
  });

  it('OV-C229: each provider’s part of Spend is its priced part, marked with how many it leaves out, the reason in its tooltip', () => {
    const split = providerSplit(totals({ by_provider: [TWO_PROVIDERS[0]!, { ...TWO_PROVIDERS[1]!, cost: 2.5, unpriced: 3 }] }), 'spend');
    expect(split?.map((p) => [p.provider, p.figure, p.share])).toEqual([
      ['anthropic', '$41.20', '94%'],
      ['kimi', '$2.50 + 3 unpriced', '5.7%'],
    ]);
    expect(split?.[1]?.marks).toEqual([{ kind: 'unpriced', value: '3 unpriced', label: UNPRICED_WHY }]);
    expect(split?.[0]?.marks).toEqual([]);
  });

  it('OV-C232: Spend, and a provider’s part of it, that rests on estimated tokens reads "≈" before it, the reason in its tooltip', () => {
    const t = totals({ cost: 41.2, estimated: 5, by_provider: [TWO_PROVIDERS[0]!, { ...TWO_PROVIDERS[1]!, cost: 2.5, unpriced: 0, estimated: 5 }] });
    expect(tile('spend', tiles(t))).toMatchObject({ value: '≈$41.20', marks: [{ kind: 'estimated', value: '5 estimated', label: ESTIMATED_WHY }] });
    expect(providerSplit(t, 'spend')?.map((p) => p.figure)).toEqual(['$41.20', '≈$2.50']);
    // Both marks at once: the estimate first, then what it leaves out.
    expect(spendValue(totals({ estimated: 5, unpriced: 2 }))).toBe('≈$41.20 + 2 unpriced');
  });

  it('OV-C226: each total keeps its one figure and carries its split by provider, Anthropic’s first: each provider’s figure as the tile writes it and its share', () => {
    const list = tiles(totals({ by_provider: TWO_PROVIDERS }));
    const split = (key: TileKey) => tile(key, list).byProvider?.map((p) => [p.provider, p.figure, p.share]);
    // One figure per total, the same as with one provider.
    expect(list.map((t) => t.value)).toEqual(tiles().map((t) => t.value));
    expect(split('prompt')).toEqual([
      ['anthropic', '12.20M', '98%'],
      ['kimi', '200k', '1.6%'],
    ]);
    expect(split('requests')).toEqual([
      ['anthropic', '400', '97%'],
      ['kimi', '12', '2.9%'],
    ]);
    expect(split('output')).toEqual([
      ['anthropic', '300k', '97%'],
      ['kimi', '10k', '3.2%'],
    ]);
    // Served from cache splits what it is made of, the cache reads; Cache writes its writes.
    expect(split('cache')).toEqual([
      ['anthropic', '10.30M', '99%'],
      ['kimi', '100k', '1.0%'],
    ]);
    expect(split('writes')).toEqual([
      ['anthropic', '1.38M', '99%'],
      ['kimi', '20k', '1.4%'],
    ]);
    // Spend: a cost of nothing beside requests the router could not price is "not priced", never "$0",
    // and has no share; with none it could not price, "—".
    expect(split('spend')).toEqual([
      ['anthropic', '$41.20', '100%'],
      ['kimi', 'not priced', null],
    ]);
    expect(providerSplit(totals({ by_provider: [TWO_PROVIDERS[0]!, { ...TWO_PROVIDERS[1]!, unpriced: 0 }] }), 'spend')?.[1]).toMatchObject({ figure: '—', share: null, value: 0 });
    // The bar's parts are sized by the figures themselves.
    expect(tile('prompt', list).byProvider?.map((p) => p.value)).toEqual([12_200_000, 200_000]);
    // Latency and errors have no figure per provider to split.
    expect([tile('latency', list).byProvider, tile('errors', list).byProvider]).toEqual([null, null]);
  });

  it('OV-C226: a router with one provider, or a scope narrowed to one, shows no split', () => {
    // The fixture's totals are Anthropic's alone.
    for (const t of tiles()) expect(t.byProvider, t.label).toBeNull();
    expect(providerSplit(totals({ by_provider: [providerTotals('kimi', { requests: 12, prompt_tokens: 200_000 })] }), 'prompt')).toBeNull();
    expect(providerSplit(totals({ by_provider: [] }), 'requests')).toBeNull();
  });

  it('OV-C182: a moment reads as its time of day when it is today, and as its date and time otherwise', () => {
    const today = NOW - 2 * HOUR;
    const earlier = NOW - 3 * 24 * HOUR;
    expect(tod(today, NOW)).toBe(new Date(today).toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit' }));
    expect(tod(earlier, NOW)).toContain(new Date(earlier).toLocaleDateString(undefined, { month: 'short', day: 'numeric' }));
    expect(tod(earlier, NOW)).toContain(new Date(earlier).toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit' }));
    expect(tod(null, NOW)).toBe('—');
  });
});
