import { fireEvent, screen, waitFor, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { AccountRow } from '~/data/settings';
import type { ProviderRegistry, RegistryModel, RegistryService } from '~/model/provider-registry';
import { DEFAULT_CAPABILITIES } from '~/model/provider-registry';
import { json, press, refusal, renderScreen, unreachable } from '~/test-utils';
import ModelsPage from './Models';
import { ACCOUNTS, HOSTED, LOCAL, ok, stubRouter, type Answer } from './testing';

const services: RegistryService[] = [
  { id: 'anthropic', label: 'Anthropic', provider: 'anthropic', base_url: 'https://api.anthropic.com', protocol: 'messages', auth_profile: 'bearer', builtin: true, capabilities: DEFAULT_CAPABILITIES },
  { id: 'openai', label: 'OpenAI', provider: 'openai', base_url: 'https://api.openai.com', protocol: 'responses', auth_profile: 'bearer', builtin: true, capabilities: { ...DEFAULT_CAPABILITIES, compaction: true } },
  { id: 'kimi', label: 'Kimi Code', provider: 'kimi', base_url: 'https://api.kimi.ai/coding', protocol: 'messages', auth_profile: 'api_key', builtin: true, capabilities: DEFAULT_CAPABILITIES },
  { id: 'team', label: 'Team service', provider: 'openai', base_url: 'https://team.example/api', protocol: 'responses', auth_profile: 'api_key', builtin: false, capabilities: { ...DEFAULT_CAPABILITIES, images: false, context_tokens: 32768, output_tokens: 4096 } },
];
const route: RegistryModel = { alias: 'team-code', service: 'team', upstream_model: 'vendor/code-model', account_id: 'openai-key', display_name: 'Team coding', capabilities: null };
const accounts: AccountRow[] = [...ACCOUNTS,
  { id: 'openai-key', label: 'OpenAI API', provider: 'openai', source: 'token' },
  { id: 'codex', label: 'Codex login', provider: 'openai', source: 'codex' },
  { id: 'kimi-key', label: 'Kimi API', provider: 'kimi', source: 'token' },
];
const field = (name: string, value: string) => fireEvent.change(screen.getByLabelText(name), { target: { value } });
const add = async (name: string) => {
  await waitFor(() => expect(screen.getByRole('button', { name })).toBeEnabled());
  await press(screen.getByRole('button', { name }));
};
function show({ registry = { services, models: [route] }, hosted = false, more = {} }: { registry?: ProviderRegistry; hosted?: boolean; more?: Record<string, Answer> } = {}) {
  let current = registry;
  const router = stubRouter({
    '/api/auth': ok(hosted ? HOSTED : LOCAL), '/api/accounts': ok(accounts),
    '/api/models': ok([{ id: 'claude-sonnet-4-6', provider: 'anthropic', serving: 'work', assigned: 'work' }, { id: route.alias, provider: 'openai', serving: 'openai-key', assigned: 'openai-key' }]),
    '/api/provider-registry': () => json(current),
    'POST /api/provider-registry/services': ({ body }) => { const service = { ...(body as RegistryService), builtin: false }; current = { ...current, services: [...current.services.filter((s) => s.id !== service.id), service] }; return json({ ok: true, service }, 201); },
    'POST /api/provider-registry/models': ({ body }) => { const model = body as RegistryModel; current = { ...current, models: [...current.models.filter((m) => m.alias !== model.alias), model] }; return json({ ok: true, model }, 201); },
    [`DELETE /api/provider-registry/models/${encodeURIComponent(route.alias)}`]: () => { current = { ...current, models: [] }; return json({ ok: true }); },
    'DELETE /api/provider-registry/services/team': () => { current = { ...current, services: current.services.filter((s) => s.id !== 'team') }; return json({ ok: true }); },
    ...more,
  });
  return { ...router, ...renderScreen(<ModelsPage />, { path: '/settings/models', at: '/settings/models' }) };
}
const routeCard = (alias: string) => document.querySelector<HTMLElement>(`[data-route="${alias}"]`)!;
const serviceCard = (id: string) => document.querySelector<HTMLElement>(`[data-service="${id}"]`)!;

afterEach(() => vi.unstubAllGlobals());

describe('Provider registry in Models settings', () => {
  it('offers an explicit model route with an upstream service and account', async () => {
    show();
    await waitFor(() => expect(screen.getByRole('button', { name: 'Add model route' })).toBeEnabled());
    expect(screen.getByRole('button', { name: 'Add provider service' })).toBeEnabled();
  });

  it('shows explicit upstream, service, account and configured limits while preserving built-in assignments', async () => {
    show();
    await screen.findByRole('heading', { name: 'Team coding' });
    expect(routeCard('team-code')).toHaveTextContent('vendor/code-model');
    expect(routeCard('team-code')).toHaveTextContent('OpenAI API');
    expect(routeCard('team-code')).toHaveTextContent('Inherits the service’s configured capabilities.');
    expect(serviceCard('team')).toHaveTextContent('32,768');
    expect(serviceCard('anthropic')).toHaveTextContent('Unknown');
    expect(screen.getByText(/Configured capabilities are your settings/)).toBeInTheDocument();
    expect(document.querySelector('[data-model="claude-sonnet-4-6"]')).not.toBeNull();
    expect(document.querySelector('[data-model="team-code"]')).toBeNull();
  });

  it('saves an exact alias and upstream model with a compatible explicit service and account', async () => {
    const router = show();
    await add('Add model route');
    field('Client model name', 'work-code');
    field('Provider service', 'team');
    field('Upstream model ID', 'vendor/coding-v2');
    field('Display name (optional)', 'Work coding');
    const choices = within(screen.getByLabelText('Provider account')).getAllByRole('option').map((o) => o.getAttribute('value'));
    expect(choices).toEqual(['', 'openai-key']);
    field('Provider account', 'openai-key');
    await press(screen.getByRole('button', { name: 'Save model route' }));
    expect(await screen.findByRole('heading', { name: 'Work coding' })).toBeInTheDocument();
    expect(router.posts('/api/provider-registry/models')).toEqual([{ alias: 'work-code', service: 'team', upstream_model: 'vendor/coding-v2', account_id: 'openai-key', display_name: 'Work coding', capabilities: null }]);
  });

  it('resets account selection when changing provider, and keeps native logins on built-in services only', async () => {
    show();
    await add('Add model route');
    field('Provider service', 'openai');
    expect(within(screen.getByLabelText('Provider account')).getByRole('option', { name: /Codex login/ })).toHaveValue('codex');
    field('Provider account', 'codex');
    field('Provider service', 'team');
    expect(screen.getByLabelText('Provider account')).toHaveValue('');
    expect(within(screen.getByLabelText('Provider account')).queryByRole('option', { name: /Codex login/ })).toBeNull();
    field('Provider service', 'kimi');
    expect(within(screen.getByLabelText('Provider account')).getAllByRole('option').map((o) => o.getAttribute('value'))).toEqual(['', 'kimi-key']);
  });

  it('keeps hosted native login accounts out of model routes', async () => {
    show({ hosted: true });
    await add('Add model route');
    field('Provider service', 'openai');
    expect(within(screen.getByLabelText('Provider account')).getAllByRole('option').map((o) => o.getAttribute('value'))).toEqual(['', 'openai-key']);
  });

  it('saves configured capability restrictions and rejects invalid token limits before sending', async () => {
    const router = show();
    await add('Add model route');
    field('Client model name', 'limited-code'); field('Provider service', 'openai'); field('Upstream model ID', 'gpt-code'); field('Provider account', 'openai-key');
    await press(screen.getByLabelText('Override configured capabilities'));
    field('Context token limit (optional)', '0');
    expect(screen.getByRole('button', { name: 'Save model route' })).toBeDisabled();
    expect(router.posts('/api/provider-registry/models')).toEqual([]);
    field('Context token limit (optional)', '16384'); field('Output token limit (optional)', '2048');
    await press(screen.getByLabelText('Allow images'));
    await press(screen.getByRole('button', { name: 'Save model route' }));
    expect(router.posts('/api/provider-registry/models')[0]).toMatchObject({ capabilities: { tools: true, images: false, reasoning: true, compaction: true, context_tokens: 16384, output_tokens: 2048 } });
    expect(await screen.findByRole('heading', { name: 'limited-code' })).toBeInTheDocument();
  });

  it('creates an HTTPS service with explicit protocol and authentication without asking for secrets', async () => {
    const router = show();
    await add('Add provider service');
    field('Service ID', 'kimi-work'); field('Service name', 'Kimi Work'); field('Account provider', 'kimi');
    field('Service base URL', 'http://kimi.example');
    expect(screen.getByRole('button', { name: 'Save provider service' })).toBeDisabled();
    field('Service base URL', 'https://token@kimi.example');
    expect(screen.getByRole('button', { name: 'Save provider service' })).toBeDisabled();
    field('Service base URL', 'https://kimi.example/v1');
    expect(screen.getByRole('button', { name: 'Save provider service' })).toBeDisabled();
    field('Service base URL', 'https://kimi.example/coding'); field('API protocol', 'messages'); field('Authentication header', 'api_key');
    expect(screen.getByLabelText('Allow compaction')).toBeDisabled();
    expect(screen.getByRole('dialog').querySelector('input[type="password"]')).toBeNull();
    await press(screen.getByRole('button', { name: 'Save provider service' }));
    expect(router.posts('/api/provider-registry/services')).toEqual([{ id: 'kimi-work', label: 'Kimi Work', provider: 'kimi', base_url: 'https://kimi.example/coding', protocol: 'messages', auth_profile: 'api_key', capabilities: DEFAULT_CAPABILITIES }]);
    expect(await screen.findByRole('heading', { name: 'Kimi Work' })).toBeInTheDocument();
  });

  it('edits custom services and routes without changing their identity', async () => {
    const router = show();
    await press(await screen.findByRole('button', { name: 'Edit provider service Team service' }));
    expect(screen.getByLabelText('Service ID')).toBeDisabled();
    expect(screen.getByLabelText('Context token limit (optional)')).toHaveValue('32768');
    field('Service base URL', 'https://team.example/new-api');
    await press(screen.getByRole('button', { name: 'Save provider service' }));
    await screen.findByText('Saved provider service.');
    expect(router.posts('/api/provider-registry/services')[0]).toMatchObject({ id: 'team', base_url: 'https://team.example/new-api' });
    await press(screen.getByRole('button', { name: 'Edit model route team-code' }));
    expect(screen.getByLabelText('Client model name')).toBeDisabled();
    field('Upstream model ID', 'vendor/code-model-v3');
    await press(screen.getByRole('button', { name: 'Save model route' }));
    expect(router.posts('/api/provider-registry/models')[0]).toMatchObject({ alias: 'team-code', upstream_model: 'vendor/code-model-v3' });
  });

  it('protects built-in services and refuses duplicate new IDs without silently replacing settings', async () => {
    show();
    await screen.findByRole('heading', { name: 'Team service' });
    expect(within(serviceCard('anthropic')).queryByRole('button')).toBeNull();
    expect(screen.getByRole('button', { name: 'Remove provider service Team service' })).toBeDisabled();
    await press(screen.getByRole('button', { name: 'Add provider service' }));
    field('Service ID', 'openai'); field('Service name', 'Replacement'); field('Service base URL', 'https://new.example');
    expect(screen.getByText('This service already exists. Edit it from its card.')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Save provider service' })).toBeDisabled();
  });

  it('removes a model route only after confirmation, then allows deleting its unused service', async () => {
    const router = show();
    await press(await screen.findByRole('button', { name: 'Remove model route team-code' }));
    expect(router.asked.some((r) => r.method === 'DELETE')).toBe(false);
    await press(within(screen.getByRole('dialog')).getByRole('button', { name: 'Remove' }));
    expect(await screen.findByText('Removed model route team-code.')).toBeInTheDocument();
    expect(routeCard('team-code')).toBeNull();
    await press(screen.getByRole('button', { name: 'Remove provider service Team service' }));
    await press(within(screen.getByRole('dialog')).getByRole('button', { name: 'Remove' }));
    expect(await screen.findByText('Removed service team.')).toBeInTheDocument();
    expect(serviceCard('team')).toBeNull();
  });

  it('encodes aliases containing slashes when deleting them', async () => {
    const alias = 'team/code';
    const router = show({ registry: { services, models: [{ ...route, alias }] }, more: { [`DELETE /api/provider-registry/models/${encodeURIComponent(alias)}`]: ok({ ok: true }) } });
    await press(await screen.findByRole('button', { name: `Remove model route ${alias}` }));
    await press(within(screen.getByRole('dialog')).getByRole('button', { name: 'Remove' }));
    expect(router.asked.some((r) => r.method === 'DELETE' && r.path.endsWith('team%2Fcode'))).toBe(true);
  });

  it('shows registry read and write failures in place and does not report refused writes as saved', async () => {
    const first = show({ more: { '/api/provider-registry': () => unreachable() } });
    expect(await screen.findByText('Could not read provider services and model routes')).toBeInTheDocument();
    expect(screen.queryByText(/^No explicit model routes/)).toBeNull();
    first.unmount();
    show({ more: { 'POST /api/provider-registry/models': () => refusal('The selected account belongs to a different provider.', 400) } });
    await press(await screen.findByRole('button', { name: 'Edit model route team-code' }));
    await press(screen.getByRole('button', { name: 'Save model route' }));
    expect(await screen.findByText('The selected account belongs to a different provider.')).toBeInTheDocument();
    expect(screen.getByRole('dialog')).toBeInTheDocument();
    expect(screen.queryByText(/^Saved model route\./)).toBeNull();
  });
});
