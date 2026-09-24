// Smoke-test assets/live.html outside a browser.
//
// The page is plain script in a served file, so nothing type-checks it and `cargo test`
// cannot reach it. Every UI bug that reached the user was of a kind this catches: a
// local shadowing a browser global, a replacement landing inside the function it was
// meant to call, a handler that throws on real data. It runs the script, drives the
// paths a reader actually takes, and pushes captured events through the renderer.
//
//   node scripts/check-live-ui.js [captured-sse-file]
const fs = require('fs');
const path = require('path');

const file = path.join(__dirname, '..', 'assets', 'live.html');
const src = fs.readFileSync(file, 'utf8');
const js = src.slice(src.indexOf('<script>') + 8, src.lastIndexOf('</script>'));

let failures = 0;
const check = (name, fn) => {
  try { fn(); console.log(`  ok    ${name}`); }
  catch (e) { failures++; console.log(`  FAIL  ${name}\n        ${e.message}`); }
};

// --- a DOM just real enough to run the page ---
const mkEl = (tag = 'div') => {
  const el = {
    tagName: tag, children: [], style: {}, dataset: {}, _cls: new Set(),
    get innerHTML() { return el._html || ''; },
    set innerHTML(v) {
      el._html = v;
      // The page builds a tab as `…<span class="lbl">Title</span>`; capture it so the
      // check can assert on what a reader would actually see.
      const m = /class="lbl">([^<]*)</.exec(v);
      if (m) el._label = m[1];
      if (v === '') el.children.length = 0;
    },
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
    querySelector: () => mkEl(), querySelectorAll: () => [],
    addEventListener() {}, closest: () => mkEl(), setSelectionRange() {},
    after() {}, remove() {}, focus() {}, select() {},
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
global.EventSource = class { addEventListener() {} close() {} };
// Two sessions in one project and one in another, so the tab strip has something to
// draw and its ordering is exercised.
const SESSIONS = [
  { session_id: 'aaa-1', cwd: '/code/project-a', project: 'p1', title: 'Older one',
    created_ms: 1000, last_activity_ms: 9000, active: true, wrapped: true, state: 'working' },
  { session_id: 'bbb-2', cwd: '/code/project-a', project: 'p1', title: 'Newer one',
    created_ms: 5000, last_activity_ms: 2000, active: false, wrapped: false, state: 'ask' },
  { session_id: 'ccc-3', cwd: '/code/project-b', project: 'p2', title: 'Elsewhere',
    created_ms: 3000, last_activity_ms: 3000, active: false, wrapped: false, state: 'failed' },
];
// A search reply, as the router spells it. The marker pair is what the page splits on.
const SEARCH = {
  query: 'kestrel', expression: 'kestrel',
  sessions: [{ session_id: 'aaaaaaaa-1111-2222-3333-444444444444', project: '/code/agent-router',
               blocks: 2, hits: 3, first_at: Date.now() - 6e5, last_at: Date.now() - 1e4 }],
  hits: [
    { block_id: 1, kind: 'tool_use', role: 'assistant', ordinal: 4, at: Date.now() - 1e4,
      session_id: 'aaaaaaaa-1111-2222-3333-444444444444', project: '/code/agent-router',
      snippet: 'where does the \u2039kestrel\u203a roost', rank: -8 },
    // A stray opening marker with no close: it may shift the emphasis, never break out.
    { block_id: 2, kind: 'text', role: 'user', ordinal: 0, at: Date.now() - 6e5,
      session_id: 'bbbbbbbb-1111-2222-3333-444444444444', project: '/code/other',
      snippet: 'a stray \u2039marker and no close', rank: -7 },
  ],
};
global.fetch = (url = '') => Promise.resolve({
  ok: true,
  json: () => Promise.resolve(
    String(url).includes('/api/search') ? SEARCH
      : String(url).includes('/api/live/sessions') ? SESSIONS : {}),
});
global.requestAnimationFrame = (f) => setTimeout(f, 0);
global.cancelAnimationFrame = () => {};
global.getComputedStyle = () => ({ lineHeight: '18' });
global.setInterval = () => 0;
global.addEventListener = () => {};

check('script evaluates', () => { new Function(js)(); });

const api = new Function(
  js + '\n; return { render, md, openSession, showList, say, askForm, clampable, loadSessions,' +
       ' tabLabels: () => [...tabs.children].map(t => (t._label || \'\')),' +
       ' openFind, closeFind, drawFind, findMark, findRows, findPane };'
)();

check('a local does not shadow a browser global the page uses', () => {
  const declared = new Set();
  for (const m of js.matchAll(/(?:let|const|var)\s+([A-Za-z_$][\w$]*)/g)) declared.add(m[1]);
  const globals = ['history', 'location', 'name', 'status', 'top', 'self', 'parent', 'origin', 'screen', 'event'];
  const clash = globals.filter((g) => declared.has(g));
  if (clash.length) throw new Error(`shadows ${clash.join(', ')} — see the \`history\` bug that stopped cards opening`);
});

check('opening a session does not throw', () => {
  api.openSession({ session_id: 'abc-1', cwd: '/code/project-a', project: 'p', git_branch: 'dev',
                    title: 't', last_activity_ms: Date.now(), active: true, wrapped: true }, true);
});

check('going back to the list does not throw', () => api.showList(true));

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

check('the session list renders every state', async () => {
  await api.loadSessions();
  for (const st of ['working', 'ask', 'idle', 'stalled', 'failed']) {
    api.openSession({ ...SESSIONS[0], state: st }, true);
  }
});

check('tabs cover the project and keep creation order', async () => {
  await api.loadSessions();
  api.openSession(SESSIONS[0], true);
  const labels = api.tabLabels();
  if (labels.length !== 2) throw new Error(`expected 2 tabs for the project, got ${labels.length}`);
  if (labels[0] !== 'Older one') throw new Error(`oldest must lead, got ${labels.join(', ')}`);
});

check('archived sessions leave the tabs, but come back when live', async () => {
  const shelved = { ...SESSIONS[1], archived: true, active: false, state: 'idle',
                    last_activity_ms: Date.now() };
  SESSIONS[1] = shelved;
  await api.loadSessions();
  api.openSession(SESSIONS[0], true);
  if (api.tabLabels().length !== 1) throw new Error('an archived session should not hold a tab');

  SESSIONS[1] = { ...shelved, state: 'ask' };  // it wants something
  await api.loadSessions();
  api.openSession(SESSIONS[0], true);
  if (api.tabLabels().length !== 2) throw new Error('an archived session that wakes must reappear');
  SESSIONS[1] = { ...shelved, archived: false, state: 'ask' };
});

check('stale sessions do not clutter the tabs', async () => {
  SESSIONS[1] = { ...SESSIONS[1], archived: false, active: false, state: 'idle',
                  last_activity_ms: Date.now() - 40 * 3600 * 1000 };
  await api.loadSessions();
  api.openSession(SESSIONS[0], true);
  if (api.tabLabels().length !== 1) throw new Error('a session idle for days should not hold a tab');
});

const captured = process.argv[2];
if (captured && fs.existsSync(captured)) {
  check('every captured event renders', () => {
    const rows = fs.readFileSync(captured, 'utf8').split('\n')
      .filter((l) => l.startsWith('data: ')).map((l) => l.slice(6));
    let n = 0;
    for (const r of rows) {
      let it; try { it = JSON.parse(r); } catch { continue; }
      if (it.state) continue;
      api.render(it); n++;
    }
    if (!n) throw new Error('no events in ' + captured);
    console.log(`        (${n} events)`);
  });
}

// The feed can only ever show the tail of a transcript, so this pane is the only way to
// the rest of it. It has to survive opening from either mode and drawing real rows.
check('find opens from the list and from a session, and closes back to where it was', () => {
  api.showList(true);
  api.openFind();
  if (api.findPane._cls.has('hide')) throw new Error('the pane stayed hidden');
  api.closeFind();
  if (!api.findPane._cls.has('hide')) throw new Error('the pane stayed open');
  api.openSession(SESSIONS[0], true);
  api.openFind();
  api.closeFind();
});

check('a hit renders, and one already in the open session is not a link to it', () => {
  api.showList(true);
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
  const out = api.findMark('a \u2039b\u203a c \u2039d');
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

console.log(failures ? `\n  ${failures} check(s) failed` : '\n  all checks passed');
process.exit(failures ? 1 : 0);
