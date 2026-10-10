// New for agent-router: the live view's `md()` (assets/live.html), as a parser that produces a
// block tree instead of HTML. A screen renders the tree as React elements whose text is text, so
// nothing a transcript contains can become markup or an attribute (SE-C90): there is no `esc()`
// here to get wrong, because nothing here is ever HTML.
//
// The rules are the web's (decision F in docs/web-console-carbon.md), as
// docs/web-console-contracts.md writes them down in SE-C90 to SE-C104, but for lists, which follow
// the phones, as decided on 2026-10-05: bullets and numbers are never one list, and a numbered list
// is drawn numbered, counting from 1 whatever numbers its source wrote (SE-C99). Where the page broke
// its own rule, the rule is kept:
// - inline code is verbatim, so `**x**` between backticks is not bold and a link there is not a
//   link (SE-C95), because inline code is cut out before emphasis and links are read;
// - a `* ` list marker is never read as emphasis (SE-C97), because a line's block is decided
//   before its inline formatting is;
// - no text is dropped (SE-C104): a fence opened and closed on one line keeps its text as code, and
//   a fence's language tag is the whole of its first line, so `c++` is not split into a tag `c`
//   and a first line of code `++`.
// One more, which no contract names: a table that starts right after a list no longer draws above
// that list, as the page's did by leaving the list open across the table.

export type Inline =
  | { readonly type: 'text'; readonly text: string }
  | { readonly type: 'code'; readonly text: string }
  | { readonly type: 'bold'; readonly children: readonly Inline[] }
  | { readonly type: 'italic'; readonly children: readonly Inline[] }
  | { readonly type: 'link'; readonly href: string; readonly children: readonly Inline[] };

export type Block =
  | { readonly type: 'paragraph'; readonly children: readonly Inline[] }
  /// `#` renders at the third heading level, `##` the fourth, `###` the fifth and anything deeper
  /// the sixth, so a heading inside a message sits under the screen's own (SE-C98).
  | { readonly type: 'heading'; readonly level: 3 | 4 | 5 | 6; readonly children: readonly Inline[] }
  /// One flat list of one kind, nothing nesting: bullets, or, `ordered`, numbers, which are drawn
  /// counting 1., 2., 3. from the list's first item, never the source's own numbers (SE-C99).
  | { readonly type: 'list'; readonly ordered: boolean; readonly items: readonly (readonly Inline[])[] }
  | { readonly type: 'quote'; readonly children: readonly Inline[] }
  | { readonly type: 'table'; readonly head: readonly (readonly Inline[])[]; readonly rows: readonly (readonly (readonly Inline[])[])[] }
  /// Verbatim: nothing inside a fence is read as Markdown (SE-C93).
  | { readonly type: 'code'; readonly text: string };

/// Transcript Markdown as blocks.
export function md(src: string | null | undefined): Block[] {
  const out: Block[] = [];
  const text = String(src ?? '');
  // A fence runs from three backticks to the next three. An unterminated one is not code: its
  // backticks and everything after them stay prose (SE-C94).
  const fence = /```([\s\S]*?)```/g;
  let at = 0;
  for (let m = fence.exec(text); m; m = fence.exec(text)) {
    prose(text.slice(at, m.index), out);
    out.push({ type: 'code', text: fenced(m[1] ?? '') });
    at = m.index + m[0].length;
  }
  prose(text.slice(at), out);
  return out;
}

