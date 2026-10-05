// New for agent-router: what the fixture harness answers for the Sessions screen, in the router's own
// shapes (src/live.rs: `LiveSession`, `Item`, `StreamStatus`, `Stats`, `project_stats`, `prompt`,
// `options`). Invented, every one. A local router whose list has every look a row can take — each
// activity, a project of ended sessions, an older router's row, the older fold and the archived
// shelf — and one session, the showcase, whose stream replays every kind of feed row there is:
// a long unbroken URL, a wide table, a fence, a long command and its long output, a subagent, a
// cancellation, a command, a question form and its answer. Its status then repeats once a second,
// as the router's heartbeat does, so the screen keeps believing it.
//
// Registered in harness/main.tsx. The reads Settings and the Overview already answer — /api/auth,
// /api/setup/shim, /api/search — are theirs (harness/settings.ts, harness/overview.ts).
import type { Asked } from './asked';
import type { StreamEvent } from './fixtures';
import type { ProjectTotals, Stats } from '~/data/live';
import type { LiveOptions, Prompt, SessionRow, Status, StreamItem } from '~/model/session';

/// The router's clock, which every age on the list is measured on (SE-C43).
const AT = Date.UTC(2026, 9, 5, 9, 0, 0);
const MIN = 60_000;
const HOUR = 60 * MIN;
const DAY = 24 * HOUR;

/// The session the shot's checks open: the one whose feed holds every kind of row.
export const SHOWCASE = '7f3a9c2e-5b1d-4e8a-9c3f-0a1b2c3d4e5f';

const AR = '/Users/sample/code/agent-router';
const LONG = '/Users/sample/code/a-directory-name-long-enough-to-wrap-in-a-phones-rail';

const row = (id: string, extra: Partial<SessionRow>): SessionRow => ({
  session_id: id,
  cwd: AR,
  project: 'agent-router',
  git_branch: 'main',
  title: null,
  created_ms: AT - 3 * HOUR,
  last_activity_ms: AT - 2 * MIN,
  active: false,
  state: 'idle',
  archived: false,
  wrapped: true,
  bytes: 524_288,
  activity: 'idle',
  open: true,
  detail: 'Waiting for you',
  since_ms: AT - 2 * MIN,
  tone: 'idle',
  source: 'registry',
  as_of_ms: AT,
  ...extra,
});

