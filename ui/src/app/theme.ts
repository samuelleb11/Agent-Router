// Copied from Northstar ui/src/app/theme.ts at e9c1589.
// Changed: the storage key is agent-router.theme and the classes are ar-light and ar-dark, the same as public/theme-boot.js, which sets them before the first paint.
import { useCallback, useSyncExternalStore } from 'react';

export type ThemeChoice = 'system' | 'light' | 'dark';
const KEY = 'agent-router.theme';

function readChoice(): ThemeChoice {
  try {
    const v = localStorage.getItem(KEY);
    return v === 'light' || v === 'dark' ? v : 'system';
  } catch {
    return 'system';
  }
}

function systemDark(): boolean {
  return globalThis.matchMedia?.('(prefers-color-scheme: dark)').matches ?? false;
}

interface ThemeState {
  readonly choice: ThemeChoice;
  readonly sysDark: boolean;
}

/// ONE theme, shared by every caller.
///
/// This was `useState` inside the hook, which gave each caller its own copy: the shell's
/// toggle changed the shell's copy, the app root's `<Theme>` wrapper kept the old one, and
/// since that wrapper re-declares every Carbon token below the `<html>` class, the page did
/// not change colour at all. A setting that two parts of one page can disagree about is not
/// a setting, so the state lives here, outside React, and every `useTheme()` reads it.
let state: ThemeState = { choice: readChoice(), sysDark: systemDark() };
const listeners = new Set<() => void>();
const snapshot = (): ThemeState => state;

function subscribe(l: () => void): () => void {
  listeners.add(l);
  return () => {
    listeners.delete(l);
  };
}

function isDark(s: ThemeState): boolean {
  return s.choice === 'system' ? s.sysDark : s.choice === 'dark';
}

/// The class on `<html>`, which `styles.scss` and `public/theme-boot.js` both switch on.
/// Applied here, at the one place the state changes, rather than in an effect per caller.
function apply(s: ThemeState): void {
  if (typeof document === 'undefined') return;
  const el = document.documentElement;
  const dark = isDark(s);
  el.classList.toggle('ar-dark', dark);
  el.classList.toggle('ar-light', !dark);
  el.style.colorScheme = dark ? 'dark' : 'light';
}

function put(next: ThemeState): void {
  state = next;
  apply(state);
  for (const l of [...listeners]) l();
}

apply(state);

// Watched once, not once per caller: "follow the system" has to keep following it.
globalThis.matchMedia?.('(prefers-color-scheme: dark)').addEventListener?.('change', (e) => {
  put({ ...state, sysDark: e.matches });
});

/// Light or dark, chosen by the viewer and remembered in this browser.
///
/// Three choices rather than two: "follow the system" is what most people want, and a
/// toggle that forgets it forces a decision nobody asked for.
///
/// localStorage is a per-viewer convenience inside try/catch: a private window simply means
/// the choice is not remembered, never that the page fails.
export function useTheme() {
  const s = useSyncExternalStore(subscribe, snapshot, snapshot);
  const dark = isDark(s);

  const set = useCallback((choice: ThemeChoice) => {
    try {
      if (choice === 'system') localStorage.removeItem(KEY);
      else localStorage.setItem(KEY, choice);
    } catch {
      /* not remembered; still applied */
    }
    put({ ...state, choice });
  }, []);

  /// Cycle for a single button: system → light → dark → system.
  const next = useCallback(() => {
    const order: ThemeChoice[] = ['system', 'light', 'dark'];
    set(order[(order.indexOf(state.choice) + 1) % order.length]!);
  }, [set]);

  return { choice: s.choice, dark, set, next, carbonTheme: dark ? ('g100' as const) : ('white' as const) };
}
