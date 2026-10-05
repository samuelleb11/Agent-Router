// Smoke-test the forecast parts of assets/index.html outside a browser.
//
// Same reasoning as check-live-ui.js: the page is plain script in a served file, so
// nothing type-checks it and `cargo test` cannot reach it. This one is aimed at the
// quota forecast — the renderers run against a captured /api/forecast payload, plus the
// shapes that payload takes when there is nothing to say: an account whose window has
// rolled over, one too fresh to have a slope, one already at 100%.
//
//   node scripts/check-dashboard-ui.js [forecast.json] [summary.json]
const fs = require('fs');
const path = require('path');

const root = path.join(__dirname, '..');
const src = fs.readFileSync(path.join(root, 'assets', 'index.html'), 'utf8');
const js = src.slice(src.indexOf('<script>') + 8, src.lastIndexOf('</script>'));

let failures = 0;
const report = (name, e) => {
  if (!e) return console.log(`  ok    ${name}`);
  failures++;
  console.log(`  FAIL  ${name}\n        ${e.stack.split('\n').slice(0, 3).join('\n        ')}`);
};
const check = (name, fn) => {
  try { fn(); report(name); }
  catch (e) { report(name, e); }
};
/// For a check that has to await the page's own async path.
///
/// `check` cannot take one: an async function handed to it returns a promise, which
/// cannot throw into its `try`, so a failing async check would report `ok` and the
/// assertion would escape as an unhandled rejection. These are settled before the
/// summary instead.
/// One chain, not one promise each: these drive the same page and the same DOM, so
/// starting them together lets a later check move state out from under an earlier one.
let queue = Promise.resolve();
const acheck = (name, fn) => {
  queue = queue.then(fn).then(() => report(name), (e) => report(name, e));
};

// --- a DOM just real enough to run the page ---
const mkEl = (tag = 'div') => {
  const el = {
    // nodeType matters: h() wraps anything without one in a text node, so a mock that
    // omits it turns every element child into the string "[object Object]".
    tagName: tag, nodeType: 1, children: [], style: { cssText: '' }, dataset: {}, _cls: new Set(),
    hidden: false, textContent: '', value: '', title: '', open: false, disabled: false,
    clientWidth: 900, clientHeight: 300, scrollTop: 0, scrollHeight: 100, isConnected: true,
    get innerHTML() { return el._html || ''; },
    set innerHTML(v) { el._html = v; if (v === '') el.children.length = 0; },
    classList: {
      add: (...c) => c.forEach((x) => el._cls.add(x)),
      remove: (...c) => c.forEach((x) => el._cls.delete(x)),
      toggle: (c, f) => { const on = f === undefined ? !el._cls.has(c) : f; on ? el._cls.add(c) : el._cls.delete(c); return on; },
      contains: (c) => el._cls.has(c),
    },
    append: (...c) => el.children.push(...c),
    insertBefore(node, ref) {
      const i = ref ? el.children.indexOf(ref) : -1;
      if (i < 0) el.children.push(node); else el.children.splice(i, 0, node);
      return node;
    },
    appendChild: (c) => (el.children.push(c), c),
    replaceChildren: (...c) => { el.children.length = 0; el.children.push(...c); },
    parentNode: null,
    // renderCrumbs reaches through a found node to its parent
    // (`querySelector('use[...]').parentNode`), so a found node has one. Only the found
    // node: giving every element a parent lazily makes the graph cyclic, and anything
    // that walks it — JSON.stringify, in one of the checks below — never returns.
    querySelector: () => { const found = mkEl(); found.parentNode = mkEl(); return found; },
    querySelectorAll: () => [],
    getBoundingClientRect: () => ({ left: 0, top: 0, right: 900, bottom: 300, width: 900, height: 300 }),
    setAttribute(k, v) { el[k] = v; }, getAttribute: (k) => el[k], removeAttribute() {},
    addEventListener() {}, removeEventListener() {}, closest: () => mkEl(),
    after() {}, remove() {}, focus() {}, select() {}, blur() {}, scrollIntoView() {},
    showModal() { el.open = true; }, close() { el.open = false; },
    createElementNS: (ns, t) => mkEl(t),
  };
  return el;
};
const byId = new Map();
global.document = {
  querySelector: () => mkEl(), querySelectorAll: () => [],
  createElement: (t) => mkEl(t), createElementNS: (ns, t) => mkEl(t),
  createTextNode: (t) => ({ nodeType: 3, textContent: String(t) }),
  getElementById: (id) => { if (!byId.has(id)) byId.set(id, mkEl()); return byId.get(id); },
  body: mkEl(), documentElement: mkEl(), addEventListener() {},
};
global.window = global;
global.location = { href: 'http://host/', origin: 'http://host', host: 'host', hash: '' };
global.history = { pushState() {}, replaceState() {} };
global.ResizeObserver = class { observe() {} unobserve() {} disconnect() {} };
global.matchMedia = () => ({ matches: false, addEventListener() {} });
global.requestAnimationFrame = (f) => setTimeout(f, 0);
global.cancelAnimationFrame = () => {};
global.getComputedStyle = () => ({ lineHeight: '18' });
global.setInterval = () => 0;
global.setTimeout = ((f, ms) => (ms > 50 ? 0 : setTimeout(f, 0)));
global.addEventListener = () => {};
// A search reply, as the router spells it. The marker pair is what the page splits on,
// and the second hit carries a stray opener to prove that cannot break out.
const SEARCH = {
  query: 'kestrel', expression: 'kestrel',
  sessions: [{ session_id: 'aaaaaaaa-1111-2222-3333-444444444444', project: '/code/agent-router',
               session_label: null, git_branch: 'dev', blocks: 2, hits: 3,
               first_at: Date.now() - 6e5, last_at: Date.now() - 1e4 }],
  hits: [
    { block_id: 1, kind: 'tool_use', chars: 40, role: 'assistant', ordinal: 4, request_id: 9,
      at: Date.now() - 1e4, session_id: 'aaaaaaaa-1111-2222-3333-444444444444',
      project: '/code/agent-router', session_label: null,
      snippet: 'where does the \u2039kestrel\u203a roost', rank: -8 },
    { block_id: 2, kind: 'text', chars: 20, role: 'user', ordinal: 0, request_id: 10,
      at: Date.now() - 6e5, session_id: 'bbbbbbbb-1111-2222-3333-444444444444',
      project: null, session_label: null,
      snippet: 'a stray \u2039marker and no close', rank: -7 },
  ],
};
// Every path the page asked for, so a check can see what it did not ask.
const fetched = [];
global.fetch = (url = '') => (fetched.push(String(url)), Promise.resolve({
  ok: true,
  json: () => Promise.resolve(
    String(url).includes('/api/search') ? SEARCH
      : String(url) === '/api/push' ? { configured: false, devices: [] }
      : /\/api\/(projects|sessions|requests|tools|accounts)/.test(String(url)) ? [] : {}),
}));

