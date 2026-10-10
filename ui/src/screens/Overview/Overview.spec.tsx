// New for agent-router: the Overview as a reader meets it (docs/web-console-contracts.md, "Overview"):
// its sections, its address, its search, its quota, its lists and requests, and how it reads the
// router — one snapshot per scope, every five seconds — against a router faked at `fetch`.
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { useNavigate } from 'react-router';
import { keys } from '~/app/query';
import type { AccountSplitRow } from '~/data/overview';
import { identityNote, notes, noteText } from '~/model/settings';
import { renderScreen } from '~/test-utils';
import Overview from './index';
import {
  ACCOUNTS,
  ANTHROPIC_ROOM,
  anthropicBlock,
  dropped,
  forecast,
  forecastAccount,
  held,
  HOSTED,
  KIMI_ROOM,
  kimiAccount,
  kimiBlock,
  LOCAL,
  MIN,
  NOW,
  ok,
  partsOf,
  project,
  PROJECTS,
  refused,
  request,
  REQUESTS,
  router,
  S1,
  S2,
  session,
  summary,
  totals,
  TWO_PROVIDERS,
} from './fixtures';

/// The whole page is drawn in jsdom, beside every other spec in the run: generous limits, so a loaded
/// machine is not mistaken for a broken page. What "at once" means is asserted on the order of events,
/// never on how long they took.
vi.setConfig({ testTimeout: 30_000 });

const chart = vi.hoisted(() => ({ StackedBarChart: vi.fn() }));
vi.mock('@carbon/charts-react', () => ({
  StackedBarChart: (props: unknown) => {
    chart.StackedBarChart(props);
    return <svg data-testid="chart" />;
  },
}));

const scss = readFileSync(join(__dirname, 'Overview.module.scss'), 'utf8');

/// Back and Forward, and a followed link, as a reader has them.
function Nav() {
  const navigate = useNavigate();
  return (
    <>
      <button type="button" onClick={() => void navigate(-1)}>
        test: back
      </button>
      <button type="button" onClick={() => void navigate('/?project=%2Fcode%2Fnorthstar')}>
        test: follow
      </button>
    </>
  );
}

// At every range unless a spec says otherwise: the specs below were written for the unscoped
// page, and the 1h the page starts at (DEFAULT_RANGE) is pinned by OV-C29's own.
const show = (at = '/?range=all') =>
  renderScreen(
    <>
      <Overview />
      <Nav />
    </>,
    { path: '/', at },
  );
const where = () => screen.getByTestId('location').textContent ?? '';
const loaded = () => screen.findByRole('heading', { level: 2, name: 'Totals in scope' }, { timeout: 20_000 });
const panel = (name: string) => screen.getByRole('heading', { level: 2, name }).closest('section') as HTMLElement;
const freshness = () => document.querySelector('[data-freshness]') as HTMLElement;
const tile = (label: string) => within(panel('Totals in scope')).getByText(label).closest('.cds--tile') as HTMLElement;
/// Opens a tab as a reader does, and waits for the panel it draws first.
const tabTo = async (name: string, heading: string) => {
  const tab = screen.getByRole('tab', { name });
  // A press moves the focus to the tab, as a browser's does; jsdom leaves it where it was.
  act(() => tab.focus());
  fireEvent.click(tab);
  return screen.findByRole('heading', { level: 2, name: heading }, { timeout: 20_000 });
};
/// The h2s a reader can see, in the page's order.
const headings = () => screen.getAllByRole('heading', { level: 2 }).map((h) => h.textContent);

beforeEach(() => {
  chart.StackedBarChart.mockClear();
  // The fixtures are built around NOW, read when they loaded; the page reads the clock when it
  // draws. Held at NOW, a countdown reads as the fixtures mean it however long the run took to
  // get here: on a slow runner "resets in 2h 00m" had become 1h 59m (AGEN-0032). Only Date is
  // held; timers stay real, and a spec that fakes them installs its own clock.
  vi.useFakeTimers({ toFake: ['Date'] });
  vi.setSystemTime(NOW);
});
afterEach(() => {
  vi.unstubAllGlobals();
  vi.useRealTimers();
  vi.restoreAllMocks();
});

