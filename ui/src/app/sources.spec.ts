// New for agent-router: the Shell contracts that are facts about the console's own files rather than
// about a render (docs/web-console-contracts.md, "Shell") — the one document's viewport, the fonts it
// serves itself, the one breakpoint, what `hidden` means, and which colours the console may define and
// how they are checked. What only a browser can show of the same rules is asserted on every shot by
// scripts/shell-checks.mjs.
import { execFileSync } from 'node:child_process';
import { existsSync, readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';
import { breakpoints } from '@carbon/layout';
import { g100, white } from '@carbon/themes';
import { describe, expect, it, vi } from 'vitest';
import { NARROW_QUERY } from '~/lib/viewport';

/// Waits and limits generous enough for a machine busy with other work: a timeout here is a slow
/// machine, never a pass.
vi.setConfig({ testTimeout: 30_000 });

const ui = join(__dirname, '..', '..');
const src = join(ui, 'src');
const read = (path: string) => readFileSync(path, 'utf8');

/// Every file under src/ with one of `extensions`, specs left out.
function files(extensions: RegExp, dir = src): string[] {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) return files(extensions, path);
    return extensions.test(name) && !/\.spec\.(ts|tsx)$/.test(name) ? [path] : [];
  });
}
const stylesheets = () => files(/\.scss$/);
const scripts = () => files(/\.(ts|tsx)$/).filter((p) => !/test-(utils|setup)\.tsx?$/.test(p) && !p.includes(`${join('model', 'fixtures')}`));
/// Sass without its comments, where a colour or a rule is often named.
const sass = (path: string) =>
  read(path)
    .replace(/\/\*[\s\S]*?\*\//g, '')
    .replace(/\/\/.*$/gm, '');
/// The body of the first block opened by `head` in `source`, braces balanced.
function block(source: string, head: string): string {
  const start = source.indexOf(head);
  if (start < 0) return '';
  const open = source.indexOf('{', start);
  let depth = 0;
  for (let i = open; i < source.length; i++) {
    if (source[i] === '{') depth += 1;
    if (source[i] === '}') depth -= 1;
    if (depth === 0) return source.slice(open + 1, i);
  }
  return '';
}

const STYLES = sass(join(src, 'styles.scss'));

describe('the one document', () => {
  it('SH-C47: carries the live view’s viewport settings, so Sessions keeps its column, its safe areas and its keyboard pin inside the shell: viewport-fit=cover and interactive-widget=resizes-content', () => {
    const page = new DOMParser().parseFromString(read(join(ui, 'index.html')), 'text/html');
    const viewport = page.querySelectorAll('meta[name="viewport"]');
    expect(viewport).toHaveLength(1);
    const settings = viewport[0]!
      .getAttribute('content')!
      .split(',')
      .map((s) => s.trim());
    expect(settings).toEqual(expect.arrayContaining(['width=device-width', 'initial-scale=1', 'viewport-fit=cover', 'interactive-widget=resizes-content']));
  });
});

describe('what hidden means', () => {
  it('SH-C29: an element with the hidden attribute is not displayed whatever class it carries: the page’s own rule outranks every class', () => {
    expect(block(STYLES, '\n[hidden] {')).toMatch(/^\s*display:\s*none\s*!important;\s*$/);
  });
});

describe('the type', () => {
  it('SH-C41: is IBM Plex Sans and Mono, served from the console’s own files, never a font service, and no stylesheet sets text under 12 px', () => {
    // Carbon's own faces would come from IBM's CDN: they are switched off, and the console declares
    // the four it uses.
    expect(read(join(src, 'styles.scss'))).toMatch(/\$css--font-face:\s*false/);
    const fonts = sass(join(src, 'fonts.scss'));
    const faces = [...fonts.matchAll(/@font-face\s*\{([^}]*)\}/g)].map((m) => ({
      family: /font-family:\s*'([^']+)'/.exec(m[1]!)?.[1],
      weight: /font-weight:\s*(\d+)/.exec(m[1]!)?.[1],
      urls: [...m[1]!.matchAll(/url\('([^']+)'\)/g)].map((u) => u[1]!),
    }));
    expect(faces.map((f) => `${f.family} ${f.weight}`)).toEqual(['IBM Plex Sans 300', 'IBM Plex Sans 400', 'IBM Plex Sans 600', 'IBM Plex Mono 400']);
    for (const face of faces) {
      expect(face.urls).toHaveLength(1);
      expect(face.urls[0]).toMatch(/^\/fonts\/IBMPlex(Sans|Mono)-\w+\.woff2$/);
      // Served from public/, which the build copies into what the router embeds.
      expect(existsSync(join(ui, 'public', face.urls[0]!))).toBe(true);
    }
    // No stylesheet reaches off the router for anything, a font or otherwise.
    const remote = stylesheets().filter((p) => /url\(\s*['"]?(https?:)?\/\//.test(sass(p)) || /@import\s+['"]?(https?:)?\/\//.test(sass(p)));
    expect(remote.map((p) => relative(src, p))).toEqual([]);
    // No size is written by hand under Carbon's 12 px floor; the type scale itself has none under it.
    const small = stylesheets().flatMap((p) =>
      [...sass(p).matchAll(/font-size:\s*([\d.]+)(px|rem|em)\b/g)].filter((m) => Number(m[1]) * (m[2] === 'px' ? 1 : 16) < 12).map((m) => `${relative(src, p)}: ${m[0]}`),
    );
    expect(small).toEqual([]);
  });
});

describe('both widths', () => {
  it('SH-C43: the console switches between a phone and a desk at one breakpoint, Carbon’s md at 41.98 rem, read by script through one module and by every stylesheet through Carbon’s mixin', () => {
    // Carbon's breakpoint-down('md') asks for 0.02rem under md's width, and so does the script.
    const md = parseFloat(breakpoints.md.width);
    expect(breakpoints.md.width).toBe('42rem');
    expect(NARROW_QUERY).toBe(`(max-width: ${Math.round((md - 0.02) * 100) / 100}rem)`);

    // No script but lib/viewport.ts asks the browser about a width, or measures one against a number.
    const viewport = join(src, 'lib', 'viewport.ts');
    const asking = scripts().filter((p) => p !== viewport && /matchMedia\([^)]*width/.test(read(p)));
    const measuring = scripts().filter((p) => /\b(inner|outer|client)Width\s*[<>]=?\s*\d|\d\s*[<>]=?\s*(window\.)?(inner|outer|client)Width\b|screen\.width/.test(read(p)));
    expect([...asking, ...measuring].map((p) => relative(src, p))).toEqual([]);

    // No stylesheet writes a width query by hand: each goes through Carbon's mixin, at md. The one
    // other width is SE-C201's, where the session list joins the open session from 1,056 px; it is
    // not a phone or a desk, and nothing in script reads it.
    const handWritten = stylesheets().filter((p) => /@media[^{]*\b(min|max)-width/.test(sass(p)));
    expect(handWritten.map((p) => relative(src, p))).toEqual([]);
    const at = stylesheets().flatMap((p) => [...sass(p).matchAll(/breakpoint(?:-down|-up)?\(\s*'(\w+)'/g)].map((m) => `${relative(src, p)} ${m[1]}`));
    expect(at.length).toBeGreaterThan(10);
    expect(at.filter((x) => !x.endsWith(' md'))).toEqual([join('screens', 'Sessions', 'Sessions.module.scss') + ' lg', join('screens', 'Sessions', 'Sessions.module.scss') + ' lg']);
    expect(scripts().filter((p) => /\b(1056|66rem)\b/.test(read(p)))).toEqual([]);
  });
});

/// The --ar-* tokens of a mixin in styles.scss, by name.
function tokens(mixin: 'ar-light' | 'ar-dark'): Map<string, string> {
  return new Map([...block(STYLES, `@mixin ${mixin} {`).matchAll(/--ar-([\w-]+):\s*([^;]+);/g)].map((m) => [m[1]!, m[2]!.trim().toLowerCase()]));
}

/// WCAG's contrast ratio between two #rrggbb colours.
function contrast(a: string, b: string): number {
  const lum = (hex: string) => {
    const [r, g, bl] = [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16) / 255).map((v) => (v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4));
    return 0.2126 * r! + 0.7152 * g! + 0.0722 * bl!;
  };
  const [hi, lo] = [lum(a), lum(b)].sort((x, y) => y - x);
  return (hi! + 0.05) / (lo! + 0.05);
}

describe('colour', () => {
  const THEMES = [
    ['ar-light', white],
    ['ar-dark', g100],
  ] as const;

  it('SH-C37: Carbon’s theme owns every neutral and all the chrome: the console defines only its --ar-* colours, one meaning each, with the accent Carbon’s blue and the collisions settled', () => {
    // No stylesheet writes a colour of its own, or redefines one of Carbon's, outside the two token
    // sets; and no script paints one.
    const outside = stylesheets().flatMap((p) => {
      const body = relative(src, p) === 'styles.scss' ? STYLES.replace(block(STYLES, '@mixin ar-light {'), '').replace(block(STYLES, '@mixin ar-dark {'), '') : sass(p);
      return [...body.matchAll(/#[0-9a-fA-F]{3,8}\b|\b(?:rgba?|hsla?|oklch|oklab|lab|lch)\(|--cds-[\w-]+\s*:/g)].map((m) => `${relative(src, p)}: ${m[0]}`);
    });
    expect(outside).toEqual([]);
    const painted = scripts().filter((p) => /['"`]#[0-9a-fA-F]{6}\b|\brgba?\(/.test(read(p)));
    expect(painted.map((p) => relative(src, p))).toEqual([]);

    for (const [mixin, carbon] of THEMES) {
      const set = tokens(mixin);
      expect([...set.values()].every((v) => /^#[0-9a-f]{6}$/.test(v)), mixin).toBe(true);
      // One meaning per colour: two tokens share a value only where they are one status's word and
      // its mark.
      const byValue = new Map<string, string[]>();
      for (const [name, value] of set) byValue.set(value, [...(byValue.get(value) ?? []), name]);
      const shared = [...byValue.values()].filter((names) => names.length > 1);
      for (const names of shared) expect(names.map((n) => n.replace(/-fill$/, '')).every((n, _, all) => n === all[0] && ['ok', 'warn', 'bad'].includes(n)), `${mixin}: ${names.join(', ')}`).toBe(true);
      // The interactive accent is Carbon's own blue, and indigo stays the tools colour.
      expect([...set.keys()].some((n) => /accent|interactive|link/.test(n)), mixin).toBe(false);
      expect(set.get('ctx-tools'), mixin).not.toBe(carbon.interactive.toLowerCase());
      expect(set.get('ctx-tools'), mixin).not.toBe(carbon.linkPrimary.toLowerCase());
      // The collisions today's pages carry, settled.
      expect(set.get('subagent'), mixin).not.toBe(set.get('ctx-results'));
      expect(set.get('session'), mixin).not.toBe(set.get('model-opus'));
      expect(set.get('project'), mixin).not.toBe(carbon.textSecondary.toLowerCase());
    }
  });

  it('SH-C38: every token passes the dataviz palette validator in both themes against Carbon’s own surfaces, and the text the shell adds meets WCAG AA', () => {
    // The validator, as `pnpm palette:check` runs it: any FAIL exits non-zero, and this throws.
    const report = execFileSync(process.execPath, [join(ui, 'scripts', 'check-palette.mjs')], { cwd: ui, encoding: 'utf8' });
    expect(report).toMatch(/white/);
    expect(report).toMatch(/g100/);
    expect(report).not.toMatch(/^FAIL/m);

    // The header is g100 on either page theme: the address it prints in Carbon's secondary text, and
    // the stale badge's inverse text on the console's warning fill.
    const dark = tokens('ar-dark');
    expect(contrast(g100.textSecondary, g100.background)).toBeGreaterThanOrEqual(4.5);
    expect(contrast(g100.textInverse, dark.get('warn-fill')!)).toBeGreaterThanOrEqual(4.5);
  });
});
