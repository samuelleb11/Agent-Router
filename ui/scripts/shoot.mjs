#!/usr/bin/env node
// Copied from Northstar ui/scripts/shoot-workmap.mjs at e9c1589.
// Changed: any routes at any widths, rather than the Work map at a desk and a phone; the themes are chosen; a page that scrolls sideways is measured, and fails the run when asked; the theme key is the router's; the Work map's presses, scrolls and measures are gone.
//
// Screenshots of console routes on fixtures, at each width in each theme, from headless Chrome
// driven over the DevTools protocol.
//
//   pnpm shots --routes /,/live,/settings/accounts --out /some/dir/outside/the/repo
//     [--widths 360,390,1440] [--themes white,g100] [--fail-on-overflow]
//     [--unread health,projects] [--base http://127.0.0.1:4392] [--full]
//
// A width under Carbon's md breakpoint (672px) is shot as a phone: touch, a coarse pointer, a
// device scale of 2 and a phone's height; anything wider as a desk.
//
// With no --base it starts the fixture harness itself (vite.harness.config.ts, 127.0.0.1:4392)
// and stops it after; with one it shoots a harness already running there. It never points at a
// router: the harness answers every read and plays every stream, and a read or a stream it
// cannot answer fails the run.
//
// Why each piece is the way it is:
//
// - Over the DevTools protocol, never Chrome's --screenshot flag, which hangs on a page like this
//   one with no file and no error.
// - Device metrics, touch, colour scheme and the remembered theme are set EXPLICITLY before
//   every shot, desk included, and each shot gets a browser context of its own. Emulation
//   persists on a target, and a "desk" shot taken after a phone one is silently a phone shot;
//   storage persists in a context, and the rail remembers whether it is collapsed. Each shot
//   then reads back its width, its theme and its pointer, and the run fails if any is not what
//   was set.
// - Reduced motion, so a transition is never caught halfway.
// - The PNGs go to --out, which must be outside the repository: an untracked file in the tree
//   changes the build's id, and a screenshot is not a source. Without --out, a fresh directory
//   under TMPDIR, named in the output.
// - Chrome's profile is a fresh directory under the system's temporary directory (TMPDIR), and
//   its debugging port is the one it picks, read back from the profile, so two runs never meet.
// - Sideways scroll is measured, not judged by eye: the page is asked to scroll right, and how
//   far it went is the overflow. A phone never scrolls sideways; the elements that stick out
//   are named, so the failure says where to look.
//
// A shot is taken once the page has settled: loaded, fonts in, no Carbon skeleton on screen,
// and no read answered for 600 ms. One that never settles is still taken and says so.
//
// Writes <out>/<name>-<width>-<white|g100>.png for each shot and <out>/shots.json, the account
// of every shot. Exits 1 when a read or a stream went unanswered, the page threw, a shot is not
// what was asked, or, with --fail-on-overflow, a page scrolls sideways.

import { spawn } from 'node:child_process';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const ui = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const repo = resolve(ui, '..');

/// Carbon's md breakpoint: under it the console lays out for a hand (src/lib/viewport.ts).
const NARROW_BELOW = 672;
const THEMES = {
  white: { theme: 'white', choice: 'light', scheme: 'light', html: 'ar-light' },
  g100: { theme: 'g100', choice: 'dark', scheme: 'dark', html: 'ar-dark' },
};
const SETTLE_QUIET_MS = 600;
const SETTLE_LIMIT_MS = 20_000;

function usage(problem) {
  console.error(`shoot: ${problem}`);
  console.error('usage: pnpm shots --routes /,/live [--out DIR] [--widths 360,390,1440] [--themes white,g100] [--fail-on-overflow] [--unread LISTS] [--base URL] [--full]');
  process.exit(2);
}

const list = (value) =>
  String(value)
    .split(',')
    .map((s) => s.trim())
    .filter(Boolean);

