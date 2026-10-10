// New for agent-router: the dashboard's quota forecast (assets/index.html, `fcState`, `fcBasis`,
// `fcLine`, `runwaySvg`, `renderRunway`, `renderAllAccounts`) with no DOM. What a window's
// projection says and how loudly, the burn-down's geometry, and the all-accounts panel, worded as
// docs/web-console-contracts.md writes them down (OV-C66 to OV-C97). The iPhone app's
// `Projection.describe` says the same words, so the phone and the Mac never disagree about the
// same numbers (OV-C69).
//
// Where the page broke its own rule, the rule is kept:
// - a window already at 100 % says it has run out, in red, whatever basis its burn came from,
//   rather than "not enough burn yet to project" under a full meter (OV-C74);
// - a tight verdict whose central estimate does not cross never prints a crossing it was not
//   given, where the page printed "runs out ~20000d before reset" (OV-C70);
// - the burn-down's projection is amber for every tight verdict, its central estimate crossing or
//   not (OV-C82);
// - Headroom turns red only for a crossing, amber and worded as too close to call for a tight
//   window, and names the first account to run out by its label, with the window and the time
//   (OV-C92, OV-C93);
// - a card's window with no reading shows "—", never "0% used" (OV-C58, OV-C94).
//
// A Kimi Code account reports its windows through its own usage API rather than rate-limit headers:
// its card shows the 5-hour window and the second under its real name, a month or a week, a spent
// window as spent until its reset, its last request whenever it served one, and "not priced" where
// the router could not price it (OV-C218 to OV-C221). Every cost here is a cost sum: its priced part,
// marked with the requests it leaves out and "≈" where it rests on estimated tokens, the reasons in
// its tooltip (OV-C229, OV-C232).
//
// Headroom is each provider's own, in that provider's quota block: no figure compares one
// provider's windows with another's, Anthropic's 7-day with Kimi Code's month (OV-C225).

import { DASH, ago, compact, costSum, count, duration, hours, plural, relative, tod, when, type CostMark } from './format';
import { providerOf, type Provider } from './settings';

export interface Duty {
  readonly low: number;
  readonly mid: number;
  readonly high: number;
}

export type Verdict = 'crosses' | 'tight' | 'resets-first';

/// One window of `GET /api/forecast` (src/api.rs, src/forecast.rs). Every key is optional: an
/// expired window carries three, and a router from before the band sends no `duty`, no `verdict`
/// and none of the `_early`, `_late`, `_low` or `_high` keys.
export interface Window {
  readonly used?: number | null;
  /// Epoch seconds.
  readonly reset_at?: number | null;
  /// Epoch seconds.
  readonly window_start?: number | null;
  readonly burn_per_active_hour?: number | null;
  readonly active_hours_left?: number | null;
  readonly duty_fraction?: number | null;
  readonly duty?: Duty | null;
  readonly verdict?: Verdict | null;
  /// Epoch milliseconds, as are the two beside it.
  readonly exhausts_at?: number | null;
  readonly exhausts_at_early?: number | null;
  readonly exhausts_at_late?: number | null;
  readonly resets_first?: boolean | null;
  readonly projected_end_util?: number | null;
  readonly projected_end_util_low?: number | null;
  readonly projected_end_util_high?: number | null;
  readonly basis?: 'this-window' | 'trailing-windows' | 'none' | (string & {}) | null;
  readonly measured_active_hours?: number | null;
  readonly readings?: number | null;
  readonly expired?: boolean | null;
  readonly seen_at?: number | null;
  /// The provider refused a request for this window: spent until `reset_at` (OV-C219).
  readonly spent?: boolean | null;
  /// Readings across the window, epoch milliseconds and a fraction used.
  readonly series?: readonly { readonly t: number; readonly u: number }[] | null;
  /// The short trend: the window at the load of the last half hour of work (OV-C223). Null
  /// without a fit on this window and priced requests to weigh.
  readonly recent?: {
    /// That stretch's load ÷ the load the long trend was fitted on.
    readonly load: number;
    readonly burn_per_active_hour: number;
    readonly measured_active_hours: number;
    /// Epoch milliseconds, as are the two after it.
    readonly measured_from: number;
    readonly measured_until: number;
    readonly exhausts_at: number | null;
    readonly projected_end_util: number;
  } | null;
}

