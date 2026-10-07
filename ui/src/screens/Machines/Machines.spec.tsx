// New for agent-router: the Machines screen as a reader meets it (docs/web-console-contracts.md,
// "Machines"), against the router's answers: on a hosted router the commands that connect a
// machine, always first, and every machine the router has seen, compared with the router's own
// build; on a local router the sentence that says why there is nothing here. The clock is pinned, so
// "online for 3h" is a fact the spec can state.
import { act, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { AGENTLESS, CONNECT_HOW, ON_THIS_MACHINE, OTHER_BUILD, SAME_BUILD, SEEN, WITH_VS_CODE, type MachineRow } from '~/model/machine';
import { HOSTED, LOCAL, ok, stubRouter, type Answer } from '~/screens/Settings/testing';
import { json, press, refusal, renderScreen, unreachable } from '~/test-utils';
import Machines from './index';

const NOW = Date.UTC(2026, 9, 7, 12, 0, 0);
const HOUR = 3600e3;
/// The router's build, and another one, as `tree:<sha>`.
const BUILD = 'tree:5d1c3b7f9e08a2e94b6dc41d07e28f3a4b6d0f13';
const OLDER = 'tree:0aa1bb2cc3dd4ee5ff6a7b8c9d0e1f2a3b4c5d6e';
const HEALTH = { ok: true, pid: 4242, version: '0.9.0', build: BUILD };

const machine = (id: string, label: string, extra: Partial<MachineRow> = {}): MachineRow => ({
  id,
  label,
  platform: 'macos',
  version: '0.9.0',
  build: BUILD,
  online: true,
  since_ms: NOW - 3 * HOUR,
  ...extra,
});

/// In the router's own order it would be studio, Laptop, win-box; the screen puts the online ones
/// first, by label with case aside, and the offline one last.
const MACHINES: MachineRow[] = [
  machine('9b3e1d47', 'win-box', { platform: 'windows', online: false, since_ms: NOW - 2 * 24 * HOUR, build: OLDER }),
  machine('5f0c2a8e', 'studio'),
  machine('c41d07e2', 'Laptop', { platform: 'linux', since_ms: NOW - 25 * 60e3, build: OLDER }),
];

/// A hosted router, its health and its machines, whatever `more` answers first.
function hosted(machines: Answer = ok(MACHINES), more: Record<string, Answer> = {}) {
  return stubRouter({ '/api/auth': ok(HOSTED), '/_router/health': ok(HEALTH), '/api/machines': machines, ...more });
}

const show = () => renderScreen(<Machines />, { path: '/machines', at: '/machines' });
const item = (id: string) => {
  const li = document.querySelector<HTMLElement>(`[data-machine="${id}"]`);
  if (!li) throw new Error(`no machine ${id}`);
  return li;
};
const items = () => [...document.querySelectorAll<HTMLElement>('[data-machine]')].map((li) => li.dataset['machine']);
const section = (name: string) => screen.getByRole('region', { name });
const never = () => new Promise<Response>(() => {});

beforeEach(() => {
  vi.useFakeTimers({ shouldAdvanceTime: true, toFake: ['Date'] });
  vi.setSystemTime(NOW);
});

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe('Machines on a hosted router', () => {
  it('MA-C1, MA-C2: names itself, and first says how to connect a machine: the installer’s command for each platform at the page’s own origin, each with a copy button that copies it, and what else the script installs', async () => {
    hosted();
    show();
    expect(await screen.findByRole('heading', { level: 1, name: 'Machines' })).toBeInTheDocument();
    expect(document.title).toBe('Machines · Agent Router');
    const connect = await screen.findByRole('region', { name: 'Connect a machine' });
    // First, above the machines, whatever the list holds.
    await waitFor(() => item('5f0c2a8e'));
    expect(connect.compareDocumentPosition(section(SEEN)) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(within(connect).getByText(CONNECT_HOW)).toBeInTheDocument();

    const commands = [
      ['macOS and Linux', `curl -fsSL ${location.origin}/agent/install.sh | sh`],
      ['Windows (PowerShell)', `irm ${location.origin}/agent/install.ps1 | iex`],
    ] as const;
    const writeText = vi.fn(async (_text: string): Promise<void> => {});
    Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true });
    try {
      for (const [label, command] of commands) {
        const install = within(connect).getByRole('group', { name: label });
        expect(within(install).getByRole('textbox', { name: `The command to run on ${label}` })).toHaveTextContent(command);
        await press(within(install).getByRole('button', { name: 'Copy command' }));
        expect(writeText, label).toHaveBeenLastCalledWith(command);
        expect(within(install).getByText('Copied')).toBeInTheDocument();
      }
    } finally {
      Reflect.deleteProperty(navigator, 'clipboard');
    }
    expect(within(connect).getByText(WITH_VS_CODE)).toBeInTheDocument();
  });

  it('MA-C3, MA-C4: lists every machine online first, then by label, each with its platform in words, online or offline and for how long, and whether it runs the router’s own build', async () => {
    const router = hosted();
    show();
    await waitFor(() => expect(items()).toEqual(['c41d07e2', '5f0c2a8e', '9b3e1d47']));
    // At a glance, over the list.
    expect(within(section(SEEN)).getByText('2 online · 1 offline')).toBeInTheDocument();

    const laptop = item('c41d07e2');
    expect(laptop).toHaveTextContent('Laptop');
    expect(laptop).toHaveTextContent('Linux');
    expect(laptop).toHaveTextContent('online for 25m');
    const studio = item('5f0c2a8e');
    expect(studio).toHaveTextContent('studio');
    expect(studio).toHaveTextContent('macOS');
    expect(studio).toHaveTextContent('online for 3h');
    const winBox = item('9b3e1d47');
    expect(winBox).toHaveTextContent('win-box');
    expect(winBox).toHaveTextContent('Windows');
    expect(winBox).toHaveTextContent('offline for 2d');
    // A dot of its own each, whose words are beside it: green while online, a hollow ring once not.
    expect(laptop.querySelector('[data-tone]')).toHaveAttribute('data-tone', 'busy');
    expect(winBox.querySelector('[data-tone]')).toHaveAttribute('data-tone', 'closed');
    // Read aloud as one line, its parts apart.
    expect(laptop).toHaveTextContent(/Laptop, Linux, online for 25m, Runs a different build/);

    // The router's own build is said quietly; another is said with what updates it.
    await waitFor(() => expect(studio).toHaveTextContent(SAME_BUILD));
    expect(within(studio).getByText(SAME_BUILD)).toHaveAttribute('data-same');
    for (const other of [laptop, winBox]) {
      expect(other).toHaveTextContent(OTHER_BUILD);
      expect(within(other).getByText(OTHER_BUILD)).not.toHaveAttribute('data-same');
      expect(other).not.toHaveTextContent(SAME_BUILD);
    }
    expect(OTHER_BUILD).toMatch(/run the installer on it again to update it\.$/);
    expect(router.gets('/api/machines').length).toBeGreaterThan(0);
  });

  it('MA-C4: says nothing of a machine’s build until the router’s own build is known', async () => {
    hosted(ok(MACHINES), { '/_router/health': () => unreachable() });
    show();
    await waitFor(() => item('5f0c2a8e'));
    expect(item('5f0c2a8e')).toHaveTextContent('online for 3h');
    expect(screen.queryByText(SAME_BUILD)).toBeNull();
    expect(screen.queryByText(OTHER_BUILD)).toBeNull();
  });

  it('MA-C5: with no machine yet, one line under the commands says none has connected', async () => {
    hosted(ok([]));
    show();
    const seen = await screen.findByRole('region', { name: SEEN });
    expect(await within(seen).findByText(AGENTLESS)).toBeInTheDocument();
    expect(AGENTLESS).toBe('No machine has connected yet.');
    expect(within(seen).queryByRole('list')).toBeNull();
    // The commands are still there to connect the first.
    expect(within(section('Connect a machine')).getByRole('group', { name: 'macOS and Linux' })).toBeInTheDocument();
  });

  it('MA-C6: a failed read keeps the machines it read, with their age and the router’s words, and the commands', async () => {
    let failing = false;
    hosted(() => (failing ? refusal('database is locked', 500) : json(MACHINES)));
    const { client } = show();
    await waitFor(() => item('5f0c2a8e'));
    failing = true;
    await act(() => client.refetchQueries({ queryKey: ['machines'] }));
    expect(await screen.findByText(/^Showing what was read .*: the last read failed — database is locked$/)).toBeInTheDocument();
    expect(items()).toEqual(['c41d07e2', '5f0c2a8e', '9b3e1d47']);
    expect(screen.queryByText(AGENTLESS)).toBeNull();
    expect(section('Connect a machine')).toBeInTheDocument();
  });

  it('MA-C6: a read that never succeeded says it could not read the machines in the router’s words, never that none has connected, and a reply that is not a list is no machines', async () => {
    hosted(() => unreachable());
    const first = show();
    expect(await screen.findByText('Could not read the machines')).toBeInTheDocument();
    expect(screen.getByText('Could not reach the router.')).toBeInTheDocument();
    expect(screen.queryByText(AGENTLESS)).toBeNull();
    expect(section('Connect a machine')).toBeInTheDocument();
    first.unmount();

    hosted(ok({ machines: MACHINES }));
    show();
    expect(await screen.findByText('Could not read the machines')).toBeInTheDocument();
    expect(screen.getByText('The router answered something that is not a list of machines.')).toBeInTheDocument();
    expect(screen.queryByText(AGENTLESS)).toBeNull();
  });
});

