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
const check = (name, fn) => {
  try { fn(); console.log(`  ok    ${name}`); }
  catch (e) { failures++; console.log(`  FAIL  ${name}\n        ${e.stack.split('\n').slice(0, 3).join('\n        ')}`); }
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
    appendChild: (c) => (el.children.push(c), c),
    replaceChildren: (...c) => { el.children.length = 0; el.children.push(...c); },
    querySelector: () => mkEl(), querySelectorAll: () => [],
    getBoundingClientRect: () => ({ left: 0, top: 0, right: 900, bottom: 300, width: 900, height: 300 }),
    setAttribute(k, v) { el[k] = v; }, getAttribute: (k) => el[k], removeAttribute() {},
    addEventListener() {}, removeEventListener() {}, closest: () => mkEl(),
    after() {}, remove() {}, focus() {}, blur() {}, scrollIntoView() {},
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
global.location = { href: 'http://host/', hash: '' };
global.history = { pushState() {}, replaceState() {} };
global.ResizeObserver = class { observe() {} unobserve() {} disconnect() {} };
global.matchMedia = () => ({ matches: false, addEventListener() {} });
global.requestAnimationFrame = (f) => setTimeout(f, 0);
global.cancelAnimationFrame = () => {};
global.getComputedStyle = () => ({ lineHeight: '18' });
global.setInterval = () => 0;
global.setTimeout = ((f, ms) => (ms > 50 ? 0 : setTimeout(f, 0)));
global.addEventListener = () => {};
global.fetch = (url = '') => Promise.resolve({
  ok: true,
  json: () => Promise.resolve(/\/api\/(projects|sessions|requests|tools|accounts)/.test(String(url)) ? [] : {}),
});

const load = (f, d) => {
  try { return JSON.parse(fs.readFileSync(f, 'utf8')); } catch { return d; }
};
const FORECAST = load(process.argv[2] || path.join(root, 'scripts', 'fixtures', 'forecast.json'), null);

check('script evaluates', () => { new Function(js)(); });

const api = new Function(
  js + '\n; return { fcState, fcBasis, fcLine, runwaySvg, renderRunway, renderAllAccounts, hrs, applyAuth, pwForm };'
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

console.log(failures ? `\n${failures} check(s) failed` : '\nall checks passed');
process.exit(failures ? 1 : 0);
