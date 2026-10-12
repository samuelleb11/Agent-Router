import { useRead } from './held';
import { deleteJson, getJson, postJson } from './http';
import type { ProviderRegistry, RegistryModel, RegistryService, ServiceInput } from '~/model/provider-registry';

export const providerRegistryKey = ['provider-registry'] as const;
export const readProviderRegistry = (signal?: AbortSignal) => getJson<ProviderRegistry>('/api/provider-registry', { signal });
export const saveProviderService = (service: ServiceInput) => postJson<{ ok: true; service: RegistryService }>('/api/provider-registry/services', service);
export const saveModelRoute = (model: RegistryModel) => postJson<{ ok: true; model: RegistryModel }>('/api/provider-registry/models', model);
export const removeProviderService = (id: string) => deleteJson<{ ok: true }>(`/api/provider-registry/services/${encodeURIComponent(id)}`);
export const removeModelRoute = (alias: string) => deleteJson<{ ok: true }>(`/api/provider-registry/models/${encodeURIComponent(alias)}`);
export function useProviderRegistry() {
  return useRead({ queryKey: providerRegistryKey, queryFn: ({ signal }) => readProviderRegistry(signal), refetchOnMount: 'always' });
}
