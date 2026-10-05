// New for agent-router: the live view's `say` timer (assets/live.html), one per place a notice
// belongs (SE-C178). Drawn by NoticeLine in ./Notice.
import { useCallback, useEffect, useRef, useState } from 'react';
import type { Notice } from '~/model/session';

/// One place's notice: a new one replaces the one before and restarts its timer; a success clears
/// itself after its time, a warning or a failure stays until dismissed or replaced (SE-C180).
/// Showing, clearing and showing again, with or without a timer and in any order, never recurses
/// (SE-C181), and nothing is set once the place has gone (SE-C193).
export function useNotice(): readonly [Notice | null, (n: Notice | null) => void] {
  const [notice, setNotice] = useState<Notice | null>(null);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const alive = useRef(false);
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
      if (timer.current) clearTimeout(timer.current);
      timer.current = null;
    };
  }, []);
  const say = useCallback((n: Notice | null) => {
    if (timer.current) clearTimeout(timer.current);
    timer.current = null;
    if (!alive.current) return;
    setNotice(n);
    if (n?.ms) {
      timer.current = setTimeout(() => {
        timer.current = null;
        if (alive.current) setNotice(null);
      }, n.ms);
    }
  }, []);
  return [notice, say] as const;
}
