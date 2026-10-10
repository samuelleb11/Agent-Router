// New for agent-router: Settings → Accounts, against the router's answers (docs/web-console-contracts.md,
// "Accounts: the list", "Accounts: editing", "Accounts: adding"), a Kimi Code account among them: a
// section per provider with the account its requests go to (ST-C125), and the provider asked first
// when one is added (ST-C124).
import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { AccountRow } from '~/data/settings';
import { useAccounts } from '~/data/settings';
import { json, refusal, renderScreen, unreachable } from '~/test-utils';
import AccountsPage from './Accounts';
import { BUILT_IN, HELD, USAGE_KEY } from './rules';
import { ACCOUNTS, HOSTED, LOCAL, ok, stubRouter, type Answer } from './testing';

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

const NO_FLOW = { phase: 'failed', url: null, label: '', account_id: null, email: null, error: null };

/// The page on a router answering these, with any answer overridden.
function show({ auth = LOCAL, accounts = ACCOUNTS, more = {} }: { auth?: typeof LOCAL; accounts?: AccountRow[]; more?: Record<string, Answer> } = {}) {
  let rows = accounts;
  const router = stubRouter({
    '/api/auth': ok(auth),
    '/api/accounts': () => json(rows),
    '/api/login/status': ok(NO_FLOW),
    ...more,
  });
  const view = renderScreen(<AccountsPage />, { path: '/settings/accounts', at: '/settings/accounts' });
  return { ...router, ...view, setRows: (next: AccountRow[]) => (rows = next) };
}

/// A Kimi Code account as `probe=1` answers it: a key the router holds, at Kimi Code's own address.
const KIMI: AccountRow = {
  id: 'kimi',
  label: 'Kimi',
  source: 'token',
  provider: 'kimi',
  base_url: 'https://api.kimi.ai/coding',
  email: null,
  org_name: null,
  subscription_type: null,
  active: 1,
  created_at: 40,
  usage_key: 0,
  ready: true,
  detail: 'a Kimi Code key',
  expires_at: 0,
  has_token: true,
};

const card = (name: string) => screen.getByLabelText(`Account ${name}`);
const cardOf = async (name: string) => screen.findByLabelText(`Account ${name}`);
const listReads = (r: ReturnType<typeof stubRouter>) => r.gets('/api/accounts').length;
/// A provider's section, by the name its heading gives it beside its mark (ST-C125).
const section = (provider: string) => screen.getByRole('region', { name: provider });
/// A provider's "Requests go to" list box.
const box = (provider: string) => within(section(provider)).getByRole('combobox', { name: 'Requests go to' });
/// What a provider's list box offers, opened, by name.
async function offered(provider: string, user: ReturnType<typeof userEvent.setup>) {
  await user.click(box(provider));
  return within(section(provider))
    .getAllByRole('option')
    .map((o) => o.textContent);
}
/// Pick an account in a provider's list box with plain events, for a spec on fake timers.
function pick(provider: string, name: string) {
  fireEvent.click(box(provider));
  fireEvent.click(within(section(provider)).getByRole('option', { name }));
}

