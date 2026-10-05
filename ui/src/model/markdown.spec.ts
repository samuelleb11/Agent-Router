// @vitest-environment node
// New for agent-router: the Markdown parser's contracts beyond what the old check pinned, among
// them the ones the live view broke (docs/web-console-contracts.md, "Markdown"). The web's
// semantics are the rule (decision F); the phones are brought to them by AGEN-0005.
import { describe, expect, it } from 'vitest';
import { md, textOf, type Inline } from './markdown';

const t = (text: string): Inline => ({ type: 'text', text });
const p = (...children: Inline[]) => ({ type: 'paragraph', children });
const b = (...children: Inline[]): Inline => ({ type: 'bold', children });
const i = (...children: Inline[]): Inline => ({ type: 'italic', children });
const code = (text: string): Inline => ({ type: 'code', text });

describe('md', () => {
  it('SE-C90: nothing in a transcript becomes markup or an attribute: a link\'s address and text are text', () => {
    // The text that once gave a Markdown link an `onmouseover` (29c5eb3).
    expect(md('[x" onmouseover="alert(1)](https://a.example/"onmouseover="y)')).toEqual([
      p({ type: 'link', href: 'https://a.example/"onmouseover="y', children: [t('x" onmouseover="alert(1)')] }),
    ]);
  });

  it('SE-C91, SE-C92: only an http or https address is a link, and a bare URL is text', () => {
    expect(md('[docs](https://example.com/a?b=1&c=2)')).toEqual([p({ type: 'link', href: 'https://example.com/a?b=1&c=2', children: [t('docs')] })]);
    for (const s of ['[x](javascript:alert(1))', '[x](data:text/html,hi)', '[x](/relative)', 'see https://example.com']) expect(md(s)).toEqual([p(t(s))]);
  });

  it('SE-C93: a fence is verbatim code, its language tag dropped, and the newline before its closing fence too', () => {
    expect(md('```ts\nconst a = **b** `c`;\n```')).toEqual([{ type: 'code', text: 'const a = **b** `c`;' }]);
  });

  it('SE-C94: an unterminated fence is not code: its backticks and what follows stay prose', () => {
    expect(md('```\nunterminated')).toEqual([p(t('```')), p(t('unterminated'))]);
  });

  it('SE-C95: inline code is verbatim, where the page read bold and links inside backticks', () => {
    expect(md('`**x**` and `[a](https://b.example)`')).toEqual([p(code('**x**'), t(' and '), code('[a](https://b.example)'))]);
    // Emphasis can still hold code, and a link's text can hold emphasis, as on the page; an italic
    // asterisk straight after "[" is not one, there as here.
    expect(md('**see `x`** [**it**](https://c.example) [*no*](https://d.example)')).toEqual([
      p(b(t('see '), code('x')), t(' '), { type: 'link', href: 'https://c.example', children: [b(t('it'))] }, t(' '), { type: 'link', href: 'https://d.example', children: [t('*no*')] }),
    ]);
  });

  it('SE-C96: bold and italic, and what is not emphasis stays as typed', () => {
    expect(md('**bold** and *it* and (*par*)')).toEqual([p(b(t('bold')), t(' and '), i(t('it')), t(' and ('), i(t('par')), t(')'))]);
    for (const s of ['d*no*e', '*.rs', '2 ** 8', 'a lone *', 'snake_case_name', '__dunder__']) expect(md(s), s).toEqual([p(t(s))]);
  });

  it('SE-C97: a "* " list marker is never read as emphasis, where the page lost the marker', () => {
    expect(md('* item *with* stars')).toEqual([{ type: 'list', items: [[t('item '), i(t('with')), t(' stars')]] }]);
  });

  it("SE-C98: headings sit under the screen's own, and seven # or no space is prose", () => {
    expect(md('# a\n## b\n### c\n#### d\n###### e')).toEqual(
      ([3, 4, 5, 6, 6] as const).map((level, n) => ({ type: 'heading', level, children: [t('abcde'.charAt(n))] })),
    );
    expect(md('####### seven\n#nospace')).toEqual([p(t('####### seven')), p(t('#nospace'))]);
  });

  it('SE-C99: bullets and numbers make one flat list, ended by a blank or any other line, the numbers not kept', () => {
    expect(md('- one\n2. two\n   * three\n3) four\n\n- five\nprose\n- six')).toEqual([
      { type: 'list', items: [[t('one')], [t('two')], [t('three')], [t('four')]] },
      { type: 'list', items: [[t('five')]] },
      p(t('prose')),
      { type: 'list', items: [[t('six')]] },
    ]);
  });

  it('SE-C100: each quoted line is a quote of its own, the space after ">" optional', () => {
    expect(md('> one\n>two')).toEqual([
      { type: 'quote', children: [t('one')] },
      { type: 'quote', children: [t('two')] },
    ]);
  });

  it('SE-C101: a table needs its separator row; cells are trimmed and keep their formatting', () => {
    expect(md('| a | **b** |\n|:--|--:|\n| 1 | `2` |\nafter')).toEqual([{ type: 'table', head: [[t('a')], [b(t('b'))]], rows: [[[t('1')], [code('2')]]] }, p(t('after'))]);
    expect(md('| not | a table |')).toEqual([p(t('| not | a table |'))]);
  });

  it('SE-C103: every other non-blank line is a paragraph of its own', () => {
    expect(md('plain\nlines\n\n\nthird')).toEqual([p(t('plain')), p(t('lines')), p(t('third'))]);
  });

  it('SE-C104: no text is dropped: a fence opened and closed on one line keeps its text, and a c++ tag leaves no ++ behind', () => {
    expect(md('```x```')).toEqual([{ type: 'code', text: 'x' }]);
    expect(md('```c++\nint main();\n```')).toEqual([{ type: 'code', text: 'int main();' }]);
    expect(md('Run this:\n```sh\nls -la\n```\nthen **stop**.')).toEqual([p(t('Run this:')), { type: 'code', text: 'ls -la' }, p(t('then '), b(t('stop')), t('.'))]);
    const prose = 'a <b> & "c" \'d\' ‹e› f';
    expect(textOf(md(prose))).toBe(prose);
  });

  it('keeps a list above the table that follows it, where the page drew the table first', () => {
    expect(md('- a\n| x | y |\n|---|---|\n| 1 | 2 |').map((block) => block.type)).toEqual(['list', 'table']);
  });
});
