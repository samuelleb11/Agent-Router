// Smoke-test assets/live.html outside a browser.
//
// The page is plain script in a served file, so nothing type-checks it and `cargo test`
// cannot reach it. Every UI bug that reached the user was of a kind this catches: a
// local shadowing a browser global, a replacement landing inside the function it was
// meant to call, a handler that throws on real data. It runs the script, drives the
// paths a reader actually takes, and pushes captured events through the renderer.
//
// It also pins how the page reads a session's state. The router derives that once and
// sends it as `activity`, `tone` and `detail` beside the older `state` — so the checks
// feed it both shapes: the new keys, and a payload from a router that predates them,
// which has to render exactly as it always did.
//
//   node scripts/check-live-ui.js [captured-sse-file]
const fs = require('fs');
const path = require('path');

const file = path.join(__dirname, '..', 'assets', 'live.html');
const src = fs.readFileSync(file, 'utf8');
const js = src.slice(src.indexOf('<script>') + 8, src.lastIndexOf('</script>'));
const css = src.slice(src.indexOf('<style>'), src.indexOf('</style>'));

let failures = 0;
const report = (name, e) => {
  if (!e) return console.log(`  ok    ${name}`);
  failures++;
  console.log(`  FAIL  ${name}\n        ${String((e && e.stack) || e).split('\n').slice(0, 3).join('\n        ')}`);
};
const check = (name, fn) => {
  try { fn(); report(name); }
  catch (e) { report(name, e); }
};
/// For a check that has to await the page's own async path.
///
/// `check` cannot take one: an async function handed to it returns a promise, which
/// cannot throw into its `try` — and the summary below used to call process.exit before
/// any such promise settled, so everything after a check's first `await` never ran at
/// all and the check reported ok whatever it asserted. eedc5a8 fixed the same thing in
/// check-dashboard-ui.js; this is that fix.
/// One chain, not one promise each: these drive the same page and the same DOM, so
/// starting them together lets a later check move state out from under an earlier one.
let queue = Promise.resolve();
const acheck = (name, fn) => {
  queue = queue.then(fn).then(() => report(name), (e) => report(name, e));
};
// Something the page starts and never awaits (a poll, a redraw) is still the page: its
// failure is a failure here, not a crash that hides the summary.
process.on('unhandledRejection', (e) => report('nothing the page starts is left to reject', e));