describe('Machines on a local router', () => {
  it('MA-C7: says machines connect to a hosted router and this one runs on this machine, offering no commands and reading no machines', async () => {
    const router = stubRouter({ '/api/auth': ok(LOCAL), '/_router/health': ok(HEALTH) });
    show();
    expect(await screen.findByText(ON_THIS_MACHINE)).toBeInTheDocument();
    expect(ON_THIS_MACHINE).toMatch(/^Machines connect to a hosted router\. This router runs on this machine/);
    expect(screen.getByRole('heading', { level: 1, name: 'Machines' })).toBeInTheDocument();
    expect(screen.queryByRole('region', { name: 'Connect a machine' })).toBeNull();
    expect(screen.queryByRole('button', { name: 'Copy command' })).toBeNull();
    expect(screen.queryByText(/agent\/install/)).toBeNull();
    expect(router.gets('/api/machines')).toEqual([]);
  });

  it('MA-C7: until the router has said which kind it is, offers neither kind, and a failed answer says so', async () => {
    const asking = stubRouter({ '/api/auth': never, '/_router/health': ok(HEALTH) });
    const first = show();
    expect(await screen.findByText('Asking the router what kind it is…')).toBeInTheDocument();
    expect(screen.queryByText(ON_THIS_MACHINE)).toBeNull();
    expect(screen.queryByRole('region', { name: 'Connect a machine' })).toBeNull();
    expect(asking.gets('/api/machines')).toEqual([]);
    first.unmount();

    stubRouter({ '/api/auth': () => unreachable(), '/_router/health': ok(HEALTH) });
    show();
    expect(await screen.findByText('Could not read what kind of router this is')).toBeInTheDocument();
    expect(screen.getByText('Could not reach the router.')).toBeInTheDocument();
    expect(screen.queryByRole('region', { name: 'Connect a machine' })).toBeNull();
  });
});
