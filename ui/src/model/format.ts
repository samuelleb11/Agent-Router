// New for agent-router: the formatters of assets/index.html and assets/live.html, with no DOM
// and no React, so the specs call them directly (docs/web-console-contracts.md, OV-C211).
//
// The dashboard's formatters are the ones the iPhone app copies by name, in
// ios/AgentRouter/Sources/Views/Format.swift (`Fmt`): `hours`, `usd`, `compact`, `count`,
// `percent`, `share`, `relative`, `ago`, `duration`, `when`, `ms`, `bucketName`, `shortModel`
// and `hm`. They keep those names here. Where the page and the phone disagreed, the contract is
// that every client says it one way (OV-C184, OV-C185), and the phone's spelling is the one its
// tests pin, so it is the one kept.
//
// Every instant is epoch milliseconds, and every function that reads the clock takes `now`, so a
// spec decides what time it is.

/// What a missing value reads as, everywhere: no data is not zero (SH-C20).
export const DASH = '—';

type Num = number | null | undefined;

const isNum = (v: unknown): v is number => typeof v === 'number' && Number.isFinite(v);

/// Hours of work: minutes under one ("15 min"), one decimal under ten ("2.4 h"), whole above
/// ("81 h"), and "—" when unknown. The dashboard's `hrs`, `Fmt.hours` (OV-C79).
export function hours(v: Num): string {
  if (!isNum(v)) return DASH;
  if (v < 1) return `${Math.round(v * 60)} min`;
  if (v < 10) return `${v.toFixed(1)} h`;
  return `${Math.round(v)} h`;
}

/// Token quantities, compact: whole below a thousand, "2.4k", "120k", "1.20M", "2.68B". The value
/// is rounded before the next threshold is tested, so 999,600 reads "1.00M", never "1000k"
/// (OV-C174). A figure the router did not record reads "—"; only a recorded zero reads "0"
/// (OV-C175). The dashboard's `k`, `Fmt.compact`.
export function compact(v: Num): string {
  if (!isNum(v)) return DASH;
  const a = Math.abs(v);
  if (a >= 1e9 || Math.round(a / 1e6) >= 1000) return `${(v / 1e9).toFixed(2)}B`;
  if (a >= 1e6 || Math.round(a / 1e3) >= 1000) return `${(v / 1e6).toFixed(2)}M`;
  if (a >= 1e4) return `${(v / 1e3).toFixed(0)}k`;
  if (a >= 1e3) return `${(v / 1e3).toFixed(1)}k`;
  return String(Math.round(v));
}

/// A count — requests, sessions, calls — exact, with thousands separators, never compacted
/// (OV-C176). The dashboard's `n`, `Fmt.count`.
export function count(v: Num): string {
  return isNum(v) ? v.toLocaleString() : DASH;
}

/// "1 session", "2 sessions": a count that agrees with its noun (OV-C176).
export function plural(v: Num, one: string, many: string): string {
  return `${count(v)} ${v === 1 ? one : many}`;
}

/// Dollars: whole with separators from $1,000, cents from $1, three places below that and four
/// below a cent, "$0" for zero, and "—" for a cost that could not be priced, never "$0"
/// (OV-C177). The dashboard's `usd`, `Fmt.usd`.
export function usd(v: Num): string {
  if (!isNum(v)) return DASH;
  const a = Math.abs(v);
  if (a >= 1000) return `$${Math.round(a).toLocaleString()}`;
  if (a >= 1) return `$${a.toFixed(2)}`;
  if (a > 0) return `$${a >= 0.01 ? a.toFixed(3) : a.toFixed(4)}`;
  return '$0';
}

/// A sum of costs as the router gives it: the dollars it could price, and beside them, counted, the
/// requests with usage it could not price, which the dollars leave out, and the requests whose
/// tokens were estimated rather than read from their stored response (OV-C229, OV-C232).
export interface CostCounts {
  readonly cost: Num;
  readonly unpriced?: Num;
  readonly estimated?: Num;
}

/// One mark on a cost and its reason, as a tooltip row pairs a value with its label.
export interface CostMark {
  readonly kind: 'unpriced' | 'estimated';
  /// "4 unpriced", "3 estimated", "not priced".
  readonly value: string;
  /// Why it is marked.
  readonly label: string;
}

/// A cost as the console writes it, with each of its marks' reasons for its tooltip.
export interface CostFigure {
  /// "$3.10", "$3.10 + 4 unpriced", "≈$3.10", "not priced", "$0", "—".
  readonly text: string;
  /// Empty when the figure carries no mark.
  readonly marks: readonly CostMark[];
}

