// @vitest-environment node
// New for agent-router: the checks of scripts/check-dashboard-ui.js, one spec each, in its order,
// over the modules underneath the old page. The old script drove the page's inline script against
// a fake DOM; these call the model functions the page's renderers stood on. Where a check is about
// drawing that has no screen yet, its comment names the slice that pins the drawing: AGEN-0010 for
// the Overview, AGEN-0011 for Settings. They run in Node, with no DOM at all.
import { describe, expect, it } from 'vitest';
import forecastText from './fixtures/forecast.json?raw';
import { hosted as hostedAuth, local as localAuth, search as searchReply, windows } from './fixtures/dashboard';
import { nonsense } from './fixtures/walk';
import { hours, marks } from './format';
import { allAccounts, burnDown, burnDowns, fcBasis, fcLine, fcState, type Forecast, type Window } from './forecast';
import { UNSCOPED } from './scope';
import { searchIdle, searchParams, searchView } from './search';
import {
  accountSources,
  addForm,
  asksFirstPassword,
  connectSnippet,
  identityNote,
  networkStatus,
  noteText,
  notes,
  opensSettings,
  passwordForm,
  pushKeyHint,
  readsSetup,
  removable,
  routerFacts,
  settingsPages,
  type Account,
  type NoteId,
} from './settings';

/// The moment every spec here reads as now.
const NOW = Date.UTC(2026, 9, 5, 12, 0, 0);
const W = windows(NOW);
const win = (name: string): Window => {
  const w = W[name];
  if (!w) throw new Error(`no window called ${name}`);
  return w;
};
const HOSTED = hostedAuth(NOW);
const LOCAL = localAuth(NOW);
const FORECAST = JSON.parse(forecastText) as Forecast;

/// Every model module's source, for the checks about the code itself.
const SOURCES = import.meta.glob(['./*.ts', '!./*.spec.ts'], { query: '?raw', import: 'default', eager: true }) as Record<string, string>;
/// The browser globals the console uses, which no name of its own may shadow (SH-C28).
const GLOBALS = ['history', 'location', 'name', 'status', 'top', 'self', 'parent', 'origin', 'screen', 'event', 'find'];

