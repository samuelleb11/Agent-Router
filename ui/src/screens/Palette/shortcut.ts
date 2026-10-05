// Copied from Northstar ui/src/screens/Palette/shortcut.ts at e9c1589.
// Changed: nothing; the palette it opens is a placeholder until AGEN-0007.
import { useEffect, useRef } from 'react';

/// ⌘K / Ctrl-K on the document opens the palette. Bound once; the latest `onOpen` is read
/// through a ref so a caller may pass an inline function without re-binding on every
/// render. The palette is primary navigation (Northstar's docs/13 §8), so the shortcut is
/// global — it fires from inside a field too.
///
/// In its own module, apart from the component, so the component's module exports only
/// components and fast refresh can replace it in place.
export function usePaletteShortcut(onOpen: () => void) {
  const latest = useRef(onOpen);
  useEffect(() => {
    latest.current = onOpen;
  }, [onOpen]);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && !e.altKey && !e.shiftKey && e.key.toLowerCase() === 'k') {
        e.preventDefault();
        latest.current();
      }
    };
    document.addEventListener('keydown', onKey);
    return () => document.removeEventListener('keydown', onKey);
  }, []);
}
