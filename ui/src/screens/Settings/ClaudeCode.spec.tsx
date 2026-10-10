// New for agent-router: Settings → Claude Code, against the router's answers (docs/web-console-contracts.md,
// "Claude Code", "Outcomes and refusals").
import { fireEvent, screen, waitFor, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { Setup, Shim } from '~/data/settings';
import { json, refusal, renderScreen } from '~/test-utils';
import ClaudeCodePage from './ClaudeCode';
import { LOCAL, ok, SETUP, SHIM, stubRouter, type Answer } from './testing';

afterEach(() => vi.unstubAllGlobals());

function show({ setup = SETUP, shim = SHIM, more = {} }: { setup?: Setup; shim?: Shim; more?: Record<string, Answer> } = {}) {
  let state = { setup, shim };
  const router = stubRouter({
    '/api/auth': ok(LOCAL),
    '/api/setup': () => json(state.setup),
    '/api/setup/shim': () => json(state.shim),
    ...more,
  });
  const view = renderScreen(<ClaudeCodePage />, { path: '/settings/claude-code', at: '/settings/claude-code' });
  return { ...router, ...view, set: (next: Partial<typeof state>) => (state = { ...state, ...next }) };
}

const sw = (name: string) => screen.findByRole('switch', { name });
/// The block a switch sits in with its explanation and its outcome.
const block = (el: HTMLElement) => el.closest('[class*="control"]') as HTMLElement;

/// A write that answers only when told to.
function held() {
  let release: (r: Response) => void = () => {};
  const answer: Answer = () => new Promise<Response>((r) => (release = r));
  return { answer, release: (r: Response) => release(r) };
}

describe('Claude Code', () => {
  it('ST-C54: Start at login is a switch with its explanation; with no service manager it is replaced by "not supported on <platform> yet"', async () => {
    show();
    const login = await sw('Start at login');
    expect(block(login)).toHaveTextContent('Runs in the background and restarts if it crashes.');
    vi.unstubAllGlobals();
    document.body.innerHTML = '';
    show({ setup: { ...SETUP, service: { registered: false, supported: false, platform: 'windows' } } });
    expect(await screen.findByText(/^Not supported on windows yet/)).toBeInTheDocument();
    expect(screen.queryByRole('switch', { name: 'Start at login' })).toBeNull();
  });

  it('ST-C55: Route every project is a switch that names the global settings file it writes', async () => {
    show();
    expect(block(await sw('Route every project'))).toHaveTextContent('/Users/me/.claude/settings.json');
  });

  it('ST-C56: the page says only env.ANTHROPIC_BASE_URL is written, the rest left alone, and what turning it off puts back', async () => {
    show();
    await sw('Route every project');
    expect(screen.getByText(/^Only/)).toHaveTextContent(
      'Only env.ANTHROPIC_BASE_URL is ever written. Everything else in your settings file is left as it is, and turning routing off puts back whatever was there before — unless you have changed it yourself since, in which case it is left alone.',
    );
  });

  it('ST-C57, ST-C9: a routing change reports the router’s own sentence for what it did, beside its own switch', async () => {
    const DETAIL = 'replaced https://proxy.example in /Users/me/.claude/settings.json; it will be restored';
    const r = show({ more: { 'POST /api/setup/config': ok({ ...SETUP, ok: true, detail: DETAIL }) } });
    fireEvent.click(await sw('Route every project'));
    await waitFor(() => expect(r.posts('/api/setup/config')).toEqual([{ enable: true }]));
    const route = await sw('Route every project');
    expect(await within(block(route)).findByText(DETAIL)).toBeInTheDocument();
    expect(within(block(await sw('Start at login'))).queryByText(DETAIL)).toBeNull();
  });

  it('ST-C58: each routed project is listed with its path, and Stop routing sends the project’s root', async () => {
    const r = show({ more: { 'POST /api/setup/config': ok({ ...SETUP, ok: true, detail: 'removed ANTHROPIC_BASE_URL' }) } });
    const list = await screen.findByRole('list', { name: 'Projects routed one by one' });
    const row = within(list).getByText('/Users/me/code/app/.claude/settings.local.json').closest('li') as HTMLElement;
    fireEvent.click(within(row).getByRole('button', { name: 'Stop routing' }));
    await waitFor(() => expect(r.posts('/api/setup/config')).toEqual([{ enable: false, project: '/Users/me/code/app' }]));
    expect(await screen.findByText('removed ANTHROPIC_BASE_URL')).toBeInTheDocument();
  });

  it('ST-C59, ST-C16: Route a single project takes a directory; a refused path shows the router’s sentence and keeps what was typed', async () => {
    const r = show({ more: { 'POST /api/setup/config': () => refusal('/nowhere is not a directory') } });
    const field = await screen.findByLabelText('Route a single project instead');
    expect(field).toHaveAttribute('autocomplete', 'off');
    expect(field).toHaveAttribute('spellcheck', 'false');
    fireEvent.change(field, { target: { value: '/nowhere' } });
    fireEvent.click(screen.getByRole('button', { name: 'Route this project' }));
    expect(await screen.findByText('/nowhere is not a directory')).toBeInTheDocument();
    expect(r.posts('/api/setup/config')).toEqual([{ enable: true, project: '/nowhere' }]);
    expect(field).toHaveValue('/nowhere');
    expect(screen.queryByText('done')).toBeNull();
  });

  it('ST-C60: the wrapper section appears when an editor extension is found, says what it does, and warns when its binary is missing', async () => {
    show({ shim: { ...SHIM, available: false } });
    const section = await screen.findByRole('region', { name: 'Type into editor sessions' });
    expect(section).toHaveTextContent('Lets the live view send messages to sessions VS Code starts. It takes effect for sessions started afterwards');
    expect(section).toHaveTextContent('The wrapper binary is missing next to agent-router');
    vi.unstubAllGlobals();
    document.body.innerHTML = '';
    show({ shim: { ...SHIM, targets: [] } });
    await sw('Start at login');
    expect(screen.queryByRole('region', { name: 'Type into editor sessions' })).toBeNull();
  });

  it('ST-C61, ST-C12: the automatic switch explains itself and shows what it last did and when; it can always be turned off, and on only while the binary is there', async () => {
    show();
    const auto = await sw('Manage it automatically');
    expect(block(auto)).toHaveTextContent('Every extension update ships a fresh, unwrapped binary');
    expect(block(auto)).toHaveTextContent('Last: wrapped anthropic.claude-code-2.0.77-darwin-arm64 · 5 min ago.');
    expect(auto).toBeEnabled();
    vi.unstubAllGlobals();
    document.body.innerHTML = '';
    show({ shim: { ...SHIM, available: false, auto: false } });
    expect(await sw('Manage it automatically')).toBeDisabled();
    vi.unstubAllGlobals();
    document.body.innerHTML = '';
    show({ shim: { ...SHIM, available: false, auto: true } });
    expect(await sw('Manage it automatically')).toBeEnabled();
  });

  it('ST-C62: each extension binary is listed with its state and Install or Remove for its path exactly as listed; Install only while the binary is there, Remove always', async () => {
    const r = show({ more: { 'POST /api/setup/shim': ok({ ok: true }) } });
    const list = await screen.findByRole('list', { name: 'Extension binaries' });
    const rows = within(list).getAllByRole('listitem');
    expect(rows[0]).toHaveTextContent('anthropic.claude-code-2.0.77-darwin-arm64');
    expect(rows[0]).toHaveTextContent('installed');
    expect(rows[1]).toHaveTextContent('not installed');
    fireEvent.click(within(rows[1]!).getByRole('button', { name: 'Install' }));
    await waitFor(() => expect(r.posts('/api/setup/shim')).toEqual([{ path: SHIM.targets![1]!.path, action: 'install' }]));
    vi.unstubAllGlobals();
    document.body.innerHTML = '';
    show({ shim: { ...SHIM, available: false } });
    const without = within(await screen.findByRole('list', { name: 'Extension binaries' })).getAllByRole('listitem');
    expect(within(without[0]!).getByRole('button', { name: 'Remove' })).toBeInTheDocument();
    expect(within(without[1]!).queryByRole('button', { name: 'Install' })).toBeNull();
  });

  it('ST-C63: a wrapper change says what follows, and a refusal shows the router’s reason', async () => {
    const r = show({ more: { 'POST /api/setup/shim': ok({ ok: true }), 'POST /api/setup/shim/auto': ok({ ok: true }) } });
    const list = await screen.findByRole('list', { name: 'Extension binaries' });
    const second = within(list).getAllByRole('listitem')[1]!;
    fireEvent.click(within(second).getByRole('button', { name: 'Install' }));
    expect(await within(second).findByText('Installed — restart a session to use it.')).toBeInTheDocument();
    fireEvent.click(await sw('Manage it automatically'));
    expect(await screen.findByText('On — new extension versions are wrapped as they arrive.')).toBeInTheDocument();
    r.answers['POST /api/setup/shim'] = ok({ ok: false, reason: 'Not a known extension binary.' });
    const first = within(list).getAllByRole('listitem')[0]!;
    fireEvent.click(within(first).getByRole('button', { name: 'Remove' }));
    expect(await within(first).findByText('Not a known extension binary.')).toBeInTheDocument();
    expect(within(first).queryByText('Removed.')).toBeNull();
  });

  it('ST-C64: the page’s heading says whether the login item is on and whether every project or only some are routed', async () => {
    show();
    expect(await screen.findByText('login item off · 1 project routed')).toBeInTheDocument();
  });

  it('ST-C9: a write shows its progress beside its own control while it runs, and its outcome there after', async () => {
    const write = held();
    const r = show({ more: { 'POST /api/setup/service': write.answer } });
    fireEvent.click(await sw('Start at login'));
    const login = block(await sw('Start at login'));
    expect(await within(login).findByText('Working…')).toBeInTheDocument();
    expect(within(block(await sw('Route every project'))).queryByText('Working…')).toBeNull();
    r.set({ setup: { ...SETUP, service: { registered: true, supported: true, platform: 'macos' } } });
    write.release(json({ ...SETUP, ok: true }));
    expect(await within(login).findByText('On — the router starts at login.')).toBeInTheDocument();
  });

  it('ST-C13: a switch shows the state the router reports, read back after the write, never one flipped in advance', async () => {
    const write = held();
    const r = show({ more: { 'POST /api/setup/service': write.answer } });
    const login = await sw('Start at login');
    expect(login).toHaveAttribute('aria-checked', 'false');
    fireEvent.click(login);
    await waitFor(() => expect(r.posts('/api/setup/service')).toEqual([{ enable: true }]));
    expect(await sw('Start at login')).toHaveAttribute('aria-checked', 'false');
    // The router answered yes, but what it reports afterwards is what the switch shows.
    const reads = r.gets('/api/setup').length;
    write.release(json({ ok: true }));
    await waitFor(() => expect(r.gets('/api/setup').length).toBeGreaterThan(reads));
    expect(await sw('Start at login')).toHaveAttribute('aria-checked', 'false');
    r.set({ setup: { ...SETUP, service: { registered: true, supported: true, platform: 'macos' } } });
    r.answers['POST /api/setup/service'] = ok({ ok: true });
    fireEvent.click(await sw('Start at login'));
    await waitFor(async () => expect(await sw('Start at login')).toHaveAttribute('aria-checked', 'true'));
  });

  it('ST-C10: a refused setup switch never reads as done', async () => {
    show({ more: { 'POST /api/setup/service': () => refusal('launchctl bootstrap failed: 5: Input/output error') } });
    fireEvent.click(await sw('Start at login'));
    const login = block(await sw('Start at login'));
    expect(await within(login).findByText('launchctl bootstrap failed: 5: Input/output error')).toBeInTheDocument();
    expect(screen.queryByText(/^On — |^done$/)).toBeNull();
    expect(await sw('Start at login')).toHaveAttribute('aria-checked', 'false');
  });

  it('ST-C4: each switch’s explanation is visible text beside it, and is what describes it', async () => {
    show();
    for (const name of ['Start at login', 'Route every project', 'Manage it automatically']) {
      const s = await sw(name);
      const why = document.getElementById(s.getAttribute('aria-describedby') ?? '');
      expect(why, name).not.toBeNull();
      expect(why).toBeVisible();
      expect(why?.textContent?.length, name).toBeGreaterThan(20);
      expect(block(s)).toContainElement(why);
    }
  });

  it('ST-C5: the page reads its setup and wrapper state when it opens, and again after each of its own writes', async () => {
    const r = show({ more: { 'POST /api/setup/shim/auto': ok({ ok: true }) } });
    await sw('Manage it automatically');
    expect([r.gets('/api/setup').length, r.gets('/api/setup/shim').length]).toEqual([1, 1]);
    fireEvent.click(await sw('Manage it automatically'));
    await waitFor(() => expect(r.gets('/api/setup/shim').length).toBe(2));
  });
});
