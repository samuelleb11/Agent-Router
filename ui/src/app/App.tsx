// Copied from Northstar ui/src/app/App.tsx at e9c1589.
// Changed: agent-router's routes (/, /live, /machines, /settings/:section), in ~/app/routeTable so a spec can mount the same table; no /login, because the router's LAN gate is HTTP Basic, which the browser asks for itself; no generated client to configure and no 401 redirect.
import { Suspense } from 'react';
import { QueryClientProvider } from '@tanstack/react-query';
import { RouterProvider, createBrowserRouter } from 'react-router';
import { Theme } from '@carbon/react';
import { queryClient } from '~/app/query';
import { routeTable } from '~/app/routeTable';
import { useTheme } from '~/app/theme';
import { Notices } from '~/app/notices';

const router = createBrowserRouter(routeTable);

export function App() {
  const theme = useTheme();
  return (
    <QueryClientProvider client={queryClient}>
      <Theme theme={theme.carbonTheme}>
        <Suspense fallback={null}>
          <RouterProvider router={router} />
        </Suspense>
        <Notices />
      </Theme>
    </QueryClientProvider>
  );
}
