// Copied from Northstar ui/src/screens/Palette/hits.ts at e9c1589.
// Changed: the hits are the router's. Words are searched in its conversation index (/api/search), whose hits run by kind, and matched against its sessions and projects to jump to; `>` lists commands that write to it (switch the serving account; set a session's model, effort or mode; archive or restore a session), the theme, and the pages; a command that needs a session or a value it was not given asks for it first, as a step. The resolver returns runs rather than flat hits, so a kind is one run however its hits interleave. Northstar's references, bare numbers, items and saved views are gone, and with them its cap of twelve: a jump run holds six and says how many it held back.
import type { ThemeChoice } from '~/app/theme';
import type { Availability } from '~/data/held';
import { projectKey, projectName, UNRESOLVED } from '~/data/projects';
import type { ProjectRow } from '~/data/router';
import { SECTIONS, routes, settingsPath } from '~/lib/routes';
import { kindRun, marks } from '~/model/format';
import type { Account, LiveOptions, LiveSession, SearchAnswer, SearchHit, SearchKind } from './api';

/// What the palette resolves a line of typing into. Pure, so the rules can be tested without a
/// modal, and so the component is only a keyboard, a list and the requests.
///
/// The rules, in the order they are tried:
///
/// 1. Inside a command's step, the typing filters that step's choices: which session, which
///    value.
/// 2. `>` leads a command.
/// 3. Nothing typed lists where to jump: the sessions the router lists, and its projects.
/// 4. Anything else is words, matched against those sessions and projects by name and searched
///    in the router's index. The index's hits run by kind, the run holding the best match first.

export const COMMAND_PREFIX = '>';

/// How many sessions, and how many projects, a list shows before saying how many it held back.
export const MAX_JUMPS = 6;

/// The search asks for as many hits as the live view's Find does, best match first (SE-C165).
export const SEARCH_LIMIT = 30;

/// One request per pause, not one per letter: the dashboard's interval (OV-C39).
export const SEARCH_DEBOUNCE_MS = 160;

/// The kinds the search can be narrowed to, as the Overview's switcher offers them, all first and
/// the default (OV-C38).
export const SEARCH_KINDS: readonly { readonly kind: SearchKind | null; readonly label: string }[] = [
  { kind: null, label: 'All' },
  { kind: 'text', label: 'Messages' },
  { kind: 'tool_use', label: 'Tool calls' },
  { kind: 'tool_result', label: 'Results' },
  { kind: 'thinking', label: 'Thinking' },
];

/// What a run of hits of each kind is called: the reader's words, the same on every client
/// (OV-C50), from the one table the model keeps for a hit's kind and a run's (`kindRun` in
/// ~/model/format). A kind the router indexes later is shown as it is sent.
export { kindRun };

/// What a session can be switched, each through its own endpoint (src/live.rs).
export type Setting = 'model' | 'effort' | 'mode';
const SETTINGS: readonly Setting[] = ['model', 'effort', 'mode'];

export type SessionCommand = Setting | 'archive' | 'restore';

/// A session as a command names it: its id, and the words a reader knows it by.
export interface SessionRef {
  readonly id: string;
  readonly name: string;
}

/// Where a command that needs more stands: asking which session, asking which value, or asking
/// whether to bypass permissions after all.
export type Step =
  | { readonly kind: 'session'; readonly command: SessionCommand }
  | { readonly kind: 'value'; readonly setting: Setting; readonly session: SessionRef }
  | { readonly kind: 'bypass'; readonly session: SessionRef };

/// What a write did, in words for the notice that follows the palette closing.
export interface Outcome {
  readonly title: string;
}

/// A write to the router: where it goes, what it sends, and the words for each outcome. Sent with
/// `postJson`, so a refusal is read the way every other one is.
export interface Post {
  readonly path: string;
  readonly body: Readonly<Record<string, unknown>>;
  /// While it is in flight.
  readonly doing: string;
  /// The title its refusal is shown under, beside the control.
  readonly refused: string;
  /// What happened, from the router's answer.
  readonly done: (answer: unknown) => Outcome;
}

