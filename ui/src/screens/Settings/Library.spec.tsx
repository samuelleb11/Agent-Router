// New for agent-router: Settings → Library as the owner meets it (docs/web-console-contracts.md, "Forges
// and the Library"), against the router's answers: sources and their webhooks, publish tokens, and
// whether publishes from machines go live.
import { fireEvent, screen, waitFor, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ForgeRow, Imported, SourceRow, TokenRow } from '~/model/forges';
import { ok, stubRouter, type Answer } from '~/screens/Settings/testing';
import { press, renderScreen, unreachable } from '~/test-utils';
import LibraryPage from './Library';

const FORGES: ForgeRow[] = [{ key: 'home', kind: 'gitlab', base_url: 'https://gitlab.example.com', token_present: true, sources: 1, created_at: 1 }];
const IMPORTED: Imported = {
  source: 'skills',
  commit: '0123456789abcdef0123456789abcdef01234567',
  fetched: true,
  items: [
    { kind: 'skill', name: 'bench', path: 'library/skills/bench', outcome: 'new', version: 2, state: 'live' },
    { kind: 'skill', name: 'vault', path: 'library/skills/vault', outcome: 'new', version: 1, state: 'live' },
    { kind: 'command', name: 'ship', path: 'library/commands/ship.md', outcome: 'unchanged', version: 1, state: 'live' },
    { kind: 'skill', name: 'agent-router', path: 'library/skills/agent-router', outcome: 'refused', why: 'comes with the router’s build' },
  ],
  gone: ['skill/old'],
};
const SOURCES: SourceRow[] = [
  {
    key: 'skills',
    forge: 'home',
    repository: 'whnet/skills',
    ref: 'main',
    path: 'library',
    live: true,
    head: IMPORTED.commit,
    checked_at: Date.now() - 60e3,
    imported_at: Date.now() - 60e3,
    last_result: IMPORTED,
    hook: true,
    created_at: 1,
  },
  { key: 'careful', forge: 'home', repository: 'whnet/other', ref: 'v2', path: '', live: false, head: null, checked_at: null, imported_at: null, last_result: { ok: false, error: 'the forge refused the token (401)' }, hook: false, created_at: 2 },
];
const TOKENS: TokenRow[] = [
  { id: 't1', name: 'ci', kinds: [], items: ['skill/bench'], live: false, created_at: Date.UTC(2026, 9, 7), last_used_at: null, revoked_at: null },
  { id: 't2', name: 'old-ci', kinds: ['command'], items: [], live: true, created_at: Date.UTC(2026, 9, 1), last_used_at: Date.UTC(2026, 9, 2), revoked_at: Date.UTC(2026, 9, 3) },
];

function router(more: Record<string, Answer> = {}) {
  return stubRouter({
    '/api/library/forges': ok({ forges: FORGES }),
    '/api/library/sources': ok({ sources: SOURCES }),
    '/api/library/tokens': ok({ tokens: TOKENS }),
    '/api/library/settings': ok({ machine_publishes: 'approval' }),
    ...more,
  });
}

const show = () => renderScreen(<LibraryPage />, { path: '/settings/library', at: '/settings/library' });
const found = (selector: string) => {
  const el = document.querySelector<HTMLElement>(selector);
  if (!el) throw new Error(`nothing at ${selector}`);
  return el;
};
const source = (key: string) => found(`[data-source="${key}"]`);
const token = (name: string) => found(`[data-token-row="${name}"]`);
const type = (label: RegExp | string, value: string) => fireEvent.change(screen.getByLabelText(label), { target: { value } });

afterEach(() => vi.unstubAllGlobals());

