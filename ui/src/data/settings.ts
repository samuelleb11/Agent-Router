// New for agent-router: what Settings reads from the router and writes to it, typed by hand from
// its Rust handlers (the router has no OpenAPI spec), every request through ~/data/http, so a
// refusal reads the same way whatever its status: a 200 carrying `error`, a 200 carrying
// `ok: false` with its `reason`, a hosted router's 409 carrying both, and any non-2xx with an error
// body all come back as `Refused`, in the router's words (SH-C23, ST-C10).

import { useQueryClient, type QueryKey } from '@tanstack/react-query';
import { keys } from '~/app/query';
import { useRead, type Read } from '~/data/held';
import { getJson, postJson } from '~/data/http';
import { Refused } from '~/lib/errors';
import type { Account, Auth, Provider } from '~/model/settings';

/// Auth and the accounts are re-read every 30 s while something shows them, as the dashboard did
/// (ST-C5, SH-C15).
export const SLOW_POLL_MS = 30_000;

/// The keys only Settings reads. The accounts list's own key, `keys.accounts`, is the palette's
/// too; the probed list sits under it, so invalidating the list moves both.
export const settingsKeys = {
  probed: [...keys.accounts, 'probed'] as const,
  /// The catalogue, which an assignment, and any change to the accounts, moves.
  models: ['models'] as const,
  setup: ['setup'] as const,
  /// Not under `setup`: a setup write re-reads the setup, and only a wrapper write the wrapper.
  shim: ['shim'] as const,
  push: ['push'] as const,
};

// ---------------------------------------------------------------- the answers

/// One row of `GET /api/accounts` (src/api.rs, `accounts_of`), everything a card shows. The
/// credential's verdict (`ready`, `detail`, `expires_at`, `has_token`) is there only when the read
/// asked for it with `probe=1`.
export interface AccountRow extends Account {
  readonly org_name?: string | null;
  readonly subscription_type?: string | null;
  readonly profile_dir?: string | null;
  readonly created_at?: number | null;
  /// 1 while a claude.ai session key is stored for the account.
  readonly usage_key?: number | boolean | null;
  /// Whether the router holds a token of its own for it; read with the credential check.
  readonly has_token?: boolean | number | null;
}

/// One model of `GET /api/models` (src/models.rs, `row`): its id, its provider, the provider's name
/// for it, the account it was assigned (none: its provider's active account) and the account its
/// next request goes to.
export interface ModelRow {
  readonly id: string;
  readonly provider: Provider;
  readonly display_name?: string | null;
  readonly assigned?: string | null;
  readonly serving?: string | null;
}

/// One registered phone, as `GET /api/push` lists it (src/notify.rs, `status`).
export interface Device {
  readonly token_tail: string;
  readonly platform?: string | null;
  readonly name?: string | null;
  readonly environment?: string | null;
  readonly registered_at?: number | null;
  readonly last_seen_at?: number | null;
  readonly last_result?: string | null;
  readonly router_ref?: string | null;
}

/// `GET /api/login/status` (src/login.rs, `Flow::to_json`), without the terminal's `tail`, which the
/// console never shows (ST-C47).
export interface LoginStatus {
  readonly phase?: 'starting' | 'awaiting_sign_in' | 'done' | 'failed' | (string & {}) | null;
  readonly url?: string | null;
  readonly label?: string | null;
  readonly account_id?: string | null;
  readonly email?: string | null;
  readonly error?: string | null;
}

/// `GET /api/setup` (src/setup.rs, `snapshot`).
export interface Setup {
  readonly service?: { readonly registered?: boolean | null; readonly supported?: boolean | null; readonly platform?: string | null } | null;
  readonly url?: string | null;
  readonly global_settings_path?: string | null;
  readonly managed?: readonly { readonly path: string; readonly scope: string; readonly value?: string | null; readonly previous?: string | null; readonly applied_at?: number | null }[] | null;
  readonly mode?: string | null;
}

/// One extension binary the wrapper can stand in for (src/shim.rs, `Target`).
export interface ShimTarget {
  readonly extension: string;
  readonly path: string;
  readonly state: string;
}

/// `GET /api/setup/shim` (src/shim.rs, `snapshot`). `auto_log` is newest first.
export interface Shim {
  readonly available?: boolean | null;
  readonly installed?: number | null;
  readonly targets?: readonly ShimTarget[] | null;
  readonly auto?: boolean | null;
  readonly auto_log?: readonly { readonly at: number; readonly what: string }[] | null;
}

