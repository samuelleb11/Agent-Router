// New for agent-router: the Overview's reads (assets/index.html, `refresh`, `tick`, `renderAuth`),
// typed by hand from the router's handlers (src/api.rs, src/quota.rs, src/auth.rs), which have no
// OpenAPI spec to generate them from. Every read goes through ~/data/http, so each has its timeout
// and a refusal reads the same way everywhere.
//
// Where the page broke its own rules, the rules are kept:
// - a scope change mid-refresh let the older reply paint last; here the read is keyed by scope, a
//   superseded read is cancelled, and only the current scope's answer is ever drawn (OV-C162);
// - no read had a timeout, so one that never answered froze the page under "live" (OV-C165);
// - an /api/accounts failure failed the whole cycle; here the accounts and the mode are read beside
//   the snapshot and fail on their own (OV-C166);
// - a kept quota or forecast was reused whatever the account or the range had become; here a kept
//   reading is shown only for the account and the range it was read for (OV-C65).

import { useEffect, useState, useSyncExternalStore } from 'react';
import { useQuery } from '@tanstack/react-query';
import { keys } from '~/app/query';
import { getJson } from '~/data/http';
import { useRead, type Read } from '~/data/held';
import type { MachineRef, ProjectRow } from '~/data/router';
import { POLL_MS } from '~/lib/freshness';
import type { Forecast, WindowKey } from '~/model/forecast';
import { sinceMs, type Level, type OverviewTab, type Scope } from '~/model/scope';
import type { Account, Auth, Provider } from '~/model/settings';

// ---------------------------------------------------------------- the router's answers

/// `totals` of `GET /api/summary` (src/api.rs, `summary_of`): every figure in scope, from one
/// statement. A count the store could not total arrives as null, never as a zero it did not count.
export interface Totals {
  readonly requests: number | null;
  readonly prompt_tokens: number | null;
  readonly cost: number | null;
  /// Uncached input: what was paid in full.
  readonly input_tokens: number | null;
  readonly output_tokens: number | null;
  readonly thinking_tokens: number | null;
  readonly cache_read_tokens: number | null;
  readonly cache_write_5m_tokens: number | null;
  readonly cache_write_1h_tokens: number | null;
  readonly cache_write_tokens: number | null;
  readonly errors: number | null;
  readonly subagent_requests: number | null;
  readonly sessions: number | null;
  readonly projects: number | null;
  /// Epoch milliseconds; null when nothing is recorded in scope.
  readonly first_seen: number | null;
  readonly last_seen: number | null;
  /// The same figures split by the provider that served them, Anthropic's first, one row per
  /// provider with requests in scope; a request recorded before providers is Anthropic's (OV-C226).
  readonly by_provider: readonly ProviderTotals[];
}

/// One provider's part of the totals (src/api.rs, `summary_of`, `by_provider`).
export interface ProviderTotals {
  readonly provider: Provider;
  readonly requests: number | null;
  readonly prompt_tokens: number | null;
  readonly cost: number | null;
  /// Requests with usage the router could not price, which add nothing to `cost`.
  readonly unpriced: number | null;
  /// Requests whose usage the router estimated rather than was told.
  readonly estimated: number | null;
  readonly output_tokens: number | null;
  readonly cache_read_tokens: number | null;
  readonly cache_write_tokens: number | null;
}

/// A provider's serving account: its active one, the one signing its new requests.
export interface ServingAccount {
  readonly id: string;
  readonly label: string | null;
  readonly email: string | null;
  readonly org_name?: string | null;
  readonly plan: string | null;
}

/// One window of a provider's block. Utilisation is a fraction, 0–1, and 1 for a window the provider
/// said is spent; a reset is epoch seconds.
export interface BlockWindow {
  readonly utilization: number | null;
  readonly resets_at: number | null;
}

