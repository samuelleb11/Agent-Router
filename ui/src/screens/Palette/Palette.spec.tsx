// Copied from Northstar ui/src/screens/Palette/Palette.spec.tsx at e9c1589.
// Changed: the fixtures are the router's (its live sessions, projects, accounts, switch options and search answers, served by test-utils' fetch stub) in place of Northstar's projects and items, and the cases are agent-router's: opening it, the search and its runs by kind, a hit opening its session at its entry, the jumps, each command and the request it makes, a refused command in the router's words, and the sheet in a hand. Each case names the contract ids it pins. The shortcut's cases and the shape of the rest are Northstar's.
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { act, fireEvent, renderHook, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { Shell } from '~/app/Shell';
import { Notices } from '~/app/notices';
import { dismiss, snapshot } from '~/app/notify';
import type { ProjectRow } from '~/data/router';
import { Unreachable } from '~/lib/errors';
import { json, refusal, renderScreen, settle, stubFetch, stubNarrow, unreachable, type Answers } from '~/test-utils';
import type { Account, LiveOptions, LiveSession, SearchAnswer, SearchHit } from './api';
import { MAX_JUMPS, SEARCH_LIMIT, markRuns, resolve, sessionHref, switchPost, validModel, type Context } from './hits';
import { Palette, usePaletteShortcut } from './index';

const NOW = Date.now();
const MIN = 60_000;
const HEALTH = { ok: true, pid: 4242, version: '0.9.0', build: 'f00dfeed' };

const S1 = 'a1b2c3d4-0000-4000-8000-000000000001';
const S2 = 'e5f6a7b8-0000-4000-8000-000000000002';
const S3 = 'c9d0e1f2-0000-4000-8000-000000000003';
/// A session the router's search still remembers and its list of sessions no longer holds.
const GONE = 'deadbeef-0000-4000-8000-000000000009';

const live = (session_id: string, over: Partial<LiveSession> = {}): LiveSession => ({
  session_id,
  cwd: '/Users/sam/code/agent-router',
  project: 'agent-router',
  git_branch: 'dev',
  title: null,
  last_activity_ms: NOW - 5 * MIN,
  archived: false,
  wrapped: true,
  ...over,
});

/// In the router's order, which is by start: the palette lists them newest activity first, and
/// leaves the archived one out until it is asked for.
const SESSIONS: LiveSession[] = [
  live(S2, { cwd: '/Users/sam/code/northstar', project: 'northstar', title: 'Fix the quota meter', last_activity_ms: NOW - 90 * MIN, wrapped: false }),
  live(S1, { title: 'Rebuild the palette', last_activity_ms: NOW - 2 * MIN }),
  live(S3, { title: 'An old experiment', last_activity_ms: NOW - 3 * 24 * 60 * MIN, archived: true }),
];

const row = (project: string | null, prompt_tokens: number): ProjectRow => ({
  project,
  sessions: 3,
  requests: 40,
  prompt_tokens,
  cost: 1.25,
  output_tokens: 900,
  cache_read_tokens: 12_000,
  subagent_tokens: 0,
  errors: 0,
  models: 'claude-opus-4-1',
  first_seen: NOW - 9 * 24 * 60 * MIN,
  last_seen: NOW - 60 * MIN,
});

const PROJECTS = [row('/Users/sam/code/agent-router', 900_000), row('/Users/sam/code/northstar', 400_000), row(null, 2_000)];

const ACCOUNTS: Account[] = [
  { id: 'default', label: 'Claude Code login', email: 'sam@example.com', active: 1 },
  { id: 'work', label: 'Work', email: 'work@example.com', active: 0 },
  { id: 'spare-token', label: 'Spare token', email: null, active: 0 },
];

const OPTIONS: LiveOptions = {
  models: ['opus', 'opus[1m]', 'sonnet', 'sonnet[1m]', 'haiku', 'haiku[1m]'],
  efforts: ['low', 'medium', 'high', 'xhigh', 'max'],
  modes: [
    { value: 'default', label: 'Ask' },
    { value: 'acceptEdits', label: 'Accept edits' },
    { value: 'plan', label: 'Plan' },
    { value: 'bypassPermissions', label: 'Bypass' },
  ],
};

const hit = (over: Partial<SearchHit>): SearchHit => ({
  block_id: 1,
  kind: 'text',
  chars: 120,
  session_id: S1,
  role: 'user',
  ordinal: 4,
  request_id: 10,
  at: NOW - 60 * MIN,
  project: '/Users/sam/code/agent-router',
  session_label: null,
  snippet: 'port the ‹palette› next',
  rank: -9,
  ...over,
});

/// The router's answer for "palette", best match first: a tool call leads, so its run comes
/// first; one message block replayed at two places is two hits; a thinking block comes from a
/// session whose transcript never arrived, so it has no project.
const PALETTE: SearchAnswer = {
  query: 'palette',
  expression: 'palette',
  sessions: [
    { session_id: S1, project: '/Users/sam/code/agent-router', hits: 4, blocks: 3 },
    { session_id: S2, project: null, hits: 1, blocks: 1 },
  ],
  hits: [
    hit({ block_id: 11, kind: 'tool_use', role: 'assistant', ordinal: 7, snippet: 'Read ui/src/screens/‹Palette›/hits.ts' }),
    hit({ block_id: 12, kind: 'text', ordinal: 4, snippet: 'port the ‹palette› next' }),
    hit({ block_id: 13, kind: 'tool_result', ordinal: 8, snippet: 'export function ‹Palette›({ open, onClose })' }),
    hit({ block_id: 12, kind: 'text', ordinal: 9, at: NOW - 30 * MIN, snippet: 'port the ‹palette› next' }),
    hit({ block_id: 14, kind: 'thinking', session_id: S2, project: null, role: 'assistant', ordinal: 2, snippet: 'the ‹palette› should run by kind' }),
  ],
};

const none = (q: string): SearchAnswer => ({ query: q, expression: q, sessions: [], hits: [] });

let answers: Record<string, () => Response | Promise<Response>>;
let fetchMock: ReturnType<typeof stubFetch>;

/// The request the stub is answering right now: it records a call before it answers it.
function current(): { url: URL; body: Record<string, unknown> } {
  const call = fetchMock.mock.lastCall as unknown as [RequestInfo | URL, RequestInit | undefined] | undefined;
  const body: unknown = call?.[1]?.body === undefined ? {} : JSON.parse(String(call[1].body));
  return { url: new URL(String(call?.[0]), 'http://console.test'), body: body as Record<string, unknown> };
}

function answer(more: Answers = {}) {
  answers = {
    '/_router/health': () => json(HEALTH),
    '/api/projects': () => json(PROJECTS),
    '/api/live/sessions': () => json(SESSIONS),
    '/api/live/options': () => json(OPTIONS),
    '/api/accounts': () => json(ACCOUNTS),
    '/api/search': () => {
      const q = current().url.searchParams.get('q') ?? '';
      return json(q === 'palette' ? PALETTE : none(q));
    },
    '/api/accounts/activate': () => json({ ok: true, active: current().body['id'] }),
    '/api/live/model': () => json({ ok: true, model: current().body['model'], notices: [] }),
    '/api/live/effort': () => json({ ok: true, effort: current().body['effort'] }),
    '/api/live/mode': () => json({ ok: true, mode: current().body['mode'] }),
    '/api/live/archive': () => json({ ok: true, archived: 1 }),
    ...more,
  };
  fetchMock = stubFetch(answers);
}

/// What the palette asked the router, in order.
function sent() {
  return (fetchMock.mock.calls as unknown as [RequestInfo | URL, RequestInit | undefined][]).map(([input, init]) => {
    const url = new URL(String(input), 'http://console.test');
    return {
      method: init?.method ?? 'GET',
      path: url.pathname,
      params: url.searchParams,
      body: init?.body === undefined ? undefined : (JSON.parse(String(init.body)) as unknown),
    };
  });
}
const posts = () => sent().filter((r) => r.method === 'POST').map(({ path, body }) => ({ path, body }));
const searches = () => sent().filter((r) => r.path === '/api/search');

async function mount(at = '/', onClose = vi.fn()) {
  // `path: '*'` keeps the palette mounted at every route, so navigating does not unmount what
  // the test is reading. The notices are drawn beside it, as the app draws them.
  const r = renderScreen(
    <>
      <Palette open onClose={onClose} />
      <Notices />
    </>,
    { path: '*', at },
  );
  // The body arrives in its own chunk the first time the palette opens.
  await screen.findByRole('searchbox', { name: 'Search and commands' });
  return { ...r, onClose, user: userEvent.setup() };
}

/// Where the router is now, path and query, from the location test-utils renders.
const where = () => screen.getByTestId('location').textContent;
const input = () => screen.getByRole('searchbox', { name: 'Search and commands' });
const results = () => screen.getByRole('listbox', { name: 'Results' });
const runLabels = () =>
  within(results())
    .getAllByRole('group')
    .map((g) => g.getAttribute('aria-label'));
const runOf = (el: Element) => el.closest('[role="group"]')?.getAttribute('aria-label');

beforeEach(() => {
  answer();
});
afterEach(() => {
  vi.unstubAllGlobals();
  vi.clearAllMocks();
  // The notices are a store outside React, and this runs before the tree is unmounted.
  act(() => {
    for (const n of snapshot()) dismiss(n.id);
  });
});

describe('Palette: opening it (SH-C50)', () => {
  it('SH-C50: the topbar’s search action and ⌘K open it on any screen, ⌘K from inside a field too', async () => {
    renderScreen(
      <>
        <Shell />
        <label>
          Elsewhere <input />
        </label>
      </>,
      { path: '*', at: '/settings/network' },
    );
    const user = userEvent.setup();
    await user.click(screen.getByRole('button', { name: 'Search and commands (⌘K)' }));
    const palette = await screen.findByRole('dialog', { name: 'Search and commands' });
    // Its body arrives in its own chunk the first time it opens, and takes the focus.
    const field = await within(palette).findByRole('searchbox', { name: 'Search and commands' });
    await waitFor(() => expect(field).toHaveFocus());
    await user.click(within(palette).getByRole('button', { name: 'Close' }));
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());

    // The palette is primary navigation, so the shortcut is global: it fires from inside a field.
    const elsewhere = screen.getByRole('textbox', { name: 'Elsewhere' });
    elsewhere.focus();
    fireEvent.keyDown(elsewhere, { key: 'k', metaKey: true });
    expect(await screen.findByRole('dialog', { name: 'Search and commands' })).toBeInTheDocument();
    fireEvent.keyDown(screen.getByRole('dialog'), { key: 'Escape' });
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());

    fireEvent.keyDown(document, { key: 'k', ctrlKey: true });
    expect(await screen.findByRole('dialog', { name: 'Search and commands' })).toBeInTheDocument();
  });
});