/// A quota window by its own name: the 5-hour, and the second, a week for an Anthropic account and a
/// Kimi Code account on a legacy plan, a month for a Kimi Code account on a newer one (OV-C218).
export type WindowKey = 'five_hour' | 'seven_day' | 'month';

const WINDOW_NAME: Readonly<Record<WindowKey, string>> = { five_hour: '5-hour', seven_day: '7-day', month: 'month' };

/// A window named by its own key, as a sentence says it: "5-hour", "7-day", "month" (OV-C224).
export const windowName = (key: WindowKey): string => WINDOW_NAME[key];

/// A window as a meter or a burn-down is captioned: "5-hour window", "7-day window", "Month window".
export const windowLabel = (key: WindowKey): string => {
  const name = windowName(key);
  return `${name.charAt(0).toUpperCase()}${name.slice(1)} window`;
};

export interface ForecastAccount {
  readonly id: string;
  readonly label?: string | null;
  readonly email?: string | null;
  readonly plan?: string | null;
  readonly active?: number | boolean | null;
  /// `anthropic` or `kimi`; absent from a router that predates providers, and for an account the
  /// router no longer holds.
  readonly provider?: string | null;
  readonly usage?: {
    readonly requests?: number | null;
    readonly prompt_tokens?: number | null;
    readonly output_tokens?: number | null;
    readonly cost?: number | null;
    /// Requests with usage the router could not price: a cost of 0 beside them is not free.
    readonly unpriced?: number | null;
    /// Requests whose tokens were estimated rather than read from their stored response (OV-C232).
    readonly estimated?: number | null;
    /// Epoch milliseconds of its newest request in the range.
    readonly last_seen?: number | null;
  } | null;
  /// Epoch milliseconds: when its rate-limit headers were last seen, or for a Kimi Code account
  /// when its usage was last read.
  readonly seen_at?: number | null;
  readonly status?: string | null;
  /// The 7-day reset that identifies a quota pool: two credentials onto one Anthropic account
  /// report the same one.
  readonly pool?: number | string | null;
  readonly five_hour?: Window | null;
  readonly seven_day?: Window | null;
  readonly month?: Window | null;
}

/// One provider's headroom (src/api.rs `headroom_of`): hours of work left across its quota pools,
/// each pool counting the window that runs dry first, and the first of its windows to run out —
/// every window its accounts have, Kimi Code's month included.
export interface Headroom {
  readonly active_hours_left?: number | null;
  readonly pools?: number | null;
  readonly first_to_exhaust?: {
    readonly account_id: string;
    readonly window: WindowKey;
    /// Epoch milliseconds.
    readonly at: number;
    readonly in_ms: number;
  } | null;
}

export interface Forecast {
  readonly generated_at?: number | null;
  readonly accounts?: readonly ForecastAccount[] | null;
  readonly global?: {
    readonly requests?: number | null;
    readonly prompt_tokens?: number | null;
    readonly output_tokens?: number | null;
    readonly cost?: number | null;
    readonly accounts?: number | null;
    /// Each provider's headroom, by provider, Anthropic's first: one per provider with an account,
    /// and none totalled across them (OV-C225).
    readonly providers?: Readonly<Record<string, Headroom>> | null;
    readonly pace_30d?: {
      readonly cost?: number | null;
      /// The week's requests the router could not price, which its cost leaves out, and those whose
      /// tokens were estimated (OV-C229, OV-C232).
      readonly unpriced?: number | null;
      readonly estimated?: number | null;
      readonly prompt_tokens?: number | null;
      readonly basis?: string | null;
    } | null;
  } | null;
}

/// How loudly a line speaks: red only for a crossing that survives even a quiet week (OV-C68).
export type Tone = 'plain' | 'warn' | 'bad';

