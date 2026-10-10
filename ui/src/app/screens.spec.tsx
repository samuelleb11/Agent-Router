// New for agent-router: the Shell contracts whose rule a screen carries (docs/web-console-contracts.md,
// "Shell") — a sentence that names a control links to it, the serving account leads to its controls,
// every figure says when it was measured, a failed refresh blanks nothing, a refusal stands beside the
// control that caused it, confirmations are in the page, "/" finds the search, and the address is the
// view — held on the screens themselves, against the router faked at `fetch`.
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';
import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useNavigate } from 'react-router';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { Shell } from '~/app/Shell';
import { Notices } from '~/app/notices';
import { dismiss, snapshot } from '~/app/notify';
import { json, refusal, renderScreen, stubFetch } from '~/test-utils';
import Overview from '~/screens/Overview';
import { anthropicBlock, HOUR, kimiBlock, MIN, NOW, QUOTA, refused, router } from '~/screens/Overview/fixtures';
import { quotaView } from '~/screens/Overview/quotaView';
import AccountsPage from '~/screens/Settings/Accounts';
import { ACCOUNTS as SETTINGS_ACCOUNTS, LOCAL as SETTINGS_LOCAL, ok as answers, stubRouter as settingsRouter } from '~/screens/Settings/testing';
import Sessions from '~/screens/Sessions';
import { FakeEventSource } from '~/screens/Sessions/fakes';
import { A, json as live, renderSessions, streamOf, stubRouter as sessionsRouter, working } from '~/screens/Sessions/kit';

/// Whole screens are drawn in jsdom beside every other spec in the run: generous limits, so a loaded
/// machine is not mistaken for a broken page.
vi.setConfig({ testTimeout: 60_000 });

// The activity chart's library is drawn by a browser, never by jsdom.
vi.mock('@carbon/charts-react', () => ({ StackedBarChart: () => <svg data-testid="chart" /> }));

/// Back, as a reader has it.
function Back() {
  const navigate = useNavigate();
  return (
    <button type="button" data-testid="back" onClick={() => void navigate(-1)}>
      back
    </button>
  );
}

const where = () => screen.getByTestId('location').textContent ?? '';
const back = () => act(async () => fireEvent.click(screen.getByTestId('back')));
const showOverview = (at = '/') =>
  renderScreen(
    <>
      <Overview />
      <Back />
    </>,
    { path: '/', at },
  );
const loaded = () => screen.findByRole('heading', { level: 2, name: 'Totals in scope' }, { timeout: 20_000 });
const panel = (name: string) => screen.getByRole('heading', { level: 2, name }).closest('section') as HTMLElement;
const freshness = () => document.querySelector('[data-freshness]') as HTMLElement;
const tile = (label: string) => within(panel('Totals in scope')).getByText(label).closest('.cds--tile') as HTMLElement;

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  act(() => {
    for (const n of snapshot()) dismiss(n.id);
  });
});

describe('a sentence that names a control links to it', () => {
  it('SH-C6: the Sessions notice that an editor is not wrapped names Settings → Claude Code, and links there', async () => {
    sessionsRouter((req) => (req.method === 'GET' && req.path === '/api/setup/shim' ? live({ targets: [{ extension: 'anthropic.claude-code-2.2.0', state: 'missing' }] }) : undefined));
    renderSessions('/live');
    expect(
      await screen.findByText('anthropic.claude-code-2.2.0 is not wrapped — sessions it starts are view-only until the wrapper is installed in Settings → Claude Code.'),
    ).toBeInTheDocument();
    const link = screen.getByRole('link', { name: 'Open Settings → Claude Code' });
    expect(link).toHaveAttribute('href', '/settings/claude-code');
    fireEvent.click(link);
    await waitFor(() => expect(where()).toBe('/settings/claude-code'));
  });

  it('SH-C17: the serving account’s name leads to Settings → Accounts, where an account is switched, edited or removed', async () => {
    router();
    showOverview();
    await loaded();
    const name = within(panel('Quota headroom')).getByRole('link', { name: 'secondary@example.test · max' });
    expect(name).toHaveAttribute('href', '/settings/accounts');
    fireEvent.click(name);
    await waitFor(() => expect(where()).toBe('/settings/accounts'));

    settingsRouter({ '/api/auth': answers(SETTINGS_LOCAL), '/api/accounts': answers(SETTINGS_ACCOUNTS), '/api/login/status': answers({ phase: 'failed', url: null }) });
    renderScreen(<AccountsPage />, { path: '/settings/accounts', at: '/settings/accounts' });
    const work = await screen.findByLabelText('Account work');
    for (const control of ['Edit', 'Remove']) expect(within(work).getByRole('button', { name: control })).toBeInTheDocument();
    // Switched in its provider's list box, above the cards (ST-C125).
    expect(screen.getByRole('combobox', { name: 'Requests go to' })).toBeInTheDocument();
  });
});

