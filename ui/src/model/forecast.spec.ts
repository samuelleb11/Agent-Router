// @vitest-environment node
// New for agent-router: the forecast's contracts beyond what the old checks pinned, among them the
// ones the dashboard broke (docs/web-console-contracts.md, "Forecast" and "All accounts").
import { describe, expect, it } from 'vitest';
import forecastText from './fixtures/forecast.json?raw';
import { windows } from './fixtures/dashboard';
import { allAccounts, burnDown, burnDownLayout, burnDowns, fcBasis, fcState, meter, providerHeadroom, windowLabel, windowMeter, windowName, type Forecast, type ForecastAccount, type Headroom, type Window } from './forecast';
import { ESTIMATED_WHY, UNPRICED_WHY, usd, when } from './format';

const NOW = Date.UTC(2026, 9, 5, 12, 0, 0);
const W = windows(NOW);
const win = (name: string): Window => {
  const w = W[name];
  if (!w) throw new Error(`no window called ${name}`);
  return w;
};
const FORECAST = JSON.parse(forecastText) as Forecast;

const HOUR = 3600e3;
/// A Kimi Code account as `/api/forecast` sends it: its windows read from its usage API, a 5-hour and
/// a month, `seen_at` the reading, and every request one the router could not price.
const KIMI: ForecastAccount = {
  id: 'kimi',
  label: 'Kimi',
  provider: 'kimi',
  active: 1,
  plan: null,
  usage: { requests: 12, prompt_tokens: 40_000, output_tokens: 900, cost: 0, unpriced: 12, last_seen: NOW - 5 * 60e3 },
  seen_at: NOW - 2 * 60e3,
  pool: null,
  five_hour: { used: 0.3, reset_at: (NOW + 2 * HOUR) / 1000, expired: false, basis: 'none', series: [], spent: false },
  seven_day: null,
  month: { used: 0.12, reset_at: (NOW + 20 * 24 * HOUR) / 1000, expired: false, basis: 'none', series: [], spent: false },
};
const only = (...accounts: ForecastAccount[]): Forecast => ({ ...FORECAST, accounts });
/// The captured payload predates providers: its headroom, as the router now gives it, Anthropic's own.
const ANTHROPIC: Headroom = { active_hours_left: 3.247111236383442, pools: 1, first_to_exhaust: { account_id: 'secondary', window: 'seven_day', at: 1_789_744_498_781, in_ms: 333_870_956 } };
const withRooms = (fc: Forecast, providers: Readonly<Record<string, Headroom>>): Forecast => ({ ...fc, global: { ...fc.global, providers } });