export interface State {
  readonly tone: Tone;
  readonly text: string;
}

const num = (v: unknown): number => (typeof v === 'number' && Number.isFinite(v) ? v : 0);
const clamp = (v: number): number => Math.min(1, Math.max(0, v));

/// The three-state answer. A router from before the band sends no verdict, and is read by the old
/// boolean — a crossing time means crosses, none means resets first — never by going quiet
/// (OV-C71).
export function verdictOf(w: Window): Verdict {
  return w.verdict ?? (w.exhausts_at ? 'crosses' : 'resets-first');
}

/// What a window's projection says, and how loudly to say it. Work left comes first, in hours of
/// work, the measured number; a calendar time follows (OV-C67).
export function fcState(w: Window | null | undefined, now: number = Date.now()): State {
  if (!w) return { tone: 'plain', text: 'no rate-limit reading yet' };
  // An old utilisation is never shown as current (OV-C72).
  if (w.expired) return { tone: 'plain', text: `window rolled over ${ago(num(w.reset_at) * 1000, now)} · nothing seen since` };
  // The provider said so: spent until it resets, with nothing to project (OV-C219).
  if (w.spent) return { tone: 'bad', text: `spent until it resets ${relative(num(w.reset_at) * 1000, now)}, at ${when(num(w.reset_at) * 1000)}` };
  // Already there, whatever basis the burn came from (OV-C74). The router says so with no hours
  // left and a crossing at the moment it read the window.
  const full = num(w.used) >= 1;
  if (!full && (w.basis === 'none' || w.active_hours_left == null)) return { tone: 'plain', text: 'not enough burn yet to project' };
  const left = hours(num(w.active_hours_left));
  const reset = num(w.reset_at) * 1000;
  const verdict = full ? 'crosses' : verdictOf(w);
  if (verdict === 'tight') {
    if (w.exhausts_at) {
      const quiet = Math.round(num(w.projected_end_util_low) * 100);
      return {
        tone: 'warn',
        text: `${left} of work left · too close to call — on this pace it runs out ${duration(reset - num(w.exhausts_at))} before reset, on a quieter week it ends ≈${quiet}%`,
      };
    }
    // Tight means only the busy edge crosses, so the central estimate may not (OV-C70): say where
    // this pace ends, and the busy edge's crossing only when the router gave one.
    const end = Math.round(num(w.projected_end_util) * 100);
    const busy = w.exhausts_at_early ? `, on a busier week it runs out ${duration(reset - num(w.exhausts_at_early))} before reset` : '';
    return { tone: 'warn', text: `${left} of work left · too close to call — on this pace it ends ≈${end}%${busy}` };
  }
  if (verdict === 'crosses') {
    const at = w.exhausts_at ? num(w.exhausts_at) : now;
    return { tone: 'bad', text: `${left} of work left · runs out ${relative(at, now)}, ${duration(reset - at)} before it resets` };
  }
  const end = Math.round(num(w.projected_end_util) * 100);
  return { tone: end >= 90 ? 'warn' : 'plain', text: `${left} of work left · resets first, ending ≈${end}%` };
}

/// What a projection was fitted on, as visible text under it (OV-C76). A borrowed rate never
/// claims a fit (OV-C77), and the duty band is disclosed exactly when it has width (OV-C78).
export function fcBasis(w: Window | null | undefined): string | null {
  if (!w || w.expired || w.spent || w.basis === 'none' || !num(w.burn_per_active_hour)) return null;
  const rate = `${(num(w.burn_per_active_hour) * 100).toFixed(1)}% per hour of work`;
  if (w.basis === 'trailing-windows') return `${rate} · nothing burnt yet this window, so this is your recent average`;
  const pct = (v: unknown) => Math.round(num(v) * 100);
  const d = w.duty;
  const duty =
    d && pct(d.low) !== pct(d.high)
      ? `${pct(d.mid)}% of the clock spent working, ${pct(d.low)}–${pct(d.high)}% across your days`
      : `${pct(w.duty_fraction)}% of the clock spent working`;
  return `${rate}, fitted on ${hours(num(w.measured_active_hours))} · ${duty}`;
}

