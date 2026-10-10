// New for agent-router: the Sessions screen as a reader meets it — the list, its headers, the tabs,
// the address, Find, the notices — against the router's answers and a stream the spec speaks as the
// router through. The session model's own rules are pinned in ~/model; these pin the screen that
// draws them.
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { useNavigate } from 'react-router';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { CONCEPTS } from '~/lib/concepts';
import type { SessionRow } from '~/model/session';
import { press } from '~/test-utils';
import { FakeEventSource } from './fakes';
import { A, AT, B, C, SESSIONS, json, renderSessions, session, streamOf, stubRouter, working, type Asked } from './kit';

beforeEach(() => FakeEventSource.reset());
afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

const where = () => screen.getByTestId('location').textContent;
const rowOf = (id: string) => {
  const li = document.querySelector<HTMLElement>(`[data-session="${id}"]`);
  if (!li) throw new Error(`no list row for ${id}`);
  return li;
};
const rowButton = (id: string) => rowOf(id).querySelector('button') as HTMLButtonElement;
const listPane = () => within(document.querySelector<HTMLElement>('[data-pane="list"]') as HTMLElement);
const group = (name: string) => {
  const label = [...document.querySelectorAll<HTMLElement>('.cds--contained-list__label')].find((l) => l.textContent?.includes(name));
  if (!label) throw new Error(`no group ${name}`);
  return label;
};
const dotOf = (el: Element | null) => el?.querySelector('[data-tone], [role="img"], [aria-hidden="true"][class]') as HTMLElement | null;

/// The browser's Back, pressed from inside the router.
function Back() {
  const navigate = useNavigate();
  return (
    <button type="button" data-testid="back" hidden onClick={() => void navigate(-1)}>
      back
    </button>
  );
}

