// Copied from Northstar ui/harness/main.tsx at e9c1589.
// Changed: one fixture, the router's, and an EventSource fake beside the fetch; `?unread=` names the router's reads that fail.
import { asked } from './asked';
import { fakeEventSource } from './eventSource';
import { fixtureServer, UNREAD, type Unread } from './fixtureFetch';
import { STREAMS } from './fixtures';

/// The console on fixtures: `pnpm harness`, then http://127.0.0.1:4392/.
///
/// This replaces `window.fetch` and `window.EventSource` and then loads the console's own
/// entry, `src/main.tsx`, unchanged, so what renders is the real console, every screen and the
/// shell around it, and only its answers are invented. The entry is imported after the
/// replacement rather than beside it, because a static import would run first, and the first
/// read would go to a router that is not there.
///
/// `?unread=health,projects` for reads that could not be made. Kept for the tab, so a reload or
/// a link followed inside the console keeps it, and taken out of the address, so no screen
/// reads it as its own parameter.

/// What `scripts/shoot.mjs` reads back: what was asked, and what could not be answered.
declare global {
  interface Window {
    __harness?: { readonly unread: readonly Unread[]; readonly answered: readonly string[]; readonly unanswered: readonly string[] };
  }
}

const KEY = 'agent-router.harness';

function chosen(): { unread: Unread[] } {
  const url = new URL(location.href);
  let kept: { unread?: Unread[] } = {};
  try {
    kept = JSON.parse(sessionStorage.getItem(KEY) ?? '{}') as typeof kept;
  } catch {
    /* not kept; the address and the default decide */
  }
  const asked = url.searchParams.get('unread');
  const unread = asked === null ? (kept.unread ?? []) : asked.split(',').filter((u): u is Unread => (UNREAD as readonly string[]).includes(u));
  try {
    sessionStorage.setItem(KEY, JSON.stringify({ unread }));
  } catch {
    /* a private window: this load still uses it */
  }
  url.searchParams.delete('unread');
  history.replaceState(history.state, '', url);
  return { unread };
}

const { unread } = chosen();
const record = asked();
// The Sessions screen's reads and streams (harness/sessions.ts), its reads answered before the rest.
const sessions = await import('./sessions');
window.fetch = sessions.withSessions(fixtureServer({ unread, asked: record }).fetch, record);
window.EventSource = fakeEventSource({ ...STREAMS, ...sessions.STREAMS }, { asked: record }).EventSource;
window.__harness = { unread, answered: record.answered, unanswered: record.unanswered };

await import('~/main');