describe('the freshness badge and an open session', () => {
  it('SH-C10: the topbar’s badge says reconnecting while the open session’s stream is being replaced, and lets it go when the session closes', async () => {
    sessionsRouter((req) =>
      req.method === 'GET' && req.path === '/_router/health' ? live({ ok: true, pid: 1, version: '0.9.0', build: 'f00dfeed' }) : req.method === 'GET' && req.path === '/api/projects' ? live([]) : undefined,
    );
    vi.stubGlobal('EventSource', FakeEventSource);
    renderScreen(
      <>
        <Shell />
        <Sessions />
        <Back />
      </>,
      { path: '*', at: '/live' },
    );
    const badge = () => within(screen.getByRole('banner', { name: 'Agent Router' })).getByRole('status');
    await waitFor(() => expect(badge()).toHaveAttribute('data-live-mode', 'live'));
    const row = await waitFor(() => {
      const button = document.querySelector<HTMLButtonElement>(`[data-session="${A}"] > button`);
      expect(button).not.toBeNull();
      return button!;
    });
    await act(async () => fireEvent.click(row));
    await waitFor(() => streamOf(A));
    act(() => {
      streamOf(A).open().status(working());
    });
    expect(badge()).toHaveAttribute('data-live-mode', 'live');

    // The connection drops, and the browser is retrying it by itself.
    act(() => {
      streamOf(A).drop(FakeEventSource.CONNECTING);
    });
    await waitFor(() => expect(badge()).toHaveAttribute('data-live-mode', 'reconnecting'));
    expect(badge()).toHaveAccessibleName(/^Freshness: reconnecting — /);

    // Closed, the session's stream is nothing the badge speaks for.
    await back();
    await waitFor(() => expect(where()).toBe('/live'));
    await waitFor(() => expect(badge()).toHaveAttribute('data-live-mode', 'live'));
  });
});

describe('every figure says when it was measured', () => {
  it('SH-C21: the quota says how old it is and which source it came from — "checked … ago" from the usage endpoint, "from the last request" from response headers', () => {
    const read = (source: string, extra: object = {}) => ({ value: { ...QUOTA, source, ...extra }, at: NOW, kept: false, for: 'secondary' });
    expect(quotaView(read('usage-api', { age_ms: MIN }), null, null, NOW)?.source).toBe('checked 1 min ago');
    expect(quotaView(read('response-headers', { seen_at: NOW - 2 * MIN }), null, null, NOW)?.source).toBe('from the last request 2 min ago');
    const headers = anthropicBlock({ at: NOW - 3 * MIN, windows: { five_hour: { utilization: 0.4, resets_at: (NOW + HOUR) / 1000 }, seven_day: { utilization: 0.5, resets_at: (NOW + 24 * HOUR) / 1000 } } });
    expect(quotaView(null, headers, null, NOW)?.source).toBe('from the last request 3 min ago');
    // Kimi Code's block is its usage API's reading: checked, as the usage endpoint's is.
    expect(quotaView(null, kimiBlock({ at: NOW - 2 * MIN }), null, NOW)?.source).toBe('checked 2 min ago');
  });

  it('SH-C21: the Overview says when its figures were read, and the quota which source they came from', async () => {
    router();
    showOverview();
    await loaded();
    await waitFor(() => expect(freshness()).toHaveTextContent(/^read (just now|\d+s ago), every 5 s$/));
    expect(within(panel('Quota headroom')).getByText('checked 1 min ago')).toBeVisible();
  });
});

describe('a failed refresh blanks nothing', () => {
  it('SH-C14: every panel of the Overview keeps what it drew through a failed refresh, and the page says how old it is', async () => {
    let failing = false;
    router((path) => (failing && path === '/api/summary' ? refused('database is locked') : undefined));
    const { client } = showOverview('/?tab=projects');
    await loaded();
    await screen.findByRole('list', { name: 'Sessions' });
    const drawn = ['Quota headroom', 'All accounts', 'Totals in scope', 'Accounts', 'Projects', 'Sessions'].map((name) => [name, panel(name).textContent]);
    failing = true;
    await act(() => client.refetchQueries({ queryKey: ['overview'] }));
    await waitFor(() => expect(freshness()).toHaveAttribute('data-freshness', 'stale'));
    expect(freshness()).toHaveTextContent(/showing what was read (just now|\d+s ago)$/);
    for (const [name, text] of drawn) expect(panel(name!).textContent, name!).toBe(text);
    expect(within(tile('Requests')).getByText('412')).toBeInTheDocument();
  });
});

