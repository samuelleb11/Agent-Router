// New for agent-router; Northstar's client is generated from its OpenAPI spec and the router has
// none. The one fetch layer every read and every write goes through, so its rules live once.

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
export function getJson<T>(path: string, options: ReadOptions = {}): Promise<T> {
  return request<T>(path, { headers: { Accept: 'application/json' } }, options);
}

/// POST a JSON body to the router, same-origin, and read the answer by `getJson`'s rules: the same
/// timeout and the same four outcomes.
///
/// A write is refused in more shapes than a read, which is why the rules are shared rather than
/// repeated: a handler's `{error}` under a 4xx or a 5xx, the same under a 200, and the live
/// endpoints' `{ok: false, reason}` (src/live.rs, `bad`). Each comes back as `Refused`, in the
/// router's words. A write that timed out may still have landed, and the caller's wording should
/// not claim otherwise.
export function postJson<T>(path: string, body: unknown, options: ReadOptions = {}): Promise<T> {
  return request<T>(
    path,
    { method: 'POST', headers: { Accept: 'application/json', 'Content-Type': 'application/json' }, body: JSON.stringify(body) },
    options,
  );
}

/// POST bytes to the router — a Library upload, `Content-Type` naming what they are — and read the
/// JSON answer by `getJson`'s rules.
export function postBytes<T>(path: string, body: Blob, contentType: string, options: ReadOptions = {}): Promise<T> {
  return request<T>(path, { method: 'POST', headers: { Accept: 'application/json', 'Content-Type': contentType }, body }, options);
}

/// GET a file's text from the router, same-origin: a refusal in the router's words, no answer in
/// time as `Unreachable`, as every read.
export async function getText(path: string, { signal, timeoutMs = TIMEOUT_MS }: ReadOptions = {}): Promise<string> {
  const timer = new AbortController();
  const timeout = setTimeout(() => timer.abort(), timeoutMs);
  const cancel = () => timer.abort();
  signal?.addEventListener('abort', cancel, { once: true });
  let response: Response;
  try {
    response = await fetch(path, { credentials: 'same-origin', signal: timer.signal });
  } catch (err) {
    if (signal?.aborted) throw err;
    if (timer.signal.aborted) throw new Unreachable(`The router did not answer within ${timeoutMs / 1000} s.`);
    throw new Unreachable();
  } finally {
    clearTimeout(timeout);
    signal?.removeEventListener('abort', cancel);
  }
  const text = await response.text();
  if (!response.ok) throw new Refused(response.status, wordsOf(text) || unexplained(response));
  return text;
}

async function request<T>(path: string, init: RequestInit, { signal, timeoutMs = TIMEOUT_MS }: ReadOptions): Promise<T> {
  const timer = new AbortController();
  const timeout = setTimeout(() => timer.abort(), timeoutMs);
  const cancel = () => timer.abort();
  signal?.addEventListener('abort', cancel, { once: true });
  let response: Response;
  try {
    response = await fetch(path, { ...init, credentials: 'same-origin', signal: timer.signal });
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
  if (!response.ok) throw new Refused(response.status, wordsOf(text) || unexplained(response));
  if (response.status === 204 || text.trim() === '') throw new EmptyBody();
  let body: unknown;
  try {
    body = JSON.parse(text);
  } catch {
    throw new Refused(response.status, `The router answered something that is not JSON: ${text.slice(0, 120)}`);
  }
  if (isRecord(body)) {
    if (typeof body['error'] === 'string' && body['error'] !== '') throw new Refused(response.status, body['error']);
    if (body['ok'] === false) throw new Refused(response.status, said(body) || 'The router refused this.');
  }
  return body as T;
}

const isRecord = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null && !Array.isArray(v);

/// The router's words in a body, by the key each of its handlers writes them under: `error` from
/// the API handlers and from a hosted router's 503s, `reason` from the live endpoints and the shim
/// switches, `message` from the rest. Empty when the body names none.
function said(body: Record<string, unknown>): string {
  for (const key of ['error', 'reason', 'message'] as const) {
    const words = body[key];
    if (typeof words === 'string' && words.trim() !== '') return words.trim();
  }
  return '';
}

/// The one plain line the router's guard refuses with, and its other plain refusals
/// (src/guard.rs, src/console.rs, src/proxy.rs): `agent-router: <why>`, a single line.
const GUARD_LINE = /^agent-router: [^\r\n]+$/;

/// The router's words from a refusal's body: what a JSON body says under one of those keys, or
/// the plain `agent-router: …` line its guard answers with. Nothing else that is not JSON is the
/// router's — a page from an ingress in front of it while it is down, several lines, another
/// proxy's text — so it gives no words, and the refusal is said by its status instead. A JSON
/// body that says nothing under any of those keys gives none either, and its JSON is never shown
/// as though it were the reason.
function wordsOf(text: string): string {
  let body: unknown;
  try {
    body = JSON.parse(text);
  } catch {
    const line = text.trim();
    return GUARD_LINE.test(line) ? line : '';
  }
  return isRecord(body) ? said(body) : '';
}

/// A refusal that gave no reason, said as a sentence: its status, never its body.
function unexplained(response: Response): string {
  return `The router answered ${`${response.status} ${response.statusText}`.trim()} and gave no reason.`;
}
