// Copied from Northstar ui/src/screens/Settings/Layout.spec.tsx at e9c1589.
// Changed: agent-router's seven sections, and its breadcrumb; the sections are offered as the router's
// /api/auth allows, so every case answers it, and the cases for a hosted router and one that has not
// answered yet are agent-router's.
import { act, fireEvent, screen, waitFor } from '@testing-library/react';
import { Route, Routes, useNavigate } from 'react-router';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Auth } from '~/model/settings';
import { renderScreen, stubNarrow } from '~/test-utils';
import SettingsLayout from './Layout';
import SettingsSection from './Section';
import { NO_MACHINE } from './rules';
import { HOSTED, LOCAL, ok, stubRouter, type Answer } from './testing';

const SEVEN = ['Accounts', 'Models', 'Claude Code', 'Network', 'Password', 'Notifications', 'Router'];

const pages = (
  <Route path="/settings" element={<SettingsLayout />}>
    <Route path="accounts" element={<div>ACCOUNTS PAGE</div>} />
    <Route path="models" element={<div>MODELS PAGE</div>} />
    <Route path="claude-code" element={<div>CLAUDE CODE PAGE</div>} />
    <Route path="network" element={<div>NETWORK PAGE</div>} />
    <Route path="password" element={<div>PASSWORD PAGE</div>} />
    <Route path="notifications" element={<div>NOTIFICATIONS PAGE</div>} />
    <Route path="router" element={<div>ROUTER PAGE</div>} />
    {/* A segment none of the seven pages owns still lands in the layout, as the app would
        route it, so the layout's own answer to it is what is tested. */}
    <Route path="*" element={<div>NOWHERE</div>} />
  </Route>
);

const show = (at: string) => renderScreen(<Routes>{pages}</Routes>, { path: '/*', at });

/// Every case starts on a local router, which has all seven pages, and waits for it to say so.
let router: ReturnType<typeof stubRouter>;
const answer = (auth: Auth | null, more: Record<string, Answer> = {}) =>
  (router = stubRouter({ '/api/auth': auth ? ok(auth) : () => new Promise<Response>(() => {}), ...more }));
beforeEach(() => {
  answer(LOCAL);
});

afterEach(() => vi.unstubAllGlobals());

describe('SettingsLayout', () => {
  it('renders the page for the URL beside seven destinations', async () => {
    show('/settings/router');
    expect(screen.getByText('ROUTER PAGE')).toBeInTheDocument();
    await waitFor(() => expect(screen.getAllByRole('tab').map((t) => t.textContent)).toEqual(SEVEN));
    expect(screen.getByRole('tab', { name: 'Router' })).toHaveAttribute('aria-selected', 'true');
  });

  it('navigates when a tab is chosen', async () => {
    show('/settings/router');
    fireEvent.click(await screen.findByRole('tab', { name: 'Network' }));
    expect(screen.getByText('NETWORK PAGE')).toBeInTheDocument();
    expect(screen.getByRole('tab', { name: 'Network' })).toHaveAttribute('aria-selected', 'true');
  });

  /// A view is a link. Each tab is an anchor to its page, so it can be middle-clicked, copied
  /// and opened in a new tab; a plain click is the router's.
  it('each tab is an anchor to its page', async () => {
    show('/settings/router');
    await screen.findByRole('tab', { name: 'Network' });
    const slugs = ['accounts', 'models', 'claude-code', 'network', 'password', 'notifications', 'router'];
    SEVEN.forEach((name, i) => {
      const tab = screen.getByRole('tab', { name });
      expect(tab.tagName).toBe('A');
      expect(tab).toHaveAttribute('href', `/settings/${slugs[i]}`);
    });
  });


  it('leaves a modified click to the browser, so open-in-new-tab keeps working', async () => {
    show('/settings/router');
    await screen.findByRole('tab', { name: 'Network' });
    // The browser would open the tab; jsdom cannot, so the click is stopped at the document,
    // after the console has had it and left it alone.
    const stop = (e: Event) => e.preventDefault();
    document.addEventListener('click', stop);
    fireEvent.click(screen.getByRole('tab', { name: 'Network' }), { metaKey: true });
    document.removeEventListener('click', stop);
    expect(screen.getByText('ROUTER PAGE')).toBeInTheDocument();
    expect(screen.queryByText('NETWORK PAGE')).not.toBeInTheDocument();
    expect(screen.getByRole('tab', { name: 'Router' })).toHaveAttribute('aria-selected', 'true');
  });

  it('matches the whole segment: /settings/routerx is not the Router page', () => {
    show('/settings/routerx');
    expect(screen.queryByText('ROUTER PAGE')).not.toBeInTheDocument();
    expect(screen.queryByText('NOWHERE')).not.toBeInTheDocument();
    expect(screen.getByRole('tab', { name: 'Router' })).toHaveAttribute('aria-selected', 'false');
    expect(screen.getByText('ACCOUNTS PAGE')).toBeInTheDocument();
  });

  it('lands /settings on the first page rather than an empty outlet', () => {
    show('/settings');
    expect(screen.getByText('ACCOUNTS PAGE')).toBeInTheDocument();
  });

  /// The SAME control in a hand — the same seven tabs, each still an anchor to its page.
  it('in a hand offers the same seven destinations, as the same tabs', async () => {
    stubNarrow();
    show('/settings/network');
    await screen.findByRole('tab', { name: 'Network' });
    const tabs = screen.getAllByRole('tab');
    expect(tabs.map((t) => t.textContent)).toEqual(SEVEN);
    expect(screen.getByRole('tab', { name: 'Network' })).toHaveAttribute('aria-selected', 'true');
    for (const t of tabs) expect(t.tagName).toBe('A');
    fireEvent.click(screen.getByRole('tab', { name: 'Router' }));
    expect(screen.getByText('ROUTER PAGE')).toBeInTheDocument();
  });

  /// The page opens the way every screen opens: a breadcrumb whose parent is a link, and a
  /// title. The seven pages are a sub-navigation under it, not seven page headers.
  it('opens with one page header: a breadcrumb to the Overview, and one title', () => {
    show('/settings/accounts');
    expect(screen.getByRole('link', { name: 'Overview' })).toHaveAttribute('href', '/');
    expect(screen.getByRole('heading', { level: 1 })).toHaveTextContent('Settings');
  });
});