export interface Line extends State {
  readonly basis: string | null;
}

/// The line under a meter. With no forecast for the window at all — the endpoint is absent, or the
/// account never carried a rate-limit header — there is no line, rather than "no reading" under a
/// meter that is showing one (OV-C75).
export function fcLine(w: Window | null | undefined, now: number = Date.now()): Line | null {
  if (!w) return null;
  return { ...fcState(w, now), basis: fcBasis(w) };
}

// ---------- meters ----------

export type Level = 'none' | 'normal' | 'warn' | 'bad';

export interface Meter {
  readonly text: string;
  /// How full the bar is drawn, 0 to 100: capped, never more.
  readonly fill: number;
  /// Amber from 70 %, red from 90 % (OV-C57); `none` when there is no reading.
  readonly level: Level;
}

/// A quota meter: "N% used · resets in …" over a bar capped at 100 %, and "—" when the window has
/// no reading, never "0% used" (OV-C57, OV-C58).
export function meter(used: number | null | undefined, resetMs: number | null | undefined, now: number = Date.now()): Meter {
  if (typeof used !== 'number' || !Number.isFinite(used)) return { text: DASH, fill: 0, level: 'none' };
  const p = Math.min(100, used * 100);
  return { text: `${p.toFixed(0)}% used · resets ${relative(resetMs ?? null, now)}`, fill: p, level: p >= 90 ? 'bad' : p >= 70 ? 'warn' : 'normal' };
}

/// A window's meter on an account card: "—" for a window rolled over or never read (OV-C94).
export function windowMeter(w: Window | null | undefined, now: number = Date.now()): Meter {
  if (!w || w.expired) return { text: DASH, fill: 0, level: 'none' };
  return meter(w.used, w.reset_at != null ? num(w.reset_at) * 1000 : null, now);
}

// ---------- the burn-down ----------

export type Point = readonly [number, number];

export interface BurnDown {
  readonly width: number;
  readonly height: number;
  /// The plot's edges: time runs from `left` (the window opening) to `right` (its reset), and
  /// usage from `bottom` (0) to `top` (100 %).
  readonly left: number;
  readonly right: number;
  readonly top: number;
  readonly bottom: number;
  /// Observed usage, oldest first, ending at now.
  readonly observed: readonly Point[];
  readonly nowX: number;
  /// Always drawn: to the crossing on the 100 % line when there is one, otherwise to the reset at
  /// the level it ends (OV-C81). Muted when it resets first, amber when tight, red when it crosses
  /// (OV-C82).
  readonly projection: { readonly from: Point; readonly to: Point; readonly tone: 'muted' | 'warn' | 'bad' };
  /// The cone between a quiet week and a busy one, only when the duty band has width (OV-C83).
  readonly cone: { readonly points: readonly Point[]; readonly tone: 'bad' | 'muted' } | null;
  /// The crossing, and only a crossing (OV-C81).
  readonly marker: Point | null;
  /// "Oct 3 09:00" at the left, "resets Oct 10 09:00" at the right.
  readonly opened: string;
  readonly resets: string;
}

const H = 84;
const L = 32;
const TOP = 12;
const BOT = 56;

