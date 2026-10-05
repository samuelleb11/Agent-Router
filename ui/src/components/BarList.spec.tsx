import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { Theme } from '@carbon/react';
import { act, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { stubCoarse, stubNarrow } from '~/test-utils';
import { BarList, type BarRow } from './BarList';
import styles from './BarList.module.scss';

const plain = (s: string) => s.replace(/\/\*[\s\S]*?\*\//g, '').replace(/\/\/.*$/gm, '');
const scss = plain(readFileSync(join(__dirname, 'BarList.module.scss'), 'utf8'));
const themes = plain(readFileSync(join(__dirname, '../styles.scss'), 'utf8'));
const declares = (mixin: string) =>
  new Set([...new RegExp(`@mixin ${mixin} \\{([^}]*)\\}`).exec(themes)![1]!.matchAll(/(--ar-[a-z0-9-]+)\s*:/g)].map((m) => m[1]));
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

/// The Projects list as the dashboard draws it.
const PROJECTS: BarRow[] = [
  {
    key: '/Users/sample/code/agent-router',
    name: 'agent-router',
    concept: 'project',
    path: '/Users/sample/code/agent-router',
    meta: '12 sessions · 840 req · 1.10M out · 92% cached · $41.20 · 3 min ago',
    errors: '3 errors',
    value: 9_000_000,
    sub: 3_000_000,
    figure: '9.00M',
    subFigure: '33% subagents',
  },
  {
    key: '/Users/sample/code/northstar',
    name: 'northstar',
    concept: 'project',
    path: '/Users/sample/code/northstar',
    meta: '4 sessions · 120 req · 300k out · 88% cached · $9.80 · 2 h ago',
    value: 4_500_000,
    figure: '4.50M',
  },
  { key: '(unresolved)', name: '(unresolved)', concept: 'project', meta: '1 session · 2 req', value: 0, figure: '0' },
];

const list = () => screen.getByRole('list', { name: 'Projects' });
const row = (name: string) => screen.getByRole('button', { name });
const barOf = (el: HTMLElement) => el.querySelector(`.${styles.bar}`) as HTMLElement | null;
const figureOf = (el: HTMLElement) => el.querySelector('[data-place]') as HTMLElement;

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe('BarList', () => {
  it('OV-C129: is a list of rows, never a table, each a button named by its row and described by the rest', () => {
    render(<BarList label="Projects" rows={PROJECTS} selected={null} onSelect={() => {}} />);
    expect(screen.queryByRole('table')).toBeNull();
    expect(within(list()).getAllByRole('listitem')).toHaveLength(3);
    expect(row('agent-router')).toHaveAccessibleDescription(
      '/Users/sample/code/agent-router 12 sessions · 840 req · 1.10M out · 92% cached · $41.20 · 3 min ago · 3 errors 9.00M 33% subagents',
    );
  });

  it('OV-C130, OV-C23: marks the scoped row pressed, leaves the others pressable, and hands every press to the caller', () => {
    const onSelect = vi.fn();
    render(<BarList label="Projects" rows={PROJECTS} selected="/Users/sample/code/northstar" onSelect={onSelect} />);
    expect(screen.getByRole('button', { name: 'northstar', pressed: true })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'agent-router', pressed: false })).toBeEnabled();
    expect(list()).toHaveAttribute('data-selection');
    fireEvent.click(row('agent-router'));
    fireEvent.click(row('northstar'));
    expect(onSelect.mock.calls).toEqual([['/Users/sample/code/agent-router'], ['/Users/sample/code/northstar']]);
    // Pressed, with the accent's edge; the others' bars step back while their words keep contrast.
    expect(block(scss, ".row[aria-pressed='true']")).toMatch(/box-shadow:\s*inset \$spacing-02 0 0 var\(--cds-border-interactive\);/);
    expect(block(scss, ".list[data-selection] .row[aria-pressed='false'] .bar")).toMatch(/opacity:/);
  });

  it('draws a list that does not scope the page as plain rows, not buttons', () => {
    render(<BarList label="By model" rows={[{ key: 'claude-opus-4-5', name: 'opus 4.5', value: 10, figure: '10' }]} />);
    expect(screen.queryByRole('button')).toBeNull();
    expect(within(screen.getByRole('list', { name: 'By model' })).getByText('opus 4.5')).toBeVisible();
  });

  it('OV-C191: scales every bar to the largest row, draws a zero as a faint stub, and a missing value as no bar', () => {
    render(
      <BarList
        label="Tools"
        rows={[
          { key: 'a', name: 'Read', value: 200 },
          { key: 'b', name: 'Bash', value: 50 },
          { key: 'c', name: 'Grep', value: 0 },
          { key: 'd', name: 'Glob', value: null },
        ]}
      />,
    );
    const item = (name: string) => screen.getByText(name).closest('li')!;
    expect(barOf(item('Read'))).toHaveStyle({ inlineSize: '100%' });
    expect(barOf(item('Bash'))).toHaveStyle({ inlineSize: '25%' });
    expect(barOf(item('Grep'))).toHaveAttribute('data-zero');
    expect(barOf(item('Glob'))).toBeNull();
    expect(figureOf(item('Glob'))).toHaveTextContent('—');
    expect(block(scss, '\n.bar {')).toMatch(/min-inline-size:\s*3px;/);
  });

  it('OV-C191: never divides by zero when every row is zero', () => {
    render(<BarList label="Tools" rows={[{ key: 'a', name: 'Read', value: 0 }]} />);
    expect(barOf(screen.getByText('Read').closest('li')!)).toHaveStyle({ inlineSize: '0%' });
  });

  it('OV-C192, OV-C135: lays the subagent share over the end of the bar, in the subagent colour, and labels it', () => {
    render(<BarList label="Projects" rows={PROJECTS} onSelect={() => {}} />);
    const bar = barOf(row('agent-router'))!;
    const [main, sub] = [...bar.children] as HTMLElement[];
    expect(main!.style.flexGrow).toBe('6000000');
    expect(sub!.style.flexGrow).toBe('3000000');
    expect(scss).toMatch(/\n\.sub \{\s*background:\s*var\(--ar-subagent\);\s*\}/);
    expect(figureOf(row('agent-router'))).toHaveTextContent('9.00M 33% subagents');
    expect(barOf(row('northstar'))!.children).toHaveLength(1);
  });

  it('OV-C4, OV-C135: draws a project’s folder in the project colour, hidden from a screen reader', () => {
    render(<BarList label="Projects" rows={PROJECTS} onSelect={() => {}} />);
    const icon = row('agent-router').querySelector('svg')!;
    expect(icon.closest('[aria-hidden="true"]')).not.toBeNull();
    expect(icon).toHaveStyle({ fill: 'var(--ar-project)' });
  });

  it('OV-C5: marks errors with the error icon in the error colour, beside the count', () => {
    render(<BarList label="Projects" rows={PROJECTS} onSelect={() => {}} />);
    const errors = within(row('agent-router')).getByText('3 errors');
    expect(errors.querySelector('svg')).toHaveStyle({ fill: 'var(--ar-bad-fill)' });
    expect(block(scss, '\n.errors {')).toMatch(/color:\s*var\(--ar-bad\);/);
  });

  /// A poll redraws the figures and never the reader's place: the row keeps its node, so the focus
  /// stays on it.
  it('OV-C170, SH-C25: keeps the focus on a row through a poll that changes its figures and its rank', () => {
    const { rerender } = render(<BarList label="Projects" rows={PROJECTS} onSelect={() => {}} />);
    act(() => row('northstar').focus());
    const focused = document.activeElement;
    const next = [{ ...PROJECTS[1]!, value: 9_500_000, figure: '9.50M' }, PROJECTS[0]!, PROJECTS[2]!];
    rerender(<BarList label="Projects" rows={next} onSelect={() => {}} />);
    expect(document.activeElement).toBe(focused);
    expect(within(row('northstar')).getByText('9.50M')).toBeInTheDocument();
    // In the order the router gave, which is now northstar first.
    const names = within(list())
      .getAllByRole('button')
      .map((b) => document.getElementById(b.getAttribute('aria-labelledby')!)!.textContent);
    expect(names).toEqual(['northstar', 'agent-router', '(unresolved)']);
  });

  it('OV-C201: shows where the focus is', () => {
    expect(block(scss, 'button.row {')).toMatch(/&:focus-visible\s*\{\s*@include focus-outline\('outline'\);/);
  });
});

describe('BarList’s figure at the bar', () => {
  /// jsdom lays nothing out: the track and the figure are given the widths a browser would.
  function laidOut(track: number, figure: number) {
    vi.spyOn(Element.prototype, 'getBoundingClientRect').mockImplementation(function (this: Element) {
      const width = this.classList.contains(styles.track!) ? track : this.hasAttribute('data-place') ? figure : 0;
      return { left: 0, top: 0, width, height: 16, right: width, bottom: 16, x: 0, y: 0, toJSON: () => ({}) } as DOMRect;
    });
  }
  const rows = (value: number): BarRow[] => [
    { key: 'top', name: 'top', value: 100, figure: '100' },
    { key: 'it', name: 'it', value, figure: `${value}` },
  ];
  const placeOf = () => figureOf(screen.getByText('it').closest('li')!);

  it('OV-C193: sits just past the bar’s end while it fits there', () => {
    laidOut(300, 40);
    render(<BarList label="L" rows={rows(50)} />);
    expect(placeOf()).toHaveAttribute('data-place', 'after');
    expect(placeOf().getAttribute('style')).toBe('inset-inline-start: calc(50% + var(--gap));');
  });

  it('OV-C193, OV-C199: moves inside the bar when it would run past the track and fits inside, on the page’s own surface', () => {
    laidOut(300, 40);
    render(<BarList label="L" rows={rows(95)} />);
    expect(placeOf()).toHaveAttribute('data-place', 'inside');
    expect(placeOf().getAttribute('style')).toBe('inset-inline-end: calc(5% + var(--gap));');
    const inside = block(scss, ".value[data-place='inside']");
    expect(inside).toMatch(/background:\s*var\(--cds-layer-01\);/);
    expect(block(scss, '\n.value {')).toMatch(/color:\s*var\(--cds-text-primary\);/);
  });

  it('OV-C193, SH-C44: takes a line of its own when it fits neither, rather than widen the row', () => {
    laidOut(100, 80);
    render(<BarList label="L" rows={rows(60)} />);
    expect(placeOf()).toHaveAttribute('data-place', 'below');
  });

  it('OV-C193, OV-C172: measures again when the track changes width', () => {
    const watched: ResizeObserverCallback[] = [];
    vi.stubGlobal(
      'ResizeObserver',
      class {
        constructor(cb: ResizeObserverCallback) {
          watched.push(cb);
        }
        observe() {}
        unobserve() {}
        disconnect() {}
      },
    );
    laidOut(300, 40);
    render(<BarList label="L" rows={rows(80)} />);
    expect(placeOf()).toHaveAttribute('data-place', 'after');
    laidOut(120, 40);
    act(() => watched.forEach((cb) => cb([], {} as ResizeObserver)));
    expect(placeOf()).toHaveAttribute('data-place', 'inside');
  });
});

describe('BarList without a hover', () => {
  /// The path was hidden at 700 px and below, and lived in a hover title above; the meta line never
  /// wrapped, which scrolled a phone sideways by 143 px at 360.
  it('SH-C42, SH-C30: says every fact on the row itself, with no title and nothing dropped on a phone', () => {
    stubNarrow();
    const { container } = render(<BarList label="Projects" rows={PROJECTS} onSelect={() => {}} />);
    expect(row('agent-router')).toBeVisible();
    for (const fact of ['agent-router', '/Users/sample/code/agent-router', '12 sessions · 840 req · 1.10M out · 92% cached · $41.20 · 3 min ago', '3 errors', '9.00M', '33% subagents']) {
      expect(row('agent-router')).toHaveTextContent(fact);
    }
    expect(container.querySelectorAll('[title]')).toHaveLength(0);
  });

  it('OV-C195, SH-C44: wraps the meta line and the path on a phone, so nothing scrolls sideways at 360 px', () => {
    const narrow = block(scss, "@include breakpoint-down('md')");
    const meta = block(narrow, '.meta {');
    expect(meta).toMatch(/flex-basis:\s*100%;/);
    expect(meta).toMatch(/white-space:\s*normal;/);
    expect(meta).toMatch(/overflow-wrap:\s*anywhere;/);
    expect(block(scss, '\n.path {')).toMatch(/overflow-wrap:\s*anywhere;/);
    expect(block(scss, '\n.name {')).toMatch(/overflow-wrap:\s*anywhere;/);
    // Nothing in a row holds a width of its own wider than a phone.
    expect(scss).not.toMatch(/min-inline-size:\s*(?!\s|0;|1px;|3px;|\$spacing-01;)[^;]+;/);
    expect(scss).not.toMatch(/(?<!max-)inline-size:\s*calc\(100% - /);
  });
});

describe('BarList on a coarse pointer', () => {
  it('SH-C45: makes every row a 48 px target', () => {
    stubCoarse();
    render(<BarList label="Projects" rows={PROJECTS} onSelect={() => {}} />);
    expect(within(list()).getAllByRole('button')).toHaveLength(3);
    expect(block(block(scss, '@media (pointer: coarse)'), 'button.row {')).toMatch(/min-block-size:\s*\$spacing-09;/);
  });
});

describe('BarList in both themes', () => {
  it('draws the same rows in white and in g100', () => {
    const drawn = (theme: 'white' | 'g100') => {
      const { container, unmount } = render(
        <Theme theme={theme}>
          <BarList label="Projects" rows={PROJECTS} selected="(unresolved)" onSelect={() => {}} />
        </Theme>,
      );
      const wrapper = container.firstElementChild as HTMLElement;
      expect(wrapper).toHaveClass(`cds--${theme}`);
      expect(within(wrapper).getByRole('button', { name: '(unresolved)', pressed: true })).toBeInTheDocument();
      const html = wrapper.innerHTML.replace(/(id|aria-labelledby|aria-describedby)="[^"]*"/g, '');
      unmount();
      return html;
    };
    expect(drawn('g100')).toBe(drawn('white'));
  });

  it('paints only in tokens both themes declare, its words in Carbon’s text tokens and its errors in the error text token', () => {
    const used = [...scss.matchAll(/var\((--ar-[a-z0-9-]+)\)/g)].map((m) => m[1]!);
    for (const mixin of ['ar-light', 'ar-dark']) for (const token of [...used, '--ar-tok', '--ar-project', '--ar-bad-fill']) expect(declares(mixin)).toContain(token);
    expect(scss).not.toMatch(/#[0-9a-f]{3,8}\b|rgba?\(|hsla?\(/i);
    for (const m of scss.matchAll(/(?<![-\w])color:\s*([^;]+);/g)) expect(m[1]).toMatch(/^var\(--(cds-text-(primary|secondary)|ar-bad)\)$/);
  });
});
