#!/usr/bin/env node
// Copied from Northstar ui/scripts/notices.mjs at e9c1589.
// Changed: Northstar's declarations for cytoscape, dagre, heap and lodash are gone, and so is the one for flatpickr's tslib, which this console does not bundle; Carbon Charts' are kept, for the Overview's activity chart; there is no generated client to credit; the notices name the router.
//
// The console's third-party notices, generated from what the production build bundles.
//
//   node scripts/notices.mjs           write public/THIRD-PARTY-NOTICES.txt and public/fonts/LICENSE.txt
//   node scripts/notices.mjs --check   write nothing; exit 1 if either is not what it would write
//
// Both files are in public/, so `vite build` copies them into dist/ and the router serves them
// beside the console: /THIRD-PARTY-NOTICES.txt and /fonts/LICENSE.txt.
//
// WHAT IS LISTED is what the build puts in dist/, asked of the build itself rather than of
// package.json. The production dependency tree is the wrong question both ways: it holds packages
// that ship nothing (type declarations, telemetry, the fonts' other scripts, a Sass compiler), and
// it misses code that arrives without being a dependency (Vite's own helpers, and libraries a
// package compiles into its dist). So this runs
// the production build in memory, with the project's own vite.config.ts and nothing written, and
// collects:
//
//   - every module rendered into a chunk, a worker's included, by the package it is in;
//     a module tree-shaken to nothing renders zero bytes and is not counted;
//   - every stylesheet Sass or the CSS pipeline read, by the package it is in (Carbon's styles
//     arrive this way, compiled, and are no module of the bundle);
//   - every package a bundled file's source map names as compiled into it, followed through the
//     maps of the files it names. Carbon Charts, when a slice adds them, compile in d3, lodash-es
//     and DOMPurify among others: none of them is a module of the bundle, and all of them are in
//     it. A map names a package but not its release: where the dist carries a licence banner with
//     a version (`@license DOMPurify 3.4.14`), that is the version, and an installed copy of
//     another release does not stand in for it; elsewhere it is the copy installed nearest the
//     host, and the notices say so;
//   - the legal comments (/*! and @license) of every rendered module that has no source map,
//     whose copyright holders must each be named by the package's own licence or by what it is
//     declared to compile in: a package that compiles others in without a map says so nowhere
//     else, and the minifier strips these comments from the bundle;
//   - Vite's helpers, and @rollup/plugin-commonjs's, which Vite bundles;
//   - the fonts in public/fonts, each matched by its bytes to the installed IBM Plex package it
//     was copied from.
//
// What no map or module can say is declared below (INLINED, SOURCE_ALIASES, VIRTUAL), and every
// declaration is checked against the build: one that no longer applies fails the run, as does a
// source, a virtual module, a font, a copyright holder or a licence banner nothing accounts for,
// and a directory in scripts/licences nothing reads. A package whose licence offers a choice
// is listed under the permissive one, and the alternative's own file is left out.
//
// `--check` is part of `pnpm run build`, so the console cannot be built with notices that no longer
// describe it. It writes nothing.

