// Copied from Northstar ui/src/app/query.ts at e9c1589.
// Changed: the keys are the router's reads; the shell needs two of them so far.
import { QueryClient } from '@tanstack/react-query';
import { POLL_MS } from '~/lib/freshness';

/// One query client, and with it one freshness clock.
///
/// Three views with three timers is three freshness clocks and three chances to disagree
/// about "as of when". Here the cache is the clock — every view reads the same query, and an
/// invalidation moves all of them together.
///
/// `placeholderData: keep` is the "hold what was last read" rule. A refetch that fails leaves
/// the previous `data` in place and raises `isError` beside it, so a view renders what it had
/// WITH its age rather than replacing the board with an error message. The `held()` helper
/// turns that pair into the shape a view renders.
export const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      staleTime: POLL_MS,
      // A failed poll must not blank the screen; one retry, then show what we had.
      retry: 1,
      refetchOnWindowFocus: true,
      // Re-read when the tab is looked at again: a read that ran while hidden is stale by
      // exactly as long as nobody was looking.
      refetchOnReconnect: true,
    },
  },
});

/// Every query key the console uses, in one place, so an invalidation cannot miss a view
/// by spelling its key differently.
export const keys = {
  health: ['health'] as const,
  projects: ['projects'] as const,
};
