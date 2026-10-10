// Copied from Northstar ui/src/test-utils.tsx at e9c1589.
// Changed: the router is read through `fetch`, not a generated client, so the mocks are Responses and a fetch stub that answers by path; a refusal is the router's `{error}` shape, under a 200 as readily as under a 4xx.
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { act, fireEvent, render, type RenderOptions } from '@testing-library/react';
import type { ReactElement, ReactNode } from 'react';
import { MemoryRouter, Route, Routes, useLocation } from 'react-router';
import { Theme } from '@carbon/react';
import { vi } from 'vitest';
import { NARROW_QUERY } from '~/lib/viewport';

/// Render a screen the way the app renders it: inside the query cache and the router.
///
/// `path` is the route pattern (`/settings/:section`) and `at` the URL to start on, so
/// `useParams` and `useSearchParams` see real values. Retries are off, so a mocked failure
/// fails once and the test sees the held/stale state, not a wait. The current location is
/// rendered, hidden, as `data-testid="location"`, for a test that follows a link.
///
/// On asserting refusals: Carbon's TextInput renders its own `role="alert"` character
/// counter beside every input, so `getByRole('alert')` on a form is ambiguous. Assert on
/// the refusal's text, never on the alert role alone.
export function renderScreen(
  ui: ReactElement,
  { path = '/', at = '/', ...opts }: { path?: string; at?: string } & Omit<RenderOptions, 'wrapper'> = {},
) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false, staleTime: 0 } } });
  // Where the router under test is: path and query.
  const Where = () => {
    const where = useLocation();
    return (
      <div data-testid="location" hidden>
        {where.pathname + where.search}
      </div>
    );
  };
  const Wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>
      <Theme theme="white">
        <MemoryRouter initialEntries={[at]}>
          <Routes>
            <Route path={path} element={children} />
            <Route path="*" element={<div data-testid="elsewhere">{'elsewhere'}</div>} />
          </Routes>
          <Where />
        </MemoryRouter>
      </Theme>
    </QueryClientProvider>
  );
  return { client, ...render(ui, { wrapper: Wrapper, ...opts }) };
}

/// The router's answers, built as `fetch` resolves them. A fresh Response per call, because a
/// body is read once.
export const json = (data: unknown, status = 200) =>
  new Response(JSON.stringify(data), { status, headers: { 'Content-Type': 'application/json' } });

/// A refusal in the router's shape. Its status is 200 by default, because about a dozen of the
/// router's endpoints refuse that way; pass the 4xx for the ones that do not.
export const refusal = (message: string, status = 200) => json({ error: message }, status);

/// A fetch that never got a response: the browser rejects with a `TypeError`.
export const unreachable = (): Promise<Response> => Promise.reject(new TypeError('fetch failed'));

/// A 2xx with nothing in it.
export const emptyBody = () => new Response(null, { status: 204 });

export type Answers = Readonly<Record<string, () => Response | Promise<Response>>>;

/// Replace `fetch` with one that answers by path, and fails loudly for a path it was not told
/// about: a screen that quietly got nothing would look like a screen with nothing in it. Undo it
/// with `vi.unstubAllGlobals()` in `afterEach`.
export function stubFetch(answers: Answers) {
  const fetch = vi.fn(async (input: RequestInfo | URL) => {
    const href = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
    const url = new URL(href, 'http://console.test');
    const answer = answers[url.pathname];
    if (!answer) throw new Error(`test fetch: no answer for GET ${url.pathname}${url.search}`);
    return answer();
  });
  vi.stubGlobal('fetch', fetch);
  return fetch;
}

/// Let pending promises and effects settle.
export const settle = () => new Promise((r) => setTimeout(r, 0));

/// `settle`, inside `act`, so React commits what the settled promises changed.
export const flush = (): Promise<void> =>
  act(async () => {
    await settle();
  });

/// Click a control and let what it started settle, all inside one `act`.
export async function press(el: Element): Promise<void> {
  await act(async () => {
    fireEvent.click(el);
    await settle();
  });
}

/// `matchMedia` stubs for the other platform. Each call adds its query to the set the stub
/// answers true for, so `stubNarrow(); stubCoarse();` is a phone. The stub is installed with
/// `vi.stubGlobal`; undo it with `vi.unstubAllGlobals()` in `afterEach`.
const COARSE_QUERY = '(pointer: coarse)';
const MATCHING = Symbol('matching');
type MediaStub = typeof globalThis.matchMedia & { [MATCHING]?: Set<string> };

function stubMedia(query: string): void {
  const current = globalThis.matchMedia as MediaStub | undefined;
  const matching = current?.[MATCHING];
  if (matching) {
    matching.add(query);
    return;
  }
  const answers = new Set([query]);
  const stub = ((q: string) => ({
    matches: answers.has(q),
    media: q,
    onchange: null,
    addListener: () => {},
    removeListener: () => {},
    addEventListener: () => {},
    removeEventListener: () => {},
    dispatchEvent: () => false,
  })) as MediaStub;
  stub[MATCHING] = answers;
  vi.stubGlobal('matchMedia', stub);
}

/// Below Carbon's `md` breakpoint: the phone layout, as `useNarrow` reads it.
export const stubNarrow = (): void => stubMedia(NARROW_QUERY);

/// A touch pointer, as `useCoarse` reads it: 48px targets, nothing hover-only.
export const stubCoarse = (): void => stubMedia(COARSE_QUERY);