import { createHash } from 'node:crypto';
import { existsSync, readdirSync, readFileSync, realpathSync, statSync, writeFileSync } from 'node:fs';
import { basename, dirname, join, relative, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'vite';

const ui = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const NOTICES = join(ui, 'public', 'THIRD-PARTY-NOTICES.txt');
const FONTS = join(ui, 'public', 'fonts');
const FONT_LICENCE = join(FONTS, 'LICENSE.txt');
// Licence files of packages that are compiled in but not installed, from their published tarballs.
const VENDORED = join(ui, 'scripts', 'licences');

// ---------------------------------------------------------------- what the build cannot say

/// Packages compiled into a bundled package's dist whose release no installed copy is, keyed by
/// the package that compiles them in: not installed at all, so no source map here can be followed
/// into them, or installed at a release other than the one the dist carries. Each says how its
/// version is known (`basis`), and its licence files are in VENDORED.
const INLINED = {
  // @carbon/charts's dist compiles in DOMPurify, and its banner there says 3.4.14; the copy
  // installed beside it is 3.4.15. The banner is checked against this on every run. Its licence
  // files are LICENSE and LICENSE-MPL from the published 3.4.14 tarball, as Northstar keeps them.
  // Carbon Charts also compile in @carbon/utils-position 1.3.0, which is installed but ships no
  // licence file: its LICENSE in VENDORED is the one at the root of
  // carbon-design-system/carbon-utils-position. Northstar's entries for cytoscape, cytoscape-dagre,
  // @dagrejs/dagre and flatpickr's tslib are left out: this console bundles none of them, and
  // renders no date picker, so the build holds no flatpickr.
  '@carbon/charts@1.27.20': {
    dompurify: { version: '3.4.14', license: '(MPL-2.0 OR Apache-2.0)', repository: 'https://github.com/cure53/DOMPurify', author: 'Dr.-Ing. Mario Heiderich, Cure53', basis: 'the version its licence banner in that dist names' },
  },
};

/// Sources a package's map names by a path in the repository it was built in, which is another
/// installed package: @carbon/charts-react's monorepo keeps @carbon/charts in ../core.
const SOURCE_ALIASES = {
  '@carbon/charts-react': [{ prefix: '../../core/', package: '@carbon/charts' }],
};

/// Rollup's virtual modules that render into the bundle.
const VIRTUAL = {
  '\0vite/preload-helper.js': { package: 'vite', how: "Vite's preload helper" },
  '\0vite/modulepreload-polyfill.js': { package: 'vite', how: "Vite's module-preload polyfill" },
  '\0commonjsHelpers.js': { package: '@rollup/plugin-commonjs', how: 'its CommonJS interop helper, from the copy Vite bundles' },
};

/// The order a choice of licences is made in: the first the package offers.
const PREFERENCE = ['MIT', 'Apache-2.0', 'ISC', 'BSD-3-Clause', 'BSD-2-Clause', '0BSD', 'Zlib', 'BSL-1.0', 'Unicode-3.0', 'Unlicense', 'CC0-1.0'];
/// Permissive: no condition beyond keeping the notice (and, for Apache-2.0, its NOTICE).
const PERMISSIVE = new Set([...PREFERENCE, 'MIT-0', 'Python-2.0', 'BlueOak-1.0.0']);

// ---------------------------------------------------------------- small helpers

const readJson = (file) => JSON.parse(readFileSync(file, 'utf8'));
const keyOf = (name, version) => name + '@' + version;
const real = (path) => {
  try {
    return realpathSync(path);
  } catch {
    return null;
  }
};

/// The installed package a file belongs to: the nearest package.json with a name and a version,
/// below a node_modules directory.
function packageDir(file) {
  let dir = dirname(file);
  while (dir.includes(sep + 'node_modules' + sep)) {
    const manifest = join(dir, 'package.json');
    if (existsSync(manifest)) {
      const json = readJson(manifest);
      if (json.name && json.version) return dir;
    }
    dir = dirname(dir);
  }
  return null;
}

/// Where `name` resolves from `dir`, as Node would look for it.
function resolveFrom(dir, name) {
  for (let at = real(dir); at; at = dirname(at) === at ? null : dirname(at)) {
    const candidate = join(at, 'node_modules', name);
    if (existsSync(join(candidate, 'package.json'))) return real(candidate);
  }
  return null;
}

const dependencyCache = new Map();
/// The installed packages a package depends on, peers included.
function dependencies(dir) {
  if (!dependencyCache.has(dir)) {
    const json = readJson(join(dir, 'package.json'));
    const names = Object.keys({ ...json.dependencies, ...json.optionalDependencies, ...json.peerDependencies });
    dependencyCache.set(dir, [...new Set(names.map((n) => resolveFrom(dir, n)).filter(Boolean))].sort());
  }
  return dependencyCache.get(dir);
}

/// The installed copies of `name` fewest dependency steps from the package in `dir`, the package
/// itself excluded: every one at that distance, since which of them a build hoisted is not known.
function nearest(dir, name) {
  let frontier = [real(dir)];
  const seen = new Set(frontier);
  while (frontier.length) {
    const hits = new Set();
    const next = [];
    for (const at of frontier)
      for (const dep of dependencies(at)) {
        if (readJson(join(dep, 'package.json')).name === name) hits.add(dep);
        if (!seen.has(dep)) {
          seen.add(dep);
          next.push(dep);
        }
      }
    if (hits.size) return [...hits].sort();
    frontier = next;
  }
  return [];
}

/// A file's source map, from its sourceMappingURL or a .map beside it.
function sourceMap(file) {
  let code;
  try {
    code = readFileSync(file, 'utf8');
  } catch {
    return null;
  }
  const url = code.match(/\/[/*][#@] sourceMappingURL=([^\s*]+)[\s*/]*$/)?.[1];
  try {
    if (url?.startsWith('data:')) {
      const body = url.slice(url.indexOf(',') + 1);
      const text = url.slice(0, url.indexOf(',')).endsWith(';base64') ? Buffer.from(body, 'base64').toString('utf8') : decodeURIComponent(body);
      return { map: JSON.parse(text), dir: dirname(file) };
    }
    const mapFile = url ? resolve(dirname(file), decodeURIComponent(url)) : file + '.map';
    if (!existsSync(mapFile)) return null;
    return { map: readJson(mapFile), dir: dirname(mapFile) };
  } catch {
    return null;
  }
}

// ---------------------------------------------------------------- SPDX

/// Parse an SPDX expression (and the older `A/B` form) into a tree of {op, args} and identifiers.
function parseSpdx(expression) {
  const tokens = expression.replace(/\//g, ' OR ').match(/\(|\)|[^\s()]+/g) ?? [];
  let at = 0;
  const primary = () => {
    const token = tokens[at++];
    if (token === '(') {
      const inner = or();
      at++;
      return inner;
    }
    if (tokens[at] === 'WITH') {
      at++;
      return token + ' WITH ' + tokens[at++];
    }
    return token;
  };
  const and = () => {
    const args = [primary()];
    while (tokens[at] === 'AND') {
      at++;
      args.push(primary());
    }
    return args.length === 1 ? args[0] : { op: 'AND', args };
  };
  const or = () => {
    const args = [and()];
    while (tokens[at] === 'OR') {
      at++;
      args.push(and());
    }
    return args.length === 1 ? args[0] : { op: 'OR', args };
  };
  return or();
}

const ids = (tree) => (typeof tree === 'string' ? [tree] : tree.args.flatMap(ids));
const rank = (id) => {
  const i = PREFERENCE.indexOf(id);
  return i === -1 ? PREFERENCE.length : i;
};

/// The licences a package is used under: all of an AND, the most preferred branch of an OR.
function choose(tree) {
  if (typeof tree === 'string') return [tree];
  if (tree.op === 'AND') return [...new Set(tree.args.flatMap(choose))];
  const options = tree.args.map(choose);
  options.sort((a, b) => Math.max(...a.map(rank)) - Math.max(...b.map(rank)));
  return options[0];
}

/// The licence a licence file is named for, when its name says.
function namedFor(file) {
  const name = file.toUpperCase();
  if (/MPL/.test(name)) return 'MPL-2.0';
  if (/APACHE/.test(name)) return 'Apache-2.0';
  if (/(^|[^A-Z])MIT([^A-Z]|$)/.test(name)) return 'MIT';
  if (/(^|[^A-Z])ISC([^A-Z]|$)/.test(name)) return 'ISC';
  if (/ZLIB/.test(name)) return 'Zlib';
  if (/BOOST|BSL/.test(name)) return 'BSL-1.0';
  if (/0BSD/.test(name)) return '0BSD';
  if (/^UNLICENSE/.test(name)) return 'Unlicense';
  return null;
}

// ---------------------------------------------------------------- the packages

const LICENCE_FILE = /^(licen[cs]e|copying|unlicense)/i;
const NOTICE_FILE = /^(notice|copyright|authors|third[-_]?party)/i;
const NOT_TEXT = /\.(js|cjs|mjs|ts|json|map|html)$/i;

const normalise = (text) =>
  text
    .replace(/^﻿/, '')
    .replace(/\r\n?/g, '\n')
    .split('\n')
    .map((line) => line.replace(/\s+$/, ''))
    .join('\n')
    .replace(/^\n+|\n+$/g, '');

function repositoryOf(json) {
  let url = typeof json.repository === 'string' ? json.repository : json.repository?.url;
  if (!url) return json.homepage ?? null;
  url = url.replace(/^git\+/, '').replace(/\.git$/, '').replace(/^git:\/\//, 'https://').replace(/^ssh:\/\/git@/, 'https://');
  if (/^github:/.test(url)) url = 'https://github.com/' + url.slice(7);
  if (/^[\w.-]+\/[\w.-]+$/.test(url)) url = 'https://github.com/' + url;
  return url;
}

function licenceOf(json) {
  if (typeof json.license === 'string') return json.license;
  if (json.license?.type) return json.license.type;
  if (Array.isArray(json.licenses)) return json.licenses.map((l) => l.type ?? l).join(' OR ');
  return null;
}

function authorOf(json) {
  const name = (person) => (typeof person === 'string' ? person : person?.name ?? '').replace(/\s*[<(].*$/, '').trim();
  return [json.author, ...(json.contributors ?? [])].map(name).filter(Boolean).slice(0, 1);
}

const entries = new Map();
const problems = [];
const usedVendored = new Set();

/// Record a package, once, and why it is in the bundle.
function addPackage(record, how) {
  const key = keyOf(record.name, record.version);
  if (!entries.has(key)) entries.set(key, { ...record, hows: new Set() });
  entries.get(key).hows.add(how);
  return entries.get(key);
}

function installed(dir) {
  const json = readJson(join(dir, 'package.json'));
  const license = licenceOf(json);
  if (!license) problems.push(json.name + '@' + json.version + ' declares no licence in its package.json: read its files and declare one here.');
  let files = readdirSync(dir)
    .filter((f) => (LICENCE_FILE.test(f) || NOTICE_FILE.test(f)) && !NOT_TEXT.test(f) && statSync(join(dir, f)).isFile())
    .sort()
    .map((f) => ({ file: f, text: readFileSync(join(dir, f), 'utf8') }));
  // Kept in VENDORED for an installed package: the licence its repository holds, when the package
  // publishes none, and the notices its dist carries in comments, which the bundle's minifier
  // removes.
  const key = keyOf(json.name, json.version);
  const kept = join(VENDORED, key);
  if (existsSync(kept)) {
    usedVendored.add(key);
    const ships = files.some((f) => LICENCE_FILE.test(f.file));
    for (const f of readdirSync(kept).sort()) {
      const text = readFileSync(join(kept, f), 'utf8');
      if (NOTICE_FILE.test(f)) files.push({ file: f + " (the notices its dist carries in comments, which the bundle's minifier removes)", text, fromDist: true });
      else if (!ships) files.push({ file: f + ' (from its repository; the package ships none)', text });
      else problems.push(relative(ui, join(kept, f)) + ' is a licence file for ' + key + ', which ships its own: remove it.');
    }
  }
  return { name: json.name, version: json.version, license: license ?? 'UNKNOWN', repository: repositoryOf(json), authors: authorOf(json), files, dir };
}

function vendored(name, declared) {
  const key = keyOf(name, declared.version);
  const dir = join(VENDORED, key);
  usedVendored.add(key);
  const files = existsSync(dir) ? readdirSync(dir).sort().map((f) => ({ file: f, text: readFileSync(join(dir, f), 'utf8') })) : [];
  if (!files.length) problems.push(key + ' is compiled in, at a release not installed here, and ' + relative(ui, dir) + ' holds no licence file for it.');
  return { name, version: declared.version, license: declared.license, repository: declared.repository, authors: declared.author ? [declared.author] : [], files, dir: null };
}

/// The directories in VENDORED, as name@version.
function vendoredKeys() {
  const keys = [];
  if (!existsSync(VENDORED)) return keys;
  for (const d of readdirSync(VENDORED)) {
    if (!statSync(join(VENDORED, d)).isDirectory()) continue;
    if (d.startsWith('@')) for (const p of readdirSync(join(VENDORED, d))) keys.push(d + '/' + p);
    else keys.push(d);
  }
  return keys.sort();
}

/// The version banners in a package's published code, `@license <name> <version>`, by lowercased
/// name: what a dist says of a release it compiled in.
const bannerCache = new Map();
function banners(dir) {
  if (!dir) return new Map();
  if (!bannerCache.has(dir)) {
    const found = new Map();
    const walk = (at) => {
      for (const f of readdirSync(at)) {
        const path = join(at, f);
        if (f === 'node_modules') continue;
        if (statSync(path).isDirectory()) walk(path);
        else if (/\.(js|mjs|cjs)$/.test(f))
          for (const m of readFileSync(path, 'utf8').matchAll(/@license\s+(@?[\w./-]+)\s+v?(\d+\.\d+\.\d+[\w.+-]*)/g)) {
            const name = m[1].toLowerCase();
            if (!found.has(name)) found.set(name, new Set());
            found.get(name).add(m[2]);
          }
      }
    };
    walk(dir);
    bannerCache.set(dir, found);
  }
  return bannerCache.get(dir);
}

/// How a package INLINED declares comes to be in the console, and how its version is known.
const declaredHow = (host, declared) => 'compiled into the dist of ' + host + ' (' + declared.basis + ')';

/// Follow a bundled file's source map to the packages it compiles in.
function followMaps(file, host, seen) {
  if (seen.has(file)) return;
  seen.add(file);
  const found = sourceMap(file);
  if (!found) return;
  const { map, dir } = found;
  for (const raw of map.sources ?? []) {
    const source = (map.sourceRoot ? map.sourceRoot.replace(/\/?$/, '/') : '') + raw;
    if (source.includes('node_modules/')) {
      const segments = source.split('node_modules/').slice(1);
      const names = segments.map((s) => s.split('/').slice(0, s.startsWith('@') ? 2 : 1).join('/'));
      const name = names[names.length - 1];
      if (name === host.name) continue;
      const into = 'compiled into the dist of ' + keyOf(host.name, host.version);
      // A release the host's dist says it carries, by its licence banner.
      const banner = banners(host.dir).get(name.toLowerCase());
      if (banner && banner.size > 1) problems.push(keyOf(host.name, host.version) + "'s dist carries " + name + ' at ' + [...banner].join(' and ') + ': say which code is which.');
      const declared = INLINED[keyOf(host.name, host.version)]?.[name];
      if (declared) {
        if (banner && !banner.has(declared.version))
          problems.push('INLINED declares ' + name + ' ' + declared.version + ' in ' + keyOf(host.name, host.version) + ', and its banner there says ' + [...banner].join(', ') + '.');
        addPackage(vendored(name, declared), declaredHow(keyOf(host.name, host.version), declared));
        continue;
      }
      // The first name is wherever the host's build had it, which was hoisted: the nearest the
      // host's own dependencies install, every version at that distance, or, when it is none of
      // theirs, where Node would resolve it from the host. Each later name is nested in the one
      // before it, so it is that package's own dependency.
      let dirs = nearest(host.dir, names[0]);
      if (!dirs.length) dirs = [resolveFrom(host.dir, names[0])].filter(Boolean);
      for (const n of names.slice(1)) dirs = dirs.map((d) => resolveFrom(d, n)).filter(Boolean);
      if (banner) {
        const versions = dirs.map((d) => readJson(join(d, 'package.json')).version);
        dirs = dirs.filter((d) => banner.has(readJson(join(d, 'package.json')).version));
        if (!dirs.length) {
          problems.push(
            keyOf(host.name, host.version) + "'s dist carries " + name + ' ' + [...banner].join(', ') + ' by its banner, and the copy installed nearest it is ' + (versions.join(', ') || 'none') + ': declare the release in INLINED, with its licence in ' + relative(ui, VENDORED) + '.',
          );
          continue;
        }
      }
      const how = into + (banner ? ' (the version its licence banner in that dist names)' : ' (the version installed here; that dist does not record which release it compiled in)');
      if (dirs.length) {
        for (const at of dirs) {
          const entry = addPackage(installed(at), how);
          const rest = segments[segments.length - 1].slice(name.length + 1);
          followMaps(join(at, rest), entry, seen);
        }
        continue;
      }
      problems.push(keyOf(host.name, host.version) + "'s source map names " + name + ', which is not installed and not declared in INLINED: ' + raw);
      continue;
    }
    const absolute = resolve(dir, source);
    if (absolute === host.dir || absolute.startsWith(host.dir + sep)) continue;
    const alias = (SOURCE_ALIASES[host.name] ?? []).find((a) => source.startsWith(a.prefix));
    if (alias) {
      const target = resolveFrom(host.dir, alias.package);
      if (!target) {
        problems.push(host.name + "'s source alias names " + alias.package + ', which it does not resolve.');
        continue;
      }
      const entry = addPackage(installed(target), 'compiled into the dist of ' + keyOf(host.name, host.version));
      followMaps(join(target, source.slice(alias.prefix.length)), entry, seen);
      continue;
    }
    problems.push(keyOf(host.name, host.version) + "'s source map names a file outside it that nothing accounts for: " + raw);
  }
}

// ---------------------------------------------------------------- the build

async function bundled() {
  const rendered = new Set();
  const styles = new Set();
  const collector = {
    name: 'agent-router-notices',
    generateBundle(_options, output) {
      for (const chunk of Object.values(output)) {
        if (chunk.type !== 'chunk') continue;
        for (const [id, module] of Object.entries(chunk.modules)) if (module.renderedLength > 0) rendered.add(id);
      }
      for (const file of this.getWatchFiles()) {
        const path = file.split('?')[0];
        if (/\.(css|scss|sass)$/.test(path)) styles.add(path);
      }
    },
  };
  await build({
    root: ui,
    configFile: join(ui, 'vite.config.ts'),
    logLevel: 'silent',
    build: { write: false },
    plugins: [collector],
    worker: { plugins: () => [collector] },
  });
  return { rendered, styles };
}

async function collect() {
  const { rendered, styles } = await bundled();
  const seen = new Set();
  const unmapped = [];
  const usedVirtual = new Set();
  for (const id of [...rendered].sort()) {
    // @rollup/plugin-commonjs's wrappers around a CommonJS file: the file's package, and the
    // plugin's interop code.
    if (id.startsWith('\0' + sep) && id.includes('?commonjs-')) {
      const dir = packageDir(id.slice(1).split('?')[0]);
      if (dir) {
        addPackage(installed(dir), 'its JavaScript');
        usedVirtual.add('@rollup/plugin-commonjs');
        continue;
      }
    }
    if (id.startsWith('\0')) {
      const virtual = VIRTUAL[id];
      if (!virtual) {
        problems.push('the virtual module ' + JSON.stringify(id) + ' renders into the bundle and VIRTUAL does not say whose it is.');
        continue;
      }
      usedVirtual.add(virtual.package);
      continue;
    }
    const file = id.split('?')[0];
    const dir = packageDir(file);
    if (dir) {
      const entry = addPackage(installed(dir), 'its JavaScript');
      followMaps(file, entry, seen);
      if (!sourceMap(file)) unmapped.push({ entry, file });
      continue;
    }
  }
  for (const file of [...styles].sort()) {
    const dir = packageDir(file);
    if (dir) addPackage(installed(dir), 'its stylesheets, compiled into the console\'s CSS');
  }

  // Vite's helpers, and the CommonJS helper of the @rollup/plugin-commonjs it bundles: their
  // licences are sections of Vite's LICENSE.md.
  if (usedVirtual.size) {
    const viteDir = resolveFrom(ui, 'vite');
    const vite = installed(viteDir);
    const licence = readFileSync(join(viteDir, 'LICENSE.md'), 'utf8');
    const core = licence.split(/^# Licenses of bundled dependencies/m)[0].replace(/^# Vite core license\n.*\n/, '');
    if (usedVirtual.has('vite')) {
      const hows = Object.values(VIRTUAL).filter((v) => v.package === 'vite').map((v) => v.how);
      addPackage({ ...vite, files: [{ file: 'LICENSE.md (Vite core license)', text: core }] }, hows.join(' and '));
    }
    if (usedVirtual.has('@rollup/plugin-commonjs')) {
      const section = licence.match(/^## [^\n]*@rollup\/plugin-commonjs[^\n]*\n([\s\S]*?)\n-{10,}/m);
      if (!section) problems.push("vite's LICENSE.md no longer has a section for @rollup/plugin-commonjs.");
      const range = readJson(join(viteDir, 'package.json')).devDependencies?.['@rollup/plugin-commonjs'] ?? 'unknown';
      const text = (section?.[1] ?? '').replace(/^> ?/gm, '');
      addPackage(
        {
          name: '@rollup/plugin-commonjs',
          version: range + ', bundled in vite ' + vite.version,
          license: 'MIT',
          repository: 'https://github.com/rollup/plugins',
          authors: [],
          files: [{ file: "LICENSE (its section of Vite's LICENSE.md)", text }],
          dir: null,
        },
        VIRTUAL['\0commonjsHelpers.js'].how,
      );
    }
  }

  // Declared inlining, for each host the bundle holds, and declarations that no longer apply.
  for (const [host, inlined] of Object.entries(INLINED)) {
    if (!entries.has(host)) {
      problems.push('INLINED declares what ' + host + ' compiles in, and the bundle holds no ' + host + '.');
      continue;
    }
    for (const [name, declared] of Object.entries(inlined)) addPackage(vendored(name, declared), declaredHow(host, declared));
  }
  for (const [host, aliases] of Object.entries(SOURCE_ALIASES)) {
    if (![...entries.values()].some((e) => e.name === host)) problems.push('SOURCE_ALIASES names ' + host + ', which the bundle does not hold.');
    for (const alias of aliases) if (![...entries.values()].some((e) => e.name === alias.package)) problems.push('SOURCE_ALIASES maps ' + host + ' to ' + alias.package + ', which no source map led to.');
  }

  // A module with no source map says what it compiles in only in its legal comments, and the
  // minifier strips them from the bundle: every copyright holder they name must be named by the
  // package's own licence files, by what INLINED declares it compiles in, or by the notices kept
  // for it in VENDORED.
  const byEntry = new Map();
  for (const { entry, file } of unmapped) {
    if (!byEntry.has(entry)) byEntry.set(entry, []);
    byEntry.get(entry).push(readFileSync(file, 'utf8'));
  }
  for (const [entry, codes] of byEntry) {
    const key = keyOf(entry.name, entry.version);
    const declared = Object.entries(INLINED[key] ?? {}).map(([name, d]) => vendored(name, d));
    const accounted = new Set(
      [...entry.files.map((f) => f.text), ...declared.flatMap((d) => d.files.map((f) => f.text)), ...entry.authors, ...declared.flatMap((d) => d.authors)]
        .join(' ')
        .toLowerCase()
        .split(/[^a-z0-9]+/),
    );
    for (const code of codes)
      for (const comment of legalComments(code))
        for (const holder of holders(comment))
          if (!holder.tokens.every((t) => accounted.has(t)))
            problems.push(key + "'s code names a copyright holder its notices do not: " + JSON.stringify(holder.text) + '. Declare what it compiles in (INLINED), or keep the notice in ' + relative(ui, join(VENDORED, key)) + '/NOTICE.');
    // A notice kept for a dist must still be in it, verbatim.
    for (const f of entry.files.filter((f) => f.fromDist))
      for (const block of f.text.match(/\/\*![\s\S]*?\*\//g) ?? [])
        if (!codes.some((code) => code.includes(block))) problems.push(relative(ui, join(VENDORED, key)) + '/' + f.file.split(' ')[0] + ' keeps a comment ' + key + "'s code no longer carries: " + JSON.stringify(block.slice(0, 60)));
  }
  for (const entry of entries.values())
    if (entry.files.some((f) => f.fromDist) && !byEntry.has(entry)) problems.push(relative(ui, join(VENDORED, keyOf(entry.name, entry.version))) + ' keeps notices from a dist with a source map, or none rendered: check them against the code.');
  for (const key of vendoredKeys()) if (!usedVendored.has(key)) problems.push(relative(ui, join(VENDORED, key)) + ' is for a package the bundle does not hold: remove it.');
}

/// A file's legal comments: /*! ... */, and any block comment that says @license.
const legalComments = (code) => code.match(/\/\*![\s\S]*?\*\/|\/\*(?:(?!\*\/)[\s\S])*?@license(?:(?!\*\/)[\s\S])*\*\//g) ?? [];

/// Words that say nothing about who holds a copyright.
const NOT_A_NAME = new Set(['and', 'the', 'its', 'inc', 'corp', 'llc', 'ltd', 'other', 'contributors', 'affiliates', 'authors', 'present', 'adapted', 'from']);

/// The copyright holders a comment names, each with the words a notice must contain to name it.
function holders(comment) {
  const out = [];
  const text = comment.replace(/^\s*\*+ ?/gm, ' ').replace(/\s+/g, ' ');
  for (const m of text.matchAll(/(?:copyright|\(c\)|©)\s*(?:\(c\)|©)?\s*(?:\d{4}(?:\s*[-–,]\s*(?:\d{4}|present))*\s*,?\s*)?([^*]+)/gi)) {
    const name = m[1]
      .split(/\s\(|\||\bLicen[sc]ed\b|\bMIT\b|\bReleased\b|\ball rights\b|https?:|<|\.\s+(?:This|See|All|Permission)\b/i)[0]
      .replace(/[\s.,;:]+$/, '')
      .trim();
    if (!name || /^(?:notice|statement|holders?|owners?|and|law|protection)\b/i.test(name)) continue;
    const tokens = name.toLowerCase().split(/[^a-z0-9]+/).filter((t) => t.length >= 3 && !NOT_A_NAME.has(t));
    if (tokens.length) out.push({ text: name, line: m[0].slice(0, m[0].length - m[1].length) + name, tokens });
  }
  return out;
}

// ---------------------------------------------------------------- the fonts

/// Each font in public/fonts, matched by its bytes to the installed IBM Plex package it came from.
function fonts() {
  const store = join(ui, 'node_modules', '.pnpm');
  const candidates = existsSync(store)
    ? readdirSync(store)
        .filter((d) => d.startsWith('@ibm+plex'))
        .flatMap((d) => {
          const scope = join(store, d, 'node_modules', '@ibm');
          return existsSync(scope) ? readdirSync(scope).map((p) => real(join(scope, p))) : [];
        })
    : [];
  const sha = (file) => createHash('sha256').update(readFileSync(file)).digest('hex');
  const index = new Map();
  const walk = (dir, root) => {
    for (const f of readdirSync(dir)) {
      const path = join(dir, f);
      if (statSync(path).isDirectory()) walk(path, root);
      else if (f.endsWith('.woff2')) index.set(f + ' ' + sha(path), root);
    }
  };
  for (const dir of new Set(candidates)) if (dir && existsSync(join(dir, 'fonts'))) walk(join(dir, 'fonts'), dir);

  const byPackage = new Map();
  for (const font of readdirSync(FONTS).filter((f) => f.endsWith('.woff2')).sort()) {
    const dir = index.get(font + ' ' + sha(join(FONTS, font)));
    if (!dir) {
      problems.push('public/fonts/' + font + ' matches no font file in an installed @ibm/plex package: say where it came from.');
      continue;
    }
    if (!byPackage.has(dir)) byPackage.set(dir, []);
    byPackage.get(dir).push(font);
  }
  const texts = new Map();
  for (const [dir, files] of [...byPackage].sort()) {
    const pkg = installed(dir);
    const family = basename(dir).replace(/^plex-/, '').replace(/(^|-)(\w)/g, (_m, dash, c) => (dash ? ' ' : '') + c.toUpperCase());
    addPackage(pkg, 'the IBM Plex ' + family + ' font files in fonts/: ' + files.join(', '));
    for (const f of pkg.files.filter((f) => LICENCE_FILE.test(f.file))) texts.set(f.text, pkg.name + ' ' + pkg.version);
  }
  // The OFL asks that its text accompany the fonts: verbatim, beside them, once per distinct text.
  if (texts.size === 1) return [...texts.keys()][0];
  return [...texts].map(([text, from]) => '---------------- ' + from + '\n\n' + text).join('\n\n');
}

// ---------------------------------------------------------------- rendering

/// A copyright line, after any comment marker; not a heading or a sentence about copyright.
const COPYRIGHT = /^\s*(?:\/\/+|#|\*)?\s*(?:(?:Copyright|COPYRIGHT)\b(?!\s+(?:notice|statement|holder|owner|license|(?:and|AND)\b))|©|\(c\)\s*\d)/;
/// A line that ends where its copyright line begins, wrapped onto the next.
const DANGLING = /(?:Copyright|COPYRIGHT)\s*(?:\(c\)|©)?$/;
const PLACEHOLDER = /\[yyyy\]|\{yyyy\}|<year>|\[name of copyright owner\]|copyright (?:notice|owner|holder|license|and license)/i;
const RULE = '='.repeat(100);

function copyrightOf(entry) {
  const found = [];
  for (const f of entry.texts) {
    // Notices kept from a dist's comments name their holders mid-sentence.
    if (f.fromDist) {
      for (const comment of legalComments(f.text)) found.push(...holders(comment).map((h) => h.line));
      continue;
    }
    const lines = normalise(f.text).split('\n');
    lines.forEach((line, i) => {
      if (DANGLING.test(line) && i + 1 < lines.length) line += ' ' + lines[i + 1].trim();
      else if (!COPYRIGHT.test(line)) return;
      if (!PLACEHOLDER.test(line)) found.push(line.replace(/^\s*(?:\/\/+|#|\*)?\s*/, ''));
    });
  }
  return [...new Set(found)];
}

function render(fontLicence) {
  const list = [...entries.values()].sort((a, b) => a.name.localeCompare(b.name) || a.version.localeCompare(b.version, undefined, { numeric: true }));
  for (const entry of list) {
    const tree = parseSpdx(entry.license);
    entry.chosen = choose(tree);
    const offered = ids(tree);
    const unchosen = new Set(offered.filter((id) => !entry.chosen.includes(id)));
    entry.texts = entry.files.filter((f) => !unchosen.has(namedFor(f.file)));
    entry.licenceTexts = entry.texts.filter((f) => !NOTICE_FILE.test(f.file));
    entry.notices = entry.texts.filter((f) => NOTICE_FILE.test(f.file));
    entry.copyright = copyrightOf(entry);
    if (!entry.licenceTexts.length) problems.push(keyOf(entry.name, entry.version) + ' ships no licence file: add its text to ' + relative(ui, VENDORED) + '.');
  }

  // Each distinct set of licence texts once, with every package it applies to.
  const groups = new Map();
  for (const entry of list) {
    const key = entry.licenceTexts.map((f) => normalise(f.text).replace(/\s+/g, ' ')).join('\u0001');
    if (!groups.has(key)) groups.set(key, []);
    groups.get(key).push(entry);
  }
  const sections = [...groups.values()];
  sections.forEach((members, i) => members.forEach((entry) => (entry.section = i + 1)));

  const flagged = list.filter((e) => ids(parseSpdx(e.license)).some((id) => !PERMISSIVE.has(id)));
  const out = [];
  out.push('Third-party notices: the Agent Router console', '');
  out.push(
    'The console is the web application in ui/dist, which the agent-router binary embeds and serves.',
    'It bundles code from the packages below and serves the IBM Plex fonts in fonts/. Each is listed',
    'with its version, its licence as it declares it, the licence it is used under where it offers a',
    'choice, how it comes to be in the console, and its copyright. The licence texts follow, each',
    'printed once with the packages it applies to; a NOTICE, COPYRIGHT or AUTHORS file a package ships',
    'is printed with that package.',
    '',
    'A package compiled into another package\'s published dist says how its version is known. A',
    'dist\'s source map names the packages it compiled in, but not their releases: where the dist',
    'carries a licence banner with a version, that version is listed; otherwise the version listed is',
    'the one installed here nearest that package (where two are equally near, both), which need not',
    'be the release the dist was built with. Where no map says, what a dist compiles in is declared',
    'from the package\'s own sources and lockfile, and its licence read from that release.',
    '',
    'Generated by ui/scripts/notices.mjs from what the production build bundles; `pnpm run build`',
    'fails when this file no longer matches it. Do not edit it: run `pnpm run notices` in ui/.',
    '',
    'Agent Router itself is under the MIT licence: LICENSE, at the root of its repository.',
    '',
  );

  out.push('Licences that are not permissive', '-'.repeat(32), '');
  if (!flagged.length) out.push('None.', '');
  for (const e of flagged) {
    const used = e.chosen.join(' AND ');
    const choice = e.chosen.every((id) => PERMISSIVE.has(id))
      ? 'It offers a choice, and is used under ' + used + ', which is permissive; the text of the other is left out.'
      : used === 'OFL-1.1'
        ? 'A font licence: the fonts are redistributed unmodified and not sold on their own, with the licence text beside them (fonts/LICENSE.txt) and below, as it requires.'
        : 'It is used under ' + used + ', whose conditions apply to that package.';
    out.push(e.name + ' ' + e.version + ': ' + e.license + '. In the console as ' + [...e.hows].join('; ') + '.', choice, '');
  }

  out.push('Packages (' + list.length + ')', '-'.repeat(12), '');
  for (const e of list) {
    const used = e.chosen.join(' AND ') === e.license ? e.license : e.license + ', used under ' + e.chosen.join(' AND ');
    out.push(e.name + ' ' + e.version + '  [' + e.section + ']', '  Licence: ' + used);
    if (e.repository) out.push('  Source: ' + e.repository);
    out.push('  In the console: ' + [...e.hows].sort().join('; '));
    if (e.copyright.length) e.copyright.forEach((c, i) => out.push((i ? '             ' : '  Copyright: ') + c));
    else if (e.authors.length) out.push('  Copyright: no line in its licence; its author: ' + e.authors.join(', '));
    else out.push('  Copyright: no line in its licence; see its licence text');
    out.push('');
  }

  sections.forEach((members, i) => {
    const chosen = [...new Set(members.flatMap((m) => m.chosen))].join(', ');
    out.push(RULE, '[' + (i + 1) + '] ' + chosen, RULE, '');
    out.push('Applies to: ' + members.map((m) => m.name + ' ' + m.version).join(', '), '');
    for (const m of members)
      for (const n of m.notices) out.push('---- ' + n.file + ', from ' + m.name + ' ' + m.version, '', normalise(n.text), '');
    const first = members[0];
    for (const f of first.licenceTexts) out.push('---- ' + f.file + (members.length > 1 ? ', as ' + first.name + ' ships it' : ''), '', normalise(f.text), '');
  });

  return { notices: '﻿' + out.join('\n').replace(/\n+$/, '') + '\n', fontLicence };
}

// ---------------------------------------------------------------- main

await collect();
const fontLicence = fonts();
const { notices } = render(fontLicence);

if (problems.length) {
  console.error('notices: cannot account for everything the console bundles:');
  for (const p of [...new Set(problems)]) console.error('  - ' + p);
  process.exit(1);
}

const outputs = [
  [NOTICES, notices],
  [FONT_LICENCE, fontLicence],
];

if (process.argv.includes('--check')) {
  const stale = outputs.filter(([file, text]) => !existsSync(file) || readFileSync(file, 'utf8') !== text);
  if (stale.length) {
    for (const [file] of stale) console.error('notices: ' + relative(ui, file) + ' is not what the build bundles');
    console.error("notices: run 'pnpm run notices' in ui/ and commit the result.");
    process.exit(1);
  }
  console.log('notices: ' + relative(ui, NOTICES) + ' and ' + relative(ui, FONT_LICENCE) + ' match the build (' + entries.size + ' packages)');
} else {
  for (const [file, text] of outputs) writeFileSync(file, text);
  console.log('notices: wrote ' + relative(ui, NOTICES) + ' and ' + relative(ui, FONT_LICENCE) + ' (' + entries.size + ' packages)');
}
