// New for agent-router: the dashboard's scope (assets/index.html: `state`, `RANGES`, `sinceMs`,
// `writeHash`, `setScope`, `renderCrumbs`, `menuRows` and the lists' click handlers) as
// values. Three nested levels — account › project › session — and a time range (OV-C12), kept in
// the address so a scoped view is a link (OV-C29, SH-C53).
//
// Where the page broke its own rule, the rule is kept:
// - leaving a level does the same from its picker as from its list: "All accounts" clears the
//   account and keeps the project and the session, as pressing the selected account does (OV-C23);
// - scoping to a session from anywhere gives that session and its project (OV-C24): see
//   `choose`, which the search's rows use too;
// - the account picker's filter says what it filters (OV-C18);
// - the session picker says when its list stops at the 200 most recent sessions (OV-C34).
// A scope change pushes a history entry rather than replacing one (OV-C30): the address is written
// by `writeQuery`, and pushed or replaced by the screen. The dashboard's own `#a=&p=&s=&r=&o=` is not
// read: the clean cut keeps no compatibility code (decision J), so an old bookmark opens unscoped.

import { DASH, count, id8, projectName } from './format';

export const RANGES = { all: null, '1h': 3_600_000, '24h': 86_400_000, '7d': 7 * 86_400_000, '30d': 30 * 86_400_000 } as const;

export type Range = keyof typeof RANGES;

/// The time range's choices, in the order the switcher shows them (OV-C26).
export const RANGE_ORDER: readonly Range[] = ['all', '1h', '24h', '7d', '30d'];

export interface Scope {
  readonly account: string | null;
  readonly project: string | null;
  readonly session: string | null;
  readonly range: Range;
}

/// Nothing narrowed, every range: where the Overview starts.
export const UNSCOPED: Scope = { account: null, project: null, session: null, range: 'all' };

/// A range named in a link reads as itself when it is one of the five, and as all otherwise —
/// never as a property every object has, `constructor` or `__proto__` (OV-C28).
export function parseRange(v: unknown): Range {
  return typeof v === 'string' && Object.hasOwn(RANGES, v) ? (v as Range) : 'all';
}

/// The lower bound a read asks for, measured back from the moment of the read, so a relative range
/// slides with the clock (OV-C26); none for all.
export function sinceMs(range: Range, now: number = Date.now()): number | null {
  const span = RANGES[parseRange(range)];
  return span == null ? null : now - span;
}

export type Level = 'account' | 'project' | 'session';

/// Choosing a level clears the levels below it and keeps those above: an account clears the
/// project and the session, a project the session, and a session also sets its own project, when
/// it has one (OV-C22, OV-C24). `null` leaves the level, the same from a picker as from its list
/// (OV-C23): the account alone, the project with its session, the session alone.
export function choose(scope: Scope, level: Level, value: string | null, sessionProject?: string | null): Scope {
  switch (level) {
    case 'account':
      return value == null ? { ...scope, account: null } : { ...scope, account: value, project: null, session: null };
    case 'project':
      return { ...scope, project: value, session: null };
    case 'session':
      return value == null ? { ...scope, session: null } : { ...scope, session: value, project: sessionProject || scope.project };
  }
}

/// Pressing a row: its level is chosen, or, when it is the row already chosen, left (OV-C23).
export function toggle(scope: Scope, level: Level, value: string, sessionProject?: string | null): Scope {
  return scope[level] === value ? choose(scope, level, null) : choose(scope, level, value, sessionProject);
}

/// Whether anything is narrowed, which is exactly when "Clear scope" shows (OV-C25).
export const isScoped = (scope: Scope): boolean => !!(scope.account || scope.project || scope.session);

/// "Clear scope": all three levels, keeping the time range (OV-C25).
export const clearScope = (scope: Scope): Scope => ({ ...scope, account: null, project: null, session: null });

export const setRange = (scope: Scope, range: unknown): Scope => ({ ...scope, range: parseRange(range) });

// ---------- the address ----------

export type RequestKind = 'all' | 'errors' | 'subagents' | 'slow';

/// The request filters, which live in the address with the scope (OV-C31).
export interface RequestFilters {
  readonly kind: RequestKind;
  readonly model: string | null;
  readonly stop: string | null;
}