describe('the list', () => {
  it('SE-C25: a project’s heading shows its dot, its name, how many sessions it holds and how long since its newest moved', async () => {
    stubRouter();
    renderSessions();
    await waitFor(() => rowOf(A));
    const head = group('agent-router');
    expect(head).toHaveTextContent('agent-router');
    expect(head).toHaveTextContent('2 sessions');
    // On the router's clock: its newest moved twelve seconds before the router's now.
    expect(head).toHaveTextContent('12s');
    expect(within(head).getByRole('img', { name: 'waiting for you' })).toHaveAttribute('data-tone', 'attention');
  });

  it('SE-C26, SE-C28: a row shows its dot with the halo when live, its name, live or view, its age, its title, what it is doing, and its id with its branch', async () => {
    stubRouter(undefined, (req) =>
      req.method === 'GET' && req.path === '/api/live/sessions'
        ? json([session(A, { title: 'Port the live view', git_branch: 'agen-12' }), session(C, { title: null, wrapped: false, activity: 'idle', tone: 'idle', detail: 'Waiting for you', active: false, git_branch: null })])
        : undefined,
    );
    renderSessions();
    await waitFor(() => rowOf(A));
    const a = rowButton(A);
    // Titled, so it is named by its branch; the project is its heading.
    expect(a).toHaveTextContent('agen-12');
    expect(a).toHaveTextContent('live');
    expect(a).toHaveTextContent('12s');
    expect(a).toHaveTextContent('Port the live view');
    expect(a).toHaveTextContent('Thinking · 12s');
    expect(a).toHaveTextContent('aaaaaaaa · agen-12');
    expect(within(a).getByRole('img', { name: 'working' })).toHaveAttribute('data-live');
    // Untitled, it is named by its project; and it cannot be typed into.
    const c = rowButton(C);
    expect(c).toHaveTextContent('agent-router');
    expect(c).toHaveTextContent('view');
    expect(within(c).getByRole('img', { name: 'idle' })).not.toHaveAttribute('data-live');
  });

  it('SE-C8: a session is live — the halo, and the count — while busy or waiting, and an older router’s row by its `active`', async () => {
    const rows = [
      session('gen-1', { activity: 'generating', tone: 'busy' }),
      session('bg-1', { activity: 'background', tone: 'busy', detail: '2 background agents running' }),
      session('stall-1', { activity: 'stalled', tone: 'warn', detail: 'No progress' }),
      session('wait-1', { activity: 'waiting', tone: 'attention', detail: 'Waiting for you' }),
      session('idle-1', { activity: 'idle', tone: 'idle', active: false }),
      session('fail-1', { activity: 'failed', tone: 'error', active: false, detail: 'API error' }),
      session('gone-1', { activity: 'closed', tone: 'closed', active: false, open: false, detail: 'Session ended' }),
      session('new-1', { activity: 'compacting', tone: 'sparkle', active: true, detail: 'Compacting' }),
      { session_id: 'old-1', cwd: '/x/legacy', project: 'legacy', last_activity_ms: AT - 1000, active: true, state: 'working', wrapped: false },
    ];
    stubRouter(undefined, (req) => (req.method === 'GET' && req.path === '/api/live/sessions' ? json(rows) : undefined));
    renderSessions();
    await waitFor(() => rowOf('gen-1'));
    const halo = (id: string) => rowButton(id).querySelector('[role="img"]')?.hasAttribute('data-live');
    expect(['gen-1', 'bg-1', 'stall-1', 'wait-1', 'new-1', 'old-1'].map(halo)).toEqual([true, true, true, true, true, true]);
    expect(['idle-1', 'fail-1', 'gone-1'].map(halo)).toEqual([false, false, false]);
    // Working is generating, tool, background or stalled; waiting is waiting; never a closed one.
    expect(listPane().getByRole('status')).toHaveTextContent('3 working · 1 waiting');
  });

  it('SE-C36: with nothing at work or waiting the header says how long since anything moved, or "nothing yet"', async () => {
    const idle = [session(A, { activity: 'idle', tone: 'idle', active: false, last_activity_ms: AT - 5 * 60e3 })];
    stubRouter(undefined, (req) => (req.method === 'GET' && req.path === '/api/live/sessions' ? json(idle) : undefined));
    const first = renderSessions();
    await waitFor(() => expect(listPane().getByRole('status')).toHaveTextContent('idle · 5m'));
    first.unmount();
    stubRouter(undefined, (req) => (req.method === 'GET' && req.path === '/api/live/sessions' ? json([]) : undefined));
    renderSessions();
    expect(await screen.findByText('No sessions in the last week.')).toBeInTheDocument();
    expect(listPane().getByRole('status')).toHaveTextContent('nothing yet');
  });

  it('SE-C212: rows from a current router and an older one draw side by side, every age on the router’s clock, set off this machine’s', async () => {
    // This machine is seven minutes ahead of the router: an age read off it would say "7m".
    vi.useFakeTimers({ shouldAdvanceTime: true, toFake: ['Date'] });
    vi.setSystemTime(AT + 7 * 60e3);
    const rows = [session(A), { session_id: 'old-1', cwd: '/x/legacy', project: 'legacy', title: 'From an older router', last_activity_ms: AT - 3000, active: true, state: 'working', wrapped: false }];
    stubRouter(undefined, (req) => (req.method === 'GET' && req.path === '/api/live/sessions' ? json(rows) : undefined));
    renderSessions();
    await waitFor(() => rowOf('old-1'));
    expect(rowButton(A)).toHaveTextContent('12s');
    expect(rowButton(A)).not.toHaveTextContent('7m');
    // The older router's row: its legacy dot, no row line, and liveness from `active`.
    const old = rowButton('old-1');
    expect(within(old).getByRole('img', { name: 'working' })).toHaveAttribute('data-tone', 'busy');
    expect(old.textContent).not.toMatch(/Thinking|undefined|NaN/);
  });

  it('SE-C41: a failed list read with no session open says "offline" and why, and keeps its rows, marked with their age', async () => {
    let fail = false;
    stubRouter((req) => (fail && req.method === 'GET' && req.path === '/api/live/sessions' ? json({ ok: false, reason: 'database is locked' }, 500) : undefined));
    const { client } = renderSessions();
    await waitFor(() => rowOf(A));
    fail = true;
    await act(async () => {
      await client.refetchQueries({ queryKey: ['live', 'sessions'] });
    });
    await waitFor(() => expect(listPane().getByRole('status')).toHaveTextContent('offline · database is locked'));
    expect(rowOf(A)).toBeInTheDocument();
    expect(screen.getByText(/Showing what was read .* the last read failed — database is locked/)).toBeInTheDocument();
    expect(screen.queryByText('No sessions in the last week.')).toBeNull();
  });

  it('SE-C40: the list is read again every five seconds, and a refresh keeps the reader’s place in it', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const router = stubRouter();
    renderSessions();
    await waitFor(() => rowOf(A));
    const scroller = document.querySelector('[data-pane="list"] > div:last-child') as HTMLElement;
    scroller.scrollTop = 120;
    const before = router.gets('/api/live/sessions').length;
    await act(async () => {
      await vi.advanceTimersByTimeAsync(5000);
    });
    await waitFor(() => expect(router.gets('/api/live/sessions').length).toBe(before + 1));
    expect(scroller.scrollTop).toBe(120);
  });

  it('SE-C31: the older fold keeps its open state across every refresh, every archive and every return to the list', async () => {
    let rows: SessionRow[] = [...SESSIONS(), session('old-a', { title: 'Long ago', activity: 'idle', tone: 'idle', active: false, last_activity_ms: AT - 13 * 3600e3 })];
    stubRouter((req) => {
      if (req.method === 'GET' && req.path === '/api/live/sessions') return json(rows);
      if (req.method === 'POST' && req.path === '/api/live/archive') {
        rows = rows.map((r) => (r.session_id === req.body?.['session'] ? { ...r, archived: true } : r));
        return json({ ok: true, archived: 1 });
      }
      return undefined;
    });
    const { client } = renderSessions('/live', <Back />);
    const fold = () => screen.getByRole('button', { name: '1 older session' });
    await waitFor(() => fold());
    expect(fold()).toHaveAttribute('aria-expanded', 'false');
    await press(fold());
    expect(fold()).toHaveAttribute('aria-expanded', 'true');
    await act(async () => {
      await client.refetchQueries({ queryKey: ['live', 'sessions'] });
    });
    expect(fold()).toHaveAttribute('aria-expanded', 'true');
    await press(within(rowOf(C)).getByRole('button', { name: /^Archive/ }));
    await waitFor(() => expect(within(rowOf(C)).getByRole('button', { name: /^Restore/ })).toBeInTheDocument());
    expect(fold()).toHaveAttribute('aria-expanded', 'true');
    await press(rowButton(A));
    await waitFor(() => expect(where()).toBe(`/live?session=${A}`));
    await press(screen.getByTestId('back'));
    await waitFor(() => expect(where()).toBe('/live'));
    expect(fold()).toHaveAttribute('aria-expanded', 'true');
  });

  it('SE-C34: every row carries a visible Archive and every shelved row a Restore, both the Archive icon; the row opens its session and its action never does', async () => {
    let rows: SessionRow[] = [...SESSIONS(), session('shelf-1', { title: 'Put away', archived: true, activity: 'idle', tone: 'idle', active: false })];
    const router = stubRouter((req) => {
      if (req.method === 'GET' && req.path === '/api/live/sessions') return json(rows);
      if (req.method === 'POST' && req.path === '/api/live/archive') {
        rows = rows.map((r) => (r.session_id === req.body?.['session'] ? { ...r, archived: !!req.body?.['archived'] } : r));
        return json({ ok: true, archived: 1 });
      }
      return undefined;
    });
    renderSessions();
    await waitFor(() => rowOf('shelf-1'));
    const icon = render(<CONCEPTS.putAway.icon />).container.querySelector('path')?.getAttribute('d');
    const archive = within(rowOf(C)).getByRole('button', { name: /^Archive/ });
    const restore = within(rowOf('shelf-1')).getByRole('button', { name: /^Restore/ });
    expect(archive).toHaveTextContent('Archive');
    expect(restore).toHaveTextContent('Restore');
    expect([archive, restore].map((b) => b.querySelector('path')?.getAttribute('d'))).toEqual([icon, icon]);
    await press(archive);
    expect(router.posts('/api/live/archive').map((p) => p.body)).toEqual([{ session: C, archived: true }]);
    expect(where()).toBe('/live');
    await press(restore);
    expect(router.posts('/api/live/archive').map((p) => p.body)[1]).toEqual({ session: 'shelf-1', archived: false });
    expect(where()).toBe('/live');
    // Restored for good: it is back among the rows, off the shelf.
    await waitFor(() => expect(within(rowOf('shelf-1')).getByRole('button', { name: /^Archive/ })).toBeInTheDocument());
    await press(rowButton(B));
    expect(where()).toBe(`/live?session=${B}`);
  });

  it('SE-C179, SE-C178: a failed Restore says so by its own row, in the list that is on screen', async () => {
    const rows = [...SESSIONS(), session('shelf-1', { title: 'Put away', archived: true })];
    stubRouter((req) => {
      if (req.method === 'GET' && req.path === '/api/live/sessions') return json(rows);
      if (req.method === 'POST' && req.path === '/api/live/archive') return json({ ok: false, error: 'That session is not on a connected machine.' }, 503);
      return undefined;
    });
    renderSessions();
    await waitFor(() => rowOf('shelf-1'));
    await press(within(rowOf('shelf-1')).getByRole('button', { name: /^Restore/ }));
    const notice = await waitFor(() => {
      const n = rowOf('shelf-1').nextElementSibling;
      expect(n).toHaveTextContent('That session is not on a connected machine.');
      return n as HTMLElement;
    });
    // Dismissible, where it is.
    await press(within(notice).getByRole('button', { name: /close/i }));
    expect(screen.queryByText('That session is not on a connected machine.')).toBeNull();
  });

  it('SE-C1: the router decides what a session is doing — the screen shows its activity, tone and words, and derives nothing from the feed or its timing', async () => {
    stubRouter(undefined, (req) =>
      req.method === 'GET' && req.path === '/api/live/sessions'
        ? json([session(A, { activity: 'idle', tone: 'idle', state: 'idle', active: false, detail: 'Waiting for you', since_ms: AT - 60e3 })])
        : undefined,
    );
    renderSessions(`/live?session=${A}`);
    await waitFor(() => streamOf(A));
    act(() => {
      // The transcript shows a tool running; the router says the session is idle.
      streamOf(A)
        .open()
        .row({ role: 'assistant', kind: 'tool_use', name: 'Bash', text: 'make' })
        .status({ state: 'idle', detail: 'Waiting for you', activity: 'idle', tone: 'idle', as_of_ms: AT, since_ms: AT - 60e3, quiet_ms: 60e3, queued: 0 });
    });
    const head = within(document.querySelector('[data-session-head]') as HTMLElement);
    await waitFor(() => expect(head.getByRole('status')).toHaveTextContent('Waiting for you · 1m'));
    expect(head.getByRole('status').querySelector('[data-tone]')).toHaveAttribute('data-tone', 'idle');
    expect(rowButton(A)).toHaveTextContent('Waiting for you · 1m');
    expect(screen.queryByRole('button', { name: 'Stop the turn' })).toBeNull();
  });
});

