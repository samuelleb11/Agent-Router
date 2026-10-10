// The status bar's figures, under plain Node: `npm test`. What the router answers for
// `/api/summary?parts=providers` goes in; the bar's text, its tooltip's sections and its
// urgency come out (AGEN-0138).

import { test } from 'node:test';
import * as assert from 'node:assert/strict';
import { barText, blocksOf, severityOf, tooltipSections, type ProviderQuota } from '../providers';

const anthropic: ProviderQuota = {
  provider: 'anthropic',
  account: { id: 'work', label: 'work', email: 'me@example.com' },
  windows: { five_hour: { utilization: 0.48, resets_at: 1_791_660_000 }, seven_day: { utilization: 0.81, resets_at: 1_792_000_000 } },
  status: 'allowed',
};
const kimi: ProviderQuota = {
  provider: 'kimi',
  account: { id: 'kimi', label: 'kimi' },
  windows: { five_hour: { utilization: 0.18, resets_at: 1_791_660_000 }, month: { utilization: 0.27, resets_at: 1_793_000_000 } },
  status: 'allowed',
};

test('the bar shows each provider by its short name, its own windows after it', () => {
  assert.equal(barText([anthropic, kimi]), 'Claude 5h 48% · 7d 81%  Kimi 5h 18% · mo 27%');
});

test('a router with one provider shows that one alone, and a window not read yet is a dash', () => {
  assert.equal(barText([kimi]), 'Kimi 5h 18% · mo 27%');
  const unread: ProviderQuota = { provider: 'anthropic', windows: { five_hour: null, seven_day: { utilization: null } } };
  assert.equal(barText([unread]), 'Claude 5h — · 7d —');
});

test('the tooltip gives each provider its name, serving account and windows with their resets', () => {
  const sections = tooltipSections([anthropic, kimi], (s) => `at ${s}`);
  assert.deepEqual(sections, [
    {
      provider: 'anthropic',
      title: 'Anthropic — me@example.com',
      lines: ['5-hour window: 48%, resets at 1791660000', '7-day window: 81%, resets at 1792000000'],
    },
    {
      provider: 'kimi',
      title: 'Kimi Code — kimi',
      lines: ['5-hour window: 18%, resets at 1791660000', 'Month window: 27%, resets at 1793000000'],
    },
  ]);
  const spent = tooltipSections([{ ...kimi, status: 'rejected' }], String)[0];
  assert.match(spent.lines.at(-1) ?? '', /spent until it resets/);
});

test('the bar is as urgent as its most urgent provider', () => {
  assert.equal(severityOf([anthropic, kimi]), 'warning', "Anthropic's 7-day at 81%");
  assert.equal(severityOf([kimi]), undefined);
  assert.equal(severityOf([{ ...kimi, status: 'rejected' }]), 'critical');
  assert.equal(severityOf([{ ...kimi, windows: { month: { utilization: 1 } } }]), 'critical');
});

test('an older router that refuses the part gives no blocks, and the bar falls back', () => {
  assert.deepEqual(blocksOf({ ok: false, error: 'the summary has no part "providers"' }), []);
  assert.deepEqual(blocksOf(null), []);
  assert.equal(blocksOf({ providers: [anthropic, { nope: 1 }] }).length, 1);
});
