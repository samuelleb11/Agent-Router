// New for agent-router: the request list's filters, figures and detail, with no DOM (assets/index.html,
// `renderRequests`, `renderDetail`, `parseCtx`).
//
// Where the page broke its own rule, the rule is kept:
// - a kind filter that is on always shows its control pressed, and a model or stop filter whose
//   value has left the loaded rows is cleared (OV-C153);
// - the Slow filter states its threshold as visible text (OV-C150), and every row it keeps carries
//   the slow badge, an error row included (OV-C151);
// - the cap's note offers "scope to a session" only where that can help (OV-C147).

import type { Fact } from '~/components/RequestRow';
import type { Part } from '~/components/ShareBar';
import type { RequestRecord } from '~/data/overview';
import { DASH, compact, count, family, hm, id8, monthDay, ms, pct, plural, projectName, shortModel, usd, when, type Family } from '~/model/format';
import type { RequestFilters, RequestKind, Scope } from '~/model/scope';
import type { Account } from '~/model/settings';
import { CONTEXT } from './panels';

/// The list holds the 300 newest requests in scope (src/api.rs).
export const REQUEST_CAP = 300;
/// A filter change returns the list to this many rows, and "Show more" adds this many (OV-C154).
export const REQUEST_PAGE = 20;

const n = (v: number | null | undefined): number => (typeof v === 'number' && Number.isFinite(v) ? v : 0);

/// An error is one thing everywhere: a status of 400 or more, or an error recorded (OV-C149).
export const isError = (r: Pick<RequestRecord, 'status' | 'error'>): boolean => n(r.status) >= 400 || !!r.error;
/// A subagent request is one marked as a sidechain (OV-C149).
export const isSubagent = (r: Pick<RequestRecord, 'is_sidechain'>): boolean => r.is_sidechain === true || n(r.is_sidechain as number | null) === 1;
/// Slow is a first token slower than the scope's p95; without a p95 there is no slow (OV-C150).
export const isSlow = (r: Pick<RequestRecord, 'ttfb_ms'>, p95: number | null | undefined): boolean => typeof p95 === 'number' && typeof r.ttfb_ms === 'number' && r.ttfb_ms > p95;

/// A request's stop reason as a filter names it, "(none)" for none (OV-C152).
export const stopOf = (r: Pick<RequestRecord, 'stop_reason'>): string => r.stop_reason || '(none)';

export interface KindChoice {
  readonly kind: RequestKind;
  readonly label: string;
  readonly count: number;
}

export interface ModelChoice {
  readonly model: string;
  readonly short: string;
  readonly family: Family;
  readonly count: number;
}

export interface Filtering {
  /// All always; the others only when some loaded row is of them, or when they are on (OV-C148,
  /// OV-C153).
  readonly kinds: readonly KindChoice[];
  /// The threshold of Slow as visible text, or null without a p95 (OV-C150).
  readonly slowRule: string | null;
  /// A chip per model when more than one is loaded (OV-C152).
  readonly models: readonly ModelChoice[];
  /// A chip per stop reason when more than one is loaded (OV-C152).
  readonly stops: readonly { readonly stop: string; readonly count: number }[];
  /// The filters in force: a model or stop that has left the loaded rows is cleared (OV-C153).
  readonly effective: RequestFilters;
  /// The rows the filters keep, newest first.
  readonly rows: readonly RequestRecord[];
}

/// The filters over what is loaded: they narrow the 300 newest, never ask for more (OV-C146).
export function filtering(list: readonly RequestRecord[], asked: RequestFilters, p95: number | null | undefined): Filtering {
  const tests: Readonly<Record<RequestKind, (r: RequestRecord) => boolean>> = {
    all: () => true,
    errors: isError,
    subagents: isSubagent,
    slow: (r) => isSlow(r, p95),
  };
  const labels: Readonly<Record<RequestKind, string>> = { all: 'All', errors: 'Errors', subagents: 'Subagents', slow: 'Slow' };
  const slowRule = typeof p95 === 'number' ? `first token slower than the p95 of ${ms(p95)}` : null;
  const offered = (k: RequestKind): boolean => {
    if (k === 'all') return true;
    // Without a p95 there is no Slow, on or not (OV-C150).
    if (k === 'slow' && slowRule === null) return false;
    return list.some(tests[k]) || asked.kind === k;
  };
  const kinds = (['all', 'errors', 'subagents', 'slow'] as const).filter(offered).map((k) => ({ kind: k, label: labels[k], count: list.filter(tests[k]).length }));
  const kind: RequestKind = kinds.some((k) => k.kind === asked.kind) ? asked.kind : 'all';

  const modelsIn = [...new Set(list.map((r) => r.model).filter((m): m is string => !!m))];
  const stopsIn = [...new Set(list.map(stopOf))];
  const model = asked.model && modelsIn.includes(asked.model) ? asked.model : null;
  const stop = asked.stop && stopsIn.includes(asked.stop) ? asked.stop : null;

  let rows = list.filter(tests[kind]);
  if (model) rows = rows.filter((r) => r.model === model);
  if (stop) rows = rows.filter((r) => stopOf(r) === stop);

  return {
    kinds,
    slowRule,
    models: modelsIn.length > 1 ? modelsIn.map((m) => ({ model: m, short: shortModel(m), family: family(m), count: list.filter((r) => r.model === m).length })) : [],
    stops: stopsIn.length > 1 ? stopsIn.map((s) => ({ stop: s, count: list.filter((r) => stopOf(r) === s).length })) : [],
    effective: { kind, model, stop },
    rows,
  };
}

