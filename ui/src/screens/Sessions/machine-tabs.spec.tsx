// New for agent-router: a hosted router's list of several machines, tabbed (SE-C226 to SE-C230) — All
// first, then a tab for each machine with what is at work there and what waits, the tab in the
// address, a machine the list no longer holds falling back to All, and the row of tabs on a phone —
// against the router faked at `fetch`. The model's rules are pinned in ~/model/session.spec.ts; these
// pin the screen that draws them.
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { act, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useNavigate } from 'react-router';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { MACHINE_UNLISTED, type SessionRow } from '~/model/session';
import { press, stubCoarse, stubNarrow } from '~/test-utils';
import { FakeEventSource } from './fakes';
import { A, AT, B, C, SESSIONS, json, renderSessions, session, streamOf, stubRouter, type Asked } from './kit';

beforeEach(() => FakeEventSource.reset());
afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

/// Three machines, as a hosted router labels them (src/machines.rs, `machine: {id, label}`).
const STUDIO = { id: '5f0c2a8e-3b1d-4c6f-9e7a-2d4b6c8e0f13', label: 'studio' };
const LAPTOP = { id: '9b3e1d47-6a2c-4f8e-b1d3-7c5e9a0b2d46', label: 'Laptop' };
const WINBOX = { id: 'c41d07e2-8f3a-4b6d-a2e9-5d1c3b7f9e08', label: 'win-box' };
const D = 'dddddddd-1111-2222-3333-444444444444';
const quiet = { activity: 'idle', tone: 'idle', state: 'idle', active: false, detail: 'Waiting for you' } as const;

/// Each machine has a part of its own: the studio two sessions of one project, so an open one has
/// tabs, and a quiet project; the laptop one waiting on you and its shelf; the Windows box one at
/// work and one folded under "older".
const MACHINES = (): SessionRow[] => [
  session(A, { machine: STUDIO, created_ms: AT - 7200e3 }),
  session(D, { machine: STUDIO, title: 'Second on the studio', created_ms: AT - 3600e3, ...quiet }),
  session(C, { machine: STUDIO, cwd: '/Users/sample/code/northstar', project: 'northstar', ...quiet }),
  session(B, { machine: LAPTOP, title: 'On the laptop', activity: 'waiting', tone: 'attention', state: 'ask', detail: 'Waiting for you' }),
  session('l-shelf', { machine: LAPTOP, title: 'Put away', archived: true, ...quiet }),
  session('w-1', { machine: WINBOX, cwd: 'C:\\Users\\sample\\code\\win-app', project: 'win-app', title: 'On Windows' }),
  session('w-old', { machine: WINBOX, cwd: 'C:\\Users\\sample\\code\\win-app', project: 'win-app', title: 'Long ago', ...quiet, last_activity_ms: AT - 13 * 3600e3 }),
];

/// A hosted router listing `rows`, whatever else it answers asked of `more` first.
const hosted = (rows: () => readonly SessionRow[] = MACHINES, more: (req: Asked) => Response | undefined = () => undefined) =>
  stubRouter((req) => {
    if (req.method === 'GET' && req.path === '/api/auth') return json({ mode: 'server' });
    if (req.method === 'GET' && req.path === '/api/live/sessions') return json(rows());
    return more(req);
  });

const where = () => screen.getByTestId('location').textContent;
const listPane = () => within(document.querySelector<HTMLElement>('[data-pane="list"]') as HTMLElement);
const rowOf = (id: string) => document.querySelector<HTMLElement>(`[data-session="${id}"]`);
const rowButton = (id: string) => {
  const li = rowOf(id);
  if (!li) throw new Error(`no list row for ${id}`);
  return li.querySelector('button') as HTMLButtonElement;
};
const tablist = () => listPane().getByRole('tablist', { name: 'Machines' });
const tabs = () => within(tablist()).getAllByRole('tab');
const tab = (name: string | RegExp) => within(tablist()).getByRole('tab', { name });
const chosen = () => tabs().find((t) => t.getAttribute('aria-selected') === 'true');
const panel = () => listPane().getByRole('tabpanel');
const machineHeadings = () => listPane().queryAllByRole('heading', { level: 2 }).map((h) => h.textContent);

/// The browser's Back, pressed from inside the router.
function Back() {
  const navigate = useNavigate();
  return (
    <button type="button" data-testid="back" hidden onClick={() => void navigate(-1)}>
      back
    </button>
  );
}

