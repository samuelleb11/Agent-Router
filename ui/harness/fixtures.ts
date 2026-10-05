// New for agent-router: what the fixture harness answers, written in the router's own shapes
// (src/api.rs for the reads, src/live.rs for the stream). Invented, every one.
import type { Health, ProjectRow } from '~/data/router';

export const HEALTH: Health = { ok: true, pid: 4242, version: 'fixture harness', build: 'fixture' };

const DAY = 86_400_000;
const AT = Date.UTC(2026, 9, 5, 9, 0, 0);

const project = (path: string | null, prompt_tokens: number, sessions: number, models: string): ProjectRow => ({
  project: path,
  sessions,
  requests: sessions * 37,
  prompt_tokens,
  cost: prompt_tokens / 250_000,
  output_tokens: Math.round(prompt_tokens / 40),
  cache_read_tokens: Math.round(prompt_tokens * 0.8),
  subagent_tokens: Math.round(prompt_tokens * 0.1),
  errors: sessions > 3 ? 2 : 0,
  models,
  first_seen: AT - 30 * DAY,
  last_seen: AT - sessions * 60_000,
});

/// Most prompt tokens first, as the router orders them. A long directory name, to see the rail
/// hold it in a hand, and the one row a session whose directory was never resolved gives.
export const PROJECTS: readonly ProjectRow[] = [
  project('/Users/sample/code/agent-router', 18_400_000, 9, 'claude-opus-4-1,claude-sonnet-4-5'),
  project('/Users/sample/code/northstar', 7_200_000, 5, 'claude-sonnet-4-5'),
  project('/Users/sample/code/a-directory-name-long-enough-to-wrap-in-a-phones-rail', 410_000, 2, 'claude-haiku-4-5'),
  project(null, 12_000, 1, 'claude-sonnet-4-5'),
];

/// One event of a stream: a transcript item as an unnamed message, or a named event (`status`,
/// `error`) as the router sends them, `after` milliseconds after the one before it.
export interface StreamEvent {
  readonly event?: 'status' | 'error';
  readonly data: unknown;
  readonly after: number;
}

/// `/api/live/stream?session=<id>`, by session id: the tail of the transcript the router
/// replays on every connection, and then what it says about the session.
export const STREAMS: Readonly<Record<string, readonly StreamEvent[]>> = {
  'fixture-session-1': [
    { after: 0, data: { role: 'user', kind: 'text', text: 'Run the tests and tell me what fails.' } },
    { after: 20, data: { role: 'assistant', kind: 'text', text: 'Running `pnpm test` in ui/.' } },
    { after: 20, data: { role: 'assistant', kind: 'tool_use', name: 'Bash', input: { command: 'pnpm test' } } },
    { after: 20, event: 'status', data: { state: 'tool', tone: 'busy', activity: 'tool', doing: 'Running Bash' } },
    { after: 40, data: { role: 'user', kind: 'tool_result', text: 'Tests  15 passed (15)' } },
    { after: 20, event: 'status', data: { state: 'idle', tone: 'idle', activity: 'idle', doing: 'Idle' } },
  ],
  'fixture-session-gone': [{ after: 0, event: 'error', data: 'no such session' }],
};