export type Action =
  | { readonly type: 'go'; readonly href: string }
  | { readonly type: 'step'; readonly step: Step }
  | { readonly type: 'back' }
  | { readonly type: 'post'; readonly post: Post }
  | { readonly type: 'theme'; readonly choice: ThemeChoice };

export interface Hit {
  readonly id: string;
  readonly label: string;
  readonly detail?: string;
  /// An identifier beside the detail, set in the mono style: a session's short id, an account's
  /// router id, a mode's value.
  readonly ident?: string;
  /// A word about the thing, kept apart from its identifier: "archived", "serving now".
  readonly note?: string;
  /// When it happened, in epoch milliseconds, for the row's age.
  readonly at?: number;
  /// A search hit's context, its matches between ‹ and ›.
  readonly snippet?: string;
  /// Words a command is also found by.
  readonly keywords?: string;
  /// What choosing it does. Null when it can do nothing: `why` says why, and choosing it goes
  /// nowhere rather than to a dead end.
  readonly action: Action | null;
  readonly why?: string;
}

/// Hits of one kind, said once above them.
export interface Run {
  readonly key: string;
  readonly label: string;
  readonly hits: readonly Hit[];
  /// How many matched, when the run holds back some. Absent when it shows them all.
  readonly of?: number;
}

export interface Notice {
  readonly tone: 'hint' | 'reading' | 'unavailable' | 'warning';
  readonly title: string;
  readonly text: string;
  /// The read's own failure, for `Refusal` to word: refused, unreachable and empty apart.
  readonly error?: unknown;
}

export type Mode = 'empty' | 'text' | 'command' | 'step';

export interface Resolved {
  readonly mode: Mode;
  readonly runs: readonly Run[];
  readonly notices: readonly Notice[];
}

/// A source the palette reads, in one of three states (`availabilityOf` in ~/data/held), with
/// what it holds and, when it could not be read, why.
export interface Source<T> {
  readonly state: Availability;
  readonly value: T;
  readonly error?: unknown;
}

/// How far the search has got with the words typed: nothing to ask, asked and not yet answered
/// (or not yet asked, while the typing has not paused), answered, or failed.
export type SearchState =
  | { readonly status: 'idle' }
  | { readonly status: 'waiting' }
  | { readonly status: 'read'; readonly answer: SearchAnswer }
  | { readonly status: 'failed'; readonly error: unknown };

export interface Context {
  /// The step a command stands at, if one does.
  readonly step: Step | null;
  readonly projects: Source<readonly ProjectRow[]>;
  readonly sessions: Source<readonly LiveSession[]>;
  readonly accounts: Source<readonly Account[]>;
  readonly options: Source<LiveOptions | null>;
  /// The session open on the Sessions screen, from its address (SE-C189); null anywhere else.
  readonly onScreen: string | null;
  readonly theme: ThemeChoice;
  readonly search: SearchState;
}

/// Every word typed must appear somewhere in the hit's words, in any order: `model agent`
/// finds "Set the model…" of the session in agent-router without a phrase match.
function matchesWords(needle: string, ...hay: (string | null | undefined)[]): boolean {
  const text = hay.filter((h): h is string => !!h).join(' ').toLowerCase();
  return needle.split(/\s+/).filter(Boolean).every((w) => text.includes(w));
}

/// A session's id the way the lists shorten it: its first eight characters (SE-C26).
export const shortId = (id: string): string => id.slice(0, 8);

/// What a reader knows a session by: its title, else its short id.
export function sessionName(s: Pick<LiveSession, 'session_id' | 'title'>): string {
  return s.title?.trim() || shortId(s.session_id);
}

/// The project a session belongs to, as the Sessions list names it: the last segment of its
/// directory, else the router's own name for it (SE-C23).
function sessionProject(s: Pick<LiveSession, 'cwd' | 'project'>): string {
  return s.cwd ? projectName(s.cwd) : s.project || UNRESOLVED;
}

