// New for agent-router: nothing a reader typed, picked or started in one session follows them to
// another. The live view broke this: its composer's draft survived a switch through a tab or a hit,
// where Enter sent it, and a slow read drew one session's model and switchers under the next
// (SE-C192, SE-C193, SE-C136). Northstar learned it in 940a783, which saved a draft onto the next
// project because a screen stayed mounted across a change of route.
import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { press } from '~/test-utils';
import { FakeEventSource } from './fakes';
import { A, B, drain, held, json, renderSessions, streamOf, stubRouter, working } from './kit';

beforeEach(() => FakeEventSource.reset());
afterEach(() => vi.unstubAllGlobals());

const where = () => screen.getByTestId('location').textContent;
const row = (id: string) => {
  const r = document.querySelector<HTMLButtonElement>(`[data-session="${id}"] > button`);
  if (!r) throw new Error(`no list row for ${id}`);
  return r;
};
const view = (id: string) => {
  const v = document.querySelector<HTMLElement>(`[data-session-view="${id}"]`);
  if (!v) throw new Error(`${id} is not on screen`);
  return within(v);
};

/// Open a session from the list, and wait for its stream.
async function open(id: string) {
  await press(row(id));
  await waitFor(() => expect(where()).toBe(`/live?session=${id}`));
  await waitFor(() => streamOf(id));
}

/// The stream of a session says its tail and its status.
function speak(id: string, ...rows: object[]) {
  act(() => {
    const s = streamOf(id).open();
    for (const r of rows) s.row(r);
    s.status(working());
  });
}

const ASK = {
  role: 'assistant',
  kind: 'ask',
  t: '2026-10-05T11:30:00.000Z',
  ask: {
    questions: [
      {
        question: 'Where should the list sit on a desk?',
        header: 'Layout',
        options: [{ label: 'Beside the feed' }, { label: 'Over the feed' }],
      },
    ],
  },
};

