// New for agent-router: Kimi Code on the Sessions screen (docs/web-console-contracts.md, "Kimi Code
// sessions" and "Starting a session"), against the router's answers: a session's client named one
// way everywhere, a Kimi Code row's client and reach, what a watched and a hosted session take from
// here, Kimi Code's modes, and starting a hosted session.
import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { SessionRow as OverviewRow } from '~/data/overview';
import { client, clientLabel } from '~/model/format';
import type { MachineRow } from '~/model/machine';
import { UNSCOPED } from '~/model/scope';
import { KIMI_NO_ASIDE, KIMI_NO_EFFORT, KIMI_QUESTION, KIMI_WATCHED, controlsOf, rowView, startMachines } from '~/model/session';
import { session as overviewSession, S1 } from '~/screens/Overview/fixtures';
import { sessionRows } from '~/screens/Overview/panels';
import { press, renderScreen } from '~/test-utils';
import { Facts } from './Facts';
import { FakeEventSource } from './fakes';
import { A, AT, drain, json, renderSessions, session, SESSIONS, streamOf, stubRouter, working, type Asked } from './kit';
import { NO_MACHINE, NO_MODEL, START, START_TITLE } from './StartSession';

beforeEach(() => FakeEventSource.reset());
afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

/// A Kimi Code session the router hosts, and one it only watches.
const HOSTED = 'session_0a0a0a0a-0000-4000-8000-000000000001';
const WATCHED = 'session_fd31bc41-dc2f-432b-8be3-2ae7f81240a6';

const kimi = (id: string, hosted: boolean) =>
  session(id, {
    client: 'kimi-code',
    hosted,
    wrapped: false,
    cwd: '/Users/sample/code/kimi-app',
    project: '-Users-sample-code-kimi-app',
    git_branch: null,
    title: hosted ? 'Hosted here' : 'In its own terminal',
  });
const ROWS = () => [...SESSIONS(), kimi(HOSTED, true), kimi(WATCHED, false)];

const KIMI_STATS = { model: 'k3', mode: 'manual', total: 12_000, output: 800, cache_read: 0, turns: 2, cost: null };
const APPROVAL = { pending: true, request_id: 'rq-k', tool: 'Shell', question: false, display_name: 'Shell', description: 'List the build', input: null, suggestions: [], count: 1 };
const ASK = { questions: [{ question: 'Which build?', header: 'Build', options: [{ label: 'Debug' }, { label: 'Release' }] }] };

/// The router: these rows, Kimi Code's totals for its sessions, an approval waiting, and `more` first.
function kimiRouter(more: (req: Asked) => Response | Promise<Response> | undefined = () => undefined) {
  return stubRouter((req) => {
    const mine = more(req);
    if (mine) return mine;
    if (req.method !== 'GET') return undefined;
    if (req.path === '/api/live/sessions') return json(ROWS());
    const id = req.query.get('session') ?? '';
    if (!id.startsWith('session_')) return undefined;
    if (req.path === '/api/live/stats') return json(KIMI_STATS);
    if (req.path === '/api/live/prompt') return json(APPROVAL);
    return undefined;
  });
}

/// The open session, its stream speaking a running turn.
async function opened(id: string) {
  await waitFor(() => streamOf(id));
  act(() => {
    streamOf(id).open().status(working());
  });
}
const facts = () => within(screen.getByRole('region', { name: 'This session’s totals and settings' }));
const box = () => screen.getByRole('textbox', { name: 'Message to this session' });
const rowOf = (id: string) => {
  const li = document.querySelector<HTMLElement>(`[data-session="${id}"]`);
  if (!li) throw new Error(`no list row for ${id}`);
  return li;
};
const cardOf = async (what: string) => (await screen.findByText(what)).closest('.cds--actionable-notification') as HTMLElement;

