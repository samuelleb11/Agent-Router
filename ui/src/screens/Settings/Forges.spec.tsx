// New for agent-router: Settings → Forges as the owner meets it (docs/web-console-contracts.md, "Forges and
// the Library"), against the router's answers.
import { screen, waitFor, within } from '@testing-library/react';
import { fireEvent } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ForgeRow } from '~/model/forges';
import { ok, stubRouter } from '~/screens/Settings/testing';
import { press, refusal, renderScreen, unreachable } from '~/test-utils';
import ForgesPage from './Forges';

const FORGES: ForgeRow[] = [
  { key: 'home', kind: 'gitlab', base_url: 'https://gitlab.example.com', token_present: true, sources: 2, created_at: 1 },
  { key: 'hub', kind: 'github', base_url: 'https://github.com', token_present: false, sources: 0, created_at: 2 },
];

const show = () => renderScreen(<ForgesPage />, { path: '/settings/forges', at: '/settings/forges' });
const card = (key: string) => {
  const found = document.querySelector<HTMLElement>(`[data-forge="${key}"]`);
  if (!found) throw new Error(`no forge ${key}`);
  return found;
};
const type = (label: RegExp | string, value: string) => fireEvent.change(screen.getByLabelText(label), { target: { value } });

afterEach(() => vi.unstubAllGlobals());

describe('Settings → Forges', () => {
  it('ST-C105, ST-C116: lists each forge — its key, kind, address, whether a token is kept and how many sources use it — and never a token; a failed read is said', async () => {
    stubRouter({ '/api/library/forges': ok({ forges: FORGES }) });
    const first = show();
    await waitFor(() => card('home'));
    const home = within(card('home'));
    for (const fact of ['home', 'GitLab', 'https://gitlab.example.com', 'kept on the router', '2']) expect(home.getByText(fact)).toBeInTheDocument();
    expect(within(card('hub')).getByText('none')).toBeInTheDocument();
    expect(within(card('hub')).getByText('GitHub')).toBeInTheDocument();
    first.unmount();
    vi.unstubAllGlobals();

    stubRouter({ '/api/library/forges': unreachable });
    show();
    expect(await screen.findByText('Could not read the forges')).toBeInTheDocument();
    expect(screen.queryByText('No forge yet: add one to import from its repositories.')).toBeNull();
  });

  it('ST-C106: adds a forge with its token, changes one keeping its token or removing it, and shows the router’s refusal beside the form', async () => {
    const stub = stubRouter({
      '/api/library/forges': ok({ forges: FORGES }),
      'PUT /api/library/forges/lab': ok({ ok: true }),
      'PUT /api/library/forges/home': ok({ ok: true }),
      'PUT /api/library/forges/plain': () => refusal('http://gitlab.example.com is plain http: use https', 422),
    });
    show();
    await waitFor(() => card('home'));
    await press(screen.getByRole('button', { name: 'Add a forge' }));
    type('Key', 'lab');
    type('Address', 'https://lab.example.com');
    type(/^Token/, 'a-token-typed-here');
    await press(screen.getByRole('button', { name: 'Save' }));
    await waitFor(() => expect(stub.asked.filter((r) => r.method === 'PUT' && r.path === '/api/library/forges/lab')).toHaveLength(1));
    expect(stub.asked.find((r) => r.path === '/api/library/forges/lab')!.body).toEqual({ kind: 'gitlab', base_url: 'https://lab.example.com', token: 'a-token-typed-here' });

    // A change with the token left empty keeps the kept one; "Remove the token" keeps none.
    await press(within(card('home')).getByRole('button', { name: 'Change' }));
    expect(screen.getByLabelText('Key')).toBeDisabled();
    await press(screen.getByRole('button', { name: 'Save' }));
    await waitFor(() => expect(stub.asked.filter((r) => r.path === '/api/library/forges/home')).toHaveLength(1));
    expect(stub.asked.find((r) => r.path === '/api/library/forges/home')!.body).toEqual({ kind: 'gitlab', base_url: 'https://gitlab.example.com' });
    await press(within(card('home')).getByRole('button', { name: 'Change' }));
    await press(screen.getByLabelText('Remove the token'));
    await press(screen.getByRole('button', { name: 'Save' }));
    await waitFor(() => expect(stub.asked.filter((r) => r.path === '/api/library/forges/home')).toHaveLength(2));
    expect(stub.asked.filter((r) => r.path === '/api/library/forges/home')[1]!.body).toEqual({ kind: 'gitlab', base_url: 'https://gitlab.example.com', token: null });

    await press(screen.getByRole('button', { name: 'Add a forge' }));
    type('Key', 'plain');
    type('Address', 'http://gitlab.example.com');
    await press(screen.getByRole('button', { name: 'Save' }));
    expect(await screen.findByText('http://gitlab.example.com is plain http: use https')).toBeInTheDocument();
  });

  it('ST-C107: Check says what came back — a heading over the router’s own words', async () => {
    stubRouter({
      '/api/library/forges': ok({ forges: FORGES }),
      'POST /api/library/forges/home/check': ok({ ok: true, result: 'unauthorised', words: 'the forge refused the token (401)' }),
      'POST /api/library/forges/hub/check': ok({ ok: true, result: 'ok', words: 'the forge answered; with no token, it reads public repositories only' }),
    });
    show();
    await waitFor(() => card('home'));
    await press(within(card('home')).getByRole('button', { name: 'Check' }));
    expect(await within(card('home')).findByText('The forge refused the token: the forge refused the token (401)')).toBeInTheDocument();
    await press(within(card('hub')).getByRole('button', { name: 'Check' }));
    const said = await within(card('hub')).findByText(/^The forge answered:/);
    expect(said).toHaveAttribute('data-checked', 'ok');
  });

  it('ST-C108: Remove asks once more, and a forge a source uses is refused in the router’s words inside the dialog', async () => {
    const stub = stubRouter({
      '/api/library/forges': ok({ forges: FORGES }),
      'DELETE /api/library/forges/home': () => refusal('2 source(s) import from the forge home: remove them first', 409),
    });
    show();
    await waitFor(() => card('home'));
    await press(within(card('home')).getByRole('button', { name: 'Remove' }));
    const dialog = await screen.findByRole('dialog');
    expect(within(dialog).getByText('Remove the forge home?')).toBeInTheDocument();
    expect(stub.asked.filter((r) => r.method === 'DELETE')).toHaveLength(0);
    await press(within(dialog).getByRole('button', { name: 'Remove' }));
    expect(await within(dialog).findByText('2 source(s) import from the forge home: remove them first')).toBeInTheDocument();
  });
});
