// New for agent-router: the provider's mark (docs/web-console-contracts.md, SH-C56) — its two forms, the
// owners' files as published, Anthropic's in the text colour and Kimi's for the surface it sits on.
import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { Theme } from '@carbon/react';
import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { ProviderMark } from './ProviderMark';

/// The stylesheet, read as source: jsdom computes no style, and what is held here is how a mark may
/// be painted.
const plain = (s: string) => s.replace(/\/\*[\s\S]*?\*\//g, '').replace(/\/\/.*$/gm, '');
const scss = plain(readFileSync(join(__dirname, 'ProviderMark.module.scss'), 'utf8'));
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

const ASSETS = join(__dirname, '..', 'assets', 'providers');
/// The Kimi file a drawn mark shows.
const kimiFile = (el: HTMLElement) => el.querySelector('img[data-glyph="kimi"]')?.getAttribute('src') ?? '';

describe('ProviderMark', () => {
  it('SH-C56: the mark and the name, where there is room — the mark hidden, since the name beside it says it', () => {
    const { container } = render(<ProviderMark provider="kimi" />);
    const mark = container.querySelector<HTMLElement>('[data-provider="kimi"]')!;
    expect(mark).toHaveTextContent(/^Kimi Code$/);
    expect(mark).not.toHaveAttribute('role');
    expect(mark).not.toHaveAttribute('title');
    expect(mark.querySelector('[data-glyph]')).toHaveAttribute('aria-hidden', 'true');
    render(<ProviderMark provider="anthropic" />);
    expect(screen.getByText('Anthropic').closest('[data-provider]')).toHaveAttribute('data-provider', 'anthropic');
  });

  it('SH-C56: the mark alone, for a dense row, named by its provider, its tooltip the same name said again', () => {
    render(
      <>
        <ProviderMark provider="anthropic" alone />
        <ProviderMark provider="kimi" alone />
      </>,
    );
    for (const name of ['Anthropic', 'Kimi Code']) {
      const mark = screen.getByRole('img', { name });
      expect(mark).toHaveAttribute('title', name);
      // A mark, not words: nothing a row's text would read twice.
      expect(mark.textContent).toBe('');
    }
  });

  it('SH-C56: beside words that name the provider already, the mark alone is hidden from a screen reader', () => {
    const { container } = render(
      <span>
        <ProviderMark provider="kimi" decorative />
        Kimi Code web
      </span>,
    );
    const mark = container.querySelector<HTMLElement>('[data-provider="kimi"]')!;
    expect(mark).toHaveAttribute('aria-hidden', 'true');
    expect(mark).not.toHaveAttribute('role');
    expect(mark).not.toHaveAttribute('title');
    expect(screen.queryByRole('img')).toBeNull();
  });

  it('SH-C56: words in place of the bare name keep the mark and the name: an account’s "active for Kimi Code"', () => {
    const { container } = render(<ProviderMark provider="kimi" words="active for Kimi Code" />);
    expect(container.querySelector('[data-provider="kimi"]')).toHaveTextContent(/^active for Kimi Code$/);
    expect(container.querySelector('[data-glyph="kimi"]')).not.toBeNull();
  });

  it('SH-C56: a row that names no provider is Anthropic’s, and a provider with no mark is named in words', () => {
    render(
      <>
        <ProviderMark provider={null} alone />
        <ProviderMark provider={undefined} />
      </>,
    );
    expect(screen.getByRole('img', { name: 'Anthropic' })).toHaveAttribute('data-provider', 'anthropic');
    expect(screen.getByText('Anthropic').closest('[data-provider]')).toHaveAttribute('data-provider', 'anthropic');
    const { container } = render(<ProviderMark provider="openrouter" alone />);
    const named = container.querySelector<HTMLElement>('[data-provider="openrouter"]')!;
    expect(named).toHaveTextContent(/^openrouter$/);
    expect(named.querySelector('[data-glyph]')).toBeNull();
  });

  it('SH-C56: Kimi’s mark is the file for the surface it sits on — the black K on a light theme, the white K on a dark one and in a dark region of a light page', () => {
    const light = render(
      <Theme theme="white">
        <ProviderMark provider="kimi" />
      </Theme>,
    );
    expect(kimiFile(light.container)).toMatch(/kimi-code(?!-dark)/);
    light.unmount();
    const dark = render(
      <Theme theme="g100">
        <ProviderMark provider="kimi" />
      </Theme>,
    );
    expect(kimiFile(dark.container)).toMatch(/kimi-code-dark/);
    dark.unmount();
    // The shell's header is g100 on either theme: a mark inside it is drawn for the header.
    const header = render(
      <Theme theme="white">
        <Theme theme="g100">
          <ProviderMark provider="kimi" />
        </Theme>
      </Theme>,
    );
    expect(kimiFile(header.container)).toMatch(/kimi-code-dark/);
  });

  it('SH-C56: Anthropic’s mark is its file as a mask over the text colour, whatever colour the words around it wear, and neither mark is recoloured or reshaped', () => {
    const { container } = render(<ProviderMark provider="anthropic" />);
    expect(container.querySelector('[data-glyph="anthropic"]')).not.toBeNull();
    const anthropic = block(scss, ".glyph[data-glyph='anthropic'] {");
    expect(anthropic).toMatch(/background-color:\s*var\(--cds-text-primary\);/);
    expect(anthropic).toMatch(/(?<!-)mask:\s*url\('\.\.\/assets\/providers\/anthropic\.svg'\)\s*center\s*\/\s*contain\s+no-repeat;/);
    expect(anthropic).toMatch(/-webkit-mask:\s*url\('\.\.\/assets\/providers\/anthropic\.svg'\)/);
    // Sized to the text and fitted whole; nothing that would change a mark's colour or its shape.
    const glyph = block(scss, '\n.glyph {');
    expect(glyph).toMatch(/inline-size:\s*1em;/);
    expect(glyph).toMatch(/block-size:\s*1em;/);
    expect(glyph).toMatch(/object-fit:\s*contain;/);
    expect(scss).not.toMatch(/\b(filter|fill|opacity|transform|mix-blend-mode|clip-path)\s*:/);
  });

  it('SH-C56: the marks are the owners’ files as published, byte for byte the digests their README records', () => {
    const readme = readFileSync(join(ASSETS, 'README.md'), 'utf8');
    const recorded = new Map([...readme.matchAll(/^- `([\w-]+\.svg)`: `([0-9a-f]{64})`$/gm)].map((m): [string, string] => [m[1]!, m[2]!]));
    expect([...recorded.keys()].sort()).toEqual(['anthropic.svg', 'kimi-code-dark.svg', 'kimi-code.svg']);
    for (const [file, digest] of recorded) expect(createHash('sha256').update(readFileSync(join(ASSETS, file))).digest('hex'), file).toBe(digest);
  });
});
