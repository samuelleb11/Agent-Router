import { useRead } from './held';
import { getJson, postJson } from './http';
import type { ClientApplied, ClientChecked, ClientPreview, ClientRestored, ClientSetup, ClientSetupInput, SetupClientId } from '~/model/client-setup';

export const clientSetupKey = ['clients', 'setup'] as const;

export const readClientSetup = (signal?: AbortSignal) => getJson<ClientSetup>('/api/clients', { signal });

export const previewClient = (client: SetupClientId, input: ClientSetupInput) =>
  postJson<ClientPreview>(`/api/clients/${client}/preview`, input);
export const checkClient = (client: SetupClientId, input: ClientSetupInput) =>
  postJson<ClientChecked>(`/api/clients/${client}/check`, input);
export const applyClient = (client: SetupClientId, input: ClientSetupInput) =>
  postJson<ClientApplied>(`/api/clients/${client}/apply`, input);
export const restoreClient = (client: SetupClientId, project?: string) =>
  postJson<ClientRestored>(`/api/clients/${client}/restore`, project ? { project } : {});

export function useClientSetup() {
  // Only secret-free installation metadata enters the query cache. Preview and write calls
  // are invoked directly by the page, so tokens and generated configuration never do.
  return useRead({ queryKey: clientSetupKey, queryFn: ({ signal }) => readClientSetup(signal), refetchOnMount: 'always' });
}
