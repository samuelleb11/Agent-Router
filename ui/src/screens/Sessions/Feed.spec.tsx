// New for agent-router: the feed as the screen draws it — each kind of row, in the order the stream
// delivered it, folded where it is tall — and the reader's place in it. The model decides how each
// row reads (feedRowView, pinned in ~/model); these pin what is drawn from it.
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { createRef } from 'react';
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { FeedEntry, StreamItem } from '~/model/session';
import { press } from '~/test-utils';
import { Clamp } from './Clamp';
import { FakeEventSource } from './fakes';
import { Feed } from './Feed';
import { A, json, renderSessions, streamOf, stubRouter, working } from './kit';

/// The stylesheets, read as source: jsdom computes no style.
const plain = (s: string) => s.replace(/\/\*[\s\S]*?\*\//g, '').replace(/\/\/.*$/gm, '');
const FEED = plain(readFileSync(join(__dirname, 'Feed.module.scss'), 'utf8'));
function block(css: string, opener: string): string {
  const at = css.indexOf(opener);
  expect(at, opener).toBeGreaterThanOrEqual(0);
  const open = css.indexOf('{', at);
  let depth = 0;
  for (let i = open; i < css.length; i++) {
    if (css[i] === '{') depth += 1;
    if (css[i] === '}' && --depth === 0) return css.slice(open + 1, i);
  }
  throw new Error(`unclosed ${opener}`);
}

const rows = (...items: StreamItem[]): FeedEntry[] => items.map((item) => ({ kind: 'row', item }));
let made = 0;
const keys = (() => {
  const map = new WeakMap<FeedEntry, string>();
  return (e: FeedEntry) => {
    let k = map.get(e);
    if (!k) {
      made += 1;
      k = `k${made}`;
      map.set(e, k);
    }
    return k;
  };
})();
const deliver = vi.fn(async () => ({ ok: true, notice: null }));

function draw(entries: FeedEntry[]) {
  return render(<Feed session={A} entries={entries} deliver={deliver} keys={keys} arrived={null} arrivedRef={createRef()} />);
}
const entriesOf = (c: HTMLElement) => [...c.querySelectorAll<HTMLElement>('[data-entry]')];

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe('the feed', () => {
  it('SE-C73: rows appear in the order the stream delivered them, never re-sorted by any timestamp', () => {
    const { container } = draw(
      rows(
        { role: 'user', kind: 'text', text: 'first said', t: '2026-10-05T12:00:05.000Z' },
        { role: 'user', kind: 'tool_result', text: 'a result before its call', t: '2026-10-05T12:00:01.000Z' },
        { role: 'assistant', kind: 'text', text: 'compacted, stamped in the past', t: '2026-10-01T00:00:00.000Z' },
      ),
    );
    expect(entriesOf(container).map((e) => e.querySelector('p, code')?.textContent)).toEqual(['first said', 'a result before its call', 'compacted, stamped in the past']);
  });

  it('SE-C77: a cancellation is a centred, faint, italic note with no speaker', () => {
    const { container } = draw(rows({ role: 'user', kind: 'interrupt', text: '[Request interrupted by user]' }));
    const [e] = entriesOf(container);
    expect(e).toHaveAttribute('data-kind', 'interrupt');
    expect(e?.textContent).toBe('[Request interrupted by user]');
    const css = block(FEED, '\n.interrupt {');
    expect(css).toMatch(/text-align:\s*center/);
    expect(css).toMatch(/font-style:\s*italic/);
    expect(css).toMatch(/color:\s*\$text-secondary/);
  });

  it('SE-C78: the person’s words, answers, commands and other rows are shown as written, never as Markdown; assistant prose and side answers are Markdown', () => {
    const { container } = draw(
      rows(
        { role: 'user', kind: 'text', text: '**as typed**\n  indented' },
        { role: 'user', kind: 'answer', text: 'Layout: **Beside**' },
        { role: 'user', kind: 'command', text: '/compact **now**' },
        { role: 'assistant', kind: 'text', text: 'it is **bold**' },
        { role: 'assistant', kind: 'aside', text: 'aside **bold**' },
      ),
    );
    const [person, answer, command, prose, aside] = entriesOf(container);
    for (const e of [person, answer, command]) expect(e?.querySelector('strong')).toBeNull();
    expect(person?.querySelector('p')?.textContent).toBe('**as typed**\n  indented');
    expect(block(FEED, '\n.plain {')).toMatch(/white-space:\s*pre-wrap/);
    expect(answer).toHaveTextContent('Layout: **Beside**');
    expect(command).toHaveTextContent('/compact **now**');
    expect(prose?.querySelector('strong')).toHaveTextContent('bold');
    expect(aside?.querySelector('strong')).toHaveTextContent('bold');
  });

  it('SE-C79: a tool call is its name and its duration over a command line, and only Bash gets the "$" glyph', () => {
    const { container } = draw(
      rows({ role: 'assistant', kind: 'tool_use', name: 'Bash', text: 'ls -la', dur_ms: 2810 }, { role: 'assistant', kind: 'tool_use', name: 'Read', text: '/x/y.rs' }),
    );
    const [bash, read] = entriesOf(container);
    expect(within(bash as HTMLElement).getByText('Bash')).toBeInTheDocument();
    expect(within(bash as HTMLElement).getByText('2.8s')).toBeInTheDocument();
    expect(bash?.querySelector('[data-part="command"]')).toHaveAttribute('data-shell');
    expect(read?.querySelector('[data-part="command"]')).not.toHaveAttribute('data-shell');
    expect(block(FEED, '.command[data-shell]')).toMatch(/content:\s*'\$ '/);
  });

  it('SE-C80: a tool’s result hangs off its call, in the same block under OUT, and an empty output adds nothing', () => {
    const { container } = draw(
      rows(
        { role: 'assistant', kind: 'tool_use', name: 'Bash', text: 'make' },
        { role: 'user', kind: 'tool_result', text: 'built' },
        { role: 'assistant', kind: 'tool_use', name: 'Read', text: '/x' },
        { role: 'user', kind: 'tool_result', text: '' },
      ),
    );
    const [make, read] = entriesOf(container);
    expect(entriesOf(container)).toHaveLength(2);
    expect(make?.querySelector('[data-part="command"]')).not.toBeNull();
    expect(make?.querySelector('[data-part="output"]')).toHaveTextContent('OUTbuilt');
    expect(read?.querySelector('[data-part="output"]')).toBeNull();
    expect(read).not.toHaveTextContent('OUT');
  });

  it('SE-C82: thinking collapses to "Thought for 2.4s ›", opens and closes on demand, and is never clamped', async () => {
    const { container } = draw(rows({ role: 'assistant', kind: 'thinking', text: 'what to do next', dur_ms: 2400 }, { kind: 'thinking', text: 'quick', dur_ms: 300 }));
    const toggle = screen.getByRole('button', { name: 'Thought for 2.4s ›' });
    expect(screen.getByRole('button', { name: 'Thought ›' })).toBeInTheDocument();
    expect(screen.queryByText('what to do next')).toBeNull();
    await press(toggle);
    expect(toggle).toHaveAttribute('aria-expanded', 'true');
    expect(screen.getByText('what to do next')).toBeInTheDocument();
    expect(entriesOf(container)[0]?.querySelector('[data-fold]')).toBeNull();
    await press(toggle);
    expect(screen.queryByText('what to do next')).toBeNull();
  });

  it('SE-C83: a subagent’s rows sit on a warn rail labelled "subagent"; an answer is "✓ answered" on an ok rail; a command "⌘ command"; a side answer "side answer"', () => {
    const { container } = draw(
      rows(
        { role: 'assistant', kind: 'text', text: 'from a subagent', sidechain: true },
        { role: 'user', kind: 'answer', text: 'B' },
        { role: 'user', kind: 'command', text: '/compact' },
        { role: 'assistant', kind: 'aside', text: 'on the side' },
      ),
    );
    const [sub, answer, command, aside] = entriesOf(container);
    expect(sub).toHaveAttribute('data-subagent');
    expect(sub).toHaveTextContent('subagent');
    expect(answer).toHaveAttribute('data-answer');
    expect(answer).toHaveTextContent('✓ answered');
    expect(command).toHaveTextContent('⌘ command');
    expect(aside).toHaveTextContent('side answer');
    expect(block(FEED, '.rail[data-subagent]')).toMatch(/border-inline-start-color:\s*var\(--ar-warn-fill\)/);
    expect(block(FEED, '.rail[data-answer]')).toMatch(/border-inline-start-color:\s*var\(--ar-ok-fill\)/);
  });

  it('SE-C91: a link opens in a new tab with no opener, and only an http or https address is one', () => {
    const { container } = draw(rows({ role: 'assistant', kind: 'text', text: '[docs](https://example.com/a) and [no](javascript:alert(1))' }));
    const links = [...container.querySelectorAll('a')];
    expect(links.map((a) => [a.getAttribute('href'), a.getAttribute('target'), a.getAttribute('rel')])).toEqual([['https://example.com/a', '_blank', 'noopener noreferrer']]);
    expect(container).toHaveTextContent('[no](javascript:alert(1))');
  });

  it('SE-C99: a numbered list is a real ordered list counting from 1, whatever numbers its source wrote, and bullets beside it are a list of their own', () => {
    const { container } = draw(rows({ role: 'assistant', kind: 'text', text: '3. third\n7) seventh\n- a bullet\n\n10. after a blank' }));
    const lists = [...container.querySelectorAll('[data-entry] ol, [data-entry] ul')];
    expect(lists.map((l) => [l.tagName, [...l.querySelectorAll(':scope > li')].map((li) => li.textContent)])).toEqual([
      ['OL', ['third', 'seventh']],
      ['UL', ['a bullet']],
      ['OL', ['after a blank']],
    ]);
    // Counted by the list itself from 1: no start, no value of the source's, nothing reversed.
    for (const ol of container.querySelectorAll('ol')) {
      expect([ol.getAttribute('start'), ol.hasAttribute('reversed'), [...ol.querySelectorAll('li[value]')].length]).toEqual([null, false, 0]);
    }
    // Its numbers hang in the list's own padding, so a phone's rail never cuts them off.
    expect(block(FEED, '.list.ordered {')).toMatch(/padding-inline-start:\s*\$spacing-07/);
  });
});

describe('the fold', () => {
  /// Lay the fold out: its content is `natural` px tall, and the fold `shown` px while clamped.
  function layout(natural: number, shown: number) {
    const seen: string[] = [];
    vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
      const fold = this.hasAttribute('data-fold') ? this : null;
      if (fold) seen.push(`${fold.dataset.fold}:${fold.className ? 'clamped' : 'open'}`);
      const h = fold ? (fold.className ? shown : natural) : natural;
      return { height: h, width: 300, top: 0, left: 0, bottom: h, right: 300, x: 0, y: 0, toJSON: () => ({}) } as DOMRect;
    });
    return seen;
  }

  it('SE-C86, SE-C85: the fold clamps first and measures second, and a fold that overflows says how many lines it hides, then "Show less"', async () => {
    const seen = layout(396, 176);
    render(
      <Clamp kind="prose">
        <p>tall</p>
      </Clamp>,
    );
    // Measured while clamped: the page that measured first never folded anything.
    expect(seen[0]).toBe('measuring:clamped');
    const toggle = screen.getByRole('button', { name: 'Show 22 lines' });
    expect(toggle).toHaveAttribute('aria-expanded', 'false');
    await press(toggle);
    expect(screen.getByRole('button', { name: 'Show less' })).toHaveAttribute('aria-expanded', 'true');
  });

  it('SE-C86: content that fits within four pixels is unclamped and gets no toggle', () => {
    layout(178, 176);
    const { container } = render(
      <Clamp kind="term">
        <p>short</p>
      </Clamp>,
    );
    expect(container.querySelector('[data-fold]')).toHaveAttribute('data-fold', 'fits');
    expect(screen.queryByRole('button')).toBeNull();
  });
});

