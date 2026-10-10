// New for agent-router: the router's answers, typed by hand from its Rust handlers, because the
// router has no OpenAPI spec to generate them from. One read per endpoint the shell uses.

import { getJson } from '~/data/http';
import type { Auth } from '~/model/settings';

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

/// A machine a hosted router heard from, by its id and its label; a blank label names none (SE-C220).
export interface MachineRef {
  readonly id: string;
  readonly label: string | null;
}

/// A git remote of a project's directory: its name, its URL as configured, and the repository it
/// spells, canonically (Northstar ADR 0028), null when the URL is not one the router can read.
export interface ProjectRemote {
  readonly name: string;
  readonly url: string;
  readonly repository: string | null;
}

/// A directory a project's sessions ran in, on the machine that ran them: null on a local router,
/// which has only its own (OV-C214).
export interface ProjectDirectory {
  readonly cwd: string;
  readonly machine: MachineRef | null;
  readonly remotes: readonly ProjectRemote[];
}

/// One row of `GET /api/projects` (src/api.rs, `projects_inner`): a project is the repository
/// Claude Code ran in, every worktree and clone of it together, else the working directory when
/// that has no git remote, with what went through the router from it. The rows come ordered by
/// prompt tokens, most first.
export interface ProjectRow {
  /// The project's key, which every scoped read takes as `project=`: the repository when its
  /// sessions ran in a git checkout with a remote, else the working directory; null when the
  /// session's directory was never resolved, which the console keys "(unresolved)" (OV-C136).
  readonly project: string | null;
  /// The canonical repository (`gitlab.example.com/acme/ui-kit-web`), null when there is none.
  readonly repository: string | null;
  /// Every directory the project's sessions ran in, each on its machine (OV-C214).
  readonly directories: readonly ProjectDirectory[];
  readonly sessions: number;
  readonly requests: number;
  readonly prompt_tokens: number;
  readonly cost: number;
  /// Requests with usage the router could not price, which `cost` leaves out, and those whose tokens
  /// were estimated rather than read from their stored response (OV-C229, OV-C232).
  readonly unpriced?: number | null;
  readonly estimated?: number | null;
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

/// `GET /api/auth` (src/auth.rs): what kind of router this is. The rail reads it for its `mode`
/// alone, to offer Machines on a hosted router only (MA-C1), under the key Settings and the Overview
/// read the whole answer by, `keys.auth`, so one read serves every reader.
export const readAuth = (signal?: AbortSignal) => getJson<Auth>('/api/auth', { signal });

/// Every project, over all time. The rail is how a project is picked, so it is never narrowed
/// by the scope it sets.
export const readProjects = (signal?: AbortSignal) => getJson<ProjectRow[]>('/api/projects', { signal });
