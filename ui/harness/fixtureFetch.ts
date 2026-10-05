// Copied from Northstar ui/harness/fixtureFetch.ts at e9c1589.
// Changed: it answers the router's reads (its health and its projects so far) from harness/fixtures.ts, where Northstar's answers its own API from the Work map's fixtures.
import { HEALTH, PROJECTS } from './fixtures';
import { asked, type Asked } from './asked';

/// A stand-in for `window.fetch` that answers the console's reads from fixtures, so the real
/// console can be seen and probed with no router: not 127.0.0.1:8787, not a throwaway one.
///
/// **Only what the router would say.** Every answer is shaped as the router's handlers write
/// it, and typed by the console's own types for them (`~/data/router`).
///
/// **Loud where it cannot answer.** A read no fixture holds, a write, or a query parameter the
/// harness cannot honour is answered 501 with the request in the message, logged with
/// `console.error`, and kept in `unanswered`, which `scripts/shoot.mjs` reads and fails on.
///
/// **A read that could not be made** is a fetch that never got a response: it rejects with the
/// `TypeError` a dropped connection gives, which the console words as unreachable. Asked for
/// with `unread`.

/// The reads the harness can stand for being unable to make.
export type Unread = 'health' | 'projects';
export const UNREAD: readonly Unread[] = ['health', 'projects'];

export interface FixtureServer extends Asked {
  readonly fetch: typeof fetch;
}

export interface FixtureOptions {
  readonly unread?: readonly Unread[];
  /// Where anything outside the router's paths goes: the module graph, fonts, the icon.
  readonly passThrough?: typeof fetch;
  /// Shared with the event streams, so one list says everything that went unanswered.
  readonly asked?: Asked;
}

const json = (status: number, payload: unknown): Response =>
  new Response(JSON.stringify(payload), { status, headers: { 'Content-Type': 'application/json' } });

/// The router's own paths: everything else is the page's.
const ROUTER = /^\/(api|_router)(\/|$)/;

export function fixtureServer(options: FixtureOptions = {}): FixtureServer {
  const unread = new Set(options.unread ?? []);
  const passThrough = options.passThrough ?? globalThis.fetch.bind(globalThis);
  const { answered, unanswered } = options.asked ?? asked();

  /// The answer to one GET, `unread` for one that cannot be made, or null when no fixture holds one.
  const read = (path: string, query: URLSearchParams): Response | 'unread' | null => {
    const only = (...allowed: string[]) => [...query.keys()].every((p) => allowed.includes(p));
    if (path === '/_router/health' && only()) return unread.has('health') ? 'unread' : json(200, HEALTH);
    if (path === '/api/projects' && only()) return unread.has('projects') ? 'unread' : json(200, PROJECTS);
    return null;
  };

  const answer = async (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
    // Resolved here rather than by `new Request`, which outside a browser has no page to resolve
    // the console's relative paths against, and throws.
    const href = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
    const url = new URL(href, globalThis.location?.href);
    if (!ROUTER.test(url.pathname)) return passThrough(input, init);
    const method = (init?.method ?? (input instanceof Request ? input.method : 'GET')).toUpperCase();
    const said = `${method} ${url.pathname}${url.search}`;
    const got = method === 'GET' ? read(url.pathname, url.searchParams) : null;
    if (got === 'unread') {
      answered.push(said);
      throw new TypeError('fetch failed');
    }
    if (got) {
      answered.push(said);
      return got;
    }
    unanswered.push(said);
    const message = `the fixture harness has no answer for ${said}: it answers the reads the screens it serves make, and no write`;
    console.error(`harness: ${message}`);
    return json(501, { error: message });
  };

  return { fetch: answer, answered, unanswered };
}