/// Where a session opens: the Sessions screen, at `?session=<id>` (SE-C189). With `at`, at one
/// entry of it, a search hit's place, named by its block and its ordinal together and never by
/// its block alone (SE-C171), for the Sessions screen to land on.
export function sessionHref(id: string, at?: { readonly block: number; readonly ordinal: number }): string {
  const params = new URLSearchParams({ session: id });
  if (at) {
    params.set('block', String(at.block));
    params.set('ordinal', String(at.ordinal));
  }
  return `${routes.sessions}?${params}`;
}

/// A snippet's plain and matched runs, split on the ‹ › pair the router marks matches with and
/// never substituted into: a marker the conversation itself contained can only shift which run
/// is emphasised, never leave a mark open, drop text or make the snippet markup (SE-C170). The
/// splitting is the model's (`marks` in ~/model/format), the one copy every screen marks with.
export function markRuns(snippet: string): { readonly text: string; readonly marked: boolean }[] {
  return marks(snippet).map((r) => ({ text: r.text, marked: r.mark }));
}

/// A model name the CLI could accept, by the router's own rule (src/live.rs, `valid_model`): an
/// alias, an id, an optional `[1m]`, at most 64 characters.
export function validModel(m: string): boolean {
  const t = m.trim();
  return t !== '' && t.length <= 64 && /^[A-Za-z0-9\-._[\]:/]+$/.test(t);
}

const isRecord = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null && !Array.isArray(v);

/// Newest first, as a jump is most often to what was just being worked on.
const byActivity = (a: LiveSession, b: LiveSession) => b.last_activity_ms - a.last_activity_ms;

/// What stands in for a source that has not been read: a read on its way, or why it could not be
/// read and what that means here. Three states, never conflated: zero rows never stand in for a
/// list that was not read (Northstar docs/13 §1, §2).
function sourceNotices(source: Source<unknown>, what: string, consequence: string): Notice[] {
  switch (source.state) {
    case 'read':
      return [];
    case 'loading':
      return [{ tone: 'reading', title: what, text: '' }];
    case 'unreachable':
      return [{ tone: 'unavailable', title: `The ${what} could not be read`, text: consequence, error: source.error }];
  }
}

// ---------------------------------------------------------------------------------------------
// Jumps
// ---------------------------------------------------------------------------------------------

function sessionHit(s: LiveSession): Hit {
  const name = sessionName(s);
  return {
    id: `session:${s.session_id}`,
    label: name,
    detail: sessionProject(s),
    ident: name === shortId(s.session_id) ? (s.git_branch ?? undefined) : shortId(s.session_id),
    ...(s.archived ? { note: 'archived' } : {}),
    at: s.last_activity_ms,
    action: { type: 'go', href: sessionHref(s.session_id) },
  };
}

function projectHit(p: ProjectRow): Hit {
  return {
    id: `project:${projectKey(p)}`,
    label: projectName(p.project),
    ...(p.project ? { detail: p.project } : {}),
    ...(p.last_seen !== null ? { at: p.last_seen } : {}),
    action: { type: 'go', href: routes.project(projectKey(p)) },
  };
}

function jumpRun(key: string, label: string, hits: readonly Hit[]): Run {
  return { key, label, hits: hits.slice(0, MAX_JUMPS), ...(hits.length > MAX_JUMPS ? { of: hits.length } : {}) };
}

// ---------------------------------------------------------------------------------------------
// The search
// ---------------------------------------------------------------------------------------------

/// Why a session cannot be opened from here, if it cannot. A session opens from the router's list
/// of sessions, so one the list does not hold, a list still on its way and a list that could not
/// be read are three different reasons, and "no longer listed" is said for none of the others
/// (SE-C173). A hosted router lists only what its connected machines run, so the words say what
/// is known, that the list does not hold it, rather than why.
function unopenable(ctx: Context, id: string): string | null {
  switch (ctx.sessions.state) {
    case 'loading':
      return 'The router’s list of sessions is still being read, so this cannot be opened yet.';
    case 'unreachable':
      return 'The router’s list of sessions could not be read, so this cannot be opened from here.';
    case 'read':
      return ctx.sessions.value.some((s) => s.session_id === id) ? null : 'The router does not list that session, so it cannot be opened.';
  }
}

