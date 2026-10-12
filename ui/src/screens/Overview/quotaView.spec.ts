// @vitest-environment node
// New for agent-router: the quota panel's contracts (docs/web-console-contracts.md, "Quota" and
// "Forecast"), on the readings it is handed, with no DOM.
import { describe, expect, it } from 'vitest';
import type { LiveQuota, ProviderQuota, Reading } from '~/data/overview';
import type { Window } from '~/model/forecast';
import { burnOf, forecastFor, history, quotaView, secondOf, servingName, statusOf } from './quotaView';
import { buckets } from './timeline';

const MIN = 60_000;
const HOUR = 60 * MIN;
const NOW = Date.UTC(2026, 9, 5, 12, 0, 0);
const secs = (ms: number) => Math.floor(ms / 1000);

const reading = (value: LiveQuota, at = NOW): Reading<LiveQuota> => ({ value, at, kept: false, for: 'secondary' });
const LIVE: LiveQuota = {
  five_hour: { utilization: 41, resets_at: new Date(NOW + 2 * HOUR).toISOString() },
  seven_day: { utilization: 58, resets_at: new Date(NOW + 72 * HOUR).toISOString() },
  severity: 'normal',
  source: 'usage-api',
  age_ms: 3 * MIN,
};
/// Anthropic's block: its serving account and the last response's headers.
const HEADERS: ProviderQuota = {
  provider: 'anthropic',
  account: { id: 'secondary', label: 'secondary', email: 'second@example.test', plan: 'max' },
  windows: { five_hour: { utilization: 0.12, resets_at: secs(NOW + HOUR) }, seven_day: { utilization: 0.71, resets_at: secs(NOW + 48 * HOUR) } },
  status: 'allowed_warning',
  at: NOW - 7 * MIN,
  history: [],
};
/// Kimi Code's block: its usage API's reading, a 5-hour window and a month.
const KIMI: ProviderQuota = {
  provider: 'kimi',
  account: { id: 'kimi', label: 'Kimi', email: null, plan: null },
  windows: { five_hour: { utilization: 0.18, resets_at: secs(NOW + 3 * HOUR) }, month: { utilization: 0.27, resets_at: secs(NOW + 20 * 24 * HOUR) } },
  status: 'allowed',
  at: NOW - 2 * MIN,
  history: [],
};

