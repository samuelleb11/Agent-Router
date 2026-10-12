// New for agent-router: Settings → Password, against the router's answers (docs/web-console-contracts.md,
// "Password").
import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { PINNED, type Auth } from '~/model/settings';
import { json, refusal, renderScreen } from '~/test-utils';
import PasswordPage from './Password';
import { HOSTED, LOCAL, ok, stubRouter, type Answer } from './testing';

afterEach(() => vi.unstubAllGlobals());

function show(auth: Auth = LOCAL, more: Record<string, Answer> = {}) {
  let state = auth;
  const router = stubRouter({ '/api/auth': () => json(state), ...more });
  const view = renderScreen(<PasswordPage />, { path: '/settings/password', at: '/settings/password' });
  return { ...router, ...view, set: (next: Partial<Auth>) => (state = { ...state, ...next }) };
}

const fields = async () => ({ password: await screen.findByLabelText('New password'), again: screen.getByLabelText('Confirm') });
const UNSET: Auth = { ...LOCAL, configured: false, updated_at: null };

describe('Password', () => {
  it('ST-C73: two fields, "at least 8 characters" said before anything is typed, both checked before sending, and Enter in the second sends it', async () => {
    const user = userEvent.setup();
    const r = show(UNSET, { 'POST /api/auth': ok({ ok: true, configured: true }) });
    const { password, again } = await fields();
    expect(screen.getByText('At least 8 characters.')).toBeInTheDocument();
    await user.type(password, 'short{Enter}');
    // Enter in the first field goes on to the second rather than sending.
    expect(document.activeElement).toBe(again);
    await user.type(again, 'short{Enter}');
    expect(await screen.findByText('Use at least 8 characters: this is 5.')).toBeInTheDocument();
    await user.clear(password);
    await user.type(password, 'long enough');
    await user.clear(again);
    await user.type(again, 'long enougH{Enter}');
    expect(await screen.findByText('The two entries differ.')).toBeInTheDocument();
    expect(r.posts('/api/auth')).toEqual([]);
    await user.clear(again);
    await user.type(again, 'long enough{Enter}');
    await waitFor(() => expect(r.posts('/api/auth')).toEqual([{ password: 'long enough' }]));
  });

  it('ST-C74: the length is counted in characters as the router counts it, and the password is sent exactly as typed, spaces included', async () => {
    const r = show(UNSET, { 'POST /api/auth': ok({ ok: true, configured: true }) });
    const { password, again } = await fields();
    // Four emoji are eight UTF-16 units and four characters: the router refuses them, so the page does.
    fireEvent.change(password, { target: { value: '😀😀😀😀' } });
    fireEvent.change(again, { target: { value: '😀😀😀😀' } });
    fireEvent.submit(password.closest('form')!);
    expect(await screen.findByText('Use at least 8 characters: this is 4.')).toBeInTheDocument();
    expect(r.posts('/api/auth')).toEqual([]);
    fireEvent.change(password, { target: { value: '  spaced out  ' } });
    fireEvent.change(again, { target: { value: '  spaced out  ' } });
    fireEvent.submit(password.closest('form')!);
    await waitFor(() => expect(r.posts('/api/auth')).toEqual([{ password: '  spaced out  ' }]));
  });

  it('ST-C75: the button reads Set password when none is set and Change password when one is, and the old one is never asked for', async () => {
    show(UNSET);
    expect(await screen.findByRole('button', { name: 'Set password' })).toBeInTheDocument();
    vi.unstubAllGlobals();
    document.body.innerHTML = '';
    show(LOCAL);
    expect(await screen.findByRole('button', { name: 'Change password' })).toBeInTheDocument();
    expect(screen.queryByLabelText(/old|current/i)).toBeNull();
    expect(screen.getAllByLabelText(/password|confirm/i).filter((e) => e.tagName === 'INPUT')).toHaveLength(2);
  });

  it('ST-C76, ST-C102: the page says who is asked for it — on a local router any other device, and from here a forgotten one is replaced; on a hosted router every caller', async () => {
    show();
    expect((await screen.findByText(/^Asked of any device that is not this Mac/)).closest('p')).toHaveTextContent(
      'Asked of any device that is not this Mac — a browser gets the usual prompt (any username), and a Claude Code elsewhere sets ANTHROPIC_AUTH_TOKEN to it. Forgotten? Set a new one from here; this Mac never needs the old one.',
    );
    vi.unstubAllGlobals();
    document.body.innerHTML = '';
    show({ ...HOSTED, password_pinned: false });
    const said = (await screen.findByText(/^Asked of every caller/)).closest('p');
    expect(said).toHaveTextContent('Asked of every caller — a browser gets the usual prompt (any username), and Claude Code sets ANTHROPIC_AUTH_TOKEN to it.');
    expect(document.body).not.toHaveTextContent('this Mac');
  });

  it('ST-C77: the heading says whether one is set and how long ago it changed, or why it matters', async () => {
    show({ ...LOCAL, updated_at: Date.now() - 3 * 60_000 });
    expect(await screen.findByText('set · changed 3 min ago')).toBeInTheDocument();
    vi.unstubAllGlobals();
    document.body.innerHTML = '';
    show(UNSET);
    expect(await screen.findByText('not set — other devices are refused')).toBeInTheDocument();
  });

  it('ST-C78, ST-C18: Remove password asks first, says every other device is refused until a new one is set here, and from another machine that this browser is locked out at once', async () => {
    const r = show({ ...LOCAL, from_loopback: false }, { 'POST /api/auth': ok({ ok: true, configured: false }) });
    fireEvent.click(await screen.findByRole('button', { name: 'Remove password' }));
    const dialog = await screen.findByRole('dialog');
    expect(dialog).toHaveTextContent('Every other device is refused until a new one is set from this Mac.');
    expect(dialog).toHaveTextContent('You are reading this from another machine: this browser is locked out the moment it is removed.');
    expect(r.posts('/api/auth')).toEqual([]);
    const confirm = within(dialog).getByRole('button', { name: 'Remove password' });
    expect(confirm.className).toMatch(/cds--btn--danger/);
    fireEvent.click(confirm);
    await waitFor(() => expect(r.posts('/api/auth')).toEqual([{ clear: true }]));
  });

  it('ST-C79: a password pinned by the deployment is said to be the deployment’s, with nothing to set, change or remove', async () => {
    show(HOSTED);
    expect(await screen.findByText(PINNED)).toBeInTheDocument();
    expect(screen.queryByRole('button')).toBeNull();
    expect(screen.queryByLabelText('New password')).toBeNull();
  });

  it('removing the legacy password preserves access tokens and explains that token users keep access', async () => {
    const r = show({ ...LOCAL, access_tokens: 1, from_loopback: false }, { 'POST /api/auth': ok({ ok: true, configured: false }) });
    fireEvent.click(await screen.findByRole('button', { name: 'Remove password' }));
    const dialog = await screen.findByRole('dialog');
    expect(dialog).toHaveTextContent('Devices using this password lose access. Access tokens remain valid.');
    expect(dialog).not.toHaveTextContent('Every other device is refused');
    expect(dialog).toHaveTextContent('If this browser uses the password, it loses access when the password is removed.');
    expect(dialog).not.toHaveTextContent('this browser is locked out the moment');
    r.set({ configured: false });
    fireEvent.click(within(dialog).getByRole('button', { name: 'Remove password' }));
    expect(await screen.findByText('Password removed. Access tokens remain valid.')).toBeInTheDocument();
    expect(await screen.findByText('not set — access tokens are configured')).toBeInTheDocument();
    expect(r.posts('/api/auth')).toEqual([{ clear: true }]);
  });

  it('ST-C83, ST-C10: once set, both fields are emptied and /api/auth is read again; a refused one stays in both fields and never reads "set"', async () => {
    const r = show(UNSET, { 'POST /api/auth': () => refusal('the store is read-only') });
    const { password, again } = await fields();
    fireEvent.change(password, { target: { value: 'correct horse' } });
    fireEvent.change(again, { target: { value: 'correct horse' } });
    fireEvent.submit(password.closest('form')!);
    expect(await screen.findByText('the store is read-only')).toBeInTheDocument();
    expect(screen.queryByText('Password set.')).toBeNull();
    expect(password).toHaveValue('correct horse');
    expect(again).toHaveValue('correct horse');
    r.answers['POST /api/auth'] = ok({ ok: true, configured: true });
    r.set({ configured: true });
    const reads = r.gets('/api/auth').length;
    fireEvent.submit(password.closest('form')!);
    expect(await screen.findByText('Password set.')).toBeInTheDocument();
    expect(password).toHaveValue('');
    expect(again).toHaveValue('');
    expect(r.gets('/api/auth').length).toBeGreaterThan(reads);
  });

  it('ST-C15: the form redraws only when its own facts change: a half-typed password survives a re-read that brings a new address', async () => {
    const r = show();
    const { password } = await fields();
    fireEvent.change(password, { target: { value: 'half-typ' } });
    password.focus();
    r.set({ lan_url: 'http://10.0.0.9:8787' });
    await act(async () => {
      await r.client.invalidateQueries();
    });
    expect(screen.getByLabelText('New password')).toBe(password);
    expect(password).toHaveValue('half-typ');
    expect(document.activeElement).toBe(password);
  });

  it('ST-C16: both fields are password inputs the browser neither fills from a saved login nor spellchecks', async () => {
    show();
    const { password, again } = await fields();
    for (const f of [password, again]) {
      expect(f).toHaveAttribute('type', 'password');
      expect(f).toHaveAttribute('autocomplete', 'new-password');
      expect(f).toHaveAttribute('spellcheck', 'false');
    }
  });
});