function searchHit(h: SearchHit, ctx: Context): Hit {
  const why = unopenable(ctx, h.session_id);
  return {
    // Its session, block and ordinal together: one block appears at every place it was
    // replayed, and a list keyed on the block alone would drop rows (SE-C171).
    id: `hit:${h.session_id}:${h.block_id}:${h.ordinal}`,
    // The project as the lists name it, "(unresolved)" when its transcript never arrived (OV-C52).
    label: h.project ? projectName(h.project) : UNRESOLVED,
    detail: h.role,
    ident: shortId(h.session_id),
    at: h.at,
    snippet: h.snippet,
    action: why === null ? { type: 'go', href: sessionHref(h.session_id, { block: h.block_id, ordinal: h.ordinal }) } : null,
    ...(why === null ? {} : { why }),
  };
}

/// The search's hits, one run per kind. A Map keeps the order each kind first appears in the
/// router's best-first list, so the run holding the best match leads, and inside a run the
/// router's order stands.
function searchRuns(ctx: Context): Run[] {
  if (ctx.search.status !== 'read') return [];
  const runs = new Map<string, Hit[]>();
  for (const h of ctx.search.answer.hits) {
    const run = runs.get(h.kind);
    if (run) run.push(searchHit(h, ctx));
    else runs.set(h.kind, [searchHit(h, ctx)]);
  }
  return [...runs].map(([kind, hits]) => ({ key: `kind:${kind}`, label: kindRun(kind), hits }));
}

// ---------------------------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------------------------

const NOT_WRAPPED = 'Only a session under the wrapper can be switched; restart it after installing the wrapper.';

/// The outcome of a switch, in the router's words when it has any (SE-C143): the CLI's own
/// notices, else that the session has not confirmed yet, else what was set and when it applies.
/// The mode is what the CLI says it took, which it echoes.
function switched(setting: Setting, value: string, answer: unknown): Outcome {
  const a = isRecord(answer) ? answer : {};
  const notices = Array.isArray(a['notices']) ? a['notices'].filter((n): n is string => typeof n === 'string' && n.trim() !== '') : [];
  if (notices.length > 0) return { title: notices.join(' ') };
  if (a['pending'] === true) return { title: `${setting} sent — the session has not confirmed yet` };
  const took = typeof a[setting] === 'string' ? a[setting] : value;
  return { title: `${setting} set to ${took} — takes effect on the next turn` };
}

/// Set a session's model, effort or mode: the body the router reads is `{session, <setting>}`.
export function switchPost(setting: Setting, session: SessionRef, value: string): Post {
  return {
    path: `/api/live/${setting}`,
    body: { session: session.id, [setting]: value },
    doing: `Setting the ${setting} of ${session.name}…`,
    refused: `Could not set the ${setting}`,
    done: (answer) => switched(setting, value, answer),
  };
}

export function archivePost(session: SessionRef, archived: boolean): Post {
  return {
    path: '/api/live/archive',
    body: { session: session.id, archived },
    doing: `${archived ? 'Archiving' : 'Restoring'} ${session.name}…`,
    refused: `Could not ${archived ? 'archive' : 'restore'} that session`,
    done: () => ({ title: `${session.name} ${archived ? 'archived' : 'restored'}` }),
  };
}

export function activatePost(account: Account): Post {
  const name = account.email || account.label;
  return {
    path: '/api/accounts/activate',
    body: { id: account.id },
    doing: `Switching to ${name}…`,
    refused: `Could not switch to ${name}`,
    done: () => ({ title: `${name} signs requests now` }),
  };
}

