// Copied from Northstar ui/src/lib/viewport.ts at e9c1589.
// Changed: nothing; the console's one breakpoint is Carbon's, as Northstar's is.
import { useEffect, useState } from 'react';

/// The one breakpoint the console has. Below it the layout is a phone's.
/// Carbon's `md` breakpoint is 672px, and its `breakpoint-down('md')`, by which every
/// stylesheet here switches, asks this, byte for byte: 0.02rem under 42rem, 671.68px. So
/// CSS and JS switch at the same width. Asked as `671px`, they disagreed between 671 and
/// 671.68 CSS pixels, a width a zoom reaches: the shell drew the rail there, and the page,
/// laid out for a phone, kept no margin for it.
export const NARROW_QUERY = '(max-width: 41.98rem)';
const COARSE_QUERY = '(pointer: coarse)';

function useMedia(query: string): boolean {
  const [matches, set] = useState(() => globalThis.matchMedia?.(query).matches ?? false);
  useEffect(() => {
    const mq = globalThis.matchMedia?.(query);
    if (!mq) return;
    const on = (e: MediaQueryListEvent) => set(e.matches);
    set(mq.matches);
    mq.addEventListener('change', on);
    return () => mq.removeEventListener('change', on);
  }, [query]);
  return matches;
}

/// Desktop-first, and the same product in a hand: every fact and every action on both,
/// the layout adapted, the feature set never reduced. Pure layout adapts in CSS; anything
/// needing a different tree reads this.
export function useNarrow(): boolean {
  return useMedia(NARROW_QUERY);
}

/// A touch pointer: affordances that appear on hover must be visible outright.
export function useCoarse(): boolean {
  return useMedia(COARSE_QUERY);
}
