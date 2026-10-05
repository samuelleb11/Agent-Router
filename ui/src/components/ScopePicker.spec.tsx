import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { Theme } from '@carbon/react';
import { act, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { stubCoarse } from '~/test-utils';
import { ScopePicker, type ScopeOption } from './ScopePicker';

const plain = (s: string) => s.replace(/\/\*[\s\S]*?\*\//g, '').replace(/\/\/.*$/gm, '');
const scss = plain(readFileSync(join(__dirname, 'ScopePicker.module.scss'), 'utf8'));
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

const ALL: ScopeOption = { key: '*', name: 'All projects', meta: '1,204 req' };
const project = (name: string, i: number): ScopeOption => ({
  key: `/Users/sam/code/${name}`,
  name,
  sub: `/Users/sam/code/${name}`,
  path: true,
  meta: `${i}.0M · ${i * 10} req · $${i}.00`,
});
const NAMES = ['agent-router', 'northstar', 'cerealkiller', 'dragonfly', 'jade', 'mamoru', 'nami', 'grace', 'cosmos', 'taida'];
const MANY = [ALL, ...NAMES.map((n, i) => project(n, i + 1))];
const FEW = [ALL, project('agent-router', 1), project('northstar', 2)];

function show(options = MANY, value = '*', onPick = vi.fn()) {
  const view = render(<ScopePicker level="project" crumb={value === '*' ? 'All projects' : 'northstar'} set={value !== '*'} value={value} options={options} onPick={onPick} />);
  return { ...view, onPick };
}
const crumb = () => screen.getByRole('button', { name: /^Project: / });
const listbox = () => screen.getByRole('listbox', { name: 'Projects' });
const option = (name: string | RegExp) => within(listbox()).getByRole('option', { name });
const filter = () => screen.getByRole('searchbox', { name: 'Filter projects' });
const open = () => fireEvent.click(crumb());

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('ScopePicker', () => {
  it('OV-C14: is a crumb named by its level and its choice, that opens a listbox', () => {
    show();
    expect(crumb()).toHaveAccessibleName('Project: All projects');
    expect(crumb()).toHaveAttribute('aria-haspopup', 'listbox');
    expect(crumb()).toHaveAttribute('aria-expanded', 'false');
    expect(screen.queryByRole('listbox')).toBeNull();
    open();
    expect(crumb()).toHaveAttribute('aria-expanded', 'true');
    expect(crumb()).toHaveAttribute('aria-controls', listbox().id);
  });

  it('OV-C4, OV-C12: wears its level’s icon in its colour, and reads strong once its level is set', () => {
    const { unmount } = show();
    expect(crumb()).not.toHaveAttribute('data-set');
    expect(crumb().querySelector('svg')).toHaveStyle({ fill: 'var(--ar-project)' });
    unmount();
    show(MANY, '/Users/sam/code/northstar');
    expect(crumb()).toHaveAccessibleName('Project: northstar');
    expect(crumb()).toHaveAttribute('data-set');
    expect(block(scss, '.crumb[data-set]')).toMatch(/color:\s*var\(--cds-text-primary\);[\s\S]*font-weight:\s*600;/);
  });

  /// A menu cannot hold a text field, so the dashboard's `role=menu` with an input inside was
  /// announced wrongly.
  it('OV-C21: is a filter field above a listbox of options, never a menu with a field inside', () => {
    show();
    open();
    expect(screen.queryByRole('menu')).toBeNull();
    expect(screen.queryByRole('menuitem')).toBeNull();
    expect(within(listbox()).getAllByRole('option')).toHaveLength(11);
    expect(listbox().contains(filter())).toBe(false);
  });

  it('OV-C16: lists "All …" first, then a row each with its name, second line and figures, the current one selected', () => {
    show(MANY, '/Users/sam/code/northstar');
    open();
    const rows = within(listbox()).getAllByRole('option');
    expect(rows[0]).toHaveAccessibleName('All projects 1,204 req');
    expect(rows[2]).toHaveAccessibleName('northstar /Users/sam/code/northstar 2.0M · 20 req · $2.00');
    expect(rows.filter((r) => r.getAttribute('aria-selected') === 'true')).toEqual([rows[2]]);
  });

  it('OV-C16: never shrinks a name, and lets a path give up its head rather than its tail', () => {
    show();
    open();
    expect(option(/^agent-router/).querySelector('[data-path]')).toHaveTextContent('/Users/sam/code/agent-router');
    expect(block(scss, '\n.name {')).toMatch(/white-space:\s*nowrap;/);
    expect(block(scss, '\n.sub {')).toMatch(/text-overflow:\s*ellipsis;/);
    expect(block(scss, '.sub[data-path]')).toMatch(/direction:\s*rtl;/);
  });

  it('OV-C18: opens with a filter above eight rows, named for what it filters, matching name and path in any case', () => {
    show();
    open();
    expect(filter()).toHaveAttribute('placeholder', 'Filter projects…');
    fireEvent.change(filter(), { target: { value: 'CODE/NA' } });
    expect(within(listbox()).getAllByRole('option').map((o) => o.getAttribute('data-key'))).toEqual(['/Users/sam/code/nami']);
    fireEvent.change(filter(), { target: { value: 'zzz' } });
    expect(screen.queryByRole('listbox')).toBeNull();
    expect(screen.getByText('Nothing matches.')).toBeVisible();
  });

  it('OV-C18: has no filter at eight rows or fewer', () => {
    show(FEW);
    open();
    expect(screen.queryByRole('searchbox')).toBeNull();
  });

  it('OV-C19: puts the focus in the filter, else on the current row, else on the first', () => {
    const many = show();
    open();
    expect(filter()).toHaveFocus();
    many.unmount();

    const current = show(FEW, '/Users/sam/code/northstar');
    open();
    expect(option(/^northstar/)).toHaveFocus();
    current.unmount();

    show(FEW, 'gone');
    open();
    expect(option(/^All projects/)).toHaveFocus();
  });

  it('OV-C19: moves through the rows with Up and Down, wrapping at both ends, from the filter in', () => {
    show();
    open();
    fireEvent.keyDown(filter(), { key: 'ArrowDown' });
    expect(option(/^All projects/)).toHaveFocus();
    fireEvent.keyDown(document.activeElement!, { key: 'ArrowUp' });
    expect(option(/^taida/)).toHaveFocus();
    fireEvent.keyDown(document.activeElement!, { key: 'ArrowDown' });
    expect(option(/^All projects/)).toHaveFocus();
    fireEvent.keyDown(document.activeElement!, { key: 'End' });
    expect(option(/^taida/)).toHaveFocus();
    // One tab stop: only the row the keyboard is on.
    expect(within(listbox()).getAllByRole('option').filter((o) => o.tabIndex === 0)).toEqual([option(/^taida/)]);
  });

  it('OV-C19: closes on Escape from a row and from the filter, which Carbon’s Search would otherwise swallow, and gives the focus back', () => {
    show();
    open();
    fireEvent.keyDown(filter(), { key: 'Escape' });
    expect(screen.queryByRole('listbox')).toBeNull();
    expect(crumb()).toHaveFocus();

    open();
    fireEvent.keyDown(filter(), { key: 'ArrowDown' });
    fireEvent.keyDown(document.activeElement!, { key: 'Escape' });
    expect(screen.queryByRole('listbox')).toBeNull();
    expect(crumb()).toHaveFocus();
  });

  it('OV-C19: closes on a second press of its crumb, a press outside and a window resize', () => {
    show();
    open();
    open();
    expect(screen.queryByRole('listbox')).toBeNull();
    open();
    fireEvent.click(document.body);
    expect(screen.queryByRole('listbox')).toBeNull();
    open();
    act(() => {
      window.dispatchEvent(new Event('resize'));
    });
    expect(screen.queryByRole('listbox')).toBeNull();
  });

  it('chooses a row by a press, by Enter on it, or by Enter in the filter, and gives the focus back to the crumb', () => {
    const { onPick } = show();
    open();
    fireEvent.click(option(/^jade/));
    expect(onPick).toHaveBeenLastCalledWith('/Users/sam/code/jade');
    expect(screen.queryByRole('listbox')).toBeNull();
    expect(crumb()).toHaveFocus();

    open();
    fireEvent.keyDown(filter(), { key: 'ArrowDown' });
    fireEvent.keyDown(document.activeElement!, { key: 'ArrowDown' });
    fireEvent.keyDown(document.activeElement!, { key: 'Enter' });
    expect(onPick).toHaveBeenLastCalledWith('/Users/sam/code/agent-router');

    open();
    fireEvent.change(filter(), { target: { value: 'grace' } });
    fireEvent.keyDown(filter(), { key: 'Enter' });
    expect(onPick).toHaveBeenLastCalledWith('/Users/sam/code/grace');
  });

  /// "Rebuilt only when the scope changes, so an open menu is not destroyed by the five-second poll."
  it('OV-C15: keeps an open picker, and the row the keyboard is on, through a poll’s new rows', () => {
    const { rerender, onPick } = show(FEW);
    open();
    fireEvent.keyDown(document.activeElement!, { key: 'ArrowDown' });
    const focused = document.activeElement;
    expect(focused).toBe(option(/^agent-router/));
    const polled = [ALL, { ...project('agent-router', 1), meta: '1.1M · 12 req · $1.10' }, project('northstar', 2)];
    rerender(<ScopePicker level="project" crumb="All projects" set={false} value="*" options={polled} onPick={onPick} />);
    expect(listbox()).toBeInTheDocument();
    expect(document.activeElement).toBe(focused);
    expect(option(/^agent-router/)).toHaveAccessibleName(/1\.1M · 12 req · \$1\.10$/);
  });

  it('OV-C33: closes when the scope is set from elsewhere, without moving the focus', () => {
    const { rerender } = render(
      <>
        <ScopePicker level="project" crumb="All projects" set={false} value="*" options={FEW} onPick={vi.fn()} />
        <button type="button">a list row</button>
      </>,
    );
    open();
    const row = screen.getByRole('button', { name: 'a list row' });
    act(() => row.focus());
    rerender(
      <>
        <ScopePicker level="project" crumb="northstar" set value="/Users/sam/code/northstar" options={FEW} onPick={vi.fn()} />
        <button type="button">a list row</button>
      </>,
    );
    expect(screen.queryByRole('listbox')).toBeNull();
    expect(row).toHaveFocus();
  });

  it('OV-C34: says what its list leaves out, where it is read', () => {
    render(<ScopePicker level="session" crumb="All sessions" set={false} value="*" options={FEW} note="the 200 most recent sessions" onPick={vi.fn()} />);
    fireEvent.click(screen.getByRole('button', { name: 'Session: All sessions' }));
    expect(screen.getByText('the 200 most recent sessions')).toBeVisible();
    expect(screen.getByRole('listbox', { name: 'Sessions' })).toBeInTheDocument();
  });

  it('OV-C20: is at most 60 % of the window and 420 px tall, scrolling within', () => {
    expect(scss).toMatch(/\$ar-picker-max-block:\s*26\.25rem;/);
    expect(26.25 * 16).toBe(420);
    const panel = block(scss, '\n.panel {');
    expect(panel).toMatch(/max-block-size:\s*min\(60vh, #\{\$ar-picker-max-block\}\);/);
    expect(panel).toMatch(/overflow-y:\s*auto;/);
  });
});

describe('ScopePicker without a hover', () => {
  /// The whole path is in each row's name, comes out whole on the row the keyboard is on, and is
  /// always whole on a touch screen.
  it('SH-C30: gives each row’s whole path to the keyboard and to a screen reader', () => {
    show();
    open();
    expect(option(/^cerealkiller/)).toHaveAccessibleName(/\/Users\/sam\/code\/cerealkiller/);
    expect(block(scss, '.option:focus-visible .sub')).toMatch(/white-space:\s*normal;/);
    // The only title is Carbon's own on the filter's clear button, and it repeats that button's name.
    for (const el of document.querySelectorAll('[title]')) expect(el).toHaveAccessibleName(el.getAttribute('title')!);
  });
});

describe('ScopePicker on a coarse pointer', () => {
  it('SH-C45: makes the crumb and every row 48 px, the filter a large field with 16 px text, and every path whole', () => {
    stubCoarse();
    show();
    open();
    // Carbon sizes its Search by the layout class, which styles.scss's layout partial defines: 48 px.
    expect(filter().closest('.cds--search')).toHaveClass('cds--layout--size-lg');
    const coarse = block(scss, '@media (pointer: coarse)');
    expect(block(coarse, '.crumb,\n  .option {')).toMatch(/min-block-size:\s*\$spacing-09;/);
    expect(block(coarse, '.panel :global(.cds--search-input)')).toMatch(/@include type-style\('body-compact-02'\);/);
    expect(block(coarse, '.sub {')).toMatch(/white-space:\s*normal;/);
  });

  it('keeps the filter small on a fine pointer', () => {
    show();
    open();
    expect(filter().closest('.cds--search')).toHaveClass('cds--layout--size-sm');
  });
});

describe('ScopePicker in both themes', () => {
  it('draws the same crumb and list in white and in g100', () => {
    const drawn = (theme: 'white' | 'g100') => {
      const { container, unmount } = render(
        <Theme theme={theme}>
          <ScopePicker level="project" crumb="All projects" set={false} value="*" options={FEW} onPick={vi.fn()} />
        </Theme>,
      );
      const wrapper = container.firstElementChild as HTMLElement;
      expect(wrapper).toHaveClass(`cds--${theme}`);
      fireEvent.click(within(wrapper).getByRole('button', { name: 'Project: All projects' }));
      expect(within(wrapper).getByRole('listbox', { name: 'Projects' })).toBeInTheDocument();
      const html = wrapper.innerHTML.replace(/(id|aria-controls)="[^"]*"/g, '').replace(/style="[^"]*"/g, '');
      unmount();
      return html;
    };
    expect(drawn('g100')).toBe(drawn('white'));
  });

  it('is drawn in Carbon’s tokens, with only its level’s colour of its own', () => {
    for (const mixin of ['ar-light', 'ar-dark']) expect(declares(mixin)).toContain('--ar-project');
    expect(scss).not.toMatch(/#[0-9a-f]{3,8}\b|rgba?\(|hsla?\(|--ar-/i);
    for (const m of scss.matchAll(/(?<![-\w])color:\s*([^;]+);/g)) expect(m[1]).toMatch(/^var\(--cds-text-(primary|secondary)\)$/);
  });
});