/// What each session command is called. With a session on screen it acts on that one; without,
/// it asks which first, in the words of `which`.
const COMMAND_LABEL: Readonly<Record<SessionCommand, { here: string; ask: string; which: string }>> = {
  model: { here: 'Set the model…', ask: 'Set a session’s model…', which: 'Which session’s model?' },
  effort: { here: 'Set the effort…', ask: 'Set a session’s effort…', which: 'Which session’s effort?' },
  mode: { here: 'Set the mode…', ask: 'Set a session’s mode…', which: 'Which session’s mode?' },
  archive: { here: 'Archive this session', ask: 'Archive a session…', which: 'Which session to archive?' },
  restore: { here: 'Restore this session', ask: 'Restore a session…', which: 'Which session to restore?' },
};

/// The session commands. With a session on screen they act on it, its switches greyed with the
/// reason when the router lists it as unwrapped (SE-C137), and archive or restore whichever it
/// needs; without one, each asks which session first.
function sessionCommands(ctx: Context): Hit[] {
  const id = ctx.onScreen;
  if (id === null) {
    return (['model', 'effort', 'mode', 'archive', 'restore'] as const).map((command): Hit => ({
      id: `cmd:${command}`,
      label: COMMAND_LABEL[command].ask,
      detail: 'asks which session',
      keywords: 'session',
      action: { type: 'step', step: { kind: 'session', command } },
    }));
  }
  const listed = ctx.sessions.value.find((s) => s.session_id === id);
  const session: SessionRef = { id, name: listed ? sessionName(listed) : shortId(id) };
  const unwrapped = listed !== undefined && !listed.wrapped;
  const switches = SETTINGS.map((setting): Hit => ({
    id: `cmd:${setting}`,
    label: COMMAND_LABEL[setting].here,
    detail: session.name,
    keywords: 'session switch',
    action: unwrapped ? null : { type: 'step', step: { kind: 'value', setting, session } },
    ...(unwrapped ? { why: NOT_WRAPPED } : {}),
  }));
  const archived = listed?.archived === true;
  return [
    ...switches,
    {
      id: archived ? 'cmd:restore' : 'cmd:archive',
      label: COMMAND_LABEL[archived ? 'restore' : 'archive'].here,
      detail: session.name,
      keywords: 'session put away shelf',
      action: { type: 'post', post: archivePost(session, !archived) },
    },
  ];
}

/// Switch the account that signs: one per account, the router's id beside its name (OV-C17), in
/// one choice with no confirmation (ST-C24). The one signing now is marked and does nothing.
function accountCommands(ctx: Context): Hit[] {
  return ctx.accounts.value.map((a): Hit => {
    const serving = Number(a.active) === 1;
    return {
      id: `cmd:account:${a.id}`,
      label: `Use ${a.email || a.label}`,
      ident: a.id,
      keywords: 'account switch sign',
      ...(serving ? { note: 'serving now' } : {}),
      action: serving ? null : { type: 'post', post: activatePost(a) },
    };
  });
}

const THEMES: readonly { readonly choice: ThemeChoice; readonly label: string }[] = [
  { choice: 'system', label: 'Theme: follow the system' },
  { choice: 'light', label: 'Theme: light' },
  { choice: 'dark', label: 'Theme: dark' },
];

function themeCommands(ctx: Context): Hit[] {
  return THEMES.map((t): Hit => ({
    id: `cmd:theme:${t.choice}`,
    label: t.label,
    keywords: 'appearance colour color',
    ...(ctx.theme === t.choice ? { note: 'in use' } : {}),
    action: { type: 'theme', choice: t.choice },
  }));
}

/// The pages, each where the rail and the settings tabs go (`routes` in ~/lib/routes). Machines and
/// the Library are listed on every router, as every Settings page is: on a local router their
/// addresses say why they have nothing to show (MA-C1, MA-C7, LB-C1).
function pageCommands(): Hit[] {
  return [
    { id: 'go:overview', label: 'Overview', keywords: 'go page', action: { type: 'go', href: routes.overview } },
    { id: 'go:sessions', label: 'Sessions', keywords: 'go page live', action: { type: 'go', href: routes.sessions } },
    { id: 'go:machines', label: 'Machines', keywords: 'go page connect install agent', action: { type: 'go', href: routes.machines } },
    { id: 'go:library', label: 'Library', keywords: 'go page skills commands subagents publish', action: { type: 'go', href: routes.library } },
    ...SECTIONS.map(
      (s): Hit => ({ id: `go:settings:${s.slug}`, label: `Settings: ${s.label}`, keywords: 'go page', action: { type: 'go', href: settingsPath(s.slug) } }),
    ),
  ];
}