describe('Accounts: the list', () => {
  it('ST-C19, ST-C3, ST-C125: each account is a card with every fact a desk row had, and Edit and Remove as buttons at rest; which one signs is a visible list box above the cards, and no card has Use', async () => {
    show();
    const work = await cardOf('work');
    const facts = within(work);
    expect(facts.getByText('Org').nextElementSibling).toHaveTextContent('—');
    expect(facts.getByText('Plan').nextElementSibling).toHaveTextContent('—');
    expect(facts.getByText('Source').nextElementSibling).toHaveTextContent('token');
    expect(facts.getByText('Credential').nextElementSibling).toHaveTextContent('sk-ant-oat01 (subscription OAuth)');
    expect(facts.getByText('Expires').nextElementSibling).toHaveTextContent('no expiry');
    for (const name of ['Edit', 'Remove']) {
      const button = facts.getByRole('button', { name });
      expect(button).toBeVisible();
      expect(button.closest('[hidden], .cds--overflow-menu-options, [role="menu"]')).toBeNull();
    }
    const active = card('me@example.com');
    expect(within(active).getByText('active')).toBeInTheDocument();
    expect(within(active).getByText('Org').nextElementSibling).toHaveTextContent('Example Org');
    expect(within(active).getByText('Plan').nextElementSibling).toHaveTextContent('max');
    expect(within(active).getByText('usage key')).toBeInTheDocument();
    // The account that signs is chosen in the provider's list box, visible at rest; no card has Use.
    expect(screen.queryByRole('button', { name: 'Use' })).toBeNull();
    expect(box('Anthropic')).toBeVisible();
    expect(box('Anthropic').closest('[hidden], .cds--overflow-menu-options, [role="menu"]')).toBeNull();
    // No overflow menu anywhere on the page.
    expect(document.querySelector('.cds--overflow-menu')).toBeNull();
  });

  it('ST-C125: one section per provider that has accounts, Anthropic’s first, each headed by its mark and name, with its own cards and a "Requests go to" list box showing its active account', async () => {
    const user = userEvent.setup();
    // The router lists the Kimi Code account first; Anthropic's section still leads.
    show({ accounts: [KIMI, ...ACCOUNTS, { ...KIMI, id: 'kimi-two', label: 'Kimi two', active: 0, created_at: 50 }] });
    await cardOf('Kimi');
    const page = screen.getByRole('region', { name: 'Accounts' });
    expect(within(page).getAllByRole('heading', { level: 3 }).map((h) => h.textContent)).toEqual(['Anthropic', 'Kimi Code']);
    for (const [name, provider] of [
      ['Anthropic', 'anthropic'],
      ['Kimi Code', 'kimi'],
    ] as const) {
      const heading = within(section(name)).getByRole('heading', { level: 3, name });
      // Its mark beside its name (SH-C56).
      expect(heading.querySelector(`[data-provider="${provider}"] [data-glyph="${provider}"]`)).not.toBeNull();
    }
    // Each section holds its own provider's cards, and only those.
    const cardsIn = (name: string) => [...section(name).querySelectorAll('[aria-label^="Account "]')].map((c) => c.getAttribute('aria-label'));
    expect(cardsIn('Anthropic')).toEqual(['Account me@example.com', 'Account work', 'Account side']);
    expect(cardsIn('Kimi Code')).toEqual(['Account Kimi', 'Account Kimi two']);
    // Each box shows its provider's active account, and offers its provider's accounts only.
    expect(box('Anthropic')).toHaveTextContent('me@example.com');
    expect(box('Kimi Code')).toHaveTextContent('Kimi');
    expect(await offered('Kimi Code', user)).toEqual(['Kimi', 'Kimi two']);
    await user.keyboard('{Escape}');
    expect(await offered('Anthropic', user)).toEqual(['me@example.com', 'work', 'side']);
    await user.keyboard('{Escape}');
    // Its helper text says where the provider's models go, and links to the Models page.
    expect(section('Kimi Code')).toHaveTextContent('Kimi Code’s models go to this account unless the Models page assigns a model elsewhere.');
    expect(within(section('Kimi Code')).getByRole('link', { name: 'Models' })).toHaveAttribute('href', '/settings/models');
    expect(box('Anthropic')).toHaveAccessibleDescription(/^Anthropic’s models go to this account unless the\s+Models\s+page assigns a model elsewhere\.$/);
  });

  it('ST-C125: a provider with no account has no section', async () => {
    show();
    await cardOf('work');
    expect(section('Anthropic')).toBeInTheDocument();
    expect(screen.queryByRole('region', { name: 'Kimi Code' })).toBeNull();
    expect(screen.getAllByRole('combobox', { name: 'Requests go to' })).toHaveLength(1);
  });

  it('ST-C119, ST-C121: each card names its provider; Org and Plan are an Anthropic account’s, a Kimi Code card shows its base URL, and each provider’s active account says whose requests it signs', async () => {
    show({ accounts: [...ACCOUNTS, KIMI] });
    const kimi = within(await cardOf('Kimi'));
    expect(kimi.getByText('Provider').nextElementSibling).toHaveTextContent('Kimi Code');
    expect(kimi.getByText('Base URL').nextElementSibling).toHaveTextContent('https://api.kimi.ai/coding');
    expect(kimi.queryByText('Org')).toBeNull();
    expect(kimi.queryByText('Plan')).toBeNull();
    expect(kimi.getByText('active')).toBeInTheDocument();
    expect(kimi.getByText('Signs every Kimi Code request now, but for a model assigned another account.')).toBeInTheDocument();
    const claude = within(card('me@example.com'));
    expect(claude.getByText('Provider').nextElementSibling).toHaveTextContent('Anthropic');
    expect(claude.getByText('Org').nextElementSibling).toHaveTextContent('Example Org');
    expect(claude.queryByText('Base URL')).toBeNull();
    expect(claude.getByText('Signs every Claude request now, but for a model assigned another account.')).toBeInTheDocument();
    expect(screen.getByText('4 accounts · me@example.com active · Kimi active for Kimi Code')).toBeInTheDocument();
  });

  it('ST-C20: a failed credential shows the router’s whole reason in the error tone; a row read without a check says "not checked"', async () => {
    show();
    const side = await cardOf('side');
    const state = within(side).getByText(/is not logged in/);
    expect(state).toHaveTextContent(ACCOUNTS[2]!.detail!);
    expect(state).toHaveAttribute('data-credential', 'failed');
  });

  it('ST-C20: rows that carry no verdict read "not checked", never ready or failed', async () => {
    const unchecked = ACCOUNTS.map(({ ready: _r, detail: _d, expires_at: _e, has_token: _h, ...rest }) => rest);
    show({ accounts: unchecked });
    const work = await cardOf('work');
    expect(within(work).getByText('not checked')).toHaveAttribute('data-credential', 'unchecked');
    expect(within(work).getByText('Expires').nextElementSibling).toHaveTextContent('—');
  });

  it('ST-C21: expiry reads "no expiry", a date, or "—" when it is not known', async () => {
    show();
    expect(within(await cardOf('work')).getByText('Expires').nextElementSibling).toHaveTextContent('no expiry');
    expect(within(card('side')).getByText('Expires').nextElementSibling).toHaveTextContent('—');
    expect(within(card('me@example.com')).getByText('Expires').nextElementSibling).not.toHaveTextContent(/no expiry|—/);
  });

  it('ST-C22: the heading counts the accounts and names the active one', async () => {
    show();
    expect(await screen.findByText('3 accounts · me@example.com active')).toBeInTheDocument();
  });

  it('ST-C23, ST-C4: the "usage key" tag and the "built in" marker carry their explanations as visible text, never only a title', async () => {
    show();
    const builtIn = await cardOf('me@example.com');
    expect(within(builtIn).getByText(USAGE_KEY)).toBeVisible();
    expect(within(builtIn).getByText('built in')).toBeInTheDocument();
    expect(within(builtIn).getByText(BUILT_IN)).toBeVisible();
    // Whatever carries a title says the same in its text: no explanation lives only there.
    for (const el of document.querySelectorAll('[title]')) expect(el.textContent).toContain(el.getAttribute('title') ?? '');
  });

  it('ST-C26, ST-C12: locally the built-in account offers no Remove and says why; on a hosted router it is removable', async () => {
    show();
    expect(within(await cardOf('me@example.com')).queryByRole('button', { name: 'Remove' })).toBeNull();
    expect(within(card('me@example.com')).getByText(BUILT_IN)).toBeInTheDocument();
    vi.unstubAllGlobals();
    document.body.innerHTML = '';
    show({ auth: HOSTED });
    expect(await within(await cardOf('me@example.com')).findByRole('button', { name: 'Remove' })).toBeInTheDocument();
    expect(screen.queryByText('built in')).toBeNull();
  });

  it('ST-C27, ST-C125: on a hosted router the list box offers no Claude Code login nor profile, and each one’s card says it cannot sign there in the router’s sentence', async () => {
    const user = userEvent.setup();
    const NOT_HERE = "this account is a Claude Code login on a developer's machine, which a hosted router cannot read — add an account with a `claude setup-token` value in the dashboard";
    const hosted: AccountRow[] = [
      { ...ACCOUNTS[0]!, active: 0, ready: false, detail: NOT_HERE, expires_at: 0 },
      { ...ACCOUNTS[1]!, active: 1 },
      { ...ACCOUNTS[2]!, ready: false, detail: NOT_HERE },
      { ...ACCOUNTS[1]!, id: 'spare', label: 'spare', active: 0 },
    ];
    show({ auth: HOSTED, accounts: hosted });
    for (const name of ['me@example.com', 'side']) {
      const c = await cardOf(name);
      expect(within(c).getByText(/^It cannot sign on this router:/)).toHaveTextContent(NOT_HERE);
    }
    expect(box('Anthropic')).toHaveTextContent('work');
    expect(await offered('Anthropic', user)).toEqual(['work', 'spare']);
  });

  it('ST-C125, ST-C24, ST-C25, ST-C13: picking an account in its provider’s list box activates it in one choice with no confirmation, the list re-reads and the box shows what it reads back; a refusal shows beside the box in the router’s words, and the list re-reads', async () => {
    const user = userEvent.setup();
    const r = show({ accounts: [...ACCOUNTS, KIMI], more: { 'POST /api/accounts/activate': ok({ ok: true, active: 'work' }) } });
    await cardOf('work');
    expect(box('Anthropic')).toHaveTextContent('me@example.com');
    const before = listReads(r);
    r.setRows([...ACCOUNTS.map((a) => ({ ...a, active: a.id === 'work' ? 1 : 0 })), KIMI]);
    await offered('Anthropic', user);
    await user.click(within(section('Anthropic')).getByRole('option', { name: 'work' }));
    expect(screen.queryByRole('dialog')).toBeNull();
    await waitFor(() => expect(r.posts('/api/accounts/activate')).toEqual([{ id: 'work' }]));
    await waitFor(() => expect(listReads(r)).toBeGreaterThan(before));
    await waitFor(() => expect(box('Anthropic')).toHaveTextContent('work'));
    expect(within(card('work')).getByText('active')).toBeInTheDocument();
    // The other provider's active account is where it was.
    expect(box('Kimi Code')).toHaveTextContent('Kimi');

    r.answers['POST /api/accounts/activate'] = () => refusal('no account with id "side"', 500);
    await waitFor(() => expect(box('Anthropic')).toBeEnabled());
    const again = listReads(r);
    await offered('Anthropic', user);
    await user.click(within(section('Anthropic')).getByRole('option', { name: 'side' }));
    const refused = await within(section('Anthropic')).findByText('no account with id "side"');
    // Beside the box, not on a card.
    expect(refused.closest('[aria-label^="Account "]')).toBeNull();
    expect(r.posts('/api/accounts/activate')).toEqual([{ id: 'work' }, { id: 'side' }]);
    await waitFor(() => expect(listReads(r)).toBeGreaterThan(again));
    // The box keeps what the router reports.
    expect(box('Anthropic')).toHaveTextContent('work');
  });

  it('ST-C125, ST-C121: picking a Kimi Code account activates it in Kimi Code’s own section, and Anthropic’s active account is left where it was', async () => {
    const user = userEvent.setup();
    const TWO: AccountRow = { ...KIMI, id: 'kimi-two', label: 'Kimi two', active: 0, created_at: 50 };
    const r = show({ accounts: [...ACCOUNTS, KIMI, TWO], more: { 'POST /api/accounts/activate': ok({ ok: true, active: 'kimi-two' }) } });
    await cardOf('Kimi two');
    r.setRows([...ACCOUNTS, { ...KIMI, active: 0 }, { ...TWO, active: 1 }]);
    await offered('Kimi Code', user);
    await user.click(within(section('Kimi Code')).getByRole('option', { name: 'Kimi two' }));
    await waitFor(() => expect(r.posts('/api/accounts/activate')).toEqual([{ id: 'kimi-two' }]));
    await waitFor(() => expect(box('Kimi Code')).toHaveTextContent('Kimi two'));
    expect(box('Anthropic')).toHaveTextContent('me@example.com');
    expect(screen.getByText('5 accounts · me@example.com active · Kimi two active for Kimi Code')).toBeInTheDocument();
  });

  it('ST-C28, ST-C18: Remove asks first in a danger modal that names the account, what goes, and the oldest one left that signs next; nothing is sent before', async () => {
    const rows: AccountRow[] = [{ ...ACCOUNTS[0]!, active: 0 }, { ...ACCOUNTS[1]!, active: 1 }, ACCOUNTS[2]!];
    const r = show({ accounts: rows, more: { 'POST /api/accounts/remove': ok() } });
    fireEvent.click(within(await cardOf('work')).getByRole('button', { name: 'Remove' }));
    const dialog = await screen.findByRole('dialog');
    expect(dialog).toHaveTextContent('Remove work from the router? Its stored credential and its claude.ai session key are deleted with it.');
    expect(dialog).toHaveTextContent('me@example.com, the oldest one left, signs from the next request');
    const confirm = within(dialog).getByRole('button', { name: 'Remove account' });
    expect(confirm.className).toMatch(/cds--btn--danger/);
    expect(r.posts('/api/accounts/remove')).toEqual([]);
    fireEvent.click(confirm);
    await waitFor(() => expect(r.posts('/api/accounts/remove')).toEqual([{ id: 'work' }]));
  });

  it('ST-C25: a refused Remove shows the router’s sentence on that account’s card', async () => {
    const r = show({ more: { 'POST /api/accounts/remove': () => refusal('the secrets file is not writable', 500) } });
    fireEvent.click(within(await cardOf('work')).getByRole('button', { name: 'Remove' }));
    fireEvent.click(within(await screen.findByRole('dialog')).getByRole('button', { name: 'Remove account' }));
    expect(await within(card('work')).findByText('the secrets file is not writable')).toBeInTheDocument();
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(r.posts('/api/accounts/remove')).toEqual([{ id: 'work' }]);
  });

  it('ST-C5, ST-C6: the page asks for every credential to be checked as it opens, after its writes and every 30 s; a read made anywhere else asks for none', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const r = show({ more: { 'POST /api/accounts/activate': ok() } });
    const first = r;
    await cardOf('work');
    expect(r.gets('/api/accounts').every((g) => g.query.get('probe') === '1')).toBe(true);
    const opened = listReads(r);
    pick('Anthropic', 'work');
    await waitFor(() => expect(listReads(r)).toBeGreaterThan(opened));
    const written = listReads(r);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(30_000);
    });
    await waitFor(() => expect(listReads(r)).toBeGreaterThan(written));
    expect(r.gets('/api/accounts').every((g) => g.query.get('probe') === '1')).toBe(true);

    first.unmount();
    // Any other reader of the list, the palette's or the Overview's, resolves no credential.
    function Elsewhere() {
      const list = useAccounts({ probe: false });
      return <span>{list.value ? `${list.value.length} read` : 'reading'}</span>;
    }
    const other = stubRouter({ '/api/accounts': ok(ACCOUNTS) });
    renderScreen(<Elsewhere />);
    expect(await screen.findByText('3 read')).toBeInTheDocument();
    expect(other.gets('/api/accounts').map((g) => g.query.toString())).toEqual(['']);
  });
});

