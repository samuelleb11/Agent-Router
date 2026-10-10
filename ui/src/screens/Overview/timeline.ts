// New for agent-router: the activity chart's figures with no DOM and no chart library (assets/index.html,
// `buckets`, `nice`, `renderActivity`). The chart itself is Carbon Charts' StackedBarChart, loaded
// lazily (./ActivityChart); what it is handed is decided here, so the specs call it directly.
//
// Where the page broke its own rule, the rule is kept:
// - a bucket's figures followed the pointer only; here they are a tip a tap pins and the keyboard
//   walks, built by `bucketTip` (OV-C116);
// - bucket sizes are spelt the phones' way, "15 minutes" (OV-C185, decision K).

import type { ChartTheme, ScaleTypes, StackedBarChartOptions, TickRotations, TruncationTypes } from '@carbon/charts';
import type { Timeline, TimelineRow } from '~/data/overview';
import type { TipRow } from '~/components/DataTip';
import { FAMILY_COLOUR } from '~/lib/concepts';
import { bucketName, compact, count, family, hm, monthDay, pct, plural, tod, usd, type Family } from '~/model/format';

/// The families, in the order they stack from the bottom and the legend reads (OV-C109).
export const FAMILIES: readonly Family[] = ['opus', 'sonnet', 'haiku', 'other'];

/// The page draws at most this many buckets (OV-C112).
export const MAX_BUCKETS = 500;

const DAY = 86_400_000;

export interface Bucket {
  /// The bucket's start, epoch milliseconds.
  readonly t: number;
  readonly requests: number;
  readonly prompt: number;
  readonly output: number;
  readonly cache: number;
  readonly sub: number;
  readonly errors: number;
  readonly cost: number;
  readonly fam: Readonly<Record<Family, number>>;
  readonly famReq: Readonly<Record<Family, number>>;
}

const zeroes = (): Record<Family, number> => ({ opus: 0, sonnet: 0, haiku: 0, other: 0 });
const n = (v: number | null | undefined): number => (typeof v === 'number' && Number.isFinite(v) ? v : 0);

/// Every bucket from the start of the span to its end, an empty one as empty, so a quiet hour still
/// shows as an hour of quiet (OV-C110). The buckets are the router's width, on the viewer's clock —
/// `tzMs` is the UTC offset the summary was asked with — and two models' rows in one bucket add up
/// (OV-C111). At most 500 (OV-C112).
export function buckets(tl: Timeline | null | undefined, tzMs: number): Bucket[] {
  if (!tl || !tl.bucket_ms || tl.start == null || tl.end == null) return [];
  const b = tl.bucket_ms;
  const t0 = Math.floor((tl.start + tzMs) / b) * b - tzMs;
  const byT = new Map<number, { requests: number; prompt: number; output: number; cache: number; sub: number; errors: number; cost: number; fam: Record<Family, number>; famReq: Record<Family, number> }>();
  for (const r of tl.rows as readonly TimelineRow[]) {
    const t = Number(r.t);
    let o = byT.get(t);
    if (!o) {
      o = { requests: 0, prompt: 0, output: 0, cache: 0, sub: 0, errors: 0, cost: 0, fam: zeroes(), famReq: zeroes() };
      byT.set(t, o);
    }
    o.requests += n(r.requests);
    o.prompt += n(r.prompt_tokens);
    o.output += n(r.output_tokens);
    o.cache += n(r.cache_read_tokens);
    o.sub += n(r.subagent_tokens);
    o.errors += n(r.errors);
    o.cost += n(r.cost);
    const f = family(r.model);
    o.fam[f] += n(r.prompt_tokens);
    o.famReq[f] += n(r.requests);
  }
  const out: Bucket[] = [];
  for (let t = t0, i = 0; t <= tl.end && i < MAX_BUCKETS; t += b, i++) {
    out.push({ t, ...(byT.get(t) ?? { requests: 0, prompt: 0, output: 0, cache: 0, sub: 0, errors: 0, cost: 0, fam: zeroes(), famReq: zeroes() }) });
  }
  return out;
}

