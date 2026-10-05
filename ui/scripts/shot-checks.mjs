// New for agent-router: the contracts only a real browser can show — layout, sticking, overflow, target
// sizes, motion — asserted on each shot by scripts/shoot.mjs, per route. Each check is an expression
// evaluated in the shot's page once it has settled and been shot; it answers `{ ok, detail }`, and a
// check that is not ok fails the run with its contract's id and its words.
//
// Only what jsdom cannot answer belongs here: whatever a spec can assert, a spec does.

import { SETTINGS_CHECKS } from './settings-checks.mjs';
import { SESSIONS_CHECKS } from './sessions-checks.mjs';

/// The checks for a route at a width: `mode` is the shot's, phone or desk (scripts/shoot.mjs).
export function checksFor(route, mode) {
  const path = route.split('?')[0] || '/';
  return (CHECKS[path] ?? []).filter((c) => c.at === 'both' || c.at === mode);
}

const OVERVIEW = [
  {
    id: 'OV-C35',
    says: 'the scope bar stays in view at the top while the page scrolls',
    at: 'both',
    expression: `(async () => {
      const bar = document.querySelector('[data-scope-bar]');
      if (!bar) return { ok: false, detail: 'no scope bar on the page' };
      const header = document.querySelector('.cds--header')?.getBoundingClientRect().bottom ?? 0;
      const y = scrollY;
      scrollTo(0, Math.min(2400, document.documentElement.scrollHeight));
      await new Promise((r) => requestAnimationFrame(() => r()));
      const r = bar.getBoundingClientRect();
      const moved = scrollY;
      scrollTo(0, y);
      const ok = moved > 0 && r.top >= header - 1 && r.top <= header + 2 && r.height > 0;
      return { ok, detail: 'scrolled ' + moved + ' px, the bar is at ' + Math.round(r.top) + ' px under a header ending at ' + Math.round(header) + ' px' };
    })()`,
  },
  {
    id: 'OV-C196',
    says: 'long unbroken paths, branches, model ids and errors wrap or ellipsise and never widen the page',
    at: 'both',
    expression: `(async () => {
      const long = ['a-directory-name-long-enough-to-wrap-in-a-phones-rail', 'gpt-oss-120b-with-a-name-long-enough-to-wrap-on-a-phone', 'a-branch-name-that-goes-on-for-quite-a-while-without-a-break'];
      // The long error is in a request's detail: open the first request that ended in one.
      const errored = [...document.querySelectorAll('[aria-label="Requests, newest first"] button[aria-expanded]')].find((b) => /529/.test(b.textContent ?? ''));
      if (errored && errored.getAttribute('aria-expanded') !== 'true') {
        errored.click();
        await new Promise((r) => setTimeout(r, 150));
      }
      const text = document.getElementById('main-content')?.innerText ?? '';
      const missing = long.filter((s) => !text.includes(s));
      const error = /longunbrokenidentifierthatkeepsgoing/.test(text);
      const root = document.scrollingElement ?? document.documentElement;
      const wide = root.scrollWidth - root.clientWidth;
      if (errored && errored.getAttribute('aria-expanded') === 'true') {
        errored.click();
        await new Promise((r) => setTimeout(r, 50));
      }
      return { ok: missing.length === 0 && error && wide <= 0, detail: (missing.length ? 'not on the page: ' + missing.join(', ') + '; ' : '') + (error ? '' : 'the long error was not shown; ') + 'scroll width over the window: ' + wide + ' px' };
    })()`,
  },
  {
    id: 'OV-C198',
    says: 'on a touch device every field is at least 16 px and every target at least 48 px',
    at: 'phone',
    expression: `(() => {
      const main = document.getElementById('main-content');
      if (!main) return { ok: false, detail: 'no main content' };
      const shown = (el) => {
        const r = el.getBoundingClientRect();
        if (!(r.width > 0 && r.height > 0)) return false;
        for (let at = el; at; at = at.parentElement) {
          const cs = getComputedStyle(at);
          if (cs.visibility === 'hidden' || cs.display === 'none' || at.getAttribute('aria-hidden') === 'true') return false;
        }
        return true;
      };
      const name = (el) => el.tagName.toLowerCase() + (el.className && typeof el.className === 'string' ? '.' + el.className.split(/\\s+/).filter(Boolean).slice(0, 2).join('.') : '') + ' "' + (el.getAttribute('aria-label') || el.textContent || '').trim().slice(0, 40) + '"';
      const targets = [...main.querySelectorAll('button, a[href], input, select, textarea, [role="tab"], [role="option"], [tabindex="0"]')].filter(shown);
      const small = targets.filter((el) => el.getBoundingClientRect().height < 47.5).map((el) => name(el) + ' ' + Math.round(el.getBoundingClientRect().height) + ' px');
      const fields = [...main.querySelectorAll('input, select, textarea')].filter(shown);
      const tiny = fields.filter((el) => parseFloat(getComputedStyle(el).fontSize) < 16).map((el) => name(el) + ' ' + getComputedStyle(el).fontSize);
      return { ok: targets.length > 0 && small.length === 0 && tiny.length === 0, detail: targets.length + ' targets, ' + fields.length + ' fields' + (small.length ? '; under 48 px: ' + small.slice(0, 6).join('; ') : '') + (tiny.length ? '; under 16 px: ' + tiny.join('; ') : '') };
    })()`,
  },
  {
    id: 'OV-C202',
    says: 'on a phone the headline tiles sit two to a row with the largest across both, and side-by-side panels stack',
    at: 'phone',
    expression: `(() => {
      const grid = document.querySelector('section[aria-label="Totals in scope"]');
      if (!grid) return { ok: false, detail: 'no Totals in scope grid' };
      const tiles = [...grid.children].map((t) => t.getBoundingClientRect());
      const g = grid.getBoundingClientRect();
      const hero = tiles[0];
      const pair = tiles[1] && tiles[2] && Math.abs(tiles[1].top - tiles[2].top) < 1 && tiles[2].left > tiles[1].left;
      const halves = tiles.slice(1).every((t) => t.width < g.width * 0.55);
      const across = hero && Math.abs(hero.width - g.width) < 1;
      const twos = [...document.querySelectorAll('[data-two]')].map((two) => [...two.children].map((c) => Math.round(c.getBoundingClientRect().left)));
      const stacked = twos.length > 0 && twos.every((lefts) => lefts.every((l) => l === lefts[0]));
      return { ok: !!(pair && halves && across && stacked), detail: 'hero ' + Math.round(hero?.width ?? 0) + ' of ' + Math.round(g.width) + ' px; two to a row: ' + !!pair + '; two-up panels stacked: ' + stacked + ' (' + twos.length + ' pairs)' };
    })()`,
  },
  {
    id: 'OV-C11',
    says: 'whatever the page turns off is not displayed and takes no space, an SVG included',
    at: 'both',
    expression: `(() => {
      const off = [...document.querySelectorAll('#main-content [hidden]')];
      const shown = off.filter((el) => { const r = el.getBoundingClientRect(); return getComputedStyle(el).display !== 'none' || r.width > 0 || r.height > 0; });
      const probe = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
      probe.setAttribute('hidden', '');
      probe.setAttribute('class', 'cds--btn__icon');
      document.getElementById('main-content')?.append(probe);
      const r = probe.getBoundingClientRect();
      const svgOff = getComputedStyle(probe).display === 'none' && r.width === 0 && r.height === 0;
      probe.remove();
      return { ok: shown.length === 0 && svgOff, detail: off.length + ' hidden elements, ' + shown.length + ' still drawn; a hidden svg under a class that styles it: ' + (svgOff ? 'not drawn' : 'drawn') };
    })()`,
  },
  {
    id: 'OV-C200',
    says: 'with reduced motion set, nothing animates',
    at: 'both',
    expression: `(() => {
      const reduced = matchMedia('(prefers-reduced-motion: reduce)').matches;
      const running = document.getAnimations().filter((a) => a.playState === 'running' && (a.effect?.getComputedTiming?.().duration ?? 0) > 1);
      return { ok: reduced && running.length === 0, detail: (reduced ? 'reduced motion is set' : 'reduced motion is NOT set') + '; ' + running.length + ' animations running' };
    })()`,
  },
];

const CHECKS = { '/': OVERVIEW, ...SETTINGS_CHECKS, ...SESSIONS_CHECKS };
