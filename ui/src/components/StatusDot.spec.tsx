import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { Theme } from '@carbon/react';
import { render, screen, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { stubCoarse } from '~/test-utils';
import { StatusDot, StatusPill } from './StatusDot';

/// The stylesheets, read as source: jsdom computes no style, and what is held here is how the
/// dot may be painted.
const plain = (s: string) => s.replace(/\/\*[\s\S]*?\*\//g, '').replace(/\/\/.*$/gm, '');
const scss = plain(readFileSync(join(__dirname, 'StatusDot.module.scss'), 'utf8'));
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

const dot = (name: string | RegExp) => screen.getByRole('img', { name });

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('StatusDot', () => {
  it('SE-C20: is an image named by its words, so it never says its state by colour alone', () => {
    render(<StatusDot tone="attention" label="Waiting for you · 1h" live />);
    expect(dot('Waiting for you · 1h')).toHaveAttribute('data-tone', 'attention');
  });

  it.each([
    ['busy', 'working'],
    ['attention', 'waiting for you'],
    ['warn', 'stalled'],
    ['error', 'failed'],
    ['idle', 'idle'],
    ['closed', 'ended'],
  ] as const)('SE-C20: says %s in the phones’ word, "%s", when it has no words of its own', (tone, words) => {
    render(<StatusDot tone={tone} />);
    expect(dot(words)).toHaveAttribute('data-tone', tone);
  });

  /// A newer router saying something this console has not heard of is not an error: the plain
  /// neutral dot, with whatever words the router gave.
  it('SE-C2, SE-C3: draws a tone it does not know as the plain neutral dot, never as an error', () => {
    render(<StatusDot tone="pondering" label="Pondering · 5m" />);
    const d = dot('Pondering · 5m');
    expect(d).not.toHaveAttribute('data-tone');
    expect(block(scss, '\n.dot {')).toMatch(/--dot:\s*var\(--cds-icon-secondary\);/);
  });

  it('SE-C21: never reads "undefined", "null" or "NaN", whatever it is given', () => {
    const { container } = render(
      <>
        <StatusDot tone={undefined} />
        <StatusDot tone={null} label="   " />
        <StatusPill tone={undefined} />
        <StatusPill tone="idle" text="" />
      </>,
    );
    const said = [...container.querySelectorAll('[aria-label]')].map((e) => e.getAttribute('aria-label')).join(' ') + container.textContent;
    expect(said).not.toMatch(/undefined|null|NaN/);
    expect(screen.getAllByRole('img', { name: 'status unknown' })).toHaveLength(2);
  });

  it('SE-C4: wears the phones’ palette — working green, waiting orange, stalled amber, failed red, idle grey', () => {
    expect(block(scss, ".dot[data-tone='busy']")).toMatch(/--dot:\s*var\(--ar-st-busy\);/);
    expect(block(scss, ".dot[data-tone='attention']")).toMatch(/--dot:\s*var\(--ar-st-wait\);/);
    expect(block(scss, ".dot[data-tone='warn']")).toMatch(/--dot:\s*var\(--ar-warn-fill\);/);
    expect(block(scss, ".dot[data-tone='error']")).toMatch(/--dot:\s*var\(--ar-bad-fill\);/);
    expect(block(scss, ".dot[data-tone='idle']")).toMatch(/--dot:\s*var\(--ar-st-idle\);/);
  });

  it('SE-C7: draws a closed session as a hollow grey ring', () => {
    render(<StatusDot tone="closed" label="Session ended · 3h ago" />);
    expect(dot('Session ended · 3h ago')).toHaveAttribute('data-tone', 'closed');
    const closed = block(scss, ".dot[data-tone='closed']");
    expect(closed).toMatch(/background:\s*transparent;/);
    expect(closed).toMatch(/box-shadow:\s*inset 0 0 0 \$spacing-01 var\(--dot\);/);
    expect(closed).toMatch(/--dot:\s*var\(--ar-st-idle\);/);
    // Last, so a halo can never fill the ring in.
    expect(scss.lastIndexOf(".dot[data-tone='closed']")).toBeGreaterThan(scss.indexOf('.dot[data-live]'));
  });

  it('SE-C6, SH-C40: pulses when busy and faster when waiting, and not at all under reduced motion', () => {
    const duration = (tone: string) => /animation:\s*pulse calc\((\d) \* #\{\$duration-(slow-0\d)\}\)/.exec(block(scss, `.dot[data-tone='${tone}']`));
    const ms = { 'slow-01': 400, 'slow-02': 700 } as Record<string, number>;
    const busy = duration('busy')!;
    const attention = duration('attention')!;
    expect(Number(attention[1]) * ms[attention[2]!]!).toBeLessThan(Number(busy[1]) * ms[busy[2]!]!);
    for (const still of ['warn', 'error', 'idle', 'closed']) expect(block(scss, `.dot[data-tone='${still}']`)).not.toMatch(/animation/);
    expect(block(block(scss, '@media (prefers-reduced-motion: reduce)'), '.dot {')).toMatch(/animation:\s*none;/);
  });

  it('SE-C9: wears the halo only when live', () => {
    render(
      <>
        <StatusDot tone="error" label="row" live />
        <StatusDot tone="error" label="tab" />
      </>,
    );
    expect(dot('row')).toHaveAttribute('data-live');
    expect(dot('tab')).not.toHaveAttribute('data-live');
    expect(block(scss, '.dot[data-live]')).toMatch(/box-shadow:/);
  });

  it('is hidden from a screen reader when drawn beside its own words', () => {
    const { container } = render(<StatusDot tone="busy" decorative />);
    expect(container.firstElementChild).toHaveAttribute('aria-hidden', 'true');
    expect(container.firstElementChild).not.toHaveAttribute('role');
    expect(screen.queryByRole('img')).toBeNull();
  });
});

describe('StatusPill', () => {
  it('SE-C62: is a status named by its line, beside a pip in the status’s tone', () => {
    render(<StatusPill tone="busy" text="Running Bash · 12s · 2 queued" />);
    const pill = screen.getByRole('status', { name: 'Running Bash · 12s · 2 queued' });
    expect(pill).toHaveTextContent('Running Bash · 12s · 2 queued');
    // The pip is drawn beside the words, so it is not read twice.
    const pip = pill.querySelector('[data-tone]')!;
    expect(pip).toHaveAttribute('data-tone', 'busy');
    expect(pip).toHaveAttribute('aria-hidden', 'true');
  });

  /// The line carries a duration that changes every second; a live region would read each one.
  it('is never re-announced as its line changes', () => {
    const { rerender } = render(<StatusPill tone="busy" text="Thinking · 5s" />);
    rerender(<StatusPill tone="busy" text="Thinking · 6s" />);
    expect(screen.getByRole('status', { name: 'Thinking · 6s' })).toHaveAttribute('aria-live', 'off');
  });

  it('SE-C63: says its state’s word when the status has no line', () => {
    render(<StatusPill tone="idle" text="" />);
    expect(screen.getByRole('status', { name: 'idle' })).toHaveTextContent('idle');
  });

  it('SE-C9: never wears the halo', () => {
    const { container } = render(<StatusPill tone="attention" text="Waiting for you" />);
    expect(container.querySelector('[data-live]')).toBeNull();
  });

  it('SE-C61, SE-C64: says connecting or reconnecting in neutral italics, with a pip of no tone', () => {
    render(<StatusPill tone="busy" text="reconnecting…" stale />);
    const pill = screen.getByRole('status', { name: 'reconnecting…' });
    expect(pill).toHaveAttribute('data-stale');
    expect(pill.querySelector('[aria-hidden]')).not.toHaveAttribute('data-tone');
    expect(block(scss, '.pill[data-stale] .words')).toMatch(/color:\s*var\(--cds-text-secondary\);[\s\S]*font-style:\s*italic;/);
  });

  it('SH-C44: wraps a long line rather than widening a phone', () => {
    expect(block(scss, '\n.words {')).toMatch(/overflow-wrap:\s*anywhere;/);
  });
});

describe('StatusDot and StatusPill in both themes', () => {
  it('draw the same marks and words in white and in g100', () => {
    const drawn = (theme: 'white' | 'g100') => {
      const { container, unmount } = render(
        <Theme theme={theme}>
          <StatusDot tone="attention" label="Waiting for you" live />
          <StatusPill tone="error" text="API error: 529 Overloaded · 1m" />
        </Theme>,
      );
      const wrapper = container.firstElementChild!;
      expect(wrapper).toHaveClass(`cds--${theme}`);
      expect(within(wrapper as HTMLElement).getByRole('img', { name: 'Waiting for you' })).toBeInTheDocument();
      const html = wrapper.innerHTML;
      unmount();
      return html;
    };
    expect(drawn('g100')).toBe(drawn('white'));
  });

  /// Each theme class carries its own --ar-* set (styles.scss), so a dot painted only in tokens is
  /// painted for whichever theme it sits in. Words wear Carbon's text tokens, never a tone's fill.
  it('paint only in tokens both themes declare, and words only in Carbon’s text tokens', () => {
    const used = [...scss.matchAll(/var\((--ar-[a-z0-9-]+)\)/g)].map((m) => m[1]!);
    expect(used.length).toBeGreaterThan(0);
    for (const mixin of ['ar-light', 'ar-dark']) for (const token of used) expect(declares(mixin), `${token} in ${mixin}`).toContain(token);
    expect(scss).not.toMatch(/#[0-9a-f]{3,8}\b|rgba?\(|hsla?\(/i);
    for (const m of scss.matchAll(/(?<![-\w])color:\s*([^;]+);/g)) expect(m[1]).toMatch(/^var\(--cds-text-(primary|secondary)\)$/);
  });
});

describe('StatusDot on a coarse pointer', () => {
  /// A dot and a pill are read, never pressed: nothing in them is a target to size, and nothing
  /// waits for a hover. Their words are there as soon as they are drawn.
  it('has no target and nothing behind a hover, so every word is there without one', () => {
    stubCoarse();
    const { container } = render(
      <>
        <StatusDot tone="warn" label="Stalled · 4m" live />
        <StatusPill tone="busy" text="Running a tool · 9s" />
      </>,
    );
    expect(container.querySelectorAll('button, a, input, [tabindex]')).toHaveLength(0);
    expect(container.querySelectorAll('[title]')).toHaveLength(0);
    expect(dot('Stalled · 4m')).toBeInTheDocument();
    expect(screen.getByText('Running a tool · 9s')).toBeVisible();
  });
});
