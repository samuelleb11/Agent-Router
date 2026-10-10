// New for agent-router: Settings → Notifications, against the router's answers
// (docs/web-console-contracts.md, "Notifications").
import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { Push } from '~/data/settings';
import type { Auth } from '~/model/settings';
import { json, refusal, renderScreen, unreachable } from '~/test-utils';
import NotificationsPage from './Notifications';
import { HOSTED, LOCAL, ok, PUSH, stubRouter, type Answer } from './testing';

afterEach(() => vi.unstubAllGlobals());

function show({ push = PUSH, auth = LOCAL, more = {} }: { push?: Push; auth?: Auth; more?: Record<string, Answer> } = {}) {
  let state = push;
  const router = stubRouter({ '/api/auth': ok(auth), '/api/push': () => json(state), ...more });
  const view = renderScreen(<NotificationsPage />, { path: '/settings/notifications', at: '/settings/notifications' });
  return { ...router, ...view, set: (next: Partial<Push>) => (state = { ...state, ...next }) };
}

const UNCONFIGURED: Push = { ...PUSH, configured: false, team_id: null, key_id: null, bundle_id: null, any_configured: false };
const apple = () => screen.findByRole('region', { name: 'Apple (iPhone)' });

describe('Notifications', () => {
  it('ST-C84, ST-C5: the push state is read when the page opens, and a failed read says so with its reason, never as an empty form', async () => {
    const r = show();
    await apple();
    expect(r.gets('/api/push')).toHaveLength(1);
    vi.unstubAllGlobals();
    document.body.innerHTML = '';
    show({ more: { '/api/push': unreachable } });
    expect(await screen.findByText('Could not read notification settings')).toBeInTheDocument();
    expect(screen.getByText('Could not reach the router.')).toBeInTheDocument();
    expect(screen.queryByLabelText('Team id')).toBeNull();
  });

  it('ST-C85: the heading names the key id and counts the phones, or says no phone can be told', async () => {
    show();
    expect(await screen.findByText('key ABC123DEF4 · 2 phones')).toBeInTheDocument();
    vi.unstubAllGlobals();
    document.body.innerHTML = '';
    show({ push: UNCONFIGURED });
    expect(await screen.findByText('no APNs key — phones cannot be told')).toBeInTheDocument();
  });

  it('ST-C86: a key file the router took in is named; with none, the page says where to drop one, locally and on a hosted router', async () => {
    show({ push: { ...PUSH, key_file: '/Users/me/.agent-router/AuthKey_ABC123DEF4.p8' } });
    expect(await screen.findByText('key file: /Users/me/.agent-router/AuthKey_ABC123DEF4.p8')).toBeInTheDocument();
    vi.unstubAllGlobals();
    document.body.innerHTML = '';
    show({ push: UNCONFIGURED });
    expect(await screen.findByText(/^Drop the AuthKey_<key id>\.p8/)).toHaveTextContent('into ~/.agent-router and the router takes it in by itself — or paste it here.');
    vi.unstubAllGlobals();
    document.body.innerHTML = '';
    show({ push: UNCONFIGURED, auth: HOSTED });
    expect(await screen.findByText(/^Put the AuthKey_<key id>\.p8/)).toHaveTextContent('in the deployment’s push Secret — or paste it here.');
  });

  it('ST-C87, ST-C16: the ids are filled from what is stored and the .p8 box never is; it says a new key replaces the stored one; none of them is filled or spellchecked by the browser', async () => {
    show();
    const region = await apple();
    expect(within(region).getByLabelText('Team id')).toHaveValue('ABCDE12345');
    expect(within(region).getByLabelText('Key id')).toHaveValue('ABC123DEF4');
    expect(within(region).getByLabelText('Bundle id')).toHaveValue('ca.example.agentrouter');
    const p8 = within(region).getByLabelText('APNs key (.p8)');
    expect(p8).toHaveValue('');
    expect(p8).toHaveAttribute('placeholder', 'a new .p8 replaces the stored one');
    expect(region).toHaveTextContent('A key is stored, and is never shown here: a new one pasted here replaces it.');
    expect(p8).toHaveAttribute('spellcheck', 'false');
    for (const label of ['Team id', 'Key id', 'Bundle id']) {
      expect(within(region).getByLabelText(label)).toHaveAttribute('autocomplete', 'off');
      expect(within(region).getByLabelText(label)).toHaveAttribute('spellcheck', 'false');
    }
  });

  it('ST-C88, ST-C11, ST-C10: Save key asks for the three together before sending; a refused key stays in its box beside the router’s sentence and never reads saved', async () => {
    const r = show({ push: UNCONFIGURED, more: { 'POST /api/push/config': () => refusal('that key does not sign: invalid PEM') } });
    const region = await apple();
    const p8 = within(region).getByLabelText('APNs key (.p8)');
    fireEvent.change(p8, { target: { value: '-----BEGIN PRIVATE KEY-----\nnot really\n-----END PRIVATE KEY-----' } });
    fireEvent.click(within(region).getByRole('button', { name: 'Save key' }));
    expect(await within(region).findByText('team id, key id and the .p8 key are all needed')).toBeInTheDocument();
    expect(r.posts('/api/push/config')).toEqual([]);
    fireEvent.change(within(region).getByLabelText('Team id'), { target: { value: 'ABCDE12345' } });
    fireEvent.change(within(region).getByLabelText('Key id'), { target: { value: 'ABC123DEF4' } });
    fireEvent.click(within(region).getByRole('button', { name: 'Save key' }));
    expect(await within(region).findByText('that key does not sign: invalid PEM')).toBeInTheDocument();
    expect(r.posts('/api/push/config')).toEqual([
      { team_id: 'ABCDE12345', key_id: 'ABC123DEF4', p8: '-----BEGIN PRIVATE KEY-----\nnot really\n-----END PRIVATE KEY-----', bundle_id: '' },
    ]);
    expect(p8).toHaveValue('-----BEGIN PRIVATE KEY-----\nnot really\n-----END PRIVATE KEY-----');
    expect(within(region).queryByText(/^Saved/)).toBeNull();
  });

  it('ST-C88: with a key stored, the button reads Replace key', async () => {
    show();
    expect(within(await apple()).getByRole('button', { name: 'Replace key' })).toBeInTheDocument();
  });

  it('ST-C89: Send a test is offered whenever either service has a key, and its answer per phone stays on screen when the page reads again', async () => {
    const r = show({
      more: { 'POST /api/push/test': ok({ ok: true, results: [{ device: 'a1b2c3', ok: true, error: null }, { device: 'd4e5f6', ok: false, error: 'BadDeviceToken' }] }) },
    });
    fireEvent.click(await screen.findByRole('button', { name: 'Send a test' }));
    const answer = await screen.findByRole('status', { name: 'The test’s answer' });
    expect(answer).toHaveTextContent('…a1b2c3: delivered');
    expect(answer).toHaveTextContent('…d4e5f6: BadDeviceToken');
    r.set({ last_error: 'BadDeviceToken' });
    await act(async () => {
      await r.client.invalidateQueries();
    });
    await screen.findByText('Last failure: BadDeviceToken');
    expect(screen.getByRole('status', { name: 'The test’s answer' })).toHaveTextContent('…d4e5f6: BadDeviceToken');

    for (const [why, push, offered] of [
      ['Firebase only', { ...UNCONFIGURED, fcm: { configured: true, project_id: 'sample-project' } }, true],
      ['no key at all', UNCONFIGURED, false],
    ] as const) {
      vi.unstubAllGlobals();
      document.body.innerHTML = '';
      show({ push });
      await apple();
      expect(!!screen.queryByRole('button', { name: 'Send a test' }), why).toBe(offered);
    }
  });

  it('ST-C89: a test that reached no phone says so per phone, from what the router recorded, or that no phone is registered', async () => {
    const before = PUSH.devices!;
    const r = show({ more: { 'POST /api/push/test': ok({ ok: false, results: [{ device: 'a1b2c3', ok: false, error: 'Unregistered' }] }) } });
    r.set({ devices: [{ ...before[0]!, last_result: 'BadDeviceToken', last_seen_at: Date.now() }, before[1]!] });
    fireEvent.click(await screen.findByRole('button', { name: 'Send a test' }));
    const answer = await screen.findByRole('status', { name: 'The test’s answer' });
    expect(answer).toHaveTextContent('…a1b2c3: BadDeviceToken');
    expect(answer).not.toHaveTextContent('The router refused this.');
    vi.unstubAllGlobals();
    document.body.innerHTML = '';
    show({ push: { ...PUSH, devices: [] }, more: { 'POST /api/push/test': ok({ ok: false, results: [] }) } });
    fireEvent.click(await screen.findByRole('button', { name: 'Send a test' }));
    expect(await screen.findByRole('status', { name: 'The test’s answer' })).toHaveTextContent('No phone is registered, so the test went nowhere.');
  });

  it('ST-C90, ST-C18: Remove key asks first, saying phones stop being told until a key is stored again', async () => {
    const r = show({ more: { 'POST /api/push/config': ok({ ok: true, configured: false }) } });
    fireEvent.click(within(await apple()).getByRole('button', { name: 'Remove key' }));
    const dialog = await screen.findByRole('dialog');
    expect(dialog).toHaveTextContent('Phones stop being told when a session waits on you, until a key is stored again.');
    expect(dialog).toHaveTextContent('Apple lets a key be downloaded only once');
    expect(r.posts('/api/push/config')).toEqual([]);
    const confirm = within(dialog).getByRole('button', { name: 'Remove key' });
    expect(confirm.className).toMatch(/cds--btn--danger/);
    fireEvent.click(confirm);
    await waitFor(() => expect(r.posts('/api/push/config')).toEqual([{ clear: true }]));
  });

  it('ST-C91: each phone is a compact row with its name, token tail, environment, platform and last result or registration; with none, how to register', async () => {
    show();
    const list = await screen.findByRole('list', { name: 'Registered phones' });
    const rows = within(list).getAllByRole('listitem');
    expect(rows[0]).toHaveTextContent('Pocket · …a1b2c3 · production · iPhone');
    expect(rows[0]).toHaveTextContent('delivered · 2 min ago');
    expect(rows[1]).toHaveTextContent('phone · …d4e5f6 · development · Android');
    expect(rows[1]).toHaveTextContent('registered 3 h ago');
    vi.unstubAllGlobals();
    document.body.innerHTML = '';
    show({ push: { ...PUSH, devices: [] } });
    expect(await screen.findByText('No phone has registered yet — in the app, Settings → Notifications → Enable.')).toBeInTheDocument();
  });

  it('ST-C92: the router’s last push failure is shown in the error tone', async () => {
    show({ push: { ...PUSH, last_error: 'ExpiredProviderToken' } });
    const failure = await screen.findByText('Last failure: ExpiredProviderToken');
    expect(failure.className).toMatch(/_bad_/);
  });

  it('ST-C93: Firebase is shown beside Apple — configured or not, its project, key file and last error — with a box for the service-account JSON whole', async () => {
    const r = show({
      push: {
        ...PUSH,
        fcm: { configured: true, project_id: 'sample-project', client_email: 'push@sample-project.iam.gserviceaccount.com', key_file: '/Users/me/.agent-router/sample-project.json', last_error: 'UNREGISTERED' },
      },
      more: { 'POST /api/push/config': ok({ ok: true, configured: true, project_id: 'sample-project' }) },
    });
    const region = await screen.findByRole('region', { name: 'Firebase (Android)' });
    expect(within(region).getByText('configured')).toBeInTheDocument();
    expect(within(region).getByText('Project').nextElementSibling).toHaveTextContent('sample-project');
    expect(within(region).getByText('Key file').nextElementSibling).toHaveTextContent('/Users/me/.agent-router/sample-project.json');
    expect(within(region).getByText('Last failure: UNREGISTERED').className).toMatch(/_bad_/);
    const box = within(region).getByLabelText('Service-account JSON');
    expect(box).toHaveValue('');
    fireEvent.change(box, { target: { value: '{"type":"service_account"}' } });
    fireEvent.click(within(region).getByRole('button', { name: 'Replace service account' }));
    await waitFor(() => expect(r.posts('/api/push/config')).toEqual([{ platform: 'fcm', service_account: '{"type":"service_account"}' }]));
    vi.unstubAllGlobals();
    document.body.innerHTML = '';
    show();
    expect(within(await screen.findByRole('region', { name: 'Firebase (Android)' })).getByText('not configured')).toBeInTheDocument();
  });

  it('ST-C94, ST-C102: the page explains what the key is for and where it lives, on a hosted router naming the deployment’s push Secret', async () => {
    show();
    const local = (await screen.findByText(/^The iPhone app is told the moment a session waits on you/)).closest('p');
    expect(local).toHaveTextContent('That needs an APNs key from your developer account (Keys → + → Apple Push Notifications service)');
    expect(local).toHaveTextContent("It stays in the router's secrets file, never in the database.");
    vi.unstubAllGlobals();
    document.body.innerHTML = '';
    show({ auth: HOSTED });
    const hosted = (await screen.findByText(/^The iPhone app is told the moment a session waits on you/)).closest('p');
    expect(hosted).toHaveTextContent('in the deployment’s push Secret, or paste it here once');
  });
});
