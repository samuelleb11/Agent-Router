// New for agent-router: the fetch layer's rules, for a read and for a write.
import { afterEach, describe, expect, it, vi } from 'vitest';
import { EmptyBody, Refused, Unreachable, refusalText } from '~/lib/errors';
import { emptyBody, json, refusal, stubFetch, unreachable } from '~/test-utils';
import { getJson, postJson } from './http';

afterEach(() => {
  vi.unstubAllGlobals();
});

/// What `fetch` was called with, the request init included: the stub records every argument.
function calls(fetch: ReturnType<typeof stubFetch>) {
  return (fetch.mock.calls as unknown as [RequestInfo | URL, RequestInit | undefined][]).map(([input, init]) => ({
    path: String(input),
    init: init ?? {},
  }));
}

/// The words a failed call carries, and the class it failed with.
async function failure(p: Promise<unknown>): Promise<{ error: unknown; words: string }> {
  try {
    await p;
  } catch (error) {
    return { error, words: refusalText(error) };
  }
  throw new Error('expected the call to fail');
}

const text = (body: string, status: number) => new Response(body, { status, headers: { 'Content-Type': 'text/plain' } });

describe('getJson', () => {
  it('answers what the router answered, same-origin and asking for JSON', async () => {
    const fetch = stubFetch({ '/api/x': () => json({ a: 1 }) });
    await expect(getJson('/api/x')).resolves.toEqual({ a: 1 });
    const [call] = calls(fetch);
    expect(call?.init.credentials).toBe('same-origin');
    expect(call?.init.headers).toMatchObject({ Accept: 'application/json' });
    expect(call?.init.method).toBeUndefined();
  });

  it('reads a 200 carrying error as a refusal, in the router’s words', async () => {
    stubFetch({ '/api/x': () => refusal('no such account') });
    const { error, words } = await failure(getJson('/api/x'));
    expect(error).toBeInstanceOf(Refused);
    expect((error as Refused).status).toBe(200);
    expect(words).toBe('no such account');
  });

  it('reads a 200 carrying ok:false as a refusal, with its reason, else its message, else a sentence', async () => {
    stubFetch({ '/api/x': () => json({ ok: false, reason: 'the wrapper is not running' }) });
    expect((await failure(getJson('/api/x'))).words).toBe('the wrapper is not running');

    stubFetch({ '/api/x': () => json({ ok: false, message: 'not now' }) });
    expect((await failure(getJson('/api/x'))).words).toBe('not now');

    stubFetch({ '/api/x': () => json({ ok: false }) });
    expect((await failure(getJson('/api/x'))).words).toBe('The router refused this.');
  });

  it('reads a 4xx or 5xx in the words its body carries: error, reason or message', async () => {
    stubFetch({ '/api/x': () => json({ ok: false, reason: 'Only a session under the wrapper can change model.' }, 503) });
    const live = await failure(getJson('/api/x'));
    expect((live.error as Refused).status).toBe(503);
    expect(live.words).toBe('Only a session under the wrapper can change model.');

    stubFetch({ '/api/x': () => refusal('That session is not on a connected machine.', 503) });
    expect((await failure(getJson('/api/x'))).words).toBe('That session is not on a connected machine.');

    stubFetch({ '/api/x': () => json({ message: 'slow down' }, 429) });
    expect((await failure(getJson('/api/x'))).words).toBe('slow down');
  });

  it('never shows a body’s JSON as its words: a refusal with none is a sentence naming the status', async () => {
    stubFetch({ '/api/x': () => json({ ok: false, code: 7 }, 500) });
    const { words } = await failure(getJson('/api/x'));
    expect(words).not.toMatch(/[{}"]/);
    expect(words).toMatch(/^The router answered 500\b.* and gave no reason\.$/);

    stubFetch({ '/api/x': () => new Response(null, { status: 503 }) });
    expect((await failure(getJson('/api/x'))).words).toMatch(/^The router answered 503\b.* and gave no reason\.$/);
  });

  it('keeps the guard’s plain line as it came', async () => {
    stubFetch({ '/api/x': () => text('agent-router: a password is required from another machine\n', 401) });
    expect((await failure(getJson('/api/x'))).words).toBe('agent-router: a password is required from another machine');
  });

  it('tells an empty answer and no answer apart from a refusal', async () => {
    stubFetch({ '/api/x': () => emptyBody() });
    expect((await failure(getJson('/api/x'))).error).toBeInstanceOf(EmptyBody);

    stubFetch({ '/api/x': unreachable });
    const gone = await failure(getJson('/api/x'));
    expect(gone.error).toBeInstanceOf(Unreachable);
    expect(gone.words).toBe('Could not reach the router.');
  });

  it('says a router that does not answer in time did not answer, and hands back the caller’s own cancel', async () => {
    const hang = vi.fn(
      (_input: RequestInfo | URL, init?: RequestInit) =>
        new Promise<Response>((_resolve, reject) =>
          init?.signal?.addEventListener('abort', () => reject(new DOMException('The operation was aborted.', 'AbortError'))),
        ),
    );
    vi.stubGlobal('fetch', hang);
    const late = await failure(getJson('/api/x', { timeoutMs: 5 }));
    expect(late.error).toBeInstanceOf(Unreachable);
    expect(late.words).toBe('The router did not answer within 0.005 s.');

    const caller = new AbortController();
    const cancelled = failure(getJson('/api/x', { signal: caller.signal }));
    caller.abort();
    const { error } = await cancelled;
    expect(error).toBeInstanceOf(DOMException);
    expect((error as DOMException).name).toBe('AbortError');
  });
});

describe('postJson', () => {
  it('sends its body as JSON, same-origin, and answers what the router answered', async () => {
    const fetch = stubFetch({ '/api/live/archive': () => json({ ok: true, archived: 3 }) });
    await expect(postJson('/api/live/archive', { session: 's1', archived: true })).resolves.toEqual({ ok: true, archived: 3 });
    const [call] = calls(fetch);
    expect(call?.path).toBe('/api/live/archive');
    expect(call?.init.method).toBe('POST');
    expect(call?.init.credentials).toBe('same-origin');
    expect(call?.init.headers).toMatchObject({ 'Content-Type': 'application/json', Accept: 'application/json' });
    expect(JSON.parse(String(call?.init.body))).toEqual({ session: 's1', archived: true });
  });

  it('reads a refused write by the same rules: a 200 error, a 200 ok:false, a 4xx or 5xx reason', async () => {
    stubFetch({ '/api/accounts/activate': () => refusal('unknown account: nobody') });
    expect((await failure(postJson('/api/accounts/activate', { id: 'nobody' }))).words).toBe('unknown account: nobody');

    stubFetch({ '/api/setup/shim': () => json({ ok: false, reason: 'the shim is not installed' }) });
    expect((await failure(postJson('/api/setup/shim', { on: true }))).words).toBe('the shim is not installed');

    stubFetch({ '/api/live/mode': () => json({ ok: false, reason: 'Cannot set permission mode to bypassPermissions.' }, 422) });
    const refused = await failure(postJson('/api/live/mode', { session: 's1', mode: 'bypassPermissions' }));
    expect((refused.error as Refused).status).toBe(422);
    expect(refused.words).toBe('Cannot set permission mode to bypassPermissions.');
  });

  it('says a write that got no answer could not reach the router', async () => {
    stubFetch({ '/api/live/effort': unreachable });
    expect((await failure(postJson('/api/live/effort', { session: 's1', effort: 'high' }))).error).toBeInstanceOf(Unreachable);
  });
});
