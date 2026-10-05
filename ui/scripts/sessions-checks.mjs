// New for agent-router: the Sessions screen's contracts that only a real browser can show — layout,
// folding, sideways scroll, target sizes, contrast — asserted on each shot of /live by
// scripts/shoot.mjs through scripts/shot-checks.mjs. Each check is an expression run in the shot's
// page once it has settled and been shot, answering `{ ok, detail }`; one that is not ok fails the
// run with its contract's id and its words. Whatever a spec can assert, a spec does: jsdom lays
// nothing out, and these are what it cannot see.
//
// The shot is of the list. Each check then opens the fixture harness's showcase session
// (harness/sessions.ts), whose feed holds every kind of row, and looks at it.

/// The session whose stream replays every kind of row (harness/sessions.ts, SHOWCASE).
const SHOWCASE = '7f3a9c2e-5b1d-4e8a-9c3f-0a1b2c3d4e5f';

/// What every check starts with: waiting, opening the showcase, and measuring sideways scroll.
const PRELUDE = `
  const wait = (ms) => new Promise((r) => setTimeout(r, ms));
  const frame = () => new Promise((r) => requestAnimationFrame(() => r()));
  const until = async (test, ms = 8000) => {
    for (const start = Date.now(); Date.now() - start < ms; await wait(50)) {
      const v = test();
      if (v) return v;
    }
    return null;
  };
  const view = () => document.querySelector('[data-session-view="${SHOWCASE}"]');
  const open = async () => {
    if (!view()) {
      const row = document.querySelector('[data-session="${SHOWCASE}"] > button');
      if (!row) return 'no list row for the showcase session';
      row.click();
    }
    const drawn = await until(() => view() && view().querySelectorAll('[data-entry]').length >= 12 && view().querySelector('[data-composer]') && /Running Bash/.test(view().querySelector('[data-session-head]')?.textContent ?? ''));
    if (!drawn) return 'the showcase session did not draw its feed and its status';
    await wait(250);
    await frame();
    return null;
  };
  const sideways = () => {
    const root = document.scrollingElement ?? document.documentElement;
    const x = scrollX;
    scrollTo(1e6, scrollY);
    const by = Math.round(scrollX);
    scrollTo(x, scrollY);
    return Math.max(by, root.scrollWidth - root.clientWidth);
  };
  const shown = (el) => {
    const r = el.getBoundingClientRect();
    if (!(r.width > 0 && r.height > 0)) return false;
    for (let at = el; at; at = at.parentElement) {
      const cs = getComputedStyle(at);
      if (cs.visibility === 'hidden' || cs.display === 'none' || at.getAttribute('aria-hidden') === 'true') return false;
    }
    return true;
  };
  const name = (el) => el.tagName.toLowerCase() + ' "' + (el.getAttribute('aria-label') || el.textContent || '').trim().slice(0, 40) + '"';
`;

/// A check whose expression runs after the prelude has opened the showcase.
const check = (id, says, at, body) => ({
  id,
  says,
  at,
  expression: `(async () => {
    ${PRELUDE}
    const failed = await open();
    if (failed) return { ok: false, detail: failed };
    ${body}
  })()`,
});