/// Why a sum leaves requests out (OV-C229).
export const UNPRICED_WHY = 'no rate for their model in the pricing table, so the sum leaves them out';
/// Why a request's tokens are an estimate (OV-C232): the Kimi Code requests recorded before the
/// router counted their cache reads once, whose responses were no longer kept to count again from
/// (src/db.rs, `recompute_kimi_usage`).
export const ESTIMATED_WHY = 'tokens estimated rather than read from the stored response, for Kimi Code requests from before cache reads were counted once whose responses were no longer kept';

const whole = (v: Num): number => (isNum(v) && v > 0 ? Math.round(v) : 0);

/// A sum of costs, one rule wherever a cost is summed (OV-C229, OV-C232): the priced part, marked
/// "+ N unpriced" when it leaves requests out, "not priced" when nothing in it was priced — never
/// "$0", which stands only for a zero the router measured — and "≈" before it when the dollars rest
/// on estimated tokens. A cost the router did not give, with nothing left out, is "—".
export function costSum(s: CostCounts): CostFigure {
  const unpriced = whole(s.unpriced);
  const estimated = whole(s.estimated);
  const priced = isNum(s.cost) && s.cost > 0;
  const marks: CostMark[] = [];
  if (unpriced) marks.push({ kind: 'unpriced', value: `${count(unpriced)} unpriced`, label: UNPRICED_WHY });
  // An estimate moves only a figure there is: nothing priced has no dollars to be approximate.
  if (estimated && priced) marks.push({ kind: 'estimated', value: `${count(estimated)} estimated`, label: ESTIMATED_WHY });
  const dollars = unpriced ? (priced ? `${usd(s.cost)} + ${count(unpriced)} unpriced` : 'not priced') : usd(s.cost);
  return { text: estimated && priced ? `≈${dollars}` : dollars, marks };
}

/// One request's cost (OV-C229): "not priced" for a request with usage and no cost, which the router
/// could not price, never "$0"; "—" for one with no usage to price.
export function requestCost(r: { readonly cost: Num; readonly prompt_tokens: Num; readonly output_tokens?: Num }): CostFigure {
  const unpriced = !isNum(r.cost) && (isNum(r.prompt_tokens) || isNum(r.output_tokens));
  return unpriced ? { text: 'not priced', marks: [{ kind: 'unpriced', value: 'not priced', label: 'no rate for its model in the pricing table' }] } : costSum({ cost: r.cost });
}

/// A figure's marks as one sentence, the tooltip of a row that is a button and so cannot hold a tip
/// of its own: "4 unpriced: no rate …". Null when it carries none.
export const costWhy = (f: CostFigure): string | null => (f.marks.length ? `${f.marks.map((m) => `${m.value}: ${m.label}`).join('. ')}.` : null);

/// What the marks on a list's costs mean, said once under the list as text, since a row that is a
/// button has only a title for its tooltip, which a phone and a keyboard never reach (OV-C229,
/// OV-C232, SH-C30). Null when no cost in it is marked.
export function costNote(figures: readonly CostFigure[]): string | null {
  const kinds = new Set(figures.flatMap((f) => f.marks.map((m) => m.kind)));
  const said = [kinds.has('unpriced') ? `Unpriced: ${UNPRICED_WHY}.` : null, kinds.has('estimated') ? `≈ estimated: ${ESTIMATED_WHY}.` : null].filter(Boolean);
  return said.length ? said.join(' ') : null;
}

/// A fraction as a whole percent, for meters. `Fmt.percent`.
export function percent(fraction: Num): string {
  return isNum(fraction) ? `${Math.round(fraction * 100)}%` : DASH;
}

/// A share: whole from 10 % and at 0, one decimal below 10 %, so 27 errors in 8,474 requests read
/// "0.3%", not "0%" (OV-C179). `Fmt.share`.
export function share(fraction: Num): string {
  if (!isNum(fraction)) return DASH;
  const p = fraction * 100;
  return `${p >= 10 || p === 0 ? p.toFixed(0) : p.toFixed(1)}%`;
}

/// `a` as a share of `b`: "—" when there is nothing to divide by, and "—" when `a` itself is
/// missing, which the dashboard's `pct` read as 0 % (OV-C61: a missing reading is never 0 %).
export function pct(a: Num, b: Num): string {
  if (!isNum(a) || !isNum(b) || b === 0) return DASH;
  return share(a / b);
}

