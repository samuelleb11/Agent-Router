// New for agent-router: the Library screen's model, with no DOM and no React (docs/web-console-contracts.md,
// "Library"; docs/library.md). The items as the screen lists them, a version's provenance and state in
// words, what can be done to a version, a folder packed into the tar the router reads (src/library/bundle.rs),
// a line diff for comparing versions, and a machine's answer to "sync now". Where a machine's Library
// stands is ~/model/machine's, which the first page load carries and this module is kept out of.

export type Kind = 'skill' | 'command' | 'subagent';
export const KINDS: readonly Kind[] = ['skill', 'command', 'subagent'];

export type State = 'live' | 'pending' | 'rejected' | 'superseded';

/// Where a version came from (src/library/store.rs, `Provenance`), its origin under `from`.
export type Provenance =
  | { readonly from: 'upload' }
  | { readonly from: 'forge'; readonly source: string; readonly forge: string; readonly repository: string; readonly commit: string; readonly path: string }
  | { readonly from: 'token'; readonly token: string }
  | { readonly from: 'machine'; readonly machine: string; readonly label?: string; readonly session?: string }
  | { readonly from: 'router'; readonly build: string };

export interface Brief {
  readonly number: number;
  readonly digest: string;
  readonly size: number;
  readonly state: State;
  readonly provenance: Provenance;
  readonly created_at: number;
}

export interface Version extends Brief {
  readonly kind: Kind;
  readonly name: string;
  readonly description: string;
  readonly decided_at: number | null;
}

/// One item as `GET /api/library` lists it.
export interface ItemRow {
  readonly kind: Kind;
  readonly name: string;
  readonly description: string;
  /// `null` for uploads and publishes, `source:<key>`, or `router`.
  readonly source: string | null;
  readonly created_at: number;
  readonly archived_at: number | null;
  readonly live: Brief | null;
  readonly newest: Brief | null;
  readonly pending: number;
  readonly versions: number;
  /// Uses in the last 30 days, and the last use ever; absent from a router that counts none
  /// (LB-C15, LB-C17).
  readonly uses_30d?: number;
  readonly last_used_at?: number | null;
}

export interface FileRow {
  readonly path: string;
  readonly size: number;
  readonly executable: boolean;
}

/// One item as `GET /api/library/items/{kind}/{name}` gives it.
export interface ItemDetail {
  readonly kind: Kind;
  readonly name: string;
  readonly description: string;
  readonly source: string | null;
  readonly archived_at: number | null;
  readonly versions: readonly Version[];
  readonly files: readonly FileRow[];
}

// ---------- the words ----------

export const TITLE = 'Library · Agent Router';

/// A local router keeps no Library: it is a hosted router's, for the machines connected to it
/// (LB-C1).
export const NOT_HERE = 'The Library is a hosted router’s: it keeps the skills, commands and subagents of the machines connected to it. This router runs on this machine, so it keeps none.';

/// An empty Library, which says what to do about it (LB-C3).
export const EMPTY = 'The Library is empty. Add a skill, a command or a subagent above, and every connected machine gets it.';

export const KIND_WORDS: Readonly<Record<Kind, string>> = { skill: 'Skill', command: 'Command', subagent: 'Subagent' };

const STATE_WORDS: Readonly<Record<State, string>> = {
  live: 'live',
  pending: 'waits for approval',
  rejected: 'rejected',
  superseded: 'superseded',
};

export const stateWords = (state: State): string => STATE_WORDS[state] ?? state;

/// Where an item takes its versions from (LB-C2).
export function sourceWords(source: string | null): string {
  if (source === 'router') return 'comes with the router’s build';
  if (source?.startsWith('source:')) return `imported by the source ${source.slice('source:'.length)}`;
  return 'uploaded and published here';
}