describe('a hosted router’s machines, tabbed', () => {
  it('SE-C226, SE-C227: over several machines, All first and then each machine in its heading’s order, each tab with its label, what is at work and what waits, and the attention dot on the one with a prompt; All is the list headed per machine, under the overall pip', async () => {
    hosted();
    renderSessions();
    await waitFor(() => tablist());
    // Its name carries its counts, which say whether something waits; the panel is labelled by it.
    expect(tabs().map((t) => t.getAttribute('aria-label'))).toEqual(['All, 2 working · 1 waiting', 'Laptop, 1 waiting', 'studio, 1 working', 'win-box, 1 working']);
    expect(tabs().map((t) => t.textContent)).toEqual(['All2 working · 1 waiting', 'Laptop1 waiting', 'studio1 working', 'win-box1 working']);
    const marked = tabs().map((t) => t.querySelector('[data-tone="attention"]') !== null);
    expect(marked).toEqual([true, true, false, false]);
    expect(chosen()).toBe(tab(/^All/));
    expect(panel()).toHaveAccessibleName('All, 2 working · 1 waiting');
    // All is the list as it was: every machine under its heading, with its fold and its shelf.
    expect(machineHeadings()).toEqual(['Laptop', 'studio', 'win-box']);
    expect([A, B, C, D, 'l-shelf', 'w-1'].map((id) => panel().contains(rowOf(id)))).toEqual([true, true, true, true, true, true]);
    expect(within(panel()).getByRole('button', { name: '1 older session' })).toBeInTheDocument();
    // The pip over it stays the whole list's.
    expect(listPane().getByRole('status')).toHaveTextContent('2 working · 1 waiting');
  });

  it('SE-C227: the tabs are reached by keyboard — arrows move between them without choosing one, and Enter or Space chooses', async () => {
    hosted();
    renderSessions();
    await waitFor(() => tablist());
    const user = userEvent.setup();
    act(() => tab(/^All/).focus());
    await user.keyboard('{ArrowRight}');
    expect(document.activeElement).toBe(tab(/^Laptop/));
    await user.keyboard('{ArrowRight}');
    expect(document.activeElement).toBe(tab(/^studio/));
    // Passing over a tab neither draws its machine nor writes the address.
    expect(where()).toBe('/live');
    expect(chosen()).toBe(tab(/^All/));
    await user.keyboard('{Enter}');
    await waitFor(() => expect(where()).toBe(`/live?machine=${STUDIO.id}`));
    expect(chosen()).toBe(tab(/^studio/));
    await user.keyboard('{End}');
    expect(document.activeElement).toBe(tab(/^win-box/));
    await user.keyboard(' ');
    await waitFor(() => expect(where()).toBe(`/live?machine=${WINBOX.id}`));
  });

  it('SE-C226, SE-C228: choosing a machine’s tab writes `?machine=` with a history entry of its own and draws its part alone — its projects, its fold and its shelf — with no heading; All is the address without it, and Back returns to the tab before', async () => {
    hosted();
    renderSessions('/live', <Back />);
    await waitFor(() => tablist());
    await press(tab(/^studio/));
    await waitFor(() => expect(where()).toBe(`/live?machine=${STUDIO.id}`));
    expect(chosen()).toBe(tab(/^studio/));
    expect(panel()).toHaveAccessibleName('studio, 1 working');
    expect([A, C, D].map((id) => panel().contains(rowOf(id)))).toEqual([true, true, true]);
    expect([B, 'l-shelf', 'w-1'].map(rowOf)).toEqual([null, null, null]);
    // The tab names the machine, so no heading does.
    expect(machineHeadings()).toEqual([]);
    expect(listPane().queryByText('studio', { selector: 'h2 *, h3 *' })).toBeNull();
    await press(tab(/^Laptop/));
    await waitFor(() => expect(where()).toBe(`/live?machine=${LAPTOP.id}`));
    expect(within(panel()).getByText('1 archived')).toBeInTheDocument();
    expect(panel().contains(rowOf('l-shelf'))).toBe(true);
    await press(tab(/^win-box/));
    await waitFor(() => expect(within(panel()).getByRole('button', { name: '1 older session' })).toBeInTheDocument());
    await press(tab(/^All/));
    await waitFor(() => expect(where()).toBe('/live'));
    expect(machineHeadings()).toEqual(['Laptop', 'studio', 'win-box']);
    await press(screen.getByTestId('back'));
    await waitFor(() => expect(where()).toBe(`/live?machine=${WINBOX.id}`));
    expect(chosen()).toBe(tab(/^win-box/));
    // Choosing the tab already chosen adds nothing.
    await press(tab(/^win-box/));
    await press(screen.getByTestId('back'));
    await waitFor(() => expect(where()).toBe(`/live?machine=${LAPTOP.id}`));
  });

  it('SE-C228: a link to a machine’s tab opens on it', async () => {
    hosted();
    renderSessions(`/live?machine=${LAPTOP.id}`);
    await waitFor(() => expect(rowOf(B)).not.toBeNull());
    expect(chosen()).toBe(tab(/^Laptop/));
    expect(rowOf(A)).toBeNull();
    expect(machineHeadings()).toEqual([]);
    expect(screen.queryByText(MACHINE_UNLISTED)).toBeNull();
  });

  it('SE-C228: a session opened from a machine’s tab keeps `machine=` — through its own tabs, Find and Back to the list — so going back, by the button or the browser, returns to the tab', async () => {
    hosted();
    renderSessions(`/live?machine=${STUDIO.id}`, <Back />);
    await waitFor(() => expect(rowOf(A)).not.toBeNull());
    await press(rowButton(A));
    await waitFor(() => expect(where()).toBe(`/live?machine=${STUDIO.id}&session=${A}`));
    await waitFor(() => streamOf(A));
    // The open session is drawn as ever; its own tabs keep the list's.
    const strip = within(screen.getByRole('navigation', { name: 'This project’s sessions' }));
    expect(strip.getAllByRole('link').map((t) => t.getAttribute('href'))).toEqual([`/live?machine=${STUDIO.id}&session=${A}`, `/live?machine=${STUDIO.id}&session=${D}`]);
    await press(strip.getByRole('link', { name: /Second on the studio/ }));
    await waitFor(() => expect(where()).toBe(`/live?machine=${STUDIO.id}&session=${D}`));
    await press(within(document.querySelector('[data-session-head]') as HTMLElement).getByRole('button', { name: 'Find' }));
    await waitFor(() => expect(where()).toBe(`/live?machine=${STUDIO.id}&session=${D}&find=&scope=session`));
    await press(screen.getByTestId('back'));
    await waitFor(() => expect(where()).toBe(`/live?machine=${STUDIO.id}&session=${D}`));
    await press(screen.getByRole('button', { name: 'Back to the list' }));
    await waitFor(() => expect(where()).toBe(`/live?machine=${STUDIO.id}`));
    expect(chosen()).toBe(tab(/^studio/));
    // The browser's Back walks the same way.
    await press(screen.getByTestId('back'));
    await waitFor(() => expect(where()).toBe(`/live?machine=${STUDIO.id}&session=${D}`));
  });

  it('SE-C31, SE-C230: a machine’s fold is the same fold on All and on its tab, kept across tabs and refreshes, and another tab is read from its top', async () => {
    hosted();
    const { client } = renderSessions();
    await waitFor(() => tablist());
    const scroller = document.querySelector('[data-pane="list"] > div:last-child') as HTMLElement;
    const fold = () => within(panel()).getByRole('button', { name: '1 older session' });
    await press(tab(/^win-box/));
    await waitFor(() => fold());
    await press(fold());
    expect(fold()).toHaveAttribute('aria-expanded', 'true');
    expect(panel().contains(rowOf('w-old'))).toBe(true);
    scroller.scrollTop = 300;
    await press(tab(/^All/));
    await waitFor(() => expect(where()).toBe('/live'));
    expect(scroller.scrollTop).toBe(0);
    expect(fold()).toHaveAttribute('aria-expanded', 'true');
    await act(async () => {
      await client.refetchQueries({ queryKey: ['live', 'sessions'] });
    });
    await press(tab(/^win-box/));
    await waitFor(() => expect(where()).toBe(`/live?machine=${WINBOX.id}`));
    expect(fold()).toHaveAttribute('aria-expanded', 'true');
  });

  it('SE-C229: an address naming a machine the list does not hold falls back to All, in place of its own history entry, and says so in one line, never an empty tab', async () => {
    hosted();
    renderSessions('/live?machine=0a1b2c3d-gone');
    expect(await screen.findByText(MACHINE_UNLISTED)).toBeInTheDocument();
    await waitFor(() => expect(where()).toBe('/live'));
    expect(chosen()).toBe(tab(/^All/));
    expect(machineHeadings()).toEqual(['Laptop', 'studio', 'win-box']);
  });

  it('SE-C229: a machine that leaves while its tab is open leaves it for All, and says so', async () => {
    let rows = MACHINES();
    hosted(() => rows);
    const { client } = renderSessions(`/live?machine=${WINBOX.id}`);
    await waitFor(() => expect(rowOf('w-1')).not.toBeNull());
    rows = rows.filter((r) => r.machine !== WINBOX);
    await act(async () => {
      await client.refetchQueries({ queryKey: ['live', 'sessions'] });
    });
    expect(await screen.findByText(MACHINE_UNLISTED)).toBeInTheDocument();
    await waitFor(() => expect(where()).toBe('/live'));
    expect(tabs().map((t) => t.getAttribute('aria-label'))).toEqual(['All, 1 working · 1 waiting', 'Laptop, 1 waiting', 'studio, 1 working']);
    expect(chosen()).toBe(tab(/^All/));
  });

  it('SE-C229, SE-C191: an old link naming a machine and a session that are both gone lands on All, and says both', async () => {
    hosted();
    renderSessions('/live?machine=0a1b2c3d-gone&session=zzzzzzzz-gone');
    expect(await screen.findByText(`That session is not on a connected machine. ${MACHINE_UNLISTED}`)).toBeInTheDocument();
    await waitFor(() => expect(where()).toBe('/live'));
    expect(chosen()).toBe(tab(/^All/));
  });

  it('SE-C226, SE-C229: with one machine a hosted router draws no tabs, and an address naming that machine is kept, unremarked; a local router draws none', async () => {
    hosted(() => SESSIONS().map((r) => ({ ...r, machine: STUDIO })));
    const first = renderSessions(`/live?machine=${STUDIO.id}`);
    await waitFor(() => expect(rowOf(C)).not.toBeNull());
    expect(listPane().queryByRole('tablist')).toBeNull();
    expect(listPane().queryByRole('tab')).toBeNull();
    expect(machineHeadings()).toEqual([]);
    expect(where()).toBe(`/live?machine=${STUDIO.id}`);
    expect(screen.queryByText(MACHINE_UNLISTED)).toBeNull();
    first.unmount();
    stubRouter();
    renderSessions();
    await waitFor(() => expect(rowOf(C)).not.toBeNull());
    expect(listPane().queryByRole('tablist')).toBeNull();
  });
});