describe('a session’s client', () => {
  it('SE-C234: a client is named one way wherever the console names it, from the router’s id — Claude Code’s as OV-C187 names them, Kimi Code’s by whose they are', () => {
    expect(['claude-code-cli', 'claude-code-vscode', 'kimi-code-cli', 'kimi-code-vscode', 'kimi-code-web'].map(clientLabel)).toEqual([
      'CLI',
      'VS Code',
      'Kimi Code CLI',
      'Kimi Code VS Code',
      'Kimi Code web',
    ]);
    // An id the console does not know is not named, and `constructor` is not an id.
    expect([clientLabel('sdk-ts'), clientLabel('constructor'), clientLabel(null)]).toEqual([null, null, null]);
    // The router's id wins; without one, the entrypoint as OV-C187 reads it.
    expect([client('kimi-code-cli/2.1.1 (web)', 'kimi-code-web'), client('claude-cli/2.1.0 (external, claude-vscode)'), client('claude-cli/2.1.0'), client(null, null)]).toEqual([
      'Kimi Code web',
      'VS Code',
      'CLI',
      null,
    ]);
    // The Overview's session lists say the same words.
    const chip = (over: Partial<OverviewRow>) => sessionRows([overviewSession(S1, over)], UNSCOPED)[0]?.chipsOf.client;
    expect(chip({ entrypoint: 'kimi-code-vscode/0.8.1', client: 'kimi-code-vscode' })).toEqual({ name: 'Kimi Code VS Code', entrypoint: 'kimi-code-vscode/0.8.1' });
    expect(chip({ entrypoint: 'kimi-code-cli/2.1.1', client: 'kimi-code-cli' })?.name).toBe('Kimi Code CLI');
    expect(chip({ entrypoint: 'claude-cli/2.1.0 (external, claude-vscode)', client: 'claude-code-vscode' })?.name).toBe('VS Code');
    // A row from before the router named clients is read from its entrypoint, as it always was.
    expect(chip({ entrypoint: 'claude-cli/2.1.0' })?.name).toBe('CLI');
  });

  it('SE-C235: a Kimi Code row names its client beside live or view — hosted is live and "Kimi Code web", watched is view and "Kimi Code" — and a Claude Code row names none', async () => {
    kimiRouter();
    renderSessions();
    await waitFor(() => rowOf(HOSTED));
    const hosted = rowOf(HOSTED);
    expect(hosted.querySelector('[data-reach]')).toHaveTextContent(/^live$/);
    expect(hosted.querySelector('[data-client]')).toHaveTextContent(/^Kimi Code web$/);
    const watched = rowOf(WATCHED);
    expect(watched.querySelector('[data-reach]')).toHaveTextContent(/^view$/);
    expect(watched.querySelector('[data-client]')).toHaveTextContent(/^Kimi Code$/);
    // Read aloud in order: its name, live or view, its client, its age.
    expect(hosted).toHaveTextContent(/live, Kimi Code web, moved/);
    // Claude Code's rows are as they were.
    expect(rowOf(A).querySelector('[data-client]')).toBeNull();
    expect(rowOf(A).querySelector('[data-reach]')).toHaveTextContent(/^live$/);
    // `wrapped` says nothing of a Kimi Code session; whether the router hosts it does.
    expect(rowView(kimi(HOSTED, true), AT).reach).toBe('live');
    expect(rowView({ ...kimi(WATCHED, false), wrapped: true }, AT).reach).toBe('view');
    // The help beside the list says what they mean for each.
    await press(screen.getByRole('button', { name: 'What live and view mean' }));
    expect(await screen.findByText(/A Kimi Code session is/)).toHaveTextContent('when this router hosts it');
  });
});

