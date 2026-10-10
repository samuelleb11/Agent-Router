// New for agent-router: Totals in scope with no DOM (assets/index.html, `renderHeadline`). Eight tiles,
// each with its meaning as visible text under its value (OV-C98).
//
// Where the page broke its own rule, the rule is kept:
// - Spend's full split is the 04efe6b one, never the older, shorter split bound after it (OV-C101);
// - every latency figure is on the page, the whole reply's p95 and the slowest reply included
//   (OV-C106);
// - Errors offers a visible, keyboard-reachable "List the N errors" (OV-C107), not a clickable div;
// - a scope with nothing priced reads "—", and one with unpriced requests in it says so, where the
//   router's sum made both look priced (OV-C178): its priced part, marked with how many it leaves
//   out, and "≈" when it rests on estimated tokens, each mark's reason in its tooltip (OV-C229,
//   OV-C232);
// - the cache figures say what happened, written to the cache or read from it, and each provider's
//   own rates are in the tooltip, where the page quoted Anthropic's alone (OV-C228).

import type { TipRow } from '~/components/DataTip';
import type { Latency, ProviderTotals, Totals } from '~/data/overview';
import { providerColour } from '~/lib/concepts';
import { DASH, ago, compact, costSum, count, ms, pct, plural, projectName, tod, type CostMark } from '~/model/format';
import type { Scope } from '~/model/scope';
import { providerName, type Provider } from '~/model/settings';

export type TileTone = 'ok' | 'warn' | 'bad';

export type TileKey = 'prompt' | 'spend' | 'requests' | 'output' | 'cache' | 'writes' | 'latency' | 'errors';

export interface Tile {
  readonly key: TileKey;
  readonly label: string;
  /// Null renders "—": a value the router did not give is not a zero.
  readonly value: string | null;
  /// Under the value: what it was counted over.
  readonly detail: string;
  /// What it means, as visible text (OV-C98).
  readonly meaning: string;
  readonly tone?: TileTone;
  /// Its total split by provider, shown under it; null with fewer than two providers in scope, and
  /// for a figure the router does not split (OV-C226).
  readonly byProvider: readonly ProviderPart[] | null;
  /// Spend's marks, each with its reason, for the value's tooltip (OV-C229, OV-C232).
  readonly marks?: readonly CostMark[];
}

/// One provider's part of a total (OV-C226).
export interface ProviderPart {
  readonly provider: Provider;
  /// Its size, for the bar of shares, which leaves out a zero.
  readonly value: number;
  /// As the tile writes its total: "12.20M", "412", "$41.20" — "$41.20 + 3 unpriced" for a cost that
  /// leaves requests out, and "not priced" for a cost of nothing beside requests the router could not
  /// price, never "$0" (OV-C178, OV-C221, OV-C229).
  readonly figure: string;
  /// Its share of the total, "98%", "0.4%" (OV-C179); null for a part with no figure to share.
  readonly share: string | null;
  /// A cost's marks, each with its reason, for the figure's tooltip (OV-C229, OV-C232).
  readonly marks?: readonly CostMark[];
}

const n = (v: number | null | undefined): number => (typeof v === 'number' && Number.isFinite(v) ? v : 0);

/// What each total is split by: the figure of each provider's row it adds up, and how it is written.
/// Latency and errors are not split: the router gives no per-provider figure for them.
const SPLITS: Readonly<Partial<Record<TileKey, { readonly of: (p: ProviderTotals) => number | null; readonly format: (v: number) => string }>>> = {
  prompt: { of: (p) => p.prompt_tokens, format: compact },
  // Spend writes each part as a cost sum, below.
  spend: { of: (p) => p.cost, format: (v) => costSum({ cost: v }).text },
  requests: { of: (p) => p.requests, format: count },
  output: { of: (p) => p.output_tokens, format: compact },
  // Served from cache splits what it is made of, the cache reads.
  cache: { of: (p) => p.cache_read_tokens, format: compact },
  writes: { of: (p) => p.cache_write_tokens, format: compact },
};