describe('the dashboard, as scripts/check-dashboard-ui.js checked it', () => {
  // ported from scripts/check-dashboard-ui.js: script evaluates
  it('OV-C211: script evaluates — every model module loads and runs without a browser', async () => {
    expect(typeof document).toBe('undefined');
    const modules = import.meta.glob(['./*.ts', '!./*.spec.ts']);
    expect(Object.keys(modules)).toEqual(expect.arrayContaining(['./format.ts', './forecast.ts', './markdown.ts', './scope.ts', './session.ts']));
    for (const [path, load] of Object.entries(modules)) {
      const mod = (await load()) as Record<string, unknown>;
      expect(Object.keys(mod).length, path).toBeGreaterThan(0);
    }
  });

  // ported from scripts/check-dashboard-ui.js: a local does not shadow a browser global the page uses
  it('OV-C212, SH-C28: a local does not shadow a browser global the page uses', () => {
    expect(Object.keys(SOURCES).length).toBeGreaterThanOrEqual(5);
    for (const [path, src] of Object.entries(SOURCES)) {
      const declared = new Set([...src.matchAll(/(?:let|const|var|function|class)\s+([A-Za-z_$][\w$]*)/g)].map((m) => m[1]));
      for (const m of src.matchAll(/import\s+(?:type\s+)?\{([^}]*)\}/g)) {
        for (const name of (m[1] ?? '').split(',')) declared.add(name.trim().split(/\s+as\s+/).pop()?.replace(/^type\s+/, ''));
      }
      expect(
        GLOBALS.filter((g) => declared.has(g)),
        `${path} shadows a browser global — see the \`history\` bug that stopped cards opening`,
      ).toEqual([]);
    }
  });

  // ported from scripts/check-dashboard-ui.js: every window state produces a line, and only the real trouble is red
  it('OV-C68, OV-C75: every window state produces a line, and only the real trouble is red', () => {
    for (const [name, w] of Object.entries(W)) {
      const st = fcState(w, NOW);
      expect(st.text, name).not.toBe('');
      expect(st.text, name).not.toMatch(/undefined|NaN|Infinity/);
      // Red is for a crossing that survives even a quiet week. A verdict of `tight` has an
      // exhausts_at too, and must not shout about it; a payload with no verdict at all is an older
      // router, and falls back to the boolean it does send.
      const shouldShout = !!w.exhausts_at && w.verdict !== 'tight';
      expect(st.tone === 'bad', `${name}: tone ${st.tone} for exhausts_at=${w.exhausts_at} verdict=${w.verdict}`).toBe(shouldShout);
      expect(fcLine(w, NOW)?.text).toBe(st.text);
    }
    // With no forecast at all, the meter above is still showing a real percentage from the response
    // headers, so a line reading "no reading yet" would contradict it. Say nothing instead.
    expect(fcLine(null, NOW)).toBeNull();
    expect(fcLine(undefined, NOW)).toBeNull();
    expect(fcState(null, NOW).tone, 'an absent window must not be alarming').toBe('plain');
  });

  // ported from scripts/check-dashboard-ui.js: a straddling band is amber and says so, and the band is disclosed
  it('OV-C69, OV-C78: a straddling band is amber and says so, and the band is disclosed', () => {
    const st = fcState(win('too close to call'), NOW);
    expect(st.tone, 'a coin flip must not be red').toBe('warn');
    expect(st.text).toMatch(/too close to call/);
    expect(st.text, 'the quiet end is the reassuring half').toMatch(/quieter week it ends ≈88%/);
    expect(fcBasis(win('too close to call')), 'the spread is why the wording hedges').toMatch(/25–41% across your days/);
    // A window with no band discloses one number, not a range of one.
    expect(fcBasis(win('resets before it fills')), 'no spread, nothing to disclose').not.toMatch(/across your days/);
  });

  // ported from scripts/check-dashboard-ui.js: a router that predates the band still renders as it always did
  it('OV-C71, OV-C83: a router that predates the band still renders as it always did', () => {
    for (const k of ['runs out before the reset', 'already at the limit']) expect(fcState(win(k), NOW).tone, `${k} lost its warning`).toBe('bad');
    expect(fcState(win('resets before it fills'), NOW).tone, 'a calm window went red').not.toBe('bad');
    // And the burn-down draws a ray, not a cone, when there is no band to draw. The SVG itself is
    // the BurnDown component's, pinned with the Overview in AGEN-0010.
    expect(burnDown(win('runs out before the reset'), 300, NOW)?.cone, 'drew an uncertainty cone with no uncertainty to show').toBeNull();
    expect(burnDown(win('too close to call'), 300, NOW)?.cone, 'a straddling band must show its cone').not.toBeNull();
  });

  // ported from scripts/check-dashboard-ui.js: the basis is disclosed, and never claims a fit it did not make
  it('OV-C76, OV-C77: the basis is disclosed, and never claims a fit it did not make', () => {
    expect(fcBasis(win('resets before it fills'))).toMatch(/fitted on/);
    const borrowed = fcBasis(win('too young to have a slope'));
    expect(borrowed, 'a borrowed rate must not claim a fit').not.toMatch(/fitted on/);
    expect(borrowed, 'the fallback says so').toMatch(/recent average/);
    for (const k of ['nothing to go on', 'rolled over while we were not looking']) expect(fcBasis(win(k)), `${k} has no basis to disclose`).toBeNull();
  });

  // ported from scripts/check-dashboard-ui.js: hours read as hours
  it('OV-C79: hours read as hours', () => {
    const cases: [number | null, string][] = [[0.25, '15 min'], [2.4, '2.4 h'], [9.96, '10.0 h'], [81, '81 h'], [null, '—']];
    for (const [v, want] of cases) expect(hours(v), `hours(${v})`).toBe(want);
  });

  // ported from scripts/check-dashboard-ui.js: the burn-down chart draws for every state that has a window
  it('OV-C81, OV-C84: the burn-down chart draws for every state that has a window', () => {
    // The geometry the BurnDown component draws; its SVG is pinned with the Overview in AGEN-0010.
    for (const [name, w] of Object.entries(W)) {
      if (w.expired) continue;
      const chart = burnDown(w, 300, NOW);
      expect(chart, `${name}: nothing drawn`).not.toBeNull();
      if (!chart) continue;
      expect(nonsense(chart), name).toEqual([]);
      // The projection ray is the point of the chart; it is always drawn, and only a real crossing
      // gets the marker.
      expect(chart.projection.to, `${name}: no projection drawn`).toBeDefined();
      expect(chart.marker !== null, `${name}: crossing marker disagrees with exhausts_at`).toBe(!!w.exhausts_at);
    }
    // A window with no span at all is refused rather than divided by zero.
    expect(burnDown({ window_start: 10, reset_at: 10 }, 300, NOW), 'a zero-length window must not draw').toBeNull();
  });

  // ported from scripts/check-dashboard-ui.js: the runway panel survives an account with nothing current
  it('OV-C84: the runway panel survives an account with nothing current', () => {
    expect(burnDowns(null)).toEqual([]);
    expect(burnDowns({ five_hour: win('rolled over while we were not looking'), seven_day: null })).toEqual([]);
    const both = burnDowns({ five_hour: win('resets before it fills'), seven_day: win('runs out before the reset') });
    expect(both.map((b) => b.label)).toEqual(['5-hour window', '7-day window']);
    for (const b of both) expect(burnDown(b.window, 300, NOW)).not.toBeNull();
  });

  // ported from scripts/check-dashboard-ui.js: the all-accounts panel renders, and hides itself when there is nothing
  it('OV-C86: the all-accounts panel renders, and hides itself when there is nothing', () => {
    expect(allAccounts(null, NOW), 'no data leaves the panel hidden').toBeNull();
    expect(allAccounts({ accounts: [], global: {} }, NOW), 'an empty list leaves the panel hidden').toBeNull();
  });

  // ported from scripts/check-dashboard-ui.js: the password is asked for once, on first boot, from this machine only
  it('ST-C80, SH-C18: the password is asked for once, on first boot, from this machine only', () => {
    // The prompt and the topbar's network status are drawn by Settings, AGEN-0011.
    const shown = { dismissed: false, settingsOpen: false };
    // Loopback-only bind: nothing to set up, nothing to show.
    const quiet = { configured: false, remote_bind: false, from_loopback: true };
    expect(asksFirstPassword(quiet, shown), 'no network bind, no prompt').toBe(false);
    expect(networkStatus(quiet), 'the status only matters when the network can reach us').toBeNull();
    // A remote viewer never sees the setup prompt: it cannot set one anyway.
    const remote = { configured: false, remote_bind: true, from_loopback: false };
    expect(asksFirstPassword(remote, shown), 'the first-boot prompt must not be offered off-host').toBe(false);
    expect(networkStatus(remote)?.text, 'the status says the network is unguarded').toMatch(/no password/);
    // First boot at the machine itself: ask.
    expect(asksFirstPassword({ configured: false, remote_bind: true, from_loopback: true }, shown), 'first boot on a network bind must ask').toBe(true);
    // Asked once: dismissed, or under Settings, it stays shut.
    expect(asksFirstPassword({ configured: false, remote_bind: true, from_loopback: true }, { dismissed: true, settingsOpen: false })).toBe(false);
    expect(asksFirstPassword({ configured: false, remote_bind: true, from_loopback: true }, { dismissed: false, settingsOpen: true })).toBe(false);
    // Once set: the status says so, and the prompt stays shut.
    const set = { configured: true, remote_bind: true, from_loopback: true, updated_at: NOW - 60000 };
    expect(asksFirstPassword(set, shown), 'a configured router must not ask again').toBe(false);
    expect(networkStatus(set)?.text, 'the status says the network is guarded').toMatch(/password set/);
    expect(networkStatus(set)?.tone, 'a set password is not an alarm').toBe('ok');
  });

  // ported from scripts/check-dashboard-ui.js: the password form builds in both homes without throwing
  it('ST-C73: the password form builds in both homes without throwing', () => {
    // The form's fields are Settings', AGEN-0011.
    expect(passwordForm(LOCAL, true)).toEqual({ save: 'Set password', secondary: 'Not now' });
    expect(passwordForm(LOCAL, false)).toEqual({ save: 'Change password', secondary: 'Remove password' });
    expect(passwordForm(null, false)).toEqual({ save: 'Set password', secondary: null });
  });

  // ported from scripts/check-dashboard-ui.js: a hosted router hides what only a Mac has, and a local one keeps all of it
  it('OV-C204, ST-C53, ST-C72: a hosted router hides what only a Mac has, and a local one keeps all of it', () => {
    // The old check also proved the markup wrapped each section; here a section is a Settings page,
    // and which pages exist is Settings' to draw, AGEN-0011.
    const machineOnly = (pages: string[]) => pages.filter((p) => p === 'claude-code' || p === 'network');
    expect(machineOnly(settingsPages(LOCAL)), 'what a local router shows').toEqual(['claude-code', 'network']);
    expect(machineOnly(settingsPages(HOSTED)), 'what a hosted router shows').toEqual([]);
    // Nothing is drawn into a page that is not there either: the setup and wrapper state go unasked.
    expect(readsSetup(HOSTED), 'a hosted router was asked for its setup').toBe(false);
    expect(machineOnly(settingsPages(LOCAL)), 'what a local router shows once more').toEqual(['claude-code', 'network']);
    expect(readsSetup(LOCAL), 'a local router stopped reading its setup').toBe(true);
  });

  // ported from scripts/check-dashboard-ui.js: a hosted router adds an account by pasted token only, and a local one every way
  it('OV-C205, ST-C40, ST-C52: a hosted router adds an account by pasted token only, and a local one every way', () => {
    const form = (auth: typeof LOCAL) => {
      const [first] = accountSources(auth);
      if (!first) throw new Error('nothing offered');
      const f = addForm(first);
      return { type: first, token: f.token, signin: f.signin, save: f.save };
    };
    // Anthropic's ways: Kimi Code is a provider, asked for before them on every router (ST-C124).
    expect(accountSources(LOCAL), 'what a local router offers').toEqual(['signin', 'token', 'profile']);
    expect(accountSources(HOSTED), 'what a hosted router offers').toEqual(['token']);
    expect(form(HOSTED), 'the form on a hosted router').toEqual({ type: 'token', token: true, signin: false, save: 'Add account' });
    expect(accountSources(LOCAL), 'what a local router offers once more').toEqual(['signin', 'token', 'profile']);
    expect(form(LOCAL), 'the form on a local router').toEqual({ type: 'signin', token: false, signin: true, save: 'Start sign-in' });
  });

  // ported from scripts/check-dashboard-ui.js: a pinned password is the deployment's: said so, with nothing to set, change or remove
  it("OV-C206, ST-C79: a pinned password is the deployment's: said so, with nothing to set, change or remove", () => {
    const shown = { dismissed: false, settingsOpen: false };
    expect(passwordForm(HOSTED, false), 'the password section').toEqual({ pinned: 'Set by the deployment. Change the Secret and restart the router.' });
    // Not even with what would ask for one at the machine itself...
    expect(asksFirstPassword({ ...HOSTED, configured: false, from_loopback: true }, shown), 'a hosted router asked for a first password').toBe(false);
    // ...which a local router does ask with: the mode is what keeps it shut.
    expect(asksFirstPassword({ ...LOCAL, configured: false, remote_bind: true, from_loopback: true }, shown), 'a local router no longer asks on first boot').toBe(true);
    expect(passwordForm(LOCAL, false), 'a local router lost its password form').toEqual({ save: 'Change password', secondary: 'Remove password' });
  });

  // ported from scripts/check-dashboard-ui.js: the connection snippet is the page's own origin, with the password wherever it is asked
  it("OV-C207, ST-C96: the connection snippet is the page's own origin, with the password wherever it is asked", () => {
    const env = (origin: string, auth: typeof LOCAL) => (JSON.parse(connectSnippet(origin, auth)) as { env: unknown }).env;
    const TOKEN = '<Client access token or router password>';
    expect(env('https://router.example', HOSTED), 'a hosted router').toEqual({ ANTHROPIC_BASE_URL: 'https://router.example', ANTHROPIC_AUTH_TOKEN: TOKEN });
    // Every caller of a hosted router gives it, a kubectl port-forward to localhost included.
    expect(env('http://localhost:8787', { ...HOSTED, from_loopback: true }), 'through a port-forward').toEqual({ ANTHROPIC_BASE_URL: 'http://localhost:8787', ANTHROPIC_AUTH_TOKEN: TOKEN });
    // A local router read from another machine on the network.
    expect(env('http://192.0.2.10:8787', { ...LOCAL, remote_bind: true, from_loopback: false }), 'a local router, off the machine').toEqual({
      ANTHROPIC_BASE_URL: 'http://192.0.2.10:8787',
      ANTHROPIC_AUTH_TOKEN: TOKEN,
    });
    // At the machine itself: the very line it always was, and the same before /api/auth answers.
    expect(connectSnippet('http://127.0.0.1:8787', LOCAL), 'a local router at the machine').toBe('{ "env": { "ANTHROPIC_BASE_URL": "http://127.0.0.1:8787" } }');
    expect(connectSnippet('http://127.0.0.1:8787', null)).toBe('{ "env": { "ANTHROPIC_BASE_URL": "http://127.0.0.1:8787" } }');
  });

  // ported from scripts/check-dashboard-ui.js: a hosted router speaks of its deployment, not of this Mac, and a local one as it always did
  it('ST-C102, SH-C19, ST-C31: a hosted router speaks of its deployment, not of this Mac, and a local one as it always did', () => {
    const ids: NoteId[] = ['store', 'push', 'password', 'usageKey', 'token'];
    const localNotes = notes(LOCAL);
    // The local words this looks for are really there to be replaced.
    expect(noteText(localNotes.password)).toMatch(/this Mac/);
    expect(noteText(localNotes.usageKey)).toMatch(/Keychain/);
    expect(noteText(localNotes.store)).toMatch(/stored locally/);
    const LOCAL_WORDS = /this Mac|this machine|stored locally|Keychain|~\/\.agent-router|the other\s+account/;
    const builtIn = { source: 'keychain' };
    const hostedNotes = notes(HOSTED);
    for (const id of ids) {
      const said = noteText(hostedNotes[id]);
      expect(said, `${id} still says what it says locally`).not.toBe(noteText(localNotes[id]));
      expect(said, `${id} speaks of this Mac`).not.toMatch(LOCAL_WORDS);
    }
    expect(noteText(hostedNotes.push), 'push keys are said to come from the deployment').toMatch(/push Secret/);
    expect(networkStatus(HOSTED)?.explanation, 'the network status').toBe('Every caller must give the password');
    const hostedId = identityNote(builtIn, HOSTED);
    expect(hostedId, 'an imported built-in account').not.toMatch(LOCAL_WORDS);
    expect(hostedId).toMatch(/hosted router/);
    // A local router gets its own words back exactly. The notes are drawn by Settings, AGEN-0011.
    for (const id of ids) expect(noteText(notes(LOCAL)[id]), `${id} on a local router`).toBe(noteText(localNotes[id]));
    expect(identityNote(builtIn, LOCAL), 'a local built-in account changed its note').toMatch(/on this machine/);
  });

  // ported from scripts/check-dashboard-ui.js: a hosted router opens the settings for a Claude Code login only while it is the one in use
  it('ST-C7, ST-C8, ST-C26, OV-C209: a hosted router opens the settings for a Claude Code login only while it is the one in use', () => {
    // What a store copied from a Mac brings: the built-in account and a profile, neither of which a
    // hosted router can ever sign with, beside a pasted token that it can.
    const NOT_HERE = "this account is a Claude Code login on a developer's machine, which a hosted router cannot read";
    const builtIn: Account = { id: 'default', label: 'default', source: 'keychain', active: 0, ready: false, detail: NOT_HERE, expires_at: 0 };
    const profile: Account = { id: 'side', label: 'side', source: 'profile', active: 0, ready: false, detail: NOT_HERE, expires_at: 0 };
    const token: Account = { id: 'work', label: 'work', source: 'token', active: 1, ready: true, detail: 'long-lived token', expires_at: 0 };
    expect(opensSettings([builtIn, profile, token], HOSTED), 'opened by itself for accounts a hosted router can never sign with').toBe(false);
    // There the built-in one goes like any other; nothing re-creates it.
    expect(removable(builtIn, HOSTED), 'an imported built-in account cannot be removed from a hosted router').toBe(true);
    // A local router still interrupts for it: its own Claude Code login stopped working.
    expect(opensSettings([builtIn, token], LOCAL), 'a local router no longer interrupts for its broken built-in account').toBe(true);
    expect(removable(builtIn, LOCAL), 'a local router offered to remove its built-in account').toBe(false);
    // With nothing else to sign with, or with the copied login still the one in use, a hosted router
    // does interrupt: nothing is signing, which is what Settings can fix. Once per load is the
    // screen's to keep, AGEN-0011.
    expect(opensSettings([builtIn], HOSTED), 'a hosted router with nothing to sign with stayed quiet').toBe(true);
    expect(opensSettings([{ ...builtIn, active: 1 }, { ...token, active: 0 }], HOSTED), 'a hosted router signing with a login it cannot read stayed quiet').toBe(true);
  });

  // ported from scripts/check-dashboard-ui.js: the real /api/forecast payload renders end to end
  it('OV-C97: the real /api/forecast payload renders end to end', () => {
    // The captured payload, copied from scripts/fixtures/forecast.json; drawn with the Overview,
    // AGEN-0010.
    const panel = allAccounts(FORECAST, NOW);
    expect(panel, 'real data must show the panel').not.toBeNull();
    expect(nonsense(panel)).toEqual([]);
    const accounts = FORECAST.accounts ?? [];
    const active = accounts.find((a) => a.active) ?? accounts[0];
    for (const b of burnDowns(active)) expect(burnDown(b.window, 300, NOW)).not.toBeNull();
    for (const a of accounts) {
      for (const key of ['five_hour', 'seven_day'] as const) expect(fcState(a[key], NOW).text, `${a.id}.${key}`).not.toMatch(/undefined|NaN/);
    }
  });

  // ported from scripts/check-dashboard-ui.js: two credentials on one quota are named as sharing it
  it('OV-C95: two credentials on one quota are named as sharing it', () => {
    const [a, b, ...rest] = FORECAST.accounts ?? [];
    if (!a || !b) throw new Error('the fixture holds fewer than two accounts');
    // The same 7-day reset: one Anthropic account.
    const pooled: Forecast = { ...FORECAST, accounts: [a, { ...b, pool: a.pool }, ...rest] };
    const cards = allAccounts(pooled, NOW)?.cards ?? [];
    expect(cards.filter((c) => /shares a quota with/.test(c.sub)).length, 'a shared quota must be disclosed, not shown as two allowances').toBe(2);
    expect(cards[0]?.sub).toMatch(new RegExp(`shares a quota with ${b.label}`));
  });

  // ported from scripts/check-dashboard-ui.js: a search renders its sessions and its hits
  it('OV-C46: a search renders its sessions and its hits', () => {
    // Search is the one panel driven by the keyboard rather than the poll, and the one that puts
    // router-supplied text on the page. Its rows are drawn with the Overview, AGEN-0010.
    expect(searchParams('kestrel', '', UNSCOPED, NOW)).toEqual({ q: 'kestrel' });
    const view = searchView(searchReply(NOW), 'kestrel', UNSCOPED, 12, NOW);
    expect(view.sessions.length, 'no session rows were built').toBe(1);
    expect(view.hits.length, 'expected both hits').toBe(2);
    expect(view.empty).toBeNull();
    expect(nonsense(view)).toEqual([]);
  });

  // ported from scripts/check-dashboard-ui.js: an empty query puts the panel away rather than searching for nothing
  it('OV-C40: an empty query puts the panel away rather than searching for nothing', () => {
    expect(searchParams('   ', '', UNSCOPED, NOW), 'a blank query searched').toBeNull();
    expect(searchIdle(UNSCOPED)).toBe('every message, tool call and result ever captured');
  });

  // ported from scripts/check-dashboard-ui.js: a hosted router's facts leave out its process id, and its key hint names the Secret
  it("OV-C210, ST-C95, ST-C86: a hosted router's facts leave out its process id, and its key hint names the Secret", () => {
    // What /_router/health and /api/push answered in the old check: nothing, and no key.
    const health = {};
    const push = { configured: false };
    const facts = (auth: typeof LOCAL) => routerFacts(health, auth, 'host').map((f) => f.label);
    expect(facts(LOCAL), "a local router's facts").toEqual(['Listening on', 'Version', 'Process']);
    expect(connectSnippet('http://host', LOCAL)).toBe('{ "env": { "ANTHROPIC_BASE_URL": "http://host" } }');
    expect(routerFacts(health, LOCAL, 'host').some((f) => f.value == null || f.value === 'null'), 'a left-out row reached the page as "null"').toBe(false);
    expect(pushKeyHint(push, LOCAL), "a local router's key hint").toMatch(/~\/\.agent-router/);
    expect(facts(HOSTED), "a hosted router's facts").toEqual(['Listening on', 'Version']);
    expect(connectSnippet('http://host', HOSTED)).toContain('"ANTHROPIC_AUTH_TOKEN": "<Client access token or router password>"');
    expect(routerFacts(health, HOSTED, 'host').some((f) => f.value == null || f.value === 'null'), 'a left-out row reached the page as "null"').toBe(false);
    expect(pushKeyHint(push, HOSTED), "a hosted router's key hint").toMatch(/push Secret/);
    expect(pushKeyHint(push, HOSTED)).not.toMatch(/agent-router/);
  });

  // ported from scripts/check-dashboard-ui.js: a stray marker cannot leave a tag open, and snippet text is escaped
  it('OV-C49, SH-C26: a stray marker cannot leave a tag open, and snippet text is escaped', () => {
    // A snippet is runs of text, never markup: a stray marker can only shift which run is marked.
    const out = marks('a ‹b› c ‹d');
    expect(out).toEqual([
      { text: 'a ', mark: false },
      { text: 'b', mark: true },
      { text: ' c ', mark: false },
      { text: 'd', mark: true },
    ]);
    expect(out.map((r) => r.text).join(''), 'no text was dropped').toBe('a b c d');
    expect(marks('<script>x</script>'), 'snippet text reached the page as markup').toEqual([{ text: '<script>x</script>', mark: false }]);
  });
});
