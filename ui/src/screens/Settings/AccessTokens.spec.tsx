import { fireEvent, screen, waitFor, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { accessTokenKey } from '~/data/access-tokens';
import type { AccessToken } from '~/model/access-tokens';
import { json, press, refusal, renderScreen, unreachable } from '~/test-utils';
import AccessTokensPage from './AccessTokens';
import { ok, stubRouter } from './testing';

const NOW = Date.UTC(2026, 9, 11, 12);
const SECRET = `arr_${'a'.repeat(64)}`;
const ACTIVE: AccessToken = {
  id: 'codex', name: 'Codex on laptop', prefix: 'arr_aaaaaaaaaaaa', scope: 'client', created_at: NOW - 60_000,
  last_used_at: null, expires_at: NOW + 90 * 86_400_000, revoked_at: null,
};
const clipboardBefore = Object.getOwnPropertyDescriptor(navigator, 'clipboard');
const TOKENS: AccessToken[] = [
  ACTIVE,
  { ...ACTIVE, id: 'machine', name: 'Build machine', scope: 'machine', expires_at: NOW - 1, last_used_at: NOW - 1000 },
  { ...ACTIVE, id: 'admin', name: 'Former admin', scope: 'admin', expires_at: null, revoked_at: NOW - 500 },
];
const show = () => renderScreen(<AccessTokensPage />, { path: '/settings/access-tokens', at: '/settings/access-tokens' });
const card = (id: string) => {
  const found = document.querySelector<HTMLElement>(`[data-token="${id}"]`);
  if (!found) throw new Error(`no token ${id}`);
  return found;
};
const field = (label: string, value: string) => fireEvent.change(screen.getByLabelText(label), { target: { value } });

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  if (clipboardBefore) Object.defineProperty(navigator, 'clipboard', clipboardBefore);
  else Reflect.deleteProperty(navigator, 'clipboard');
});