describe('a session’s providers', () => {
  it('SE-C242: a Kimi Code row wears Kimi Code’s mark beside its client, which names it, so the mark is not heard again; a Claude Code row names no provider, rather than guess', async () => {
    kimiRouter();
    renderSessions();
    await waitFor(() => rowOf(WATCHED), { timeout: 10_000 });
    for (const [id, words] of [
      [HOSTED, 'Kimi Code web'],
      [WATCHED, 'Kimi Code'],
    ] as const) {
      const client = rowOf(id).querySelector<HTMLElement>('[data-client]')!;
      expect(client.querySelector('[data-provider="kimi"] [data-glyph="kimi"]'), id).not.toBeNull();
      expect(client.querySelector('[data-provider="kimi"]'), id).toHaveAttribute('aria-hidden', 'true');
      // The client's words are as they were (SE-C235).
      expect(client).toHaveTextContent(new RegExp(`^${words}$`));
    }
    // A Claude Code row says neither its client nor a provider: its requests can go to either.
    expect(rowOf(A).querySelector('[data-provider]')).toBeNull();
    expect(rowView(kimi(HOSTED, true), AT).provider).toBe('kimi');
    expect(rowView(session(A), AT).provider).toBeNull();
  });

  it('SE-C242: an open Kimi Code session’s facts say its client with Kimi Code’s mark, or before the list has listed it Kimi Code’s mark and name; a Claude Code session’s name no provider', async () => {
    kimiRouter();
    const hosted = renderSessions(`/live?session=${HOSTED}`);
    await opened(HOSTED);
    const client = await facts().findByText('Kimi Code web', {}, { timeout: 10_000 });
    expect(client.querySelector('[data-provider="kimi"]')).toHaveAttribute('aria-hidden', 'true');
    hosted.unmount();

    // Just started, not yet listed: known as Kimi Code's by its id, and said by its mark and name.
    const unlisted = renderScreen(<Facts session={WATCHED} row={null} />);
    const named = await facts().findByText('Kimi Code', {}, { timeout: 10_000 });
    expect(named.closest('[data-provider]')).toHaveAttribute('data-provider', 'kimi');
    expect(named.closest('[data-provider]')?.querySelector('[data-glyph="kimi"]')).not.toBeNull();
    unlisted.unmount();

    kimiRouter();
    renderSessions(`/live?session=${A}`);
    await opened(A);
    await waitFor(() => expect(facts().getByText('opus-4-1')).toBeInTheDocument(), { timeout: 10_000 });
    expect(screen.getByRole('region', { name: 'This session’s totals and settings' }).querySelector('[data-provider]')).toBeNull();
  });
});