describe('usePaletteShortcut', () => {
  it('SH-C50: opens on ⌘K and on Ctrl-K, and claims the key', () => {
    const onOpen = vi.fn();
    renderHook(() => usePaletteShortcut(onOpen));
    const meta = new KeyboardEvent('keydown', { key: 'k', metaKey: true, cancelable: true, bubbles: true });
    document.dispatchEvent(meta);
    expect(onOpen).toHaveBeenCalledTimes(1);
    expect(meta.defaultPrevented).toBe(true);
    const ctrl = new KeyboardEvent('keydown', { key: 'K', ctrlKey: true, cancelable: true, bubbles: true });
    document.dispatchEvent(ctrl);
    expect(onOpen).toHaveBeenCalledTimes(2);
    fireEvent.keyDown(document, { key: 'k' });
    expect(onOpen).toHaveBeenCalledTimes(2);
  });

  it('unbinds on unmount', () => {
    const onOpen = vi.fn();
    const { unmount } = renderHook(() => usePaletteShortcut(onOpen));
    unmount();
    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'k', metaKey: true, bubbles: true }));
    expect(onOpen).not.toHaveBeenCalled();
  });
});

describe('Palette: searching the router’s index (SH-C51, SH-C52)', () => {
  it('SH-C51, OV-C50, SE-C169: typing searches /api/search with q and limit, and its hits run by kind, the run with the best match first', async () => {
    const { user } = await mount();
    await user.type(input(), 'palette');
    const call = await screen.findByRole('option', { name: /hits\.ts/ });

    const asked = searches().find((r) => r.params.get('q') === 'palette');
    expect(asked?.params.get('limit')).toBe(String(SEARCH_LIMIT));
    // All kinds is the default, and the router reads an absent kind as every kind.
    expect(asked?.params.has('kind')).toBe(false);

    // The session whose name matches is a jump; the index's hits follow, one run per kind, in the
    // order the router's best of each came.
    expect(runLabels()).toEqual(['Sessions', 'Tool calls', 'Messages', 'Tool results', 'Thinking']);
    // A hit names its project, its role, its session's short id and its age, over its snippet.
    expect(runOf(call)).toBe('Tool calls');
    expect(call).toHaveTextContent('agent-router');
    expect(call).toHaveTextContent('assistant');
    expect(call).toHaveTextContent(S1.slice(0, 8));
    expect(call).toHaveTextContent('1h ago');
    expect(within(call).getByText('Palette', { selector: 'mark' })).toBeInTheDocument();
  });

  it('OV-C52: a hit whose session’s project is unknown names it as the lists do', async () => {
    const { user } = await mount();
    await user.type(input(), 'palette');
    const thinking = await screen.findByRole('option', { name: /should run by kind/ });
    expect(runOf(thinking)).toBe('Thinking');
    expect(thinking).toHaveTextContent('(unresolved)');
  });

  it('SE-C171: a block replayed at two places is two hits, each its own row', async () => {
    const { user } = await mount();
    await user.type(input(), 'palette');
    await screen.findByRole('option', { name: /hits\.ts/ });
    const messages = within(results()).getByRole('group', { name: 'Messages' });
    expect(within(messages).getAllByRole('option')).toHaveLength(2);
  });

  it('OV-C38: the kinds stay in sight as a switcher, All by default; choosing one searches with its kind', async () => {
    const { user } = await mount();
    const kinds = screen.getByRole('tablist', { name: 'Search in' });
    expect(within(kinds).getAllByRole('tab').map((t) => t.textContent)).toEqual(['All', 'Messages', 'Tool calls', 'Results', 'Thinking']);
    expect(within(kinds).getByRole('tab', { name: 'All' })).toHaveAttribute('aria-selected', 'true');
    await user.type(input(), 'palette');
    await screen.findByRole('option', { name: /hits\.ts/ });
    expect(searches().every((r) => !r.params.has('kind'))).toBe(true);

    await user.click(within(kinds).getByRole('tab', { name: 'Tool calls' }));
    await waitFor(() => expect(searches().at(-1)?.params.get('kind')).toBe('tool_use'));
    expect(searches().at(-1)?.params.get('q')).toBe('palette');
    await waitFor(() => expect(screen.getByText(/Only tool calls\./)).toBeInTheDocument());
  });

  it('SH-C52: the search waits for a pause in the typing, and a late answer to an earlier query is never shown', async () => {
    const held = new Map<string, (r: Response) => void>();
    answers['/api/search'] = () => {
      const q = current().url.searchParams.get('q') ?? '';
      return new Promise<Response>((resolve) => held.set(q, resolve));
    };
    await mount();
    // Three keystrokes with no pause between them are one request, for what was typed last.
    fireEvent.change(input(), { target: { value: 'p' } });
    fireEvent.change(input(), { target: { value: 'pa' } });
    fireEvent.change(input(), { target: { value: 'pal' } });
    await waitFor(() => expect(searches().map((r) => r.params.get('q'))).toEqual(['pal']));
    expect(screen.getByText('Searching for “pal”…')).toBeInTheDocument();

    fireEvent.change(input(), { target: { value: 'palette' } });
    await waitFor(() => expect(held.has('palette')).toBe(true));

    // The later query answers first, and then the earlier one lands.
    await act(async () => {
      held.get('palette')?.(json(PALETTE));
      await settle();
    });
    await screen.findByRole('option', { name: /hits\.ts/ });
    await act(async () => {
      held.get('pal')?.(json({ ...none('pal'), hits: [hit({ block_id: 99, snippet: 'a ‹pal› from earlier' })] }));
      await settle();
    });
    expect(screen.queryByText(/from earlier/)).toBeNull();
    expect(screen.getByRole('option', { name: /hits\.ts/ })).toBeInTheDocument();
  });

  it('SE-C170: a snippet’s matches are marked by splitting on ‹ ›, so a stray marker opens no mark and markup stays text', async () => {
    answers['/api/search'] = () =>
      json({
        ...none('tag'),
        sessions: [{ session_id: S1, project: null, hits: 1, blocks: 1 }],
        hits: [hit({ snippet: 'an <b>‹tag›</b> and a stray › here' })],
      });
    const { user } = await mount();
    await user.type(input(), 'tag');
    const found = await screen.findByRole('option', { name: /stray/ });
    expect(found.querySelector('b')).toBeNull();
    expect(found).toHaveTextContent('an <b>tag</b> and a stray here');
    expect([...found.querySelectorAll('mark')].map((m) => m.textContent)).toEqual(['tag', ' here']);
  });

  it('SE-C166: a search that matched nothing says so, never an empty pane', async () => {
    const { user } = await mount();
    await user.type(input(), 'zzzz');
    expect(await screen.findByText('Nothing matched “zzzz”')).toBeInTheDocument();
    expect(screen.getByText('0 matches in 0 sessions.')).toBeInTheDocument();
  });

  it('SE-C167: a failed search says so in the router’s words, and one that got no answer says that; neither says nothing matched', async () => {
    answers['/api/search'] = () => json({ error: 'database disk image is malformed' }, 500);
    const first = await mount();
    await first.user.type(input(), 'palette');
    const failed = await screen.findByRole('alert');
    expect(failed).toHaveTextContent('The search failed');
    expect(failed).toHaveTextContent('database disk image is malformed');
    expect(screen.queryByText(/Nothing matched/)).toBeNull();
    first.unmount();

    answers['/api/search'] = unreachable;
    const second = await mount();
    await second.user.type(input(), 'palette');
    expect(await screen.findByRole('alert')).toHaveTextContent('Could not reach the router');
    expect(screen.queryByText(/Nothing matched/)).toBeNull();
  });

  it('SE-C165, SE-C168: the line under the list says what the router counted beyond what is shown, and what it ran when that was not what was typed', async () => {
    answers['/api/search'] = () =>
      json({ ...PALETTE, query: "don't", expression: '"don\'t"', sessions: [{ session_id: S1, project: null, hits: 214, blocks: 90 }] });
    const first = await mount();
    await first.user.type(input(), "don't");
    expect(await screen.findByText(/Showing the 5 best of 214 matches, in 1 session; type more to narrow\./)).toBeInTheDocument();
    expect(screen.getByText(/Searched for "don't"\./)).toBeInTheDocument();
    first.unmount();

    // At the router's cap of 200 sessions, its count is a floor.
    const many = Array.from({ length: 200 }, (_, i) => ({ session_id: `s${i}`, project: null, hits: 2, blocks: 2 }));
    answers['/api/search'] = () => json({ ...PALETTE, sessions: many });
    const second = await mount();
    await second.user.type(input(), 'palette');
    expect(await screen.findByText(/Showing the 5 best of at least 400 matches, in 200 sessions/)).toBeInTheDocument();
    expect(screen.queryByText(/Searched for/)).toBeNull();
  });
});