/// One bucket of a provider's quota history: the worst reading in it for each of its windows, by the
/// window's own name. Fractions, 0–1.
export interface BlockHistoryRow {
  /// The bucket's start, epoch milliseconds, on the viewer's clock.
  readonly t: number;
  readonly five_hour?: number | null;
  readonly seven_day?: number | null;
  readonly month?: number | null;
}

/// One entry of the summary's `providers` (src/api.rs `providers_quota`): a provider with an account,
/// Anthropic's first, with its serving account and that account's windows — Anthropic's 5-hour and
/// 7-day from the rate-limit headers its requests carry, Kimi Code's 5-hour and its second by its own
/// name, `month` or `seven_day`, from its usage API — their status, when they were read, and their
/// history over the timeline's buckets. A provider with no account has no entry (OV-C224).
export interface ProviderQuota {
  readonly provider: Provider;
  readonly account: ServingAccount;
  readonly windows: Readonly<Partial<Record<WindowKey, BlockWindow | null>>>;
  /// The last reading's status: Anthropic's header's own, Kimi Code's "allowed" or "rejected".
  readonly status: string | null;
  /// Epoch milliseconds of the reading: the request that carried the headers, or the usage read.
  readonly at: number | null;
  readonly history: readonly BlockHistoryRow[];
}

/// One part of the context window, summed over the scope. Estimated tokens, by character share.
export interface ContextRow {
  /// tools | system | history | tool_results
  readonly component: string;
  readonly est_tokens: number | null;
  readonly chars: number | null;
  /// Priced as if paid once as fresh input: a floor (src/api.rs).
  readonly est_cost_once: number | null;
  /// How many requests carried this part.
  readonly requests: number | null;
}

export interface ModelRow {
  readonly model: string;
  /// The provider that served it: `anthropic`, `kimi` (OV-C231).
  readonly provider?: Provider | null;
  readonly requests: number | null;
  readonly prompt_tokens: number | null;
  readonly cost: number | null;
  readonly output_tokens: number | null;
  readonly thinking_tokens: number | null;
  readonly cache_read_tokens: number | null;
  readonly errors: number | null;
  readonly subagent_requests: number | null;
}

export interface ThreadRow {
  readonly thread: 'main' | 'subagent';
  readonly requests: number | null;
  readonly prompt_tokens: number | null;
  readonly cost: number | null;
  readonly output_tokens: number | null;
  readonly cache_read_tokens: number | null;
  /// Distinct agent ids.
  readonly agents: number | null;
}

/// Milliseconds.
export interface Latency {
  readonly ttfb_p50: number | null;
  readonly ttfb_p95: number | null;
  readonly duration_p50: number | null;
  readonly duration_p95: number | null;
  readonly duration_max: number | null;
}

/// One account's traffic in the range. Never narrowed by project or session (it is how an account
/// is picked).
export interface AccountSplitRow {
  readonly id: string;
  /// Its email, else its label, else its id (src/api.rs).
  readonly label: string | null;
  /// Its own provider, else the one its requests were signed for (OV-C231).
  readonly provider?: Provider | null;
  /// Signs its provider's new requests: one account per provider can (OV-C231).
  readonly active: number | boolean | null;
  readonly requests: number | null;
  readonly prompt_tokens: number | null;
  readonly cost: number | null;
  readonly output_tokens: number | null;
  readonly sessions: number | null;
  readonly last_seen: number | null;
}

/// One model's requests in one bucket. Two models in a bucket are two rows.
export interface TimelineRow {
  /// The bucket's start, epoch milliseconds, on the viewer's clock.
  readonly t: number;
  readonly model: string | null;
  readonly requests: number | null;
  readonly prompt_tokens: number | null;
  readonly cost: number | null;
  readonly output_tokens: number | null;
  readonly cache_read_tokens: number | null;
  readonly subagent_tokens: number | null;
  readonly errors: number | null;
}

export interface Timeline {
  /// The span drawn: the range when one was asked for, the data's own otherwise; null with
  /// nothing in either.
  readonly start: number | null;
  readonly end: number | null;
  /// The router's bucket width; 0 with nothing to bucket.
  readonly bucket_ms: number;
  readonly rows: readonly TimelineRow[];
}