// What the markup gives the page that this DOM does not parse: the account form's three
// ways in, and the notes a hosted router rewords, in their own local words.
const sourceSelect = document.getElementById('a-source');
sourceSelect.children.push(...['signin', 'token', 'profile'].map((v) => Object.assign(mkEl('option'), { value: v })));
sourceSelect.value = 'signin';
const MARKUP = {};
for (const id of ['storenote', 'pushnote', 'pwnote', 'e-usage-note', 'a-token-note']) {
  const m = new RegExp(`<p[^>]*\\sid="${id}"[^>]*>([\\s\\S]*?)</p>`).exec(src);
  if (m) document.getElementById(id).innerHTML = MARKUP[id] = m[1];
}

const load = (f, d) => {
  try { return JSON.parse(fs.readFileSync(f, 'utf8')); } catch { return d; }
};
const FORECAST = load(process.argv[2] || path.join(root, 'scripts', 'fixtures', 'forecast.json'), null);

check('script evaluates', () => { new Function(js)(); });

const api = new Function(
  js + '\n; return { fcState, fcBasis, fcLine, runwaySvg, renderRunway, renderAllAccounts, hrs, applyAuth, pwForm,' +
       ' markup, runSearch, qbox, renderAccounts, renderSetup, renderRouterInfo, renderPush, connectSnippet, openEdit,' +
       ' searchEls: () => ({ box: document.getElementById(\'sresults\'),' +
       '   sess: document.getElementById(\'ssess\'), hits: document.getElementById(\'shits\') }) };'
)();

check('a local does not shadow a browser global the page uses', () => {
  const declared = new Set();
  for (const m of js.matchAll(/(?:let|const|var)\s+([A-Za-z_$][\w$]*)/g)) declared.add(m[1]);
  const clash = ['history', 'location', 'name', 'status', 'top', 'self', 'parent', 'origin', 'screen', 'event']
    .filter((g) => declared.has(g));
  if (clash.length) throw new Error(`shadows ${clash.join(', ')} — see the \`history\` bug that stopped cards opening`);
});

