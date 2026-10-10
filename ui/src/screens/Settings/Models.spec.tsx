// New for agent-router: Settings → Models as the owner meets it (docs/web-console-contracts.md, "Models"),
// against the router's answers: the catalogue, who serves each model, and assigning one from its list box.
import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { AccountRow, ModelRow } from '~/data/settings';
import { json, refusal, renderScreen, unreachable } from '~/test-utils';
import ModelsPage from './Models';
import { ACCOUNTS, HOSTED, LOCAL, ok, stubRouter, type Answer } from './testing';

afterEach(() => vi.unstubAllGlobals());

const KIMI: AccountRow[] = [
  { id: 'kimi', label: 'Kimi', source: 'token', provider: 'kimi', base_url: 'https://api.kimi.ai/coding', active: 1, created_at: 40 },
  { id: 'kimi-two', label: 'Kimi two', source: 'token', provider: 'kimi', base_url: 'https://api.kimi.ai/coding', active: 0, created_at: 50 },
];

/// The catalogue as `GET /api/models` answers it: Anthropic's models first, one given an account of
/// its own, then Kimi Code's.
const MODELS: ModelRow[] = [
  { id: 'claude-haiku-4-5', provider: 'anthropic', display_name: 'claude-haiku-4-5', assigned: 'work', serving: 'work' },
  { id: 'claude-opus-5-5', provider: 'anthropic', display_name: 'claude-opus-5-5', assigned: null, serving: 'default' },
  { id: 'k3', provider: 'kimi', display_name: 'Kimi K3', assigned: null, serving: 'kimi' },
  { id: 'kimi-for-coding', provider: 'kimi', display_name: 'Kimi for Coding', assigned: null, serving: 'kimi' },
];

function show({ auth = LOCAL, models = MODELS, more = {} }: { auth?: typeof LOCAL; models?: ModelRow[]; more?: Record<string, Answer> } = {}) {
  let rows = models;
  const router = stubRouter({ '/api/auth': ok(auth), '/api/accounts': ok([...ACCOUNTS, ...KIMI]), '/api/models': () => json(rows), ...more });
  const view = renderScreen(<ModelsPage />, { path: '/settings/models', at: '/settings/models' });
  return { ...router, ...view, setRows: (next: ModelRow[]) => (rows = next) };
}

const card = (id: string) => {
  const found = document.querySelector<HTMLElement>(`[data-model="${id}"]`);
  if (!found) throw new Error(`no model ${id}`);
  return found;
};
const fact = (id: string, term: string) => within(card(id)).getByText(term).nextElementSibling;
/// A model's list box, opened, and the names of what it offers.
async function offered(id: string, user: ReturnType<typeof userEvent.setup>) {
  await user.click(within(card(id)).getByRole('combobox'));
  return within(card(id))
    .getAllByRole('option')
    .map((o) => o.textContent);
}

