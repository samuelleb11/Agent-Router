#!/usr/bin/env node
// Copied from Northstar ui/scripts/check-chunks.mjs at e9c1589.
// Changed: the library whose place it checks is Carbon Charts, not Northstar's graph libraries, and it holds the first load to a byte budget, which Northstar's never had.
//
// Two questions about what the build actually wrote, which no rule about source can answer for
// the bundler.
//
// 1. Are Carbon Charts only where a chart loads them?
//
//    Northstar's charts weigh most of a megabyte with d3 inside, and its charts stylesheet another
//    241 kB. Where they land is decided by the import graph, silently: one static import from a
//    module the shell reaches puts them in the entry chunk, which every page loads before it
//    draws anything, and one from a module a screen imports puts them in that screen's chunk.
//    Nothing fails either way; the console just gets slower to open. The lint rule in
//    eslint.config.mjs allows them only through import(). This asks the same of the output.
//
//    The rule, in chunks: Carbon Charts may be in no chunk that is loaded with the entry, and in
//    no chunk that is loaded with a screen. The first is the module script named in
//    dist/index.html and everything it imports statically; the second is every chunk the entry
//    imports dynamically (the route table's screens) and everything those import statically. So
//    the charts can only arrive through a dynamic import made from inside a screen: their own
//    lazy chunk. Until a slice adds them, @carbon/charts is not installed and nothing can hold
//    it, which this says rather than passing in silence; the rule is in place for the day it is.
//
//    A chunk is recognised by a string the library ships and no minifier rewrites, and once the
//    library is installed the string is first looked for in its own files: a later release that
//    drops it would otherwise make this check pass with the charts anywhere at all.
//
// 2. Is the first load within its budget?
//
//    What a page loads before it can draw: the entry's script and everything it imports
//    statically, and the stylesheets dist/index.html links. The budget is a little above what
//    this scaffold emits, so that a change which grows the first load has to raise the number
//    here, in review, rather than by accident. See BUDGET.
//
// Run after `vite build`, which `pnpm build` does. It reads dist/ and writes nothing.