export const SESSIONS: readonly SessionRow[] = [
  row(SHOWCASE, {
    title: 'Port the live view to the console and keep every contract green',
    created_ms: AT - 5 * HOUR,
    last_activity_ms: AT - 4_000,
    active: true,
    state: 'working',
    activity: 'tool',
    detail: 'Running Bash',
    since_ms: AT - 12_000,
    tone: 'busy',
  }),
  row('a1b2c3d4-0000-4000-8000-000000000002', {
    title: 'Write the session specs',
    created_ms: AT - 4 * HOUR,
    last_activity_ms: AT - 65 * MIN,
    active: true,
    state: 'ask',
    activity: 'waiting',
    detail: 'Waiting for you · background shell running',
    since_ms: AT - 65 * MIN,
    tone: 'attention',
    waiting_on: { kind: 'permission', tool: 'Bash', request_id: 'req-2', tool_use_id: 'toolu_2', summary: 'git push' },
  }),
  row('a1b2c3d4-0000-4000-8000-000000000003', {
    title: 'Review the palette',
    git_branch: 'palette',
    created_ms: AT - 2 * HOUR,
    last_activity_ms: AT - 40 * MIN,
    wrapped: false,
  }),
  row('a1b2c3d4-0000-4000-8000-000000000004', {
    title: 'An idea put away',
    created_ms: AT - 6 * HOUR,
    last_activity_ms: AT - 3 * HOUR,
    archived: true,
  }),
  row('a1b2c3d4-0000-4000-8000-000000000005', {
    cwd: '/Users/sample/code/northstar',
    project: 'northstar',
    title: 'Close the definition-of-done gap',
    git_branch: 'dod',
    last_activity_ms: AT - 9 * MIN,
    state: 'failed',
    activity: 'failed',
    detail: 'API error: 529 Overloaded',
    since_ms: AT - 9 * MIN,
    tone: 'error',
  }),
  row('a1b2c3d4-0000-4000-8000-000000000006', {
    cwd: LONG,
    project: 'a-directory-name-long-enough-to-wrap-in-a-phones-rail',
    title: 'A title long enough to need its two lines and then some more, so that a phone has to cut it short with an ellipsis',
    git_branch: 'a-branch-name-that-goes-on-for-quite-a-while-without-a-break',
    last_activity_ms: AT - 14 * MIN,
    active: true,
    state: 'stalled',
    activity: 'stalled',
    detail: 'No progress',
    since_ms: AT - 12 * MIN,
    tone: 'warn',
  }),
  row('a1b2c3d4-0000-4000-8000-000000000007', {
    cwd: '/Users/sample/code/menubar',
    project: 'menubar',
    title: 'Ship the menubar build',
    last_activity_ms: AT - 3 * DAY,
    open: false,
    activity: 'closed',
    tone: 'closed',
    detail: 'Session ended',
    since_ms: undefined,
    ended_ms: AT - 3 * DAY,
    ended_how: 'exit',
    mid_turn: false,
  }),
  // As an older router lists a session: no activity, no tone, no detail (SE-C17).
  {
    session_id: 'a1b2c3d4-0000-4000-8000-000000000008',
    cwd: '/Users/sample/code/legacy',
    project: 'legacy',
    git_branch: null,
    title: 'From an older router',
    created_ms: AT - 2 * HOUR,
    last_activity_ms: AT - 30_000,
    active: true,
    state: 'working',
    archived: false,
    wrapped: false,
    bytes: 2048,
  },
];

const t = (ago: number): string => new Date(AT - ago).toISOString();

/// Forty lines of a test run: long enough that the fold has something to hide.
const OUTPUT = [
  ' RUN  v3.2.7 /Users/sample/code/agent-router/ui',
  '',
  ...Array.from({ length: 34 }, (_, i) => ` ✓ src/screens/Sessions/a-spec-file-${i + 1}.spec.tsx (${(i % 7) + 3} tests) ${120 + i * 7}ms`),
  '',
  ' Test Files  34 passed (34)',
  '      Tests  211 passed (211)',
  '   Duration  10.26s (transform 2.69s, setup 4.91s, collect 17.39s, tests 25.40s, environment 10.13s, prepare 2.57s)',
].join('\n');

