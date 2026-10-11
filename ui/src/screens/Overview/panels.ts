// New for agent-router: what the Overview's share bars and bar lists say, with no DOM (assets/index.html,
// `renderRead`, `renderContext`, `renderModels`, `renderThreads`, `renderAccountSplit`,
// `renderProjects`, `renderSessions`, `orphanRow`, `renderTools`).
//
// Where the page broke its own rule, the rule is kept:
// - "not yet attributed" is the router's own count, never the total less the rows a capped list
//   held (OV-C141), and explains itself beside it (OV-C142);
// - an empty panel says which empty it is, never "yet" under a scope or a range (OV-C145);
// - errors wear the error mark beside their count in every list (OV-C5), not a "⚠ N" in the meta;
// - a context part's cost and characters, and a tool's characters and largest result, are on the
//   page, not in a tooltip (OV-C124);
// - with no older router to meet (decision J), paid in full is the router's own uncached input and a
//   part's average is over the requests that carried it, with nothing worked out in their place
//   (OV-C119, OV-C123);
// - a cost that leaves out requests the router could not price, or rests on estimated tokens, says
//   so beside its figure, where the page summed both as if priced (OV-C229, OV-C232);
// - where a client is meant it is named, Claude Code and Kimi Code alike (OV-C230).

import type { BarRow } from '~/components/BarList';
import type { Part } from '~/components/ShareBar';
import type { Concept } from '~/lib/concepts';
import { FAMILY_COLOUR } from '~/lib/concepts';
import type { ProjectDirectory, ProjectRow } from '~/data/router';
import type { AccountSplitRow, ContextRow, ModelRow, SessionRow, ThreadRow, ToolCallRow, ToolOutputRow, Totals } from '~/data/overview';
import { DASH, ago, client, compact, costSum, costWhy, count, duration, family, id8, pct, plural, projectName, shortModel, tod, usd, UNRESOLVED, type CostFigure, type Family } from '~/model/format';
import { SESSION_CAP, isScoped, type Scope } from '~/model/scope';
import { providerName, providerOf, type Provider } from '~/model/settings';

const n = (v: number | null | undefined): number => (typeof v === 'number' && Number.isFinite(v) ? v : 0);
const has = (v: number | null | undefined): v is number => typeof v === 'number' && Number.isFinite(v);

// ---------------------------------------------------------------- empties

export type Empty = 'projects' | 'sessions' | 'toolOutput' | 'toolCalls' | 'read' | 'models';

/// Which empty a panel is (OV-C145): nothing recorded at all, with how to start; nothing in the
/// range; or nothing in this scope.
export function emptyWords(what: Empty, scope: Scope): string {
  const narrowed = isScoped(scope);
  const range = scope.range;
  if (!narrowed && range === 'all') {
    return {
      // Each client by its name (OV-C230).
      projects: 'No projects yet — point Claude Code, Kimi Code or Codex at this router and run it.',
      sessions: 'No sessions yet — point Claude Code, Kimi Code or Codex at this router and run it.',
      toolOutput: 'No tool results captured yet.',
      toolCalls: 'No tool calls captured yet.',
      read: 'Nothing read yet.',
      models: 'No requests yet.',
    }[what];
  }
  if (!narrowed) {
    return {
      projects: `No project sent a request in the last ${range}.`,
      sessions: `No session ran in the last ${range}.`,
      toolOutput: `No tool result came back in the last ${range}.`,
      toolCalls: `No tool was called in the last ${range}.`,
      read: `Nothing was read in the last ${range}.`,
      models: `No requests in the last ${range}.`,
    }[what];
  }
  const within = range === 'all' ? 'in this scope' : `in this scope in the last ${range}`;
  return {
    projects: `No project sent a request ${within}.`,
    sessions: `No sessions ${within}.`,
    toolOutput: `No tool results ${within}.`,
    toolCalls: `No tool calls ${within}.`,
    read: `Nothing read ${within}.`,
    models: `No requests ${within}.`,
  }[what];
}

