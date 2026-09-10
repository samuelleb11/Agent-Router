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
    innerHTML: '', textContent: '', value: '', title: '',
    scrollTop: 0, scrollHeight: 100, clientHeight: 50,
    selectionStart: 0, selectionEnd: 0, disabled: false,
    classList: {
      add: (...c) => c.forEach((x) => el._cls.add(x)),
      remove: (...c) => c.forEach((x) => el._cls.delete(x)),
      toggle: (c, f) => { const on = f === undefined ? !el._cls.has(c) : f; on ? el._cls.add(c) : el._cls.delete(c); return on; },
      contains: (c) => el._cls.has(c),
    },
    appendChild: (c) => (el.children.push(c), c),
    append: (...c) => el.children.push(...c),
    querySelector: () => mkEl(), querySelectorAll: () => [],
    addEventListener() {}, closest: () => mkEl(), setSelectionRange() {},
    after() {}, remove() {}, focus() {},
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
global.fetch = () => Promise.resolve({ ok: true, json: () => Promise.resolve([]) });
global.requestAnimationFrame = (f) => setTimeout(f, 0);
global.cancelAnimationFrame = () => {};
global.getComputedStyle = () => ({ lineHeight: '18' });
global.setInterval = () => 0;
global.addEventListener = () => {};

check('script evaluates', () => { new Function(js)(); });

const api = new Function(
  js + '\n; return { render, md, openSession, showList, say, askForm, clampable };'
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

console.log(failures ? `\n  ${failures} check(s) failed` : '\n  all checks passed');
process.exit(failures ? 1 : 0);