/// `GET /api/summary`, asked for by part (`parts=`, SV-C45): the parts the top of the Overview
/// shows are always asked for; the others only when a tab or a crumb shows them (OV-C216). The
/// router's `quota`, `account` and `quota_history` are Anthropic's alone, and the phones' to read:
/// the console reads `providers`, a block per provider (OV-C224).
export interface Summary {
  readonly totals: Totals;
  /// Requests in scope that no session holds, counted by the router (OV-C141). Comes with totals.
  readonly unattributed: number | null;
  readonly models: readonly ModelRow[];
  readonly latency: Latency | null;
  readonly timeline: Timeline;
  /// Each provider with an account, Anthropic's first, for Quota headroom's blocks (OV-C224).
  readonly providers: readonly ProviderQuota[];
  /// For the Context tab.
  readonly context?: readonly ContextRow[];
  readonly threads?: readonly ThreadRow[];
  /// For the Projects & sessions tab, and for the crumb of a scoped account.
  readonly by_account?: readonly AccountSplitRow[];
  /// Never asked for by the console; the phones read it.
  readonly stop_reasons?: readonly { readonly stop_reason: string; readonly requests: number }[];
}

/// The summary's parts, in the order the router answers them (src/api.rs `SUMMARY_PARTS`).
export const SUMMARY_PARTS = ['totals', 'quota', 'account', 'context', 'models', 'threads', 'latency', 'stop_reasons', 'by_account', 'timeline', 'quota_history', 'providers'] as const;
export type SummaryPart = (typeof SUMMARY_PARTS)[number];

/// What the top of the Overview shows whatever tab is open: Quota headroom, a block per provider
/// (OV-C224), All accounts and Totals in scope, with the timeline their sparklines and history are
/// bucketed on.
const TOP_PARTS: readonly SummaryPart[] = ['totals', 'models', 'latency', 'timeline', 'providers'];

/// One row of `GET /api/sessions`: the 200 most recently seen sessions in scope, and no more.
export interface SessionRow {
  readonly id: string;
  /// The session's project key: its repository, else its directory, else null (`ProjectRow`).
  readonly project: string | null;
  /// The directory it ran in, the repository that is a checkout of, and the machine it ran on (null
  /// on a local router).
  readonly cwd: string | null;
  readonly repository: string | null;
  readonly machine: MachineRef | null;
  readonly git_branch: string | null;
  readonly entrypoint: string | null;
  /// The client its requests came from, as the router reads their User-Agent (src/analyze.rs,
  /// `client_of`): `claude-code-cli`, `claude-code-vscode`, `kimi-code-cli`, `kimi-code-web`,
  /// `kimi-code-vscode`; absent for any other caller (SE-C234).
  readonly client?: string | null;
  readonly started_at: number | null;
  readonly last_seen_at: number | null;
  readonly requests: number | null;
  readonly prompt_tokens: number | null;
  readonly cost: number | null;
  readonly output_tokens: number | null;
  readonly cache_read_tokens: number | null;
  readonly subagent_tokens: number | null;
  readonly errors: number | null;
  readonly subagents: number | null;
  /// The models seen, comma-separated.
  readonly models: string | null;
  /// The providers its requests went to, Anthropic's first: one session can move between them
  /// through `/model` (SE-C242).
  readonly providers?: readonly Provider[] | null;
  readonly first_request_at: number | null;
  readonly last_request_at: number | null;
}

/// What a tool's results put in front of the model, estimated.
export interface ToolOutputRow {
  readonly name: string | null;
  readonly results: number | null;
  readonly chars: number | null;
  readonly est_tokens: number | null;
  readonly max_tokens: number | null;
  readonly est_cost_once: number | null;
  readonly errors: number | null;
}

/// What the model asked for, counted.
export interface ToolCallRow {
  readonly name: string | null;
  readonly calls: number | null;
  readonly requests: number | null;
  readonly subagent_calls: number | null;
}

