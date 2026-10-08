// New for agent-router: the Sessions screen's reads and writes of the router's live endpoints
// (src/live.rs, and src/hosted.rs where a hosted router answers the same paths), typed by hand
// because the router has no OpenAPI spec. Every one goes through ~/data/http, the one fetch layer,
// so a refusal and a timeout read the same way here as on every other screen (SH-C23).

import { getJson, postJson } from '~/data/http';
import { EmptyBody, Refused } from '~/lib/errors';
import type { SearchReply } from '~/model/search';
import type { LiveOptions, Prompt, SessionRow, Shim } from '~/model/session';

/// The Sessions screen's keys. The list's and the options' are the palette's (`paletteKeys` in
/// ~/screens/Palette/api), so a palette opened over the screen shows what the screen shows and one
/// read serves both.
export const liveKeys = {
  sessions: ['live', 'sessions'] as const,
  options: ['live', 'options'] as const,
  hosted: ['live', 'hosted'] as const,
  shim: ['live', 'shim'] as const,
  stats: (session: string) => ['live', 'stats', session] as const,
  project: (key: string) => ['live', 'project', key] as const,
  prompt: (session: string) => ['live', 'prompt', session] as const,
  find: (q: string, scope: string, session: string | null) => ['live', 'find', q, scope, session ?? ''] as const,
};

const enc = encodeURIComponent;

/// What one session has spent and how it is set (src/live.rs, `Stats`).
export interface Stats {
  readonly model?: string | null;
  readonly effort?: string | null;
  /// The permission mode in force, as the session last stamped it on a turn.
  readonly mode?: string | null;
  readonly input?: number | null;
  readonly output?: number | null;
  readonly cache_read?: number | null;
  readonly cache_write?: number | null;
  /// US dollars; absent or null when the model is not priced.
  readonly cost?: number | null;
  /// Everything read or written, cache included.
  readonly total?: number | null;
  readonly turns?: number | null;
}

/// What a project's sessions on screen have spent together (src/live.rs, `project_stats`).
export interface ProjectTotals {
  readonly sessions?: number | null;
  readonly models?: readonly string[] | null;
  readonly total?: number | null;
  readonly output?: number | null;
  readonly cache_read?: number | null;
  readonly turns?: number | null;
  readonly cost?: number | null;
}

const isRecord = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null && !Array.isArray(v);

/// `GET /api/live/sessions`. A reply that is not a list — the router's 500 `{ok:false, reason}`
/// is refused by the fetch layer already — is a failed read, never rows (SE-C41); a row that is
/// not an object is dropped, and every other row is kept whatever keys it has (SE-C22).
export async function readSessions(signal?: AbortSignal): Promise<SessionRow[]> {
  const body = await getJson<unknown>('/api/live/sessions', { signal });
  if (!Array.isArray(body)) throw new Refused(200, 'The router answered something that is not a list of sessions.');
  return body.filter(isRecord) as unknown as SessionRow[];
}

/// `GET /api/live/options`: what a session can be switched to, one list for every client (SE-C138).
export const readOptions = (signal?: AbortSignal) => getJson<LiveOptions>('/api/live/options', { signal });

/// Whether the router is a hosted one, from `/api/auth`'s `mode`. A router that predates the
/// field, or does not answer, is local, so the screen keeps working against it (SE-C186).
export async function readHosted(signal?: AbortSignal): Promise<boolean> {
  try {
    const auth = await getJson<unknown>('/api/auth', { signal });
    return isRecord(auth) && auth['mode'] === 'server';
  } catch (err) {
    if (signal?.aborted) throw err;
    return false;
  }
}

/// `GET /api/setup/shim`: the editor extensions and whether each is wrapped. Never asked of a
/// hosted router (SE-C188).
export const readShim = (signal?: AbortSignal) => getJson<Shim>('/api/setup/shim', { signal });

/// `GET /api/live/stats?session=`: read once each time a session opens (SE-C133).
export const readStats = (session: string, signal?: AbortSignal) => getJson<Stats>(`/api/live/stats?session=${enc(session)}`, { signal });