export interface OverviewAddress {
  readonly scope: Scope;
  /// The request rows open, by id.
  readonly open: readonly string[];
  readonly requests: RequestFilters;
}

const KINDS: readonly RequestKind[] = ['all', 'errors', 'subagents', 'slow'];
const NO_FILTERS: RequestFilters = { kind: 'all', model: null, stop: null };

const ids = (v: string | null): string[] => (v ?? '').split('.').filter(Boolean);

/// The Overview's state from its query string: `?account=&project=&session=&range=&open=&kind=
/// &model=&stop=`, every key optional.
export function readQuery(search: string): OverviewAddress {
  const p = new URLSearchParams(search);
  const kind = p.get('kind');
  return {
    scope: { account: p.get('account') || null, project: p.get('project') || null, session: p.get('session') || null, range: parseRange(p.get('range')) },
    open: ids(p.get('open')),
    requests: { kind: KINDS.find((k) => k === kind) ?? 'all', model: p.get('model') || null, stop: p.get('stop') || null },
  };
}

/// The Overview's query string, "" when nothing is narrowed: the range left out when all, the open
/// request ids joined by dots, as the dashboard spelled them.
export function writeQuery(address: Partial<OverviewAddress> & { readonly scope: Scope }): string {
  const p = new URLSearchParams();
  const { scope, open = [], requests = NO_FILTERS } = address;
  if (scope.account) p.set('account', scope.account);
  if (scope.project) p.set('project', scope.project);
  if (scope.session) p.set('session', scope.session);
  if (scope.range !== 'all') p.set('range', scope.range);
  if (open.length) p.set('open', open.join('.'));
  if (requests.kind !== 'all') p.set('kind', requests.kind);
  if (requests.model) p.set('model', requests.model);
  if (requests.stop) p.set('stop', requests.stop);
  const q = p.toString();
  return q ? `?${q}` : '';
}

// ---------- the crumbs and their pickers ----------

export interface Crumb {
  readonly label: string;
  /// The whole of what the label shortens — a path, an id — one press away (OV-C13).
  readonly full: string | null;
  /// In the strong style once its level is set (OV-C12).
  readonly set: boolean;
}

/// The breadcrumb: "All accounts", "All projects" and "All sessions" until a level is set; then an
/// account by its label (its id until the summary knows it), a project by the last part of its
/// path, a session by its first eight characters and its git branch when known (OV-C12, OV-C13).
export function crumbs(scope: Scope, known: { readonly accountLabel?: string | null; readonly branch?: string | null } = {}): Record<Level, Crumb> {
  return {
    account: scope.account ? { label: known.accountLabel || scope.account, full: scope.account, set: true } : { label: 'All accounts', full: null, set: false },
    project: scope.project ? { label: projectName(scope.project), full: scope.project, set: true } : { label: 'All projects', full: null, set: false },
    session: scope.session
      ? { label: `${id8(scope.session)}${known.branch ? ` (${known.branch})` : ''}`, full: scope.session, set: true }
      : { label: 'All sessions', full: null, set: false },
  };
}

/// A picker of more than this many rows opens with a filter (OV-C18).
export const FILTER_ABOVE = 8;

/// The filter names the list it filters (OV-C18).
export const pickerPlaceholder = (level: Level): string => `Filter ${level === 'account' ? 'accounts' : level === 'project' ? 'projects' : 'sessions'}…`;

/// The rows a filter keeps: those whose name or second line holds it, ignoring case (OV-C18).
export function pickerMatches<T extends { readonly name: string; readonly sub?: string | null }>(rows: readonly T[], term: string): T[] {
  const t = term.trim().toLowerCase();
  return t ? rows.filter((r) => `${r.name} ${r.sub ?? ''}`.toLowerCase().includes(t)) : [...rows];
}

/// The router lists at most this many sessions (src/api.rs).
export const SESSION_CAP = 200;

/// How many sessions the picker and the Sessions list hold, saying so when the list stopped at the
/// cap (OV-C34).
export const sessionsHeld = (n: number | null | undefined): string =>
  n == null ? DASH : n >= SESSION_CAP ? `${count(SESSION_CAP)} most recent` : `${count(n)} session${n === 1 ? '' : 's'}`;
