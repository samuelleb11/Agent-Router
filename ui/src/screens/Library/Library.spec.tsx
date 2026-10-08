// New for agent-router: the Library screen as a reader meets it (docs/web-console-contracts.md, "Library"),
// against the router's answers: on a hosted router every item at a glance, the item opened by its address
// with its file, its files and its versions, the decisions on each, adding a version, and telling the
// machines to sync, and how each item is used and what the Library's skills add to every turn; on a local
// router the sentence that says why there is nothing here.
import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { EMPTY, NOT_HERE, listingWords, type ItemDetail, type ItemRow, type ItemUsage, type Provenance, type State, type Usage, type Version } from '~/model/library';
import { HOSTED, LOCAL, ok, stubRouter, type Answer } from '~/screens/Settings/testing';
import { json, press, refusal, renderScreen, unreachable } from '~/test-utils';
import Library from './index';

const NOW = Date.UTC(2026, 9, 8, 12, 0, 0);
const HOUR = 3600e3;
const SESSION = 'd5f37e26-c662-492c-b0af-8895c68d0ffe';

const brief = (number: number, state: State, provenance: Provenance = { from: 'upload' }) => ({
  number,
  digest: `digest-${number}`,
  size: 200,
  state,
  provenance,
  created_at: NOW - (10 - number) * HOUR,
});

const row = (kind: ItemRow['kind'], name: string, extra: Partial<ItemRow> = {}): ItemRow => ({
  kind,
  name,
  description: `What ${name} is for.`,
  source: null,
  created_at: NOW - 48 * HOUR,
  archived_at: null,
  live: brief(1, 'live'),
  newest: brief(1, 'live'),
  pending: 0,
  versions: 1,
  ...extra,
});

/// In the router's own order: a subagent, a skill, a command, a skill. The command comes as a router
/// that counts no usage sends it.
const ROWS: ItemRow[] = [
  row('subagent', 'reviewer', { live: null, newest: brief(1, 'pending'), pending: 1, uses_30d: 0, last_used_at: null }),
  row('skill', 'vault', { source: 'source:skills', live: brief(2, 'live'), uses_30d: 12, last_used_at: NOW - HOUR }),
  row('command', 'ship', { archived_at: NOW - HOUR }),
  row('skill', 'agent-router', { source: 'router', uses_30d: 1, last_used_at: NOW - 3 * HOUR }),
];

const DAY = 24 * HOUR;
/// The 30 days a usage report tells, oldest first.
const DAYS = Array.from({ length: 30 }, (_, i) => NOW - (29 - i) * DAY);
const daily = (counts: Record<number, number>) => DAYS.map((_, i) => counts[i] ?? 0);
const used = (name: string, extra: Partial<ItemUsage> = {}): ItemUsage => ({
  kind: 'skill',
  name,
  uses_30d: 0,
  last_used_at: null,
  daily: daily({}),
  machines: [],
  projects: [],
  ...extra,
});
const USAGE: Usage = {
  days: DAYS,
  listing: {
    at: NOW - HOUR,
    chars: 1600,
    tokens: 400,
    items: [
      { name: 'agent-router', chars: 400, tokens: 100 },
      { name: 'vault', chars: 1200, tokens: 300 },
    ],
  },
  items: [
    used('bench', {
      uses_30d: 5,
      last_used_at: NOW - 2 * HOUR,
      daily: daily({ 27: 2, 29: 3 }),
      machines: [
        { id: '5f0c2a8e0000', label: 'studio', uses: 4 },
        { id: null, label: null, uses: 1 },
      ],
      projects: [
        { project: '/work/app', uses: 3 },
        { project: null, uses: 2 },
      ],
    }),
    used('vault', { uses_30d: 12, last_used_at: NOW - HOUR }),
  ],
};

const version = (number: number, state: State, provenance: Provenance): Version => ({
  ...brief(number, state, provenance),
  kind: 'skill',
  name: 'bench',
  description: `Drives the bench, v${number}.`,
  decided_at: null,
});