/// The layout with the real pages under it, as the app routes them.
const real = (at: string) =>
  renderScreen(
    <Routes>
      <Route path="/settings" element={<SettingsLayout />}>
        <Route path=":section" element={<SettingsSection />} />
      </Route>
    </Routes>,
    { path: '/*', at },
  );

describe('Settings, as the router allows', () => {
  it('ST-C1: Settings is pages under /settings — one header, the sections as tabs that are real links, /settings alone opening Accounts, and Back moving between them', async () => {
    let go: ReturnType<typeof useNavigate> = () => {};
    function Back() {
      go = useNavigate();
      return null;
    }
    renderScreen(
      <>
        <Routes>{pages}</Routes>
        <Back />
      </>,
      { path: '/*', at: '/settings' },
    );
    expect(screen.getByText('ACCOUNTS PAGE')).toBeInTheDocument();
    expect(screen.getByTestId('location')).toHaveTextContent('/settings/accounts');
    expect(screen.getAllByRole('heading', { level: 1 })).toHaveLength(1);
    fireEvent.click(await screen.findByRole('tab', { name: 'Password' }));
    expect(screen.getByText('PASSWORD PAGE')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('tab', { name: 'Router' }));
    expect(screen.getByTestId('location')).toHaveTextContent('/settings/router');
    for (const t of screen.getAllByRole('tab')) expect(t.tagName).toBe('A');
    act(() => {
      void go(-1);
    });
    expect(screen.getByText('PASSWORD PAGE')).toBeInTheDocument();
    expect(screen.getByRole('tab', { name: 'Password' })).toHaveAttribute('aria-selected', 'true');
  });

  it('ST-C101: until /api/auth has said what kind of router this is, neither Claude Code nor Network is offered, and their addresses draw nothing of theirs', async () => {
    answer(null);
    real('/settings/network');
    expect(await screen.findByText('Asking the router what kind it is…')).toBeInTheDocument();
    expect(screen.getAllByRole('tab').map((t) => t.textContent)).toEqual(['Accounts', 'Models', 'Password', 'Notifications', 'Router']);
    expect(screen.queryByRole('switch')).toBeNull();
  });

  it('ST-C100: an answer without mode is a local router that predates the field, and reads exactly as local', async () => {
    answer({ configured: true, remote_bind: false, from_loopback: true }, { '/api/setup': ok({ service: { registered: false, supported: true, platform: 'macos' }, managed: [], global_settings_path: '/x' }), '/api/setup/shim': ok({ targets: [] }) });
    real('/settings/claude-code');
    await waitFor(() => expect(screen.getAllByRole('tab').map((t) => t.textContent)).toEqual(SEVEN));
    expect(await screen.findByRole('switch', { name: 'Start at login' })).toBeInTheDocument();
  });

  it('ST-C103, ST-C53, ST-C72, ST-C104: on a hosted router Claude Code and Network are not offered, and Forges and Library are; a bookmarked address says why in the router’s sentence, and nothing of theirs is read', async () => {
    answer(HOSTED);
    real('/settings/network');
    expect(await screen.findByText(NO_MACHINE)).toBeInTheDocument();
    expect(screen.getByText('Network is not on this router')).toBeInTheDocument();
    expect(screen.getAllByRole('tab').map((t) => t.textContent)).toEqual(['Accounts', 'Models', 'Password', 'Notifications', 'Router', 'Forges', 'Library']);
    expect(screen.queryByRole('tab', { selected: true })).toBeNull();
    vi.unstubAllGlobals();
    document.body.innerHTML = '';
    answer(HOSTED);
    real('/settings/claude-code');
    expect(await screen.findByText(NO_MACHINE)).toBeInTheDocument();
    expect(router.said().filter((s) => s.startsWith('GET /api/setup'))).toEqual([]);
  });

  it('SH-C7: each Settings page names itself in the document’s title', async () => {
    answer(LOCAL, { '/_router/health': ok({ ok: true, pid: 1, version: '0.9.0', build: 'x' }) });
    real('/settings/router');
    await waitFor(() => expect(document.title).toBe('Router · Settings · Agent Router'));
  });

  it('SH-C48: once the page is 600 px down a Back to top button appears and returns to the top, smoothly unless reduced motion is asked for', async () => {
    const to = vi.fn();
    vi.stubGlobal('scrollTo', to);
    show('/settings/accounts');
    expect(screen.queryByRole('button', { name: 'Back to top' })).toBeNull();
    Object.defineProperty(window, 'scrollY', { value: 700, configurable: true });
    try {
      act(() => {
        window.dispatchEvent(new Event('scroll'));
      });
      fireEvent.click(await screen.findByRole('button', { name: 'Back to top' }));
      expect(to).toHaveBeenCalledWith({ top: 0, behavior: 'smooth' });
      Object.defineProperty(window, 'scrollY', { value: 100, configurable: true });
      act(() => {
        window.dispatchEvent(new Event('scroll'));
      });
      expect(screen.queryByRole('button', { name: 'Back to top' })).toBeNull();
    } finally {
      Reflect.deleteProperty(window, 'scrollY');
    }
  });
});
