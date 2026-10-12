import type { AddProvider } from './settings';

export interface RegistryCapabilities {
  readonly tools: boolean;
  readonly images: boolean;
  readonly reasoning: boolean;
  readonly compaction: boolean;
  readonly context_tokens: number | null;
  readonly output_tokens: number | null;
}

export const DEFAULT_CAPABILITIES: RegistryCapabilities = { tools: true, images: true, reasoning: true, compaction: false, context_tokens: null, output_tokens: null };

export interface RegistryService {
  readonly id: string;
  readonly label: string;
  readonly provider: AddProvider;
  readonly base_url: string;
  readonly protocol: 'messages' | 'responses';
  readonly auth_profile: 'bearer' | 'api_key';
  readonly builtin: boolean;
  readonly capabilities: RegistryCapabilities;
}

export type ServiceInput = Omit<RegistryService, 'builtin'>;

export interface RegistryModel {
  readonly alias: string;
  readonly service: string;
  readonly upstream_model: string;
  readonly account_id: string;
  readonly display_name: string | null;
  readonly capabilities: RegistryCapabilities | null;
}

export interface ProviderRegistry {
  readonly services: readonly RegistryService[];
  readonly models: readonly RegistryModel[];
}

export function tokenLimitProblem(value: string): string | null {
  if (!value.trim()) return null;
  return /^\d+$/.test(value) && Number(value) >= 1 && Number(value) <= 100_000_000 ? null : 'Use a whole number between 1 and 100,000,000, or leave blank for an unknown limit.';
}

export function serviceUrlProblem(value: string): string | null {
  try {
    const url = new URL(value);
    if (url.protocol === 'https:' && !url.username && !url.password && !value.includes('?') && !value.includes('#') && !url.pathname.replace(/\/+$/, '').endsWith('/v1')) return null;
  } catch { /* The field reports one clear rule for every invalid address. */ }
  return 'Use an HTTPS base URL without credentials, query, fragment or a final /v1.';
}
