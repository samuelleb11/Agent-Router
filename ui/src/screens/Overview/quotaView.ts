// New for agent-router: the quota panel's figures with no DOM (assets/index.html, `renderQuota`,
// `renderRunway`, `renderHeader`'s account chip). The panel holds one block per provider with an
// account, and each block's meters, status tag, history and burn-downs describe that provider's
// serving account, whatever the page is scoped to (OV-C54, OV-C224): each window by its own name, Kimi
// Code's month as a month.
//
// Where the page broke its own rule, the rule is kept:
// - a window with no reading, or a usage reply without a 7-day window, shows "—", never "0% used"
//   (OV-C58);
// - the status tag's colour and its word come from one field, and the error icon only from a red
//   one (OV-C59), where the colour also read the last header's `rl_status`;
// - a header reading from before its window's reset is never shown as current (OV-C72);
// - the second window wears slate on its meter as on its line and its burn-down (OV-C62).

import type { Burn } from '~/components/BurnDown';
import type { Bucket } from './timeline';
import type { BlockHistoryRow, LiveQuota, ProviderQuota, Reading, ServingAccount } from '~/data/overview';
import { ago, relative, resetAt, tod, when } from '~/model/format';
import { fcLine, windowLabel, windowName, type ForecastAccount, type Line, type Window, type WindowKey } from '~/model/forecast';
import type { Series } from '~/components/Sparkline';

export type Tone = 'accent' | 'slate';

/// A block's second window: a week for Anthropic and a Kimi Code legacy plan, a month for a Kimi Code
/// newer plan (OV-C218, OV-C224).
export type SecondKey = Exclude<WindowKey, 'five_hour'>;

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
  /// The 5-hour window is the accent, the second slate (OV-C62).
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
  /// The 5-hour window, then the second by its own name.
  readonly windows: readonly [QuotaWindow, QuotaWindow];
  readonly status: Status | null;
  /// Where the numbers came from and how old they are (OV-C56).
  readonly source: string;
  /// Which reading it is: a usage API's, current on an idle machine — Anthropic's live reading, or
  /// Kimi Code's own — or the last response's headers (OV-C55).
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

/// A block's second window, by the name its provider gives it: `month` when the block carries one,
/// else `seven_day` when it carries that, else what the forecast has for the account (OV-C224).
export function secondOf(block: Pick<ProviderQuota, 'windows'> | null | undefined, fc?: Pick<ForecastAccount, 'month'> | null): SecondKey {
  const keys = block?.windows ? Object.keys(block.windows) : [];
  if (keys.includes('month')) return 'month';
  if (keys.includes('seven_day')) return 'seven_day';
  return fc?.month != null ? 'month' : 'seven_day';
}

/// A live reading the panel can stand on: one the router says it has, with at least its 5-hour window.
const usable = (live: LiveQuota | null | undefined): live is LiveQuota => !!live && live.source !== 'unavailable' && !!live.five_hour;

/// One block's meters, status and source (OV-C224). Anthropic's block prefers the live usage reading
/// (`/api/quota`, Anthropic's serving account's), which stays current on an idle machine, for the
/// account it was read for, else the last response's rate-limit headers, which move only when traffic
/// flows (OV-C55); and it says which, and how old (OV-C56). Kimi Code's windows are its usage API's
/// reading, checked, its second by its own name. Null with no reading: the block says there is none
/// yet.
export function quotaView(
  live: Reading<LiveQuota> | null | undefined,
  block: Pick<ProviderQuota, 'provider' | 'account' | 'windows' | 'status' | 'at'> | null | undefined,
  fc: Pick<ForecastAccount, 'five_hour' | 'seven_day' | 'month'> | null | undefined,
  now: number = Date.now(),
): QuotaView | null {
  const window = (key: WindowKey, used: number | null, reset: number | null): QuotaWindow => {
    const rolled = reset !== null && reset <= now;
    return {
      key,
      label: windowLabel(key),
      used: rolled ? null : used,
      resets: reset !== null && !rolled ? `resets ${relative(reset, now)}` : undefined,
      resetsAt: reset !== null && !rolled ? when(reset) : undefined,
      expired: rolled ? `window rolled over ${ago(reset, now)} · nothing seen since` : undefined,
      tone: key === 'five_hour' ? 'accent' : 'slate',
      line: fcLine(fc?.[key] ?? null, now),
    };
  };
  const pct = (v: number | null | undefined): number | null => (typeof v === 'number' && Number.isFinite(v) ? v : null);

  // `/api/quota` reads Anthropic's serving account: it stands for that block alone, and only for the
  // account it says it read.
  const anthropic = !block || block.provider === 'anthropic';
  if (anthropic && live && usable(live.value) && (!block || !live.value.account_id || live.value.account_id === block.account.id)) {
    const q = live.value;
    const headersOnly = q.source === 'response-headers';
    const seen = headersOnly ? (q.seen_at ?? live.at) : live.at - (q.age_ms ?? 0);
    return {
      windows: [window('five_hour', pct(q.five_hour?.utilization), resetAt(q.five_hour?.resets_at)), window('seven_day', pct(q.seven_day?.utilization), resetAt(q.seven_day?.resets_at))],
      status: statusOf(q.severity),
      source: `${headersOnly ? 'from the last request' : 'checked'} ${ago(seen, now)}`,
      from: headersOnly ? 'headers' : 'live',
    };
  }
  if (block && Object.values(block.windows ?? {}).some((w) => w != null)) {
    const frac = (v: number | null | undefined) => (typeof v === 'number' && Number.isFinite(v) ? v * 100 : null);
    const secs = (v: number | null | undefined) => (typeof v === 'number' && Number.isFinite(v) ? v * 1000 : null);
    const of = (key: WindowKey) => window(key, frac(block.windows[key]?.utilization), secs(block.windows[key]?.resets_at));
    // Anthropic's block is its last response's headers; any other provider's, its usage API's reading.
    // A window the provider said is spent can stand before any usage was read.
    const read = anthropic ? 'from the last request' : 'checked';
    return {
      windows: [of('five_hour'), of(secondOf(block, fc))],
      status: statusOf(block.status),
      source: block.at != null ? `${read} ${ago(block.at, now)}` : anthropic ? read : 'usage not read yet',
      from: anthropic ? 'headers' : 'live',
    };
  }
  return null;
}