describe('the machines’ tabs in a hand', () => {
  const plain = (s: string) => s.replace(/\/\*[\s\S]*?\*\//g, '').replace(/\/\/.*$/gm, '');
  const CSS = plain(readFileSync(join(__dirname, 'Sessions.module.scss'), 'utf8'));
  function block(css: string, opener: string): string {
    const at = css.indexOf(opener);
    expect(at, opener).toBeGreaterThanOrEqual(0);
    const open = css.indexOf('{', at);
    let depth = 0;
    for (let i = open; i < css.length; i++) {
      if (css[i] === '{') depth += 1;
      if (css[i] === '}' && --depth === 0) return css.slice(open + 1, i);
    }
    throw new Error(`unclosed ${opener}`);
  }

  it('SE-C230: on a phone the same tabs are one row that scrolls sideways inside the list — never wrapping, never wider than the list — stays at its top, cuts a long label short, and is a fingertip’s target', async () => {
    stubNarrow();
    stubCoarse();
    hosted();
    renderSessions();
    await waitFor(() => tablist());
    // The same control as on a desk, every tab in it: never a picker in a hand.
    expect(tabs()).toHaveLength(4);
    expect(tabs().every((t) => t.tagName === 'BUTTON')).toBe(true);
    // jsdom lays nothing out, so the row is read from its stylesheet.
    const row = block(CSS, '\n.machineTabs:global(.cds--tabs) {');
    expect(row).toMatch(/max-inline-size:\s*100%;/);
    expect(row).toMatch(/min-inline-size:\s*0;/);
    expect(row).toMatch(/position:\s*sticky;/);
    expect(row).toMatch(/inset-block-start:\s*0;/);
    const list = block(row, ':global(.cds--tab--list) {');
    expect(list).toMatch(/flex-wrap:\s*nowrap;/);
    expect(list).toMatch(/overflow-x:\s*auto;/);
    expect(list).toMatch(/min-inline-size:\s*0;/);
    // Nothing in a tab holds a width a phone cannot give, and a long label ends in an ellipsis.
    const name = block(CSS, '\n.machineTabName {');
    expect(name).toMatch(/text-overflow:\s*ellipsis;/);
    expect(name).toMatch(/overflow:\s*hidden;/);
    expect(name).toMatch(/max-inline-size:\s*12rem;/);
    expect(block(CSS, '\n.machineTab {')).toMatch(/max-inline-size:\s*18rem;/);
    for (const opener of ['\n.machineTabs:global(.cds--tabs) {', '\n.machineTab {', '\n.machineTabName {', '\n.machineTabCounts {']) {
      expect(block(CSS, opener)).not.toMatch(/(?<!max-)inline-size:\s*(?!\s|0;|100%;)[^;]+;|flex-wrap:\s*wrap;/);
    }
    expect(block(block(CSS, '@media (pointer: coarse)'), '.machineTabs :global(.cds--tabs__nav-item) {')).toMatch(/min-block-size:\s*\$spacing-09;/);
  });
});
