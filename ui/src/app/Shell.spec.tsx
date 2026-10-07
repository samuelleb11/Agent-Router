// Copied from Northstar ui/src/app/Shell.spec.tsx at e9c1589.
// Changed: the router's shell has no sign-in and no log-out to test, so those cases are gone; the topbar, the rail and the phone drawer are tested instead, each case under the Shell contract it holds (docs/web-console-contracts.md, "Shell"). The narrow-width margin case is Northstar's, kept with its stylesheet reader.
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { act, configure, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { ProjectRow } from '~/data/router';
import { LiveBadge } from '~/components/LiveBadge';
import { flush, json, refusal, renderScreen, stubFetch, stubNarrow, unreachable, type Answers } from '~/test-utils';
import { LIVE_WORD, UNREACHABLE_AFTER, lastGood, sentence, useStreamReport, type LiveMode } from './live';
import { Shell } from './Shell';

/// Waits and limits generous enough for a machine busy with other work: a timeout here is a slow
/// machine, never a pass.
vi.setConfig({ testTimeout: 30_000 });
configure({ asyncUtilTimeout: 10_000 });

const HEALTH = { ok: true, pid: 4242, version: '0.9.0', build: 'f00dfeed' };

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
  first_seen: 1_759_600_000_000,
  last_seen: 1_759_680_000_000,
});

/// The router's own order, most prompt tokens first, and a session whose directory was never
/// resolved: the dashboard lists that one as "(unresolved)", and so does the rail.
const PROJECTS = [row('/Users/sample/code/agent-router', 900_000), row('/Users/sample/code/northstar', 400_000), row(null, 2_000)];

/// The rail's links in order, by name, and where each goes.
const RAIL: [string, string][] = [
  ['Overview', '/'],
  ['Sessions', '/live'],
  ['agent-router', '/?project=%2FUsers%2Fsample%2Fcode%2Fagent-router'],
  ['northstar', '/?project=%2FUsers%2Fsample%2Fcode%2Fnorthstar'],
  ['(unresolved)', '/?project=(unresolved)'],
  ['Settings', '/settings'],
];

let answers: Record<string, () => Response | Promise<Response>>;
const answer = (more: Answers = {}) => stubFetch((answers = { '/_router/health': () => json(HEALTH), '/api/projects': () => json(PROJECTS), ...more }));
const show = (at = '/') => renderScreen(<Shell />, { path: '*', at });
const topbar = () => screen.getByRole('banner', { name: 'Agent Router' });
const badge = () => within(topbar()).getByRole('status');
const rail = () => screen.getByRole('navigation', { name: 'Console' });
const links = () => within(rail()).getAllByRole('link');
/// Each rail link as a reader meets it: its label, which is also its accessible name (a
/// project's initials are hidden from a screen reader), and where it goes.
const railLinks = () =>
  links().map((a) => {
    const label = a.querySelector('.cds--side-nav__link-text')?.textContent ?? '';
    expect(a).toHaveAccessibleName(label);
    return [label, a.getAttribute('href')];
  });
const sideNav = () => document.querySelector('.cds--side-nav');
const projectsRead = () => within(rail()).findByRole('link', { name: 'agent-router' });

/// Storage that refuses every read and write, as a private window or a frame with storage off.
function refuseStorage(): () => void {
  const was = Object.getOwnPropertyDescriptor(window, 'localStorage')!;
  const refusing = {
    getItem: () => {
      throw new DOMException('The operation is insecure.', 'SecurityError');
    },
    setItem: () => {
      throw new DOMException('The operation is insecure.', 'SecurityError');
    },
    removeItem: () => {
      throw new DOMException('The operation is insecure.', 'SecurityError');
    },
  };
  for (const target of new Set<object>([globalThis, window])) Object.defineProperty(target, 'localStorage', { value: refusing, configurable: true, writable: true });
  return () => {
    for (const target of new Set<object>([globalThis, window])) Object.defineProperty(target, 'localStorage', was);
  };
}