const BENCH: ItemDetail = {
  kind: 'skill',
  name: 'bench',
  description: 'Drives the bench.',
  source: null,
  archived_at: null,
  versions: [
    version(4, 'pending', { from: 'machine', machine: '5f0c2a8e0000', label: 'studio', session: SESSION }),
    version(3, 'live', { from: 'upload' }),
    version(2, 'superseded', { from: 'token', token: 'ci' }),
    version(1, 'rejected', { from: 'forge', source: 'skills', forge: 'home', repository: 'whnet/skills', commit: '0123456789abcdef', path: 'library/skills/bench' }),
  ],
  files: [
    { path: 'SKILL.md', size: 120, executable: false },
    { path: 'scripts/run.sh', size: 18, executable: true },
  ],
};

const SKILL_3 = '---\nname: bench\ndescription: Drives the bench.\n---\n\n# Bench\n\nDrives the **bench**.\n\n<script>alert(1)</script>\n';
const SKILL_4 = '---\nname: bench\ndescription: Drives the bench.\n---\n\n# Bench\n\nDrives the **bench**, better.\n\n<script>alert(1)</script>\n';
const text = (body: string): Answer => () => new Response(body, { status: 200, headers: { 'Content-Type': 'text/plain' } });
const FILES = '/api/library/items/skill/bench/versions';

function hosted(more: Record<string, Answer> = {}) {
  return stubRouter({
    '/api/auth': ok(HOSTED),
    '/api/library': ok({ items: ROWS }),
    '/api/library/usage': ok(USAGE),
    '/api/library/items/skill/bench': ok(BENCH),
    [`${FILES}/3/files/SKILL.md`]: text(SKILL_3),
    [`${FILES}/4/files/SKILL.md`]: text(SKILL_4),
    [`${FILES}/3/files/scripts/run.sh`]: text('#!/bin/sh\necho hi\n'),
    ...more,
  });
}

const show = (at = '/library') => renderScreen(<Library />, { path: '/library', at });
const items = () => [...document.querySelectorAll<HTMLElement>('[data-item]')].map((li) => li.dataset['item']);
const item = (key: string) => {
  const li = document.querySelector<HTMLElement>(`[data-item="${key}"]`);
  if (!li) throw new Error(`no item ${key}`);
  return li;
};
const versionRow = (n: number) => {
  const li = document.querySelector<HTMLElement>(`[data-version="${n}"]`);
  if (!li) throw new Error(`no version ${n}`);
  return li;
};
const opened = () => screen.findByRole('region', { name: 'Skill bench' });

