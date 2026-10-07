// Copied from Northstar ui/vite.config.ts at e9c1589.
// Changed: the dev server binds 127.0.0.1 only, /api and /_router go to a scratch router on AGENT_ROUTER_DEV_PORT (8799 by default, never 8787), and the build inlines no asset.
/// <reference types="vitest/config" />
import { fileURLToPath, URL } from 'node:url';
import react from '@vitejs/plugin-react';
import { defineConfig } from 'vite';

/// The live router answers on 8787. A dev server pointed there would read and WRITE the store
/// the operator's own Claude Code sessions run through, so it is refused outright rather than
/// left to a typo.
const LIVE_ROUTER_PORT = '8787';
const devPort = process.env['AGENT_ROUTER_DEV_PORT'] ?? '8799';
if (devPort === LIVE_ROUTER_PORT) {
  throw new Error(`AGENT_ROUTER_DEV_PORT=${devPort} is the live router; start a scratch router on another port`);
}
const target = `http://127.0.0.1:${devPort}`;

/// The router serves /api and /_router itself in production, same-origin with this app, so a
/// relative path is right in both places and the dev server only has to forward them.
///
/// `changeOrigin` stays false. The router's guard accepts any loopback Host, whatever its port,
/// and for a write it requires the `Origin` the browser sends to name the same authority as
/// `Host`. Rewritten to the router's own address, `Host` would stop matching the dev server's
/// `Origin`, and every write from here would be refused as cross-site.
const proxied = { target, secure: false, changeOrigin: false };

export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: { '~': fileURLToPath(new URL('./src', import.meta.url)) },
  },
  css: {
    preprocessorOptions: {
      scss: {
        // Carbon's Sass reaches into node_modules for its own partials.
        loadPaths: ['node_modules'],
        quietDeps: true,
        silenceDeprecations: ['global-builtin', 'import'],
      },
    },
  },
  server: {
    // LOOPBACK ONLY, DELIBERATELY. The router's guard trusts the connecting peer: a request
    // from 127.0.0.1 is the operator's own machine and is never asked for the password. The dev
    // server is a hop that erases where a request came from, since everything it forwards
    // arrives from its own loopback address. Listening on the LAN, it would make every browser
    // on the network look local, and the password would never be asked for. Northstar forwards
    // the browser's address in X-Forwarded-For to a server that honours it; the router does
    // not, so the only safe dev server here is one the LAN cannot reach.
    host: '127.0.0.1',
    port: Number(process.env['AGENT_ROUTER_UI_PORT'] ?? 4300),
    strictPort: true,
    proxy: {
      '/api': proxied,
      '/_router': proxied,
    },
  },
  build: {
    outDir: 'dist',
    sourcemap: false,
    // Nothing inlined as a data: URL. The router serves the console under a CSP of
    // `script-src 'self'; style-src 'self'; font-src 'self'`, and an inlined asset would need
    // `data:` added to one of them. Every asset is a file under /assets instead.
    assetsInlineLimit: 0,
  },
  test: {
    globals: true,
    environment: 'jsdom',
    setupFiles: ['./src/test-setup.ts'],
    css: false,
    // Specs draw whole screens in jsdom. The CI runner builds with no cache and runs them about
    // twice as slowly as a Mac, so Vitest's 5 s default failed a screen that drew in 5.9 s
    // (SE-C216, pipeline 3307). A slow machine is not a broken screen. Specs that need longer
    // still set their own.
    testTimeout: 30_000,
    // The fixture harness (`harness/`, `pnpm harness`) is tested where it lives.
    include: ['src/**/*.spec.{ts,tsx}', 'harness/**/*.spec.{ts,tsx}'],
  },
});
