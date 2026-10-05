// @vitest-environment node
// New for agent-router: the session model's contracts beyond what the old checks pinned, among them
// the ones the live view broke (docs/web-console-contracts.md, "Sessions").
import { describe, expect, it } from 'vitest';
import { base, live, OPTIONS } from './fixtures/live';
import { nonsense } from './fixtures/walk';
import {
  answerTarget,
  asideNotice,
  composeAnswers,
  feedAside,
  feedEcho,
  feedError,
  feedHeader,
  feedMessage,
  feedRefused,
  feedRowView,
  feedStatus,
  feedTick,
  hitTarget,
  isCurrent,
  listHeader,
  listView,
  openFeed,
  permissionCard,
  permissionSubject,
  projectOf,
  questionTabs,
  QUEUED,
  SIDE_QUESTION,
  sendNotice,
  silent,
  switcherOptions,
  switchNotice,
  tabsFor,
  vouched,
  type ListView,
  type RowView,
  type SessionRow,
} from './session';

const NOW = Date.UTC(2026, 9, 5, 12, 0, 0);
const { act } = live(NOW);

const rowsOf = (v: ListView): RowView[] => [...v.recent.flatMap((g) => g.rows), ...(v.older?.groups.flatMap((g) => g.rows) ?? []), ...(v.shelf?.rows ?? [])];
const rowOf = (v: ListView, id: string): RowView | undefined => rowsOf(v).find((r) => r.id === id);

describe('the session list', () => {
  it('SE-C22: a row is drawn from whatever keys it has, and a minimal one blanks nothing, where one without an id stopped the list', () => {
    const rows = [{ session_id: 'min-1' }, { cwd: '/code/x' } as SessionRow, ...base()];
    const view = listView(rows, NOW);
    expect(rowsOf(view)).toHaveLength(rows.length);
    expect(nonsense(view)).toEqual([]);
    expect(rowOf(view, 'min-1')).toMatchObject({ name: '—', age: '—', line: null, tone: null, halo: false, reach: 'view' });
  });

  it("SE-C23: a session with no cwd is grouped under the router's project, which the page's `base` never reached", () => {
    expect(listView([{ session_id: 's-1', project: 'p9', last_activity_ms: NOW, active: true }], NOW).recent.map((g) => g.key)).toEqual(['p9']);
    expect([projectOf({ cwd: '/code/a/' }), projectOf({ cwd: 'C:\\code\\win-app' }), projectOf({})]).toEqual(['a', 'win-app', '—']);
  });

  it('SE-C24: projects alphabetically, so a project keeps its place, and sessions newest first within one', () => {
    const view = listView(
      [
        { session_id: 'b-1', cwd: '/code/beta', last_activity_ms: NOW, active: true },
        { session_id: 'a-1', cwd: '/code/alpha', last_activity_ms: NOW - 60e3, active: true },
        { session_id: 'a-2', cwd: '/code/alpha', last_activity_ms: NOW - 1e3, active: true },
      ],
      NOW,
    );
    expect(view.recent.map((g) => [g.key, g.rows.map((r) => r.id)])).toEqual([
      ['alpha', ['a-2', 'a-1']],
      ['beta', ['b-1']],
    ]);
  });

  it('SE-C32, SE-C33: the shelf holds exactly what the router marks archived, flat and newest first, however recently it moved', () => {
    const view = listView(
      [
        { session_id: 'old', cwd: '/code/a', archived: true, last_activity_ms: NOW - 3 * 3600e3 },
        { session_id: 'new', cwd: '/code/a', archived: true, last_activity_ms: NOW, active: true },
        { session_id: 'here', cwd: '/code/a', last_activity_ms: NOW, active: true },
      ],
      NOW,
    );
    expect(view.shelf?.label).toBe('2 archived');
    // The page listed the shelf in the router's order.
    expect(view.shelf?.rows.map((r) => r.id)).toEqual(['new', 'old']);
    expect(view.recent.flatMap((g) => g.rows.map((r) => r.id))).toEqual(['here']);
  });

  it("SE-C43: ages, the older fold and the tab window are measured on the router's clock, where the page used this machine's", () => {
    // The router's clock is eight days behind this machine's: measured here, every session would be
    // days old, folded away and out of the tabs.
    const asOf = NOW - 8 * 86400e3;
    const r = act('r-1', 'idle', { tone: 'idle', detail: 'Waiting for you', last_activity_ms: asOf - 5 * 60e3, as_of_ms: asOf, since_ms: asOf - 5 * 60e3, active: false });
    const view = listView([r], NOW);
    expect(view.older).toBeNull();
    expect([rowOf(view, 'r-1')?.age, rowOf(view, 'r-1')?.line]).toEqual(['5m', 'Waiting for you · 5m']);
    expect(isCurrent(r, NOW)).toBe(true);
    expect(listHeader([r], NOW).text).toBe('idle · 5m');
  });

  it("SE-C20: every dot has its words, a heading's and a tab's included, which the page left as bare colour", () => {
    const rows = [
      act('w-1', 'waiting', { tone: 'attention', active: true, detail: 'Bash wants to run', created_ms: 1 }),
      act('i-1', 'idle', { tone: 'idle', active: false, detail: 'Waiting for you', created_ms: 2 }),
    ];
    const view = listView(rows, NOW);
    expect(view.recent[0]?.label).toBe('waiting for you');
    expect([rowOf(view, 'w-1')?.label, rowOf(view, 'i-1')?.label]).toEqual(['waiting for you', 'idle']);
    expect(tabsFor(rows[1]!, rows, null, NOW).map((t) => t.spoken)).toEqual(['waiting for you', 'idle']);
    expect(listView([{ session_id: 'legacy', cwd: '/c', state: 'ask', last_activity_ms: NOW }], NOW).recent[0]?.rows[0]?.label).toBe('waiting for you');
  });
});

