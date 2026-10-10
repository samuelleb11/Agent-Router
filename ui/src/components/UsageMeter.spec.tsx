import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { Theme } from '@carbon/react';
import { render, screen, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { stubCoarse } from '~/test-utils';
import { UsageMeter } from './UsageMeter';

const plain = (s: string) => s.replace(/\/\*[\s\S]*?\*\//g, '').replace(/\/\/.*$/gm, '');
const scss = plain(readFileSync(join(__dirname, 'UsageMeter.module.scss'), 'utf8'));
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

const levelOf = (container: HTMLElement) => container.firstElementChild!.getAttribute('data-level');

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('UsageMeter', () => {
  it('OV-C57: is a meter named by its window, valued in the words the eye reads', () => {
    render(<UsageMeter label="5-hour window" used={42.4} resets="resets in 2h 10m" resetsAt="14:05" />);
    const meter = screen.getByRole('meter', { name: '5-hour window' });
    expect(meter).toHaveAttribute('aria-valuemin', '0');
    expect(meter).toHaveAttribute('aria-valuemax', '100');
    expect(meter).toHaveAttribute('aria-valuenow', '42');
    expect(meter).toHaveAttribute('aria-valuetext', '42% used · resets in 2h 10m');
    expect(screen.getByText('42% used · resets in 2h 10m')).toBeVisible();
  });

  it.each([
    [0, 'ok'],
    [69, 'ok'],
    [69.4, 'ok'],
    [69.6, 'warn'],
    [70, 'warn'],
    [89, 'warn'],
    [90, 'bad'],
    [100, 'bad'],
  ] as const)('OV-C57: at %s %% is %s — accent below 70, amber from 70, red from 90, as the figure reads', (used, level) => {
    const { container } = render(<UsageMeter label="5-hour window" used={used} />);
    expect(levelOf(container)).toBe(level);
  });

  it('OV-C57: caps the bar and its figure at 100 %', () => {
    const { container } = render(<UsageMeter label="7-day window" used={104} tone="slate" />);
    expect(screen.getByRole('meter')).toHaveAttribute('aria-valuenow', '100');
    expect(screen.getByText('100% used')).toBeInTheDocument();
    expect(container.querySelector('[role=meter] > span')).toHaveStyle({ inlineSize: '100%' });
  });

  it('OV-C57, OV-C62: paints the 5-hour window in the accent, the 7-day in slate, and amber and red over either', () => {
    expect(block(scss, '\n.fill {')).toMatch(/background:\s*var\(--cds-interactive\);/);
    expect(block(scss, ".meter[data-tone='slate'] .fill")).toMatch(/background:\s*var\(--ar-tok\);/);
    expect(block(scss, ".meter[data-level='warn'] .fill")).toMatch(/background:\s*var\(--ar-warn-fill\);/);
    expect(block(scss, ".meter[data-level='bad'] .fill")).toMatch(/background:\s*var\(--ar-bad-fill\);/);
    // The thresholds come after the tone, so they win over it on specificity's tie.
    expect(scss.indexOf(".meter[data-level='warn'] .fill")).toBeGreaterThan(scss.indexOf(".meter[data-tone='slate'] .fill"));
    const { container } = render(<UsageMeter label="7-day window" used={30} tone="slate" />);
    expect(container.firstElementChild).toHaveAttribute('data-tone', 'slate');
  });

  /// Missing is not zero: a screen reader must not hear "0 %" where the eye reads "—".
  it('OV-C58, SH-C20: shows a window with no reading as "—", never as 0 % used and never as a meter at zero', () => {
    const { container } = render(<UsageMeter label="7-day window" used={null} resets="resets in 3d 4h" />);
    expect(screen.getByText('—')).toBeVisible();
    expect(screen.queryByRole('meter')).toBeNull();
    expect(container.textContent).not.toMatch(/0%/);
    expect(levelOf(container)).toBe('none');
  });

  it('OV-C72: shows a window that rolled over as grey, with its sentence and never its old figure', () => {
    const { container } = render(
      <UsageMeter label="5-hour window" used={88} resets="resets in 0 min" resetsAt="09:00" expired="window rolled over 3 h ago · nothing seen since" />,
    );
    expect(screen.getByText('window rolled over 3 h ago · nothing seen since')).toBeVisible();
    expect(screen.queryByRole('meter')).toBeNull();
    expect(container.textContent).not.toMatch(/88|resets/);
    expect(levelOf(container)).toBe('expired');
    // Grey: the bare track, with no fill at all.
    expect(container.querySelector('[class*=fill]')).toBeNull();
  });

  it('is never re-announced by a poll', () => {
    const { container, rerender } = render(<UsageMeter label="5-hour window" used={42} resets="resets in 2h 10m" />);
    rerender(<UsageMeter label="5-hour window" used={43} resets="resets in 2h 05m" />);
    expect(screen.getByRole('meter', { name: '5-hour window' })).toHaveAttribute('aria-valuetext', '43% used · resets in 2h 05m');
    expect(container.querySelector('[aria-live]')).toBeNull();
    expect(screen.queryByRole('progressbar')).toBeNull();
    expect(screen.queryByRole('status')).toBeNull();
    expect(screen.queryByRole('alert')).toBeNull();
  });

  it('carries the forecast line under the meter', () => {
    render(
      <UsageMeter label="7-day window" used={49} tone="slate">
        <p>81 h of work left · resets first, ending ≈71%</p>
      </UsageMeter>,
    );
    expect(screen.getByText('81 h of work left · resets first, ending ≈71%')).toBeVisible();
  });
});

describe('UsageMeter without a hover', () => {
  /// The reset time once lived only in the meter's tooltip.
  it('OV-C63, SH-C30: shows every fact it draws as text: the window, the figure, when it resets and the exact time', () => {
    const { container } = render(<UsageMeter label="5-hour window" used={42} resets="resets in 2h 10m" resetsAt="Oct 5, 14:05" />);
    for (const fact of ['5-hour window', '42% used · resets in 2h 10m', 'resets at Oct 5, 14:05']) expect(screen.getByText(fact)).toBeVisible();
    expect(container.querySelectorAll('[title]')).toHaveLength(0);
  });

  /// A meter is read, never pressed: on a touch screen there is nothing to aim at and nothing that
  /// waits for a hover.
  it('has no target on a coarse pointer, and says everything at once', () => {
    stubCoarse();
    const { container } = render(<UsageMeter label="7-day window" used={91} resets="resets in 3d 4h" resetsAt="Oct 9" tone="slate" />);
    expect(container.querySelectorAll('button, a, input, [tabindex]')).toHaveLength(0);
    expect(screen.getByRole('meter', { name: '7-day window' })).toHaveAttribute('aria-valuetext', '91% used · resets in 3d 4h');
    expect(screen.getByText('resets at Oct 9')).toBeVisible();
  });
});

describe('UsageMeter in both themes', () => {
  it('draws the same meter in white and in g100', () => {
    const drawn = (theme: 'white' | 'g100') => {
      const { container, unmount } = render(
        <Theme theme={theme}>
          <UsageMeter label="5-hour window" used={75} resets="resets in 1h" />
        </Theme>,
      );
      const wrapper = container.firstElementChild as HTMLElement;
      expect(wrapper).toHaveClass(`cds--${theme}`);
      expect(within(wrapper).getByRole('meter', { name: '5-hour window' })).toBeInTheDocument();
      // useId differs between renders; the drawing does not.
      const html = wrapper.innerHTML.replace(/id="[^"]*"|aria-labelledby="[^"]*"/g, '');
      unmount();
      return html;
    };
    expect(drawn('g100')).toBe(drawn('white'));
  });

  it('paints only in tokens both themes declare, and its words only in Carbon’s text tokens', () => {
    const used = [...scss.matchAll(/var\((--ar-[a-z0-9-]+)\)/g)].map((m) => m[1]!);
    expect(used).toEqual(expect.arrayContaining(['--ar-tok', '--ar-warn-fill', '--ar-bad-fill']));
    for (const mixin of ['ar-light', 'ar-dark']) for (const token of used) expect(declares(mixin), `${token} in ${mixin}`).toContain(token);
    expect(scss).not.toMatch(/#[0-9a-f]{3,8}\b|rgba?\(|hsla?\(/i);
    for (const m of scss.matchAll(/(?<![-\w])color:\s*([^;]+);/g)) expect(m[1]).toMatch(/^var\(--cds-text-(primary|secondary)\)$/);
  });
});
