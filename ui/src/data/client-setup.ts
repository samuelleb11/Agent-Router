import { useRead } from './held';
import { getJson, postJson } from './http';
import type { ClientApplied, ClientChecked, ClientPreview, ClientRestored, ClientSetup, ClientSetupInput, SetupClientId } from '~/model/client-setup';

export const clientSetupKey = ['clients', 'setup'] as const;

/// A path of the Clients API, about one machine when one is chosen: a hosted router reads, applies
/// and restores that machine's Claude Code and Codex through its agent (AGEN-0200).
export const clientsOnMachine = (path: string, machine?: string | null) => (machine ? `${path}?machine=${encodeURIComponent(machine)}` : path);

export const readClientSetup = (signal?: AbortSignal, machine?: string | null) => getJson<ClientSetup>(clientsOnMachine('/api/clients', machine), { signal });

export const previewClient = (client: SetupClientId, input: ClientSetupInput, machine?: string | null) =>
  postJson<ClientPreview>(clientsOnMachine(`/api/clients/${client}/preview`, machine), input);
export const checkClient = (client: SetupClientId, input: ClientSetupInput, machine?: string | null) =>
  postJson<ClientChecked>(clientsOnMachine(`/api/clients/${client}/check`, machine), input);
export const applyOnMachine = (client: SetupClientId, input: ClientSetupInput, machine?: string | null) =>
  postJson<ClientApplied>(clientsOnMachine(`/api/clients/${client}/apply`, machine), input);
export const restoreClient = (client: SetupClientId, project?: string, machine?: string | null) =>
  postJson<ClientRestored>(clientsOnMachine(`/api/clients/${client}/restore`, machine), project ? { project } : {});

/// The router's own clients, or with `machine` that machine's, as its agent reads them.
export function useClientSetup(machine: string | null = null) {
  // Only secret-free installation metadata enters the query cache. Preview and write calls
  // are invoked directly by the page, so tokens and generated configuration never do.
  return useRead({
    queryKey: machine ? [...clientSetupKey, machine] : clientSetupKey,
    queryFn: ({ signal }) => readClientSetup(signal, machine),
    refetchOnMount: 'always',
    // An offline machine, or an agent that predates client setup, says so at once.
    ...(machine ? { retry: false } : {}),
  });
}