describe('what a Kimi Code session takes from here', () => {
  it('SE-C236: a watched one’s composer, Send, Stop, card, model and mode are drawn disabled, the router’s sentence beside them, and nothing is sent', async () => {
    const router = kimiRouter();
    renderSessions(`/live?session=${WATCHED}`);
    await opened(WATCHED);
    await waitFor(() => expect(box()).toBeDisabled());
    expect(screen.getByRole('button', { name: 'Send' })).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Stop the turn' })).toBeDisabled();
    await waitFor(() => expect(facts().getByRole('combobox', { name: 'Model' })).toBeDisabled());
    expect(facts().getByRole('combobox', { name: 'Mode' })).toBeDisabled();
    expect(facts().getByText(KIMI_WATCHED)).toBeInTheDocument();
    const card = await cardOf('Shell is waiting for permission');
    expect(within(card).getAllByRole('button').map((b) => [b.textContent, (b as HTMLButtonElement).disabled])).toEqual([
      ['Allow', true],
      ['Deny', true],
    ]);
    expect(within(card).getByText(KIMI_WATCHED)).toBeInTheDocument();
    // Beside the composer too, in the router's words exactly.
    const composer = document.querySelector<HTMLElement>('[data-composer]') as HTMLElement;
    expect(within(composer).getByText(KIMI_WATCHED)).toBeInTheDocument();
    expect(KIMI_WATCHED).toBe('This Kimi Code session runs in its own terminal or in VS Code, which takes no outside control. Start a hosted session to drive one.');
    // A question in its feed is held back with the same sentence, which the router answers first.
    act(() => {
      streamOf(WATCHED).row({ role: 'assistant', kind: 'ask', t: 'T1', ask: ASK });
    });
    expect(await screen.findByRole('button', { name: 'Submit answers' })).toBeDisabled();
    await press(screen.getByRole('button', { name: 'Stop the turn' }));
    await drain();
    expect(router.asked.filter((r) => r.method === 'POST')).toEqual([]);
  });

  it('SE-C236: a hosted one’s composer, Send, Stop, card, model and mode are on, as a wrapped Claude Code session’s are', async () => {
    const router = kimiRouter((req) => (req.method === 'POST' && req.path === '/api/live/send' ? json({ ok: true, via: 'kimi-web', framed: false }) : undefined));
    renderSessions(`/live?session=${HOSTED}`);
    await opened(HOSTED);
    await waitFor(() => expect(facts().getByRole('combobox', { name: 'Model' })).toBeEnabled());
    expect(facts().getByRole('combobox', { name: 'Mode' })).toBeEnabled();
    expect(box()).toBeEnabled();
    expect(screen.getByRole('button', { name: 'Stop the turn' })).toBeEnabled();
    expect(screen.queryByText(KIMI_WATCHED)).toBeNull();
    const card = await cardOf('Shell is waiting for permission');
    expect(within(card).getAllByRole('button').every((b) => !(b as HTMLButtonElement).disabled)).toBe(true);
    fireEvent.change(box(), { target: { value: 'go on' } });
    await press(screen.getByRole('button', { name: 'Send' }));
    await waitFor(() => expect(router.posts('/api/live/send').map((p) => p.body)).toEqual([{ session: HOSTED, text: 'go on' }]));
    // Before the list has said whether the router hosts it, nothing is held back.
    expect(controlsOf(HOSTED, null).held).toBeNull();
    expect(controlsOf(WATCHED, kimi(WATCHED, false)).held).toBe(KIMI_WATCHED);
  });

  it('SE-C237: every Kimi Code session has its effort and side question off, and its question form, in the router’s words; a hosted one’s approval is answered; a Claude Code session keeps all three', async () => {
    const router = kimiRouter((req) => (req.method === 'POST' && req.path === '/api/live/answer' ? json({ ok: true, request_id: 'rq-k' }) : undefined));
    const first = renderSessions(`/live?session=${HOSTED}`);
    await opened(HOSTED);
    await waitFor(() => expect(facts().getByRole('combobox', { name: 'Effort' })).toBeDisabled());
    expect(facts().getByText(KIMI_NO_EFFORT)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Side question' })).toBeDisabled();
    expect(screen.getByText(KIMI_NO_ASIDE)).toBeInTheDocument();
    act(() => {
      streamOf(HOSTED).row({ role: 'assistant', kind: 'ask', t: 'T1', ask: ASK });
    });
    expect(await screen.findByRole('button', { name: 'Submit answers' })).toBeDisabled();
    expect(screen.getByText(KIMI_QUESTION)).toBeInTheDocument();
    expect(screen.getByRole('radio', { name: /Debug/ })).toBeDisabled();
    // The composer still reaches it, and its approval is answered by name.
    expect(box()).toBeEnabled();
    const card = await cardOf('Shell is waiting for permission');
    await press(within(card).getByRole('button', { name: 'Allow' }));
    await waitFor(() => expect(router.posts('/api/live/answer').map((p) => p.body)).toEqual([{ session: HOSTED, allow: true, request_id: 'rq-k' }]));
    expect(router.posts('/api/live/effort')).toEqual([]);
    expect(router.posts('/api/live/aside')).toEqual([]);
    first.unmount();

    // A Claude Code session is as it was: effort, side question and its form all on.
    renderSessions(`/live?session=${A}`);
    await opened(A);
    await waitFor(() => expect(facts().getByRole('combobox', { name: 'Effort' })).toBeEnabled());
    expect(screen.getByRole('button', { name: 'Side question' })).toBeEnabled();
    expect(screen.queryByText(KIMI_NO_EFFORT)).toBeNull();
    expect(screen.queryByText(KIMI_NO_ASIDE)).toBeNull();
  });

  it('SE-C238: a Kimi Code session’s mode switcher offers Manual, Auto and YOLO, led by the mode in force; YOLO asks first; a choice sends Kimi Code’s mode', async () => {
    const router = kimiRouter((req) => (req.method === 'POST' && req.path === '/api/live/mode' ? json({ ok: true }) : undefined));
    renderSessions(`/live?session=${HOSTED}`);
    await opened(HOSTED);
    const mode = () => facts().getByRole('combobox', { name: 'Mode' });
    await waitFor(() => expect(mode()).toHaveDisplayValue('mode: Manual'));
    expect([...mode().querySelectorAll('option')].map((o) => o.textContent)).toEqual(['mode: Manual', 'Auto', 'YOLO']);
    fireEvent.change(mode(), { target: { value: 'yolo' } });
    const asked = await screen.findByText('This session will stop asking before it runs commands or edits files.');
    const modal = asked.closest('.cds--modal') as HTMLElement;
    expect(within(modal).getByRole('button', { name: 'Switch to YOLO' })).toHaveClass('cds--btn--danger');
    await press(within(modal).getByRole('button', { name: 'Keep the current mode' }));
    expect(mode()).toHaveDisplayValue('mode: Manual');
    expect(router.posts('/api/live/mode')).toEqual([]);
    fireEvent.change(mode(), { target: { value: 'auto' } });
    await waitFor(() => expect(router.posts('/api/live/mode').map((p) => p.body)).toEqual([{ session: HOSTED, mode: 'auto' }]));
    await waitFor(() => expect(mode()).toHaveDisplayValue('mode: Auto'));
  });
});

describe('starting a session', () => {
  const HOUR = 3600e3;
  const machine = (id: string, label: string, online: boolean, clients: string[]): MachineRow => ({
    id,
    label,
    platform: 'macos',
    version: '0.9.0',
    build: 'tree:5d1c3b7f9e08a2e94b6dc41d07e28f3a4b6d0f13',
    online,
    since_ms: AT - HOUR,
    clients: clients.map((c) => ({ client: c, cli: { path: `/u/bin/${c}`, version: '2.1.1' }, extensions: [], home: `/u/.${c}` })),
  });
  const MACHINES = [
    machine('m-studio', 'studio', true, ['claude-code', 'kimi-code']),
    machine('m-old', 'old-box', false, ['kimi-code']),
    machine('m-claude', 'claude-only', true, ['claude-code']),
    machine('m-laptop', 'Laptop', true, ['kimi-code']),
  ];
  const MODELS = [
    { id: 'opus', provider: 'anthropic', display_name: 'Claude Opus' },
    { id: 'k3', provider: 'kimi', display_name: 'Kimi K3' },
    { id: 'kimi-for-coding', provider: 'kimi', display_name: 'Kimi for Coding' },
  ];

  /// A hosted router, or a local one, with these machines and models, and `start` answering a start.
  function startRouter(opts: { hosted?: boolean; machines?: MachineRow[]; models?: object[]; start?: (req: Asked) => Response } = {}) {
    const { hosted = true, machines = MACHINES, models = MODELS, start = () => json({ ok: true, session: HOSTED }) } = opts;
    return stubRouter((req) => {
      if (req.method === 'POST' && req.path === '/api/live/start') return start(req);
      if (req.method !== 'GET') return undefined;
      if (req.path === '/api/auth') return json({ mode: hosted ? 'server' : 'local', configured: true });
      if (req.path === '/api/machines') return json(machines);
      if (req.path === '/api/models') return json(models);
      if (req.path === '/api/live/sessions') return json(ROWS());
      if ((req.query.get('session') ?? '').startsWith('session_')) {
        if (req.path === '/api/live/stats') return json(KIMI_STATS);
        if (req.path === '/api/live/prompt') return json({ pending: false, count: 0 });
      }
      return undefined;
    });
  }
  const openForm = async () => {
    await press(await screen.findByRole('button', { name: START }));
    return screen.findByRole('dialog', { name: START_TITLE });
  };
  const optionsOf = (select: HTMLElement) => [...select.querySelectorAll('option')].map((o) => o.textContent);

  it('SE-C239: "Start a session" opens the form; a hosted router offers the online machines with Kimi Code, by label, the first chosen; with none it says so and links to Machines; a local router asks for none', async () => {
    const router = startRouter();
    const first = renderSessions();
    // Nothing is read for it until it is opened.
    await waitFor(() => rowOf(A));
    expect(router.gets('/api/machines')).toEqual([]);
    const dialog = await openForm();
    const select = await within(dialog).findByRole('combobox', { name: 'Machine' });
    expect(optionsOf(select)).toEqual(['Laptop', 'studio']);
    expect(select).toHaveDisplayValue('Laptop');
    expect(startMachines(MACHINES).map((m) => m.id)).toEqual(['m-laptop', 'm-studio']);
    first.unmount();

    startRouter({ machines: [MACHINES[1] as MachineRow, MACHINES[2] as MachineRow] });
    const second = renderSessions();
    const none = await openForm();
    expect(await within(none).findByText(NO_MACHINE, { exact: false })).toBeInTheDocument();
    expect(within(none).getByRole('link', { name: 'Open Machines' })).toHaveAttribute('href', '/machines');
    fireEvent.change(within(none).getByRole('textbox', { name: 'Directory' }), { target: { value: '/Users/sample/code/app' } });
    await waitFor(() => expect(within(none).getByRole('combobox', { name: 'Model' })).toBeInTheDocument());
    expect(within(none).getByRole('button', { name: 'Start' })).toBeDisabled();
    second.unmount();

    const local = startRouter({ hosted: false });
    renderSessions();
    const here = await openForm();
    await waitFor(() => expect(within(here).getByRole('combobox', { name: 'Model' })).toBeInTheDocument());
    expect(within(here).queryByRole('combobox', { name: 'Machine' })).toBeNull();
    expect(local.gets('/api/machines')).toEqual([]);
  });

  it('SE-C240: the form asks for the directory and a Kimi Code model of the catalogue, by display name; Start waits for both; no Kimi Code model says so and links to Settings', async () => {
    startRouter();
    const first = renderSessions();
    const dialog = await openForm();
    const model = await within(dialog).findByRole('combobox', { name: 'Model' });
    expect(optionsOf(model)).toEqual(['Kimi K3', 'Kimi for Coding']);
    expect(model).toHaveDisplayValue('Kimi K3');
    await within(dialog).findByRole('combobox', { name: 'Machine' });
    const start = within(dialog).getByRole('button', { name: 'Start' });
    expect(start).toBeDisabled();
    fireEvent.change(within(dialog).getByRole('textbox', { name: 'Directory' }), { target: { value: '  ' } });
    expect(start).toBeDisabled();
    fireEvent.change(within(dialog).getByRole('textbox', { name: 'Directory' }), { target: { value: '/Users/sample/code/app' } });
    expect(start).toBeEnabled();
    first.unmount();

    startRouter({ models: [MODELS[0] as object] });
    renderSessions();
    const none = await openForm();
    expect(await within(none).findByText(NO_MODEL, { exact: false })).toBeInTheDocument();
    expect(within(none).getByRole('link', { name: 'Open Settings → Accounts' })).toHaveAttribute('href', '/settings/accounts');
    fireEvent.change(within(none).getByRole('textbox', { name: 'Directory' }), { target: { value: '/Users/sample/code/app' } });
    expect(within(none).getByRole('button', { name: 'Start' })).toBeDisabled();
  });

  it('SE-C241: a start the router takes opens the session and reads the list again; one it refuses keeps the form and what was typed, in the router’s words', async () => {
    const REFUSED = 'Start a session in a directory that exists on that machine, by its full path.';
    let refuse = true;
    const router = startRouter({ start: () => (refuse ? json({ ok: false, reason: REFUSED }, 400) : json({ ok: true, session: HOSTED })) });
    renderSessions();
    const dialog = await openForm();
    const select = await within(dialog).findByRole('combobox', { name: 'Machine' });
    fireEvent.change(select, { target: { value: 'm-studio' } });
    await waitFor(() => within(dialog).getByRole('combobox', { name: 'Model' }));
    fireEvent.change(within(dialog).getByRole('combobox', { name: 'Model' }), { target: { value: 'kimi-for-coding' } });
    fireEvent.change(within(dialog).getByRole('textbox', { name: 'Directory' }), { target: { value: ' /Users/sample/code/nowhere ' } });
    await press(within(dialog).getByRole('button', { name: 'Start' }));
    expect(await within(dialog).findByText(REFUSED)).toBeInTheDocument();
    expect(within(dialog).getByRole('textbox', { name: 'Directory' })).toHaveValue(' /Users/sample/code/nowhere ');
    expect(router.posts('/api/live/start').map((p) => p.body)).toEqual([{ machine: 'm-studio', cwd: '/Users/sample/code/nowhere', model: 'kimi-for-coding' }]);

    refuse = false;
    const listed = router.gets('/api/live/sessions').length;
    await press(within(dialog).getByRole('button', { name: 'Start' }));
    await waitFor(() => expect(screen.queryByRole('dialog', { name: START_TITLE })).toBeNull());
    // The session it started is open, and the list is read again so its row comes in.
    await waitFor(() => streamOf(HOSTED));
    await waitFor(() => expect(router.gets('/api/live/sessions').length).toBeGreaterThan(listed));
    expect(document.querySelector(`[data-session-view="${HOSTED}"]`)).not.toBeNull();
  });
});