beforeEach(() => {
  vi.useFakeTimers({ shouldAdvanceTime: true, toFake: ['Date'] });
  vi.setSystemTime(NOW);
});

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe('the Library on a hosted router', () => {
  it('LB-C1: names itself, and on a local router says it keeps no Library, reading nothing but what kind it is', async () => {
    hosted();
    const first = show();
    expect(await screen.findByRole('heading', { level: 1, name: 'Library' })).toBeInTheDocument();
    expect(document.title).toBe('Library · Agent Router');
    await waitFor(() => item('skill/vault'));
    first.unmount();
    vi.unstubAllGlobals();

    const local = stubRouter({ '/api/auth': ok(LOCAL) });
    show();
    expect(await screen.findByText(NOT_HERE)).toBeInTheDocument();
    expect(local.said()).toEqual(['GET /api/auth']);
    expect(screen.queryByRole('button', { name: 'Publish' })).not.toBeInTheDocument();
  });

  it('LB-C2, LB-C3: lists every item by kind then name, each with its kind, what it is for, its live version, where its versions come from, what waits and whether it is archived; an empty Library says what adding one does', async () => {
    hosted();
    const first = show();
    await waitFor(() => item('skill/vault'));
    expect(items()).toEqual(['skill/agent-router', 'skill/vault', 'command/ship', 'subagent/reviewer']);
    const vault = within(item('skill/vault'));
    expect(vault.getByRole('link', { name: 'Skill vault' })).toHaveAttribute('href', '/library?item=skill/vault');
    expect(vault.getByText('What vault is for.')).toBeInTheDocument();
    expect(vault.getByText('v2 live')).toBeInTheDocument();
    expect(vault.getByText('imported by the source skills')).toBeInTheDocument();
    expect(within(item('skill/agent-router')).getByText('comes with the router’s build')).toBeInTheDocument();
    const reviewer = within(item('subagent/reviewer'));
    expect(reviewer.getByText('none live')).toBeInTheDocument();
    expect(reviewer.getByText('1 waiting for approval')).toBeInTheDocument();
    expect(reviewer.getByText('uploaded and published here')).toBeInTheDocument();
    expect(within(item('command/ship')).getByText('archived')).toBeInTheDocument();
    first.unmount();
    vi.unstubAllGlobals();

    hosted({ '/api/library': ok({ items: [] }) });
    show();
    expect(await screen.findByText(EMPTY)).toBeInTheDocument();
  });

  it('LB-C13: a read that fails is not an empty Library — one never read says it could not be read, and the last good list is kept and marked with its age', async () => {
    hosted({ '/api/library': unreachable });
    const first = show();
    expect(await screen.findByText('Could not read the Library')).toBeInTheDocument();
    expect(screen.queryByText(EMPTY)).not.toBeInTheDocument();
    first.unmount();
    vi.unstubAllGlobals();

    let reads = 0;
    hosted({ '/api/library': () => (reads++ === 0 ? json({ items: ROWS }) : refusal('the store is busy', 500)) });
    const { client } = show();
    await waitFor(() => item('skill/vault'));
    await act(async () => {
      await client.refetchQueries({ queryKey: ['library', 'list'] });
    });
    expect(await screen.findByText(/Showing what was read .*: the last read failed — the store is busy/)).toBeInTheDocument();
    expect(items()).toHaveLength(4);
  });

  it('LB-C4: an item opens by its address — its file as Claude Code reads it, without its frontmatter, as Markdown and never as HTML — and its files, each with its size and the executable mark, open as text', async () => {
    hosted();
    show('/library?item=skill/bench');
    const panel = await opened();
    expect(await within(panel).findByRole('heading', { name: 'Bench' })).toBeInTheDocument();
    const main = panel.querySelector('[data-main="SKILL.md"]')!;
    expect(main.textContent).not.toContain('name: bench');
    expect(within(main as HTMLElement).getByText('bench').closest('strong')).not.toBeNull();
    expect(main.querySelector('script')).toBeNull();
    expect(main.textContent).toContain('<script>alert(1)</script>');
    expect(within(item('skill/vault')).getByRole('link', { name: 'Skill vault' })).toBeInTheDocument();

    const files = [...panel.querySelectorAll<HTMLElement>('[data-file]')];
    expect(files.map((f) => f.dataset['file'])).toEqual(['SKILL.md', 'scripts/run.sh']);
    expect(within(files[1]!).getByText('18 bytes, executable')).toBeInTheDocument();
    await press(within(files[1]!).getByRole('button', { name: 'scripts/run.sh' }));
    await waitFor(() => expect(panel.querySelector('[data-text="scripts/run.sh"]')?.textContent).toBe('#!/bin/sh\necho hi\n'));
  });

  it('LB-C5: its versions, newest first, each with its state and where it came from in words, the session that published one opening from it, and when', async () => {
    hosted();
    show('/library?item=skill/bench');
    await opened();
    await waitFor(() => versionRow(4));
    const order = [...document.querySelectorAll<HTMLElement>('[data-version]')].map((v) => v.dataset['version']);
    expect(order).toEqual(['4', '3', '2', '1']);
    const four = within(versionRow(4));
    expect(four.getByText('waits for approval')).toBeInTheDocument();
    expect(four.getByText('published from studio by a Claude Code session')).toBeInTheDocument();
    expect(four.getByRole('link', { name: 'open the session' })).toHaveAttribute('href', `/live?session=${SESSION}`);
    expect(within(versionRow(3)).getByText('uploaded from the console')).toBeInTheDocument();
    expect(within(versionRow(3)).getByText('live')).toBeInTheDocument();
    expect(within(versionRow(2)).getByText('published with the token ci')).toBeInTheDocument();
    expect(within(versionRow(1)).getByText('imported from whnet/skills at 01234567, library/skills/bench')).toBeInTheDocument();
  });

  it('LB-C6: a waiting version offers Approve and Reject, a superseded one Make live again, the live and the rejected nothing; each is sent once, says what it did or the router’s refusal, and the Library is read again', async () => {
    const stub = hosted({
      [`POST ${FILES}/4/live`]: ok({ ok: true, version: version(4, 'live', { from: 'upload' }) }),
      [`POST ${FILES}/2/live`]: () => refusal('version 2 of the skill bench was rejected', 409),
    });
    show('/library?item=skill/bench');
    await opened();
    await waitFor(() => versionRow(4));
    const buttons = (n: number) => within(versionRow(n)).queryAllByRole('button').map((b) => b.textContent);
    expect(buttons(4)).toEqual(['Approve', 'Reject', 'Compare with the live one']);
    expect(buttons(3)).toEqual([]);
    expect(buttons(2)).toEqual(['Make live again', 'Compare with the live one']);
    expect(buttons(1)).toEqual(['Compare with the live one']);

    const listReads = stub.gets('/api/library').length;
    await press(within(versionRow(4)).getByRole('button', { name: 'Approve' }));
    expect(await screen.findByText('Version 4 is live: every connected machine is told now.')).toBeInTheDocument();
    expect(stub.posts(`${FILES}/4/live`)).toHaveLength(1);
    await waitFor(() => expect(stub.gets('/api/library').length).toBeGreaterThan(listReads));

    await press(within(versionRow(2)).getByRole('button', { name: 'Make live again' }));
    expect(await screen.findByText('version 2 of the skill bench was rejected')).toBeInTheDocument();
  });

  it('LB-C7: compares a version with the live one, line by line, the added and the removed marked', async () => {
    hosted();
    show('/library?item=skill/bench');
    await opened();
    await waitFor(() => versionRow(4));
    await press(within(versionRow(4)).getByRole('button', { name: 'Compare with the live one' }));
    const compare = await waitFor(() => {
      const c = document.querySelector<HTMLElement>('[data-compare="3..4"]');
      if (!c?.querySelector('[data-diff]')) throw new Error('not yet');
      return c;
    });
    const marked = (kind: string) => [...compare.querySelectorAll(`[data-diff="${kind}"]`)].map((l) => l.textContent?.trim());
    expect(marked('removed')).toEqual(['- Drives the **bench**.']);
    expect(marked('added')).toEqual(['+ Drives the **bench**, better.']);
  });

  it('LB-C8: Archive asks once more before it takes the item off every machine, and an archived item says so', async () => {
    const stub = hosted({ 'POST /api/library/items/skill/bench/archive': ok({ ok: true }) });
    const first = show('/library?item=skill/bench');
    const panel = await opened();
    await press(await within(panel).findByRole('button', { name: 'Archive' }));
    expect(stub.posts('/api/library/items/skill/bench/archive')).toHaveLength(0);
    await press(within(panel).getByRole('button', { name: 'Take it off every machine' }));
    expect(await screen.findByText('The skill bench is archived: every machine takes it away.')).toBeInTheDocument();
    expect(stub.posts('/api/library/items/skill/bench/archive')).toHaveLength(1);
    first.unmount();
    vi.unstubAllGlobals();

    hosted({ '/api/library/items/skill/bench': ok({ ...BENCH, archived_at: NOW - HOUR }) });
    show('/library?item=skill/bench');
    const archived = await opened();
    expect(await within(archived).findByText('archived: on no machine until a version of it goes live again')).toBeInTheDocument();
    expect(within(archived).queryByRole('button', { name: 'Archive' })).not.toBeInTheDocument();
  });

  it('LB-C9: publishes a skill’s folder, packed here, or one file, under the name it comes with or one typed, and says what happened or the router’s refusal', async () => {
    const stub = hosted({
      'POST /api/library/items/skill/bench/versions': ok({ ok: true, unchanged: false, version: version(5, 'live', { from: 'upload' }) }, 201),
      'POST /api/library/items/command/ship-it/versions': () => refusal('ship.md line 3 holds what looks like a GitLab token', 422),
    });
    show();
    await waitFor(() => item('skill/vault'));
    const publish = screen.getByRole('button', { name: 'Publish' });
    expect(publish).toBeDisabled();

    // A folder, as the browser hands one over: every file by its path inside it.
    const file = (path: string, body: string) => {
      const f = new File([body], path.split('/').pop()!);
      Object.defineProperty(f, 'webkitRelativePath', { value: path });
      return f;
    };
    const folder = document.querySelector<HTMLInputElement>('[data-pick="folder"]')!;
    await act(async () => {
      fireEvent.change(folder, { target: { files: [file('bench/SKILL.md', SKILL_3), file('bench/.DS_Store', 'finder')] } });
    });
    expect(await screen.findByText('Chosen: the folder bench')).toBeInTheDocument();
    expect(screen.getByRole('textbox', { name: /Name/ })).toHaveAttribute('placeholder', 'bench');
    await waitFor(() => expect(publish).toBeEnabled());
    await press(publish);
    expect(await screen.findByText('Published the skill bench as version 5: live, and every connected machine is told now.')).toBeInTheDocument();
    const sent = stub.fetch.mock.calls.find(([url, init]) => String(url).endsWith('/api/library/items/skill/bench/versions') && init?.method === 'POST')!;
    expect((sent[1]!.headers as Record<string, string>)['Content-Type']).toBe('application/x-tar');
    const tar = await new Promise<string>((resolve) => {
      const reader = new FileReader();
      reader.onload = () => resolve(String(reader.result));
      reader.readAsText(sent[1]!.body as Blob);
    });
    expect(tar.startsWith('SKILL.md\0')).toBe(true);
    expect(tar).not.toContain('.DS_Store');

    // One file, under a name typed, and the router's refusal in its words.
    fireEvent.change(screen.getByRole('combobox', { name: 'Kind' }), { target: { value: 'command' } });
    const one = document.querySelector<HTMLInputElement>('[data-pick="file"]')!;
    await act(async () => {
      fireEvent.change(one, { target: { files: [new File(['---\ndescription: Ship.\n---\n'], 'ship.md')] } });
    });
    const name = screen.getByRole('textbox', { name: /Name/ });
    fireEvent.change(name, { target: { value: 'Ship It' } });
    expect(await screen.findByText(/cannot name an item/)).toBeInTheDocument();
    expect(publish).toBeDisabled();
    fireEvent.change(name, { target: { value: 'ship-it' } });
    await press(publish);
    expect(await screen.findByText('ship.md line 3 holds what looks like a GitLab token')).toBeInTheDocument();
  });

  it('LB-C10: Sync every machine now lists what each machine said', async () => {
    hosted({
      'POST /api/library/sync': ok({
        ok: true,
        machines: [
          { machine: 'a', label: 'studio', outcome: 'synced', report: { items: [{ outcome: 'placed' }, { outcome: 'in_step' }] } },
          { machine: 'b', label: 'old-box', outcome: 'predates' },
          { machine: 'c', label: 'quiet', outcome: 'opted_out' },
          { machine: 'd', label: 'laptop', outcome: 'offline' },
          { machine: 'e', label: 'win-box', outcome: 'failed', why: 'win-box did not answer in time' },
        ],
      }),
    });
    show();
    await waitFor(() => item('skill/vault'));
    await press(screen.getByRole('button', { name: 'Sync every machine now' }));
    const told = await screen.findByRole('status');
    expect([...told.querySelectorAll('li')].map((l) => l.textContent)).toEqual([
      'studio: synced, 1 changed',
      'old-box: its agent predates the Library — run the installer on it again',
      'quiet: takes no part in the Library',
      'laptop: offline — it syncs when it connects again',
      'win-box: failed — win-box did not answer in time',
    ]);
  });
});