describe('the stream', () => {
  it('SE-C59, SE-C60: the router refusing a stream is said as such and never retried, where the page read "reconnecting…" for good', () => {
    let feed = openFeed(NOW);
    feed = feedRefused(feed, feed.connection, 'no such session');
    expect(feedHeader(feed)).toEqual({ tone: 'stale', text: 'no such session', stop: false });
    expect(feedTick(feed, NOW + 60e3).reconnect).toBe(false);
    expect(silent(feed, NOW + 60e3)).toBe(false);
    expect(feedError(feed, feed.connection)).toBe(feed);
  });

  it("SE-C63: a status with no detail shows its state's word, where the page left the header blank", () => {
    const feed = feedStatus(openFeed(NOW), 1, JSON.stringify({ state: 'idle', detail: '', quiet_ms: 300e3, queued: 0 }), NOW);
    expect(feedHeader(feed).text).toBe('idle · 5m');
  });

  it('SE-C72: a row that is not a row is skipped and the rows after it land; a status that is not JSON is not believed, though it was heard', () => {
    let feed = openFeed(NOW);
    feed = feedMessage(feed, 1, '{not json', NOW);
    feed = feedMessage(feed, 1, '[1,2]', NOW);
    feed = feedMessage(feed, 1, JSON.stringify({ role: 'user', kind: 'text', text: 'hi' }), NOW);
    expect(feed.entries).toHaveLength(1);
    feed = feedStatus(feed, 1, 'nope', NOW + 5000);
    expect([feed.phase, feed.heard, vouched(feed)]).toEqual(['connecting', NOW + 5000, null]);
  });

  it("SE-C155: ArrowUp offers the person's own messages, a consecutive repeat once, the newest 200", () => {
    let feed = openFeed(NOW);
    const typed = (text: string, extra: object = {}) => {
      feed = feedMessage(feed, 1, JSON.stringify({ role: 'user', kind: 'text', text, ...extra }), NOW);
    };
    typed('a');
    typed('a');
    typed('b');
    typed('from a subagent', { sidechain: true });
    feed = feedMessage(feed, 1, JSON.stringify({ role: 'assistant', kind: 'text', text: 'c' }), NOW);
    expect(feed.sent).toEqual(['a', 'b']);
    for (let n = 0; n < 250; n++) typed(`m${n}`);
    expect([feed.sent.length, feed.sent[199]]).toEqual([200, 'm249']);
  });

  it('SE-C150, SE-C160: a send is echoed "you · queued", a side question "you · side question", and its answer lands as a side answer, "(no answer)" when empty; none of them is history', () => {
    let feed = openFeed(NOW);
    feed = feedEcho(feed, 'sent at once');
    feed = feedEcho(feed, 'what is this?', SIDE_QUESTION);
    feed = feedAside(feed, '**it is** that');
    feed = feedAside(feed, '');
    expect(feed.entries.map((e) => (e.kind === 'echo' ? `${e.label}: ${e.text}` : feedRowView(e.item).who))).toEqual([
      `${QUEUED}: sent at once`,
      'you · side question: what is this?',
      'side answer',
      'side answer',
    ]);
    const last = feed.entries[3];
    expect(last?.kind === 'row' && last.item.text).toBe('(no answer)');
    expect(feed.sent).toEqual([]);
  });
});

