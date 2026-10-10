// New for agent-router: the quota panel's figures with no DOM (assets/index.html, `renderQuota`,
// `renderRunway`, `renderHeader`'s account chip). The meters, the status tag, the history and the
// burn-downs describe the account serving now, whatever the page is scoped to (OV-C54).
//
// Where the page broke its own rule, the rule is kept:
// - a window with no reading, or a usage reply without a 7-day window, shows "—", never "0% used"
//   (OV-C58);
// - the status tag's colour and its word come from one field, and the error icon only from a red
//   one (OV-C59), where the colour also read the last header's `rl_status`;
// - a header reading from before its window's reset is never shown as current (OV-C72);
// - the 7-day window wears slate on its meter as on its line and its burn-down (OV-C62).

import type { Burn } from '~/components/BurnDown';
import type { Bucket } from './timeline';
import type { HeaderQuota, LiveQuota, QuotaHistoryRow, Reading, ServingAccount } from '~/data/overview';
import { ago, relative, resetAt, tod, when } from '~/model/format';
import { fcLine, type ForecastAccount, type Line, type Window, type WindowKey } from '~/model/forecast';
import type { Series } from '~/components/Sparkline';

export type Tone = 'accent' | 'slate';

export interface QuotaWindow {
  readonly key: WindowKey;
  readonly label: string;
  /// Percent used, 0–100, or null with no reading (OV-C58).
  readonly used: number | null;
  /// "resets in 2h 10m".
  readonly resets: string | undefined;
  /// The exact reset, as text (OV-C63).
  readonly resetsAt: string | undefined;
  /// The window rolled over: its sentence, and no figure (OV-C72).
  readonly expired: string | undefined;
  /// The 5-hour window is the accent, the 7-day slate (OV-C62).
  readonly tone: Tone;
  /// Its forecast line, or none when the forecast has nothing for it (OV-C75).
  readonly line: Line | null;
}

export type StatusTone = 'ok' | 'warn' | 'bad' | 'neutral';

export interface Status {
  /// The one field's word: "normal", "warning", "critical", or the header's own.
  readonly word: string;
  readonly tone: StatusTone;
  /// The error icon, only on a red tag (OV-C59).
  readonly icon: boolean;
}

export interface QuotaView {
  readonly windows: readonly [QuotaWindow, QuotaWindow];
  readonly status: Status | null;
  /// Where the numbers came from and how old they are (OV-C56).
  readonly source: string;
  /// Which reading it is: the live usage, or the last response's headers (OV-C55).
  readonly from: 'live' | 'headers';
}

/// The tag's tone and word, from one field: the live reading's severity, or the header's status when
/// that is all there is (OV-C59). Red only for critical or rejected, with the error icon.
export function statusOf(field: string | null | undefined): Status | null {
  if (!field) return null;
  const f = field.toLowerCase();
  if (f === 'critical' || f === 'rejected' || f.includes('critical')) return { word: field, tone: 'bad', icon: true };
  if (f === 'warning' || f.includes('warning')) return { word: field, tone: 'warn', icon: false };
  if (f === 'normal' || f === 'allowed') return { word: field, tone: 'ok', icon: false };
  return { word: field, tone: 'neutral', icon: false };
}

const LABEL: Readonly<Record<WindowKey, string>> = { five_hour: '5-hour window', seven_day: '7-day window' };
const TONE: Readonly<Record<WindowKey, Tone>> = { five_hour: 'accent', seven_day: 'slate' };

/// A live reading the panel can stand on: one the router says it has, with at least its 5-hour window.
const usable = (live: LiveQuota | null | undefined): live is LiveQuota => !!live && live.source !== 'unavailable' && !!live.five_hour;