/// A fence's code: its first line dropped when it is a language tag — one word, or nothing — and
/// a newline before the closing fence dropped (SE-C93). With no newline at all the fence was
/// opened and closed on one line, and everything between is code (SE-C104).
function fenced(inner: string): string {
  const nl = inner.indexOf('\n');
  const code = nl >= 0 && /^[ \t]*[^\s`]*[ \t]*$/.test(inner.slice(0, nl)) ? inner.slice(nl + 1) : inner;
  return code.replace(/\n$/, '');
}

const BULLET = /^\s*[-*]\s+(.*)$/;
const NUMBERED = /^\s*\d+[.)]\s+(.*)$/;
const HEADING = /^(#{1,6})\s+(.*)$/;
const QUOTE = /^>\s?(.*)$/;
/// A table is a row of pipes immediately followed by a separator row of pipes, dashes, colons and
/// spaces; without the separator a line of pipes is prose and stays prose (SE-C101).
const ROW = /^\s*\|.*\|\s*$/;
const RULE = /^\s*\|[\s:|-]+\|\s*$/;

const cells = (line: string): Inline[][] =>
  line
    .replace(/^\s*\|/, '')
    .replace(/\|\s*$/, '')
    .split('|')
    .map((c) => inline(c.trim()));

/// The blocks of a stretch of prose, line by line: every non-blank line not part of a list, a
/// table, a heading or a quote is a paragraph of its own, consecutive lines are not joined, and
/// blank lines add nothing (SE-C103).
function prose(src: string, out: Block[]): void {
  if (!src) return;
  let list: { ordered: boolean; items: Inline[][] } | null = null;
  let table: { head: Inline[][]; rows: Inline[][][] } | null = null;
  const flushList = () => {
    if (list) out.push({ type: 'list', ordered: list.ordered, items: list.items });
    list = null;
  };
  const flushTable = () => {
    if (table) out.push({ type: 'table', head: table.head, rows: table.rows });
    table = null;
  };
  const lines = src.split('\n');
  for (let i = 0; i < lines.length; i++) {
    const line = (lines[i] ?? '').trimEnd();
    const isRow = ROW.test(line);
    if (isRow && !table && RULE.test((lines[i + 1] ?? '').trimEnd())) {
      flushList();
      table = { head: cells(line), rows: [] };
      i++;
      continue;
    }
    if (table) {
      if (isRow && !RULE.test(line)) {
        table.rows.push(cells(line));
        continue;
      }
      flushTable();
    }
    // A bullet is read first, as the phones read it, so "- 1. x" is a bullet whose text is "1. x".
    const bullet = BULLET.exec(line);
    const item = bullet ?? NUMBERED.exec(line);
    if (item) {
      // Where the kind changes, the list ends and another starts: "- one", "2. two", "* three" is a
      // bulleted list, a numbered list of one and another bulleted list (SE-C99).
      const ordered = !bullet;
      if (list && list.ordered !== ordered) flushList();
      (list ??= { ordered, items: [] }).items.push(inline(item[1] ?? ''));
      continue;
    }
    flushList();
    const head = HEADING.exec(line);
    const quote = QUOTE.exec(line);
    if (head) {
      const level = Math.min((head[1] ?? '').length + 2, 6) as 3 | 4 | 5 | 6;
      out.push({ type: 'heading', level, children: inline(head[2] ?? '') });
    } else if (quote) {
      out.push({ type: 'quote', children: inline(quote[1] ?? '') });
    } else if (line.trim()) {
      out.push({ type: 'paragraph', children: inline(line) });
    }
  }
  flushList();
  flushTable();
}

// ---------- inline ----------
//
// The page read inline Markdown with four regular expressions in a row — inline code, bold,
// italic, links — each over the output of the one before. Here each pass reads a sequence of
// units: a character of text, or a node an earlier pass built, which a later pattern sees as one
// opaque character. So bold can still hold inline code, italic can still hold bold and a link's
// text can still hold all three, as on the page, while nothing ever reads inside inline code.

type Unit = { readonly ch: string } | { readonly node: Inline; readonly src: string };

/// What a node stands for in the string a pattern reads: one character that no pattern names.
const NODE = String.fromCharCode(0xe000);

const flat = (units: readonly Unit[]): string => units.map((u) => ('ch' in u ? u.ch : NODE)).join('');
const source = (units: readonly Unit[]): string => units.map((u) => ('ch' in u ? u.ch : u.src)).join('');

function inlines(units: readonly Unit[]): Inline[] {
  const out: Inline[] = [];
  let text = '';
  for (const u of units) {
    if ('ch' in u) {
      text += u.ch;
      continue;
    }
    if (text) out.push({ type: 'text', text });
    text = '';
    out.push(u.node);
  }
  if (text) out.push({ type: 'text', text });
  return out;
}

/// One pass: every match of `pattern` (global, leftmost first, never overlapping) becomes a node.
/// `build` gets the match and the units it covers, and says which of them the node replaces —
/// from `start` to the end of the match — and what it is.
function pass(
  units: Unit[],
  pattern: RegExp,
  build: (m: RegExpExecArray, covered: Unit[]) => { start: number; node: Inline } | null,
): Unit[] {
  const s = flat(units);
  const out: Unit[] = [];
  let at = 0;
  for (let m = pattern.exec(s); m; m = pattern.exec(s)) {
    const end = m.index + m[0].length;
    const covered = units.slice(m.index, end);
    const built = build(m, covered);
    if (!built) continue;
    out.push(...units.slice(at, m.index + built.start));
    out.push({ node: built.node, src: source(units.slice(m.index + built.start, end)) });
    at = end;
  }
  out.push(...units.slice(at));
  return out;
}

/// One unit per UTF-16 code unit, so a unit's index is its index in the string a pattern reads.
function chars(s: string): Unit[] {
  const out: Unit[] = [];
  for (let i = 0; i < s.length; i++) out.push({ ch: s.charAt(i) });
  return out;
}

/// A line's inline formatting.
export function inline(text: string): Inline[] {
  // Inline code first, from one backtick to the next on the same line: verbatim (SE-C95).
  let units: Unit[] = [];
  const code = /`([^`\n]+)`/g;
  let at = 0;
  for (let m = code.exec(text); m; m = code.exec(text)) {
    units.push(...chars(text.slice(at, m.index)), { node: { type: 'code', text: m[1] ?? '' }, src: m[0] });
    at = m.index + m[0].length;
  }
  units.push(...chars(text.slice(at)));

  // `**bold**`, on one line with no asterisk inside (SE-C96).
  units = pass(units, /\*\*([^*\n]+)\*\*/g, (_m, covered) => ({
    start: 0,
    node: { type: 'bold', children: inlines(covered.slice(2, -2)) },
  }));
  // `*italic*`, only when its opening asterisk follows the start of the text, whitespace or "(",
  // so `d*no*e`, `*.rs` and `2 ** 8` stay as typed; underscores never mean emphasis (SE-C96).
  units = pass(units, /(^|[\s(])\*([^*\n]+)\*/g, (m, covered) => {
    const lead = (m[1] ?? '').length;
    return { start: lead, node: { type: 'italic', children: inlines(covered.slice(lead + 1, -1)) } };
  });
  // `[text](https://…)`: only an http or https address is a link (SE-C91); a bare URL is text
  // (SE-C92). The address is read from the source, so it is never anything but text.
  units = pass(units, /\[([^\]\n]+)\]\((https?:[^)\s]+)\)/g, (m, covered) => {
    const label = (m[1] ?? '').length;
    return {
      start: 0,
      node: { type: 'link', href: source(covered.slice(label + 3, -1)), children: inlines(covered.slice(1, label + 1)) },
    };
  });
  return inlines(units);
}

/// The text a reader sees in a block tree, markers gone: for a test that nothing was dropped, and
/// for an accessible name.
export function textOf(blocks: readonly Block[]): string {
  const ofInline = (xs: readonly Inline[]): string =>
    xs.map((x) => (x.type === 'text' || x.type === 'code' ? x.text : ofInline(x.children))).join('');
  return blocks
    .map((b) => {
      switch (b.type) {
        case 'code':
          return b.text;
        case 'list':
          return b.items.map(ofInline).join('\n');
        case 'table':
          return [b.head, ...b.rows].map((r) => r.map(ofInline).join(' ')).join('\n');
        default:
          return ofInline(b.children);
      }
    })
    .join('\n');
}
