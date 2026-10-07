// New for agent-router: the console's route table, apart from the router that serves it, so a spec
// can mount the same table in memory. Moved here from App.tsx, which was copied from Northstar
// ui/src/app/App.tsx at e9c1589.
import { lazy } from 'react';
import type { RouteObject } from 'react-router';
import { Shell } from '~/app/Shell';
import { ErrorScreen, NotFound } from '~/app/ErrorScreen';

const Overview = lazy(() => import('~/screens/Overview'));
const Sessions = lazy(() => import('~/screens/Sessions'));
const Machines = lazy(() => import('~/screens/Machines'));
const SettingsLayout = lazy(() => import('~/screens/Settings').then((m) => ({ default: m.SettingsLayout })));
const SettingsSection = lazy(() => import('~/screens/Settings').then((m) => ({ default: m.SettingsSection })));

/// Every screen route carries its own `errorElement`, so a crash inside one is caught at
/// that screen and the shell around it survives: the rail, the header and the palette still
/// work, and the reader can navigate away rather than reloading (SH-C8). React Router's own
/// screen is a stack trace addressed to a developer, which on an operator's machine is neither
/// usable nor honest about whose fault it is — see `~/app/ErrorScreen`.
///
/// The route table is the contract every screen is built to. The paths are the ones the old
/// pages answer, because the VS Code extension, the menubar app, the iPhone and every bookmark
/// open them: `/` for the Overview, `/live` for Sessions. `/machines` is new, a hosted router's
/// machines (MA-C1). The screens are lazy, so nothing loads before it is looked at.
///
/// No `/login`: a LAN browser meets the router's password as HTTP Basic, before this page loads
/// at all, and a loopback one is never asked.
export const routeTable: RouteObject[] = [
  {
    path: '/',
    element: <Shell />,
    children: [
      { index: true, element: <Overview />, errorElement: <ErrorScreen /> },
      { path: 'live', element: <Sessions />, errorElement: <ErrorScreen /> },
      { path: 'machines', element: <Machines />, errorElement: <ErrorScreen /> },
      {
        path: 'settings',
        element: <SettingsLayout />,
        errorElement: <ErrorScreen />,
        children: [{ path: ':section', element: <SettingsSection />, errorElement: <ErrorScreen /> }],
      },
      // An address no route claims. Inside the shell, so the rail is still there to leave by.
      { path: '*', element: <NotFound />, errorElement: <ErrorScreen /> },
    ],
    // The shell itself failing has nowhere else to go, so this one replaces the page.
    errorElement: <ErrorScreen />,
  },
];