/// The quota panel: the live usage reading when there is one, which stays current on an idle machine,
/// else the last response's rate-limit headers, which move only when traffic flows (OV-C55); and it
/// says which, and how old (OV-C56). Null with neither: the panel says there is no reading yet.
export function quotaView(
  live: Reading<LiveQuota> | null | undefined,
  headers: HeaderQuota | null | undefined,
  fc: Pick<ForecastAccount, 'five_hour' | 'seven_day'> | null | undefined,
  now: number = Date.now(),
): QuotaView | null {
  const window = (key: WindowKey, used: number | null, reset: number | null): QuotaWindow => {
    const rolled = reset !== null && reset <= now;
    return {
      key,
      label: LABEL[key],
      used: rolled ? null : used,
      resets: reset !== null && !rolled ? `resets ${relative(reset, now)}` : undefined,
      resetsAt: reset !== null && !rolled ? when(reset) : undefined,
      expired: rolled ? `window rolled over ${ago(reset, now)} · nothing seen since` : undefined,
      tone: TONE[key],
      line: fcLine(fc?.[key] ?? null, now),
    };
  };
  const pct = (v: number | null | undefined): number | null => (typeof v === 'number' && Number.isFinite(v) ? v : null);

  if (live && usable(live.value)) {
    const q = live.value;
    const headersOnly = q.source === 'response-headers';
    const seen = headersOnly ? (q.seen_at ?? live.at) : live.at - (q.age_ms ?? 0);
    return {
      windows: [
        window('five_hour', pct(q.five_hour?.utilization), resetAt(q.five_hour?.resets_at)),
        window('seven_day', pct(q.seven_day?.utilization), resetAt(q.seven_day?.resets_at)),
      ],
      status: statusOf(q.severity),
      source: `${headersOnly ? 'from the last request' : 'checked'} ${ago(seen, now)}`,
      from: headersOnly ? 'headers' : 'live',
    };
  }
  if (headers) {
    const frac = (v: number | null) => (typeof v === 'number' && Number.isFinite(v) ? v * 100 : null);
    const secs = (v: number | null) => (typeof v === 'number' && Number.isFinite(v) ? v * 1000 : null);
    return {
      windows: [window('five_hour', frac(headers.rl_5h_utilization), secs(headers.rl_5h_reset)), window('seven_day', frac(headers.rl_7d_utilization), secs(headers.rl_7d_reset))],
      status: statusOf(headers.rl_status),
      source: `from the last request ${ago(headers.started_at, now)}`,
      from: 'headers',
    };
  }
  return null;
}

/// The serving account as the caption names it: its email, else its label, with its plan; "no
/// account" when nothing serves (SH-C16).
export function servingName(account: ServingAccount | null | undefined): string {
  if (!account) return 'no account';
  const name = account.email || account.label || account.id;
  return account.subscription_type ? `${name} · ${account.subscription_type}` : name;
}

/// The serving account's forecast: the one whose id the summary names (OV-C54).
export function forecastFor(accounts: readonly ForecastAccount[] | null | undefined, account: ServingAccount | null | undefined): ForecastAccount | null {
  if (!account) return null;
  return (accounts ?? []).find((a) => a.id === account.id) ?? null;
}

/// The quota history: the 5-hour and 7-day percentages as two lines on the activity chart's time
/// axis, one point per bucket, the worst reading in it, capped at 100 %. A bucket with no reading for
/// a window is left off that window's line, never 0 % (OV-C60, OV-C61).
export function history(rows: readonly QuotaHistoryRow[] | null | undefined, bk: readonly Bucket[], now: number = Date.now()): { readonly buckets: string[]; readonly series: Series[] } | null {
  const slot = new Map(bk.map((b, i) => [b.t, i]));
  const points = (rows ?? []).filter((r) => (r.rl_5h != null || r.rl_7d != null) && slot.has(Number(r.t)));
  if (!points.length || !bk.length) return null;
  const five: (number | null)[] = bk.map(() => null);
  const seven: (number | null)[] = bk.map(() => null);
  const cap = (v: number) => Math.min(100, Math.max(0, v * 100));
  for (const p of points) {
    const i = slot.get(Number(p.t))!;
    if (typeof p.rl_5h === 'number') five[i] = cap(p.rl_5h);
    if (typeof p.rl_7d === 'number') seven[i] = cap(p.rl_7d);
  }
  return {
    buckets: bk.map((b) => tod(b.t, now)),
    series: [
      { key: '5h', label: '5-hour used', values: five, colour: '--cds-interactive' },
      { key: '7d', label: '7-day used', values: seven, colour: '--ar-tok' },
    ],
  };
}

/// A forecast window as the burn-down draws it, in milliseconds and fractions of the limit; null for a
/// window that cannot be drawn, which the burn-down then leaves out (OV-C84).
export function burnOf(w: Window | null | undefined): Burn | null {
  if (!w || w.expired || w.window_start == null || w.reset_at == null) return null;
  // The cone only when the band has width, as the model's own burn-down decides it (OV-C83).
  const band = !!w.duty && w.duty.low !== w.duty.high;
  return {
    start: w.window_start * 1000,
    reset: w.reset_at * 1000,
    used: typeof w.used === 'number' ? w.used : 0,
    series: w.series ?? [],
    crossing: w.exhausts_at ?? null,
    end: w.projected_end_util ?? null,
    verdict: w.verdict ?? null,
    band: band
      ? {
          quiet: { crossing: w.exhausts_at_late ?? null, end: w.projected_end_util_low ?? null },
          busy: { crossing: w.exhausts_at_early ?? null, end: w.projected_end_util_high ?? null },
        }
      : null,
    expired: false,
  };
}