describe('Settings → Models', () => {
  it('ST-C122: lists the catalogue — each model by its provider’s name for it, its id, its provider and who serves it — and reads it as the page opens', async () => {
    const r = show();
    await waitFor(() => card('k3'));
    expect(within(card('k3')).getByRole('heading', { name: 'Kimi K3' })).toBeInTheDocument();
    expect(fact('k3', 'Id')).toHaveTextContent('k3');
    expect(fact('k3', 'Provider')).toHaveTextContent('Kimi Code');
    expect(fact('k3', 'Served by')).toHaveTextContent('Kimi, Kimi Code’s active account');
    expect(fact('claude-haiku-4-5', 'Provider')).toHaveTextContent('Anthropic');
    expect(fact('claude-haiku-4-5', 'Served by')).toHaveTextContent('work, assigned to it');
    expect(fact('claude-opus-5-5', 'Served by')).toHaveTextContent('me@example.com, Anthropic’s active account');
    // The accounts are read without a probe: nothing here needs a credential checked (ST-C6).
    expect(r.gets('/api/accounts').every((g) => g.query.get('probe') === null)).toBe(true);
    expect(r.gets('/api/models').length).toBeGreaterThan(0);
  });

  it('ST-C122: a failed read is said in the router’s words, never drawn as an empty list, and an empty catalogue says how a model comes to be listed', async () => {
    const first = show({ more: { '/api/models': unreachable } });
    expect(await screen.findByText('Could not read the models')).toBeInTheDocument();
    expect(screen.queryByText(/^No model yet/)).toBeNull();
    first.unmount();
    vi.unstubAllGlobals();
    show({ models: [] });
    expect(await screen.findByText(/^No model yet\. A Claude model is listed once a request has asked for it/)).toBeInTheDocument();
  });

  it('ST-C123: a model’s list box offers its provider’s active account first, then its own provider’s accounts that can sign here; on a hosted router only a pasted key, or the one it is assigned', async () => {
    const user = userEvent.setup();
    const first = show();
    await waitFor(() => card('k3'));
    await waitFor(() => expect(within(card('k3')).getByRole('combobox')).toBeEnabled());
    expect(await offered('k3', user)).toEqual(['Kimi Code’s active account (Kimi)', 'Kimi', 'Kimi two']);
    await user.keyboard('{Escape}');
    expect(await offered('claude-opus-5-5', user)).toEqual(['Anthropic’s active account (me@example.com)', 'me@example.com', 'work', 'side']);
    first.unmount();
    vi.unstubAllGlobals();
    // A hosted router signs with a pasted key only: the login and the profile are not offered.
    show({ auth: HOSTED });
    await waitFor(() => card('claude-opus-5-5'));
    await waitFor(() => expect(within(card('claude-opus-5-5')).getByRole('combobox')).toBeEnabled());
    expect(await offered('claude-opus-5-5', user)).toEqual(['Anthropic’s active account (me@example.com)', 'work']);
  });

  it('ST-C123, ST-C13: choosing an account posts {model, account}, and the box shows what the router reads back; the active account again posts null', async () => {
    const user = userEvent.setup();
    const r = show({ more: { 'POST /api/models/assign': ok({ ok: true }) } });
    await waitFor(() => expect(within(card('k3')).getByRole('combobox')).toBeEnabled());
    expect(within(card('k3')).getByRole('combobox')).toHaveTextContent('Kimi Code’s active account (Kimi)');
    r.setRows(MODELS.map((m) => (m.id === 'k3' ? { ...m, assigned: 'kimi-two', serving: 'kimi-two' } : m)));
    await offered('k3', user);
    await user.click(within(card('k3')).getByRole('option', { name: 'Kimi two' }));
    await waitFor(() => expect(r.posts('/api/models/assign')).toEqual([{ model: 'k3', account: 'kimi-two' }]));
    expect(await within(card('k3')).findByText('Kimi K3 goes to Kimi two now.')).toBeInTheDocument();
    await waitFor(() => expect(within(card('k3')).getByRole('combobox')).toHaveTextContent('Kimi two'));
    expect(fact('k3', 'Served by')).toHaveTextContent('Kimi two, assigned to it');

    await offered('k3', user);
    await user.click(within(card('k3')).getByRole('option', { name: 'Kimi Code’s active account (Kimi)' }));
    await waitFor(() => expect(r.posts('/api/models/assign')).toEqual([{ model: 'k3', account: 'kimi-two' }, { model: 'k3', account: null }]));
  });

  it('ST-C123: a refusal shows beside the list box in the router’s words, and the box keeps what the router reports', async () => {
    const user = userEvent.setup();
    const r = show({ more: { 'POST /api/models/assign': () => refusal('"kimi" is a Kimi Code account, and "claude-opus-5-5" is Anthropic\'s', 409) } });
    await waitFor(() => expect(within(card('claude-opus-5-5')).getByRole('combobox')).toBeEnabled());
    await offered('claude-opus-5-5', user);
    await user.click(within(card('claude-opus-5-5')).getByRole('option', { name: 'work' }));
    expect(await within(card('claude-opus-5-5')).findByText('"kimi" is a Kimi Code account, and "claude-opus-5-5" is Anthropic\'s')).toBeInTheDocument();
    expect(r.posts('/api/models/assign')).toEqual([{ model: 'claude-opus-5-5', account: 'work' }]);
    expect(within(card('claude-opus-5-5')).getByRole('combobox')).toHaveTextContent('Anthropic’s active account (me@example.com)');
  });
});
