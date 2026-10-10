// @vitest-environment node
// New for agent-router: the activity chart's contracts (docs/web-console-contracts.md, "Activity"), on the
// figures it is handed, with no chart library and no DOM.
import { describe, expect, it } from 'vitest';
import type { Timeline, TimelineRow } from '~/data/overview';
import { activityCaption, activityEmpty, bucketTip, buckets, chartData, chartOptions, errorMarks, FAMILIES, legend, MAX_BUCKETS, niceTop, perBucket, timeLabels } from './timeline';

const MIN = 60_000;
const HOUR = 60 * MIN;
const DAY = 24 * HOUR;
const T0 = Date.UTC(2026, 9, 5, 9, 0, 0);

const row = (t: number, model: string | null, over: Partial<TimelineRow> = {}): TimelineRow => ({
  t,
  model,
  requests: 1,
  prompt_tokens: 100,
  cost: 0.01,
  output_tokens: 10,
  cache_read_tokens: 50,
  subagent_tokens: 0,
  errors: 0,
  ...over,
});

const LOOK = { theme: 'white' as const, fills: { opus: '#7c3aed', sonnet: '#ea580c', haiku: '#0d9488', other: '#854d0e' } };

describe('the activity chart', () => {
  it('OV-C110: spans the selected range, quiet stretches included, every bucket drawn and an empty one as empty', () => {
    const tl: Timeline = { start: T0, end: T0 + 5 * HOUR, bucket_ms: HOUR, rows: [row(T0, 'claude-opus-5'), row(T0 + 3 * HOUR, 'claude-sonnet-4-5')] };
    const bk = buckets(tl, 0);
    expect(bk.map((b) => b.t)).toEqual([0, 1, 2, 3, 4, 5].map((i) => T0 + i * HOUR));
    expect(bk.map((b) => b.requests)).toEqual([1, 0, 0, 1, 0, 0]);
    // A range with no requests in it still shows the range, as empty buckets, and says which empty
    // it is rather than drawing nothing at all.
    const quiet = buckets({ start: T0, end: T0 + 2 * HOUR, bucket_ms: HOUR, rows: [] }, 0);
    expect(quiet).toHaveLength(3);
    expect(activityEmpty(quiet)).toBe('No requests in this range.');
    // With the range all, the router hands the data's own span, and the buckets span exactly that.
    expect(buckets({ start: T0 + HOUR, end: T0 + 2 * HOUR, bucket_ms: HOUR, rows: [row(T0 + HOUR, null), row(T0 + 2 * HOUR, null)] }, 0)).toHaveLength(2);
  });

  it("OV-C111: follows the viewer's clock, so a day bar starts at the viewer's midnight, and two models' rows in one bucket add up", () => {
    // A viewer four hours behind UTC: the router keys the day at 04:00 UTC, which is that viewer's
    // midnight, and the page buckets the same way.
    const tz = -4 * HOUR;
    const midnight = Date.UTC(2026, 9, 5, 4, 0, 0);
    const tl: Timeline = {
      start: midnight + 6 * HOUR,
      end: midnight + DAY + 3 * HOUR,
      bucket_ms: DAY,
      rows: [row(midnight, 'claude-opus-5', { prompt_tokens: 100, requests: 2 }), row(midnight, 'claude-sonnet-4-5', { prompt_tokens: 200, requests: 3 })],
    };
    const bk = buckets(tl, tz);
    expect(bk.map((b) => b.t)).toEqual([midnight, midnight + DAY]);
    expect(bk[0]).toMatchObject({ prompt: 300, requests: 5, fam: { opus: 100, sonnet: 200, haiku: 0, other: 0 }, famReq: { opus: 2, sonnet: 3 } });
  });

  it("OV-C112: the bucket width is the router's, and the page draws at most 500 buckets", () => {
    const bk = buckets({ start: T0, end: T0 + 1_000 * MIN, bucket_ms: MIN, rows: [] }, 0);
    expect(bk).toHaveLength(MAX_BUCKETS);
    expect(MAX_BUCKETS).toBe(500);
    expect(bk[1]!.t - bk[0]!.t).toBe(MIN);
  });

  it('OV-C113: tells the two empty charts apart', () => {
    expect(activityEmpty([])).toBe('No requests in this range.');
    const silent = buckets({ start: T0, end: T0 + HOUR, bucket_ms: HOUR, rows: [row(T0, 'claude-opus-5', { requests: 3, prompt_tokens: 0 })] }, 0);
    expect(activityEmpty(silent)).toBe('3 requests, none of which reported any tokens.');
    expect(activityEmpty(buckets({ start: T0, end: T0, bucket_ms: HOUR, rows: [row(T0, 'claude-opus-5')] }, 0))).toBeNull();
  });

  it('OV-C114: the y axis tops at a round number above the tallest bar and is labelled at half and top', () => {
    expect([0, 1, 3, 7, 12, 45, 1_234, 19_000_001, 20_000_000].map(niceTop)).toEqual([1, 1, 5, 10, 20, 50, 2_000, 20_000_000, 20_000_000]);
    const bk = buckets({ start: T0, end: T0 + HOUR, bucket_ms: HOUR, rows: [row(T0, 'claude-opus-5', { prompt_tokens: 1_234 })] }, 0);
    const left = chartOptions(bk, ['', 'x'], LOOK, 'chart').axes!.left!;
    expect(left.domain).toEqual([0, 2_000]);
    expect(left.ticks?.values).toEqual([0, 1_000, 2_000]);
  });

  it('OV-C114: time labels never collide, the last bucket is always labelled, and they read as dates, dates and times, or times', () => {
    const hourly = buckets({ start: T0, end: T0 + 71 * HOUR, bucket_ms: HOUR, rows: [] }, 0);
    const width = 700;
    const labels = timeLabels(hourly, HOUR, { start: T0, end: T0 + 71 * HOUR }, width, false);
    const at = labels.flatMap((l, i) => (l ? [i] : []));
    const sw = width / hourly.length;
    expect(at.at(-1)).toBe(hourly.length - 1);
    // Every label but the last sits a full stride after the one before, and the last at least the
    // room it needs; more than a day, so each says its date and its time.
    for (let k = 1; k < at.length - 1; k++) expect((at[k]! - at[k - 1]!) * sw).toBeGreaterThanOrEqual(124);
    expect((at.at(-1)! - at.at(-2)!) * sw).toBeGreaterThanOrEqual(96);
    expect(labels[at[0]!]).toMatch(/\d.*\d{1,2}:\d{2}/);
    // Within a day, times alone; in day buckets, dates alone.
    const short = timeLabels(buckets({ start: T0, end: T0 + 5 * HOUR, bucket_ms: HOUR, rows: [] }, 0), HOUR, { start: T0, end: T0 + 5 * HOUR }, width, false);
    expect(short.filter(Boolean).every((l) => /^\d{1,2}:\d{2}/.test(l))).toBe(true);
    const daily = timeLabels(buckets({ start: T0, end: T0 + 9 * DAY, bucket_ms: DAY, rows: [] }, 0), DAY, { start: T0, end: T0 + 9 * DAY }, width, false);
    expect(daily.filter(Boolean).every((l) => !/:/.test(l))).toBe(true);
    // Only the labelled buckets get a tick, so no blank tick crowds a label.
    const bottom = chartOptions(hourly, labels, LOOK, 'chart').axes!.bottom!;
    expect(bottom.ticks?.values).toEqual(at.map((i) => String(hourly[i]!.t)));
    // Carbon hands a labels scale's tick to the formatter as the domain's own string.
    const format = bottom.ticks!.formatter as unknown as (tick: string) => string;
    expect(at.map((i) => format(String(hourly[i]!.t)))).toEqual(at.map((i) => labels[i]));
  });

  it('OV-C109: stacks prompt tokens by family, opus at the bottom, in the family colours, with a legend that keeps the order, dims an absent family and never filters', () => {
    const bk = buckets({ start: T0, end: T0 + HOUR, bucket_ms: HOUR, rows: [row(T0, 'claude-haiku-4-5', { prompt_tokens: 5 }), row(T0, 'claude-opus-5', { prompt_tokens: 7 })] }, 0);
    expect(FAMILIES).toEqual(['opus', 'sonnet', 'haiku', 'other']);
    expect(chartData(bk).slice(0, 4)).toEqual([
      { group: 'opus', key: String(T0), value: 7 },
      { group: 'sonnet', key: String(T0), value: 0 },
      { group: 'haiku', key: String(T0), value: 5 },
      { group: 'other', key: String(T0), value: 0 },
    ]);
    expect(legend(bk)).toEqual([
      { family: 'opus', present: true },
      { family: 'sonnet', present: false },
      { family: 'haiku', present: true },
      { family: 'other', present: false },
    ]);
    const options = chartOptions(bk, ['x', 'y'], LOOK, 'chart');
    // The page draws its own legend; the chart's, which filters on a press, is off.
    expect(options.legend).toMatchObject({ enabled: false, clickable: false });
    expect(options.color?.scale).toEqual(LOOK.fills);
    expect([perBucket(HOUR), perBucket(15 * MIN), perBucket(DAY)]).toEqual(['prompt tokens per hour', 'prompt tokens per 15 minutes', 'prompt tokens per day']);
  });

  it('OV-C115: a bucket with errors carries a mark above its bar', () => {
    const bk = buckets({ start: T0, end: T0 + 2 * HOUR, bucket_ms: HOUR, rows: [row(T0, 'claude-opus-5', { errors: 2, prompt_tokens: 300 }), row(T0 + 2 * HOUR, 'claude-opus-5', { errors: 1 })] }, 0);
    expect(errorMarks(bk)).toEqual([
      { index: 0, total: 300, errors: 2 },
      { index: 2, total: 100, errors: 1 },
    ]);
  });

  it("OV-C116: a bucket's tip gives its span, each family's tokens and requests, output, cost, the share from cache and by subagents, and its errors", () => {
    const bk = buckets(
      { start: T0, end: T0, bucket_ms: HOUR, rows: [row(T0, 'claude-opus-5', { prompt_tokens: 1_000, requests: 4, cache_read_tokens: 800, subagent_tokens: 250, errors: 1, output_tokens: 2_000, cost: 0.5 })] },
      0,
    );
    const tip = bucketTip(bk[0]!, HOUR, T0);
    expect(tip.title).toMatch(/–/);
    expect(tip.rows.map((r) => `${r.value} ${r.label}`)).toEqual(['1.0k opus · 4 req', '2.0k output tokens', '$0.500 cost', '80% from cache', '25% by subagents', '1 error']);
  });

  it('OV-C117: the caption gives the range’s prompt tokens and requests', () => {
    const bk = buckets({ start: T0, end: T0 + HOUR, bucket_ms: HOUR, rows: [row(T0, 'claude-opus-5', { prompt_tokens: 1_200_000, requests: 3 }), row(T0 + HOUR, null, { prompt_tokens: 0, requests: 1 })] }, 0);
    expect(activityCaption(bk)).toBe('1.20M prompt tokens · 4 requests');
  });

  it('OV-C200, OV-C213: the chart is handed no animation at all, so a redraw queues no transition a hidden tab would hold', () => {
    expect(chartOptions([], [], LOOK, 'chart').animations).toBe(false);
  });
});