function parseArgs(argv) {
  const args = { full: false, failOnOverflow: false, widths: '360,390,1440', themes: 'white,g100' };
  for (let i = 0; i < argv.length; i++) {
    const flag = argv[i];
    if (flag === '--') continue;
    if (flag === '--full') {
      args.full = true;
      continue;
    }
    if (flag === '--fail-on-overflow') {
      args.failOnOverflow = true;
      continue;
    }
    const value = argv[i + 1];
    if (!flag.startsWith('--') || value === undefined) usage(`cannot read ${flag}`);
    const key = flag.slice(2);
    if (!['routes', 'widths', 'themes', 'out', 'unread', 'base'].includes(key)) usage(`unknown flag ${flag}`);
    args[key] = value;
    i += 1;
  }
  if (!args.routes) usage('--routes is required: a comma list of paths, such as /,/live,/settings/accounts');
  args.routes = list(args.routes);
  for (const r of args.routes) if (!r.startsWith('/')) usage(`--routes takes paths, and ${r} is not one`);
  args.widths = list(args.widths).map((w) => {
    const n = Number(w);
    if (!Number.isInteger(n) || n < 200 || n > 4000) usage(`--widths takes CSS pixel widths, and ${w} is not one`);
    return n;
  });
  args.themes = list(args.themes).map((t) => THEMES[t] ?? usage(`--themes takes white and g100, and ${t} is neither`));
  // Without --out the shots go to a fresh directory under the system's temporary directory,
  // outside the repository by construction. That lets a check that only asks "does it
  // overflow?" run as a bare command (the screens' Northstar clauses do); the run names the
  // directory, so the pictures can still be looked at.
  args.out = resolve(args.out ?? mkdtempSync(join(tmpdir(), 'agent-router-shots-')));
  const inside = relative(repo, args.out);
  if (!inside.startsWith('..') && !inside.startsWith('/')) usage(`--out ${args.out} is inside the repository; write screenshots elsewhere`);
  return args;
}

/// A phone under the md breakpoint, a desk above it.
function view(width) {
  if (width < NARROW_BELOW) return { mode: 'phone', width, height: width <= 360 ? 800 : 844, scale: 2, mobile: true };
  return { mode: 'desk', width, height: width < 1056 ? 1024 : 900, scale: 1, mobile: false };
}

/// A route as a file name: `/settings/accounts` is settings-accounts, `/` is root, and a query
/// is spelt out after the path, decoded, so `/?project=%2Fa%2Fb` is root-project-a-b.
const nameOf = (route) => {
  const [path = '', query = ''] = route.split('?');
  const words = (s) => s.replace(/[^A-Za-z0-9]+/g, '-').replace(/^-+|-+$/g, '');
  const named = words(path) || 'root';
  return query ? `${named}-${words(decodeURIComponent(query))}` : named;
};
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/// A DevTools protocol connection to the browser, with flattened sessions for its pages.
class Cdp {
  static async connect(url) {
    const ws = new WebSocket(url);
    await new Promise((ok, fail) => {
      ws.addEventListener('open', ok, { once: true });
      ws.addEventListener('error', () => fail(new Error(`cannot open ${url}`)), { once: true });
    });
    return new Cdp(ws);
  }

  constructor(ws) {
    this.ws = ws;
    this.next = 1;
    this.pending = new Map();
    this.listeners = new Set();
    ws.addEventListener('message', (e) => {
      const msg = JSON.parse(String(e.data));
      if (msg.id !== undefined) {
        const p = this.pending.get(msg.id);
        this.pending.delete(msg.id);
        if (msg.error) p?.fail(new Error(`${p.method}: ${msg.error.message}`));
        else p?.ok(msg.result);
      } else {
        for (const l of this.listeners) l(msg);
      }
    });
  }

  send(method, params = {}, sessionId = undefined) {
    const id = this.next++;
    this.ws.send(JSON.stringify({ id, method, params, ...(sessionId ? { sessionId } : {}) }));
    return new Promise((ok, fail) => this.pending.set(id, { ok, fail, method }));
  }