/// One window drawn across its own life, observed left of now and projected right of it, at the
/// width it is shown. A window that cannot be drawn is not: an expired or absent one, one with no
/// start, or one whose reset is not after its start draws nothing rather than dividing by zero,
/// and values outside the window are held to its edges (OV-C84).
export function burnDown(w: Window | null | undefined, width: number, now: number = Date.now()): BurnDown | null {
  if (!w || w.expired || w.window_start == null) return null;
  const t0 = num(w.window_start) * 1000;
  const t1 = num(w.reset_at) * 1000;
  if (!(t1 > t0)) return null;
  const R = width - 8;
  const x = (t: number) => L + (R - L) * clamp((t - t0) / (t1 - t0));
  const y = (u: number) => BOT - (BOT - TOP) * clamp(u);
  const at = Math.min(now, t1);
  const used = num(w.used);
  const here: Point = [x(at), y(used)];
  const observed: Point[] = [...(w.series ?? []).map((p): Point => [x(num(p.t)), y(num(p.u))]), here];
  // Where one duty cycle lands: the crossing if it reaches 100 %, otherwise the level it stops
  // under at the reset.
  const ray = (crossing: number | null | undefined, end: number | null | undefined): Point =>
    crossing ? [x(crossing), y(1)] : [x(t1), y(num(end))];
  const to = ray(w.exhausts_at, w.projected_end_util);
  const verdict = verdictOf(w);
  const band = !!w.duty && num(w.duty.low) !== num(w.duty.high);
  const hi = ray(w.exhausts_at_early, w.projected_end_util_high);
  const lo = ray(w.exhausts_at_late, w.projected_end_util_low);
  return {
    width,
    height: H,
    left: L,
    right: R,
    top: TOP,
    bottom: BOT,
    observed,
    nowX: x(at),
    projection: { from: here, to, tone: verdict === 'crosses' ? 'bad' : verdict === 'tight' ? 'warn' : 'muted' },
    cone: band ? { points: [here, hi, [x(t1), hi[1]], [x(t1), lo[1]], lo], tone: w.verdict === 'crosses' ? 'bad' : 'muted' } : null,
    marker: w.exhausts_at ? to : null,
    opened: tod(t0, now),
    resets: `resets ${tod(t1, now)}`,
  };
}

export interface BurnDownWindow {
  readonly key: WindowKey;
  readonly label: string;
  readonly window: Window;
}

/// A provider's serving account's windows that get a burn-down, captioned "<window> · burn-down",
/// Kimi Code's month by its name (OV-C80, OV-C224); none for an account with nothing current
/// (OV-C84).
export function burnDowns(account: Pick<ForecastAccount, 'five_hour' | 'seven_day' | 'month'> | null | undefined): BurnDownWindow[] {
  const keys: readonly WindowKey[] = ['five_hour', 'seven_day', 'month'];
  return keys.flatMap((key) => {
    const window = account?.[key];
    return window && !window.expired && window.window_start != null ? [{ key, label: windowLabel(key), window }] : [];
  });
}

/// Two burn-downs sit side by side only when both fit at 250 px, otherwise they stack, and each is
/// drawn at the width it is shown (OV-C85).
export function burnDownLayout(gridWidth: number, n: number): { columns: number; width: number } {
  const columns = n > 1 && gridWidth >= 250 * n + 22 * (n - 1) ? n : 1;
  return { columns, width: Math.max(200, Math.floor((gridWidth - 22 * (columns - 1)) / columns)) };
}

// ---------- every account ----------

export interface Tile {
  readonly label: string;
  readonly value: string;
  readonly sub: string;
  readonly tone: Tone;
  /// A cost's marks, each with its reason, for the value's tooltip (OV-C229, OV-C232).
  readonly marks?: readonly CostMark[];
}

export interface CardWindow {
  readonly key: WindowKey;
  readonly label: string;
  /// The window as the router sent it, for the meter to draw.
  readonly window: Window | null;
  readonly meter: Meter;
  readonly line: Line | null;
}

export interface AccountCard {
  readonly id: string;
  readonly name: string;
  /// Its provider, Anthropic's for an account the router names none for, tagged beside its name by
  /// its mark and name (OV-C218, OV-C231).
  readonly provider: Provider;
  /// Serving its provider's new requests.
  readonly active: boolean;
  /// Its plan, when it was last seen, and any quota it shares.
  readonly sub: string;
  /// The 5-hour window, then the second: 7-day, or a Kimi Code account's month (OV-C218).
  readonly windows: readonly [CardWindow, CardWindow];
  readonly cost: string;
  /// Its cost's marks, each with its reason, for the cost's tooltip (OV-C229, OV-C232).
  readonly costMarks: readonly CostMark[];
  readonly read: string;
  readonly requests: string;
}

