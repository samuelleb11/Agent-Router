/// Router credentials for clients and machines, separate from provider account credentials.
export type TokenScope = 'client' | 'machine' | 'admin';

export interface AccessToken {
  readonly id: string;
  readonly name: string;
  readonly prefix: string;
  readonly scope: TokenScope;
  readonly created_at: number;
  readonly last_used_at: number | null;
  readonly expires_at: number | null;
  readonly revoked_at: number | null;
}

export const TOKEN_SCOPES: readonly { readonly value: TokenScope; readonly label: string; readonly description: string }[] = [
  { value: 'client', label: 'Client', description: 'Send model requests and read the model catalogue. Use for Claude Code, Codex or OpenCode.' },
  { value: 'machine', label: 'Machine', description: 'Connect a machine agent, report its sessions and send model requests.' },
  { value: 'admin', label: 'Administrator', description: 'Access the dashboard and change router settings, accounts and access tokens.' },
];

export type TokenLifetime = '30' | '90' | '365' | 'never';
export const TOKEN_LIFETIMES: readonly { readonly value: TokenLifetime; readonly label: string }[] = [
  { value: '30', label: '30 days' },
  { value: '90', label: '90 days' },
  { value: '365', label: '1 year' },
  { value: 'never', label: 'Never' },
];

export const tokenExpiry = (lifetime: TokenLifetime, now: number = Date.now()): number | null =>
  lifetime === 'never' ? null : now + Number(lifetime) * 86_400_000;

export function tokenStatus(token: AccessToken, now: number = Date.now()): 'Active' | 'Expired' | 'Revoked' {
  if (token.revoked_at !== null) return 'Revoked';
  if (token.expires_at !== null && token.expires_at <= now) return 'Expired';
  return 'Active';
}

export const tokenScopeLabel = (scope: TokenScope): string => TOKEN_SCOPES.find((s) => s.value === scope)?.label ?? scope;
