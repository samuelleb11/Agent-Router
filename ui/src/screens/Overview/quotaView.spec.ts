// @vitest-environment node
// New for agent-router: the quota panel's contracts (docs/web-console-contracts.md, "Quota" and
// "Forecast"), on the readings it is handed, with no DOM.
import { describe, expect, it } from 'vitest';
import type { HeaderQuota, LiveQuota, Reading } from '~/data/overview';
import type { Window } from '~/model/forecast';
import { burnOf, forecastFor, history, quotaView, servingName, statusOf } from './quotaView';
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
const HEADERS: HeaderQuota = { rl_status: 'allowed_warning', rl_5h_utilization: 0.12, rl_7d_utilization: 0.71, rl_5h_reset: secs(NOW + HOUR), rl_7d_reset: secs(NOW + 48 * HOUR), started_at: NOW - 7 * MIN };

describe('the quota panel', () => {
  it('OV-C55: prefers the live usage reading, and falls back to the last response’s headers', () => {
    const live = quotaView(reading(LIVE), HEADERS, null, NOW)!;
    expect(live.from).toBe('live');
    expect(live.windows.map((w) => w.used)).toEqual([41, 58]);
    // A reading the router says it does not have is no reading: the headers stand in.
    const unavailable = quotaView(reading({ source: 'unavailable', detail: 'upstream returned 403' }), HEADERS, null, NOW)!;
    expect(unavailable.from).toBe('headers');
    expect(unavailable.windows.map((w) => w.used)).toEqual([12, 71]);
    expect(quotaView(null, null, null, NOW)).toBeNull();
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
    const view = quotaView(null, { ...HEADERS, rl_5h_reset: secs(NOW - 2 * HOUR) }, null, NOW)!;
    expect(view.windows[0]).toMatchObject({ used: null, resets: undefined, resetsAt: undefined, expired: 'window rolled over 2 h ago · nothing seen since' });
    expect(view.windows[1]!.used).toBe(71);
  });

  it('OV-C54, SH-C16: describes the serving account, named by its email with its plan, or "no account"', () => {
    const serving = { id: 'secondary', label: 'secondary', email: 'second@example.test', subscription_type: 'max' };
    expect(servingName(serving)).toBe('second@example.test · max');
    expect(servingName({ ...serving, email: null, subscription_type: null })).toBe('secondary');
    expect(servingName(null)).toBe('no account');
    expect(forecastFor([{ id: 'primary' }, { id: 'secondary' }], serving)?.id).toBe('secondary');
    expect(forecastFor([{ id: 'primary' }], serving)).toBeNull();
  });

  it('OV-C66, OV-C67: each window’s line gives work left first, in hours of work, and what the burn was fitted on, per hour of work', () => {
    const w: Window = {
      used: 0.4, reset_at: secs(NOW + 3 * HOUR), window_start: secs(NOW - 2 * HOUR), burn_per_active_hour: 0.1, active_hours_left: 6,
      duty_fraction: 0.5, exhausts_at: null, projected_end_util: 0.7, basis: 'this-window', measured_active_hours: 1.5, expired: false, verdict: 'resets-first',
    };
    const line = quotaView(reading(LIVE), null, { five_hour: w, seven_day: null }, NOW)!.windows[0]!.line!;
    expect(line.text).toBe('6.0 h of work left · resets first, ending ≈70%');
    expect(line.basis).toBe('10.0% per hour of work, fitted on 1.5 h · 50% of the clock spent working');
    expect(line.text).not.toMatch(/per hour on the clock/);
  });

  it('OV-C60, OV-C61: the history is two lines on the activity chart’s buckets, the worst reading capped at 100 %, a bucket with no reading left off', () => {
    const t0 = NOW - 3 * HOUR;
    const bk = buckets({ start: t0, end: t0 + 3 * HOUR, bucket_ms: HOUR, rows: [] }, 0);
    const h = history(
      [
        { t: t0, rl_5h: 0.2, rl_7d: 0.5 },
        { t: t0 + 2 * HOUR, rl_5h: 1.3, rl_7d: null },
        { t: t0 + 99 * HOUR, rl_5h: 0.9, rl_7d: 0.9 },
      ],
      bk,
      NOW,
    )!;
    expect(h.buckets).toHaveLength(4);
    expect(h.series.map((s) => s.values)).toEqual([
      [20, null, 100, null],
      [50, null, null, null],
    ]);
    expect(h.series.map((s) => s.colour)).toEqual(['--cds-interactive', '--ar-tok']);
    expect(history([], bk, NOW)).toBeNull();
  });

  it('OV-C83, OV-C84: a window is drawn with its cone only when the band has width, and not at all when it cannot be', () => {
    const w: Window = { used: 0.5, reset_at: secs(NOW + HOUR), window_start: secs(NOW - HOUR), exhausts_at: null, projected_end_util: 0.8, duty: { low: 0.3, mid: 0.4, high: 0.5 }, projected_end_util_low: 0.7, projected_end_util_high: 0.9 };
    expect(burnOf(w)).toMatchObject({ start: (NOW - HOUR) / 1000 * 1000, used: 0.5, band: { quiet: { end: 0.7 }, busy: { end: 0.9 } } });
    expect(burnOf({ ...w, duty: { low: 0.4, mid: 0.4, high: 0.4 } })!.band).toBeNull();
    expect([burnOf({ ...w, expired: true }), burnOf({ ...w, window_start: null }), burnOf(null)]).toEqual([null, null, null]);
  });
});
