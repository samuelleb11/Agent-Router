// @vitest-environment node
// New for agent-router: the forecast's contracts beyond what the old checks pinned, among them the
// ones the dashboard broke (docs/web-console-contracts.md, "Forecast" and "All accounts").
import { describe, expect, it } from 'vitest';
import forecastText from './fixtures/forecast.json?raw';
import { windows } from './fixtures/dashboard';
import { allAccounts, burnDown, burnDownLayout, fcBasis, fcState, meter, windowMeter, type Forecast, type ForecastAccount, type Window } from './forecast';
import { when } from './format';

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
    const tight = allAccounts(FORECAST, NOW);
    expect(tight?.headroom).toEqual({
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
    expect(allAccounts(crossing as Forecast, NOW)?.headroom).toMatchObject({ sub: "first to go: secondary@example.test's 7-day window, in 3d 20h", tone: 'bad' });
  });

  it('OV-C88, OV-C89, OV-C90, OV-C91: five tiles, quota pools counted rather than credentials, and the pace over a fixed horizon', () => {
    const panel = allAccounts({ ...FORECAST, global: { ...FORECAST.global, combined_headroom: { active_hours_left: 3.247, pools: 1, first_to_exhaust: null } } }, NOW);
    expect(panel?.caption).toBe('3 accounts · spend and tokens add up, quota does not');
    expect([panel?.spend.label, panel?.tokens.label, panel?.requests.label, panel?.headroom.label, panel?.pace.label]).toEqual([
      'Spend, every account',
      'Tokens read',
      'Requests',
      'Headroom left',
      '30-day pace',
    ]);
    expect(panel?.headroom).toMatchObject({ value: '3.2 h', sub: '1 quota pool, none projected to run out', tone: 'plain' });
    expect(panel?.pace.sub).toBe('20.67B tokens · from the last 7 days of recorded spend');
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

  it('OV-C221: an account the router could not price reads "not priced", never "$0", and Spend says how many requests it leaves out', () => {
    expect(allAccounts(only(KIMI), NOW)?.cards[0]?.cost).toBe('not priced');
    expect(allAccounts(only({ ...KIMI, usage: { ...KIMI.usage, cost: 1.5, unpriced: 3 } }), NOW)?.cards[0]?.cost).toBe('$1.50 + 3 not priced');
    const mixed = allAccounts({ ...FORECAST, accounts: [...(FORECAST.accounts ?? []), KIMI] }, NOW);
    expect(mixed?.spend.sub).toBe('across 3 accounts, at API rates · leaves out 12 requests the router could not price');
    expect(mixed?.spend.value, 'what was priced is still the total').not.toBe('—');
    expect(mixed?.cards.find((c) => c.id === 'primary')?.cost, 'a priced account as before').toBe('$699.48');
    // Nothing priced at all: no total, rather than $0.
    expect(allAccounts({ ...only(KIMI), global: { ...FORECAST.global, cost: 0, accounts: 1 } }, NOW)?.spend).toMatchObject({ value: '—' });
  });
});