export interface Tools {
  readonly produced: readonly ToolOutputRow[];
  readonly called: readonly ToolCallRow[];
}

/// One row of `GET /api/requests`: the 300 newest in scope.
export interface RequestRecord {
  readonly id: number;
  readonly request_id: string | null;
  readonly session_id: string | null;
  readonly project: string | null;
  readonly model: string | null;
  /// The provider and the account that served it; a request recorded before providers is
  /// Anthropic's (OV-C231).
  readonly provider?: Provider | null;
  readonly account_id?: string | null;
  readonly status: number | null;
  readonly ttfb_ms: number | null;
  readonly duration_ms: number | null;
  readonly stop_reason: string | null;
  readonly is_sidechain: number | boolean | null;
  readonly agent_id: string | null;
  readonly started_at: number;
  /// The first 200 characters of the error recorded.
  readonly error: string | null;
  readonly prompt_tokens: number | null;
  readonly input_tokens: number | null;
  readonly output_tokens: number | null;
  readonly thinking_tokens: number | null;
  readonly cost: number | null;
  readonly cache_read_tokens: number | null;
  readonly cache_write_tokens: number | null;
  /// The tools called, in the order first called, comma-separated, repeats included.
  readonly tools: string | null;
  /// `component:est_tokens` pairs, comma-separated.
  readonly context: string | null;
}

/// One window of `GET /api/quota` (src/quota.rs). Utilisation is a percentage, 0–100, unlike the
/// headers' fraction; the reset is an ISO date from the usage API, or "@<epoch seconds>" from stored
/// headers (OV-C183).
export interface LiveWindow {
  readonly utilization: number | null;
  readonly resets_at: string | null;
}

/// `GET /api/quota`: Anthropic's serving account's usage, polled from Anthropic, or the last headers
/// when that is unavailable. A 200 carrying `error` is a refusal, which ~/data/http throws: on a
/// router with no Anthropic account, the read fails on its own and the page reads on (OV-C224).
export interface LiveQuota {
  readonly five_hour?: LiveWindow | null;
  readonly seven_day?: LiveWindow | null;
  /// normal | warning | critical: the one field the status tag reads (OV-C59).
  readonly severity?: string | null;
  /// Epoch milliseconds of the request the headers came from, when they are headers.
  readonly seen_at?: number | null;
  readonly account_id?: string | null;
  readonly source?: 'usage-api' | 'response-headers' | 'unavailable' | (string & {}) | null;
  readonly detail?: string | null;
  /// How old the router's cached reading is.
  readonly age_ms?: number | null;
}

// ---------------------------------------------------------------- one snapshot

/// A reading the snapshot may carry from an earlier cycle: optional reads failing on their own
/// never blank their panel (SH-C12, OV-C64), and what is kept says how old it is (OV-C173).
export interface Reading<T> {
  readonly value: T;
  /// When it was read.
  readonly at: number;
  /// Carried over from an earlier read, because this cycle's failed.
  readonly kept: boolean;
  /// What it was read for: Anthropic's serving account for a quota, the range for a forecast
  /// (OV-C65).
  readonly for: string;
}

export type Outcome<T> = { readonly ok: true; readonly value: T } | { readonly ok: false };

/// This cycle's optional reading, or the last good one when this cycle's failed and it was read for
/// the same thing — the same serving account, the same range — else nothing (OV-C64, OV-C65).
export function keepReading<T>(fresh: Outcome<T>, previous: Reading<T> | null, forKey: string, at: number): Reading<T> | null {
  if (fresh.ok) return { value: fresh.value, at, kept: false, for: forKey };
  return previous && previous.for === forKey ? { ...previous, kept: true } : null;
}