/// Where a version came from, in words (LB-C5).
export function provenanceWords(p: Provenance): string {
  switch (p.from) {
    case 'upload':
      return 'uploaded from the console';
    case 'forge':
      return `imported from ${p.repository} at ${p.commit.slice(0, 8)}${p.path ? `, ${p.path}` : ''}`;
    case 'token':
      return `published with the token ${p.token}`;
    case 'machine':
      return `published from ${p.label || p.machine.slice(0, 8)}${p.session ? ' by a Claude Code session' : ''}`;
    case 'router':
      return 'built into the router';
    default:
      return 'from somewhere the console does not know';
  }
}

/// The session a version was published by, which the console can open (LB-C5).
export const sessionOf = (p: Provenance): string | null => (p.from === 'machine' && p.session ? p.session : null);

/// The live version's words in the list: "v3 live", or that none is (LB-C2).
export const liveWords = (row: Pick<ItemRow, 'live'>): string => (row.live ? `v${row.live.number} live` : 'none live');

/// The file Claude Code reads of an item, which the screen shows first (LB-C4).
export const mainFile = (kind: Kind, called: string): string => (kind === 'skill' ? 'SKILL.md' : `${called}.md`);

/// What can be done to a version: approve or reject one that waits, make an older one live again
/// (a rollback). A live one and a rejected one take nothing (LB-C6).
export type Act = 'approve' | 'reject' | 'restore';
export function actsOn(v: Pick<Version, 'state'>): Act[] {
  if (v.state === 'pending') return ['approve', 'reject'];
  if (v.state === 'superseded') return ['restore'];
  return [];
}

export const ACT_WORDS: Readonly<Record<Act, string>> = { approve: 'Approve', reject: 'Reject', restore: 'Make live again' };

/// Items as the screen lists them: by kind, then name (LB-C3).
export const itemOrder = (a: Pick<ItemRow, 'kind' | 'name'>, b: Pick<ItemRow, 'kind' | 'name'>): number =>
  KINDS.indexOf(a.kind) - KINDS.indexOf(b.kind) || a.name.localeCompare(b.name);

/// `?item=skill/bench` read back, or null.
export function itemParam(value: string | null): { kind: Kind; name: string } | null {
  const [kind, called, ...rest] = (value ?? '').split('/');
  return KINDS.includes(kind as Kind) && called && rest.length === 0 ? { kind: kind as Kind, name: called } : null;
}

// ---------- uploading ----------

/// A name the Library takes: lowercase letters and digits, single hyphens between (src/library.rs).
const NAME = /^[a-z0-9]+(-[a-z0-9]+)*$/;
export const isName = (called: string): boolean => NAME.test(called) && called.length <= 64;

/// The name an upload is offered under: a folder's own name, a file's without what it is.
export function nameFrom(fileOrFolder: string): string {
  const last = fileOrFolder.split('/').filter(Boolean).pop() ?? '';
  return last.replace(/\.(tar\.gz|tgz|tar|zip|md)$/i, '').toLowerCase();
}

/// The `Content-Type` a chosen file is sent with, by what it is; the router reads it by its
/// first bytes anyway (src/library/bundle.rs, `from_upload`).
export function typeOf(fileName: string): string {
  const lower = fileName.toLowerCase();
  if (lower.endsWith('.zip')) return 'application/zip';
  if (lower.endsWith('.tar.gz') || lower.endsWith('.tgz')) return 'application/gzip';
  if (lower.endsWith('.tar')) return 'application/x-tar';
  return 'text/markdown';
}

export interface Packed {
  readonly path: string;
  readonly bytes: Uint8Array;
}

/// What a machine's own clutter is, never part of an item: the router leaves the same out of a
/// directory it packs (src/library/bundle.rs, `LEFT_OUT`).
const LEFT_OUT = new Set(['.git', '.DS_Store', '__pycache__', '.venv', 'venv', 'node_modules']);
export const leftOut = (path: string): boolean => path.split('/').some((part) => LEFT_OUT.has(part) || part.startsWith('._'));

