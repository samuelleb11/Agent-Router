#!/usr/bin/env node
// New for agent-router. The --ar-* colour tokens, held to the dataviz skill's colour method on
// Carbon's own surfaces. Its maths and thresholds are the skill's validator's
// (scripts/validate_palette.js in the dataviz skill), reimplemented here so the check lives in the
// repository; nothing here is eyeballed.
//
//   node scripts/check-palette.mjs        print the report; exit 1 on any FAIL
//
// What it reads:
//   - the --ar-* tokens from src/styles.scss: the `ar-light` mixin is Carbon white's set and
//     `ar-dark` is g100's. Each value must be a literal #rrggbb, so this can read it without
//     compiling Sass; and the mixins must be applied to the theme classes they are named for;
//   - Carbon white's and g100's $background and $layer-01 (and $text-secondary and $interactive,
//     for the collisions below) from @carbon/themes, as installed.
//
// What it checks, for each theme:
//   1. Every token of a set is told apart from every other token of its set, under protanopia,
//      deuteranopia AND tritanopia (Machado, Oliveira & Fernandes 2009, severity 1.0, in linear
//      RGB), as Euclidean distance in OKLab ×100. Every pair, not only neighbours: a legend, a
//      row of chips or a tooltip puts any two of a set side by side. The validator's thresholds:
//      8 is the target; 6 to 8 is the floor band, legal ONLY with a second channel, which each set
//      states below and the report prints; under 6 fails. The validator gates protanopia and
//      deuteranopia and reports tritanopia; this gates all three, at the same thresholds.
//      With full colour vision every pair must be 15 apart (the validator's normal-vision floor,
//      a hard gate that no second channel excuses).
//   2. Every token used as text reaches 4.5:1 (WCAG, normal text) on $background and $layer-01.
//      Only the status text tokens are text; a usage of any other --ar-* token as a CSS `color`
//      anywhere under src/ fails, because text wears Carbon's text tokens.
// And the rest of the validator's method, by the job a set does:
//   - identity sets (categorical): OKLCH lightness inside the theme's band (0.43–0.77 white,
//     0.48–0.67 g100) and chroma at least 0.10, both hard gates;
//   - the token-magnitude ramp (ordinal): monotone lightness, adjacent steps at least 0.06
//     apart, the step nearest the surface at least 2:1 against it, and one hue;
//   - every mark at least 3:1 against both surfaces, or a WARN: legal only where the values are
//     readable another way (visible labels or the table view);
//   - the three collisions the plan names, which these tokens settle, at least 15 apart with
//     full colour vision: session from Opus, subagent from tool results, project from Carbon's
//     secondary text. (The fourth, accent = tools, is moot: the chrome is Carbon's blue. Its
//     distance is printed, not gated.)