/// The top of the y axis: the round number above the tallest bar, 1, 2 or 5 times a power of ten
/// (OV-C114). One for an empty chart, never zero, so nothing divides by it.
export function niceTop(max: number): number {
  if (!(max > 0)) return 1;
  const p = 10 ** Math.floor(Math.log10(max));
  for (const m of [1, 2, 5, 10]) if (m * p >= max) return m * p;
  return 10 * p;
}

/// Which buckets carry a time label, and what it says: time labels never collide, the last bucket is
/// always labelled, and they read as dates for day buckets, as a date and a time across more than a
/// day, and as a time otherwise (OV-C114). `plotWidth` is the plot's width in CSS pixels: the labels
/// are spaced for the width the chart is drawn at, and spaced again when it changes (OV-C172).
export function timeLabels(bk: readonly Bucket[], bucketMs: number, span: { readonly start: number | null; readonly end: number | null }, plotWidth: number, narrow: boolean): string[] {
  const nB = bk.length;
  if (!nB) return [];
  const spanDays = span.start != null && span.end != null && span.end - span.start > DAY;
  const sw = Math.max(1, plotWidth) / nB;
  const every = Math.max(1, Math.ceil((spanDays ? 124 : narrow ? 70 : 90) / sw));
  const room = spanDays ? 96 : 64;
  const said = (t: number) => (bucketMs >= DAY ? monthDay(t) : spanDays ? `${monthDay(t)} ${hm(t)}` : hm(t));
  return bk.map((b, i) => (i === nB - 1 || (i % every === 0 && (nB - 1 - i) * sw >= room) ? said(b.t) : ''));
}

/// Which empty the chart is, or null when it has bars to draw: no requests at all, and requests that
/// reported no tokens, are told apart (OV-C113).
export function activityEmpty(bk: readonly Bucket[]): string | null {
  const total = bk.reduce((a, b) => a + b.prompt, 0);
  const requests = bk.reduce((a, b) => a + b.requests, 0);
  if (!bk.length || (!total && !requests)) return 'No requests in this range.';
  if (!total) return `${plural(requests, 'request', 'requests')}, none of which reported any tokens.`;
  return null;
}

/// The caption: the range's prompt tokens and requests (OV-C117).
export function activityCaption(bk: readonly Bucket[]): string {
  const total = bk.reduce((a, b) => a + b.prompt, 0);
  const requests = bk.reduce((a, b) => a + b.requests, 0);
  return `${compact(total)} prompt tokens · ${plural(requests, 'request', 'requests')}`;
}

export interface LegendEntry {
  readonly family: Family;
  /// Absent from the range: dimmed, never dropped (OV-C109).
  readonly present: boolean;
}

/// The legend: every family in the stacking order, a family absent from the range dimmed rather than
/// dropped (OV-C109).
export function legend(bk: readonly Bucket[]): LegendEntry[] {
  return FAMILIES.map((f) => ({ family: f, present: bk.some((b) => b.fam[f] > 0) }));
}

/// What the legend says the bars are: "prompt tokens per hour" (OV-C109).
export const perBucket = (bucketMs: number): string => (bucketMs ? `prompt tokens per ${bucketName(bucketMs)}` : 'prompt tokens');

/// One bar segment as Carbon Charts takes it.
export interface Datum {
  readonly group: Family;
  /// The bucket's start as text: the labels scale's domain value.
  readonly key: string;
  readonly value: number;
}

/// The bars: each bucket's prompt tokens by family, every family in every bucket in the stacking
/// order, so opus is at the bottom of every stack (OV-C109).
export function chartData(bk: readonly Bucket[]): Datum[] {
  return bk.flatMap((b) => FAMILIES.map((f) => ({ group: f, key: String(b.t), value: b.fam[f] })));
}

/// The buckets that carry an error mark above their bar (OV-C115), with the height of the bar the
/// mark sits over.
export function errorMarks(bk: readonly Bucket[]): { readonly index: number; readonly total: number; readonly errors: number }[] {
  return bk.flatMap((b, index) => (b.errors > 0 ? [{ index, total: b.prompt, errors: b.errors }] : []));
}

