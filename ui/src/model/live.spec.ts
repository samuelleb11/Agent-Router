// @vitest-environment node
// New for agent-router: the checks of scripts/check-live-ui.js, one spec each, in its order, over
// the modules underneath the old page. The old script drove the page's inline script against a
// fake DOM and a fake EventSource; these drive the session model with an explicit clock, the way
// the old script moved `Date.now` rather than waiting out ten real seconds (SE-C211). Where a check
// is about drawing that has no screen yet, its comment names the slice that pins the drawing:
// AGEN-0012, the Sessions screen. They run in Node, with no DOM at all.
//
// One check exists only in the working copy of scripts/check-live-ui.js, uncommitted when this was
// written: "sessions from two machines say which, and one project on each stays apart". It is
// ported too. The hosted router's wording is the committed page's (SE-C187).
import { existsSync, readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import wire from './fixtures/wire.sse?raw';
import { base, HOSTED_ERROR, live, OPTIONS, search as searchReply, third } from './fixtures/live';
import { frames } from './fixtures/sse';
import { nonsense } from './fixtures/walk';
import { marks } from './format';
import { md } from './markdown';
import { findPlaceholder, findScopes, findView } from './search';
import {
  AGENTLESS,
  OFF_MACHINE,
  answerBody,
  answerTarget,
  asideNotice,
  asksWrapper,
  closeFind,
  composeAnswers,
  emptyList,
  feedEcho,
  feedError,
  feedHeader,
  feedMessage,
  feedRefused,
  feedRowView,
  feedStatus,
  feedTick,
  formOutcome,
  header,
  headingOf,
  hitTarget,
  listHeader,
  listView,
  notice,
  openFeed,
  openFind,
  permissionAnswer,
  permissionCard,
  permissionOutcome,
  refusal,
  sendNotice,
  sessionIn,
  silent,
  stoppable,
  stopNotice,
  stripShown,
  switcherOptions,
  switchNotice,
  tabsFor,
  vouched,
  withoutSession,
  withSession,
  wrapperWarning,
  type Feed,
  type GroupView,
  type ListView,
  type LiveView,
  type Pip,
  type Prompt,
  type RowView,
  type SessionRow,
  type Status,
  type Tone,
} from './session';
import { isHosted } from './settings';

/// The moment every spec here starts at; the router's clock is seven minutes behind it.
const NOW = Date.UTC(2026, 9, 5, 12, 0, 0);
const { at: AT, act, activities, status } = live(NOW);

/// Every model module's source, for the checks about the code itself.
const SOURCES = import.meta.glob(['./*.ts', '!./*.spec.ts'], { query: '?raw', import: 'default', eager: true }) as Record<string, string>;
/// The browser globals the console uses, which no name of its own may shadow (SE-C217).
const GLOBALS = ['history', 'location', 'name', 'status', 'top', 'self', 'parent', 'origin', 'screen', 'event', 'find'];

// --- reading what the model drew, in the words the old checks read the page in ---

/// A dot as the page's classes spelled it: its tone, then the halo.
const dot = (x: { readonly tone: Tone | null; readonly halo: boolean } | undefined): string[] =>
  x ? [...(x.tone ? [`t-${x.tone}`] : []), ...(x.halo ? ['on'] : [])] : ['(no such dot)'];
/// The header's pip as the page's class spelled it.
const pip = (p: Pip): string => (p.tone === 'stale' ? 'stale' : p.tone ? `t-${p.tone}` : '');
const rowsOf = (v: ListView): RowView[] => [...v.recent.flatMap((g) => g.rows), ...(v.older?.groups.flatMap((g) => g.rows) ?? []), ...(v.shelf?.rows ?? [])];
const rowOf = (v: ListView, id: string): RowView => {
  const r = rowsOf(v).find((x) => x.id === id);
  if (!r) throw new Error(`no row for ${id}`);
  return r;
};
/// A project's heading on the list itself, as the page's `headingOf` found it: not inside "older".
const headingOf2 = (v: ListView, key: string): GroupView | undefined => v.recent.find((g) => g.key === key);
const json = (x: unknown): string => JSON.stringify(x);
const said = (feed: Feed, s: Status, now = NOW): Feed => feedStatus(feed, feed.connection, json(s), now);
const row = (text: string) => ({ role: 'assistant', kind: 'text', text });
const TONE_CLASSES = new Set(['busy', 'attention', 'warn', 'error', 'idle', 'closed'].map((t) => `t-${t}`));

/// Replay a stream through the session model as the router wrote it and a browser delivers it:
/// every feed row to the feed, every `status` to the header, and the header read after each
/// status. A row that did not land is a failure, and so is a header that reads "undefined", "NaN"
/// or "null" where the router said no such thing, says nothing, or wears a colour that is not a
/// known tone — or not the one the router chose (SE-C214).
function replay(text: string, what: string): { items: number; headers: [string, string][] } {
  let feed = openFeed(NOW);
  let items = 0;
  const headers: [string, string][] = [];
  for (const e of frames(text)) {
    if (e.type === 'message') {
      const before = feed.entries.length;
      feed = feedMessage(feed, feed.connection, e.data, NOW);
      expect(feed.entries.length, `row ${items + 1} of ${what} did not land: ${e.data.slice(0, 160)}`).toBe(before + 1);
      // Drawn, too: the page swallowed a row that threw, and here that is a failure.
      const last = feed.entries[feed.entries.length - 1];
      if (last?.kind === 'row') expect(() => feedRowView(last.item), `row ${items + 1} of ${what} did not draw`).not.toThrow();
      items++;
      continue;
    }
    // The router's own `error` event: it will not stream this session (SE-C59).
    if (e.type === 'error') feed = feedRefused(feed, feed.connection, e.data);
    if (e.type !== 'status') continue;
    let s: Status;
    try {
      s = JSON.parse(e.data) as Status;
    } catch {
      throw new Error(`status ${headers.length + 1} of ${what} is not JSON: ${e.data.slice(0, 160)}`);
    }
    feed = feedStatus(feed, feed.connection, e.data, NOW);
    const h = feedHeader(feed);
    const read: [string, string] = [pip(h), h.text];
    headers.push(read);
    const chosen = s.tone && TONE_CLASSES.has(`t-${s.tone}`) ? `t-${s.tone}` : null;
    // Only what the model added: an API error's own text may well say "null".
    const added = s.detail ? h.text.replace(s.detail, '') : h.text;
    const where = `status ${headers.length} of ${what} read ${json(read)}: ${e.data.slice(0, 200)}`;
    expect(TONE_CLASSES.has(read[0]) || read[0] === 'stale', where).toBe(true);
    if (chosen) expect(read[0], where).toBe(chosen);
    expect(h.text, where).not.toBe('');
    expect(added, where).not.toMatch(/undefined|NaN|null/);
  }
  if (!items && !headers.length) throw new Error(`nothing to replay in ${what}`);
  return { items, headers };
}

describe('the live view, as scripts/check-live-ui.js checked it', () => {
  // ported from scripts/check-live-ui.js: script evaluates
  it('SE-C218: script evaluates — the session model loads and runs without a browser', async () => {
    expect(typeof document).toBe('undefined');
    const modules = import.meta.glob(['./*.ts', '!./*.spec.ts']);
    expect(Object.keys(modules)).toEqual(expect.arrayContaining(['./session.ts', './markdown.ts', './format.ts', './search.ts']));
    for (const [path, load] of Object.entries(modules)) {
      const mod = (await load()) as Record<string, unknown>;
      expect(Object.keys(mod).length, path).toBeGreaterThan(0);
    }
  });

  // ported from scripts/check-live-ui.js: a local does not shadow a browser global the page uses
  it('SE-C217: a local does not shadow a browser global the page uses', () => {
    for (const [path, src] of Object.entries(SOURCES)) {
      const declared = new Set([...src.matchAll(/(?:let|const|var|function|class)\s+([A-Za-z_$][\w$]*)/g)].map((m) => m[1]));
      const clash = GLOBALS.filter((g) => declared.has(g));
      expect(clash, `${path} shadows ${clash.join(', ')} — see the \`history\` bug that stopped cards opening`).toEqual([]);
    }
  });

  // ported from scripts/check-live-ui.js: opening a session does not throw
  it('SE-C190, SE-C194: opening a session does not throw', () => {
    const r: SessionRow = { session_id: 'abc-1', cwd: '/code/project-a', project: 'p', git_branch: 'dev', title: 't', last_activity_ms: NOW, active: true, wrapped: true };
    // The open session lives in the address, so a refresh comes back to it (SE-C189).
    expect(withSession('http://host/live', r.session_id)).toBe('/live?session=abc-1');
    expect(sessionIn('http://host/live?session=abc-1')).toBe('abc-1');
    // Another session's rows go at once, and until the stream speaks the header has nothing true
    // to say. The screen itself is AGEN-0012's.
    const feed = openFeed(NOW);
    expect(feed.entries).toEqual([]);
    expect(feedHeader(feed)).toEqual({ tone: 'stale', text: 'connecting…', stop: false });
    expect(tabsFor(r, [], null, NOW)).toEqual([]);
    expect(headingOf({ session: r.session_id, find: null }, r)).toBe('project-a');
  });

  // ported from scripts/check-live-ui.js: going back to the list does not throw
  it('SE-C190, SE-C195: going back to the list does not throw', () => {
    expect(withoutSession('http://host/live?session=abc-1')).toBe('/live');
    expect(sessionIn('http://host/live')).toBeNull();
    const view = listView(base(), NOW);
    expect(view.recent.map((g) => g.key)).toEqual(['project-a']);
    expect(view.older?.groups.map((g) => g.key)).toEqual(['project-a', 'project-b']);
    expect(header({ feed: null, list: { rows: base() } }, NOW).text).toBe('1 active');
    expect(headingOf({ session: null, find: null }, null)).toBe('Live sessions');
  });

  // ported from scripts/check-live-ui.js: notices do not recurse
  it('SE-C181: notices do not recurse', () => {
    // Clearing once called itself instead of clearing, which blew the stack inside openSession and
    // left every session blank.
    expect(notice('something happened')).toEqual({ text: 'something happened', ok: false, ms: null });
    expect(notice('', {})).toBeNull();
    expect(notice('and again', { ok: true, for: 10 })).toEqual({ text: 'and again', ok: true, ms: 10 });
  });

  // ported from scripts/check-live-ui.js: markdown survives hostile input
  it('SE-C90: markdown survives hostile input', () => {
    const t = (text: string) => ({ type: 'text', text });
    // A block tree with the text as text: the <img> is words for React to escape, never an element.
    expect(md('<img src=x onerror=alert(1)>\n**b** `c`\n- one\n\n| a | b |\n|---|---|\n| 1 | 2 |')).toEqual([
      { type: 'paragraph', children: [t('<img src=x onerror=alert(1)>')] },
      { type: 'paragraph', children: [{ type: 'bold', children: [t('b')] }, t(' '), { type: 'code', text: 'c' }] },
      { type: 'list', items: [[t('one')]] },
      { type: 'table', head: [[t('a')], [t('b')]], rows: [[[t('1')], [t('2')]]] },
    ]);
  });

  // ported from scripts/check-live-ui.js: the session list renders every state
  it('SE-C21: the session list renders every state', () => {
    // Every word either router can say, in the list and in the open view, and one neither has
    // said yet. Nothing may throw, and nothing may read "undefined" or "NaN".
    const rows = [...base(), ...activities(), ...['stalled', 'idle', 'mystery'].map((st, i): SessionRow => ({ ...base()[0]!, session_id: `leg-${i}`, state: st }))];
    const view = listView(rows, NOW);
    expect(rowsOf(view).length).toBe(rows.length);
    expect(nonsense(view)).toEqual([]);
    for (const st of ['working', 'ask', 'idle', 'stalled', 'failed', 'tool', 'mystery']) {
      expect(nonsense(feedHeader(said(openFeed(NOW), { state: st, detail: 'Working', quiet_ms: 0, queued: 0 }))), st).toEqual([]);
    }
    for (const r of activities()) {
      const h = feedHeader(said(openFeed(NOW), { ...r, quiet_ms: 12e3, queued: 0 }));
      expect(nonsense(h, [r.detail ?? '']), r.session_id).toEqual([]);
    }
  });

  // ported from scripts/check-live-ui.js: tabs cover the project and keep creation order
  it('SE-C46: tabs cover the project and keep creation order', () => {
    const rows = base();
    const labels = tabsFor(rows[0]!, rows, null, NOW).map((t) => t.label);
    expect(labels.length, 'two tabs for the project').toBe(2);
    expect(labels[0], 'oldest leads').toBe('Older one');
  });

  // ported from scripts/check-live-ui.js: sessions from two machines say which, and one project on each stays apart
  it('sessions from two machines say which, and one project on each stays apart', () => {
    // Only in the working copy of scripts/check-live-ui.js and assets/live.html, uncommitted: a
    // hosted router lists several machines' sessions (docs/hosting-machine-agent.md). Both recent,
    // so neither folds into the older section.
    const [a, b] = base().map((r) => ({ ...r, last_activity_ms: NOW }));
    if (!a || !b) throw new Error('the fixture lost its rows');
    const mac = { id: 'm-1', label: 'mac' };
    const win = { id: 'm-2', label: 'win-box' };
    let view = listView([{ ...a, machine: mac }, { ...b, machine: win }], NOW);
    const groups = view.recent.map((g) => g.key);
    expect(groups, 'one project on two machines is two groups').toEqual(expect.arrayContaining(['project-a · mac', 'project-a · win-box']));
    expect(rowOf(view, 'aaa-1').meta, 'a card names its machine').toContain(' · on mac');
    view = listView([{ ...a, machine: mac }, { ...b, machine: mac }], NOW);
    expect(rowsOf(view).some((r) => r.meta.includes(' · on ')), 'one machine needs no naming').toBe(false);
  });

  // ported from scripts/check-live-ui.js: archived sessions leave the tabs, but come back when live
  it('SE-C48: archived sessions leave the tabs, but come back when live', () => {
    const [open, other, elsewhere] = base();
    if (!open || !other || !elsewhere) throw new Error('the fixture lost its rows');
    const shelved: SessionRow = { ...other, archived: true, active: false, state: 'idle', last_activity_ms: NOW };
    const labels = (rows: SessionRow[]) => tabsFor(open, rows, null, NOW).map((t) => t.label);
    expect(labels([open, shelved, elsewhere, third(NOW)]), 'an archived session holds no tab').toEqual(['Older one', 'Third one']);
    // It wakes. `archived` arrives already decided, and the router clears it itself the moment the
    // session is busy or needs you (e784bcd), so waking is the router saying so.
    const woke: SessionRow = { ...shelved, archived: false, active: true, state: 'ask' };
    expect(labels([open, woke, elsewhere, third(NOW)]), 'an archived session that wakes reappears').toEqual(['Older one', 'Newer one', 'Third one']);
  });

  // ported from scripts/check-live-ui.js: stale sessions do not clutter the tabs
  it('SE-C44, SE-C47: stale sessions do not clutter the tabs', () => {
    const [open, other] = base();
    if (!open || !other) throw new Error('the fixture lost its rows');
    const stale: SessionRow = { ...other, archived: false, active: false, state: 'idle', last_activity_ms: NOW - 40 * 3600 * 1000 };
    expect(tabsFor(open, [open, stale, third(NOW)], null, NOW).map((t) => t.label), 'a session idle for days holds no tab').toEqual(['Older one', 'Third one']);
    // With nothing current beside it, the open session has no strip at all.
    const lone = tabsFor(open, [open, stale], null, NOW);
    expect(lone, 'a lone session draws no one-tab strip').toEqual([]);
    expect(stripShown({ session: open.session_id, find: null }, lone)).toBe(false);
  });

  // ported from scripts/check-live-ui.js: every activity and tone renders, and a word the page does not know is plain gray
  it('SE-C2, SE-C3, SE-C11, SE-C27, SE-C35: every activity and tone renders, and a word the page does not know is plain gray', () => {
    const want: Record<string, [string[], string]> = {
      'gen-1': [['t-busy', 'on'], 'Thinking · 12s'],
      'tool-1': [['t-busy', 'on'], 'Running Bash · 12s'],
      'bg-1': [['t-busy', 'on'], '2 background agents running · 12s'],
      'wait-1': [['t-attention', 'on'], 'Bash wants to run · 12s'],
      'stall-1': [['t-warn', 'on'], 'No progress · 12m'],
      'fail-1': [['t-error'], 'API error: overloaded · 12s'],
      'idle-1': [['t-idle'], 'Waiting for you · 12s'],
      'gone-1': [['t-closed'], 'Session ended · 3h ago'],
      // Unknown activity, unknown tone: no colour at all, still its words.
      'new-1': [['on'], 'Compacting · 12s'],
    };
    let view = listView(activities(), NOW);
    for (const [id, [d, line]] of Object.entries(want)) {
      expect(dot(rowOf(view, id)), `${id} dot`).toEqual(d);
      expect(rowOf(view, id).line, `${id} line (durations are measured on the router's clock)`).toBe(line);
    }
    // Only what needs a reader is coloured in the words themselves.
    expect(rowOf(view, 'wait-1').lineTone, 'a waiting line stands out').toBe('attention');
    expect(rowOf(view, 'gen-1').lineTone).toBeNull();
    // Without a tone the activity alone gives the same colours: `tone` is the router's choice when
    // it makes one, not something the screen cannot do without.
    view = listView(
      activities().map(({ tone: _tone, ...r }) => r),
      NOW,
    );
    for (const [id, [d]] of Object.entries(want)) expect(dot(rowOf(view, id)), `${id} dot, no tone`).toEqual(d);
    // The router's tone wins over the screen's own reading of the activity...
    view = listView([act('odd-1', 'tool', { tone: 'warn', active: true, detail: 'Running Bash' })], NOW);
    expect(dot(rowOf(view, 'odd-1')), 'the router picks the colour').toEqual(['t-warn', 'on']);
    // ...and a tone it does not know falls back to that reading, not to gray.
    view = listView([act('odd-2', 'waiting', { tone: 'sparkle', active: true, detail: 'Waiting for your answer' })], NOW);
    expect(dot(rowOf(view, 'odd-2')), 'an unknown tone with a known activity').toEqual(['t-attention', 'on']);
    // The project heading takes the most pressing of its sessions.
    view = listView(activities(), NOW);
    expect(dot(headingOf2(view, 'project-c')), 'the heading of a project with one waiting').toEqual(['t-attention', 'on']);
    // And the counts come from the activity.
    const h = listHeader(activities(), NOW);
    expect(h.text, 'the list header').toBe('4 working · 1 waiting');
    expect(pip(h), 'the list header pip').toBe('t-attention');
  });

  // ported from scripts/check-live-ui.js: a closed session is a hollow ring, says when it ended, and is never counted
  it('SE-C7, SE-C13, SE-C35: a closed session is a hollow ring, says when it ended, and is never counted', () => {
    const gone = (id: string, extra: Partial<SessionRow> = {}) =>
      act(id, 'closed', { tone: 'closed', state: 'idle', active: false, open: false, since_ms: undefined, detail: 'Session ended', ended_ms: AT - 3 * 3600e3, ended_how: 'exit', ...extra });
    let rows = [
      gone('gone-1'),
      gone('gone-2', { detail: 'Session ended mid-turn', ended_how: 'unknown', mid_turn: true }),
      act('idle-1', 'idle', { tone: 'idle', state: 'idle', active: false, detail: 'Waiting for you' }),
      // Stalled is still at work — the process says busy — though its older word is not `working`.
      act('stall-1', 'stalled', { tone: 'warn', state: 'stalled', active: true, detail: 'No progress' }),
    ];
    let view = listView(rows, NOW);
    expect(dot(rowOf(view, 'gone-1')), 'a closed dot').toEqual(['t-closed']);
    // Hollow, the ring the colour and the middle empty, is how the StatusDot draws `closed`; the
    // drawing is pinned with the Sessions screen, AGEN-0012. Its words say it too.
    expect(rowOf(view, 'gone-1').label).toBe('ended');
    expect(rowOf(view, 'gone-2').line, 'a session that died mid-turn').toBe('Session ended mid-turn · 3h ago');
    expect(listHeader(rows, NOW).text, 'the count').toBe('1 working');
    // A project where every session has ended says so at its heading; with nothing busy or waiting
    // anywhere, the header counts nothing and says when anything last moved.
    rows = [gone('gone-1'), gone('gone-2')];
    view = listView(rows, NOW);
    expect(dot(headingOf2(view, 'project-c')), 'a project of ended sessions').toEqual(['t-closed']);
    expect(listHeader(rows, NOW).text, 'counted a closed session').toMatch(/^idle · /);
    // Opened, it says the same, and offers no Stop.
    const feed = said(openFeed(NOW), status('closed', { state: 'idle', tone: 'closed', detail: 'Session ended', ended_ms: AT - 3 * 3600e3, since_ms: undefined, quiet_ms: 0, open: false }));
    expect(feedHeader(feed), 'the open view of a closed session').toEqual({ tone: 'closed', text: 'Session ended · 3h ago', stop: false });
  });

  // ported from scripts/check-live-ui.js: a session busy or waiting on you is never folded into "older"
  it('SE-C29, SE-C30: a session busy or waiting on you is never folded into "older"', () => {
    const old = NOW - 30 * 3600 * 1000;
    const view = listView(
      [
        act('wait-1', 'waiting', { tone: 'attention', state: 'ask', active: true, detail: 'Waiting for your answer', last_activity_ms: old }),
        act('gen-1', 'generating', { tone: 'busy', state: 'working', active: true, detail: 'Thinking', last_activity_ms: old }),
        act('stall-1', 'stalled', { tone: 'warn', state: 'stalled', active: true, detail: 'No progress', last_activity_ms: old }),
        act('idle-1', 'idle', { tone: 'idle', state: 'idle', active: false, detail: 'Waiting for you', last_activity_ms: old }),
        act('gone-1', 'closed', { tone: 'closed', state: 'idle', active: false, detail: 'Session ended', last_activity_ms: old }),
      ],
      NOW,
    );
    const folded = (view.older?.groups ?? []).flatMap((g) => g.rows.map((r) => r.id));
    expect(folded.sort(), 'what folds into older').toEqual(['gone-1', 'idle-1']);
    expect(view.older?.label, 'the fold says how many').toBe('2 older sessions');
    const recent = view.recent.flatMap((g) => g.rows.map((r) => r.id));
    for (const id of ['wait-1', 'gen-1', 'stall-1']) expect(recent, `${id} folded away`).toContain(id);
  });

  // ported from scripts/check-live-ui.js: the tab dots follow the list while a session is open
  it('SE-C51, SE-C52, SE-C53: the tab dots follow the list while a session is open', () => {
    const mk = (id: string, title: string, created: number) =>
      act(id, 'idle', { cwd: '/code/project-t', title, created_ms: created, tone: 'idle', state: 'idle', active: false, detail: 'Waiting for you' });
    let rows = [mk('tab-1', 'First', 1), mk('tab-2', 'Second', 2)];
    const open = rows[0]!;
    let feed = openFeed(NOW);
    const dots = () => tabsFor(open, rows, vouched(feed), NOW).map((t) => (t.tone ? `t-${t.tone}` : ''));
    // Still connecting, the open tab says what the list said; its stream's first word recolours it
    // at once, not at the next list refresh.
    expect(dots(), 'the dots as opened').toEqual(['t-idle', 't-idle']);
    feed = said(feed, status('tool', { tone: 'busy', detail: 'Running Bash' }));
    expect(dots(), 'the open tab once its stream speaks').toEqual(['t-busy', 't-idle']);
    // A sibling starts asking, and the next list read redraws the strip.
    rows = [rows[0]!, { ...rows[1]!, activity: 'waiting', tone: 'attention', state: 'ask', active: true, detail: 'Waiting for your answer' }];
    // The open tab says what its own stream says, like the header beside it...
    expect(dots(), 'the dots after a refresh').toEqual(['t-busy', 't-attention']);
    // ...and the refresh left the header alone: it is the open session's, not the list's.
    expect(header({ feed, list: { rows } }, NOW).text, 'the header after a refresh').toBe('Running Bash · 12s');
    // Hidden under Find, the strip stays hidden; closing Find brings it back current.
    let view: LiveView = openFind({ session: open.session_id, find: null });
    rows = [rows[0]!, { ...rows[1]!, activity: 'generating', tone: 'busy', state: 'working', detail: 'Thinking' }];
    expect(stripShown(view, tabsFor(open, rows, vouched(feed), NOW)), 'a refresh drew the tabs over Find').toBe(false);
    view = closeFind(view);
    expect(stripShown(view, tabsFor(open, rows, vouched(feed), NOW))).toBe(true);
    expect(dots(), 'the dots once Find closes').toEqual(['t-busy', 't-busy']);
  });

  // ported from scripts/check-live-ui.js: a dropped stream leaves the header stale, not showing its last word
  it('SE-C61, SE-C62, SE-C64, SE-C42: a dropped stream leaves the header stale, not showing its last word', () => {
    const rows = base();
    const before = header({ feed: null, list: { rows } }, NOW).text;
    let feed = openFeed(NOW);
    // Nothing from the stream yet: not the list's summary, and no Stop.
    expect([pip(feedHeader(feed)), feedHeader(feed).text], 'before the first status').toEqual(['stale', 'connecting…']);
    expect(feedHeader(feed).text, 'the list summary was left as the session status').not.toBe(before);
    expect(feedHeader(feed).stop, 'Stop offered before anything is known').toBe(false);

    feed = said(feed, status('tool', { tone: 'busy', detail: 'Running Bash', queued: 2 }));
    expect([pip(feedHeader(feed)), feedHeader(feed).text], 'a live status').toEqual(['t-busy', 'Running Bash · 12s · 2 queued']);
    expect(feedHeader(feed).stop, 'Stop missing while a tool runs').toBe(true);

    feed = feedError(feed, feed.connection);
    expect([pip(feedHeader(feed)), feedHeader(feed).text], 'after the stream drops').toEqual(['stale', 'reconnecting…']);
    expect(feedHeader(feed).stop, 'Stop left up on a status nobody can vouch for').toBe(false);
    // The list poll failing too, while the session is open, says nothing over the header.
    expect(header({ feed, list: { failed: 'connection refused' } }, NOW).text, 'a failed list poll while a session is open').toBe('reconnecting…');
    expect(header({ feed: null, list: { failed: 'connection refused' } }, NOW).text).toBe('offline · connection refused');

    // The browser reconnects by itself; the first status it brings is believed again.
    feed = said(feed, status('generating', { tone: 'busy', detail: 'Thinking' }));
    expect([pip(feedHeader(feed)), feedHeader(feed).text], 'after reconnecting').toEqual(['t-busy', 'Thinking · 12s']);
  });

  // ported from scripts/check-live-ui.js: a stream silent for ten seconds is taken for dead: stale, no Stop, and reconnected
  it('SE-C66, SE-C67, SE-C68, SE-C56, SE-C55: a stream silent for ten seconds is taken for dead: stale, no Stop, and reconnected', () => {
    // A connection can die without an error: a NAT that forgot it, a Mac that slept. The router
    // speaks on an open stream at least once a second, so ten seconds of nothing is the signal.
    let now = NOW;
    const later = (ms: number) => {
      now += ms;
    };
    const mk = (id: string, title: string, created: number) =>
      act(id, 'idle', { cwd: '/code/project-w', title, created_ms: created, tone: 'idle', state: 'idle', active: false, detail: 'Waiting for you' });
    const rows = [mk('w-1', 'First', 1), mk('w-2', 'Second', 2)];
    // With nothing open, nothing is watched.
    later(60e3);
    expect(silent(null, now), 'a connection watched with only the list on screen').toBe(false);

    let feed = openFeed(now);
    const first = feed.connection;
    const opened: number[] = [first];
    const tick = () => {
      const t = feedTick(feed, now);
      feed = t.feed;
      if (t.reconnect) opened.push(feed.connection);
    };
    const busy = (connection: number) => {
      feed = feedStatus(feed, connection, json(status('tool', { tone: 'busy', detail: 'Running Bash' })), now);
    };
    const stands = (when: string) => {
      const h = feedHeader(feed);
      expect([pip(h), h.text, h.stop], `the header ${when}`).toEqual(['t-busy', 'Running Bash · 12s', true]);
      expect(feed.connection, `taken for dead ${when}`).toBe(first);
    };
    // Every row and every status starts the ten seconds again: each probe comes 9.9 s after the
    // last thing said, and more than ten seconds after the one before it.
    feed = feedMessage(feed, first, json(row('one')), now);
    busy(first);
    later(5000);
    tick();
    feed = feedMessage(feed, first, json(row('two')), now);
    later(9900);
    tick();
    stands('9.9 s after a row, 14.9 s after a status');
    busy(first);
    later(9900);
    tick();
    stands('9.9 s after a status, 19.8 s after a row');
    const dots = () => tabsFor(rows[0]!, rows, vouched(feed), now).map((t) => (t.tone ? `t-${t.tone}` : ''));
    expect(dots(), 'the open tab while its stream speaks').toEqual(['t-busy', 't-idle']);

    later(100);
    tick();
    expect([pip(feedHeader(feed)), feedHeader(feed).text], 'after ten silent seconds').toEqual(['stale', 'reconnecting…']);
    expect(feedHeader(feed).stop, 'Stop left up on a stream nobody can vouch for').toBe(false);
    // The tab beside the header stops claiming what the stream last said, too.
    expect(dots(), 'the open tab once its stream is silent').toEqual(['t-idle', 't-idle']);
    const second = feed.connection;
    expect(second, 'the silent connection was kept').not.toBe(first);
    expect(opened).toEqual([first, second]);
    // What was read stays readable while the new connection has nothing to show...
    expect(feed.entries.length, 'the rows while reconnecting').toBe(2);
    // ...and what it replays replaces them: the router starts every connection with the
    // transcript's tail, which must land once, not twice.
    feed = feedMessage(feed, second, json(row('one')), now);
    feed = feedMessage(feed, second, json(row('two')), now);
    expect(feed.entries.length, 'the rows once the new connection has replayed the tail').toBe(2);
    feed = feedStatus(feed, second, json(status('generating', { tone: 'busy', detail: 'Thinking' })), now);
    expect([pip(feedHeader(feed)), feedHeader(feed).text, feedHeader(feed).stop], 'the header once the new connection speaks').toEqual(['t-busy', 'Thinking · 12s', true]);
    // The dead connection is history: whatever it still delivers changes nothing.
    feed = feedStatus(feed, first, json(status('idle', { tone: 'idle', detail: 'Waiting for you' })), now);
    feed = feedMessage(feed, first, json(row('late')), now);
    expect([feedHeader(feed).text, feed.entries.length], 'after the old connection spoke').toEqual(['Thinking · 12s', 2]);

    // Each connection gets ten seconds of its own. One that goes quiet is replaced in turn, and one
    // that never says anything — the router down — is retried every ten seconds, not every second,
    // with the rows still there to read.
    later(9900);
    tick();
    expect(feed.connection, 'replaced inside its own ten seconds').toBe(second);
    later(100);
    tick();
    const third_ = feed.connection;
    expect(third_, 'a connection gone quiet was kept').not.toBe(second);
    later(9900);
    tick();
    expect(feed.connection, 'a connection that has said nothing yet was not given ten seconds').toBe(third_);
    later(100);
    tick();
    expect(feed.connection, 'a connection that never spoke was kept').not.toBe(third_);
    expect([pip(feedHeader(feed)), feed.entries.length], 'while the router says nothing').toEqual(['stale', 2]);

    // A clock set back an hour must not hold the next reconnect off for that hour.
    const fourth = feed.connection;
    busy(fourth);
    later(-3600e3);
    tick();
    expect(feed.connection, 'a clock set back read as a silence').toBe(fourth);
    later(10000);
    tick();
    expect(feed.connection, 'a clock set back an hour postponed the reconnect').not.toBe(fourth);

    // Back on the list, nothing is watched any more: there is no feed to tick (SE-C57).
    later(3600e3);
    later(60e3);
    expect(silent(null, now), 'a connection watched after going back to the list').toBe(false);
  });

  // ported from scripts/check-live-ui.js: the browser reconnecting by itself does not show the tail twice
  it('SE-C69, SE-C70, SE-C71: the browser reconnecting by itself does not show the tail twice', () => {
    // The router cannot resume a stream, so it answers every connection — the browser's own retry
    // after an error included — with the transcript's tail.
    let feed = openFeed(NOW);
    const c = feed.connection;
    const tail = [
      { role: 'user', kind: 'text', text: 'first ask' },
      { role: 'assistant', kind: 'text', text: 'a reply' },
      { role: 'user', kind: 'text', text: 'second ask' },
    ];
    // Only a connection that follows another starts a new copy: what was sent before the first one
    // spoke — its echo, on a feed opening just emptied — is still there after.
    expect(sendNotice(200, {})?.ok, 'the send went through').toBe(true);
    feed = feedEcho(feed, 'sent at once');
    for (const it of tail) feed = feedMessage(feed, c, json(it), NOW);
    expect(feed.entries.length, 'the rows, with the echo of what was sent before the stream spoke').toBe(4);
    feed = feedError(feed, c);
    expect(feed.entries.length, 'the rows while the browser retries').toBe(4);
    for (const it of tail) feed = feedMessage(feed, c, json(it), NOW);
    expect(feed.entries.length, 'the rows after the browser reconnected').toBe(3);
    expect(feed.sent, 'what ArrowUp offers').toEqual(['first ask', 'second ask']);
  });

  // ported from scripts/check-live-ui.js: a project heading shows its most pressing session, ranked as on the phones
  it('SE-C18, SE-C9: a project heading shows its most pressing session, ranked as on the phones', () => {
    // One ranking on the web, iOS and Android: waiting on you, then a turn that failed, then one
    // that stalled — each a thing to go and look at — then work under way, then open and idle; the
    // empty ring only once every session there has ended.
    const H = 3600e3;
    const as: Record<string, Partial<SessionRow>> = {
      waiting: { tone: 'attention', state: 'ask', active: true, detail: 'Bash wants to run' },
      failed: { tone: 'error', state: 'failed', active: false, detail: 'API error: overloaded' },
      stalled: { tone: 'warn', state: 'stalled', active: true, detail: 'No progress' },
      generating: { tone: 'busy', state: 'working', active: true, detail: 'Thinking' },
      tool: { tone: 'busy', state: 'working', active: true, detail: 'Running Bash' },
      background: { tone: 'busy', state: 'working', active: true, detail: 'Background workflow running' },
      idle: { tone: 'idle', state: 'idle', active: false, detail: 'Waiting for you' },
      closed: { tone: 'closed', state: 'idle', active: false, open: false, detail: 'Session ended', since_ms: undefined, ended_ms: AT - 3 * H },
      // A newer router's word: it outranks nothing, and neither has it ended.
      compacting: { tone: 'sparkle', state: 'working', active: true, detail: 'Compacting' },
    };
    const LIVE = ['waiting', 'stalled', 'generating', 'tool', 'background', 'compacting'];
    const cases: [string[], Tone | null][] = [
      // Each rank over everything below it.
      [['waiting', 'failed', 'stalled', 'generating', 'idle', 'closed'], 'attention'],
      [['failed', 'stalled', 'generating', 'tool', 'background', 'idle', 'closed'], 'error'],
      [['stalled', 'generating', 'tool', 'background', 'idle', 'closed'], 'warn'],
      [['generating', 'idle', 'closed'], 'busy'], [['tool', 'idle'], 'busy'], [['background', 'closed'], 'busy'],
      [['idle', 'closed'], 'idle'], [['idle'], 'idle'],
      // The ring is for a project where nothing is left open.
      [['closed', 'closed'], 'closed'], [['closed'], 'closed'],
      // The pairs the three clients once ranked three ways, and a failure beside anything.
      [['generating', 'stalled'], 'warn'], [['stalled', 'closed'], 'warn'],
      [['failed', 'idle'], 'error'], [['failed', 'closed'], 'error'], [['failed', 'generating'], 'error'],
      [['waiting', 'generating'], 'attention'], [['waiting', 'failed'], 'attention'], [['waiting', 'closed'], 'attention'],
      // An unknown word beside an idle session is idle; with only ended ones, neither colour nor ring.
      [['compacting', 'idle'], 'idle'], [['compacting', 'closed'], null], [['compacting'], null],
    ];
    const rows = (withTone: boolean) =>
      cases.flatMap(([acts], i) =>
        acts.map((a, j) => {
          const { tone, ...rest } = as[a] ?? {};
          return act(`rk${i}-${j}`, a, { cwd: `/code/rank-${i}`, ...rest, ...(withTone ? { tone } : {}) });
        }),
      );
    // With the router's tone, and with the activity alone: the same heading either way.
    for (const withTone of [true, false]) {
      const view = listView(rows(withTone), NOW);
      cases.forEach(([acts, want], i) => {
        // The halo still says whether anything there is busy or waiting, whatever wins the colour.
        const halo = acts.some((a) => LIVE.includes(a));
        expect(dot(headingOf2(view, `rank-${i}`)), `the heading of {${acts.join(', ')}}${withTone ? '' : ' without a tone'}`).toEqual([
          ...(want ? [`t-${want}`] : []),
          ...(halo ? ['on'] : []),
        ]);
      });
    }
  });

  // ported from scripts/check-live-ui.js: each row says what it is doing and for how long, on the router's clock
  it("SE-C11, SE-C12, SE-C13, SE-C14: each row says what it is doing and for how long, on the router's clock", () => {
    const H = 3600e3;
    const rows = [
      act('idle-1', 'idle', { tone: 'idle', state: 'idle', active: false, detail: 'Waiting for you', since_ms: AT - H }),
      act('shell-1', 'idle', { tone: 'idle', state: 'idle', active: false, detail: 'Waiting for you · background shell running', since_ms: AT - 10 * H }),
      act('gone-1', 'closed', { tone: 'closed', state: 'idle', active: false, open: false, detail: 'Session ended', since_ms: AT - 3 * H, ended_ms: AT - 3 * H, ended_how: 'exit', mid_turn: false }),
      act('fail-1', 'failed', { tone: 'error', state: 'failed', active: false, detail: 'API error: 529 Overloaded', since_ms: AT - 60e3 }),
      // Three seconds in, the words stand alone: a "· 3s" would only flicker.
      act('gen-1', 'generating', { tone: 'busy', state: 'working', active: true, detail: 'Thinking', since_ms: AT - 3e3 }),
      // Nothing to measure against but this machine's clock: no duration, rather than a wrong one.
      act('nc-1', 'idle', { tone: 'idle', state: 'idle', active: false, detail: 'Waiting for you', since_ms: AT - H, as_of_ms: undefined }),
      act('nc-2', 'closed', { tone: 'closed', state: 'idle', active: false, open: false, detail: 'Session ended', since_ms: undefined, ended_ms: AT - 3 * H, as_of_ms: undefined }),
    ];
    const view = listView(rows, NOW);
    const want: Record<string, string> = {
      'idle-1': 'Waiting for you · 1h',
      'shell-1': 'Waiting for you · background shell running · 10h',
      'gone-1': 'Session ended · 3h ago',
      'fail-1': 'API error: 529 Overloaded · 1m',
      'gen-1': 'Thinking',
      'nc-1': 'Waiting for you',
      'nc-2': 'Session ended',
    };
    for (const [id, line] of Object.entries(want)) expect(rowOf(view, id).line, `${id} line`).toBe(line);
    // The open view's header reads the same way...
    let feed = said(openFeed(NOW), status('closed', { state: 'idle', tone: 'closed', detail: 'Session ended', open: false, since_ms: undefined, ended_ms: AT - 3 * H, quiet_ms: 0 }));
    expect(feedHeader(feed).text, 'the header of a closed session').toBe('Session ended · 3h ago');
    // ...and a status without the router's clock takes the router's own measure of it.
    feed = said(feed, status('idle', { state: 'idle', tone: 'idle', detail: 'Waiting for you', as_of_ms: undefined, quiet_ms: 300e3 }));
    expect(feedHeader(feed).text, 'a status with no router clock').toBe('Waiting for you · 5m');
  });

  // ported from scripts/check-live-ui.js: Stop is offered for a running turn only — never for background work alone
  it('SE-C10, SE-C157: Stop is offered for a running turn only — never for background work alone', () => {
    const cases: [Status, boolean][] = [
      [status('generating'), true], [status('tool', { state: 'tool' }), true], [status('stalled'), true],
      // Background still reads `working` to an older client. Only the activity tells it apart.
      [status('background', { detail: '2 background agents running' }), false],
      [status('waiting', { state: 'ask' }), false], [status('idle', { state: 'idle' }), false],
      [status('failed', { state: 'failed' }), false], [status('closed', { state: 'idle' }), false],
      // A word this screen does not know: no Stop, whatever the older field says.
      [status('compacting'), false],
      // An older router: its own two busy words.
      [{ state: 'working', detail: 'Working', quiet_ms: 0, queued: 0 }, true],
      [{ state: 'tool', detail: 'Running Bash', quiet_ms: 0, queued: 0 }, true],
      [{ state: 'ask', detail: 'Waiting for your answer', quiet_ms: 0, queued: 0 }, false],
      [{ state: 'idle', detail: 'Waiting for you', quiet_ms: 0, queued: 0 }, false],
    ];
    for (const [s, want] of cases) {
      const what = `${s.activity ?? `legacy ${s.state}`}: Stop ${want ? 'missing' : 'offered'}`;
      expect(stoppable(s), what).toBe(want);
      // And through the stream, which is where the header and its Stop come from.
      expect(feedHeader(said(openFeed(NOW), s)).stop, what).toBe(want);
    }
  });

  // ported from scripts/check-live-ui.js: an answer names the prompt it answers
  it('SE-C112, SE-C113, SE-C114, SE-C120, SE-C121, SE-C124, SE-C126: an answer names the prompt it answers', () => {
    const session = 'aaa-1';
    // A permission prompt, with one more behind it. The stream reporting it has it asked for at
    // once, without waiting for the two-second poll.
    const feed = said(
      openFeed(NOW),
      status('waiting', { state: 'ask', tone: 'attention', detail: 'Bash wants to run', waiting_on: { kind: 'permission', tool: 'Bash', request_id: 'req-7', tool_use_id: 't-7', summary: 'git push' } }),
    );
    expect(feed.cue, 'the stream reported a prompt; it was not asked for').toBe('req-7');
    const PROMPT: Prompt = {
      pending: true, request_id: 'req-7', tool: 'Bash', question: false, display_name: 'Bash',
      input: { command: 'git push' }, suggestions: [{ index: 0, label: 'Always allow Bash(git push:*)' }], count: 2,
    };
    const card = permissionCard(PROMPT);
    if (!card) throw new Error('the stream reported a prompt; no card');
    expect(card.what).toMatch(/Bash is waiting/);
    expect(card.more, 'a second prompt behind this one went unsaid').toBe('1 more waiting after this one');
    expect(card.choices.map((c) => permissionAnswer(session, card, c)), 'the permission answers').toEqual([
      { session: 'aaa-1', allow: true, request_id: 'req-7' },
      { session: 'aaa-1', allow: true, suggestion: 0, request_id: 'req-7' },
      { session: 'aaa-1', allow: false, message: 'Declined from the live view.', request_id: 'req-7' },
    ]);
    // Answered elsewhere meanwhile: the router refuses, and the card becomes what waits now.
    const edit = permissionCard({ ...PROMPT, request_id: 'req-8', display_name: 'Edit', tool: 'Edit', count: 1 });
    if (!edit) throw new Error('no card for the second prompt');
    const gone = permissionOutcome(409, { ok: false, reason: 'That prompt is no longer waiting' }, edit.choices[0]!);
    expect(gone.kind, 'a refused card was left up').toBe('gone');
    expect(permissionCard({ ...PROMPT, request_id: 'req-9', display_name: 'Write', tool: 'Write', count: 1 })?.what).toMatch(/Write is waiting/);

    // A question is drawn from the transcript, which has no prompt id; the router's live prompt
    // does, and the form sends it when that prompt is its question.
    const ASK = { questions: [{ question: 'Which one?', header: 'Pick', options: [{ label: 'A' }, { label: 'B' }] }] };
    const { answers, lines } = composeAnswers(ASK, [{ chosen: ['B'] }]);
    const liveQuestion: Prompt = { pending: true, request_id: 'req-q', tool: 'AskUserQuestion', question: true, count: 1, input: { questions: [{ question: 'Which one?' }] } };
    expect(answerBody(session, answers, answerTarget(liveQuestion, ASK)), 'the question answer').toEqual({ session: 'aaa-1', answers: { 'Which one?': 'B' }, request_id: 'req-q' });

    // Not its question: a permission prompt handed these answers would take them as its approval,
    // and another question would get answers to this one. Neither is sent, not even as a message.
    const permission = answerTarget({ pending: true, request_id: 'req-p', tool: 'Bash', question: false, count: 1 }, ASK);
    expect(permission.kind === 'refused' && permission.text).toMatch(/^Not sent: a permission prompt/);
    const other = answerTarget({ pending: true, request_id: 'req-r', tool: 'AskUserQuestion', question: true, count: 1, input: { questions: [{ question: 'Something else?' }] } }, ASK);
    expect(other.kind === 'refused' && other.text).toMatch(/^Not sent: a different question/);

    // Nothing live to name (an unwrapped session): asked as before, and sent as a message when the
    // router cannot answer it directly.
    expect(answerBody(session, answers, answerTarget({ pending: false }, ASK)), 'an answer with nothing to name').toEqual({ session: 'aaa-1', answers: { 'Which one?': 'B' } });
    const fallback = formOutcome(503, { ok: false, reason: "That session isn't waiting on anything right now." });
    expect(fallback).toEqual({ spent: false, hint: "That session isn't waiting on anything right now.", sendAsMessage: true });
    expect(lines, 'the typed answer that goes instead').toEqual(['Pick: B']);
  });

  // ported from scripts/check-live-ui.js: a payload from an older router renders as it always did
  it('SE-C15, SE-C17, SE-C19, SE-C37: a payload from an older router renders as it always did', () => {
    // Nothing but the keys every router has always sent.
    const rows: SessionRow[] = [
      ...base(),
      { session_id: 'eee-5', cwd: '/code/project-b', project: 'p2', title: 'Stuck', created_ms: 4000, last_activity_ms: NOW - 60e3, active: false, wrapped: true, state: 'stalled' },
      { session_id: 'fff-6', cwd: '/code/project-b', project: 'p2', title: 'Quiet', created_ms: 4500, last_activity_ms: NOW - 30e3, active: false, wrapped: true, state: 'idle' },
      { session_id: 'ggg-7', cwd: '/code/project-b', project: 'p2', title: 'From the future', created_ms: 4600, last_activity_ms: NOW - 20e3, active: false, wrapped: true, state: 'mystery' },
    ];
    const view = listView(rows, NOW);
    // The same tones as the old per-state classes: working with its ring, ask, failed, stalled,
    // idle, and an unknown word plain gray.
    const want: Record<string, string[]> = {
      'aaa-1': ['t-busy', 'on'], 'bbb-2': ['t-attention'], 'ccc-3': ['t-error'], 'eee-5': ['t-warn'], 'fff-6': ['t-idle'], 'ggg-7': [],
    };
    for (const [id, d] of Object.entries(want)) {
      expect(dot(rowOf(view, id)), `${id} dot`).toEqual(d);
      expect(rowOf(view, id).line, `${id} grew a status line with nothing to say`).toBeNull();
    }
    // `active` is all such a router can say, so the header and the headings say what they always
    // did: how many moved lately, and whether each project did.
    expect(listHeader(rows, NOW).text, 'the header').toBe('1 active');
    expect(dot(headingOf2(view, 'project-a')), 'a project where something moved').toEqual(['t-busy', 'on']);
    expect(dot(headingOf2(view, 'project-b')), 'a project where nothing did').toEqual(['t-idle']);
    // Rows quiet for over twelve hours still fold away. aaa-1 stays because it is `active`.
    const folded = (view.older?.groups ?? []).flatMap((g) => g.rows.map((r) => r.id));
    expect(folded.sort(), 'what folds into older').toEqual(['bbb-2', 'ccc-3']);

    const open = rows[0]!;
    expect(tabsFor(open, rows, null, NOW).map((t) => (t.tone ? `t-${t.tone}` : '')), 'the tabs').toEqual(['t-busy', 't-attention']);
    let feed = said(openFeed(NOW), { state: 'tool', detail: 'Running Bash', quiet_ms: 12e3, queued: 1 });
    expect([pip(feedHeader(feed)), feedHeader(feed).text], 'a running tool').toEqual(['t-busy', 'Running Bash · 12s · 1 queued']);
    expect(feedHeader(feed).stop, 'Stop missing for a legacy tool').toBe(true);
    feed = said(feed, { state: 'ask', detail: 'Waiting for your answer', quiet_ms: 2e3, queued: 0 });
    expect([pip(feedHeader(feed)), feedHeader(feed).text], 'a question').toEqual(['t-attention', 'Waiting for your answer']);
    expect(feedHeader(feed).stop, 'Stop offered while asking').toBe(false);
    feed = said(feed, { state: 'idle', detail: 'Waiting for you', quiet_ms: 300e3, queued: 0 });
    expect(feedHeader(feed).text, 'an idle session').toBe('Waiting for you · 5m');

    // A prompt without a count is still answered by its id, and says nothing of others.
    const card = permissionCard({ pending: true, request_id: 'req-old', tool: 'Bash', question: false, display_name: 'Bash', input: { command: 'ls' }, suggestions: [] });
    if (!card) throw new Error('no card for an older router');
    expect(card.more, 'invented a count').toBeNull();
    expect(permissionAnswer('aaa-1', card, card.choices[0]!).request_id, 'the answer').toBe('req-old');
  });

  // ported from scripts/check-live-ui.js: the wire, replayed: every row renders and every status reads cleanly
  it('SE-C213, SE-C214: the wire, replayed: every row renders and every status reads cleanly', () => {
    // The wire as the router writes it — a feed row is an unnamed event, a status a named one, the
    // keep-alive a comment — moved from the old script to fixtures/wire.sse, byte for byte.
    const { items, headers } = replay(wire, 'the built-in stream');
    expect(items, 'rows rendered').toBe(3);
    expect(headers, 'the header after each status').toEqual([
      ['t-idle', 'Waiting for you · background shell running · 10h'],
      ['t-busy', 'Thinking'],
      ['t-busy', 'Running Bash · 2m'],
      ['t-attention', 'Bash wants to run · 1m · 1 queued'],
      ['t-busy', '2 background agents running · 5m'],
      ['t-warn', 'No progress · 12m'],
      ['t-error', 'API error: 529 Overloaded · 1m'],
      ['t-closed', 'Session ended mid-turn · 3h ago'],
      ['t-idle', 'Waiting for you · 15m'],
    ]);
  });

  // A capture of a real stream replays the same way when one is named, as the old script took one
  // on its command line: AGENT_ROUTER_CAPTURE=<file> pnpm exec vitest run src/model/live.spec.ts.
  // A path that names no file is a mistake to hear about, not a replay to skip in silence.
  const capture = process.env['AGENT_ROUTER_CAPTURE'];
  // ported from scripts/check-live-ui.js: every captured event renders, statuses included
  it.skipIf(!capture)('SE-C215: every captured event renders, statuses included', () => {
    if (!capture || !existsSync(capture)) throw new Error(`no such capture: ${capture}`);
    const { items, headers } = replay(readFileSync(capture, 'utf8'), capture);
    expect(items + headers.length).toBeGreaterThan(0);
  });

  // ported from scripts/check-live-ui.js: find opens from the list and from a session, and closes back to where it was
  it('SE-C162, SE-C163, SE-C174: find opens from the list and from a session, and closes back to where it was', () => {
    // The feed can only ever show the tail of a transcript, so Find is the only way to the rest.
    let view: LiveView = openFind({ session: null, find: null });
    expect(view.find?.scope, 'from the list, Find searches everywhere').toBe('all');
    expect(findScopes(null).find((s) => s.value === 'session')?.disabled, 'nothing to narrow to').toBe(true);
    expect(findPlaceholder('all', null)).toBe('Find in every session');
    expect(headingOf(view, null)).toBe('Find');
    view = closeFind(view);
    expect(view, 'Find stayed open').toEqual({ session: null, find: null });
    expect(headingOf(view, null)).toBe('Live sessions');

    const rows = base();
    const open = rows[0]!;
    view = openFind({ session: open.session_id, find: null });
    expect(view.find?.scope, 'from a session, Find searches it').toBe('session');
    expect(findPlaceholder('session', open.session_id)).toBe('Find anywhere in this session');
    view = closeFind(view);
    expect(stripShown(view, tabsFor(open, rows, null, NOW)), 'closing Find left the tabs hidden').toBe(true);
    expect(headingOf(view, open)).toBe('project-a');
  });

  // ported from scripts/check-live-ui.js: a hit renders, and one already in the open session is not a link to it
  it('SE-C169, SE-C172: a hit renders, and one already in the open session is not a link to it', () => {
    const reply = searchReply(NOW);
    const fromList = findView({ reply }, 'kestrel', 'all', null, NOW);
    expect(fromList.hits[0]?.snippet, 'the match was not marked').toContainEqual({ text: 'kestrel', mark: true });
    expect(fromList.hits[0]?.kind, "the kind was not put into the reader's words").toBe('tool call');
    expect(fromList.hits.filter((h) => h.opens).length, 'from the list, every hit opens its session').toBe(2);
    expect(nonsense(fromList)).toEqual([]);
    const open = base()[0]!;
    const hit = reply.hits?.[0];
    if (!hit) throw new Error('the fixture lost its hits');
    const here = findView({ reply: { hits: [{ ...hit, session_id: open.session_id }] } }, 'kestrel', 'session', open.session_id, NOW);
    expect(here.hits[0]?.opens, 'a hit in the session already open was offered as a link to it').toBeNull();
  });

  // ported from scripts/check-live-ui.js: a stray marker cannot leave a tag open, and snippet text is escaped
  it('SE-C170: a stray marker cannot leave a tag open, and snippet text is escaped', () => {
    const out = marks('a ‹b› c ‹d');
    expect(out.filter((r) => r.mark).map((r) => r.text)).toEqual(['b', 'd']);
    expect(out.map((r) => r.text).join(''), 'no text was dropped').toBe('a b c d');
    expect(marks('<script>x</script>'), 'snippet text reached the page as markup').toEqual([{ text: '<script>x</script>', mark: false }]);
  });

  // ported from scripts/check-live-ui.js: an empty result says so rather than showing nothing
  it('SE-C166: an empty result says so rather than showing nothing', () => {
    expect(findView({ reply: { hits: [] } }, 'nothing', 'session', 'aaa-1', NOW).note, 'an empty answer was silent').toBe('Nothing matched nothing in this session.');
    expect(findView({ reply: { hits: [] } }, 'nothing', 'all', null, NOW).note).toBe('Nothing matched nothing.');
  });

  // ported from scripts/check-live-ui.js: a per-session action a hosted router refuses says what it said
  it('SE-C183, SE-C116, SE-C144, SE-C158: a per-session action a hosted router refuses says what it said', () => {
    // Every per-session route of a hosted router answers 503 and why. Whichever action the reader
    // tried, that is what they are told, not a generic failure.
    const body = { ok: false, error: HOSTED_ERROR };
    const options = switcherOptions('model', 'claude-opus-4-1', OPTIONS);
    expect(options[0], 'the switcher leads with the current model').toEqual({ value: 'opus-4-1', label: 'model: opus-4-1', current: true });
    const said503: [string, string | undefined][] = [
      ['send', sendNotice(503, body)?.text],
      ['stop', stopNotice(503, body)?.text],
      ['side question', asideNotice(503, body)?.text],
      ['model switch', switchNotice('model', 'sonnet', 503, body)?.text],
      ['permission answer', permissionOutcome(503, body, { label: 'Allow', body: { allow: true }, done: 'Allowed.' }).notice?.text],
      ['archive', notice(refusal('archive', 503, body))?.text],
    ];
    for (const [what, text] of said503) expect(text, what).toBe(HOSTED_ERROR);
    // A question's answer says it beside the form, then sends the text as a message, which says
    // it too.
    expect(formOutcome(503, body), "a question's answer").toEqual({ spent: false, hint: HOSTED_ERROR, sendAsMessage: true });
    expect(sendNotice(503, body, { ask: true })?.text, 'that answer, sent as a message').toBe(HOSTED_ERROR);
    // Only a 503's `error` is a refusal to repeat. A local router's own failures carry it only on a
    // 500, and those still read as they always did.
    expect(stopNotice(500, { error: 'database is locked' })?.text, "a local router's internal error").toBe('Could not stop this session.');
  });

  // ported from scripts/check-live-ui.js: a hosted router says why its list is empty, and warns of no wrapper
  it('SE-C38, SE-C39, SE-C186, SE-C187, SE-C188, SE-C173: a hosted router says why its list is empty, and warns of no wrapper', () => {
    // An editor that is not wrapped: a local router warns of it above the list.
    const SHIM = { targets: [{ extension: 'anthropic.claude-code-9.9.9-darwin-arm64', path: '/x/claude', state: 'not_installed' }] };
    expect(emptyList(false), "a local router's empty list").toBe('No sessions in the last week.');
    expect(wrapperWarning(SHIM, false), 'a local router no longer warns of an unwrapped editor').toMatch(/is not wrapped/);
    // A hosted router, as /api/auth names it; one that predates the field is local.
    const hosted = isHosted({ configured: true, from_loopback: false, remote_bind: true, lan_url: 'https://router.example', env_override: false, mode: 'server', password_pinned: true, public_url: 'https://router.example' });
    expect(hosted).toBe(true);
    expect(isHosted({})).toBe(false);
    expect(emptyList(hosted), "a hosted router's empty list").toBe(AGENTLESS);
    expect(emptyList(hosted), 'a hosted router blamed the week').not.toMatch(/in the last week/);
    expect(asksWrapper(hosted), 'a hosted router asked for a wrapper it has no use for').toBe(false);
    expect(wrapperWarning(SHIM, hosted), 'a hosted router warned of a wrapper it has no use for').toBeNull();
    // A search hit cannot be opened there either, and says why rather than "no longer listed".
    expect(hitTarget('zzz-9', { rows: [] }, hosted), 'a search hit on a hosted router').toEqual({ say: OFF_MACHINE });
  });
});