/// `GET /api/push` (src/notify.rs, `status`). `configured`, the ids and `key_file` are Apple's;
/// `fcm` is Firebase's.
export interface Push {
  readonly configured?: boolean | null;
  readonly team_id?: string | null;
  readonly key_id?: string | null;
  readonly bundle_id?: string | null;
  readonly key_file?: string | null;
  readonly fcm?: {
    readonly configured?: boolean | null;
    readonly project_id?: string | null;
    readonly client_email?: string | null;
    readonly private_key_id?: string | null;
    readonly key_file?: string | null;
    readonly last_error?: string | null;
  } | null;
  readonly any_configured?: boolean | null;
  readonly devices?: readonly Device[] | null;
  readonly last_error?: string | null;
  readonly last_sent?: { readonly at: number; readonly what: string } | null;
}

/// What `POST /api/accounts` answers: the new id, and for a profile the command to run.
export interface Added {
  readonly id: string;
  readonly ready?: boolean | null;
  readonly command?: string | null;
  readonly profile_dir?: string | null;
}

/// What `POST /api/accounts/probe` answers about a profile (src/accounts.rs, `AuthStatus`).
export interface ProfileStatus {
  readonly loggedIn?: boolean | null;
  readonly email?: string | null;
}

/// One phone's answer to a test push.
export interface TestResult {
  readonly device: string;
  readonly ok: boolean;
  readonly error?: string | null;
}

// ---------------------------------------------------------------- reads

export const readAuth = (signal?: AbortSignal) => getJson<Auth>('/api/auth', { signal });

/// Every account, oldest first. With `probe` the router checks each credential, which is not free,
/// so only the Accounts page asks for it, and only while it is open (ST-C6).
export const readAccounts = (probe: boolean, signal?: AbortSignal) => getJson<AccountRow[]>(probe ? '/api/accounts?probe=1' : '/api/accounts', { signal });

/// The catalogue: every model the router can serve now, Anthropic's first (ST-C122).
export const readModels = (signal?: AbortSignal) => getJson<ModelRow[]>('/api/models', { signal });

export const readSetup = (signal?: AbortSignal) => getJson<Setup>('/api/setup', { signal });
export const readShim = (signal?: AbortSignal) => getJson<Shim>('/api/setup/shim', { signal });
export const readPush = (signal?: AbortSignal) => getJson<Push>('/api/push', { signal });

/// The sign-in flow's status. A failed flow carries its reason as `error` under a 200, which the
/// fetch layer reads as a refusal: here it is the flow's own state, so it comes back as one.
export async function readLogin(signal?: AbortSignal): Promise<LoginStatus> {
  try {
    return await getJson<LoginStatus>('/api/login/status', { signal });
  } catch (err) {
    if (err instanceof Refused && err.status === 200) return { phase: 'failed', error: err.message };
    throw err;
  }
}

// ---------------------------------------------------------------- writes

/// Make an account the one that signs; it takes effect on the very next request (ST-C24).
export const activateAccount = (id: string) => postJson<{ ok: true; active: string }>('/api/accounts/activate', { id });
export const removeAccount = (id: string) => postJson<{ ok: true }>('/api/accounts/remove', { id });

/// The fields an edit sends: only what the reader changed (ST-C14, ST-C34, ST-C35).
export interface AccountEdit {
  readonly label?: string;
  readonly email?: string;
  readonly token?: string;
}

export const editAccount = (id: string, edit: AccountEdit) => postJson<{ ok: true; id: string }>('/api/accounts/edit', { id, ...edit });
export const setSessionKey = (accountId: string, token: string) => postJson<{ ok: true }>('/api/usage-token', { account_id: accountId, token });
export const clearSessionKey = (accountId: string) => postJson<{ ok: true }>('/api/usage-token', { account_id: accountId, clear: true });
/// How a new account is added: a pasted token, a Kimi Code key with the base URL its requests go to
/// when not Kimi Code's default, or a separate profile.
export type NewAccount =
  | { readonly source: 'token'; readonly token: string }
  | { readonly source: 'kimi'; readonly token: string; readonly base_url?: string }
  | { readonly source: 'openai'; readonly token: string; readonly base_url?: string }
  | { readonly source: 'profile' };

/// An Anthropic token goes as it always did, naming no provider; a Kimi Code key is a token account
/// of the provider `kimi`, its base URL sent only when one was typed (ST-C117).
export function addAccount(label: string, how: NewAccount) {
  if (how.source === 'profile') return postJson<Added>('/api/accounts', { label, source: 'profile' });
  if (how.source === 'token') return postJson<Added>('/api/accounts', { label, source: 'token', token: how.token });
  const baseUrl = how.base_url?.trim();
  return postJson<Added>('/api/accounts', { label, source: 'token', provider: how.source, token: how.token, ...(baseUrl ? { base_url: baseUrl } : {}) });
}
export const probeProfile = (id: string) => postJson<ProfileStatus>('/api/accounts/probe', { id });

/// Assign a model to an account of its own provider, or with null give it back to its provider's
/// active account. The router refuses an account of another provider (409) and a model nothing
/// serves (400), in its words (ST-C123).
export const assignModel = (model: string, account: string | null) => postJson<{ ok: true; model: string; account: string | null }>('/api/models/assign', { model, account });
export const startSignin = (label: string) => postJson<LoginStatus>('/api/login/start', { label });