export interface ChartLook {
  /// Carbon Charts' theme, from the class on <html>.
  readonly theme: 'white' | 'g100';
  /// The family fills read off the tokens in force; null when they could not be read (a test).
  readonly fills: Readonly<Record<Family, string>> | null;
}

/// What Carbon Charts is handed, besides the data. The legend is the page's own, so the chart's is
/// off and can never filter it (OV-C109); so is its tooltip, which follows the pointer only — the
/// page's tip is reachable by a tap and the keyboard (OV-C116). The y axis tops at `top` and is
/// labelled at half and top (OV-C114); the time labels are `labels`, blank where one would collide.
/// No animation at all: a redraw starts d3 transitions a hidden tab never runs (OV-C213), and
/// reduced motion asks for none (OV-C200).
export function chartOptions(bk: readonly Bucket[], labels: readonly string[], look: ChartLook, name: string): StackedBarChartOptions {
  const top = niceTop(Math.max(0, ...bk.map((b) => b.prompt)));
  const labelOf = new Map(bk.map((b, i) => [String(b.t), labels[i] ?? '']));
  return {
    accessibility: { svgAriaLabel: name },
    animations: false,
    axes: {
      bottom: {
        mapsTo: 'key',
        scaleType: 'labels' as ScaleTypes,
        domain: bk.map((b) => String(b.t)),
        // A tick only where a label goes, never rotated and never cut: `labels` already leaves out
        // every label that would collide. Without `values`, Carbon writes the raw keys over the
        // formatter's words for a labels scale.
        ticks: {
          values: bk.filter((_, i) => (labels[i] ?? '') !== '').map((b) => String(b.t)),
          formatter: (tick: number | Date) => labelOf.get(String(tick)) ?? '',
          rotation: 'never' as TickRotations,
        },
        truncation: { type: 'none' as TruncationTypes },
      },
      left: {
        mapsTo: 'value',
        stacked: true,
        domain: [0, top],
        includeZero: true,
        ticks: { values: [0, top / 2, top], formatter: (tick: number | Date) => compact(Number(tick)) },
      },
    },
    bars: { maxWidth: 24 },
    color: look.fills ? { scale: { ...look.fills } } : undefined,
    data: { groupMapsTo: 'group' },
    grid: { x: { enabled: false }, y: { enabled: true } },
    height: '220px',
    legend: { enabled: false, clickable: false, order: [...FAMILIES] },
    theme: look.theme as ChartTheme,
    toolbar: { enabled: false },
    tooltip: { enabled: false },
  };
}

/// The family fills as tokens, for a reader of the computed style.
export const FAMILY_TOKENS: Readonly<Record<Family, string>> = FAMILY_COLOUR;

/// One bucket's figures, as a tip: its span, each family's tokens and requests, output, cost, the
/// share from cache, the share by subagents and its errors (OV-C116).
export function bucketTip(b: Bucket, bucketMs: number, now: number = Date.now()): { readonly title: string; readonly rows: TipRow[] } {
  const rows: TipRow[] = FAMILIES.filter((f) => b.fam[f] > 0 || b.famReq[f] > 0).map((f) => ({
    value: compact(b.fam[f]),
    label: `${f} · ${count(b.famReq[f])} req`,
    colour: FAMILY_COLOUR[f],
  }));
  rows.push({ value: compact(b.output), label: 'output tokens' }, { value: usd(b.cost), label: 'cost' }, { value: pct(b.cache, b.prompt), label: 'from cache' });
  if (b.sub > 0) rows.push({ value: pct(b.sub, b.prompt), label: 'by subagents', colour: '--ar-subagent' });
  if (b.errors > 0) rows.push({ value: count(b.errors), label: b.errors === 1 ? 'error' : 'errors', colour: '--ar-bad-fill' });
  return { title: `${tod(b.t, now)} – ${hm(b.t + bucketMs)}`, rows };
}
