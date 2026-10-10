// New for agent-router: the first-boot password prompt the shell opens, against the router's answers
// (docs/web-console-contracts.md, "Password").
import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import { Link } from 'react-router';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { Auth } from '~/model/settings';
import { json, renderScreen } from '~/test-utils';
import { FirstBootGate } from './FirstBootGate';
import { HOSTED, LOCAL, ok, stubRouter, type Answer } from './testing';

afterEach(() => vi.unstubAllGlobals());

/// First boot: on the network, nothing set, read at the machine itself.
const FIRST: Auth = { ...LOCAL, configured: false, updated_at: null, remote_bind: true, from_loopback: true };

function show(auth: Auth = FIRST, at = '/', more: Record<string, Answer> = {}) {
  let state = auth;
  const router = stubRouter({ '/api/auth': () => json(state), ...more });
  const view = renderScreen(
    <>
      <FirstBootGate />
      <Link to="/settings/accounts">to settings</Link>
      <Link to="/">to the overview</Link>
    </>,
    { path: '*', at },
  );
  return { ...router, ...view, set: (next: Partial<Auth>) => (state = { ...state, ...next }) };
}

const prompt = () => screen.findByRole('dialog', { name: 'Set a password' });
/// Long enough for the router's answer to have arrived and been drawn, or not.
const settled = () => act(async () => new Promise((r) => setTimeout(r, 30)));

describe('the first-boot password prompt', () => {
  it('ST-C80: opens by itself only when the router listens on the network, nothing is set, it is read here, it is local, not pinned, not dismissed, and Settings is not on screen', async () => {
    show();
    expect(await prompt()).toBeInTheDocument();
    const quiet: [string, Auth, string][] = [
      ['loopback only', { ...FIRST, remote_bind: false }, '/'],
      ['a password set', { ...FIRST, configured: true }, '/'],
      ['read from another machine', { ...FIRST, from_loopback: false }, '/'],
      ['a hosted router', { ...HOSTED, configured: false, password_pinned: false, from_loopback: true }, '/'],
      ['a pinned password', { ...FIRST, password_pinned: true }, '/'],
      ['under Settings', FIRST, '/settings/accounts'],
    ];
    for (const [why, auth, at] of quiet) {
      vi.unstubAllGlobals();
      document.body.innerHTML = '';
      show(auth, at);
      await settled();
      expect(screen.queryByRole('dialog'), why).toBeNull();
    }
  });

  it('ST-C81: it says the router is reachable, who will be asked and who never is, and that until one is set others are refused; it offers Set password and Not now', async () => {
    show();
    const dialog = await prompt();
    expect(dialog).toHaveTextContent(
      'This router is reachable from your network. Other devices will be asked for this password; this Mac never is. Until one is set, other devices are refused.',
    );
    expect(within(dialog).getByRole('button', { name: 'Set password' })).toBeInTheDocument();
    expect(within(dialog).getByRole('button', { name: 'Not now' })).toBeInTheDocument();
  });

  it('ST-C82: Not now closes it for the rest of the load, through re-reads and a visit to Settings', async () => {
    const r = show();
    fireEvent.click(within(await prompt()).getByRole('button', { name: 'Not now' }));
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    await act(async () => {
      await r.client.invalidateQueries();
    });
    fireEvent.click(screen.getByRole('link', { name: 'to settings' }));
    fireEvent.click(screen.getByRole('link', { name: 'to the overview' }));
    await settled();
    expect(screen.queryByRole('dialog')).toBeNull();
  });

  it('ST-C82: Escape closes it for the rest of the load too', async () => {
    const r = show();
    const dialog = await prompt();
    fireEvent.keyDown(dialog, { key: 'Escape', code: 'Escape' });
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    await act(async () => {
      await r.client.invalidateQueries();
    });
    await settled();
    expect(screen.queryByRole('dialog')).toBeNull();
  });

  it('ST-C83, ST-C82: once the password is set from it, it closes, /api/auth is read again, and it does not come back', async () => {
    const r = show(FIRST, '/', { 'POST /api/auth': ok({ ok: true, configured: true }) });
    const dialog = await prompt();
    fireEvent.change(within(dialog).getByLabelText('New password'), { target: { value: 'a good password' } });
    fireEvent.change(within(dialog).getByLabelText('Confirm'), { target: { value: 'a good password' } });
    const reads = r.gets('/api/auth').length;
    fireEvent.click(within(dialog).getByRole('button', { name: 'Set password' }));
    await waitFor(() => expect(r.posts('/api/auth')).toEqual([{ password: 'a good password' }]));
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(r.gets('/api/auth').length).toBeGreaterThan(reads);
    // Even if the router were to say again that nothing is set, this load has been asked once.
    await act(async () => {
      await r.client.invalidateQueries();
    });
    await settled();
    expect(screen.queryByRole('dialog')).toBeNull();
  });
});
