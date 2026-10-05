// New for agent-router: what the Sessions screen draws, held against the fixtures the web and both
// phones share, fixtures/live/ (AGEN-0005). ~/model/shared-fixtures.spec.ts holds the web's model to
// them; this holds the screen that draws the model: its Markdown, read back from the elements it
// renders, and its list, read back from the groups and rows on screen. A screen that dropped a run,
// a style or a row would pass the model's check and fail this one.
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { render } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { md } from '~/model/markdown';
import { listView, type SessionRow } from '~/model/session';
import { press } from '~/test-utils';
import { List, type RowActions } from './List';
import { Markdown } from './Markdown';

const DIR = join(__dirname, '..', '..', '..', '..', 'fixtures', 'live');

/// The shapes of the shared files, as their spec writes them.
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
interface Groups {
  recent: Group[];
  older: Group[];
  archived: string[];
}
interface Case {
  name: string;
  pending?: string;
}
interface MarkdownCase extends Case {
  input: string;
  expected: Canonical[];
}
interface GroupsCase extends Case {
  now: number;
  sessions: SessionRow[];
  expected: Groups;
}

const read = <C,>(file: string): C[] => (JSON.parse(readFileSync(join(DIR, file), 'utf8')) as { cases: C[] }).cases;

/// An element's text as runs, each with the styles its ancestors give it, merged where they agree.
function runsOf(root: Element): Run[] {
  const out: Run[] = [];
  const walk = (node: Node, style: Omit<Run, 'text'>) => {
    if (node.nodeType === Node.TEXT_NODE) {
      const text = node.textContent ?? '';
      if (!text) return;
      const last = out.at(-1);
      if (last && last.bold === style.bold && last.italic === style.italic && last.code === style.code && last.href === style.href) {
        last.text += text;
        return;
      }
      out.push({ text, ...style });
      return;
    }
    if (!(node instanceof Element)) return;
    const tag = node.tagName.toLowerCase();
    const next = { ...style };
    if (tag === 'strong') next.bold = true;
    if (tag === 'em') next.italic = true;
    if (tag === 'code') next.code = true;
    if (tag === 'a') next.href = node.getAttribute('href') ?? '';
    for (const child of node.childNodes) walk(child, next);
  };
  for (const child of root.childNodes) walk(child, {});
  return out;
}

/// What the screen drew, block by block, in the shared shape.
function drawn(container: Element): Canonical[] {
  const md = container.firstElementChild as Element;
  return [...md.children].map((el): Canonical => {
    const tag = el.tagName.toLowerCase();
    if (tag === 'p') return { type: 'paragraph', runs: runsOf(el) };
    if (/^h[3-6]$/.test(tag)) return { type: 'heading', level: Number(tag.slice(1)), runs: runsOf(el) };
    if (tag === 'ul' || tag === 'ol') return { type: 'list', ordered: tag === 'ol', items: [...el.querySelectorAll(':scope > li')].map(runsOf) };
    if (tag === 'blockquote') return { type: 'quote', runs: runsOf(el) };
    if (el.querySelector('table')) {
      const table = el.querySelector('table') as HTMLTableElement;
      return {
        type: 'table',
        head: [...table.querySelectorAll('thead th')].map(runsOf),
        rows: [...table.querySelectorAll('tbody tr')].map((tr) => [...tr.querySelectorAll('td')].map(runsOf)),
      };
    }
    // A fence: Carbon's snippet, its code verbatim and its language tag gone.
    return { type: 'code', lang: null, text: el.querySelector('pre code')?.textContent ?? '' };
  });
}

const actions: RowActions = { open: () => {}, archive: () => {}, busy: new Set(), notices: new Map(), dismiss: () => {}, openId: null };

/// The groups the screen drew: each heading's project, tone and halo, and its rows in order.
function groupsOf(container: Element): Group[] {
  return [...container.querySelectorAll('.cds--contained-list')]
    .filter((list) => !list.closest('section'))
    .map((list) => {
      const dot = list.querySelector('.cds--contained-list__label [role="img"]');
      return {
        project: list.querySelector('.cds--contained-list__label span span:nth-child(2)')?.textContent ?? '',
        tone: dot?.getAttribute('data-tone') ?? null,
        halo: dot?.hasAttribute('data-live') ?? false,
        sessions: [...list.querySelectorAll('[data-session]')].map((li) => li.getAttribute('data-session') ?? ''),
      };
    });
}

describe('the screen against the shared fixtures', () => {
  it('SE-C216: the screen draws the Markdown blocks and the session groups of fixtures/live/, the files the web and both phones test against', async () => {
    const markdown = read<MarkdownCase>('markdown-blocks.json').filter((c) => !c.pending);
    const groups = read<GroupsCase>('session-groups.json').filter((c) => !c.pending);
    expect(markdown.length).toBeGreaterThan(0);
    expect(groups.length).toBeGreaterThan(0);
    for (const c of markdown) {
      const { container, unmount } = render(<Markdown blocks={md(c.input)} />);
      expect(drawn(container), c.name).toEqual(c.expected);
      unmount();
    }
    for (const c of groups) {
      const view = listView(c.sessions, c.now);
      const { container, unmount, getByRole } = render(<List view={view} hosted={false} wrapperWarning={null} actions={actions} />);
      // The older fold is drawn closed; opened, its groups are drawn as the recent ones are.
      if (view.older) await press(getByRole('button', { name: view.older.label }));
      const all = groupsOf(container);
      const recent = all.slice(0, view.recent.length);
      const older = all.slice(view.recent.length);
      const archived = [...container.querySelectorAll('section [data-session]')].map((li) => li.getAttribute('data-session') ?? '');
      expect({ recent, older, archived }, c.name).toEqual(c.expected);
      unmount();
    }
  });
});
