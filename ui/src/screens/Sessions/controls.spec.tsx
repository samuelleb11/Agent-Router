// New for agent-router: the open session's controls — the question form, the permission card, the
// facts strip and its switchers, the composer — and the notices beside them, against the router's
// answers. The model's rules for what each sends and says are pinned in ~/model; these pin that the
// controls send it, say it beside themselves, and hold their ground while a request flies.
import { act, fireEvent, render, renderHook, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { notice, type Ask } from '~/model/session';
import { press } from '~/test-utils';
import { FakeEventSource } from './fakes';
import { A, B, C, defaults, drain, held, hang, json, renderSessions, session, SESSIONS, streamOf, stubRouter, working } from './kit';
import { COMPOSING, walk } from './logic';
import { NoticeLine } from './Notice';
import { useNotice } from './useNotice';
import { QuestionForm } from './QuestionForm';

beforeEach(() => FakeEventSource.reset());
afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

const ASK: Ask = {
  questions: [
    {
      question: 'Where should the list sit on a desk?',
      header: 'Layout',
      options: [
        { label: 'Beside the feed', description: 'From 1,056 px' },
        { label: 'Say "over"', description: 'A label with quotes' },
      ],
    },
    { question: 'Which checks run in CI?', header: 'Checks', multiSelect: true, options: [{ label: 'Vitest' }, { label: 'The shots' }] },
  ],
};

function drawForm(deliver = vi.fn(async () => ({ ok: true, notice: notice('Sent as a message — it may not dismiss the prompt.') }))) {
  render(<QuestionForm session={A} ask={ASK} formKey="row-1" deliver={deliver} />);
  return deliver;
}
const tab = (name: string) => screen.getByRole('tab', { name: new RegExp(name) });
const hint = () => document.querySelector('[role="status"]');

/// The open session, its stream speaking, ready.
async function opened(id = A, status: object = working()) {
  await waitFor(() => streamOf(id));
  act(() => {
    streamOf(id).open().status(status);
  });
}
const box = () => screen.getByRole('textbox', { name: 'Message to this session' });
const facts = () => within(screen.getByRole('region', { name: 'This session’s totals and settings' }));

describe('the question form', () => {
  it('SE-C106: one choice, or several when it says multiSelect; each option its label and description; the chosen one marked, and its tab ticked', async () => {
    stubRouter();
    drawForm();
    expect(screen.getAllByRole('radio')).toHaveLength(3);
    expect(screen.getByText('From 1,056 px')).toBeInTheDocument();
    await press(screen.getByRole('radio', { name: /Beside the feed/ }));
    expect(screen.getByRole('radio', { name: /Beside the feed/ })).toBeChecked();
    expect(tab('Layout')).toHaveTextContent('✓');
    await press(tab('Checks'));
    const boxes = screen.getAllByRole('checkbox');
    expect(boxes).toHaveLength(3);
    await press(boxes[0] as HTMLElement);
    await press(boxes[1] as HTMLElement);
    expect(boxes.slice(0, 2).every((b) => (b as HTMLInputElement).checked)).toBe(true);
    expect(tab('Checks')).toHaveTextContent('✓');
  });

  it('SE-C107: Other is always offered, with a field whose typing chooses it', async () => {
    stubRouter();
    drawForm();
    const other = screen.getByRole('radio', { name: 'Other' });
    expect(other).not.toBeChecked();
    fireEvent.change(screen.getByRole('textbox', { name: 'Other answer' }), { target: { value: 'In a drawer' } });
    expect(other).toBeChecked();
    await press(tab('Checks'));
    // The first question's panel is hidden now; the field on screen is the second's.
    fireEvent.change(screen.getByRole('textbox', { name: 'Other answer' }), { target: { value: 'Lint' } });
    expect(screen.getByRole('checkbox', { name: 'Other' })).toBeChecked();
  });

  it('SE-C111: Submit sends every answered question together; with none it says "Pick at least one answer first." and sends nothing; the composer is named as the other way', async () => {
    const router = stubRouter((req) => (req.method === 'POST' && req.path === '/api/live/answer' ? json({ ok: true, request_id: 'rq' }) : undefined));
    drawForm();
    expect(hint()).toHaveTextContent('or type a reply below');
    await press(screen.getByRole('button', { name: 'Submit answers' }));
    expect(hint()).toHaveTextContent('Pick at least one answer first.');
    expect(router.asked).toEqual([]);
    await press(screen.getByRole('radio', { name: /Say "over"/ }));
    await press(tab('Checks'));
    await press(screen.getByRole('checkbox', { name: 'The shots' }));
    await press(screen.getByRole('button', { name: 'Submit answers' }));
    await waitFor(() => expect(router.posts('/api/live/answer')).toHaveLength(1));
    // Labels round-trip exactly, quotes included.
    expect(router.posts('/api/live/answer')[0]?.body).toEqual({
      session: A,
      answers: { 'Where should the list sit on a desk?': 'Say "over"', 'Which checks run in CI?': 'The shots' },
    });
  });

  it('SE-C115: the form is spent — disabled, saying "answered" — only once the router has answered the prompt itself', async () => {
    stubRouter((req) => (req.method === 'POST' && req.path === '/api/live/answer' ? json({ ok: true, request_id: 'rq' }) : undefined));
    drawForm();
    await press(screen.getByRole('radio', { name: /Beside the feed/ }));
    await press(screen.getByRole('button', { name: 'Submit answers' }));
    await waitFor(() => expect(hint()).toHaveTextContent('answered'));
    expect(screen.getByRole('button', { name: 'Submit answers' })).toBeDisabled();
    expect(screen.getByRole('radio', { name: /Beside the feed/ })).toBeDisabled();
  });

  it('SE-C117: a submit that reaches nothing says "could not answer — <error>", and the form stays live', async () => {
    stubRouter((req) => (req.method === 'POST' && req.path === '/api/live/answer' ? Promise.reject(new TypeError('fetch failed')) : undefined));
    drawForm();
    await press(screen.getByRole('radio', { name: /Beside the feed/ }));
    await press(screen.getByRole('button', { name: 'Submit answers' }));
    await waitFor(() => expect(hint()).toHaveTextContent('could not answer — Could not reach the router.'));
    expect(screen.getByRole('button', { name: 'Submit answers' })).toBeEnabled();
    expect(screen.getByRole('radio', { name: /Beside the feed/ })).toBeEnabled();
  });

  it('SE-C118: Submit is guarded against a double tap: disabled while an answer flies, saying "answering…"', async () => {
    const slow = held();
    const router = stubRouter((req) => (req.method === 'GET' && req.path === '/api/live/prompt' ? slow.reply(req) : undefined));
    drawForm();
    await press(screen.getByRole('radio', { name: /Beside the feed/ }));
    const submit = screen.getByRole('button', { name: 'Submit answers' });
    act(() => {
      fireEvent.click(submit);
      fireEvent.click(submit);
    });
    expect(hint()).toHaveTextContent('answering…');
    expect(submit).toBeDisabled();
    expect(router.gets('/api/live/prompt')).toHaveLength(1);
  });

  it('SE-C116, SE-C152, SE-C154: answers the router cannot take directly go as a message, which says it may not dismiss the prompt, and the composer’s draft is left alone', async () => {
    const router = stubRouter((req) => {
      if (req.method === 'POST' && req.path === '/api/live/answer') return json({ ok: false, reason: 'That prompt is no longer waiting.' }, 409);
      if (req.method === 'POST' && req.path === '/api/live/send') return json({ ok: true });
      return undefined;
    });
    renderSessions(`/live?session=${A}`);
    await waitFor(() => streamOf(A));
    act(() => {
      streamOf(A).open().row({ role: 'assistant', kind: 'ask', t: 'T1', ask: ASK }).status(working());
    });
    fireEvent.change(box(), { target: { value: 'my own draft' } });
    await press(screen.getByRole('radio', { name: /Beside the feed/ }));
    await press(screen.getByRole('button', { name: 'Submit answers' }));
    await waitFor(() => expect(router.posts('/api/live/send')).toHaveLength(1));
    expect(router.posts('/api/live/send')[0]?.body).toEqual({ session: A, text: 'Layout: Beside the feed' });
    const form = screen.getByRole('button', { name: 'Submit answers' }).closest('.cds--tile') as HTMLElement;
    expect(within(form).getByText('That prompt is no longer waiting.')).toBeInTheDocument();
    expect(await within(form).findByText('Sent as a message — it may not dismiss the prompt.')).toBeInTheDocument();
    expect(box()).toHaveValue('my own draft');
  });
});

describe('the permission card', () => {
  const PROMPT = {
    pending: true,
    request_id: 'rq-1',
    tool: 'Bash',
    display_name: 'Bash',
    description: 'List the build',
    input: { command: 'ls build' },
    suggestions: [{ index: 0, label: 'Yes, and don’t ask again for ls' }],
    count: 1,
  };

  it('SE-C123: every choice is a visible button — Allow, each of the prompt’s own suggestions by its label, then Deny — and nothing else', async () => {
    stubRouter((req) => (req.method === 'GET' && req.path === '/api/live/prompt' ? json(PROMPT) : undefined));
    renderSessions(`/live?session=${A}`);
    await opened();
    const card = await screen.findByText('Bash is waiting for permission');
    const callout = card.closest('.cds--actionable-notification') as HTMLElement;
    expect(within(callout).getAllByRole('button').map((b) => b.textContent)).toEqual(['Allow', 'Yes, and don’t ask again for ls', 'Deny']);
    expect(within(callout).getByText('ls build')).toBeInTheDocument();
  });

  it('SE-C125: while an answer flies every button is disabled; done, the card goes and "Allowed." shows for five seconds; refused, the router’s words and the buttons back', async () => {
    const slow = held();
    let waiting = true;
    const router = stubRouter((req) => {
      if (req.method === 'GET' && req.path === '/api/live/prompt') return json(waiting ? PROMPT : { pending: false, count: 0 });
      if (req.method === 'POST' && req.path === '/api/live/answer') return slow.reply(req);
      return undefined;
    });
    renderSessions(`/live?session=${A}`);
    await opened();
    const callout = (await screen.findByText('Bash is waiting for permission')).closest('.cds--actionable-notification') as HTMLElement;
    await press(within(callout).getByRole('button', { name: 'Allow' }));
    expect(within(callout).getAllByRole('button').every((b) => (b as HTMLButtonElement).disabled)).toBe(true);
    expect(router.posts('/api/live/answer')[0]?.body).toEqual({ session: A, allow: true, request_id: 'rq-1' });
    vi.useFakeTimers({ shouldAdvanceTime: true });
    waiting = false;
    await act(async () => {
      slow.release(json({ ok: true, request_id: 'rq-1' }));
      await vi.advanceTimersByTimeAsync(0);
    });
    await waitFor(() => expect(screen.queryByText('Bash is waiting for permission')).toBeNull());
    expect(screen.getByText('Allowed.')).toBeInTheDocument();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(5000);
    });
    expect(screen.queryByText('Allowed.')).toBeNull();
  });

  it('SE-C125, SE-C183: a refused answer says the router’s words beside the card and gives its buttons back', async () => {
    stubRouter((req) => {
      if (req.method === 'GET' && req.path === '/api/live/prompt') return json(PROMPT);
      if (req.method === 'POST' && req.path === '/api/live/answer') return json({ ok: false, error: 'That session is not on a connected machine.' }, 503);
      return undefined;
    });
    renderSessions(`/live?session=${A}`);
    await opened();
    const callout = (await screen.findByText('Bash is waiting for permission')).closest('.cds--actionable-notification') as HTMLElement;
    await press(within(callout).getByRole('button', { name: 'Deny' }));
    expect(await within(callout).findByText('That session is not on a connected machine.')).toBeInTheDocument();
    expect(within(callout).getAllByRole('button').every((b) => !(b as HTMLButtonElement).disabled)).toBe(true);
  });

  it('SE-C130: the card never takes focus and never answers on Enter: only a press on one of its buttons does', async () => {
    let waiting = false;
    const router = stubRouter((req) => (req.method === 'GET' && req.path === '/api/live/prompt' ? json(waiting ? PROMPT : { pending: false, count: 0 }) : undefined));
    renderSessions(`/live?session=${A}`);
    await opened();
    box().focus();
    waiting = true;
    act(() => {
      // The status names the waiting request, so the card is asked for at once.
      streamOf(A).status(working({ activity: 'waiting', tone: 'attention', detail: 'Bash wants to run', waiting_on: { kind: 'permission', request_id: 'rq-1' } }));
    });
    const card = await screen.findByText('Bash is waiting for permission');
    expect(document.activeElement).toBe(box());
    const callout = card.closest('.cds--actionable-notification') as HTMLElement;
    expect(callout).not.toHaveAttribute('role', 'alertdialog');
    expect(callout).not.toHaveAttribute('tabindex');
    fireEvent.keyDown(box(), { key: 'Enter' });
    fireEvent.keyDown(callout, { key: 'Enter' });
    await drain();
    expect(router.posts('/api/live/answer')).toEqual([]);
  });
});

