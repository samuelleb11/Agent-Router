// New for agent-router: the console wherever it is opened (docs/web-console-contracts.md, "Shell") — the
// whole app, its real screens on the router's answers, in a browser whose storage refuses every read
// and write, as a private window or a frame with storage off does. What a sandboxed frame does is shown
// in a browser by scripts/shell-checks.mjs, which opens the console in one.
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ok, router } from '~/screens/Overview/fixtures';
import { LOCAL as SETTINGS_LOCAL, ACCOUNTS as SETTINGS_ACCOUNTS } from '~/screens/Settings/testing';
import { FakeEventSource } from '~/screens/Sessions/fakes';

vi.setConfig({ testTimeout: 60_000 });

// The activity chart's library is drawn by a browser, never by jsdom.
vi.mock('@carbon/charts-react', () => ({ StackedBarChart: () => <svg data-testid="chart" /> }));

/// React Router's data router builds a Request for every navigation with jsdom's AbortSignal, which
/// Node's own Request refuses; a browser has one of each and no such quarrel. The signal is dropped:
/// no route here loads data, so nothing reads it.
class LenientRequest extends Request {
  constructor(input: RequestInfo | URL, init: RequestInit = {}) {
    super(input, { ...init, signal: null });
  }
}

/// Storage that refuses every read and write, local and session alike.
function refuseStorage(): () => void {
  const names = ['localStorage', 'sessionStorage'] as const;
  const was = names.map((name) => [name, Object.getOwnPropertyDescriptor(window, name)] as const);
  const refuse = () => {
    throw new DOMException('The operation is insecure.', 'SecurityError');
  };
  const refusing = { getItem: refuse, setItem: refuse, removeItem: refuse, clear: refuse, key: refuse, get length(): number { return refuse(); } };
  for (const name of names) for (const target of new Set<object>([globalThis, window])) Object.defineProperty(target, name, { get: () => refusing, configurable: true });
  return () => {
    for (const [name, descriptor] of was) for (const target of new Set<object>([globalThis, window])) if (descriptor) Object.defineProperty(target, name, descriptor);
  };
}

afterEach(() => {
  vi.unstubAllGlobals();
  window.history.replaceState(null, '', '/');
});

describe('wherever the console is opened', () => {
  it('SH-C55: with browser storage refused, the whole console draws, every screen opens from the rail, and the theme and the rail still switch, only unremembered', async () => {
    const restore = refuseStorage();
    try {
      vi.stubGlobal('EventSource', FakeEventSource);
      vi.stubGlobal('Request', LenientRequest);
      router((path) => {
        switch (path) {
          case '/_router/health':
            return ok({ ok: true, pid: 4242, version: '0.9.0', build: 'f00dfeed' });
          case '/api/live/sessions':
            return ok([]);
          case '/api/setup/shim':
            return ok({ targets: [] });
          case '/api/login/status':
            return ok({ phase: 'failed', url: null, label: '', account_id: null, email: null, error: null });
          case '/api/auth':
            return ok(SETTINGS_LOCAL);
          case '/api/accounts':
            return ok(SETTINGS_ACCOUNTS);
          default:
            return undefined;
        }
      });
      window.history.replaceState(null, '', '/');
      // The app as main.tsx starts it, loaded after the storage is refused, as a private window
      // loads it: the theme and the rail read the storage as they load.
      vi.resetModules();
      const { App } = await import('~/app/App');
      render(<App />);

      expect(await screen.findByRole('heading', { level: 2, name: 'Totals in scope' }, { timeout: 30_000 })).toBeInTheDocument();
      const topbar = screen.getByRole('banner', { name: 'Agent Router' });
      await waitFor(() => expect(within(topbar).getByRole('status')).toHaveAttribute('data-live-mode', 'live'));

      // The theme and the rail are a convenience: refused storage forgets them, never breaks them.
      fireEvent.click(within(topbar).getByRole('button', { name: /^Theme: system/ }));
      expect(document.documentElement).toHaveClass('ar-light');
      fireEvent.click(within(topbar).getByRole('button', { name: /^Theme: light/ }));
      expect(document.documentElement).toHaveClass('ar-dark');
      fireEvent.click(within(topbar).getByRole('button', { name: /^Theme: dark/ }));
      fireEvent.click(within(topbar).getByRole('button', { name: 'Collapse the rail' }));
      expect(within(topbar).getByRole('button', { name: 'Expand the rail' })).toBeInTheDocument();

      const rail = screen.getByRole('navigation', { name: 'Console' });
      await act(async () => fireEvent.click(within(rail).getByRole('link', { name: 'Sessions' })));
      expect(await screen.findByRole('heading', { level: 1, name: 'Live sessions' }, { timeout: 30_000 })).toBeInTheDocument();
      await act(async () => fireEvent.click(within(rail).getByRole('link', { name: 'Settings' })));
      expect(await screen.findByLabelText('Account work', {}, { timeout: 30_000 })).toBeInTheDocument();
      expect(window.location.pathname).toBe('/settings/accounts');
    } finally {
      restore();
    }
  });
});
