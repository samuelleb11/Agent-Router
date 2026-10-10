import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { Theme } from '@carbon/react';
import { fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { stubCoarse } from '~/test-utils';
import { BurnDown, type Burn } from './BurnDown';
import tipStyles from './DataTip.module.scss';

const plain = (s: string) => s.replace(/\/\*[\s\S]*?\*\//g, '').replace(/\/\/.*$/gm, '');
const scss = plain(readFileSync(join(__dirname, 'BurnDown.module.scss'), 'utf8'));
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

/// A 7-day window, half gone: opened at T0, resets a week later, 49 % used now.
const HOUR = 3_600_000;
const T0 = Date.UTC(2026, 9, 5, 9);
const NOW = T0 + 84 * HOUR;
const when = (ms: number) => `d${(ms - T0) / HOUR}h`;
const base: Burn = {
  start: T0,
  reset: T0 + 168 * HOUR,
  used: 0.49,
  series: [
    { t: T0 + 12 * HOUR, u: 0.1 },
    { t: T0 + 48 * HOUR, u: 0.3 },
  ],
  verdict: 'resets-first',
  crossing: null,
  end: 0.71,
  band: null,
};
/// The weekly window that started it all: crossing 2.2 h before a reset 94.7 h away, inside the
/// spread of the reader's own days, so too close to call (5b2c120).
const tight: Burn = {
  ...base,
  verdict: 'tight',
  crossing: T0 + 166 * HOUR,
  end: null,
  band: { quiet: { crossing: null, end: 0.86 }, busy: { crossing: T0 + 150 * HOUR, end: null } },
};
const crosses: Burn = {
  ...base,
  used: 0.7,
  verdict: 'crosses',
  crossing: T0 + 120 * HOUR,
  end: null,
  band: { quiet: { crossing: T0 + 140 * HOUR, end: null }, busy: { crossing: T0 + 110 * HOUR, end: null } },
};

const show = (burn: Burn | null, label = '7-day window') => render(<BurnDown label={label} burn={burn} now={NOW} tone="slate" formatTime={when} />);
const chart = () => screen.getByRole('img', { name: /burn-down/ });
const ray = (c: HTMLElement) => c.querySelector('path[data-verdict]:not([class*=cone])')!;
const marker = (c: HTMLElement) => c.querySelector('span[data-verdict]');

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('BurnDown', () => {
  it('OV-C80: is an image named by its window, its times, its use now and where the projection lands', () => {
    show(base);
    expect(screen.getByRole('img', { name: '7-day window · burn-down: opened d0h, resets d168h; 49% used now, ends ≈71% at the reset' })).toBeInTheDocument();
    expect(screen.getByText('7-day window · burn-down')).toBeVisible();
  });

  it('OV-C80: draws what was seen up to now and the dashed projection from it, between rules at 0 and 100 %', () => {
    const { container } = show(base);
    expect(container.querySelector('path[data-tone="slate"]')).toHaveAttribute('d', 'M7.14,90.00L28.57,70.00L50.00,51.00');
    expect(ray(container)).toHaveAttribute('d', 'M50.00,51.00L100.00,29.00');
    expect([...container.querySelectorAll('line')].map((l) => [l.getAttribute('y1'), l.getAttribute('x1')])).toEqual([
      ['0', '0'],
      ['100', '0'],
      ['0', '50'],
    ]);
    expect(block(scss, '\n.ray {')).toMatch(/stroke-dasharray:/);
  });

  it('OV-C81: draws the projection to its crossing on the 100 % line, with the one marker there', () => {
    const { container } = show(crosses);
    expect(ray(container)).toHaveAttribute('d', 'M50.00,30.00L71.43,0.00');
    const m = marker(container)!;
    expect(m.getAttribute('style')).toContain('inset-inline-start: 71.42857142857143%');
    expect(m.getAttribute('style')).toContain('inset-block-start: 0%');
    expect(container.querySelectorAll('span[data-verdict]')).toHaveLength(1);
  });

  it('OV-C81: draws a projection that resets first to the reset at the level it ends, with no marker', () => {
    const { container } = show(base);
    expect(ray(container)).toHaveAttribute('d', 'M50.00,51.00L100.00,29.00');
    expect(marker(container)).toBeNull();
  });

  /// 2026-09-14: a 7-day window at 49 %, crossing 2.2 h before a reset 94.7 h away, was drawn red.
  it('OV-C68, OV-C82: is red only for a crossing that survives a quiet week, amber when too close to call, muted when it resets first', () => {
    const verdicts = [crosses, tight, base].map((b) => {
      const { container, unmount } = show(b);
      const v = ray(container).getAttribute('data-verdict');
      unmount();
      return v;
    });
    expect(verdicts).toEqual(['crosses', 'tight', 'resets-first']);
    expect(block(scss, ".ray[data-verdict='crosses']")).toMatch(/stroke:\s*var\(--ar-bad-fill\);/);
    expect(block(scss, ".ray[data-verdict='tight']")).toMatch(/stroke:\s*var\(--ar-warn-fill\);/);
    expect(block(scss, ".ray[data-verdict='resets-first']")).toMatch(/stroke:\s*var\(--cds-icon-secondary\);/);
    // A tight window's central crossing gets its marker, in amber, never in red.
    const { container } = show(tight);
    expect(marker(container)).toHaveAttribute('data-verdict', 'tight');
    expect(block(scss, ".marker[data-verdict='tight']")).toMatch(/background:\s*var\(--ar-warn-fill\);/);
    expect(scss.match(/--ar-bad-fill/g)).toHaveLength(3); // the red ray, the red cone and the red marker: all three gated on crosses
    for (const rule of [".ray[data-verdict='crosses']", ".cone[data-verdict='crosses']", ".marker[data-verdict='crosses']"]) expect(block(scss, rule)).toMatch(/--ar-bad-fill/);
  });

  it('OV-C71: reads a router with no verdict by the old boolean, a crossing as crosses and none as resets first', () => {
    const { container, unmount } = show({ ...crosses, verdict: null, band: null });
    expect(ray(container)).toHaveAttribute('data-verdict', 'crosses');
    expect(marker(container)).toHaveAttribute('data-verdict', 'crosses');
    unmount();
    const older = show({ ...base, verdict: null });
    expect(ray(older.container)).toHaveAttribute('data-verdict', 'resets-first');
  });

  it('OV-C83: shades the cone between a quiet week and a busy one only when the band has width, red-tinted only when it crosses', () => {
    const red = show(crosses);
    const cone = red.container.querySelector('path[class*=cone]')!;
    expect(cone).toHaveAttribute('data-verdict', 'crosses');
    // From now to the busy week's crossing, along the ceiling to the reset, and back by the quiet week's.
    expect(cone).toHaveAttribute('d', 'M50.00,30.00L65.48,0.00L100.00,0.00L100.00,0.00L83.33,0.00Z');
    red.unmount();
    const grey = show(tight);
    expect(grey.container.querySelector('path[class*=cone]')).toHaveAttribute('data-verdict', 'tight');
    expect(block(scss, '\n.cone {')).toMatch(/fill:\s*var\(--cds-icon-secondary\);/);
    grey.unmount();
    // No band — the 5-hour window, an older router — draws the single ray.
    expect(show(base).container.querySelector('path[class*=cone]')).toBeNull();
  });

  /// Tight means only the busy edge crosses, so the central estimate may never reach 100 %.
  it('OV-C70: never draws or says a crossing a tight window was not given, and stays amber', () => {
    const { container } = show({ ...tight, crossing: null, end: 0.96 });
    expect(marker(container)).toBeNull();
    expect(ray(container)).toHaveAttribute('data-verdict', 'tight');
    expect(ray(container)).toHaveAttribute('d', 'M50.00,51.00L100.00,4.00');
    expect(screen.getByText(/used now/)).toHaveTextContent(/^49% used now · ends ≈96% at the reset$/);
  });

  it.each([
    ['absent', null],
    ['rolled over', { ...base, expired: true }],
    ['with no start', { ...base, start: null }],
    ['resetting before it starts', { ...base, reset: T0 }],
  ] as const)('OV-C84: draws nothing for a window %s', (_, burn) => {
    const { container } = show(burn);
    expect(container).toBeEmptyDOMElement();
  });

  it('OV-C84: holds everything outside the window to its edges', () => {
    const { container } = render(
      <BurnDown label="5-hour window" burn={{ ...base, series: [{ t: T0 - 10 * HOUR, u: 1.4 }] }} now={T0 + 200 * HOUR} formatTime={when} />,
    );
    expect(container.querySelector('path[data-tone]')).toHaveAttribute('d', 'M0.00,0.00L100.00,51.00');
  });
});

describe('BurnDown without a hover', () => {
  /// When the window opened, the use now and the reset were in the burn-down's tooltip only.
  it('OV-C63, SH-C30: says when the window opened, the use now, where it lands and when it resets, as text', () => {
    show(tight);
    for (const text of ['d0h', 'resets d168h']) expect(screen.getByText(text)).toBeVisible();
    expect(screen.getByText(/used now/)).toHaveTextContent(/^49% used now · reaches 100% d166h$/);
    expect(screen.getByText('a quiet week ends ≈86% at the reset, a busy one reaches 100% d150h')).toBeVisible();
  });

  it('OV-C197: gives the same in its tip on focus', () => {
    show(crosses);
    fireEvent.focus(chart());
    const tip = screen.getByRole('tooltip');
    for (const text of ['70%', 'used now', 'reaches 100% d120h', 'projection', 'd0h', 'window opened', 'd168h', 'window resets']) expect(tip).toHaveTextContent(text);
  });
});

describe('BurnDown on a coarse pointer', () => {
  it('OV-C197, SH-C45: pins its tip on a tap, on a target a fingertip tall', () => {
    stubCoarse();
    show(base);
    fireEvent.click(chart());
    expect(screen.getByRole('tooltip')).toHaveTextContent('ends ≈71% at the reset');
    fireEvent.pointerLeave(chart(), { pointerType: 'touch' });
    expect(screen.getByRole('tooltip')).toBeInTheDocument();
    expect(chart()).toHaveClass(tipStyles.target!);
  });
});

describe('BurnDown in both themes', () => {
  it('draws the same chart in white and in g100', () => {
    const drawn = (theme: 'white' | 'g100') => {
      const { container, unmount } = render(
        <Theme theme={theme}>
          <BurnDown label="7-day window" burn={crosses} now={NOW} tone="slate" formatTime={when} />
        </Theme>,
      );
      const wrapper = container.firstElementChild as HTMLElement;
      expect(wrapper).toHaveClass(`cds--${theme}`);
      expect(within(wrapper).getByRole('img', { name: /^7-day window · burn-down/ })).toBeInTheDocument();
      const html = wrapper.innerHTML;
      unmount();
      return html;
    };
    expect(drawn('g100')).toBe(drawn('white'));
  });

  it('draws the 7-day window in slate and the 5-hour in Carbon’s blue, in tokens both themes declare', () => {
    expect(block(scss, '\n.seen {')).toMatch(/stroke:\s*var\(--cds-interactive\);/);
    expect(block(scss, ".seen[data-tone='slate']")).toMatch(/stroke:\s*var\(--ar-tok\);/);
    const used = [...scss.matchAll(/var\((--ar-[a-z0-9-]+)\)/g)].map((m) => m[1]!);
    for (const mixin of ['ar-light', 'ar-dark']) for (const token of used) expect(declares(mixin)).toContain(token);
    expect(scss).not.toMatch(/#[0-9a-f]{3,8}\b|rgba?\(|hsla?\(/i);
    for (const m of scss.matchAll(/(?<![-\w])color:\s*([^;]+);/g)) expect(m[1]).toMatch(/^var\(--cds-text-(primary|secondary)\)$/);
  });
});
