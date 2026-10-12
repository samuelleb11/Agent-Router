// @vitest-environment node
// New for agent-router: the colours the Overview's figures wear (docs/web-console-contracts.md, "Layout
// and identity"): the meaningful ones kept as tokens over Carbon's theme, and checked with the dataviz
// palette validator rather than by eye.
import { execFileSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

const ui = join(__dirname, '../../..');
const plain = (s: string) => s.replace(/\/\*[\s\S]*?\*\//g, '').replace(/\/\/.*$/gm, '');
const styles = plain(readFileSync(join(ui, 'src/styles.scss'), 'utf8'));
const tokens = (mixin: string): Map<string, string> => {
  const body = new RegExp(`@mixin ${mixin} \\{([^}]*)\\}`).exec(styles)?.[1] ?? '';
  return new Map([...body.matchAll(/(--ar-[a-z0-9-]+)\s*:\s*(#[0-9a-f]{6})\s*;/gi)].map((m) => [m[1]!, m[2]!.toLowerCase()]));
};

/// Relative luminance, for "more is darker" on a light surface and lighter on a dark one.
function luminance(hex: string): number {
  const c = [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16) / 255).map((v) => (v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4));
  return 0.2126 * c[0]! + 0.7152 * c[1]! + 0.0722 * c[2]!;
}

const MEANINGS = {
  context: ['--ar-ctx-tools', '--ar-ctx-system', '--ar-ctx-history', '--ar-ctx-results'],
  ramp: ['--ar-tok-2', '--ar-tok', '--ar-tok-3'],
  identity: ['--ar-project', '--ar-session', '--ar-subagent'],
  family: ['--ar-model-opus', '--ar-model-sonnet', '--ar-model-haiku', '--ar-model-kimi', '--ar-model-other'],
  status: ['--ar-ok', '--ar-warn', '--ar-bad', '--ar-ok-fill', '--ar-warn-fill', '--ar-bad-fill'],
} as const;

describe('the Overview’s colours', () => {
  it('OV-C7: the colours that carry meaning are tokens over Carbon’s theme, in both themes, and Carbon owns the neutrals and the chrome', () => {
    for (const theme of ['ar-light', 'ar-dark']) {
      const set = tokens(theme);
      for (const [meaning, names] of Object.entries(MEANINGS)) for (const name of names) expect(set.has(name), `${theme}: ${meaning} ${name}`).toBe(true);
      // The ramp is one hue, more is darker on white and lighter on g100.
      const [less, mid, more] = MEANINGS.ramp.map((n) => luminance(set.get(n)!));
      if (theme === 'ar-light') expect(less! > mid! && mid! > more!, theme).toBe(true);
      else expect(less! < mid! && mid! < more!, theme).toBe(true);
    }
    // Nothing the Overview draws picks a neutral or a chrome colour of its own: those are Carbon's.
    for (const file of ['Overview.module.scss', 'ActivityChart.module.scss']) {
      const scss = plain(readFileSync(join(__dirname, file), 'utf8'));
      expect(scss, file).not.toMatch(/#[0-9a-f]{3,8}\b|rgba?\(|hsla?\(/i);
      const colours = [...scss.matchAll(/(?<![-\w])(?:color|background)\s*:\s*([^;]+);/g)].map((m) => m[1]!.trim());
      for (const c of colours) expect(c, file).toMatch(/^(var\(--cds-[a-z0-9-]+\)|var\(--ar-[a-z0-9-]+\)|transparent)$/);
    }
  });

  it('OV-C9: every meaningful colour passes the dataviz palette validator, in both themes, on the surfaces it is drawn on', () => {
    const report = execFileSync(process.execPath, [join(ui, 'scripts/check-palette.mjs')], { cwd: ui, encoding: 'utf8' });
    expect(report).toMatch(/^Carbon white: \$background #ffffff, \$layer-01 #f4f4f4/m);
    expect(report).toMatch(/^Carbon g100: \$background #161616, \$layer-01 #262626/m);
    expect(report).toMatch(/^PASS: every set told apart under protanopia, deuteranopia and tritanopia, and every text token at 4\.5:1 on both surfaces of both themes\.$/m);
    expect(report).not.toMatch(/^FAIL/m);
  });
});