beforeEach(() => {
  answer();
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('Shell — the topbar', () => {
  it('SH-C1: holds, in this order, the menu button, the name, the freshness badge, the search action and the theme switch, g100 in either page theme, and no way to sign in or out', async () => {
    show();
    const bar = topbar();
    const parts = [
      within(bar).getByRole('button', { name: 'Collapse the rail' }),
      within(bar).getByRole('link', { name: 'Agent Router' }),
      within(bar).getByRole('status', { name: /^Freshness: / }),
      within(bar).getByRole('button', { name: 'Search and commands (⌘K)' }),
      within(bar).getByRole('button', { name: /^Theme: system/ }),
    ];
    for (let i = 1; i < parts.length; i++) expect(parts[i - 1]!.compareDocumentPosition(parts[i]!) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    // The name is the way home, as a real link.
    expect(parts[1]).toHaveAttribute('href', '/');
    // No way in and no way out: a LAN browser meets the router's password as HTTP Basic.
    expect(within(bar).queryByRole('button', { name: /sign in|log ?in|log ?out|sign out/i })).toBeNull();
    expect(within(bar).queryByRole('link', { name: /sign in|log ?in|log ?out|sign out/i })).toBeNull();

    // The topbar is its own g100 region inside a page that wears white, and stays one when the
    // page turns dark.
    expect(bar.closest('.cds--g100')).not.toBeNull();
    expect(bar.closest('.cds--g100')!.closest('.cds--white')).not.toBeNull();
    fireEvent.click(within(bar).getByRole('button', { name: /^Theme: system/ }));
    fireEvent.click(within(bar).getByRole('button', { name: /^Theme: light/ }));
    expect(document.documentElement).toHaveClass('ar-dark');
    expect(topbar().closest('.cds--g100')).not.toBeNull();
    // Back to following the system: the theme is one state for the whole page, this spec's included.
    fireEvent.click(within(bar).getByRole('button', { name: /^Theme: dark/ }));
  });

  it('SH-C9: says, beside the name, the address the console was reached at, at both widths', async () => {
    const desk = show();
    expect(within(topbar()).getByText(window.location.host)).toHaveAttribute('data-host');
    desk.unmount();

    stubNarrow();
    show();
    expect(await within(topbar()).findByRole('button', { name: 'Open the menu' })).toBeInTheDocument();
    expect(within(topbar()).getByText(window.location.host)).toBeVisible();
    // Beside the name, and not inside its link: the link is still called by the name alone.
    expect(within(topbar()).getByRole('link', { name: 'Agent Router' })).not.toHaveTextContent(window.location.host);
  });

  it('SH-C10: has its own word and its own tone for each of five states, the whole sentence as its accessible name, announced as a status and never an alert', () => {
    const at = Date.UTC(2026, 9, 5, 14, 2, 11);
    const cases: [LiveMode, number, number | null][] = [
      ['connecting', 0, null],
      ['live', 0, at],
      ['stale', 1, at],
      ['unreachable', UNREACHABLE_AFTER, at],
      ['reconnecting', 0, at],
    ];
    const seen = cases.map(([mode, strikes, lastOk]) => {
      const label = sentence(mode, { strikes, lastOk });
      const { unmount } = render(<LiveBadge mode={mode} label={label} />);
      const tag = screen.getByRole('status');
      expect(screen.queryByRole('alert')).toBeNull();
      expect(tag).toHaveAccessibleName(`Freshness: ${label}`);
      expect(tag).toHaveTextContent(LIVE_WORD[mode]);
      expect(label.startsWith(`${LIVE_WORD[mode]} — `)).toBe(true);
      const tone = [...tag.classList].filter((c) => /^cds--tag--(?!sm$)/.test(c) || !c.startsWith('cds--')).sort().join(' ');
      unmount();
      return { mode, word: LIVE_WORD[mode], tone, label };
    });
    expect(new Set(seen.map((s) => s.word)).size).toBe(5);
    expect(new Set(seen.map((s) => s.tone)).size).toBe(5);
    // A state still finding out is neutral, and never wears the live colour.
    expect(seen.find((s) => s.mode === 'connecting')!.tone).toBe('cds--tag--gray');
    expect(seen.find((s) => s.mode === 'live')!.tone).toBe('cds--tag--green');
    // Stale names the time of the last good read, or says there never was one.
    expect(seen.find((s) => s.mode === 'stale')!.label).toContain(lastGood(at));
    expect(lastGood(at)).toMatch(/^last good read at .*\d{1,2}.\d{2}.\d{2}/);
    expect(sentence('stale', { strikes: 1, lastOk: null })).toContain('never read successfully');
    expect(sentence('unreachable', { strikes: 5, lastOk: null })).toContain('never read successfully');
  });

  it('SH-C10, SH-C11: connecting until the health check answers, live while it does, stale with the last good read’s time after a failed cycle, unreachable only after five in a row, and live again after one good one', async () => {
    const { client } = show();
    expect(badge()).toHaveAttribute('data-live-mode', 'connecting');
    expect(badge()).toHaveTextContent('connecting');

    await waitFor(() => expect(badge()).toHaveAttribute('data-live-mode', 'live'));
    expect(badge()).toHaveTextContent('live');
    expect(badge()).toHaveAccessibleName(/^Freshness: live — the router answers its health check, asked every 5s$/);

    // A cycle is one health read: one failure is a blip, said as stale, with when the last good
    // read was.
    answers['/_router/health'] = unreachable;
    const cycle = () => act(() => client.refetchQueries({ queryKey: ['health'] }));
    await cycle();
    await waitFor(() => expect(badge()).toHaveAttribute('data-live-mode', 'stale'));
    expect(badge()).toHaveTextContent('stale');
    expect(badge()).toHaveAccessibleName(/^Freshness: stale — the last refresh failed; last good read at .+; asking again every 5s$/);
    for (let i = 2; i < UNREACHABLE_AFTER; i++) await cycle();
    await waitFor(() => expect(badge()).toHaveAccessibleName(/the last 4 refreshes failed/));
    expect(badge()).toHaveAttribute('data-live-mode', 'stale');

    // The fifth in a row.
    await cycle();
    await waitFor(() => expect(badge()).toHaveAttribute('data-live-mode', 'unreachable'));
    expect(badge()).toHaveAccessibleName(/^Freshness: unreachable — 5 refreshes in a row failed; last good read at /);

    // One good cycle sets the count back to zero.
    answers['/_router/health'] = () => json(HEALTH);
    await cycle();
    await waitFor(() => expect(badge()).toHaveAttribute('data-live-mode', 'live'));
    answers['/_router/health'] = unreachable;
    await cycle();
    await waitFor(() => expect(badge()).toHaveAttribute('data-live-mode', 'stale'));
    expect(badge()).toHaveAccessibleName(/the last refresh failed/);
  });

  it('SH-C10, SH-C11: a refused health check is a failed cycle, never live, and a page that never had an answer says it was never read, not a time it does not have', async () => {
    answer({ '/_router/health': () => refusal('agent-router: Host header is not an address we serve', 403) });
    const { client } = show();
    await waitFor(() => expect(badge()).toHaveAttribute('data-live-mode', 'stale'));
    expect(badge()).toHaveAccessibleName(/^Freshness: stale — the last refresh failed; never read successfully; /);
    for (let i = 1; i < UNREACHABLE_AFTER; i++) await act(() => client.refetchQueries({ queryKey: ['health'] }));
    await waitFor(() => expect(badge()).toHaveAttribute('data-live-mode', 'unreachable'));
    expect(badge()).toHaveAccessibleName(/never read successfully/);
  });

  it('SH-C10: says reconnecting while the open session’s stream is being replaced, in its own tone, and lets it go when the session closes; five failed refreshes still say unreachable', async () => {
    function Stream({ phase }: { phase: 'live' | 'reconnecting' }) {
      useStreamReport(phase);
      return null;
    }
    const { client } = show();
    await waitFor(() => expect(badge()).toHaveAttribute('data-live-mode', 'live'));
    const reporter = render(<Stream phase="reconnecting" />);
    await waitFor(() => expect(badge()).toHaveAttribute('data-live-mode', 'reconnecting'));
    expect(badge()).toHaveTextContent('reconnecting');
    expect(badge()).toHaveClass('cds--tag--magenta');
    expect(badge()).toHaveAccessibleName(/^Freshness: reconnecting — the open session’s stream dropped and is being replaced$/);
    reporter.rerender(<Stream phase="live" />);
    await waitFor(() => expect(badge()).toHaveAttribute('data-live-mode', 'live'));
    reporter.rerender(<Stream phase="reconnecting" />);
    await waitFor(() => expect(badge()).toHaveAttribute('data-live-mode', 'reconnecting'));

    answers['/_router/health'] = unreachable;
    for (let i = 0; i < UNREACHABLE_AFTER; i++) await act(() => client.refetchQueries({ queryKey: ['health'] }));
    await waitFor(() => expect(badge()).toHaveAttribute('data-live-mode', 'unreachable'));
    answers['/_router/health'] = () => json(HEALTH);
    await act(() => client.refetchQueries({ queryKey: ['health'] }));
    await waitFor(() => expect(badge()).toHaveAttribute('data-live-mode', 'reconnecting'));

    // The session closes: its stream is no longer anything the badge speaks for.
    reporter.unmount();
    await waitFor(() => expect(badge()).toHaveAttribute('data-live-mode', 'live'));
  });

  it('SH-C14: a failed refresh blanks nothing: the rail keeps the projects it read, and the badge says how old the reading is', async () => {
    const { client } = show();
    await projectsRead();
    await waitFor(() => expect(badge()).toHaveAttribute('data-live-mode', 'live'));
    answers['/api/projects'] = unreachable;
    answers['/_router/health'] = unreachable;
    await act(() => client.refetchQueries());
    await waitFor(() => expect(badge()).toHaveAttribute('data-live-mode', 'stale'));
    expect(badge()).toHaveAccessibleName(/last good read at /);
    expect(railLinks()).toEqual(RAIL);
    expect(within(rail()).queryByText('Could not read the projects')).toBeNull();
  });

  it('opens the palette from the search action and from ⌘K, its search and commands field ready', async () => {
    show();
    fireEvent.click(screen.getByRole('button', { name: 'Search and commands (⌘K)' }));
    const palette = await screen.findByRole('dialog');
    // The field arrives with the palette's own chunk, the first time it opens.
    expect(await within(palette).findByRole('searchbox', { name: 'Search and commands' })).toBeInTheDocument();
    fireEvent.click(within(palette).getByRole('button', { name: /close/i }));
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());

    fireEvent.keyDown(document, { key: 'k', metaKey: true });
    expect(await within(await screen.findByRole('dialog')).findByRole('searchbox', { name: 'Search and commands' })).toBeInTheDocument();
    fireEvent.keyDown(screen.getByRole('dialog'), { key: 'Escape' });
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());

    fireEvent.keyDown(document, { key: 'k', ctrlKey: true });
    expect(await screen.findByRole('dialog')).toBeInTheDocument();
  });

  it('SH-C35: one topbar button cycles the theme system, light, dark and back, on the page and in this browser', () => {
    show();
    const html = document.documentElement;
    fireEvent.click(screen.getByRole('button', { name: /^Theme: system/ }));
    expect(screen.getByRole('button', { name: /^Theme: light/ })).toBeInTheDocument();
    expect(html).toHaveClass('ar-light');
    expect(localStorage.getItem('agent-router.theme')).toBe('light');

    fireEvent.click(screen.getByRole('button', { name: /^Theme: light/ }));
    expect(html).toHaveClass('ar-dark');
    expect(html).not.toHaveClass('ar-light');
    expect(localStorage.getItem('agent-router.theme')).toBe('dark');

    fireEvent.click(screen.getByRole('button', { name: /^Theme: dark/ }));
    expect(screen.getByRole('button', { name: /^Theme: system/ })).toBeInTheDocument();
    expect(localStorage.getItem('agent-router.theme')).toBeNull();
  });

  it('SH-C35: a browser that will not store the theme still switches it, and only forgets it', () => {
    const restore = refuseStorage();
    try {
      show();
      fireEvent.click(screen.getByRole('button', { name: /^Theme: system/ }));
      expect(document.documentElement).toHaveClass('ar-light');
      fireEvent.click(screen.getByRole('button', { name: /^Theme: light/ }));
      expect(document.documentElement).toHaveClass('ar-dark');
      fireEvent.click(screen.getByRole('button', { name: /^Theme: dark/ }));
      expect(screen.getByRole('button', { name: /^Theme: system/ })).toBeInTheDocument();
    } finally {
      restore();
    }
  });
});

describe('Shell — the rail', () => {
  it('SH-C3: lists Overview, Sessions, each project opening the Overview scoped to it, and Settings at its foot', async () => {
    show();
    await projectsRead();
    expect(railLinks()).toEqual(RAIL);
    expect(within(rail()).getByText('Projects')).toBeInTheDocument();

    // Settings is the whole of the second list, after the divider, which Carbon's layout pins
    // to the foot: the projects grow above it and never push it out of reach.
    const lists = rail().querySelectorAll('ul');
    const foot = lists[lists.length - 1]!;
    expect(within(foot).getAllByRole('link').map((a) => a.textContent)).toEqual(['Settings']);
    expect(within(lists[0]!).queryByRole('link', { name: 'Settings' })).toBeNull();
    expect(foot.previousElementSibling).toHaveClass('cds--side-nav__divider');
  });

  it('marks where the reader is: the Overview unscoped, and a project when the Overview is scoped to it', async () => {
    const first = show('/');
    await projectsRead();
    expect(within(rail()).getByRole('link', { name: 'Overview' })).toHaveAttribute('aria-current', 'page');
    expect(within(rail()).getByRole('link', { name: 'agent-router' })).not.toHaveAttribute('aria-current');
    first.unmount();

    show('/?project=%2FUsers%2Fsample%2Fcode%2Fagent-router');
    await projectsRead();
    expect(within(rail()).getByRole('link', { name: 'agent-router' })).toHaveAttribute('aria-current', 'page');
    expect(within(rail()).getByRole('link', { name: 'Overview' })).not.toHaveAttribute('aria-current');
  });

  /// A local router takes no machines, so the rail offers the screen only where there are some to
  /// connect, and not before the router has said which kind it is.
  it('MA-C1: on a hosted router lists Machines right after Sessions, at both widths, marked on its screen; a local router, and one that has not said which kind it is, list no Machines', async () => {
    const withMachines = [RAIL[0], RAIL[1], ['Machines', '/machines'], ...RAIL.slice(2)];
    answer({ '/api/auth': () => json({ mode: 'server' }) });
    const hosted = show('/machines');
    const machines = await within(rail()).findByRole('link', { name: 'Machines' });
    await projectsRead();
    expect(railLinks()).toEqual(withMachines);
    expect(machines).toHaveAttribute('aria-current', 'page');
    expect(within(rail()).getByRole('link', { name: 'Sessions' })).not.toHaveAttribute('aria-current');
    hosted.unmount();

    stubNarrow();
    const hand = show();
    fireEvent.click(await screen.findByRole('button', { name: 'Open the menu' }));
    await within(rail()).findByRole('link', { name: 'Machines' });
    await projectsRead();
    expect(railLinks()).toEqual(withMachines);
    hand.unmount();
    vi.unstubAllGlobals();

    for (const [who, auth] of [
      ['a local router', () => json({ mode: 'local' })],
      ['a local router that predates the field', () => json({ configured: true })],
      ['a router whose answer failed', unreachable],
    ] as const) {
      const fetch = answer({ '/api/auth': auth });
      const page = show();
      await projectsRead();
      await waitFor(() => expect(fetch.mock.calls.some(([url]) => String(url) === '/api/auth'), who).toBe(true));
      await flush();
      expect(railLinks(), who).toEqual(RAIL);
      page.unmount();
    }

    // Not yet answered: nothing is offered that a local router would not have.
    answer({ '/api/auth': () => new Promise<Response>(() => {}) });
    show();
    await projectsRead();
    expect(railLinks()).toEqual(RAIL);
  });

  it('marks Settings on any of its pages, and Sessions on the live view', async () => {
    const first = show('/settings/network');
    expect(within(rail()).getByRole('link', { name: 'Settings' })).toHaveAttribute('aria-current', 'page');
    first.unmount();
    show('/live');
    expect(within(rail()).getByRole('link', { name: 'Sessions' })).toHaveAttribute('aria-current', 'page');
  });

  /// An empty list and a list that could not be read are different facts: a rail that drew
  /// nothing for both would hide which one it is.
  it('SH-C22: says when there are no projects, and when they could not be read, in two different sentences', async () => {
    answer({ '/api/projects': () => json([]) });
    const first = show();
    expect(await within(rail()).findByText('No projects yet')).toBeInTheDocument();
    expect(within(rail()).queryByText('Could not read the projects')).toBeNull();
    first.unmount();

    answer({ '/api/projects': unreachable });
    show();
    expect(await within(rail()).findByText('Could not read the projects')).toBeInTheDocument();
    expect(within(rail()).queryByText('No projects yet')).toBeNull();
  });

  it('SH-C4: collapses on a desk and remembers it in this browser', () => {
    const first = show();
    fireEvent.click(screen.getByRole('button', { name: 'Collapse the rail' }));
    expect(screen.getByRole('button', { name: 'Expand the rail' })).toBeInTheDocument();
    expect(sideNav()).not.toHaveClass('cds--side-nav--expanded');
    expect(localStorage.getItem('agent-router.rail')).toBe('collapsed');
    first.unmount();

    // A new page in the same browser starts the way it was left.
    show();
    expect(screen.getByRole('button', { name: 'Expand the rail' })).toBeInTheDocument();
    expect(sideNav()).not.toHaveClass('cds--side-nav--expanded');
  });

  it('SH-C4: a browser that will not store the rail’s state draws it open and still collapses it, reading and writing inside try/catch', () => {
    const restore = refuseStorage();
    try {
      show();
      expect(screen.getByRole('button', { name: 'Collapse the rail' })).toBeInTheDocument();
      expect(sideNav()).toHaveClass('cds--side-nav--expanded');
      fireEvent.click(screen.getByRole('button', { name: 'Collapse the rail' }));
      expect(sideNav()).not.toHaveClass('cds--side-nav--expanded');
      fireEvent.click(screen.getByRole('button', { name: 'Expand the rail' }));
      expect(sideNav()).toHaveClass('cds--side-nav--expanded');
    } finally {
      restore();
    }
  });
});

describe('Shell — the drawer in a hand', () => {
  beforeEach(() => {
    stubNarrow();
  });

  it('SH-C3: opens the same links, in the same order, in a drawer from the menu button', async () => {
    show();
    const menu = await screen.findByRole('button', { name: 'Open the menu' });
    expect(sideNav()).not.toHaveClass('cds--side-nav--expanded');

    fireEvent.click(menu);
    expect(sideNav()).toHaveClass('cds--side-nav--expanded');
    expect(screen.getByRole('button', { name: 'Close the menu' })).toBeInTheDocument();
    await projectsRead();
    expect(railLinks()).toEqual(RAIL);
  });

  it('SH-C4: closes on Escape', async () => {
    show();
    fireEvent.click(await screen.findByRole('button', { name: 'Open the menu' }));
    expect(sideNav()).toHaveClass('cds--side-nav--expanded');

    fireEvent.keyDown(document, { key: 'Escape' });
    expect(sideNav()).not.toHaveClass('cds--side-nav--expanded');
    expect(screen.getByRole('button', { name: 'Open the menu' })).toBeInTheDocument();
  });

  it('SH-C4: closes on arriving: at another screen, at a project, which changes only the query, and at the page it was opened on', async () => {
    show();
    await projectsRead();
    const location = () => screen.getByTestId('location');

    fireEvent.click(screen.getByRole('button', { name: 'Open the menu' }));
    fireEvent.click(within(rail()).getByRole('link', { name: 'Sessions' }));
    expect(location()).toHaveTextContent(/^\/live$/);
    expect(sideNav()).not.toHaveClass('cds--side-nav--expanded');

    fireEvent.click(screen.getByRole('button', { name: 'Open the menu' }));
    fireEvent.click(within(rail()).getByRole('link', { name: 'northstar' }));
    expect(location()).toHaveTextContent('/?project=%2FUsers%2Fsample%2Fcode%2Fnorthstar');
    expect(sideNav()).not.toHaveClass('cds--side-nav--expanded');

    fireEvent.click(screen.getByRole('button', { name: 'Open the menu' }));
    fireEvent.click(within(rail()).getByRole('link', { name: 'northstar' }));
    expect(sideNav()).not.toHaveClass('cds--side-nav--expanded');
  });

  it('SH-C4: closes when the overlay beside it is pressed', async () => {
    show();
    fireEvent.click(await screen.findByRole('button', { name: 'Open the menu' }));
    const overlay = document.querySelector('.cds--side-nav__overlay');
    expect(overlay).not.toBeNull();
    fireEvent.click(overlay!);
    expect(sideNav()).not.toHaveClass('cds--side-nav--expanded');
  });

  it('SH-C4: is never remembered: opening it writes nothing to this browser, and a new page starts with it shut', async () => {
    const first = show();
    fireEvent.click(await screen.findByRole('button', { name: 'Open the menu' }));
    expect(sideNav()).toHaveClass('cds--side-nav--expanded');
    expect(localStorage.length).toBe(0);
    first.unmount();

    show();
    expect(await screen.findByRole('button', { name: 'Open the menu' })).toBeInTheDocument();
    expect(sideNav()).not.toHaveClass('cds--side-nav--expanded');
  });
});

describe('Shell — Overview and Sessions from anywhere', () => {
  /// Wherever the reader is — the Overview, a project, the list, an open session, Find, a Settings
  /// page, an address that is nothing — the shell's navigation is outside the screen, so no screen
  /// can hide it: one press on a desk, and on a phone the same links in the drawer the menu button
  /// opens, with the name in the topbar going home in one.
  const PLACES = ['/', '/?project=%2FUsers%2Fsample%2Fcode%2Fnorthstar', '/live', '/live?session=aaaaaaaa-1111-2222-3333-444444444444', '/live?find=kestrel&scope=all', '/settings/accounts', '/no/such/page'];

  it('SH-C5: Overview and Sessions are each one press away in the rail on every screen, an open session and Find included', async () => {
    for (const at of PLACES) {
      const page = show(at);
      expect(within(rail()).getByRole('link', { name: 'Overview' }), at).toHaveAttribute('href', '/');
      expect(within(rail()).getByRole('link', { name: 'Sessions' }), at).toHaveAttribute('href', '/live');
      expect(within(rail()).getByRole('link', { name: 'Overview' })).toBeVisible();
      page.unmount();
    }
  });

  it('SH-C5: in a hand, the topbar’s name goes to the Overview, and the drawer holds Overview and Sessions on every screen', async () => {
    stubNarrow();
    for (const at of PLACES) {
      const page = show(at);
      expect(within(topbar()).getByRole('link', { name: 'Agent Router' }), at).toHaveAttribute('href', '/');
      fireEvent.click(await screen.findByRole('button', { name: 'Open the menu' }));
      expect(sideNav(), at).toHaveClass('cds--side-nav--expanded');
      expect(within(rail()).getByRole('link', { name: 'Overview' }), at).toHaveAttribute('href', '/');
      expect(within(rail()).getByRole('link', { name: 'Sessions' }), at).toHaveAttribute('href', '/live');
      page.unmount();
    }
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

const plain = (scss: string) => scss.replace(/\/\/.*$/gm, '');
const SHELL_SCSS = plain(readFileSync(join(__dirname, 'Shell.module.scss'), 'utf8'));
const STYLES_SCSS = plain(readFileSync(join(__dirname, '..', 'styles.scss'), 'utf8'));

/// Below 672px the rail is a drawer over the page, and Carbon still reserves the rail's 3rem
/// beside it, off `.cds--side-nav ~ .cds--content`: a dead strip down the left of every page
/// in a hand. The stylesheet takes it back by the Content's id, whose weight outranks
/// Carbon's classes. A CSS module renames every id and class it is not told is global, so
/// written `&#main-content` the rule compiled to `._content_…#_main-content_…`, which nothing
/// carries, and the strip stayed. jsdom has no layout, so the rule is read from the
/// stylesheet, and the page is held to the id it names.
describe('Shell — the page in a hand', () => {
  it('gives the page the whole width below 672px, and leaves the rail its room on a desk', async () => {
    stubNarrow();
    show();
    // The drawer, not the rail: what the narrow rule is for.
    expect(await screen.findByRole('button', { name: 'Open the menu' })).toBeInTheDocument();
    expect(screen.getByRole('main')).toHaveAttribute('id', 'main-content');

    const content = block(SHELL_SCSS, '\n.content {');
    const narrow = block(content, "@include breakpoint-down('md')");
    expect(narrow).toMatch(/&:global\(#main-content\)\s*\{\s*margin-inline-start:\s*0;\s*\}/);
    // On a desk the inline margin is Carbon's: 3rem beside the rail, 16rem beside it open.
    expect(content.replace(narrow, '')).not.toMatch(/margin-inline/);
  });

  /// The header is fixed, so nothing in the flow of the page knows how tall it is: everything that
  /// sits under it is told, from one value, the header's own height. jsdom lays nothing out, so the
  /// rules are read from the stylesheets; scripts/shell-checks.mjs measures them in a browser under
  /// a notch.
  it('SH-C2: the fixed header pads the top safe area, its bar is bounded by it, and the page, the rail and its overlay start where it ends', () => {
    // One value: Carbon's 48 px bar under the notch's safe area.
    expect(block(STYLES_SCSS, '\n:root {')).toMatch(/--ar-shell-header:\s*calc\(#\{\$spacing-09\}\s*\+\s*env\(safe-area-inset-top, 0px\)\);/);
    const header = block(SHELL_SCSS, '\n.header {');
    const fixed = block(header, '&:global(.cds--header) {');
    expect(fixed).toMatch(/block-size:\s*var\(--ar-shell-header\);/);
    expect(fixed).toMatch(/padding-block-start:\s*env\(safe-area-inset-top, 0px\);/);
    expect(block(header, ':global(.cds--header__global) {')).toMatch(/max-inline-size:\s*100%;[^}]*min-inline-size:\s*0;/);
    // The page clears it by padding, never a margin that collapses through #root.
    const page = block(block(SHELL_SCSS, '\n.content {'), '&:global(#main-content) {');
    expect(page).toMatch(/margin-block-start:\s*0;/);
    expect(page).toMatch(/padding-block-start:\s*var\(--ar-shell-header\);/);
    expect(block(block(SHELL_SCSS, '\n.rail {'), '&:global(.cds--side-nav) {')).toMatch(/inset-block-start:\s*var\(--ar-shell-header\);/);
    expect(block(SHELL_SCSS, ':global(.cds--side-nav__overlay.cds--side-nav__overlay) {')).toMatch(/inset-block-start:\s*var\(--ar-shell-header\);/);
  });
});
