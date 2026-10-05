// New for agent-router: Settings → Accounts opening by itself from the Overview (docs/web-console-contracts.md,
// "Pages and tabs" and "Hosted router"): once per load, only from the Overview as the load opened it,
// when nothing can sign or a credential that could sign does not work. Each case is a load of its own:
// the modules are loaded afresh, since "once per load" is module state.
import { act, configure, fireEvent, screen, waitFor, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { Account } from '~/model/settings';
import { renderScreen } from '~/test-utils';
import { held, HOSTED, LOCAL, ok, router } from './fixtures';

/// The whole page is drawn in jsdom, beside every other spec in the run: generous limits, so a loaded
/// machine is not mistaken for a broken page. What "at once" means is asserted on the order of events,
/// never on how long they took.
vi.setConfig({ testTimeout: 30_000 });
configure({ asyncUtilTimeout: 10_000 });

vi.mock('@carbon/charts-react', () => ({ StackedBarChart: () => <svg data-testid="chart" /> }));

/// A fresh load of the Overview: its module state, the one check and the one decision, unspent.
async function load() {
  vi.resetModules();
  return (await import('./index')).default;
}

const where = () => screen.getByTestId('location').textContent ?? '';
const loaded = () => screen.findByRole('heading', { level: 2, name: 'Totals in scope' }, { timeout: 20_000 });

const token = (id: string, over: Partial<Account> = {}): Account => ({ id, label: id, email: `${id}@example.test`, source: 'token', active: 0, ready: true, ...over });
const login = (over: Partial<Account> = {}): Account => ({ id: 'default', label: 'default', email: 'me@example.test', source: 'keychain', active: 0, ready: true, ...over });

/// A router whose checked accounts are `checked` and whose mode is `auth`.
const routerWith = (checked: Account[], auth = LOCAL) =>
  router((path, q) => (path === '/api/accounts' && q.get('probe') === '1' ? ok(checked) : path === '/api/auth' ? ok(auth) : undefined));
const probes = (r: ReturnType<typeof router>) => r.calls('/api/accounts').filter((c) => c.query.get('probe') === '1');

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('Settings → Accounts, opened by the Overview', () => {
  it('ST-C7, OV-C209: opens by itself when nothing can sign, once per load, so a reader who leaves it is never dragged back', async () => {
    const Overview = await load();
    const r = routerWith([]);
    const first = renderScreen(<Overview />, { path: '/', at: '/' });
    await waitFor(() => expect(screen.getByTestId('elsewhere')).toBeInTheDocument());
    expect(where()).toBe('/settings/accounts');
    first.unmount();
    // The same load, back on the Overview: nothing opens again, and nothing is checked again.
    renderScreen(<Overview />, { path: '/', at: '/' });
    await loaded();
    await new Promise((done) => setTimeout(done, 50));
    expect(where()).toBe('/');
    expect(probes(r)).toHaveLength(1);
  });

  it('ST-C7: a credential that could sign and does not work opens it; when every signer is ready, nothing interrupts', async () => {
    let Overview = await load();
    routerWith([token('work', { active: 1 }), login({ ready: false, detail: 'not logged in' })]);
    let view = renderScreen(<Overview />, { path: '/', at: '/' });
    await waitFor(() => expect(where()).toBe('/settings/accounts'));
    view.unmount();

    Overview = await load();
    const r = routerWith([token('work', { active: 1 }), login()]);
    view = renderScreen(<Overview />, { path: '/', at: '/' });
    await loaded();
    await waitFor(() => expect(probes(r)).toHaveLength(1));
    await new Promise((done) => setTimeout(done, 50));
    expect(where()).toBe('/');
  });

  it('ST-C8: on a hosted router only token accounts and the active one count, so a Claude Code login not in use never interrupts', async () => {
    let Overview = await load();
    const r = routerWith([token('work', { active: 1 }), login({ ready: false })], HOSTED);
    let view = renderScreen(<Overview />, { path: '/', at: '/' });
    await loaded();
    await waitFor(() => expect(probes(r)).toHaveLength(1));
    await new Promise((done) => setTimeout(done, 50));
    expect(where()).toBe('/');
    view.unmount();

    // The same login while it is the one in use does interrupt.
    Overview = await load();
    routerWith([token('work'), login({ active: 1, ready: false })], HOSTED);
    view = renderScreen(<Overview />, { path: '/', at: '/' });
    await waitFor(() => expect(where()).toBe('/settings/accounts'));
  });

  it('ST-C7, OV-C167: the load makes one credential check, every other read of the accounts checks none, and only the Overview as the load opened it interrupts', async () => {
    const Overview = await load();
    const answer = held();
    const r = router((path, q) => (path === '/api/accounts' && q.get('probe') === '1' ? answer.promise : path === '/api/auth' ? ok(LOCAL) : undefined));
    renderScreen(<Overview />, { path: '/', at: '/' });
    await loaded();
    await waitFor(() => expect(probes(r)).toHaveLength(1));
    expect(r.calls('/api/accounts').filter((c) => !c.query.has('probe')).length).toBeGreaterThan(0);
    // The reader goes somewhere before the check answers: it answers "nothing can sign", and the page
    // they chose stays theirs.
    fireEvent.click(within(screen.getByRole('heading', { level: 2, name: 'Projects' }).closest('section')!).getByRole('button', { name: 'agent-router' }));
    await waitFor(() => expect(where()).toBe('/?project=%2Fcode%2Fagent-router'));
    await act(async () => {
      answer.release(ok([]));
      await new Promise((done) => setTimeout(done, 50));
    });
    expect(where()).toBe('/?project=%2Fcode%2Fagent-router');
    expect(probes(r)).toHaveLength(1);
  });
});
