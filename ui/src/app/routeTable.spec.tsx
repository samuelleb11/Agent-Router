// New for agent-router: the route table the console serves, mounted in memory with stand-in screens, one
// of which can be made to throw: a screen that fails and an address that is nothing both stay inside
// the shell (docs/web-console-contracts.md, "Shell").
import { Theme } from '@carbon/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { configure, fireEvent, render, screen, within } from '@testing-library/react';
import { RouterProvider, createMemoryRouter } from 'react-router';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { json, stubFetch, type Answers } from '~/test-utils';
import { routeTable } from './routeTable';

/// Waits and limits generous enough for a machine busy with other work: a timeout here is a slow
/// machine, never a pass.
vi.setConfig({ testTimeout: 30_000 });
configure({ asyncUtilTimeout: 10_000 });

const crash = vi.hoisted(() => ({ overview: false }));

vi.mock('~/screens/Overview', () => ({
  default: function Overview() {
    if (crash.overview) throw new Error('the Overview fell over drawing its totals');
    return <h1>The Overview</h1>;
  },
}));
vi.mock('~/screens/Sessions', () => ({ default: () => <h1>The Sessions screen</h1> }));
vi.mock('~/screens/Machines', () => ({ default: () => <h1>The Machines screen</h1> }));
vi.mock('~/screens/Settings', async () => {
  const { Outlet } = await import('react-router');
  return { SettingsLayout: () => <Outlet />, SettingsSection: () => <h1>A Settings page</h1> };
});

/// React Router's data router builds a Request for every navigation with jsdom's AbortSignal, which
/// Node's own Request refuses; a browser has one of each and no such quarrel. The signal is dropped:
/// no route here loads data, so nothing reads it.
class LenientRequest extends Request {
  constructor(input: RequestInfo | URL, init: RequestInit = {}) {
    super(input, { ...init, signal: null });
  }
}

function mount(at: string, more: Answers = {}) {
  vi.stubGlobal('Request', LenientRequest);
  stubFetch({ '/_router/health': () => json({ ok: true, pid: 1, version: '0.9.0', build: 'f00dfeed' }), '/api/projects': () => json([]), ...more });
  const router = createMemoryRouter(routeTable, { initialEntries: [at] });
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(
    <QueryClientProvider client={client}>
      <Theme theme="white">
        <RouterProvider router={router} />
      </Theme>
    </QueryClientProvider>,
  );
  return router;
}

const topbar = () => screen.getByRole('banner', { name: 'Agent Router' });
const rail = () => screen.getByRole('navigation', { name: 'Console' });

afterEach(() => {
  crash.overview = false;
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe('the route table', () => {
  it('SH-C8: a screen that throws renders an error screen inside the shell, and the topbar and the navigation keep working', async () => {
    crash.overview = true;
    // React reports the error it caught; the point is that the page survives it.
    vi.spyOn(console, 'error').mockImplementation(() => {});
    const router = mount('/');
    expect(await screen.findByRole('heading', { name: 'This page stopped working' })).toBeInTheDocument();
    expect(screen.getByText(/the Overview fell over drawing its totals/)).toBeInTheDocument();
    expect(screen.getByText(/a fault in the console, not an answer from the router/)).toBeInTheDocument();
    expect(within(topbar()).getByRole('status', { name: /^Freshness: / })).toBeInTheDocument();

    // The navigation still leaves by itself, with no reload.
    fireEvent.click(within(rail()).getByRole('link', { name: 'Sessions' }));
    expect(await screen.findByRole('heading', { name: 'The Sessions screen' })).toBeInTheDocument();
    expect(router.state.location.pathname).toBe('/live');
    expect(screen.queryByRole('heading', { name: 'This page stopped working' })).toBeNull();

    // And the topbar's actions still answer.
    fireEvent.click(within(topbar()).getByRole('button', { name: 'Search and commands (⌘K)' }));
    expect(await screen.findByRole('dialog', { name: 'Search and commands' })).toBeInTheDocument();
  });

  it('MA-C1: /machines is the Machines screen, inside the shell, and a hosted router’s rail reaches it from anywhere', async () => {
    const router = mount('/machines', { '/api/auth': () => json({ mode: 'server' }) });
    expect(await screen.findByRole('heading', { name: 'The Machines screen' })).toBeInTheDocument();
    fireEvent.click(within(rail()).getByRole('link', { name: 'Sessions' }));
    expect(await screen.findByRole('heading', { name: 'The Sessions screen' })).toBeInTheDocument();
    fireEvent.click(await within(rail()).findByRole('link', { name: 'Machines' }));
    expect(await screen.findByRole('heading', { name: 'The Machines screen' })).toBeInTheDocument();
    expect(router.state.location.pathname).toBe('/machines');
  });

  it('SH-C8: an address no route claims renders a not-found screen inside the shell, never a blank page', async () => {
    const router = mount('/no/such/page');
    expect(await screen.findByRole('heading', { name: 'No page at this address' })).toBeInTheDocument();
    expect(screen.getByText('/no/such/page')).toBeInTheDocument();
    expect(within(topbar()).getByRole('link', { name: 'Agent Router' })).toHaveAttribute('href', '/');
    fireEvent.click(within(rail()).getByRole('link', { name: 'Overview' }));
    expect(await screen.findByRole('heading', { name: 'The Overview' })).toBeInTheDocument();
    expect(router.state.location.pathname).toBe('/');
  });
});