/// Whether nothing at all is recorded: the empty that says how to start (OV-C145).
export const nothingRecorded = (scope: Scope): boolean => !isScoped(scope) && scope.range === 'all';

// ---------------------------------------------------------------- what the model read

export interface ReadPanel {
  readonly parts: readonly Part[];
  readonly total: number | null;
  /// The hit rate as its formula, under the bar; or which empty it is (OV-C120, OV-C145).
  readonly note: string;
}

/// One share bar of three parts in a fixed order on the token ramp: from cache, written to cache,
/// paid in full, the write row split into its tiers (OV-C118). Paid in full is the router's uncached
/// input (OV-C119). The note gives the hit rate as its formula (OV-C120).
export function readPanel(t: Totals, scope: Scope): ReadPanel {
  const prompt = t.prompt_tokens;
  const read = t.cache_read_tokens;
  const write = t.cache_write_tokens;
  const input = t.input_tokens;
  return {
    parts: [
      { key: 'read', label: 'from cache', value: read, colour: '--ar-tok-3', concept: 'cache', figure: compact(read) },
      { key: 'write', label: 'written to cache', value: write, colour: '--ar-tok', figure: compact(write), detail: `5-minute ${compact(t.cache_write_5m_tokens)} · 1-hour ${compact(t.cache_write_1h_tokens)}` },
      { key: 'input', label: 'paid in full', value: input, colour: '--ar-tok-2', figure: compact(input) },
    ],
    total: prompt,
    note: n(prompt)
      ? `Hit rate = cache reads ÷ everything read = ${pct(read, prompt)}. Cached tokens are far cheaper against the quota than tokens paid in full.`
      : emptyWords('read', scope),
  };
}

// ---------------------------------------------------------------- the context window

/// The four parts of the context window, in their fixed order, with their fixed labels, colours and
/// icons, here and in a request's detail alike (OV-C121).
export const CONTEXT: readonly { readonly key: string; readonly label: string; readonly concept: Concept; readonly colour: Part['colour'] }[] = [
  { key: 'tools', label: 'tools (definitions)', concept: 'tools', colour: '--ar-ctx-tools' },
  { key: 'system', label: 'system prompt', concept: 'systemPrompt', colour: '--ar-ctx-system' },
  { key: 'history', label: 'conversation', concept: 'conversation', colour: '--ar-ctx-history' },
  { key: 'tool_results', label: 'tool results', concept: 'toolResults', colour: '--ar-ctx-results' },
];

/// The estimate's own words, beside the figures (OV-C122), naming both providers' models, which the
/// split apportions alike (OV-C230).
export const CONTEXT_NOTE =
  "Totals are exact. The split apportions the API's exact prompt-token count by character share — there is no local tokenizer for current Claude or Kimi models, so treat component ratios as reliable and absolutes as approximate.";

/// Each part of the window: its estimated tokens and share, "≈ per request" over the requests that
/// carried it (OV-C123), and its cost as the floor it is, with its characters, on the page (OV-C124).
export function contextParts(rows: readonly ContextRow[]): Part[] {
  const byKey = new Map(rows.map((r) => [r.component, r]));
  return CONTEXT.map((c) => {
    const r = byKey.get(c.key);
    const v = r?.est_tokens ?? null;
    const per = r && has(r.requests) && r.requests > 0 && has(v) ? `≈${compact(v / r.requests)} per request` : null;
    const cost = r && has(r.est_cost_once) ? `≥${usd(r.est_cost_once)} if paid once as input` : null;
    const chars = r && has(r.chars) ? `${compact(r.chars)} characters` : null;
    return {
      key: c.key,
      label: c.label,
      value: r ? v : null,
      colour: c.colour,
      concept: c.concept,
      figure: compact(r ? v : null),
      detail: [per, cost, chars].filter(Boolean).join(' · ') || undefined,
    };
  });
}

