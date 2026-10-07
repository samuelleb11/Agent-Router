// New for agent-router: the live view's session model (assets/live.html) with no DOM and no
// React. What a session is doing and in which colour, the list's grouping and order, the project
// dot's ranking, the tabs, the stream's ten seconds of silence, the feed, and which form an
// answer goes to. The web view is the reference both phones follow (docs/session-liveness.md), so
// most of what is here is a three-client contract; each rule names its id in
// docs/web-console-contracts.md.
//
// Where the page broke its own rule, the rule is kept:
// - every age on the list, and the twelve-hour windows that fold sessions and admit tabs, are
//   measured on the router's clock when the rows carry it (SE-C43);
// - a row is drawn from whatever keys it has: one without a session id or a last activity no
//   longer throws or reads "NaN" (SE-C22), and a row with no `cwd` is grouped under the router's
//   `project` (SE-C23), which the page's `base` never reached;
// - the archived shelf is newest first (SE-C32);
// - a status with no `detail` still says something: its state's word (SE-C63);
// - the router's own `error` event and a refused stream are said as such and never retried as a
//   dropped connection (SE-C59, SE-C60);
// - a permission card just answered is not drawn again while the router catches up (SE-C128);
// - a switcher whose current value is unknown says so (SE-C140);
// - a search hit whose session cannot be looked up because the list could not be read says that,
//   never "no longer listed" (SE-C173);
// - every dot carries its words, a heading's and a tab's included (SE-C20).

import { DASH, factUsd, id8, rowDuration, span } from './format';
import { md, type Block } from './markdown';
import type { FindScope } from './search';

// ---------- what a session is doing ----------

export type Tone = 'busy' | 'attention' | 'warn' | 'error' | 'idle' | 'closed';

/// What either router says about a session, on a list row or in a `status` event: the router
/// derives `activity`, `tone` and `detail` once, for every client (SE-C1). An older router sent
/// only `state`, in one of two vocabularies — the list's working|ask|idle|stalled|failed and the
/// stream's working|tool|ask|idle|failed — and `quiet_ms` on a status.
export interface Said {
  readonly state?: string | null;
  readonly activity?: string | null;
  readonly tone?: string | null;
  readonly detail?: string | null;
  /// Epoch milliseconds on the router's clock, as is `as_of_ms`, the router's now.
  readonly since_ms?: number | null;
  readonly as_of_ms?: number | null;
  readonly ended_ms?: number | null;
  readonly quiet_ms?: number | null;
  /// What a finished turn left running.
  readonly background?: { readonly agents?: number; readonly workflows?: number; readonly shells?: number } | null;
}

export interface WaitingOn {
  readonly kind?: string | null;
  readonly tool?: string | null;
  readonly request_id?: string | null;
  readonly tool_use_id?: string | null;
  readonly summary?: string | null;
}

/// One row of `GET /api/live/sessions`. Every key but the id is optional, because an older router
/// sends only some of them and every key the router added since is optional (SE-C17).
export interface SessionRow extends Said {
  readonly session_id: string;
  readonly cwd?: string | null;
  readonly project?: string | null;
  readonly title?: string | null;
  readonly git_branch?: string | null;
  readonly created_ms?: number | null;
  readonly last_activity_ms?: number | null;
  /// From a current router, busy or waiting on you; from an older one, "its file moved in the
  /// last two minutes".
  readonly active?: boolean | number | null;
  /// Under the wrapper, so it takes a typed message and a Stop.
  readonly wrapped?: boolean | null;
  /// Put away, and quiet ever since: the router decides it, and clears it itself (SE-C33).
  readonly archived?: boolean | null;
  readonly open?: boolean | null;
  readonly ended_how?: string | null;
  readonly mid_turn?: boolean | null;
  readonly waiting_on?: WaitingOn | null;
  /// The machine a hosted router heard it from (docs/hosting-machine-agent.md).
  readonly machine?: { readonly id?: string | null; readonly label?: string | null } | null;
  /// The transcript's size, and where the router learned of the session.
  readonly bytes?: number | null;
  readonly source?: string | null;
}

/// The open view's `status` event.
export interface Status extends Said {
  readonly queued?: number | null;
  readonly open?: boolean | null;
  readonly waiting_on?: WaitingOn | null;
}

const TONES: ReadonlySet<string> = new Set<Tone>(['busy', 'attention', 'warn', 'error', 'idle', 'closed']);
const ACTIVITY_TONE: Readonly<Record<string, Tone>> = {
  generating: 'busy',
  tool: 'busy',
  background: 'busy',
  waiting: 'attention',
  stalled: 'warn',
  failed: 'error',
  idle: 'idle',
  closed: 'closed',
};
const STATE_TONE: Readonly<Record<string, Tone>> = { working: 'busy', tool: 'busy', ask: 'attention', stalled: 'warn', failed: 'error', idle: 'idle' };

/// A table lookup that a word like `constructor` cannot reach through the prototype.
const own = <T>(table: Readonly<Record<string, T>>, key: unknown): T | undefined =>
  typeof key === 'string' && Object.hasOwn(table, key) ? table[key] : undefined;

/// A session's tone: the router's `tone` when it is one of the six; otherwise its `activity`'s;
/// only with no `activity` at all, its legacy `state`'s. A tone the screen does not know, beside a
/// known activity, gives the activity's tone, never gray (SE-C2); an activity it does not know has
/// none — the plain neutral dot (SE-C3).
export function toneOf(x: Said | null | undefined): Tone | null {
  if (!x) return null;
  if (typeof x.tone === 'string' && TONES.has(x.tone)) return x.tone as Tone;
  if (x.activity != null) return own(ACTIVITY_TONE, x.activity) ?? null;
  return own(STATE_TONE, x.state) ?? null;
}

const knows = (x: Said | null | undefined): boolean => !!x && own(ACTIVITY_TONE, x.activity) !== undefined;
const BUSY: ReadonlySet<string> = new Set(['generating', 'tool', 'background', 'stalled']);

/// Busy — generating, tool, background, or stalled, which is still busy — or waiting on you. For a
/// row whose activity the screen does not know, or that has none, the router's `active`. Live is
/// what counts a session, keeps it out of "older", keeps it in the tabs and puts the halo on its dot
/// (SE-C8).
export function isLive(r: SessionRow): boolean {
  return knows(r) ? BUSY.has(r.activity ?? '') || r.activity === 'waiting' : !!r.active;
}

/// Whether Stop means anything: only a running turn — generating, tool or stalled, or from a
/// status with no activity the legacy tool or working. Background work alone, and any activity the
/// screen does not know, is not stoppable (SE-C10).
export function stoppable(s: Said | null | undefined): boolean {
  if (!s) return false;
  return s.activity != null ? ['generating', 'tool', 'stalled'].includes(s.activity) : s.state === 'tool' || s.state === 'working';
}