const REPLAY: readonly StreamItem[] = [
  {
    role: 'user',
    kind: 'text',
    t: t(32 * MIN),
    text:
      'Port the live view to the console and keep every contract green. The Carbon story is at https://github.com/carbon-design-system/carbon/blob/main/packages/react/src/components/ContainedList/ContainedList.stories.js?plain=1&a-query-string-long-enough-that-it-never-breaks=true',
  },
  { role: 'assistant', kind: 'thinking', t: t(31.9 * MIN), dur_ms: 2400, text: 'The list first, then the feed, then the composer. The stream hook is the part with rules.' },
  {
    role: 'assistant',
    kind: 'text',
    t: t(31.8 * MIN),
    dur_ms: 1200,
    text: [
      '## The plan',
      '',
      'Three parts, **in this order**, each with its *own* spec:',
      '- the list, one `ContainedList` per project;',
      '- the feed, drawn from [the model’s block tree](https://carbondesignsystem.com/components/code-snippet/usage/);',
      '1. the composer, last.',
      '',
      '| File | Lines | What it holds |',
      '| --- | ---: | --- |',
      '| src/screens/Sessions/index.tsx | 321 | the screen, its address, the list and the open session beside it on a desk |',
      '| src/screens/Sessions/Feed.tsx | 240 | every kind of row the stream delivers, in the order it delivers them |',
      '',
      '> The router decides what a session is doing.',
      '',
      '```tsx',
      'export default function Sessions() { return <div className={styles.screen} data-sessions-screen="" aria-label="a line long enough to scroll sideways inside its own box" />; }',
      '```',
      'A bare address stays text: https://example.com/a/bare/address/that/is/very/long/and/unbroken/so/it/has/to/wrap/anywhere/in/a/phones/column',
    ].join('\n'),
  },
  {
    role: 'assistant',
    kind: 'tool_use',
    name: 'Bash',
    t: t(31 * MIN),
    dur_ms: 800,
    text: 'pnpm exec vitest run src/screens/Sessions/useSessionStream.spec.ts src/screens/Sessions/session-switch.spec.tsx --reporter=verbose --coverage.enabled=false',
  },
  { role: 'user', kind: 'tool_result', t: t(30.5 * MIN), dur_ms: 30_000, text: OUTPUT },
  { role: 'assistant', kind: 'tool_use', name: 'Read', t: t(30 * MIN), text: '/Users/sample/code/agent-router/ui/src/screens/Sessions/a-file-with-a-name-long-enough-to-wrap.tsx' },
  { role: 'user', kind: 'tool_result', t: t(29.9 * MIN), text: '' },
  { role: 'assistant', kind: 'text', sidechain: true, t: t(29 * MIN), text: 'A subagent read the contracts: **90** ids were still unnamed.' },
  { role: 'user', kind: 'interrupt', t: t(28 * MIN), text: '[Request interrupted by user]' },
  { role: 'user', kind: 'command', t: t(27 * MIN), text: '/compact keep the contracts and the plan' },
  {
    role: 'assistant',
    kind: 'ask',
    t: t(26 * MIN),
    ask: {
      questions: [
        {
          question: 'Where should the list sit on a desk?',
          header: 'Layout',
          multiSelect: false,
          options: [
            { label: 'Beside the feed', description: 'From 1,056 px, as decision H says' },
            { label: 'Over the feed', description: 'One pane at a time at every width' },
          ],
        },
        {
          question: 'Which checks should run in CI?',
          header: 'Checks',
          multiSelect: true,
          options: [{ label: 'Vitest', description: 'Every spec' }, { label: 'The shots' }],
        },
      ],
    },
  },
  { role: 'user', kind: 'answer', t: t(25 * MIN), text: 'Layout: Beside the feed\nChecks: Vitest, The shots' },
  { role: 'assistant', kind: 'text', t: t(2 * MIN), dur_ms: 5200, text: 'Done. The shots are next.' },
  { role: 'user', kind: 'text', t: t(MIN), text: 'Run the shots at 360, 390 and 1440 in both themes.' },
  { role: 'assistant', kind: 'tool_use', name: 'Bash', t: t(12_000), text: 'pnpm run shots -- --routes /live --widths 360,390,1440 --themes white,g100 --fail-on-overflow' },
];

/// The heartbeat: the same status once a second, its clock moving, for fifteen minutes.
const heartbeat = (base: Status, seconds = 900): StreamEvent[] =>
  Array.from({ length: seconds }, (_, i) => ({ event: 'status' as const, after: i === 0 ? 30 : 1000, data: { ...base, as_of_ms: (base.as_of_ms ?? AT) + i * 1000, quiet_ms: (base.quiet_ms ?? 0) + i * 1000 } }));

const replay = (items: readonly StreamItem[]): StreamEvent[] => items.map((data, i) => ({ after: i === 0 ? 0 : 5, data }));

export const STREAMS: Readonly<Record<string, readonly StreamEvent[]>> = {
  [SHOWCASE]: [
    ...replay(REPLAY),
    ...heartbeat({ state: 'tool', detail: 'Running Bash', quiet_ms: 12_000, queued: 1, activity: 'tool', open: true, since_ms: AT - 12_000, as_of_ms: AT, tone: 'busy' }),
  ],
};