// ---------------------------------------------------------------- models and threads

/// A family's swatch colour.
export const familyColour = (f: Family) => FAMILY_COLOUR[f];

/// A list row's cost: the sum as `costSum` writes it, and its marks' reasons as the row's tooltip
/// (OV-C229, OV-C232). The rows are buttons, so the reasons are also said once under the list
/// (`costNote`).
export interface Costed {
  readonly cost: CostFigure;
  readonly metaTip?: string;
}

const costed = (r: Parameters<typeof costSum>[0]): Costed => {
  const cost = costSum(r);
  const tip = costWhy(cost);
  return tip ? { cost, metaTip: tip } : { cost };
};

/// By model: one row per model in the router's order, prompt tokens read, the family's colour, and
/// "N req · out · % cached · cost", with subagent requests and errors when there are any (OV-C126).
/// The full id is the row's second line, and its provider is tagged beside it (OV-C231). A model the
/// router could not price reads "not priced", never "$0" (OV-C177, OV-C229).
export function modelRows(models: readonly ModelRow[]): (BarRow & Costed & { readonly provider: Provider })[] {
  return models.map((m) => {
    const f = family(m.model);
    const c = costed(m);
    return {
      ...c,
      key: m.model,
      provider: providerOf(m),
      name: shortModel(m.model),
      path: m.model,
      value: m.prompt_tokens,
      figure: compact(m.prompt_tokens),
      colour: FAMILY_COLOUR[f],
      meta: [
        `${count(m.requests)} req`,
        `${compact(m.output_tokens)} out`,
        `${pct(m.cache_read_tokens, m.prompt_tokens)} cached`,
        c.cost.text,
        n(m.subagent_requests) ? `${count(m.subagent_requests)} by subagents` : null,
      ]
        .filter(Boolean)
        .join(' · '),
      errors: n(m.errors) ? plural(m.errors, 'error', 'errors') : undefined,
    };
  });
}

/// Main thread vs subagents: exactly two parts, a request from before agent ids were recorded
/// counting as main (OV-C127), each with its tokens and share, then "N req · out · cost · % cached",
/// and for subagents how many agents, or "none in scope" (OV-C128). A cost is its priced part,
/// marked, each mark's reason in the part's tip (OV-C229, OV-C232).
export function threadParts(threads: readonly ThreadRow[]): Part[] {
  const main = threads.find((t) => t.thread === 'main');
  const sub = threads.find((t) => t.thread === 'subagent');
  const cost = (t: ThreadRow | undefined) => costSum({ cost: t ? t.cost : 0, unpriced: t?.unpriced, estimated: t?.estimated });
  const line = (t: ThreadRow | undefined) =>
    `${count(n(t?.requests))} req · ${compact(n(t?.output_tokens))} out · ${cost(t).text} · ${pct(t?.cache_read_tokens ?? null, t?.prompt_tokens ?? null)} cached`;
  const tip = (t: ThreadRow | undefined) => cost(t).marks.map((m) => ({ value: m.value, label: m.label }));
  return [
    { key: 'main', label: 'main thread', value: n(main?.prompt_tokens), colour: '--ar-tok', concept: 'mainThread', figure: compact(n(main?.prompt_tokens)), detail: line(main), tip: tip(main) },
    {
      key: 'sub',
      label: 'subagents',
      value: n(sub?.prompt_tokens),
      colour: '--ar-subagent',
      concept: 'subagents',
      figure: compact(n(sub?.prompt_tokens)),
      detail: n(sub?.requests) ? `${line(sub)} · ${plural(n(sub?.agents), 'agent', 'agents')}` : 'none in scope',
      tip: n(sub?.requests) ? tip(sub) : [],
    },
  ];
}

// ---------------------------------------------------------------- the lists