/// The first four seconds go unsaid: "Thinking", not "Thinking · 2s" (SE-C14).
export const UNSAID_MS = 4000;

/// What it is doing, and for how long: the router's `detail`, then " · " and how long since
/// `since_ms` — "Waiting for you · 1h" — or for a closed session how long ago it ended, "Session
/// ended · 3h ago" (SE-C11, SE-C13). Measured on the router's clock only, `as_of_ms`; without it a
/// row gets no duration, and a status falls back to the router's own `quiet_ms` (SE-C12). No
/// `detail`, no line, so an older router's rows look as they did (SE-C15).
export function doing(x: Said | null | undefined, min: number = UNSAID_MS): string {
  if (!x || !x.detail) return '';
  const now = x.as_of_ms;
  if (x.activity === 'closed') return now != null && x.ended_ms != null ? `${x.detail} · ${span(now - x.ended_ms)} ago` : x.detail;
  const t = now != null && x.since_ms != null ? now - x.since_ms : x.quiet_ms;
  return t != null && t > min ? `${x.detail} · ${span(t)}` : x.detail;
}

/// The header's line for a status: its line as a row's, then " · N queued" (SE-C62). A status with
/// no `detail` shows its state's word, never a blank header (SE-C63).
export function statusLine(s: Status): string {
  const words = doing(s.detail ? s : { ...s, detail: s.state || s.activity || DASH });
  return s.queued ? `${words} · ${s.queued} queued` : words;
}

const ACTIVITY_WORDS: Readonly<Record<string, string>> = {
  generating: 'working',
  tool: 'running a tool',
  background: 'working in the background',
  waiting: 'waiting for you',
  stalled: 'stalled',
  failed: 'failed',
  idle: 'idle',
  closed: 'ended',
};
const STATE_WORDS: Readonly<Record<string, string>> = { working: 'working', tool: 'running a tool', ask: 'waiting for you', stalled: 'stalled', failed: 'failed', idle: 'idle' };
const TONE_WORDS: Readonly<Record<Tone, string>> = { busy: 'working', attention: 'waiting for you', warn: 'stalled', error: 'failed', idle: 'idle', closed: 'ended' };

/// A dot's words, in the phones' words, so no dot carries its meaning by colour alone (SE-C20).
export function spoken(x: Said | null | undefined): string {
  if (!x) return 'idle';
  if (x.activity != null) return own(ACTIVITY_WORDS, x.activity) ?? x.activity;
  return own(STATE_WORDS, x.state) ?? (x.state || 'idle');
}

const RANK: readonly Tone[] = ['attention', 'error', 'warn', 'busy', 'idle'];

/// A project heading's dot: its most pressing session — attention over error over warn over busy
/// over idle — and the hollow ring only when every session in it has ended. An activity the screen
/// does not know outranks nothing and has not ended (SE-C18). From an older router, whose rows name
/// no activity: busy if any row is `active`, else idle (SE-C19).
export function groupTone(items: readonly SessionRow[]): Tone | null {
  if (!items.some((r) => r.activity != null)) return items.some((r) => !!r.active) ? 'busy' : 'idle';
  const tones = new Set(items.map(toneOf));
  for (const t of RANK) if (tones.has(t)) return t;
  return items.length > 0 && items.every((r) => toneOf(r) === 'closed') ? 'closed' : null;
}

// ---------- the list ----------

/// Quiet for longer than this folds a session into "older", and keeps a sibling out of the tabs.
export const OLDER_MS = 12 * 3600 * 1000;

/// How long since a row last moved, measured on the router's clock when the row carries it
/// (SE-C43), and null when the row does not say when it last moved.
export function quietFor(r: SessionRow, now: number): number | null {
  if (typeof r.last_activity_ms !== 'number') return null;
  return (r.as_of_ms ?? now) - r.last_activity_ms;
}

/// Current: live, an older router's working or ask, or active in the last twelve hours (SE-C44).
export function isCurrent(r: SessionRow, now: number): boolean {
  const q = quietFor(r, now);
  return isLive(r) || r.state === 'working' || r.state === 'ask' || (q != null && q <= OLDER_MS);
}

/// The last segment of a path, split on "/" and "\" alike: a hosted router lists Windows machines'
/// sessions, whose `cwd` is `C:\…` (SE-C222).
const lastSegment = (p: string | null | undefined): string => (p ?? '').split(/[\\/]/).filter(Boolean).pop() ?? '';

/// A session's project: the last segment of its `cwd`, else the router's `project` (SE-C23).
export function projectOf(r: Pick<SessionRow, 'cwd' | 'project'>): string {
  return lastSegment(r.cwd) || r.project || DASH;
}

/// The machine a hosted router heard a session from, by its label, trimmed; null when the row names
/// none, as a local router's never does, or names it with a blank label (SE-C220).
export function machineOf(r: Pick<SessionRow, 'machine'>): string | null {
  const label = r.machine?.label;
  return typeof label === 'string' && label.trim() ? label.trim() : null;
}

export interface RowView {
  readonly id: string;
  readonly tone: Tone | null;
  /// Live: the halo, in a list row and a heading only (SE-C9).
  readonly halo: boolean;
  /// The dot's words (SE-C20).
  readonly label: string;
  /// The git branch or the id's first eight characters when the session has a title, the project
  /// otherwise (SE-C26).
  readonly name: string;
  /// "live" takes a typed message and a Stop; "view" is read-only (SE-C28).
  readonly reach: 'live' | 'view';
  readonly age: string;
  readonly title: string | null;
  readonly line: string | null;
  /// Only what needs a reader is coloured in the words themselves (SE-C27).
  readonly lineTone: 'attention' | 'warn' | 'error' | null;
  readonly meta: string;
  readonly archived: boolean;
  readonly row: SessionRow;
}

const ageOf = (q: number | null): string => (q == null ? DASH : span(q));

/// One row of the list. It never names its machine: on a hosted router listing several, the
/// machine's heading over the row does (SE-C219).
export function rowView(r: SessionRow, now: number): RowView {
  const tone = toneOf(r);
  const id = String(r.session_id ?? '');
  const line = doing(r);
  return {
    id,
    tone,
    halo: isLive(r),
    label: spoken(r),
    name: r.title ? r.git_branch || id8(id) : projectOf(r),
    reach: r.wrapped ? 'live' : 'view',
    age: ageOf(quietFor(r, now)),
    title: r.title || null,
    line: line || null,
    lineTone: tone === 'attention' || tone === 'warn' || tone === 'error' ? tone : null,
    meta: [id8(id), r.git_branch].filter(Boolean).join(' · '),
    archived: !!r.archived,
    row: r,
  };
}

