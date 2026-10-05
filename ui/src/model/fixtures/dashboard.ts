// New for agent-router: the fixtures scripts/check-dashboard-ui.js builds inline, as values built
// around a clock the spec chooses rather than the moment the script loaded.

import type { Window } from '../forecast';
import type { SearchReply } from '../search';
import type { Auth } from '../settings';

const secs = (ms: number): number => (ms / 1000) | 0;

/// Every state a window can be in, including the ones with nothing to say.
export function windows(now: number): Record<string, Window> {
  return {
    'runs out before the reset': {
      used: 0.98, reset_at: secs(now + 30 * 3600e3), window_start: secs(now - 100 * 3600e3),
      burn_per_active_hour: 0.0121, active_hours_left: 1.65, duty_fraction: 0.139,
      exhausts_at: now + 12 * 3600e3, resets_first: false, projected_end_util: 1,
      basis: 'this-window', measured_active_hours: 7.4, readings: 2639, expired: false,
      series: [{ t: now - 90 * 3600e3, u: 0.89 }, { t: now - 20 * 3600e3, u: 0.98 }],
    },
    'resets before it fills': {
      used: 0.06, reset_at: secs(now + 3.8 * 3600e3), window_start: secs(now - 1.2 * 3600e3),
      burn_per_active_hour: 0.0536, active_hours_left: 17.5, duty_fraction: 0.955,
      exhausts_at: null, resets_first: true, projected_end_util: 0.26,
      basis: 'this-window', measured_active_hours: 1.1, readings: 273, expired: false,
      series: [{ t: now - 3600e3, u: 0.01 }, { t: now - 600e3, u: 0.06 }],
    },
    'too young to have a slope': {
      used: 0.01, reset_at: secs(now + 4.9 * 3600e3), window_start: secs(now - 360e3),
      burn_per_active_hour: 0.05, active_hours_left: 19.8, duty_fraction: 1,
      exhausts_at: null, resets_first: true, projected_end_util: 0.25,
      basis: 'trailing-windows', measured_active_hours: 0, readings: 12, expired: false, series: [],
    },
    'nothing to go on': {
      used: 0, reset_at: secs(now + 5 * 3600e3), window_start: secs(now),
      burn_per_active_hour: 0, active_hours_left: null, duty_fraction: 0.01,
      exhausts_at: null, resets_first: true, projected_end_util: 0,
      basis: 'none', measured_active_hours: 0, readings: 1, expired: false, series: [],
    },
    'already at the limit': {
      used: 1, reset_at: secs(now + 2 * 3600e3), window_start: secs(now - 100 * 3600e3),
      burn_per_active_hour: 0.012, active_hours_left: 0, duty_fraction: 0.2,
      exhausts_at: now, resets_first: false, projected_end_util: 1,
      basis: 'this-window', measured_active_hours: 5, readings: 900, expired: false,
      series: [{ t: now - 50 * 3600e3, u: 0.9 }],
    },
    // The state the band exists to express: the central estimate crosses, but a quieter week does
    // not. Amber, and worded as the coin flip it is.
    'too close to call': {
      used: 0.49, reset_at: secs(now + 95 * 3600e3), window_start: secs(now - 73 * 3600e3),
      burn_per_active_hour: 0.0167, active_hours_left: 30.3, duty_fraction: 0.329,
      duty: { low: 0.248, mid: 0.329, high: 0.409 }, verdict: 'tight',
      exhausts_at: now + 93 * 3600e3, exhausts_at_early: now + 74 * 3600e3, exhausts_at_late: null,
      resets_first: false, projected_end_util: 1, projected_end_util_low: 0.883, projected_end_util_high: 1,
      basis: 'this-window', measured_active_hours: 29.1, readings: 7935, expired: false,
      series: [{ t: now - 70 * 3600e3, u: 0.01 }, { t: now - 10 * 3600e3, u: 0.41 }],
    },
    'rolled over while we were not looking': { expired: true, reset_at: secs(now - 26 * 3600e3), seen_at: now - 30 * 3600e3 },
  };
}

/// A search reply, as the router spells it. The marker pair is what a snippet splits on, and the
/// second hit carries a stray opener to prove that it cannot break out.
export function search(now: number): SearchReply {
  return {
    query: 'kestrel',
    expression: 'kestrel',
    sessions: [
      { session_id: 'aaaaaaaa-1111-2222-3333-444444444444', project: '/code/agent-router', session_label: null, git_branch: 'dev', blocks: 2, hits: 3, first_at: now - 6e5, last_at: now - 1e4 },
    ],
    hits: [
      { block_id: 1, kind: 'tool_use', chars: 40, role: 'assistant', ordinal: 4, request_id: 9, at: now - 1e4, session_id: 'aaaaaaaa-1111-2222-3333-444444444444',
        project: '/code/agent-router', session_label: null, snippet: 'where does the ‹kestrel› roost', rank: -8 },
      { block_id: 2, kind: 'text', chars: 20, role: 'user', ordinal: 0, request_id: 10, at: now - 6e5, session_id: 'bbbbbbbb-1111-2222-3333-444444444444',
        project: null, session_label: null, snippet: 'a stray ‹marker and no close', rank: -7 },
    ],
  };
}

/// A hosted router's `/api/auth` (docs/hosting-on-kubernetes.md, "Web UI in server mode").
export const hosted = (now: number): Auth => ({
  configured: true, updated_at: now - 3600e3, from_loopback: false, remote_bind: true,
  listening_on: '0.0.0.0:8787', lan_url: 'https://router.example', env_override: false,
  mode: 'server', password_pinned: true, public_url: 'https://router.example',
});

/// A local router's, as it has always been: no `mode` at all, which has to read as local.
export const local = (now: number): Auth => ({
  configured: true, updated_at: now - 60e3, from_loopback: true, remote_bind: false,
  listening_on: '127.0.0.1:8787', lan_url: 'http://10.30.0.183:8787', env_override: false,
});
