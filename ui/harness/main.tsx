// Copied from Northstar ui/harness/main.tsx at e9c1589.
// Changed: one fixture, the router's, and an EventSource fake beside the fetch; `?unread=` names the router's reads that fail, and `?router=hosted` makes it a hosted router hearing from several machines.
import { asked } from './asked';
import { fakeEventSource } from './eventSource';
import { fixtureServer, UNREAD, type Unread } from './fixtureFetch';
import { STREAMS } from './fixtures';
import type { HarnessRouter } from './sessions';

/// The console on fixtures: `pnpm harness`, then http://127.0.0.1:4392/.
///
/// This replaces `window.fetch` and `window.EventSource` and then loads the console's own
/// entry, `src/main.tsx`, unchanged, so what renders is the real console, every screen and the
/// shell around it, and only its answers are invented. The entry is imported after the
/// replacement rather than beside it, because a static import would run first, and the first
/// read would go to a router that is not there.
///
/// `?unread=health,projects` for reads that could not be made, and `?router=hosted` for a hosted
/// router, whose sessions come from several machines (harness/sessions.ts). Kept for the tab, so a
/// reload or a link followed inside the console keeps them, and taken out of the address, so no
/// screen reads them as its own parameters.

/// What `scripts/shoot.mjs` and its checks read back: what was asked, what could not be answered,
/// and which router the harness stands for.
declare global {
  interface Window {
    __harness?: { readonly unread: readonly Unread[]; readonly router: HarnessRouter; readonly answered: readonly string[]; readonly unanswered: readonly string[] };
  }
}

const KEY = 'agent-router.harness';

function chosen(): { unread: Unread[]; router: HarnessRouter } {
  const url = new URL(location.href);
  let kept: { unread?: Unread[]; router?: HarnessRouter } = {};
  try {
    kept = JSON.parse(sessionStorage.getItem(KEY) ?? '{}') as typeof kept;
  } catch {
    /* not kept; the address and the default decide */
  }
  const asked = url.searchParams.get('unread');
  const unread = asked === null ? (kept.unread ?? []) : asked.split(',').filter((u): u is Unread => (UNREAD as readonly string[]).includes(u));
  const named = url.searchParams.get('router') ?? kept.router;
  const router: HarnessRouter = named === 'hosted' ? 'hosted' : 'local';
  try {
    sessionStorage.setItem(KEY, JSON.stringify({ unread, router }));
  } catch {
    /* a private window: this load still uses it */
  }
  url.searchParams.delete('unread');
  url.searchParams.delete('router');
  history.replaceState(history.state, '', url);
  return { unread, router };
}

const { unread, router } = chosen();
const record = asked();
// The Sessions screen's reads and streams (harness/sessions.ts), its reads answered before the rest,
// and a hosted router's machines (harness/machines.ts) before those.
const sessions = await import('./sessions');
const machines = await import('./machines');
window.fetch = machines.withMachines(sessions.withSessions(fixtureServer({ unread, asked: record }).fetch, record, router), record, router);
window.EventSource = fakeEventSource({ ...STREAMS, ...sessions.STREAMS }, { asked: record }).EventSource;
window.__harness = { unread, router, answered: record.answered, unanswered: record.unanswered };

await import('~/main');