export interface GroupView {
  readonly key: string;
  readonly tone: Tone | null;
  readonly halo: boolean;
  readonly label: string;
  readonly count: number;
  /// How long since its newest session moved.
  readonly age: string;
  readonly rows: readonly RowView[];
}

export interface ListView {
  readonly recent: readonly GroupView[];
  readonly older: { readonly label: string; readonly count: number; readonly groups: readonly GroupView[] } | null;
  readonly shelf: { readonly label: string; readonly rows: readonly RowView[] } | null;
}

const newestFirst = (a: SessionRow, b: SessionRow) => (b.last_activity_ms ?? 0) - (a.last_activity_ms ?? 0);

function groupInto(rows: readonly SessionRow[], now: number): GroupView[] {
  const by = new Map<string, SessionRow[]>();
  for (const r of rows) {
    const k = projectOf(r);
    by.set(k, [...(by.get(k) ?? []), r]);
  }
  // Projects alphabetically, so a project keeps its place instead of jumping to the top whenever
  // one of its sessions moves; sessions newest first (SE-C24).
  return [...by.keys()]
    .sort((a, b) => a.localeCompare(b))
    .map((key) => {
      const items = [...(by.get(key) ?? [])].sort(newestFirst);
      const tone = groupTone(items);
      const quiet = items.map((r) => quietFor(r, now)).filter((q): q is number => q != null);
      return {
        key,
        tone,
        halo: items.some(isLive),
        label: tone ? TONE_WORDS[tone] : 'idle',
        count: items.length,
        age: ageOf(quiet.length ? Math.min(...quiet) : null),
        rows: items.map((r) => rowView(r, now)),
      };
    });
}

/// The list as one machine's sessions make it: recent sessions by project; an "older" group of
/// sessions quiet for more than twelve hours, which a live session never joins (SE-C29, SE-C30);
/// and the archived shelf, flat and newest first, holding exactly what the router marks archived
/// (SE-C32, SE-C33). A hosted router's list from several machines is this, a machine at a time
/// (`byMachine`).
export function listView(rows: readonly SessionRow[], now: number): ListView {
  const shelved = rows.filter((r) => r.archived);
  const rest = rows.filter((r) => !r.archived);
  const isOlder = (r: SessionRow) => {
    const q = quietFor(r, now);
    return q != null && q > OLDER_MS && !isLive(r);
  };
  const older = rest.filter(isOlder);
  return {
    recent: groupInto(
      rest.filter((r) => !isOlder(r)),
      now,
    ),
    older: older.length ? { label: `${older.length} older session${older.length > 1 ? 's' : ''}`, count: older.length, groups: groupInto(older, now) } : null,
    shelf: shelved.length ? { label: `${shelved.length} archived`, rows: [...shelved].sort(newestFirst).map((r) => rowView(r, now)) } : null,
  };
}

/// The heading over the rows that name no machine, beside machines that are named: the phones'
/// words (SE-C220).
export const UNNAMED_MACHINE = 'Unnamed machine';

/// One machine's part of the list: its sessions, arranged exactly as a whole list is.
export interface MachineView extends ListView {
  /// Which part it is, for a key and for its own fold: `machine:<label>`, `unnamed`, or empty for
  /// the one part of a list that names no machine.
  readonly key: string;
  /// The machine's label; null for the rows that name none.
  readonly machine: string | null;
  /// The heading drawn over the part: the machine's label, or "Unnamed machine" for the rows that
  /// name none beside machines that are named. Null when the list is not headed per machine at all.
  readonly heading: string | null;
}

/// Machines by label, alphabetically with case aside, so a machine keeps its place as its sessions
/// move; two labels that differ only in case stay two machines, the capitalised one first, as the
/// iPhone orders them (SE-C220).
const machineOrder = (a: string, b: string): number => a.localeCompare(b, undefined, { sensitivity: 'accent' }) || (a < b ? -1 : a > b ? 1 : 0);

/// The list a machine at a time, as both phones' `SessionGroups.byMachine` arrange it. A hosted
/// router that lists sessions from more than one machine heads each machine, alphabetically, with
/// that machine's own projects, older fold and shelf beneath it, so one project on two machines is
/// two projects; rows that name no machine, beside machines that do, come last under a heading of
/// their own (SE-C219, SE-C220). With one machine or none — a local router names none — it is one
/// part with no heading, the whole list exactly as `listView` arranges it.
export function byMachine(rows: readonly SessionRow[], now: number): MachineView[] {
  const labels = [...new Set(rows.map(machineOf).filter((m): m is string => m !== null))];
  if (labels.length <= 1) return [{ key: '', machine: null, heading: null, ...listView(rows, now) }];
  const parts: MachineView[] = labels.sort(machineOrder).map((m) => ({ key: `machine:${m}`, machine: m, heading: m, ...listView(rows.filter((r) => machineOf(r) === m), now) }));
  const unnamed = rows.filter((r) => machineOf(r) === null);
  if (unnamed.length) parts.push({ key: 'unnamed', machine: null, heading: UNNAMED_MACHINE, ...listView(unnamed, now) });
  return parts;
}

/// The header's pip and words. `stale` is the neutral, italic pip of a stream that does not vouch
/// for anything.
export interface Pip {
  readonly tone: Tone | 'stale' | null;
  readonly text: string;
  readonly stop: boolean;
}

/// With no session open, the header counts what is at work and what waits on you, "4 working · 1
/// waiting", never a closed, idle or failed session (SE-C35); with neither, how long since anything
/// moved, "idle · 5m", or "nothing yet" (SE-C36); from an older router, "N active" (SE-C37).
export function listHeader(rows: readonly SessionRow[], now: number): Pip {
  const lastMoved = (): string => {
    if (!rows.length) return 'nothing yet';
    const quiet = rows.map((r) => quietFor(r, now)).filter((q): q is number => q != null);
    return quiet.length ? `idle · ${span(Math.min(...quiet))}` : 'idle';
  };
  if (rows.some((r) => r.activity != null)) {
    const working = rows.filter((r) => BUSY.has(r.activity ?? '')).length;
    const waiting = rows.filter((r) => r.activity === 'waiting').length;
    const said = [working ? `${working} working` : '', waiting ? `${waiting} waiting` : ''].filter(Boolean).join(' · ');
    return { tone: waiting ? 'attention' : working ? 'busy' : 'idle', text: said || lastMoved(), stop: false };
  }
  const live = rows.filter((r) => !!r.active).length;
  return { tone: live ? 'busy' : 'idle', text: live ? `${live} active` : lastMoved(), stop: false };
}

/// What the header says: the open session's stream while one is open, and the list otherwise. A
/// failed list read says so only when no session is open (SE-C41, SE-C42).
export function header(view: { readonly feed: Feed | null; readonly list: { readonly rows: readonly SessionRow[] } | { readonly failed: string } }, now: number): Pip {
  if (view.feed) return feedHeader(view.feed);
  if ('failed' in view.list) return { tone: null, text: `offline · ${view.list.failed || 'no reply'}`, stop: false };
  return listHeader(view.list.rows, now);
}