// Every state a window can be in, including the ones with nothing to say.
const NOW = Date.now();
const WINDOWS = {
  'runs out before the reset': {
    used: 0.98, reset_at: (NOW + 30 * 3600e3) / 1000 | 0, window_start: (NOW - 100 * 3600e3) / 1000 | 0,
    burn_per_active_hour: 0.0121, active_hours_left: 1.65, duty_fraction: 0.139,
    exhausts_at: NOW + 12 * 3600e3, resets_first: false, projected_end_util: 1,
    basis: 'this-window', measured_active_hours: 7.4, readings: 2639, expired: false,
    series: [{ t: NOW - 90 * 3600e3, u: 0.89 }, { t: NOW - 20 * 3600e3, u: 0.98 }],
  },
  'resets before it fills': {
    used: 0.06, reset_at: (NOW + 3.8 * 3600e3) / 1000 | 0, window_start: (NOW - 1.2 * 3600e3) / 1000 | 0,
    burn_per_active_hour: 0.0536, active_hours_left: 17.5, duty_fraction: 0.955,
    exhausts_at: null, resets_first: true, projected_end_util: 0.26,
    basis: 'this-window', measured_active_hours: 1.1, readings: 273, expired: false,
    series: [{ t: NOW - 3600e3, u: 0.01 }, { t: NOW - 600e3, u: 0.06 }],
  },
  'too young to have a slope': {
    used: 0.01, reset_at: (NOW + 4.9 * 3600e3) / 1000 | 0, window_start: (NOW - 360e3) / 1000 | 0,
    burn_per_active_hour: 0.05, active_hours_left: 19.8, duty_fraction: 1,
    exhausts_at: null, resets_first: true, projected_end_util: 0.25,
    basis: 'trailing-windows', measured_active_hours: 0, readings: 12, expired: false, series: [],
  },
  'nothing to go on': {
    used: 0, reset_at: (NOW + 5 * 3600e3) / 1000 | 0, window_start: NOW / 1000 | 0,
    burn_per_active_hour: 0, active_hours_left: null, duty_fraction: 0.01,
    exhausts_at: null, resets_first: true, projected_end_util: 0,
    basis: 'none', measured_active_hours: 0, readings: 1, expired: false, series: [],
  },
  'already at the limit': {
    used: 1, reset_at: (NOW + 2 * 3600e3) / 1000 | 0, window_start: (NOW - 100 * 3600e3) / 1000 | 0,
    burn_per_active_hour: 0.012, active_hours_left: 0, duty_fraction: 0.2,
    exhausts_at: NOW, resets_first: false, projected_end_util: 1,
    basis: 'this-window', measured_active_hours: 5, readings: 900, expired: false,
    series: [{ t: NOW - 50 * 3600e3, u: 0.9 }],
  },
  // The state the band exists to express: the central estimate crosses, but a quieter
  // week does not. Amber, and worded as the coin flip it is.
  'too close to call': {
    used: 0.49, reset_at: (NOW + 95 * 3600e3) / 1000 | 0, window_start: (NOW - 73 * 3600e3) / 1000 | 0,
    burn_per_active_hour: 0.0167, active_hours_left: 30.3, duty_fraction: 0.329,
    duty: { low: 0.248, mid: 0.329, high: 0.409 }, verdict: 'tight',
    exhausts_at: NOW + 93 * 3600e3, exhausts_at_early: NOW + 74 * 3600e3, exhausts_at_late: null,
    resets_first: false, projected_end_util: 1, projected_end_util_low: 0.883, projected_end_util_high: 1,
    basis: 'this-window', measured_active_hours: 29.1, readings: 7935, expired: false,
    series: [{ t: NOW - 70 * 3600e3, u: 0.01 }, { t: NOW - 10 * 3600e3, u: 0.41 }],
  },
  'rolled over while we were not looking': { expired: true, reset_at: (NOW - 26 * 3600e3) / 1000 | 0, seen_at: NOW - 30 * 3600e3 },
};

