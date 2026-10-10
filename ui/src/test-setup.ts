// Copied from Northstar ui/src/test-setup.ts at e9c1589.
// Changed: one wait limit for every spec, 15 s (`configure`, below).
import '@testing-library/jest-dom/vitest';
import { cleanup, configure } from '@testing-library/react';
import { afterEach, vi } from 'vitest';

afterEach(cleanup);

/// Every wait for a screen (`waitFor`, `findBy*`) may take 15 s, not Testing Library's 1 s. The CI
/// runner draws screens several times slower than a Mac, and slower still when its cluster is busy:
/// specs that waited 1 s failed there and passed here (AGEN-0041). A timeout is a slow machine, never
/// a pass, and no spec relies on a wait running out. Each test's own limit is in vite.config.ts.
configure({ asyncUtilTimeout: 15_000 });

/// A Storage in memory, installed over BOTH `globalThis.localStorage` and
/// `window.localStorage`.
///
/// Node 22+ exposes an experimental `localStorage` on its global — an object with no
/// methods unless `--localstorage-file` is set — and vitest's jsdom environment leaves it
/// in place because the key already exists, so neither name can store anything. This is
/// defined with `Object.defineProperty` rather than `vi.stubGlobal` so a spec's own
/// `vi.unstubAllGlobals()` returns here, not to the dead stub. Cleared after each test.
class MemoryStorage implements Storage {
  private map = new Map<string, string>();
  get length() {
    return this.map.size;
  }
  clear() {
    this.map.clear();
  }
  getItem(key: string) {
    return this.map.get(key) ?? null;
  }
  key(index: number) {
    return [...this.map.keys()][index] ?? null;
  }
  removeItem(key: string) {
    this.map.delete(key);
  }
  setItem(key: string, value: string) {
    this.map.set(key, String(value));
  }
}

const storage = new MemoryStorage();
for (const target of new Set<object>([globalThis, globalThis.window].filter(Boolean))) {
  Object.defineProperty(target, 'localStorage', { value: storage, configurable: true, writable: true, enumerable: true });
}
afterEach(() => storage.clear());

// Carbon components measure and observe; jsdom provides neither.
globalThis.ResizeObserver ??= class {
  observe() {}
  unobserve() {}
  disconnect() {}
};
globalThis.matchMedia ??= ((query: string) => ({
  matches: false,
  media: query,
  onchange: null,
  addListener: () => {},
  removeListener: () => {},
  addEventListener: () => {},
  removeEventListener: () => {},
  dispatchEvent: () => false,
})) as typeof globalThis.matchMedia;
vi.stubGlobal('scrollTo', () => {});
// Carbon's list boxes scroll the highlighted option into view; jsdom has no layout. A spec
// under `// @vitest-environment node` (the graph's engine, which runs its layout libraries with
// no DOM at all) has no Element to patch, and this file runs before every spec.
if (typeof Element !== 'undefined') Element.prototype.scrollIntoView ??= () => {};