describe('the open session', () => {
  it('SE-C5: one tone vocabulary serves the row, the project’s heading, the tab and the header', async () => {
    stubRouter();
    renderSessions(`/live?session=${B}`);
    await waitFor(() => streamOf(B));
    act(() => {
      streamOf(B).open().status({ state: 'ask', detail: 'Waiting for you', activity: 'waiting', tone: 'attention', as_of_ms: AT, since_ms: AT - 12e3, quiet_ms: 12e3, queued: 0 });
    });
    const tab = document.querySelector('nav [aria-current="page"]');
    const head = document.querySelector('[data-session-head] [role="status"]');
    await waitFor(() => expect(head).toHaveTextContent('Waiting for you · 12s'));
    expect([dotOf(rowButton(B)), dotOf(group('agent-router')), dotOf(tab), dotOf(head)].map((d) => d?.getAttribute('data-tone'))).toEqual([
      'attention',
      'attention',
      'attention',
      'attention',
    ]);
  });

  it('SE-C45, SE-C49: the open session always has its tab, archived or ended; each tab shows its dot and its title, else its id; the open one is marked; a tab opens its session', async () => {
    const rows = [
      session(A, { title: 'Port the live view', archived: true, activity: 'closed', tone: 'closed', active: false, open: false, detail: 'Session ended', last_activity_ms: AT - 20 * 3600e3 }),
      session(B, { title: null }),
    ];
    stubRouter(undefined, (req) => (req.method === 'GET' && req.path === '/api/live/sessions' ? json(rows) : undefined));
    renderSessions(`/live?session=${A}`);
    await waitFor(() => streamOf(A));
    const strip = within(await waitFor(() => screen.getByRole('navigation', { name: 'This project’s sessions' })));
    const tabs = strip.getAllByRole('link');
    expect(tabs.map((t) => t.textContent)).toEqual(['Port the live viewended', 'bbbbbbbbworking']);
    expect(tabs[0]).toHaveAttribute('aria-current', 'page');
    expect(tabs[1]).not.toHaveAttribute('aria-current');
    await press(tabs[1] as HTMLElement);
    await waitFor(() => expect(where()).toBe(`/live?session=${B}`));
  });

  it('SE-C50: every tab carries an Archive action of its own, never taken for a tap on the tab, and archiving the open session keeps its tab while it is open', async () => {
    let rows: SessionRow[] = SESSIONS();
    const router = stubRouter((req) => {
      if (req.method === 'GET' && req.path === '/api/live/sessions') return json(rows);
      if (req.method === 'POST' && req.path === '/api/live/archive') {
        rows = rows.map((r) => (r.session_id === req.body?.['session'] ? { ...r, archived: true } : r));
        return json({ ok: true, archived: 1 });
      }
      return undefined;
    });
    renderSessions(`/live?session=${A}`);
    await waitFor(() => streamOf(A));
    const strip = within(await waitFor(() => screen.getByRole('navigation', { name: 'This project’s sessions' })));
    const archiveA = strip.getByRole('button', { name: 'Archive Port the live view' });
    expect(archiveA.closest('a')).toBeNull();
    await press(archiveA);
    expect(router.posts('/api/live/archive').map((p) => p.body)).toEqual([{ session: A, archived: true }]);
    expect(where()).toBe(`/live?session=${A}`);
    await waitFor(() => expect(within(rowOf(A)).getByRole('button', { name: /^Restore/ })).toBeInTheDocument());
    expect(strip.getByRole('link', { name: /Port the live view/ })).toHaveAttribute('aria-current', 'page');
  });

  it('SE-C146: the composer belongs to an open session: not on the list, not in Find', async () => {
    stubRouter();
    renderSessions(`/live?session=${A}`, <Back />);
    await waitFor(() => streamOf(A));
    expect(screen.getByRole('textbox', { name: 'Message to this session' })).toBeInTheDocument();
    await press(within(document.querySelector('[data-session-head]') as HTMLElement).getByRole('button', { name: 'Find' }));
    await waitFor(() => expect(screen.getByRole('searchbox')).toBeInTheDocument());
    expect(screen.queryByRole('textbox', { name: 'Message to this session' })).toBeNull();
    // Back closes Find, and the composer is the session's again.
    await press(screen.getByTestId('back'));
    await waitFor(() => expect(screen.getByRole('textbox', { name: 'Message to this session' })).toBeInTheDocument());
    await press(screen.getByRole('button', { name: 'Back to the list' }));
    await waitFor(() => expect(where()).toBe('/live'));
    expect(screen.queryByRole('textbox', { name: 'Message to this session' })).toBeNull();
  });

  it('SE-C196: the heading says where the reader is — the screen’s name on the list, the project with a session open, "Find" in Find', async () => {
    stubRouter();
    renderSessions('/live', <Back />);
    await waitFor(() => rowOf(A));
    expect(screen.getByRole('heading', { level: 1 })).toHaveTextContent('Live sessions');
    await press(rowButton(C));
    await waitFor(() => expect(screen.getByRole('heading', { level: 1 })).toHaveTextContent('northstar'));
    await press(within(document.querySelector('[data-session-head]') as HTMLElement).getByRole('button', { name: 'Find' }));
    await waitFor(() => expect(screen.getByRole('heading', { level: 1 })).toHaveTextContent('Find'));
  });

  it('SE-C191, SE-C182: an address naming a session the router does not list lands on the list and says so; opening a session clears the notice', async () => {
    stubRouter();
    renderSessions('/live?session=zzzzzzzz-gone');
    await waitFor(() => expect(where()).toBe('/live'));
    expect(await screen.findByText('That session is no longer listed.')).toBeInTheDocument();
    await press(rowButton(A));
    await waitFor(() => expect(where()).toBe(`/live?session=${A}`));
    expect(screen.queryByText('That session is no longer listed.')).toBeNull();
  });

  it('SE-C206: the header’s status is announced when what the session is doing changes, never at each heartbeat whose duration ticks', async () => {
    stubRouter();
    renderSessions(`/live?session=${A}`);
    await waitFor(() => streamOf(A));
    const live = () => document.querySelector('[data-session-head] [aria-live="polite"]');
    const pill = () => document.querySelector('[data-session-head] [role="status"]');
    act(() => {
      streamOf(A).open().status(working({ quiet_ms: 12e3 }));
    });
    expect(live()).toHaveTextContent('Running Bash');
    const announced = live()?.textContent;
    act(() => {
      streamOf(A).status(working({ as_of_ms: AT + 1000 }));
    });
    expect(pill()).toHaveTextContent('Running Bash · 13s');
    expect(live()?.textContent).toBe(announced);
    // The pill itself is never a live region: its duration ticks every second.
    expect(pill()).toHaveAttribute('aria-live', 'off');
    act(() => {
      streamOf(A).status(working({ activity: 'generating', detail: 'Thinking' }));
    });
    expect(live()).toHaveTextContent('Thinking');
  });
});