/// The open tab's own figures, read in the same cycle as the top (OV-C216). Activity draws from the
/// top's timeline, and Search asks the router only when typed to, so neither reads anything here.
export type TabPart =
  | { readonly tab: 'activity' }
  | { readonly tab: 'search' }
  | { readonly tab: 'context'; readonly context: readonly ContextRow[]; readonly threads: readonly ThreadRow[] }
  | { readonly tab: 'projects'; readonly byAccount: readonly AccountSplitRow[]; readonly projects: readonly ProjectRow[]; readonly sessions: readonly SessionRow[] }
  | { readonly tab: 'tools'; readonly tools: Tools }
  | { readonly tab: 'requests'; readonly requests: readonly RequestRecord[] };

/// Every figure on screen, from one cycle of reads scoped alike, so the tiles, the charts and the open
/// tab agree (OV-C160): the top, whatever tab is open, and the open tab's own (OV-C216).
export interface Snapshot {
  /// What it was read for.
  readonly scope: Scope;
  /// When the cycle that read it finished.
  readonly at: number;
  /// The lower bound it asked for, measured back from the read (OV-C26).
  readonly since: number | null;
  /// The viewer's UTC offset in minutes it asked with, which its buckets are keyed on (OV-C111).
  readonly tz: number;
  readonly summary: Summary;
  /// The scoped session's row, for its crumb's branch (OV-C13); null with no session scoped, and for
  /// one past the 200 most recent the router lists.
  readonly session: SessionRow | null;
  readonly quota: Reading<LiveQuota> | null;
  readonly forecast: Reading<Forecast> | null;
  readonly part: TabPart;
}

/// A query string from the parameters that are set: an absent level is not sent.
export function query(params: Readonly<Record<string, string | number | null | undefined>>): string {
  const p = new URLSearchParams();
  for (const [k, v] of Object.entries(params)) if (v !== null && v !== undefined && v !== '') p.set(k, String(v));
  const s = p.toString();
  return s ? `?${s}` : '';
}

/// The summary's parts a cycle asks for: the top's, the open tab's, and by_account while an account is
/// scoped, for its crumb's label (OV-C13, OV-C216).
export function summaryParts(scope: Scope, tab: OverviewTab): SummaryPart[] {
  const wanted = new Set<SummaryPart>(TOP_PARTS);
  if (tab === 'context') {
    wanted.add('context');
    wanted.add('threads');
  }
  if (tab === 'projects' || scope.account) wanted.add('by_account');
  return SUMMARY_PARTS.filter((p) => wanted.has(p));
}

/// The paths a cycle may read, each with the scope the router narrows it by. The provider narrows
/// every one the scope does (OV-C227). The navigators are never narrowed by the level they pick
/// (OV-C131): projects by the account and the range, sessions by the account, the project and the
/// range. The quota is Anthropic's serving account's whatever the scope (OV-C54), and the forecast
/// covers every account, its usage over the range (OV-C27, OV-C87).
export function snapshotPaths(scope: Scope, tab: OverviewTab, since: number | null, tz: number) {
  const { provider, account, project, session } = scope;
  return {
    summary: `/api/summary${query({ provider, account, project, session, since, tz, parts: summaryParts(scope, tab).join(',') })}`,
    projects: `/api/projects${query({ provider, account, since })}`,
    sessions: `/api/sessions${query({ provider, account, project, since })}`,
    tools: `/api/tools${query({ provider, account, project, session, since })}`,
    requests: `/api/requests${query({ provider, account, project, session, since })}`,
    quota: '/api/quota',
    forecast: `/api/forecast${query({ since })}`,
  } as const;
}

export interface Kept {
  readonly quota: Reading<LiveQuota> | null;
  readonly forecast: Reading<Forecast> | null;
}