const STATS: Readonly<Record<string, Stats>> = {
  [SHOWCASE]: { model: 'claude-opus-4-1-20250805', effort: 'high', mode: 'acceptEdits', input: 41_000, output: 182_000, cache_read: 8_950_000, cache_write: 410_000, total: 9_583_000, turns: 47, cost: 23.41 },
};

const PROJECT: ProjectTotals = { sessions: 3, models: ['claude-opus-4-1-20250805', 'claude-sonnet-4-5'], total: 12_402_000, output: 240_000, cache_read: 11_200_000, turns: 63, cost: 31.07 };

const OPTIONS: LiveOptions = {
  models: ['opus', 'opus[1m]', 'sonnet', 'sonnet[1m]', 'haiku', 'haiku[1m]'],
  efforts: ['low', 'medium', 'high', 'xhigh', 'max'],
  modes: [
    { value: 'default', label: 'Ask' },
    { value: 'acceptEdits', label: 'Accept edits' },
    { value: 'auto', label: 'Auto' },
    { value: 'dontAsk', label: 'Don’t ask' },
    { value: 'plan', label: 'Plan' },
    { value: 'bypassPermissions', label: 'Bypass' },
  ],
};

/// The showcase waits on a permission too, so its card is drawn above the composer: a command long
/// enough that the card has to wrap it.
const PROMPT: Prompt = {
  pending: true,
  request_id: 'req-showcase',
  tool: 'Bash',
  question: false,
  display_name: 'Bash',
  description: 'Push the branch and open the merge request',
  input: { command: 'git push --set-upstream origin agen-0012-sessions-shows-everything-the-live-view-shows-and-nothing-a-reader-typed-follows-them && glab mr create --fill --yes' },
  suggestions: [{ index: 0, label: 'Yes, and don’t ask again for git push in this project' }],
  count: 2,
};

const json = (payload: unknown): Response => new Response(JSON.stringify(payload), { status: 200, headers: { 'Content-Type': 'application/json' } });

/// The answer to one of the Sessions screen's GETs, or null for a read this file does not hold.
export function sessionsRead(path: string, query: URLSearchParams): Response | null {
  const only = (...allowed: string[]) => [...query.keys()].every((p) => allowed.includes(p));
  const id = query.get('session') ?? '';
  const listed = SESSIONS.some((s) => s.session_id === id);
  if (path === '/api/live/sessions' && only()) return json(SESSIONS);
  if (path === '/api/live/options' && only()) return json(OPTIONS);
  if (path === '/api/live/stats' && only('session') && listed) return json(STATS[id] ?? { model: 'claude-sonnet-4-5', total: 120_000, output: 4_000, cache_read: 90_000, turns: 3, cost: 0.42 });
  if (path === '/api/live/project' && only('cwd')) return json(query.get('cwd') === AR ? PROJECT : { sessions: 1, total: 120_000, turns: 3, cost: 0.42 });
  if (path === '/api/live/prompt' && only('session') && listed) return json(id === SHOWCASE ? PROMPT : { pending: false, count: 0 });
  return null;
}

/// The harness's fetch with the Sessions screen's reads answered first: a GET this file holds is
/// answered here, and kept in what the harness was asked; everything else goes on to `next`.
export function withSessions(next: typeof fetch, asked: Asked): typeof fetch {
  return async (input: RequestInfo | URL, init?: RequestInit) => {
    const href = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
    const url = new URL(href, globalThis.location?.href);
    const method = (init?.method ?? (input instanceof Request ? input.method : 'GET')).toUpperCase();
    const got = method === 'GET' ? sessionsRead(url.pathname, url.searchParams) : null;
    if (!got) return next(input, init);
    asked.answered.push(`${method} ${url.pathname}${url.search}`);
    return got;
  };
}
