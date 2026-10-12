import { useRead } from '~/data/held';
import { deleteJson, getJson, postJson } from '~/data/http';
import type { AccessToken, TokenScope } from '~/model/access-tokens';

export const accessTokenKey = ['auth', 'tokens'] as const;

export async function readAccessTokens(signal?: AbortSignal): Promise<AccessToken[]> {
  const answer = await getJson<{ tokens: AccessToken[] }>('/api/auth/tokens', { signal });
  return answer.tokens;
}

export interface CreatedAccessToken extends AccessToken {
  readonly ok: true;
  /// Returned once by the create endpoint; never a query-cache value.
  readonly token: string;
}

export const createAccessToken = (name: string, scope: TokenScope, expires_at: number | null) =>
  postJson<CreatedAccessToken>('/api/auth/tokens', { name, scope, expires_at });

export const revokeAccessToken = (id: string) => deleteJson<{ ok: true }>(`/api/auth/tokens/${encodeURIComponent(id)}`);

export function useAccessTokens() {
  return useRead({
    queryKey: accessTokenKey,
    queryFn: ({ signal }) => readAccessTokens(signal),
    refetchOnMount: 'always',
    refetchInterval: 30_000,
  });
}