describe('Palette: a hit opens its session at the matching entry (SE-C189, SE-C171)', () => {
  it('SE-C189, SE-C171: Enter on a hit opens /live?session=<id> at its block and ordinal, and closes the palette', async () => {
    const { user, onClose } = await mount();
    await user.type(input(), 'palette');
    await screen.findByRole('option', { name: /hits\.ts/ });
    // The first row is the session the words name; the first of the index's hits is under it.
    await user.keyboard('{ArrowDown}');
    expect(screen.getByRole('option', { name: /hits\.ts/ })).toHaveAttribute('aria-selected', 'true');
    expect(input()).toHaveAttribute('aria-activedescendant', 'palette-hit-1');
    await user.keyboard('{Enter}');
    await waitFor(() => expect(where()).toBe(`/live?session=${S1}&block=11&ordinal=7`));
    expect(onClose).toHaveBeenCalled();
  });

  it('SE-C171: a replayed block opens at the place that was chosen', async () => {
    const { user } = await mount();
    await user.type(input(), 'palette');
    await screen.findByRole('option', { name: /hits\.ts/ });
    const [, second] = within(within(results()).getByRole('group', { name: 'Messages' })).getAllByRole('option');
    await user.click(second!);
    expect(where()).toBe(`/live?session=${S1}&block=12&ordinal=9`);
  });

  it('SE-C173: a hit in a session the router does not list says so by the hit, and opens nothing', async () => {
    answers['/api/search'] = () =>
      json({ ...none('palette'), sessions: [{ session_id: GONE, project: null, hits: 1, blocks: 1 }], hits: [hit({ session_id: GONE, snippet: 'an old ‹palette›' })] });
    const { user, onClose } = await mount();
    await user.type(input(), 'palette');
    const old = await screen.findByRole('option', { name: /an old palette/ });
    expect(old).toHaveAttribute('aria-disabled', 'true');
    expect(old).toHaveTextContent('The router does not list that session, so it cannot be opened.');
    expect(old).not.toHaveTextContent(/no longer/);
    await user.click(old);
    expect(where()).toBe('/');
    expect(onClose).not.toHaveBeenCalled();
  });

  it('SE-C173: while the list of sessions cannot be read, a hit says that, never that its session is not listed', async () => {
    answers['/api/live/sessions'] = () => json({ error: 'the registry could not be read' }, 500);
    const { user } = await mount();
    await user.type(input(), 'palette');
    const call = await screen.findByRole('option', { name: /hits\.ts/ });
    expect(call).toHaveAttribute('aria-disabled', 'true');
    expect(call).toHaveTextContent('The router’s list of sessions could not be read, so this cannot be opened from here.');
    expect(call).not.toHaveTextContent(/does not list/);
  });
});

