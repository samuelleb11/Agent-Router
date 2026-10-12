import { fireEvent, screen, within } from '@testing-library/react';
import { Route, Routes } from 'react-router';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { clientSetupKey } from '~/data/client-setup';
import type { ClientPreview, ClientSetup, ClientSetupInput, SetupClientId } from '~/model/client-setup';
import { json, press, refusal, renderScreen, unreachable } from '~/test-utils';
import ClientsPage from './Clients';
import SettingsLayout from './Layout';
import SettingsSection from './Section';
import { HOSTED, LOCAL, ok, stubRouter, type Answer } from './testing';

const SECRET = `arr_${'b'.repeat(64)}`;
const SETUP: ClientSetup = {
  mode: 'local', url: 'http://127.0.0.1:8787',
  clients: [
    { id: 'claude-code', name: 'Claude Code', protocol: 'messages', installed: true, version: '2.0.77', configured: true, managed: true, config_path: '/Users/me/.claude/settings.json', can_apply: true },
    { id: 'codex', name: 'Codex', protocol: 'responses', installed: true, version: '0.114.0', configured: false, managed: false, config_path: '/Users/me/.codex/config.toml', can_apply: true },
    { id: 'opencode', name: 'OpenCode', protocol: 'messages', installed: false, version: null, configured: false, managed: false, config_path: '/Users/me/.config/opencode/opencode.json', can_apply: true },
  ],
  models: [
    { id: 'claude-sonnet-4-6', display_name: 'Sonnet', provider: 'anthropic', account_id: 'anthropic-key', available: true },
    { id: 'gpt-5.4', display_name: 'GPT', provider: 'openai', account_id: 'openai-key', available: true },
    { id: 'kimi-for-coding', display_name: 'Kimi', provider: 'kimi', account_id: 'kimi-key', available: true },
    { id: 'claude-unavailable', display_name: 'Unavailable', provider: 'anthropic', account_id: null, available: false },
  ],
};
const preview = (client: SetupClientId, body: unknown, setup = SETUP): ClientPreview => {
  const input = body as ClientSetupInput;
  const model = setup.models.find((m) => m.id === input.model)!;
  return {
    ok: true, client, model: model.id, provider: model.provider, account_id: model.account_id,
    protocol: client === 'codex' ? 'responses' : 'messages', translated: model.provider !== 'openai',
    can_apply: setup.mode === 'local', path: setup.mode === 'local' ? setup.clients.find((c) => c.id === client)!.config_path : null,
    format: client === 'codex' ? 'toml' : 'json', content: `model = ${model.id}\nrouter_token = ${input.token || '<AGENT_ROUTER_CLIENT_TOKEN>'}`,
    warnings: [], revision: 'file-revision-1', connection_tested: false,
    checks: [{ id: 'provider', label: 'Provider credential', status: 'pass', detail: `Ready for ${model.provider}` }],
  };
};
const clipboardBefore = Object.getOwnPropertyDescriptor(navigator, 'clipboard');
const show = () => renderScreen(<ClientsPage />, { path: '/settings/clients', at: '/settings/clients' });
const field = (name: string, value: string) => fireEvent.change(screen.getByLabelText(name), { target: { value } });
function answer(setup = SETUP, more: Record<string, Answer> = {}) {
  const endpoints: Record<string, Answer> = { '/api/clients': ok(setup) };
  for (const c of setup.clients) {
    endpoints[`POST /api/clients/${c.id}/preview`] = ({ body }) => json(preview(c.id, body, setup));
    endpoints[`POST /api/clients/${c.id}/check`] = ({ body }) => json(preview(c.id, body, setup));
    endpoints[`POST /api/clients/${c.id}/apply`] = ok({ ok: true, client: c.id, path: c.config_path, changed: true, warnings: [] });
    endpoints[`POST /api/clients/${c.id}/restore`] = ok({ ok: true, client: c.id, path: c.config_path, changed: true, preserved_edits: true });
  }
  return stubRouter({ ...endpoints, ...more });
}

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  if (clipboardBefore) Object.defineProperty(navigator, 'clipboard', clipboardBefore);
  else Reflect.deleteProperty(navigator, 'clipboard');
});

