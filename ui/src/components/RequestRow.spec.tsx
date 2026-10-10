import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { Theme } from '@carbon/react';
import { act, fireEvent, render, screen, within } from '@testing-library/react';
import { useState, type ComponentProps } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { stubCoarse, stubNarrow } from '~/test-utils';
import { RequestRow, type Detail } from './RequestRow';

const plain = (s: string) => s.replace(/\/\*[\s\S]*?\*\//g, '').replace(/\/\/.*$/gm, '');
const scss = plain(readFileSync(join(__dirname, 'RequestRow.module.scss'), 'utf8'));
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

type Props = ComponentProps<typeof RequestRow>;
const DETAIL: Detail = {
  facts: [
    { label: 'request id', value: 'req_011CUa', mono: true },
    { label: 'session', value: '3f2a9c1e · agent-router' },
    { label: 'stopped', value: 'end_turn' },
    { label: 'cache', value: '1.1k read · 40 written · 60 uncached' },
  ],
  error: 'API error: 529 Overloaded\nretrying in 2s',
  context: [
    { key: 'tools', label: 'tools (definitions)', value: 600, colour: '--ar-ctx-tools', concept: 'tools' },
    { key: 'system', label: 'system prompt', value: 300, colour: '--ar-ctx-system', concept: 'systemPrompt' },
    { key: 'history', label: 'conversation', value: 100, colour: '--ar-ctx-history', concept: 'conversation' },
  ],
};
const BASE: Props = {
  time: '14:05:12',
  at: 'Oct 5, 2026, 14:05:12',
  model: 'claude-opus-4-5-20251101',
  modelShort: 'opus 4.5',
  family: 'opus',
  subagent: true,
  slow: true,
  status: 529,
  error: true,
  prompt: 1_200,
  max: 4_800,
  figures: { prompt: '1.2k', cached: '92% cached', output: '1.1k out', cost: '$0.012', firstToken: '3.4 s' },
  open: false,
  onToggle: () => {},
  detail: () => DETAIL,
};

/// The row as the Overview holds it: open or not by the address, here a state.
function Held(props: Partial<Props>) {
  const [open, setOpen] = useState(false);
  return <RequestRow {...BASE} {...props} open={open} onToggle={() => setOpen((o) => !o)} />;
}
const summary = () => screen.getByRole('button', { name: /^14:05:12/ });
const region = () => screen.getByRole('region');

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('RequestRow', () => {
  it('OV-C157: is a button that opens its detail in place, and a region the button names', () => {
    render(<Held />);
    expect(summary()).toHaveAttribute('aria-expanded', 'false');
    expect(screen.queryByRole('region')).toBeNull();
    fireEvent.click(summary());
    expect(summary()).toHaveAttribute('aria-expanded', 'true');
    expect(summary()).toHaveAttribute('aria-controls', region().id);
    expect(region()).toHaveAccessibleName(summary().textContent!.replace(/\s+/g, ' ').trim());
    fireEvent.click(summary());
    expect(screen.queryByRole('region')).toBeNull();
  });

  it('OV-C156: says its time, model, badges, status, prompt, cache, output, cost and first token, in words', () => {
    render(<RequestRow {...BASE} />);
    expect(summary()).toHaveAccessibleName('14:05:12 opus 4.5 subagent slow 529 1.2k 92% cached 1.1k out · $0.012 3.4 s first token');
  });

  it('OV-C156: draws the prompt’s bar against the largest in the filtered set, and none for a prompt not recorded', () => {
    const { container, rerender } = render(<RequestRow {...BASE} />);
    expect(container.querySelector('[aria-hidden] > span')).toHaveStyle({ inlineSize: '25%' });
    rerender(<RequestRow {...BASE} prompt={null} figures={{ ...BASE.figures, prompt: '—', cached: '—' }} />);
    expect(container.querySelector('[aria-hidden] > span[style]')).toBeNull();
    expect(summary()).toHaveAccessibleName(/— — 1\.1k out/);
  });

  it('OV-C5, OV-C149: marks an error with the error icon in the error colour, with its code, or ERR without one', () => {
    const { rerender } = render(<RequestRow {...BASE} />);
    const tag = within(summary()).getByText('529');
    expect(tag.querySelector('svg')).toHaveStyle({ fill: 'var(--ar-bad-fill)' });
    expect(block(scss, '\n.error {')).toMatch(/color:\s*var\(--ar-bad\);/);
    rerender(<RequestRow {...BASE} status={null} />);
    expect(within(summary()).getByText('ERR')).toBeInTheDocument();
    rerender(<RequestRow {...BASE} error={false} status={200} />);
    expect(within(summary()).getByText('200').querySelector('svg')).toBeNull();
  });

  it('OV-C10: wears its family’s swatch, and "other" as a hollow ring', () => {
    const { container, rerender } = render(<RequestRow {...BASE} />);
    expect(container.querySelector('[data-family="opus"]')).toHaveStyle({ background: 'var(--ar-model-opus)' });
    rerender(<RequestRow {...BASE} family="other" model="mystery-1" modelShort="mystery-1" />);
    const other = container.querySelector('[data-family="other"]') as HTMLElement;
    expect(other.style.background).toBe('');
    expect(block(scss, ".swatch[data-family='other']")).toMatch(/box-shadow:\s*inset 0 0 0 \$spacing-01 var\(--ar-model-other\);/);
  });

  it('OV-C184, OV-C196: names its model as every client spells it, whole up to "sonnet-4-5-20250929" and ellipsised past that, never cut mid-letter', () => {
    render(<RequestRow {...BASE} model="claude-sonnet-4-5-20250929" modelShort="sonnet-4-5-20250929" family="sonnet" />);
    const name = within(summary()).getByText('sonnet-4-5-20250929');
    expect(name.className).toMatch(/modelName/);
    // The cell is wide enough for the swatch and the nineteen characters of that spelling, and a
    // longer name gives up its tail with an ellipsis rather than being clipped.
    expect(block(scss, '\n.model {')).toMatch(/flex:\s*0 1 22ch;/);
    expect(block(scss, '.modelName {')).toMatch(/overflow:\s*hidden;\s*text-overflow:\s*ellipsis;/);
  });

  it('OV-C158: opens into every fact recorded, the error at full width with its line breaks, and its context window', () => {
    render(<RequestRow {...BASE} open />);
    const facts = within(region()).getAllByRole('term').map((dt) => [dt.textContent, dt.nextElementSibling!.textContent]);
    expect(facts).toEqual([
      ['at', 'Oct 5, 2026, 14:05:12'],
      ['model', 'claude-opus-4-5-20251101'],
      ['first token', '3.4 s'],
      ['request id', 'req_011CUa'],
      ['session', '3f2a9c1e · agent-router'],
      ['stopped', 'end_turn'],
      ['cache', '1.1k read · 40 written · 60 uncached'],
    ]);
    expect(within(region()).getByText(/^API error: 529 Overloaded/).textContent).toBe('API error: 529 Overloaded\nretrying in 2s');
    expect(block(scss, '\n.message {')).toMatch(/white-space:\s*pre-wrap;/);
    expect(within(region()).getByRole('img', { name: /^Context window: tools \(definitions\) 600, 60%; system prompt 300, 30%; conversation 100, 10%$/ })).toBeInTheDocument();
  });

  /// The list holds 300 rows; a detail is built only for a row that is open.
  it('builds its detail only while it is open', () => {
    const detail = vi.fn(() => DETAIL);
    const { rerender } = render(<RequestRow {...BASE} detail={detail} />);
    expect(detail).not.toHaveBeenCalled();
    rerender(<RequestRow {...BASE} detail={detail} open />);
    expect(detail).toHaveBeenCalled();
  });

  /// Open is the address's, so a poll that redraws the figures keeps the row open and the focus on
  /// it.
  it('OV-C157, SH-C25: stays open, with the focus where it was, through a poll that changes its figures', () => {
    const { rerender } = render(<RequestRow {...BASE} open />);
    act(() => summary().focus());
    rerender(<RequestRow {...BASE} open figures={{ ...BASE.figures, cost: '$0.013' }} />);
    expect(summary()).toHaveFocus();
    expect(summary()).toHaveAttribute('aria-expanded', 'true');
    expect(region()).toBeInTheDocument();
    expect(summary()).toHaveAccessibleName(/\$0\.013/);
  });
});

describe('RequestRow and its provider', () => {
  it('OV-C231: given the provider that served it, the card wears its mark alone, named by it, and the detail names the provider and the account in words', () => {
    render(<Held provider="kimi" account="Kimi · kimi-acc" model="k3" modelShort="k3" family="kimi" />);
    const mark = within(summary()).getByRole('img', { name: 'Kimi Code' });
    // After the time, before the model: the card's name reads the provider with the model it served.
    expect(summary()).toHaveAccessibleName(/^14:05:12 Kimi Code k3 /);
    // Its tooltip is its name said again, never a fact of its own (SH-C30).
    expect(mark).toHaveAttribute('title', 'Kimi Code');
    fireEvent.click(summary());
    const facts = within(region()).getAllByRole('term').map((dt) => [dt.textContent, dt.nextElementSibling!.textContent]);
    expect(facts.slice(0, 5)).toEqual([
      ['at', 'Oct 5, 2026, 14:05:12'],
      ['model', 'k3'],
      ['provider', 'Kimi Code'],
      ['account', 'Kimi · kimi-acc'],
      ['first token', '3.4 s'],
    ]);
    // In the detail, the mark and the name together.
    expect(within(region()).getByText('Kimi Code').closest('[data-provider]')).toHaveAttribute('data-provider', 'kimi');
  });

  it('OV-C231: a card given no provider draws none, as before', () => {
    render(<RequestRow {...BASE} open />);
    expect(document.querySelector('[data-provider]')).toBeNull();
    expect(within(region()).queryByText('provider')).toBeNull();
  });
});

describe('RequestRow without a hover', () => {
  /// The dashboard gave the full time and model id as titles, the slow badge's meaning as a title, and
  /// the request's split as the bar's tooltip.
  it('OV-C156, SH-C30: says everything on the card or one press away, never in a title', () => {
    const { container } = render(<Held />);
    expect(container.querySelectorAll('[title]')).toHaveLength(0);
    expect(within(summary()).getByText('slow')).toBeVisible();
    expect(within(summary()).getByText('subagent')).toBeVisible();
    fireEvent.click(summary());
    expect(within(region()).getByText('Oct 5, 2026, 14:05:12')).toBeVisible();
    expect(within(region()).getByText('claude-opus-4-5-20251101')).toBeVisible();
  });

  /// The eleven-track row was one of the causes of the dashboard's 143 px of sideways scroll at 360.
  it('OV-C159, SH-C44: is a two-line card on a phone, the first token in its detail, and never wider than the screen', () => {
    stubNarrow();
    render(<Held />);
    const narrow = block(scss, "@include breakpoint-down('md')");
    expect(block(narrow, '.summary {')).toMatch(/grid-template-areas:\s*'chev head head head status'\s*'\. bar prompt cached out';/);
    expect(block(narrow, '.ttfb {')).toMatch(/display:\s*none;/);
    expect(narrow).toMatch(/\n\s*\.out \{\s*white-space:\s*normal;\s*overflow-wrap:\s*anywhere;\s*\}/);
    // The desk's row wraps rather than scrolls, and nothing holds a width a phone cannot give.
    expect(block(scss, '\n.summary {')).toMatch(/flex-wrap:\s*wrap;/);
    expect(scss).not.toMatch(/min-inline-size:\s*(?!\s|0;|4ch;|10ch;|\$spacing-01;)[^;]+;/);
    fireEvent.click(summary());
    expect(within(region()).getByText('first token').nextElementSibling).toHaveTextContent('3.4 s');
  });
});

describe('RequestRow on a coarse pointer', () => {
  it('SH-C45: is a 48 px target', () => {
    stubCoarse();
    render(<Held />);
    expect(summary()).toBeEnabled();
    expect(block(block(scss, '@media (pointer: coarse)'), '.summary {')).toMatch(/min-block-size:\s*\$spacing-09;/);
  });
});

describe('RequestRow in both themes', () => {
  it('draws the same card and detail in white and in g100', () => {
    const drawn = (theme: 'white' | 'g100') => {
      const { container, unmount } = render(
        <Theme theme={theme}>
          <RequestRow {...BASE} open />
        </Theme>,
      );
      const wrapper = container.firstElementChild as HTMLElement;
      expect(wrapper).toHaveClass(`cds--${theme}`);
      expect(within(wrapper).getByRole('region')).toBeInTheDocument();
      const html = wrapper.innerHTML.replace(/(id|aria-controls|aria-labelledby)="[^"]*"/g, '');
      unmount();
      return html;
    };
    expect(drawn('g100')).toBe(drawn('white'));
  });

  it('paints only in tokens both themes declare, its words in Carbon’s text tokens and its errors in the error text token', () => {
    const used = [...scss.matchAll(/var\((--ar-[a-z0-9-]+)\)/g)].map((m) => m[1]!);
    for (const mixin of ['ar-light', 'ar-dark']) for (const token of [...used, '--ar-model-opus', '--ar-subagent', '--ar-bad-fill']) expect(declares(mixin)).toContain(token);
    expect(scss).not.toMatch(/#[0-9a-f]{3,8}\b|rgba?\(|hsla?\(/i);
    for (const m of scss.matchAll(/(?<![-\w])color:\s*([^;]+);/g)) expect(m[1]).toMatch(/^var\(--(cds-text-(primary|secondary)|ar-bad)\)$/);
  });
});