/// The caption of a list that scopes the page: how many, and how it works (OV-C130).
export function scopingCaption(what: string, rows: number, selected: boolean): string {
  return selected ? `click the highlighted ${what} again to clear` : `${plural(rows, what, `${what}s`)} · click one to scope the page`;
}

/// The Accounts list shows only once more than one account served traffic in the range (OV-C133).
export function accountSplit(list: readonly AccountSplitRow[]): AccountSplitRow[] | null {
  const served = list.filter((a) => n(a.requests) > 0);
  return served.length > 1 ? served : null;
}

/// Whether an account signs its provider's new requests: one account per provider can (OV-C231).
export const isActive = (a: Pick<AccountSplitRow, 'active'>): boolean => a.active === true || n(a.active as number | null) > 0;

/// What an account's provider tag says beside its mark: the provider's name, and while the account
/// signs that provider's new requests, "active for" it, since an Anthropic account and a Kimi Code
/// account can both be active at once (OV-C231).
export const providerWords = (provider: Provider, active: boolean): string => (active ? `active for ${providerName(provider)}` : providerName(provider));

/// The providers a session's requests went to, Anthropic's first as the router lists them; Anthropic's
/// alone for a row that lists none, recorded before providers were (SE-C242).
export function providersOf(s: Pick<SessionRow, 'providers'>): Provider[] {
  const listed = [...new Set((s.providers ?? []).map((p) => providerOf({ provider: p })))];
  return listed.length ? listed : ['anthropic'];
}

/// An account row: its icon, its label and router id, "active" while it serves, a bar of prompt
/// tokens and "N sessions · N req · out · <last seen>" (OV-C134, OV-C17), and its provider, whose
/// tag says "active for" it while it serves (OV-C231).
export function accountRows(list: readonly AccountSplitRow[], now: number = Date.now()): (BarRow & { readonly active: boolean; readonly provider: Provider })[] {
  return list.map((a) => ({
    key: a.id,
    name: a.label || a.id,
    path: a.id,
    concept: 'account',
    active: isActive(a),
    provider: providerOf(a),
    value: a.prompt_tokens,
    figure: compact(a.prompt_tokens),
    meta: [plural(n(a.sessions), 'session', 'sessions'), `${count(a.requests)} req`, `${compact(a.output_tokens)} out`, ago(a.last_seen, now)].join(' · '),
  }));
}

/// The families a project or a session used, one swatch each, four at most, as its chips.
export function familiesOf(csv: string | null | undefined): { readonly family: Family; readonly model: string }[] {
  const seen = new Set<Family>();
  const out: { family: Family; model: string }[] = [];
  for (const m of String(csv ?? '').split(',').filter(Boolean)) {
    const f = family(m);
    if (seen.has(f)) continue;
    seen.add(f);
    out.push({ family: f, model: m });
  }
  return out.slice(0, 4);
}

/// A project's key: its repository, else its directory, "(unresolved)" when it has neither, which
/// the router selects exactly those sessions for (OV-C136, OV-C214).
export const projectKeyOf = (p: Pick<ProjectRow, 'project'>): string => p.project ?? UNRESOLVED;

/// Where a project's sessions ran: its repository when it has one, and every directory, each said
/// "studio · /Users/sample/code/ui-kit-web", the path alone where no machine is named, as on a local
/// router (OV-C214).
export interface ProjectPlaces {
  readonly repository: string | null;
  readonly directories: readonly { readonly machine: string | null; readonly path: string; readonly label: string }[];
}

/// A directory's machine by its label, trimmed; a blank one names none (SE-C220).
const placeOf = (d: ProjectDirectory): ProjectPlaces['directories'][number] => {
  const machine = d.machine?.label?.trim() || null;
  return { machine, path: d.cwd, label: machine ? `${machine} · ${d.cwd}` : d.cwd };
};

