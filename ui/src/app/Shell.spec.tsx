// Copied from Northstar ui/src/app/Shell.spec.tsx at e9c1589.
// Changed: the router's shell has no sign-in and no log-out to test, so those cases are gone; the topbar, the rail and the phone drawer are tested instead. The narrow-width margin case is Northstar's, kept with its stylesheet reader.
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { ProjectRow } from '~/data/router';
import { json, refusal, renderScreen, stubFetch, stubNarrow, unreachable, type Answers } from '~/test-utils';
import { Shell } from './Shell';

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
const PROJECTS = [row('/Users/sam/code/agent-router', 900_000), row('/Users/sam/code/northstar', 400_000), row(null, 2_000)];

/// The rail's links in order, by name, and where each goes.
const RAIL: [string, string][] = [
  ['Overview', '/'],
  ['Sessions', '/live'],
  ['agent-router', '/?project=%2FUsers%2Fsam%2Fcode%2Fagent-router'],
  ['northstar', '/?project=%2FUsers%2Fsam%2Fcode%2Fnorthstar'],
  ['(unresolved)', '/?project=(unresolved)'],
  ['Settings', '/settings'],
];

let answers: Record<string, () => Response | Promise<Response>>;
const answer = (more: Answers = {}) => stubFetch((answers = { '/_router/health': () => json(HEALTH), '/api/projects': () => json(PROJECTS), ...more }));
const show = (at = '/') => renderScreen(<Shell />, { path: '*', at });
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