import { readdirSync, readFileSync, statSync } from 'node:fs';
import { dirname, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { g100, white } from '@carbon/themes';

const ui = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const STYLES = join(ui, 'src', 'styles.scss');

// ---------------------------------------------------------------- the validator's thresholds

const CVD_TARGET = 8.0;
const CVD_FLOOR = 6.0;
const NORMAL_FLOOR = 15.0;
const MARK_CONTRAST = 3.0;
const TEXT_CONTRAST = 4.5;
const BAND = { white: [0.43, 0.77], g100: [0.48, 0.67] };
const CHROMA_FLOOR = 0.1;
const ORDINAL_MIN_DL = 0.06;
const ORDINAL_LIGHT_FLOOR = 2.0;
const ORDINAL_MAX_HUE_SPREAD = 40;

// ---------------------------------------------------------------- the sets

/// Each token belongs to one set, by the job its colour does. `second` is the channel that
/// carries the same fact when two colours of the set sit in the floor band: what makes that
/// band legal, and what the components that draw these colours promise to keep.
const SETS = [
  {
    name: 'context window',
    job: 'categorical',
    tokens: ['ctx-tools', 'ctx-system', 'ctx-history', 'ctx-results'],
    second: "the share bar's legend names every segment",
  },
  {
    name: 'model family',
    job: 'categorical',
    tokens: ['model-opus', 'model-sonnet', 'model-haiku', 'model-kimi', 'model-other'],
    second: "the activity chart's legend and table view; a model chip carries the model's name",
  },
  {
    name: 'identity',
    job: 'categorical',
    tokens: ['project', 'session', 'subagent'],
    second: "each concept's icon (Folder, Terminal, Fork) beside its name",
  },
  {
    name: 'status fills',
    job: 'status',
    tokens: ['ok-fill', 'warn-fill', 'bad-fill'],
    second: 'a status mark always ships with its icon and its words',
  },
  {
    name: 'session states',
    job: 'status',
    tokens: ['st-busy', 'st-wait', 'st-idle'],
    second: "a state dot is never shown without its words, in Carbon's text tokens",
  },
  {
    name: 'token magnitude',
    job: 'ordinal',
    // Least to most.
    tokens: ['tok-2', 'tok', 'tok-3'],
    second: 'a bar is read by its length and its figure, not by its shade',
  },
];

/// The tokens that are words. Held to 4.5:1, and to nothing about hue: their meaning is the
/// word they colour.
const TEXT = ['ok', 'warn', 'bad'];

const THEMES = [
  { name: 'white', mixin: 'ar-light', selectors: ['.ar-light', '.cds--white'], carbon: white },
  { name: 'g100', mixin: 'ar-dark', selectors: ['.ar-dark', '.cds--g100'], carbon: g100 },
];

// ---------------------------------------------------------------- colour maths (the validator's)

const MACHADO = {
  protanopia: [
    [0.152286, 1.052583, -0.204868],
    [0.114503, 0.786281, 0.099216],
    [-0.003882, -0.048116, 1.051998],
  ],
  deuteranopia: [
    [0.367322, 0.860646, -0.227968],
    [0.280085, 0.672501, 0.047413],
    [-0.01182, 0.04294, 0.968881],
  ],
  tritanopia: [
    [1.255528, -0.076749, -0.178779],
    [-0.078411, 0.930809, 0.147602],
    [0.004733, 0.691367, 0.3039],
  ],
};
const DEFICIENCIES = Object.keys(MACHADO);

const srgb = (hex) => [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16) / 255);
const toLinear = (c) => (c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4);
const linear = (hex) => srgb(hex).map(toLinear);
const luminance = (hex) => {
  const [r, g, b] = linear(hex);
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
};
const contrast = (a, b) => {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
};
function oklab([r, g, b]) {
  const l = Math.cbrt(0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b);
  const m = Math.cbrt(0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b);
  const s = Math.cbrt(0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b);
  return [
    0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s,
    1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s,
    0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s,
  ];
}
const oklch = (hex) => {
  const [L, a, b] = oklab(linear(hex));
  return { L, C: Math.hypot(a, b), H: ((Math.atan2(b, a) * 180) / Math.PI + 360) % 360 };
};
function simulate(hex, deficiency) {
  const [r, g, b] = linear(hex);
  const M = MACHADO[deficiency];
  const clamp = (c) => Math.max(0, Math.min(1, c));
  return M.map((row) => clamp(row[0] * r + row[1] * g + row[2] * b));
}
/// OKLab distance ×100; with no deficiency, as full colour vision sees it.
function deltaE(a, b, deficiency) {
  const x = oklab(deficiency ? simulate(a, deficiency) : linear(a));
  const y = oklab(deficiency ? simulate(b, deficiency) : linear(b));
  return 100 * Math.hypot(x[0] - y[0], x[1] - y[1], x[2] - y[2]);
}

// ---------------------------------------------------------------- reading the tokens

const problems = [];
const warnings = [];
const scss = readFileSync(STYLES, 'utf8');

/// The body of the first block whose head matches, braces balanced.
function body(source, head) {
  const at = source.search(head);
  if (at < 0) return null;
  const open = source.indexOf('{', at);
  let depth = 0;
  for (let i = open; i < source.length; i++) {
    if (source[i] === '{') depth += 1;
    if (source[i] === '}') depth -= 1;
    if (depth === 0) return source.slice(open + 1, i);
  }
  return null;
}

