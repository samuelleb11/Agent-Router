// Copied from Northstar ui/vite.harness.config.ts at e9c1589.
// Changed: its port is 4392, on loopback; it starts the router's console on fixtures, where Northstar's starts its own.
import { defineConfig, type Plugin } from 'vite';
import base from './vite.config';

/// The fixture harness: the console's own build settings, served with every read of the router
/// answered from fixtures (`harness/fixtureFetch.ts`) and every event stream played from them
/// (`harness/eventSource.ts`), so it needs no router and reaches none — not 8787, not a scratch
/// one.
///
/// `pnpm harness`, then http://127.0.0.1:4392/. `HARNESS_PORT` moves it and `HARNESS_CACHE_DIR`
/// moves Vite's dependency cache, which belongs under a directory named node_modules wherever it
/// goes: @vitejs/plugin-react runs Babel over every module outside one, the optimised react-dom
/// included. Three differences from `vite.config.ts`, each the point of it:
///
/// - **No proxy.** Nothing under /api or /_router leaves the page: `harness/main.tsx` answers it,
///   and a read it has no answer for fails in the page, loudly, rather than reaching a router.
/// - **Its own port, held strictly, on loopback.** The dev server's is 4300; a harness that
///   drifted to the next free port could be answering on someone else's, and a screenshot of it
///   would be of the wrong thing.
/// - **The same index.html, with the harness's entry.** The page is served as the console
///   serves it, theme script included, so the harness cannot drift from it; only the module it
///   starts is swapped, and a page that no longer names the console's entry is refused rather
///   than served without fixtures.
const ENTRY = '/src/main.tsx';
const HARNESS_ENTRY = '/harness/main.tsx';

const harnessEntry = (): Plugin => ({
  name: 'agent-router-harness-entry',
  apply: 'serve',
  transformIndexHtml: {
    order: 'pre',
    handler: (html) => {
      if (!html.includes(`src="${ENTRY}"`)) throw new Error(`harness: index.html no longer starts ${ENTRY}; update vite.harness.config.ts`);
      return html.replace(`src="${ENTRY}"`, `src="${HARNESS_ENTRY}"`);
    },
  },
});

/// `server` is replaced whole below, which is what drops the proxy.
const { plugins = [], ...shared } = base;

export default defineConfig({
  ...shared,
  plugins: [...plugins, harnessEntry()],
  cacheDir: process.env['HARNESS_CACHE_DIR'] ?? 'node_modules/.vite-harness',
  server: { host: '127.0.0.1', port: Number(process.env['HARNESS_PORT'] ?? 4392), strictPort: true },
});
