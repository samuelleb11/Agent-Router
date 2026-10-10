// New for agent-router: what the harness answers for the Sessions screen, as each router it can stand
// for answers it. The hosted one is what the shots of `/live?router=hosted` draw: the list headed
// per machine (SE-C219, SE-C220), and the showcase session the shot checks open still listed.
import { describe, expect, it } from 'vitest';
import { byMachine, machineOf, projectOf, UNNAMED_MACHINE, type SessionRow } from '~/model/session';
import { asked } from './asked';
import { HOSTED_SESSIONS, SESSIONS, SHOWCASE, sessionsRead, withSessions } from './sessions';

const AT = Date.UTC(2026, 9, 5, 9, 0, 0);
const read = async <T>(path: string, query: Record<string, string> = {}, router?: 'local' | 'hosted'): Promise<T> => {
  const r = sessionsRead(path, new URLSearchParams(query), router);
  if (!r) throw new Error(`no answer for ${path}`);
  return (await r.json()) as T;
};

describe('the Sessions fixtures', () => {
  it('stand for a local router by default: its sessions name no machine, and /api/auth is left to Settings', async () => {
    expect(await read<SessionRow[]>('/api/live/sessions')).toEqual(SESSIONS);
    expect(SESSIONS.map(machineOf).filter(Boolean)).toEqual([]);
    expect(sessionsRead('/api/auth', new URLSearchParams())).toBeNull();
  });

  it('stand for a hosted router when asked: it says so, and lists the same sessions from several machines, headed per machine', async () => {
    expect(await read<{ mode: string }>('/api/auth', {}, 'hosted')).toMatchObject({ mode: 'server' });
    const rows = await read<SessionRow[]>('/api/live/sessions', {}, 'hosted');
    expect(rows).toEqual(HOSTED_SESSIONS);
    expect(rows.map((r) => r.session_id)).toEqual(SESSIONS.map((r) => r.session_id));
    const parts = byMachine(rows, AT);
    expect(parts.length).toBeGreaterThanOrEqual(3);
    expect(parts.at(-1)?.heading).toBe(UNNAMED_MACHINE);
    // A label long enough that a phone has to cut it short, on a Windows machine.
    const windows = rows.filter((r) => r.cwd?.startsWith('C:\\'));
    expect(windows.length).toBeGreaterThan(0);
    for (const r of windows) expect([projectOf(r), (machineOf(r) ?? '').length > 40]).toEqual([projectOf(SESSIONS.find((s) => s.session_id === r.session_id)!), true]);
    // The showcase is listed, on a machine, and its own reads are still answered.
    expect(machineOf(rows.find((r) => r.session_id === SHOWCASE)!)).toBe('studio');
    expect(sessionsRead('/api/live/prompt', new URLSearchParams({ session: SHOWCASE }), 'hosted')).not.toBeNull();
  });

  it('answer through the harness’s fetch as the router they stand for, and pass everything else on', async () => {
    const record = asked();
    const next = async () => new Response('next');
    const hosted = withSessions(next, record, 'hosted');
    expect(((await (await hosted('/api/auth')).json()) as { mode: string }).mode).toBe('server');
    expect(await (await withSessions(next, record)('/api/auth')).text()).toBe('next');
    expect(record.answered).toEqual(['GET /api/auth']);
  });
});