describe('Palette: jumping to a session or a project', () => {
  it('OV-C40: lists the sessions, newest first and the archived left out, then the projects, before anything is typed, and asks the index nothing', async () => {
    await mount();
    const projects = await screen.findByRole('group', { name: 'Projects' });
    await within(projects).findByRole('option', { name: /^northstar/ });
    expect(runLabels()).toEqual(['Sessions', 'Projects']);
    const sessions = within(results()).getByRole('group', { name: 'Sessions' });
    expect(within(sessions).getAllByRole('option').map((o) => o.textContent)).toEqual([
      expect.stringContaining('Rebuild the palette'),
      expect.stringContaining('Fix the quota meter'),
    ]);
    expect(within(projects).getAllByRole('option').map((o) => o.textContent)).toEqual([
      expect.stringMatching(/^agent-router/),
      expect.stringMatching(/^northstar/),
      expect.stringMatching(/^\(unresolved\)/),
    ]);
    expect(screen.queryByRole('option', { name: /An old experiment/ })).toBeNull();
    expect(searches()).toHaveLength(0);
  });

  it('jumps to a project at /?project=<key>, the Overview scoped to it', async () => {
    const { user, onClose } = await mount();
    const projects = await screen.findByRole('group', { name: 'Projects' });
    await user.click(await within(projects).findByRole('option', { name: /^northstar/ }));
    expect(where()).toBe('/?project=%2FUsers%2Fsam%2Fcode%2Fnorthstar');
    expect(onClose).toHaveBeenCalled();
  });

  it('SE-C189: jumps to a session found by its name at /live?session=<id>, and an archived one is found and marked', async () => {
    const first = await mount();
    await first.user.type(input(), 'quota');
    const found = await screen.findByRole('option', { name: /Fix the quota meter/ });
    expect(runOf(found)).toBe('Sessions');
    expect(found).toHaveTextContent('northstar');
    await first.user.keyboard('{Enter}');
    expect(where()).toBe(`/live?session=${S2}`);
    first.unmount();

    const second = await mount();
    await second.user.type(input(), 'experiment');
    expect(await screen.findByRole('option', { name: /An old experiment/ })).toHaveTextContent('archived');
  });

  it('holds six of a kind of jump and says how many it held back', async () => {
    const many = Array.from({ length: 9 }, (_, i) => live(`${i}0000000-0000-4000-8000-00000000000${i}`, { title: `Bulk ${i}`, last_activity_ms: NOW - i * MIN }));
    answers['/api/live/sessions'] = () => json(many);
    await mount();
    const sessions = await screen.findByRole('group', { name: 'Sessions' });
    await waitFor(() => expect(within(sessions).getAllByRole('option')).toHaveLength(MAX_JUMPS));
    expect(sessions).toHaveTextContent(`Sessions · ${MAX_JUMPS} of 9`);
  });

  it('says the sessions could not be read in the router’s words, rather than listing none', async () => {
    answers['/api/live/sessions'] = () => json({ ok: false, reason: 'the registry is locked' }, 503);
    await mount();
    const alert = await screen.findByRole('alert');
    expect(alert).toHaveTextContent('The sessions could not be read');
    expect(alert).toHaveTextContent('the registry is locked');
    expect(screen.getByText('No session can be jumped to; the search and the commands still work.')).toBeInTheDocument();
  });
});