describe('a session’s things stay with that session', () => {
  it('SE-C192: a composer draft stays with the session it was typed in; another session’s box is empty, and Enter there sends nothing of it', async () => {
    const router = stubRouter((req) => (req.method === 'POST' && req.path === '/api/live/send' ? json({ ok: true }) : undefined));
    renderSessions(`/live?session=${A}`);
    await waitFor(() => streamOf(A));
    speak(A);
    const box = () => screen.getByRole('textbox', { name: 'Message to this session' });
    fireEvent.change(box(), { target: { value: 'half a thought' } });
    await open(B);
    speak(B);
    expect(box()).toHaveValue('');
    fireEvent.keyDown(box(), { key: 'Enter' });
    await drain();
    expect(router.posts('/api/live/send')).toEqual([]);
    // Back in the session it was typed in, it is still there.
    await open(A);
    expect(box()).toHaveValue('half a thought');
  });

  it('SE-C192, SE-C108: a half-answered question form stays with its session, and the same question in another starts unanswered', async () => {
    stubRouter();
    renderSessions(`/live?session=${A}`);
    await waitFor(() => streamOf(A));
    speak(A, ASK);
    await press(view(A).getByRole('radio', { name: /Beside the feed/ }));
    expect(view(A).getByRole('radio', { name: /Beside the feed/ })).toBeChecked();
    await open(B);
    speak(B, ASK);
    expect(view(B).getByRole('radio', { name: /Beside the feed/ })).not.toBeChecked();
    expect(view(B).getByRole('radio', { name: /Over the feed/ })).not.toBeChecked();
    // Its own session's replay draws the form again, as it was left.
    await open(A);
    speak(A, ASK);
    expect(view(A).getByRole('radio', { name: /Beside the feed/ })).toBeChecked();
  });

  it('SE-C136, SE-C193: a pending model switch stays with its session — it names that session, and its outcome never lands in another', async () => {
    const slow = held();
    const router = stubRouter((req) => (req.method === 'POST' && req.path === '/api/live/model' ? slow.reply(req) : undefined));
    renderSessions(`/live?session=${A}`);
    await waitFor(() => streamOf(A));
    speak(A);
    const model = () => screen.getByRole('combobox', { name: 'Model' });
    await waitFor(() => expect(model()).toHaveDisplayValue('model: opus-4-1'));
    fireEvent.change(model(), { target: { value: 'sonnet' } });
    await waitFor(() => expect(model()).toBeDisabled());
    expect(router.posts('/api/live/model').map((p) => p.body)).toEqual([{ session: A, model: 'sonnet' }]);
    await open(B);
    speak(B);
    await waitFor(() => expect(model()).toHaveDisplayValue('model: sonnet-4-5'));
    expect(model()).toBeEnabled();
    act(() => slow.release(json({ ok: true, model: 'sonnet', notices: ['Set model to Sonnet for this session'] })));
    await drain();
    expect(screen.queryByText('Set model to Sonnet for this session')).toBeNull();
    expect(model()).toHaveDisplayValue('model: sonnet-4-5');
    expect(model()).toBeEnabled();
  });

  it('SE-C136, SE-C193: a slow read for one session is dropped once another is on screen, and never dresses it', async () => {
    const slow = held();
    stubRouter((req) => (req.method === 'GET' && req.path === '/api/live/stats' && req.query.get('session') === A ? slow.reply(req) : undefined));
    renderSessions(`/live?session=${A}`);
    await waitFor(() => streamOf(A));
    speak(A);
    expect(screen.getByText('reading totals…')).toBeInTheDocument();
    await open(B);
    speak(B);
    await waitFor(() => expect(screen.getByText('sonnet-4-5')).toBeInTheDocument());
    act(() => slow.release(json({ model: 'claude-opus-4-1', effort: 'high', mode: 'default', total: 1, output: 1, cache_read: 0, turns: 99, cost: 1 })));
    await drain();
    expect(screen.queryByText('opus-4-1')).toBeNull();
    expect(screen.queryByText('99 turns')).toBeNull();
    expect(screen.getByRole('combobox', { name: 'Model' })).toHaveDisplayValue('model: sonnet-4-5');
  });

  it('SE-C119: a question form answers the session it was drawn in, even when another is open by the time the router replies', async () => {
    const slow = held();
    let holding = false;
    const router = stubRouter((req) => {
      // From the press of Submit on, what waits in the first session is slow to say.
      if (holding && req.method === 'GET' && req.path === '/api/live/prompt' && req.query.get('session') === A) return slow.reply(req);
      if (req.method === 'POST' && req.path === '/api/live/answer') return json({ ok: true, request_id: 'rq-a' });
      return undefined;
    });
    renderSessions(`/live?session=${A}`);
    await waitFor(() => streamOf(A));
    speak(A, ASK);
    await press(view(A).getByRole('radio', { name: /Over the feed/ }));
    holding = true;
    await press(view(A).getByRole('button', { name: 'Submit answers' }));
    expect(slow.waiting()).toBeGreaterThan(0);
    await open(B);
    speak(B);
    act(() => slow.release(json({ pending: true, question: true, request_id: 'rq-a', input: { questions: ASK.ask.questions }, count: 1 })));
    await waitFor(() => expect(router.posts('/api/live/answer')).toHaveLength(1));
    expect(router.posts('/api/live/answer')[0]?.body).toEqual({ session: A, answers: { 'Where should the list sit on a desk?': 'Over the feed' }, request_id: 'rq-a' });
    // Nothing it says lands in the session now on screen.
    expect(screen.queryByText('answered')).toBeNull();
    expect(router.posts('/api/live/send')).toEqual([]);
  });

  it('SE-C129, SE-C192: a permission card belongs to the session it was drawn for — another session opened clears it at once', async () => {
    const router = stubRouter((req) =>
      req.method === 'GET' && req.path === '/api/live/prompt' && req.query.get('session') === A
        ? json({ pending: true, request_id: 'rq-perm', tool: 'Bash', display_name: 'Bash', input: { command: 'rm -r build' }, suggestions: [], count: 1 })
        : undefined,
    );
    renderSessions(`/live?session=${A}`);
    await waitFor(() => streamOf(A));
    speak(A);
    await waitFor(() => expect(screen.getByText('Bash is waiting for permission')).toBeInTheDocument());
    const before = router.gets('/api/live/prompt').length;
    await open(B);
    expect(screen.queryByText('Bash is waiting for permission')).toBeNull();
    speak(B);
    await waitFor(() => expect(router.gets('/api/live/prompt').length).toBeGreaterThan(before));
    expect(router.gets('/api/live/prompt').slice(before).every((r) => r.query.get('session') === B)).toBe(true);
    expect(screen.queryByText('Bash is waiting for permission')).toBeNull();
  });

  it('SE-C192: the arrow history and the notices belong to one session and never follow the reader to another', async () => {
    stubRouter((req) => (req.method === 'POST' && req.path === '/api/live/send' ? json({ ok: true }) : undefined));
    renderSessions(`/live?session=${A}`);
    await waitFor(() => streamOf(A));
    speak(A, { role: 'user', kind: 'text', text: 'said in a' });
    const box = () => screen.getByRole('textbox', { name: 'Message to this session' });
    fireEvent.keyDown(box(), { key: 'ArrowUp' });
    await waitFor(() => expect(box()).toHaveValue('said in a'));
    fireEvent.change(box(), { target: { value: 'sent from a' } });
    await press(screen.getByRole('button', { name: 'Send' }));
    await waitFor(() => expect(screen.getByText('Delivered as your message.')).toBeInTheDocument());
    await open(B);
    speak(B);
    expect(screen.queryByText('Delivered as your message.')).toBeNull();
    fireEvent.keyDown(box(), { key: 'ArrowUp' });
    expect(box()).toHaveValue('');
  });
});
