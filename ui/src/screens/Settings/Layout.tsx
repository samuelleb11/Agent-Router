// Copied from Northstar ui/src/screens/Settings/Layout.tsx at e9c1589.
// Changed: agent-router's six sections; each tab's href carries the console's basename (/next while it is served there), which a bare path in an anchor does not; the header says what these pages are about here.
import type { MouseEvent } from 'react';
import { Tab, TabList, Tabs } from '@carbon/react';
import { Navigate, Outlet, useHref, useLocation, useNavigate } from 'react-router';
import { Page } from '~/components/Page';
import { PageHeader } from '~/components/PageHeader';
import { SECTIONS, settingsPath, type SectionSlug } from '~/lib/routes';
import styles from './Settings.module.scss';

/// Whether a URL is this section's page or one beneath it — the whole segment, so
/// `/settings/routerx` is not the Router page.
const isUnder = (pathname: string, slug: SectionSlug) => {
  const path = settingsPath(slug);
  return pathname === path || pathname.startsWith(`${path}/`);
};

/// A plain left click is the router's; anything else — the middle button, a modifier held —
/// is the browser's, so "open in a new tab" keeps working on a tab that is a link.
const plain = (e: MouseEvent) => e.button === 0 && !e.metaKey && !e.altKey && !e.ctrlKey && !e.shiftKey;

/// Settings: this router's own, held in its store on the machine it runs on.
///
/// One page header for the whole area, and its pages as a sub-navigation under it. Each tab is
/// a real anchor to its page, so every destination can be middle-clicked, copied and opened in
/// a new tab. The SAME control in a hand — Carbon's tab list wraps rather than scrolls there
/// (Settings.module.scss) — so nothing is a tab on one platform and a picker on the other. The
/// URL is the selection at both widths.
export default function SettingsLayout() {
  const location = useLocation();
  const navigate = useNavigate();
  // The console's own root as the browser sees it: "/" or "/next/". An anchor's href is the
  // browser's, so it needs the basename the router's own navigation adds by itself.
  const root = useHref('/').replace(/\/$/, '');

  const found = SECTIONS.findIndex((s) => isUnder(location.pathname, s.slug));
  // `/settings` on its own is nowhere, and so is a segment none of the pages owns:
  // land on the first page rather than an empty outlet. The address bar shows where.
  if (found < 0) {
    return <Navigate to={settingsPath(SECTIONS[0].slug)} replace />;
  }
  const current = SECTIONS[found] ?? SECTIONS[0];

  return (
    <Page className={styles.settings}>
      <div className={styles.head}>
        <PageHeader
          breadcrumb={[{ label: 'Overview', to: '/' }, { label: 'Settings' }]}
          title="Settings"
          meta={
            <span>
              This router's own: the accounts it serves, how Claude Code reaches it, who on the network may, its
              password and where it sends notifications. Held in its store on the machine it runs on.
            </span>
          }
        />
        {/* Controlled by the URL alone. Carbon still reports a click through `onChange`;
            it is ignored, because the anchor's own handler decides whether the click was
            the router's or the browser's. */}
        <Tabs selectedIndex={found}>
          <TabList aria-label="Settings pages" contained className={styles.tabs}>
            {SECTIONS.map((s) => {
              const path = settingsPath(s.slug);
              return (
                // Carbon's `as` swaps the element but not its prop type, so `href` goes
                // in as a spread rather than a named attribute.
                <Tab
                  key={s.slug}
                  as="a"
                  {...{ href: `${root}${path}` }}
                  onClick={(e) => {
                    if (e.defaultPrevented || !plain(e)) return;
                    e.preventDefault();
                    if (s.slug !== current.slug) void navigate(path);
                  }}
                >
                  {s.label}
                </Tab>
              );
            })}
          </TabList>
        </Tabs>
      </div>
      <Outlet />
    </Page>
  );
}