describe('Palette: commands behind > (SH-C51)', () => {
  it("SH-C51: '>' lists the commands: a session's model, effort and mode, archive and restore, each account, the theme and the pages", async () => {
    const { user } = await mount();
    await user.type(input(), '>');
    await screen.findByRole('option', { name: /Use work@example\.com/ });
    expect(runLabels()).toEqual(['Sessions', 'Accounts', 'Theme', 'Go to']);
    const names = within(results())
      .getAllByRole('option')
      .map((o) => o.textContent ?? '');
    for (const expected of [
      'Set a session’s model…',
      'Set a session’s effort…',
      'Set a session’s mode…',
      'Archive a session…',
      'Restore a session…',
      'Use sam@example.com',
      'Use work@example.com',
      'Use Spare token',
      'Theme: follow the system',
      'Theme: light',
      'Theme: dark',
      'Overview',
      'Settings: Accounts',
      'Settings: Router',
    ]) {
      expect(
        names.some((n) => n.includes(expected)),
        expected,
      ).toBe(true);
    }
    // A command is never sent to the index as words.
    expect(searches()).toHaveLength(0);
    expect(screen.getByText('No session is on screen, so a session command asks which one first.')).toBeInTheDocument();
  });

  it('ST-C24, OV-C17: Use posts the account’s id to /api/accounts/activate in one choice; the serving one is marked and does nothing', async () => {
    const { user, onClose } = await mount();
    await user.type(input(), '> use');
    const serving = await screen.findByRole('option', { name: /Use sam@example\.com/ });
    expect(serving).toHaveTextContent('default');
    expect(serving).toHaveTextContent('serving now');
    expect(serving).toHaveAttribute('aria-disabled', 'true');
    await user.click(serving);
    expect(posts()).toEqual([]);

    await user.click(screen.getByRole('option', { name: /Use work@example\.com/ }));
    await waitFor(() => expect(posts()).toEqual([{ path: '/api/accounts/activate', body: { id: 'work' } }]));
    await waitFor(() => expect(onClose).toHaveBeenCalled());
    expect(await screen.findByText('work@example.com signs requests now')).toBeInTheDocument();
    // The list is read again, so whatever shows the serving account shows the new one.
    await waitFor(() => expect(sent().filter((r) => r.path === '/api/accounts')).toHaveLength(2));
  });

  it('SE-C138, SE-C143: with a session on screen, Set the model lists the router’s models and posts {session, model} to /api/live/model', async () => {
    const { user, onClose } = await mount(`/live?session=${S1}`);
    await user.type(input(), '> model');
    const command = await screen.findByRole('option', { name: /Set the model…/ });
    expect(runOf(command)).toBe('This session');
    expect(command).toHaveTextContent('Rebuild the palette');
    await user.keyboard('{Enter}');

    expect(await screen.findByText('Set the model of Rebuild the palette')).toBeInTheDocument();
    expect(input()).toHaveValue('');
    await waitFor(() => expect(within(results()).getAllByRole('option').map((o) => o.textContent)).toEqual([...OPTIONS.models]));
    await user.type(input(), 'sonnet');
    await user.keyboard('{Enter}');
    await waitFor(() => expect(posts()).toEqual([{ path: '/api/live/model', body: { session: S1, model: 'sonnet' } }]));
    expect(await screen.findByText('model set to sonnet — takes effect on the next turn')).toBeInTheDocument();
    expect(onClose).toHaveBeenCalled();
  });

  it('SE-C138: effort and mode post {session, effort} and {session, mode}, the modes by the router’s labels', async () => {
    const cases = [
      ['effort', 'high', { effort: 'high' }],
      ['mode', 'Plan', { mode: 'plan' }],
    ] as const;
    for (const [setting, pick, body] of cases) {
      const { user, unmount } = await mount(`/live?session=${S1}`);
      await user.type(input(), `> ${setting}`);
      await user.click(await screen.findByRole('option', { name: new RegExp(`Set the ${setting}…`) }));
      await user.click(await screen.findByRole('option', { name: new RegExp(`^${pick}`) }));
      await waitFor(() => expect(posts(), setting).toEqual([{ path: `/api/live/${setting}`, body: { session: S1, ...body } }]));
      unmount();
      fetchMock.mockClear();
    }
  });

  it('SE-C141: a model the router’s list does not hold can be typed, and nothing typed offers nothing', async () => {
    const { user } = await mount(`/live?session=${S1}`);
    await user.type(input(), '> model');
    await user.click(await screen.findByRole('option', { name: /Set the model…/ }));
    await screen.findByRole('option', { name: 'opus' });
    expect(screen.queryByRole('option', { name: /^Use “/ })).toBeNull();
    await user.type(input(), 'claude-opus-4-1');
    const typed = await screen.findByRole('option', { name: /Use “claude-opus-4-1”/ });
    expect(within(results()).getAllByRole('option')).toEqual([typed]);
    await user.keyboard('{Enter}');
    await waitFor(() => expect(posts()).toEqual([{ path: '/api/live/model', body: { session: S1, model: 'claude-opus-4-1' } }]));
  });

  it('SE-C142: Bypass asks first, and keeping the mode sends nothing', async () => {
    const { user } = await mount(`/live?session=${S1}`);
    await user.type(input(), '> mode');
    await user.click(await screen.findByRole('option', { name: /Set the mode…/ }));
    await user.click(await screen.findByRole('option', { name: /^Bypass/ }));
    expect(await screen.findByText('This session will stop asking before it runs commands or edits files.')).toBeInTheDocument();
    expect(posts()).toEqual([]);

    await user.click(screen.getByRole('option', { name: 'Keep the current mode' }));
    expect(await screen.findByText('Set the mode of Rebuild the palette')).toBeInTheDocument();
    expect(posts()).toEqual([]);

    await user.click(await screen.findByRole('option', { name: /^Bypass/ }));
    await user.click(await screen.findByRole('option', { name: 'Bypass permissions on Rebuild the palette' }));
    await waitFor(() => expect(posts()).toEqual([{ path: '/api/live/mode', body: { session: S1, mode: 'bypassPermissions' } }]));
  });

  it('SE-C137: with no session on screen, a switch asks which session first, from /api/live/sessions, an unwrapped one greyed with the reason', async () => {
    const { user } = await mount('/');
    await user.type(input(), '> effort');
    await user.click(await screen.findByRole('option', { name: /Set a session’s effort…/ }));
    expect(await screen.findByText('Which session’s effort?')).toBeInTheDocument();
    const unwrapped = await screen.findByRole('option', { name: /Fix the quota meter/ });
    expect(unwrapped).toHaveAttribute('aria-disabled', 'true');
    expect(unwrapped).toHaveTextContent('Only a session under the wrapper can be switched');

    await user.click(screen.getByRole('option', { name: /Rebuild the palette/ }));
    expect(await screen.findByText('Set the effort of Rebuild the palette')).toBeInTheDocument();
    await user.click(await screen.findByRole('option', { name: 'high' }));
    await waitFor(() => expect(posts()).toEqual([{ path: '/api/live/effort', body: { session: S1, effort: 'high' } }]));
    expect(sent().some((r) => r.path === '/api/live/sessions')).toBe(true);
  });

  it('goes back a step with Back, and with Backspace in an empty field', async () => {
    const { user } = await mount('/');
    await user.type(input(), '> mode');
    await user.click(await screen.findByRole('option', { name: /Set a session’s mode…/ }));
    await user.click(await screen.findByRole('option', { name: /Rebuild the palette/ }));
    expect(await screen.findByText('Set the mode of Rebuild the palette')).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Back' }));
    expect(await screen.findByText('Which session’s mode?')).toBeInTheDocument();
    await user.click(input());
    await user.keyboard('{Backspace}');
    expect(screen.queryByText('Which session’s mode?')).toBeNull();
    expect(input()).toHaveValue('');
  });

  it('archive and restore post {session, archived} to /api/live/archive: the session on screen, or one asked for from the shelf', async () => {
    const first = await mount(`/live?session=${S1}`);
    await first.user.type(input(), '> archive');
    await first.user.click(await screen.findByRole('option', { name: /Archive this session/ }));
    await waitFor(() => expect(posts()).toEqual([{ path: '/api/live/archive', body: { session: S1, archived: true } }]));
    expect(await screen.findByText('Rebuild the palette archived')).toBeInTheDocument();
    first.unmount();
    fetchMock.mockClear();

    const second = await mount('/');
    await second.user.type(input(), '> restore');
    await second.user.click(await screen.findByRole('option', { name: /Restore a session…/ }));
    expect(await screen.findByText('Which session to restore?')).toBeInTheDocument();
    // Only what is archived can be restored.
    const shelf = await within(results()).findAllByRole('option');
    expect(shelf.map((o) => o.textContent)).toEqual([expect.stringContaining('An old experiment')]);
    await second.user.click(shelf[0]!);
    await waitFor(() => expect(posts()).toEqual([{ path: '/api/live/archive', body: { session: S3, archived: false } }]));
  });

  it('changes the theme through the console’s one theme, on the page and in this browser', async () => {
    const first = await mount();
    await first.user.type(input(), '> dark');
    await first.user.keyboard('{Enter}');
    expect(document.documentElement).toHaveClass('ar-dark');
    expect(localStorage.getItem('agent-router.theme')).toBe('dark');
    expect(first.onClose).toHaveBeenCalled();
    first.unmount();

    const second = await mount();
    await second.user.type(input(), '> theme');
    expect(await screen.findByRole('option', { name: /Theme: dark/ })).toHaveTextContent('in use');
    await second.user.click(screen.getByRole('option', { name: /Theme: follow the system/ }));
    expect(localStorage.getItem('agent-router.theme')).toBeNull();
  });

  it('goes where the rail and the settings tabs go', async () => {
    const { user } = await mount();
    await user.type(input(), '> settings acc');
    await screen.findByRole('option', { name: 'Settings: Accounts' });
    await user.keyboard('{Enter}');
    expect(where()).toBe('/settings/accounts');
  });

  it('says when no command matches', async () => {
    const { user } = await mount();
    await user.type(input(), '> nothing like this');
    expect(await screen.findByText('No command matches')).toBeInTheDocument();
  });
});