/// A total's split by provider, in the router's order, Anthropic's first: each provider's figure and
/// its share of the whole. Null when fewer than two providers served in scope — a router with one
/// provider, or a scope narrowed to one, shows no split — and for a tile with no split (OV-C226).
export function providerSplit(t: Totals, key: TileKey): ProviderPart[] | null {
  const rows = t.by_provider;
  const split = SPLITS[key];
  if (!split || rows.length < 2) return null;
  const whole = rows.reduce((a, r) => a + n(split.of(r)), 0);
  return rows.map((r) => {
    const value = n(split.of(r));
    if (key === 'spend') {
      // A cost sum (OV-C229, OV-C232): its priced part, marked; a cost of nothing reads "not priced"
      // beside requests the router could not price, and "—" otherwise, never "$0" (OV-C178, OV-C221).
      const sum = costSum(r);
      const priced = value > 0;
      return { provider: r.provider, value: priced ? value : 0, figure: priced || n(r.unpriced) > 0 ? sum.text : DASH, share: priced ? pct(value, whole) : null, marks: sum.marks };
    }
    return { provider: r.provider, value, figure: split.format(value), share: pct(value, whole) };
  });
}

/// Spend in words: the priced part, marked with the requests it leaves out and "≈" when it rests on
/// estimated tokens (OV-C229, OV-C232); "not priced" when nothing in scope was priced beside requests
/// the router could not price, and "—" when nothing was priced at all, never "$0" (OV-C178).
export function spendValue(t: Totals): string | null {
  return n(t.cost) > 0 || n(t.unpriced) > 0 ? costSum(t).text : null;
}

/// Each provider's cache rates, as multiples of its own uncached input, from the rate table's seeds
/// (src/pricing.rs): the console does not read the table, so an edited rate is not shown here.
/// Anthropic's writes are priced by tier; Kimi Code's cost what an uncached input does, and its
/// reads differ by model (OV-C228).
const CACHE_RATES: Readonly<Record<string, readonly { readonly value: string; readonly label: string }[]>> = {
  anthropic: [
    { value: '1.25× input', label: 'written to cache, 5 minutes' },
    { value: '2× input', label: 'written to cache, 1 hour' },
    { value: '0.1× input', label: 'read from cache' },
  ],
  kimi: [
    { value: '1× input', label: 'written to cache' },
    { value: '0.1–0.2× input', label: 'read from cache, by model' },
  ],
};

/// The providers in scope, Anthropic's first as the router lists them; Anthropic's alone for totals
/// that list none.
const providersIn = (t: Totals): Provider[] => (t.by_provider.length ? t.by_provider.map((p) => p.provider) : ['anthropic']);

/// The cache rates of each provider in scope, for a tooltip: each row its provider's, keyed in its
/// colour and named by it, "Kimi Code · read from cache, by model" (OV-C228). A provider whose rates
/// the console does not know says where they are kept.
export function cacheRates(t: Totals): TipRow[] {
  return providersIn(t).flatMap((p) =>
    (CACHE_RATES[p] ?? [{ value: 'its own rates', label: 'in the pricing table' }]).map((r) => ({ value: r.value, label: `${providerName(p)} · ${r.label}`, colour: providerColour(p) })),
  );
}

