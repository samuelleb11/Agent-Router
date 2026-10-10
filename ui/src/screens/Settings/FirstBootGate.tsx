// New for agent-router: whether the first-boot password prompt opens, decided on every screen but
// Settings (ST-C80). Kept small, because the shell carries it into every page's first load; the
// prompt itself arrives in a chunk of its own, only when it opens.
import { lazy, Suspense, useState } from 'react';
import { useLocation } from 'react-router';
import { useAuth } from '~/data/settings';
import { asksFirstPassword } from '~/model/settings';

const FirstBoot = lazy(() => import('./FirstBoot'));

/// The router listens on the network, no password is set, the console is read from this machine,
/// the router is local, the password is not pinned, the reader has not dismissed it in this load,
/// and Settings is not on screen: all of them, or it stays shut.
export function FirstBootGate() {
  const auth = useAuth();
  const { pathname } = useLocation();
  const [dismissed, setDismissed] = useState(false);
  const settingsOpen = pathname === '/settings' || pathname.startsWith('/settings/');
  if (!asksFirstPassword(auth.value, { dismissed, settingsOpen })) return null;
  return (
    <Suspense fallback={null}>
      <FirstBoot auth={auth.value} onClose={() => setDismissed(true)} />
    </Suspense>
  );
}