describe('Palette: a refused command shows the router’s words (SH-C51)', () => {
  it('SE-C144: a refused switch shows the router’s reason beside the list, and the palette stays open on its step', async () => {
    answers['/api/live/mode'] = () => json({ ok: false, reason: 'Cannot set permission mode to plan: the session is mid-turn' }, 422);
    const { user, onClose } = await mount(`/live?session=${S1}`);
    await user.type(input(), '> mode');
    await user.click(await screen.findByRole('option', { name: /Set the mode…/ }));
    await user.click(await screen.findByRole('option', { name: /^Plan/ }));
    const alert = await screen.findByRole('alert');
    expect(alert).toHaveTextContent('Could not set the mode');
    expect(alert).toHaveTextContent('Cannot set permission mode to plan: the session is mid-turn');
    expect(onClose).not.toHaveBeenCalled();
    expect(screen.getByText('Set the mode of Rebuild the palette')).toBeInTheDocument();
    expect(screen.queryByText(/takes effect/)).toBeNull();
  });

  it('ST-C25: a refused Use shows the router’s sentence, whether it came under a 200 or a 500', async () => {
    const words = 'this account is a Claude Code login on a developer’s machine, which a hosted router cannot read';
    for (const status of [200, 500]) {
      answers['/api/accounts/activate'] = () => refusal(words, status);
      const { user, onClose, unmount } = await mount();
      await user.type(input(), '> use work');
      await user.click(await screen.findByRole('option', { name: /Use work@example\.com/ }));
      const alert = await screen.findByRole('alert');
      expect(alert, String(status)).toHaveTextContent('Could not switch to work@example.com');
      expect(alert, String(status)).toHaveTextContent(words);
      expect(onClose).not.toHaveBeenCalled();
      // And the list is read again: what was refused may have changed since it was drawn.
      await waitFor(() => expect(sent().filter((r) => r.path === '/api/accounts')).toHaveLength(2));
      unmount();
      fetchMock.mockClear();
    }
  });

  it('a hosted router’s 503 sentence is the words, and a command that got no answer says it could not reach the router', async () => {
    answers['/api/live/archive'] = () => refusal('That session is not on a connected machine.', 503);
    const first = await mount(`/live?session=${S1}`);
    await first.user.type(input(), '> archive');
    await first.user.click(await screen.findByRole('option', { name: /Archive this session/ }));
    const refused = await screen.findByRole('alert');
    expect(refused).toHaveTextContent('Could not archive that session');
    expect(refused).toHaveTextContent('That session is not on a connected machine.');
    first.unmount();

    answers['/api/live/archive'] = unreachable;
    const second = await mount(`/live?session=${S1}`);
    await second.user.type(input(), '> archive');
    await second.user.click(await screen.findByRole('option', { name: /Archive this session/ }));
    const gone = await screen.findByRole('alert');
    expect(gone).toHaveTextContent('Could not reach the router');
    expect(gone).not.toHaveTextContent(/refused/i);
  });
});