describe('Accounts: editing', () => {
  const edit = async (name: string) => {
    fireEvent.click(within(await cardOf(name)).getByRole('button', { name: 'Edit' }));
    return within(card(name)).getByRole('form', { name: `Edit ${name}` });
  };

  it('ST-C29: Edit opens its form inside the account’s own card', async () => {
    show();
    const form = await edit('work');
    expect(card('work')).toContainElement(form);
    expect(screen.queryByRole('dialog')).toBeNull();
  });

  it('ST-C14: Enter saves, sending only what changed; a refusal keeps it open with everything typed; it closes once the save reads back; Escape cancels', async () => {
    const user = userEvent.setup();
    const r = show({ more: { 'POST /api/accounts/edit': () => refusal('the store is locked', 500) } });
    const form = await edit('work');
    const identity = within(form).getByLabelText('Identity');
    await user.type(identity, 'me@work.example{Enter}');
    await waitFor(() => expect(r.posts('/api/accounts/edit')).toEqual([{ id: 'work', email: 'me@work.example' }]));
    expect(await within(form).findByText('the store is locked')).toBeInTheDocument();
    expect(identity).toHaveValue('me@work.example');

    r.answers['POST /api/accounts/edit'] = ok({ ok: true, id: 'work' });
    r.setRows(ACCOUNTS.map((a) => (a.id === 'work' ? { ...a, email: 'me@work.example' } : a)));
    await user.type(identity, '{Enter}');
    await waitFor(() => expect(screen.queryByRole('form', { name: 'Edit work' })).toBeNull());
    expect(r.posts('/api/accounts/edit')).toHaveLength(2);

    const again = await edit('me@work.example');
    await user.type(within(again).getByLabelText('Name'), 'x');
    await user.keyboard('{Escape}');
    expect(screen.queryByRole('form', { name: 'Edit me@work.example' })).toBeNull();
    expect(r.posts('/api/accounts/edit')).toHaveLength(2);
  });

  it('ST-C11: the editor checks the router’s rules beside each field before sending, with the router’s sentences', async () => {
    const r = show({ more: { 'POST /api/accounts/edit': ok(), 'POST /api/usage-token': ok() } });
    const form = await edit('work');
    fireEvent.change(within(form).getByLabelText('Identity'), { target: { value: 'bob' } });
    fireEvent.change(within(form).getByLabelText('Credential'), { target: { value: 'sk-ant-sid01-cookie' } });
    fireEvent.change(within(form).getByLabelText('claude.ai session key'), { target: { value: 'sk-ant-oat01-token' } });
    fireEvent.submit(form);
    expect(await within(form).findByText('"bob" is not an email address')).toBeInTheDocument();
    expect(within(form).getByText('that is a claude.ai session key — it belongs in the session-key field, and cannot sign API requests')).toBeInTheDocument();
    expect(within(form).getByText('that does not look like a claude.ai session key (expected sk-ant-sid…)')).toBeInTheDocument();
    expect(r.posts('/api/accounts/edit')).toEqual([]);
    expect(r.posts('/api/usage-token')).toEqual([]);
  });

  it('ST-C30: the credential field appears only for a token account', async () => {
    show();
    expect(within(await edit('work')).getByLabelText('Credential')).toBeInTheDocument();
    expect(within(await edit('side')).queryByLabelText('Credential')).toBeNull();
    expect(within(await edit('me@example.com')).queryByLabelText('Credential')).toBeNull();
  });

  it('ST-C31, ST-C32: a Claude Code login’s identity is shown and cannot be typed over, with the reason; a token’s says setting it labels its traffic', async () => {
    show();
    const builtIn = await edit('me@example.com');
    const identity = within(builtIn).getByLabelText('Identity');
    expect(identity).toHaveValue('me@example.com');
    expect(identity).toHaveAttribute('readonly');
    expect(builtIn).toHaveTextContent('Identity is read from the Claude Code login on this machine and refreshes on every request');
    const token = await edit('work');
    expect(token).toHaveTextContent('setting the identity here is what labels this account’s traffic in the charts');
    expect(within(await edit('side')).getByLabelText('Identity')).not.toHaveAttribute('readonly');
  });

  it('ST-C33: replacing a token keeps the account’s id, and the field says so', async () => {
    const r = show({ more: { 'POST /api/accounts/edit': ok() } });
    const form = await edit('work');
    expect(form).toHaveTextContent('The account keeps its id, so everything already recorded against it stays intact.');
    fireEvent.change(within(form).getByLabelText('Credential'), { target: { value: 'sk-ant-oat01-fresh' } });
    fireEvent.submit(form);
    await waitFor(() => expect(r.posts('/api/accounts/edit')).toEqual([{ id: 'work', token: 'sk-ant-oat01-fresh' }]));
  });

  it('ST-C34: a blank field means keep, and sends nothing for that field', async () => {
    const r = show({ more: { 'POST /api/accounts/edit': ok(), 'POST /api/usage-token': ok() } });
    const form = await edit('work');
    fireEvent.change(within(form).getByLabelText('Name'), { target: { value: '' } });
    // Nothing typed but a blank: nothing to send.
    expect(within(form).getByRole('button', { name: 'Save' })).toBeDisabled();
    fireEvent.change(within(form).getByLabelText('claude.ai session key'), { target: { value: 'sk-ant-sid01-key' } });
    fireEvent.submit(form);
    await waitFor(() => expect(r.posts('/api/usage-token')).toEqual([{ account_id: 'work', token: 'sk-ant-sid01-key' }]));
    expect(r.posts('/api/accounts/edit')).toEqual([]);
  });

  it('ST-C35: the editor sends the account’s id and what was typed, never a copy of the row taken when it opened', async () => {
    const r = show({ more: { 'POST /api/accounts/edit': ok() } });
    const form = await edit('work');
    // Renamed elsewhere while the editor is open; the next poll brings it, and the untouched name follows.
    r.setRows(ACCOUNTS.map((a) => (a.id === 'work' ? { ...a, label: 'renamed' } : a)));
    await act(async () => {
      await r.client.invalidateQueries();
    });
    await waitFor(() => expect(within(form).getByLabelText('Name')).toHaveValue('renamed'));
    fireEvent.change(within(form).getByLabelText('Identity'), { target: { value: 'me@work.example' } });
    fireEvent.submit(form);
    await waitFor(() => expect(r.posts('/api/accounts/edit')).toEqual([{ id: 'work', email: 'me@work.example' }]));
  });

  it('ST-C36: the account’s fields and its session key each report on their own; a refused key stays beside its field while the saved fields say saved', async () => {
    const r = show({
      more: {
        'POST /api/accounts/edit': ok(),
        'POST /api/usage-token': () => refusal('the secrets file is not writable'),
      },
    });
    const form = await edit('work');
    const reads = listReads(r);
    fireEvent.change(within(form).getByLabelText('Identity'), { target: { value: 'me@work.example' } });
    fireEvent.change(within(form).getByLabelText('claude.ai session key'), { target: { value: 'sk-ant-sid01-key' } });
    fireEvent.submit(form);
    expect(await within(form).findByText('the secrets file is not writable')).toBeInTheDocument();
    expect(within(form).getByText('The session key was refused')).toBeInTheDocument();
    expect(within(form).getByText('Saved the identity.')).toBeInTheDocument();
    expect(within(form).getByLabelText('claude.ai session key')).toHaveValue('sk-ant-sid01-key');
    await waitFor(() => expect(listReads(r)).toBeGreaterThan(reads));
  });

  it('ST-C37: Remove stored key is offered only while a session key is stored', async () => {
    show();
    expect(within(await edit('me@example.com')).getByRole('button', { name: 'Remove stored key' })).toBeInTheDocument();
    expect(within(await edit('work')).queryByRole('button', { name: 'Remove stored key' })).toBeNull();
  });

  it('ST-C18: removing an account’s session key asks first, naming what goes and what follows', async () => {
    const r = show({ more: { 'POST /api/usage-token': ok({ ok: true, cleared: true }) } });
    fireEvent.click(within(await edit('me@example.com')).getByRole('button', { name: 'Remove stored key' }));
    const dialog = await screen.findByRole('dialog');
    expect(dialog).toHaveTextContent('Its usage is no longer read from claude.ai until a new key is stored here');
    expect(r.posts('/api/usage-token')).toEqual([]);
    fireEvent.click(within(dialog).getByRole('button', { name: 'Remove stored key' }));
    await waitFor(() => expect(r.posts('/api/usage-token')).toEqual([{ account_id: 'default', clear: true }]));
  });

  it('ST-C17, ST-C16: a stored secret shows as held — dots and words — never its value, and every secret field is a password field the browser neither fills nor spellchecks', async () => {
    show();
    const builtIn = await edit('me@example.com');
    const key = within(builtIn).getByLabelText('claude.ai session key');
    expect(key).toHaveValue('');
    expect(key).toHaveAttribute('placeholder', HELD);
    expect(builtIn).toHaveTextContent('Stored; leave it blank to keep it');
    const token = await edit('work');
    const credential = within(token).getByLabelText('Credential');
    expect(credential).toHaveValue('');
    expect(credential).toHaveAttribute('placeholder', HELD);
    expect(token).toHaveTextContent('Stored; leave it blank to keep it.');
    for (const field of [key, credential, within(token).getByLabelText('claude.ai session key')]) {
      expect(field).toHaveAttribute('type', 'password');
      expect(field).toHaveAttribute('autocomplete', 'new-password');
      expect(field).toHaveAttribute('spellcheck', 'false');
    }
  });

  it('ST-C120: a Kimi Code key rotates by Kimi Code’s rule, with the Kimi Code console’s words, and offers no claude.ai session key', async () => {
    const r = show({ accounts: [...ACCOUNTS, { ...KIMI, has_token: false }], more: { 'POST /api/accounts/edit': ok({ ok: true, id: 'kimi' }) } });
    const form = await edit('Kimi');
    const key = within(form).getByLabelText('Credential');
    expect(key).toHaveAttribute('placeholder', 'the key from the Kimi Code console');
    expect(form).toHaveTextContent('A fresh key from the Kimi Code console');
    expect(form).not.toHaveTextContent('claude setup-token');
    expect(within(form).queryByLabelText('claude.ai session key')).toBeNull();
    fireEvent.change(key, { target: { value: 'sk-ant-oat01-wrong-provider' } });
    fireEvent.submit(form);
    expect(await within(form).findByText('that is an Anthropic token — a Kimi Code account takes a key from the Kimi Code console')).toBeInTheDocument();
    expect(r.posts('/api/accounts/edit')).toEqual([]);
    // A Kimi Code key is not held to sk-ant-.
    fireEvent.change(key, { target: { value: 'sk-kimi-rotated' } });
    fireEvent.submit(form);
    await waitFor(() => expect(r.posts('/api/accounts/edit')).toEqual([{ id: 'kimi', token: 'sk-kimi-rotated' }]));
  });

  it('ST-C38: the session-key field says what the key is for, and locally that it lives in a Keychain entry apart from the credential', async () => {
    show();
    const local = await edit('work');
    expect(local).toHaveTextContent('Copied from your claude.ai browser cookie. Used only to read usage');
    expect(local).toHaveTextContent('never sent to api.anthropic.com and never signs a request');
    expect(local).toHaveTextContent('separate Keychain entry');
    vi.unstubAllGlobals();
    document.body.innerHTML = '';
    show({ auth: HOSTED });
    const hosted = await edit('work');
    expect(hosted).toHaveTextContent('never signs a request');
    expect(hosted).not.toHaveTextContent('Keychain');
  });

  it('ST-C15: a poll never rebuilds the page under the reader: an open editor, a half-typed value and the focus survive it', async () => {
    const r = show();
    const form = await edit('work');
    const name = within(form).getByLabelText('Name');
    fireEvent.change(name, { target: { value: 'half-typ' } });
    name.focus();
    const reads = listReads(r);
    r.setRows(ACCOUNTS.map((a) => (a.id === 'side' ? { ...a, subscription_type: 'max' } : a)));
    await act(async () => {
      await r.client.invalidateQueries();
    });
    await waitFor(() => expect(listReads(r)).toBeGreaterThan(reads));
    await waitFor(() => expect(within(card('side')).getByText('Plan').nextElementSibling).toHaveTextContent('max'));
    expect(screen.getByRole('form', { name: 'Edit work' })).toBe(form);
    expect(within(form).getByLabelText('Name')).toHaveValue('half-typ');
    expect(document.activeElement).toBe(within(form).getByLabelText('Name'));
  });
});

