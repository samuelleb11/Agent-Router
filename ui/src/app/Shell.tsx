// Copied from Northstar ui/src/app/Shell.tsx at e9c1589.
// Changed: agent-router's navigation (Overview, Sessions, Machines on a hosted router, the projects, Settings at the foot); the freshness badge reads the router's health and the open session's stream; no sign-in and no log-out, because the router's LAN gate is HTTP Basic, which the browser asks for itself; the address the console was reached at beside the name; no wide routes; the phone drawer closes on every arrival, a project link's included; the palette is open at a location as the drawer is, and opening either puts the other away; the first-boot password prompt is decided here, on every screen but Settings, in a chunk of its own.
import { Suspense, lazy, useCallback, useEffect, useMemo, useState, type MouseEvent } from 'react';
import { Link, Outlet, useHref, useLocation, useNavigate } from 'react-router';
import {
  Content,
  Header,
  HeaderGlobalAction,
  HeaderGlobalBar,
  HeaderMenuButton,
  HeaderName,
  SideNav,
  SideNavDivider,
  SideNavItems,
  SideNavLink,
  SkipToContent,
  Theme,
} from '@carbon/react';
import { Asleep, Awake, Dashboard, Screen, Search, Settings, Terminal } from '@carbon/icons-react';
import { useLive } from '~/app/live';
import { keys } from '~/app/query';
import { useTheme } from '~/app/theme';
import { projectKey, projectName, UNRESOLVED, useProjects } from '~/data/projects';
import { availabilityOf, useRead } from '~/data/held';
import { readAuth } from '~/data/router';
import { useNarrow } from '~/lib/viewport';
import { routes } from '~/lib/routes';
import { InitialsIcon } from '~/components/InitialsIcon';
import { LiveBadge } from '~/components/LiveBadge';
import { Palette, usePaletteShortcut } from '~/screens/Palette';
import styles from './Shell.module.scss';

/// Whether the first-boot password prompt opens (ST-C80), in a chunk of its own: it reads the
/// router's auth with Settings' own reader, which every page's first load need not carry.
const FirstBootGate = lazy(() => import('~/screens/Settings/FirstBootGate').then((m) => ({ default: m.FirstBootGate })));

const RAIL_KEY = 'agent-router.rail';

/// How often the rail asks what kind of router this is: as Settings and the Overview ask it (ST-C5).
const AUTH_EVERY_MS = 30_000;

function readRail(): boolean {
  try {
    return localStorage.getItem(RAIL_KEY) !== 'collapsed';
  } catch {
    return true;
  }
}

/// A plain left click is the router's; a modified one is the browser's, so "open in a new
/// tab" keeps working on the name in the header.
const plain = (e: MouseEvent) => e.button === 0 && !e.metaKey && !e.altKey && !e.ctrlKey && !e.shiftKey;