beforeEach(() => {
  answer();
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('Shell — the topbar', () => {
  it('holds the menu button, the freshness badge, the search action and the theme switch', async () => {
    show();
    const topbar = screen.getByRole('banner', { name: 'Agent Router' });
    expect(within(topbar).getByRole('button', { name: 'Collapse the rail' })).toBeInTheDocument();
    expect(within(topbar).getByRole('status', { name: /^Freshness: / })).toBeInTheDocument();
    expect(within(topbar).getByRole('button', { name: 'Search and commands (⌘K)' })).toBeInTheDocument();
    expect(within(topbar).getByRole('button', { name: /^Theme: system/ })).toBeInTheDocument();
    // The name is the way home, as a real link.
    expect(within(topbar).getByRole('link', { name: 'Agent Router' })).toHaveAttribute('href', '/');
    // No way in and no way out: a LAN browser meets the router's password as HTTP Basic.
    expect(within(topbar).queryByRole('button', { name: /sign in|log out/i })).toBeNull();
  });

  /// The badge is the router's health check, read every 5 s: "connecting" until it first
  /// answers, "live" while it answers, "unreachable" once it fails, and back when it answers.
  it('says live while the health check answers, unreachable once it fails, and live again', async () => {
    const { client } = show();
    const badge = within(screen.getByRole('banner')).getByRole('status');
    expect(badge).toHaveAttribute('data-live-mode', 'unknown');
    expect(badge).toHaveTextContent('connecting');

    await waitFor(() => expect(badge).toHaveAttribute('data-live-mode', 'live'));
    expect(badge).toHaveTextContent('live');
    expect(badge).toHaveAccessibleName(/^Freshness: live — the router answers its health check, asked every 5s$/);

    answers['/_router/health'] = unreachable;
    await act(() => client.refetchQueries({ queryKey: ['health'] }));
    await waitFor(() => expect(badge).toHaveAttribute('data-live-mode', 'unreachable'));
    expect(badge).toHaveTextContent('unreachable');

    answers['/_router/health'] = () => json(HEALTH);
    await act(() => client.refetchQueries({ queryKey: ['health'] }));
    await waitFor(() => expect(badge).toHaveAttribute('data-live-mode', 'live'));
  });

  it('reads a refusal from the health check as unreachable, never as live', async () => {
    answer({ '/_router/health': () => refusal('agent-router: Host header is not an address we serve', 403) });
    show();
    const badge = within(screen.getByRole('banner')).getByRole('status');
    await waitFor(() => expect(badge).toHaveAttribute('data-live-mode', 'unreachable'));
  });

  it('opens the palette from the search action and from ⌘K, and it names the item that builds it', async () => {
    show();
    fireEvent.click(screen.getByRole('button', { name: 'Search and commands (⌘K)' }));
    const palette = await screen.findByRole('dialog');
    expect(palette).toHaveTextContent('AGEN-0007');
    fireEvent.click(within(palette).getByRole('button', { name: /close/i }));
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());

    fireEvent.keyDown(document, { key: 'k', metaKey: true });
    expect(await screen.findByRole('dialog')).toHaveTextContent('AGEN-0007');
    fireEvent.keyDown(screen.getByRole('dialog'), { key: 'Escape' });
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());

    fireEvent.keyDown(document, { key: 'k', ctrlKey: true });
    expect(await screen.findByRole('dialog')).toBeInTheDocument();
  });

  it('cycles the theme system, light, dark and back, on the page and in this browser', () => {
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
});

describe('Shell — the rail', () => {
  it('lists Overview, Sessions, each project, and Settings at its foot', async () => {
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

    show('/?project=%2FUsers%2Fsam%2Fcode%2Fagent-router');
    await projectsRead();
    expect(within(rail()).getByRole('link', { name: 'agent-router' })).toHaveAttribute('aria-current', 'page');
    expect(within(rail()).getByRole('link', { name: 'Overview' })).not.toHaveAttribute('aria-current');
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
  it('says when there are no projects, and when they could not be read', async () => {
    answer({ '/api/projects': () => json([]) });
    const first = show();
    expect(await within(rail()).findByText('No projects yet')).toBeInTheDocument();
    first.unmount();

    answer({ '/api/projects': unreachable });
    show();
    expect(await within(rail()).findByText('Could not read the projects')).toBeInTheDocument();
    expect(within(rail()).queryByText('No projects yet')).toBeNull();
  });

  it('collapses on a desk and remembers it in this browser', () => {
    show();
    fireEvent.click(screen.getByRole('button', { name: 'Collapse the rail' }));
    expect(screen.getByRole('button', { name: 'Expand the rail' })).toBeInTheDocument();
    expect(sideNav()).not.toHaveClass('cds--side-nav--expanded');
    expect(localStorage.getItem('agent-router.rail')).toBe('collapsed');
  });
});

describe('Shell — the drawer in a hand', () => {
  beforeEach(() => {
    stubNarrow();
  });

  it('opens the same links in a drawer from the menu button', async () => {
    show();
    const menu = await screen.findByRole('button', { name: 'Open the menu' });
    expect(sideNav()).not.toHaveClass('cds--side-nav--expanded');

    fireEvent.click(menu);
    expect(sideNav()).toHaveClass('cds--side-nav--expanded');
    expect(screen.getByRole('button', { name: 'Close the menu' })).toBeInTheDocument();
    await projectsRead();
    expect(railLinks()).toEqual(RAIL);
  });

  it('closes on Escape', async () => {
    show();
    fireEvent.click(await screen.findByRole('button', { name: 'Open the menu' }));
    expect(sideNav()).toHaveClass('cds--side-nav--expanded');

    fireEvent.keyDown(document, { key: 'Escape' });
    expect(sideNav()).not.toHaveClass('cds--side-nav--expanded');
    expect(screen.getByRole('button', { name: 'Open the menu' })).toBeInTheDocument();
  });

  it('closes on arriving: at another screen, at a project, which changes only the query, and at the page it was opened on', async () => {
    show();
    await projectsRead();
    const location = () => screen.getByTestId('location');

    fireEvent.click(screen.getByRole('button', { name: 'Open the menu' }));
    fireEvent.click(within(rail()).getByRole('link', { name: 'Sessions' }));
    expect(location()).toHaveTextContent(/^\/live$/);
    expect(sideNav()).not.toHaveClass('cds--side-nav--expanded');

    fireEvent.click(screen.getByRole('button', { name: 'Open the menu' }));
    fireEvent.click(within(rail()).getByRole('link', { name: 'northstar' }));
    expect(location()).toHaveTextContent('/?project=%2FUsers%2Fsam%2Fcode%2Fnorthstar');
    expect(sideNav()).not.toHaveClass('cds--side-nav--expanded');

    fireEvent.click(screen.getByRole('button', { name: 'Open the menu' }));
    fireEvent.click(within(rail()).getByRole('link', { name: 'northstar' }));
    expect(sideNav()).not.toHaveClass('cds--side-nav--expanded');
  });

  it('closes when the overlay beside it is pressed', async () => {
    show();
    fireEvent.click(await screen.findByRole('button', { name: 'Open the menu' }));
    const overlay = document.querySelector('.cds--side-nav__overlay');
    expect(overlay).not.toBeNull();
    fireEvent.click(overlay!);
    expect(sideNav()).not.toHaveClass('cds--side-nav--expanded');
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

    const scss = readFileSync(join(__dirname, 'Shell.module.scss'), 'utf8').replace(/\/\/.*$/gm, '');
    const content = block(scss, '\n.content {');
    const narrow = block(content, "@include breakpoint-down('md')");
    expect(narrow).toMatch(/&:global\(#main-content\)\s*\{\s*margin-inline-start:\s*0;\s*\}/);
    // On a desk the margin is Carbon's: 3rem beside the rail, 16rem beside it open.
    expect(content.replace(narrow, '')).not.toMatch(/margin/);
  });
});