/// Every command, in its runs, before filtering.
export function commands(ctx: Context): Run[] {
  return [
    { key: 'commands:session', label: ctx.onScreen === null ? 'Sessions' : 'This session', hits: sessionCommands(ctx) },
    { key: 'commands:account', label: 'Accounts', hits: accountCommands(ctx) },
    { key: 'commands:theme', label: 'Theme', hits: themeCommands(ctx) },
    { key: 'commands:page', label: 'Go to', hits: pageCommands() },
  ];
}

// ---------------------------------------------------------------------------------------------
// Steps
// ---------------------------------------------------------------------------------------------

/// What a step asks, as the bar over its choices says it.
export function describeStep(step: Step): string {
  switch (step.kind) {
    case 'session':
      return COMMAND_LABEL[step.command].which;
    case 'value':
      return `Set the ${step.setting} of ${step.session.name}`;
    case 'bypass':
      return `Bypass permissions on ${step.session.name}?`;
  }
}

function pickSession(s: LiveSession, command: SessionCommand): Hit {
  const session: SessionRef = { id: s.session_id, name: sessionName(s) };
  const base: Hit = { ...sessionHit(s), id: `pick:${s.session_id}` };
  if (command === 'archive' || command === 'restore') {
    return { ...base, action: { type: 'post', post: archivePost(session, command === 'archive') } };
  }
  if (!s.wrapped) return { ...base, action: null, why: NOT_WRAPPED };
  return { ...base, action: { type: 'step', step: { kind: 'value', setting: command, session } } };
}

function sessionStep(command: SessionCommand, needle: string, ctx: Context): Resolved {
  const wanted = ctx.sessions.value.filter((s) => (command === 'archive' ? !s.archived : command === 'restore' ? s.archived : true));
  const hits = wanted
    .filter((s) => !needle || matchesWords(needle, s.title, s.session_id, s.cwd, s.project, s.git_branch))
    .sort(byActivity)
    .map((s) => pickSession(s, command));
  return {
    mode: 'step',
    runs: hits.length > 0 ? [{ key: 'pick:session', label: 'Sessions', hits }] : [],
    notices: sourceNotices(ctx.sessions, 'sessions', 'No session can be chosen from here.'),
  };
}

/// What a step's run of values is called.
const VALUE_RUN: Readonly<Record<Setting, string>> = { model: 'Models', effort: 'Effort levels', mode: 'Modes' };

function valueStep(setting: Setting, session: SessionRef, raw: string, ctx: Context): Resolved {
  const options = ctx.options.value;
  const values =
    options === null
      ? []
      : setting === 'mode'
        ? options.modes
        : (setting === 'model' ? options.models : options.efforts).map((v) => ({ value: v, label: v }));
  const needle = raw.toLowerCase();
  const hits = values
    .filter((v) => !needle || matchesWords(needle, v.label, v.value))
    .map((v): Hit => ({
      id: `value:${v.value}`,
      label: v.label,
      ...(v.label === v.value ? {} : { ident: v.value }),
      // Bypass asks first: one choice would otherwise stop the session asking about anything,
      // on a machine the reader may not be sitting at (SE-C142).
      action:
        setting === 'mode' && v.value === 'bypassPermissions'
          ? { type: 'step', step: { kind: 'bypass', session } }
          : { type: 'post', post: switchPost(setting, session, v.value) },
    }));
  // The router's list is not every model the CLI accepts (SE-C141): typed words that could name
  // one are offered as one. Nothing typed offers nothing.
  if (setting === 'model' && validModel(raw) && !values.some((v) => v.value === raw)) {
    hits.push({
      id: 'value:typed',
      label: `Use “${raw}”`,
      detail: 'a model name or alias the list does not hold',
      action: { type: 'post', post: switchPost('model', session, raw) },
    });
  }
  return {
    mode: 'step',
    runs: hits.length > 0 ? [{ key: `pick:${setting}`, label: VALUE_RUN[setting], hits }] : [],
    notices: sourceNotices(ctx.options, 'switch options', 'The values the router offers cannot be listed; a model can still be typed.'),
  };
}