describe('Clients setup', () => {
  it.each([LOCAL, HOSTED])('offers a Clients destination on local and hosted routers', async (auth) => {
    stubRouter({ '/api/auth': ok(auth) });
    renderScreen(<Routes><Route path="/settings" element={<SettingsLayout />}><Route path="router" element={<div>Router</div>} /></Route></Routes>, { path: '/*', at: '/settings/router' });
    expect(await screen.findByRole('tab', { name: 'Clients' })).toHaveAttribute('href', '/settings/clients');
  });

  it('opens the registered Clients page and preserves existing local Claude Code setup', async () => {
    answer(SETUP, { '/api/auth': ok(LOCAL) });
    renderScreen(<SettingsSection />, { path: '/settings/:section', at: '/settings/clients' });
    expect(await screen.findByRole('heading', { name: 'Clients' })).toBeInTheDocument();
    expect(screen.getByRole('link', { name: /Claude Code service/ })).toHaveAttribute('href', '/settings/claude-code');
    expect(document.title).toBe('Clients · Settings · Agent Router');
  });

  it('reports each client installation and uses exact model IDs from all three providers', async () => {
    answer();
    show();
    expect(await screen.findByText('2.0.77')).toBeInTheDocument();
    expect(screen.getByText('0.114.0')).toBeInTheDocument();
    expect(screen.getByText('Not detected on this machine')).toBeInTheDocument();
    expect(screen.getByLabelText('Model')).toHaveValue('claude-sonnet-4-6');
    for (const m of SETUP.models) expect(screen.getByRole('option', { name: new RegExp(m.id) })).toHaveValue(m.id);
    expect(screen.getByRole('option', { name: /claude-unavailable/ })).toBeDisabled();
    expect(screen.getByRole('link', { name: 'Client access token' })).toHaveAttribute('href', '/settings/access-tokens');
  });

  it.each(['claude-code', 'codex', 'opencode'] as const)('previews %s with the selected model, router credential and client-specific path', async (client) => {
    const router = answer();
    show();
    const name = SETUP.clients.find((c) => c.id === client)!.name;
    await press(await screen.findByRole('button', { name: `Configure ${name}` }));
    field('Model', 'kimi-for-coding');
    field('Router URL', 'https://router.example');
    field('Client access token', SECRET);
    await press(screen.getByRole('button', { name: 'Preview configuration' }));
    expect(await screen.findByLabelText(`${name} configuration preview`)).toHaveTextContent(SECRET);
    expect(router.posts(`/api/clients/${client}/preview`)).toEqual([{ model: 'kimi-for-coding', url: 'https://router.example', token: SECRET }]);
    expect(screen.getByText(/This configuration contains your router access token/)).toBeInTheDocument();
  });

  it('shows Codex compatibility limits before preview and clears secrets when changing clients', async () => {
    answer();
    const shown = show();
    await press(await screen.findByRole('button', { name: 'Configure Codex' }));
    expect(screen.getByRole('note', { name: 'Codex compatibility' })).toHaveTextContent('compaction is unavailable');
    expect(screen.getByRole('note')).toHaveTextContent('web search is disabled');
    expect(screen.getByRole('note')).toHaveTextContent('Saved private reasoning');
    field('Client access token', SECRET);
    await press(screen.getByRole('button', { name: 'Preview configuration' }));
    await screen.findByLabelText('Codex configuration preview');
    expect(JSON.stringify(shown.client.getQueryData(clientSetupKey))).not.toContain(SECRET);
    expect(shown.client.getMutationCache().getAll()).toHaveLength(0);
    await press(screen.getByRole('button', { name: 'Configure OpenCode' }));
    expect(screen.getByLabelText('Client access token')).toHaveValue('');
    expect(screen.queryByLabelText('Codex configuration preview')).toBeNull();
    expect(document.body.textContent).not.toContain(SECRET);
  });

  it('invalidates reviewed configuration when its model or credential changes', async () => {
    const router = answer();
    show();
    await screen.findByLabelText('Model');
    await press(screen.getByRole('button', { name: 'Preview configuration' }));
    await screen.findByLabelText('Claude Code configuration preview');
    field('Model', 'gpt-5.4');
    expect(screen.queryByRole('button', { name: 'Apply configuration' })).toBeNull();
    expect(screen.queryByLabelText('Claude Code configuration preview')).toBeNull();
    expect(router.posts('/api/clients/claude-code/apply')).toEqual([]);
  });

  it('applies only after reviewing and confirming, and sends the preview revision to prevent overwriting newer edits', async () => {
    const router = answer();
    show();
    await screen.findByLabelText('Model');
    field('Client access token', SECRET);
    field('Project directory (optional)', '/Users/me/code/app');
    await press(screen.getByRole('button', { name: 'Preview configuration' }));
    await screen.findByLabelText('Claude Code configuration preview');
    await press(screen.getByRole('button', { name: 'Apply configuration' }));
    expect(router.posts('/api/clients/claude-code/apply')).toEqual([]);
    const dialog = screen.getByRole('dialog');
    expect(dialog).toHaveTextContent('router access token will be stored');
    await press(within(dialog).getByRole('button', { name: 'Apply configuration' }));
    expect(await screen.findByText('Saved Claude Code configuration. Start a new client session to use it.')).toBeInTheDocument();
    expect(router.posts('/api/clients/claude-code/apply')).toEqual([{ model: 'claude-sonnet-4-6', url: SETUP.url, token: SECRET, project: '/Users/me/code/app', revision: 'file-revision-1' }]);
  });

  it('keeps a refused stale-file apply in the confirmation and never reports it as saved', async () => {
    answer(SETUP, { 'POST /api/clients/claude-code/apply': () => refusal('The client configuration changed after preview; preview again.', 409) });
    show();
    await screen.findByLabelText('Model');
    await press(screen.getByRole('button', { name: 'Preview configuration' }));
    await screen.findByLabelText('Claude Code configuration preview');
    await press(screen.getByRole('button', { name: 'Apply configuration' }));
    await press(within(screen.getByRole('dialog')).getByRole('button', { name: 'Apply configuration' }));
    expect(await screen.findByText('The client configuration changed after preview; preview again.')).toBeInTheDocument();
    expect(screen.queryByText(/Saved Claude Code configuration/)).toBeNull();
  });

  it('restores managed routing only after confirmation and reports preserved user edits', async () => {
    const router = answer();
    show();
    await press(await screen.findByRole('button', { name: 'Restore previous configuration' }));
    expect(router.posts('/api/clients/claude-code/restore')).toEqual([]);
    await press(within(screen.getByRole('dialog')).getByRole('button', { name: 'Restore configuration' }));
    expect(await screen.findByText('Restored unchanged settings. Settings you edited remain in place; review the configuration for any remaining router URL or token.')).toBeInTheDocument();
    expect(router.posts('/api/clients/claude-code/restore')).toEqual([{}]);
  });

  it('exports hosted setup, copies and downloads the preview, and offers no remote filesystem writes', async () => {
    const hosted: ClientSetup = { ...SETUP, mode: 'server', url: 'https://router.example', clients: SETUP.clients.map((c) => ({ ...c, config_path: null, can_apply: false })) };
    const router = answer(hosted);
    const copy = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText: copy } });
    const create = vi.fn().mockReturnValue('blob:client-config');
    const revoke = vi.fn();
    const BaseURL = URL;
    vi.stubGlobal('URL', class extends BaseURL { static override createObjectURL = create; static override revokeObjectURL = revoke; });
    const click = vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(() => {});
    show();
    await press(await screen.findByRole('button', { name: 'Configure Codex' }));
    expect(screen.queryByLabelText('Project directory (optional)')).toBeNull();
    await press(screen.getByRole('button', { name: 'Preview configuration' }));
    const content = await screen.findByLabelText('Codex configuration preview');
    expect(content).toHaveTextContent('<AGENT_ROUTER_CLIENT_TOKEN>');
    await press(screen.getByRole('button', { name: 'Copy configuration' }));
    expect(copy).toHaveBeenCalledWith(content.textContent);
    await press(screen.getByRole('button', { name: 'Download configuration' }));
    expect(create).toHaveBeenCalledWith(expect.any(Blob));
    expect(click.mock.instances[0]).toHaveProperty('download', 'config.toml');
    expect(revoke).toHaveBeenCalledWith('blob:client-config');
    expect(screen.queryByRole('button', { name: 'Apply configuration' })).toBeNull();
    expect(screen.queryByRole('button', { name: 'Restore previous configuration' })).toBeNull();
    expect(router.asked.some((r) => /\/(apply|restore)$/.test(r.path))).toBe(false);
  });

  it('checks the configured route and credentials without claiming a live model request succeeded', async () => {
    const router = answer(SETUP, { 'POST /api/clients/claude-code/check': ({ body }) => json({ ...preview('claude-code', body), checks: [{ id: 'provider', label: 'Provider credential', status: 'fail', detail: 'Add a working provider credential.' }], warnings: ['Token expires tomorrow.'] }) });
    show();
    await press(await screen.findByRole('button', { name: 'Check connection' }));
    expect(await screen.findByText('Provider credential: Failed')).toBeInTheDocument();
    expect(screen.getByText('Add a working provider credential.')).toBeInTheDocument();
    expect(screen.getByText('Token expires tomorrow.')).toBeInTheDocument();
    expect(screen.getByText(/It does not run a model request/)).toBeInTheDocument();
    expect(router.posts('/api/clients/claude-code/check')).toEqual([{ model: 'claude-sonnet-4-6', url: SETUP.url }]);
    expect(screen.queryByLabelText('Claude Code configuration preview')).toBeNull();
  });

  it('clears a generated secret explicitly, without writing it into any browser or query store', async () => {
    answer();
    const persistent = vi.spyOn(localStorage, 'setItem');
    const session = vi.spyOn(sessionStorage, 'setItem');
    const shown = show();
    await screen.findByLabelText('Client access token');
    field('Client access token', SECRET);
    await press(screen.getByRole('button', { name: 'Preview configuration' }));
    await screen.findByLabelText('Claude Code configuration preview');
    expect(persistent).not.toHaveBeenCalled();
    expect(session).not.toHaveBeenCalled();
    expect(JSON.stringify(shown.client.getQueryCache().getAll().map((q) => q.state))).not.toContain(SECRET);
    await press(screen.getByRole('button', { name: 'Clear preview and token' }));
    expect(screen.getByLabelText('Client access token')).toHaveValue('');
    expect(document.body.textContent).not.toContain(SECRET);
  });

  it('blocks setup when no model has a provider account and reports failed status reads', async () => {
    answer({ ...SETUP, models: SETUP.models.map((m) => ({ ...m, available: false, account_id: null })) });
    const shown = show();
    expect(await screen.findByRole('button', { name: 'Preview configuration' })).toBeDisabled();
    expect(screen.getByRole('link', { name: 'Accounts' })).toHaveAttribute('href', '/settings/accounts');
    shown.unmount();
    stubRouter({ '/api/clients': () => unreachable() });
    show();
    expect(await screen.findByText('Could not read client setup')).toBeInTheDocument();
  });

  it('uses an explicit service protocol and configured limits instead of inferring them from the provider or alias', async () => {
    answer({ ...SETUP, models: [{ id: 'team-code', display_name: 'Team code', provider: 'openai', account_id: 'openai-key', available: true, service_id: 'team-messages', native_protocol: 'messages', capabilities: { tools: false, images: false, reasoning: true, compaction: false, context_tokens: 32768, output_tokens: null } }] });
    show();
    await press(await screen.findByRole('button', { name: 'Configure Codex' }));
    expect(screen.getByLabelText('Model')).toHaveValue('team-code');
    expect(screen.getByRole('note', { name: 'Codex compatibility' })).toHaveTextContent('compaction is unavailable');
    expect(screen.getByText(/Service:.*API:/)).toHaveTextContent('Anthropic Messages');
    expect(screen.getByText(/Configured limits:/)).toHaveTextContent('context 32,768 tokens; output unknown tokens');
    expect(screen.getByText(/Configured limits:/)).toHaveTextContent('Tools disabled');
  });
});