describe('a refusal stands beside its control', () => {
  const SESSION = 'a1b2c3d4-0000-4000-8000-000000000001';
  const WORDS = 'That session is not on a connected machine.';

  /// The shell at an open session, the palette's reads answered, and an archive held until released.
  function mountShell() {
    let release: (r: Response) => void = () => {};
    stubFetch({
      '/_router/health': () => json({ ok: true, pid: 1, version: '0.9.0', build: 'f00dfeed' }),
      '/api/projects': () => json([]),
      '/api/live/sessions': () => json([{ session_id: SESSION, cwd: '/Users/sample/code/agent-router', project: 'agent-router', git_branch: 'dev', title: 'Rebuild the palette', last_activity_ms: Date.now(), archived: false, wrapped: true }]),
      '/api/live/options': () => json({ models: [], efforts: [], modes: [] }),
      '/api/accounts': () => json([]),
      '/api/live/archive': () =>
        new Promise<Response>((done) => {
          release = done;
        }),
    });
    renderScreen(
      <>
        <Shell />
        <Notices />
      </>,
      { path: '*', at: `/live?session=${SESSION}` },
    );
    return { user: userEvent.setup(), release: (r: Response) => act(async () => release(r)) };
  }

  async function archiveFromPalette(user: ReturnType<typeof userEvent.setup>) {
    await user.click(screen.getByRole('button', { name: 'Search and commands (⌘K)' }));
    const field = await screen.findByRole('searchbox', { name: 'Search and commands' });
    await user.type(field, '> archive');
    await user.click(await screen.findByRole('option', { name: /Archive this session/ }));
  }

  it('SH-C24: a refused command shows the router’s words beside the list it was chosen from while that is on screen, and as a notice only once it is not', async () => {
    const { user, release } = mountShell();
    await archiveFromPalette(user);
    await release(refusal(WORDS, 503));
    const beside = await within(screen.getByRole('dialog')).findByRole('alert');
    expect(beside).toHaveTextContent('Could not archive that session');
    expect(beside).toHaveTextContent(WORDS);
    expect(snapshot()).toEqual([]);

    // Asked again, and the palette put away before the answer: the control is gone, so the refusal
    // is a notice, in the router's words.
    fireEvent.keyDown(screen.getByRole('dialog'), { key: 'Escape' });
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    await archiveFromPalette(user);
    fireEvent.keyDown(screen.getByRole('dialog'), { key: 'Escape' });
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    await release(refusal(WORDS, 503));
    await waitFor(() => expect(snapshot()).toEqual([expect.objectContaining({ kind: 'error', title: 'Could not archive that session', detail: WORDS })]));
    expect(await screen.findByText(WORDS)).toBeInTheDocument();
  });

  it('SH-C24: a refused Remove shows the router’s sentence on the account’s own card, never as a notice', async () => {
    settingsRouter({
      '/api/auth': answers(SETTINGS_LOCAL),
      '/api/accounts': answers(SETTINGS_ACCOUNTS),
      '/api/login/status': answers({ phase: 'failed', url: null }),
      'POST /api/accounts/remove': () => refusal('the secrets file is not writable', 500),
    });
    renderScreen(
      <>
        <AccountsPage />
        <Notices />
      </>,
      { path: '/settings/accounts', at: '/settings/accounts' },
    );
    fireEvent.click(within(await screen.findByLabelText('Account work')).getByRole('button', { name: 'Remove' }));
    fireEvent.click(within(await screen.findByRole('dialog')).getByRole('button', { name: 'Remove account' }));
    expect(await within(screen.getByLabelText('Account work')).findByText('the secrets file is not writable')).toBeInTheDocument();
    expect(snapshot()).toEqual([]);
  });
});

