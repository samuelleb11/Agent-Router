import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { Theme } from '@carbon/react';
import { fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { stubCoarse } from '~/test-utils';
import tipStyles from './DataTip.module.scss';
import { ShareBar, type Part } from './ShareBar';

const plain = (s: string) => s.replace(/\/\*[\s\S]*?\*\//g, '').replace(/\/\/.*$/gm, '');
const scss = plain(readFileSync(join(__dirname, 'ShareBar.module.scss'), 'utf8'));
const tipScss = plain(readFileSync(join(__dirname, 'DataTip.module.scss'), 'utf8'));
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

/// What the model read, as the dashboard draws it: cache reads, writes and paid in full, on the
/// token ramp, darkest first.
const READ: Part[] = [
  { key: 'read', label: 'from cache', value: 1_200_000, figure: '1.20M', colour: '--ar-tok-3', concept: 'cache' },
  { key: 'write', label: 'written to cache', value: 200_000, figure: '200k', colour: '--ar-tok', detail: '5-min 150k · 1-hour 50k' },
  { key: 'input', label: 'paid in full', value: 100_000, figure: '100k', colour: '--ar-tok-2' },
];
const NAME = 'What the model read: from cache 1.20M, 80%; written to cache 200k, 13%; paid in full 100k, 6.7%';

/// jsdom lays nothing out: each part is given the box a browser would draw it in.
const rect = (left: number, width: number) =>
  ({ left, right: left + width, width, top: 40, bottom: 52, height: 12, x: left, y: 40, toJSON: () => ({}) }) as DOMRect;
let boxes: Record<string, DOMRect> = {};
beforeEach(() => {
  boxes = { read: rect(0, 240), write: rect(242, 40), input: rect(284, 20) };
  vi.spyOn(Element.prototype, 'getBoundingClientRect').mockImplementation(function (this: Element) {
    const key = (this as HTMLElement).dataset?.key;
    return (key && this.matches('span') && boxes[key]) || rect(0, 304);
  });
});
afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

const bar = () => screen.getByRole('img', { name: NAME });
const segment = (key: string) => bar().querySelector(`[data-key="${key}"]`)!;
const row = (label: string) => within(screen.getByRole('list', { name: /, by part$/ })).getByText(label).closest('li')!;
const point = (x: number) => fireEvent.pointerMove(bar(), { clientX: x, clientY: 46, pointerType: 'mouse' });

describe('ShareBar', () => {
  it('is an image named by every part, its figure and its share', () => {
    render(<ShareBar label="What the model read" parts={READ} of="of everything read" />);
    expect(bar()).toBeInTheDocument();
  });

  it('OV-C189: draws its parts in their fixed order, sized by their share, and leaves a part of zero out', () => {
    render(<ShareBar label="Threads" parts={[{ key: 'main', label: 'main thread', value: 300, colour: '--ar-tok' }, { key: 'sub', label: 'subagents', value: 0, colour: '--ar-subagent' }, { key: 'x', label: 'more', value: 100, colour: '--ar-tok-3' }]} />);
    const drawn = [...screen.getByRole('img').querySelectorAll('[data-key]')] as HTMLElement[];
    expect(drawn.map((s) => s.dataset.key)).toEqual(['main', 'x']);
    expect(drawn.map((s) => s.style.flexGrow)).toEqual(['300', '100']);
    // The legend still names the empty part, with its zero.
    expect(within(screen.getByRole('list')).getByText('subagents')).toBeInTheDocument();
    // A part above zero is at least 3 px wide.
    expect(block(scss, '\n.segment {')).toMatch(/min-inline-size:\s*3px;/);
  });

  it('OV-C179, SH-C20: writes a share whole from 10 %, to one decimal below, and "—" with nothing to divide by', () => {
    render(
      <ShareBar
        label="Errors"
        parts={[
          { key: 'bad', label: 'failed', value: 27, colour: '--ar-bad-fill' },
          { key: 'ok', label: 'answered', value: 8_447, colour: '--ar-tok' },
          { key: 'none', label: 'not recorded', value: null, colour: '--ar-tok-2' },
        ]}
      />,
    );
    const pct = (label: string) => row(label).lastElementChild!.textContent;
    expect(pct('failed')).toBe('0.3%');
    expect(pct('answered')).toBe('100%');
    expect(pct('not recorded')).toBe('—');
    expect(row('not recorded').textContent).not.toMatch(/0%/);
  });

  it('OV-C190: lets a part under 24 px own the 24 px around its centre, and points 8 px above and below the bar', () => {
    boxes = { read: rect(0, 200), write: rect(202, 4), input: rect(208, 96) };
    render(<ShareBar label="What the model read" parts={READ} />);
    point(212); // inside "paid in full", but within 12 px of the sliver's centre (204)
    expect(segment('write')).toHaveAttribute('data-hot');
    expect(screen.getByRole('tooltip')).toHaveTextContent('written to cache');
    point(230);
    expect(segment('input')).toHaveAttribute('data-hot');
    expect(segment('write')).not.toHaveAttribute('data-hot');
    expect(block(scss, '&::before')).toMatch(/inset-block:\s*calc\(-1 \* #\{\$spacing-03\}\);/);
  });

  it('OV-C125: lights a part from its legend row, and a part and its row from the part, with its figures', () => {
    render(<ShareBar label="What the model read" parts={READ} of="of everything read" />);
    fireEvent.pointerEnter(row('written to cache'));
    expect(segment('write')).toHaveAttribute('data-hot');
    expect(row('written to cache')).toHaveAttribute('data-hot');
    fireEvent.pointerLeave(row('written to cache'));
    expect(segment('write')).not.toHaveAttribute('data-hot');

    point(100);
    expect(segment('read')).toHaveAttribute('data-hot');
    expect(row('from cache')).toHaveAttribute('data-hot');
    const tip = screen.getByRole('tooltip');
    expect(tip).toHaveTextContent('from cache');
    expect(tip).toHaveTextContent('1.20M');
    expect(tip).toHaveTextContent('80%');
    expect(tip).toHaveTextContent('of everything read');
    expect(block(scss, '.segment[data-hot]')).toMatch(/outline:\s*\$spacing-01 solid var\(--cds-text-primary\);/);
  });

  it('OV-C197: walks its parts from the keyboard, each with its figures', () => {
    render(<ShareBar label="What the model read" parts={READ} />);
    fireEvent.focus(bar());
    expect(screen.getByRole('tooltip')).toHaveTextContent('from cache');
    fireEvent.keyDown(bar(), { key: 'ArrowRight' });
    expect(screen.getByRole('tooltip')).toHaveTextContent('written to cache');
    expect(screen.getByRole('tooltip')).toHaveTextContent('5-min 150k · 1-hour 50k');
    expect(segment('write')).toHaveAttribute('data-hot');
  });
});

describe('ShareBar without a hover', () => {
  /// Whatever a part's tip says, its legend row already says: the tip is a way to it in place,
  /// never the only one.
  it('SH-C30: says every part’s name, figure, share and detail in its legend', () => {
    render(<ShareBar label="What the model read" parts={READ} />);
    const legend = screen.getByRole('list', { name: 'What the model read, by part' });
    expect(within(legend).getAllByRole('listitem').map((li) => li.textContent)).toEqual([
      'from cache 1.20M 80%',
      'written to cache 5-min 150k · 1-hour 50k 200k 13%',
      'paid in full 100k 6.7%',
    ]);
    expect(document.querySelectorAll('[title]')).toHaveLength(0);
  });

  it('draws a part’s concept icon in its colour beside its name, hidden from a screen reader', () => {
    render(<ShareBar label="What the model read" parts={READ} />);
    const icon = row('from cache').querySelector('svg')!;
    expect(icon).toHaveAttribute('aria-hidden', 'true');
    expect(icon).toHaveStyle({ fill: 'var(--ar-tok-3)' });
  });
});

describe('ShareBar on a coarse pointer', () => {
  beforeEach(() => {
    stubCoarse();
  });

  it('OV-C197: pins a part’s figures on a tap', () => {
    render(<ShareBar label="What the model read" parts={READ} />);
    fireEvent.pointerEnter(bar(), { clientX: 290, clientY: 46, pointerType: 'touch' });
    expect(screen.queryByRole('tooltip')).toBeNull();
    fireEvent.click(bar(), { clientX: 290, clientY: 46 });
    expect(screen.getByRole('tooltip')).toHaveTextContent('paid in full');
    expect(segment('input')).toHaveAttribute('data-hot');
    fireEvent.pointerLeave(bar(), { pointerType: 'touch' });
    expect(screen.getByRole('tooltip')).toHaveTextContent('paid in full');
  });

  /// The bar is a DataTip's target, which is a fingertip tall on a touch screen.
  it('SH-C45: is a 48 px target', () => {
    render(<ShareBar label="What the model read" parts={READ} />);
    expect(bar()).toHaveClass(tipStyles.target!);
    expect(block(block(tipScss, '@media (pointer: coarse)'), '.target {')).toMatch(/min-block-size:\s*\$spacing-09;/);
  });
});

describe('ShareBar in both themes', () => {
  it('draws the same bar and legend in white and in g100', () => {
    const drawn = (theme: 'white' | 'g100') => {
      const { container, unmount } = render(
        <Theme theme={theme}>
          <ShareBar label="What the model read" parts={READ} />
        </Theme>,
      );
      const wrapper = container.firstElementChild as HTMLElement;
      expect(wrapper).toHaveClass(`cds--${theme}`);
      expect(within(wrapper).getByRole('img', { name: NAME })).toBeInTheDocument();
      const html = wrapper.innerHTML;
      unmount();
      return html;
    };
    expect(drawn('g100')).toBe(drawn('white'));
  });

  it('paints its parts in their tokens, and everything else in Carbon’s', () => {
    render(<ShareBar label="What the model read" parts={READ} />);
    expect((segment('read') as HTMLElement).style.background).toBe('var(--ar-tok-3)');
    for (const p of READ) for (const mixin of ['ar-light', 'ar-dark']) expect(declares(mixin)).toContain(p.colour);
    expect(scss).not.toMatch(/#[0-9a-f]{3,8}\b|rgba?\(|hsla?\(|--ar-/i);
    for (const m of scss.matchAll(/(?<![-\w])color:\s*([^;]+);/g)) expect(m[1]).toMatch(/^var\(--cds-text-(primary|secondary)\)$/);
  });
});