/// A hosted router lists the sessions of the machines connected to it, and a machine connects by
/// running the installer the router serves without a password, which installs the Agent Router
/// agent and asks there for the router's password. With none connected, the list says so, above
/// the commands that connect one (SE-C187).
export const AGENTLESS = 'No machine has connected yet. Run this on a machine to connect it — it asks for this router’s password.';
/// What else the installer does where VS Code is: it adds the Agent Router extension, which wraps
/// Claude Code there, so the machine's sessions can be typed into and not only followed (SE-C187).
export const WITH_VS_CODE = 'With VS Code’s code command on the machine, the script also adds the Agent Router extension, '
  + 'so its sessions can be typed into from here and from the phone; without it, they can only be followed.';

export interface InstallCommand {
  /// The platforms it runs on, which label it.
  readonly label: string;
  readonly command: string;
}

/// The commands that connect a machine to the router at `origin`, one for each platform: a shell's
/// on macOS and Linux, PowerShell's on Windows (SE-C187).
export const installCommands = (origin: string): readonly InstallCommand[] => [
  { label: 'macOS and Linux', command: `curl -fsSL ${origin}/agent/install.sh | sh` },
  { label: 'Windows (PowerShell)', command: `irm ${origin}/agent/install.ps1 | iex` },
];

/// What a search hit says on a hosted router when its session is on no connected machine (60cf5f8).
export const OFF_MACHINE = 'That session is not on a connected machine.';

/// An empty list says which empty it is (SE-C38).
export const emptyList = (hosted: boolean): string => (hosted ? AGENTLESS : 'No sessions in the last week.');

export interface Shim {
  readonly targets?: readonly { readonly extension: string; readonly path?: string; readonly state: string }[] | null;
}

/// A hosted router wraps nothing, so the wrapper's state is not even asked for (SE-C188).
export const asksWrapper = (hosted: boolean): boolean => !hosted;

/// A local router warns above the list when the newest editor extension is not wrapped (SE-C39).
export function wrapperWarning(shim: Shim | null | undefined, hosted: boolean): string | null {
  if (hosted) return null;
  const targets = shim?.targets ?? [];
  const newest = targets[targets.length - 1];
  if (!newest || newest.state === 'installed') return null;
  return `${newest.extension} is not wrapped — sessions it starts are view-only until the wrapper is installed in Settings → Claude Code.`;
}

// ---------- the tabs ----------

export interface TabView {
  readonly id: string;
  readonly label: string;
  readonly open: boolean;
  readonly tone: Tone | null;
  /// The tab's words, which the page kept in a hover title (SE-C20).
  readonly spoken: string;
}

const createdAt = (r: SessionRow): number => r.created_ms || r.last_activity_ms || 0;

/// The strip over an open session: its project's sessions on its machine — the open one, always,
/// and every sibling that is current and not archived (SE-C44, SE-C45, SE-C48) — oldest first by
/// creation, so a tab keeps its place (SE-C46), and no strip at all for fewer than two (SE-C47).
/// The same project on another machine is another project, as both phones' tabs have it (SE-C221).
/// The open tab says what its stream says while the stream vouches for anything (`vouched`), and
/// its list row otherwise; every other tab is its list row (SE-C51).
export function tabsFor(open: SessionRow, rows: readonly SessionRow[], vouched: Status | null, now: number): TabView[] {
  const listed = rows.some((x) => x.session_id === open.session_id) ? rows : [open, ...rows];
  const project = projectOf(open);
  const machine = machineOf(open);
  const siblings = listed
    .filter((x) => projectOf(x) === project && machineOf(x) === machine)
    .filter((x) => x.session_id === open.session_id || (isCurrent(x, now) && !x.archived))
    .sort((a, b) => createdAt(a) - createdAt(b));
  if (siblings.length < 2) return [];
  return siblings.map((x) => {
    const isOpen = x.session_id === open.session_id;
    const look: Said = (isOpen && vouched) || x;
    return { id: x.session_id, label: x.title || id8(x.session_id), open: isOpen, tone: toneOf(look), spoken: spoken(look) };
  });
}

// ---------- the stream ----------

/// Ten seconds with nothing on the stream — no row and no status — is a connection that died
/// without a word: the router speaks at least once a second whatever the session is doing
/// (SE-C66). The phones give up after the same ten seconds.
export const SILENCE_MS = 10_000;

/// Where the open session's stream stands. `connecting` until its first status, `live` while a
/// status stands, `reconnecting` after an error or a silence — or while a hosted router answers
/// this session with an error status, which is retried (SE-C223, SE-C224) — and `refused` once the
/// router has said it will not stream this session, which is never retried (SE-C59, SE-C60).
export type Phase = 'connecting' | 'live' | 'reconnecting' | 'refused';

/// One feed row as the stream delivers it (src/live.rs).
export interface StreamItem {
  readonly role?: string | null;
  readonly kind?: string | null;
  readonly text?: string | null;
  readonly name?: string | null;
  readonly dur_ms?: number | null;
  readonly sidechain?: boolean | null;
  readonly t?: string | null;
  readonly ask?: Ask | null;
}

/// A row the stream delivered, or something the reader did echoed at once: a message sent, "you ·
/// queued" (SE-C150), or a side question asked, "you · side question" (SE-C160).
export type FeedEntry = { readonly kind: 'row'; readonly item: StreamItem } | { readonly kind: 'echo'; readonly text: string; readonly label: string };

/// What an echoed message is labelled: one sent, and one asked on the side.
export const QUEUED = 'you · queued';
export const SIDE_QUESTION = 'you · side question';

/// The open session's stream and feed. A pure value: each event makes a new one.
export interface Feed {
  /// The connection being listened to. Each new connection gets the next number, and anything an
  /// older one still delivers changes nothing (SE-C56).
  readonly connection: number;
  /// When the connection last said anything, on this machine's clock.
  readonly heard: number;
  /// Whether the next thing the connection says starts a new copy of the feed (SE-C69): true for a
  /// connection that follows another, never for the first one after opening (SE-C70).
  readonly fresh: boolean;
  readonly phase: Phase;
  readonly status: Status | null;
  readonly refusal: string | null;
  /// Why a hosted router answers this session with an error status for now, in its words: its 503
  /// "the machine running this session (<label>) is offline", which comes back later and never
  /// means the session ended (SE-C223); its 404 when no machine holds the session, or its 502 when
  /// the relay failed (SE-C224). Said in the header while the stream is asked for again, and gone
  /// with the first thing a connection delivers.
  readonly unavailable: string | null;
  readonly entries: readonly FeedEntry[];
  /// What the person has already said in this session, newest last: ArrowUp's history (SE-C155).
  readonly sent: readonly string[];
  /// The newest waiting request a status named, so its card is asked for at once (SE-C120).
  readonly cue: string | null;
}