  on(listener) {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  close() {
    this.ws.close();
  }
}

async function startChrome() {
  const binary = process.env.CHROME ?? '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
  if (!existsSync(binary)) throw new Error(`no Chrome at ${binary}; set CHROME`);
  const profile = mkdtempSync(join(tmpdir(), 'shoot-console-'));
  const child = spawn(
    binary,
    ['--headless=new', '--disable-gpu', '--no-first-run', '--no-default-browser-check', '--hide-scrollbars', `--user-data-dir=${profile}`, '--remote-debugging-port=0', 'about:blank'],
    { stdio: 'ignore' },
  );
  const portFile = join(profile, 'DevToolsActivePort');
  for (let waited = 0; !existsSync(portFile) || readFileSync(portFile, 'utf8').split('\n').length < 2; waited += 100) {
    if (waited > 15_000) throw new Error('Chrome did not open its debugging port within 15 s');
    if (child.exitCode !== null) throw new Error(`Chrome exited with ${child.exitCode} before opening its debugging port`);
    await sleep(100);
  }
  const [port, path] = readFileSync(portFile, 'utf8').split('\n');
  const stop = async () => {
    child.kill();
    await new Promise((r) => (child.exitCode !== null ? r() : child.once('exit', r)));
    rmSync(profile, { recursive: true, force: true });
  };
  return { url: `ws://127.0.0.1:${port}${path}`, stop };
}

async function startHarness() {
  const { createServer } = await import('vite');
  const server = await createServer({ configFile: join(ui, 'vite.harness.config.ts'), root: ui, logLevel: 'warn' });
  await server.listen();
  const base = server.resolvedUrls?.local[0]?.replace(/\/$/, '');
  if (!base) throw new Error('the harness started with no local address');
  return { base, stop: () => server.close() };
}

/// What the page says about itself once it is drawn: what a shot is checked against.
const READ_BACK = `(() => ({
  width: innerWidth,
  height: innerHeight,
  html: document.documentElement.className,
  dark: matchMedia('(prefers-color-scheme: dark)').matches,
  coarse: matchMedia('(pointer: coarse)').matches,
  path: location.pathname + location.search,
  harness: window.__harness ? { unread: window.__harness.unread, answered: window.__harness.answered.length, unanswered: [...window.__harness.unanswered] } : null,
}))()`;

/// How many reads the page has had answered once it looks drawn, or -1 while it does not. A page
/// that has read nothing has not started: the shell reads the router's health and its projects
/// before anything else, and on a cold cache Vite reloads the page once while it optimises
/// dependencies.
const SETTLED = `(() => document.readyState === 'complete' && document.fonts.status === 'loaded'
  && document.getElementById('root')?.childElementCount > 0
  && !document.querySelector('.cds--skeleton, .cds--skeleton__text, .cds--skeleton__placeholder')
  && window.__harness?.answered.length > 0 ? window.__harness.answered.length : -1)()`;

/// Whether the page scrolls sideways, and by how much: it is asked to, and how far it went is
/// the answer. The elements that stick out past the right edge are named, deepest first, so a
/// failure says where to look.
const OVERFLOW = `(() => {
  const root = document.scrollingElement ?? document.documentElement;
  const x = scrollX, y = scrollY;
  scrollTo(1e6, y);
  const by = Math.round(scrollX);
  scrollTo(x, y);
  const edge = root.clientWidth;
  const name = (el) => el.tagName.toLowerCase() + (el.id ? '#' + el.id : '') + [...el.classList].slice(0, 2).map((c) => '.' + c).join('');
  const out = [...document.body.querySelectorAll('*')]
    .filter((el) => { const r = el.getBoundingClientRect(); return r.width > 0 && r.right > edge + 0.5; })
    .filter((el, _, all) => !all.some((other) => other !== el && el.contains(other)))
    .slice(0, 5)
    .map((el) => name(el) + ' to ' + Math.round(el.getBoundingClientRect().right) + 'px');
  return { by, scrollWidth: root.scrollWidth, clientWidth: edge, out };
})()`;

async function shoot(cdp, args, base, route, width, theme) {
  const v = view(width);
  const { browserContextId } = await cdp.send('Target.createBrowserContext');
  const { targetId } = await cdp.send('Target.createTarget', { url: 'about:blank', browserContextId });
  const { sessionId } = await cdp.send('Target.attachToTarget', { targetId, flatten: true });
  const send = (method, params) => cdp.send(method, params, sessionId);
  const thrown = [];
  const consoleErrors = [];
  const off = cdp.on((msg) => {
    if (msg.sessionId !== sessionId) return;
    if (msg.method === 'Runtime.exceptionThrown') thrown.push(msg.params.exceptionDetails.exception?.description ?? msg.params.exceptionDetails.text);
    if (msg.method === 'Runtime.consoleAPICalled' && msg.params.type === 'error') {
      consoleErrors.push(msg.params.args.map((a) => a.value ?? a.description ?? '').join(' '));
    }
  });
  try {
    await send('Page.enable');
    await send('Runtime.enable');
    await send('Emulation.setDeviceMetricsOverride', { width: v.width, height: v.height, deviceScaleFactor: v.scale, mobile: v.mobile });
    await send('Emulation.setTouchEmulationEnabled', v.mobile ? { enabled: true, maxTouchPoints: 5 } : { enabled: false });
    await send('Emulation.setEmulatedMedia', {
      features: [
        { name: 'prefers-color-scheme', value: theme.scheme },
        { name: 'prefers-reduced-motion', value: 'reduce' },
      ],
    });
    // The console's own remembered choice, which wins over the scheme: both say the same.
    await send('Page.addScriptToEvaluateOnNewDocument', {
      source: `try { localStorage.setItem('agent-router.theme', ${JSON.stringify(theme.choice)}); } catch (e) {}`,
    });

    const url = new URL(route, `${base}/`);
    if (args.unread) url.searchParams.set('unread', args.unread);
    const loaded = new Promise((r) => {
      const stop = cdp.on((msg) => msg.sessionId === sessionId && msg.method === 'Page.loadEventFired' && (stop(), r()));
    });
    await send('Page.navigate', { url: url.href });
    await loaded;

    const evaluate = async (expression) => (await send('Runtime.evaluate', { expression, returnByValue: true })).result.value;
    let settled = false;
    let last = -1;
    let quietSince = Date.now();
    for (const start = Date.now(); Date.now() - start < SETTLE_LIMIT_MS; await sleep(100)) {
      const answered = await evaluate(SETTLED);
      if (answered !== last || answered <= 0) {
        last = answered;
        quietSince = Date.now();
      } else if (Date.now() - quietSince >= SETTLE_QUIET_MS) {
        settled = true;
        break;
      }
    }

    const page = await evaluate(READ_BACK);
    page.overflow = await evaluate(OVERFLOW);
    const { data } = await send('Page.captureScreenshot', { format: 'png', captureBeyondViewport: args.full });
    const file = join(args.out, `${nameOf(route)}-${width}-${theme.theme}.png`);
    writeFileSync(file, Buffer.from(data, 'base64'));

    const wrong = [];
    if (page.width !== v.width) wrong.push(`width ${page.width}, not ${v.width}`);
    if (!page.html.split(/\s+/).includes(theme.html)) wrong.push(`<html class="${page.html}">, not ${theme.html}`);
    if (page.dark !== (theme.scheme === 'dark')) wrong.push(`prefers-color-scheme is not ${theme.scheme}`);
    if (page.coarse !== v.mobile) wrong.push(`pointer is ${page.coarse ? 'coarse' : 'fine'}`);
    if (!page.harness) wrong.push('no fixture harness on the page: is --base a harness?');
    if (page.harness?.answered === 0) wrong.push('the page read nothing: it was shot before the console started');
    if (page.harness?.unanswered.length) wrong.push(`unanswered: ${page.harness.unanswered.join('; ')}`);
    if (thrown.length) wrong.push(`threw: ${thrown.join('; ')}`);
    const sideways = page.overflow.by > 0 || page.overflow.scrollWidth > page.overflow.clientWidth;
    const overflowNote = sideways
      ? `scrolls sideways by ${Math.max(page.overflow.by, page.overflow.scrollWidth - page.overflow.clientWidth)}px${page.overflow.out.length ? `: ${page.overflow.out.join(', ')}` : ''}`
      : null;
    if (sideways && args.failOnOverflow) wrong.push(overflowNote);
    return { file, route, url: url.href, width, mode: v.mode, theme: theme.theme, settled, page, overflow: overflowNote, consoleErrors, wrong };
  } finally {
    off();
    await cdp.send('Target.closeTarget', { targetId }).catch(() => {});
    await cdp.send('Target.disposeBrowserContext', { browserContextId }).catch(() => {});
  }
}

const args = parseArgs(process.argv.slice(2));
mkdirSync(args.out, { recursive: true });
const harness = args.base ? { base: args.base.replace(/\/$/, ''), stop: async () => {} } : await startHarness();
const shots = [];
try {
  const chrome = await startChrome();
  try {
    const cdp = await Cdp.connect(chrome.url);
    try {
      for (const route of args.routes) {
        for (const width of args.widths) {
          for (const theme of args.themes) shots.push(await shoot(cdp, args, harness.base, route, width, theme));
        }
      }
    } finally {
      cdp.close();
    }
  } finally {
    await chrome.stop();
  }
} finally {
  await harness.stop();
}

writeFileSync(join(args.out, 'shots.json'), `${JSON.stringify({ base: harness.base, failOnOverflow: args.failOnOverflow, shots }, null, 2)}\n`);
for (const s of shots) {
  const notes = [s.settled ? null : 'NOT SETTLED', ...s.wrong, s.wrong.includes(s.overflow) ? null : s.overflow, ...s.consoleErrors.map((e) => `console.error: ${e.slice(0, 200)}`)].filter(Boolean);
  console.log(`${s.wrong.length ? 'FAIL' : 'ok  '} ${s.file}${notes.length ? `\n       ${notes.join('\n       ')}` : ''}`);
}
process.exit(shots.some((s) => s.wrong.length) ? 1 : 0);
