// Copied from Northstar ui/src/app/tick.ts at e9c1589.
// Changed: nothing.
import { useEffect, useReducer } from 'react';

/// A re-render beat, so "read 12s ago" keeps ageing on a screen that is not polling.
///
/// `since()` is computed at render time. A screen with nothing to re-read — no query in
/// flight, no stream event — would otherwise keep showing the age it had when it last
/// rendered, which is a display that can no longer say when it was measured. This is not a
/// clock in the query cache's sense: it re-reads nothing and invalidates nothing; it only
/// makes the render current. Returns the beat count so a memo can depend on it.
export function useTick(ms = 1000): number {
  const [tick, beat] = useReducer((n: number) => n + 1, 0);
  useEffect(() => {
    const t = setInterval(beat, ms);
    return () => clearInterval(t);
  }, [ms]);
  return tick;
}
