// New for agent-router; Northstar's client is generated from its OpenAPI spec and the router has
// none. The one fetch layer every read goes through, so its rules live once.

import { EmptyBody, Refused, Unreachable } from '~/lib/errors';

/// No read waits forever. The router answers its own reads from SQLite in milliseconds, so ten
/// seconds without an answer is a router that is not answering, and the view says so instead of
/// waiting on a promise that never settles.
export const TIMEOUT_MS = 10_000;

export interface ReadOptions {
  /// The caller's cancellation, TanStack Query's when it is the caller.
  readonly signal?: AbortSignal;
  readonly timeoutMs?: number;
}

/// GET a JSON answer from the router, same-origin.
///
/// Four outcomes, four signals:
/// - the answer, when the router answered 2xx with a body and refused nothing in it;
/// - `Refused` when it answered non-2xx, or 2xx with `error` or `ok: false` in the body, in the
///   router's own words;
/// - `EmptyBody` when a 2xx carried nothing;
/// - `Unreachable` when no answer came, or none in time.
export async function getJson<T>(path: string, { signal, timeoutMs = TIMEOUT_MS }: ReadOptions = {}): Promise<T> {
  const timer = new AbortController();
  const timeout = setTimeout(() => timer.abort(), timeoutMs);
  const cancel = () => timer.abort();
  signal?.addEventListener('abort', cancel, { once: true });
  let response: Response;
  try {
    response = await fetch(path, { headers: { Accept: 'application/json' }, credentials: 'same-origin', signal: timer.signal });
  } catch (err) {
    // The caller cancelling is not the router failing: hand the cancellation back as it came.
    if (signal?.aborted) throw err;
    if (timer.signal.aborted) throw new Unreachable(`The router did not answer within ${timeoutMs / 1000} s.`);
    throw new Unreachable();
  } finally {
    clearTimeout(timeout);
    signal?.removeEventListener('abort', cancel);
  }

  const text = await response.text();
  if (!response.ok) throw new Refused(response.status, wordsOf(text) || `${response.status} ${response.statusText}`.trim());
  if (response.status === 204 || text.trim() === '') throw new EmptyBody();
  let body: unknown;
  try {
    body = JSON.parse(text);
  } catch {
    throw new Refused(response.status, `The router answered something that is not JSON: ${text.slice(0, 120)}`);
  }
  if (isRecord(body)) {
    if (typeof body['error'] === 'string' && body['error'] !== '') throw new Refused(response.status, body['error']);
    if (body['ok'] === false) throw new Refused(response.status, typeof body['message'] === 'string' ? body['message'] : 'The router refused this.');
  }
  return body as T;
}

const isRecord = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null && !Array.isArray(v);

/// The router's words from a refusal's body: `{"error": "…"}` from a handler, or the plain
/// `agent-router: …` line its guard answers with.
function wordsOf(text: string): string {
  try {
    const body: unknown = JSON.parse(text);
    if (isRecord(body) && typeof body['error'] === 'string') return body['error'];
  } catch {
    /* not JSON: the guard's plain line */
  }
  return text.trim();
}