// --- a DOM just real enough to run the page ---
const mkEl = (tag = 'div') => {
  const el = {
    tagName: tag, children: [], style: {}, dataset: {}, _cls: new Set(), _q: {}, _qa: {},
    get innerHTML() { return el._html || ''; },
    set innerHTML(v) {
      el._html = v;
      // The page builds a tab as `…<span class="lbl">Title</span>`; capture it so the
      // check can assert on what a reader would actually see.
      const m = /class="lbl">([^<]*)</.exec(v);
      if (m) el._label = m[1];
      if (v === '') el.children.length = 0;
    },
    // One set of classes whichever way the page reaches them, as in a browser: it sets
    // `className` on some elements and toggles `classList` on others.
    get className() { return [...el._cls].join(' '); },
    set className(v) { el._cls = new Set(String(v).split(/\s+/).filter(Boolean)); },
    textContent: '', value: '', title: '',
    scrollTop: 0, scrollHeight: 100, clientHeight: 50,
    selectionStart: 0, selectionEnd: 0, disabled: false,
    classList: {
      add: (...c) => c.forEach((x) => el._cls.add(x)),
      remove: (...c) => c.forEach((x) => el._cls.delete(x)),
      toggle: (c, f) => { const on = f === undefined ? !el._cls.has(c) : f; on ? el._cls.add(c) : el._cls.delete(c); return on; },
      contains: (c) => el._cls.has(c),
    },
    appendChild: (c) => (el.children.push(c), c),
    set _ignore(v) {},
    append: (...c) => el.children.push(...c),
    prepend: (...c) => el.children.unshift(...c),
    replaceChildren: (...c) => { el.children.length = 0; el.children.push(...c); },
    // The same selector finds the same node, as it would in a real tree, so a check can
    // reach what the page reached: a <details>' inner box, a question form's Submit.
    // `_qa` is where a check puts what a selector should find (a ticked option).
    querySelector: (sel) => {
      // The open tab's dot, which the page recolours in place when its stream speaks.
      // A tab here is markup, not nodes, so the new class is written back into that
      // markup — where tabDots() reads it, as a reader would see it.
      if (sel === '.stab.on .dot') {
        const tab = el.children.find((c) => c._cls && c._cls.has('stab') && c._cls.has('on'));
        return tab ? { set className(v) { tab._html = tab._html.replace(/class="dot[^"]*"/, `class="${v}"`); } } : null;
      }
      return (el._q[sel] ||= mkEl());
    },
    querySelectorAll: (sel) => el._qa[sel] || [],
    setAttribute(k, v) { el[k] = v; }, getAttribute: (k) => el[k],
    addEventListener() {}, closest: () => mkEl(), setSelectionRange() {},
    // drawTabs scrolls the open tab into view; every real element can.
    after() {}, remove() {}, focus() {}, select() {}, scrollIntoView() {},
  };
  return el;
};
global.document = {
  querySelector: () => mkEl(), querySelectorAll: () => [],
  createElement: (t) => mkEl(t), getElementById: () => mkEl(),
  body: mkEl(), addEventListener() {},
};
global.window = global;
global.location = { href: 'http://host/live?session=abc-1' };
global.history = { pushState() {}, replaceState() {} };
global.visualViewport = null;
global.ResizeObserver = class { observe() {} unobserve() {} disconnect() {} };
// Every stream the page opens, so a check can speak as the router: a feed row, a
// `status` event, or the error a dropped connection raises.
const streams = [];
global.EventSource = class {
  constructor(url) { this.url = url; this.on = {}; this.closed = false; streams.push(this); }
  addEventListener(type, f) { (this.on[type] ||= []).push(f); }
  close() { this.closed = true; }
  emit(type, data) {
    const e = { data: typeof data === 'string' ? data : JSON.stringify(data) };
    if (type === 'message' && this.onmessage) this.onmessage(e);
    for (const f of this.on[type] || []) f(e);
  }
};
const stream = () => streams[streams.length - 1];
// Two sessions in one project and one in another, so the tab strip has something to
// draw and its ordering is exercised. A function, so each check starts from the same rows.
const BASE = () => [
  { session_id: 'aaa-1', cwd: '/code/project-a', project: 'p1', title: 'Older one',
    created_ms: 1000, last_activity_ms: 9000, active: true, wrapped: true, state: 'working' },
  { session_id: 'bbb-2', cwd: '/code/project-a', project: 'p1', title: 'Newer one',
    created_ms: 5000, last_activity_ms: 2000, active: false, wrapped: false, state: 'ask' },
  { session_id: 'ccc-3', cwd: '/code/project-b', project: 'p2', title: 'Elsewhere',
    created_ms: 3000, last_activity_ms: 3000, active: false, wrapped: false, state: 'failed' },
];
let SESSIONS = BASE();
// A search reply, as the router spells it. The marker pair is what the page splits on.
const SEARCH = {
  query: 'kestrel', expression: 'kestrel',
  sessions: [{ session_id: 'aaaaaaaa-1111-2222-3333-444444444444', project: '/code/agent-router',
               blocks: 2, hits: 3, first_at: Date.now() - 6e5, last_at: Date.now() - 1e4 }],
  hits: [
    { block_id: 1, kind: 'tool_use', role: 'assistant', ordinal: 4, at: Date.now() - 1e4,
      session_id: 'aaaaaaaa-1111-2222-3333-444444444444', project: '/code/agent-router',
      snippet: 'where does the ‹kestrel› roost', rank: -8 },
    // A stray opening marker with no close: it may shift the emphasis, never break out.
    { block_id: 2, kind: 'text', role: 'user', ordinal: 0, at: Date.now() - 6e5,
      session_id: 'bbbbbbbb-1111-2222-3333-444444444444', project: '/code/other',
      snippet: 'a stray ‹marker and no close', rank: -7 },
  ],
};
// What the prompt and answer endpoints say, set by the checks that need them, and every
// POST the page makes, so a check can read exactly what it sent.
let PROMPT = { pending: false };
let ANSWER = { status: 200, body: { ok: true } };
let LIST_DOWN = false;
const posts = [];
global.fetch = (url = '', init = {}) => {
  const u = String(url);
  if (init.method === 'POST') posts.push({ url: u, body: JSON.parse(init.body || '{}') });
  if (LIST_DOWN && u.includes('/api/live/sessions')) return Promise.reject(new Error('connection refused'));
  const [status, body] =
    u.includes('/api/search') ? [200, SEARCH]
      : u.includes('/api/live/sessions') ? [200, SESSIONS]
      : u.includes('/api/live/prompt') ? [200, PROMPT]
      : u.includes('/api/live/answer') ? [ANSWER.status, ANSWER.body]
      : [200, {}];
  return Promise.resolve({ ok: status < 300, status, json: () => Promise.resolve(body) });
};
global.requestAnimationFrame = (f) => setTimeout(f, 0);
global.cancelAnimationFrame = () => {};
global.getComputedStyle = () => ({ lineHeight: '18' });
// Kept rather than run, so a check can fire the page's own timers — the five-second
// list refresh, the two-second prompt poll — at the moment it wants.
const intervals = [];
global.setInterval = (f, ms) => (intervals.push({ f, ms }), intervals.length);
global.addEventListener = () => {};
// Every promise the page started has settled once a macrotask comes round.
const flush = () => new Promise((r) => setTimeout(r, 0));
const tick = async (ms) => { for (const t of intervals.filter((t) => t.ms === ms)) t.f(); await flush(); await flush(); };
// The page times a stream's silence by Date.now, so a check moves this clock rather than
// wait out ten real seconds — forward, or back as a clock being corrected does. It moves
// only by what the checks ask; everything that reads it, page and fixtures, reads it alike.
const realNow = Date.now.bind(Date);
let skew = 0;
Date.now = () => realNow() + skew;
const later = (ms) => { skew += ms; };

check('script evaluates', () => { new Function(js)(); });

intervals.length = 0;  // the copy above is done with; only the one below may be ticked
const api = new Function(
  js + '\n; return { render, md, openSession, showList, say, askForm, clampable, loadSessions,' +
       ' pollPrompt, deliver, list, stat, statText, stopBtn, perm, feed, sentLog: () => [...sent],' +
       ' tabLabels: () => [...tabs.children].map(t => (t._label || \'\')),' +
       ' tabDots: () => [...tabs.children].map(t => (/class="dot ([^"]*)"/.exec(t.innerHTML) || [0, \'\'])[1].trim()),' +
       ' tabsHidden: () => tabs.classList.contains(\'hide\'),' +
       ' openFind, closeFind, drawFind, findMark, findRows, findPane };'
)();

// --- reading what the page drew ---
// Every card on the list, folded sections included; a card is found by its session id,
// which is what its last line prints.
const cardsIn = (el) => el.children.flatMap((c) =>
  (c._html || '').includes('class="row1"') ? [c]
    : c.tagName === 'details' ? cardsIn(c.querySelector('div')) : []);
const cardOf = (id, within = api.list) => {
  const c = cardsIn(within).find((x) => x._html.includes(`class="meta">${id}`));
  if (!c) throw new Error(`no card for ${id}`);
  return c;
};
const classesOf = (html) => ((/class="dot([^"]*)"/.exec(html) || [0, ''])[1]).trim().split(/\s+/).filter(Boolean);
const lineOf = (html) => { const m = /class="doing[^"]*">([^<]*)</.exec(html); return m ? m[1] : null; };
const older = () => api.list.children.find((c) => c.id === 'older');
const headingOf = (project) => api.list.children.find((c) => (c._html || '').includes(`<span>${project}</span>`));
const eq = (got, want, what) => {
  if (JSON.stringify(got) !== JSON.stringify(want)) {
    throw new Error(`${what}: got ${JSON.stringify(got)}, expected ${JSON.stringify(want)}`);
  }
};
const stopShown = () => !api.stopBtn.classList.contains('hide');

// The router's words for a session: every activity it can send, and one it has not sent
// yet. `as_of_ms` is the router's clock, set seven minutes behind this machine's so that
// a duration measured against the wrong clock shows. Everything happened 12 s before it.
const AT = Date.now() - 7 * 60e3;
const act = (id, activity, extra = {}) => ({
  session_id: id, cwd: '/code/project-c', project: 'p3', title: `Session ${id}`,
  created_ms: AT - 3600e3, last_activity_ms: Date.now() - 12e3, wrapped: true, archived: false,
  bytes: 1, as_of_ms: AT, since_ms: AT - 12e3, source: 'registry', open: true, activity, ...extra });
const ACTIVITIES = () => [
  act('gen-1', 'generating', { tone: 'busy', state: 'working', active: true, detail: 'Thinking' }),
  act('tool-1', 'tool', { tone: 'busy', state: 'working', active: true, detail: 'Running Bash' }),
  act('bg-1', 'background', { tone: 'busy', state: 'working', active: true,
    detail: '2 background agents running', background: { agents: 2, workflows: 0, shells: 0 } }),
  act('wait-1', 'waiting', { tone: 'attention', state: 'ask', active: true, detail: 'Bash wants to run',
    waiting_on: { kind: 'permission', tool: 'Bash', request_id: 'r-1', tool_use_id: 't-1', summary: 'git status' } }),
  act('stall-1', 'stalled', { tone: 'warn', state: 'stalled', active: true, detail: 'No progress',
    since_ms: AT - 700e3 }),
  act('fail-1', 'failed', { tone: 'error', state: 'failed', active: false, detail: 'API error: overloaded' }),
  act('idle-1', 'idle', { tone: 'idle', state: 'idle', active: false, detail: 'Waiting for you' }),
  act('gone-1', 'closed', { tone: 'closed', state: 'idle', active: false, open: false, since_ms: undefined,
    detail: 'Session ended', ended_ms: AT - 3 * 3600e3, ended_how: 'exit', mid_turn: false }),
  // A newer router's word, in a tone this page has never heard of.
  act('new-1', 'compacting', { tone: 'sparkle', state: 'working', active: true, detail: 'Compacting' }),
];
// The same, as the open view's `status` event carries it.
const status = (activity, extra = {}) => ({
  state: 'working', detail: 'Working', quiet_ms: 12e3, queued: 0,
  activity, as_of_ms: AT, since_ms: AT - 12e3, open: true, ...extra });

check('a local does not shadow a browser global the page uses', () => {
  const declared = new Set();
  for (const m of js.matchAll(/(?:let|const|var)\s+([A-Za-z_$][\w$]*)/g)) declared.add(m[1]);
  const globals = ['history', 'location', 'name', 'status', 'top', 'self', 'parent', 'origin', 'screen', 'event'];
  const clash = globals.filter((g) => declared.has(g));
  if (clash.length) throw new Error(`shadows ${clash.join(', ')} — see the \`history\` bug that stopped cards opening`);
});

acheck('opening a session does not throw', async () => {
  api.openSession({ session_id: 'abc-1', cwd: '/code/project-a', project: 'p', git_branch: 'dev',
                    title: 't', last_activity_ms: Date.now(), active: true, wrapped: true }, true);
  await flush();
});

acheck('going back to the list does not throw', () => api.showList(true));

check('notices do not recurse', () => {
  // say('') once called itself instead of clearing, which blew the stack inside
  // openSession and left every session blank.
  api.say('something happened');
  api.say('', {});
  api.say('and again', { ok: true, for: 10 });
});

check('markdown survives hostile input', () => {
  const out = api.md('<img src=x onerror=alert(1)>\n**b** `c`\n- one\n\n| a | b |\n|---|---|\n| 1 | 2 |');
  if (/<img/.test(out)) throw new Error('escaping failed');
  if (!/<table>/.test(out) || !/<b>b<\/b>/.test(out)) throw new Error('markdown did not render');
});

acheck('the session list renders every state', async () => {
  // Every word either router can say, in the list and in the open view, and one neither
  // has said yet. Nothing may throw, and nothing may reach the page as "undefined".
  SESSIONS = [...BASE(), ...ACTIVITIES(),
    ...['stalled', 'idle', 'mystery'].map((st, i) => ({ ...BASE()[0], session_id: `leg-${i}`, state: st }))];
  await api.showList(true);
  for (const c of cardsIn(api.list)) {
    if (/undefined|NaN/.test(c._html)) throw new Error(c._html.match(/.{0,50}(undefined|NaN).{0,30}/)[0]);
  }
  for (const st of ['working', 'ask', 'idle', 'stalled', 'failed', 'tool', 'mystery']) {
    api.openSession({ ...BASE()[0], state: st }, true);
    stream().emit('status', { state: st, detail: 'Working', quiet_ms: 0, queued: 0 });
  }
  for (const r of ACTIVITIES()) {
    api.openSession(r, true);
    stream().emit('status', { ...r, quiet_ms: 12e3, queued: 0 });
    if (/undefined|NaN/.test(api.statText.textContent)) throw new Error(api.statText.textContent);
  }
});

acheck('tabs cover the project and keep creation order', async () => {
  SESSIONS = BASE();
  await api.showList(true);
  api.openSession(SESSIONS[0], true);
  const labels = api.tabLabels();
  if (labels.length !== 2) throw new Error(`expected 2 tabs for the project, got ${labels.length}`);
  if (labels[0] !== 'Older one') throw new Error(`oldest must lead, got ${labels.join(', ')}`);
});

// A third session current in the same project, so the strip is drawn at all: a project
// with one session on screen gets no strip (fd95731), which the counts these two checks
// first asserted — one tab each — never allowed for. Neither could fail back then, so
// neither was ever seen to be wrong.
const THIRD = () => ({ session_id: 'ddd-4', cwd: '/code/project-a', project: 'p1', title: 'Third one',
  created_ms: 7000, last_activity_ms: Date.now(), active: false, wrapped: false, state: 'idle' });

acheck('archived sessions leave the tabs, but come back when live', async () => {
  const [open, other, elsewhere] = BASE();
  const shelved = { ...other, archived: true, active: false, state: 'idle', last_activity_ms: Date.now() };
  SESSIONS = [open, shelved, elsewhere, THIRD()];
  await api.showList(true);
  api.openSession(open, true);
  eq(api.tabLabels(), ['Older one', 'Third one'], 'an archived session should not hold a tab');

  // It wakes. `archived` arrives already decided — "put away, and quiet ever since" — and
  // the router clears it itself the moment the session is busy or needs you (e784bcd;
  // the router's `archived && !active`), so waking is the router saying so. A row still
  // marked archived is one the router says is put away, and the page takes its word.
  SESSIONS[1] = { ...shelved, archived: false, active: true, state: 'ask' };
  await api.loadSessions();
  api.openSession(open, true);
  eq(api.tabLabels(), ['Older one', 'Newer one', 'Third one'], 'an archived session that wakes must reappear');
});

acheck('stale sessions do not clutter the tabs', async () => {
  const [open, other] = BASE();
  const stale = { ...other, archived: false, active: false, state: 'idle',
                  last_activity_ms: Date.now() - 40 * 3600 * 1000 };
  SESSIONS = [open, stale, THIRD()];
  await api.showList(true);
  api.openSession(open, true);
  eq(api.tabLabels(), ['Older one', 'Third one'], 'a session idle for days should not hold a tab');
  // With nothing current beside it, the open session has no strip at all.
  SESSIONS = [open, stale];
  await api.loadSessions();
  api.openSession(open, true);
  if (api.tabLabels().length || !api.tabsHidden()) throw new Error('a lone session should not draw a one-tab strip');
});

acheck('every activity and tone renders, and a word the page does not know is plain gray', async () => {
  SESSIONS = ACTIVITIES();
  await api.showList(true);
  const want = {
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
  for (const [id, [dot, line]] of Object.entries(want)) {
    const html = cardOf(id)._html;
    eq(classesOf(html), dot, `${id} dot`);
    eq(lineOf(html), line, `${id} line (durations are measured on the router's clock)`);
  }
  // Only what needs a reader is coloured in the words themselves.
  if (!/class="doing t-attention"/.test(cardOf('wait-1')._html)) throw new Error('a waiting line should stand out');
  // Without a tone the activity alone gives the same colours: `tone` is the router's
  // choice when it makes one, not something the page cannot do without.
  SESSIONS = ACTIVITIES().map(({ tone, ...r }) => r);
  await api.showList(true);
  for (const [id, [dot]] of Object.entries(want)) eq(classesOf(cardOf(id)._html), dot, `${id} dot, no tone`);
  // The router's tone wins over the page's own reading of the activity...
  SESSIONS = [act('odd-1', 'tool', { tone: 'warn', active: true, detail: 'Running Bash' })];
  await api.showList(true);
  eq(classesOf(cardOf('odd-1')._html), ['t-warn', 'on'], 'the router picks the colour');
  // ...and a tone it does not know falls back to that reading, not to gray.
  SESSIONS = [act('odd-2', 'waiting', { tone: 'sparkle', active: true, detail: 'Waiting for your answer' })];
  await api.showList(true);
  eq(classesOf(cardOf('odd-2')._html), ['t-attention', 'on'], 'an unknown tone with a known activity');
  // The project heading takes the most pressing of its sessions.
  SESSIONS = ACTIVITIES();
  await api.showList(true);
  eq(classesOf(headingOf('project-c')._html), ['t-attention', 'on'], 'the heading of a project with one waiting');
  // And the counts come from the activity.
  eq(api.statText.textContent, '4 working · 1 waiting', 'the list header');
  eq(api.stat.className, 't-attention', 'the list header pip');
});

acheck('a closed session is a hollow ring, says when it ended, and is never counted', async () => {
  const gone = (id, extra = {}) => act(id, 'closed', { tone: 'closed', state: 'idle', active: false, open: false,
    since_ms: undefined, detail: 'Session ended', ended_ms: AT - 3 * 3600e3, ended_how: 'exit', ...extra });
  SESSIONS = [gone('gone-1'), gone('gone-2', { detail: 'Session ended mid-turn', ended_how: 'unknown', mid_turn: true }),
              act('idle-1', 'idle', { tone: 'idle', state: 'idle', active: false, detail: 'Waiting for you' }),
              // Stalled is still at work — the process says busy — though its older word is not `working`.
              act('stall-1', 'stalled', { tone: 'warn', state: 'stalled', active: true, detail: 'No progress' })];
  await api.showList(true);
  eq(classesOf(cardOf('gone-1')._html), ['t-closed'], 'a closed dot');
  eq(lineOf(cardOf('gone-2')._html), 'Session ended mid-turn · 3h ago', 'a session that died mid-turn');
  eq(api.statText.textContent, '1 working', 'the count');
  // Hollow: the ring is the colour, the middle is empty.
  const rule = (/\.dot\.t-closed\s*\{([^}]*)\}/.exec(css) || [0, ''])[1];
  if (!/background:\s*transparent/.test(rule) || !/border:/.test(rule)) throw new Error(`not a ring: ${rule}`);
  // A project where every session has ended says so at its heading; with nothing busy or
  // waiting anywhere, the header counts nothing and says when anything last moved.
  SESSIONS = [gone('gone-1'), gone('gone-2')];
  await api.showList(true);
  eq(classesOf(headingOf('project-c')._html), ['t-closed'], 'a project of ended sessions');
  if (!/^idle · /.test(api.statText.textContent)) throw new Error(`counted a closed session: ${api.statText.textContent}`);
  // Opened, it says the same, and offers no Stop.
  api.openSession(SESSIONS[0], true);
  stream().emit('status', status('closed', { state: 'idle', tone: 'closed', detail: 'Session ended',
    ended_ms: AT - 3 * 3600e3, since_ms: undefined, quiet_ms: 0, open: false }));
  eq(api.statText.textContent, 'Session ended · 3h ago', 'the open view of a closed session');
  eq(api.stat.className, 't-closed', 'its pip');
  if (stopShown()) throw new Error('Stop offered on a session that has ended');
});

acheck('a session busy or waiting on you is never folded into "older"', async () => {
  const old = Date.now() - 30 * 3600 * 1000;
  SESSIONS = [
    act('wait-1', 'waiting', { tone: 'attention', state: 'ask', active: true, detail: 'Waiting for your answer', last_activity_ms: old }),
    act('gen-1', 'generating', { tone: 'busy', state: 'working', active: true, detail: 'Thinking', last_activity_ms: old }),
    act('stall-1', 'stalled', { tone: 'warn', state: 'stalled', active: true, detail: 'No progress', last_activity_ms: old }),
    act('idle-1', 'idle', { tone: 'idle', state: 'idle', active: false, detail: 'Waiting for you', last_activity_ms: old }),
    act('gone-1', 'closed', { tone: 'closed', state: 'idle', active: false, detail: 'Session ended', last_activity_ms: old }),
  ];
  await api.showList(true);
  const folded = cardsIn(older().querySelector('div')).map((c) => /class="meta">([^<]*)</.exec(c._html)[1]);
  eq(folded.sort(), ['gone-1', 'idle-1'], 'what folds into older');
  eq(older().innerHTML.match(/<summary>([^<]*)</)[1], '2 older sessions', 'the fold says how many');
  for (const id of ['wait-1', 'gen-1', 'stall-1']) {
    if (!api.list.children.includes(cardOf(id))) throw new Error(`${id} folded away`);
  }
});

acheck('the tab dots follow the list while a session is open', async () => {
  const mk = (id, title, created, extra) => act(id, 'idle', { cwd: '/code/project-t', title, created_ms: created,
    tone: 'idle', state: 'idle', active: false, detail: 'Waiting for you', ...extra });
  SESSIONS = [mk('tab-1', 'First', 1, {}), mk('tab-2', 'Second', 2, {})];
  await api.showList(true);
  api.openSession(SESSIONS[0], true);
  // Still connecting, the open tab says what the list said; its stream's first word
  // recolours it at once, not at the next list refresh.
  eq(api.tabDots(), ['t-idle', 't-idle'], 'the dots as opened');
  stream().emit('status', status('tool', { tone: 'busy', detail: 'Running Bash' }));
  eq(api.tabDots(), ['t-busy', 't-idle'], 'the open tab once its stream speaks');

  // A sibling starts asking. Before, the strip kept the states it was opened with.
  SESSIONS[1] = { ...SESSIONS[1], activity: 'waiting', tone: 'attention', state: 'ask', active: true,
                  detail: 'Waiting for your answer' };
  await tick(5000);
  // The open tab says what its own stream says, like the header beside it.
  eq(api.tabDots(), ['t-busy', 't-attention'], 'the dots after a refresh');
  // And the refresh left the header alone: it is the open session's, not the list's.
  eq(api.statText.textContent, 'Running Bash · 12s', 'the header after a refresh');

  // Hidden under the find pane, the strip stays hidden; closing it brings it back current.
  api.openFind();
  SESSIONS[1] = { ...SESSIONS[1], activity: 'generating', tone: 'busy', state: 'working', detail: 'Thinking' };
  await tick(5000);
  if (!api.tabsHidden()) throw new Error('a refresh drew the tabs over the find pane');
  api.closeFind();
  eq(api.tabDots(), ['t-busy', 't-busy'], 'the dots once find closes');
});

acheck('a dropped stream leaves the header stale, not showing its last word', async () => {
  SESSIONS = BASE();
  await api.showList(true);
  const before = api.statText.textContent;
  api.openSession(SESSIONS[0], true);
  // Nothing from the stream yet: not the list's summary, and no Stop.
  eq([api.stat.className, api.statText.textContent], ['stale', 'connecting…'], 'before the first status');
  if (api.statText.textContent === before) throw new Error('the list summary was left as the session status');
  if (stopShown()) throw new Error('Stop offered before anything is known');

  stream().emit('status', status('tool', { tone: 'busy', detail: 'Running Bash', queued: 2 }));
  eq([api.stat.className, api.statText.textContent], ['t-busy', 'Running Bash · 12s · 2 queued'], 'a live status');
  if (!stopShown()) throw new Error('Stop missing while a tool runs');

  stream().emit('error', '');
  eq([api.stat.className, api.statText.textContent], ['stale', 'reconnecting…'], 'after the stream drops');
  if (stopShown()) throw new Error('Stop left up on a status nobody can vouch for');
  // The list poll failing too, while the session is open, says nothing over the header.
  LIST_DOWN = true;
  await tick(5000);
  LIST_DOWN = false;
  eq(api.statText.textContent, 'reconnecting…', 'a failed list poll while a session is open');

  // The browser reconnects by itself; the first status it brings is believed again.
  stream().emit('status', status('generating', { tone: 'busy', detail: 'Thinking' }));
  eq([api.stat.className, api.statText.textContent], ['t-busy', 'Thinking · 12s'], 'after reconnecting');
});

// A connection can die without an error: a NAT that forgot it, a Mac that slept. The
// browser may take minutes to notice, and until the page does, the header goes on saying
// "Running Bash · 12s", counter frozen, Stop up. The router speaks on an open stream at
// least once a second whatever the session is doing, so ten seconds of nothing is the
// signal — on the web as on both phones.
acheck('a stream silent for ten seconds is taken for dead: stale, no Stop, and reconnected', async () => {
  const mk = (id, title, created) => act(id, 'idle', { cwd: '/code/project-w', title, created_ms: created,
    tone: 'idle', state: 'idle', active: false, detail: 'Waiting for you' });
  SESSIONS = [mk('w-1', 'First', 1), mk('w-2', 'Second', 2)];
  await api.showList(true);
  // With nothing open, nothing is watched.
  let opened = streams.length;
  later(60e3); await tick(1000);
  eq(streams.length, opened, 'connections opened with only the list on screen');

  api.openSession(SESSIONS[0], true);
  const first = stream();
  const row = (text) => ({ role: 'assistant', kind: 'text', text });
  const busy = () => first.emit('status', status('tool', { tone: 'busy', detail: 'Running Bash' }));
  const stands = (when) => {
    eq([api.stat.className, api.statText.textContent, stopShown()], ['t-busy', 'Running Bash · 12s', true],
      `the header ${when}`);
    if (first.closed || stream() !== first) throw new Error(`taken for dead ${when}`);
  };
  // Every row and every status starts the ten seconds again. A status, a row five seconds
  // on, then a status: each probe comes 9.9 s after the last thing said, and more than ten
  // seconds after the one before it.
  first.emit('message', row('one'));
  busy();
  later(5000); await tick(1000);
  first.emit('message', row('two'));
  later(9900); await tick(1000);
  stands('9.9 s after a row, 14.9 s after a status');
  busy();
  later(9900); await tick(1000);
  stands('9.9 s after a status, 19.8 s after a row');
  eq(api.tabDots(), ['t-busy', 't-idle'], 'the open tab while its stream speaks');

  later(100); await tick(1000);
  eq([api.stat.className, api.statText.textContent], ['stale', 'reconnecting…'], 'after ten silent seconds');
  if (stopShown()) throw new Error('Stop left up on a stream nobody can vouch for');
  // The tab beside the header stops claiming what the stream last said, too.
  eq(api.tabDots(), ['t-idle', 't-idle'], 'the open tab once its stream is silent');
  if (!first.closed) throw new Error('the silent connection was left open');
  const second = stream();
  if (second === first || second.url !== first.url) throw new Error('not reconnected to the same session');
  // What was read stays readable while the new connection has nothing to show...
  eq(api.feed.children.length, 2, 'the rows while reconnecting');
  // ...and what it replays replaces them: the router starts every connection with the
  // transcript's tail, which must land once, not twice.
  second.emit('message', row('one'));
  second.emit('message', row('two'));
  eq(api.feed.children.length, 2, 'the rows once the new connection has replayed the tail');
  second.emit('status', status('generating', { tone: 'busy', detail: 'Thinking' }));
  eq([api.stat.className, api.statText.textContent, stopShown()], ['t-busy', 'Thinking · 12s', true],
    'the header once the new connection speaks');
  // The dead connection is history: whatever it still delivers changes nothing.
  first.emit('status', status('idle', { tone: 'idle', detail: 'Waiting for you' }));
  first.emit('message', row('late'));
  eq([api.statText.textContent, api.feed.children.length], ['Thinking · 12s', 2], 'after the old connection spoke');

  // Each connection gets ten seconds of its own. One that goes quiet is replaced in turn,
  // and one that never says anything — the router down — is retried every ten seconds,
  // not every second, with the rows still there to read.
  later(9900); await tick(1000);
  if (stream() !== second) throw new Error('replaced inside its own ten seconds');
  later(100); await tick(1000);
  const third = stream();
  if (third === second || !second.closed) throw new Error('a connection gone quiet was kept');
  later(9900); await tick(1000);
  if (stream() !== third) throw new Error('a connection that has said nothing yet was not given ten seconds');
  later(100); await tick(1000);
  if (stream() === third || !third.closed) throw new Error('a connection that never spoke was kept');
  eq([api.stat.className, api.feed.children.length], ['stale', 2], 'while the router says nothing');

  // A clock set back an hour must not hold the next reconnect off for that hour.
  const fourth = stream();
  fourth.emit('status', status('tool', { tone: 'busy', detail: 'Running Bash' }));
  later(-3600e3); await tick(1000);
  if (stream() !== fourth) throw new Error('a clock set back read as a silence');
  later(10000); await tick(1000);
  if (stream() === fourth) throw new Error('a clock set back an hour postponed the reconnect');

  // Back on the list, nothing is watched any more.
  await api.showList(true);
  later(3600e3);
  opened = streams.length;
  later(60e3); await tick(1000);
  eq(streams.length, opened, 'connections opened after going back to the list');
});

// The router has no way to resume a stream where it left off, so it answers every
// connection — the browser's own retry after an error included — with the transcript's
// tail. A feed that appended it again would show every row twice after any hiccup.
acheck('the browser reconnecting by itself does not show the tail twice', async () => {
  SESSIONS = BASE();
  await api.showList(true);
  api.openSession(SESSIONS[0], true);
  const s = stream();
  const tail = [{ role: 'user', kind: 'text', text: 'first ask' }, { role: 'assistant', kind: 'text', text: 'a reply' },
                { role: 'user', kind: 'text', text: 'second ask' }];
  // Only a connection that follows another starts a new copy: what was sent before the
  // first one spoke — its echo, on a feed opening just emptied — is still there after.
  await api.deliver('sent at once');
  for (const it of tail) s.emit('message', it);
  eq(api.feed.children.length, 4, 'the rows, with the echo of what was sent before the stream spoke');
  s.emit('error', '');
  eq(api.feed.children.length, 4, 'the rows while the browser retries');
  for (const it of tail) s.emit('message', it);
  eq(api.feed.children.length, 3, 'the rows after the browser reconnected');
  eq(api.sentLog(), ['first ask', 'second ask'], 'what ArrowUp offers');
});

// One ranking for a project's heading on the web, iOS and Android: waiting on you, then a
// turn that failed, then one that stalled — each a thing to go and look at — then work
// under way, then open and idle; the empty ring only once every session there has ended.
acheck('a project heading shows its most pressing session, ranked as on the phones', async () => {
  const H = 3600e3;
  const as = {
    waiting: { tone: 'attention', state: 'ask', active: true, detail: 'Bash wants to run' },
    failed: { tone: 'error', state: 'failed', active: false, detail: 'API error: overloaded' },
    stalled: { tone: 'warn', state: 'stalled', active: true, detail: 'No progress' },
    generating: { tone: 'busy', state: 'working', active: true, detail: 'Thinking' },
    tool: { tone: 'busy', state: 'working', active: true, detail: 'Running Bash' },
    background: { tone: 'busy', state: 'working', active: true, detail: 'Background workflow running' },
    idle: { tone: 'idle', state: 'idle', active: false, detail: 'Waiting for you' },
    closed: { tone: 'closed', state: 'idle', active: false, open: false, detail: 'Session ended',
              since_ms: undefined, ended_ms: AT - 3 * H },
    // A newer router's word: it outranks nothing, and neither has it ended.
    compacting: { tone: 'sparkle', state: 'working', active: true, detail: 'Compacting' },
  };
  const LIVE = ['waiting', 'stalled', 'generating', 'tool', 'background', 'compacting'];
  const cases = [
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
  const rows = (withTone) => cases.flatMap(([acts], i) => acts.map((a, j) => {
    const { tone, ...rest } = as[a];
    return act(`rk${i}-${j}`, a, { cwd: `/code/rank-${i}`, ...rest, ...(withTone ? { tone } : {}) });
  }));
  // With the router's tone, and with the activity alone: the same heading either way.
  for (const withTone of [true, false]) {
    SESSIONS = rows(withTone);
    await api.showList(true);
    cases.forEach(([acts, want], i) => {
      // The halo still says whether anything there is busy or waiting, whatever wins the colour.
      const halo = acts.some((a) => LIVE.includes(a));
      eq(classesOf(headingOf(`rank-${i}`)._html), [...(want ? [`t-${want}`] : []), ...(halo ? ['on'] : [])],
        `the heading of {${acts.join(', ')}}${withTone ? '' : ' without a tone'}`);
    });
  }
});

// The row line, as all three clients draw it: the router's words and how long, measured
// on the router's clock, for every activity — idle included — and for an ended session
// how long ago it ended.
acheck('each row says what it is doing and for how long, on the router\'s clock', async () => {
  const H = 3600e3;
  SESSIONS = [
    act('idle-1', 'idle', { tone: 'idle', state: 'idle', active: false, detail: 'Waiting for you', since_ms: AT - H }),
    act('shell-1', 'idle', { tone: 'idle', state: 'idle', active: false,
      detail: 'Waiting for you · background shell running', since_ms: AT - 10 * H }),
    act('gone-1', 'closed', { tone: 'closed', state: 'idle', active: false, open: false, detail: 'Session ended',
      since_ms: AT - 3 * H, ended_ms: AT - 3 * H, ended_how: 'exit', mid_turn: false }),
    act('fail-1', 'failed', { tone: 'error', state: 'failed', active: false, detail: 'API error: 529 Overloaded',
      since_ms: AT - 60e3 }),
    // Four seconds in, the words stand alone: a "· 2s" would only flicker.
    act('gen-1', 'generating', { tone: 'busy', state: 'working', active: true, detail: 'Thinking', since_ms: AT - 3e3 }),
    // Nothing to measure against but this machine's clock: no duration, rather than a wrong one.
    act('nc-1', 'idle', { tone: 'idle', state: 'idle', active: false, detail: 'Waiting for you',
      since_ms: AT - H, as_of_ms: undefined }),
    act('nc-2', 'closed', { tone: 'closed', state: 'idle', active: false, open: false, detail: 'Session ended',
      since_ms: undefined, ended_ms: AT - 3 * H, as_of_ms: undefined }),
  ];
  await api.showList(true);
  const want = {
    'idle-1': 'Waiting for you · 1h', 'shell-1': 'Waiting for you · background shell running · 10h',
    'gone-1': 'Session ended · 3h ago', 'fail-1': 'API error: 529 Overloaded · 1m', 'gen-1': 'Thinking',
    'nc-1': 'Waiting for you', 'nc-2': 'Session ended',
  };
  for (const [id, line] of Object.entries(want)) eq(lineOf(cardOf(id)._html), line, `${id} line`);
  // The open view's header reads the same way...
  api.openSession(SESSIONS[2], true);
  stream().emit('status', status('closed', { state: 'idle', tone: 'closed', detail: 'Session ended', open: false,
    since_ms: undefined, ended_ms: AT - 3 * H, quiet_ms: 0 }));
  eq(api.statText.textContent, 'Session ended · 3h ago', 'the header of a closed session');
  // ...and a status without the router's clock takes the router's own measure of it.
  stream().emit('status', status('idle', { state: 'idle', tone: 'idle', detail: 'Waiting for you',
    as_of_ms: undefined, quiet_ms: 300e3 }));
  eq(api.statText.textContent, 'Waiting for you · 5m', 'a status with no router clock');
});

acheck('Stop is offered for a running turn only — never for background work alone', async () => {
  SESSIONS = BASE();
  await api.showList(true);
  api.openSession(SESSIONS[0], true);
  const shown = (s) => { stream().emit('status', s); return stopShown(); };
  const cases = [
    [status('generating'), true], [status('tool', { state: 'tool' }), true], [status('stalled'), true],
    // Background still reads `working` to an older client. Only the activity tells it apart.
    [status('background', { detail: '2 background agents running' }), false],
    [status('waiting', { state: 'ask' }), false], [status('idle', { state: 'idle' }), false],
    [status('failed', { state: 'failed' }), false], [status('closed', { state: 'idle' }), false],
    // A word this page does not know: no Stop, whatever the older field says.
    [status('compacting'), false],
    // An older router: its own two busy words.
    [{ state: 'working', detail: 'Working', quiet_ms: 0, queued: 0 }, true],
    [{ state: 'tool', detail: 'Running Bash', quiet_ms: 0, queued: 0 }, true],
    [{ state: 'ask', detail: 'Waiting for your answer', quiet_ms: 0, queued: 0 }, false],
    [{ state: 'idle', detail: 'Waiting for you', quiet_ms: 0, queued: 0 }, false],
  ];
  for (const [s, want] of cases) {
    if (shown(s) !== want) throw new Error(`${s.activity || 'legacy ' + s.state}: Stop ${want ? 'missing' : 'offered'}`);
  }
});

acheck('an answer names the prompt it answers', async () => {
  SESSIONS = BASE();
  await api.showList(true);
  api.openSession(SESSIONS[0], true);
  const answers = () => posts.filter((p) => p.url === '/api/live/answer').map((p) => p.body);
  const buttons = () => (api.perm.children.find((c) => c.className === 'row') || { children: [] }).children;
  posts.length = 0;

  // A permission prompt, with one more behind it. The stream reporting it draws the card
  // at once, without waiting for the two-second poll.
  PROMPT = { pending: true, request_id: 'req-7', tool: 'Bash', question: false, display_name: 'Bash',
             input: { command: 'git push' }, suggestions: [{ index: 0, label: 'Always allow Bash(git push:*)' }],
             count: 2 };
  stream().emit('status', status('waiting', { state: 'ask', tone: 'attention', detail: 'Bash wants to run',
    waiting_on: { kind: 'permission', tool: 'Bash', request_id: 'req-7', tool_use_id: 't-7', summary: 'git push' } }));
  await flush();
  if (!/Bash is waiting/.test(api.perm.children[0].textContent)) throw new Error('the stream reported a prompt; no card');
  if (!api.perm.children.some((c) => c.textContent === '1 more waiting after this one')) {
    throw new Error('a second prompt behind this one went unsaid');
  }
  const [allow, always, deny] = buttons();
  await allow.onclick();
  await always.onclick();
  await deny.onclick();
  eq(answers(), [
    { session: 'aaa-1', allow: true, request_id: 'req-7' },
    { session: 'aaa-1', allow: true, suggestion: 0, request_id: 'req-7' },
    { session: 'aaa-1', allow: false, message: 'Declined from the live view.', request_id: 'req-7' },
  ], 'the permission answers');

  // Answered elsewhere meanwhile: the router refuses, and the card becomes what waits now.
  PROMPT = { ...PROMPT, request_id: 'req-8', display_name: 'Edit', tool: 'Edit', count: 1 };
  await api.pollPrompt();
  PROMPT = { ...PROMPT, request_id: 'req-9', display_name: 'Write', tool: 'Write' };
  ANSWER = { status: 409, body: { ok: false, reason: 'That prompt is no longer waiting' } };
  await buttons()[0].onclick();
  await flush();
  ANSWER = { status: 200, body: { ok: true } };
  if (!/Write is waiting/.test(api.perm.children[0].textContent)) throw new Error('a refused card was left up');

  // A question is drawn from the transcript, which has no prompt id; the router's live
  // prompt does, and the form sends it when that prompt is its question.
  const ASK = { questions: [{ question: 'Which one?', header: 'Pick', options: [{ label: 'A' }, { label: 'B' }] }] };
  const form = () => {
    const wrap = api.askForm(ASK);
    wrap.children[1]._qa['input[type=radio], input[type=checkbox]'] = [{ checked: true, value: 'B' }];
    const foot = wrap.children[wrap.children.length - 1];
    return { submit: foot.querySelector('.btn').onclick, hint: foot.querySelector('.ask-hint') };
  };
  posts.length = 0;
  PROMPT = { pending: true, request_id: 'req-q', tool: 'AskUserQuestion', question: true, count: 1,
             input: { questions: [{ question: 'Which one?' }] } };
  await form().submit();
  eq(answers(), [{ session: 'aaa-1', answers: { 'Which one?': 'B' }, request_id: 'req-q' }], 'the question answer');

  // Not its question: a permission prompt handed these answers would take them as its
  // approval, and another question would get answers to this one. Neither is sent.
  posts.length = 0;
  for (const [live, why] of [
    [{ pending: true, request_id: 'req-p', tool: 'Bash', question: false, count: 1 }, /^Not sent: a permission prompt/],
    [{ pending: true, request_id: 'req-r', tool: 'AskUserQuestion', question: true, count: 1,
       input: { questions: [{ question: 'Something else?' }] } }, /^Not sent: a different question/],
  ]) {
    PROMPT = live;
    const f = form();
    await f.submit();
    if (!why.test(f.hint.textContent)) throw new Error(`unexplained: ${f.hint.textContent}`);
  }
  eq(answers(), [], 'answers sent to a prompt that was not the question');

  // Nothing live to name (an unwrapped session): asked as before, and sent as a message
  // when the router cannot answer it directly.
  PROMPT = { pending: false };
  ANSWER = { status: 503, body: { ok: false, reason: "That session isn't waiting on anything right now." } };
  await form().submit();
  ANSWER = { status: 200, body: { ok: true } };
  eq(answers(), [{ session: 'aaa-1', answers: { 'Which one?': 'B' } }], 'an answer with nothing to name');
  if (!posts.some((p) => p.url === '/api/live/send')) throw new Error('the typed answer was lost');
  PROMPT = { pending: false };
});

acheck('a payload from an older router renders as it always did', async () => {
  // Nothing but the keys every router has always sent.
  SESSIONS = [...BASE(),
    { session_id: 'eee-5', cwd: '/code/project-b', project: 'p2', title: 'Stuck',
      created_ms: 4000, last_activity_ms: Date.now() - 60e3, active: false, wrapped: true, state: 'stalled' },
    { session_id: 'fff-6', cwd: '/code/project-b', project: 'p2', title: 'Quiet',
      created_ms: 4500, last_activity_ms: Date.now() - 30e3, active: false, wrapped: true, state: 'idle' },
    { session_id: 'ggg-7', cwd: '/code/project-b', project: 'p2', title: 'From the future',
      created_ms: 4600, last_activity_ms: Date.now() - 20e3, active: false, wrapped: true, state: 'mystery' }];
  await api.showList(true);
  // The same tones as the old per-state classes: working with its ring, ask, failed,
  // stalled, idle, and an unknown word plain gray (the hues are the phones' since the
  // palette was unified: working green, waiting orange, idle gray).
  const want = { 'aaa-1': ['t-busy', 'on'], 'bbb-2': ['t-attention'], 'ccc-3': ['t-error'],
                 'eee-5': ['t-warn'], 'fff-6': ['t-idle'], 'ggg-7': [] };
  for (const [id, dot] of Object.entries(want)) {
    eq(classesOf(cardOf(id)._html), dot, `${id} dot`);
    if (lineOf(cardOf(id)._html) !== null) throw new Error(`${id} grew a status line with nothing to say`);
  }
  // `active` is all such a router can say, so the header and the headings say what they
  // always did: how many moved lately, and whether each project did.
  eq(api.statText.textContent, '1 active', 'the header');
  eq(classesOf(headingOf('project-a')._html), ['t-busy', 'on'], 'a project where something moved');
  eq(classesOf(headingOf('project-b')._html), ['t-idle'], 'a project where nothing did');
  // Rows quiet for over twelve hours still fold away. aaa-1 stays because it is `active` —
  // which from such a router means its file moved in the last two minutes, so a real one
  // is never twelve hours old and nothing changes for it; only this fixture is ancient.
  const folded = cardsIn(older().querySelector('div')).map((c) => /class="meta">([^<]*)</.exec(c._html)[1]);
  eq(folded.sort(), ['bbb-2', 'ccc-3'], 'what folds into older');

  api.openSession(SESSIONS[0], true);
  eq(api.tabDots(), ['t-busy', 't-attention'], 'the tabs');
  stream().emit('status', { state: 'tool', detail: 'Running Bash', quiet_ms: 12e3, queued: 1 });
  eq([api.stat.className, api.statText.textContent], ['t-busy', 'Running Bash · 12s · 1 queued'], 'a running tool');
  if (!stopShown()) throw new Error('Stop missing for a legacy tool');
  stream().emit('status', { state: 'ask', detail: 'Waiting for your answer', quiet_ms: 2e3, queued: 0 });
  eq([api.stat.className, api.statText.textContent], ['t-attention', 'Waiting for your answer'], 'a question');
  if (stopShown()) throw new Error('Stop offered while asking');
  stream().emit('status', { state: 'idle', detail: 'Waiting for you', quiet_ms: 300e3, queued: 0 });
  eq(api.statText.textContent, 'Waiting for you · 5m', 'an idle session');

  // A prompt without a count is still answered by its id, and says nothing of others.
  posts.length = 0;
  PROMPT = { pending: true, request_id: 'req-old', tool: 'Bash', question: false, display_name: 'Bash',
             input: { command: 'ls' }, suggestions: [] };
  await api.pollPrompt();
  if (api.perm.children.some((c) => c.className === 'also')) throw new Error('invented a count');
  await api.perm.children.find((c) => c.className === 'row').children[0].onclick();
  eq(posts.map((p) => p.body.request_id), ['req-old'], 'the answer');
  PROMPT = { pending: false };
});

/// Replay a stream through the page as the router wrote it and a browser delivers it: a
/// session is opened, every feed row goes to the stream's message handler and every
/// `status` event to its status listener, and the header is read after each status. The
/// page swallows a row that throws, so the feed stays whole; here a row that did not land
/// is a failure, and so is a header that reads "undefined", "NaN" or "null" where the
/// router said no such thing, says nothing, or wears a colour the page has no rule for —
/// or not the one the router chose. The synthetic checks above only ever see the payloads
/// they were written with; a replay is what the router really sends. (This used to skip
/// every status, so no captured status ever reached the header.)
const TONE_CLASSES = new Set(['busy', 'attention', 'warn', 'error', 'idle', 'closed'].map((t) => `t-${t}`));
function replay(text, what) {
  // Server-sent events: one ends at a blank line; `event:` names it, and one with no name
  // is a message; `data:` lines join with newlines; a line opening with ':' is a comment,
  // which is all the keep-alive is. An event the capture cut off before its blank line is
  // dropped, as a browser drops it: `curl -N --max-time` ends a capture wherever it likes.
  const events = [];
  let type = '', data = [];
  for (const line of text.split(/\r\n|\r|\n/)) {
    if (!line) {
      if (data.length) events.push({ type: type || 'message', data: data.join('\n') });
      type = ''; data = [];
      continue;
    }
    if (line.startsWith(':')) continue;
    const at = line.indexOf(':');
    const field = at < 0 ? line : line.slice(0, at);
    const value = at < 0 ? '' : line.slice(at + 1).replace(/^ /, '');
    if (field === 'event') type = value;
    else if (field === 'data') data.push(value);
  }
  api.openSession(BASE()[0], true);
  const s = stream();
  let items = 0;
  const headers = [];
  for (const e of events) {
    if (e.type === 'message') {
      const before = api.feed.children.length;
      s.emit('message', e.data);
      if (api.feed.children.length !== before + 1) {
        api.render(JSON.parse(e.data));  // throws what the page swallowed, when it can
        throw new Error(`row ${items + 1} of ${what} did not render: ${e.data.slice(0, 160)}`);
      }
      items++;
      continue;
    }
    s.emit(e.type, e.data);
    if (e.type !== 'status') continue;
    let said;
    try { said = JSON.parse(e.data); } catch { throw new Error(`status ${headers.length + 1} of ${what} is not JSON: ${e.data.slice(0, 160)}`); }
    const header = [api.stat.className, api.statText.textContent];
    headers.push(header);
    const [cls, words] = header;
    const chosen = TONE_CLASSES.has(`t-${said && said.tone}`) ? `t-${said.tone}` : null;
    // Only what the page added: an API error's own text may well say "null".
    const added = said && said.detail ? words.replace(said.detail, '') : words;
    if (!(TONE_CLASSES.has(cls) || cls === 'stale') || (chosen && cls !== chosen) || !words
        || /undefined|NaN|null/.test(added)) {
      throw new Error(`status ${headers.length} of ${what} read ${JSON.stringify(header)}: ${e.data.slice(0, 200)}`);
    }
  }
  if (!items && !headers.length) throw new Error(`nothing to replay in ${what}`);
  return { items, headers };
}

// The wire as the router writes it — a feed row is an unnamed event, a status a named one,
// the keep-alive a comment — replayed on every run, not only when a capture is handed in.
// Synthetic words in the router's real shapes, one status per activity and one from an
// older router. The first is a real session's heartbeat after ten idle hours with a shell
// left running: a stretch of time none of the fixtures above ever asked the page to print.
const AS_OF = 1790688279420;
const wireStatus = (o) => ['event: status', `data: ${JSON.stringify(o)}`, ''];
const wireRow = (o) => [`data: ${JSON.stringify({ t: '2026-09-29T11:14:19.178Z', dur_ms: null, sidechain: false, ...o })}`, ''];
const since = (ms) => ({ quiet_ms: ms, since_ms: AS_OF - ms, as_of_ms: AS_OF });
const WIRE = [
  ':', '',
  ...wireRow({ role: 'user', kind: 'text', text: 'Is the build green?' }),
  ...wireRow({ role: 'assistant', kind: 'tool_use', name: 'Bash', text: 'cargo test', dur_ms: 2810 }),
  ...wireRow({ role: 'user', kind: 'tool_result', text: 'test result: ok. 12 passed', dur_ms: 18112 }),
  ...wireStatus({ state: 'idle', detail: 'Waiting for you · background shell running', ...since(36123781), queued: 0,
    activity: 'idle', open: true, tone: 'idle' }),
  ...wireStatus({ state: 'working', detail: 'Thinking', ...since(0), queued: 0, activity: 'generating', open: true, tone: 'busy' }),
  ...wireStatus({ state: 'tool', detail: 'Running Bash', ...since(95e3), queued: 0, activity: 'tool', open: true, tone: 'busy' }),
  ...wireStatus({ state: 'ask', detail: 'Bash wants to run', ...since(60e3), queued: 1, activity: 'waiting', open: true,
    waiting_on: { kind: 'permission', tool: 'Bash', request_id: 'r-wire', tool_use_id: 't-wire', summary: 'git push' },
    tone: 'attention' }),
  ...wireStatus({ state: 'working', detail: '2 background agents running', ...since(300e3), queued: 0, activity: 'background',
    open: true, background: { agents: 2, workflows: 0, shells: 0 }, tone: 'busy' }),
  ...wireStatus({ state: 'working', detail: 'No progress', ...since(700e3), queued: 0, activity: 'stalled', open: true, tone: 'warn' }),
  ...wireStatus({ state: 'failed', detail: 'API error: 529 Overloaded', ...since(60e3), queued: 0, activity: 'failed',
    open: true, tone: 'error' }),
  ...wireStatus({ state: 'idle', detail: 'Session ended mid-turn', ...since(3 * 3600e3), queued: 0, activity: 'closed',
    open: false, tone: 'closed', ended_ms: AS_OF - 3 * 3600e3 }),
  ...wireStatus({ state: 'idle', detail: 'Waiting for you', quiet_ms: 893924, queued: 0 }),
].join('\n');

acheck('the wire, replayed: every row renders and every status reads cleanly', () => {
  const { items, headers } = replay(WIRE, 'the built-in stream');
  eq(items, 3, 'rows rendered');
  eq(headers, [
    ['t-idle', 'Waiting for you · background shell running · 10h'],
    ['t-busy', 'Thinking'],
    ['t-busy', 'Running Bash · 2m'],
    ['t-attention', 'Bash wants to run · 1m · 1 queued'],
    ['t-busy', '2 background agents running · 5m'],
    ['t-warn', 'No progress · 12m'],
    ['t-error', 'API error: 529 Overloaded · 1m'],
    ['t-closed', 'Session ended mid-turn · 3h ago'],
    ['t-idle', 'Waiting for you · 15m'],
  ], 'the header after each status');
});

// A path that names no file is a mistake to hear about, not a replay to skip in silence.
const captured = process.argv[2];
if (captured) {
  acheck('every captured event renders, statuses included', () => {
    if (!fs.existsSync(captured)) throw new Error(`no such capture: ${captured}`);
    const { items, headers } = replay(fs.readFileSync(captured, 'utf8'), captured);
    console.log(`        (${items} events, ${headers.length} statuses)`);
  });
}

// The feed can only ever show the tail of a transcript, so this pane is the only way to
// the rest of it. It has to survive opening from either mode and drawing real rows.
acheck('find opens from the list and from a session, and closes back to where it was', async () => {
  SESSIONS = BASE();
  await api.showList(true);
  api.openFind();
  if (api.findPane._cls.has('hide')) throw new Error('the pane stayed hidden');
  api.closeFind();
  if (!api.findPane._cls.has('hide')) throw new Error('the pane stayed open');
  api.openSession(SESSIONS[0], true);
  api.openFind();
  api.closeFind();
  if (api.tabsHidden()) throw new Error('closing find left the tabs hidden');
});

acheck('a hit renders, and one already in the open session is not a link to it', async () => {
  await api.showList(true);
  api.drawFind(SEARCH, 'kestrel');
  const html = api.findRows.innerHTML;
  if (!html.includes('<mark>kestrel</mark>')) throw new Error('the match was not marked');
  if (!html.includes('tool call')) throw new Error("the kind was not put into the reader's words");
  if ([...html.matchAll(/data-open="([^"]+)"/g)].length !== 2) {
    throw new Error('from the list, every hit should open its session');
  }
  api.openSession(SESSIONS[0], true);
  api.drawFind({ hits: [{ ...SEARCH.hits[0], session_id: SESSIONS[0].session_id }] }, 'kestrel');
  if (api.findRows.innerHTML.includes('data-open')) {
    throw new Error('a hit in the session already open was offered as a link to it');
  }
});

check('a stray marker cannot leave a tag open, and snippet text is escaped', () => {
  const out = api.findMark('a ‹b› c ‹d');
  const opens = (out.match(/<mark>/g) || []).length;
  const closes = (out.match(/<\/mark>/g) || []).length;
  if (opens !== closes) throw new Error(`unbalanced: ${opens} open, ${closes} close`);
  if (api.findMark('<script>x</script>').includes('<script>')) {
    throw new Error('snippet text reached the page as markup');
  }
});

check('an empty result says so rather than showing nothing', () => {
  api.drawFind({ hits: [] }, 'nothing');
  if (!api.findRows.innerHTML.includes('Nothing matched')) throw new Error('an empty answer was silent');
});

queue.then(() => {
  console.log(failures ? `\n  ${failures} check(s) failed` : '\n  all checks passed');
  process.exit(failures ? 1 : 0);
});