describe('arriving at an entry', () => {
  const HIT = { block_id: 7, kind: 'text', role: 'assistant', ordinal: 3, at: AT - 60e3, session_id: A, project: '/Users/sample/code/agent-router', snippet: 'where does the ‹kestrel› roost … at dusk', rank: -2 };
  const answer = (hits: object[]) => ({ query: 'kestrel', expression: 'kestrel', sessions: [{ session_id: A, project: null, hits: hits.length, blocks: 1 }], hits });

  it('SE-C171: an address naming a hit’s session, block and ordinal together lands on that entry — scrolled to, focused once, and kept marked', async () => {
    stubRouter();
    const scrolled = vi.spyOn(Element.prototype, 'scrollIntoView');
    const { client } = renderSessions(`/live?session=${A}&block=7&ordinal=3`);
    // The search that found it, as the palette or Find left it in the page's cache; the same block
    // at another place is another hit, and is not the one named.
    client.setQueryData(['search', 'kestrel', 'all'], answer([{ ...HIT, ordinal: 9, snippet: 'the ‹kestrel› again' }, HIT]));
    await waitFor(() => streamOf(A));
    act(() => {
      streamOf(A)
        .open()
        .row({ role: 'assistant', kind: 'text', text: 'the kestrel again, later' })
        .row({ role: 'assistant', kind: 'text', text: 'And where does the kestrel roost? Up high, at dusk.' })
        .row({ role: 'user', kind: 'text', text: 'thanks' })
        .status(working());
    });
    const marked = await waitFor(() => {
      const m = document.querySelector('[data-entry][aria-current="true"]');
      expect(m).not.toBeNull();
      return m as HTMLElement;
    });
    expect(marked).toHaveTextContent('And where does the kestrel roost? Up high, at dusk.');
    expect(document.activeElement).toBe(marked);
    expect(scrolled.mock.contexts).toContain(marked);
    // A replay of the tail draws the feed anew: the entry is marked again, and not focused twice.
    (document.activeElement as HTMLElement).blur();
    act(() => {
      streamOf(A).drop(0).open().row({ role: 'assistant', kind: 'text', text: 'And where does the kestrel roost? Up high, at dusk.' });
    });
    await waitFor(() => expect(document.querySelector('[data-entry][aria-current="true"]')).toHaveTextContent('where does the kestrel roost'));
    expect(document.activeElement).not.toBe(document.querySelector('[data-entry][aria-current="true"]'));
  });

  it('SE-C171: an entry the feed does not reach is said to be earlier than its tail, with Find as the way there', async () => {
    stubRouter();
    const { client } = renderSessions(`/live?session=${A}&block=7&ordinal=3`);
    client.setQueryData(['search', 'kestrel', 'all'], answer([HIT]));
    await waitFor(() => streamOf(A));
    act(() => {
      streamOf(A).open().row({ role: 'assistant', kind: 'text', text: 'nothing about birds' }).status(working());
    });
    expect(await screen.findByText(/the entry this address names is earlier than it reaches\. Find searches all of this session\./)).toBeInTheDocument();
    expect(document.querySelector('[data-entry][aria-current="true"]')).toBeNull();
  });
});