export interface FirstToGo {
  readonly name: string;
  readonly window: string;
  /// Epoch milliseconds.
  readonly at: number;
  readonly when: string;
  readonly verdict: Verdict;
}

/// Spend, tokens, requests and pace add up across accounts and providers; headroom does not, and is
/// each provider's own, in its quota block (OV-C225).
export interface AllAccounts {
  readonly caption: string;
  readonly spend: Tile;
  readonly tokens: Tile;
  readonly requests: Tile;
  readonly pace: Tile;
  readonly cards: readonly AccountCard[];
}

/// One provider's headroom, for its quota block: "Headroom left", and the first of its windows to run
/// out (OV-C225).
export interface ProviderHeadroom {
  readonly tile: Tile;
  readonly first: FirstToGo | null;
}

const nameOf = (a: Pick<ForecastAccount, 'id' | 'label'>): string => a.label || a.id;

/// An account's second window and its real name: a Kimi Code account on a newer plan has a month
/// where every other has a week (OV-C218).
function secondWindow(a: ForecastAccount): { readonly key: WindowKey; readonly label: string; readonly window: Window | null | undefined } {
  return a.month != null ? { key: 'month', label: 'Month', window: a.month } : { key: 'seven_day', label: '7-day', window: a.seven_day };
}

/// One provider's headroom, from its own entry in `global.providers`: its hours of work left across
/// its quota pools, and the first of its windows to run out, named by its account's label, with the
/// window by its own name and the time (OV-C225). Red only for a window whose verdict crosses, amber
/// and worded as too close to call for a tight one (OV-C92, OV-C93). Null when the forecast has
/// nothing for the provider, or could not be read. Nothing here reads another provider's windows.
export function providerHeadroom(fc: Forecast | null | undefined, provider: Provider, now: number = Date.now()): ProviderHeadroom | null {
  const room = fc?.global?.providers?.[provider];
  if (!room) return null;
  const first = room.first_to_exhaust;
  if (!first) {
    return {
      tile: { label: 'Headroom left', value: hours(room.active_hours_left), sub: `${plural(room.pools, 'quota pool', 'quota pools')}, none projected to run out`, tone: 'plain' },
      first: null,
    };
  }
  const account = (fc?.accounts ?? []).find((a) => !!a && a.id === first.account_id);
  const w = account?.[first.window];
  const verdict: Verdict = w ? verdictOf(w) : 'crosses';
  const window = windowName(first.window);
  const who = account ? nameOf(account) : first.account_id;
  const at = first.in_ms > 0 ? relative(now + first.in_ms, now) : 'now';
  return {
    tile: {
      label: 'Headroom left',
      value: hours(room.active_hours_left),
      sub:
        verdict === 'tight'
          ? `too close to call: ${who}'s ${window} window could run out ${at}`
          : verdict === 'crosses'
            ? `first to go: ${who}'s ${window} window, ${at}`
            : `${who}'s ${window} window is projected to reset first`,
      tone: verdict === 'crosses' ? 'bad' : verdict === 'tight' ? 'warn' : 'plain',
    },
    first: { name: who, window, at: first.at, when: when(first.at), verdict },
  };
}

/// When an account was last seen, in words (OV-C94, OV-C220). An account that served requests is
/// never "never seen": a rate-limit reading says when its last request was, and with none its newest
/// request in the range does. A Kimi Code account's reading is of its usage, not of a request, so it
/// says both.
function seenWords(a: ForecastAccount, now: number): string {
  const lastRequest = a.usage?.last_seen;
  if (a.provider === 'kimi') {
    const said = [lastRequest ? `last request ${ago(lastRequest, now)}` : null, a.seen_at ? `usage read ${ago(a.seen_at, now)}` : null].filter(Boolean);
    return said.length ? said.join(' · ') : 'never seen';
  }
  const at = a.seen_at || lastRequest;
  return at ? `last request ${ago(at, now)}` : 'never seen';
}