import { existsSync, readdirSync, readFileSync, statSync } from 'node:fs';
import { dirname, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { gzipSync } from 'node:zlib';

const ui = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const dist = join(ui, 'dist');

/// The first load, in bytes, raw and gzipped. Set a little above what the scaffold emitted on
/// 2026-10-05 (AGEN-0003): 496.8 kB of script, 156.7 kB gzipped (the entry chunk and
/// theme-boot.js), and 697.2 kB of stylesheet, 71.3 kB gzipped, which is Carbon's partials for
/// the components styles.scss lists.
/// Northstar's console, for scale, loads 572 kB of script and 704 kB of stylesheet first. A
/// screen arrives in a chunk of its own and is not counted; a change that grows what every page
/// loads first is, and moves these numbers on purpose or not at all.
/// 2026-10-08 (AGEN-0051): the script budget moves from 520 to 525 kB for the Library, a
/// first-level screen whose rail entry, icon and palette entry every page carries. It took the
/// first load from 518.5 to 521.1 kB; the screen itself is a chunk of its own.
/// 2026-10-10 (AGEN-0092): the gzipped script budget moves from 165 to 167 kB for sources, which
/// every page carries in the palette and the Settings model: a run of models per provider, the
/// Kimi Code account commands, the Models page's route and the provider types. It took the first
/// load to 165.1 kB gzipped (522.1 kB raw); the Models page itself is a chunk of its own.
const BUDGET = {
  script: { raw: 525_000, gzip: 167_000 },
  style: { raw: 720_000, gzip: 75_000 },
};

const LIBRARIES = [
  // A class name @carbon/charts writes on every chart's wrapper. It is a string in its dist, so a
  // minifier keeps it, and @carbon/react ships no such word.
  { name: '@carbon/charts', marker: 'chart-holder', ships: '@carbon/charts' },
];

const fail = (lines) => {
  for (const line of lines) console.error(line);
  process.exit(1);
};

/// Where an installed package lives, or null when it is not installed.
function packageDir(name) {
  let entry;
  try {
    entry = fileURLToPath(import.meta.resolve(name));
  } catch {
    return null;
  }
  for (let dir = dirname(entry); dir !== dirname(dir); dir = dirname(dir)) {
    const manifest = join(dir, 'package.json');
    if (existsSync(manifest) && JSON.parse(readFileSync(manifest, 'utf8')).name === name) return dir;
  }
  return null;
}

/// Whether any of a package's own script files contains a string.
function ships(dir, marker) {
  const walk = (at) =>
    readdirSync(at).some((f) => {
      const path = join(at, f);
      if (f === 'node_modules') return false;
      if (statSync(path).isDirectory()) return walk(path);
      return /\.(m?js|cjs)$/.test(f) && readFileSync(path, 'utf8').includes(marker);
    });
  return walk(dir);
}

const installed = LIBRARIES.filter((lib) => packageDir(lib.ships) !== null);
for (const lib of installed) {
  const dir = packageDir(lib.ships);
  if (!ships(dir, lib.marker)) {
    fail([
      `check-chunks: ${relative(ui, dir)} no longer contains "${lib.marker}",`,
      `  so this check cannot see ${lib.name} in a chunk. Pick a string the release does ship.`,
    ]);
  }
}

const html = join(dist, 'index.html');
if (!existsSync(html)) fail(['check-chunks: no dist/index.html; run it after `vite build`.']);
const page = readFileSync(html, 'utf8');

/// Chunks are named by their path under dist/, as the browser asks for them.
const entries = [...page.matchAll(/<script\b[^>]*\btype="module"[^>]*\bsrc="\/([^"]+\.js)"/g)].map((m) => m[1]);
if (entries.length === 0) fail(['check-chunks: dist/index.html names no module script to start from.']);
const classic = [...page.matchAll(/<script\b(?![^>]*\btype="module")[^>]*\bsrc="\/([^"]+\.js)"/g)].map((m) => m[1]);
const stylesheets = [...page.matchAll(/<link\b[^>]*\brel="stylesheet"[^>]*\bhref="\/([^"]+\.css)"/g)].map((m) => m[1]);

const sources = new Map();
const source = (chunk) => {
  if (!sources.has(chunk)) {
    const file = join(dist, chunk);
    if (!existsSync(file)) fail([`check-chunks: ${chunk} is imported but is not in dist/.`]);
    sources.set(chunk, readFileSync(file, 'utf8'));
  }
  return sources.get(chunk);
};

/// Rollup writes a static import as `import{a as b}from"./x.js"` or `import"./x.js"`, and a
/// dynamic one as `import("./x.js")`, which the first pattern cannot match for its parenthesis.
const STATIC = /(?:\bfrom|\bimport)\s*["'](\.{1,2}\/[^"']+\.js)["']/g;
const DYNAMIC = /\bimport\(\s*["'](\.{1,2}\/[^"']+\.js)["']\s*\)/g;
const imports = (chunk, pattern) =>
  [...source(chunk).matchAll(pattern)].map((m) => (m[1].startsWith('/') ? m[1].slice(1) : relative(dist, resolve(dist, dirname(chunk), m[1]))));

/// Every chunk loaded with `from`, each with the chain that loads it, for the failure to name.
const closure = (from) => {
  const chains = new Map(from.map((chunk) => [chunk, [chunk]]));
  const queue = [...from];
  while (queue.length > 0) {
    const chunk = queue.shift();
    for (const next of imports(chunk, STATIC)) {
      if (chains.has(next)) continue;
      chains.set(next, [...chains.get(chunk), next]);
      queue.push(next);
    }
  }
  return chains;
};

const main = closure(entries);

/// The screens: every chunk the main bundle imports dynamically, with the chain that reaches it.
const screens = new Map();
for (const [chunk, chain] of main) {
  for (const screen of imports(chunk, DYNAMIC)) {
    if (!main.has(screen) && !screens.has(screen)) screens.set(screen, [...chain, `import() ${screen}`]);
  }
}
// The route table is lazy (src/app/routeTable.tsx), so a build with no dynamic import from the main
// bundle means these patterns no longer read what Rollup writes, and every answer below is empty.
if (screens.size === 0) fail(['check-chunks: found no screen chunk the main bundle imports; the import patterns are stale.']);

const loadedFirst = new Map(main);
for (const [screen, reached] of screens) {
  for (const [chunk, chain] of closure([screen])) {
    if (!loadedFirst.has(chunk)) loadedFirst.set(chunk, [...reached, ...chain.slice(1)]);
  }
}

// ---------------------------------------------------------------- 1. where the charts are

const holding = (chunk) => LIBRARIES.filter((lib) => source(chunk).includes(lib.marker)).map((lib) => lib.name);
const wrong = [...loadedFirst].filter(([chunk]) => holding(chunk).length > 0);
if (wrong.length > 0) {
  fail([
    'check-chunks: Carbon Charts are in a chunk the entry or a screen loads:',
    ...wrong.flatMap(([chunk, chain]) => [`  ${chunk}: ${holding(chunk).join(', ')}`, `    loaded by ${chain.join(' -> ')}`]),
    "  Load them only with import(), so they arrive in a chunk of their own when a chart is drawn.",
  ]);
}

// ---------------------------------------------------------------- 2. the first load's budget

const kb = (bytes) => `${(bytes / 1000).toFixed(1)} kB`;
const size = (files, read) => {
  const raw = files.reduce((n, f) => n + Buffer.byteLength(read(f)), 0);
  const gzip = files.reduce((n, f) => n + gzipSync(read(f)).length, 0);
  return { raw, gzip };
};
const readDist = (f) => {
  const file = join(dist, f);
  if (!existsSync(file)) fail([`check-chunks: dist/index.html loads ${f}, which is not in dist/.`]);
  return readFileSync(file);
};
const script = size([...main.keys(), ...classic], readDist);
const style = size(stylesheets, readDist);
const over = [
  ...['raw', 'gzip'].filter((k) => script[k] > BUDGET.script[k]).map((k) => `  script ${k}: ${kb(script[k])}, over its budget of ${kb(BUDGET.script[k])}`),
  ...['raw', 'gzip'].filter((k) => style[k] > BUDGET.style[k]).map((k) => `  stylesheet ${k}: ${kb(style[k])}, over its budget of ${kb(BUDGET.style[k])}`),
];
if (over.length > 0) {
  fail([
    'check-chunks: the first load is over its budget:',
    ...over,
    `  It loads ${[...main.keys(), ...classic, ...stylesheets].join(', ')}.`,
    '  Move what grew into a screen\'s chunk, or raise BUDGET in scripts/check-chunks.mjs and say why.',
  ]);
}

if (installed.length === 0) {
  console.log(`check-chunks: @carbon/charts is not installed, so no chunk can hold it; ${screens.size} screens and the entry checked for it all the same`);
} else {
  console.log(`check-chunks: no chunk loaded with the entry or any of ${screens.size} screens holds Carbon Charts`);
}
console.log(`check-chunks: first load ${kb(script.raw)} of script (${kb(script.gzip)} gzipped, budget ${kb(BUDGET.script.raw)} / ${kb(BUDGET.script.gzip)})`);
console.log(`check-chunks: first load ${kb(style.raw)} of stylesheet (${kb(style.gzip)} gzipped, budget ${kb(BUDGET.style.raw)} / ${kb(BUDGET.style.gzip)})`);
