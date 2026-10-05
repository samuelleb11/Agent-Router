// New for agent-router: the Sessions screen's own rules that need no React — how an outcome is
// worded, how the arrow keys walk the history, what the facts strip says, and which feed entry a
// search hit lands on. The session model (~/model/session) keeps the rules the phones share; these
// are the screen's.

import type { Outcome, ProjectTotals, Stats } from '~/data/live';
import { factTokens, factUsd, plural } from '~/model/format';
import type { SearchHit, SearchReply } from '~/model/search';
import { refusal, type Action, type FeedEntry } from '~/model/session';

// ---------- outcomes ----------

/// What a control says when a request reached nothing: the action, then why (SE-C153, SE-C117).
const NO_REPLY: Readonly<Record<Action, string>> = {
  send: 'Send failed',
  stop: 'Could not stop',
  aside: 'The side question failed',
  archive: 'Could not archive',
  answer: 'Could not answer that prompt',
  form: 'could not answer',
  model: 'Could not set the model',
  effort: 'Could not set the effort',
  mode: 'Could not set the mode',
};

/// The words for a write that did not land, by the screen's rule (SE-C183): the router's own words
/// — its `reason`, or a hosted router's 503 `error` — else a sentence naming the action. A local
/// router's own 5xx reads as that sentence (SE-C158), and so does a refusal that said nothing. A
/// request that reached nothing, or none in time, says so after the action (SE-C153, SE-C185).
export function failed(action: Action, outcome: Exclude<Outcome<unknown>, { ok: true }>): string {
  if (outcome.status === null) return `${NO_REPLY[action]} — ${outcome.words}`;
  if (outcome.status >= 500 && outcome.status !== 503) return refusal(action, outcome.status, {});
  return outcome.words || refusal(action, outcome.status, {});
}

// ---------- the arrow keys ----------

/// Where the composer stands in the history: composing (`at` null), or showing entry `at`, with the
/// draft that was there when browsing began.
export interface Browse {
  readonly at: number | null;
  readonly draft: string;
}

export const COMPOSING: Browse = { at: null, draft: '' };

/// One arrow key in the composer (SE-C156): taken only when the caret is on the edge it would leave
/// — ArrowUp at the very start, ArrowDown at the very end, nothing selected — so a multi-line draft
/// is edited normally. ArrowUp from a draft keeps it, ArrowDown past the newest entry brings it
/// back, ArrowUp at the oldest stays put. Null when the key is not the history's.
export function walk(
  key: 'ArrowUp' | 'ArrowDown',
  sent: readonly string[],
  state: Browse,
  box: { readonly value: string; readonly start: number; readonly end: number },
): { readonly state: Browse; readonly value: string } | null {
  if (!sent.length) return null;
  const atStart = box.start === 0 && box.end === 0;
  const atEnd = box.start === box.value.length && box.end === box.value.length;
  if (key === 'ArrowUp') {
    if (!atStart) return null;
    const from = state.at ?? sent.length;
    const draft = state.at === null ? box.value : state.draft;
    if (from === 0) return { state: { at: 0, draft }, value: box.value };
    const at = from - 1;
    return { state: { at, draft }, value: sent[at] ?? '' };
  }
  if (!atEnd || state.at === null) return null;
  const at = state.at + 1;
  if (at >= sent.length) return { state: COMPOSING, value: state.draft };
  return { state: { at, draft: state.draft }, value: sent[at] ?? '' };
}

// ---------- the facts strip ----------

/// What a session is and what it is costing (SE-C131, SE-C134): its model without "claude-", or
/// "unknown" when its transcript names none; its effort when known; its tokens with cache reads
/// included; its output and cached tokens; its turns; and its cost only when priced.
export function sessionFacts(t: Stats): string[] {
  const model = t.model ? t.model.replace(/^claude-/, '') : 'unknown';
  const cost = factUsd(t.cost);
  return [
    model,
    ...(t.effort ? [`effort ${t.effort}`] : []),
    `${factTokens(t.total)} tokens`,
    `${factTokens(t.output)} out · ${factTokens(t.cache_read)} cached`,
    plural(t.turns, 'turn', 'turns'),
    ...(cost ? [cost] : []),
  ];
}

/// The project's own total, apart on the same strip, and only when it has more than one session
/// (SE-C132).
export function projectFacts(p: ProjectTotals | null | undefined): string | null {
  if (!p || typeof p.sessions !== 'number' || p.sessions <= 1) return null;
  const cost = factUsd(p.cost);
  return ['project', plural(p.sessions, 'session', 'sessions'), `${factTokens(p.total)} tokens`, plural(p.turns, 'turn', 'turns'), ...(cost ? [cost] : [])].join(
    ' · ',
  );
}

// ---------- arriving at an entry ----------

/// The entry an address names: `/live?session=<id>&block=<block_id>&ordinal=<n>`, as a search hit
/// in the palette or in Find opens it, named by its block and its ordinal together (SE-C171).
export interface Arrival {
  readonly block: number;
  readonly ordinal: number;
}

export function arrivalIn(params: URLSearchParams): Arrival | null {
  const block = Number(params.get('block'));
  const ordinal = Number(params.get('ordinal'));
  if (!params.has('block') || !params.has('ordinal') || !Number.isInteger(block) || !Number.isInteger(ordinal)) return null;
  return { block, ordinal };
}

/// The hit an arrival names, from the search answers this page already holds: the router's index
/// has no read of one block by its id, and its stream's rows carry no block id, so the hit's own
/// words are what a feed entry is recognised by.
export function hitIn(answers: readonly unknown[], session: string, at: Arrival): SearchHit | null {
  for (const a of answers) {
    const hits = a && typeof a === 'object' && Array.isArray((a as SearchReply).hits) ? ((a as SearchReply).hits ?? []) : [];
    const hit = hits.find((h) => h && h.session_id === session && h.block_id === at.block && h.ordinal === at.ordinal);
    if (hit) return hit;
  }
  return null;
}

/// The router's snippet ellipsis, between the fragments of a long block.
const ELLIPSIS = /\s*…\s*/;
const squeeze = (s: string): string => s.replace(/\s+/g, ' ').trim().toLowerCase();

/// Which feed kind each indexed kind is drawn as.
const DRAWN_AS: Readonly<Record<string, readonly string[]>> = {
  text: ['text', 'aside'],
  thinking: ['thinking'],
  tool_use: ['tool_use'],
  tool_result: ['tool_result'],
};

/// The feed entry a hit lands on: the newest row whose text holds every fragment of the hit's
/// snippet, its markers taken out; a row of the hit's own kind first. Null when the feed holds
/// none, which it may not: the feed is the transcript's tail.
export function entryFor(entries: readonly FeedEntry[], hit: Pick<SearchHit, 'snippet' | 'kind'>): number | null {
  const fragments = String(hit.snippet ?? '')
    .replace(/[‹›]/g, '')
    .split(ELLIPSIS)
    .map(squeeze)
    .filter((f) => f.length > 0);
  if (!fragments.length) return null;
  const kinds = DRAWN_AS[hit.kind ?? ''] ?? [];
  let fallback: number | null = null;
  for (let i = entries.length - 1; i >= 0; i--) {
    const e = entries[i];
    if (!e || e.kind !== 'row') continue;
    const text = squeeze(String(e.item.text ?? ''));
    if (!fragments.every((f) => text.includes(f))) continue;
    if (kinds.includes(String(e.item.kind ?? ''))) return i;
    fallback ??= i;
  }
  return fallback;
}
