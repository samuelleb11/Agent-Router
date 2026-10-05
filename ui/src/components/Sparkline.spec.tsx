import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { Theme } from '@carbon/react';
import { fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { stubCoarse } from '~/test-utils';
import tipStyles from './DataTip.module.scss';
import { Sparkline, type Series } from './Sparkline';

const plain = (s: string) => s.replace(/\/\*[\s\S]*?\*\//g, '').replace(/\/\/.*$/gm, '');
const scss = plain(readFileSync(join(__dirname, 'Sparkline.module.scss'), 'utf8'));
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

const HOURS = ['09:00', '10:00', '11:00', '12:00', '13:00', '14:00'];
const k = (v: number) => (v >= 1000 ? `${(v / 1000).toFixed(1)}k` : String(v));
const TOKENS: Series[] = [{ key: 'prompt', label: 'prompt tokens', values: [5_300, 0, null, 12, 2_000, 1_200], colour: '--ar-tok' }];
const QUOTA: Series[] = [
  { key: '5h', label: '5-hour used', values: [10, 30, null, 55, 61, null], colour: '--cds-interactive' },
  { key: '7d', label: '7-day used', values: [40, 41, 42, 43, 44, 45], colour: '--ar-tok' },
];
const pct = (v: number) => `${Math.round(v)}%`;

/// jsdom lays nothing out: the strip is given the box a browser would, 600 px wide.
beforeEach(() => {
  vi.spyOn(Element.prototype, 'getBoundingClientRect').mockImplementation(
    () => ({ left: 0, top: 0, width: 600, height: 68, right: 600, bottom: 68, x: 0, y: 0, toJSON: () => ({}) }) as DOMRect,
  );
});
afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

const tokens = () => render(<Sparkline label="Prompt tokens per hour" buckets={HOURS} series={TOKENS} format={k} />);
const quota = () =>
  render(<Sparkline label="Quota history" kind="lines" buckets={HOURS} series={QUOTA} max={100} rules={[100, 90, 0]} format={pct} ends legend="percent used, over the window charted below" />);
const strip = (name: RegExp) => screen.getByRole('img', { name });
const bars = (container: HTMLElement) => [...container.querySelectorAll('rect')];

describe('Sparkline', () => {
  it('is an image named by its span, its latest figure and its peak', () => {
    tokens();
    expect(strip(/^Prompt tokens per hour, 6 buckets from 09:00 to 14:00; prompt tokens: latest 1\.2k, peak 5\.3k$/)).toBeInTheDocument();
  });

  it('OV-C194: draws one bar per bucket scaled to the largest, a pixel at least above zero, and an empty bucket blank', () => {
    const { container } = tokens();
    const drawn = bars(container);
    // 0 and null draw nothing; 5,300 is the full height; 12 of 5,300 is still a pixel of the 68.
    expect(drawn.map((r) => r.getAttribute('data-bucket'))).toEqual(['0', '3', '4', '5']);
    expect(Number(drawn[0]!.getAttribute('height'))).toBe(100);
    expect(Number(drawn[1]!.getAttribute('height'))).toBeCloseTo(100 / 68, 6);
    expect(Number(drawn[2]!.getAttribute('height'))).toBeCloseTo((100 * 2000) / 5300, 6);
    expect(container.querySelectorAll('line')).toHaveLength(1); // the baseline
  });

  it('OV-C194: marks the last bucket, the one still filling, in the darker step', () => {
    const { container } = tokens();
    expect(bars(container).filter((r) => r.hasAttribute('data-last')).map((r) => r.getAttribute('data-bucket'))).toEqual(['5']);
    expect(block(scss, '\n.bar {')).toMatch(/fill:\s*var\(--ar-tok\);/);
    expect(block(scss, '.bar[data-last]')).toMatch(/fill:\s*var\(--ar-tok-3\);/);
  });

  it('is a 68 px strip, so a pixel is a sixty-eighth of it', () => {
    expect(block(scss, '\n.plot {')).toMatch(/block-size:\s*4\.25rem;/);
    expect(4.25 * 16).toBe(68);
  });

  it('OV-C60: draws a line per series with rules at 100, 90 and 0, and marks each line’s latest point', () => {
    const { container } = quota();
    expect(container.querySelectorAll('path')).toHaveLength(2);
    expect([...container.querySelectorAll('line')].map((l) => Number(l.getAttribute('y1')))).toEqual([0, 10, 100]);
    const marks = [...container.querySelectorAll('[data-series]')].filter((e) => e.tagName === 'SPAN') as HTMLElement[];
    expect(marks.map((m) => m.dataset.series)).toEqual(['5h', '7d']);
    // The 7-day line ends at its last bucket, 45 % used.
    expect(marks[1]!.getAttribute('style')).toContain('inset-inline-start: 91.66666666666666%');
    expect(marks[1]!.getAttribute('style')).toContain('inset-block-start: 55%');
  });

  /// A window with no reading in a bucket is left off its line, and its end marker sits on its last
  /// reading, never on the 0 line.
  it('OV-C61: leaves a bucket with no reading off its line, and marks the last reading, never 0', () => {
    const { container } = quota();
    const five = container.querySelector('path[data-series="5h"]')!.getAttribute('d')!;
    expect(five.match(/M/g)).toHaveLength(2);
    const mark = [...container.querySelectorAll('span[data-series="5h"]')][0] as HTMLElement;
    expect(mark.getAttribute('style')).toContain('inset-inline-start: 75%');
    expect(mark.getAttribute('style')).toContain('inset-block-start: 39%');
  });

  it('OV-C60, OV-C61: gives the pointed bucket’s time and every series’ figure, "—" where there is none', () => {
    quota();
    fireEvent.pointerMove(strip(/^Quota history/), { clientX: 550, clientY: 30, pointerType: 'mouse' });
    const tip = screen.getByRole('tooltip');
    expect(tip).toHaveTextContent('14:00');
    expect(tip).toHaveTextContent('— 5-hour used');
    expect(tip).toHaveTextContent('45% 7-day used');
    expect(tip).not.toHaveTextContent('0%');
  });
});

describe('Sparkline without a hover', () => {
  it('OV-C116, OV-C197: opens on the latest bucket from the keyboard and walks back through the others', () => {
    const { container } = tokens();
    const s = strip(/^Prompt tokens per hour/);
    fireEvent.focus(s);
    expect(screen.getByRole('tooltip')).toHaveTextContent('14:00');
    expect(screen.getByRole('tooltip')).toHaveTextContent('1.2k prompt tokens');
    fireEvent.keyDown(s, { key: 'ArrowLeft' });
    expect(screen.getByRole('tooltip')).toHaveTextContent('13:00');
    expect(screen.getByRole('tooltip')).toHaveTextContent('2.0k prompt tokens');
    // A crosshair marks the bucket the tip is about.
    expect(container.querySelector(`line[x1="4.5"]`)).not.toBeNull();
  });

  it('OV-C60, SH-C30: says the first and last buckets’ times and what each line is, as text', () => {
    quota();
    for (const text of ['09:00', '14:00', '5-hour used', '7-day used', 'percent used, over the window charted below']) expect(screen.getByText(text)).toBeVisible();
  });
});

describe('Sparkline on a coarse pointer', () => {
  beforeEach(() => {
    stubCoarse();
  });

  it('OV-C197: pins a bucket’s figures on a tap', () => {
    tokens();
    const s = strip(/^Prompt tokens per hour/);
    fireEvent.click(s, { clientX: 10, clientY: 30 });
    expect(screen.getByRole('tooltip')).toHaveTextContent('09:00');
    expect(screen.getByRole('tooltip')).toHaveTextContent('5.3k');
    fireEvent.pointerLeave(s, { pointerType: 'touch' });
    expect(screen.getByRole('tooltip')).toHaveTextContent('09:00');
  });

  it('SH-C45: is a target taller than a fingertip', () => {
    tokens();
    expect(strip(/^Prompt tokens per hour/)).toHaveClass(tipStyles.target!);
    // 68 px of strip, over the 48 px a fingertip needs.
    expect(4.25).toBeGreaterThanOrEqual(3);
  });
});

describe('Sparkline in both themes', () => {
  it('draws the same strip in white and in g100', () => {
    const drawn = (theme: 'white' | 'g100') => {
      const { container, unmount } = render(
        <Theme theme={theme}>
          <Sparkline label="Quota history" kind="lines" buckets={HOURS} series={QUOTA} max={100} rules={[100, 90, 0]} format={pct} ends />
        </Theme>,
      );
      const wrapper = container.firstElementChild as HTMLElement;
      expect(wrapper).toHaveClass(`cds--${theme}`);
      expect(within(wrapper).getByRole('img', { name: /^Quota history/ })).toBeInTheDocument();
      const html = wrapper.innerHTML;
      unmount();
      return html;
    };
    expect(drawn('g100')).toBe(drawn('white'));
  });

  it('paints only in tokens both themes declare, and its words in Carbon’s text tokens', () => {
    const used = [...scss.matchAll(/var\((--ar-[a-z0-9-]+)\)/g)].map((m) => m[1]!);
    expect(used).toEqual(['--ar-tok', '--ar-tok-3']);
    for (const mixin of ['ar-light', 'ar-dark']) for (const token of used) expect(declares(mixin)).toContain(token);
    expect(scss).not.toMatch(/#[0-9a-f]{3,8}\b|rgba?\(|hsla?\(/i);
    for (const m of scss.matchAll(/(?<![-\w])color:\s*([^;]+);/g)) expect(m[1]).toMatch(/^var\(--cds-text-(primary|secondary)\)$/);
  });
});
