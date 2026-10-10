// New for agent-router: what the Settings specs share — a fetch stub that answers by method and
// path and keeps every request with its body, and the router's answers they start from. Used by
// specs only.
import { vi } from 'vitest';
import type { AccountRow, Push, Setup, Shim } from '~/data/settings';
import type { Auth } from '~/model/settings';
import { json } from '~/test-utils';

export interface Asked {
  readonly method: string;
  readonly path: string;
  readonly query: URLSearchParams;
  readonly body: unknown;
}

export type Answer = (req: Asked) => Response | Promise<Response>;

/// Replace `fetch` with one that answers `"POST /api/x"` or, for a GET, `"/api/x"`, and fails loudly
/// for anything it was not told about. Undo it with `vi.unstubAllGlobals()`.
export function stubRouter(answers: Record<string, Answer>) {
  const asked: Asked[] = [];
  const fetch = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const href = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
    const url = new URL(href, 'http://console.test');
    const method = (init?.method ?? 'GET').toUpperCase();
    const body: unknown = typeof init?.body === 'string' ? JSON.parse(init.body) : undefined;
    const req = { method, path: url.pathname, query: url.searchParams, body };
    asked.push(req);
    const answer = answers[`${method} ${url.pathname}`] ?? (method === 'GET' ? answers[url.pathname] : undefined);
    if (!answer) throw new Error(`test fetch: no answer for ${method} ${url.pathname}${url.search}`);
    return answer(req);
  });
  vi.stubGlobal('fetch', fetch);
  return {
    fetch,
    asked,
    answers,
    /// The bodies POSTed to a path, oldest first.
    posts: (path: string) => asked.filter((r) => r.method === 'POST' && r.path === path).map((r) => r.body),
    /// The GETs of a path, with their queries.
    gets: (path: string) => asked.filter((r) => r.method === 'GET' && r.path === path),
    /// Everything sent, as `METHOD /path?query`.
    said: () => asked.map((r) => `${r.method} ${r.path}${[...r.query].length ? `?${r.query.toString()}` : ''}`),
  };
}

/// An answer that is always the same.
export const ok = (data: unknown = { ok: true }, status = 200): Answer => () => json(data, status);

/// A local router at the machine itself, guarded and on the network.
export const LOCAL: Auth = {
  configured: true,
  updated_at: Date.UTC(2026, 9, 1),
  from_loopback: true,
  remote_bind: true,
  listening_on: '0.0.0.0',
  lan_url: 'http://192.168.1.20:8787',
  env_override: false,
  mode: 'local',
  password_pinned: false,
  public_url: null,
};

/// A hosted router (`serve --server`) with its password pinned by the deployment.
export const HOSTED: Auth = { ...LOCAL, mode: 'server', lan_url: 'https://router.example', password_pinned: true, public_url: 'https://router.example' };

/// The accounts as `probe=1` answers them: the built-in login, active; a token; a profile whose
/// login failed with a long reason.
export const ACCOUNTS: AccountRow[] = [
  {
    id: 'default',
    label: 'default',
    source: 'keychain',
    email: 'me@example.com',
    org_name: 'Example Org',
    subscription_type: 'max',
    active: 1,
    created_at: 10,
    usage_key: 1,
    ready: true,
    detail: 'sk-ant-oat01 (subscription OAuth)',
    expires_at: Date.UTC(2026, 9, 6, 9, 0),
    has_token: false,
  },
  {
    id: 'work',
    label: 'work',
    source: 'token',
    email: null,
    org_name: null,
    subscription_type: null,
    active: 0,
    created_at: 20,
    usage_key: 0,
    ready: true,
    detail: 'sk-ant-oat01 (subscription OAuth)',
    expires_at: 0,
    has_token: true,
  },
  {
    id: 'side',
    label: 'side',
    source: 'profile',
    email: null,
    org_name: null,
    subscription_type: 'pro',
    active: 0,
    created_at: 30,
    usage_key: 0,
    ready: false,
    detail:
      'the profile at /Users/me/.agent-router/accounts/side is not logged in: run `CLAUDE_CONFIG_DIR=/Users/me/.agent-router/accounts/side claude auth login` in a terminal, then come back',
    expires_at: 0,
    has_token: false,
  },
];

export const SETUP: Setup = {
  service: { registered: false, supported: true, platform: 'macos' },
  url: 'http://127.0.0.1:8787',
  global_settings_path: '/Users/me/.claude/settings.json',
  managed: [{ path: '/Users/me/code/app/.claude/settings.local.json', scope: 'project', value: 'http://127.0.0.1:8787', previous: null, applied_at: 1 }],
};

export const SHIM: Shim = {
  available: true,
  installed: 1,
  targets: [
    { extension: 'anthropic.claude-code-2.0.77-darwin-arm64', path: '/Users/me/.vscode/extensions/anthropic.claude-code-2.0.77-darwin-arm64/resources/native-binary/claude', state: 'installed' },
    { extension: 'anthropic.claude-code-2.0.78-darwin-arm64', path: '/Users/me/.vscode/extensions/anthropic.claude-code-2.0.78-darwin-arm64/resources/native-binary/claude', state: 'not_installed' },
  ],
  auto: false,
  auto_log: [{ at: Date.now() - 5 * 60_000, what: 'wrapped anthropic.claude-code-2.0.77-darwin-arm64' }],
};

export const PUSH: Push = {
  configured: true,
  team_id: 'ABCDE12345',
  key_id: 'ABC123DEF4',
  bundle_id: 'ca.example.agentrouter',
  key_file: null,
  fcm: { configured: false, project_id: null, client_email: null, private_key_id: null, key_file: null, last_error: null },
  any_configured: true,
  devices: [
    { token_tail: 'a1b2c3', platform: 'apns', name: 'Pocket', environment: 'production', registered_at: Date.now() - 86_400_000, last_seen_at: Date.now() - 120_000, last_result: 'delivered' },
    { token_tail: 'd4e5f6', platform: 'fcm', name: null, environment: 'development', registered_at: Date.now() - 3 * 3_600_000, last_seen_at: null, last_result: null },
  ],
  last_error: null,
  last_sent: null,
};