describe('Settings → Access tokens', () => {
  it('shows token metadata, expiry and permissions, keeping full secrets out of the list', async () => {
    vi.spyOn(Date, 'now').mockReturnValue(NOW);
    stubRouter({ '/api/auth/tokens': ok({ tokens: TOKENS }) });
    show();
    await waitFor(() => card('codex'));
    const codex = within(card('codex'));
    for (const text of ['Codex on laptop', 'arr_aaaaaaaaaaaa…', 'Client', 'Active', 'Never used']) expect(codex.getByText(text)).toBeInTheDocument();
    expect(card('codex').querySelector(`time[datetime="${new Date(ACTIVE.expires_at!).toISOString()}"]`)).toBeInTheDocument();
    expect(within(card('machine')).getByText('Expired')).toBeInTheDocument();
    expect(within(card('machine')).getByText('Machine')).toBeInTheDocument();
    expect(within(card('admin')).getByText('Administrator')).toBeInTheDocument();
    expect(within(card('admin')).getByRole('button', { name: 'Revoke' })).toBeDisabled();
    expect(screen.queryByText(SECRET)).toBeNull();
  });

  it('creates a client token with a 90-day expiry, shows it once, copies it, and keeps it out of query caches', async () => {
    vi.spyOn(Date, 'now').mockReturnValue(NOW);
    const copy = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText: copy } });
    let tokens: AccessToken[] = [];
    const stub = stubRouter({
      '/api/auth/tokens': () => json({ tokens }),
      'POST /api/auth/tokens': ({ body }) => {
        const created = { ...ACTIVE, id: 'new', ...(body as Pick<AccessToken, 'name' | 'scope' | 'expires_at'>), created_at: NOW };
        tokens = [created];
        return json({ ok: true, ...created, token: SECRET }, 201);
      },
    });
    const shown = show();
    await screen.findByText('No access tokens yet. Create one for each client or machine you connect.');
    await press(screen.getByRole('button', { name: 'Create a token' }));
    expect(screen.getByLabelText('Permissions')).toHaveValue('client');
    expect(screen.getByLabelText('Expires after')).toHaveValue('90');
    field('Name', '  My Codex  ');
    await press(screen.getByRole('button', { name: 'Create token' }));
    const dialog = await screen.findByRole('dialog', { name: 'Save your access token' });
    expect(stub.posts('/api/auth/tokens')).toEqual([{ name: 'My Codex', scope: 'client', expires_at: NOW + 90 * 86_400_000 }]);
    expect(within(dialog).getByText(SECRET)).toBeInTheDocument();
    await press(within(dialog).getByRole('button', { name: 'Copy token' }));
    expect(copy).toHaveBeenCalledWith(SECRET);
    await waitFor(() => expect(shown.client.getQueryData(accessTokenKey)).toEqual(tokens));
    expect(JSON.stringify(shown.client.getQueryCache().getAll().map((q) => q.state.data))).not.toContain(SECRET);
    expect(shown.client.getMutationCache().getAll()).toHaveLength(0);
    await press(within(dialog).getByRole('button', { name: 'I have saved the token' }));
    expect(screen.queryByText(SECRET)).toBeNull();
    expect(screen.getByText('My Codex')).toBeInTheDocument();
  });

  it('offers machine and administrator permissions and can create a token without expiry', async () => {
    const stub = stubRouter({
      '/api/auth/tokens': ok({ tokens: [] }),
      'POST /api/auth/tokens': ({ body }) => json({ ok: true, ...ACTIVE, ...(body as object), token: SECRET }, 201),
    });
    show();
    await screen.findByText('No access tokens yet. Create one for each client or machine you connect.');
    await press(screen.getByRole('button', { name: 'Create a token' }));
    field('Name', 'Admin phone');
    field('Permissions', 'machine');
    expect(screen.getByText('Connect a machine agent, report its sessions and send model requests.')).toBeInTheDocument();
    field('Permissions', 'admin');
    expect(screen.getByText('Access the dashboard and change router settings, accounts and access tokens.')).toBeInTheDocument();
    field('Expires after', 'never');
    await press(screen.getByRole('button', { name: 'Create token' }));
    const dialog = await screen.findByRole('dialog', { name: 'Save your access token' });
    expect(stub.posts('/api/auth/tokens')).toEqual([{ name: 'Admin phone', scope: 'admin', expires_at: null }]);
    expect(within(dialog).getByText(/It does not expire/)).toBeInTheDocument();
  });

  it('keeps a create refusal beside the form without displaying a secret or closing the form', async () => {
    stubRouter({ '/api/auth/tokens': ok({ tokens: [] }), 'POST /api/auth/tokens': () => refusal('name must be at most 64 characters', 400) });
    show();
    await screen.findByText('No access tokens yet. Create one for each client or machine you connect.');
    await press(screen.getByRole('button', { name: 'Create a token' }));
    expect(screen.getByRole('button', { name: 'Create token' })).toBeDisabled();
    field('Name', 'Name');
    await press(screen.getByRole('button', { name: 'Create token' }));
    const dialog = await screen.findByRole('dialog', { name: 'Create an access token' });
    expect(within(dialog).getByText('name must be at most 64 characters')).toBeInTheDocument();
    expect(screen.queryByRole('dialog', { name: 'Save your access token' })).toBeNull();
  });

  it('revokes only after confirmation and re-reads the list so it shows the revoked status', async () => {
    vi.spyOn(Date, 'now').mockReturnValue(NOW);
    let tokens = [ACTIVE];
    const stub = stubRouter({
      '/api/auth/tokens': () => json({ tokens }),
      'DELETE /api/auth/tokens/codex': () => { tokens = [{ ...ACTIVE, revoked_at: NOW }]; return json({ ok: true }); },
    });
    show();
    await waitFor(() => card('codex'));
    await press(within(card('codex')).getByRole('button', { name: 'Revoke' }));
    const dialog = await screen.findByRole('dialog');
    expect(within(dialog).getByText('Revoke Codex on laptop?')).toBeInTheDocument();
    expect(stub.asked.filter((r) => r.method === 'DELETE')).toHaveLength(0);
    await press(within(dialog).getByRole('button', { name: 'Revoke token' }));
    await waitFor(() => expect(within(card('codex')).getByText('Revoked')).toBeInTheDocument());
    expect(stub.asked.filter((r) => r.method === 'DELETE')).toHaveLength(1);
    expect(within(card('codex')).getByRole('button', { name: 'Revoke' })).toBeDisabled();
  });

  it('keeps a revoke refusal inside the confirmation and leaves the token active', async () => {
    vi.spyOn(Date, 'now').mockReturnValue(NOW);
    stubRouter({ '/api/auth/tokens': ok({ tokens: [ACTIVE] }), 'DELETE /api/auth/tokens/codex': () => refusal('token was not found', 404) });
    show();
    await waitFor(() => card('codex'));
    await press(within(card('codex')).getByRole('button', { name: 'Revoke' }));
    const dialog = await screen.findByRole('dialog');
    await press(within(dialog).getByRole('button', { name: 'Revoke token' }));
    expect(await within(dialog).findByText('token was not found')).toBeInTheDocument();
    expect(within(card('codex')).getByText('Active')).toBeInTheDocument();
  });

  it('distinguishes a failed read from an empty list and disables creation until the list is readable', async () => {
    stubRouter({ '/api/auth/tokens': unreachable });
    show();
    expect(await screen.findByText('Could not read access tokens')).toBeInTheDocument();
    expect(screen.queryByText('No access tokens yet. Create one for each client or machine you connect.')).toBeNull();
    expect(screen.getByRole('button', { name: 'Create a token' })).toBeDisabled();
  });

  it('keeps the page usable when the API supplies an expiry outside the browser date range', async () => {
    stubRouter({ '/api/auth/tokens': ok({ tokens: [{ ...ACTIVE, expires_at: 9_000_000_000_000_000 }] }) });
    show();
    expect(await screen.findByText('Unknown date')).toBeInTheDocument();
    expect(within(card('codex')).getByRole('button', { name: 'Revoke' })).toBeEnabled();
  });
});
