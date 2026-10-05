// @vitest-environment node
// New for agent-router: the forecast's contracts beyond what the old checks pinned, among them the
// ones the dashboard broke (docs/web-console-contracts.md, "Forecast" and "All accounts").
import { describe, expect, it } from 'vitest';
import forecastText from './fixtures/forecast.json?raw';
import { windows } from './fixtures/dashboard';
import { allAccounts, burnDown, burnDownLayout, fcBasis, fcState, meter, windowMeter, type Forecast, type Window } from './forecast';

const NOW = Date.UTC(2026, 9, 5, 12, 0, 0);
const W = windows(NOW);
const win = (name: string): Window => {
  const w = W[name];
  if (!w) throw new Error(`no window called ${name}`);
  return w;
};
const FORECAST = JSON.parse(forecastText) as Forecast;

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
});