describe('a hosted router', () => {
  it('SE-C187, SE-C188, MA-C2: says why its list is empty and links to the Machines screen to connect a machine, never blaming the week nor repeating the commands, and never asks after a wrapper', async () => {
    const router = stubRouter((req) => {
      if (req.method === 'GET' && req.path === '/api/auth') return json({ mode: 'server' });
      if (req.method === 'GET' && req.path === '/api/live/sessions') return json([]);
      return undefined;
    });
    renderSessions();
    expect(await screen.findByText('No machine has connected yet.')).toBeInTheDocument();
    expect(screen.getByText('No sessions here yet')).toBeInTheDocument();
    // The commands have one home, the Machines screen, and the empty list links there.
    const connect = listPane().getByRole('link', { name: 'Connect a machine' });
    expect(connect).toHaveAttribute('href', '/machines');
    expect(screen.queryByRole('group', { name: 'macOS and Linux' })).toBeNull();
    expect(screen.queryByText(/agent\/install\.sh/)).toBeNull();
    expect(screen.queryByRole('button', { name: 'Copy command' })).toBeNull();
    expect(screen.queryByText('No sessions in the last week.')).toBeNull();
    expect(router.gets('/api/setup/shim')).toEqual([]);
    await press(connect);
    expect(where()).toBe('/machines');
  });

  it('SE-C191, SE-C173: an address naming a session on no connected machine lands on the list and says so in the hosted router’s words', async () => {
    stubRouter((req) => (req.method === 'GET' && req.path === '/api/auth' ? json({ mode: 'server' }) : undefined));
    renderSessions('/live?session=zzzzzzzz-elsewhere');
    expect(await screen.findByText('That session is not on a connected machine.')).toBeInTheDocument();
    expect(where()).toBe('/live');
    expect(screen.queryByText('That session is no longer listed.')).toBeNull();
  });

  const STUDIO = { id: '5f0c2a8e', label: 'studio' };
  const LAPTOP = { id: '9b3e1d47', label: 'Laptop' };
  const quiet = { activity: 'idle', tone: 'idle', state: 'idle', active: false, detail: 'Waiting for you' } as const;
  /// A hosted router listing `rows`, whatever else it answers asked of `more` first.
  const hosted = (rows: () => readonly SessionRow[], more: (req: Asked) => Response | undefined = () => undefined) =>
    stubRouter((req) => {
      if (req.method === 'GET' && req.path === '/api/auth') return json({ mode: 'server' });
      if (req.method === 'GET' && req.path === '/api/live/sessions') return json(rows());
      return more(req);
    });
  /// A machine's part of the list: a region named by its heading.
  const part = (name: string) => listPane().getByRole('region', { name });
  const machineHeadings = () => listPane().queryAllByRole('heading', { level: 2 }).map((h) => h.textContent);

  it('SE-C219: sessions from two machines are headed per machine, each heading over its own projects, older fold and shelf, and no row names its machine', async () => {
    const rows = [
      session(A, { machine: STUDIO }),
      session(B, { machine: LAPTOP, title: 'On the laptop' }),
      session(C, { cwd: '/Users/sample/code/northstar', project: 'northstar', machine: STUDIO, ...quiet, last_activity_ms: AT - 13 * 3600e3 }),
      session('shelf-1', { machine: LAPTOP, title: 'Put away', archived: true, ...quiet }),
    ];
    hosted(() => rows);
    renderSessions();
    await waitFor(() => rowOf(A));
    expect(machineHeadings()).toEqual(['Laptop', 'studio']);
    // The same project on each machine is a project under each heading, and each machine has its
    // own fold and its own shelf.
    expect([part('studio').contains(rowOf(A)), part('Laptop').contains(rowOf(B)), part('Laptop').contains(rowOf('shelf-1'))]).toEqual([true, true, true]);
    expect(within(part('studio')).getByRole('button', { name: '1 older session' })).toBeInTheDocument();
    expect(within(part('Laptop')).queryByRole('button', { name: /older session/ })).toBeNull();
    expect(within(part('Laptop')).getByText('1 archived')).toBeInTheDocument();
    expect(within(part('studio')).queryByText(/archived/)).toBeNull();
    // The heading names the machine; no row says it again.
    expect([rowButton(A), rowButton(B), rowButton('shelf-1')].map((b) => /\bon (studio|Laptop)\b/.test(b.textContent ?? ''))).toEqual([false, false, false]);
  });

  it('SE-C219: with one machine, nothing is named and no machine heading is drawn, the list as a local router’s', async () => {
    hosted(() => SESSIONS().map((r) => ({ ...r, machine: STUDIO })));
    renderSessions();
    await waitFor(() => rowOf(C));
    expect(machineHeadings()).toEqual([]);
    expect(listPane().queryByText(/studio/)).toBeNull();
    expect(group('agent-router')).toHaveTextContent('2 sessions');
  });

  it('SE-C220: a machine with nothing recent still heads its shelf, and rows that name no machine come last, under "Unnamed machine"', async () => {
    const rows = [session(A, { machine: STUDIO }), session(B, { machine: LAPTOP, archived: true, ...quiet }), session(C, { cwd: '/Users/sample/code/northstar', project: 'northstar' })];
    hosted(() => rows);
    renderSessions();
    await waitFor(() => rowOf(C));
    expect(machineHeadings()).toEqual(['Laptop', 'studio', 'Unnamed machine']);
    expect(within(part('Laptop')).getByText('1 archived')).toBeInTheDocument();
    expect(part('Laptop').contains(rowOf(B))).toBe(true);
    expect(part('Unnamed machine').contains(rowOf(C))).toBe(true);
  });

  it('SE-C219, SE-C31: each machine’s older fold opens on its own, and keeps its state across every refresh', async () => {
    const old = { ...quiet, last_activity_ms: AT - 13 * 3600e3 };
    const rows = [session(A, { machine: STUDIO }), session('s-old', { machine: STUDIO, ...old }), session(B, { machine: LAPTOP }), session('l-old', { machine: LAPTOP, ...old })];
    hosted(() => rows);
    const { client } = renderSessions();
    const fold = (name: string) => within(part(name)).getByRole('button', { name: '1 older session' });
    await waitFor(() => fold('studio'));
    await press(fold('studio'));
    expect([fold('studio'), fold('Laptop')].map((b) => b.getAttribute('aria-expanded'))).toEqual(['true', 'false']);
    expect(part('studio').contains(rowOf('s-old'))).toBe(true);
    expect(document.querySelector('[data-session="l-old"]')).toBeNull();
    await act(async () => {
      await client.refetchQueries({ queryKey: ['live', 'sessions'] });
    });
    expect([fold('studio'), fold('Laptop')].map((b) => b.getAttribute('aria-expanded'))).toEqual(['true', 'false']);
  });

  it('SE-C221: the open session names its machine beside its project, as both phones do, and its tabs are its project’s sessions on that machine only', async () => {
    const rows = [session(A, { machine: STUDIO, created_ms: AT - 7200e3 }), session(B, { machine: STUDIO, created_ms: AT - 3600e3 }), session(C, { machine: LAPTOP, created_ms: AT - 1800e3 })];
    hosted(() => rows);
    renderSessions(`/live?session=${A}`);
    await waitFor(() => streamOf(A));
    const head = () => document.querySelector('[data-session-head]') as HTMLElement;
    await waitFor(() => expect(head().querySelector('[data-session-machine]')).toHaveTextContent('on studio'));
    expect(within(head()).getByRole('heading', { level: 1 })).toHaveTextContent('agent-router');
    const strip = within(screen.getByRole('navigation', { name: 'This project’s sessions' }));
    expect(strip.getAllByRole('link').map((t) => t.getAttribute('href'))).toEqual([`/live?session=${A}`, `/live?session=${B}`]);
  });

  it('SE-C221: a session on a hosted router with one machine still names it, and a local router’s names none', async () => {
    hosted(() => [session(A, { machine: STUDIO })]);
    const first = renderSessions(`/live?session=${A}`);
    await waitFor(() => expect(document.querySelector('[data-session-head] [data-session-machine]')).toHaveTextContent('on studio'));
    first.unmount();
    stubRouter();
    renderSessions(`/live?session=${A}`);
    await waitFor(() => expect(document.querySelector('[data-session-head] h1')).toHaveTextContent('agent-router'));
    expect(document.querySelector('[data-session-head] [data-session-machine]')).toBeNull();
  });

  it('SE-C223: a session whose machine is offline says the router’s sentence in its header, keeps its rows and never reads as ended, and streams again once its machine is back', async () => {
    const OFFLINE = 'the machine running this session (studio) is offline';
    vi.useFakeTimers({ shouldAdvanceTime: true });
    let offline = false;
    hosted(
      () => [session(A, { machine: STUDIO })],
      (req) => (offline && req.path.startsWith('/api/live/') && req.path !== '/api/live/sessions' ? json({ ok: false, error: OFFLINE }, 503) : undefined),
    );
    renderSessions(`/live?session=${A}`);
    await waitFor(() => streamOf(A));
    const pill = () => document.querySelector('[data-session-head] [role="status"]') as HTMLElement;
    act(() => {
      streamOf(A).open().row({ role: 'assistant', kind: 'text', text: 'Read before the machine went.' }).status(working());
    });
    await waitFor(() => expect(pill()).toHaveTextContent('Running Bash · 12s'));
    // The machine goes: the browser gives the stream up, and the router says why.
    offline = true;
    const gone = streamOf(A);
    act(() => {
      gone.drop(2);
    });
    await waitFor(() => expect(pill()).toHaveTextContent(OFFLINE));
    expect(pill()).toHaveAttribute('data-stale');
    expect(screen.queryByRole('button', { name: 'Stop the turn' })).toBeNull();
    expect(screen.getByText('Read before the machine went.')).toBeInTheDocument();
    expect(document.querySelector('[data-session-head]')?.textContent).not.toMatch(/ended|reconnecting/);
    // Asked for again ten seconds on, and the sentence stands while it is.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(10_000);
    });
    await waitFor(() => expect(streamOf(A)).not.toBe(gone));
    expect(pill()).toHaveTextContent(OFFLINE);
    // The machine is back: the first thing its stream says ends the sentence.
    offline = false;
    act(() => {
      streamOf(A).open().row({ role: 'assistant', kind: 'text', text: 'Read before the machine went.' }).status(working());
    });
    await waitFor(() => expect(pill()).toHaveTextContent('Running Bash · 12s'));
    expect(screen.getAllByText('Read before the machine went.')).toHaveLength(1);
  });
});

