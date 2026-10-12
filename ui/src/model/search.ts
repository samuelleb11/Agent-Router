// New for agent-router: the two searches of the router's conversation index with no DOM — the
// dashboard's "Search the transcripts" (assets/index.html, `runSearch`) and the live view's Find
// (assets/live.html, `runFind`, `drawFind`). Snippets are runs of text, matched or not, never HTML
// (`marks` in ./format).
//
// Where the pages broke their own rules, the rules are kept:
// - a search's session rows and hits scope the page to that session and its project, not to the
//   session under "All projects" (OV-C24);
// - the idle caption says when the scope narrows the search (OV-C44);
// - a failed search reads "Search failed: <the router's words>", not the reply's JSON (OV-C45);
// - the caption and Find say when the router's caps were reached (OV-C47, SE-C165);
// - a Find the router failed says its error, and one that did not reach it says so; neither reads
//   "Nothing matched" (SE-C167).

import { DASH, ago, count, id8, kindWord, marks, plural, projectName, span, tod, type Run } from './format';
import { choose, isScoped, sinceMs, type Scope } from './scope';

/// One session of a search reply (src/api.rs, `search`): the sessions that mention the query,
/// with how many places do.
export interface SearchSession {
  readonly session_id: string;
  readonly project?: string | null;
  readonly session_label?: string | null;
  readonly git_branch?: string | null;
  readonly blocks?: number | null;
  readonly hits?: number | null;
  readonly first_at?: number | null;
  readonly last_at?: number | null;
}

/// One hit, best first. The same block can appear at several places, so a hit is its session,
/// block and position together (OV-C51, SE-C171).
export interface SearchHit {
  readonly block_id: number;
  readonly kind?: string | null;
  readonly chars?: number | null;
  readonly role?: string | null;
  readonly ordinal?: number | null;
  readonly request_id?: number | null;
  readonly at?: number | null;
  readonly session_id: string;
  readonly project?: string | null;
  readonly session_label?: string | null;
  /// The matches wrapped in ‹ ›.
  readonly snippet?: string | null;
  readonly rank?: number | null;
}

export interface SearchReply {
  readonly query?: string | null;
  /// What the router ran, which differs from what was typed when it retried a query its search
  /// language rejected as plain words.
  readonly expression?: string | null;
  readonly sessions?: readonly SearchSession[] | null;
  readonly hits?: readonly SearchHit[] | null;
}

const hitKey = (h: SearchHit): string => `${h.session_id}:${h.block_id}:${h.ordinal ?? ''}`;

// ---------- the Overview's search ----------

export type SearchKind = '' | 'text' | 'tool_use' | 'tool_result' | 'thinking';

/// Typing searches this long after the last keystroke; changing the kind searches at once (OV-C39).
export const SEARCH_DEBOUNCE_MS = 160;

/// The router stops listing sessions at this many (src/api.rs).
export const SEARCH_SESSION_CAP = 200;

/// What a search asks the router, or nothing: a blank query is not a search (OV-C40). The text goes
/// verbatim, trimmed (OV-C43), with the page's whole scope: its provider, account, project, session
/// and range (OV-C44, OV-C227). The range is measured back from now and has no end, so no `until`
/// goes, as none goes with any other read.
export function searchParams(text: string, kind: SearchKind, scope: Scope, now: number = Date.now()): Record<string, string> | null {
  const q = text.trim();
  if (!q) return null;
  const out: Record<string, string> = { q };
  if (kind) out['kind'] = kind;
  if (scope.provider) out['provider'] = scope.provider;
  if (scope.account) out['account'] = scope.account;
  if (scope.project) out['project'] = scope.project;
  if (scope.session) out['session'] = scope.session;
  const since = sinceMs(scope.range, now);
  if (since != null) out['since'] = String(since);
  return out;
}

/// Whether the page's scope narrows the search: any of its levels, its provider or its range.
const narrowed = (scope: Scope): boolean => isScoped(scope) || scope.range !== 'all';

/// The caption while the box is empty: every message, tool call and result ever captured — or, when
/// the page's scope narrows the search, that it does (OV-C36, OV-C44, OV-C227).
export function searchIdle(scope: Scope): string {
  return narrowed(scope) ? 'every message, tool call and result captured in the current scope' : 'every message, tool call and result ever captured';
}

/// A search that failed, in the router's own words (OV-C45).
export const searchFailed = (message: string): string => `Search failed: ${message}`;

export interface SearchSessionRow {
  readonly id: string;
  readonly name: string;
  readonly project: string;
  readonly hits: number;
  readonly meta: string;
  readonly selected: boolean;
  /// What pressing the row scopes the page to: that session and its project, or, pressed again,
  /// the session level cleared (OV-C23, OV-C24).
  readonly scope: Scope;
}

export interface SearchHitRow {
  readonly key: string;
  readonly kind: string;
  readonly role: string;
  readonly project: string;
  readonly session: string;
  readonly at: string;
  readonly snippet: readonly Run[];
  /// What pressing the hit scopes the page to (OV-C24, OV-C48).
  readonly scope: Scope;
}

export interface SearchView {
  readonly caption: string;
  readonly sessionsHeading: string;
  readonly sessions: readonly SearchSessionRow[];
  readonly hitsHeading: string;
  readonly hits: readonly SearchHitRow[];
  /// Said where the hits go when there are none.
  readonly empty: string | null;
}