export const setService = (enable: boolean) => postJson<Setup & { ok: true }>('/api/setup/service', { enable });
/// Point every project at the router, or one project's directory; `detail` is the router's sentence
/// for what it did to the file (ST-C57).
export const setRouting = (enable: boolean, project?: string) =>
  postJson<Setup & { ok: true; detail?: string | null }>('/api/setup/config', project === undefined ? { enable } : { enable, project });
/// Install or remove the wrapper at a path the listing gave, exactly as it gave it (ST-C62).
export const setWrapper = (path: string, action: 'install' | 'remove') => postJson<{ ok: true }>('/api/setup/shim', { path, action });
export const setWrapperAuto = (enabled: boolean) => postJson<{ ok: true }>('/api/setup/shim/auto', { enabled });

export const setNetwork = (enable: boolean) => postJson<{ ok: true }>('/api/network', { enable });

/// Sent exactly as typed, spaces included: a password is whatever was typed (ST-C74).
export const setPassword = (password: string) => postJson<{ ok: true }>('/api/auth', { password });
export const clearPassword = () => postJson<{ ok: true }>('/api/auth', { clear: true });

export const savePushKey = (key: { team_id: string; key_id: string; p8: string; bundle_id: string }) => postJson<{ ok: true }>('/api/push/config', key);
export const clearPushKey = () => postJson<{ ok: true }>('/api/push/config', { clear: true });
export const saveFirebase = (serviceAccount: string) => postJson<{ ok: true }>('/api/push/config', { platform: 'fcm', service_account: serviceAccount });

/// A test push to every phone. The router answers `ok: false` when no phone took it, with nobody to
/// blame in words: that comes back here as the phones' results, read again from the push state by
/// the caller, rather than as a refusal with no sentence in it.
export async function sendTest(): Promise<{ readonly results: readonly TestResult[] } | { readonly undelivered: true }> {
  try {
    return { results: (await postJson<{ ok: boolean; results?: TestResult[] }>('/api/push/test', {})).results ?? [] };
  } catch (err) {
    if (err instanceof Refused && err.status === 200 && err.message === 'The router refused this.') return { undelivered: true };
    throw err;
  }
}

// ---------------------------------------------------------------- hooks

/// What the router says it is: local or hosted, guarded or not, on the network or not. Re-read every
/// 30 s while anything shows it (ST-C5).
export function useAuth(): Read<Auth> {
  return useRead({ queryKey: keys.auth, queryFn: ({ signal }) => readAuth(signal), refetchInterval: SLOW_POLL_MS });
}

/// The accounts. The Accounts page asks with `probe`, read afresh each time it opens (ST-C5); every
/// other reader asks without, so it resolves no credential (ST-C6).
export function useAccounts({ probe }: { probe: boolean }): Read<AccountRow[]> {
  return useRead({
    queryKey: probe ? settingsKeys.probed : keys.accounts,
    queryFn: ({ signal }) => readAccounts(probe, signal),
    refetchInterval: SLOW_POLL_MS,
    refetchOnMount: probe ? 'always' : true,
  });
}

/// The catalogue, read afresh each time the Models page opens and after each of its writes (ST-C5,
/// ST-C122).
export function useModels(): Read<ModelRow[]> {
  return useRead({ queryKey: settingsKeys.models, queryFn: ({ signal }) => readModels(signal), refetchOnMount: 'always' });
}

/// The Claude Code page's two reads, each made afresh when it opens, and never on a router that
/// has not said it is local (ST-C5, ST-C53).
export function useSetup(enabled: boolean): Read<Setup> {
  return useRead({ queryKey: settingsKeys.setup, queryFn: ({ signal }) => readSetup(signal), enabled, refetchOnMount: 'always' });
}

export function useShim(enabled: boolean): Read<Shim> {
  return useRead({ queryKey: settingsKeys.shim, queryFn: ({ signal }) => readShim(signal), enabled, refetchOnMount: 'always' });
}

/// The push state, read when the Notifications page opens (ST-C84).
export function usePush(): Read<Push> {
  return useRead({ queryKey: settingsKeys.push, queryFn: ({ signal }) => readPush(signal), refetchOnMount: 'always' });
}

/// Re-read what a write changed, and wait for it: a switch shows the state read back after it,
/// never one flipped in advance (ST-C13). Each read settles on its own; a failed re-read keeps
/// what was there, held and aged as any read is.
export function useReread(): (...keysToRead: QueryKey[]) => Promise<void> {
  const client = useQueryClient();
  return async (...keysToRead) => {
    await Promise.all(keysToRead.map((queryKey) => client.invalidateQueries({ queryKey }, { throwOnError: false })));
  };
}