describe('how the Library is used', () => {
  it('LB-C14: Items opens with what the Library’s skills add to every turn, each skill’s share a bar; before any turn listed one, it says it is not measured yet', async () => {
    hosted();
    const first = show();
    expect(await screen.findByText('The Library’s skills add about 400 tokens to every turn.')).toBeInTheDocument();
    const shares = screen.getByRole('list', { name: 'What each skill adds to every turn' });
    expect(within(shares).getAllByRole('listitem').map((li) => li.textContent)).toEqual([
      expect.stringContaining('vault'),
      expect.stringContaining('agent-router'),
    ]);
    expect(within(shares).getByText('about 300 tokens')).toBeInTheDocument();
    first.unmount();
    vi.unstubAllGlobals();

    hosted({ '/api/library/usage': ok({ ...USAGE, listing: null }) });
    show();
    expect(await screen.findByText(listingWords(null))).toBeInTheDocument();
    expect(screen.queryByRole('list', { name: 'What each skill adds to every turn' })).not.toBeInTheDocument();
  });

  it('LB-C15: each item says how often it was used in 30 days, and one not used is marked', async () => {
    hosted();
    show();
    await waitFor(() => item('skill/vault'));
    const usedOf = (key: string) => item(key).querySelector<HTMLElement>('[data-used]');
    expect(usedOf('skill/vault')?.textContent).toBe('used 12 times in 30 days');
    expect(usedOf('skill/agent-router')?.textContent).toBe('used once in 30 days');
    expect(usedOf('subagent/reviewer')?.textContent).toBe('not used in 30 days');
    expect(usedOf('subagent/reviewer')?.className).not.toBe('');
    expect(usedOf('skill/vault')?.className).toBe('');
  });

  it('LB-C16: an opened item draws its uses as a bar per day, in the viewer’s days, and splits them by machine and by project, naming what is not known; one not used says so', async () => {
    const stub = hosted();
    const first = show('/library?item=skill/bench');
    const panel = await opened();
    expect(await within(panel).findByText('used 5 times in 30 days, last 2 h ago')).toBeInTheDocument();
    const use = panel.querySelector<HTMLElement>('[data-use=""]')!;
    expect([...use.querySelectorAll<SVGElement>('rect[data-bucket]')].map((r) => r.dataset['bucket'])).toEqual(['27', '29']);
    const names = (label: string) => within(within(use).getByRole('list', { name: label })).getAllByRole('listitem').map((li) => li.textContent ?? '');
    expect(names('Uses by machine')).toEqual([expect.stringContaining('studio'), expect.stringContaining('a machine not known')]);
    expect(names('Uses by project')).toEqual([expect.stringContaining('app'), expect.stringContaining('(unresolved)')]);
    expect(stub.gets('/api/library/usage')[0]!.query.get('tz')).toBe(String(-new Date(NOW).getTimezoneOffset()));
    first.unmount();
    vi.unstubAllGlobals();

    hosted({ '/api/library/usage': ok({ ...USAGE, items: [used('bench', { last_used_at: NOW - 40 * DAY })] }) });
    show('/library?item=skill/bench');
    const quiet = await opened();
    expect(await within(quiet).findByText(/^Not used in the last 30 days; last used /)).toBeInTheDocument();
    expect(quiet.querySelector('rect[data-bucket]')).toBeNull();
  });

  it('LB-C17: usage that cannot be read is never read as no use, and a row from a router that counts none says nothing of use', async () => {
    hosted({ '/api/library/usage': unreachable });
    show('/library?item=skill/bench');
    const panel = await opened();
    expect(await screen.findByText(/^The Library’s usage could not be read: /)).toBeInTheDocument();
    expect(await within(panel).findByText(/^Its usage could not be read: /)).toBeInTheDocument();
    expect(screen.queryByText(/Not used in the last 30 days/)).not.toBeInTheDocument();
    expect(screen.queryByText(listingWords(null))).not.toBeInTheDocument();
    // The list's own counts still read, and the command's router counted none.
    await waitFor(() => expect(item('skill/vault').querySelector('[data-used]')?.textContent).toBe('used 12 times in 30 days'));
    expect(item('command/ship').querySelector('[data-used]')).toBeNull();
  });
});