describe('the facts strip and its switchers', () => {
  it('SE-C131, SE-C134: the strip says the model without "claude-", its effort, its tokens with cache reads, its output and cached tokens, its turns, and its cost only when priced', async () => {
    stubRouter();
    const first = renderSessions(`/live?session=${B}`);
    await opened(B);
    await waitFor(() => expect(facts().getByText('sonnet-4-5')).toBeInTheDocument());
    for (const f of ['effort low', '3k tokens', '512 out · 0 cached', '1 turn', '$0.123']) expect(facts().getByText(f)).toBeInTheDocument();
    first.unmount();
    renderSessions(`/live?session=${C}`);
    await opened(C);
    await waitFor(() => expect(facts().getByText('unknown')).toBeInTheDocument());
    expect(facts().queryByText(/^\$/)).toBeNull();
  });

  it('SE-C132: the project’s own total sits apart on the strip, and only when the project has more than one session', async () => {
    let sessions = 2;
    stubRouter((req) => (req.method === 'GET' && req.path === '/api/live/project' ? json({ sessions, total: 1_237_400, turns: 13, cost: 4.68 }) : undefined));
    const first = renderSessions(`/live?session=${A}`);
    await opened();
    expect(await facts().findByText('project · 2 sessions · 1.2M tokens · 13 turns · $4.68')).toBeInTheDocument();
    first.unmount();
    sessions = 1;
    renderSessions(`/live?session=${A}`);
    await opened();
    await waitFor(() => expect(facts().getByText('opus-4-1')).toBeInTheDocument());
    expect(facts().queryByText(/^project ·/)).toBeNull();
  });

  it('SE-C232: a session’s live totals are asked by its project key, the repository else the directory', async () => {
    const repository = 'gitlab.example.com/acme/agent-router';
    const rows = () => [session(A, { cwd: '/Users/sample/code/agent-router-lane-2', repository }), ...SESSIONS().slice(1)];
    const router = stubRouter(undefined, defaults(rows));
    const first = renderSessions(`/live?session=${A}`);
    await opened();
    expect(await facts().findByText('project · 2 sessions · 1.2M tokens · 13 turns · $4.68')).toBeInTheDocument();
    // Never the directory, which is one worktree of the repository: the repository counts them all.
    expect(router.gets('/api/live/project').map((g) => [...g.query])).toEqual([[['project', repository]]]);
    first.unmount();
    // A session whose directory names no repository is asked by its directory, as before.
    renderSessions(`/live?session=${C}`);
    await opened(C);
    await waitFor(() => expect(router.gets('/api/live/project')).toHaveLength(2));
    expect([...router.gets('/api/live/project')[1]!.query]).toEqual([['project', '/Users/sample/code/northstar']]);
  });

  it('SE-C133: totals are read once each time a session opens, never pushed with every event', async () => {
    const router = stubRouter();
    renderSessions(`/live?session=${A}`);
    await opened();
    await waitFor(() => expect(facts().getByText('opus-4-1')).toBeInTheDocument());
    act(() => {
      for (let i = 0; i < 20; i++) streamOf(A).row({ role: 'assistant', kind: 'text', text: `row ${i}` }).status(working({ as_of_ms: i }));
    });
    expect(router.gets('/api/live/stats').filter((g) => g.query.get('session') === A)).toHaveLength(1);
  });

  it('SE-C135: while reading it says "reading totals…", and a read that fails says so — never "unknown", zeros or "undefined turns"', async () => {
    const slow = held();
    stubRouter((req) => (req.method === 'GET' && req.path === '/api/live/stats' ? slow.reply(req) : undefined));
    renderSessions(`/live?session=${A}`);
    await opened();
    expect(facts().getByText('reading totals…')).toBeInTheDocument();
    act(() => slow.release(json({ ok: false, reason: 'No such session.' }, 404)));
    expect(await facts().findByText('Could not read this session’s totals: No such session.')).toBeInTheDocument();
    const said = (screen.getByRole('region', { name: 'This session’s totals and settings' }).textContent ?? '').replace(/model: unknown|effort: unknown|mode: unknown/g, '');
    expect(said).not.toMatch(/\bunknown\b|undefined|0 tokens|0 turns/);
  });

  it('SE-C145: after a switch, the switcher’s current value is the new one, and choosing the previous value again sends it', async () => {
    const router = stubRouter((req) => (req.method === 'POST' && req.path === '/api/live/model' ? json({ ok: true, model: req.body?.['model'] }) : undefined));
    renderSessions(`/live?session=${A}`);
    await opened();
    const model = () => facts().getByRole('combobox', { name: 'Model' });
    await waitFor(() => expect(model()).toHaveDisplayValue('model: opus-4-1'));
    fireEvent.change(model(), { target: { value: 'sonnet' } });
    await waitFor(() => expect(model()).toHaveDisplayValue('model: sonnet'));
    expect(await facts().findByText('model set to sonnet — takes effect on the next turn')).toBeInTheDocument();
    fireEvent.change(model(), { target: { value: 'opus-4-1' } });
    await waitFor(() => expect(router.posts('/api/live/model').map((p) => p.body)).toEqual([
      { session: A, model: 'sonnet' },
      { session: A, model: 'opus-4-1' },
    ]));
  });

  it('SE-C141, SE-C142: "other…" asks in a modal, where an empty name changes nothing; Bypass asks first in a danger modal, and declining sends nothing', async () => {
    const router = stubRouter();
    renderSessions(`/live?session=${A}`);
    await opened();
    const model = () => facts().getByRole('combobox', { name: 'Model' });
    await waitFor(() => expect(model()).toHaveDisplayValue('model: opus-4-1'));
    fireEvent.change(model(), { target: { value: '__other__' } });
    const dialog = await screen.findByRole('dialog', { name: 'Use another model' });
    expect(within(dialog).getByRole('button', { name: 'Use this model' })).toBeDisabled();
    await press(within(dialog).getByRole('button', { name: 'Cancel' }));
    expect(model()).toHaveDisplayValue('model: opus-4-1');
    fireEvent.change(facts().getByRole('combobox', { name: 'Mode' }), { target: { value: 'bypassPermissions' } });
    const bypass = await screen.findByText('This session will stop asking before it runs commands or edits files.');
    const danger = bypass.closest('.cds--modal') as HTMLElement;
    expect(within(danger).getByRole('button', { name: 'Bypass permissions' })).toHaveClass('cds--btn--danger');
    await press(within(danger).getByRole('button', { name: 'Keep the current mode' }));
    expect(facts().getByRole('combobox', { name: 'Mode' })).toHaveDisplayValue('mode: Ask');
    expect([...router.posts('/api/live/model'), ...router.posts('/api/live/mode')]).toEqual([]);
  });
});