const withoutComments = (text) => text.replace(/\/\*[\s\S]*?\*\//g, '').replace(/\/\/.*$/gm, '');

function tokensOf(theme) {
  const block = body(scss, new RegExp(`@mixin\\s+${theme.mixin}\\s*\\{`));
  if (block === null) {
    problems.push(`src/styles.scss has no \`@mixin ${theme.mixin}\`, which holds Carbon ${theme.name}'s --ar-* tokens.`);
    return new Map();
  }
  const tokens = new Map();
  for (const m of withoutComments(block).matchAll(/--ar-([a-z0-9-]+)\s*:\s*([^;]+);/g)) {
    const [, name, raw] = m;
    const value = raw.trim();
    if (!/^#[0-9a-fA-F]{6}$/.test(value)) {
      problems.push(`--ar-${name} in ${theme.mixin} is ${JSON.stringify(value)}: write it as a literal #rrggbb, so this check can read it.`);
      continue;
    }
    if (tokens.has(name)) problems.push(`--ar-${name} is defined twice in ${theme.mixin}.`);
    tokens.set(name, value.toLowerCase());
  }
  return tokens;
}

/// A mixin applied to the wrong theme class would pass every check below with the other
/// theme's surfaces, so where each is applied is checked too.
function appliedWhereNamed(theme) {
  const blocks = [...withoutComments(scss).matchAll(/([^{};]+)\{([^{}]*)\}/g)];
  for (const selector of theme.selectors) {
    const block = blocks.find(([, head]) => head.split(',').map((s) => s.trim()).includes(selector));
    if (!block) problems.push(`src/styles.scss has no rule for ${selector}, which should apply \`${theme.mixin}\`.`);
    else if (!new RegExp(`@include\\s+${theme.mixin}\\s*;`).test(block[2])) problems.push(`${selector} in src/styles.scss does not apply \`${theme.mixin}\`.`);
  }
  for (const other of THEMES.filter((t) => t !== theme)) {
    for (const selector of theme.selectors) {
      const block = blocks.find(([, head]) => head.split(',').map((s) => s.trim()).includes(selector));
      if (block && new RegExp(`@include\\s+${other.mixin}\\s*;`).test(block[2])) problems.push(`${selector} applies \`${other.mixin}\`, the other theme's tokens.`);
    }
  }
}

const read = new Map(THEMES.map((t) => [t.name, tokensOf(t)]));
for (const t of THEMES) appliedWhereNamed(t);

// Every token belongs somewhere, and both themes define the same ones.
const known = new Set([...SETS.flatMap((s) => s.tokens), ...TEXT]);
for (const [theme, tokens] of read) {
  for (const name of tokens.keys()) if (!known.has(name)) problems.push(`--ar-${name} (${theme}) belongs to no set this check knows: add it to SETS or TEXT in scripts/check-palette.mjs.`);
  for (const name of known) if (!tokens.has(name)) problems.push(`--ar-${name} is not defined for Carbon ${theme}.`);
}

// ---------------------------------------------------------------- the report

const fmt = (n, d = 1) => n.toFixed(d);
const pad = (s, n) => String(s).padEnd(n);
const VERDICT = { pass: 'ok  ', warn: 'WARN', fail: 'FAIL' };
const out = [];
const line = (s = '') => out.push(s);

line('Agent Router colour tokens, checked on Carbon white and g100 (scripts/check-palette.mjs)');
line(`Method: the dataviz skill's validator. CVD: Machado 2009 at severity 1.0; distance is OKLab ΔE ×100.`);
line(`Every pair of a set: target ${CVD_TARGET}, floor ${CVD_FLOOR} (the band between is legal only with a second channel), full colour vision ${NORMAL_FLOOR}. Text: ${TEXT_CONTRAST}:1.`);

for (const theme of THEMES) {
  const tokens = read.get(theme.name);
  const surfaces = [
    ['$background', theme.carbon.background],
    ['$layer-01', theme.carbon.layer01],
  ];
  line();
  line('='.repeat(100));
  line(`Carbon ${theme.name}: $background ${theme.carbon.background}, $layer-01 ${theme.carbon.layer01}   (src/styles.scss, @mixin ${theme.mixin})`);
  line('='.repeat(100));

  for (const set of SETS) {
    const present = set.tokens.filter((t) => tokens.has(t));
    if (present.length !== set.tokens.length) continue;
    line();
    line(`${set.name} (${set.job})`);
    for (const t of set.tokens) {
      const hex = tokens.get(t);
      const { L, C, H } = oklch(hex);
      const marks = surfaces.map(([label, s]) => `${label} ${fmt(contrast(hex, s), 2)}:1`).join('  ');
      line(`  ${pad(`--ar-${t}`, 20)} ${hex}  L ${fmt(L, 3)}  C ${fmt(C, 3)}  H ${pad(fmt(H, 0), 3)}  ${marks}`);

      if (set.job === 'categorical') {
        const [lo, hi] = BAND[theme.name];
        if (L < lo || L > hi) problems.push(`${theme.name}: --ar-${t} ${hex} has OKLCH lightness ${fmt(L, 3)}, outside the band ${lo}–${hi}.`);
        if (C < CHROMA_FLOOR) problems.push(`${theme.name}: --ar-${t} ${hex} has chroma ${fmt(C, 3)}, under ${CHROMA_FLOOR}: it reads as grey and stops doing identity work.`);
      }
      for (const [label, s] of surfaces) {
        const c = contrast(hex, s);
        const floor = set.job === 'ordinal' ? 0 : MARK_CONTRAST;
        if (c < floor) warnings.push(`${theme.name}: --ar-${t} ${hex} is ${fmt(c, 2)}:1 on ${label}, under ${MARK_CONTRAST}:1 for a mark: legal only with visible labels or the table view.`);
      }
    }

    // Every pair, under each deficiency and with full colour vision.
    line(`  ${pad('pair', 32)} ${DEFICIENCIES.map((d) => pad(d, 13)).join('')}${pad('full colour', 12)}verdict`);
    for (let i = 0; i < set.tokens.length; i++) {
      for (let j = i + 1; j < set.tokens.length; j++) {
        const [a, b] = [set.tokens[i], set.tokens[j]];
        const [x, y] = [tokens.get(a), tokens.get(b)];
        const cvd = DEFICIENCIES.map((d) => deltaE(x, y, d));
        const normal = deltaE(x, y);
        const worst = Math.min(...cvd);
        const verdict = worst < CVD_FLOOR || normal < NORMAL_FLOOR ? 'fail' : worst < CVD_TARGET ? 'warn' : 'pass';
        const which = DEFICIENCIES[cvd.indexOf(worst)];
        line(`  ${pad(`${a} / ${b}`, 32)} ${cvd.map((d) => pad(fmt(d), 13)).join('')}${pad(fmt(normal), 12)}${VERDICT[verdict]}`);
        if (verdict === 'fail') {
          if (worst < CVD_FLOOR) problems.push(`${theme.name}: --ar-${a} ${x} and --ar-${b} ${y} are ${fmt(worst)} apart under ${which}, under the floor of ${CVD_FLOOR}.`);
          if (normal < NORMAL_FLOOR) problems.push(`${theme.name}: --ar-${a} ${x} and --ar-${b} ${y} are ${fmt(normal)} apart with full colour vision, under ${NORMAL_FLOOR}.`);
        } else if (verdict === 'warn') {
          warnings.push(`${theme.name}: --ar-${a} and --ar-${b} are ${fmt(worst)} apart under ${which}, in the floor band: legal with its second channel — ${set.second}.`);
        }
      }
    }

    if (set.job === 'ordinal') {
      const steps = set.tokens.map((t) => ({ t, hex: tokens.get(t), ...oklch(tokens.get(t)) }));
      const Ls = steps.map((s) => s.L);
      const rising = Ls.every((L, i) => i === 0 || L > Ls[i - 1]);
      const falling = Ls.every((L, i) => i === 0 || L < Ls[i - 1]);
      if (!rising && !falling) problems.push(`${theme.name}: the ${set.name} ramp's lightness is not monotone: ${Ls.map((L) => fmt(L, 3)).join(', ')}.`);
      for (let i = 1; i < steps.length; i++) {
        const gap = Math.abs(Ls[i] - Ls[i - 1]);
        if (gap < ORDINAL_MIN_DL) problems.push(`${theme.name}: --ar-${steps[i - 1].t} and --ar-${steps[i].t} are ${fmt(gap, 3)} apart in lightness, under ${ORDINAL_MIN_DL}.`);
      }
      // The step nearest the surface: the lightest on white, the darkest on g100.
      const nearest = theme.name === 'white' ? steps.reduce((a, b) => (b.L > a.L ? b : a)) : steps.reduce((a, b) => (b.L < a.L ? b : a));
      for (const [label, s] of surfaces) {
        const c = contrast(nearest.hex, s);
        if (c < ORDINAL_LIGHT_FLOOR) problems.push(`${theme.name}: --ar-${nearest.t}, the ramp's step nearest the surface, is ${fmt(c, 2)}:1 on ${label}, under ${ORDINAL_LIGHT_FLOOR}:1.`);
      }
      const hues = steps.map((s) => s.H);
      let spread = Math.max(...hues) - Math.min(...hues);
      if (spread > 180) spread = 360 - spread;
      if (spread > ORDINAL_MAX_HUE_SPREAD) problems.push(`${theme.name}: the ${set.name} ramp spans ${fmt(spread, 0)}° of hue; a ramp is one hue.`);
      line(`  ramp: lightness ${Ls.map((L) => fmt(L, 3)).join(' → ')} (${rising || falling ? 'monotone' : 'NOT monotone'}), hue spread ${fmt(spread, 0)}°, step nearest the surface ${nearest.hex} at ${surfaces.map(([, s]) => fmt(contrast(nearest.hex, s), 2)).join(' / ')}:1`);
    }
  }

  line();
  line('text tokens');
  for (const t of TEXT) {
    const hex = tokens.get(t);
    if (!hex) continue;
    const ratios = surfaces.map(([label, s]) => [label, contrast(hex, s)]);
    const ok = ratios.every(([, c]) => c >= TEXT_CONTRAST);
    line(`  ${pad(`--ar-${t}`, 20)} ${hex}  ${ratios.map(([label, c]) => `${label} ${fmt(c, 2)}:1`).join('  ')}  ${VERDICT[ok ? 'pass' : 'fail']}`);
    for (const [label, c] of ratios) if (c < TEXT_CONTRAST) problems.push(`${theme.name}: --ar-${t} ${hex} is ${fmt(c, 2)}:1 on ${label}, under ${TEXT_CONTRAST}:1 for text.`);
  }

  // The collisions the plan names.
  line();
  line('collisions the plan names');
  const collision = (label, a, b, gated) => {
    if (!a || !b) return;
    const normal = deltaE(a, b);
    const cvd = Math.min(...DEFICIENCIES.map((d) => deltaE(a, b, d)));
    const verdict = !gated ? 'shown' : normal < NORMAL_FLOOR ? 'FAIL' : 'ok  ';
    line(`  ${pad(label, 44)} ${a} / ${b}   full colour ${pad(fmt(normal), 6)} worst deficiency ${pad(fmt(cvd), 6)} ${verdict}`);
    if (gated && normal < NORMAL_FLOOR) problems.push(`${theme.name}: ${label} are ${fmt(normal)} apart with full colour vision, under ${NORMAL_FLOOR}: the collision is back.`);
  };
  collision('--ar-session / --ar-model-opus', tokens.get('session'), tokens.get('model-opus'), true);
  collision('--ar-subagent / --ar-ctx-results', tokens.get('subagent'), tokens.get('ctx-results'), true);
  collision('--ar-project / Carbon $text-secondary', tokens.get('project'), theme.carbon.textSecondary, true);
  collision('--ar-ctx-tools / Carbon $interactive (moot)', tokens.get('ctx-tools'), theme.carbon.interactive, false);
}

// ---------------------------------------------------------------- tokens used as text

/// Every place under src/ that sets an --ar-* token as a CSS `color`: text. Only the text
/// tokens may be text.
function* files(dir) {
  for (const f of readdirSync(dir)) {
    const path = join(dir, f);
    if (statSync(path).isDirectory()) yield* files(path);
    else if (/\.(s?css|tsx?)$/.test(f)) yield path;
  }
}
const asText = [];
for (const file of files(join(ui, 'src'))) {
  const lines = readFileSync(file, 'utf8').split('\n');
  lines.forEach((text, i) => {
    for (const m of text.matchAll(/(?<![-\w])color\s*:\s*['"`]?var\(\s*--ar-([a-z0-9-]+)/g)) asText.push({ file: relative(ui, file), line: i + 1, token: m[1] });
  });
}
line();
line('=' .repeat(100));
line(`tokens used as text under src/: ${asText.length ? asText.map((u) => `--ar-${u.token} (${u.file}:${u.line})`).join(', ') : 'none'}`);
for (const u of asText) {
  if (!TEXT.includes(u.token)) problems.push(`${u.file}:${u.line} uses --ar-${u.token} as text. Text wears Carbon's text tokens; only ${TEXT.map((t) => `--ar-${t}`).join(', ')} are held to ${TEXT_CONTRAST}:1.`);
}

// ---------------------------------------------------------------- the verdict

line();
if (warnings.length) {
  line(`WARN (${warnings.length}): legal under the method, each with the channel it rests on`);
  for (const w of warnings) line(`  - ${w}`);
  line();
}
if (problems.length) {
  line(`FAIL (${problems.length})`);
  for (const p of [...new Set(problems)]) line(`  - ${p}`);
  console.log(out.join('\n'));
  process.exit(1);
}
line(`PASS: every set told apart under protanopia, deuteranopia and tritanopia, and every text token at ${TEXT_CONTRAST}:1 on both surfaces of both themes.`);
console.log(out.join('\n'));