check('every window state produces a line, and only the real trouble is red', () => {
  for (const [name, w] of Object.entries(WINDOWS)) {
    const st = api.fcState(w);
    if (!st.text || /undefined|NaN|Infinity/.test(st.text)) throw new Error(`${name}: ${st.text}`);
    // Red is for a crossing that survives even a quiet week. A verdict of `tight` has an
    // exhausts_at too, and must not shout about it; a payload with no verdict at all is
    // an older router, and falls back to the boolean it does send.
    const shouldShout = !!w.exhausts_at && w.verdict !== 'tight';
    if (!!(st.tone === 'bad') !== shouldShout) throw new Error(`${name}: tone ${st.tone || 'plain'} for exhausts_at=${w.exhausts_at} verdict=${w.verdict}`);
    api.fcLine(w);
  }
  // With no forecast at all — the endpoint is missing, or this build predates it — the
  // meter above is still showing a real percentage from the response headers, so a line
  // reading "no reading yet" would contradict it. Say nothing instead.
  if (api.fcLine(null) !== null || api.fcLine(undefined) !== null) {
    throw new Error('an absent forecast must add no line under a meter that has a reading');
  }
  if (api.fcState(null).tone) throw new Error('an absent window must not be alarming');
});

check('a straddling band is amber and says so, and the band is disclosed', () => {
  const st = api.fcState(WINDOWS['too close to call']);
  if (st.tone !== 'warn') throw new Error(`a coin flip must not be red: ${st.tone}`);
  if (!/too close to call/.test(st.text)) throw new Error(st.text);
  if (!/quieter week it ends ≈88%/.test(st.text)) throw new Error(`the quiet end is the reassuring half: ${st.text}`);
  const basis = api.fcBasis(WINDOWS['too close to call']);
  if (!/25–41% across your days/.test(basis)) throw new Error(`the spread is why the wording hedges: ${basis}`);
  // A window with no band discloses one number, not a range of one.
  const flat = api.fcBasis(WINDOWS['resets before it fills']);
  if (/across your days/.test(flat)) throw new Error(`no spread, nothing to disclose: ${flat}`);
});

check('a router that predates the band still renders as it always did', () => {
  for (const k of ['runs out before the reset', 'already at the limit']) {
    if (api.fcState(WINDOWS[k]).tone !== 'bad') throw new Error(`${k} lost its warning`);
  }
  if (api.fcState(WINDOWS['resets before it fills']).tone === 'bad') throw new Error('a calm window went red');
  // And the runway draws a ray, not a cone, when there is no band to draw.
  if (/opacity=".13"/.test(api.runwaySvg(WINDOWS['runs out before the reset'], 'var(--tok)', 300))) {
    throw new Error('drew an uncertainty cone with no uncertainty to show');
  }
  if (!/opacity=".13"/.test(api.runwaySvg(WINDOWS['too close to call'], 'var(--tok)', 300))) {
    throw new Error('a straddling band must show its cone');
  }
});

check('the basis is disclosed, and never claims a fit it did not make', () => {
  const fitted = api.fcBasis(WINDOWS['resets before it fills']);
  if (!/fitted on/.test(fitted)) throw new Error(`expected a disclosed fit, got ${fitted}`);
  const borrowed = api.fcBasis(WINDOWS['too young to have a slope']);
  if (/fitted on/.test(borrowed)) throw new Error(`a borrowed rate must not claim a fit: ${borrowed}`);
  if (!/recent average/.test(borrowed)) throw new Error(`expected the fallback to say so, got ${borrowed}`);
  for (const k of ['nothing to go on', 'rolled over while we were not looking']) {
    if (api.fcBasis(WINDOWS[k]) !== null) throw new Error(`${k} has no basis to disclose`);
  }
});

check('hours read as hours', () => {
  const cases = [[0.25, '15 min'], [2.4, '2.4 h'], [9.96, '10.0 h'], [81, '81 h'], [null, '—']];
  for (const [v, want] of cases) {
    if (api.hrs(v) !== want) throw new Error(`hrs(${v}) = ${api.hrs(v)}, expected ${want}`);
  }
});

check('the burn-down chart draws for every state that has a window', () => {
  for (const [name, w] of Object.entries(WINDOWS)) {
    if (w.expired) continue;
    const svg = api.runwaySvg(w, 'var(--accent)', 300);
    if (!/<svg/.test(svg)) throw new Error(`${name}: no svg`);
    if (/NaN|undefined|Infinity/.test(svg)) throw new Error(`${name}: ${svg.match(/.{0,40}(NaN|undefined|Infinity).{0,40}/)[0]}`);
    // The projection ray is the point of the chart; it must always be drawn.
    if (!/class="proj"/.test(svg)) throw new Error(`${name}: no projection drawn`);
    // Only a real crossing gets the marker.
    if (/<circle/.test(svg) !== !!w.exhausts_at) throw new Error(`${name}: crossing marker disagrees with exhausts_at`);
  }
  // A window with no span at all must be refused rather than divided by zero.
  if (api.runwaySvg({ window_start: 10, reset_at: 10 }, 'x', 300) !== '') throw new Error('a zero-length window must not draw');
});