/// Every account at once, whatever the page is scoped to (OV-C87); null — the panel, its heading
/// and its tiles gone — when the forecast lists no account or could not be read (OV-C86). Spend,
/// tokens and requests add up across accounts; quota percentages never do (OV-C88), and headroom is
/// each provider's own, in its quota block (OV-C225).
export function allAccounts(fc: Forecast | null | undefined, now: number = Date.now()): AllAccounts | null {
  const list = (fc?.accounts ?? []).filter((a): a is ForecastAccount => !!a && !!a.id);
  if (!list.length) return null;
  const g = fc?.global ?? {};
  const pace = g.pace_30d ?? {};

  // Two credentials onto one Anthropic account share a quota and report the same 7-day reset: say
  // so on every card in the pool, rather than letting the page read as two allowances (OV-C95).
  const pools = new Map<string, string[]>();
  for (const a of list) {
    if (a.pool == null) continue;
    const key = String(a.pool);
    pools.set(key, [...(pools.get(key) ?? []), nameOf(a)]);
  }
  const cardWindow = (key: WindowKey, label: string, w: Window | null | undefined): CardWindow => ({
    key,
    label,
    window: w ?? null,
    meter: windowMeter(w, now),
    line: fcLine(w, now),
  });
  const cards = list.map((a): AccountCard => {
    const shared = a.pool == null ? [] : (pools.get(String(a.pool)) ?? []).filter((x) => x !== nameOf(a));
    // The provider is the tag beside the name, every card's (OV-C231), so the line does not say it again.
    const sub = [a.plan, seenWords(a, now), shared.length ? `shares a quota with ${shared.join(', ')}` : null].filter(Boolean).join(' · ');
    const u = a.usage ?? {};
    // What it cost (OV-C221, OV-C229): a cost of 0 beside requests the router could not price is not
    // free, so it reads "not priced", never "$0", and a priced cost with some it could not price
    // says how many it leaves out.
    const cost = costSum({ cost: u.cost, unpriced: u.unpriced, estimated: u.estimated });
    const second = secondWindow(a);
    return {
      id: a.id,
      name: nameOf(a),
      provider: providerOf(a),
      active: a.active === true || num(a.active) > 0,
      sub,
      windows: [cardWindow('five_hour', '5-hour', a.five_hour), cardWindow(second.key, second.label, second.window)],
      cost: cost.text,
      costMarks: cost.marks,
      read: `${compact(u.prompt_tokens)} read`,
      requests: `${count(u.requests)} req`,
    };
  });

  // The requests no account's cost includes, and those whose tokens are estimates: a total over them
  // is marked, never read as their price (OV-C221, OV-C229, OV-C232).
  const spend = costSum({
    cost: g.cost,
    unpriced: list.reduce((sum, a) => sum + num(a.usage?.unpriced), 0),
    estimated: list.reduce((sum, a) => sum + num(a.usage?.estimated), 0),
  });
  const paced = costSum({ cost: pace.cost, unpriced: pace.unpriced, estimated: pace.estimated });
  return {
    caption: `${plural(list.length, 'account', 'accounts')} · spend and tokens add up, quota does not`,
    spend: {
      label: 'Spend, every account',
      value: spend.text,
      sub: `across ${plural(g.accounts, 'account', 'accounts')}, at API rates`,
      tone: 'plain',
      marks: spend.marks,
    },
    tokens: { label: 'Tokens read', value: compact(g.prompt_tokens), sub: `${compact(g.output_tokens)} written back`, tone: 'plain' },
    requests: { label: 'Requests', value: count(g.requests), sub: 'in the selected range', tone: 'plain' },
    // A pace is only meaningful against a fixed horizon: the last seven days scaled to thirty,
    // whatever range the page shows (OV-C91).
    // Its week's unpriced and estimated requests mark it as they mark any cost sum (OV-C229, OV-C232).
    pace: { label: '30-day pace', value: paced.text, sub: `${compact(pace.prompt_tokens)} tokens · from ${pace.basis || 'recent spend'}`, tone: 'plain', marks: paced.marks },
    cards,
  };
}