/// A request's timing: "812 ms" below a second, "3.2 s" above (OV-C180). `Fmt.ms`.
export function ms(v: Num): string {
  if (!isNum(v)) return DASH;
  return v < 1000 ? `${Math.round(v)} ms` : `${(v / 1000).toFixed(1)} s`;
}

/// A span: "45s", "3m 05s", "3h 12m", "1d 0h", never negative (OV-C181). The dashboard's `dur`,
/// in milliseconds; `Fmt.duration` takes seconds.
export function duration(spanMs: Num): string {
  if (!isNum(spanMs)) return DASH;
  const s = Math.max(0, Math.round(spanMs / 1000));
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m ${String(s % 60).padStart(2, '0')}s`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h}h ${String(m % 60).padStart(2, '0')}m`;
  return `${Math.floor(h / 24)}d ${h % 24}h`;
}

/// A time to come: "in 5 min", "in 2h 10m", "in 3d 4h", or "now" once it has passed (OV-C181).
/// The dashboard's `rel`, which took epoch seconds; this takes milliseconds. `Fmt.relative`.
export function relative(atMs: Num, now: number = Date.now()): string {
  if (!isNum(atMs) || atMs === 0) return DASH;
  const d = atMs - now;
  if (d <= 0) return 'now';
  const m = Math.round(d / 60000);
  if (m < 60) return `in ${m} min`;
  const h = Math.floor(m / 60);
  if (h < 24) return `in ${h}h ${String(m % 60).padStart(2, '0')}m`;
  return `in ${Math.floor(h / 24)}d ${h % 24}h`;
}

/// A time gone: "just now" under 45 seconds, then "N min ago", "N h ago", "N d ago" for a week,
/// then the date (OV-C181). `Fmt.ago`.
export function ago(atMs: Num, now: number = Date.now()): string {
  if (!isNum(atMs) || atMs === 0) return DASH;
  const s = Math.round((now - atMs) / 1000);
  if (s < 45) return 'just now';
  const m = Math.round(s / 60);
  if (m < 60) return `${m} min ago`;
  const h = Math.round(m / 60);
  if (h < 24) return `${h} h ago`;
  const d = Math.round(h / 24);
  return d < 7 ? `${d} d ago` : monthDay(atMs);
}

/// "14:05", in the reader's locale. `Fmt.hm`.
export const hm = (atMs: number): string => new Date(atMs).toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit' });

/// "Oct 5", in the reader's locale. The dashboard's `md`, renamed: `md` is the Markdown parser.
export const monthDay = (atMs: number): string => new Date(atMs).toLocaleDateString(undefined, { month: 'short', day: 'numeric' });

