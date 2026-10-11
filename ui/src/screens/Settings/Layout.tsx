// Copied from Northstar ui/src/screens/Settings/Layout.tsx at e9c1589.
// Changed: agent-router's six sections, offered as the router allows (none of the machine-only ones before /api/auth has said it is local, nor on a hosted router); each tab's href is resolved through the router, as a bare path in an anchor is not; the header says what these pages are about here.
import { useEffect, useState, type MouseEvent } from 'react';
import { Button, Tab, TabList, Tabs } from '@carbon/react';
import { ArrowUp } from '@carbon/icons-react';
import { Navigate, Outlet, useHref, useLocation, useNavigate } from 'react-router';
import { Page } from '~/components/Page';
import { PageHeader } from '~/components/PageHeader';
import { useAuth } from '~/data/settings';
import { SECTIONS, settingsPath, type SectionSlug } from '~/lib/routes';
import { useCoarse } from '~/lib/viewport';
import { settingsPages } from '~/model/settings';
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
  const auth = useAuth();
  // The pages this router has (ST-C53, ST-C72, ST-C101). An address for one it has not is still
  // its page, which says why (ST-C103); it is simply not a tab.
  const offered = settingsPages(auth.value);
  const tabs = SECTIONS.filter((s) => offered.includes(s.slug));
  // The console's own root as the browser sees it. An anchor's href is the browser's, so it is
  // resolved through the router, which would add any basename by itself.
  const root = useHref('/').replace(/\/$/, '');

  const found = SECTIONS.findIndex((s) => isUnder(location.pathname, s.slug));
  // `/settings` on its own is nowhere, and so is a segment none of the pages owns:
  // land on the first page rather than an empty outlet. The address bar shows where.
  if (found < 0) {
    return <Navigate to={settingsPath(SECTIONS[0].slug)} replace />;
  }
  const current = SECTIONS[found] ?? SECTIONS[0];
  const selected = tabs.findIndex((s) => s.slug === current.slug);

  return (
    <Page className={styles.settings}>
      <div className={styles.head}>
        <PageHeader
          breadcrumb={[{ label: 'Overview', to: '/' }, { label: 'Settings' }]}
          title="Settings"
          meta={
            <span>
              This router's own: the accounts it serves and which model goes to which, how clients reach it,
              access tokens, its password and where it sends notifications. Held in its store on the
              machine it runs on.
            </span>
          }
        />
        {/* Controlled by the URL alone. Carbon still reports a click through `onChange`;
            it is ignored, because the anchor's own handler decides whether the click was
            the router's or the browser's. */}
        <Tabs selectedIndex={selected}>
          <TabList aria-label="Settings pages" contained className={styles.tabs}>
            {tabs.map((s) => {
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
      <BackToTop />
    </Page>
  );
}

/// "Back to top", once the page is 600 px down, smoothly unless reduced motion is asked for: the
/// Settings pages scroll, the Accounts and Notifications pages a long way in a hand (SH-C48).
function BackToTop() {
  const coarse = useCoarse();
  const [down, setDown] = useState(() => (globalThis.scrollY ?? 0) >= 600);
  useEffect(() => {
    const on = () => setDown(window.scrollY >= 600);
    window.addEventListener('scroll', on, { passive: true });
    return () => window.removeEventListener('scroll', on);
  }, []);
  if (!down) return null;
  const reduced = globalThis.matchMedia?.('(prefers-reduced-motion: reduce)').matches ?? false;
  return (
    <Button className={styles.toTop} kind="secondary" size={coarse ? 'lg' : 'md'} renderIcon={ArrowUp} onClick={() => window.scrollTo({ top: 0, behavior: reduced ? 'auto' : 'smooth' })}>
      Back to top
    </Button>
  );
}
