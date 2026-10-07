// New for agent-router: what the fixture harness answers for the Machines screen, in the router's own
// shape (src/machines.rs, `Seen`). Invented, every one. Only a hosted router lists machines, so only
// `?router=hosted` answers /api/machines: the machines its sessions come from (harness/sessions.ts),
// two online and one gone offline, one on the harness's own build and the others not, and a Windows
// one whose label is long enough to be cut short on a phone (MA-C3, MA-C4).
//
// Registered in harness/main.tsx, after the Sessions screen's reads.
import type { Asked } from './asked';
import { HEALTH } from './fixtures';
import { LAPTOP, STUDIO, WINDOWS, type HarnessRouter } from './sessions';
import type { MachineRow } from '~/model/machine';

const MIN = 60_000;
const HOUR = 60 * MIN;

/// Now, to the minute, when the page loads: every "online for" is measured on the reader's clock.
const NOW = Math.floor(Date.now() / MIN) * MIN;

/// A build that is not the harness router's own.
const OLDER = 'tree:0aa1bb2cc3dd4ee5ff6a7b8c9d0e1f2a3b4c5d6e';

/// In the router's order, connected first and then by label, as `/api/machines` lists them.
export const MACHINES: readonly MachineRow[] = [
  { ...LAPTOP, platform: 'linux', version: '0.9.0', build: OLDER, online: true, since_ms: NOW - 25 * MIN },
  { ...STUDIO, platform: 'macos', version: '0.9.0', build: HEALTH.build, online: true, since_ms: NOW - 3 * HOUR },
  { ...WINDOWS, platform: 'windows', version: '0.9.0', build: OLDER, online: false, since_ms: NOW - 2 * 24 * HOUR },
];

const json = (payload: unknown): Response => new Response(JSON.stringify(payload), { status: 200, headers: { 'Content-Type': 'application/json' } });

/// The answer to the Machines screen's read, or null: a local router lists none, and leaves the
/// path to the harness's own answer for a read it does not hold.
export function machinesRead(path: string, query: URLSearchParams, router: HarnessRouter = 'local'): Response | null {
  if (router === 'hosted' && path === '/api/machines' && [...query.keys()].length === 0) return json(MACHINES);
  return null;
}

/// The harness's fetch with the Machines screen's read answered first, as `router` would answer it,
/// and kept in what the harness was asked; everything else goes on to `next`.
export function withMachines(next: typeof fetch, asked: Asked, router: HarnessRouter = 'local'): typeof fetch {
  return async (input: RequestInfo | URL, init?: RequestInit) => {
    const href = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
    const url = new URL(href, globalThis.location?.href);
    const method = (init?.method ?? (input instanceof Request ? input.method : 'GET')).toUpperCase();
    const got = method === 'GET' ? machinesRead(url.pathname, url.searchParams, router) : null;
    if (!got) return next(input, init);
    asked.answered.push(`${method} ${url.pathname}${url.search}`);
    return got;
  };
}