/// "Oct 5, 14:05", or "—" when there is no moment. `Fmt.when`.
export function when(atMs: Num): string {
  if (!isNum(atMs) || atMs === 0) return DASH;
  return new Date(atMs).toLocaleString(undefined, { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' });
}

/// A moment as its time of day when it is today, as its date and time otherwise (OV-C182).
export function tod(atMs: Num, now: number = Date.now()): string {
  if (!isNum(atMs) || atMs === 0) return DASH;
  return new Date(atMs).toDateString() === new Date(now).toDateString() ? hm(atMs) : `${monthDay(atMs)} ${hm(atMs)}`;
}

/// A reset instant in either spelling the router sends — "@<epoch seconds>" from stored headers,
/// an ISO date from the usage API — as epoch milliseconds; anything else is unknown, never the
/// epoch (OV-C183).
export function resetAt(v: unknown): number | null {
  if (v == null) return null;
  const t = String(v);
  if (t.startsWith('@')) {
    const n = Number(t.slice(1));
    return t.length > 1 && Number.isFinite(n) ? n * 1000 : null;
  }
  const parsed = Date.parse(t);
  return Number.isFinite(parsed) ? parsed : null;
}

/// A bucket's name, one spelling on every client: "minute", "5 minutes", "hour", "6 hours",
/// "day" (OV-C185). The page said "15 min" where the phone says "15 minutes"; the phone's
/// spelling is the one `testRangesAndFormats` pins. `Fmt.bucketName`.
export function bucketName(bucketMs: number): string {
  if (bucketMs < 60_000) return `${Math.round(bucketMs / 1000)}s`;
  if (bucketMs === 60_000) return 'minute';
  if (bucketMs < 3_600_000) return `${Math.floor(bucketMs / 60_000)} minutes`;
  if (bucketMs === 3_600_000) return 'hour';
  if (bucketMs < 86_400_000) return `${Math.floor(bucketMs / 3_600_000)} hours`;
  return 'day';
}

/// A model's short name, the same on every client (OV-C184): the name without "claude-",
/// "claude-opus-5-20260115" reading "opus-5-20260115" as the phones print it. The page also
/// dropped the date and joined the version ("opus 5"), which no phone did.
export function shortModel(m: string | null | undefined): string {
  if (!m) return DASH;
  return m.startsWith('claude-') ? m.slice('claude-'.length) : m;
}

export type Family = 'opus' | 'sonnet' | 'haiku' | 'kimi' | 'other';

/// A model's family, read from its name; anything else is other (OV-C10). Kimi Code's models are
/// one family, `kimi`: `k3`, `k3-256k`, and every `kimi-…` id (OV-C222).
export function family(m: string | null | undefined): Family {
  const s = m ?? '';
  if (/opus/.test(s)) return 'opus';
  if (/sonnet/.test(s)) return 'sonnet';
  if (/haiku/.test(s)) return 'haiku';
  return /^(?:k3|kimi)\b/.test(s) ? 'kimi' : 'other';
}

/// What the dashboard calls a request with no project: the join behind it is a LEFT JOIN, so a
/// request whose transcript never arrived has none (OV-C52).
export const UNRESOLVED = '(unresolved)';

/// A project by the last part of its path, "(unresolved)" when it has none (OV-C186). The
/// dashboard's `base`.
export function projectName(path: string | null | undefined): string {
  if (!path) return UNRESOLVED;
  const parts = path.replace(/[\\/]+$/, '').split(/[\\/]/);
  return parts[parts.length - 1] || path;
}

/// A session by the first eight characters of its id (OV-C186).
export const id8 = (id: string | null | undefined): string => String(id ?? '').replace(/^(?:codex:|opencode:ses_)/, '').slice(0, 8);

/// The clients the router names a session's requests by, read off their User-Agent
/// (src/analyze.rs, `client_of`): Claude Code's terminal and VS Code, and Kimi Code's terminal,
/// VS Code and web.
export type ClientId = 'claude-code-cli' | 'claude-code-vscode' | 'kimi-code-cli' | 'kimi-code-vscode' | 'kimi-code-web' | 'codex-cli' | 'codex-vscode' | 'opencode';

/// Each client's label, one spelling wherever the console names a session's client (SE-C234).
/// Claude Code's keep the words its entrypoint was always named by (OV-C187); Kimi Code's say whose
/// they are.
const CLIENT_LABELS: Readonly<Record<ClientId, string>> = {
  'claude-code-cli': 'CLI',
  'claude-code-vscode': 'VS Code',
  'kimi-code-cli': 'Kimi Code CLI',
  'kimi-code-vscode': 'Kimi Code VS Code',
  'kimi-code-web': 'Kimi Code web',
  'codex-cli': 'Codex CLI',
  'codex-vscode': 'Codex VS Code',
  opencode: 'OpenCode',
};

/// A client's label by the router's id for it; null for an id the console does not know, which is
/// not named (SE-C234). Looked up as an own property, so `constructor` names nothing.
export function clientLabel(id: string | null | undefined): string | null {
  return typeof id === 'string' && Object.hasOwn(CLIENT_LABELS, id) ? (CLIENT_LABELS[id as ClientId] ?? null) : null;
}

/// The products a machine's agent reports (src/clients.rs, `Client`), by their names.
const PRODUCTS: Readonly<Record<string, string>> = { 'claude-code': 'Claude Code', 'kimi-code': 'Kimi Code', codex: 'Codex' };

/// A product by its name — "Claude Code", "Kimi Code" — and one the console does not know as sent
/// (MA-C8, SE-C235).
export function productName(id: string | null | undefined): string {
  const p = typeof id === 'string' ? id : '';
  return (Object.hasOwn(PRODUCTS, p) ? PRODUCTS[p] : undefined) ?? p;
}

/// The client a session ran in: the router's id for it when it names one (SE-C234), else from the
/// entrypoint, "VS Code" when it mentions claude-vscode, else "CLI" for claude-cli, else not named
/// (OV-C187). The VS Code extension launches the CLI, so its mark is tested first.
export function client(entrypoint: string | null | undefined, id?: string | null): string | null {
  const named = clientLabel(id);
  if (named) return named;
  if (!entrypoint) return null;
  if (entrypoint.startsWith('opencode/')) return CLIENT_LABELS.opencode;
  if (/codex[_-](?:vscode|vsc)|vscode.*codex/.test(entrypoint)) return CLIENT_LABELS['codex-vscode'];
  if (/codex[_-]cli/.test(entrypoint)) return CLIENT_LABELS['codex-cli'];
  return /claude-vscode/.test(entrypoint) ? CLIENT_LABELS['claude-code-vscode'] : /claude-cli/.test(entrypoint) ? CLIENT_LABELS['claude-code-cli'] : null;
}

// ---------- the live view's own spellings ----------

/// A stretch of time the way it is glanced at: one unit, rounded — "12s", "5m", "3h", "2d" — and
/// never negative (SE-C16). The live view's `span`; the phones spell two units, a known
/// difference.
export function span(spanMs: number): string {
  const s = Math.max(0, spanMs / 1000);
  if (s < 60) return `${Math.round(s)}s`;
  if (s < 3600) return `${Math.round(s / 60)}m`;
  if (s < 86400) return `${Math.round(s / 3600)}h`;
  return `${Math.round(s / 86400)}d`;
}

/// How long a feed row took: "820ms" under a second, "2.8s" under ten, "18s" under a minute, else
/// "3m" (SE-C84). The live view's `dur`.
export function rowDuration(v: number): string {
  if (v < 1000) return `${Math.round(v)}ms`;
  if (v < 60000) return `${(v / 1000).toFixed(v < 10000 ? 1 : 0)}s`;
  return `${Math.round(v / 60000)}m`;
}

/// Money on the facts strip: "$0.123" under a dollar, "$4.56" under a thousand, "$1,234" above;
/// a cost the router could not price is left out, never "$0" (SE-C134).
export function factUsd(v: Num): string | null {
  if (!isNum(v)) return null;
  if (v >= 1000) return `$${Math.round(v).toLocaleString()}`;
  if (v >= 1) return `$${v.toFixed(2)}`;
  return `$${v.toFixed(3)}`;
}

/// Tokens on the facts strip: "1.2B", "3.4M", "56k" or the plain number (SE-C134); "—" when the
/// router did not say.
export function factTokens(v: Num): string {
  if (!isNum(v)) return DASH;
  if (v >= 1e9) return `${(v / 1e9).toFixed(1)}B`;
  if (v >= 1e6) return `${(v / 1e6).toFixed(1)}M`;
  if (v >= 1e3) return `${Math.round(v / 1e3)}k`;
  return String(v);
}

// ---------- search snippets, on both pages ----------

/// One run of a search snippet: text, matched or not.
export interface Run {
  readonly text: string;
  readonly mark: boolean;
}

/// A snippet split on the router's ‹ › marker pair: odd runs are the matches. Split rather than
/// substituted, so a marker the conversation itself contained can only shift which run is marked:
/// it never leaves a mark open, never drops text, and the snippet stays text (SH-C26, OV-C49,
/// SE-C170). The dashboard's `markup` and the live view's `findMark`, which built HTML.
export function marks(snippet: string | null | undefined): Run[] {
  return String(snippet ?? '')
    .split(/[‹›]/)
    .map((text, i) => ({ text, mark: i % 2 === 1 }))
    .filter((run) => run.text !== '');
}

/// The reader's words for each kind of indexed block, the same on every client: one hit's kind —
/// message, tool call, tool result, thinking, system prompt — and a run of hits of that kind, as
/// the palette groups them; anything else as sent (OV-C50, SE-C169). One table, so a hit and its
/// run cannot name a kind two ways. The dashboard had no word for `system`. Looked up as an own
/// property, so a kind spelled like one every object has (`constructor`) is a kind like any other.
const KINDS: Readonly<Record<string, { readonly one: string; readonly run: string }>> = {
  text: { one: 'message', run: 'Messages' },
  tool_use: { one: 'tool call', run: 'Tool calls' },
  tool_result: { one: 'tool result', run: 'Tool results' },
  thinking: { one: 'thinking', run: 'Thinking' },
  system: { one: 'system prompt', run: 'System prompts' },
};

export function kindWord(kind: string | null | undefined): string {
  const k = kind ?? '';
  return Object.hasOwn(KINDS, k) ? (KINDS[k]?.one ?? k) : k;
}

/// What a run of hits of one kind is called: "Messages", "Tool calls"…, and a kind the router
/// indexes later as it is sent.
export function kindRun(kind: string | null | undefined): string {
  const k = kind ?? '';
  return Object.hasOwn(KINDS, k) ? (KINDS[k]?.run ?? k) : k;
}
