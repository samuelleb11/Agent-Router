// Copied from Northstar ui/src/screens/Settings/Layout.spec.tsx at e9c1589.
// Changed: agent-router's six sections, its breadcrumb, and the /next basename every tab's href must carry.
import { fireEvent, render, screen } from '@testing-library/react';
import { MemoryRouter, Route, Routes } from 'react-router';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { renderScreen, stubNarrow } from '~/test-utils';
import SettingsLayout from './Layout';

const SIX = ['Accounts', 'Claude Code', 'Network', 'Password', 'Notifications', 'Router'];

const pages = (
  <Route path="/settings" element={<SettingsLayout />}>
    <Route path="accounts" element={<div>ACCOUNTS PAGE</div>} />
    <Route path="claude-code" element={<div>CLAUDE CODE PAGE</div>} />
    <Route path="network" element={<div>NETWORK PAGE</div>} />
    <Route path="password" element={<div>PASSWORD PAGE</div>} />
    <Route path="notifications" element={<div>NOTIFICATIONS PAGE</div>} />
    <Route path="router" element={<div>ROUTER PAGE</div>} />
    {/* A segment none of the six pages owns still lands in the layout, as the app would
        route it, so the layout's own answer to it is what is tested. */}
    <Route path="*" element={<div>NOWHERE</div>} />
  </Route>
);

const show = (at: string) => renderScreen(<Routes>{pages}</Routes>, { path: '/*', at });

afterEach(() => vi.unstubAllGlobals());

describe('SettingsLayout', () => {
  it('renders the page for the URL beside six destinations', () => {
    show('/settings/router');
    expect(screen.getByText('ROUTER PAGE')).toBeInTheDocument();
    expect(screen.getAllByRole('tab').map((t) => t.textContent)).toEqual(SIX);
    expect(screen.getByRole('tab', { name: 'Router' })).toHaveAttribute('aria-selected', 'true');
  });

  it('navigates when a tab is chosen', () => {
    show('/settings/router');
    fireEvent.click(screen.getByRole('tab', { name: 'Network' }));
    expect(screen.getByText('NETWORK PAGE')).toBeInTheDocument();
    expect(screen.getByRole('tab', { name: 'Network' })).toHaveAttribute('aria-selected', 'true');
  });

  /// A view is a link. Each tab is an anchor to its page, so it can be middle-clicked, copied
  /// and opened in a new tab; a plain click is the router's.
  it('each tab is an anchor to its page', () => {
    show('/settings/router');
    const slugs = ['accounts', 'claude-code', 'network', 'password', 'notifications', 'router'];
    SIX.forEach((name, i) => {
      const tab = screen.getByRole('tab', { name });
      expect(tab.tagName).toBe('A');
      expect(tab).toHaveAttribute('href', `/settings/${slugs[i]}`);
    });
  });

  /// Served at /next/ until the switch, the console's own navigation adds the basename by
  /// itself, and an anchor's href does not: a tab copied or opened in a new tab must still land
  /// on the console, not on the old dashboard at /settings.
  it('gives every tab the basename the console is served under', () => {
    render(
      <MemoryRouter basename="/next" initialEntries={['/next/settings/password']}>
        <Routes>{pages}</Routes>
      </MemoryRouter>,
    );
    expect(screen.getByText('PASSWORD PAGE')).toBeInTheDocument();
    expect(screen.getByRole('tab', { name: 'Accounts' })).toHaveAttribute('href', '/next/settings/accounts');
    expect(screen.getByRole('tab', { name: 'Router' })).toHaveAttribute('href', '/next/settings/router');
    fireEvent.click(screen.getByRole('tab', { name: 'Router' }));
    expect(screen.getByText('ROUTER PAGE')).toBeInTheDocument();
  });

  it('leaves a modified click to the browser, so open-in-new-tab keeps working', () => {
    show('/settings/router');
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

  /// The SAME control in a hand — the same six tabs, each still an anchor to its page.
  it('in a hand offers the same six destinations, as the same tabs', () => {
    stubNarrow();
    show('/settings/network');
    const tabs = screen.getAllByRole('tab');
    expect(tabs.map((t) => t.textContent)).toEqual(SIX);
    expect(screen.getByRole('tab', { name: 'Network' })).toHaveAttribute('aria-selected', 'true');
    for (const t of tabs) expect(t.tagName).toBe('A');
    fireEvent.click(screen.getByRole('tab', { name: 'Router' }));
    expect(screen.getByText('ROUTER PAGE')).toBeInTheDocument();
  });

  /// The page opens the way every screen opens: a breadcrumb whose parent is a link, and a
  /// title. The six pages are a sub-navigation under it, not six page headers.
  it('opens with one page header: a breadcrumb to the Overview, and one title', () => {
    show('/settings/accounts');
    expect(screen.getByRole('link', { name: 'Overview' })).toHaveAttribute('href', '/');
    expect(screen.getByRole('heading', { level: 1 })).toHaveTextContent('Settings');
  });
});
