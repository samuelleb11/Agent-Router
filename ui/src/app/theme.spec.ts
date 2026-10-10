// New for agent-router: the console's theme as the Shell section of docs/web-console-contracts.md
// states it — a three-way choice that follows the system while it is chosen, one state for the whole
// page, worn as a class on <html> with its colour scheme, and set before the first paint by
// public/theme-boot.js. The theme module keeps its state at module level, so each case imports a
// fresh copy, after the operating system's preference and the browser's storage are what it needs.
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { runInNewContext } from 'node:vm';
import { act, renderHook } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

/// Waits and limits generous enough for a machine busy with other work: a timeout here is a slow
/// machine, never a pass.
vi.setConfig({ testTimeout: 30_000 });

const ui = join(__dirname, '..', '..');
const html = () => document.documentElement;
const DARK = '(prefers-color-scheme: dark)';

/// The operating system's colour preference, which a spec can change while the page is open.
function stubSystem(dark: boolean) {
  let matches = dark;
  const listeners = new Set<(e: MediaQueryListEvent) => void>();
  const list = (query: string) => ({
    get matches() {
      return query === DARK ? matches : false;
    },
    media: query,
    onchange: null,
    addListener: () => {},
    removeListener: () => {},
    addEventListener: (_: string, l: (e: MediaQueryListEvent) => void) => {
      if (query === DARK) listeners.add(l);
    },
    removeEventListener: (_: string, l: (e: MediaQueryListEvent) => void) => listeners.delete(l),
    dispatchEvent: () => false,
  });
  vi.stubGlobal('matchMedia', list);
  return {
    set(next: boolean) {
      matches = next;
      for (const l of [...listeners]) l({ matches: next } as MediaQueryListEvent);
    },
  };
}

/// A fresh theme module: its state is read when it loads.
async function freshTheme() {
  vi.resetModules();
  return import('./theme');
}

/// Storage that refuses every read and write, as a private window or a frame with storage off.
function refuseStorage(): () => void {
  const was = Object.getOwnPropertyDescriptor(window, 'localStorage')!;
  const refuse = () => {
    throw new DOMException('The operation is insecure.', 'SecurityError');
  };
  const refusing = { getItem: refuse, setItem: refuse, removeItem: refuse };
  for (const target of new Set<object>([globalThis, window])) Object.defineProperty(target, 'localStorage', { value: refusing, configurable: true, writable: true });
  return () => {
    for (const target of new Set<object>([globalThis, window])) Object.defineProperty(target, 'localStorage', was);
  };
}

afterEach(() => {
  vi.unstubAllGlobals();
  html().className = '';
  html().style.colorScheme = '';
});

describe('the theme', () => {
  it('SH-C35: is system, light or dark, system by default; light is Carbon white, dark is g100, and system follows the operating system for as long as the page is open', async () => {
    const os = stubSystem(false);
    const { useTheme } = await freshTheme();
    const { result } = renderHook(() => useTheme());
    expect(result.current.choice).toBe('system');
    expect(result.current.carbonTheme).toBe('white');
    expect(html()).toHaveClass('ar-light');

    // The system turns dark while the page is open, and the page follows it.
    act(() => os.set(true));
    expect(result.current.carbonTheme).toBe('g100');
    expect(html()).toHaveClass('ar-dark');
    act(() => os.set(false));
    expect(result.current.carbonTheme).toBe('white');

    // A choice of light or dark holds whatever the system does.
    act(() => result.current.set('dark'));
    expect(result.current.carbonTheme).toBe('g100');
    act(() => os.set(false));
    expect(html()).toHaveClass('ar-dark');
    act(() => result.current.set('light'));
    act(() => os.set(true));
    expect(result.current.carbonTheme).toBe('white');
    expect(html()).toHaveClass('ar-light');

    // Back to the system, which is dark by now.
    act(() => result.current.set('system'));
    expect(result.current.carbonTheme).toBe('g100');

    // One button cycles the three, in this order.
    const order: string[] = [];
    for (let i = 0; i < 3; i++) {
      act(() => result.current.next());
      order.push(result.current.choice);
    }
    expect(order).toEqual(['light', 'dark', 'system']);
  });

  it('SH-C35: the choice is remembered in this browser, and read and written inside try/catch, so a browser that refuses storage starts at system and still switches', async () => {
    stubSystem(false);
    localStorage.setItem('agent-router.theme', 'dark');
    const remembered = await freshTheme();
    expect(renderHook(() => remembered.useTheme()).result.current.choice).toBe('dark');
    expect(html()).toHaveClass('ar-dark');

    const restore = refuseStorage();
    try {
      const refused = await freshTheme();
      const { result } = renderHook(() => refused.useTheme());
      expect(result.current.choice).toBe('system');
      act(() => result.current.set('dark'));
      expect(result.current.choice).toBe('dark');
      expect(html()).toHaveClass('ar-dark');
      act(() => result.current.set('system'));
      expect(html()).toHaveClass('ar-light');
    } finally {
      restore();
    }
  });

  it('SH-C36: one theme state for the whole page: every part sees the choice any part makes, and the page wears it as a class on <html> with a matching color-scheme', async () => {
    stubSystem(false);
    const { useTheme } = await freshTheme();
    const shell = renderHook(() => useTheme());
    const root = renderHook(() => useTheme());
    act(() => shell.result.current.set('dark'));
    expect(root.result.current.choice).toBe('dark');
    expect(root.result.current.carbonTheme).toBe('g100');
    expect(html()).toHaveClass('ar-dark');
    expect(html()).not.toHaveClass('ar-light');
    expect(html().style.colorScheme).toBe('dark');

    act(() => root.result.current.set('light'));
    expect(shell.result.current.choice).toBe('light');
    expect(html()).toHaveClass('ar-light');
    expect(html()).not.toHaveClass('ar-dark');
    expect(html().style.colorScheme).toBe('light');
  });
});