describe('following the newest row', () => {
  beforeEach(() => FakeEventSource.reset());

  /// The feed's scroller, laid out: `content` px of rows in a `box` px window.
  function scroller(content: () => number, box = 200) {
    const el = document.querySelector('[data-feed-scroller]') as HTMLElement;
    Object.defineProperty(el, 'scrollHeight', { configurable: true, get: content });
    Object.defineProperty(el, 'clientHeight', { configurable: true, get: () => box });
    return el;
  }

  it('SE-C88, SE-C87: following resumes when the session opens, when the feed is rebuilt, and when the reader sends; scrolled up, the reader is left where they are', async () => {
    stubRouter((req) => (req.method === 'POST' && req.path === '/api/live/send' ? json({ ok: true }) : undefined));
    renderSessions(`/live?session=${A}`);
    await waitFor(() => streamOf(A));
    let tall = 1000;
    const el = scroller(() => tall);
    act(() => {
      streamOf(A).open().row({ role: 'assistant', kind: 'text', text: 'one' }).status(working());
    });
    // Opened at its end.
    await waitFor(() => expect(el.scrollTop).toBe(1000));
    // The reader scrolls up: a new row leaves them where they are.
    el.scrollTop = 100;
    fireEvent.scroll(el);
    tall = 1400;
    act(() => {
      streamOf(A).row({ role: 'assistant', kind: 'text', text: 'two' });
    });
    expect(el.scrollTop).toBe(100);
    // They send: following again, at the bottom.
    fireEvent.change(screen.getByRole('textbox', { name: 'Message to this session' }), { target: { value: 'go on' } });
    await press(screen.getByRole('button', { name: 'Send' }));
    await waitFor(() => expect(el.scrollTop).toBe(1400));
    // Up again; then the link drops and the browser's retry replays the tail: a new feed, followed.
    el.scrollTop = 50;
    fireEvent.scroll(el);
    tall = 900;
    act(() => {
      streamOf(A).drop(0).open().row({ role: 'assistant', kind: 'text', text: 'one' });
    });
    await waitFor(() => expect(el.scrollTop).toBe(900));
  });
});
