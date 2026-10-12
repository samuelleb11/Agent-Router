// New for agent-router: the router reads the palette makes, typed by hand from its Rust handlers
// (the router has no OpenAPI spec), each through ~/data/http so a refusal reads the same way
// everywhere. The palette's writes are data in ./hits, sent with `postJson` by the component.

import { getJson } from '~/data/http';

/// What the search may be narrowed to (src/api.rs, `SearchQuery.kind`). Absent is every kind,
/// system prompts included.
export type SearchKind = 'text' | 'tool_use' | 'tool_result' | 'thinking';

/// One place a matching block appeared: a row of `GET /api/search`'s `hits` (src/api.rs,
/// `hits_for`). The same block appears at every place it was replayed, so a hit is its session,
/// its block and its ordinal together (SE-C171).
export interface SearchHit {
  readonly block_id: number;
  /// text | tool_use | tool_result | thinking | system, or whatever the router indexes next.
  readonly kind: string;
  readonly chars: number;
  readonly session_id: string;
  readonly role: string;
  /// Where the block sat: its index in the request's `messages`.
  readonly ordinal: number;
  readonly request_id: number;
  /// Epoch milliseconds.
  readonly at: number;
  /// The session's working directory; null when its transcript never arrived (OV-C52).
  readonly project: string | null;
  readonly session_label: string | null;
  /// The match in context, each matched run between ‹ and ›.
  readonly snippet: string;
  readonly rank: number;
}

/// The same matches rolled up per session (src/api.rs, `sessions_for`), at most 200 of them.
export interface SearchSession {
  readonly session_id: string;
  readonly project: string | null;
  readonly hits: number;
  readonly blocks: number;
}

/// `GET /api/search` (src/api.rs, `search_inner`).
export interface SearchAnswer {
  readonly query: string;
  /// What the router ran: the query as typed, or its retry as plain words when its search
  /// language rejected what was typed.
  readonly expression: string;
  readonly sessions: readonly SearchSession[];
  readonly hits: readonly SearchHit[];
}

/// The router's cap on the per-session roll-up: at the cap, the matches it counts are a floor.
export const SEARCH_SESSIONS_CAP = 200;

export function readSearch(q: string, kind: SearchKind | null, limit: number, signal?: AbortSignal): Promise<SearchAnswer> {
  const params = new URLSearchParams({ q, limit: String(limit) });
  if (kind) params.set('kind', kind);
  return getJson<SearchAnswer>(`/api/search?${params}`, { signal });
}

/// One row of `GET /api/live/sessions` (src/live.rs, `LiveSession`), the fields the palette reads.
export interface LiveSession {
  readonly session_id: string;
  readonly cwd: string | null;
  readonly project: string;
  /// The repository its directory is a checkout of, canonically; absent when it has none (SE-C231).
  readonly repository?: string | null;
  readonly git_branch: string | null;
  /// Claude Code's own name for the session, falling back to what was first asked.
  readonly title: string | null;
  /// Epoch milliseconds of the newest content.
  readonly last_activity_ms: number;
  /// Put away by the reader.
  readonly archived: boolean;
  /// Under the wrapper and answering on its socket: only such a session can be switched.
  readonly wrapped: boolean;
  readonly client?: string | null;
  readonly hosted?: boolean | null;
}

export const readLiveSessions = (signal?: AbortSignal) => getJson<LiveSession[]>('/api/live/sessions', { signal });

/// `GET /api/live/options` (src/live.rs, `options`): what a session can be switched to, one list
/// for every client. Models and efforts are bare values; modes carry the router's labels (SE-C138).
/// `by_provider` is the same models grouped by provider, each with its display name; a router from
/// before there was a second provider sends none (SE-C233).
export interface LiveOptions {
  readonly models: readonly string[];
  readonly by_provider?: readonly {
    readonly provider: string;
    readonly name: string;
    readonly models: readonly { readonly id: string; readonly display_name?: string | null }[];
  }[];
  readonly efforts: readonly string[];
  readonly modes: readonly { readonly value: string; readonly label: string }[];
}

export const readLiveOptions = (signal?: AbortSignal) => getJson<LiveOptions>('/api/live/options', { signal });

/// One row of `GET /api/accounts` (src/api.rs, `accounts_list`), the fields the palette reads.
export interface Account {
  /// The router's own id for the account, which tells apart two rows with one identity (OV-C17).
  readonly id: string;
  readonly label: string;
  readonly email: string | null;
  /// SQLite's integer: 1 for the account that signs its provider's requests now. Each provider has
  /// its own.
  readonly active: number | boolean;
  /// `anthropic` or `kimi`; absent from a router that predates providers.
  readonly provider?: string | null;
}

/// Read only while the commands are on screen: the router checks every account's credential on
/// each read of this list (AGEN-0015).
export const readAccounts = (signal?: AbortSignal) => getJson<Account[]>('/api/accounts', { signal });

/// The palette's keys. The project list's is the rail's (`keys.projects` in ~/app/query), so the
/// two share one read.
export const paletteKeys = {
  search: (q: string, kind: SearchKind | null) => ['search', q, kind ?? 'all'] as const,
  sessions: ['live', 'sessions'] as const,
  options: ['live', 'options'] as const,
  accounts: ['accounts'] as const,
};
