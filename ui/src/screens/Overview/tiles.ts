// New for agent-router: Totals in scope with no DOM (assets/index.html, `renderHeadline`). Eight tiles,
// each with its meaning as visible text under its value (OV-C98).
//
// Where the page broke its own rule, the rule is kept:
// - Spend's full split is the 04efe6b one, never the older, shorter split bound after it (OV-C101);
// - every latency figure is on the page, the whole reply's p95 and the slowest reply included
//   (OV-C106);
// - Errors offers a visible, keyboard-reachable "List the N errors" (OV-C107), not a clickable div;
// - a scope with nothing priced reads "—", and one with unpriced requests in it says so, where the
//   router's sum made both look priced (OV-C178).

import type { Latency, ModelRow, Totals } from '~/data/overview';
import { ago, compact, count, ms, pct, plural, projectName, tod, usd } from '~/model/format';
import type { Scope } from '~/model/scope';
import { unpricedModel } from './panels';

export type TileTone = 'ok' | 'warn' | 'bad';

export interface Tile {
  readonly key: 'prompt' | 'spend' | 'requests' | 'output' | 'cache' | 'writes' | 'latency' | 'errors';
  readonly label: string;
  /// Null renders "—": a value the router did not give is not a zero.
  readonly value: string | null;
  /// Under the value: what it was counted over.
  readonly detail: string;
  /// What it means, as visible text (OV-C98).
  readonly meaning: string;
  readonly tone?: TileTone;
}

const n = (v: number | null | undefined): number => (typeof v === 'number' && Number.isFinite(v) ? v : 0);

/// The requests in scope the router could not price: a model with tokens and no cost contributes
/// nothing to the total rather than counting as free (src/pricing.rs), so its requests are named
/// rather than left to read as priced (OV-C178).
export function unpriced(models: readonly ModelRow[]): { readonly requests: number; readonly models: readonly string[] } {
  const none = models.filter(unpricedModel);
  return { requests: none.reduce((a, m) => a + n(m.requests), 0), models: none.map((m) => m.model) };
}

/// Spend in words: the total, or "—" when nothing in scope was priced, never "$0" (OV-C178).
export function spendValue(t: Totals): string | null {
  return n(t.cost) > 0 ? usd(t.cost) : null;
}

/// The eight tiles, the largest first (OV-C98).
export function headline(t: Totals, latency: Latency | null | undefined, models: readonly ModelRow[], scope: Scope, now: number = Date.now()): Tile[] {
  const prompt = n(t.prompt_tokens);
  const cached = n(t.cache_read_tokens);
  const left = unpriced(models);
  const leaves = left.requests ? ` · leaves out ${plural(left.requests, 'request', 'requests')} the router could not price` : '';
  return [
    {
      key: 'prompt',
      label: 'Prompt tokens read',
      value: compact(t.prompt_tokens),
      // What it counts and over what span; with nothing recorded, how to start (OV-C99).
      detail: t.first_seen ? `${tod(t.first_seen, now)} → ${tod(t.last_seen, now)} · everything the model read, cached or not` : 'nothing recorded yet — point Claude Code at this router',
      meaning: 'Every prompt token the model read in scope, from cache or paid in full.',
    },
    {
      key: 'spend',
      label: 'Spend',
      value: spendValue(t),
      // The two figures that dominate it (OV-C100).
      detail: n(t.cost) > 0 ? `${compact(t.cache_read_tokens)} cached reads · ${compact(t.output_tokens)} out${leaves}` : 'nothing priced in scope',
      meaning: 'At API rates, for the requests in scope the router could price.',
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
      meaning: 'Prompt tokens written to cache, at 1.25× input for 5 minutes and 2× for an hour.',
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
}

/// Spend's full split: the total, uncached input, the two cache-write tiers, cache reads, output,
/// the share of the prompt from cache, requests, and where the rates are kept (OV-C101).
export function spendSplit(t: Totals): { readonly value: string; readonly label: string }[] {
  return [
    { value: spendValue(t) ?? '—', label: 'total in scope' },
    { value: compact(t.input_tokens), label: 'uncached input' },
    { value: compact(t.cache_write_5m_tokens), label: 'cache writes, 5 min — 1.25× input' },
    { value: compact(t.cache_write_1h_tokens), label: 'cache writes, 1 hour — 2× input' },
    { value: compact(t.cache_read_tokens), label: 'cache reads — 0.1× input' },
    { value: compact(t.output_tokens), label: 'output — dearest per token' },
    { value: pct(t.cache_read_tokens, t.prompt_tokens), label: 'of prompt served from cache' },
    { value: count(t.requests), label: 'requests' },
    { value: 'editable in the pricing table', label: 'rates' },
  ];
}

/// The heading's caption: how much and how recent, once anything is recorded (OV-C108).
export function totalsCaption(t: Totals, now: number = Date.now()): string {
  return t.first_seen ? `${plural(n(t.requests), 'request', 'requests')} · ${ago(t.last_seen, now)}` : '';
}