check('the runway panel survives an account with nothing current', () => {
  api.renderRunway(null);
  api.renderRunway({ five_hour: WINDOWS['rolled over while we were not looking'], seven_day: null });
  api.renderRunway({ five_hour: WINDOWS['resets before it fills'], seven_day: WINDOWS['runs out before the reset'] });
});

check('the all-accounts panel renders, and hides itself when there is nothing', () => {
  api.renderAllAccounts(null);
  if (!document.getElementById('allacct').hidden) throw new Error('no data must leave the panel hidden');
  api.renderAllAccounts({ accounts: [], global: {} });
  if (!document.getElementById('allacct').hidden) throw new Error('an empty list must leave the panel hidden');
});

check('the password is asked for once, on first boot, from this machine only', () => {
  const dlg = document.getElementById('pw'), chip = document.getElementById('remote');
  // Loopback-only bind: nothing to set up, nothing to show.
  api.applyAuth({ configured: false, remote_bind: false, from_loopback: true });
  if (dlg.open) throw new Error('no network bind, no prompt');
  if (!chip.hidden) throw new Error('the chip only matters when the network can reach us');
  // A remote viewer must never see the setup prompt — it cannot set one anyway.
  api.applyAuth({ configured: false, remote_bind: true, from_loopback: false });
  if (dlg.open) throw new Error('the first-boot prompt must not be offered off-host');
  if (chip.hidden || !/no password/.test(document.getElementById('remotetext').textContent)) throw new Error('the chip must say the network is unguarded');
  // First boot at the machine itself: ask.
  api.applyAuth({ configured: false, remote_bind: true, from_loopback: true });
  if (!dlg.open) throw new Error('first boot on a network bind must ask for a password');
  // Once set: chip says so, dialog stays shut.
  dlg.close();
  api.applyAuth({ configured: true, remote_bind: true, from_loopback: true, updated_at: Date.now() - 60000 });
  if (dlg.open) throw new Error('a configured router must not ask again');
  if (!/password set/.test(document.getElementById('remotetext').textContent)) throw new Error('the chip must say the network is guarded');
  if (document.getElementById('remotedot').className !== 'dot') throw new Error('a set password is not an alarm');
});

check('the password form builds in both homes without throwing', () => {
  api.pwForm(document.getElementById('pw-first'), true);
  api.pwForm(document.getElementById('pw-settings'), false);
});

// --- a hosted router: docs/hosting-on-kubernetes.md, "Web UI in server mode" ---
// What one says at /api/auth, and what a local router says today: no `mode` at all,
// which has to read as local, exactly as before.
const HOSTED = { configured: true, updated_at: Date.now() - 3600e3, from_loopback: false, remote_bind: true,
  listening_on: '0.0.0.0:8787', lan_url: 'https://router.example', env_override: false,
  mode: 'server', password_pinned: true, public_url: 'https://router.example' };
const LOCAL = { configured: true, updated_at: Date.now() - 60e3, from_loopback: true, remote_bind: false,
  listening_on: '127.0.0.1:8787', lan_url: 'http://10.30.0.183:8787', env_override: false };
const textOf = (el) => (el == null ? '' : (el.textContent || '') + (el.children || []).map(textOf).join(''));
const eq = (got, want, what) => {
  if (JSON.stringify(got) !== JSON.stringify(want)) {
    throw new Error(`${what}: got ${JSON.stringify(got)}, expected ${JSON.stringify(want)}`);
  }
};

