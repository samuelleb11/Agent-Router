// New for agent-router: the Machines screen's read of the router, `GET /api/machines` (src/machines.rs,
// `seen`, answered by a hosted router only, src/hosted.rs), typed by hand because the router has no
// OpenAPI spec. It goes through ~/data/http, the one fetch layer, so a refusal and a timeout read
// the same way here as on every other screen (SH-C23).

import { getJson } from '~/data/http';
import { Refused } from '~/lib/errors';
import type { MachineRow } from '~/model/machine';

/// The Machines screen's key.
export const machineKeys = {
  list: ['machines'] as const,
};

const isRecord = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null && !Array.isArray(v);

/// `GET /api/machines`: every machine the router has seen since it last started. A reply that is
/// not a list is a failed read, never machines (MA-C6); a row that is not an object is dropped.
export async function readMachines(signal?: AbortSignal): Promise<MachineRow[]> {
  const body = await getJson<unknown>('/api/machines', { signal });
  if (!Array.isArray(body)) throw new Refused(200, 'The router answered something that is not a list of machines.');
  return body.filter(isRecord) as unknown as MachineRow[];
}