describe('the composer', () => {
  it('SE-C147: Enter sends and Shift+Enter starts a new line; the box grows with its text up to 120 px and shrinks back after a send', async () => {
    const router = stubRouter((req) => (req.method === 'POST' && req.path === '/api/live/send' ? json({ ok: true }) : undefined));
    renderSessions(`/live?session=${A}`);
    await opened();
    Object.defineProperty(box(), 'scrollHeight', { configurable: true, get: () => ((box() as HTMLTextAreaElement).value.length > 20 ? 400 : 40) });
    fireEvent.change(box(), { target: { value: 'a message long enough to grow the box' } });
    expect(box().style.height).toBe('120px');
    const shift = fireEvent.keyDown(box(), { key: 'Enter', shiftKey: true });
    expect(shift).toBe(true);
    expect(router.posts('/api/live/send')).toEqual([]);
    fireEvent.keyDown(box(), { key: 'Enter' });
    await waitFor(() => expect(router.posts('/api/live/send')).toHaveLength(1));
    await waitFor(() => expect(box()).toHaveValue(''));
    await waitFor(() => expect(box().style.height).toBe('40px'));
  });

  it('SE-C148: Send trims the text, sends nothing when it is empty, and is disabled while a message flies', async () => {
    const slow = held();
    const router = stubRouter((req) => (req.method === 'POST' && req.path === '/api/live/send' ? slow.reply(req) : undefined));
    renderSessions(`/live?session=${A}`);
    await opened();
    fireEvent.change(box(), { target: { value: '   ' } });
    await press(screen.getByRole('button', { name: 'Send' }));
    expect(router.posts('/api/live/send')).toEqual([]);
    fireEvent.change(box(), { target: { value: '  go on  ' } });
    await press(screen.getByRole('button', { name: 'Send' }));
    expect(router.posts('/api/live/send').map((p) => p.body)).toEqual([{ session: A, text: 'go on' }]);
    expect(screen.getByRole('button', { name: 'Send' })).toBeDisabled();
  });

  it('SE-C149, SE-C157: Send stays while a turn runs, with Stop beside it, and Stop goes when the turn is over', async () => {
    stubRouter();
    renderSessions(`/live?session=${A}`);
    await opened(A, working());
    expect(screen.getByRole('button', { name: 'Send' })).toBeEnabled();
    expect(screen.getByRole('button', { name: 'Stop the turn' })).toBeEnabled();
    act(() => {
      streamOf(A).status(working({ activity: 'idle', tone: 'idle', detail: 'Waiting for you', state: 'idle' }));
    });
    expect(screen.queryByRole('button', { name: 'Stop the turn' })).toBeNull();
    expect(screen.getByRole('button', { name: 'Send' })).toBeEnabled();
  });

  it('SE-C150, SE-C152: a send is echoed at once as "you · queued", and says "Delivered as your message." for four seconds', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    stubRouter((req) => (req.method === 'POST' && req.path === '/api/live/send' ? json({ ok: true }) : undefined));
    renderSessions(`/live?session=${A}`);
    await opened();
    fireEvent.change(box(), { target: { value: 'next, the shots' } });
    await press(screen.getByRole('button', { name: 'Send' }));
    expect(await screen.findByText('Delivered as your message.')).toBeInTheDocument();
    expect(screen.getByText('you · queued')).toBeInTheDocument();
    expect(screen.getByText('next, the shots')).toBeInTheDocument();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(4000);
    });
    expect(screen.queryByText('Delivered as your message.')).toBeNull();
  });

  it('SE-C151: a send that went over the peer socket warns, until dismissed, that the session will see it as from another Claude', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    stubRouter((req) => (req.method === 'POST' && req.path === '/api/live/send' ? json({ ok: true, via: 'peer', framed: true }) : undefined));
    renderSessions(`/live?session=${A}`);
    await opened();
    fireEvent.change(box(), { target: { value: 'hello' } });
    await press(screen.getByRole('button', { name: 'Send' }));
    const warning = await screen.findByText(/Sent, but as a peer message/);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(30_000);
    });
    expect(warning).toBeInTheDocument();
    await press(within(warning.closest('.cds--inline-notification') as HTMLElement).getByRole('button', { name: /close/i }));
    expect(screen.queryByText(/Sent, but as a peer message/)).toBeNull();
  });

  it('SE-C156: the arrows walk the history only from the edge they would leave; a draft is kept and brought back; ArrowUp at the oldest stays put', () => {
    const sent = ['first', 'second'];
    // Mid-draft, the arrows edit the draft.
    expect(walk('ArrowUp', sent, COMPOSING, { value: 'a draft', start: 3, end: 3 })).toBeNull();
    const up = walk('ArrowUp', sent, COMPOSING, { value: 'a draft', start: 0, end: 0 });
    expect(up).toEqual({ state: { at: 1, draft: 'a draft' }, value: 'second' });
    const upper = walk('ArrowUp', sent, up?.state ?? COMPOSING, { value: 'second', start: 0, end: 0 });
    expect(upper?.value).toBe('first');
    expect(walk('ArrowUp', sent, upper?.state ?? COMPOSING, { value: 'first', start: 0, end: 0 })?.value).toBe('first');
    // A selection is not an edge.
    expect(walk('ArrowDown', sent, upper?.state ?? COMPOSING, { value: 'first', start: 0, end: 5 })).toBeNull();
    const down = walk('ArrowDown', sent, upper?.state ?? COMPOSING, { value: 'first', start: 5, end: 5 });
    expect(down?.value).toBe('second');
    expect(walk('ArrowDown', sent, down?.state ?? COMPOSING, { value: 'second', start: 6, end: 6 })).toEqual({ state: COMPOSING, value: 'a draft' });
    // With nothing said yet, the arrows are the box's own.
    expect(walk('ArrowUp', [], COMPOSING, { value: '', start: 0, end: 0 })).toBeNull();
  });

  it('SE-C159, SE-C160: a side question is the composer’s text, else asked for in a modal; with nothing typed nothing is asked; it is echoed and its answer lands in the feed', async () => {
    const router = stubRouter((req) =>
      req.method === 'POST' && req.path === '/api/live/aside' ? json({ ok: true, answer: 'Because **the router** says so.', took_ms: 4200, cost: 0.0213 }) : undefined,
    );
    renderSessions(`/live?session=${A}`);
    await opened();
    await press(screen.getByRole('button', { name: 'Side question' }));
    const dialog = await screen.findByRole('dialog', { name: 'Ask on the side' });
    expect(within(dialog).getByText('Answered from this session’s context, without interrupting it.')).toBeInTheDocument();
    expect(within(dialog).getByRole('button', { name: 'Ask' })).toBeDisabled();
    fireEvent.change(within(dialog).getByRole('textbox', { name: 'Your side question' }), { target: { value: 'why this order?' } });
    await press(within(dialog).getByRole('button', { name: 'Ask' }));
    await waitFor(() => expect(router.posts('/api/live/aside').map((p) => p.body)).toEqual([{ session: A, text: 'why this order?' }]));
    expect(await screen.findByText('you · side question')).toBeInTheDocument();
    expect(await screen.findByText('side answer')).toBeInTheDocument();
    expect(screen.getByText('the router').closest('strong')).not.toBeNull();
    expect(await screen.findByText('answered in 4.2s · $0.021')).toBeInTheDocument();
    // From the box, its text is the question, and the box is cleared.
    fireEvent.change(box(), { target: { value: 'and this?' } });
    await press(screen.getByRole('button', { name: 'Side question' }));
    await waitFor(() => expect(router.posts('/api/live/aside')).toHaveLength(2));
    expect(box()).toHaveValue('');
  });

  it('SE-C185: a request that never answers gives its control back when it times out, and says so', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    stubRouter((req) => (req.method === 'POST' && req.path === '/api/live/send' ? hang(req) : undefined));
    renderSessions(`/live?session=${A}`);
    await opened();
    fireEvent.change(box(), { target: { value: 'into the void' } });
    await press(screen.getByRole('button', { name: 'Send' }));
    expect(screen.getByRole('button', { name: 'Send' })).toBeDisabled();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(10_000);
    });
    await waitFor(() => expect(screen.getByRole('button', { name: 'Send' })).toBeEnabled());
    expect(screen.getByText('Send failed — The router did not answer within 10 s.')).toBeInTheDocument();
    // Nothing typed is lost to it.
    expect(box()).toHaveValue('into the void');
  });

  it('SE-C177: a notice is text, never markup, whatever words the router sent', async () => {
    stubRouter((req) => (req.method === 'POST' && req.path === '/api/live/send' ? json({ ok: false, reason: '<img src=x onerror=alert(1)><b>refused</b>' }, 409) : undefined));
    renderSessions(`/live?session=${A}`);
    await opened();
    fireEvent.change(box(), { target: { value: 'hello' } });
    await press(screen.getByRole('button', { name: 'Send' }));
    const said = await screen.findByText('<img src=x onerror=alert(1)><b>refused</b>');
    expect(said.querySelector('img, b')).toBeNull();
    expect(document.querySelector('[data-sessions-screen] img')).toBeNull();
  });

  it('SE-C183: a hosted router’s 502 on an action says the router’s error, never "Send failed (502)"; only a refusal that said nothing reads as the sentence naming the action', async () => {
    // What a hosted router answers when the relay to the session's machine failed (SE-C224).
    const LATE = 'win-box did not answer in time';
    const CLOSED = 'win-box: the connection to its agent closed';
    let quiet = false;
    stubRouter((req) => {
      if (req.method !== 'POST') return undefined;
      if (req.path === '/api/live/send') return quiet ? new Response(null, { status: 502 }) : json({ ok: false, error: LATE }, 502);
      if (req.path === '/api/live/interrupt') return json({ ok: false, error: CLOSED }, 502);
      return undefined;
    });
    renderSessions(`/live?session=${A}`);
    await opened(A, working());
    fireEvent.change(box(), { target: { value: 'next, the shots' } });
    await press(screen.getByRole('button', { name: 'Send' }));
    expect(await screen.findByText(LATE)).toBeInTheDocument();
    expect(screen.queryByText('Send failed (502)')).toBeNull();
    // Nothing typed is lost to it (SE-C153).
    expect(box()).toHaveValue('next, the shots');
    // Stop, refused the same way, says what the router said too (SE-C158).
    await press(screen.getByRole('button', { name: 'Stop the turn' }));
    expect(await screen.findByText(CLOSED)).toBeInTheDocument();
    expect(screen.queryByText('Could not stop this session.')).toBeNull();
    // A 502 that carried no words: the sentence naming the action, and its status.
    quiet = true;
    await press(screen.getByRole('button', { name: 'Send' }));
    expect(await screen.findByText('Send failed (502)')).toBeInTheDocument();
  });

  it('SE-C183: a 502 whose body is a page from the ingress in front of the router is not the router’s words: Send says the sentence naming the action, and never any of the page or its markup', async () => {
    // What an nginx ingress answers while the router behind it is down.
    const PAGE = '<html>\r\n<head><title>502 Bad Gateway</title></head>\r\n<body>\r\n<center><h1>502 Bad Gateway</h1></center>\r\n<hr><center>nginx</center>\r\n</body>\r\n</html>\r\n';
    stubRouter((req) =>
      req.method === 'POST' && req.path === '/api/live/send'
        ? new Response(PAGE, { status: 502, statusText: 'Bad Gateway', headers: { 'Content-Type': 'text/html' } })
        : undefined,
    );
    renderSessions(`/live?session=${A}`);
    await opened();
    fireEvent.change(box(), { target: { value: 'next, the shots' } });
    await press(screen.getByRole('button', { name: 'Send' }));
    expect(await screen.findByText('Send failed (502)')).toBeInTheDocument();
    const shown = document.querySelector('[data-sessions-screen]')?.textContent ?? '';
    expect(shown, 'the page reached the screen as text').not.toMatch(/<\/?(html|head|title|body|center|h1|hr)\b|nginx|Bad Gateway/i);
    expect(document.querySelector('[data-sessions-screen] center'), 'the page reached the screen as markup').toBeNull();
    // Nothing typed is lost to it (SE-C153).
    expect(box()).toHaveValue('next, the shots');
  });
});

