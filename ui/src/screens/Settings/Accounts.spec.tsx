// New for agent-router: Settings → Accounts, against the router's answers (docs/web-console-contracts.md,
// "Accounts: the list", "Accounts: editing", "Accounts: adding").
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

const card = (name: string) => screen.getByLabelText(`Account ${name}`);
const cardOf = async (name: string) => screen.findByLabelText(`Account ${name}`);
const listReads = (r: ReturnType<typeof stubRouter>) => r.gets('/api/accounts').length;

describe('Accounts: the list', () => {
  it('ST-C19, ST-C3: each account is a card with every fact a desk row had, and Use, Edit and Remove as buttons at rest', async () => {
    show();
    const work = await cardOf('work');
    const facts = within(work);
    expect(facts.getByText('Org').nextElementSibling).toHaveTextContent('—');
    expect(facts.getByText('Plan').nextElementSibling).toHaveTextContent('—');
    expect(facts.getByText('Source').nextElementSibling).toHaveTextContent('token');
    expect(facts.getByText('Credential').nextElementSibling).toHaveTextContent('sk-ant-oat01 (subscription OAuth)');
    expect(facts.getByText('Expires').nextElementSibling).toHaveTextContent('no expiry');
    for (const name of ['Use', 'Edit', 'Remove']) {
      const button = facts.getByRole('button', { name });
      expect(button).toBeVisible();
      expect(button.closest('[hidden], .cds--overflow-menu-options, [role="menu"]')).toBeNull();
    }
    const active = card('me@example.com');
    expect(within(active).getByText('active')).toBeInTheDocument();
    expect(within(active).getByText('Org').nextElementSibling).toHaveTextContent('Example Org');
    expect(within(active).getByText('Plan').nextElementSibling).toHaveTextContent('max');
    expect(within(active).getByText('usage key')).toBeInTheDocument();
    // The one signing now has no Use: it is the one in use.
    expect(within(active).queryByRole('button', { name: 'Use' })).toBeNull();
    // No overflow menu anywhere on the page.
    expect(document.querySelector('.cds--overflow-menu')).toBeNull();
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

  it('ST-C27: on a hosted router a Claude Code login or a profile offers no Use, and its card says it cannot sign there in the router’s sentence', async () => {
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
      expect(within(c).queryByRole('button', { name: 'Use' }), name).toBeNull();
      expect(within(c).getByText(/^It cannot sign on this router:/)).toHaveTextContent(NOT_HERE);
    }
    expect(within(card('spare')).getByRole('button', { name: 'Use' })).toBeInTheDocument();
  });

  it('ST-C24, ST-C25: Use switches in one click with no confirmation and the list re-reads; a refusal shows on that card, and the list re-reads', async () => {
    const r = show({ more: { 'POST /api/accounts/activate': ok({ ok: true, active: 'work' }) } });
    await cardOf('work');
    const before = listReads(r);
    fireEvent.click(within(card('work')).getByRole('button', { name: 'Use' }));
    expect(screen.queryByRole('dialog')).toBeNull();
    await waitFor(() => expect(r.posts('/api/accounts/activate')).toEqual([{ id: 'work' }]));
    await waitFor(() => expect(listReads(r)).toBeGreaterThan(before));

    r.answers['POST /api/accounts/activate'] = () => refusal('no account with id "work"', 500);
    const again = listReads(r);
    fireEvent.click(within(card('work')).getByRole('button', { name: 'Use' }));
    expect(await within(card('work')).findByText('no account with id "work"')).toBeInTheDocument();
    await waitFor(() => expect(listReads(r)).toBeGreaterThan(again));
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
    fireEvent.click(within(card('work')).getByRole('button', { name: 'Use' }));
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

  it('ST-C40, ST-C43: each way in is a choice carrying its explanation; locally sign-in comes first and is chosen, and its button reads Start sign-in', async () => {
    show();
    const form = await open();
    const radios = within(form).getAllByRole('radio') as HTMLInputElement[];
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

  it('ST-C52, ST-C101: a hosted router, and one that has not said what it is, add by pasted token only, drawn for a token from the start', async () => {
    show({ auth: HOSTED });
    const form = await open();
    expect(within(form).queryAllByRole('radio')).toEqual([]);
    expect(within(form).getByLabelText('Token')).toBeInTheDocument();
    expect(within(form).getByRole('button', { name: 'Add account' })).toBeInTheDocument();
    expect(form).not.toHaveTextContent('Sign in from here');
    vi.unstubAllGlobals();
    document.body.innerHTML = '';
    show({ more: { '/api/auth': () => new Promise<Response>(() => {}) } });
    const unknown = await open();
    expect(within(unknown).queryAllByRole('radio')).toEqual([]);
    expect(within(unknown).getByLabelText('Token')).toBeInTheDocument();
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