const CHECKS = [
  check(
    'SE-C89',
    'nothing in the feed widens it: transcripts, output, paths and links wrap, and only a command line, a fence or a table scrolls, inside its own box',
    'both',
    `
    const feed = view().querySelector('[data-feed-scroller]');
    const edge = feed.getBoundingClientRect().right + 0.5;
    const wide = [...view().querySelectorAll('[data-entry]')].filter((e) => e.getBoundingClientRect().right > edge).map(name);
    const by = sideways();
    const inside = feed.scrollWidth - feed.clientWidth;
    return { ok: wide.length === 0 && by <= 0 && inside <= 0, detail: 'page sideways by ' + by + ' px, feed by ' + inside + ' px' + (wide.length ? '; wider than the feed: ' + wide.slice(0, 4).join('; ') : '') };
  `,
  ),
  check(
    'SE-C202',
    'nothing scrolls the screen sideways: the header’s items shrink and ellipsise, and the status never crushes the heading',
    'both',
    `
    const head = view().querySelector('[data-session-head]');
    const heading = head.querySelector('h1, h2');
    const status = head.querySelector('[role="status"]');
    const hr = heading.getBoundingClientRect();
    const sr = status.getBoundingClientRect();
    const by = sideways();
    const ok = by <= 0 && hr.width >= 48 && sr.right <= document.documentElement.clientWidth + 0.5 && head.getBoundingClientRect().right <= document.documentElement.clientWidth + 0.5;
    return { ok, detail: 'sideways by ' + by + ' px; heading ' + Math.round(hr.width) + ' px wide; status ends at ' + Math.round(sr.right) + ' px of ' + document.documentElement.clientWidth };
  `,
  ),
  check(
    'SE-C201',
    'from 1,056 px the list sits beside the open session and marks it; below that one pane shows at a time',
    'both',
    `
    const list = document.querySelector('[data-pane="list"]');
    const main = document.querySelector('[data-pane="main"]');
    const desk = innerWidth >= 1056;
    const l = list.getBoundingClientRect();
    const m = main.getBoundingClientRect();
    const marked = list.querySelector('[data-session="${SHOWCASE}"][aria-current="true"]');
    const ok = desk ? shown(list) && shown(main) && l.right <= m.left + 1 && Math.abs(l.top - m.top) < 1 && !!marked : !shown(list) && shown(main);
    return { ok, detail: (desk ? 'desk' : 'phone') + ': list ' + (shown(list) ? Math.round(l.left) + '–' + Math.round(l.right) : 'hidden') + ', session ' + (shown(main) ? Math.round(m.left) + '–' + Math.round(m.right) : 'hidden') + (desk ? (marked ? ', the open session marked' : ', the open session NOT marked in the list') : '') };
  `,
  ),
  check(
    'SE-C197',
    'the screen is a column pinned to the viewport under the shell’s header — header, strips, one scrolling pane, composer last — and the page itself does not scroll',
    'both',
    `
    const screen = document.querySelector('[data-sessions-screen]');
    const s = screen.getBoundingClientRect();
    const dock = view().querySelector('[data-dock]').getBoundingClientRect();
    const feed = view().querySelector('[data-feed-scroller]').getBoundingClientRect();
    const head = view().querySelector('[data-session-head]').getBoundingClientRect();
    const root = document.scrollingElement ?? document.documentElement;
    const tall = root.scrollHeight - root.clientHeight;
    const ok = Math.abs(s.bottom - innerHeight) <= 1 && Math.abs(dock.bottom - s.bottom) <= 1 && head.bottom <= feed.top + 0.5 && feed.bottom <= dock.top + 0.5 && tall <= 1;
    return { ok, detail: 'screen ends at ' + Math.round(s.bottom) + ' of ' + innerHeight + ' px; composer ends at ' + Math.round(dock.bottom) + '; page scrolls by ' + tall + ' px' };
  `,
  ),
  check(
    'SE-C198',
    'whichever pane is visible owns the scroll, and its scroll never chains to the page',
    'both',
    `
    const feed = view().querySelector('[data-feed-scroller]');
    const cs = getComputedStyle(feed);
    const before = scrollY;
    feed.scrollTop = 0;
    feed.dispatchEvent(new WheelEvent('wheel', { deltaY: -400, bubbles: true }));
    await frame();
    const ok = cs.overflowY === 'auto' && cs.overscrollBehaviorY === 'contain' && feed.scrollHeight > feed.clientHeight && scrollY === before;
    return { ok, detail: 'overflow-y ' + cs.overflowY + ', overscroll ' + cs.overscrollBehaviorY + ', feed ' + feed.scrollHeight + ' over ' + feed.clientHeight + ' px' };
  `,
  ),
  check(
    'SE-C200',
    'the column is pinned to the visual viewport, so a keyboard that shrinks it never covers the composer',
    'both',
    `
    const screen = document.querySelector('[data-sessions-screen]');
    const pinned = screen.style.getPropertyValue('--ar-viewport-block');
    const ok = !!visualViewport && pinned === visualViewport.height + 'px';
    return { ok, detail: 'pinned to ' + (pinned || 'nothing') + ', the visual viewport is ' + (visualViewport ? visualViewport.height + 'px' : 'missing') };
  `,
  ),
  check(
    'SE-C85',
    'anything tall folds behind a toggle that says how many lines it hides and becomes "Show less", and appears only when the content overflows',
    'both',
    `
    const output = view().querySelector('[data-part="output"]');
    if (!output) return { ok: false, detail: 'no terminal output in the feed' };
    const toggle = [...output.querySelectorAll('button')].find((b) => /^Show \\d+ lines$/.test(b.textContent.trim()));
    if (!toggle) return { ok: false, detail: 'the forty-line output has no "Show N lines" toggle' };
    const lines = Number(/\\d+/.exec(toggle.textContent)[0]);
    const box = output.querySelector('[data-fold]');
    const folded = box.getBoundingClientRect().height;
    toggle.click();
    await frame();
    const opened = box.getBoundingClientRect().height;
    const less = toggle.textContent.trim();
    toggle.click();
    await frame();
    const short = [...view().querySelectorAll('[data-entry]')].find((e) => /Done\\. The shots are next\\./.test(e.textContent));
    const quiet = short && ![...short.querySelectorAll('button')].some((b) => /^Show /.test(b.textContent.trim()));
    const ok = lines >= 30 && folded < 100 && opened > folded * 3 && less === 'Show less' && !!quiet;
    return { ok, detail: 'says ' + lines + ' lines; folded ' + Math.round(folded) + ' px, opened ' + Math.round(opened) + ' px, then "' + less + '"' + (quiet ? '' : '; a short row has a toggle') };
  `,
  ),
  check(
    'SE-C86',
    'the fold clamps first and measures second: a long output is folded at four lines, a short one is not folded at all',
    'both',
    `
    const folds = [...view().querySelectorAll('[data-fold]')];
    const folded = folds.filter((f) => f.dataset.fold === 'folded');
    const fits = folds.filter((f) => f.dataset.fold === 'fits');
    const stuck = folds.filter((f) => f.dataset.fold === 'measuring');
    const ok = folded.length >= 1 && fits.length >= 3 && stuck.length === 0;
    return { ok, detail: folded.length + ' folded, ' + fits.length + ' fitting, ' + stuck.length + ' never measured' };
  `,
  ),
  check(
    'SE-C81',
    'a command line never wraps, it scrolls sideways inside its block, while output wraps anywhere',
    'both',
    `
    const cmd = view().querySelector('[data-part="command"] pre');
    const out = view().querySelector('[data-part="output"] pre');
    const c = getComputedStyle(cmd);
    const o = getComputedStyle(out);
    const scrolls = cmd.scrollWidth > cmd.clientWidth || innerWidth >= 1056;
    const ok = c.whiteSpace === 'pre' && scrolls && o.whiteSpace === 'pre-wrap' && o.overflowWrap === 'anywhere';
    return { ok, detail: 'command: ' + c.whiteSpace + ', ' + cmd.scrollWidth + ' in ' + cmd.clientWidth + ' px; output: ' + o.whiteSpace + ', ' + o.overflowWrap };
  `,
  ),
  check(
    'SE-C102',
    'a wide table scrolls sideways inside the column rather than widening it, its cells unwrapped',
    'both',
    `
    const box = view().querySelector('[role="region"][aria-label="Table"]');
    if (!box) return { ok: false, detail: 'no table in the feed' };
    const cell = box.querySelector('td');
    const ok = getComputedStyle(box).overflowX === 'auto' && getComputedStyle(cell).whiteSpace === 'nowrap' && box.getBoundingClientRect().right <= view().querySelector('[data-feed-scroller]').getBoundingClientRect().right + 0.5 && (box.scrollWidth > box.clientWidth || innerWidth >= 1056);
    return { ok, detail: 'table ' + box.scrollWidth + ' px in a box of ' + box.clientWidth + ' px' };
  `,
  ),
  check(
    'SE-C75',
    'the person’s words are the one filled bubble, at the right, no wider than 85 % of the column or 40em; the answer sits on the page, at most 46em',
    'both',
    `
    const feed = view().querySelector('[data-feed]').getBoundingClientRect();
    const bubble = view().querySelector('[data-kind="person"]');
    const prose = view().querySelector('[data-kind="assistant"]');
    const b = bubble.getBoundingClientRect();
    const em = parseFloat(getComputedStyle(bubble).fontSize);
    const filled = getComputedStyle(bubble).backgroundColor !== 'rgba(0, 0, 0, 0)';
    const proseBare = getComputedStyle(prose).backgroundColor === 'rgba(0, 0, 0, 0)';
    const ok = filled && proseBare && b.right >= feed.right - 24 && b.width <= Math.min(0.85 * feed.width, 40 * em) + 1 && prose.getBoundingClientRect().width <= 46 * parseFloat(getComputedStyle(prose).fontSize) + 1;
    return { ok, detail: 'bubble ' + Math.round(b.left) + '–' + Math.round(b.right) + ' in a column ' + Math.round(feed.left) + '–' + Math.round(feed.right) };
  `,
  ),
  check(
    'SE-C207',
    'every text meets WCAG AA contrast, the person’s bubble included',
    'both',
    `
    const rgb = (s) => (s.match(/[\\d.]+/g) || []).slice(0, 3).map(Number);
    const lum = ([r, g, b]) => [r, g, b].map((v) => { v /= 255; return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4; }).reduce((a, v, i) => a + v * [0.2126, 0.7152, 0.0722][i], 0);
    const ratio = (a, b) => { const [x, y] = [lum(rgb(a)), lum(rgb(b))].sort((p, q) => q - p); return (x + 0.05) / (y + 0.05); };
    const bubble = view().querySelector('[data-kind="person"]');
    const words = bubble.querySelector('p');
    const r = ratio(getComputedStyle(words).color, getComputedStyle(bubble).backgroundColor);
    return { ok: r >= 4.5, detail: 'the bubble’s words at ' + r.toFixed(2) + ':1' };
  `,
  ),
  check(
    'SE-C54',
    'the tab strip is a single row that scrolls sideways and never wraps, with the open tab in view',
    'both',
    `
    const strip = view().querySelector('nav ul');
    if (!strip) return { ok: false, detail: 'no tab strip over the showcase session' };
    const tabs = [...strip.children];
    const tops = new Set(tabs.map((t) => Math.round(t.getBoundingClientRect().top)));
    const here = strip.querySelector('[aria-current="page"]').getBoundingClientRect();
    const s = strip.getBoundingClientRect();
    const ok = tabs.length >= 2 && tops.size === 1 && getComputedStyle(strip).overflowX === 'auto' && getComputedStyle(strip).flexWrap === 'nowrap' && here.left >= s.left - 1 && here.right <= s.right + 1;
    return { ok, detail: tabs.length + ' tabs on ' + tops.size + ' row(s); the open tab ' + Math.round(here.left) + '–' + Math.round(here.right) + ' in a strip ' + Math.round(s.left) + '–' + Math.round(s.right) };
  `,
  ),
  check(
    'SE-C122',
    'the permission card sits above the composer and wraps its command rather than widening the screen',
    'both',
    `
    const card = view().querySelector('[data-dock] .cds--actionable-notification');
    if (!card) return { ok: false, detail: 'no permission card for the showcase session' };
    const composer = view().querySelector('[data-composer]').getBoundingClientRect();
    const c = card.getBoundingClientRect();
    const cmd = card.querySelector('pre');
    const ok = c.bottom <= composer.top + 0.5 && c.right <= document.documentElement.clientWidth + 0.5 && getComputedStyle(cmd).whiteSpace === 'pre-wrap' && cmd.scrollWidth <= cmd.clientWidth + 1;
    return { ok, detail: 'card ' + Math.round(c.top) + '–' + Math.round(c.bottom) + ', composer from ' + Math.round(composer.top) + '; command ' + cmd.scrollWidth + ' in ' + cmd.clientWidth + ' px' };
  `,
  ),
  check(
    'SE-C87',
    'following the newest row is a state: a session opened at its end stays at its end once its rows have folded',
    'both',
    `
    const feed = view().querySelector('[data-feed-scroller]');
    feed.scrollTop = feed.scrollHeight;
    await wait(300);
    const gap = feed.scrollHeight - feed.scrollTop - feed.clientHeight;
    return { ok: gap < 140, detail: gap + ' px from the bottom' };
  `,
  ),
  check(
    'SE-C205',
    'on a touch pointer every field is 16 px and every target 48 px',
    'phone',
    `
    const main = document.querySelector('[data-sessions-screen]');
    // An inline link inside prose is text (WCAG 2.5.8's inline exception), and a code box's own
    // focusable scroller is how a keyboard scrolls it, not a control.
    const targets = [...main.querySelectorAll('button, a[href], input:not([type="radio"]):not([type="checkbox"]), select, textarea, [tabindex="0"]')]
      .filter(shown)
      .filter((el) => !(el.tagName === 'A' && el.closest('[data-entry] p, [data-entry] li, [data-entry] td')))
      .filter((el) => !el.matches('.cds--snippet-container'));
    const small = targets.filter((el) => el.getBoundingClientRect().height < 47.5).map((el) => name(el) + ' ' + Math.round(el.getBoundingClientRect().height) + ' px');
    const fields = [...main.querySelectorAll('input:not([type="radio"]):not([type="checkbox"]), select, textarea')].filter(shown);
    const tiny = fields.filter((el) => parseFloat(getComputedStyle(el).fontSize) < 16).map((el) => name(el) + ' ' + getComputedStyle(el).fontSize);
    return { ok: targets.length > 0 && small.length === 0 && tiny.length === 0, detail: targets.length + ' targets, ' + fields.length + ' fields' + (small.length ? '; under 48 px: ' + small.slice(0, 8).join('; ') : '') + (tiny.length ? '; under 16 px: ' + tiny.join('; ') : '') };
  `,
  ),
];

export const SESSIONS_CHECKS = { '/live': CHECKS };