describe('notices', () => {
  function Shown({ n }: { n: ReturnType<typeof useNotice> }) {
    return <NoticeLine notice={n[0]} onClose={() => n[1](null)} />;
  }

  it('SE-C180: a success clears itself after its time; a warning stays until dismissed or replaced; a new notice replaces the one before and restarts the timer', () => {
    vi.useFakeTimers();
    const { result } = renderHook(() => useNotice());
    const view = render(<Shown n={result.current} />);
    const redraw = () => view.rerender(<Shown n={result.current} />);
    act(() => result.current[1](notice('Stopped.', { ok: true, for: 4000 })));
    redraw();
    expect(screen.getByText('Stopped.')).toBeInTheDocument();
    act(() => {
      vi.advanceTimersByTime(3000);
    });
    // Replaced, and the new one's own time starts now.
    act(() => result.current[1](notice('Allowed.', { ok: true, for: 5000 })));
    redraw();
    act(() => {
      vi.advanceTimersByTime(4000);
    });
    redraw();
    expect(screen.queryByText('Stopped.')).toBeNull();
    expect(screen.getByText('Allowed.')).toBeInTheDocument();
    act(() => {
      vi.advanceTimersByTime(1000);
    });
    redraw();
    expect(screen.queryByText('Allowed.')).toBeNull();
    // A warning stays.
    act(() => result.current[1](notice('Could not stop this session.')));
    act(() => {
      vi.advanceTimersByTime(60_000);
    });
    redraw();
    expect(screen.getByText('Could not stop this session.')).toBeInTheDocument();
    act(() => result.current[1](null));
    redraw();
    expect(screen.queryByText('Could not stop this session.')).toBeNull();
  });
});
