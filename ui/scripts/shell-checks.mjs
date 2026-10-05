// New for agent-router: the shell's contracts that only a real browser can show — the topbar's colour,
// order and geometry under a notch, where the page starts, the address in the header, what `hidden`
// does whatever a class says, the smallest text and where the fonts come from, what a hover reveals,
// and the console inside a sandboxed frame — asserted on every shot of every route by scripts/shoot.mjs
// through scripts/shot-checks.mjs. Each check is an expression run in the shot's page once it has
// settled and been shot, answering `{ ok, detail }`; one that is not ok fails the run with its
// contract's id and its words. Whatever a spec can assert, a spec does (src/app/*.spec.*): jsdom lays
// nothing out, computes no style and loads no font, and these are what it cannot see.

/// What every check starts with.
const PRELUDE = `
  const wait = (ms) => new Promise((r) => setTimeout(r, ms));
  const frame = () => new Promise((r) => requestAnimationFrame(() => r()));
  const header = () => document.querySelector('header.cds--header');
  /// A safe-area inset as the page has it, in CSS pixels.
  const inset = (side) => {
    const probe = document.createElement('div');
    probe.style.cssText = 'position: fixed; visibility: hidden; padding-top: env(safe-area-inset-' + side + ', 0px)';
    document.body.append(probe);
    const v = parseFloat(getComputedStyle(probe).paddingTop);
    probe.remove();
    return v;
  };
  /// Drawn, with a box, and neither hidden nor transparent on the way up.
  const shown = (el) => {
    const r = el.getBoundingClientRect();
    if (!(r.width > 0 && r.height > 0)) return false;
    for (let at = el; at && at.nodeType === 1; at = at.parentElement) {
      const cs = getComputedStyle(at);
      if (cs.visibility === 'hidden' || cs.display === 'none' || parseFloat(cs.opacity) === 0 || at.getAttribute('aria-hidden') === 'true') return false;
    }
    return true;
  };
  const name = (el) => el.tagName.toLowerCase() + (typeof el.className === 'string' && el.className ? '.' + el.className.split(/\\s+/).filter(Boolean).slice(0, 2).join('.') : '') + ' "' + (el.getAttribute('aria-label') || el.textContent || '').trim().slice(0, 40) + '"';
`;

/// A check whose expression runs after the prelude, with the safe areas `insets` names for the shot's
/// mode emulated while it runs (scripts/shoot.mjs).
const check = (id, says, at, body, insets = undefined) => ({
  id,
  says,
  at,
  ...(insets ? { insets } : {}),
  expression: `(async () => {
    ${PRELUDE}
    ${body}
  })()`,
});

/// An iPhone in portrait, drawn under its status bar and over its home indicator (viewport-fit=cover):
/// the safe areas it reports, in CSS pixels.
const NOTCH = { top: 47, right: 0, bottom: 34, left: 0 };