/// A session opened: its first connection, with nothing on screen yet (SE-C194).
export function openFeed(now: number): Feed {
  return { connection: 1, heard: now, fresh: false, phase: 'connecting', status: null, refusal: null, unavailable: null, entries: [], sent: [], cue: null };
}

/// The connection spoke: note when, and start a new copy of the feed if it is a fresh one. A
/// session its hosted router answered with an error status streams again, so nothing stands in
/// the way of it any more (SE-C223). Null when it is not the connection being listened to.
function speaks(feed: Feed, connection: number, now: number): Feed | null {
  if (connection !== feed.connection || feed.phase === 'refused') return null;
  return feed.fresh ? { ...feed, heard: now, fresh: false, unavailable: null, entries: [], sent: [] } : { ...feed, heard: now, unavailable: null };
}

const HISTORY = 200;

/// An unnamed event: a feed row. Rows arrive in the order the stream delivers them and are never
/// re-sorted (SE-C73); one that is not a row is skipped, and the rows after it still land (SE-C72).
/// A row is not a status, and vouches for none (SE-C61).
export function feedMessage(feed: Feed, connection: number, data: string, now: number): Feed {
  const f = speaks(feed, connection, now);
  if (!f) return feed;
  let item: unknown;
  try {
    item = JSON.parse(data);
  } catch {
    return f;
  }
  if (!item || typeof item !== 'object' || Array.isArray(item)) return f;
  const it = item as StreamItem;
  let sent = f.sent;
  // Every message of the person's in the transcript, not a subagent's, a consecutive repeat once,
  // the newest 200 (SE-C155).
  if (it.role === 'user' && it.kind === 'text' && it.text && !it.sidechain && sent[sent.length - 1] !== it.text) {
    sent = [...sent, it.text].slice(-HISTORY);
  }
  return { ...f, entries: [...f.entries, { kind: 'row', item: it }], sent };
}

/// A `status` event: believed, and the header shows it while the connection vouches for it.
export function feedStatus(feed: Feed, connection: number, data: string, now: number): Feed {
  const f = speaks(feed, connection, now);
  if (!f) return feed;
  let s: unknown;
  try {
    s = JSON.parse(data);
  } catch {
    return f;
  }
  if (!s || typeof s !== 'object' || Array.isArray(s)) return f;
  const said = s as Status;
  const rid = said.waiting_on?.request_id;
  return { ...f, phase: 'live', status: said, cue: rid || f.cue };
}

/// The connection errored: the header stops vouching at once, and what the browser's own retry
/// brings first is the transcript's tail again, which replaces the rows rather than repeating them
/// (SE-C64, SE-C65, SE-C69).
export function feedError(feed: Feed, connection: number): Feed {
  if (connection !== feed.connection || feed.phase === 'refused') return feed;
  return { ...feed, fresh: true, phase: 'reconnecting', status: null };
}

/// The router's own `error` event, or a refusal of the caller (401): said as such, and never taken
/// for a dropped connection nor retried (SE-C59, SE-C60).
export function feedRefused(feed: Feed, connection: number, reason: string): Feed {
  if (connection !== feed.connection) return feed;
  return { ...feed, phase: 'refused', status: null, refusal: reason || 'The router will not stream this session.' };
}

/// A hosted router answered this session's stream with an error status, in these words: its 503
/// for a machine that is offline, its 404 for a session no machine holds, its 502 for a relay that
/// failed. Not the session ending, and not the caller refused: the header says the words in place
/// of "reconnecting…", the rows read so far stay, and the stream is asked for again as after a
/// drop (SE-C223, SE-C224). The same words again change nothing.
export function feedUnavailable(feed: Feed, connection: number, words: string): Feed {
  if (connection !== feed.connection || feed.phase === 'refused' || !words) return feed;
  if (feed.phase === 'reconnecting' && feed.status === null && feed.unavailable === words) return feed;
  return { ...feed, fresh: true, phase: 'reconnecting', status: null, unavailable: words };
}

/// The watchdog, run every second while a session is open. Every row and every status starts the
/// ten seconds again, and each connection gets ten seconds of its own from when it opens (SE-C67).
/// A clock set back never reads as a silence that has not begun: silence is measured from no later
/// than now (SE-C68). After ten seconds the silent connection is replaced: `reconnect` says to
/// close it and open connection number `feed.connection` (SE-C66).
export function feedTick(feed: Feed, now: number): { readonly feed: Feed; readonly reconnect: boolean } {
  if (feed.phase === 'refused') return { feed, reconnect: false };
  const heard = Math.min(feed.heard, now);
  if (now - heard < SILENCE_MS) return { feed: heard === feed.heard ? feed : { ...feed, heard }, reconnect: false };
  return { feed: { ...feed, connection: feed.connection + 1, heard: now, fresh: true, phase: 'reconnecting', status: null }, reconnect: true };
}

/// Whether the stream has gone quiet for ten seconds: never with no session open, since then
/// nothing is watched (SE-C55).
export function silent(feed: Feed | null, now: number): boolean {
  return !!feed && feed.phase !== 'refused' && now - Math.min(feed.heard, now) >= SILENCE_MS;
}

/// A message the person sent, echoed at once as "you · queued" until the transcript has it
/// (SE-C150), or a side question, "you · side question" (SE-C160). It is not history: ArrowUp
/// offers what the transcript says was sent.
export function feedEcho(feed: Feed, text: string, label: string = QUEUED): Feed {
  return { ...feed, entries: [...feed.entries, { kind: 'echo', text, label }] };
}

/// A side question's answer, landed as a "side answer" row in Markdown, "(no answer)" when it is
/// empty (SE-C160). Like an echo, a rebuilt feed drops it: the session never saw the question.
export function feedAside(feed: Feed, answer: string | null | undefined): Feed {
  const item: StreamItem = { role: 'assistant', kind: 'aside', text: answer || '(no answer)', sidechain: false };
  return { ...feed, entries: [...feed.entries, { kind: 'row', item }] };
}

/// The status the stream vouches for, or none: connecting, reconnecting, silent or refused.
export const vouched = (feed: Feed | null): Status | null => (feed?.phase === 'live' ? feed.status : null);