describe('Find', () => {
  it('SE-C175: Find is in the address — opening it adds a history entry, typing replaces it, Back closes it, and a reload keeps it', async () => {
    stubRouter((req) => (req.method === 'GET' && req.path === '/api/search' ? json({ query: req.query.get('q'), expression: req.query.get('q'), sessions: [], hits: [] }) : undefined));
    const first = renderSessions('/live', <Back />);
    await waitFor(() => rowOf(A));
    await press(listPane().getByRole('button', { name: 'Find' }));
    await waitFor(() => expect(where()).toBe('/live?find=&scope=all'));
    fireEvent.change(screen.getByRole('searchbox'), { target: { value: 'kestrel' } });
    await waitFor(() => expect(where()).toBe('/live?find=kestrel&scope=all'));
    await press(screen.getByTestId('back'));
    await waitFor(() => expect(where()).toBe('/live'));
    expect(screen.queryByRole('searchbox')).toBeNull();
    first.unmount();
    renderSessions('/live?find=kestrel&scope=all');
    await waitFor(() => expect(screen.getByRole('searchbox')).toHaveValue('kestrel'));
  });

  it('SE-C176: Escape closes Find wherever the focus is, and the Find button toggles it', async () => {
    stubRouter();
    renderSessions('/live', <Back />);
    await waitFor(() => rowOf(A));
    const find = () => listPane().getByRole('button', { name: 'Find' });
    await press(find());
    await waitFor(() => expect(screen.getByRole('searchbox')).toBeInTheDocument());
    // From the field itself, whose own Escape would only clear it.
    fireEvent.change(screen.getByRole('searchbox'), { target: { value: 'kestrel' } });
    fireEvent.keyDown(screen.getByRole('searchbox'), { key: 'Escape' });
    await waitFor(() => expect(screen.queryByRole('searchbox')).toBeNull());
    await press(find());
    await waitFor(() => expect(screen.getByRole('searchbox')).toBeInTheDocument());
    // From anywhere else.
    act(() => {
      fireEvent.keyDown(document.body, { key: 'Escape' });
    });
    await waitFor(() => expect(screen.queryByRole('searchbox')).toBeNull());
    await press(find());
    await waitFor(() => expect(screen.getByRole('searchbox')).toBeInTheDocument());
    expect(find()).toHaveAttribute('aria-pressed', 'true');
    await press(find());
    await waitFor(() => expect(screen.queryByRole('searchbox')).toBeNull());
  });

  it('SE-C161: from a session, Find searches the router’s whole index of it — every turn, not the feed’s tail — and says so', async () => {
    const router = stubRouter((req) => (req.method === 'GET' && req.path === '/api/search' ? json({ query: 'kestrel', expression: 'kestrel', sessions: [], hits: [] }) : undefined));
    renderSessions(`/live?session=${A}`);
    await waitFor(() => streamOf(A));
    await press(within(document.querySelector('[data-session-head]') as HTMLElement).getByRole('button', { name: 'Find' }));
    fireEvent.change(await screen.findByRole('searchbox'), { target: { value: 'kestrel' } });
    await waitFor(() => expect(router.gets('/api/search')).toHaveLength(1));
    const q = router.gets('/api/search')[0]?.query;
    expect(Object.fromEntries(q ?? [])).toEqual({ q: 'kestrel', limit: '30', session: A });
    expect(screen.getByText(/Find searches everything this router has recorded, every transcript to its first turn/)).toBeInTheDocument();
    // Find never touches the stream: the session stays connected under it.
    expect(streamOf(A).closed).toBe(false);
  });

  it('SE-C178, SE-C179: opening a hit whose session the router no longer lists says so by the hit', async () => {
    const reply = {
      query: 'kestrel',
      expression: 'kestrel',
      sessions: [{ session_id: 'zzzzzzzz-1111', project: '/x/gone', hits: 1, blocks: 1 }],
      hits: [{ block_id: 7, kind: 'text', role: 'user', ordinal: 3, at: AT - 60e3, session_id: 'zzzzzzzz-1111', project: '/x/gone', snippet: 'where is the ‹kestrel›', rank: -1 }],
    };
    stubRouter((req) => (req.method === 'GET' && req.path === '/api/search' ? json(reply) : undefined));
    renderSessions('/live?find=kestrel&scope=all');
    const hit = await screen.findByRole('button', { name: /where is the kestrel/ });
    expect(hit).toHaveAccessibleName(/^message, user, \S+ ago: where is the kestrel\. In gone · zzzzzzzz$/);
    await press(hit);
    const wrap = hit.parentElement as HTMLElement;
    expect(await within(wrap).findByText('That session is no longer listed.')).toBeInTheDocument();
    expect(where()).toBe('/live?find=kestrel&scope=all');
  });
});