describe('forecast', () => {
  it('OV-C69: the words are the same on every client', () => {
    expect(fcState(win('runs out before the reset'), NOW)).toEqual({ tone: 'bad', text: '1.6 h of work left · runs out in 12h 00m, 18h 00m before it resets' });
    expect(fcState(win('too close to call'), NOW)).toEqual({
      tone: 'warn',
      text: '30 h of work left · too close to call — on this pace it runs out 2h 00m before reset, on a quieter week it ends ≈88%',
    });
    expect(fcState(win('resets before it fills'), NOW)).toEqual({ tone: 'plain', text: '18 h of work left · resets first, ending ≈26%' });
    expect(fcState({ ...win('resets before it fills'), projected_end_util: 0.93 }, NOW).tone, 'ending at 90 % or more is amber').toBe('warn');
  });

  it('OV-C70: a tight verdict whose central estimate does not cross never prints a crossing it was not given', () => {
    // The page subtracted nothing from the reset and printed "runs out ~20000d before reset".
    const central = { ...win('too close to call'), exhausts_at: null, projected_end_util: 0.97 };
    expect(fcState(central, NOW)).toEqual({
      tone: 'warn',
      text: '30 h of work left · too close to call — on this pace it ends ≈97%, on a busier week it runs out 21h 00m before reset',
    });
    expect(fcState({ ...central, exhausts_at_early: null }, NOW)).toEqual({ tone: 'warn', text: '30 h of work left · too close to call — on this pace it ends ≈97%' });
  });

  it('OV-C72, OV-C73: an expired window and one with nothing to project from say so plainly, with no basis', () => {
    const expired = win('rolled over while we were not looking');
    expect(fcState(expired, NOW)).toEqual({ tone: 'plain', text: 'window rolled over 1 d ago · nothing seen since' });
    expect([fcBasis(expired), burnDown(expired, 300, NOW)]).toEqual([null, null]);
    expect(fcState(win('nothing to go on'), NOW)).toEqual({ tone: 'plain', text: 'not enough burn yet to project' });
  });

  it('OV-C74: a window already at 100 % says it has run out, in red, whatever basis its burn came from', () => {
    // The page tested the basis first, and read "not enough burn yet to project" under a full meter.
    const full = { ...win('already at the limit'), basis: 'none', burn_per_active_hour: 0 };
    expect(fcState(full, NOW)).toEqual({ tone: 'bad', text: '0 min of work left · runs out now, 2h 00m before it resets' });
    expect(fcState({ ...full, active_hours_left: null, exhausts_at: null }, NOW).tone).toBe('bad');
  });

  it('OV-C82: the projection is amber for a tight verdict, its central estimate crossing or not', () => {
    expect(burnDown(win('too close to call'), 300, NOW)?.projection.tone).toBe('warn');
    // The page drew it muted once the central estimate stopped crossing.
    expect(burnDown({ ...win('too close to call'), exhausts_at: null }, 300, NOW)?.projection.tone).toBe('warn');
    expect(burnDown(win('runs out before the reset'), 300, NOW)?.projection.tone).toBe('bad');
    expect(burnDown(win('resets before it fills'), 300, NOW)?.projection.tone).toBe('muted');
  });

  it('OV-C80, OV-C84: the burn-down runs from the window opening to its reset, holding values outside it to its edges', () => {
    const chart = burnDown({ ...win('resets before it fills'), series: [{ t: NOW - 100 * 3600e3, u: 1.7 }] }, 300, NOW);
    expect(chart?.observed[0]).toEqual([chart?.left, chart?.top]);
    expect(chart?.resets).toMatch(/^resets /);
  });

  it('OV-C85: two burn-downs sit side by side only when both fit at 250 px', () => {
    expect([burnDownLayout(620, 2), burnDownLayout(500, 2), burnDownLayout(620, 1)]).toEqual([
      { columns: 2, width: 299 },
      { columns: 1, width: 500 },
      { columns: 1, width: 620 },
    ]);
  });

  it('OV-C57, OV-C58, OV-C94: a meter is amber from 70 % and red from 90 %, and a window with no reading shows "—", never 0% used', () => {
    expect(meter(0.75, NOW + 3600e3, NOW)).toEqual({ text: '75% used · resets in 1h 00m', fill: 75, level: 'warn' });
    expect([meter(0.95, null, NOW).level, meter(0.5, null, NOW).level, meter(1.4, null, NOW).fill]).toEqual(['bad', 'normal', 100]);
    expect(meter(null, null, NOW)).toEqual({ text: '—', fill: 0, level: 'none' });
    expect(windowMeter({ reset_at: 10 }, NOW).text, 'a window never read').toBe('—');
    const primary = allAccounts(FORECAST, NOW)?.cards.find((c) => c.id === 'primary');
    expect(primary?.windows.map((w) => w.meter.text), 'windows rolled over').toEqual(['—', '—']);
  });

  it('OV-C92, OV-C93: Headroom is red only for a crossing and names the first account to run out by its label, window and time', () => {
    // The bundled payload's first to go is a tight 7-day window, which the page painted red under
    // the router's id.
    const tight = providerHeadroom(withRooms(FORECAST, { anthropic: ANTHROPIC }), 'anthropic', NOW);
    expect(tight?.tile).toEqual({
      label: 'Headroom left',
      value: '3.2 h',
      sub: "too close to call: secondary@example.test's 7-day window could run out in 3d 20h",
      tone: 'warn',
    });
    expect(tight?.first).toMatchObject({ name: 'secondary@example.test', window: '7-day', at: 1_789_744_498_781, verdict: 'tight' });
    const crossing = structuredClone(FORECAST) as { accounts: { id: string; seven_day: { verdict: string } }[] };
    const secondary = crossing.accounts.find((a) => a.id === 'secondary');
    if (!secondary) throw new Error('the fixture lost its serving account');
    secondary.seven_day.verdict = 'crosses';
    expect(providerHeadroom(withRooms(crossing as Forecast, { anthropic: ANTHROPIC }), 'anthropic', NOW)?.tile).toMatchObject({ sub: "first to go: secondary@example.test's 7-day window, in 3d 20h", tone: 'bad' });
  });

  it('OV-C88, OV-C89, OV-C90, OV-C91: four tiles that add up, quota pools counted rather than credentials, and the pace over a fixed horizon', () => {
    const panel = allAccounts(FORECAST, NOW);
    expect(panel?.caption).toBe('3 accounts · spend and tokens add up, quota does not');
    // Headroom is no tile of every account's: it is each provider's own, in its quota block (OV-C225).
    expect([panel?.spend.label, panel?.tokens.label, panel?.requests.label, panel?.pace.label]).toEqual(['Spend, every account', 'Tokens read', 'Requests', '30-day pace']);
    expect(panel).not.toHaveProperty('headroom');
    const room = providerHeadroom(withRooms(FORECAST, { anthropic: { active_hours_left: 3.247, pools: 1, first_to_exhaust: null } }), 'anthropic', NOW);
    expect(room).toEqual({ tile: { label: 'Headroom left', value: '3.2 h', sub: '1 quota pool, none projected to run out', tone: 'plain' }, first: null });
    expect(panel?.pace.sub).toBe('20.67B tokens · from the last 7 days of recorded spend');
  });

  it('OV-C225: headroom and the first to run out are each provider’s own, and no figure sets one provider’s windows against another’s', () => {
    // Kimi Code's month runs out an hour and twenty minutes before Anthropic's 5-hour window: each
    // block names its own first to run out, never the other's.
    const kimi: ForecastAccount = {
      ...KIMI,
      five_hour: { used: 0.3, reset_at: (NOW + 2 * HOUR) / 1000, expired: false, spent: false, basis: 'this-window', active_hours_left: 4, burn_per_active_hour: 0.1, projected_end_util: 0.6, verdict: 'resets-first' },
      month: { used: 0.97, reset_at: (NOW + 20 * 24 * HOUR) / 1000, expired: false, spent: false, basis: 'this-window', active_hours_left: 0.5, burn_per_active_hour: 0.05, exhausts_at: NOW + 40 * 60e3, verdict: 'crosses' },
    };
    const work: ForecastAccount = { id: 'work', label: 'work', provider: 'anthropic', five_hour: { used: 0.7, reset_at: (NOW + 3 * HOUR) / 1000, active_hours_left: 3, exhausts_at: NOW + 2 * HOUR, verdict: 'crosses' } };
    const rooms: Record<string, Headroom> = {
      anthropic: { active_hours_left: 3, pools: 1, first_to_exhaust: { account_id: 'work', window: 'five_hour', at: NOW + 2 * HOUR, in_ms: 2 * HOUR } },
      kimi: { active_hours_left: 0.5, pools: 1, first_to_exhaust: { account_id: 'kimi', window: 'month', at: NOW + 40 * 60e3, in_ms: 40 * 60e3 } },
    };
    const fc: Forecast = { accounts: [work, kimi], global: { providers: rooms } };
    const anthropic = providerHeadroom(fc, 'anthropic', NOW);
    const own = providerHeadroom(fc, 'kimi', NOW);
    expect(anthropic?.tile).toMatchObject({ value: '3.0 h', sub: "first to go: work's 5-hour window, in 2h 00m", tone: 'bad' });
    expect(anthropic?.first).toMatchObject({ name: 'work', window: '5-hour', at: NOW + 2 * HOUR, verdict: 'crosses' });
    // Kimi Code's month counts in its own headroom, named by its own key.
    expect(own?.tile).toMatchObject({ value: '30 min', sub: "first to go: Kimi's month window, in 40 min", tone: 'bad' });
    expect(own?.first).toMatchObject({ name: 'Kimi', window: 'month', at: NOW + 40 * 60e3, verdict: 'crosses' });
    // Neither figure names the other provider's account or window.
    expect(JSON.stringify(anthropic)).not.toMatch(/Kimi|month/);
    expect(JSON.stringify(own)).not.toMatch(/work|5-hour|7-day/);
    // A provider the router has no headroom for has none here: nothing reads "no account" for it.
    expect(providerHeadroom({ ...fc, global: { providers: { kimi: rooms['kimi']! } } }, 'anthropic', NOW)).toBeNull();
    expect(providerHeadroom(null, 'kimi', NOW)).toBeNull();
    // All accounts totals no headroom across the two providers.
    const all = allAccounts(fc, NOW);
    expect(all).not.toHaveProperty('headroom');
    expect(JSON.stringify(all)).not.toMatch(/Headroom/);
  });

  it('OV-C224: a window is named by its own key, and a Kimi Code month gets its burn-down by that name', () => {
    expect((['five_hour', 'seven_day', 'month'] as const).map((k) => [windowName(k), windowLabel(k)])).toEqual([
      ['5-hour', '5-hour window'],
      ['7-day', '7-day window'],
      ['month', 'Month window'],
    ]);
    const month: Window = { used: 0.27, reset_at: (NOW + 20 * 24 * HOUR) / 1000, window_start: (NOW - 10 * 24 * HOUR) / 1000, expired: false };
    expect(burnDowns({ ...KIMI, month }).map((b) => [b.key, b.label])).toEqual([['month', 'Month window']]);
  });

  it('OV-C218: a Kimi Code account’s card names Kimi Code and shows its two windows, the second under its real name, a month or a week', () => {
    const card = allAccounts(only(KIMI), NOW)?.cards[0];
    // Kimi Code is named by the provider's tag beside the account's name, as every card's provider is
    // (OV-C231), and its line does not say it again.
    expect(card?.provider).toBe('kimi');
    expect(card?.sub).not.toMatch(/Kimi Code/);
    expect(card?.windows.map((w) => [w.key, w.label, w.meter.text])).toEqual([
      ['five_hour', '5-hour', '30% used · resets in 2h 00m'],
      ['month', 'Month', '12% used · resets in 20d 0h'],
    ]);
    // A legacy plan's second window is a week, and reads as one.
    const legacy = allAccounts(only({ ...KIMI, month: null, seven_day: { used: 0.4, reset_at: (NOW + 3 * 24 * HOUR) / 1000, expired: false, basis: 'none', spent: false } }), NOW)?.cards[0];
    expect(legacy?.windows.map((w) => [w.key, w.label])).toEqual([
      ['five_hour', '5-hour'],
      ['seven_day', '7-day'],
    ]);
    // An Anthropic account is drawn as it always was.
    expect(allAccounts(FORECAST, NOW)?.cards[1]?.windows.map((w) => w.label)).toEqual(['5-hour', '7-day']);
  });

  it('OV-C219: a spent window says it is spent until its reset, in red, with nothing projected over it', () => {
    const reset = NOW + 2 * HOUR;
    const spent: Window = { used: 1, reset_at: reset / 1000, expired: false, spent: true, basis: 'this-window', burn_per_active_hour: 0.2, active_hours_left: 0, exhausts_at: NOW, measured_active_hours: 1 };
    expect(fcState(spent, NOW)).toEqual({ tone: 'bad', text: `spent until it resets in 2h 00m, at ${when(reset)}` });
    expect(fcBasis(spent)).toBeNull();
    const card = allAccounts(only({ ...KIMI, status: 'rejected', five_hour: spent }), NOW)?.cards[0];
    expect(card?.windows[0]?.meter).toMatchObject({ text: '100% used · resets in 2h 00m', level: 'bad' });
    expect(card?.windows[0]?.line).toEqual({ tone: 'bad', text: `spent until it resets in 2h 00m, at ${when(reset)}`, basis: null });
  });

  it('OV-C220: an account that served requests is never "never seen", and a Kimi Code account’s usage reading reads as one', () => {
    const sub = (a: ForecastAccount) => allAccounts(only(a), NOW)?.cards[0]?.sub;
    expect(sub(KIMI)).toBe('last request 5 min ago · usage read 2 min ago');
    expect(sub({ ...KIMI, seen_at: null })).toBe('last request 5 min ago');
    expect(sub({ ...KIMI, seen_at: null, usage: { ...KIMI.usage, last_seen: null } })).toBe('never seen');
    // An Anthropic account with no rate-limit reading still served its requests.
    const claude: ForecastAccount = { id: 'work', label: 'work', plan: 'max', seen_at: null, usage: { requests: 3, last_seen: NOW - 3 * 60e3 } };
    expect(sub(claude)).toBe('max · last request 3 min ago');
    expect(sub({ ...claude, seen_at: NOW - 60 * 60e3 })).toBe('max · last request 1 h ago');
    expect(sub({ ...claude, usage: { requests: 0, last_seen: null } })).toBe('max · never seen');
  });

  it('OV-C221, OV-C229: an account the router could not price reads "not priced", never "$0"; one partly priced, and Spend, read their priced part marked with how many they leave out', () => {
    expect(allAccounts(only(KIMI), NOW)?.cards[0]).toMatchObject({ cost: 'not priced', costMarks: [{ kind: 'unpriced', value: '12 unpriced', label: UNPRICED_WHY }] });
    expect(allAccounts(only({ ...KIMI, usage: { ...KIMI.usage, cost: 1.5, unpriced: 3 } }), NOW)?.cards[0]?.cost).toBe('$1.50 + 3 unpriced');
    const mixed = allAccounts({ ...FORECAST, accounts: [...(FORECAST.accounts ?? []), KIMI] }, NOW);
    // The mark is on the figure, its reason in the figure's tooltip, so the line under it says no more.
    expect(mixed?.spend).toMatchObject({ value: `${usd(FORECAST.global?.cost)} + 12 unpriced`, sub: 'across 3 accounts, at API rates', marks: [{ kind: 'unpriced', value: '12 unpriced', label: UNPRICED_WHY }] });
    expect(mixed?.cards.find((c) => c.id === 'primary'), 'a priced account as before').toMatchObject({ cost: '$699.48', costMarks: [] });
    // Nothing priced at all: "not priced", never $0.
    expect(allAccounts({ ...only(KIMI), global: { ...FORECAST.global, cost: 0, accounts: 1 } }, NOW)?.spend).toMatchObject({ value: 'not priced' });
  });

  it('OV-C232: an account, and Spend, whose cost rests on estimated tokens read "≈" before it, the reason in its tooltip', () => {
    const estimated: ForecastAccount = { ...KIMI, usage: { ...KIMI.usage, cost: 1.5, unpriced: 0, estimated: 4 } };
    expect(allAccounts(only(estimated), NOW)?.cards[0]).toMatchObject({ cost: '≈$1.50', costMarks: [{ kind: 'estimated', value: '4 estimated', label: ESTIMATED_WHY }] });
    expect(allAccounts({ ...only(estimated), global: { ...FORECAST.global, cost: 1.5, accounts: 1 } }, NOW)?.spend).toMatchObject({ value: '≈$1.50', marks: [{ kind: 'estimated', value: '4 estimated' }] });
  });

  it('OV-C229, OV-C232: the 30-day pace is its priced part, marked with the week’s unpriced requests and "≈" for its estimated ones, each with its reason', () => {
    type Pace = NonNullable<NonNullable<Forecast['global']>['pace_30d']>;
    const paced = (over: Pace) => allAccounts({ ...FORECAST, global: { ...FORECAST.global, pace_30d: { ...FORECAST.global?.pace_30d, ...over } } }, NOW)?.pace;
    expect(paced({})).toMatchObject({ value: usd(FORECAST.global?.pace_30d?.cost), marks: [] });
    expect(paced({ cost: 120, unpriced: 2, estimated: 5 })).toMatchObject({
      value: '≈$120.00 + 2 unpriced',
      marks: [
        { kind: 'unpriced', value: '2 unpriced', label: UNPRICED_WHY },
        { kind: 'estimated', value: '5 estimated', label: ESTIMATED_WHY },
      ],
    });
    expect(paced({ cost: 0, unpriced: 2 })?.value).toBe('not priced');
  });
});