describe('Accounts: adding', () => {
  const open = async () => {
    fireEvent.click(await screen.findByRole('button', { name: 'Add account' }));
    return screen.getByRole('form', { name: 'Add an account' });
  };
  const choose = (form: HTMLElement, way: RegExp) => fireEvent.click(within(form).getByRole('radio', { name: way }));

  it('ST-C39, ST-C3: a visible Add account button opens the form in place, not in a dialog', async () => {
    show();
    const add = await screen.findByRole('button', { name: 'Add account' });
    expect(add).toBeVisible();
    const form = await open();
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(screen.getByRole('region', { name: 'Accounts' })).toContainElement(form);
  });

  it('ST-C40, ST-C43, ST-C124: each of Anthropic’s ways is a choice carrying its explanation; locally sign-in comes first and is chosen, and its button reads Start sign-in', async () => {
    show();
    const form = await open();
    const radios = within(within(form).getByRole('group', { name: 'How to add it' })).getAllByRole('radio') as HTMLInputElement[];
    const titles = ['Sign in from here (recommended)', 'Paste a token I already have', 'Separate Claude Code profile'];
    expect(radios).toHaveLength(3);
    radios.forEach((r, i) => expect(r.labels?.[0]?.textContent?.startsWith(titles[i]!), titles[i]).toBe(true));
    // Each carries its own explanation, in its own choice.
    expect(radios[1]!.labels?.[0]).toHaveTextContent('The value claude setup-token prints');
    expect(radios[2]!.labels?.[0]).toHaveTextContent('The router creates an isolated profile directory');
    expect(radios[0]).toBeChecked();
    expect(form).toHaveTextContent('The router runs claude setup-token for you and issues a long-lived (one-year) credential, so it never needs refreshing.');
    expect(form).toHaveTextContent('Sign in as the account you want to add — not the one you are already using.');
    expect(within(form).getByRole('button', { name: 'Start sign-in' })).toBeInTheDocument();
    choose(form, /^Paste a token/);
    expect(within(form).getByRole('button', { name: 'Add account' })).toBeInTheDocument();
  });

  it('ST-C41: a name is required before anything is sent, and the form says so beside the Name field', async () => {
    const r = show({ more: { 'POST /api/login/start': ok({ phase: 'starting' }) } });
    const form = await open();
    fireEvent.submit(form);
    expect(await within(form).findByText('Give the account a name first.')).toBeInTheDocument();
    expect(r.posts('/api/login/start')).toEqual([]);
  });

  it('ST-C42: a name whose id is an existing account’s is said beside the Name field, and nothing is sent', async () => {
    const r = show({ more: { 'POST /api/accounts': ok({ id: 'work', ready: true }) } });
    const form = await open();
    choose(form, /^Paste a token/);
    fireEvent.change(within(form).getByLabelText('Name'), { target: { value: 'Work' } });
    fireEvent.change(within(form).getByLabelText('Token'), { target: { value: 'sk-ant-oat01-new' } });
    expect(await within(form).findByText(/That name is the id “work”, which work already has: adding it would replace that account/)).toBeInTheDocument();
    fireEvent.submit(form);
    await act(async () => {});
    expect(r.posts('/api/accounts')).toEqual([]);
  });

  it('ST-C48, ST-C11: the token field says how to get a token and that a session key will not authenticate; its prefix rules are checked before sending', async () => {
    const r = show({ more: { 'POST /api/accounts': ok({ id: 'new', ready: true }) } });
    const form = await open();
    choose(form, /^Paste a token/);
    expect(form).toHaveTextContent('Run claude setup-token in a terminal that is logged into the other account');
    expect(form).toHaveTextContent('is a browser cookie, not an API credential, and will not authenticate');
    fireEvent.change(within(form).getByLabelText('Name'), { target: { value: 'new' } });
    fireEvent.change(within(form).getByLabelText('Token'), { target: { value: 'sk-ant-sid01-cookie' } });
    fireEvent.submit(form);
    expect(await within(form).findByText('that is a claude.ai session key — it belongs in the session-key field, and cannot sign API requests')).toBeInTheDocument();
    expect(r.posts('/api/accounts')).toEqual([]);
    vi.unstubAllGlobals();
    document.body.innerHTML = '';
    show({ auth: HOSTED });
    expect(await open()).toHaveTextContent('in a terminal on any machine, sign in as the account to add');
  });

  it('ST-C49: a token account named with an email address is told the name becomes its identity', async () => {
    show();
    const form = await open();
    choose(form, /^Paste a token/);
    fireEvent.change(within(form).getByLabelText('Name'), { target: { value: 'new@example.com' } });
    expect(form).toHaveTextContent('This name is an email address, so it becomes the account’s identity');
  });

  it('ST-C50: a pasted token stays in its field beside a refusal, and leaves once the router has stored it', async () => {
    const r = show({ more: { 'POST /api/accounts': () => refusal('the secrets file is not writable') } });
    const form = await open();
    choose(form, /^Paste a token/);
    fireEvent.change(within(form).getByLabelText('Name'), { target: { value: 'new' } });
    fireEvent.change(within(form).getByLabelText('Token'), { target: { value: 'sk-ant-oat01-pasted' } });
    fireEvent.submit(form);
    expect(await within(form).findByText('the secrets file is not writable')).toBeInTheDocument();
    expect(within(form).getByLabelText('Token')).toHaveValue('sk-ant-oat01-pasted');
    r.answers['POST /api/accounts'] = ok({ id: 'new', ready: true });
    fireEvent.submit(form);
    expect(await screen.findByText('Added new.')).toBeInTheDocument();
    expect(screen.queryByDisplayValue('sk-ant-oat01-pasted')).toBeNull();
    expect(r.posts('/api/accounts')).toEqual([
      { label: 'new', source: 'token', token: 'sk-ant-oat01-pasted' },
      { label: 'new', source: 'token', token: 'sk-ant-oat01-pasted' },
    ]);
  });

  it('ST-C124: Add account asks the provider first — Anthropic and Kimi Code, each a tile with its mark, its name and its explanation — and Anthropic is chosen as the form opens', async () => {
    show();
    const form = await open();
    const providers = within(form).getByRole('group', { name: 'Provider' });
    const radios = within(providers).getAllByRole('radio') as HTMLInputElement[];
    expect(radios).toHaveLength(2);
    const [anthropic, kimi] = radios as [HTMLInputElement, HTMLInputElement];
    expect(anthropic.labels?.[0]?.textContent?.startsWith('Anthropic')).toBe(true);
    expect(kimi.labels?.[0]?.textContent?.startsWith('Kimi Code')).toBe(true);
    // Each by the shared mark beside its name (SH-C56).
    expect(anthropic.labels?.[0]?.querySelector('[data-provider="anthropic"] [data-glyph="anthropic"]')).not.toBeNull();
    expect(kimi.labels?.[0]?.querySelector('[data-provider="kimi"] [data-glyph="kimi"]')).not.toBeNull();
    expect(anthropic.labels?.[0]).toHaveTextContent('Claude’s models, signed by a Claude subscription.');
    expect(kimi.labels?.[0]).toHaveTextContent('Kimi’s models (k3, kimi-for-coding), signed by a key from the Kimi Code console.');
    expect(anthropic).toBeChecked();
    expect(kimi).not.toBeChecked();
    // The provider is asked before the way.
    const ways = within(form).getByRole('group', { name: 'How to add it' });
    expect(providers.compareDocumentPosition(ways) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    // Kimi Code is a provider, not one of Anthropic's ways.
    expect(within(ways).queryByRole('radio', { name: /^Kimi Code/ })).toBeNull();
  });

  it('ST-C124, ST-C52, ST-C101: on a hosted router, and on one that has not said what it is, Anthropic’s one way is no choice — the setup token’s field, with its explanation as helper text', async () => {
    show({ auth: HOSTED });
    const form = await open();
    expect(within(form).getByRole('radio', { name: /^Anthropic/ })).toBeChecked();
    expect(within(form).queryByRole('group', { name: 'How to add it' })).toBeNull();
    // The providers are the only choice left.
    expect(within(form).getAllByRole('radio')).toHaveLength(2);
    expect(within(form).getByLabelText('Token')).toHaveAccessibleDescription(/claude setup-token.*on any machine.*will not authenticate/);
    expect(within(form).getByRole('button', { name: 'Add account' })).toBeInTheDocument();
    expect(form).not.toHaveTextContent('Sign in from here');
    expect(form).not.toHaveTextContent('Separate Claude Code profile');
    vi.unstubAllGlobals();
    document.body.innerHTML = '';
    show({ more: { '/api/auth': () => new Promise<Response>(() => {}) } });
    const unknown = await open();
    expect(within(unknown).getByRole('radio', { name: /^Anthropic/ })).toBeChecked();
    expect(within(unknown).queryByRole('group', { name: 'How to add it' })).toBeNull();
    expect(within(unknown).getByLabelText('Token')).toHaveAccessibleDescription(/claude setup-token/);
    expect(unknown).not.toHaveTextContent('Sign in from here');
  });

  it('ST-C124, ST-C117: Kimi Code goes straight to its name, key and base URL, with no way to choose, on a local router as on a hosted one', async () => {
    show();
    const form = await open();
    choose(form, /^Kimi Code/);
    expect(within(form).queryByRole('group', { name: 'How to add it' })).toBeNull();
    expect(within(form).getByLabelText('Name')).toBeInTheDocument();
    expect(within(form).getByLabelText('Key')).toBeInTheDocument();
    expect(within(form).getByLabelText('Base URL (optional)')).toBeInTheDocument();
    expect(within(form).queryByLabelText('Token')).toBeNull();
    expect(within(form).queryByRole('button', { name: 'Start sign-in' })).toBeNull();
    expect(within(form).getByRole('button', { name: 'Add account' })).toBeInTheDocument();
    // Back to Anthropic: its ways again, sign-in still the one chosen.
    choose(form, /^Anthropic/);
    expect(within(form).getByRole('radio', { name: /^Sign in from here/ })).toBeChecked();
    expect(within(form).queryByLabelText('Key')).toBeNull();
    vi.unstubAllGlobals();
    document.body.innerHTML = '';
    show({ auth: HOSTED });
    const hosted = await open();
    choose(hosted, /^Kimi Code/);
    expect(within(hosted).queryByLabelText('Token')).toBeNull();
    expect(within(hosted).getByLabelText('Key')).toBeInTheDocument();
  });

  it('ST-C117, ST-C118: Kimi Code takes a key and an optional base URL, Kimi Code’s own when blank, checked by its own rules before it is sent as a Kimi Code token account', async () => {
    const r = show({ more: { 'POST /api/accounts': ok({ id: 'kimi', ready: true }) } });
    const form = await open();
    expect((within(form).getByRole('radio', { name: /^Kimi Code/ }) as HTMLInputElement).labels?.[0]).toHaveTextContent('signed by a key from the Kimi Code console');
    choose(form, /^Kimi Code/);
    expect(within(form).queryByLabelText('Token')).toBeNull();
    const url = within(form).getByLabelText('Base URL (optional)');
    expect(url).toHaveAttribute('placeholder', 'https://api.kimi.ai/coding');
    expect(form).toHaveTextContent('Left blank, Kimi Code’s own: https://api.kimi.ai/coding.');
    fireEvent.change(within(form).getByLabelText('Name'), { target: { value: 'kimi' } });
    // Nothing given, an Anthropic token, a plain-http address: each said beside its field, nothing sent.
    fireEvent.submit(form);
    expect(await within(form).findByText('a token is required for this account type')).toBeInTheDocument();
    fireEvent.change(within(form).getByLabelText('Key'), { target: { value: 'sk-ant-oat01-claude' } });
    fireEvent.change(url, { target: { value: 'http://api.kimi.ai/coding' } });
    fireEvent.submit(form);
    expect(await within(form).findByText('that is an Anthropic token — a Kimi Code account takes a key from the Kimi Code console')).toBeInTheDocument();
    expect(within(form).getByText('"http://api.kimi.ai/coding" is not an https:// address')).toBeInTheDocument();
    expect(r.posts('/api/accounts')).toEqual([]);
    // A key that is not sk-ant- is Kimi Code's to judge, and a blank address is its own.
    fireEvent.change(within(form).getByLabelText('Key'), { target: { value: 'sk-kimi-0123' } });
    fireEvent.change(url, { target: { value: '' } });
    fireEvent.submit(form);
    expect(await screen.findByText('Added kimi.')).toBeInTheDocument();
    expect(r.posts('/api/accounts')).toEqual([{ label: 'kimi', source: 'token', provider: 'kimi', token: 'sk-kimi-0123' }]);
  });

  it('ST-C117: a base URL typed for Kimi Code is sent with it', async () => {
    const r = show({ more: { 'POST /api/accounts': ok({ id: 'kimi-china', ready: true }) } });
    const form = await open();
    choose(form, /^Kimi Code/);
    fireEvent.change(within(form).getByLabelText('Name'), { target: { value: 'kimi china' } });
    fireEvent.change(within(form).getByLabelText('Key'), { target: { value: 'sk-kimi-0123' } });
    fireEvent.change(within(form).getByLabelText('Base URL (optional)'), { target: { value: 'https://api.kimi.com/coding/' } });
    fireEvent.submit(form);
    await waitFor(() =>
      expect(r.posts('/api/accounts')).toEqual([{ label: 'kimi china', source: 'token', provider: 'kimi', token: 'sk-kimi-0123', base_url: 'https://api.kimi.com/coding/' }]),
    );
  });

  it('ST-C44: the sign-in page never opens by itself; the whole link is shown, opened in a new tab with no opener or referrer, and copying says when it failed', async () => {
    const URL_ = 'https://claude.ai/oauth/authorize?code=true&client_id=abc&redirect_uri=http%3A%2F%2Flocalhost%3A54545%2Fcallback&state=xyz';
    const opened = vi.spyOn(window, 'open').mockImplementation(() => null);
    const waiting = { phase: 'awaiting_sign_in', url: URL_, label: 'work2', account_id: null, email: null, error: null };
    show({ more: { 'POST /api/login/start': ok(waiting), '/api/login/status': ok(waiting) } });
    const form = await open();
    fireEvent.change(within(form).getByLabelText('Name'), { target: { value: 'work2' } });
    fireEvent.submit(form);
    const link = await within(form).findByRole('link', { name: 'Open sign-in page' });
    expect(link).toHaveAttribute('href', URL_);
    expect(link).toHaveAttribute('target', '_blank');
    expect(link.getAttribute('rel')?.split(/\s+/)).toEqual(expect.arrayContaining(['noopener', 'noreferrer']));
    expect(within(form).getByLabelText('The sign-in link')).toHaveTextContent(URL_);
    expect(opened).not.toHaveBeenCalled();
    // The clipboard refuses, as it does in a sandboxed frame and over plain HTTP on the LAN.
    const writeText = vi.fn(async (_text: string): Promise<void> => {
      throw new DOMException('Write permission denied.', 'NotAllowedError');
    });
    Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true });
    try {
      fireEvent.click(within(form).getByRole('button', { name: 'Copy link' }));
      expect(await within(form).findByText('Copying failed — select the link and copy it by hand.')).toBeInTheDocument();
      writeText.mockImplementation(async () => {});
      fireEvent.click(within(form).getByRole('button', { name: 'Copy link' }));
      expect(await within(form).findByText('Copied')).toBeInTheDocument();
      expect(writeText).toHaveBeenCalledWith(URL_);
    } finally {
      Reflect.deleteProperty(navigator, 'clipboard');
      opened.mockRestore();
    }
  });

  it('ST-C45: a sign-in says its stage, reading the status every 1.5 s, letting one failed read pass, and stops reading when it ends', async () => {
    vi.useFakeTimers();
    const URL_ = 'https://claude.ai/oauth/authorize?x=1';
    const statuses: (() => Response | Promise<Response>)[] = [
      () => json(NO_FLOW),
      () => json({ phase: 'awaiting_sign_in', url: URL_, label: 'new' }),
      unreachable,
      () => json({ phase: 'awaiting_sign_in', url: URL_, label: 'new' }),
      () => json({ phase: 'done', url: URL_, label: 'new', email: 'new@example.com' }),
    ];
    let i = 0;
    const r = show({ more: { 'POST /api/login/start': ok({ phase: 'starting', label: 'new' }), '/api/login/status': () => (statuses[Math.min(i++, statuses.length - 1)] as () => Response)() } });
    const step = async (ms: number) => {
      await act(async () => {
        await vi.advanceTimersByTimeAsync(ms);
      });
    };
    await step(0);
    fireEvent.click(screen.getByRole('button', { name: 'Add account' }));
    await step(0);
    const form = screen.getByRole('form', { name: 'Add an account' });
    fireEvent.change(within(form).getByLabelText('Name'), { target: { value: 'new' } });
    fireEvent.submit(form);
    await step(0);
    expect(form).toHaveTextContent('Waiting for you to sign in…');
    const reads = () => r.gets('/api/login/status').length;
    expect(reads()).toBe(2);
    await step(1500);
    expect(reads()).toBe(3);
    // One failed read is let pass: nothing is said about it.
    expect(form).not.toHaveTextContent('Could not read the sign-in’s progress');
    expect(form).toHaveTextContent('Waiting for you to sign in…');
    await step(1500);
    expect(reads()).toBe(4);
    await step(1500);
    expect(reads()).toBe(5);
    expect(screen.getByText('Signed in as new@example.com. The account is ready.')).toBeInTheDocument();
    await step(6000);
    expect(reads()).toBe(5);
  });

  it('ST-C46, ST-C47: a sign-in already running is picked up as the form opens, with its link and stage, no second start is offered, and its terminal is never shown', async () => {
    const TAIL = 'Paste code here if prompted > TERMINAL-TAIL-TEXT';
    const running = { phase: 'awaiting_sign_in', url: 'https://claude.ai/oauth/authorize?resume=1', label: 'earlier', account_id: null, email: null, error: null, tail: TAIL };
    show({ more: { '/api/login/status': ok(running) } });
    const form = await open();
    expect(await within(form).findByText('Waiting for you to sign in…')).toBeInTheDocument();
    expect(within(form).getByRole('link', { name: 'Open sign-in page' })).toHaveAttribute('href', running.url);
    expect(within(form).queryByRole('button', { name: 'Start sign-in' })).toBeNull();
    expect(document.body.textContent).not.toContain('TERMINAL-TAIL-TEXT');
  });

  it('ST-C46: a status that names no flow is no flow, never shown as a failure', async () => {
    show();
    const form = await open();
    await act(async () => {});
    expect(within(form).getByRole('button', { name: 'Start sign-in' })).toBeInTheDocument();
    expect(form).not.toHaveTextContent(/did not complete|failed/i);
  });

  it('ST-C51: a profile shows the command to run in a copyable block, says the page will notice, re-checks every 3 s until logged in, then says who and closes', async () => {
    vi.useFakeTimers();
    const COMMAND = 'CLAUDE_CONFIG_DIR=/Users/me/.agent-router/accounts/second claude auth login';
    let probes = 0;
    const r = show({
      more: {
        'POST /api/accounts': ok({ id: 'second', ready: false, command: COMMAND, profile_dir: '/Users/me/.agent-router/accounts/second' }),
        'POST /api/accounts/probe': () => json(++probes < 2 ? { loggedIn: false } : { loggedIn: true, email: 'second@example.com' }),
      },
    });
    const step = async (ms: number) => {
      await act(async () => {
        await vi.advanceTimersByTimeAsync(ms);
      });
    };
    await step(0);
    fireEvent.click(screen.getByRole('button', { name: 'Add account' }));
    await step(0);
    const form = screen.getByRole('form', { name: 'Add an account' });
    fireEvent.click(within(form).getByRole('radio', { name: /^Separate Claude Code profile/ }));
    fireEvent.change(within(form).getByLabelText('Name'), { target: { value: 'second' } });
    fireEvent.submit(form);
    await step(0);
    expect(form).toHaveTextContent('Run this in a terminal — this page will notice when you are done:');
    expect(within(form).getByLabelText('The command to run')).toHaveTextContent(COMMAND);
    expect(within(form).getByRole('button', { name: 'Copy command' })).toBeInTheDocument();
    expect(r.posts('/api/accounts/probe')).toHaveLength(0);
    await step(3000);
    expect(r.posts('/api/accounts/probe')).toEqual([{ id: 'second' }]);
    await step(3000);
    expect(r.posts('/api/accounts/probe')).toHaveLength(2);
    expect(screen.getByText('Signed in as second@example.com.')).toBeInTheDocument();
    expect(screen.queryByRole('form', { name: 'Add an account' })).toBeNull();
    await step(9000);
    expect(r.posts('/api/accounts/probe')).toHaveLength(2);
  });
});