describe('the quota panel', () => {
  it('OV-C55: prefers the live usage reading, and falls back to the last response’s headers', () => {
    const live = quotaView(reading(LIVE), HEADERS, null, NOW)!;
    expect(live.from).toBe('live');
    expect(live.windows.map((w) => w.used)).toEqual([41, 58]);
    // A reading the router says it does not have is no reading: the headers stand in.
    const unavailable = quotaView(reading({ source: 'unavailable', detail: 'upstream returned 403' }), HEADERS, null, NOW)!;
    expect(unavailable.from).toBe('headers');
    expect(unavailable.windows.map((w) => w.used)).toEqual([12, 71]);
    // A live reading of another account than the block's serving one is not its reading.
    expect(quotaView(reading({ ...LIVE, account_id: 'primary' }), HEADERS, null, NOW)!.windows.map((w) => w.used)).toEqual([12, 71]);
    expect(quotaView(null, null, null, NOW)).toBeNull();
    expect(quotaView(null, { ...HEADERS, windows: { five_hour: null, seven_day: null }, status: null, at: null }, null, NOW), 'a block with no reading').toBeNull();
  });

  it('OV-C56: says where its numbers came from and how old they are', () => {
    expect(quotaView(reading(LIVE), HEADERS, null, NOW)!.source).toBe('checked 3 min ago');
    expect(quotaView(reading({ ...LIVE, source: 'response-headers', seen_at: NOW - 20 * MIN }), null, null, NOW)!.source).toBe('from the last request 20 min ago');
    expect(quotaView(null, HEADERS, null, NOW)!.source).toBe('from the last request 7 min ago');
  });

  it('OV-C58: a usage reply without a 7-day window shows "—", never "0% used"', () => {
    const view = quotaView(reading({ ...LIVE, seven_day: null }), null, null, NOW)!;
    expect(view.windows[1]).toMatchObject({ used: null, resets: undefined });
  });

  it('OV-C59: the tag’s colour and its word come from the same field, and the error icon only on a red tag', () => {
    // The live severity is the field: a stale header warning no longer paints "normal" red.
    expect(quotaView(reading(LIVE), HEADERS, null, NOW)!.status).toEqual({ word: 'normal', tone: 'ok', icon: false });
    expect(quotaView(reading({ ...LIVE, severity: 'warning' }), null, null, NOW)!.status).toEqual({ word: 'warning', tone: 'warn', icon: false });
    expect(quotaView(reading({ ...LIVE, severity: 'critical' }), null, null, NOW)!.status).toEqual({ word: 'critical', tone: 'bad', icon: true });
    // With headers alone, the header's own status is the one field.
    expect(quotaView(null, HEADERS, null, NOW)!.status).toEqual({ word: 'allowed_warning', tone: 'warn', icon: false });
    expect(statusOf('rejected')).toEqual({ word: 'rejected', tone: 'bad', icon: true });
    expect(statusOf(null)).toBeNull();
  });

  it('OV-C62, OV-C63: each window keeps its own colour, and its exact reset is text', () => {
    const view = quotaView(reading(LIVE), null, null, NOW)!;
    expect(view.windows.map((w) => w.tone)).toEqual(['accent', 'slate']);
    expect(view.windows[0]!.resets).toBe('resets in 2h 00m');
    expect(view.windows[0]!.resetsAt).toMatch(/\d{1,2}:\d{2}/);
  });

  it('OV-C72: a header reading from before its window reset is never shown as current', () => {
    const view = quotaView(null, { ...HEADERS, windows: { ...HEADERS.windows, five_hour: { utilization: 0.12, resets_at: secs(NOW - 2 * HOUR) } } }, null, NOW)!;
    expect(view.windows[0]).toMatchObject({ used: null, resets: undefined, resetsAt: undefined, expired: 'window rolled over 2 h ago · nothing seen since' });
    expect(view.windows[1]!.used).toBe(71);
  });

  it('OV-C54, SH-C16: a block describes its provider’s serving account, named by its email with its plan', () => {
    const serving = HEADERS.account;
    expect(servingName(serving)).toBe('second@example.test · max');
    expect(servingName({ ...serving, email: null, plan: null })).toBe('secondary');
    expect(servingName(KIMI.account)).toBe('Kimi');
    expect(forecastFor([{ id: 'primary' }, { id: 'secondary' }], serving)?.id).toBe('secondary');
    expect(forecastFor([{ id: 'primary' }], serving)).toBeNull();
  });

  it('OV-C224: Kimi Code’s block is its usage reading, checked, with its second window named by its own key, month', () => {
    // `/api/quota` is Anthropic's: it never stands in Kimi Code's block.
    const view = quotaView(null, KIMI, null, NOW)!;
    expect(view.from).toBe('live');
    expect(view.source).toBe('checked 2 min ago');
    expect(view.windows.map((w) => [w.key, w.label, w.used, w.tone])).toEqual([
      ['five_hour', '5-hour window', 18, 'accent'],
      ['month', 'Month window', 27, 'slate'],
    ]);
    expect(view.windows[1]!.resets).toBe('resets in 20d 0h');
    expect(view.status).toEqual({ word: 'allowed', tone: 'ok', icon: false });
    // A legacy plan's week, a week; Anthropic's second window, the 7-day.
    expect(secondOf(KIMI)).toBe('month');
    expect(secondOf({ windows: { five_hour: null, seven_day: null } })).toBe('seven_day');
    expect(secondOf(HEADERS)).toBe('seven_day');
    // A month the provider holds no reading of yet is still a month, "—", never a 7-day window.
    const unread = quotaView(null, { ...KIMI, windows: { five_hour: KIMI.windows.five_hour, month: null } }, null, NOW)!;
    expect(unread.windows[1]).toMatchObject({ key: 'month', label: 'Month window', used: null });
    // A spent window reads full, and the provider's word for it is red.
    const spent = quotaView(null, { ...KIMI, windows: { ...KIMI.windows, five_hour: { utilization: 1, resets_at: secs(NOW + HOUR) } }, status: 'rejected' }, null, NOW)!;
    expect(spent.windows[0]!.used).toBe(100);
    expect(spent.status).toEqual({ word: 'rejected', tone: 'bad', icon: true });
    // Its forecast lines are the month's, by name.
    const month: Window = { used: 0.27, reset_at: secs(NOW + 20 * 24 * HOUR), window_start: secs(NOW - 10 * 24 * HOUR), basis: 'none', expired: false };
    expect(quotaView(null, KIMI, { five_hour: null, seven_day: null, month }, NOW)!.windows[1]!.line?.text).toBe('not enough burn yet to project');
  });

  it('OV-C66, OV-C67: each window’s line gives work left first, in hours of work, and what the burn was fitted on, per hour of work', () => {
    const w: Window = {
      used: 0.4, reset_at: secs(NOW + 3 * HOUR), window_start: secs(NOW - 2 * HOUR), burn_per_active_hour: 0.1, active_hours_left: 6,
      duty_fraction: 0.5, exhausts_at: null, projected_end_util: 0.7, basis: 'this-window', measured_active_hours: 1.5, expired: false, verdict: 'resets-first',
    };
    const line = quotaView(reading(LIVE), null, { five_hour: w, seven_day: null, month: null }, NOW)!.windows[0]!.line!;
    expect(line.text).toBe('6.0 h of work left · resets first, ending ≈70%');
    expect(line.basis).toBe('10.0% per hour of work, fitted on 1.5 h · 50% of the clock spent working');
    expect(line.text).not.toMatch(/per hour on the clock/);
  });

  it('OV-C60, OV-C61: the history is two lines on the activity chart’s buckets, the worst reading capped at 100 %, a bucket with no reading left off', () => {
    const t0 = NOW - 3 * HOUR;
    const bk = buckets({ start: t0, end: t0 + 3 * HOUR, bucket_ms: HOUR, rows: [] }, 0);
    const h = history(
      [
        { t: t0, five_hour: 0.2, seven_day: 0.5 },
        { t: t0 + 2 * HOUR, five_hour: 1.3, seven_day: null },
        { t: t0 + 99 * HOUR, five_hour: 0.9, seven_day: 0.9 },
      ],
      'seven_day',
      bk,
      NOW,
    )!;
    expect(h.buckets).toHaveLength(4);
    expect(h.series.map((s) => s.values)).toEqual([
      [20, null, 100, null],
      [50, null, null, null],
    ]);
    expect(h.series.map((s) => s.colour)).toEqual(['--cds-interactive', '--ar-tok']);
    expect(h.series.map((s) => s.label)).toEqual(['5-hour used', '7-day used']);
    expect(history([], 'seven_day', bk, NOW)).toBeNull();
  });

  it('OV-C224, OV-C60: Kimi Code’s history draws its month by its own name, in the second window’s colour', () => {
    const t0 = NOW - 3 * HOUR;
    const bk = buckets({ start: t0, end: t0 + 3 * HOUR, bucket_ms: HOUR, rows: [] }, 0);
    const h = history(
      [
        { t: t0 + HOUR, five_hour: 0.1, month: 0.25 },
        { t: t0 + 2 * HOUR, five_hour: 0.18, month: 0.27 },
      ],
      'month',
      bk,
      NOW,
    )!;
    expect(h.series.map((s) => [s.key, s.label, s.colour])).toEqual([
      ['5h', '5-hour used', '--cds-interactive'],
      ['month', 'month used', '--ar-tok'],
    ]);
    expect(h.series[1]!.values).toEqual([null, 25, 27, null]);
    // A row read for the month says nothing of a week.
    expect(history([{ t: t0 + HOUR, month: 0.25 }], 'seven_day', bk, NOW)).toBeNull();
  });

  it('OV-C83, OV-C84: a window is drawn with its cone only when the band has width, and not at all when it cannot be', () => {
    const w: Window = { used: 0.5, reset_at: secs(NOW + HOUR), window_start: secs(NOW - HOUR), exhausts_at: null, projected_end_util: 0.8, duty: { low: 0.3, mid: 0.4, high: 0.5 }, projected_end_util_low: 0.7, projected_end_util_high: 0.9 };
    expect(burnOf(w)).toMatchObject({ start: (NOW - HOUR) / 1000 * 1000, used: 0.5, band: { quiet: { end: 0.7 }, busy: { end: 0.9 } } });
    expect(burnOf({ ...w, duty: { low: 0.4, mid: 0.4, high: 0.4 } })!.band).toBeNull();
    expect([burnOf({ ...w, expired: true }), burnOf({ ...w, window_start: null }), burnOf(null)]).toEqual([null, null, null]);
  });

  it('OV-C223: a window carries its short trend to the burn-down when the router weighed the recent load, and none otherwise', () => {
    const w: Window = { used: 0.5, reset_at: secs(NOW + HOUR), window_start: secs(NOW - HOUR), exhausts_at: null, projected_end_util: 0.8 };
    const recent = { load: 2.6, burn_per_active_hour: 0.3, measured_active_hours: 0.5, measured_from: NOW - HOUR / 2, measured_until: NOW, exhausts_at: NOW + HOUR / 2, projected_end_util: 1 };
    expect(burnOf({ ...w, recent })!.recent).toEqual({ crossing: NOW + HOUR / 2, end: 1, load: 2.6, hours: 0.5, until: NOW });
    expect(burnOf(w)!.recent).toBeNull();
    expect(burnOf({ ...w, recent: { ...recent, load: 0 } })!.recent).toBeNull();
  });
});