/// The shell: Carbon's UI Shell, with a rail that collapses to icons.
///
/// The rail holds the console's three places and, between them, the projects, each opening
/// the Overview scoped to it, the way Northstar's lists its projects. Collapsed, a project is
/// its initials, so the collapsed state is still readable rather than a column of identical
/// glyphs. The choice is remembered per viewer.
export function Shell() {
  const [expanded, setExpanded] = useState(readRail);
  const theme = useTheme();
  const live = useLive();
  const projects = useProjects();
  // Machines is offered on a hosted router only: a local router takes no machines, and until
  // /api/auth has said which kind this is, it is not offered at all (MA-C1). `mode` is read as
  // `isHosted` in ~/model/settings reads it, which every page's first load need not carry.
  const auth = useRead({ queryKey: keys.auth, queryFn: ({ signal }) => readAuth(signal), refetchInterval: AUTH_EVERY_MS });
  const hosted = auth.value?.mode === 'server';
  const navigate = useNavigate();
  const location = useLocation();
  // The console's root as the browser addresses it: the header's name is a
  // real link, and an href is the browser's, not the router's.
  const home = useHref('/');
  // The address the console was reached at, as the browser has it: the same for the page's life.
  const host = globalThis.location.host;

  // On a phone there is no room beside the page for a rail, so the same navigation is a
  // drawer over it: opened from the header's menu button, closed by the overlay, by Escape,
  // or by arriving somewhere. Same links, same order — only the container differs. Nothing
  // about the drawer is remembered; it is a moment, not a preference.
  //
  // It is open AT a location: the one it was opened on. Every navigation is a new location,
  // the same address followed again included, so arriving anywhere closes it without an
  // effect to remember to run — a project link changes only the query, which a pathname
  // check would miss.
  const narrow = useNarrow();
  const [drawerAt, setDrawerAt] = useState<string | null>(null);
  const drawer = narrow && drawerAt === location.key;

  // ⌘K is primary navigation. The same palette opens from the header's search action, which
  // is the whole of it on a phone. Like the drawer it is open AT a location, so arriving
  // anywhere puts it away. The two never stand open together: on a phone both lie under the
  // header, which stays in reach while either is open (SH-C51), so opening one puts the other
  // away rather than leaving one hidden beneath the other.
  const [paletteAt, setPaletteAt] = useState<string | null>(null);
  const paletteOpen = paletteAt === location.key;
  const openPalette = useCallback(() => {
    setDrawerAt(null);
    setPaletteAt(location.key);
  }, [location.key]);
  const closePalette = useCallback(() => setPaletteAt(null), []);
  usePaletteShortcut(openPalette);

  const closeDrawer = useCallback(() => setDrawerAt(null), []);
  const toggleDrawer = useCallback(() => {
    setPaletteAt(null);
    setDrawerAt((at) => (at === location.key ? null : location.key));
  }, [location.key]);
  // Escape from anywhere in the page, the menu button that opened it included.
  useEffect(() => {
    if (!drawer) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setDrawerAt(null);
    };
    document.addEventListener('keydown', onKey);
    return () => document.removeEventListener('keydown', onKey);
  }, [drawer]);

  const toggle = useCallback(() => {
    setExpanded((was) => {
      const next = !was;
      try {
        localStorage.setItem(RAIL_KEY, next ? 'expanded' : 'collapsed');
      } catch {
        /* not remembered; still applied */
      }
      return next;
    });
  }, []);

  // Where the reader is, for the rail's current mark. A project's link differs from the
  // Overview's only in its query, which a NavLink does not compare, so the marks are worked
  // out here: the Overview is current unscoped, and a project when the Overview is scoped to it.
  const scoped = location.pathname === '/' ? new URLSearchParams(location.search).get('project') : null;
  const under = (path: string) => location.pathname === path || location.pathname.startsWith(`${path}/`);
  const current = (on: boolean) => (on ? { isActive: true, 'aria-current': 'page' as const } : { isActive: false });

  const list = projects.value;
  const icons = useMemo(() => new Map((list ?? []).map((p) => [projectKey(p), InitialsIcon(projectName(p.project))])), [list]);
  const open = narrow ? drawer : expanded;
  const availability = availabilityOf(projects);

  return (
    <>
      <Theme theme="g100">
        <Header aria-label="Agent Router" className={styles.header}>
          <SkipToContent />
          {narrow ? (
            <HeaderMenuButton aria-label={drawer ? 'Close the menu' : 'Open the menu'} isActive={drawer} onClick={toggleDrawer} />
          ) : (
            <HeaderMenuButton
              aria-label={expanded ? 'Collapse the rail' : 'Expand the rail'}
              isCollapsible
              isActive={expanded}
              onClick={toggle}
            />
          )}
          <div className={styles.brand}>
            <HeaderName
              prefix=""
              href={home}
              onClick={(e: MouseEvent) => {
                if (!plain(e)) return;
                e.preventDefault();
                void navigate('/');
              }}
            >
              Agent Router
            </HeaderName>
            {/* Which router this tab is showing: the Mac's, one on the LAN, or a hosted one (SH-C9). */}
            <span className={styles.host} data-host="">
              {host}
            </span>
          </div>
          <HeaderGlobalBar>
            <LiveBadge mode={live.mode} label={live.label} />
            <HeaderGlobalAction aria-label="Search and commands (⌘K)" tooltipAlignment="end" onClick={openPalette}>
              <Search size={20} />
            </HeaderGlobalAction>
            <HeaderGlobalAction aria-label={`Theme: ${theme.choice} — click to change`} tooltipAlignment="end" onClick={theme.next}>
              {theme.dark ? <Awake size={20} /> : <Asleep size={20} />}
            </HeaderGlobalAction>
          </HeaderGlobalBar>
        </Header>
      </Theme>

      <SideNav
        className={styles.rail}
        aria-label="Console"
        expanded={open}
        isRail={!narrow}
        isPersistent={!narrow}
        isChildOfHeader
        onOverlayClick={closeDrawer}
        addMouseListeners={false}
        addFocusListeners={false}
      >
        <SideNavItems>
          <SideNavLink renderIcon={Dashboard} as={Link} to={routes.overview} {...current(location.pathname === '/' && scoped === null)}>
            Overview
          </SideNavLink>
          <SideNavLink renderIcon={Terminal} as={Link} to={routes.sessions} {...current(under(routes.sessions))}>
            Sessions
          </SideNavLink>
          {/* The machine concept's icon (CONCEPTS.machine), imported alone so the rail does not
              carry the whole registry into every page's first load. */}
          {hosted && (
            <SideNavLink renderIcon={Screen} as={Link} to={routes.machines} {...current(under(routes.machines))}>
              Machines
            </SideNavLink>
          )}
          {/* A list item, not Northstar's div: the only child a <ul> may have. */}
          <li className={styles.section} aria-hidden={!open}>
            Projects
          </li>

          {(list ?? []).map((p) => {
            const key = projectKey(p);
            return (
              <SideNavLink
                key={key}
                as={Link}
                to={routes.project(key)}
                renderIcon={icons.get(key)}
                title={p.project ?? UNRESOLVED}
                {...current(scoped === key)}
              >
                {projectName(p.project)}
              </SideNavLink>
            );
          })}
          {/* An empty list and a list that could not be read are different facts, and a rail
              that drew nothing for both would hide which one it is. */}
          {availability === 'unreachable' && (
            <li className={styles.note} aria-hidden={!open}>
              Could not read the projects
            </li>
          )}
          {availability === 'read' && list?.length === 0 && (
            <li className={styles.note} aria-hidden={!open}>
              No projects yet
            </li>
          )}
        </SideNavItems>

        {/*
          Settings sits at the FOOT of the rail, under the projects rather than above them:
          the list above is where the work is and it grows, and a destination you visit
          occasionally should not sit between the viewer and the thing they came for — nor
          move down the list every time a project appears. Carbon's items list is `flex: 1`,
          so a second list after it is pinned to the bottom by the layout itself.
        */}
        <SideNavDivider className={styles.footerRule} />
        <SideNavItems className={styles.footer}>
          <SideNavLink renderIcon={Settings} as={Link} to={routes.settings} {...current(under(routes.settings))}>
            Settings
          </SideNavLink>
        </SideNavItems>
      </SideNav>

      <Content id="main-content" className={styles.content}>
        {/* The shell stays drawn while a screen's chunk loads. */}
        <Suspense fallback={null}>
          <Outlet />
        </Suspense>
      </Content>
      <Palette open={paletteOpen} onClose={closePalette} />
      {/* The first-boot password prompt: on any screen but Settings, once (ST-C80). */}
      <Suspense fallback={null}>
        <FirstBootGate />
      </Suspense>
    </>
  );
}
