// New for agent-router: the Settings pages' contracts that only a real browser can show, asserted on
// each shot by scripts/shoot.mjs through scripts/shot-checks.mjs. Each check is an expression run in
// the shot's page once it has settled and been shot, answering `{ ok, detail }`; one that is not ok
// fails the run with its contract's id and its words.

/// ST-C2. Carbon caps its tab list at one 64 px row, so on a Northstar phone the tabs that wrapped
/// onto a second row were clipped to a sliver nobody could tap, while every test passed: jsdom lays
/// nothing out. Here each tab is brought into view and must be drawn whole inside the screen, and be
/// what is under its own middle — not clipped away, not covered.
const TABS_REACHABLE = {
  id: 'ST-C2',
  says: 'every Settings tab can be reached: drawn whole inside the screen, and neither clipped nor covered',
  at: 'both',
  expression: `(async () => {
    const list = document.querySelector('[role="tablist"][aria-label="Settings pages"]');
    if (!list) return { ok: false, detail: 'no Settings tab list on the page' };
    const tabs = [...list.querySelectorAll('[role="tab"]')];
    if (tabs.length === 0) return { ok: false, detail: 'the tab list holds no tab' };
    const edge = document.documentElement.clientWidth;
    const y0 = scrollY;
    const bad = [];
    for (const tab of tabs) {
      const name = (tab.textContent || '').trim();
      tab.scrollIntoView({ block: 'center', inline: 'nearest' });
      await new Promise((r) => requestAnimationFrame(() => r()));
      const r = tab.getBoundingClientRect();
      if (!(r.width > 0 && r.height > 0)) {
        bad.push(name + ' is not drawn');
        continue;
      }
      if (r.left < -0.5 || r.right > edge + 0.5) {
        bad.push(name + ' lies outside the screen, ' + Math.round(r.left) + ' to ' + Math.round(r.right) + ' px of ' + edge);
        continue;
      }
      const hit = document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2);
      if (!hit || !(hit === tab || tab.contains(hit))) {
        const what = hit ? hit.tagName.toLowerCase() + (typeof hit.className === 'string' && hit.className ? '.' + hit.className.split(/\\s+/)[0] : '') : 'nothing';
        bad.push(name + ' is clipped or covered at its middle, by ' + what);
      }
    }
    scrollTo(0, y0);
    return { ok: bad.length === 0, detail: tabs.length + ' tabs at ' + edge + ' px' + (bad.length ? ': ' + bad.join('; ') : '') };
  })()`,
};

/// The checks for each Settings page, by its path.
export const SETTINGS_CHECKS = Object.fromEntries(
  ['accounts', 'claude-code', 'network', 'password', 'notifications', 'router'].map((slug) => [`/settings/${slug}`, [TABS_REACHABLE]]),
);