/// One cycle: the top's answers and the open tab's, read together and drawn whole or not at all
/// (OV-C160), and nothing for a tab that is not open (OV-C216). The sessions are read once, for the
/// Projects & sessions tab and a scoped session's crumb alike. A core read failing fails the cycle;
/// the quota and the forecast are optional and keep their last good value (SH-C12, OV-C64). The
/// viewer's UTC offset goes with the summary, so a day bar starts at the viewer's midnight (OV-C111).
export async function readSnapshot(
  scope: Scope,
  tab: OverviewTab,
  { signal, now = Date.now(), tz = -new Date(now).getTimezoneOffset(), kept = { quota: null, forecast: null } }: { signal?: AbortSignal; now?: number; tz?: number; kept?: Kept } = {},
): Promise<Snapshot> {
  const since = sinceMs(scope.range, now);
  const paths = snapshotPaths(scope, tab, since, tz);
  const read = <T>(path: string) => getJson<T>(path, { signal });
  const optional = <T>(path: string): Promise<Outcome<T>> =>
    read<T>(path).then(
      (value): Outcome<T> => ({ ok: true, value }),
      (error: unknown): Outcome<T> => {
        // A cancelled cycle is not an optional read failing: hand the cancellation on.
        if (signal?.aborted) throw error;
        return { ok: false };
      },
    );
  const [summary, sessions, projects, tools, requests, quota, forecast] = await Promise.all([
    read<Summary>(paths.summary),
    tab === 'projects' || scope.session ? read<SessionRow[]>(paths.sessions) : null,
    tab === 'projects' ? read<ProjectRow[]>(paths.projects) : null,
    tab === 'tools' ? read<Tools>(paths.tools) : null,
    tab === 'requests' ? read<RequestRecord[]>(paths.requests) : null,
    optional<LiveQuota>(paths.quota),
    optional<Forecast>(paths.forecast),
  ]);
  const part = ((): TabPart => {
    switch (tab) {
      case 'context':
        return { tab, context: summary.context ?? [], threads: summary.threads ?? [] };
      case 'projects':
        return { tab, byAccount: summary.by_account ?? [], projects: projects ?? [], sessions: sessions ?? [] };
      case 'tools':
        return { tab, tools: tools ?? { produced: [], called: [] } };
      case 'requests':
        return { tab, requests: requests ?? [] };
      default:
        return { tab };
    }
  })();
  const at = Date.now();
  return {
    scope,
    at,
    since,
    tz,
    summary,
    session: (scope.session && sessions?.find((x) => x.id === scope.session)) || null,
    // Kept only for the Anthropic account it was read for (OV-C65): `/api/quota` reads Anthropic's.
    quota: keepReading(quota, kept.quota, (summary.providers ?? []).find((b) => b.provider === 'anthropic')?.account.id ?? '', at),
    forecast: keepReading(forecast, kept.forecast, scope.range, at),
    part,
  };
}

// ---------------------------------------------------------------- the poll

/// Five failed cycles in a row and the router is called unreachable (SH-C11, OV-C164).
export const UNREACHABLE_AFTER = 5;

export type Freshness = 'connecting' | 'live' | 'stale' | 'unreachable';

export interface PollState {
  /// Failed cycles in a row; a good one sets it back to zero. Cycles, never HTTP attempts.
  readonly strikes: number;
  /// When the last good cycle finished, or null when none has.
  readonly lastOk: number | null;
}

/// The poll's own account of itself, kept outside React so the read, which runs outside a render,
/// can keep it, and every render reads one value of it.
export function createPoll() {
  let state: PollState = { strikes: 0, lastOk: null };
  let kept: Kept = { quota: null, forecast: null };
  const listeners = new Set<() => void>();
  const put = (next: PollState) => {
    state = next;
    for (const l of [...listeners]) l();
  };
  return {
    subscribe(l: () => void) {
      listeners.add(l);
      return () => {
        listeners.delete(l);
      };
    },
    snapshot: (): PollState => state,
    kept: (): Kept => kept,
    /// One cycle, counted: a good one clears the strikes, a failed one adds one, and a cancelled
    /// one — a scope that changed under it — is neither.
    async cycle(read: () => Promise<Snapshot>, signal?: AbortSignal): Promise<Snapshot> {
      try {
        const s = await read();
        if (signal?.aborted) throw new DOMException('superseded', 'AbortError');
        kept = { quota: s.quota ?? kept.quota, forecast: s.forecast ?? kept.forecast };
        put({ strikes: 0, lastOk: s.at });
        return s;
      } catch (error) {
        if (!signal?.aborted) put({ ...state, strikes: state.strikes + 1 });
        throw error;
      }
    },
  };
}