/// The header of an open session: "connecting…" until the first status (SE-C61), the status while
/// it stands (SE-C62), "reconnecting…" once the connection errs or falls silent (SE-C64, SE-C66),
/// or, while a hosted router answers the session with an error status, its words — "the machine
/// running this session (mini) is offline" (SE-C223, SE-C224) — and the router's words once it
/// refuses (SE-C59). Stop only while it vouches for a stoppable status (SE-C157).
export function feedHeader(feed: Feed): Pip {
  switch (feed.phase) {
    case 'connecting':
      return { tone: 'stale', text: 'connecting…', stop: false };
    case 'reconnecting':
      return { tone: 'stale', text: feed.unavailable ?? 'reconnecting…', stop: false };
    case 'refused':
      return { tone: 'stale', text: feed.refusal ?? 'The router will not stream this session.', stop: false };
    case 'live': {
      const s = feed.status ?? {};
      return { tone: toneOf(s), text: statusLine(s), stop: stoppable(s) };
    }
  }
}

// ---------- the feed ----------

export type RowKind = 'ask' | 'tool' | 'result' | 'thinking' | 'aside' | 'assistant' | 'person' | 'interrupt' | 'answer' | 'command' | 'other';

export interface FeedRowView {
  readonly kind: RowKind;
  /// The words over the row: a tool's name, "Thought for 2s ›", "side answer", "✓ answered",
  /// "⌘ command", "subagent", a role; empty where nobody speaks.
  readonly who: string;
  /// How long it took, only above 400 ms (SE-C84).
  readonly duration: string | null;
  readonly text: string;
  /// Markdown, for assistant prose and side answers only (SE-C78).
  readonly blocks: readonly Block[] | null;
  /// Only Bash gets the "$" prompt glyph (SE-C79).
  readonly shell: boolean;
  /// A subagent's rows sit on their own rail (SE-C83).
  readonly subagent: boolean;
  readonly ask: Ask | null;
}

/// How a feed row is drawn, by its kind before its role: only a user `text` row is the person's
/// bubble, a tool result is never drawn as their words, and an unknown kind falls back on its role
/// (SE-C76). Total: no row makes it throw.
export function feedRowView(it: StreamItem): FeedRowView {
  const text = typeof it.text === 'string' ? it.text : it.text == null ? '' : String(it.text);
  const subagent = !!it.sidechain;
  const who = subagent ? 'subagent' : String(it.role ?? '');
  const ms = typeof it.dur_ms === 'number' ? it.dur_ms : null;
  const duration = ms != null && ms > 400 ? rowDuration(ms) : null;
  const base = { text, duration, blocks: null, shell: false, subagent, ask: null } as const;
  if (it.kind === 'ask' && it.ask) return { ...base, kind: 'ask', who: '', duration: null, ask: it.ask };
  switch (it.kind) {
    case 'tool_use':
      return { ...base, kind: 'tool', who: String(it.name ?? ''), shell: it.name === 'Bash' };
    case 'tool_result':
      return { ...base, kind: 'result', who: 'OUT' };
    case 'thinking':
      // Collapsed to "Thought for 2s ›", "Thought ›" at 400 ms or less (SE-C82).
      return { ...base, kind: 'thinking', who: `Thought${duration ? ` for ${duration}` : ''} ›`, duration: null };
    case 'aside':
      return { ...base, kind: 'aside', who: 'side answer', blocks: md(text) };
    case 'interrupt':
      // Something that happened, not something anyone said (SE-C77).
      return { ...base, kind: 'interrupt', who: '' };
    case 'answer':
      return { ...base, kind: 'answer', who: '✓ answered' };
    case 'command':
      return { ...base, kind: 'command', who: '⌘ command' };
    case 'text':
      if (it.role === 'assistant') return { ...base, kind: 'assistant', who, blocks: md(text) };
      if (it.role === 'user') return { ...base, kind: 'person', who };
      return { ...base, kind: 'other', who };
    default:
      return { ...base, kind: 'other', who };
  }
}

// ---------- notices and refusals ----------

export interface Notice {
  readonly text: string;
  readonly ok: boolean;
  /// A success clears itself after this long; a warning or a failure stays until dismissed or
  /// replaced (SE-C180).
  readonly ms: number | null;
}

/// A notice, as text, never markup (SE-C177), or none: an empty text clears it. Showing one,
/// clearing it and showing another never recurses (SE-C181).
export function notice(text: string | null | undefined, opts: { readonly ok?: boolean; readonly for?: number } = {}): Notice | null {
  if (!text) return null;
  return { text: String(text), ok: !!opts.ok, ms: opts.for ?? null };
}

const record = (body: unknown): Record<string, unknown> => (body && typeof body === 'object' && !Array.isArray(body) ? (body as Record<string, unknown>) : {});

/// Whether an action worked: a 2xx that does not carry `ok: false` (SE-C184).
export const succeeded = (status: number, body: unknown): boolean => status >= 200 && status < 300 && record(body)['ok'] !== false;

export type Action = 'send' | 'stop' | 'aside' | 'archive' | 'answer' | 'form' | 'model' | 'effort' | 'mode';

/// The sentence for a refused action (SE-C183): the router's own words whenever it sent any — its
/// `reason`, else its `error`, at any status, as the phones say them — else, for a refusal that
/// carried no words, a sentence naming the action.
export function refusal(action: Action, status: number, body: unknown): string {
  const b = record(body);
  for (const key of ['reason', 'error'] as const) {
    const words = b[key];
    if (typeof words === 'string' && words) return words;
  }
  switch (action) {
    case 'send':
      return `Send failed (${status})`;
    case 'stop':
      return 'Could not stop this session.';
    case 'aside':
      return 'The side question failed.';
    case 'archive':
      return 'Could not archive that session.';
    case 'answer':
      return 'Could not answer that prompt.';
    case 'form':
      return 'could not answer directly';
    default:
      return `Could not set the ${action}.`;
  }
}

/// What a send says once the router answered (SE-C150 to SE-C153).
export function sendNotice(status: number, body: unknown, opts: { readonly ask?: boolean } = {}): Notice | null {
  if (!succeeded(status, body)) return notice(refusal('send', status, body));
  if (record(body)['framed']) {
    return notice(
      'Sent, but as a peer message — this session will see it as coming from another Claude, not from you. Install the wrapper in Settings and restart the session for it to arrive as your own message.',
    );
  }
  // A queued user turn does not satisfy a pending question, so "delivered" there would be true
  // and misleading (SE-C152).
  return opts.ask ? notice('Sent as a message — it may not dismiss the prompt.') : notice('Delivered as your message.', { ok: true, for: 4000 });
}

/// What Stop says once the router answered (SE-C158).
export const stopNotice = (status: number, body: unknown): Notice | null =>
  succeeded(status, body) ? notice('Stopped.', { ok: true, for: 4000 }) : notice(refusal('stop', status, body));

/// What a switch says once the router answered: the CLI's own notices, else pending or set
/// (SE-C143), or the refusal (SE-C144).
export function switchNotice(kind: 'model' | 'effort' | 'mode', value: string, status: number, body: unknown): Notice | null {
  if (!succeeded(status, body)) return notice(refusal(kind, status, body));
  const b = record(body);
  const notices = Array.isArray(b['notices']) ? (b['notices'] as unknown[]).filter((x): x is string => typeof x === 'string') : [];
  const text = notices.join(' ') || (b['pending'] ? `${kind} sent — the session has not confirmed yet` : `${kind} set to ${value} — takes effect on the next turn`);
  return notice(text, { ok: true, for: 6000 });
}