/// The scoped project's places, from the first list that holds its row by its key: the rail's, over
/// all time, then the snapshot's, narrowed by the range. Null with no project scoped, with its row
/// in neither list, or with nothing to say — an unresolved project has no directory (OV-C214).
export function projectPlaces(key: string | null, ...lists: readonly (readonly ProjectRow[] | null | undefined)[]): ProjectPlaces | null {
  if (!key) return null;
  const row = lists.map((l) => l?.find((p) => projectKeyOf(p) === key)).find((p) => p !== undefined);
  if (!row) return null;
  const directories = row.directories.map(placeOf);
  return row.repository || directories.length ? { repository: row.repository, directories } : null;
}

/// A project row: the project's icon, its folder's name and path, a bar of prompt tokens with the
/// subagent share over its end, and "N sessions · req · out · % cached · cost · <last seen>"
/// (OV-C135); errors beside them in the error mark (OV-C5); its cost marked, with its reasons as the
/// row's tooltip (OV-C229, OV-C232).
export function projectRows(list: readonly ProjectRow[], now: number = Date.now()): (BarRow & Costed)[] {
  return list.map((p) => {
    const prompt = p.prompt_tokens;
    const sub = p.subagent_tokens;
    const c = costed(p);
    return {
      ...c,
      key: projectKeyOf(p),
      name: projectName(p.project),
      path: p.project ?? undefined,
      concept: 'project',
      value: prompt,
      sub,
      figure: compact(prompt),
      subFigure: n(sub) ? `${pct(sub, prompt)} subagents` : undefined,
      meta: [
        plural(n(p.sessions), 'session', 'sessions'),
        `${count(p.requests)} req`,
        `${compact(p.output_tokens)} out`,
        `${pct(p.cache_read_tokens, prompt)} cached`,
        c.cost.text,
        ago(p.last_seen, now),
      ].join(' · '),
      errors: n(p.errors) ? plural(p.errors, 'error', 'errors') : undefined,
    };
  });
}

export interface SessionChips {
  readonly branch: string | null;
  /// The project, only when no project is scoped.
  readonly project: { readonly name: string; readonly path: string } | null;
  /// "VS Code", "CLI", "Kimi Code CLI"…, with the whole entrypoint (OV-C187, SE-C234).
  readonly client: { readonly name: string; readonly entrypoint: string } | null;
  readonly families: readonly { readonly family: Family; readonly model: string }[];
}

/// A session row: its icon, its first eight characters, chips for its branch, its project when none
/// is scoped, its client and its families, a bar with the subagent share over it, and "N req · cost ·
/// out · N subagents · started <time> · <duration>" (OV-C137); the providers of its requests are
/// marked beside its client (SE-C242); its cost marked, with its reasons as the row's tooltip
/// (OV-C229, OV-C232).
export function sessionRows(list: readonly SessionRow[], scope: Scope, now: number = Date.now()): (BarRow & Costed & { readonly chipsOf: SessionChips; readonly providers: readonly Provider[] })[] {
  return list.map((s) => {
    const prompt = s.prompt_tokens;
    const sub = s.subagent_tokens;
    const ran = client(s.entrypoint, s.client);
    const c = costed(s);
    return {
      ...c,
      key: s.id,
      providers: providersOf(s),
      name: id8(s.id),
      mono: true,
      concept: 'session',
      chipsOf: {
        branch: s.git_branch,
        project: scope.project ? null : { name: projectName(s.project), path: s.project ?? UNRESOLVED },
        client: ran && s.entrypoint ? { name: ran, entrypoint: s.entrypoint } : null,
        families: familiesOf(s.models),
      },
      value: prompt,
      sub,
      figure: compact(prompt),
      subFigure: n(sub) ? `${pct(sub, prompt)} subagents` : undefined,
      meta: [
        `${count(s.requests)} req`,
        c.cost.text,
        `${compact(s.output_tokens)} out`,
        n(s.subagents) ? plural(s.subagents, 'subagent', 'subagents') : null,
        `started ${tod(s.started_at, now)}`,
        has(s.started_at) && has(s.last_seen_at) ? duration(s.last_seen_at - s.started_at) : DASH,
      ]
        .filter(Boolean)
        .join(' · '),
      errors: n(s.errors) ? plural(s.errors, 'error', 'errors') : undefined,
    };
  });
}

