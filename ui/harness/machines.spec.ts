// New for agent-router: what the harness answers for the Machines screen, as each router it can stand
// for answers it. The hosted one is what a shot of `/machines?router=hosted` draws: the machines its
// sessions come from, online and offline, on the harness router's build and not (MA-C3, MA-C4).
import { describe, expect, it } from 'vitest';
import { machineItems, type MachineRow } from '~/model/machine';
import { machineOf } from '~/model/session';
import { asked } from './asked';
import { HEALTH } from './fixtures';
import { MACHINES, machinesRead, withMachines } from './machines';
import { HOSTED_SESSIONS } from './sessions';

describe('the Machines fixtures', () => {
  it('stand for a local router by default, which lists no machines', () => {
    expect(machinesRead('/api/machines', new URLSearchParams())).toBeNull();
  });

  it('stand for a hosted router when asked: the machines its sessions name, online and offline, on the harness’s build and not', async () => {
    const r = machinesRead('/api/machines', new URLSearchParams(), 'hosted');
    if (!r) throw new Error('no answer for /api/machines');
    const rows = (await r.json()) as MachineRow[];
    expect(rows).toEqual(MACHINES);
    const named = new Set(HOSTED_SESSIONS.map(machineOf).filter(Boolean));
    expect(new Set(rows.map((m) => m.label))).toEqual(named);
    const items = machineItems(rows, HEALTH.build, Date.now());
    expect(items.map((m) => m.online)).toEqual([true, true, false]);
    expect(items.map((m) => m.build?.same)).toContain(true);
    expect(items.map((m) => m.build?.same)).toContain(false);
  });

  it('answer through the harness’s fetch as the router they stand for, and pass everything else on', async () => {
    const record = asked();
    const next = async () => new Response('next');
    expect(await (await withMachines(next, record, 'hosted')('/api/machines')).json()).toEqual(MACHINES);
    expect(await (await withMachines(next, record)('/api/machines')).text()).toBe('next');
    expect(await (await withMachines(next, record, 'hosted')('/api/auth')).text()).toBe('next');
    expect(record.answered).toEqual(['GET /api/machines']);
  });
});