describe('what a reader can reach', () => {
  it('SE-C203: every control is a real button or link with a name — a row, Restore, a tab and its Archive, the fold, a hit, a notice’s dismiss, Back', async () => {
    const rows = [...SESSIONS(), session('shelf-1', { title: 'Put away', archived: true }), session('old-a', { title: 'Long ago', activity: 'idle', tone: 'idle', active: false, last_activity_ms: AT - 13 * 3600e3 })];
    stubRouter((req) => (req.method === 'GET' && req.path === '/api/live/sessions' ? json(rows) : undefined));
    renderSessions(`/live?session=${A}`);
    await waitFor(() => streamOf(A));
    act(() => {
      streamOf(A).open().row({ role: 'assistant', kind: 'thinking', text: 'hmm', dur_ms: 2400 }).status(working());
    });
    expect(rowButton(B).tagName).toBe('BUTTON');
    expect(within(rowOf('shelf-1')).getByRole('button', { name: 'Restore Put away' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: '1 older session' })).toBeInTheDocument();
    const strip = within(screen.getByRole('navigation', { name: 'This project’s sessions' }));
    expect(strip.getByRole('link', { name: /Write the specs/ })).toBeInTheDocument();
    expect(strip.getByRole('button', { name: 'Archive Write the specs' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Thought for 2.4s ›' })).toHaveAttribute('aria-expanded', 'false');
    expect(screen.getByRole('button', { name: 'Back to the list' })).toBeInTheDocument();
    // Nothing a reader presses is a bare div or span.
    const clickable = [...document.querySelectorAll('[data-sessions-screen] [onclick], [data-sessions-screen] div[role="button"], [data-sessions-screen] span[role="button"]')];
    expect(clickable).toEqual([]);
  });

  it('SE-C204: what live and view mean, what a tab’s session is doing, what the switchers, Find, a side question, Stop and Archive do — each is visible text or one tap away', async () => {
    stubRouter();
    renderSessions(`/live?session=${A}`);
    await waitFor(() => streamOf(A));
    act(() => {
      streamOf(A).open().status(working());
    });
    const tap = async (label: string, says: RegExp) => {
      await press(screen.getByRole('button', { name: label }));
      await waitFor(() => expect(screen.getByText(says)).toBeVisible());
    };
    await tap('What live and view mean', /takes a typed message and a Stop/);
    await tap('What Archive does', /Archive puts a session away/);
    await tap('What these switch', /what it stops to ask you about/);
    await tap('What these do', /A side question is answered from this session’s context, without interrupting it/);
    expect(screen.getByText(/Stop the turn interrupts the turn running now/)).toBeInTheDocument();
    // A tab says what its session is doing in words beside its dot.
    const strip = within(screen.getByRole('navigation', { name: 'This project’s sessions' }));
    expect(strip.getByText('waiting for you')).toBeVisible();
    await press(within(document.querySelector('[data-session-head]') as HTMLElement).getByRole('button', { name: 'Find' }));
    expect(await screen.findByText(/Find searches everything this router has recorded/)).toBeVisible();
  });
});
