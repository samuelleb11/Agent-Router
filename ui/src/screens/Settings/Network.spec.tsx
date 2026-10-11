// New for agent-router: Settings → Network, "Listen on your network", against the router's answers
// (docs/web-console-contracts.md, "Network").
import { fireEvent, screen, waitFor, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { Auth } from '~/model/settings';
import { json, refusal, renderScreen } from '~/test-utils';
import NetworkPage from './Network';
import { LOCAL, ok, stubRouter, type Answer } from './testing';

afterEach(() => vi.unstubAllGlobals());

function show(auth: Auth = LOCAL, more: Record<string, Answer> = {}) {
  let state = auth;
  const router = stubRouter({ '/api/auth': () => json(state), ...more });
  const view = renderScreen(<NetworkPage />, { path: '/settings/network', at: '/settings/network' });
  return { ...router, ...view, set: (next: Partial<Auth>) => (state = { ...state, ...next }) };
}

const sw = () => screen.findByRole('switch', { name: 'Listen on your network' });
const OFF: Auth = { ...LOCAL, remote_bind: false, listening_on: null };

describe('Network', () => {
  it('ST-C65: the page and its switch keep the name "Listen on your network"', async () => {
    show();
    expect(await screen.findByRole('heading', { name: 'Listen on your network' })).toBeInTheDocument();
    expect(await sw()).toBeInTheDocument();
  });

  it('ST-C66, ST-C13: the switch shows what /api/auth reports, and /api/auth is read again after every answer, a refusal included', async () => {
    const SAID = 'listening changed, but it could not be saved for next time: the store is read-only';
    const r = show(OFF, { 'POST /api/network': () => refusal(SAID) });
    expect(await sw()).toHaveAttribute('aria-checked', 'false');
    const reads = r.gets('/api/auth').length;
    // The socket did change; only saving it failed. What the router reports next is what shows.
    r.set({ remote_bind: true });
    fireEvent.click(await sw());
    expect(await screen.findByText(SAID)).toBeInTheDocument();
    expect(r.posts('/api/network')).toEqual([{ enable: true }]);
    await waitFor(() => expect(r.gets('/api/auth').length).toBeGreaterThan(reads));
    await waitFor(async () => expect(await sw()).toHaveAttribute('aria-checked', 'true'));
  });

  it('ST-C67: on, the address other devices reach the router at, copyable; off, "This Mac only" and where turning it on would put it', async () => {
    show();
    const address = await screen.findByRole('textbox', { name: 'The router’s address on your network' });
    expect(address).toHaveTextContent('http://192.168.1.20:8787');
    expect(screen.getByRole('button', { name: 'Copy' })).toBeInTheDocument();
    vi.unstubAllGlobals();
    document.body.innerHTML = '';
    show(OFF);
    expect(await screen.findByText('This Mac only. Turning it on would put it at http://192.168.1.20:8787.')).toBeInTheDocument();
  });

  it('ST-C68: turning it on requires a router credential, with links to create a token or set a password; off is always offered', async () => {
    show({ ...OFF, configured: false });
    expect(await sw()).toBeDisabled();
    expect(screen.getByText(/^Create an/)).toHaveTextContent('other devices need a router credential');
    expect(screen.getByRole('link', { name: 'password' })).toHaveAttribute('href', '/settings/password');
    expect(screen.getByRole('link', { name: 'access token' })).toHaveAttribute('href', '/settings/access-tokens');
    vi.unstubAllGlobals();
    document.body.innerHTML = '';
    show({ ...LOCAL, configured: false });
    expect(await sw()).toBeEnabled();
  });

  it('allows network access when a token is configured and no legacy password is set', async () => {
    const r = show({ ...OFF, configured: false, access_tokens: 1 }, { 'POST /api/network': ok({ ok: true }) });
    expect(await sw()).toBeEnabled();
    expect(screen.queryByRole('link', { name: 'access token' })).toBeNull();
    fireEvent.click(await sw());
    await waitFor(() => expect(r.posts('/api/network')).toEqual([{ enable: true }]));
  });

  it('ST-C69: with AGENT_ROUTER_BIND set, the page says beside the switch that the variable decides the next start', async () => {
    show({ ...LOCAL, env_override: true });
    const s = await sw();
    const block = s.closest('[class*="control"]') as HTMLElement;
    expect(block).toHaveTextContent('AGENT_ROUTER_BIND is set in this router’s environment; it decides the next time the router starts');
  });

  it('ST-C70: the page explains off and on, that it takes effect at once with nothing restarted, and that it is remembered across a reboot', async () => {
    show();
    const s = await sw();
    expect(s.closest('[class*="control"]')).toHaveTextContent(
      'Off, the router answers only on this Mac. On, it also listens on your local network, so the phone apps — and a Claude Code on another machine — can reach it; every one of them must present an access token or the router password. It takes effect at once, with nothing restarted and no session dropped, and it is remembered across a reboot.',
    );
  });

  it('ST-C71, ST-C18: turning it off asks first in a danger modal, and read from another machine says this browser will lose the router', async () => {
    const r = show({ ...LOCAL, from_loopback: false }, { 'POST /api/network': ok({ ok: true }) });
    fireEvent.click(await sw());
    const dialog = await screen.findByRole('dialog');
    expect(dialog).toHaveTextContent('The router answers only on this Mac from then on');
    expect(dialog).toHaveTextContent('You are reading this from another machine. This browser loses the router at once, and cannot turn it back on from here.');
    const confirm = within(dialog).getByRole('button', { name: 'Turn off' });
    expect(confirm.className).toMatch(/cds--btn--danger/);
    expect(r.posts('/api/network')).toEqual([]);
    fireEvent.click(confirm);
    await waitFor(() => expect(r.posts('/api/network')).toEqual([{ enable: false }]));
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  });

  it('ST-C71: from this machine, turning it off still asks, without the lock-out warning; turning it on does not ask', async () => {
    const r = show(LOCAL, { 'POST /api/network': ok({ ok: true }) });
    fireEvent.click(await sw());
    const dialog = await screen.findByRole('dialog');
    expect(dialog).not.toHaveTextContent('You are reading this from another machine');
    fireEvent.click(within(dialog).getByRole('button', { name: 'Cancel' }));
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(r.posts('/api/network')).toEqual([]);
    vi.unstubAllGlobals();
    document.body.innerHTML = '';
    const off = show(OFF, { 'POST /api/network': ok({ ok: true }) });
    fireEvent.click(await sw());
    await waitFor(() => expect(off.posts('/api/network')).toEqual([{ enable: true }]));
    expect(screen.queryByRole('dialog')).toBeNull();
  });
});