describe('Settings → Library', () => {
  it('ST-C109: lists each source — where it imports from, its trust, what its last import came to or the forge’s refusal, and whether its webhook has a secret, never the secret', async () => {
    router();
    show();
    await waitFor(() => source('skills'));
    const skills = within(source('skills'));
    for (const fact of ['home', 'whnet/skills', 'main', 'library', 'go live', 'a secret is set']) expect(skills.getByText(fact)).toBeInTheDocument();
    expect(skills.getByText(/^Imported at 01234567: 2 new, 1 unchanged, 1 refused; no longer in the repository: skill\/old\. \(/)).toBeInTheDocument();
    const careful = within(source('careful'));
    for (const fact of ['the whole repository', 'wait for approval', 'no secret yet', 'The forge said no: the forge refused the token (401)']) {
      expect(careful.getByText(fact)).toBeInTheDocument();
    }
  });

  it('ST-C110: adds a source on a chosen forge, its versions waiting for approval unless switched to go live', async () => {
    const stub = router({ 'PUT /api/library/sources/bench': ok({ ok: true }) });
    show();
    await waitFor(() => source('skills'));
    await press(screen.getByRole('button', { name: 'Add a source' }));
    const form = screen.getByRole('form', { name: 'Add a source' });
    expect(within(form).getByRole('switch', { name: 'Its new versions go live' })).not.toBeChecked();
    expect(within(form).getByLabelText('Ref')).toHaveValue('main');
    type('Key', 'bench');
    type('Repository', 'whnet/bench');
    type('Path', 'skills/bench');
    await press(within(form).getByRole('button', { name: 'Add the source' }));
    await waitFor(() => expect(stub.asked.filter((r) => r.method === 'PUT')).toHaveLength(1));
    expect(stub.asked.find((r) => r.method === 'PUT')!.body).toEqual({ forge: 'home', repository: 'whnet/bench', ref: 'main', path: 'skills/bench', live: false });
  });

  it('ST-C111: Import now imports at once and says what came of it', async () => {
    router({ 'POST /api/library/sources/skills/import': ok({ ok: true, imported: IMPORTED }) });
    show();
    await waitFor(() => source('skills'));
    await press(within(source('skills')).getByRole('button', { name: 'Import now' }));
    expect(await within(source('skills')).findByText('Imported at 01234567: 2 new, 1 unchanged, 1 refused; no longer in the repository: skill/old.', { selector: '[role="status"] *' })).toBeInTheDocument();
  });

  it('ST-C112: a webhook secret is made on request and shown once with the hook’s URL, each with a copy button', async () => {
    router({ 'POST /api/library/sources/careful/secret': ok({ ok: true, secret: 'a-fresh-hook-secret', path: '/hooks/library/careful', url: 'https://router.example/hooks/library/careful' }) });
    show();
    await waitFor(() => source('careful'));
    await press(within(source('careful')).getByRole('button', { name: 'Make a webhook secret' }));
    const box = await waitFor(() => found('[data-source="careful"] [data-secret]'));
    expect(within(box).getByText('https://router.example/hooks/library/careful')).toBeInTheDocument();
    expect(within(box).getByText('a-fresh-hook-secret')).toBeInTheDocument();
    expect(within(box).getByText(/shown now and never again, and any secret made before it no longer works/)).toBeInTheDocument();
    expect(within(box).getAllByRole('button', { name: 'Copy' })).toHaveLength(2);
    expect(within(source('skills')).getByRole('button', { name: 'Make a new webhook secret' })).toBeInTheDocument();
  });

  it('ST-C113, ST-C116: lists the tokens by name with what each may publish, its trust, when made, last used and revoked, never a value; a failed read is said', async () => {
    router();
    const first = show();
    await waitFor(() => token('ci'));
    const ci = within(token('ci'));
    for (const fact of ['skill/bench', 'waits for approval', 'never']) expect(ci.getByText(fact)).toBeInTheDocument();
    expect(ci.getByRole('button', { name: 'Revoke' })).toBeInTheDocument();
    const old = within(token('old-ci'));
    for (const fact of ['any command', 'goes live', 'Revoked']) expect(old.getByText(fact)).toBeInTheDocument();
    expect(old.queryByRole('button', { name: 'Revoke' })).toBeNull();
    first.unmount();
    vi.unstubAllGlobals();

    router({ '/api/library/tokens': unreachable });
    show();
    expect(await screen.findByText('Could not read the tokens')).toBeInTheDocument();
  });

  it('ST-C114: makes a token, its value shown once with a copy button; revoking asks once more', async () => {
    const stub = router({
      'POST /api/library/tokens': ok({ ok: true, id: 't3', name: 'pipeline', token: 'arl_0123456789abcdef0123456789abcdef01234567' }, 201),
      'DELETE /api/library/tokens/t1': ok({ ok: true }),
    });
    show();
    await waitFor(() => token('ci'));
    await press(screen.getByRole('button', { name: 'Make a token' }));
    const form = screen.getByRole('form', { name: 'Make a token' });
    type('Name', 'pipeline');
    await press(within(form).getByLabelText('skill'));
    type('Only these items', 'skill/bench, command/ship');
    await press(within(form).getByRole('button', { name: 'Make the token' }));
    const box = await waitFor(() => found('[data-token]'));
    expect(within(box).getByText('arl_0123456789abcdef0123456789abcdef01234567')).toBeInTheDocument();
    expect(within(box).getByText(/shown now and never again/)).toBeInTheDocument();
    expect(within(box).getByRole('button', { name: 'Copy' })).toBeInTheDocument();
    expect(stub.posts('/api/library/tokens')).toEqual([{ name: 'pipeline', kinds: ['skill'], items: ['skill/bench', 'command/ship'], live: false }]);

    await press(within(token('ci')).getByRole('button', { name: 'Revoke' }));
    const dialog = await screen.findByRole('dialog');
    expect(within(dialog).getByText('Revoke the token ci?')).toBeInTheDocument();
    expect(stub.asked.filter((r) => r.method === 'DELETE')).toHaveLength(0);
    await press(within(dialog).getByRole('button', { name: 'Revoke' }));
    await waitFor(() => expect(stub.asked.filter((r) => r.method === 'DELETE' && r.path === '/api/library/tokens/t1')).toHaveLength(1));
  });

  it('ST-C115: publishes from machines wait for approval until switched to go live, and the change is saved as it is made', async () => {
    const stub = router({ 'PUT /api/library/settings': ok({ ok: true, machine_publishes: 'live' }) });
    show();
    const toggle = await screen.findByRole('switch', { name: 'Publishes from machines go live' });
    expect(toggle).not.toBeChecked();
    await press(toggle);
    await waitFor(() => expect(stub.asked.filter((r) => r.method === 'PUT' && r.path === '/api/library/settings')).toHaveLength(1));
    expect(stub.asked.find((r) => r.path === '/api/library/settings' && r.method === 'PUT')!.body).toEqual({ machine_publishes: 'live' });
    expect(await screen.findByText('Publishes from machines go live from now on.')).toBeInTheDocument();
  });
});