/// `GET /api/live/project?project=`: the project's own total (SE-C132), asked by the session's project
/// key, its repository else its directory (`liveProjectKey`, SE-C232).
export const readProject = (key: string, signal?: AbortSignal) => getJson<ProjectTotals>(`/api/live/project?project=${enc(key)}`, { signal });

/// `GET /api/live/prompt?session=`: what the session is blocked on now, if anything (SE-C120).
export const readPrompt = (session: string, signal?: AbortSignal) => getJson<Prompt>(`/api/live/prompt?session=${enc(session)}`, { signal });

/// `GET /api/search`, as Find asks it (SE-C164, SE-C165).
export const readFind = (params: Readonly<Record<string, string>>, signal?: AbortSignal) =>
  getJson<SearchReply>(`/api/search?${new URLSearchParams(params)}`, { signal });

/// Where a session's stream is (SE-C55): its real id, URL-encoded.
export const streamUrl = (session: string): string => `/api/live/stream?session=${enc(session)}`;

/// What the router says of a stream the browser has given up on, in its words: `refused`, it
/// refuses the caller, which no retry changes (SE-C60); `unavailable`, it answers this session
/// with an error status, as a hosted router does while the session's machine is offline (503),
/// when no machine holds the session (404) or when the relay failed (502), which is said and
/// retried (SE-C223, SE-C224).
export interface StreamRefusal {
  readonly kind: 'refused' | 'unavailable';
  readonly words: string;
}

/// Why the router will not stream a session, asked once the browser has given its stream up
/// (SE-C60, SE-C65). `EventSource` reports no status, so the question goes to the same session's
/// prompt, a finite read behind the same guard and routed as the stream is: the guard refuses a
/// caller without the password on every route alike, with a 401 and its own words, and a hosted
/// router relays every per-session route to the machine holding the session, and answers each
/// with the same 503, 404 or 502 when it cannot. The stream's own address cannot be asked through
/// the fetch layer, whose timeout ends when the headers arrive, so a stream that is there after
/// all would be read for ever. Null when the prompt answers, or no answer came: the watchdog then
/// reconnects in its own time.
export async function streamRefusal(session: string, signal?: AbortSignal): Promise<StreamRefusal | null> {
  try {
    await readPrompt(session, signal);
    return null;
  } catch (err) {
    if (!(err instanceof Refused) || err.status < 400) return null;
    if (err.status === 401 || err.status === 403) return { kind: 'refused', words: err.message || 'The router refused this stream: a password is needed.' };
    return { kind: 'unavailable', words: err.message };
  }
}

/// What became of a write. Done, with the router's answer; refused, with its status and its
/// words, empty when it gave none; or never answered, `status` null, with why.
export type Outcome<T> =
  | { readonly ok: true; readonly body: T }
  | { readonly ok: false; readonly status: number; readonly words: string }
  | { readonly ok: false; readonly status: null; readonly words: string };

/// The fetch layer's sentence for a refusal that carried no words of the router's own
/// (~/data/http, `unexplained`, and its `ok:false` with nothing said).
const UNSAID = /gave no reason\.$|^The router refused this\.$/;

/// Every live write: `postJson`, with its timeout, and the outcome as a value, so a control can
/// word it by the screen's rules (SE-C183) and come back when it times out (SE-C185).
export async function postLive<T = Record<string, unknown>>(path: string, body: unknown, timeoutMs?: number): Promise<Outcome<T>> {
  try {
    const answer = await postJson<T>(path, body, timeoutMs === undefined ? {} : { timeoutMs });
    return { ok: true, body: answer };
  } catch (err) {
    if (err instanceof EmptyBody) return { ok: true, body: {} as T };
    if (err instanceof Refused) return { ok: false, status: err.status, words: UNSAID.test(err.message) ? '' : err.message };
    return { ok: false, status: null, words: err instanceof Error && err.message ? err.message : 'Could not reach the router.' };
  }
}

/// A side question waits for a forked replay of the session, which the router allows three
/// minutes (src/live.rs, `aside`); this gives its answer a little longer than that to arrive.
export const ASIDE_TIMEOUT_MS = 185_000;
