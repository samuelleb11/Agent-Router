import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { Theme } from '@carbon/react';
import { act, fireEvent, render, screen, within } from '@testing-library/react';
import { useState } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { stubCoarse } from '~/test-utils';
import { DataTip, type Tip } from './DataTip';

const plain = (s: string) => s.replace(/\/\*[\s\S]*?\*\//g, '').replace(/\/\/.*$/gm, '');
const scss = plain(readFileSync(join(__dirname, 'DataTip.module.scss'), 'utf8'));
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

/// jsdom lays nothing out, so the target and the tip are given the boxes a browser would.
const rect = (left: number, top: number, width: number, height: number) =>
  ({ left, top, width, height, right: left + width, bottom: top + height, x: left, y: top, toJSON: () => ({}) }) as DOMRect;
let targetBox = rect(100, 40, 300, 20);
let tipBox = { width: 200, height: 60 };
beforeEach(() => {
  targetBox = rect(100, 40, 300, 20);
  tipBox = { width: 200, height: 60 };
  vi.spyOn(Element.prototype, 'getBoundingClientRect').mockImplementation(function (this: Element) {
    return this.getAttribute('role') === 'tooltip' ? rect(0, 0, tipBox.width, tipBox.height) : targetBox;
  });
});
afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

const SPEND: Tip = {
  title: 'Spend',
  rows: [
    { value: '$12.40', label: 'cache reads', colour: '--ar-tok-3' },
    { value: '$3.10', label: 'output', colour: '--ar-tok', mark: 'line' },
    { value: '91%', label: 'of the prompt from cache' },
  ],
};

const show = (tip: Tip | null = SPEND) =>
  render(
    <DataTip label="Spend this week" tip={() => tip}>
      <span>$15.50</span>
    </DataTip>,
  );
const target = () => screen.getByRole('img', { name: 'Spend this week' });
const tip = () => screen.getByRole('tooltip');
const at = () => ({ left: tip().style.left, top: tip().style.top });

describe('DataTip', () => {
  it('is a target named in words, which the keyboard can reach', () => {
    show();
    expect(target()).toHaveAttribute('tabindex', '0');
    expect(screen.queryByRole('tooltip')).toBeNull();
  });

  it('SH-C31: opens 12 px right of and 14 px below the pointer, and follows it', () => {
    show();
    fireEvent.pointerEnter(target(), { clientX: 150, clientY: 50, pointerType: 'mouse' });
    expect(at()).toEqual({ left: '162px', top: '64px' });
    fireEvent.pointerMove(target(), { clientX: 180, clientY: 52, pointerType: 'mouse' });
    expect(at()).toEqual({ left: '192px', top: '66px' });
  });

  it('SH-C31: flips to the pointer’s other side rather than cross the window’s edge, and keeps 8 px from every edge', () => {
    show();
    fireEvent.pointerEnter(target(), { clientX: 1000, clientY: 740, pointerType: 'mouse' });
    // 1024 × 768: right of 1000 a 200 px tip would cross the edge, and below 740 a 60 px one.
    expect(at()).toEqual({ left: `${1000 - 200 - 12}px`, top: `${740 - 60 - 12}px` });
    tipBox = { width: 1100, height: 900 };
    fireEvent.pointerMove(target(), { clientX: 20, clientY: 20, pointerType: 'mouse' });
    expect(at()).toEqual({ left: '8px', top: '8px' });
  });

  it('SH-C32: closes when the pointer leaves, and on any scroll', () => {
    show();
    fireEvent.pointerEnter(target(), { clientX: 150, clientY: 50, pointerType: 'mouse' });
    fireEvent.pointerLeave(target(), { pointerType: 'mouse' });
    expect(screen.queryByRole('tooltip')).toBeNull();

    fireEvent.pointerEnter(target(), { clientX: 150, clientY: 50, pointerType: 'mouse' });
    fireEvent.scroll(window);
    expect(screen.queryByRole('tooltip')).toBeNull();
  });

  it('SH-C32: takes no pointer events, so it never steals the hover from what it explains', () => {
    expect(block(scss, '\n.tip {')).toMatch(/pointer-events:\s*none;/);
  });

  /// A tip built when it opened would show an older poll's numbers.
  it('SH-C32: shows the latest figures while it is open', () => {
    const { rerender } = show();
    fireEvent.focus(target());
    expect(tip()).toHaveTextContent('$12.40');
    rerender(
      <DataTip label="Spend this week" tip={() => ({ title: 'Spend', rows: [{ value: '$12.90', label: 'cache reads' }] })}>
        <span>$16.00</span>
      </DataTip>,
    );
    expect(tip()).toHaveTextContent('$12.90');
    expect(tip()).not.toHaveTextContent('$12.40');
  });

  it('SH-C33, SH-C30: shows the same tip on focus, anchored under the target, and is described by it', () => {
    show();
    fireEvent.focus(target());
    // Under the target's bottom-left corner: (100, 60) → (112, 74).
    expect(at()).toEqual({ left: '112px', top: '74px' });
    expect(target()).toHaveAccessibleDescription(/Spend.*\$12\.40.*cache reads.*\$3\.10.*output.*91%.*of the prompt from cache/);
    fireEvent.blur(target());
    expect(screen.queryByRole('tooltip')).toBeNull();
  });

  it('SH-C34: pairs each figure with its label and a key in its series’ colour and mark', () => {
    show();
    fireEvent.focus(target());
    const rows = within(tip()).getAllByText(/\$|%/).map((v) => v.parentElement!);
    const keys = rows.map((r) => r.querySelector('[aria-hidden]')!);
    expect(keys.map((k) => k.getAttribute('data-mark'))).toEqual(['square', 'line', 'none']);
    expect(keys[0]).toHaveStyle({ background: 'var(--ar-tok-3)' });
    expect(keys[1]).toHaveStyle({ background: 'var(--ar-tok)' });
    expect(rows.map((r) => r.textContent)).toEqual(['$12.40 cache reads', '$3.10 output', '91% of the prompt from cache']);
    // A row with no series keeps its column, so the figures line up.
    expect(block(scss, ".key[data-mark='none']")).toMatch(/visibility:\s*hidden;/);
  });

  it('shows nothing for a part with nothing to say', () => {
    show(null);
    fireEvent.focus(target());
    expect(screen.queryByRole('tooltip')).toBeNull();
    expect(target()).not.toHaveAttribute('aria-describedby');
  });

  it('pins on a click and on Enter, and lets go on a second, on Escape, or on a press elsewhere', () => {
    render(
      <>
        <DataTip label="Spend this week" tip={() => SPEND}>
          <span>$15.50</span>
        </DataTip>
        <button type="button">elsewhere</button>
      </>,
    );
    fireEvent.click(target());
    expect(target()).toHaveAttribute('data-tip', 'pinned');
    fireEvent.pointerLeave(target(), { pointerType: 'mouse' });
    fireEvent.blur(target());
    expect(tip()).toBeInTheDocument();
    fireEvent.click(target());
    expect(screen.queryByRole('tooltip')).toBeNull();

    fireEvent.keyDown(target(), { key: 'Enter' });
    expect(target()).toHaveAttribute('data-tip', 'pinned');
    fireEvent.keyDown(document, { key: 'Escape' });
    expect(screen.queryByRole('tooltip')).toBeNull();

    fireEvent.click(target());
    fireEvent.pointerDown(screen.getByRole('button', { name: 'elsewhere' }));
    expect(screen.queryByRole('tooltip')).toBeNull();
  });
});

/// A target in parts: the arrow keys walk them, as the pointer does, and the owner lights the one
/// the tip is about.
function Parts() {
  const [lit, setLit] = useState<string | null>(null);
  return (
    <>
      <DataTip
        label="Three parts"
        keys={['a', 'b', 'c']}
        locate={(x) => (x < 200 ? 'a' : x < 300 ? 'b' : 'c')}
        onActive={setLit}
        tip={(part) => (part ? { title: `part ${part}` } : { title: 'all parts' })}
      >
        <span>parts</span>
      </DataTip>
      <output aria-label="lit">{lit ?? 'none'}</output>
    </>
  );
}

describe('DataTip over a target in parts', () => {
  const lit = () => screen.getByRole('status', { name: 'lit' });
  const parts = () => screen.getByRole('img', { name: 'Three parts' });

  it('says which part is under the pointer, and lets the owner light it', () => {
    render(<Parts />);
    fireEvent.pointerEnter(parts(), { clientX: 250, clientY: 50, pointerType: 'mouse' });
    expect(tip()).toHaveTextContent('part b');
    expect(lit()).toHaveTextContent('b');
    fireEvent.pointerLeave(parts(), { pointerType: 'mouse' });
    expect(lit()).toHaveTextContent('none');
  });

  it('OV-C116, OV-C197: walks the parts from the keyboard, Left and Right wrapping, Home and End at the ends', () => {
    render(<Parts />);
    fireEvent.focus(parts());
    expect(tip()).toHaveTextContent('part a');
    fireEvent.keyDown(parts(), { key: 'ArrowRight' });
    expect(tip()).toHaveTextContent('part b');
    expect(lit()).toHaveTextContent('b');
    fireEvent.keyDown(parts(), { key: 'End' });
    expect(tip()).toHaveTextContent('part c');
    fireEvent.keyDown(parts(), { key: 'ArrowRight' });
    expect(tip()).toHaveTextContent('part a');
    fireEvent.keyDown(parts(), { key: 'ArrowLeft' });
    expect(tip()).toHaveTextContent('part c');
    fireEvent.keyDown(parts(), { key: 'Escape' });
    expect(screen.queryByRole('tooltip')).toBeNull();
    expect(lit()).toHaveTextContent('none');
  });
});

describe('DataTip on a coarse pointer', () => {
  const lit = () => screen.getByRole('status', { name: 'lit' });
  const parts = () => screen.getByRole('img', { name: 'Three parts' });

  beforeEach(() => {
    stubCoarse();
  });

  /// A phone has no hover: what the tip says reaches it by a tap, which pins it.
  it('OV-C197, SH-C30: pins on a tap, holds through the finger lifting, and lets go on a second tap', () => {
    render(<Parts />);
    fireEvent.pointerEnter(parts(), { clientX: 320, clientY: 50, pointerType: 'touch' });
    expect(screen.queryByRole('tooltip')).toBeNull();
    fireEvent.click(parts(), { clientX: 320, clientY: 50 });
    expect(tip()).toHaveTextContent('part c');
    expect(lit()).toHaveTextContent('c');
    fireEvent.pointerLeave(parts(), { pointerType: 'touch' });
    expect(tip()).toHaveTextContent('part c');
    // A tap on another part moves the pin there; a tap on the same part lets go.
    fireEvent.click(parts(), { clientX: 120, clientY: 50 });
    expect(tip()).toHaveTextContent('part a');
    fireEvent.click(parts(), { clientX: 130, clientY: 50 });
    expect(screen.queryByRole('tooltip')).toBeNull();
  });

  it('SH-C45: is a fingertip tall', () => {
    expect(block(block(scss, '@media (pointer: coarse)'), '.target {')).toMatch(/min-block-size:\s*\$spacing-09;/);
  });
});

describe('DataTip in both themes', () => {
  it('draws the same target in white and in g100, and its tip on the page’s own surface', () => {
    const drawn = (theme: 'white' | 'g100') => {
      const { container, unmount } = render(
        <Theme theme={theme}>
          <DataTip label="Spend this week" tip={() => SPEND}>
            <span>$15.50</span>
          </DataTip>
        </Theme>,
      );
      const wrapper = container.firstElementChild as HTMLElement;
      expect(wrapper).toHaveClass(`cds--${theme}`);
      act(() => within(wrapper).getByRole('img', { name: 'Spend this week' }).focus());
      const html = wrapper.innerHTML.replace(/aria-describedby="[^"]*"/g, '') + tip().innerHTML;
      unmount();
      return html;
    };
    expect(drawn('g100')).toBe(drawn('white'));
  });

  /// The keys are --ar-* colours, validated on $background and $layer-01: on an inverse surface a
  /// series' key would vanish, so the tip is drawn on the layer, in Carbon's text tokens.
  it('is drawn in Carbon’s tokens only, on $layer-01', () => {
    const tipRule = block(scss, '\n.tip {');
    expect(tipRule).toMatch(/background:\s*var\(--cds-layer-01\);/);
    expect(tipRule).toMatch(/color:\s*var\(--cds-text-primary\);/);
    expect(scss).not.toMatch(/inverse/);
    expect(scss).not.toMatch(/#[0-9a-f]{3,8}\b|rgba?\(|hsla?\(/i);
    for (const m of scss.matchAll(/(?<![-\w])color:\s*([^;]+);/g)) expect(m[1]).toMatch(/^var\(--cds-text-(primary|secondary)\)$/);
  });
});
