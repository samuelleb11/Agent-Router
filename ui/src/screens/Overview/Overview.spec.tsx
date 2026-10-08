// New for agent-router: the Overview as a reader meets it (docs/web-console-contracts.md, "Overview"):
// its sections, its address, its search, its quota, its lists and requests, and how it reads the
// router — one snapshot per scope, every five seconds — against a router faked at `fetch`.
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { useNavigate } from 'react-router';
import { keys } from '~/app/query';
import { identityNote, notes, noteText } from '~/model/settings';
import { renderScreen } from '~/test-utils';
import Overview from './index';
import {
  ACCOUNTS,
  dropped,
  forecast,
  forecastAccount,
  held,
  HOSTED,
  LOCAL,
  NOW,
  ok,
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
  it('OV-C1: keeps every section the dashboard shows, in its order, the scope bar first', async () => {
    router();
    show();
    await loaded();
    expect(screen.getAllByRole('heading', { level: 2 }).map((h) => h.textContent)).toEqual([
      'Search the transcripts',
      'Quota headroom',
      'All accounts',
      'Totals in scope',
      'Activity over time',
      'What the model read',
      'What fills the context window',
      'By model',
      'Main thread vs subagents',
      'Accounts',
      'Projects',
      'Sessions',
      'Tool output the model read',
      'Tool calls made',
      'Requests',
    ]);
    const bar = screen.getByRole('region', { name: 'Scope' });
    expect(bar.compareDocumentPosition(panel('Search the transcripts')) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  it('OV-C2: every heading carries its caption as visible text beside it', async () => {
    router();
    show();
    await loaded();
    const captions: Record<string, string | RegExp> = {
      'Search the transcripts': 'every message, tool call and result ever captured',
      'Quota headroom': /^the account that is serving right now/,
      'All accounts': '2 accounts · spend and tokens add up, quota does not',
      'Totals in scope': /^412 requests · /,
      'Activity over time': '12.40M prompt tokens · 412 requests',
      'What the model read': 'cache vs. paid in full',
      'What fills the context window': 'estimate · by character share',
      'By model': 'prompt tokens read',
      'Main thread vs subagents': 'share of prompt tokens',
      Projects: '2 projects · click one to scope the page',
      'Tool output the model read': 'est. tokens',
      'Tool calls made': 'by the model',
    };
    for (const [heading, caption] of Object.entries(captions)) {
      const h = screen.getByRole('heading', { level: 2, name: heading });
      // Beside it: the heading's own row, as text, never a title.
      expect(h.nextElementSibling?.textContent, heading).toMatch(caption);
      expect(h.parentElement?.querySelector('[title]'), heading).toBeNull();
    }
  });

  it('OV-C3: every control is visible at rest, none in an overflow menu or behind a disclosure', async () => {
    router();
    show('/?project=%2Fcode%2Fagent-router');
    await loaded();
    expect(within(screen.getByRole('tablist', { name: 'Time range' })).getAllByRole('tab').map((t) => t.textContent)).toEqual(['all', '1h', '24h', '7d', '30d']);
    expect(screen.getByRole('button', { name: 'Clear scope' })).toBeVisible();
    expect(within(screen.getByRole('tablist', { name: 'What to search' })).getAllByRole('tab')).toHaveLength(5);
    expect(within(screen.getByRole('tablist', { name: 'Which requests' })).getAllByRole('tab').map((t) => t.textContent)).toEqual(['All 30', 'Errors 1', 'Subagents 1', 'Slow 1']);
    expect(within(screen.getByRole('group', { name: 'Models' })).getAllByRole('button')).toHaveLength(2);
    expect(within(screen.getByRole('group', { name: 'Stop reasons' })).getAllByRole('button')).toHaveLength(3);
    expect(screen.getByRole('button', { name: 'Show 10 more' })).toBeVisible();
    expect(document.querySelector('.cds--overflow-menu, details, [aria-expanded="false"][aria-haspopup="menu"]')).toBeNull();
  });

  it('OV-C11, OV-C86: a panel the page turns off is not in the page at all', async () => {
    router((path) => (path === '/api/forecast' ? ok(forecast([])) : path === '/api/summary' ? ok(summary({ by_account: summary().by_account.slice(0, 1) })) : undefined));
    show();
    await loaded();
    expect(screen.queryByRole('heading', { name: 'All accounts' })).toBeNull();
    expect(screen.queryByRole('heading', { name: 'Accounts' })).toBeNull();
    expect(screen.queryByText('Spend, every account')).toBeNull();
    expect(document.querySelectorAll('#main-content [hidden], [hidden] [data-panel]')).toHaveLength(0);
  });
});

describe('the address', () => {
  it('OV-C30: a scope or range change pushes a history entry, so Back un-scopes the page', async () => {
    router();
    show();
    await loaded();
    fireEvent.click(within(panel('Projects')).getByRole('button', { name: 'agent-router' }));
    await waitFor(() => expect(where()).toBe('/?project=%2Fcode%2Fagent-router&range=all'));
    fireEvent.click(within(screen.getByRole('tablist', { name: 'Time range' })).getByRole('tab', { name: '24h' }));
    await waitFor(() => expect(where()).toBe('/?project=%2Fcode%2Fagent-router&range=24h'));
    fireEvent.click(screen.getByRole('button', { name: 'test: back' }));
    await waitFor(() => expect(where()).toBe('/?project=%2Fcode%2Fagent-router&range=all'));
    fireEvent.click(screen.getByRole('button', { name: 'test: back' }));
    await waitFor(() => expect(where()).toBe('/?range=all'));
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
    show('/?project=%2Fcode%2Fnorthstar&range=7d&open=3');
    await loaded();
    const asked = r.calls('/api/summary')[0]!.query;
    expect([asked.get('project'), asked.has('since')]).toEqual(['/code/northstar', true]);
    expect(within(panel('Requests')).getAllByRole('button', { expanded: true })).toHaveLength(1);

    // An old bookmark opens the page unscoped at the range an address with none starts at.
    const old = router();
    show('/#a=primary&p=%2Fcode%2Fnorthstar&r=7d');
    await waitFor(() => expect(old.calls('/api/summary').length).toBeGreaterThan(0));
    expect([...old.calls('/api/summary')[0]!.query.keys()].sort()).toEqual(['since', 'tz']);

    // That range is the last hour, and choosing all puts it in the address.
    const bare = router();
    show('/');
    await waitFor(() => expect(bare.calls('/api/summary').length).toBeGreaterThan(0));
    const since = Number(bare.calls('/api/summary')[0]!.query.get('since'));
    expect(Math.abs(Date.now() - 3600e3 - since)).toBeLessThan(60e3);
  });

  it('OV-C33: a scope change reads at once and returns the request list to its first twenty rows', async () => {
    const r = router();
    show();
    await loaded();
    fireEvent.click(screen.getByRole('button', { name: 'Show 10 more' }));
    expect(within(panel('Requests')).getAllByRole('listitem')).toHaveLength(30);
    const before = r.calls('/api/summary').length;
    // Read at once: by the time the press has been handled, not at the next tick.
    await act(async () => {
      fireEvent.click(within(panel('Projects')).getByRole('button', { name: 'northstar' }));
    });
    expect(r.calls('/api/summary').length).toBe(before + 1);
    expect(r.calls('/api/summary').at(-1)!.query.get('project')).toBe('/code/northstar');
    await waitFor(() => expect(within(panel('Requests')).getAllByRole('listitem')).toHaveLength(20));
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
    show();
    await loaded();
    expect(box()).toHaveAttribute('placeholder', 'a word, a phrase, an identifier…');
    expect(box()).toHaveAttribute('spellcheck', 'false');
    expect(box()).toHaveAttribute('autocomplete', 'off');
    expect(within(panel('Search the transcripts')).getByText('every message, tool call and result ever captured')).toBeVisible();
  });

  it('OV-C37: "/" focuses the box and selects its text from anywhere, except in a field or with a modifier, beside a visible "/" hint', async () => {
    router();
    show();
    await loaded();
    type('kestrel');
    act(() => box().blur());
    fireEvent.keyDown(document, { key: '/', metaKey: true });
    expect(box()).not.toHaveFocus();
    fireEvent.keyDown(document, { key: '/' });
    expect(box()).toHaveFocus();
    expect([(box() as HTMLInputElement).selectionStart, (box() as HTMLInputElement).selectionEnd]).toEqual([0, 7]);
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
    show();
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
    show();
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
    const { client } = show();
    await loaded();
    type('kestrel');
    await waitFor(() => expect(r.calls('/api/search')).toHaveLength(1));
    const reads = r.calls('/api/summary').length;
    await act(() => client.refetchQueries({ queryKey: keys.overview({ account: null, project: null, session: null, range: 'all' }) }));
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
    show();
    await loaded();
    type('kestrel');
    const hits = await screen.findByRole('list', { name: 'Best matches' });
    const hit = within(hits).getByRole('button');
    expect(hit).toHaveTextContent(/^tool call assistant · agent-router · aaaaaaaa · \d{1,2}:\d{2}/);
    expect(within(hit).getByText('kestrel', { selector: 'mark' })).toBeInTheDocument();
    expect(scss).toMatch(/\.snippet \{[^}]*-webkit-line-clamp: 3;[^}]*line-clamp: 3;/);
    fireEvent.click(hit);
    await waitFor(() => expect(where()).toBe(`/?project=%2Fcode%2Fagent-router&session=${S1}&range=all`));
  });

  it('OV-C53: the search’s session rows mark the session the page is scoped to as soon as it changes', async () => {
    const r = router((path, q) => (path === '/api/search' ? ok(reply(q.get('q') ?? '')) : undefined));
    show();
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
  it('OV-C54: describes the account serving now whatever the page is scoped to, and its caption names it', async () => {
    const r = router();
    show('/?account=primary&project=%2Fcode%2Fnorthstar');
    await loaded();
    const quota = panel('Quota headroom');
    expect(within(quota).getByRole('link', { name: 'secondary@example.test · max' })).toHaveAttribute('href', '/settings/accounts');
    expect(within(quota).getByText('41% used · resets in 2h 00m')).toBeInTheDocument();
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

describe('all accounts', () => {
  it('OV-C96: an account removed from the router keeps its card while it has history to report', async () => {
    router((path) =>
      path === '/api/accounts' ? ok(ACCOUNTS.slice(0, 1)) : path === '/api/forecast' ? ok(forecast([forecastAccount('secondary'), forecastAccount('retired', { five_hour: null, seven_day: null, label: 'retired' })])) : undefined,
    );
    show();
    await loaded();
    expect([...document.querySelectorAll('[data-account-card]')].map((c) => c.getAttribute('data-account-card'))).toEqual(['secondary', 'retired']);
  });
});

describe('Totals in scope', () => {
  it('OV-C107: Errors offers a visible "List the N errors" that sets the filter to errors, returns that list to its first page and brings it into view', async () => {
    const into = vi.spyOn(Element.prototype, 'scrollIntoView').mockImplementation(() => {});
    router();
    show();
    await loaded();
    fireEvent.click(screen.getByRole('button', { name: 'Show 10 more' }));
    const button = within(tile('Errors')).getByRole('button', { name: 'List the 3 errors' });
    expect(button.tagName).toBe('BUTTON');
    fireEvent.click(button);
    await waitFor(() => expect(where()).toBe('/?range=all&kind=errors'));
    expect(within(panel('Requests')).getAllByRole('listitem')).toHaveLength(1);
    await waitFor(() => expect(into).toHaveBeenCalled());
    expect(into.mock.contexts.at(-1)).toBe(document.getElementById('requests'));
  });
});

describe('the lists', () => {
  it('OV-C132: lists keep the router’s order', async () => {
    router((path) => (path === '/api/projects' ? ok([project('/code/small', 10), project('/code/large', 9_000_000)]) : undefined));
    show();
    await loaded();
    expect(within(panel('Projects')).getAllByRole('button').map((b) => b.getAttribute('aria-labelledby') && b.textContent?.split('/')[0])).toEqual(['small', 'large']);
  });

  it('OV-C136: a project with no recorded path is "(unresolved)" and scopes like any other', async () => {
    const r = router((path) => (path === '/api/projects' ? ok([...PROJECTS, project(null, 1_000)]) : undefined));
    show();
    await loaded();
    fireEvent.click(within(panel('Projects')).getByRole('button', { name: '(unresolved)' }));
    await waitFor(() => expect(r.calls('/api/summary').some((c) => c.query.get('project') === '(unresolved)')).toBe(true));
    expect(where()).toBe('/?project=%28unresolved%29&range=all');
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
    show();
    await loaded();
    expect(screen.queryByLabelText('Where this project ran')).toBeNull();
    fireEvent.click(within(panel('Projects')).getByRole('button', { name: 'ui-kit-web' }));
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
    show();
    await loaded();
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
    const filtered = show('/?kind=errors&stop=end_turn');
    await loaded();
    expect(within(panel('Requests')).getByText('No request matches these filters.')).toBeInTheDocument();
    fireEvent.click(within(panel('Requests')).getByRole('button', { name: 'Clear the filters' }));
    await waitFor(() => expect(where()).toBe('/'));
    filtered.unmount();

    router((path) => (path === '/api/requests' ? ok([]) : undefined));
    show();
    await loaded();
    expect(within(panel('Requests')).getByText('No requests for this scope.')).toBeInTheDocument();
  });

  it('OV-C153: a model filter whose model has left the loaded rows is cleared from the address, never left filtering to nothing', async () => {
    router();
    show('/?model=claude-haiku-4-5&kind=subagents');
    await loaded();
    await waitFor(() => expect(where()).toBe('/?kind=subagents'));
    expect(within(panel('Requests')).getAllByRole('listitem')).toHaveLength(1);
  });

  it('OV-C154: a filter change returns the list to its first twenty rows', async () => {
    router();
    show();
    await loaded();
    fireEvent.click(screen.getByRole('button', { name: 'Show 10 more' }));
    expect(within(panel('Requests')).getAllByRole('listitem')).toHaveLength(30);
    fireEvent.click(within(screen.getByRole('group', { name: 'Stop reasons' })).getByRole('button', { name: /^tool_use/ }));
    await waitFor(() => expect(within(panel('Requests')).getAllByRole('listitem')).toHaveLength(20));
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
    show('/?open=1');
    await loaded();
    const main = document.body;
    expect(main.querySelector('img, b, script')).toBeNull();
    expect(within(panel('Projects')).getByText(path)).toBeInTheDocument();
    expect(within(panel('By model')).getByText('<b>bold</b>')).toBeInTheDocument();
    expect(within(panel('Requests')).getByText('<script>window.hacked=2</script>')).toBeInTheDocument();
    expect((window as unknown as { hacked?: number }).hacked).toBeUndefined();
  });
});

describe('a hosted router', () => {
  const empty = (path: string) =>
    path === '/api/projects' || path === '/api/sessions' || path === '/api/requests' ? ok([]) : path === '/api/summary' ? ok(summary({ totals: totals({ requests: 0, first_seen: null, last_seen: null }), unattributed: 0 })) : undefined;

  it('OV-C203: the page learns what the router can do from /api/auth’s mode, never assuming a Mac', async () => {
    router((path) => empty(path) ?? (path === '/api/auth' ? ok(LOCAL) : undefined));
    const local = show();
    await loaded();
    expect(await within(panel('Projects')).findByRole('link', { name: 'Settings → Claude Code' })).toHaveAttribute('href', '/settings/claude-code');
    local.unmount();

    router((path) => empty(path) ?? (path === '/api/auth' ? ok(HOSTED) : undefined));
    const hosted = show();
    await loaded();
    expect(await within(panel('Projects')).findByRole('link', { name: 'Settings → Router' })).toHaveAttribute('href', '/settings/router');
    expect(within(panel('Projects')).queryByRole('link', { name: 'Settings → Claude Code' })).toBeNull();
    hosted.unmount();

    router((path) => empty(path) ?? (path === '/api/auth' ? dropped() : undefined));
    show();
    await loaded();
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