/// The results of a search: the sessions that mention it, a row each sized by its matching places,
/// then the best matches (OV-C46), under a caption of what came back and how long it took (OV-C47).
export function searchView(reply: SearchReply, query: string, scope: Scope, tookMs: number, now: number = Date.now()): SearchView {
  const sessions = reply.sessions ?? [];
  const hits = reply.hits ?? [];
  const took = `${Math.round(tookMs)} ms`;
  const capped = sessions.length >= SEARCH_SESSION_CAP ? ', the most the router lists' : '';
  return {
    caption: sessions.length ? `${plural(sessions.length, 'session', 'sessions')}${capped} · ${took}` : `nothing matched · ${took}`,
    sessionsHeading: sessions.length ? 'Sessions that mention it — click one to scope the page' : '',
    sessions: sessions.map((s) => ({
      id: s.session_id,
      name: id8(s.session_id),
      project: s.project ?? '',
      hits: s.hits ?? 0,
      meta: [plural(s.blocks, 'message', 'messages'), ago(s.last_at, now)].join(' · '),
      selected: scope.session === s.session_id,
      scope: scope.session === s.session_id ? choose(scope, 'session', null) : choose(scope, 'session', s.session_id, s.project ?? null),
    })),
    hitsHeading: hits.length ? `Best matches — ${count(hits.length)} shown` : '',
    hits: hits.map((h) => ({
      key: hitKey(h),
      kind: kindWord(h.kind),
      role: h.role ?? '',
      project: projectName(h.project),
      session: id8(h.session_id),
      at: tod(h.at, now),
      snippet: marks(h.snippet),
      scope: choose(scope, 'session', h.session_id, h.project ?? null),
    })),
    empty: hits.length ? null : `Nothing matched “${query}”${narrowed(scope) ? ' in the current scope' : ''}.`,
  };
}

// ---------- the live view's Find ----------

/// Typing finds this long after the last keystroke (SE-C164).
export const FIND_DEBOUNCE_MS = 200;
/// Find asks for this many hits, best match first (SE-C165).
export const FIND_LIMIT = 30;

export type FindScope = 'session' | 'all';

/// The two scopes, always visible; "This session" is disabled with no session open (SE-C162).
export function findScopes(openId: string | null): { readonly value: FindScope; readonly label: string; readonly disabled: boolean }[] {
  return [
    { value: 'session', label: 'This session', disabled: !openId },
    { value: 'all', label: 'Everywhere', disabled: false },
  ];
}

/// The field says what it searches (SE-C163).
export const findPlaceholder = (scope: FindScope, openId: string | null): string =>
  scope === 'session' && openId ? 'Find anywhere in this session' : 'Find in every session';

/// What Find asks the router, or nothing for an empty query (SE-C164).
export function findParams(text: string, scope: FindScope, openId: string | null): Record<string, string> | null {
  const q = text.trim();
  if (!q) return null;
  return scope === 'session' && openId ? { q, limit: String(FIND_LIMIT), session: openId } : { q, limit: String(FIND_LIMIT) };
}

export interface FindHit {
  readonly key: string;
  readonly kind: string;
  readonly role: string;
  readonly age: string;
  readonly snippet: readonly Run[];
  /// The session a press opens, or none: a hit in the session already open is not a link, since
  /// the feed cannot scroll to a turn it never loaded (SE-C172).
  readonly opens: string | null;
  /// Where a hit from another session is: its project ("—" when unknown) and its id (SE-C169).
  readonly where: string | null;
}

export interface FindView {
  /// Said in place of hits: no match, a failure.
  readonly note: string | null;
  /// "Searched for <expression>", only when the router ran something other than what was typed
  /// (SE-C168).
  readonly searched: string | null;
  /// Said when the router counts more matches than came back (SE-C165).
  readonly more: string | null;
  readonly hits: readonly FindHit[];
}

export type FindResult = { readonly reply: SearchReply } | { readonly unreachable: true } | { readonly failed: string };

/// Find's results. No match says so (SE-C166); a search that did not reach the router, or one the
/// router failed, says that and never "Nothing matched" (SE-C167).
export function findView(result: FindResult, query: string, scope: FindScope, openId: string | null, now: number = Date.now()): FindView {
  const none = { searched: null, more: null, hits: [] };
  if ('unreachable' in result) return { ...none, note: 'Could not reach the router.' };
  if ('failed' in result) return { ...none, note: result.failed };
  const d = result.reply;
  const hits = d.hits ?? [];
  if (!hits.length) return { ...none, note: `Nothing matched ${query}${scope === 'session' && openId ? ' in this session.' : '.'}` };
  const total = (d.sessions ?? []).reduce((a, s) => a + (s.hits ?? 0), 0);
  return {
    note: null,
    searched: d.expression && d.expression !== query ? `Searched for ${d.expression}` : null,
    more: total > hits.length ? `${count(hits.length)} best of ${count(total)} matches shown` : null,
    hits: hits.map((h) => {
      const elsewhere = !openId || h.session_id !== openId;
      return {
        key: hitKey(h),
        kind: kindWord(h.kind),
        role: h.role ?? '',
        age: typeof h.at === 'number' ? span(now - h.at) : DASH,
        snippet: marks(h.snippet),
        opens: elsewhere ? h.session_id : null,
        where: elsewhere ? `${h.project ? projectName(h.project) : DASH} · ${id8(h.session_id)}` : null,
      };
    }),
  };
}