describe('the feed', () => {
  it('SE-C76 to SE-C84: a row is classified by its kind before its role', () => {
    // A tool result, which the transcript writes under the user role, is never the person's words.
    expect(feedRowView({ role: 'user', kind: 'tool_result', text: 'ok' }).kind).toBe('result');
    expect(feedRowView({ role: 'user', kind: 'text', text: '**as typed**' })).toMatchObject({ kind: 'person', blocks: null });
    expect(feedRowView({ role: 'assistant', kind: 'text', text: '**md**' }).blocks).toEqual([{ type: 'paragraph', children: [{ type: 'bold', children: [{ type: 'text', text: 'md' }] }] }]);
    expect(feedRowView({ role: 'assistant', kind: 'tool_use', name: 'Bash', text: 'ls', dur_ms: 2810 })).toMatchObject({ kind: 'tool', who: 'Bash', shell: true, duration: '2.8s' });
    expect(feedRowView({ role: 'assistant', kind: 'tool_use', name: 'Read', text: '/x' }).shell).toBe(false);
    expect([feedRowView({ kind: 'thinking', text: '…', dur_ms: 2400 }).who, feedRowView({ kind: 'thinking', text: '…', dur_ms: 400 }).who]).toEqual(['Thought for 2.4s ›', 'Thought ›']);
    expect(feedRowView({ kind: 'interrupt', role: 'user', text: '[Request interrupted by user]' })).toMatchObject({ kind: 'interrupt', who: '' });
    expect(feedRowView({ role: 'assistant', kind: 'text', text: 'x', sidechain: true })).toMatchObject({ who: 'subagent', subagent: true });
    expect([feedRowView({ kind: 'answer', text: 'B' }).who, feedRowView({ kind: 'command', text: '/compact' }).who]).toEqual(['✓ answered', '⌘ command']);
    expect(feedRowView({ role: 'assistant', kind: 'text', text: 'x', dur_ms: 300 }).duration).toBeNull();
    expect(feedRowView({ role: 'mystery', kind: 'new-kind', text: 'x' })).toMatchObject({ kind: 'other', who: 'mystery' });
  });
});