/// The caption: what is shown of what the filters kept, what they were filtered from, and the cap,
/// with its remedy only where one exists (OV-C146, OV-C147).
export function requestsCaption(loaded: number, kept: number, shown: number, scope: Scope): string {
  return [
    kept ? `${count(Math.min(shown, kept))} of ${count(kept)} shown` : 'nothing matches',
    kept !== loaded ? `filtered from ${count(loaded)}` : null,
    loaded >= REQUEST_CAP ? `${count(REQUEST_CAP)} newest loaded${scope.session ? '' : ' — scope to a session for the rest'}` : null,
  ]
    .filter(Boolean)
    .join(' · ');
}

/// "Show N more", twenty at a time, or "Show fewer" once everything kept is shown (OV-C154).
export function moreOrFewer(kept: number, shown: number): { readonly more: number } | { readonly fewer: true } | null {
  if (kept > shown) return { more: Math.min(REQUEST_PAGE, kept - shown) };
  if (kept > REQUEST_PAGE) return { fewer: true };
  return null;
}

/// When a request started: with seconds today, with the date otherwise (OV-C156).
export function requestTime(at: number, now: number = Date.now()): string {
  const today = new Date(at).toDateString() === new Date(now).toDateString();
  return today ? new Date(at).toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit', second: '2-digit' }) : `${monthDay(at)} ${hm(at)}`;
}

/// The card's figures, in the formatters' words (OV-C156): the share cached is "—" without a prompt.
export function figures(r: RequestRecord): { prompt: string; cached: string; output: string; cost: string; firstToken: string } {
  return {
    prompt: compact(r.prompt_tokens),
    cached: n(r.prompt_tokens) ? `${pct(r.cache_read_tokens, r.prompt_tokens)} cached` : '—',
    output: `${compact(r.output_tokens)} out`,
    cost: usd(r.cost),
    firstToken: ms(r.ttfb_ms),
  };
}

/// The context window of one request, from its `component:tokens` pairs.
export function parseContext(s: string | null | undefined): Record<string, number> {
  const out: Record<string, number> = {};
  for (const pair of String(s ?? '').split(',')) {
    const [key, v] = pair.split(':');
    if (key) out[key] = Number(v) || 0;
  }
  return out;
}

/// The tools called, in the order first called, with "×N" for repeats (OV-C158).
export function toolsCalled(s: string | null | undefined): string[] {
  const counted = new Map<string, number>();
  for (const t of String(s ?? '').split(',').filter(Boolean)) counted.set(t, (counted.get(t) ?? 0) + 1);
  return [...counted].map(([t, c]) => (c > 1 ? `${t} ×${c}` : t));
}

/// Everything recorded about one request (OV-C158): its id; its session and project unless a session
/// is scoped; the whole reply's time with the first token's; the stop reason; thinking; cache read ·
/// written · uncached; a subagent's agent id; the tools called; the error at full width; and its
/// context window part by part.
export function detail(r: RequestRecord, scope: Scope): { facts: Fact[]; error?: string; context?: Part[] } {
  const facts: Fact[] = [{ label: 'request id', value: r.request_id || '—', mono: true }];
  if (!scope.session) facts.push({ label: 'session', value: `${id8(r.session_id)}${r.session_id ? ` · ${projectName(r.project)}` : '—'}`, mono: false });
  facts.push(
    { label: 'whole reply', value: r.duration_ms != null ? `${ms(r.duration_ms)} · first token ${ms(r.ttfb_ms)}` : '—' },
    { label: 'stopped', value: r.stop_reason || '—' },
    { label: 'thinking', value: `${compact(r.thinking_tokens)} tokens` },
    { label: 'cache', value: `${compact(r.cache_read_tokens)} read · ${compact(r.cache_write_tokens)} written · ${compact(r.input_tokens)} uncached` },
  );
  if (r.agent_id) facts.push({ label: 'agent id', value: r.agent_id, mono: true });
  const tools = toolsCalled(r.tools);
  if (tools.length) facts.push({ label: 'tools called', value: tools.join(', '), wide: true });
  const ctx = parseContext(r.context);
  const total = Object.values(ctx).reduce((a, b) => a + b, 0);
  const context: Part[] | undefined =
    total > 0
      ? CONTEXT.map((c) => ({ key: c.key, label: c.label, value: ctx[c.key] ?? 0, colour: c.colour, concept: c.concept, figure: compact(ctx[c.key] ?? 0) }))
      : undefined;
  return { facts, ...(r.error ? { error: r.error } : {}), ...(context ? { context } : {}) };
}

/// The account that served a request, as the lists name an account — its email, else its label, else
/// its id — with its router id beside it (OV-C17, OV-C231). An account the router no longer holds is
/// its id, and a request recorded before accounts were is "—".
export function servedBy(r: Pick<RequestRecord, 'account_id'>, accounts: readonly Pick<Account, 'id' | 'label' | 'email'>[] | null | undefined): string {
  const id = r.account_id;
  if (!id) return DASH;
  const a = accounts?.find((x) => x.id === id);
  const name = a?.email || a?.label || id;
  return name === id ? id : `${name} · ${id}`;
}

/// The full date and time a request started, for its detail.
export const startedAt = (r: Pick<RequestRecord, 'started_at'>): string => when(r.started_at);

/// How the badges count: a row the Slow filter keeps carries the slow badge, an error included
/// (OV-C151).
export const badges = (r: RequestRecord, p95: number | null | undefined) => ({ subagent: isSubagent(r), slow: isSlow(r, p95), error: isError(r) });

/// "3 errors" for the tile's button.
export const listErrors = (errors: number): string => `List the ${plural(errors, 'error', 'errors')}`;