/// The body of the first block opened by `head` in `source`, braces balanced.
function block(source: string, head: string): string {
  const start = source.indexOf(head);
  if (start < 0) return '';
  const open = source.indexOf('{', start);
  let depth = 0;
  for (let i = open; i < source.length; i++) {
    if (source[i] === '{') depth += 1;
    if (source[i] === '}') depth -= 1;
    if (depth === 0) return source.slice(open + 1, i);
  }
  return '';
}

describe('Palette: the modal, and the sheet in a hand (SH-C51)', () => {
  it('reads nothing while closed, and focuses its field when it opens', async () => {
    const closed = renderScreen(<Palette open={false} onClose={() => {}} />, { path: '*', at: '/' });
    expect(screen.queryByRole('searchbox')).toBeNull();
    expect(fetchMock).not.toHaveBeenCalled();
    closed.unmount();

    await mount();
    await waitFor(() => expect(input()).toHaveFocus());
  });

  it('closes on Escape from inside the field, and on its Close button', async () => {
    const { user, onClose } = await mount();
    await user.type(input(), 'anything');
    await user.keyboard('{Escape}');
    expect(onClose).toHaveBeenCalledTimes(1);
    await user.click(screen.getByRole('button', { name: 'Close' }));
    expect(onClose).toHaveBeenCalledTimes(2);
  });

  it('SH-C54: starts clean each time it opens, with no typing and no step carried over', async () => {
    const onClose = vi.fn();
    const ui = (open: boolean) => <Palette open={open} onClose={onClose} />;
    const { rerender } = renderScreen(ui(true), { path: '*', at: `/live?session=${S1}` });
    const user = userEvent.setup();
    await user.type(input(), '> model');
    await user.click(await screen.findByRole('option', { name: /Set the model…/ }));
    expect(await screen.findByText('Set the model of Rebuild the palette')).toBeInTheDocument();
    rerender(ui(false));
    rerender(ui(true));
    expect(input()).toHaveValue('');
    expect(screen.queryByText('Set the model of Rebuild the palette')).toBeNull();
  });

  it('SH-C51: in a hand it is a full-height sheet under the topbar, and the topbar stays in reach while it is open', async () => {
    stubNarrow();
    renderScreen(<Shell />, { path: '*', at: '/' });
    const user = userEvent.setup();
    const topbar = screen.getByRole('banner', { name: 'Agent Router' });
    await user.click(within(topbar).getByRole('button', { name: 'Search and commands (⌘K)' }));
    const palette = await screen.findByRole('dialog', { name: 'Search and commands' });
    expect(palette.contains(topbar)).toBe(false);

    // The topbar's theme switch works with the palette open, and the palette stays open.
    await user.click(within(topbar).getByRole('button', { name: /^Theme: system/ }));
    expect(within(topbar).getByRole('button', { name: /^Theme: light/ })).toBeInTheDocument();
    expect(screen.getByRole('dialog', { name: 'Search and commands' })).toBeInTheDocument();
    // Back to the system's, so no other case starts on a chosen theme.
    await user.click(within(topbar).getByRole('button', { name: /^Theme: light/ }));
    await user.click(within(topbar).getByRole('button', { name: /^Theme: dark/ }));

    // The menu opens the drawer and puts the palette away, and the search action the reverse:
    // the two share the space under the topbar, so neither is left hidden under the other.
    const drawer = () => document.querySelector('.cds--side-nav');
    await user.click(within(topbar).getByRole('button', { name: 'Open the menu' }));
    expect(drawer()).toHaveClass('cds--side-nav--expanded');
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    await user.click(within(topbar).getByRole('button', { name: 'Search and commands (⌘K)' }));
    expect(await screen.findByRole('dialog', { name: 'Search and commands' })).toBeInTheDocument();
    expect(drawer()).not.toHaveClass('cds--side-nav--expanded');

    // jsdom lays nothing out, so the sheet is read from its stylesheet: below md the overlay and
    // the sheet both start where the shell's topbar ends — Carbon's 3rem, under a notch's safe
    // area — and the sheet runs to the foot of the screen whatever it holds. Starting the overlay
    // there too is what lets a tap reach the topbar.
    const scss = readFileSync(join(__dirname, 'Palette.module.scss'), 'utf8').replace(/\/\/.*$/gm, '');
    expect(scss).toMatch(/\$ar-header-height:\s*var\(--ar-shell-header\);/);
    const overlay = block(block(scss, '\n.modal:global(.cds--modal) {'), "@include breakpoint-down('md')");
    expect(overlay).toMatch(/inset-block:\s*\$ar-header-height 0;/);
    expect(overlay).toMatch(/block-size:\s*auto;/);
    const sheet = block(block(scss, '\n.container:global(.cds--modal-container) {'), "@include breakpoint-down('md')");
    expect(sheet).toMatch(/inset-block:\s*\$ar-header-height 0;/);
    expect(sheet).toMatch(/max-block-size:\s*none;/);
  });
});

