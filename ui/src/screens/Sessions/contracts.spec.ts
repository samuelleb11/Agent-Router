// New for agent-router: the Sessions section's contracts about its own checks — that the old page's
// checks are ported, that the specs wait for what they assert and run one after another, that a
// rejection nobody awaited fails the run — and the safe-area rule, which only the stylesheet can be
// asked about here (jsdom has no notch).
import { existsSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { act, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { FakeEventSource } from './fakes';
import { A, json, renderSessions, streamOf, stubRouter } from './kit';

const UI = join(__dirname, '../../..');
const REPO = join(UI, '..');

/// The checks scripts/check-live-ui.js held, in its order, as they stood when they were ported. The
/// list outlives the script: the clean cut deletes it (decision J), and what was ported stays ported.
const PORTED = [
  'script evaluates',
  'a local does not shadow a browser global the page uses',
  'opening a session does not throw',
  'going back to the list does not throw',
  'notices do not recurse',
  'markdown survives hostile input',
  'the session list renders every state',
  'tabs cover the project and keep creation order',
  'sessions from two machines say which, and one project on each stays apart',
  'archived sessions leave the tabs, but come back when live',
  'stale sessions do not clutter the tabs',
  'every activity and tone renders, and a word the page does not know is plain gray',
  'a closed session is a hollow ring, says when it ended, and is never counted',
  'a session busy or waiting on you is never folded into "older"',
  'the tab dots follow the list while a session is open',
  'a dropped stream leaves the header stale, not showing its last word',
  'a stream silent for ten seconds is taken for dead: stale, no Stop, and reconnected',
  'the browser reconnecting by itself does not show the tail twice',
  'a project heading shows its most pressing session, ranked as on the phones',
  "each row says what it is doing and for how long, on the router's clock",
  'Stop is offered for a running turn only — never for background work alone',
  'an answer names the prompt it answers',
  'a payload from an older router renders as it always did',
  'the wire, replayed: every row renders and every status reads cleanly',
  'every captured event renders, statuses included',
  'find opens from the list and from a session, and closes back to where it was',
  'a hit renders, and one already in the open session is not a link to it',
  'a stray marker cannot leave a tag open, and snippet text is escaped',
  'an empty result says so rather than showing nothing',
  'a per-session action a hosted router refuses says what it said',
  'a hosted router says why its list is empty, and warns of no wrapper',
];

/// Every spec under src/, as source.
const SPECS = import.meta.glob(['../../**/*.spec.ts', '../../**/*.spec.tsx'], { query: '?raw', import: 'default', eager: true }) as Record<string, string>;
/// This screen's specs.
const MINE = Object.entries(SPECS).filter(([path]) => path.startsWith('./') || path.includes('/screens/Sessions/'));
const CONFIG = readFileSync(join(UI, 'vite.config.ts'), 'utf8');

/// The test titles in a spec's source: `it(…)`, `it.each(…)(…)`, `it.skipIf(…)(…)` and the like.
const titles = (source: string): string[] =>
  [...source.matchAll(/\b(?:it|test)(?:\.\w+(?:\((?:[^()]|\([^()]*\))*\))?)*\(\s*(['"`])((?:\\.|(?!\1)[^\\])*)\1/g)].map((m) => (m[2] ?? '').replace(/\\(['"`])/g, '$1'));

describe('the checks themselves', () => {
  it('SE-C208: every check of scripts/check-live-ui.js is ported to a Vitest spec whose title names it', () => {
    const script = join(REPO, 'scripts', 'check-live-ui.js');
    if (existsSync(script)) {
      // While the script stands, a check added to it and not here fails this.
      const names = [...readFileSync(script, 'utf8').matchAll(/\ba?check\(\s*(['"`])((?:\\.|(?!\1)[^\\])*)\1/g)].map((m) => (m[2] ?? '').replace(/\\'/g, "'"));
      expect(names).toEqual(PORTED);
    }
    const all = Object.values(SPECS).flatMap(titles);
    const unported = PORTED.filter((name) => !all.some((t) => t.includes(name)));
    expect(unported).toEqual([]);
  });

  it('SE-C209: the specs that drive this screen run one after another and wait for what they assert: none runs concurrently, and none sleeps a fixed time', () => {
    expect(CONFIG).not.toMatch(/concurrent\s*:\s*true|shuffle\s*:\s*true/);
    const loose = MINE.filter(([, src]) => /\b(?:it|describe|test)\.concurrent\b/.test(src)).map(([p]) => p);
    expect(loose).toEqual([]);
    // A wait of so many milliseconds is a guess at how long a condition takes; waitFor and findBy
    // wait for the condition itself.
    const sleeps = MINE.filter(([, src]) => /setTimeout\(\s*\w+\s*,\s*[1-9]/.test(src)).map(([p]) => p);
    expect(sleeps).toEqual([]);
    const screens = MINE.filter(([, src]) => src.includes('renderSessions('));
    expect(screens.length).toBeGreaterThan(0);
    for (const [path, src] of screens) expect(/\bwaitFor\(|\bfindBy|\bfindAllBy/.test(src), path).toBe(true);
  });
});

describe('a rejection nobody awaited', () => {
  const seen: unknown[] = [];
  const keep = (why: unknown) => seen.push(why);
  beforeEach(() => {
    FakeEventSource.reset();
    seen.length = 0;
    process.on('unhandledRejection', keep);
  });
  afterEach(() => {
    process.off('unhandledRejection', keep);
    vi.unstubAllGlobals();
  });

  it('SE-C210: anything the screen starts and does not await that rejects fails the run — the runner is not told to ignore it, and the screen leaves none against a failing router', async () => {
    expect(CONFIG).not.toMatch(/dangerouslyIgnoreUnhandledErrors/);
    // A router that refuses or drops everything: every poll, every probe, every write fails.
    let n = 0;
    const router = stubRouter((req) => {
      if (req.path === '/api/live/sessions' && n++ === 0) return undefined;
      return n % 2 ? Promise.reject(new TypeError('fetch failed')) : json({ ok: false, reason: 'refused' }, 500);
    });
    renderSessions(`/live?session=${A}`);
    await waitFor(() => streamOf(A));
    act(() => {
      streamOf(A).drop(2);
    });
    await waitFor(() => expect(router.gets('/api/live/prompt').length).toBeGreaterThan(0));
    await new Promise((r) => setImmediate(r));
    expect(seen).toEqual([]);
  });
});

describe('the safe areas', () => {
  const plain = (s: string) => s.replace(/\/\*[\s\S]*?\*\//g, '').replace(/\/\/.*$/gm, '');
  const CSS = plain(readFileSync(join(__dirname, 'Sessions.module.scss'), 'utf8'));
  function block(opener: string): string {
    const at = CSS.indexOf(opener);
    expect(at, opener).toBeGreaterThanOrEqual(0);
    const open = CSS.indexOf('{', at);
    let depth = 0;
    for (let i = open; i < CSS.length; i++) {
      if (CSS[i] === '{') depth += 1;
      if (CSS[i] === '}' && --depth === 0) return CSS.slice(open + 1, i);
    }
    throw new Error(`unclosed ${opener}`);
  }

  it('SE-C199: the column clears the top safe area under the shell’s fixed header, the composer the bottom one, and the headers and composer the sides', () => {
    expect(block('\n.screen {')).toMatch(/block-size:[^;]*-\s*env\(safe-area-inset-top\)/);
    expect(block(':global(#main-content):has(> [data-sessions-screen])')).toMatch(/padding-block-start:[^;]*env\(safe-area-inset-top\)/);
    const dock = block('\n.dock {');
    expect(dock).toMatch(/env\(safe-area-inset-bottom\)/);
    expect(dock).toMatch(/env\(safe-area-inset-left\)/);
    expect(dock).toMatch(/env\(safe-area-inset-right\)/);
    const head = block('\n.paneHead {');
    expect(head).toMatch(/env\(safe-area-inset-left\)/);
    expect(head).toMatch(/env\(safe-area-inset-right\)/);
  });
});
