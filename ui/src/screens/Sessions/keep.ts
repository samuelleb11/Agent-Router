// New for agent-router: where a session's half-typed things wait while the reader is in another.
//
// The screen under a session is keyed on its id (SE-C192), so nothing typed into one is ever drawn
// in another: Northstar once kept a screen mounted across a change of route and saved a draft onto
// the next project (940a783). Keyed alone, though, a draft would be lost to a look at a sibling's
// tab. So what is half-typed is kept here, by its session's id, and handed back only to that
// session's screen: it stays with its session and follows the reader nowhere.
import { createContext, useCallback, useContext, useState } from 'react';

/// One per Sessions screen, so a reload, or another test, starts from nothing.
export const KeepContext = createContext<Map<string, unknown> | null>(null);

/// A value kept under `key` — which names its session — across the reader's visits to others.
export function useKept<T>(key: string, initial: T): readonly [T, (next: T) => void] {
  const kept = useContext(KeepContext);
  const [value, setValue] = useState<T>(() => (kept?.has(key) ? (kept.get(key) as T) : initial));
  const set = useCallback(
    (next: T) => {
      setValue(next);
      kept?.set(key, next);
    },
    [kept, key],
  );
  return [value, set] as const;
}
