// @vitest-environment node
// New for agent-router: the fixtures the web console and both phones share, fixtures/live/, checked
// against the web's own model. The web is the reference for both phones (docs/session-liveness.md,
// "Client presentation policy"; decision F in docs/web-console-carbon.md), but where the three
// clients were decided in the phones' favour — numbered lists, and a hosted router's sessions headed
// per machine, both on 2026-10-05 — the web's model follows the phones. Each phone reads the same
// two files — markdown-blocks.json and session-groups.json — through an adapter of its own:
// ios/AgentRouter/Tests/SharedFixtureTests.swift and
// android/app/src/test/java/ca/whnet/agentrouter/SharedFixtureTest.kt.
//
// `expected` is never written by hand: this spec writes it from the model, keeping every case's
// name, input and `pending`, so the web cannot drift from its own fixtures. After adding or editing
// a case, from ui/:
//   UPDATE_SHARED_FIXTURES=1 pnpm exec vitest run src/model/shared-fixtures.spec.ts
// A `pending` case is one the clients disagree on, where a phone would lose information by matching
// the web: every suite skips it, by name, until it is decided.
import { readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import { md, type Block, type Inline } from './markdown';
import { byMachine, type GroupView, type ListView, type SessionRow } from './session';

const DIR = join(__dirname, '..', '..', '..', 'fixtures', 'live');
const MARKDOWN = join(DIR, 'markdown-blocks.json');
const GROUPS = join(DIR, 'session-groups.json');
const UPDATE = process.env['UPDATE_SHARED_FIXTURES'] === '1';

// ---------- the canonical shapes (each file's `about` says the same) ----------

/// Inline text with its styles, flattened: a style is set only when it applies.
interface Run {
  text: string;
  bold?: true;
  italic?: true;
  code?: true;
  href?: string;
}

type Canonical =
  | { type: 'paragraph'; runs: Run[] }
  | { type: 'heading'; level: number; runs: Run[] }
  | { type: 'list'; ordered: boolean; items: Run[][] }
  | { type: 'quote'; runs: Run[] }
  | { type: 'code'; lang: string | null; text: string }
  | { type: 'table'; head: Run[][]; rows: Run[][][] };

interface Group {
  project: string;
  tone: string | null;
  halo: boolean;
  sessions: string[];
}

/// One list as one machine's sessions make it: recent projects, the older fold and the shelf.
interface Part {
  recent: Group[];
  older: Group[];
  archived: string[];
}

/// One machine's part of a list headed per machine: its label, null for the rows that name none.
interface MachinePart extends Part {
  machine: string | null;
}

/// A list with no machine heading is one part; a hosted router's list from several machines is a
/// part per machine, in the order they are headed.
type Groups = Part | { machines: MachinePart[] };

interface Case<I, E> {
  name: string;
  pending?: string;
  expected?: E;
  input?: I;
}

interface MarkdownCase extends Case<string, Canonical[]> {
  input: string;
}

interface GroupsCase extends Case<never, Groups> {
  now: number;
  sessions: SessionRow[];
}

interface Fixture<C> {
  about: string[];
  cases: C[];
}

type Style = Omit<Run, 'text'>;

/// The model's inline tree as runs: adjacent runs with the same styles merged, empty ones left out.
function runs(xs: readonly Inline[]): Run[] {
  const out: Run[] = [];
  const add = (text: string, s: Style) => {
    if (!text) return;
    const last = out.at(-1);
    if (last && last.bold === s.bold && last.italic === s.italic && last.code === s.code && last.href === s.href) {
      last.text += text;
      return;
    }
    const run: Run = { text };
    if (s.bold) run.bold = true;
    if (s.italic) run.italic = true;
    if (s.code) run.code = true;
    if (s.href !== undefined) run.href = s.href;
    out.push(run);
  };
  const walk = (ys: readonly Inline[], s: Style) => {
    for (const y of ys) {
      switch (y.type) {
        case 'text':
          add(y.text, s);
          break;
        case 'code':
          add(y.text, { ...s, code: true });
          break;
        case 'bold':
          walk(y.children, { ...s, bold: true });
          break;
        case 'italic':
          walk(y.children, { ...s, italic: true });
          break;
        case 'link':
          walk(y.children, { ...s, href: y.href });
          break;
      }
    }
  };
  walk(xs, {});
  return out;
}

/// The model's blocks in the canonical shape. A numbered list is ordered, counting from 1 whatever
/// its source's numbers (SE-C99), and a fence's language tag is dropped (SE-C93).
function blocks(src: string): Canonical[] {
  return md(src).map((b: Block): Canonical => {
    switch (b.type) {
      case 'paragraph':
        return { type: 'paragraph', runs: runs(b.children) };
      case 'heading':
        return { type: 'heading', level: b.level, runs: runs(b.children) };
      case 'list':
        return { type: 'list', ordered: b.ordered, items: b.items.map(runs) };
      case 'quote':
        return { type: 'quote', runs: runs(b.children) };
      case 'code':
        return { type: 'code', lang: null, text: b.text };
      case 'table':
        return { type: 'table', head: b.head.map(runs), rows: b.rows.map((row) => row.map(runs)) };
    }
  });
}

/// The list as the web groups it: recent projects, the older fold and the archived shelf, a machine
/// at a time when a hosted router lists sessions from more than one (SE-C219, SE-C220).
function groups(rows: readonly SessionRow[], now: number): Groups {
  const group = (g: GroupView): Group => ({ project: g.key, tone: g.tone, halo: g.halo, sessions: g.rows.map((r) => r.id) });
  const part = (view: ListView): Part => ({
    recent: view.recent.map(group),
    older: view.older?.groups.map(group) ?? [],
    archived: view.shelf?.rows.map((r) => r.id) ?? [],
  });
  const machines = byMachine(rows, now);
  const [only] = machines;
  if (machines.length === 1 && only && only.heading === null) return part(only);
  return { machines: machines.map((m) => ({ machine: m.machine, ...part(m) })) };
}

// ---------- writing the files ----------

/// Whitespace that cannot be seen, control characters, and the line and paragraph separators.
const unseen = (c: number): boolean =>
  c < 0x20 || (c >= 0x7f && c <= 0xa0) || c === 0x1680 || (c >= 0x2000 && c <= 0x200f) || (c >= 0x2028 && c <= 0x202f) || c === 0x205f || c === 0x3000 || c === 0xfeff;

/// One value on one line, as JSON with a space after each colon and comma. Whitespace that cannot
/// be seen, and the line and paragraph separators, are escaped so a case's input reads as typed.
function one(x: unknown): string {
  if (Array.isArray(x)) return `[${x.map(one).join(', ')}]`;
  if (x && typeof x === 'object')
    return `{${Object.entries(x)
      .filter(([, v]) => v !== undefined)
      .map(([k, v]) => `${JSON.stringify(k)}: ${one(v)}`)
      .join(', ')}}`;
  return JSON.stringify(x).replace(/[^ -~]/g, (c) => (unseen(c.charCodeAt(0)) ? `\\u${c.charCodeAt(0).toString(16).padStart(4, '0')}` : c));
}

const list = (xs: readonly unknown[], indent: string): string => (xs.length ? `[\n${xs.map((x) => `${indent}  ${one(x)}`).join(',\n')}\n${indent}]` : '[]');

function file(about: readonly string[], cases: readonly string[]): string {
  return `{\n  "about": ${list(about, '  ')},\n  "cases": [\n${cases.join(',\n')}\n  ]\n}\n`;
}

function caseOf(fields: readonly (readonly [string, string] | null)[]): string {
  return `    {\n${fields
    .filter((f): f is readonly [string, string] => f !== null)
    .map(([k, v]) => `      ${JSON.stringify(k)}: ${v}`)
    .join(',\n')}\n    }`;
}

const pendingOf = (c: Case<unknown, unknown>) => (c.pending ? (['pending', one(c.pending)] as const) : null);

function writeMarkdown(f: Fixture<MarkdownCase>): void {
  const cases = f.cases.map((c) => caseOf([['name', one(c.name)], ['input', one(c.input)], pendingOf(c), ['expected', list(blocks(c.input), '      ')]]));
  writeFileSync(MARKDOWN, file(f.about, cases));
}

/// One part, its keys at `indent` and its groups a line each.
const partOf = (p: Part, indent: string): string =>
  `"recent": ${list(p.recent, indent)},\n${indent}"older": ${list(p.older, indent)},\n${indent}"archived": ${one(p.archived)}`;

function writeGroups(f: Fixture<GroupsCase>): void {
  const cases = f.cases.map((c) => {
    const g = groups(c.sessions, c.now);
    const expected =
      'machines' in g
        ? `{\n        "machines": [\n${g.machines
            .map((m) => `          {\n            "machine": ${one(m.machine)},\n            ${partOf(m, '            ')}\n          }`)
            .join(',\n')}\n        ]\n      }`
        : `{\n        ${partOf(g, '        ')}\n      }`;
    return caseOf([['name', one(c.name)], ['now', one(c.now)], ['sessions', list(c.sessions, '      ')], pendingOf(c), ['expected', expected]]);
  });
  writeFileSync(GROUPS, file(f.about, cases));
}

const read = <C>(path: string): Fixture<C> => JSON.parse(readFileSync(path, 'utf8')) as Fixture<C>;

if (UPDATE) {
  writeMarkdown(read<MarkdownCase>(MARKDOWN));
  writeGroups(read<GroupsCase>(GROUPS));
}

const markdown = read<MarkdownCase>(MARKDOWN);
const sessionGroups = read<GroupsCase>(GROUPS);

// ---------- the checks ----------

/// A pending case is skipped, and says so by name, in the report and in the run's output.
function each<C extends Case<unknown, unknown>>(file: string, cases: readonly C[], check: (c: C) => void): void {
  for (const c of cases) {
    if (c.pending) {
      console.info(`pending, skipped: ${file}: ${c.name} (${c.pending})`);
      it.skip(`${c.name} — pending: ${c.pending}`, () => {});
    } else it(c.name, () => check(c));
  }
}

describe('fixtures/live/markdown-blocks.json', () => {
  it('names every case once, and gives every case an input and an expected value', () => {
    expect(new Set(markdown.cases.map((c) => c.name)).size).toBe(markdown.cases.length);
    for (const c of markdown.cases) expect([c.name, typeof c.input, Array.isArray(c.expected)]).toEqual([c.name, 'string', true]);
  });

  each('markdown-blocks.json', markdown.cases, (c) => {
    expect(blocks(c.input)).toEqual(c.expected);
  });
});

describe('fixtures/live/session-groups.json', () => {
  it('names every case once, and gives every case a clock, its rows and an expected value', () => {
    expect(new Set(sessionGroups.cases.map((c) => c.name)).size).toBe(sessionGroups.cases.length);
    for (const c of sessionGroups.cases) expect([c.name, typeof c.now, Array.isArray(c.sessions), typeof c.expected]).toEqual([c.name, 'number', true, 'object']);
  });

  each('session-groups.json', sessionGroups.cases, (c) => {
    expect(groups(c.sessions, c.now)).toEqual(c.expected);
  });
});