export const SHELL_CHECKS = [
  check(
    'SH-C1',
    'the topbar is Carbon g100 in both themes and holds, left to right, the menu button, the name, the freshness badge, the search action and the theme switch, and no way to sign in or out',
    'both',
    `
    const h = header();
    if (!h) return { ok: false, detail: 'no header on the page' };
    const parts = [
      ['menu button', h.querySelector('.cds--header__menu-toggle')],
      ['name', h.querySelector('a.cds--header__name')],
      ['freshness badge', h.querySelector('[data-live-mode]')],
      ['search action', h.querySelector('button[aria-label^="Search and commands"]')],
      ['theme switch', h.querySelector('button[aria-label^="Theme:"]')],
    ];
    const missing = parts.filter(([, el]) => !el || !shown(el)).map(([n]) => n);
    const lefts = parts.filter(([, el]) => el && shown(el)).map(([n, el]) => [n, Math.round(el.getBoundingClientRect().left)]);
    const ordered = lefts.every((p, i) => i === 0 || p[1] > lefts[i - 1][1]);
    // Carbon g100's $background, whichever theme the page wears.
    const g100 = 'rgb(22, 22, 22)';
    const bg = getComputedStyle(h).backgroundColor;
    const page = getComputedStyle(document.body).backgroundColor;
    const signs = [...h.querySelectorAll('a, button, [role="button"]')].filter((el) => /sign[ -]?(in|out)|log[ -]?(in|out)/i.test((el.getAttribute('aria-label') || '') + ' ' + el.textContent));
    const ok = missing.length === 0 && ordered && bg === g100 && signs.length === 0;
    return { ok, detail: 'header ' + bg + ' on a page of ' + page + '; ' + lefts.map(([n, l]) => n + ' at ' + l).join(', ') + (missing.length ? '; not drawn: ' + missing.join(', ') : '') + (signs.length ? '; a way to sign in or out: ' + signs.map(name).join(', ') : '') };
  `,
  ),
  check(
    'SH-C2',
    'the topbar’s bar is bounded and its actions whole on screen, the fixed header pads the top safe area so a notch never hides it, and the page, the rail and its overlay, a sticky bar and the palette’s sheet start where it ends',
    'both',
    `
    const h = header();
    if (!h) return { ok: false, detail: 'no header on the page' };
    const y0 = scrollY;
    scrollTo(0, 0);
    await frame();
    await frame();
    const top = inset('top');
    const cs = getComputedStyle(h);
    const hr = h.getBoundingClientRect();
    const bad = [];
    if (cs.position !== 'fixed') bad.push('the header is ' + cs.position + ', not fixed');
    if (Math.abs(parseFloat(cs.paddingTop) - top) > 0.5) bad.push('the header pads ' + cs.paddingTop + ' under a ' + top + ' px safe area');
    if (Math.abs(hr.top) > 0.5 || Math.abs(hr.height - (48 + top)) > 1) bad.push('the header spans ' + Math.round(hr.top) + '–' + Math.round(hr.bottom) + ' px, not 0–' + (48 + top));
    const bar = h.querySelector('.cds--header__global');
    const br = bar.getBoundingClientRect();
    if (br.right > innerWidth + 0.5 || br.right > hr.right + 0.5) bad.push('the bar runs to ' + Math.round(br.right) + ' px of ' + innerWidth);
    const actions = [...h.querySelectorAll('.cds--header__action')].filter(shown);
    for (const a of actions) {
      const r = a.getBoundingClientRect();
      const what = a.getAttribute('aria-label');
      if (r.left < -0.5 || r.right > innerWidth + 0.5) bad.push(what + ' lies off the screen, ' + Math.round(r.left) + '–' + Math.round(r.right) + ' px');
      if (r.width < 47.5 || r.height < 47.5) bad.push(what + ' is ' + Math.round(r.width) + '×' + Math.round(r.height) + ' px');
      if (r.top < top - 0.5) bad.push(what + ' starts at ' + Math.round(r.top) + ' px, under the ' + top + ' px safe area');
      const hit = document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2);
      if (!hit || !(hit === a || a.contains(hit))) bad.push(what + ' is covered at its middle');
    }
    // The page starts at the top of the document and clears the header by its padding: a margin
    // there collapses through #root and makes every page 48 px taller than the window.
    const main = document.getElementById('main-content');
    const mr = main.getBoundingClientRect();
    const mcs = getComputedStyle(main);
    const clears = mr.top + parseFloat(mcs.paddingTop);
    if (Math.abs(mr.top) > 0.5 || parseFloat(mcs.marginTop) !== 0) bad.push('the page starts ' + Math.round(mr.top) + ' px down with a ' + mcs.marginTop + ' margin');
    if (Math.abs(clears - hr.bottom) > 1) bad.push('the page’s content starts at ' + Math.round(clears) + ' px, the header ends at ' + Math.round(hr.bottom));
    const root = document.scrollingElement ?? document.documentElement;
    const extra = root.scrollHeight - Math.max(innerHeight, Math.ceil(mr.bottom));
    if (extra > 1) bad.push('the page is ' + extra + ' px taller than what it holds');
    const nav = document.querySelector('nav.cds--side-nav');
    if (nav && Math.abs(parseFloat(getComputedStyle(nav).top) - hr.bottom) > 1) bad.push('the rail starts at ' + getComputedStyle(nav).top);
    const overlay = document.querySelector('.cds--side-nav__overlay');
    if (overlay && Math.abs(parseFloat(getComputedStyle(overlay).top) - hr.bottom) > 1) bad.push('the drawer’s overlay starts at ' + getComputedStyle(overlay).top);
    // Whatever sticks to the page sticks where the header ends, never under it. One that sticks in a
    // box of its own scrolls with that box, and is that box's business.
    const onPage = (el) => {
      for (let at = el.parentElement; at && at !== document.body; at = at.parentElement) {
        const o = getComputedStyle(at);
        if (o.overflowX !== 'visible' || o.overflowY !== 'visible') return false;
      }
      return true;
    };
    const sticky = [...document.querySelectorAll('#main-content *')].filter((el) => getComputedStyle(el).position === 'sticky' && onPage(el));
    for (const el of sticky) {
      const at = parseFloat(getComputedStyle(el).top);
      if (Math.abs(at - hr.bottom) > 1) bad.push(name(el) + ' sticks at ' + at + ' px');
    }
    // In a hand the palette is a sheet under the header, and the header's actions stay pressable
    // over it.
    let palette = 'not opened';
    if (innerWidth < 672) {
      h.querySelector('button[aria-label^="Search and commands"]').click();
      let sheet = null;
      for (let i = 0; i < 80 && !(sheet = document.querySelector('.cds--modal.is-visible .cds--modal-container')); i++) await wait(100);
      if (!sheet) bad.push('the palette did not open');
      else {
        await wait(300);
        const sr = sheet.getBoundingClientRect();
        palette = 'sheet from ' + Math.round(sr.top) + ' px';
        if (Math.abs(sr.top - hr.bottom) > 1) bad.push('the palette’s sheet starts at ' + Math.round(sr.top) + ' px');
        const theme = h.querySelector('button[aria-label^="Theme:"]');
        const tr = theme.getBoundingClientRect();
        const hit = document.elementFromPoint(tr.left + tr.width / 2, tr.top + tr.height / 2);
        if (!hit || !(hit === theme || theme.contains(hit))) bad.push('the palette covers the theme switch');
        // Put away as a reader would, by Escape from the field it opened on.
        (document.activeElement ?? document.body).dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', code: 'Escape', bubbles: true, cancelable: true }));
        // Closed is gone from the page, not only marked so: Carbon hides the modal after a delay.
        const modal = sheet.closest('.cds--modal');
        for (let i = 0; i < 40 && (modal.classList.contains('is-visible') || getComputedStyle(modal).visibility !== 'hidden'); i++) await wait(100);
        if (modal.classList.contains('is-visible') || getComputedStyle(modal).visibility !== 'hidden') bad.push('the palette did not close');
      }
    }
    scrollTo(0, y0);
    return { ok: actions.length >= 3 && bad.length === 0, detail: top + ' px safe area; header 0–' + Math.round(hr.bottom) + ' px; ' + actions.length + ' actions; ' + sticky.length + ' sticky bars; palette ' + palette + (bad.length ? ': ' + bad.join('; ') : '') };
  `,
    { phone: NOTCH },
  ),
  check(
    'SH-C9',
    'the address the console was reached at is in the header, whole and in view, at both widths',
    'both',
    `
    const h = header();
    const el = h?.querySelector('[data-host]');
    if (!el) return { ok: false, detail: 'no address in the header' };
    const r = el.getBoundingClientRect();
    const hr = h.getBoundingClientRect();
    const text = el.textContent.trim();
    const whole = el.scrollWidth <= el.clientWidth + 1;
    const inside = r.left >= -0.5 && r.right <= innerWidth + 0.5 && r.top >= hr.top - 0.5 && r.bottom <= hr.bottom + 0.5;
    const nameLink = h.querySelector('a.cds--header__name');
    const hit = document.elementFromPoint(r.left + Math.min(8, r.width / 2), r.top + r.height / 2);
    const reached = !!hit && (hit === el || el.contains(hit) || hit === nameLink || nameLink.contains(hit));
    const ok = text === location.host && shown(el) && whole && inside && reached;
    return { ok, detail: '"' + text + '" for ' + location.host + ' at ' + Math.round(r.left) + '–' + Math.round(r.right) + ' × ' + Math.round(r.top) + '–' + Math.round(r.bottom) + ' px' + (whole ? '' : ', cut short') + (inside ? '' : ', outside the header or the screen') + (reached ? '' : ', covered by ' + (hit ? name(hit) : 'nothing')) };
  `,
  ),
  check(
    'SH-C29',
    'an element with the hidden attribute is not drawn and takes no space, whatever class styles it',
    'both',
    `
    const off = [...document.querySelectorAll('[hidden]')];
    const drawn = off.filter((el) => getComputedStyle(el).display !== 'none' || el.getClientRects().length > 0).map(name);
    // A hidden element under classes that set display: Carbon's, which set it throughout.
    const make = (tag, cls) => {
      const el = tag === 'svg' ? document.createElementNS('http://www.w3.org/2000/svg', 'svg') : document.createElement(tag);
      el.setAttribute('class', cls);
      el.setAttribute('hidden', '');
      if (tag !== 'svg') el.textContent = 'probe';
      return el;
    };
    const probes = [make('button', 'cds--btn cds--btn--primary'), make('div', 'cds--tag cds--tag--green'), make('a', 'cds--link'), make('div', 'cds--tile'), make('ul', 'cds--side-nav__items'), make('svg', 'cds--btn__icon')];
    const host = document.getElementById('main-content') ?? document.body;
    for (const p of probes) host.append(p);
    const leaked = probes.filter((el) => getComputedStyle(el).display !== 'none' || el.getClientRects().length > 0).map((el) => el.getAttribute('class'));
    for (const p of probes) p.remove();
    return { ok: drawn.length === 0 && leaked.length === 0, detail: off.length + ' hidden elements on the page, ' + drawn.length + ' drawn' + (drawn.length ? ': ' + drawn.slice(0, 4).join('; ') : '') + '; ' + probes.length + ' probes under Carbon classes' + (leaked.length ? ', drawn: ' + leaked.join(', ') : ', none drawn') };
  `,
  ),
  check(
    'SH-C41',
    'no text on screen is smaller than 12 px, the type is IBM Plex Sans and Mono, and every font and file the page loads comes from the router that served it',
    'both',
    `
    const small = [];
    const seen = new Set();
    const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
    for (let n = walker.nextNode(); n; n = walker.nextNode()) {
      const el = n.parentElement;
      if (!el || seen.has(el) || !n.textContent.trim()) continue;
      seen.add(el);
      if (!shown(el)) continue;
      const r = el.getBoundingClientRect();
      // Text that is only for a screen reader is clipped to a pixel; nobody reads it on screen.
      if (r.width <= 2 || r.height <= 2) continue;
      const size = parseFloat(getComputedStyle(el).fontSize);
      if (size < 11.95) small.push(name(el) + ' at ' + size + ' px');
    }
    const sans = getComputedStyle(document.body).fontFamily;
    const monoAt = document.querySelector('[data-host]');
    const mono = monoAt ? getComputedStyle(monoAt).fontFamily : '';
    // Every @font-face the page declares, and where it fetches its file from.
    const faces = [];
    const walk = (rules) => {
      for (const r of rules) {
        if (r instanceof CSSFontFaceRule) faces.push({ family: r.style.getPropertyValue('font-family').replace(/["']/g, '').trim(), src: r.style.getPropertyValue('src') });
        else if (r.cssRules) walk(r.cssRules);
      }
    };
    for (const sheet of document.styleSheets) {
      try { walk(sheet.cssRules); } catch { faces.push({ family: '(unreadable sheet)', src: String(sheet.href) }); }
    }
    const urls = faces.flatMap((f) => [...f.src.matchAll(/url\\(["']?([^"')]+)["']?\\)/g)].map((m) => new URL(m[1], location.href)));
    const foreignFaces = urls.filter((u) => u.origin !== location.origin).map(String);
    const families = new Set(faces.map((f) => f.family));
    const foreign = performance.getEntriesByType('resource').map((e) => e.name).filter((u) => !u.startsWith('data:') && new URL(u).origin !== location.origin);
    await document.fonts.ready;
    const loaded = new Set([...document.fonts].filter((f) => f.status === 'loaded').map((f) => f.family.replace(/["']/g, '')));
    const ok = small.length === 0 && /^"?IBM Plex Sans"?/.test(sans) && /^"?IBM Plex Mono"?/.test(mono) && families.has('IBM Plex Sans') && families.has('IBM Plex Mono') && loaded.has('IBM Plex Sans') && loaded.has('IBM Plex Mono') && foreignFaces.length === 0 && foreign.length === 0;
    return { ok, detail: seen.size + ' elements with text' + (small.length ? ', under 12 px: ' + small.slice(0, 6).join('; ') : ', none under 12 px') + '; body ' + sans.split(',')[0] + ', code ' + mono.split(',')[0] + '; faces loaded: ' + [...loaded].join(', ') + (foreignFaces.length ? '; fonts from elsewhere: ' + foreignFaces.join(', ') : '') + (foreign.length ? '; files from elsewhere: ' + foreign.slice(0, 4).join(', ') : '') };
  `,
  ),
  check(
    'SH-C46',
    'where there is no hover, nothing a hover would reveal is hidden: every control a stylesheet shows on hover is drawn outright',
    'phone',
    `
    const reveal = [];
    const walk = (rules) => {
      for (const r of rules) {
        if (r instanceof CSSStyleRule && r.selectorText.includes(':hover')) {
          const st = r.style;
          const shows = (st.opacity !== '' && parseFloat(st.opacity) > 0) || st.visibility === 'visible' || (st.display !== '' && st.display !== 'none');
          if (shows) reveal.push(r.selectorText);
        }
        if (r.cssRules) walk(r.cssRules);
      }
    };
    for (const sheet of document.styleSheets) {
      try { walk(sheet.cssRules); } catch {}
    }
    // What each rule would reveal, written without its hover; the controls among it, and the ones
    // that hold a control.
    const CONTROL = 'button, a[href], input, select, textarea, [role="button"], [role="tab"], [tabindex="0"]';
    const bases = [...new Set(reveal.flatMap((s) => s.split(',')).filter((s) => s.includes(':hover')).map((s) => s.replace(/:hover/g, '').trim()))];
    const hidden = [];
    let looked = 0;
    for (const base of bases) {
      let els = [];
      try { els = [...document.querySelectorAll(base)]; } catch { continue; }
      for (const el of els) {
        if (!(el.matches(CONTROL) || el.querySelector(CONTROL))) continue;
        if (el.getClientRects().length === 0) continue;
        const r = el.getBoundingClientRect();
        if (r.right < 0 || r.left > innerWidth || r.width <= 2 || r.height <= 2) continue;
        looked += 1;
        if (!shown(el)) hidden.push(name(el) + ' (' + base + ')');
      }
    }
    // The case the contract was written for: a session tab's Archive is there on every tab, not
    // only the open one.
    const tabs = [...document.querySelectorAll('nav[aria-label="This project’s sessions"] li')];
    const tabless = tabs.filter((t) => { const b = t.querySelector('button'); return !b || !shown(b); }).map(name);
    const ok = hidden.length === 0 && tabless.length === 0 && !matchMedia('(hover: hover)').matches;
    return { ok, detail: reveal.length + ' hover rules reveal something; ' + looked + ' controls they reveal are on the page' + (hidden.length ? ', hidden: ' + hidden.slice(0, 6).join('; ') : ', all drawn') + (tabs.length ? '; ' + tabs.length + ' session tabs, ' + (tabs.length - tabless.length) + ' with their Archive drawn' : '') + (matchMedia('(hover: hover)').matches ? '; this pointer hovers' : '') };
  `,
  ),
  check(
    'SH-C55',
    'the console draws and works inside a frame sandboxed as VS Code’s fallback webview frames it: no modals, no popups',
    'desk',
    `
    const frameEl = document.createElement('iframe');
    frameEl.setAttribute('sandbox', 'allow-scripts allow-forms allow-same-origin allow-downloads');
    frameEl.style.cssText = 'position: fixed; inset: 0; inline-size: ' + innerWidth + 'px; block-size: ' + innerHeight + 'px; border: 0; z-index: -1; visibility: hidden';
    frameEl.src = location.href;
    document.body.append(frameEl);
    const errors = [];
    let doc = null;
    const drawn = () => {
      try {
        doc = frameEl.contentDocument;
        const w = frameEl.contentWindow;
        if (w && !w.__sh_c55) {
          w.__sh_c55 = true;
          w.addEventListener('error', (e) => errors.push(String(e.message)));
          w.addEventListener('unhandledrejection', (e) => errors.push(String(e.reason)));
        }
        const main = doc?.getElementById('main-content');
        return !!(doc?.querySelector('header.cds--header [data-live-mode]') && main && main.querySelector('h1, h2, [data-sessions-screen]') && w.__harness?.answered.length > 0);
      } catch (e) {
        errors.push(String(e));
        return false;
      }
    };
    let ok = false;
    for (const start = Date.now(); Date.now() - start < 15000; await wait(100)) {
      if (drawn()) { ok = true; break; }
    }
    await wait(500);
    const w = frameEl.contentWindow;
    const framed = !!w && w !== w.top;
    const stopped = !!doc && /This page stopped working/.test(doc.body?.textContent ?? '');
    const unanswered = w?.__harness ? [...w.__harness.unanswered] : ['no harness in the frame'];
    const heading = doc?.querySelector('#main-content h1, #main-content h2')?.textContent?.trim() ?? (doc?.querySelector('[data-sessions-screen]') ? 'the Sessions screen' : 'nothing');
    frameEl.remove();
    return { ok: ok && framed && !stopped && unanswered.length === 0 && errors.length === 0, detail: (ok ? 'drew ' + heading + ' in the frame' : 'the frame drew no shell and screen in 15 s') + (framed ? '' : '; not framed') + (stopped ? '; the error screen' : '') + (unanswered.length ? '; unanswered: ' + unanswered.join('; ') : '') + (errors.length ? '; threw: ' + errors.slice(0, 3).join('; ') : '') };
  `,
  ),
];
