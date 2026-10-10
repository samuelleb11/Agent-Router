// New for agent-router: what the fixture harness answers for the reads Settings makes, in the
// router's own shapes (src/auth.rs, src/api.rs, src/setup.rs, src/shim.rs, src/notify.rs,
// src/login.rs). Invented, every one. A local router, guarded and on the network, so every page
// is drawn and the first-boot prompt stays shut; the long names and the long reason are there to
// see a phone wrap them.
import type { AccountRow, LoginStatus, ModelRow, Push, Setup, Shim } from '~/data/settings';
import type { Auth } from '~/model/settings';

const AT = Date.UTC(2026, 9, 5, 9, 0, 0);
const DAY = 86_400_000;

export const AUTH: Auth = {
  configured: true,
  updated_at: AT - 3 * DAY,
  from_loopback: true,
  remote_bind: true,
  listening_on: '0.0.0.0',
  lan_url: 'http://192.168.1.20:8787',
  env_override: false,
  mode: 'local',
  password_pinned: false,
  public_url: null,
};

/// The rows as stored: what any read without `probe=1` answers.
export const ACCOUNTS: readonly AccountRow[] = [
  {
    id: 'default',
    label: 'default',
    source: 'keychain',
    profile_dir: null,
    email: 'sample.person@example.com',
    org_name: 'Sample Org',
    subscription_type: 'max',
    active: 1,
    created_at: AT - 60 * DAY,
    usage_key: 1,
  },
  {
    id: 'work',
    label: 'work',
    source: 'token',
    profile_dir: null,
    email: 'a.rather.long.identity.for.a.phone@an-example-organisation.example',
    org_name: null,
    subscription_type: null,
    active: 0,
    created_at: AT - 20 * DAY,
    usage_key: 0,
  },
  {
    id: 'side-project',
    label: 'side project',
    source: 'profile',
    profile_dir: '/Users/sample/.agent-router/accounts/side-project',
    email: null,
    org_name: null,
    subscription_type: 'pro',
    active: 0,
    created_at: AT - 5 * DAY,
    usage_key: 0,
  },
];

/// The same rows with each credential checked, as `probe=1` answers them: every one ready.
///
/// A healthy router is the harness's default. The Overview checks the credentials once per load and
/// opens Settings → Accounts by itself when one that could sign does not work (ST-C7), so a failing
/// row here would send every harness load of / to Settings, and no shot of the Overview would show
/// the Overview. The failing cases — a profile that is not logged in, a token that expired — live in
/// the specs, which build their answers in src/screens/Settings/testing.ts.
export const PROBED: readonly AccountRow[] = [
  { ...ACCOUNTS[0]!, ready: true, detail: 'sk-ant-oat01 (subscription OAuth)', expires_at: AT + 6 * 3_600_000, has_token: false },
  { ...ACCOUNTS[1]!, ready: true, detail: 'sk-ant-oat01 (subscription OAuth)', expires_at: 0, has_token: true },
  { ...ACCOUNTS[2]!, ready: true, detail: 'sk-ant-oat01 (subscription OAuth)', expires_at: 0, has_token: false },
];

export const SETUP: Setup = {
  service: { registered: true, supported: true, platform: 'macos' },
  url: 'http://127.0.0.1:8787',
  global_settings_path: '/Users/sample/.claude/settings.json',
  managed: [
    { path: '/Users/sample/.claude/settings.json', scope: 'global', value: 'http://127.0.0.1:8787', previous: null, applied_at: AT - 30 * DAY },
    {
      path: '/Users/sample/code/a-directory-name-long-enough-to-wrap-in-a-phone/.claude/settings.local.json',
      scope: 'project',
      value: 'http://127.0.0.1:8787',
      previous: null,
      applied_at: AT - 2 * DAY,
    },
  ],
};

export const SHIM: Shim = {
  available: true,
  installed: 1,
  targets: [
    {
      extension: 'anthropic.claude-code-2.0.77-darwin-arm64',
      path: '/Users/sample/.vscode/extensions/anthropic.claude-code-2.0.77-darwin-arm64/resources/native-binary/claude',
      state: 'installed',
    },
    {
      extension: 'anthropic.claude-code-2.0.78-darwin-arm64',
      path: '/Users/sample/.vscode/extensions/anthropic.claude-code-2.0.78-darwin-arm64/resources/native-binary/claude',
      state: 'not_installed',
    },
  ],
  auto: true,
  auto_log: [{ at: AT - 3_600_000, what: 'wrapped anthropic.claude-code-2.0.77-darwin-arm64' }],
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
    { token_tail: 'a1b2c3', platform: 'apns', name: 'Sample’s iPhone', environment: 'production', router_ref: null, registered_at: AT - 9 * DAY, last_seen_at: AT - 3_600_000, last_result: 'delivered' },
    { token_tail: 'd4e5f6', platform: 'fcm', name: null, environment: 'production', router_ref: null, registered_at: AT - DAY, last_seen_at: null, last_result: null },
  ],
  last_error: null,
  last_sent: { at: AT - 3_600_000, what: 'agent-router: waiting on you' },
};

/// The catalogue (ST-C122): the Claude models the request log has seen, one of them given an account
/// of its own, each served by the account that would take its next request.
export const MODELS: readonly ModelRow[] = [
  { id: 'claude-haiku-4-5', provider: 'anthropic', display_name: 'claude-haiku-4-5', assigned: 'work', serving: 'work' },
  { id: 'claude-opus-5-5', provider: 'anthropic', display_name: 'claude-opus-5-5', assigned: null, serving: 'default' },
  { id: 'claude-sonnet-5-5', provider: 'anthropic', display_name: 'claude-sonnet-5-5', assigned: null, serving: 'default' },
];

/// No sign-in has run: what the router answers before the first one (ST-C46).
export const LOGIN: LoginStatus = { phase: 'failed', url: null, label: '', account_id: null, email: null, error: null };

const json = (payload: unknown): Response => new Response(JSON.stringify(payload), { status: 200, headers: { 'Content-Type': 'application/json' } });

/// The answer to one of Settings' reads, or null when it is not one of them, or asks for a query
/// parameter the harness cannot honour: that read then goes unanswered, loudly.
export function settingsRead(path: string, query: URLSearchParams): Response | null {
  const params = [...query.keys()];
  const only = (...allowed: string[]) => params.every((p) => allowed.includes(p));
  if (path === '/api/auth' && only()) return json(AUTH);
  if (path === '/api/accounts' && only('probe')) return json(query.get('probe') === '1' ? PROBED : ACCOUNTS);
  if (path === '/api/setup' && only()) return json(SETUP);
  if (path === '/api/setup/shim' && only()) return json(SHIM);
  if (path === '/api/push' && only()) return json(PUSH);
  if (path === '/api/login/status' && only()) return json(LOGIN);
  if (path === '/api/models' && only()) return json(MODELS);
  return null;
}