function bypassStep(session: SessionRef): Resolved {
  return {
    mode: 'step',
    notices: [{ tone: 'warning', title: 'Bypass permissions?', text: 'This session will stop asking before it runs commands or edits files.' }],
    runs: [
      {
        key: 'pick:bypass',
        label: 'Bypass permissions',
        hits: [
          {
            id: 'bypass:yes',
            label: `Bypass permissions on ${session.name}`,
            action: { type: 'post', post: switchPost('mode', session, 'bypassPermissions') },
          },
          { id: 'bypass:no', label: 'Keep the current mode', action: { type: 'back' } },
        ],
      },
    ],
  };
}

// ---------------------------------------------------------------------------------------------
// The rules
// ---------------------------------------------------------------------------------------------

export function resolve(query: string, ctx: Context): Resolved {
  const raw = query.trim();

  if (ctx.step) {
    switch (ctx.step.kind) {
      case 'session':
        return sessionStep(ctx.step.command, raw.toLowerCase(), ctx);
      case 'value':
        return valueStep(ctx.step.setting, ctx.step.session, raw, ctx);
      case 'bypass':
        return bypassStep(ctx.step.session);
    }
  }

  if (raw.startsWith(COMMAND_PREFIX)) {
    const needle = raw.slice(COMMAND_PREFIX.length).trim().toLowerCase();
    const runs = commands(ctx)
      .map((run) => ({ ...run, hits: run.hits.filter((c) => !needle || matchesWords(needle, c.label, c.detail, c.ident, c.keywords)) }))
      .filter((run) => run.hits.length > 0);
    return {
      mode: 'command',
      runs,
      notices: sourceNotices(ctx.accounts, 'accounts', 'No account can be switched to from here; the other commands still run.'),
    };
  }

  if (!raw) {
    const sessions = ctx.sessions.value.filter((s) => !s.archived).sort(byActivity).map(sessionHit);
    const projects = ctx.projects.value.map(projectHit);
    const nothing = ctx.sessions.state === 'read' && ctx.projects.state === 'read' && sessions.length === 0 && projects.length === 0;
    return {
      mode: 'empty',
      runs: [jumpRun('jump:session', 'Sessions', sessions), jumpRun('jump:project', 'Projects', projects)].filter((r) => r.hits.length > 0),
      notices: [
        {
          tone: 'hint',
          title: 'Type to search',
          text: nothing
            ? `Nothing has gone through the router yet, so there is nowhere to jump; ${COMMAND_PREFIX} lists commands.`
            : `Words search every conversation the router recorded and find a session or a project by name; ${COMMAND_PREFIX} lists commands.`,
        },
        ...sourceNotices(ctx.sessions, 'sessions', 'No session can be jumped to; the search and the commands still work.'),
        ...sourceNotices(ctx.projects, 'projects', 'No project can be jumped to; the search and the commands still work.'),
      ],
    };
  }

  const needle = raw.toLowerCase();
  const sessions = ctx.sessions.value
    .filter((s) => matchesWords(needle, s.title, s.session_id, s.cwd, s.project, s.git_branch))
    .sort(byActivity)
    .map(sessionHit);
  const projects = ctx.projects.value.filter((p) => matchesWords(needle, p.project ?? UNRESOLVED)).map(projectHit);
  return {
    mode: 'text',
    runs: [jumpRun('jump:session', 'Sessions', sessions), jumpRun('jump:project', 'Projects', projects), ...searchRuns(ctx)].filter(
      (r) => r.hits.length > 0,
    ),
    notices: [],
  };
}