describe('the theme before the first paint', () => {
  const page = new DOMParser().parseFromString(readFileSync(join(ui, 'index.html'), 'utf8'), 'text/html');
  const boot = readFileSync(join(ui, 'public', 'theme-boot.js'), 'utf8');

  /// public/theme-boot.js run as the browser runs it, against a stored choice and a system preference.
  function runBoot(stored: string | null | 'refused', systemDark: boolean | 'none') {
    const classes = new Set<string>();
    const style: { colorScheme?: string } = {};
    const asked: string[] = [];
    const localStorage = {
      getItem: (key: string) => {
        asked.push(key);
        if (stored === 'refused') throw new DOMException('The operation is insecure.', 'SecurityError');
        return stored;
      },
    };
    const context: Record<string, unknown> = {
      localStorage,
      document: { documentElement: { classList: { add: (c: string) => classes.add(c) }, style } },
    };
    if (systemDark !== 'none') context['matchMedia'] = (q: string) => ({ matches: q === DARK && systemDark });
    runInNewContext(boot, context);
    return { classes: [...classes], scheme: style.colorScheme, asked };
  }

  it('SH-C36: index.html runs public/theme-boot.js as a classic script in its head, ahead of the console, so the page wears its theme before it first paints', () => {
    const scripts = [...page.querySelectorAll('script')];
    const first = scripts[0]!;
    expect(first.getAttribute('src')).toBe('/theme-boot.js');
    expect(first.parentElement?.tagName).toBe('HEAD');
    // Classic and blocking: a module, or a deferred or async script, would run after the first paint.
    expect(first.hasAttribute('type')).toBe(false);
    expect(first.hasAttribute('defer')).toBe(false);
    expect(first.hasAttribute('async')).toBe(false);
    const app = scripts.find((s) => s.getAttribute('type') === 'module');
    expect(app?.getAttribute('src')).toBe('/src/main.tsx');
    expect(first.compareDocumentPosition(app!) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(page.querySelector('meta[name="color-scheme"]')?.getAttribute('content')).toBe('light dark');
  });

  it('SH-C36: theme-boot.js wears the class and the color-scheme the console will, from the same remembered choice, and a refused storage falls to the system', async () => {
    expect(runBoot('dark', false)).toEqual({ classes: ['ar-dark'], scheme: 'dark', asked: ['agent-router.theme'] });
    expect(runBoot('light', true)).toEqual({ classes: ['ar-light'], scheme: 'light', asked: ['agent-router.theme'] });
    expect(runBoot(null, true)).toMatchObject({ classes: ['ar-dark'], scheme: 'dark' });
    expect(runBoot(null, false)).toMatchObject({ classes: ['ar-light'], scheme: 'light' });
    expect(runBoot('refused', true)).toMatchObject({ classes: ['ar-dark'], scheme: 'dark' });
    expect(runBoot(null, 'none')).toMatchObject({ classes: ['ar-light'], scheme: 'light' });

    // And the console, starting from the same stored choice, wears the same class: the paint after
    // the first one never flips the page.
    for (const [stored, systemDark] of [
      ['dark', false],
      ['light', true],
      [null, true],
      [null, false],
    ] as const) {
      localStorage.clear();
      if (stored) localStorage.setItem('agent-router.theme', stored);
      stubSystem(systemDark);
      html().className = '';
      await freshTheme();
      expect([...html().classList], `${stored} on a ${systemDark ? 'dark' : 'light'} system`).toEqual(runBoot(stored, systemDark).classes);
      expect(html().style.colorScheme).toBe(runBoot(stored, systemDark).scheme);
    }
  });
});