describe('confirmations are in the page', () => {
  const src = join(__dirname, '..');
  function sources(dir = src): string[] {
    return readdirSync(dir).flatMap((name) => {
      const path = join(dir, name);
      if (statSync(path).isDirectory()) return sources(path);
      return /\.(ts|tsx)$/.test(name) && !/\.spec\.(ts|tsx)$/.test(name) && !/^test-(utils|setup)\./.test(name) ? [path] : [];
    });
  }
  /// Code without its comments, where the rule is often named.
  const code = (path: string) =>
    readFileSync(path, 'utf8')
      .replace(/\/\*[\s\S]*?\*\//g, '')
      .replace(/^\s*\/\/.*$/gm, '');

  it('SH-C27: no source asks the browser for a native confirm, prompt or alert, or opens a window by script', () => {
    // A call, as the formatter writes one: the name and its parenthesis together. A sentence that
    // says "the usual prompt (any username)" is not one.
    const native = /(?<![\w.$])(?:(?:window|globalThis|self)\.)?(?:confirm|alert|prompt)\(|(?:window|globalThis|self)\.open\(/;
    expect(sources().filter((path) => native.test(code(path))).map((path) => relative(src, path))).toEqual([]);
  });

  it('SH-C27: in a frame that allows no modals and no popups, Remove still asks first, in the page, and the sign-in page’s link is shown whole and copyable beside the button that opens it', async () => {
    const natives = (['confirm', 'alert', 'prompt', 'open'] as const).map((name) =>
      vi.spyOn(window, name).mockImplementation(() => {
        throw new Error(`${name}() is refused in a sandboxed frame`);
      }),
    );
    const SIGN_IN = 'https://claude.ai/oauth/authorize?code=true&client_id=abc&state=xyz';
    const waiting = { phase: 'awaiting_sign_in', url: SIGN_IN, label: 'work2', account_id: null, email: null, error: null };
    const r = settingsRouter({
      '/api/auth': answers(SETTINGS_LOCAL),
      '/api/accounts': answers(SETTINGS_ACCOUNTS),
      '/api/login/status': answers({ phase: 'failed', url: null, label: '', account_id: null, email: null, error: null }),
      'POST /api/accounts/remove': answers(),
      'POST /api/login/start': answers(waiting),
    });
    renderScreen(<AccountsPage />, { path: '/settings/accounts', at: '/settings/accounts' });

    fireEvent.click(within(await screen.findByLabelText('Account work')).getByRole('button', { name: 'Remove' }));
    const dialog = await screen.findByRole('dialog');
    expect(document.body).toContainElement(dialog);
    expect(r.posts('/api/accounts/remove')).toEqual([]);
    fireEvent.click(within(dialog).getByRole('button', { name: 'Remove account' }));
    await waitFor(() => expect(r.posts('/api/accounts/remove')).toEqual([{ id: 'work' }]));

    r.answers['/api/login/status'] = answers(waiting);
    fireEvent.click(await screen.findByRole('button', { name: 'Add account' }));
    const form = screen.getByRole('form', { name: 'Add an account' });
    fireEvent.change(within(form).getByLabelText('Name'), { target: { value: 'work2' } });
    fireEvent.submit(form);
    const opens = await within(form).findByRole('link', { name: 'Open sign-in page' });
    expect(opens).toHaveAttribute('href', SIGN_IN);
    expect(within(form).getByLabelText('The sign-in link')).toHaveTextContent(SIGN_IN);
    // Beside it: one row holds the button that opens the page and the one that copies its link.
    expect(opens.parentElement).toContainElement(within(form).getByRole('button', { name: 'Copy link' }));
    for (const n of natives) expect(n).not.toHaveBeenCalled();
  });
});

describe('the keyboard', () => {
  it('SH-C49: "/" focuses the Overview’s search and selects its text from anywhere, never from a field, a select or anything editable, nor with ⌘, Ctrl or Alt held; the field shows a "/" hint', async () => {
    router();
    showOverview('/?tab=search');
    await loaded();
    const box = screen.getByRole('searchbox', { name: 'Search the transcripts' }) as HTMLInputElement;
    fireEvent.change(box, { target: { value: 'kestrel' } });
    act(() => box.blur());

    for (const held of ['metaKey', 'ctrlKey', 'altKey'] as const) {
      expect(fireEvent.keyDown(document.body, { key: '/', [held]: true }), held).toBe(true);
      expect(box, held).not.toHaveFocus();
    }

    // A slash typed where a slash belongs stays there: the shortcut neither moves the focus nor
    // swallows the key.
    const editable = document.createElement('div');
    editable.setAttribute('contenteditable', 'true');
    editable.tabIndex = 0;
    // jsdom does not derive this from the attribute; a browser does.
    Object.defineProperty(editable, 'isContentEditable', { value: true });
    const select = document.createElement('select');
    select.append(new Option('one'));
    for (const el of [document.createElement('input'), document.createElement('textarea'), select, editable]) {
      document.body.append(el);
      act(() => el.focus());
      expect(fireEvent.keyDown(el, { key: '/' }), el.tagName).toBe(true);
      expect(el, el.tagName).toHaveFocus();
      el.remove();
    }

    expect(fireEvent.keyDown(document.body, { key: '/' })).toBe(false);
    expect(box).toHaveFocus();
    expect([box.selectionStart, box.selectionEnd]).toEqual([0, 'kestrel'.length]);
    expect(within(panel('Search the transcripts')).getByText('/', { selector: 'kbd' })).toBeInTheDocument();
  });
});

describe('the address is the view', () => {
  it('SH-C53: the Overview’s scope, range, open tab and request filters are its address: a change of scope, range or tab pushes, so Back undoes it, a filter replaces, and the address reloads as itself', async () => {
    router();
    const first = showOverview('/?tab=projects');
    await loaded();
    fireEvent.click(within(await screen.findByRole('list', { name: 'Projects' })).getByRole('button', { name: 'agent-router' }));
    await waitFor(() => expect(where()).toBe('/?project=%2Fcode%2Fagent-router&tab=projects'));
    fireEvent.click(within(screen.getByRole('tablist', { name: 'Time range' })).getByRole('tab', { name: '24h' }));
    await waitFor(() => expect(where()).toBe('/?project=%2Fcode%2Fagent-router&range=24h&tab=projects'));
    fireEvent.click(screen.getByRole('tab', { name: 'Requests' }));
    await waitFor(() => expect(where()).toBe('/?project=%2Fcode%2Fagent-router&range=24h&tab=requests'));
    fireEvent.click(within(await screen.findByRole('group', { name: 'Stop reasons' })).getByRole('button', { name: /^tool_use/ }));
    await waitFor(() => expect(where()).toBe('/?project=%2Fcode%2Fagent-router&range=24h&tab=requests&stop=tool_use'));

    // The filter replaced the entry the tab pushed: Back undoes the tab, filter and all, then the
    // range, then the scope.
    await back();
    await waitFor(() => expect(where()).toBe('/?project=%2Fcode%2Fagent-router&range=24h&tab=projects'));
    await back();
    await waitFor(() => expect(where()).toBe('/?project=%2Fcode%2Fagent-router&tab=projects'));
    await back();
    await waitFor(() => expect(where()).toBe('/?tab=projects'));
    first.unmount();

    // A link pastes and reloads as the view it was.
    showOverview('/?project=%2Fcode%2Fnorthstar&range=7d&tab=requests&stop=tool_use');
    await loaded();
    expect(screen.getByRole('button', { name: 'Project: northstar' })).toBeInTheDocument();
    expect(within(screen.getByRole('tablist', { name: 'Time range' })).getByRole('tab', { name: '7d' })).toHaveAttribute('aria-selected', 'true');
    expect(within(screen.getByRole('tablist', { name: 'Overview sections' })).getByRole('tab', { name: 'Requests' })).toHaveAttribute('aria-selected', 'true');
    expect(within(await screen.findByRole('group', { name: 'Stop reasons' })).getByRole('button', { name: /^tool_use/ })).toHaveAttribute('aria-pressed', 'true');
  });

  it('SH-C53: on Sessions the open session and Find are the address: opening either pushes, so Back undoes it, typing in Find replaces, and the address reloads as itself', async () => {
    sessionsRouter((req) => (req.method === 'GET' && req.path === '/api/search' ? live({ query: req.query.get('q'), expression: req.query.get('q'), sessions: [], hits: [] }) : undefined));
    const first = renderSessions('/live', <Back />);
    const row = await waitFor(() => {
      const button = document.querySelector<HTMLButtonElement>(`[data-session="${A}"] > button`);
      expect(button).not.toBeNull();
      return button!;
    });
    await act(async () => fireEvent.click(row));
    await waitFor(() => expect(where()).toBe(`/live?session=${A}`));
    await back();
    await waitFor(() => expect(where()).toBe('/live'));

    const list = within(document.querySelector<HTMLElement>('[data-pane="list"]')!);
    await act(async () => fireEvent.click(list.getByRole('button', { name: 'Find' })));
    await waitFor(() => expect(where()).toBe('/live?find=&scope=all'));
    fireEvent.change(screen.getByRole('searchbox'), { target: { value: 'kestrel' } });
    await waitFor(() => expect(where()).toBe('/live?find=kestrel&scope=all'));
    await back();
    await waitFor(() => expect(where()).toBe('/live'));
    first.unmount();

    renderSessions(`/live?session=${A}`);
    await waitFor(() => expect(document.querySelector(`[data-session-view="${A}"]`)).not.toBeNull());
    expect(where()).toBe(`/live?session=${A}`);
  });
});
