// New for agent-router: the router's answers, typed by hand from its Rust handlers, because the
// router has no OpenAPI spec to generate them from. One read per endpoint the shell uses.

import { getJson } from '~/data/http';

/// `GET /_router/health` (src/api.rs, `health`). Open to every caller, the LAN included,
/// because the handover probe reads it before it has anything to authenticate with.
export interface Health {
  readonly ok: boolean;
  readonly pid: number;
  readonly version: string;
  /// The source tree the binary was compiled from (build.rs): the crate version is the same
  /// for every build, so it cannot say whether what serves is what was deployed.
  readonly build: string;
}

/// One row of `GET /api/projects` (src/api.rs, `projects_inner`): a project is a working
/// directory Claude Code ran in, with what went through the router from it. The rows come
/// ordered by prompt tokens, most first.
export interface ProjectRow {
  /// The working directory. Null when the session's directory was never resolved.
  readonly project: string | null;
  readonly sessions: number;
  readonly requests: number;
  readonly prompt_tokens: number;
  readonly cost: number;
  readonly output_tokens: number;
  readonly cache_read_tokens: number;
  readonly subagent_tokens: number;
  readonly errors: number;
  /// The models seen, comma-separated, as SQLite's group_concat writes them.
  readonly models: string | null;
  /// Epoch milliseconds.
  readonly first_seen: number | null;
  readonly last_seen: number | null;
}

export const readHealth = (signal?: AbortSignal) => getJson<Health>('/_router/health', { signal });

/// Every project, over all time. The rail is how a project is picked, so it is never narrowed
/// by the scope it sets.
export const readProjects = (signal?: AbortSignal) => getJson<ProjectRow[]>('/api/projects', { signal });