/// What a side question says once the router answered (SE-C160).
export function asideNotice(status: number, body: unknown): Notice | null {
  if (!succeeded(status, body)) return notice(refusal('aside', status, body));
  const b = record(body);
  const took = typeof b['took_ms'] === 'number' ? b['took_ms'] : 0;
  const cost = typeof b['cost'] === 'number' ? factUsd(b['cost']) : null;
  return notice(`answered in ${rowDuration(took)}${cost ? ` · ${cost}` : ''}`, { ok: true, for: 6000 });
}

// ---------- switchers ----------

export interface LiveOptions {
  readonly models?: readonly string[] | null;
  readonly efforts?: readonly string[] | null;
  readonly modes?: readonly { readonly value: string; readonly label: string }[] | null;
}

export interface SwitchOption {
  readonly value: string;
  readonly label: string;
  readonly current: boolean;
}

/// The "other…" entry of the model switcher, which asks for a name in a modal (SE-C141).
export const OTHER_MODEL = '__other__';

/// A switcher's options: the current value first, marked ("model: opus") and offered even when the
/// router's list lacks it (SE-C139), or "unknown" when the session has none on record, never its
/// first option as though it were current (SE-C140); the router's own labels for modes (SE-C138).
export function switcherOptions(kind: 'model' | 'effort' | 'mode', current: string | null | undefined, options: LiveOptions): SwitchOption[] {
  const modes = options.modes ?? [];
  const list = kind === 'model' ? (options.models ?? []) : kind === 'effort' ? (options.efforts ?? []) : modes.map((m) => m.value);
  const labelOf = (v: string) => (kind === 'mode' ? modes.find((m) => m.value === v)?.label || v : v);
  const cur = (current ?? '').replace(/^claude-/, '');
  const out: SwitchOption[] = [...new Set([cur, ...list].filter(Boolean))].map((v) => ({ value: v, label: `${v === cur ? `${kind}: ` : ''}${labelOf(v)}`, current: v === cur }));
  if (!cur) out.unshift({ value: '', label: `${kind}: unknown`, current: true });
  if (kind === 'model') out.push({ value: OTHER_MODEL, label: 'other…', current: false });
  return out;
}

// ---------- permission prompts ----------

/// What `GET /api/live/prompt` says a session is blocked on.
export interface Prompt {
  readonly pending?: boolean | null;
  readonly request_id?: string | null;
  readonly tool?: string | null;
  readonly question?: boolean | null;
  readonly display_name?: string | null;
  readonly description?: string | null;
  readonly input?: unknown;
  readonly suggestions?: readonly { readonly index: number; readonly label: string }[] | null;
  /// How many are waiting in all; an older router sends none.
  readonly count?: number | null;
}

export interface Choice {
  readonly label: string;
  readonly body: { readonly allow: boolean; readonly suggestion?: number; readonly message?: string };
  /// What a success notice says (SE-C125).
  readonly done: string;
}

export interface PermissionCard {
  readonly requestId: string;
  readonly what: string;
  readonly more: string | null;
  readonly description: string | null;
  readonly subject: string | null;
  readonly choices: readonly Choice[];
  /// Redrawn when this changes — the prompt, or the count behind it — and not otherwise (SE-C127).
  readonly seen: string;
}

export const DECLINED = 'Declined from the live view.';

/// What a prompt would act on: the command, file path, path, URL or pattern, else its whole input,
/// cut at 400 characters (SE-C121).
export function permissionSubject(input: unknown): string | null {
  if (!input || typeof input !== 'object') return null;
  const i = input as Record<string, unknown>;
  const one = i['command'] || i['file_path'] || i['path'] || i['url'] || i['pattern'];
  const text = one ? String(one) : JSON.stringify(input);
  return text.length > 400 ? `${text.slice(0, 400)}…` : text;
}

/// The permission card, or none: a question is left to the feed's form, and a prompt this screen
/// has answered is not drawn again while the router catches up (SE-C120, SE-C128). Every choice is
/// a button — Allow, each of the prompt's own suggestions, Deny — and no suggestion it did not
/// offer (SE-C123).
export function permissionCard(p: Prompt | null | undefined, answered: ReadonlySet<string> = new Set()): PermissionCard | null {
  if (!p || !p.pending || p.question) return null;
  const requestId = String(p.request_id ?? '');
  if (requestId && answered.has(requestId)) return null;
  const count = typeof p.count === 'number' ? p.count : null;
  return {
    requestId,
    what: `${p.display_name || p.tool || 'A tool'} is waiting for permission`,
    more: count != null && count > 1 ? `${count - 1} more waiting after this one` : null,
    description: p.description || null,
    subject: permissionSubject(p.input),
    choices: [
      { label: 'Allow', body: { allow: true }, done: 'Allowed.' },
      ...(p.suggestions ?? []).map((s) => ({ label: s.label, body: { allow: true, suggestion: s.index }, done: `${s.label}.` })),
      { label: 'Deny', body: { allow: false, message: DECLINED }, done: 'Declined.' },
    ],
    seen: `${requestId}#${count || 1}`,
  };
}

/// The answer a choice sends: it names its prompt (SE-C124), and the session it was drawn in
/// (SE-C129).
export const permissionAnswer = (session: string, card: Pick<PermissionCard, 'requestId'>, choice: Choice) => ({
  session,
  ...choice.body,
  request_id: card.requestId,
});

/// What became of an answer: done, and the card goes with a success notice; gone (409), and the
/// card is replaced at once by whatever waits now (SE-C126); or refused, in the router's words,
/// with the buttons given back (SE-C125).
export function permissionOutcome(status: number, body: unknown, choice: Choice): { readonly kind: 'done' | 'gone' | 'refused'; readonly notice: Notice | null } {
  if (succeeded(status, body)) return { kind: 'done', notice: notice(choice.done, { ok: true, for: 5000 }) };
  return { kind: status === 409 ? 'gone' : 'refused', notice: notice(refusal('answer', status, body)) };
}

// ---------- question forms ----------

export interface AskOption {
  readonly label: string;
  readonly description?: string | null;
}

export interface AskQuestion {
  readonly question: string;
  readonly header?: string | null;
  readonly multiSelect?: boolean | null;
  readonly options?: readonly AskOption[] | null;
}

/// An AskUserQuestion, as the transcript carries it.
export interface Ask {
  readonly questions?: readonly AskQuestion[] | null;
}

/// What is picked in one question: labels, and the Other field when its box is ticked.
export interface Selection {
  readonly chosen: readonly string[];
  readonly other?: string | null;
}