/// A chosen folder's files as one ustar archive, which the router reads as it reads `tar`'s: each
/// file's path relative to the folder, the folder's own name dropped, clutter left out (LB-C9).
export function packFolder(files: readonly Packed[]): Uint8Array {
  const kept = files
    .map((f) => ({ path: f.path.split('/').slice(1).join('/'), bytes: f.bytes }))
    .filter((f) => f.path !== '' && !leftOut(f.path))
    .sort((a, b) => (a.path < b.path ? -1 : a.path > b.path ? 1 : 0));
  const blocks: Uint8Array[] = [];
  const text = new TextEncoder();
  for (const f of kept) {
    const header = new Uint8Array(512);
    const encoded = text.encode(f.path);
    const [prefix, rest] = encoded.length <= 100 ? [new Uint8Array(0), encoded] : splitPath(f.path, text);
    header.set(rest.subarray(0, 100), 0);
    octal(header, 100, 8, 0o644);
    octal(header, 108, 8, 0);
    octal(header, 116, 8, 0);
    octal(header, 124, 12, f.bytes.length);
    octal(header, 136, 12, 0);
    header[156] = 0x30; // '0': a file
    header.set(text.encode('ustar\0'), 257);
    header.set(text.encode('00'), 263);
    header.set(prefix.subarray(0, 155), 345);
    header.fill(0x20, 148, 156);
    const sum = header.reduce((acc, b) => acc + b, 0);
    header.set(text.encode(`${sum.toString(8).padStart(6, '0')}\0 `), 148);
    blocks.push(header, f.bytes, new Uint8Array((512 - (f.bytes.length % 512)) % 512));
  }
  blocks.push(new Uint8Array(1024));
  const out = new Uint8Array(blocks.reduce((n, b) => n + b.length, 0));
  let at = 0;
  for (const b of blocks) {
    out.set(b, at);
    at += b.length;
  }
  return out;
}

function splitPath(path: string, text: TextEncoder): [Uint8Array, Uint8Array] {
  const cut = path.lastIndexOf('/', path.length - 1);
  return cut > 0 ? [text.encode(path.slice(0, cut)), text.encode(path.slice(cut + 1))] : [new Uint8Array(0), text.encode(path)];
}

function octal(header: Uint8Array, at: number, width: number, value: number) {
  const digits = value.toString(8).padStart(width - 1, '0');
  for (let i = 0; i < digits.length; i++) header[at + i] = digits.charCodeAt(i);
  header[at + width - 1] = 0;
}

/// What an upload came to, in words (LB-C9).
export function publishedWords(kind: Kind, called: string, unchanged: boolean, v: Pick<Version, 'number' | 'state'>): string {
  const what = `the ${kind} ${called}`;
  if (unchanged) return `Nothing new: ${what} is version ${v.number} already, and it is ${stateWords(v.state)}.`;
  return v.state === 'live'
    ? `Published ${what} as version ${v.number}: live, and every connected machine is told now.`
    : `Published ${what} as version ${v.number}: it ${stateWords(v.state)}.`;
}

// ---------- comparing ----------

export interface DiffLine {
  readonly kind: 'same' | 'added' | 'removed';
  readonly text: string;
}

/// `after` against `before`, line by line: what one has and the other has not, marked (LB-C7). A
/// longest-common-subsequence diff, which a skill's few hundred lines afford.
export function diffLines(before: string, after: string): DiffLine[] {
  const a = before.split('\n');
  const b = after.split('\n');
  const lcs: number[][] = Array.from({ length: a.length + 1 }, () => new Array<number>(b.length + 1).fill(0));
  for (let i = a.length - 1; i >= 0; i--) {
    for (let j = b.length - 1; j >= 0; j--) {
      lcs[i]![j] = a[i] === b[j] ? lcs[i + 1]![j + 1]! + 1 : Math.max(lcs[i + 1]![j]!, lcs[i]![j + 1]!);
    }
  }
  const out: DiffLine[] = [];
  let i = 0;
  let j = 0;
  while (i < a.length && j < b.length) {
    if (a[i] === b[j]) {
      out.push({ kind: 'same', text: a[i]! });
      i++;
      j++;
    } else if (lcs[i + 1]![j]! >= lcs[i]![j + 1]!) {
      out.push({ kind: 'removed', text: a[i++]! });
    } else {
      out.push({ kind: 'added', text: b[j++]! });
    }
  }
  while (i < a.length) out.push({ kind: 'removed', text: a[i++]! });
  while (j < b.length) out.push({ kind: 'added', text: b[j++]! });
  return out;
}

