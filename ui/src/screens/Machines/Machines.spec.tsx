// New for agent-router: the Machines screen as a reader meets it (docs/web-console-contracts.md,
// "Machines"), against the router's answers: on a hosted router the commands that connect a
// machine, always first, and every machine the router has seen, compared with the router's own
// build; on a local router the sentence that says why there is nothing here. The clock is pinned, so
// "online for 3h" is a fact the spec can state.
import { act, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { AGENTLESS, CONNECT_HOW, NO_CLIENTS, ON_THIS_MACHINE, OTHER_BUILD, SAME_BUILD, SEEN, WITH_VS_CODE, type MachineRow } from '~/model/machine';
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

describe('Machines and their clients', () => {
  const standing = (state: string, extra: Record<string, unknown> = {}) => ({ state, behind: 0, failed: 0, replaced: [], at_ms: NOW - HOUR, ...extra });
  const CLAUDE = {
    client: 'claude-code',
    cli: { path: '/u/.local/bin/claude', version: '2.1.267' },
    extensions: [
      { editor: 'vscode', version: '2.1.296' },
      { editor: 'cursor', version: '2.1.290' },
    ],
    home: '/u/.claude',
  };
  const KIMI = { client: 'kimi-code', cli: { path: '/u/.kimi-code/bin/kimi', version: '2.1.1' }, extensions: [], home: '/u/.kimi-code' };
  const clientsOf = (id: string) => within(item(id)).getByRole('list', { name: /^Clients on / });
  const lineOf = (id: string, client: string) => {
    const li = item(id).querySelector<HTMLElement>(`[data-client="${client}"]`);
    if (!li) throw new Error(`no ${client} on ${id}`);
    return li;
  };

  it('MA-C8: each machine lists its clients in the order its agent reported them, each with its CLI’s version, each editor’s extension and whether its home folder is there', async () => {
    hosted(
      ok([
        machine('a1', 'studio', { clients: [CLAUDE, KIMI] }),
        machine('b2', 'laptop', { clients: [{ client: 'kimi-code', cli: { path: '/u/kimi', version: null }, extensions: [{ editor: 'vscode', version: '0.8.1' }], home: null }] }),
        machine('c3', 'odd', { clients: [{ client: 'new-thing', cli: null, extensions: [{ editor: 'zed', version: '1.0' }], home: '/u/.new' }] }),
      ]),
    );
    show();
    await waitFor(() => item('a1'));
    expect([...clientsOf('a1').querySelectorAll('[data-client]')].map((li) => li.getAttribute('data-client'))).toEqual(['claude-code', 'kimi-code']);
    expect(lineOf('a1', 'claude-code')).toHaveTextContent(/^Claude Code: CLI 2\.1\.267 · VS Code 2\.1\.296 · Cursor 2\.1\.290 · home folder$/);
    expect(lineOf('a1', 'kimi-code')).toHaveTextContent(/^Kimi Code: CLI 2\.1\.1 · no editor extension · home folder$/);
    // A command that did not answer, and no home folder.
    expect(lineOf('b2', 'kimi-code')).toHaveTextContent(/^Kimi Code: CLI, version not read · VS Code 0\.8\.1 · no home folder$/);
    // A client and an editor the console does not know, as the agent sent them.
    expect(lineOf('c3', 'new-thing')).toHaveTextContent(/^new-thing: no CLI · zed 1\.0 · home folder$/);
  });

  it('MA-C9: each client’s line ends with where its own Library stands, Claude Code’s from the machine’s and Kimi Code’s from library.clients; the machine does not say it twice, and keeps its sync', async () => {
    hosted(
      ok([
        machine('a1', 'studio', { clients: [CLAUDE, KIMI], library: { ...standing('in_step'), clients: { 'kimi-code': standing('behind', { behind: 1 }) } } } as Partial<MachineRow>),
        machine('b2', 'laptop', { clients: [CLAUDE, KIMI], library: standing('in_step') } as Partial<MachineRow>),
        machine('c3', 'kimi-only', { clients: [KIMI], library: standing('in_step') } as Partial<MachineRow>),
      ]),
    );
    show();
    await waitFor(() => item('a1'));
    const said = (el: HTMLElement) => el.querySelector<HTMLElement>('[data-client-library]');
    expect(said(lineOf('a1', 'claude-code'))).toHaveTextContent(/^, Library in step$/);
    expect(said(lineOf('a1', 'claude-code'))?.dataset['clientLibrary']).toBe('well');
    expect(said(lineOf('a1', 'kimi-code'))).toHaveTextContent(/^, Library behind on 1 item$/);
    expect(said(lineOf('a1', 'kimi-code'))?.dataset['clientLibrary']).toBe('needs');
    // Claude Code's line says the machine's standing, so the machine does not say it again.
    expect(item('a1').querySelector('[data-library]')).toBeNull();
    expect(within(item('a1')).getByRole('button', { name: 'Sync its Library now' })).toBeInTheDocument();
    // A client with no standing of its own says nothing of the Library.
    expect(said(lineOf('b2', 'kimi-code'))).toBeNull();
    // With no Claude Code line to say it, the machine says its own, as before.
    expect(item('c3').querySelector('[data-library]')).toHaveTextContent('Library in step');
  });

  it('MA-C10: a machine whose agent reported no client says so in one sentence, never an empty list, and keeps its Library line', async () => {
    hosted(ok([machine('a1', 'studio', { clients: [], library: standing('in_step') } as Partial<MachineRow>), machine('b2', 'laptop')]));
    show();
    await waitFor(() => item('a1'));
    for (const id of ['a1', 'b2']) {
      expect(within(item(id)).getByText(NO_CLIENTS)).toBeInTheDocument();
      expect(within(item(id)).queryByRole('list', { name: /^Clients on / })).toBeNull();
    }
    expect(NO_CLIENTS).toBe('Its agent has reported no Claude Code, Kimi Code or Codex.');
    expect(item('a1').querySelector('[data-library]')).toHaveTextContent('Library in step');
  });
});

describe('Machines and the Library', () => {
  const standing = (state: string, extra: Record<string, unknown> = {}) => ({ state, behind: 0, failed: 0, replaced: [], at_ms: NOW - HOUR, ...extra });

  it('LB-C11: each machine says where its Library stands, marked when it wants something done, and a router that keeps none says nothing of it', async () => {
    hosted(
      ok([
        machine('a1', 'studio', { library: standing('in_step') } as Partial<MachineRow>),
        machine('b2', 'laptop', { library: standing('behind', { behind: 2 }) } as Partial<MachineRow>),
        machine('c3', 'old-box', { library: standing('predates') } as Partial<MachineRow>),
        machine('d4', 'quiet', { library: standing('opted_out') } as Partial<MachineRow>),
        machine('e5', 'bench', { library: standing('replaced', { replaced: ['skill/bench'] }) } as Partial<MachineRow>),
        machine('f6', 'win-box', { online: false, library: standing('failed', { failed: 1 }) } as Partial<MachineRow>),
        machine('g7', 'new', { library: standing('not_yet') } as Partial<MachineRow>),
        machine('h8', 'plain'),
      ]),
    );
    show();
    await waitFor(() => item('a1'));
    const library = (id: string) => item(id).querySelector<HTMLElement>('[data-library]');
    const words = (id: string) => library(id)?.textContent?.replace(/^, /, '');
    expect(words('a1')).toBe('Library in step');
    expect(library('a1')?.dataset['library']).toBe('well');
    expect(words('b2')).toBe('Library behind on 2 items');
    expect(library('b2')?.dataset['library']).toBe('needs');
    expect(words('c3')).toBe('Its agent predates the Library — run the installer on it again');
    expect(words('d4')).toBe('Takes no part in the Library');
    expect(words('e5')).toBe('Library in step; it put back skill/bench over a copy changed there');
    expect(words('f6')).toBe('Library: 1 item could not be placed');
    expect(words('g7')).toBe('Nothing heard of its Library yet');
    expect(library('h8')).toBeNull();
  });

  it('LB-C12: an online machine offers a sync of its Library alone, which is sent for it and says its answer; an offline one offers none', async () => {
    const stub = hosted(
      ok([
        machine('a1', 'studio', { library: standing('behind', { behind: 1 }) } as Partial<MachineRow>),
        machine('f6', 'win-box', { online: false, library: standing('in_step') } as Partial<MachineRow>),
      ]),
      { 'POST /api/library/sync': ok({ ok: true, machines: [{ machine: 'a1', label: 'studio', outcome: 'synced', report: { items: [{ outcome: 'placed' }] } }] }) },
    );
    show();
    await waitFor(() => item('a1'));
    expect(within(item('f6')).queryByRole('button', { name: 'Sync its Library now' })).toBeNull();
    await press(within(item('a1')).getByRole('button', { name: 'Sync its Library now' }));
    expect(await within(item('a1')).findByText('studio: synced, 1 changed')).toBeInTheDocument();
    expect(stub.posts('/api/library/sync')).toEqual([{ machine: 'a1' }]);
  });
});