/// One tab per question, named by its header or else "Question N" (SE-C105).
export const questionTabs = (ask: Ask): string[] => (ask.questions ?? []).map((q, i) => q.header || `Question ${i + 1}`);

/// A question's answer: its chosen labels in the tool's option order, then the typed Other text,
/// trimmed, joined by ", ". Other with nothing typed adds nothing; typed Other alone is an answer
/// (SE-C109). Labels round-trip exactly (SE-C110).
export function answerOf(q: AskQuestion, sel: Selection | null | undefined): string {
  if (!sel) return '';
  const picked = (q.options ?? []).map((o) => o.label).filter((l) => sel.chosen.includes(l));
  const other = (sel.other ?? '').trim();
  return [...picked, ...(other ? [other] : [])].join(', ');
}

/// Every answered question: by its text for the router, and as the message a person would have
/// typed, "<header, else question>: <answer>" a line each. A question with nothing chosen is left
/// out (SE-C109).
export function composeAnswers(ask: Ask, selections: readonly (Selection | null | undefined)[]): { readonly answers: Record<string, string>; readonly lines: string[] } {
  const answers: Record<string, string> = {};
  const lines: string[] = [];
  (ask.questions ?? []).forEach((q, i) => {
    const a = answerOf(q, selections[i]);
    if (!a) return;
    answers[q.question] = a;
    lines.push(`${q.header || q.question}: ${a}`);
  });
  return { answers, lines };
}

export const PICK_FIRST = 'Pick at least one answer first.';
export const PERMISSION_FIRST = 'Not sent: a permission prompt is waiting first — answer it, then this.';
export const DIFFERENT_QUESTION = 'Not sent: a different question is waiting now.';

/// Whether a live prompt is the question a form was drawn for: the same questions, in the same
/// order. A prompt that does not list its questions, or lists them in a shape that is not a list,
/// is taken at its word (SE-C113).
export function sameQuestions(p: Prompt, ask: Ask): boolean {
  const input = p.input && typeof p.input === 'object' ? (p.input as Record<string, unknown>) : null;
  const theirs = input && Array.isArray(input['questions']) ? (input['questions'] as unknown[]) : null;
  if (!theirs) return true;
  const texts = (qs: readonly unknown[]) => JSON.stringify(qs.map((q) => (q && typeof q === 'object' ? (q as { question?: unknown }).question : undefined)));
  return texts(theirs) === texts(ask.questions ?? []);
}

export type AnswerTarget =
  | { readonly kind: 'refused'; readonly text: string }
  | { readonly kind: 'named'; readonly requestId: string }
  | { readonly kind: 'unnamed' };

/// Which prompt a form's answers go to, asked of what waits now. A permission request waiting
/// takes nothing, not even a message (SE-C112); nor does a different question (SE-C113). This
/// form's question takes its answers by `request_id`; with nothing to name, they go unnamed, byte
/// for byte what older clients send (SE-C114).
export function answerTarget(live: Prompt | null | undefined, ask: Ask): AnswerTarget {
  if (live?.pending && !live.question) return { kind: 'refused', text: PERMISSION_FIRST };
  if (live?.pending && !sameQuestions(live, ask)) return { kind: 'refused', text: DIFFERENT_QUESTION };
  if (live?.pending && live.request_id) return { kind: 'named', requestId: live.request_id };
  return { kind: 'unnamed' };
}

/// The body of `POST /api/live/answer` for a form, for the session it was drawn in (SE-C119).
export function answerBody(session: string, answers: Record<string, string>, target: AnswerTarget): { session: string; answers: Record<string, string>; request_id?: string } {
  return target.kind === 'named' ? { session, answers, request_id: target.requestId } : { session, answers };
}

/// What a form says once the router answered: spent, "answered", only once the router answered the
/// prompt itself (SE-C115); otherwise why, and the answers go as an ordinary message (SE-C116).
export function formOutcome(status: number, body: unknown): { readonly spent: boolean; readonly hint: string; readonly sendAsMessage: boolean } {
  if (succeeded(status, body)) return { spent: true, hint: 'answered', sendAsMessage: false };
  return { spent: false, hint: refusal('form', status, body), sendAsMessage: true };
}

// ---------- where the reader is ----------

/// A hit's session opened from Find: looked up in the router's list. A session no longer listed
/// says so; a hosted router says why it cannot open it; a list that could not be read says that,
/// never "no longer listed" (SE-C173).
export function hitTarget(
  id: string,
  list: { readonly rows: readonly SessionRow[] } | { readonly failed: string },
  hosted: boolean,
): { readonly row: SessionRow } | { readonly say: string } {
  if ('failed' in list) return { say: `Could not read the session list — ${list.failed || 'no reply'}` };
  const row = list.rows.find((r) => r.session_id === id);
  if (row) return { row };
  return { say: hosted ? OFF_MACHINE : 'That session is no longer listed.' };
}

/// The live view's address with a session open: `?session=<id>` (SE-C189).
export function withSession(href: string, id: string): string {
  const u = new URL(href);
  u.searchParams.set('session', id);
  return u.pathname + u.search + u.hash;
}

/// The live view's address on the list: no session (SE-C189).
export function withoutSession(href: string): string {
  const u = new URL(href);
  u.searchParams.delete('session');
  return u.pathname + u.search + u.hash;
}

/// The session an address names, if any (SE-C190).
export const sessionIn = (href: string): string | null => new URL(href).searchParams.get('session') || null;

/// What the live view shows: the list or one session, and Find over either.
export interface LiveView {
  readonly session: string | null;
  readonly find: { readonly scope: FindScope; readonly query: string } | null;
}

/// Find opened: scoped to "This session" from a session, "Everywhere" from the list (SE-C162).
export function openFind(view: LiveView): LiveView {
  return { ...view, find: { scope: view.session ? 'session' : 'all', query: view.find?.query ?? '' } };
}

/// Find closed: back to where it was, the session still connected, since Find never touches the
/// stream (SE-C174).
export const closeFind = (view: LiveView): LiveView => ({ ...view, find: null });

/// Whether the tab strip shows: with a session open, Find closed, and at least two tabs (SE-C47,
/// SE-C53).
export const stripShown = (view: LiveView, tabs: readonly TabView[]): boolean => !!view.session && !view.find && tabs.length >= 2;

/// The composer belongs to an open session: not on the list, not in Find (SE-C146).
export const composerShown = (view: LiveView): boolean => !!view.session && !view.find;

/// The heading: the screen's name on the list, the project with a session open, "Find" in Find
/// (SE-C196).
export function headingOf(view: LiveView, open: Pick<SessionRow, 'cwd' | 'project'> | null): string {
  if (view.find) return 'Find';
  return view.session && open ? projectOf(open) : 'Live sessions';
}