// ---------- the machines ----------

/// What one machine said when told to sync (src/library/push.rs, `Told`).
export interface Told {
  readonly machine: string;
  readonly label: string;
  readonly outcome: 'synced' | 'predates' | 'opted_out' | 'offline' | 'failed';
  readonly report?: { readonly items: readonly { readonly outcome: string }[] };
  readonly why?: string;
}

/// A machine's answer to "sync now", in words (LB-C10, LB-C12).
export function toldWords(t: Told): string {
  switch (t.outcome) {
    case 'synced': {
      const changed = (t.report?.items ?? []).filter((l) => l.outcome !== 'in_step').length;
      return changed ? `${t.label}: synced, ${changed} changed` : `${t.label}: synced, already in step`;
    }
    case 'predates':
      return `${t.label}: its agent predates the Library — run the installer on it again`;
    case 'opted_out':
      return `${t.label}: takes no part in the Library`;
    case 'offline':
      return `${t.label}: offline — it syncs when it connects again`;
    default:
      return `${t.label}: failed${t.why ? ` — ${t.why}` : ''}`;
  }
}

/// A Markdown file without its frontmatter, which the screen shows apart: the item's description
/// heads it (LB-C4).
export function bodyOf(text: string): string {
  const lines = text.replace(/^\uFEFF/, '').split('\n');
  if (lines[0]?.trimEnd() !== '---') return text;
  const end = lines.findIndex((line, i) => i > 0 && line.trimEnd() === '---');
  return end < 0 ? text : lines.slice(end + 1).join('\n').replace(/^\n+/, '');
}

// ---------- usage ----------

/// One machine's share of an item's uses: `id` is null for sessions no machine has listed.
export interface ByMachine {
  readonly id: string | null;
  readonly label: string | null;
  readonly uses: number;
}

/// One project's share: the session's working directory, null while it is not known.
export interface ByProject {
  readonly project: string | null;
  readonly uses: number;
}

/// One item's use as `GET /api/library/usage` tells it (src/library/usage.rs, `ItemUsage`).
export interface ItemUsage {
  readonly kind: Kind;
  readonly name: string;
  readonly uses_30d: number;
  readonly last_used_at: number | null;
  /// A count per day, oldest first, one for each of the report's `days`.
  readonly daily: readonly number[];
  readonly machines: readonly ByMachine[];
  readonly projects: readonly ByProject[];
}

/// What one Library skill's entry in the skill listing adds to every turn.
export interface Entry {
  readonly name: string;
  readonly chars: number;
  readonly tokens: number;
}

/// The Library's share of the newest skill listing the router carried.
export interface Listing {
  readonly at: number;
  readonly chars: number;
  readonly tokens: number;
  readonly items: readonly Entry[];
}

export interface Usage {
  /// Where each day begins, in the viewer's days, oldest first.
  readonly days: readonly number[];
  readonly listing: Listing | null;
  readonly items: readonly ItemUsage[];
}

/// How often an item was used, in the list (LB-C15).
export function usedWords(uses: number): string {
  if (uses <= 0) return 'not used in 30 days';
  return uses === 1 ? 'used once in 30 days' : `used ${uses.toLocaleString()} times in 30 days`;
}

/// What the Library's skills add to every turn, or that it is not measured yet (LB-C14).
export function listingWords(listing: Listing | null): string {
  if (listing === null) return 'What the Library’s skills add to every turn is not measured yet: no turn the router has carried lists one of them.';
  return `The Library’s skills add about ${listing.tokens.toLocaleString()} tokens to every turn.`;
}

/// Where a share of uses ran, in words (LB-C16).
export const MACHINE_NOT_KNOWN = 'a machine not known';
export const machineWords = (m: ByMachine): string => m.label ?? m.id ?? MACHINE_NOT_KNOWN;