/// The eight tiles, the largest first (OV-C98), each total with its split by provider (OV-C226).
export function headline(t: Totals, latency: Latency | null | undefined, scope: Scope, now: number = Date.now()): Tile[] {
  const prompt = n(t.prompt_tokens);
  const cached = n(t.cache_read_tokens);
  const tiles: Omit<Tile, 'byProvider'>[] = [
    {
      key: 'prompt',
      label: 'Prompt tokens read',
      value: compact(t.prompt_tokens),
      // What it counts and over what span; with nothing recorded, how to start (OV-C99).
      // Each client by its name (OV-C230).
      detail: t.first_seen ? `${tod(t.first_seen, now)} → ${tod(t.last_seen, now)} · everything the model read, cached or not` : 'nothing recorded yet — point Claude Code or Kimi Code at this router',
      meaning: 'Every prompt token the model read in scope, from cache or paid in full.',
    },
    {
      key: 'spend',
      label: 'Spend',
      value: spendValue(t),
      // The two figures that dominate it (OV-C100).
      detail: n(t.cost) > 0 ? `${compact(t.cache_read_tokens)} cached reads · ${compact(t.output_tokens)} out` : 'nothing priced in scope',
      meaning: 'At API rates, for the requests in scope the router could price.',
      marks: costSum(t).marks,
    },
    {
      key: 'requests',
      label: 'Requests',
      value: count(t.requests),
      // Where they came from (OV-C102).
      detail: scope.session
        ? `1 session · ${projectName(scope.project)}`
        : `${plural(n(t.sessions), 'session', 'sessions')} · ${plural(n(t.projects), 'project', 'projects')}${n(t.subagent_requests) ? ` · ${count(t.subagent_requests)} by subagents` : ''}`,
      meaning: 'Requests that went through the router in scope.',
    },
    {
      key: 'output',
      label: 'Output tokens',
      value: compact(t.output_tokens),
      // How much of it was thinking (OV-C103).
      detail: `${compact(t.thinking_tokens)} of it thinking`,
      meaning: 'What the model wrote back, thinking included.',
    },
    {
      key: 'cache',
      label: 'Served from cache',
      // Cache reads ÷ every prompt token read: one definition of the hit rate (OV-C104).
      value: pct(t.cache_read_tokens, t.prompt_tokens),
      detail: prompt ? `${compact(cached)} of ${compact(prompt)} came from cache` : 'no prompt tokens yet',
      meaning: 'Cache reads as a share of every prompt token read.',
    },
    {
      key: 'writes',
      label: 'Cache writes',
      value: compact(t.cache_write_tokens),
      // The tiers are priced apart (OV-C105).
      detail: `5-minute ${compact(t.cache_write_5m_tokens)} · 1-hour ${compact(t.cache_write_1h_tokens)}`,
      // What happened, in words; each provider's rates are in its tooltip (OV-C228).
      meaning: 'Prompt tokens written to the cache, to be read back for less; each provider prices them at its own rates.',
    },
    {
      key: 'latency',
      label: 'Time to first token',
      value: latency && latency.ttfb_p50 != null ? ms(latency.ttfb_p50) : null,
      // Every latency figure on the page (OV-C106).
      detail:
        latency && latency.ttfb_p95 != null
          ? `p95 ${ms(latency.ttfb_p95)} · whole reply ${ms(latency.duration_p50)}, p95 ${ms(latency.duration_p95)} · slowest ${ms(latency.duration_max)}`
          : 'median · no requests yet',
      meaning: 'The median wait for the first token of a reply.',
    },
    {
      key: 'errors',
      label: 'Errors',
      value: count(t.errors),
      detail: `of ${plural(n(t.requests), 'request', 'requests')}`,
      meaning: 'A status of 400 or more, or an error recorded.',
      // Red when above zero (OV-C107).
      tone: n(t.errors) > 0 ? 'bad' : undefined,
    },
  ];
  return tiles.map((tile) => ({ ...tile, byProvider: providerSplit(t, tile.key) }));
}

/// Spend's full split: the total, uncached input, the two cache-write tiers, cache reads, output,
/// the share of the prompt from cache, requests, and where the rates are kept (OV-C101); the cache
/// rows say what happened, and each provider's rates follow them (OV-C228).
export function spendSplit(t: Totals): TipRow[] {
  return [
    { value: spendValue(t) ?? '—', label: 'total in scope' },
    { value: compact(t.input_tokens), label: 'uncached input' },
    { value: compact(t.cache_write_5m_tokens), label: 'written to cache, 5 minutes' },
    { value: compact(t.cache_write_1h_tokens), label: 'written to cache, 1 hour' },
    { value: compact(t.cache_read_tokens), label: 'read from cache' },
    { value: compact(t.output_tokens), label: 'output — dearest per token' },
    { value: pct(t.cache_read_tokens, t.prompt_tokens), label: 'of prompt served from cache' },
    { value: count(t.requests), label: 'requests' },
    ...cacheRates(t),
    { value: 'editable in the pricing table', label: 'rates' },
  ];
}

/// The heading's caption: how much and how recent, once anything is recorded (OV-C108).
export function totalsCaption(t: Totals, now: number = Date.now()): string {
  return t.first_seen ? `${plural(n(t.requests), 'request', 'requests')} · ${ago(t.last_seen, now)}` : '';
}