describe('answers and prompts', () => {
  it("SE-C105, SE-C109, SE-C110: an answer is the chosen labels in the tool's order, then the typed Other; a label round-trips exactly", () => {
    const ask = {
      questions: [
        { question: 'Colour?', header: 'Colour', options: [{ label: 'Red' }, { label: 'Blue' }] },
        { question: 'Toppings?', header: 'Toppings', multiSelect: true, options: [{ label: 'Olives' }, { label: 'Basil' }, { label: 'Chili' }] },
        { question: 'Name?', options: [] },
        { question: 'Quote?', options: [{ label: 'say "hi" & <b>' }] },
      ],
    };
    const { answers, lines } = composeAnswers(ask, [{ chosen: ['Blue'] }, { chosen: ['Chili', 'Olives'], other: '  extra cheese ' }, { chosen: [] }, { chosen: ['say "hi" & <b>'] }]);
    expect(answers).toEqual({ 'Colour?': 'Blue', 'Toppings?': 'Olives, Chili, extra cheese', 'Quote?': 'say "hi" & <b>' });
    expect(lines).toEqual(['Colour: Blue', 'Toppings: Olives, Chili, extra cheese', 'Quote?: say "hi" & <b>']);
    // Typed Other alone is an answer; Other with nothing typed adds nothing.
    expect(composeAnswers(ask, [undefined, { chosen: [], other: '   ' }, { chosen: [], other: ' Sam ' }]).answers).toEqual({ 'Name?': 'Sam' });
    expect(questionTabs(ask)).toEqual(['Colour', 'Toppings', 'Question 3', 'Question 4']);
  });

  it('SE-C113: a prompt that does not list its questions, or not as a list, is taken at its word; every question counts', () => {
    const ask = { questions: [{ question: 'Which?' }] };
    expect(answerTarget({ pending: true, question: true, request_id: 'q1' }, ask)).toEqual({ kind: 'named', requestId: 'q1' });
    expect(answerTarget({ pending: true, question: true, request_id: 'q2', input: { questions: 'Which?' } }, ask)).toEqual({ kind: 'named', requestId: 'q2' });
    expect(answerTarget({ pending: true, question: true, request_id: 'q3', input: { questions: [{ question: 'Which?' }, { question: 'And?' }] } }, ask).kind).toBe('refused');
  });

  it('SE-C121, SE-C127, SE-C128: a card says what waits and on what, is redrawn when its prompt or count changes, and is not drawn again once answered', () => {
    const p = { pending: true, request_id: 'r-1', tool: 'Bash', question: false, count: 1 };
    expect([permissionCard(p)?.seen, permissionCard({ ...p, count: 3 })?.seen]).toEqual(['r-1#1', 'r-1#3']);
    // The next poll could draw again the card just answered, on the page.
    expect(permissionCard(p, new Set(['r-1']))).toBeNull();
    expect([permissionCard({ ...p, question: true }), permissionCard({ pending: false })]).toEqual([null, null]);
    expect(permissionCard({ pending: true, request_id: 'r', count: 1 })?.what).toBe('A tool is waiting for permission');
    expect(permissionCard({ pending: true, request_id: 'r', tool: 'Write', input: { file_path: 'x'.repeat(450) } })?.subject).toBe(`${'x'.repeat(400)}…`);
    expect(permissionSubject({ query: 'q' })).toBe('{"query":"q"}');
  });

  it('SE-C139, SE-C140: a switcher leads with the current value, and says when there is none, where the page showed its first option', () => {
    expect(switcherOptions('effort', null, OPTIONS).map((o) => o.label)).toEqual(['effort: unknown', 'low', 'medium', 'high']);
    expect(switcherOptions('mode', 'default', OPTIONS)).toEqual([{ value: 'default', label: 'mode: Ask first', current: true }]);
    expect(switcherOptions('model', 'claude-haiku-4', OPTIONS).map((o) => o.label)).toEqual(['model: haiku-4', 'opus', 'sonnet', 'other…']);
  });

  it("SE-C143, SE-C144: a switch says what the CLI said, else pending or set, and a refusal in the router's words", () => {
    expect(switchNotice('effort', 'high', 200, { ok: true, notices: ['Effort set to high'] })).toEqual({ text: 'Effort set to high', ok: true, ms: 6000 });
    expect(switchNotice('model', 'opus', 200, { ok: true, pending: true })?.text).toBe('model sent — the session has not confirmed yet');
    expect(switchNotice('model', 'opus', 200, { ok: true })?.text).toBe('model set to opus — takes effect on the next turn');
    expect(switchNotice('mode', 'bypassPermissions', 400, { ok: false, reason: 'not launched for bypass' })).toEqual({ text: 'not launched for bypass', ok: false, ms: null });
  });

  it('SE-C150 to SE-C153, SE-C160, SE-C184: an action says how it went, and a 2xx carrying ok:false is a failure', () => {
    expect(sendNotice(200, { ok: true })).toEqual({ text: 'Delivered as your message.', ok: true, ms: 4000 });
    expect(sendNotice(200, { ok: true, framed: true })?.text).toMatch(/peer message/);
    expect(sendNotice(200, { ok: false, reason: 'not wrapped' })?.text).toBe('not wrapped');
    expect(sendNotice(500, {})?.text).toBe('Send failed (500)');
    expect(asideNotice(200, { answer: 'x', took_ms: 41000, cost: 0.0123 })).toEqual({ text: 'answered in 41s · $0.012', ok: true, ms: 6000 });
  });

  it('SE-C173: opening a hit whose list could not be read says so, never "no longer listed", where the page said it was gone', () => {
    expect(hitTarget('x', { failed: 'connection refused' }, false)).toEqual({ say: 'Could not read the session list — connection refused' });
    expect(hitTarget('x', { rows: [] }, false)).toEqual({ say: 'That session is no longer listed.' });
    const row = { session_id: 'x' };
    expect(hitTarget('x', { rows: [row] }, true)).toEqual({ row });
  });
});
