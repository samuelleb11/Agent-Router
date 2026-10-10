// New for agent-router: the fixtures scripts/check-live-ui.js builds inline, as values built around
// a clock the spec chooses. The router's clock (`as_of_ms`) is set seven minutes behind it, so a
// duration measured against the wrong clock shows (SE-C212).

import type { SearchReply } from '../search';
import type { LiveOptions, SessionRow, Status } from '../session';

/// Two sessions in one project and one in another, as an older router lists them: no activity, no
/// tone, no detail.
export const base = (): SessionRow[] => [
  { session_id: 'aaa-1', cwd: '/code/project-a', project: 'p1', title: 'Older one', created_ms: 1000, last_activity_ms: 9000, active: true, wrapped: true, state: 'working' },
  { session_id: 'bbb-2', cwd: '/code/project-a', project: 'p1', title: 'Newer one', created_ms: 5000, last_activity_ms: 2000, active: false, wrapped: false, state: 'ask' },
  { session_id: 'ccc-3', cwd: '/code/project-b', project: 'p2', title: 'Elsewhere', created_ms: 3000, last_activity_ms: 3000, active: false, wrapped: false, state: 'failed' },
];

/// A third session current in the same project as the first two, so the tab strip is drawn at all.
export const third = (now: number): SessionRow => ({
  session_id: 'ddd-4', cwd: '/code/project-a', project: 'p1', title: 'Third one', created_ms: 7000, last_activity_ms: now, active: false, wrapped: false, state: 'idle',
});

export interface Live {
  /// The router's clock: seven minutes behind the spec's.
  readonly at: number;
  /// A session as a current router lists it, doing `activity`; everything happened 12 s before
  /// the router's clock.
  readonly act: (id: string, activity: string, extra?: Partial<SessionRow>) => SessionRow;
  /// Every activity the router can send, and one it has not sent yet, in a tone the page has
  /// never heard of.
  readonly activities: () => SessionRow[];
  /// The same, as the open view's `status` event carries it.
  readonly status: (activity: string, extra?: Partial<Status>) => Status;
}

export function live(now: number): Live {
  const at = now - 7 * 60e3;
  const act = (id: string, activity: string, extra: Partial<SessionRow> = {}): SessionRow => ({
    session_id: id, cwd: '/code/project-c', project: 'p3', title: `Session ${id}`,
    created_ms: at - 3600e3, last_activity_ms: now - 12e3, wrapped: true, archived: false,
    bytes: 1, as_of_ms: at, since_ms: at - 12e3, source: 'registry', open: true, activity, ...extra,
  });
  const activities = (): SessionRow[] => [
    act('gen-1', 'generating', { tone: 'busy', state: 'working', active: true, detail: 'Thinking' }),
    act('tool-1', 'tool', { tone: 'busy', state: 'working', active: true, detail: 'Running Bash' }),
    act('bg-1', 'background', { tone: 'busy', state: 'working', active: true, detail: '2 background agents running', background: { agents: 2, workflows: 0, shells: 0 } }),
    act('wait-1', 'waiting', {
      tone: 'attention', state: 'ask', active: true, detail: 'Bash wants to run',
      waiting_on: { kind: 'permission', tool: 'Bash', request_id: 'r-1', tool_use_id: 't-1', summary: 'git status' },
    }),
    act('stall-1', 'stalled', { tone: 'warn', state: 'stalled', active: true, detail: 'No progress', since_ms: at - 700e3 }),
    act('fail-1', 'failed', { tone: 'error', state: 'failed', active: false, detail: 'API error: overloaded' }),
    act('idle-1', 'idle', { tone: 'idle', state: 'idle', active: false, detail: 'Waiting for you' }),
    act('gone-1', 'closed', {
      tone: 'closed', state: 'idle', active: false, open: false, since_ms: undefined,
      detail: 'Session ended', ended_ms: at - 3 * 3600e3, ended_how: 'exit', mid_turn: false,
    }),
    // A newer router's word, in a tone this page has never heard of.
    act('new-1', 'compacting', { tone: 'sparkle', state: 'working', active: true, detail: 'Compacting' }),
  ];
  const statusOf = (activity: string, extra: Partial<Status> = {}): Status => ({
    state: 'working', detail: 'Working', quiet_ms: 12e3, queued: 0, activity, as_of_ms: at, since_ms: at - 12e3, open: true, ...extra,
  });
  return { at, act, activities, status: statusOf };
}

/// A search reply, as the router spells it. The second hit carries a stray opening marker with no
/// close: it may shift the emphasis, never break out.
export function search(now: number): SearchReply {
  return {
    query: 'kestrel',
    expression: 'kestrel',
    sessions: [{ session_id: 'aaaaaaaa-1111-2222-3333-444444444444', project: '/code/agent-router', blocks: 2, hits: 3, first_at: now - 6e5, last_at: now - 1e4 }],
    hits: [
      { block_id: 1, kind: 'tool_use', role: 'assistant', ordinal: 4, at: now - 1e4, session_id: 'aaaaaaaa-1111-2222-3333-444444444444', project: '/code/agent-router', snippet: 'where does the ‹kestrel› roost', rank: -8 },
      { block_id: 2, kind: 'text', role: 'user', ordinal: 0, at: now - 6e5, session_id: 'bbbbbbbb-1111-2222-3333-444444444444', project: '/code/other', snippet: 'a stray ‹marker and no close', rank: -7 },
    ],
  };
}

/// What a session can be switched to, which every router sends — a hosted one included.
export const OPTIONS: LiveOptions = { models: ['opus', 'sonnet'], efforts: ['low', 'medium', 'high'], modes: [{ value: 'default', label: 'Ask first' }] };

/// What every per-session route of a hosted router answers, with a 503
/// (docs/hosting-on-kubernetes.md).
export const HOSTED_ERROR = 'live sessions need the Agent Router agent on the machine that runs them; a hosted router does not have one yet';