export type Poll = ReturnType<typeof createPoll>;

/// What the strikes mean: connecting until anything has answered, live while the last cycle was
/// good, stale after a failed one, unreachable after five in a row (OV-C164).
export function freshnessOf(state: PollState, drawn: boolean): Freshness {
  if (state.strikes >= UNREACHABLE_AFTER) return 'unreachable';
  if (state.strikes > 0) return 'stale';
  return drawn ? 'live' : 'connecting';
}

export interface OverviewRead {
  /// What to draw: this scope and tab's latest snapshot, else the previous one while this one is read
  /// (OV-C163); null until anything has been read. Its `part` says which tab it holds figures for.
  readonly snapshot: Snapshot | null;
  /// The figures drawn belong to another scope than the crumbs, because this one is still being read.
  readonly pending: boolean;
  readonly freshness: Freshness;
  readonly strikes: number;
  readonly lastOk: number | null;
  /// The last cycle's failure, while the last cycle failed.
  readonly error: unknown;
  /// Ask the poll to read now; the next tick is timed from that read (OV-C168).
  readonly refresh: () => void;
}

/// The Overview's one poll: one query per scope and open tab, reading the top and that tab as one
/// snapshot (OV-C160, OV-C216).
///
/// - The next read starts five seconds after the previous one finished: while a read is in flight
///   there is no interval to fire, so reads never overlap or pile up (OV-C161).
/// - A scope change reads at once (a new key) and keeps the previous figures on screen until it
///   answers (OV-C33, OV-C163). The superseded read is cancelled when its query loses its observer,
///   and its answer, landing under another key, is never drawn (OV-C162). A tab change reads at
///   once the same way.
/// - A key left behind is dropped at once (gcTime 0): coming back to a tab or a scope reads it
///   afresh, rather than drawing what was read minutes ago under "read 2s ago" (OV-C173).
/// - Nothing is retried quietly: a failed cycle is a strike (OV-C164), and every read inside it has
///   ~/data/http's timeout, so a hung read fails its cycle rather than stopping the poll (OV-C165).
/// - A tab shown again reads at once (OV-C169); a hidden one does not poll at all, so a chart is
///   handed nothing while it is hidden (OV-C213).
export function useOverview(scope: Scope, tab: OverviewTab): OverviewRead {
  const [poll] = useState(createPoll);
  const state = useSyncExternalStore(poll.subscribe, poll.snapshot, poll.snapshot);
  const q = useQuery({
    queryKey: keys.overview(scope, tab),
    queryFn: ({ signal }) => poll.cycle(() => readSnapshot(scope, tab, { signal, kept: poll.kept() }), signal),
    refetchInterval: (query) => (query.state.fetchStatus === 'fetching' ? false : POLL_MS),
    refetchIntervalInBackground: false,
    refetchOnWindowFocus: true,
    staleTime: 0,
    gcTime: 0,
    retry: false,
  });

  // The last figures drawn, from whichever scope: what stays on screen while a new scope is read.
  const [last, setLast] = useState<Snapshot | null>(null);
  const data = q.data ?? null;
  useEffect(() => {
    if (data) setLast(data);
  }, [data]);
  const snapshot = data ?? last;

  return {
    snapshot,
    pending: snapshot !== null && !sameScope(snapshot.scope, scope),
    freshness: freshnessOf(state, snapshot !== null),
    strikes: state.strikes,
    lastOk: state.lastOk,
    error: state.strikes > 0 ? q.error : null,
    refresh: () => void q.refetch({ cancelRefetch: false }),
  };
}

const sameScope = (a: Scope, b: Scope): boolean =>
  a.provider === b.provider && a.account === b.account && a.project === b.project && a.session === b.session && a.range === b.range;