/// A serving account as its block names it: its email, else its label, with its plan (SH-C16).
export function servingName(account: Pick<ServingAccount, 'id' | 'label' | 'email' | 'plan'>): string {
  const name = account.email || account.label || account.id;
  return account.plan ? `${name} · ${account.plan}` : name;
}

/// A serving account's forecast: the one with its id (OV-C54).
export function forecastFor(accounts: readonly ForecastAccount[] | null | undefined, account: Pick<ServingAccount, 'id'> | null | undefined): ForecastAccount | null {
  if (!account) return null;
  return (accounts ?? []).find((a) => a.id === account.id) ?? null;
}

/// A block's quota history: its 5-hour and second windows' percentages as two lines on the activity
/// chart's time axis, one point per bucket, the worst reading in it, capped at 100 %, each line named
/// by its window. A bucket with no reading for a window is left off that window's line, never 0 %
/// (OV-C60, OV-C61, OV-C224).
export function history(rows: readonly BlockHistoryRow[] | null | undefined, second: SecondKey, bk: readonly Bucket[], now: number = Date.now()): { readonly buckets: string[]; readonly series: Series[] } | null {
  const slot = new Map(bk.map((b, i) => [b.t, i]));
  const points = (rows ?? []).filter((r) => (r.five_hour != null || r[second] != null) && slot.has(Number(r.t)));
  if (!points.length || !bk.length) return null;
  const five: (number | null)[] = bk.map(() => null);
  const other: (number | null)[] = bk.map(() => null);
  const cap = (v: number) => Math.min(100, Math.max(0, v * 100));
  for (const p of points) {
    const i = slot.get(Number(p.t))!;
    const a = p.five_hour;
    const b = p[second];
    if (typeof a === 'number') five[i] = cap(a);
    if (typeof b === 'number') other[i] = cap(b);
  }
  return {
    buckets: bk.map((b) => tod(b.t, now)),
    series: [
      { key: '5h', label: `${windowName('five_hour')} used`, values: five, colour: '--cds-interactive' },
      { key: second === 'month' ? 'month' : '7d', label: `${windowName(second)} used`, values: other, colour: '--ar-tok' },
    ],
  };
}

/// A forecast window as the burn-down draws it, in milliseconds and fractions of the limit; null for a
/// window that cannot be drawn, which the burn-down then leaves out (OV-C84).
export function burnOf(w: Window | null | undefined): Burn | null {
  if (!w || w.expired || w.window_start == null || w.reset_at == null) return null;
  // The cone only when the band has width, as the model's own burn-down decides it (OV-C83).
  const band = !!w.duty && w.duty.low !== w.duty.high;
  // The short trend beside the long one, when the router could weigh the recent load (OV-C223).
  const r = w.recent;
  const recent = r && Number.isFinite(r.load) && r.load > 0 ? { crossing: r.exhausts_at ?? null, end: r.projected_end_util, load: r.load, hours: r.measured_active_hours, until: r.measured_until } : null;
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
    recent,
  };
}