describe('resolve (the rules, without a modal)', () => {
  const ctx = (over: Partial<Context> = {}): Context => ({
    step: null,
    projects: { state: 'read', value: PROJECTS },
    sessions: { state: 'read', value: SESSIONS },
    accounts: { state: 'read', value: ACCOUNTS },
    options: { state: 'read', value: OPTIONS },
    onScreen: null,
    theme: 'system',
    search: { status: 'read', answer: PALETTE },
    ...over,
  });

  it('OV-C50: runs the hits by kind in the order the best of each came, a system prompt in its word and a new kind as sent', () => {
    expect(resolve('palette', ctx()).runs.map((r) => r.label)).toEqual(['Sessions', 'Tool calls', 'Messages', 'Tool results', 'Thinking']);
    const answer: SearchAnswer = {
      ...PALETTE,
      hits: [hit({ kind: 'system', block_id: 1 }), hit({ kind: 'image', block_id: 2 }), hit({ kind: 'constructor', block_id: 3 })],
    };
    expect(resolve('palette', ctx({ search: { status: 'read', answer } })).runs.map((r) => r.label)).toEqual([
      'Sessions',
      'System prompts',
      'image',
      'constructor',
    ]);
  });

  it('SE-C171: a hit is its session, block and ordinal together, and opens at that place', () => {
    const messages = resolve('palette', ctx()).runs.find((r) => r.label === 'Messages');
    expect(messages?.hits.map((h) => h.id)).toEqual([`hit:${S1}:12:4`, `hit:${S1}:12:9`]);
    expect(messages?.hits.map((h) => h.action)).toEqual([
      { type: 'go', href: `/live?session=${S1}&block=12&ordinal=4` },
      { type: 'go', href: `/live?session=${S1}&block=12&ordinal=9` },
    ]);
    expect(sessionHref(S1)).toBe(`/live?session=${S1}`);
  });

  it('SE-C170: splits a snippet on its markers and never substitutes into it', () => {
    expect(markRuns('a ‹b› c')).toEqual([
      { text: 'a ', marked: false },
      { text: 'b', marked: true },
      { text: ' c', marked: false },
    ]);
    // A stray marker shifts which run is marked; it never drops text or leaves a run open.
    expect(
      markRuns('x › y ‹z')
        .map((r) => r.text)
        .join(''),
    ).toBe('x  y z');
  });

  it('SE-C143: a switch’s outcome is the CLI’s notices, else that it is pending, else what was set', () => {
    const session = { id: S1, name: 'Rebuild the palette' };
    const model = switchPost('model', session, 'sonnet');
    expect(model.body).toEqual({ session: S1, model: 'sonnet' });
    expect(model.done({ ok: true, model: 'sonnet', notices: ['Set model to Sonnet 5'] }).title).toBe('Set model to Sonnet 5');
    expect(model.done({ ok: true, model: 'sonnet', pending: true }).title).toBe('model sent — the session has not confirmed yet');
    expect(model.done({ ok: true, model: 'sonnet', notices: [] }).title).toBe('model set to sonnet — takes effect on the next turn');
    // The mode is the one the CLI says it took.
    expect(switchPost('mode', session, 'auto').done({ ok: true, mode: 'default' }).title).toBe('mode set to default — takes effect on the next turn');
  });

  it('SE-C141: offers a typed model only when the router’s own rule could accept it', () => {
    expect(validModel('claude-opus-4-1')).toBe(true);
    expect(validModel('sonnet[1m]')).toBe(true);
    expect(validModel('two words')).toBe(false);
    expect(validModel('')).toBe(false);
    expect(validModel('x'.repeat(65))).toBe(false);
  });

  it('SE-C137: greys the switches of a session the router lists as unwrapped, with the reason, and still offers to archive it', () => {
    const run = resolve('>', ctx({ onScreen: S2 })).runs.find((r) => r.key === 'commands:session');
    expect(run?.label).toBe('This session');
    expect(run?.hits.filter((h) => h.action === null).map((h) => h.label)).toEqual(['Set the model…', 'Set the effort…', 'Set the mode…']);
    expect(run?.hits.find((h) => h.label === 'Archive this session')?.action).toMatchObject({
      type: 'post',
      post: { path: '/api/live/archive', body: { session: S2, archived: true } },
    });
  });

  it('says a source is still being read or could not be read, and never lists it as empty', () => {
    const r = resolve('', ctx({ sessions: { state: 'unreachable', value: [], error: new Unreachable() }, projects: { state: 'loading', value: [] } }));
    expect(r.runs).toEqual([]);
    expect(r.notices.map((n) => [n.tone, n.title])).toEqual([
      ['hint', 'Type to search'],
      ['unavailable', 'The sessions could not be read'],
      ['reading', 'projects'],
    ]);
    const commands = resolve('>', ctx({ accounts: { state: 'unreachable', value: [], error: new Unreachable() } }));
    expect(commands.runs.map((r) => r.label)).toEqual(['Sessions', 'Theme', 'Go to']);
    expect(commands.notices[0]).toMatchObject({ tone: 'unavailable', title: 'The accounts could not be read' });
  });
});