check('a hosted router hides what only a Mac has, and a local one keeps all of it', () => {
  // The wrappers hold what they are named for: setup with its login item and wrapper rows
  // (drawn into #setupbody), and the network switch.
  const span = (from, to) => src.slice(src.indexOf(from), src.indexOf(to, src.indexOf(from)));
  if (!/Claude Code setup[\s\S]*id="setupbody"[\s\S]*id="s-project-add"[\s\S]*id="s-msg"[\s\S]*<\/div>\s*$/
    .test(span('<div id="setupsec">', '<h4 class="modal-sec next">Router</h4>'))) {
    throw new Error('#setupsec does not wrap the whole Claude Code setup section');
  }
  if (!/Listen on your network[\s\S]*id="netbox"[\s\S]*<\/div>\s*$/
    .test(span('<div id="netsec">', '<h4 class="modal-sec next">Password'))) {
    throw new Error('#netsec does not wrap the whole network switch');
  }
  const shown = () => ['setupsec', 'netsec'].filter((id) => !document.getElementById(id).hidden);
  api.applyAuth(LOCAL);
  eq(shown(), ['setupsec', 'netsec'], 'what a local router shows');
  api.applyAuth(HOSTED);
  eq(shown(), [], 'what a hosted router shows');
  // Nothing is drawn into the hidden section either: the setup and wrapper state go unasked.
  fetched.length = 0;
  api.renderSetup();
  if (fetched.length) throw new Error(`a hosted router was asked for ${fetched.join(', ')}`);
  api.applyAuth(LOCAL);
  eq(shown(), ['setupsec', 'netsec'], 'what a local router shows once more');
  fetched.length = 0;
  api.renderSetup();
  if (!fetched.includes('/api/setup')) throw new Error('a local router stopped reading its setup');
});

check('a hosted router adds an account by pasted token only, and a local one every way', () => {
  const offered = () => sourceSelect.children.map((o) => o.value);
  const form = () => ({ type: sourceSelect.value, token: !document.getElementById('a-token-row').hidden,
    signin: !document.getElementById('signin-row').hidden, save: document.getElementById('a-save').textContent });
  api.applyAuth(LOCAL);
  eq(offered(), ['signin', 'token', 'profile'], 'what a local router offers');
  api.applyAuth(HOSTED);
  eq(offered(), ['token'], 'what a hosted router offers');
  eq(form(), { type: 'token', token: true, signin: false, save: 'Add account' }, 'the form on a hosted router');
  api.applyAuth(LOCAL);
  eq(offered(), ['signin', 'token', 'profile'], 'what a local router offers once more');
  eq(form(), { type: 'signin', token: false, signin: true, save: 'Start sign-in' }, 'the form on a local router');
});

check('a pinned password is the deployment\'s: said so, with nothing to set, change or remove', () => {
  const root = document.getElementById('pw-settings'), dlg = document.getElementById('pw');
  api.applyAuth(HOSTED);
  eq(textOf(root), 'Set by the deployment. Change the Secret and restart the router.', 'the password section');
  // Not even with what would ask for one at the machine itself...
  dlg.close();
  api.applyAuth({ ...HOSTED, configured: false, from_loopback: true });
  if (dlg.open) throw new Error('a hosted router asked for a first password');
  // ...which a local router does ask with: the mode is what keeps it shut.
  api.applyAuth({ ...LOCAL, configured: false, remote_bind: true, from_loopback: true });
  if (!dlg.open) throw new Error('a local router no longer asks on first boot');
  dlg.close();
  api.applyAuth(LOCAL);
  if (!/Change password/.test(textOf(root)) || !/Remove password/.test(textOf(root))) {
    throw new Error(`a local router lost its password form: ${textOf(root)}`);
  }
});