describe('the Overview’s layout', () => {
  it('OV-C1: Quota headroom, All accounts and Totals in scope sit above the tabs, which keep every other section the dashboard shows in its order, the scope bar first', async () => {
    router();
    show();
    await loaded();
    const top = ['Quota headroom', 'All accounts', 'Totals in scope'];
    expect(within(screen.getByRole('tablist', { name: 'Overview sections' })).getAllByRole('tab').map((t) => t.textContent)).toEqual(['Activity', 'Context', 'Projects & sessions', 'Tools', 'Requests', 'Search']);
    expect(headings()).toEqual([...top, 'Activity over time']);
    const tabs: [string, string[]][] = [
      ['Context', ['What the model read', 'What fills the context window', 'By model', 'Main thread vs subagents']],
      ['Projects & sessions', ['Accounts', 'Projects', 'Sessions']],
      ['Tools', ['Tool output the model read', 'Tool calls made']],
      ['Requests', ['Requests']],
      ['Search', ['Search the transcripts']],
    ];
    for (const [name, panels] of tabs) {
      await tabTo(name, panels[0]!);
      expect(headings(), name).toEqual([...top, ...panels]);
    }
    const bar = screen.getByRole('region', { name: 'Scope' });
    expect(bar.compareDocumentPosition(panel('Quota headroom')) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(panel('Totals in scope').compareDocumentPosition(screen.getByRole('tablist', { name: 'Overview sections' })) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  it('OV-C2: every heading carries its caption as visible text beside it', async () => {
    router();
    show();
    await loaded();
    const captions: [string | null, Record<string, string | RegExp>][] = [
      [
        null,
        {
          'Quota headroom': "each provider's account that is serving right now",
          'All accounts': '2 accounts · spend and tokens add up, quota does not',
          'Totals in scope': /^412 requests · /,
          'Activity over time': '12.40M prompt tokens · 412 requests',
        },
      ],
      [
        'Context',
        {
          'What the model read': 'cache vs. paid in full',
          'What fills the context window': 'estimate · by character share',
          'By model': 'prompt tokens read',
          'Main thread vs subagents': 'share of prompt tokens',
        },
      ],
      ['Projects & sessions', { Projects: '2 projects · click one to scope the page' }],
      ['Tools', { 'Tool output the model read': 'est. tokens', 'Tool calls made': 'by the model' }],
      ['Search', { 'Search the transcripts': 'every message, tool call and result ever captured' }],
    ];
    for (const [tab, panels] of captions) {
      if (tab) await tabTo(tab, Object.keys(panels)[0]!);
      for (const [heading, caption] of Object.entries(panels)) {
        const h = screen.getByRole('heading', { level: 2, name: heading });
        // Beside it: the heading's own row, as text, never a title.
        expect(h.nextElementSibling?.textContent, heading).toMatch(caption);
        expect(h.parentElement?.querySelector('[title]'), heading).toBeNull();
      }
    }
  });

  it('OV-C3: every control is visible at rest — the tabs, and in the open tab its own — none in an overflow menu or behind a disclosure', async () => {
    router();
    show('/?project=%2Fcode%2Fagent-router&tab=requests');
    await loaded();
    expect(within(screen.getByRole('tablist', { name: 'Time range' })).getAllByRole('tab').map((t) => t.textContent)).toEqual(['all', '1h', '24h', '7d', '30d']);
    expect(within(screen.getByRole('tablist', { name: 'Overview sections' })).getAllByRole('tab')).toHaveLength(6);
    expect(screen.getByRole('button', { name: 'Clear scope' })).toBeVisible();
    expect(within(await screen.findByRole('tablist', { name: 'Which requests' })).getAllByRole('tab').map((t) => t.textContent)).toEqual(['All 30', 'Errors 1', 'Subagents 1', 'Slow 1']);
    expect(within(screen.getByRole('group', { name: 'Models' })).getAllByRole('button')).toHaveLength(2);
    expect(within(screen.getByRole('group', { name: 'Stop reasons' })).getAllByRole('button')).toHaveLength(3);
    expect(screen.getByRole('button', { name: 'Show 10 more' })).toBeVisible();
    expect(document.querySelector('.cds--overflow-menu, details, [aria-expanded="false"][aria-haspopup="menu"]')).toBeNull();
    await tabTo('Search', 'Search the transcripts');
    expect(within(screen.getByRole('tablist', { name: 'What to search' })).getAllByRole('tab')).toHaveLength(5);
  });

  it('OV-C11, OV-C86: a panel the page turns off is not in the page at all', async () => {
    router((path) => (path === '/api/forecast' ? ok(forecast([])) : path === '/api/summary' ? ok(summary({ by_account: summary().by_account!.slice(0, 1) })) : undefined));
    show('/?range=all&tab=projects');
    await loaded();
    await screen.findByRole('heading', { level: 2, name: 'Projects' });
    expect(screen.queryByRole('heading', { name: 'All accounts' })).toBeNull();
    expect(screen.queryByRole('heading', { name: 'Accounts' })).toBeNull();
    expect(screen.queryByText('Spend, every account')).toBeNull();
    // The one panel kept while hidden is Search, inside its closed tab, so what was asked survives
    // a tab change (OV-C37); nothing else is hidden rather than left out.
    expect(document.querySelectorAll('#main-content [hidden]')).toHaveLength(0);
    expect([...document.querySelectorAll('[hidden] [data-panel]')].map((p) => [p.getAttribute('data-panel'), p.closest('[hidden]')?.getAttribute('role')])).toEqual([['search', 'tabpanel']]);
  });
});

describe('the address', () => {
  it('OV-C30: a scope or range change pushes a history entry, so Back un-scopes the page', async () => {
    router();
    show('/?range=all&tab=projects');
    await loaded();
    fireEvent.click(within(await screen.findByRole('list', { name: 'Projects' })).getByRole('button', { name: 'agent-router' }));
    await waitFor(() => expect(where()).toBe('/?project=%2Fcode%2Fagent-router&range=all&tab=projects'));
    fireEvent.click(within(screen.getByRole('tablist', { name: 'Time range' })).getByRole('tab', { name: '24h' }));
    await waitFor(() => expect(where()).toBe('/?project=%2Fcode%2Fagent-router&range=24h&tab=projects'));
    fireEvent.click(screen.getByRole('button', { name: 'test: back' }));
    await waitFor(() => expect(where()).toBe('/?project=%2Fcode%2Fagent-router&range=all&tab=projects'));
    fireEvent.click(screen.getByRole('button', { name: 'test: back' }));
    await waitFor(() => expect(where()).toBe('/?range=all&tab=projects'));
  });

  it('OV-C215: the open tab is in the address, Activity by default, and choosing a tab pushes a history entry, so Back returns to the last one', async () => {
    router();
    show();
    await loaded();
    const selected = () => within(screen.getByRole('tablist', { name: 'Overview sections' })).getByRole('tab', { selected: true }).textContent;
    expect(selected()).toBe('Activity');
    await tabTo('Context', 'What the model read');
    expect(where()).toBe('/?range=all&tab=context');
    await tabTo('Requests', 'Requests');
    expect(where()).toBe('/?range=all&tab=requests');
    fireEvent.click(screen.getByRole('button', { name: 'test: back' }));
    await waitFor(() => expect(where()).toBe('/?range=all&tab=context'));
    expect(await screen.findByRole('heading', { level: 2, name: 'What the model read' })).toBeInTheDocument();
    expect(selected()).toBe('Context');
    fireEvent.click(screen.getByRole('button', { name: 'test: back' }));
    await waitFor(() => expect(where()).toBe('/?range=all'));
    expect(selected()).toBe('Activity');
  });

  it('OV-C32: an edited or followed address re-scopes the page without a reload', async () => {
    const r = router();
    show();
    await loaded();
    fireEvent.click(screen.getByRole('button', { name: 'test: follow' }));
    await waitFor(() => expect(r.calls('/api/summary').some((c) => c.query.get('project') === '/code/northstar')).toBe(true));
    expect(screen.getByRole('button', { name: 'Project: northstar' })).toBeInTheDocument();
  });

  it('OV-C29: the scope, the range and the open rows are in the address, and an old #a=&p=… bookmark opens the page unscoped, never rewritten (decision J)', async () => {
    const r = router();
    show('/?project=%2Fcode%2Fnorthstar&range=7d&tab=requests&open=3');
    await loaded();
    const asked = r.calls('/api/summary')[0]!.query;
    expect([asked.get('project'), asked.has('since')]).toEqual(['/code/northstar', true]);
    await screen.findByRole('heading', { level: 2, name: 'Requests' });
    expect(within(panel('Requests')).getAllByRole('button', { expanded: true })).toHaveLength(1);

    // An old bookmark opens the page unscoped at the range an address with none starts at.
    const old = router();
    show('/#a=primary&p=%2Fcode%2Fnorthstar&r=7d');
    await waitFor(() => expect(old.calls('/api/summary').length).toBeGreaterThan(0));
    expect([...old.calls('/api/summary')[0]!.query.keys()].sort()).toEqual(['parts', 'since', 'tz']);

    // That range is the last hour, and choosing all puts it in the address.
    const bare = router();
    show('/');
    await waitFor(() => expect(bare.calls('/api/summary').length).toBeGreaterThan(0));
    const since = Number(bare.calls('/api/summary')[0]!.query.get('since'));
    expect(Math.abs(Date.now() - 3600e3 - since)).toBeLessThan(60e3);
  });

  it('OV-C33: a scope change reads at once and returns the request list to its first twenty rows', async () => {
    const r = router();
    show('/?range=all&tab=requests');
    await loaded();
    fireEvent.click(await screen.findByRole('button', { name: 'Show 10 more' }));
    expect(within(panel('Requests')).getAllByRole('listitem')).toHaveLength(30);
    const before = r.calls('/api/summary').length;
    // Read at once: by the time the press has been handled, not at the next tick.
    await act(async () => {
      fireEvent.click(within(screen.getByRole('tablist', { name: 'Time range' })).getByRole('tab', { name: '7d' }));
    });
    expect(r.calls('/api/summary').length).toBe(before + 1);
    expect(r.calls('/api/requests').at(-1)!.query.has('since')).toBe(true);
    await waitFor(() => expect(within(panel('Requests')).getAllByRole('listitem')).toHaveLength(20));
  });

  it('OV-C216: only the open tab is read: the top and its tab every cycle, a tab opened read at once, and nothing for one that is closed', async () => {
    const r = router();
    const { client } = show();
    await loaded();
    const tabReads = ['/api/projects', '/api/sessions', '/api/tools', '/api/requests'];
    const read = () => tabReads.filter((path) => r.calls(path).length > 0);
    expect(read()).toEqual([]);
    expect(r.calls('/api/summary')[0]!.query.get('parts')).toBe('totals,models,latency,timeline,providers');
    // Opened, a tab is read at once, by the time the press has been handled.
    await act(async () => {
      fireEvent.click(screen.getByRole('tab', { name: 'Tools' }));
    });
    expect(read()).toEqual(['/api/tools']);
    await screen.findByRole('heading', { level: 2, name: 'Tool calls made' });
    await tabTo('Context', 'By model');
    expect(r.calls('/api/summary').at(-1)!.query.get('parts')).toBe('totals,context,models,threads,latency,timeline,providers');
    // Closed, it is read no more: the next cycles read the top and the open tab.
    const tools = r.calls('/api/tools').length;
    await act(() => client.refetchQueries({ queryKey: ['overview'] }));
    await act(() => client.refetchQueries({ queryKey: ['overview'] }));
    expect(r.calls('/api/tools')).toHaveLength(tools);
    expect(read()).toEqual(['/api/tools']);
  });

  it('OV-C217: the scope pickers read their lists when opened, never on the five-second poll', async () => {
    const r = router();
    const { client } = show('/?range=all&account=secondary');
    await loaded();
    expect(r.calls('/api/projects')).toHaveLength(0);
    expect(r.calls('/api/sessions')).toHaveLength(0);
    const pickers = r.calls('/api/summary').filter((c) => c.query.get('parts') === 'by_account');
    expect(pickers).toHaveLength(0);
    // The scoped account's crumb is named from what the top's read asked for it (OV-C13).
    expect(screen.getByRole('button', { name: 'Account: secondary@example.test' })).toBeInTheDocument();

    // Reached through the crumb that controls it: Carbon's popover keeps itself invisible until
    // floating-ui has placed it, and in jsdom, where nothing has a size, it judges the crumb clipped
    // inside the page's layout, so the listbox has no accessible name to be found by.
    const listbox = async (crumb: string) => {
      const button = screen.getByRole('button', { name: crumb });
      await waitFor(() => expect(button).toHaveAttribute('aria-controls'));
      return document.getElementById(button.getAttribute('aria-controls')!)!;
    };
    fireEvent.click(screen.getByRole('button', { name: 'Project: All projects' }));
    const options = await listbox('Project: All projects');
    await waitFor(() => expect(options.textContent).toContain('northstar'));
    expect(r.calls('/api/projects').map((c) => Object.fromEntries(c.query))).toEqual([{ account: 'secondary' }]);
    // Open, it is not on the clock: the poll's cycles read nothing of it.
    await act(() => client.refetchQueries({ queryKey: ['overview'] }));
    await act(() => client.refetchQueries({ queryKey: ['overview'] }));
    expect(r.calls('/api/projects')).toHaveLength(1);
    // Opened again, it reads again.
    fireEvent.keyDown(within(options).getAllByRole('option', { hidden: true })[0]!, { key: 'Escape' });
    await waitFor(() => expect(screen.getByRole('button', { name: 'Project: All projects' })).not.toHaveAttribute('aria-controls'));
    fireEvent.click(screen.getByRole('button', { name: 'Project: All projects' }));
    await waitFor(() => expect(r.calls('/api/projects')).toHaveLength(2));

    fireEvent.click(screen.getByRole('button', { name: 'Session: All sessions' }));
    await listbox('Session: All sessions');
    expect(r.calls('/api/sessions').map((c) => Object.fromEntries(c.query))).toEqual([{ account: 'secondary' }]);
    fireEvent.click(screen.getByRole('button', { name: 'Account: secondary@example.test' }));
    await listbox('Account: secondary@example.test');
    expect(r.calls('/api/summary').filter((c) => c.query.get('parts') === 'by_account')).toHaveLength(1);
  });
});

describe('searching the transcripts', () => {
  const box = () => screen.getByRole('searchbox', { name: 'Search the transcripts' });
  const type = (text: string) => fireEvent.change(box(), { target: { value: text } });
  const reply = (q: string, hits = 1) => ({
    query: q,
    expression: q,
    sessions: [
      { session_id: S1, project: '/code/agent-router', session_label: null, git_branch: 'dev', blocks: 2, hits: 3, first_at: Date.now() - 60_000, last_at: Date.now() - 10_000 },
      { session_id: S2, project: null, session_label: null, git_branch: null, blocks: 1, hits: 1, first_at: Date.now() - 60_000, last_at: Date.now() - 20_000 },
    ],
    hits: Array.from({ length: hits }, (_, i) => ({
      block_id: i + 1,
      kind: 'tool_use',
      chars: 40,
      role: 'assistant',
      ordinal: i,
      request_id: 9,
      at: Date.now() - 10_000,
      session_id: S1,
      project: '/code/agent-router',
      session_label: null,
      snippet: `where does the ‹${q}› roost`,
      rank: -8,
    })),
  });

  it('OV-C36: the box is labelled, invites a word, a phrase, an identifier, does no spell-checking or completion, and the idle caption says what it searches', async () => {
    router();
    show('/?range=all&tab=search');
    await loaded();
    expect(box()).toHaveAttribute('placeholder', 'a word, a phrase, an identifier…');
    expect(box()).toHaveAttribute('spellcheck', 'false');
    expect(box()).toHaveAttribute('autocomplete', 'off');
    expect(within(panel('Search the transcripts')).getByText('every message, tool call and result ever captured')).toBeVisible();
  });

  it('OV-C37: "/" opens the Search tab and focuses the box, selecting its text, from anywhere except in a field or with a modifier, beside a visible "/" hint; what was asked survives a tab change', async () => {
    router();
    show();
    await loaded();
    fireEvent.keyDown(document, { key: '/', metaKey: true });
    expect(where()).toBe('/?range=all');
    fireEvent.keyDown(document, { key: '/' });
    await waitFor(() => expect(box()).toHaveFocus());
    expect(where()).toBe('/?range=all&tab=search');
    type('kestrel');
    // From another tab, the text typed is still there, and selected.
    await tabTo('Activity', 'Activity over time');
    fireEvent.keyDown(document, { key: '/' });
    await waitFor(() => expect(box()).toHaveFocus());
    expect([(box() as HTMLInputElement).selectionStart, (box() as HTMLInputElement).selectionEnd]).toEqual([0, 7]);
    // And on its own tab, at once.
    act(() => box().blur());
    fireEvent.keyDown(document, { key: '/', ctrlKey: true });
    expect(box()).not.toHaveFocus();
    fireEvent.keyDown(document, { key: '/' });
    expect(box()).toHaveFocus();
    const field = document.createElement('input');
    document.body.append(field);
    act(() => field.focus());
    fireEvent.keyDown(field, { key: '/' });
    expect(field).toHaveFocus();
    field.remove();
    expect(within(panel('Search the transcripts')).getByText('/', { selector: 'kbd' })).toBeInTheDocument();
  });

  it('OV-C39: typing searches 160 ms after the last keystroke, and changing the kind searches at once', async () => {
    const r = router();
    show('/?range=all&tab=search');
    await loaded();
    type('k');
    type('ke');
    type('kes');
    await new Promise((done) => setTimeout(done, 100));
    expect(r.calls('/api/search')).toHaveLength(0);
    await waitFor(() => expect(r.calls('/api/search')).toHaveLength(1));
    expect(r.calls('/api/search')[0]!.query.get('q')).toBe('kes');
    // Changing the kind asks by the time the press has been handled, with no pause to wait out.
    await act(async () => {
      fireEvent.click(within(screen.getByRole('tablist', { name: 'What to search' })).getByRole('tab', { name: 'messages' }));
    });
    expect(r.calls('/api/search')).toHaveLength(2);
    expect(r.calls('/api/search')[1]!.query.get('kind')).toBe('text');
  });

  it('OV-C41: a reply that arrives after a newer query was sent is dropped', async () => {
    const first = held();
    router((path, q) => (path === '/api/search' && q.get('q') === 'one' ? first.promise : path === '/api/search' ? ok(reply(q.get('q') ?? '')) : undefined));
    show('/?range=all&tab=search');
    await loaded();
    type('one');
    await new Promise((done) => setTimeout(done, 250));
    type('two');
    expect(await within(panel('Search the transcripts')).findByText('two', { selector: 'mark' })).toBeInTheDocument();
    first.release(ok(reply('one')));
    await new Promise((done) => setTimeout(done, 50));
    expect(within(panel('Search the transcripts')).queryByText('one', { selector: 'mark' })).toBeNull();
    expect(within(panel('Search the transcripts')).getByText('two', { selector: 'mark' })).toBeInTheDocument();
  });

  it('OV-C42: the search is not part of the five-second poll, and asking again asks the router again', async () => {
    const r = router((path, q) => (path === '/api/search' ? ok(reply(q.get('q') ?? '')) : undefined));
    const { client } = show('/?range=all&tab=search');
    await loaded();
    type('kestrel');
    await waitFor(() => expect(r.calls('/api/search')).toHaveLength(1));
    const reads = r.calls('/api/summary').length;
    await act(() => client.refetchQueries({ queryKey: keys.overview({ provider: null, account: null, project: null, session: null, range: 'all' }, 'search') }));
    expect(r.calls('/api/summary').length).toBe(reads + 1);
    expect(r.calls('/api/search')).toHaveLength(1);
    const kinds = within(screen.getByRole('tablist', { name: 'What to search' }));
    fireEvent.click(kinds.getByRole('tab', { name: 'messages' }));
    fireEvent.click(kinds.getByRole('tab', { name: 'all' }));
    await waitFor(() => expect(r.calls('/api/search')).toHaveLength(3));
    expect(r.calls('/api/search').map((c) => c.query.get('kind'))).toEqual([null, 'text', null]);
  });

  it('OV-C48: a hit is a button naming its kind, role, project, session and time over a snippet of three lines at most, and pressing it scopes the page', async () => {
    router((path, q) => (path === '/api/search' ? ok(reply(q.get('q') ?? '')) : undefined));
    show('/?range=all&tab=search');
    await loaded();
    type('kestrel');
    const hits = await screen.findByRole('list', { name: 'Best matches' });
    const hit = within(hits).getByRole('button');
    expect(hit).toHaveTextContent(/^tool call assistant · agent-router · aaaaaaaa · \d{1,2}:\d{2}/);
    expect(within(hit).getByText('kestrel', { selector: 'mark' })).toBeInTheDocument();
    expect(scss).toMatch(/\.snippet \{[^}]*-webkit-line-clamp: 3;[^}]*line-clamp: 3;/);
    fireEvent.click(hit);
    await waitFor(() => expect(where()).toBe(`/?project=%2Fcode%2Fagent-router&session=${S1}&range=all&tab=search`));
  });

  it('OV-C53: the search’s session rows mark the session the page is scoped to as soon as it changes', async () => {
    const r = router((path, q) => (path === '/api/search' ? ok(reply(q.get('q') ?? '')) : undefined));
    show('/?range=all&tab=search');
    await loaded();
    type('kestrel');
    const rows = await screen.findByRole('list', { name: 'Sessions that mention it' });
    const row = within(rows).getByRole('button', { name: 'aaaaaaaa' });
    expect(row).toHaveAttribute('aria-pressed', 'false');
    fireEvent.click(row);
    await waitFor(() => expect(within(screen.getByRole('list', { name: 'Sessions that mention it' })).getByRole('button', { name: 'aaaaaaaa' })).toHaveAttribute('aria-pressed', 'true'));
    expect(r.calls('/api/search')).toHaveLength(1);
  });
});

describe('the quota', () => {
  it('OV-C54: a provider’s block describes its account serving now whatever the page is scoped to, and names it', async () => {
    const r = router();
    show('/?account=primary&project=%2Fcode%2Fnorthstar');
    await loaded();
    const quota = panel('Quota headroom');
    const anthropic = within(quota).getByRole('region', { name: 'Anthropic' });
    expect(within(anthropic).getByRole('link', { name: 'secondary@example.test · max' })).toHaveAttribute('href', '/settings/accounts');
    expect(within(anthropic).getByText('41% used · resets in 2h 00m')).toBeInTheDocument();
    expect(r.calls('/api/quota').every((c) => [...c.query.keys()].length === 0)).toBe(true);
  });

  it('OV-C64: a quota or forecast read that fails keeps the last good reading, with its age, and the rest of the page draws as usual', async () => {
    let failing = false;
    const r = router((path) => (failing && (path === '/api/quota' || path === '/api/forecast') ? dropped() : undefined));
    const { client } = show();
    await loaded();
    failing = true;
    await act(() => client.refetchQueries({ queryKey: ['overview'] }));
    await waitFor(() => expect(within(panel('Quota headroom')).getByText(/^kept from a read .*, the last one failed$/)).toBeInTheDocument());
    expect(within(panel('Quota headroom')).getByText('41% used · resets in 2h 00m')).toBeInTheDocument();
    expect(screen.getByRole('heading', { name: 'All accounts' })).toBeInTheDocument();
    expect(freshness()).toHaveAttribute('data-freshness', 'live');
    expect(r.calls('/api/quota').length).toBeGreaterThan(1);
  });
});

describe('a quota block per provider', () => {
  /// A router with an Anthropic account and a Kimi Code account, each provider's headroom its own.
  const both = () =>
    router((path, query) =>
      path === '/api/summary'
        ? ok(partsOf(summary({ providers: [anthropicBlock(), kimiBlock()] }), query.get('parts')))
        : path === '/api/forecast'
          ? ok(forecast([forecastAccount('secondary'), forecastAccount('primary', { five_hour: null, seven_day: null, seen_at: null }), kimiAccount()], { anthropic: ANTHROPIC_ROOM, kimi: KIMI_ROOM }))
          : undefined,
    );
  const block = (name: string) => within(panel('Quota headroom')).getByRole('region', { name });
  const meter = (b: HTMLElement, name: string) => within(b).getByRole('meter', { name }).getAttribute('aria-valuetext');

  it('OV-C224: Quota headroom holds a block per provider with an account — its mark and name, its serving account, its own windows’ meters, history and burn-downs', async () => {
    both();
    show();
    await loaded();
    const quota = panel('Quota headroom');
    expect([...quota.querySelectorAll('[data-quota-provider]')].map((b) => b.getAttribute('data-quota-provider'))).toEqual(['anthropic', 'kimi']);
    expect(quota.querySelector('[data-quota-blocks]')).toHaveAttribute('data-quota-blocks', '2');
    const anthropic = block('Anthropic');
    const kimi = block('Kimi Code');
    // Each headed by its provider's mark and name (SH-C56).
    expect(within(anthropic).getByRole('heading', { level: 3, name: 'Anthropic' }).querySelector('[data-glyph="anthropic"]')).not.toBeNull();
    expect(within(kimi).getByRole('heading', { level: 3, name: 'Kimi Code' }).querySelector('[data-glyph="kimi"]')).not.toBeNull();
    // Each its own serving account, leading to Settings → Accounts (SH-C17).
    expect(within(anthropic).getByRole('link', { name: 'secondary@example.test · max' })).toHaveAttribute('href', '/settings/accounts');
    expect(within(kimi).getByRole('link', { name: 'Kimi' })).toHaveAttribute('href', '/settings/accounts');
    // Each its own windows: Anthropic's live reading, Kimi Code's usage reading, never the other's.
    expect([meter(anthropic, '5-hour window'), meter(anthropic, '7-day window')]).toEqual(['41% used · resets in 2h 00m', '58% used · resets in 3d 0h']);
    expect([meter(kimi, '5-hour window'), meter(kimi, 'Month window')]).toEqual(['18% used · resets in 3h 00m', '27% used · resets in 20d 0h']);
    expect(within(kimi).getByText('checked 2 min ago')).toBeInTheDocument();
    // Each its own history, its second line by its window's name, and its own burn-downs.
    expect(within(anthropic).getByText('7-day used')).toBeInTheDocument();
    expect(within(kimi).getByText('month used')).toBeInTheDocument();
    expect(kimi.querySelector('path[data-series="month"]')).not.toBeNull();
    expect(within(anthropic).getByText('7-day window · burn-down')).toBeInTheDocument();
    expect(within(kimi).getByText('Month window · burn-down')).toBeInTheDocument();
    expect(within(kimi).queryByText(/7-day/)).toBeNull();
    // Side by side on a desk, stacked on a phone, never wider than the screen.
    expect(scss).toMatch(/\.blocks \{[^}]*repeat\(auto-fit, minmax\(min\(100%, 24rem\), 1fr\)\)[^}]*breakpoint-down\('md'\) \{\s*grid-template-columns: minmax\(0, 1fr\);/);
  });

  it('OV-C224: a router with Kimi Code’s accounts alone shows Kimi Code’s block alone, and nothing reads “no account” for Anthropic', async () => {
    router((path, query) =>
      path === '/api/summary'
        ? ok(partsOf(summary({ providers: [kimiBlock()] }), query.get('parts')))
        : path === '/api/quota'
          ? refused('no Anthropic account')
          : path === '/api/forecast'
            ? ok(forecast([kimiAccount()], { kimi: KIMI_ROOM }))
            : undefined,
    );
    show();
    await loaded();
    const quota = panel('Quota headroom');
    expect([...quota.querySelectorAll('[data-quota-provider]')].map((b) => b.getAttribute('data-quota-provider'))).toEqual(['kimi']);
    expect(within(quota).queryByRole('region', { name: 'Anthropic' })).toBeNull();
    expect(quota).not.toHaveTextContent(/Anthropic|no account|No account|No quota reading/);
    expect(meter(block('Kimi Code'), 'Month window')).toBe('27% used · resets in 20d 0h');
    expect(within(block('Kimi Code')).getByText('Headroom left')).toBeInTheDocument();
  });

  it('OV-C224, OV-C225: Kimi Code’s month is named by its own key and counted in its own block’s headroom', async () => {
    both();
    show();
    await loaded();
    const kimi = block('Kimi Code');
    const tile = within(kimi).getByText('Headroom left').closest('.cds--tile') as HTMLElement;
    expect(within(tile).getByText('3.5 h')).toBeInTheDocument();
    expect(within(tile).getByText("first to go: Kimi's month window, in 5d 0h")).toBeInTheDocument();
    expect(tile).toHaveAttribute('data-tone', 'bad');
    expect(kimi.querySelector('[data-first-to-go]')).toHaveTextContent(/^First to run out: Kimi's month window, .+\.$/);
  });

  it('OV-C225: headroom left and first to run out are each block’s own, and no figure compares one provider’s windows with another’s', async () => {
    both();
    show();
    await loaded();
    const anthropic = block('Anthropic');
    const kimi = block('Kimi Code');
    // Anthropic's: its own hours, and its own first to go, a 7-day window too close to call.
    const own = within(anthropic).getByText('Headroom left').closest('.cds--tile') as HTMLElement;
    expect(within(own).getByText('4.9 h')).toBeInTheDocument();
    expect(within(own).getByText("too close to call: secondary@example.test's 7-day window could run out in 2d 20h")).toBeInTheDocument();
    expect(anthropic.querySelector('[data-first-to-go]')).toHaveTextContent(/^First to run out: secondary@example\.test's 7-day window, .+ — too close to call\.$/);
    // Kimi Code's: its own hours, and its month, by name.
    const theirs = within(kimi).getByText('Headroom left').closest('.cds--tile') as HTMLElement;
    expect(within(theirs).getByText('3.5 h')).toBeInTheDocument();
    expect(kimi.querySelector('[data-first-to-go]')).toHaveTextContent(/^First to run out: Kimi's month window, /);
    // Neither block names the other provider's account or windows: no figure sets Anthropic's 7-day
    // against Kimi Code's month.
    expect(anthropic).not.toHaveTextContent(/Kimi|month/);
    expect(kimi).not.toHaveTextContent(/secondary@example\.test|7-day/);
    // One headroom per block, and none totalled across providers in All accounts.
    expect(within(panel('Quota headroom')).getAllByText('Headroom left')).toHaveLength(2);
    expect(within(panel('All accounts')).queryByText('Headroom left')).toBeNull();
    expect(panel('All accounts').querySelector('[data-first-to-go]')).toBeNull();
  });
});

describe('all accounts', () => {
  it('OV-C96: an account removed from the router keeps its card while it has history to report', async () => {
    router((path) =>
      path === '/api/accounts' ? ok(ACCOUNTS.slice(0, 1)) : path === '/api/forecast' ? ok(forecast([forecastAccount('secondary'), forecastAccount('retired', { five_hour: null, seven_day: null, label: 'retired' })])) : undefined,
    );
    show();
    await loaded();
    expect([...document.querySelectorAll('[data-account-card]')].map((c) => c.getAttribute('data-account-card'))).toEqual(['secondary', 'retired']);
  });

  it('OV-C218, OV-C219, OV-C221: a Kimi Code account’s card names Kimi Code, draws its second window as Month, a spent window as spent until its reset, and its cost as not priced', async () => {
    const at = (ms: number) => Math.floor(ms / 1000);
    const kimi = forecastAccount('kimi', {
      label: 'Kimi',
      email: null,
      provider: 'kimi',
      plan: null,
      pool: null,
      status: 'rejected',
      usage: { requests: 12, prompt_tokens: 40_000, output_tokens: 900, cost: 0, unpriced: 12, last_seen: NOW - 5 * 60_000 },
      five_hour: { used: 1, reset_at: at(NOW + 2 * 3_600_000), expired: false, spent: true, basis: 'none', series: [] },
      seven_day: null,
      month: { used: 0.12, reset_at: at(NOW + 20 * 86_400_000), expired: false, spent: false, basis: 'none', series: [] },
    });
    router((path) => (path === '/api/forecast' ? ok(forecast([forecastAccount('secondary'), kimi])) : undefined));
    show();
    await loaded();
    const card = within(document.querySelector<HTMLElement>('[data-account-card="kimi"]')!);
    // Named by its provider's tag beside the account's name (OV-C231), and not again on its line.
    expect(card.getByText('Kimi Code').closest('[data-provider]')).toHaveAttribute('data-provider', 'kimi');
    expect(card.getByText(/^last request 5 min ago/)).toBeInTheDocument();
    expect(card.getByText('5-hour')).toBeInTheDocument();
    expect(card.getByText('Month')).toBeInTheDocument();
    expect(card.queryByText('7-day')).toBeNull();
    expect(card.getByText(/^spent until it resets in 2h 00m/)).toBeInTheDocument();
    expect(card.getByText('not priced')).toBeInTheDocument();
    expect(card.queryByText('$0')).toBeNull();
  });
});

describe('Totals in scope', () => {
  it('OV-C107: Errors offers a visible "List the N errors" that opens the Requests tab filtered to errors, returns that list to its first page and brings it into view', async () => {
    const into = vi.spyOn(Element.prototype, 'scrollIntoView').mockImplementation(() => {});
    router();
    const first = show();
    await loaded();
    // From another tab: the Requests tab opens, and the list is brought into view once it is drawn.
    const button = within(tile('Errors')).getByRole('button', { name: 'List the 3 errors' });
    expect(button.tagName).toBe('BUTTON');
    fireEvent.click(button);
    await waitFor(() => expect(where()).toBe('/?range=all&tab=requests&kind=errors'));
    await waitFor(() => expect(into).toHaveBeenCalled());
    expect(into.mock.contexts.at(-1)).toBe(document.getElementById('requests'));
    expect(within(panel('Requests')).getAllByRole('listitem')).toHaveLength(1);
    first.unmount();

    // On the Requests tab, from its second page.
    into.mockClear();
    router();
    show('/?range=all&tab=requests');
    await loaded();
    fireEvent.click(await screen.findByRole('button', { name: 'Show 10 more' }));
    fireEvent.click(within(tile('Errors')).getByRole('button', { name: 'List the 3 errors' }));
    await waitFor(() => expect(where()).toBe('/?range=all&tab=requests&kind=errors'));
    expect(within(panel('Requests')).getAllByRole('listitem')).toHaveLength(1);
    await waitFor(() => expect(into).toHaveBeenCalled());
    expect(into.mock.contexts.at(-1)).toBe(document.getElementById('requests'));
  });
});

describe('the lists', () => {
  it('OV-C132: lists keep the router’s order', async () => {
    router((path) => (path === '/api/projects' ? ok([project('/code/small', 10), project('/code/large', 9_000_000)]) : undefined));
    show('/?range=all&tab=projects');
    await loaded();
    await screen.findByRole('list', { name: 'Projects' });
    expect(within(panel('Projects')).getAllByRole('button').map((b) => b.getAttribute('aria-labelledby') && b.textContent?.split('/')[0])).toEqual(['small', 'large']);
  });

  it('OV-C136: a project with no recorded path is "(unresolved)" and scopes like any other', async () => {
    const r = router((path) => (path === '/api/projects' ? ok([...PROJECTS, project(null, 1_000)]) : undefined));
    show('/?range=all&tab=projects');
    await loaded();
    fireEvent.click(within(await screen.findByRole('list', { name: 'Projects' })).getByRole('button', { name: '(unresolved)' }));
    await waitFor(() => expect(r.calls('/api/summary').some((c) => c.query.get('project') === '(unresolved)')).toBe(true));
    expect(where()).toBe('/?project=%28unresolved%29&range=all&tab=projects');
  });

  it('OV-C214: a scoped project says its repository and every directory on its machine under the scope bar, and an unscoped page says neither', async () => {
    const repository = 'gitlab.example.com/acme/ui-kit-web';
    const ui = project(repository, 12_000_000, {
      repository,
      directories: [
        { cwd: '/Users/sample/code/ui-kit-web', machine: { id: 'm-studio', label: 'studio' }, remotes: [] },
        { cwd: '/home/sample/ui-kit-web-lane-2', machine: { id: 'm-linux', label: 'linux-box' }, remotes: [] },
        { cwd: '/Users/sample/code/ui-kit-web-lane-3', machine: null, remotes: [] },
      ],
    });
    const r = router((path) => (path === '/api/projects' ? ok([ui, ...PROJECTS]) : undefined));
    show('/?range=all&tab=projects');
    await loaded();
    expect(screen.queryByLabelText('Where this project ran')).toBeNull();
    fireEvent.click(within(await screen.findByRole('list', { name: 'Projects' })).getByRole('button', { name: 'ui-kit-web' }));
    await waitFor(() => expect(r.calls('/api/summary').some((c) => c.query.get('project') === repository)).toBe(true));
    const places = await screen.findByLabelText('Where this project ran');
    expect(within(places).getAllByRole('term').map((t) => t.textContent)).toEqual(['Repository', '3 directories']);
    expect(within(places).getAllByRole('definition')[0]).toHaveTextContent(repository);
    expect(within(places).getAllByRole('listitem').map((li) => li.textContent)).toEqual([
      'studio · /Users/sample/code/ui-kit-web',
      'linux-box · /home/sample/ui-kit-web-lane-2',
      '/Users/sample/code/ui-kit-web-lane-3',
    ]);
    // Visible as facts, with nothing to open; the scope bar still comes first.
    expect(places.querySelector('button, [aria-expanded]')).toBeNull();
    expect(screen.getByRole('region', { name: 'Scope' }).compareDocumentPosition(places) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  it('OV-C139: a long list shows a first page and a visible "Show more"; Sessions holds up to 200', async () => {
    const many = Array.from({ length: 45 }, (_, i) => session(`${String(i).padStart(8, '0')}-0000-4000-8000-000000000000`));
    router((path) => (path === '/api/sessions' ? ok(many) : undefined));
    show('/?range=all&tab=projects');
    await loaded();
    await screen.findByRole('list', { name: 'Sessions' });
    const list = () => within(panel('Sessions')).getByRole('list', { name: 'Sessions' });
    expect(within(list()).getAllByRole('listitem')).toHaveLength(20);
    fireEvent.click(within(panel('Sessions')).getByRole('button', { name: 'Show 20 more' }));
    fireEvent.click(within(panel('Sessions')).getByRole('button', { name: 'Show 5 more' }));
    expect(within(list()).getAllByRole('listitem')).toHaveLength(45);
    fireEvent.click(within(panel('Sessions')).getByRole('button', { name: 'Show fewer' }));
    expect(within(list()).getAllByRole('listitem')).toHaveLength(20);
  });
});

describe('the requests', () => {
  it('OV-C155: filters that leave nothing say so beside "Clear the filters", and no requests at all say that', async () => {
    router();
    const filtered = show('/?tab=requests&kind=errors&stop=end_turn');
    await loaded();
    expect(await within(panel('Requests')).findByText('No request matches these filters.')).toBeInTheDocument();
    fireEvent.click(within(panel('Requests')).getByRole('button', { name: 'Clear the filters' }));
    await waitFor(() => expect(where()).toBe('/?tab=requests'));
    filtered.unmount();

    router((path) => (path === '/api/requests' ? ok([]) : undefined));
    show('/?range=all&tab=requests');
    await loaded();
    expect(await within(panel('Requests')).findByText('No requests for this scope.')).toBeInTheDocument();
  });

  it('OV-C153: a model filter whose model has left the loaded rows is cleared from the address, never left filtering to nothing', async () => {
    router();
    show('/?tab=requests&model=claude-haiku-4-5&kind=subagents');
    await loaded();
    await waitFor(() => expect(where()).toBe('/?tab=requests&kind=subagents'));
    expect(within(panel('Requests')).getAllByRole('listitem')).toHaveLength(1);
  });

  it('OV-C154: a filter change returns the list to its first twenty rows', async () => {
    router();
    show('/?range=all&tab=requests');
    await loaded();
    fireEvent.click(await screen.findByRole('button', { name: 'Show 10 more' }));
    expect(within(panel('Requests')).getAllByRole('listitem')).toHaveLength(30);
    fireEvent.click(within(screen.getByRole('group', { name: 'Stop reasons' })).getByRole('button', { name: /^tool_use/ }));
    await waitFor(() => expect(within(panel('Requests')).getAllByRole('listitem')).toHaveLength(20));
  });
});

describe('each provider', () => {
  it('OV-C231: the Accounts list and every All accounts card tag an account with its provider, and "active" says for which provider', async () => {
    const kimi = { id: 'kimi-acc', label: 'Kimi', provider: 'kimi', active: 1, requests: 12, prompt_tokens: 40_000, cost: 0, output_tokens: 900, sessions: 1, last_seen: NOW - 5 * MIN };
    const byAccount = [...(summary().by_account ?? []), kimi];
    router((path, query) => (path === '/api/summary' ? ok(partsOf(summary({ by_account: byAccount }), query.get('parts'))) : undefined));
    show('/?range=all&tab=projects');
    await loaded();
    const list = await screen.findByRole('list', { name: 'Accounts' }, { timeout: 10_000 });
    const row = (name: string) => within(list).getByRole('button', { name });
    // An Anthropic account and a Kimi Code account can both be active: each says for which provider.
    expect(row('secondary@example.test')).toHaveTextContent('active for Anthropic');
    expect(row('Kimi')).toHaveTextContent('active for Kimi Code');
    expect(row('Kimi').querySelector('[data-provider="kimi"] [data-glyph="kimi"]')).not.toBeNull();
    expect(row('primary@example.test').querySelector('[data-provider="anthropic"] [data-glyph="anthropic"]')).not.toBeNull();
    expect(row('primary@example.test')).toHaveTextContent('Anthropic');
    expect(row('primary@example.test')).not.toHaveTextContent(/active/);
    // All accounts: every card tagged, the serving one's tag saying for which provider.
    const card = (id: string) => within(document.querySelector<HTMLElement>(`[data-account-card="${id}"]`)!);
    expect(card('secondary').getByText('active for Anthropic').closest('[data-provider]')).toHaveAttribute('data-provider', 'anthropic');
    expect(card('primary').getByText('Anthropic').closest('[data-provider]')).toHaveAttribute('data-provider', 'anthropic');
    expect(card('primary').queryByText(/^active/)).toBeNull();
  });

  it('OV-C231: a request’s card wears its provider’s mark, and its detail names the provider and the account that served it', async () => {
    router((path) =>
      path === '/api/requests'
        ? ok([request(1, { provider: 'kimi', account_id: 'kimi-acc', model: 'k3' }), request(2, { account_id: 'retired' })])
        : path === '/api/accounts'
          ? ok([...ACCOUNTS, { id: 'kimi-acc', label: 'Kimi', email: null, source: 'token', provider: 'kimi', active: 1 }])
          : undefined,
    );
    show('/?range=all&tab=requests');
    await loaded();
    const list = await screen.findByRole('list', { name: 'Requests, newest first' }, { timeout: 10_000 });
    const [kimi, older] = within(list).getAllByRole('listitem') as [HTMLElement, HTMLElement];
    // On the card, the mark alone, named by its provider; a request recorded with none is Anthropic's.
    expect(within(kimi).getByRole('img', { name: 'Kimi Code' })).toBeInTheDocument();
    expect(within(older).getByRole('img', { name: 'Anthropic' })).toBeInTheDocument();
    const fact = (item: HTMLElement, label: string) => within(item).getByText(label, { selector: 'dt' }).nextElementSibling?.textContent;
    fireEvent.click(within(kimi).getByRole('button'));
    await waitFor(() => expect(fact(kimi, 'account')).toBe('Kimi · kimi-acc'), { timeout: 10_000 });
    expect(fact(kimi, 'provider')).toBe('Kimi Code');
    // An account the router no longer holds is its id.
    fireEvent.click(within(older).getByRole('button'));
    await waitFor(() => expect(fact(older, 'account')).toBe('retired'), { timeout: 10_000 });
    expect(fact(older, 'provider')).toBe('Anthropic');
  });

  it('OV-C231: By model tags each model with the provider that served it, by its mark and name', async () => {
    const models = [
      ...summary().models,
      { model: 'k3', provider: 'kimi', requests: 2, prompt_tokens: 400, cost: 0, output_tokens: 10, thinking_tokens: 0, cache_read_tokens: 0, errors: 0, subagent_requests: 0 },
    ];
    router((path, query) => (path === '/api/summary' ? ok(partsOf(summary({ models }), query.get('parts'))) : undefined));
    show('/?range=all&tab=context');
    await loaded();
    const rows = within(await screen.findByRole('list', { name: 'By model' }, { timeout: 10_000 })).getAllByRole('listitem');
    expect(rows.map((r) => r.querySelector('[data-provider]')?.getAttribute('data-provider'))).toEqual(['anthropic', 'anthropic', 'kimi']);
    expect(rows[0]).toHaveTextContent('Anthropic');
    expect(rows[2]).toHaveTextContent('Kimi Code');
  });

  it('SE-C242: the Overview’s session rows mark the providers of their requests beside their client, each its mark alone', async () => {
    router((path) =>
      path === '/api/sessions'
        ? ok([session(S1, { providers: ['anthropic', 'kimi'] }), session(S2, { providers: ['kimi'], entrypoint: 'kimi-code-cli/2.1.1', client: 'kimi-code-cli' })])
        : undefined,
    );
    show('/?range=all&tab=projects');
    await loaded();
    const list = await screen.findByRole('list', { name: 'Sessions' }, { timeout: 10_000 });
    const marks = (name: string) => [...within(list).getByRole('button', { name }).querySelectorAll('[role="img"]')].map((m) => [m.getAttribute('aria-label'), m.getAttribute('title')]);
    expect(marks('aaaaaaaa')).toEqual([
      ['Anthropic', 'Anthropic'],
      ['Kimi Code', 'Kimi Code'],
    ]);
    expect(marks('bbbbbbbb')).toEqual([['Kimi Code', 'Kimi Code']]);
    // Beside its client, in the client's chip.
    expect(within(list).getByRole('button', { name: 'bbbbbbbb' }).querySelector('[data-providers]')).toHaveTextContent(/^Kimi Code CLI$/);
  });
});

describe('the provider in the scope, and each total by provider', () => {
  const KIMI_ROW: AccountSplitRow = { id: 'kimi-acc', label: 'Kimi', provider: 'kimi', active: 1, requests: 12, prompt_tokens: 200_000, cost: 0, output_tokens: 10_000, sessions: 1, last_seen: NOW - 5 * MIN };
  /// A router with an Anthropic account and a Kimi Code account, its totals split between them, and
  /// By account narrowed by the provider asked for, as the router narrows it.
  const both = () =>
    router((path, query) => {
      if (path === '/api/summary') {
        const all: AccountSplitRow[] = [...(summary().by_account ?? []), KIMI_ROW];
        const provider = query.get('provider');
        const byAccount = provider ? all.filter((a) => (a.provider || 'anthropic') === provider) : all;
        return ok(partsOf(summary({ totals: totals({ by_provider: TWO_PROVIDERS }), by_account: byAccount, providers: [anthropicBlock(), kimiBlock()] }), query.get('parts')));
      }
      if (path === '/api/accounts') return ok([...ACCOUNTS, { id: 'kimi-acc', label: 'Kimi', email: null, source: 'token', provider: 'kimi', active: 1 }]);
      return undefined;
    });
  const switcher = () => screen.getByRole('tablist', { name: 'Provider' });
  /// A total's lines by provider: each provider's mark, by its name, and its figure and share.
  const lines = (label: string) =>
    within(within(tile(label)).getByRole('list', { name: `${label}, by provider` }))
      .getAllByRole('listitem')
      .map((li) => [within(li).getByRole('img').getAttribute('aria-label'), li.textContent?.trim()]);

  it('OV-C227: the scope offers every provider, then each provider with an account by its mark and name, beside the account picker; a router with one provider offers no choice', async () => {
    both();
    const first = show();
    await loaded();
    const tabs = within(switcher()).getAllByRole('tab');
    expect(tabs.map((t) => t.textContent)).toEqual(['All providers', 'Anthropic', 'Kimi Code']);
    expect(tabs[0]).toHaveAttribute('aria-selected', 'true');
    expect(tabs[1]!.querySelector('[data-glyph="anthropic"]')).not.toBeNull();
    expect(tabs[2]!.querySelector('[data-glyph="kimi"]')).not.toBeNull();
    // In the scope bar, visible at rest, just before the account picker it narrows.
    expect(within(screen.getByRole('region', { name: 'Scope' })).getByRole('tablist', { name: 'Provider' })).toBe(switcher());
    expect(switcher().compareDocumentPosition(screen.getByRole('button', { name: 'Account: All accounts' })) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    first.unmount();

    // Anthropic's accounts alone: nothing to choose between.
    router();
    show();
    await loaded();
    expect(screen.queryByRole('tablist', { name: 'Provider' })).toBeNull();
  });

  it('OV-C227: choosing a provider puts it in the address and every read follows it; an account picked from another provider is let go, the project kept; Quota headroom and All accounts still cover every provider', async () => {
    const r = both();
    show('/?account=secondary&project=%2Fcode%2Fagent-router&range=all');
    await loaded();
    fireEvent.click(within(switcher()).getByRole('tab', { name: 'Kimi Code' }));
    await waitFor(() => expect(where()).toBe('/?provider=kimi&project=%2Fcode%2Fagent-router&range=all'));
    await waitFor(() => expect(r.calls('/api/summary').some((c) => c.query.get('provider') === 'kimi')).toBe(true));
    const asked = r.calls('/api/summary').find((c) => c.query.get('provider') === 'kimi')!.query;
    expect([asked.get('account'), asked.get('project')]).toEqual([null, '/code/agent-router']);
    await waitFor(() => expect(within(switcher()).getByRole('tab', { selected: true })).toHaveTextContent('Kimi Code'));
    // The quota and the forecast keep their own horizon: every provider's blocks, every account.
    expect(r.calls('/api/forecast').every((c) => !c.query.has('provider'))).toBe(true);
    expect([...panel('Quota headroom').querySelectorAll('[data-quota-provider]')].map((b) => b.getAttribute('data-quota-provider'))).toEqual(['anthropic', 'kimi']);
    // Clear scope clears the provider too, keeping the range; Back returns to it (OV-C25, OV-C30).
    fireEvent.click(screen.getByRole('button', { name: 'Clear scope' }));
    await waitFor(() => expect(where()).toBe('/?range=all'));
    fireEvent.click(screen.getByRole('button', { name: 'test: back' }));
    await waitFor(() => expect(where()).toBe('/?provider=kimi&project=%2Fcode%2Fagent-router&range=all'));
    // Every provider again: the provider leaves the address, the rest of the scope stays.
    fireEvent.click(within(switcher()).getByRole('tab', { name: 'All providers' }));
    await waitFor(() => expect(where()).toBe('/?project=%2Fcode%2Fagent-router&range=all'));
  });

  it('OV-C227: an account of the provider chosen is kept, and with a provider chosen the account picker reads and offers only that provider’s accounts', async () => {
    const r = both();
    show('/?account=kimi-acc&range=all');
    await loaded();
    fireEvent.click(within(switcher()).getByRole('tab', { name: 'Kimi Code' }));
    await waitFor(() => expect(where()).toBe('/?provider=kimi&account=kimi-acc&range=all'));
    fireEvent.click(within(switcher()).getByRole('tab', { name: 'All providers' }));
    await waitFor(() => expect(where()).toBe('/?account=kimi-acc&range=all'));
    fireEvent.click(within(switcher()).getByRole('tab', { name: 'Kimi Code' }));
    await waitFor(() => expect(where()).toBe('/?provider=kimi&account=kimi-acc&range=all'));
    fireEvent.click(screen.getByRole('button', { name: 'Clear scope' }));
    await waitFor(() => expect(where()).toBe('/?range=all'));
    fireEvent.click(within(switcher()).getByRole('tab', { name: 'Kimi Code' }));
    await waitFor(() => expect(where()).toBe('/?provider=kimi&range=all'));

    // Reached through the crumb that controls it, as in OV-C217's spec.
    const crumb = screen.getByRole('button', { name: 'Account: All accounts' });
    fireEvent.click(crumb);
    await waitFor(() => expect(crumb).toHaveAttribute('aria-controls'));
    const listbox = document.getElementById(crumb.getAttribute('aria-controls')!)!;
    await waitFor(() => expect(listbox.textContent).toContain('kimi-acc'));
    expect(r.calls('/api/summary').filter((c) => c.query.get('parts') === 'by_account').map((c) => c.query.get('provider'))).toEqual(['kimi']);
    expect(within(listbox).getAllByRole('option', { hidden: true }).map((o) => o.getAttribute('data-key'))).toEqual(['*', 'kimi-acc']);
    // The project picker reads under the provider too.
    fireEvent.keyDown(within(listbox).getAllByRole('option', { hidden: true })[0]!, { key: 'Escape' });
    await waitFor(() => expect(crumb).not.toHaveAttribute('aria-controls'));
    fireEvent.click(screen.getByRole('button', { name: 'Project: All projects' }));
    await waitFor(() => expect(r.calls('/api/projects')).toHaveLength(1));
    expect(Object.fromEntries(r.calls('/api/projects')[0]!.query)).toEqual({ provider: 'kimi' });
  });

  it('OV-C227: Search follows the whole scope, its provider and its account with the rest, and says the scope narrows it', async () => {
    const r = both();
    show('/?provider=kimi&account=kimi-acc&range=all&tab=search');
    await loaded();
    expect(within(panel('Search the transcripts')).getByText('every message, tool call and result captured in the current scope')).toBeVisible();
    fireEvent.change(screen.getByRole('searchbox', { name: 'Search the transcripts' }), { target: { value: 'kestrel' } });
    await waitFor(() => expect(r.calls('/api/search')).toHaveLength(1));
    expect(Object.fromEntries(r.calls('/api/search')[0]!.query)).toEqual({ q: 'kestrel', provider: 'kimi', account: 'kimi-acc' });
  });

  it('OV-C226: each total keeps its figure and shows its split by provider under it — each provider’s mark, figure and share over a thin bar of their shares — and a router with one provider shows none', async () => {
    both();
    const first = show();
    await loaded();
    // One figure per total, as with one provider.
    expect(tile('Prompt tokens read')).toHaveTextContent(/^Prompt tokens read12\.40M/);
    expect(lines('Prompt tokens read')).toEqual([
      ['Anthropic', '12.20M 98%'],
      ['Kimi Code', '200k 1.6%'],
    ]);
    expect(lines('Spend')).toEqual([
      ['Anthropic', '$41.20 100%'],
      ['Kimi Code', 'not priced'],
    ]);
    expect(lines('Requests')).toEqual([
      ['Anthropic', '400 97%'],
      ['Kimi Code', '12 2.9%'],
    ]);
    expect(lines('Output tokens')).toEqual([
      ['Anthropic', '300k 97%'],
      ['Kimi Code', '10k 3.2%'],
    ]);
    expect(lines('Served from cache')).toEqual([
      ['Anthropic', '10.30M 99%'],
      ['Kimi Code', '100k 1.0%'],
    ]);
    expect(lines('Cache writes')).toEqual([
      ['Anthropic', '1.38M 99%'],
      ['Kimi Code', '20k 1.4%'],
    ]);
    // The thin bar: a part per provider, sized by its figure, in its provider's colour; hidden from a
    // screen reader, which hears the lines, and no tab stop.
    const bar = tile('Prompt tokens read').querySelector('[data-provider-split] > [aria-hidden="true"]') as HTMLElement;
    const parts = [...bar.querySelectorAll<HTMLElement>('[data-provider]')];
    expect(parts.map((p) => [p.dataset['provider'], p.style.flexGrow, p.style.background])).toEqual([
      ['anthropic', '12200000', 'var(--ar-tok)'],
      ['kimi', '200000', 'var(--ar-model-kimi)'],
    ]);
    expect(tile('Prompt tokens read').querySelector('[data-provider-split] [tabindex]')).toBeNull();
    // A provider with nothing priced adds nothing to Spend's bar.
    expect(tile('Spend').querySelectorAll('[data-provider-split] > [aria-hidden="true"] > [data-provider]')).toHaveLength(1);
    // No figure per provider for latency or errors.
    expect([tile('Time to first token'), tile('Errors')].map((t) => t.querySelector('[data-provider-split]'))).toEqual([null, null]);
    first.unmount();

    // Anthropic's alone: one figure per total and no split anywhere.
    router();
    show();
    await loaded();
    expect(document.querySelector('[data-provider-split]')).toBeNull();
  });
});

describe('reading the router', () => {
  it('OV-C161: the next read starts five seconds after the previous one finished, so reads never overlap', async () => {
    // Fake timers already in place (the held Date) would keep this spec's own from installing.
    vi.useRealTimers();
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'setInterval', 'clearInterval', 'Date'], now: NOW });
    const r = router((path) => (path === '/api/summary' ? new Promise<Response>((done) => setTimeout(() => done(ok(summary())), 3_000)) : undefined));
    show();
    const step = (ms: number) => act(() => vi.advanceTimersByTimeAsync(ms));
    await step(10);
    expect(r.calls('/api/summary')).toHaveLength(1);
    // The read takes three seconds; the next starts five after it finished, at eight, not at five.
    await step(3_000);
    await step(4_900);
    expect(r.calls('/api/summary')).toHaveLength(1);
    await step(300);
    expect(r.calls('/api/summary')).toHaveLength(2);
  });

  it('OV-C162: a reply for a scope that is no longer current is never drawn', async () => {
    const late = held();
    router((path, q) =>
      path === '/api/summary' && q.get('project') === '/code/agent-router'
        ? late.promise
        : path === '/api/summary' && q.get('project') === '/code/northstar'
          ? ok(summary({ totals: totals({ requests: 7 }) }))
          : undefined,
    );
    show('/?project=%2Fcode%2Fagent-router');
    fireEvent.click(screen.getByRole('button', { name: 'test: follow' }));
    await loaded();
    expect(within(tile('Requests')).getByText('7')).toBeInTheDocument();
    late.release(ok(summary({ totals: totals({ requests: 999 }) })));
    await new Promise((done) => setTimeout(done, 50));
    expect(within(tile('Requests')).queryByText('999')).toBeNull();
    expect(within(tile('Requests')).getByText('7')).toBeInTheDocument();
  });

  it('OV-C163: while a new scope is read, the previous figures stay on screen', async () => {
    const next = held();
    router((path, q) => (path === '/api/summary' && q.get('project') === '/code/northstar' ? next.promise : undefined));
    show();
    await loaded();
    fireEvent.click(screen.getByRole('button', { name: 'test: follow' }));
    await waitFor(() => expect(freshness()).toHaveTextContent('reading the new scope…'));
    expect(within(tile('Requests')).getByText('412')).toBeInTheDocument();
    expect(panel('Totals in scope')).toHaveAttribute('aria-busy', 'true');
    next.release(ok(summary({ totals: totals({ requests: 8 }) })));
    await waitFor(() => expect(within(tile('Requests')).getByText('8')).toBeInTheDocument());
    expect(panel('Totals in scope')).not.toHaveAttribute('aria-busy');
  });

  it('OV-C164: failed cycles mark the page stale with the time of the last good read, five unreachable, and a good one clears both; nothing drawn is blanked', async () => {
    let failing = false;
    router((path) => (failing && path === '/api/summary' ? refused('database is locked') : undefined));
    const { client } = show();
    await loaded();
    failing = true;
    const cycle = () => act(() => client.refetchQueries({ queryKey: ['overview'] }));
    await cycle();
    await waitFor(() => expect(freshness()).toHaveAttribute('data-freshness', 'stale'));
    expect(freshness()).toHaveTextContent(/^stale — the last refresh failed: database is locked; showing what was read (just now|\d+s ago)/);
    for (let i = 0; i < 4; i++) await cycle();
    await waitFor(() => expect(freshness()).toHaveAttribute('data-freshness', 'unreachable'));
    expect(freshness()).toHaveTextContent(/^unreachable — 5 refreshes in a row failed/);
    expect(within(tile('Requests')).getByText('412')).toBeInTheDocument();
    failing = false;
    await cycle();
    await waitFor(() => expect(freshness()).toHaveAttribute('data-freshness', 'live'));
  });

  it('OV-C166: the accounts and the router’s mode are read beside the snapshot, and their failing leaves the figures and freshness alone', async () => {
    const r = router((path) => (path === '/api/accounts' ? refused('keychain locked') : path === '/api/auth' ? dropped() : undefined));
    show();
    await loaded();
    await waitFor(() => expect(r.calls('/api/accounts').length).toBeGreaterThan(0));
    expect(r.calls('/api/auth').length).toBeGreaterThan(0);
    expect(freshness()).toHaveAttribute('data-freshness', 'live');
    expect(within(tile('Requests')).getByText('412')).toBeInTheDocument();
  });

  it('OV-C168: an action that changes what the page shows asks the one poll to read now, and never starts a second read beside it', async () => {
    const gates: ReturnType<typeof held>[] = [];
    const r = router((path, _q, n) => {
      if (path !== '/api/summary' || n === 1) return undefined;
      const g = held();
      gates.push(g);
      return g.promise;
    });
    const { client } = show();
    await loaded();
    act(() => {
      void client.invalidateQueries();
    });
    await waitFor(() => expect(r.calls('/api/summary')).toHaveLength(2));
    act(() => {
      void client.invalidateQueries();
    });
    await waitFor(() => expect(r.calls('/api/summary')).toHaveLength(3));
    // The read in flight is given up for the new one: never two at once.
    const live = r.calls('/api/summary').filter((c, i) => i > 0 && !c.signal?.aborted);
    expect(live).toHaveLength(1);
    gates.at(-1)!.release(ok(summary()));
  });

  it('OV-C169: re-reads at once when its tab becomes visible again', async () => {
    const r = router();
    show();
    await loaded();
    await waitFor(() => expect(freshness()).toHaveAttribute('data-freshness', 'live'));
    const before = r.calls('/api/summary').length;
    // At once: by the time the event has been handled, never at the next five-second tick.
    await act(async () => {
      window.dispatchEvent(new Event('visibilitychange'));
    });
    expect(r.calls('/api/summary').length).toBe(before + 1);
  });

  it('OV-C173: the snapshot’s time is on the page, and a value kept from an earlier read carries its own age', async () => {
    let failing = false;
    router((path) => (failing && path === '/api/quota' ? dropped() : undefined));
    const { client } = show();
    await loaded();
    await waitFor(() => expect(freshness()).toHaveTextContent(/^read (just now|\d+s ago), every 5 s$/));
    expect(within(panel('Quota headroom')).getByText('checked 1 min ago')).toBeInTheDocument();
    failing = true;
    await act(() => client.refetchQueries({ queryKey: ['overview'] }));
    await waitFor(() => expect(within(panel('Quota headroom')).getByText(/^kept from a read (just now|\d+ min ago), the last one failed$/)).toBeInTheDocument());
  });
});

describe('what the router sends', () => {
  it('OV-C188: reaches the page as text, never as markup', async () => {
    const path = '/code/<img src=x onerror="window.hacked=1">';
    router((p) =>
      p === '/api/projects'
        ? ok([project(path, 9_000_000)])
        : p === '/api/summary'
          ? ok(summary({ models: [{ ...summary().models[0]!, model: 'claude-<b>bold</b>' }] }))
          : p === '/api/requests'
            ? ok([request(1, { error: '<script>window.hacked=2</script>' }), ...REQUESTS.slice(1)])
            : undefined,
    );
    show('/?tab=requests&open=1');
    await loaded();
    const main = document.body;
    expect(await within(panel('Requests')).findByText('<script>window.hacked=2</script>')).toBeInTheDocument();
    await tabTo('Context', 'By model');
    expect(within(panel('By model')).getByText('<b>bold</b>')).toBeInTheDocument();
    await tabTo('Projects & sessions', 'Projects');
    expect(await within(panel('Projects')).findByText(path)).toBeInTheDocument();
    expect(main.querySelector('img, b, script')).toBeNull();
    expect((window as unknown as { hacked?: number }).hacked).toBeUndefined();
  });
});

describe('a hosted router', () => {
  const empty = (path: string) =>
    path === '/api/projects' || path === '/api/sessions' || path === '/api/requests' ? ok([]) : path === '/api/summary' ? ok(summary({ totals: totals({ requests: 0, first_seen: null, last_seen: null }), unattributed: 0 })) : undefined;

  it('OV-C203: the page learns what the router can do from /api/auth’s mode, never assuming a Mac', async () => {
    router((path) => empty(path) ?? (path === '/api/auth' ? ok(LOCAL) : undefined));
    const local = show('/?range=all&tab=projects');
    await loaded();
    await screen.findByRole('heading', { level: 2, name: 'Projects' });
    expect(await within(panel('Projects')).findByRole('link', { name: 'Settings → Claude Code' })).toHaveAttribute('href', '/settings/claude-code');
    local.unmount();

    router((path) => empty(path) ?? (path === '/api/auth' ? ok(HOSTED) : undefined));
    const hosted = show('/?range=all&tab=projects');
    await loaded();
    await screen.findByRole('heading', { level: 2, name: 'Projects' });
    expect(await within(panel('Projects')).findByRole('link', { name: 'Settings → Router' })).toHaveAttribute('href', '/settings/router');
    expect(within(panel('Projects')).queryByRole('link', { name: 'Settings → Claude Code' })).toBeNull();
    hosted.unmount();

    router((path) => empty(path) ?? (path === '/api/auth' ? dropped() : undefined));
    show('/?range=all&tab=projects');
    await loaded();
    await screen.findByRole('heading', { level: 2, name: 'Projects' });
    await new Promise((done) => setTimeout(done, 50));
    expect(within(panel('Projects')).queryByRole('link')).toBeNull();
  });

  it('OV-C208: a hosted router speaks of its deployment, and a local one in its own words', async () => {
    router((path) => (path === '/api/auth' ? ok(HOSTED) : undefined));
    const hosted = show();
    await loaded();
    expect(await screen.findByText('— Every caller must give the password')).toBeInTheDocument();
    hosted.unmount();

    router();
    show();
    await loaded();
    expect(await screen.findByText('— Reachable from your network; other devices must give the password')).toBeInTheDocument();

    expect(noteText(notes(HOSTED).store)).toMatch(/set by the deployment/);
    expect(noteText(notes(LOCAL).store)).toMatch(/^Everything above is stored locally/);
    expect(identityNote({ source: 'keychain' }, HOSTED)).toMatch(/A hosted router cannot sign with it\.$/);
    expect(identityNote({ source: 'keychain' }, LOCAL)).toMatch(/^Identity is read from the Claude Code login on this machine/);
  });
});

describe('going to the top', () => {
  it('OV-C200, SH-C48: "Back to top" appears 600 px down and returns there, at once when reduced motion is asked for', async () => {
    router();
    show();
    await loaded();
    expect(screen.queryByRole('button', { name: 'Back to top' })).toBeNull();
    const to = vi.fn();
    vi.stubGlobal('scrollTo', to);
    Object.defineProperty(window, 'scrollY', { configurable: true, value: 700 });
    act(() => {
      window.dispatchEvent(new Event('scroll'));
    });
    fireEvent.click(await screen.findByRole('button', { name: 'Back to top' }));
    expect(to).toHaveBeenLastCalledWith({ top: 0, behavior: 'smooth' });
    vi.stubGlobal('matchMedia', (q: string) => ({ matches: q === '(prefers-reduced-motion: reduce)', media: q, addEventListener: () => {}, removeEventListener: () => {} }));
    fireEvent.click(screen.getByRole('button', { name: 'Back to top' }));
    expect(to).toHaveBeenLastCalledWith({ top: 0, behavior: 'auto' });
    Object.defineProperty(window, 'scrollY', { configurable: true, value: 0 });
  });
});