// ---------------------------------------------------------------- the scope pickers

/// A picker's rows, read when it opens and never on the five-second clock (OV-C217); opened again, it
/// reads afresh, showing meanwhile what the last opening read.
function usePickerRead<T>(queryKey: readonly unknown[], path: () => string, open: boolean) {
  return useQuery({
    queryKey,
    queryFn: ({ signal }) => getJson<T>(path(), { signal }),
    enabled: open,
    staleTime: 0,
    retry: false,
    refetchOnWindowFocus: false,
    refetchOnReconnect: false,
  });
}

/// The three pickers' rows, each read only while its picker is open: the accounts that served in the
/// range, the projects under the account, and the sessions under the account and the project — never
/// narrowed by the level they pick (OV-C131) — each of the chosen provider alone, so the account
/// picker offers only that provider's accounts (OV-C227).
export function usePickers(scope: Scope, open: Level | null) {
  const { provider, account, project, range } = scope;
  const since = () => sinceMs(range);
  return {
    account: usePickerRead<Pick<Summary, 'by_account'>>(keys.picker('account', range, provider), () => `/api/summary${query({ provider, since: since(), parts: 'by_account' })}`, open === 'account'),
    project: usePickerRead<ProjectRow[]>(keys.picker('project', range, provider, account), () => `/api/projects${query({ provider, account, since: since() })}`, open === 'project'),
    session: usePickerRead<SessionRow[]>(
      keys.picker('session', range, provider, account, project),
      () => `/api/sessions${query({ provider, account, project, since: since() })}`,
      open === 'session',
    ),
  };
}

// ---------------------------------------------------------------- beside the snapshot

/// The accounts and the router's mode, every 30 s, beside the snapshot and apart from it: a failure
/// in either leaves the Overview's figures and freshness alone (OV-C166).
export const SETTINGS_EVERY_MS = 30_000;

/// `GET /api/auth`: what kind of router this is, and whether it listens beyond loopback.
export const readAuth = (signal?: AbortSignal) => getJson<Auth>('/api/auth', { signal });

/// `GET /api/accounts`, never with `probe=1`: credentials are checked only while Settings is open,
/// so this read stays cheap (OV-C167).
export const readAccounts = (signal?: AbortSignal) => getJson<Account[]>('/api/accounts', { signal });

/// `GET /api/accounts?probe=1`: every credential checked. Made once per load, from the Overview as the
/// load opened it, and nowhere else outside Settings (OV-C167): it is how a credential that stopped
/// working gets to open Settings by itself (ST-C7).
export const readAccountsChecked = (signal?: AbortSignal) => getJson<Account[]>('/api/accounts?probe=1', { signal });

/// Whether this load has made its one credential check. Per load, so module state: a remount, or a
/// new query client, is not a new load.
let checkedThisLoad = false;

/// The one credential check of a load, asked for while `wanted`; null until it answers, and for
/// every later caller once this load has made it.
export function useAccountsCheck(wanted: boolean): Account[] | null {
  const q = useQuery({
    queryKey: keys.accountsChecked,
    queryFn: ({ signal }) => {
      checkedThisLoad = true;
      return readAccountsChecked(signal);
    },
    enabled: wanted && !checkedThisLoad,
    staleTime: Infinity,
    gcTime: Infinity,
    retry: false,
    refetchOnWindowFocus: false,
    refetchOnReconnect: false,
    refetchOnMount: false,
  });
  return q.data ?? null;
}

export function useAuth(): Read<Auth> {
  return useRead({ queryKey: keys.auth, queryFn: ({ signal }) => readAuth(signal), refetchInterval: SETTINGS_EVERY_MS, staleTime: SETTINGS_EVERY_MS, retry: false });
}

export function useAccounts(): Read<Account[]> {
  return useRead({ queryKey: keys.accounts, queryFn: ({ signal }) => readAccounts(signal), refetchInterval: SETTINGS_EVERY_MS, staleTime: SETTINGS_EVERY_MS, retry: false });
}