check('the connection snippet is the page\'s own origin, with the password wherever it is asked', () => {
  const was = global.location;
  const at = (origin) => { global.location = { ...was, origin, host: origin.replace(/^\w+:\/\//, '') }; };
  const env = () => JSON.parse(api.connectSnippet()).env;
  const TOKEN = '<the router password>';
  try {
    at('https://router.example');
    api.applyAuth(HOSTED);
    eq(env(), { ANTHROPIC_BASE_URL: 'https://router.example', ANTHROPIC_AUTH_TOKEN: TOKEN }, 'a hosted router');
    // Every caller of a hosted router gives it, a kubectl port-forward to localhost included.
    at('http://localhost:8787');
    api.applyAuth({ ...HOSTED, from_loopback: true });
    eq(env(), { ANTHROPIC_BASE_URL: 'http://localhost:8787', ANTHROPIC_AUTH_TOKEN: TOKEN }, 'through a port-forward');
    // A local router read from another machine on the network.
    at('http://10.30.0.183:8787');
    api.applyAuth({ ...LOCAL, remote_bind: true, from_loopback: false });
    eq(env(), { ANTHROPIC_BASE_URL: 'http://10.30.0.183:8787', ANTHROPIC_AUTH_TOKEN: TOKEN }, 'a local router, off the machine');
    // At the machine itself: the very line it always was.
    at('http://127.0.0.1:8787');
    api.applyAuth(LOCAL);
    eq(api.connectSnippet(), '{ "env": { "ANTHROPIC_BASE_URL": "http://127.0.0.1:8787" } }', 'a local router at the machine');
  } finally { global.location = was; }
});

check('a hosted router speaks of its deployment, not of this Mac, and a local one as it always did', () => {
  const ids = ['storenote', 'pushnote', 'pwnote', 'e-usage-note', 'a-token-note'];
  const missing = ids.filter((id) => !MARKUP[id]);
  if (missing.length) throw new Error(`not in the markup: #${missing.join(', #')}`);
  // The local words this looks for are really there to be replaced.
  if (!/this Mac/.test(MARKUP.pwnote) || !/Keychain/.test(MARKUP['e-usage-note']) || !/stored locally/.test(MARKUP.storenote)) {
    throw new Error('the local wording changed; this check no longer tests what it says');
  }
  const LOCAL_WORDS = /this Mac|this machine|stored locally|Keychain|~\/\.agent-router|the other\s+account/;
  const builtIn = { id: 'default', label: 'default', source: 'keychain', active: 0 };
  api.applyAuth(HOSTED);
  for (const id of ids) {
    const html = document.getElementById(id).innerHTML;
    const said = LOCAL_WORDS.exec(html);
    if (said || html === MARKUP[id]) throw new Error(`#${id} still says ${said ? `"${said[0]}"` : 'what it says locally'}`);
  }
  if (!/push Secret/.test(document.getElementById('pushnote').innerHTML)) throw new Error('push keys are not said to come from the deployment');
  eq(document.getElementById('remote').title, 'Every caller must give the password', 'the network chip');
  api.openEdit(builtIn);
  const idnote = document.getElementById('e-idnote').textContent;
  if (LOCAL_WORDS.test(idnote) || !/hosted router/.test(idnote)) throw new Error(`an imported built-in account reads: ${idnote}`);
  api.applyAuth(LOCAL);
  for (const id of ids) eq(document.getElementById(id).innerHTML, MARKUP[id], `#${id} on a local router`);
  api.openEdit(builtIn);
  if (!/on this machine/.test(document.getElementById('e-idnote').textContent)) throw new Error('a local built-in account changed its note');
  document.getElementById('editbox').hidden = true;
});

check('a hosted router opens the settings for a Claude Code login only while it is the one in use', () => {
  const dlg = document.getElementById('settings');
  // What a store copied from a Mac brings: the built-in account and a profile, neither of
  // which a hosted router can ever sign with, beside a pasted token that it can.
  const NOT_HERE = 'this account is a Claude Code login on a developer\'s machine, which a hosted router cannot read';
  const builtIn = { id: 'default', label: 'default', source: 'keychain', active: 0, ready: false, detail: NOT_HERE, expires_at: 0 };
  const profile = { id: 'side', label: 'side', source: 'profile', active: 0, ready: false, detail: NOT_HERE, expires_at: 0 };
  const token = { id: 'work', label: 'work', source: 'token', active: 1, ready: true, detail: 'long-lived token', expires_at: 0 };
  const removable = () => /data-del="default"/.test(document.getElementById('accounts').innerHTML);
  // One load opens the settings by itself at most once, so each case that should open
  // them gets a fresh load of its own.
  const opensOnLoad = (list) => {
    dlg.close();
    const page = new Function(js + '\n; return { applyAuth, renderAccounts };')();
    page.applyAuth(HOSTED);
    page.renderAccounts(list);
    const opened = dlg.open;
    dlg.close();
    return opened;
  };
  dlg.close();
  api.applyAuth(HOSTED);
  api.renderAccounts([builtIn, profile, token]);
  if (dlg.open) throw new Error('opened by itself for accounts a hosted router can never sign with');
  // There the built-in one goes like any other; nothing re-creates it.
  if (!removable()) throw new Error('an imported built-in account cannot be removed from a hosted router');
  // A local router still interrupts for it: its own Claude Code login stopped working.
  api.applyAuth(LOCAL);
  api.renderAccounts([builtIn, token]);
  if (!dlg.open) throw new Error('a local router no longer interrupts for its broken built-in account');
  if (removable()) throw new Error('a local router offered to remove its built-in account');
  dlg.close();
  // With nothing else to sign with, or with the copied login still the one in use, a
  // hosted router does interrupt: nothing is signing, which is what the panel can fix.
  if (!opensOnLoad([builtIn])) throw new Error('a hosted router with nothing to sign with stayed quiet');
  if (!opensOnLoad([{ ...builtIn, active: 1 }, { ...token, active: 0 }])) {
    throw new Error('a hosted router signing with a login it cannot read stayed quiet');
  }
  api.applyAuth(LOCAL);
});

if (FORECAST) {
  check('the real /api/forecast payload renders end to end', () => {
    api.renderAllAccounts(FORECAST);
    if (document.getElementById('allacct').hidden) throw new Error('real data must show the panel');
    const active = FORECAST.accounts.find((a) => a.active) || FORECAST.accounts[0];
    api.renderRunway(active);
    for (const a of FORECAST.accounts) {
      for (const key of ['five_hour', 'seven_day']) {
        const st = api.fcState(a[key]);
        if (/undefined|NaN/.test(st.text)) throw new Error(`${a.id}.${key}: ${st.text}`);
      }
    }
  });

  check('two credentials on one quota are named as sharing it', () => {
    const pooled = JSON.parse(JSON.stringify(FORECAST));
    const [a, b] = pooled.accounts;
    b.pool = a.pool;                      // same 7-day reset: one Anthropic account
    api.renderAllAccounts(pooled);
    const cards = document.getElementById('acards');
    const text = JSON.stringify(cards.children).match(/shares a quota with/);
    if (!text) throw new Error('a shared quota must be disclosed, not shown as two allowances');
  });
} else {
  console.log('  skip  real-payload checks (no fixture given)');
}

// Search is the one panel driven by the keyboard rather than the poll, and the one that
// puts router-supplied text on the page — so it is worth proving it renders and escapes.
acheck('a search renders its sessions and its hits', async () => {
  api.qbox.value = 'kestrel';
  await api.runSearch();
  const els = api.searchEls();
  if (els.box.hidden) throw new Error('the results stayed hidden');
  if (!els.sess.children.length) throw new Error('no session rows were built');
  if (els.hits.children.length !== 2) {
    throw new Error(`expected both hits, got ${els.hits.children.length}`);
  }
});

acheck('an empty query puts the panel away rather than searching for nothing', async () => {
  api.qbox.value = '   ';
  await api.runSearch();
  if (!api.searchEls().box.hidden) throw new Error('a blank query left results on screen');
});

acheck('a hosted router\'s facts leave out its process id, and its key hint names the Secret', async () => {
  // The page's own first polls, from every copy of it loaded above, settle first: each
  // applies whatever /api/auth answered, and this check sets its own.
  await new Promise((r) => setImmediate(r));
  const box = document.getElementById('routerinfo');
  const facts = () => box.children.filter((c) => c.class === 'kvline').map((c) => c.children[0].textContent);
  const snippet = () => (box.children.find((c) => c.tagName === 'pre') || {}).textContent;
  const keyHint = () => textOf(document.getElementById('pushbox').children[0]);
  api.applyAuth(LOCAL);
  await api.renderRouterInfo();
  eq(facts(), ['Listening on', 'Version', 'Process'], 'a local router\'s facts');
  eq(snippet(), api.connectSnippet(), 'the snippet a local router shows');
  if (box.children.some((c) => c == null)) throw new Error('a left-out row reached the page as "null"');
  await api.renderPush();
  if (!/~\/\.agent-router/.test(keyHint())) throw new Error(`a local router's key hint: ${keyHint()}`);
  api.applyAuth(HOSTED);
  await api.renderRouterInfo();
  eq(facts(), ['Listening on', 'Version'], 'a hosted router\'s facts');
  if (!snippet().includes('"ANTHROPIC_AUTH_TOKEN": "<the router password>"')) throw new Error(`no password line: ${snippet()}`);
  if (box.children.some((c) => c == null)) throw new Error('a left-out row reached the page as "null"');
  await api.renderPush();
  if (!/push Secret/.test(keyHint()) || /agent-router/.test(keyHint())) throw new Error(`a hosted router's key hint: ${keyHint()}`);
  api.applyAuth(LOCAL);
});

check('a stray marker cannot leave a tag open, and snippet text is escaped', () => {
  const out = api.markup('a \u2039b\u203a c \u2039d');
  const opens = (out.match(/<mark>/g) || []).length;
  const closes = (out.match(/<\/mark>/g) || []).length;
  if (opens !== closes) throw new Error(`unbalanced: ${opens} open, ${closes} close`);
  if (api.markup('<script>x</script>').includes('<script>')) {
    throw new Error('snippet text reached the page as markup');
  }
});

queue.then(() => {
  console.log(failures ? `\n${failures} check(s) failed` : '\nall checks passed');
  process.exit(failures ? 1 : 0);
});