/// The Sessions caption: the scope, the count, "200 most recent" at the router's cap (OV-C138), and
/// how the list works (OV-C130).
export function sessionsCaption(n: number, scope: Scope): string {
  const where = scope.project ? `in ${projectName(scope.project)}` : 'all projects';
  const held = n >= SESSION_CAP ? `${count(SESSION_CAP)} most recent` : plural(n, 'session', 'sessions');
  return `${where} · ${held} · ${scope.session ? 'click the highlighted session again to clear' : 'click one to scope the page'}`;
}

/// A long list shows its first page and a visible "Show more" (OV-C139).
export const LIST_PAGE = 20;

export interface Unattributed {
  readonly label: string;
  readonly meta: string;
  /// Beside the row, never in a tip (OV-C142).
  readonly why: string;
}

/// The "not yet attributed" row: the router's own count of requests no session holds (OV-C141),
/// under Projects, and under Sessions when no project is scoped; absent when there are none or a
/// session is scoped (OV-C140).
export function unattributed(count_: number | null | undefined, list: 'project' | 'session', scope: Scope): Unattributed | null {
  if (!has(count_) || count_ <= 0 || scope.session) return null;
  if (list === 'session' && scope.project) return null;
  return {
    label: 'not yet attributed',
    meta: `${plural(count_, 'request', 'requests')} · no ${list} recorded yet`,
    // Each client by its name (OV-C230).
    why: "Claude Code and Kimi Code each label a request with its session's id; the project comes from the session's own files, which can appear a moment later. These requests are counted in the totals above.",
  };
}

/// Tool output the model read: one row per tool by estimated tokens, "N results · ≈X each · ≥$cost",
/// its characters and largest result, and "N failed" in the error mark (OV-C143, OV-C124).
export function toolOutputRows(rows: readonly ToolOutputRow[]): BarRow[] {
  return rows.map((t) => {
    const results = n(t.results);
    return {
      key: t.name ?? DASH,
      name: t.name ?? DASH,
      concept: 'tools',
      colour: '--ar-ctx-tools',
      value: t.est_tokens,
      figure: compact(t.est_tokens),
      meta: [
        plural(results, 'result', 'results'),
        results && has(t.est_tokens) ? `≈${compact(t.est_tokens / results)} each` : null,
        has(t.est_cost_once) ? `≥${usd(t.est_cost_once)} if paid once as input` : null,
        has(t.chars) ? `${compact(t.chars)} characters` : null,
        has(t.max_tokens) ? `largest ${compact(t.max_tokens)}` : null,
      ]
        .filter(Boolean)
        .join(' · '),
      errors: n(t.errors) ? `${count(t.errors)} failed` : undefined,
    };
  });
}

/// Tool calls made: one row per tool by calls, the count exact, the calls by subagents over the bar's
/// end, and "N calls · in M requests" (OV-C144).
export function toolCallRows(rows: readonly ToolCallRow[]): BarRow[] {
  return rows.map((t) => {
    const calls = n(t.calls);
    const sub = n(t.subagent_calls);
    return {
      key: t.name ?? DASH,
      name: t.name ?? DASH,
      concept: 'tools',
      colour: '--ar-ctx-tools',
      value: calls,
      sub,
      figure: count(calls),
      subFigure: sub ? `${count(sub)} by subagents` : undefined,
      meta: [plural(calls, 'call', 'calls'), has(t.requests) ? `in ${plural(t.requests, 'request', 'requests')}` : null].filter(Boolean).join(' · '),
    };
  });
}
