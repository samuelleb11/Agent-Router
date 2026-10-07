// Remote mode, without the editor: whether `agentRouter.server` makes this a remote
// machine, the server's address in one canonical form, the password this machine holds
// for it, what the server answers, the agent's command lines, and what to take over from
// a set-up the agent made without the editor. Nothing here imports `vscode`, so all of
// it runs under plain Node (see src/test).
//
// Remote mode means a hosted router (`agent-router serve --server`, see
// docs/hosting-on-kubernetes.md) does the proxying and keeps what it records, and this
// machine runs only the agent (docs/hosting-machine-agent.md), which dials out to it.

/** Where the router this machine uses lives. `invalid` is a server setting that is not
 *  a usable address: neither a local router nor the agent is started for it, because
 *  either would be a guess at what the person meant. */
export type Mode =
  | { kind: 'local' }
  | { kind: 'remote'; server: string }
  | { kind: 'invalid'; value: string; why: string };

export type Normalised = { ok: true; url: string; note?: string } | { ok: false; why: string };

/** An address as typed, reduced to the router's origin: scheme, host and a port that is
 *  not the default one, with no trailing slash. A bare host is taken as https. Empty
 *  input is the empty string, which means local mode.
 *
 *  A path is refused rather than kept: the router's pages fetch `/api/...` from the
 *  root, so a router served under a prefix would not work anyway. Credentials in the
 *  address are refused so the password can never end up in settings. */
export function normaliseServerUrl(input: string): Normalised {
  const typed = input.trim();
  if (!typed) { return { ok: true, url: '' }; }
  // A scheme must be followed by `//` to count: `router.example.com:443` parses as a URL
  // whose scheme is `router.example.com:`, which is not what anyone meant.
  const withScheme = /^[a-z][a-z0-9+.-]*:\/\//i.test(typed) ? typed : `https://${typed}`;
  let u: URL;
  try {
    u = new URL(withScheme);
  } catch {
    return { ok: false, why: `"${clip(typed, 80)}" is not an address` };
  }
  if (u.protocol !== 'https:' && u.protocol !== 'http:') {
    return { ok: false, why: 'The address must start with https:// (or http:// on a network you trust).' };
  }
  if (u.username || u.password) {
    return { ok: false, why: 'Leave the password out of the address: it is asked for separately and kept in VS Code\'s secret storage.' };
  }
  if (!u.hostname) { return { ok: false, why: 'The address has no host.' }; }
  if (u.pathname !== '/') {
    return { ok: false, why: 'Give the router\'s address without a path: its pages are served from the root.' };
  }
  if (u.search || u.hash) {
    return { ok: false, why: 'Give the router\'s address without a query or a fragment.' };
  }
  return u.protocol === 'http:'
    ? { ok: true, url: u.origin, note: 'plain http sends the password unencrypted; use it only on a network you trust' }
    : { ok: true, url: u.origin };
}

/** The mode a value of `agentRouter.server` selects. Empty, absent or blank: local. */
export function decideMode(setting: unknown): Mode {
  if (setting === undefined || setting === null) { return { kind: 'local' }; }
  if (typeof setting !== 'string') {
    return { kind: 'invalid', value: clip(String(setting), 80), why: 'agentRouter.server must be text: the router\'s address.' };
  }
  const n = normaliseServerUrl(setting);
  if (!n.ok) { return { kind: 'invalid', value: clip(setting.trim(), 80), why: n.why }; }
  return n.url ? { kind: 'remote', server: n.url } : { kind: 'local' };
}

/** What to take over from the agent's own file (`agent.json`, which `agent install`
 *  writes) on a machine the server's install script set up, where VS Code was never told
 *  of the server. Only from local mode: a setting that names a server, or one that is not
 *  usable, is the person's, and the file does not outvote it. Undefined for anything but
 *  a usable address with a password beside it; after `agent uninstall` the file keeps
 *  only the machine's id. The password is given back as the agent keeps it, since it is
 *  the one the agent connects with. */
export function adoptable(agentJson: string | undefined, setting: unknown): { server: string; password: string } | undefined {
  if (decideMode(setting).kind !== 'local' || !agentJson) { return undefined; }
  let v: unknown;
  try { v = JSON.parse(agentJson); } catch { return undefined; }
  if (typeof v !== 'object' || v === null) { return undefined; }
  const { server, password } = v as { server?: unknown; password?: unknown };
  if (typeof server !== 'string' || typeof password !== 'string' || !password.trim()) { return undefined; }
  const n = normaliseServerUrl(server);
  return n.ok && n.url ? { server: n.url, password } : undefined;
}

/** The host (and port, when it is not the default) to name a server by in messages. */
export function hostOf(server: string): string {
  try { return new URL(server).host || server; } catch { return server; }
}

/** The router as it runs on this machine: the hosted kind runs on Windows only as the
 *  agent, so local mode is not offered there (docs/hosting-machine-agent.md, Windows). */
export const localModeAvailable = (platform: NodeJS.Platform = process.platform): boolean => platform !== 'win32';

// ---------------------------------------------------------------------------------------
// The password, in the editor's secret storage
// ---------------------------------------------------------------------------------------

export const PASSWORD_KEY = 'agentRouter.password';

/** The part of `vscode.SecretStorage` used here, so tests can hand in a map. */
export interface Secrets {
  get(key: string): PromiseLike<string | undefined>;
  store(key: string, value: string): PromiseLike<void>;
  delete(key: string): PromiseLike<void>;
}

/** Stored with the server it was checked against. A password is only ever sent to that
 *  server: if `agentRouter.server` is later edited to name another host, this machine
 *  has no password for it, rather than handing the old one to whoever answers there. */
export function encodeStored(server: string, password: string): string {
  return JSON.stringify({ server, password });
}

export function decodeStored(raw: string | undefined, server: string): string | undefined {
  if (!raw) { return undefined; }
  try {
    const v = JSON.parse(raw) as { server?: unknown; password?: unknown };
    if (v && v.server === server && typeof v.password === 'string' && v.password) { return v.password; }
  } catch { /* not ours */ }
  return undefined;
}

export async function readPassword(secrets: Secrets, server: string): Promise<string | undefined> {
  return decodeStored(await secrets.get(PASSWORD_KEY), server);
}

export async function savePassword(secrets: Secrets, server: string, password: string): Promise<void> {
  await secrets.store(PASSWORD_KEY, encodeStored(server, password));
}

export async function forgetPassword(secrets: Secrets): Promise<void> {
  await secrets.delete(PASSWORD_KEY);
}

// ---------------------------------------------------------------------------------------
// Talking to the server
// ---------------------------------------------------------------------------------------

/** How long a request to a server across the internet may take: the router derives
 *  the password once per session (about 300 ms), and a wrong one waits 400 ms more. */
export const WAN_TIMEOUT_MS = 10_000;
/** The connect check: a person is waiting on it, but a cold ingress can be slow. */
export const CHECK_TIMEOUT_MS = 15_000;

/** The router takes the password as the Basic password and ignores the user name. */
export function basicAuth(password: string): string {
  return `Basic ${Buffer.from(`agent-router:${password}`, 'utf8').toString('base64')}`;
}

/** What a request to the server came to. 401 and 403 are one case, `rejected`: the
 *  router answers 401 to a missing or wrong password and 403 to a caller it refuses
 *  outright, and either way the fix starts with the connect command. */
export type Answer<T> =
  | { kind: 'ok'; status: number; body: T }
  | { kind: 'rejected'; status: number; detail: string }
  | { kind: 'unreachable'; detail: string }
  | { kind: 'unexpected'; status: number; detail: string };

export interface QuotaWindow { utilization?: number | null; resets_at?: string | null }
export interface Quota { five_hour?: QuotaWindow; seven_day?: QuotaWindow; severity?: string; error?: string }
export interface Health { ok?: boolean; pid?: number; version?: string; build?: string }
/** `GET /api/auth`, the fields read here. `mode` is `server` on a hosted router. */
export interface AuthStatus { configured?: boolean; mode?: string; password_pinned?: boolean; public_url?: string | null }

/** GET a JSON document. Redirects are not followed: the password must reach the server
 *  it was given for and no other, and a redirect usually means the address is wrong
 *  (http where the ingress wants https), which is better said than silently fixed. */
export async function getJson<T>(url: string, opts: { password?: string; timeoutMs: number }): Promise<Answer<T>> {
  let r: Response;
  try {
    r = await fetch(url, {
      headers: { accept: 'application/json', ...(opts.password !== undefined ? { authorization: basicAuth(opts.password) } : {}) },
      redirect: 'manual',
      signal: AbortSignal.timeout(opts.timeoutMs),
    });
  } catch (e) {
    return { kind: 'unreachable', detail: describeFetchError(e, opts.timeoutMs) };
  }
  if (r.status === 401 || r.status === 403) {
    return { kind: 'rejected', status: r.status, detail: await sentence(r) };
  }
  if (r.type === 'opaqueredirect') {
    // What a browser-faithful fetch gives for a redirect it was told not to follow.
    return { kind: 'unexpected', status: r.status, detail: 'it redirects elsewhere' };
  }
  if (!r.ok) {
    const to = r.status >= 300 && r.status < 400 ? r.headers.get('location') : null;
    const said = await sentence(r);
    return {
      kind: 'unexpected',
      status: r.status,
      detail: to ? `it redirects to ${clip(to, 120)}` : `${r.status} ${r.statusText}${said ? `: ${said}` : ''}`.trim(),
    };
  }
  try {
    return { kind: 'ok', status: r.status, body: (await r.json()) as T };
  } catch {
    return { kind: 'unexpected', status: r.status, detail: 'the answer was not JSON' };
  }
}

/** The password check behind the connect command. A 200 that is not the router's own
 *  answer (a captive portal, a different service) is not taken as a yes. */
export async function checkPassword(server: string, password: string, timeoutMs = CHECK_TIMEOUT_MS): Promise<Answer<AuthStatus>> {
  const a = await getJson<AuthStatus>(`${server}/api/auth`, { password, timeoutMs });
  if (a.kind === 'ok' && (typeof a.body !== 'object' || a.body === null || !('configured' in a.body))) {
    return { kind: 'unexpected', status: a.status, detail: 'it answered, but not as an Agent Router' };
  }
  return a;
}

export const fetchQuota = (server: string, password: string, timeoutMs = WAN_TIMEOUT_MS): Promise<Answer<Quota>> =>
  getJson<Quota>(`${server}/api/quota`, { password, timeoutMs });

/** Health is open on every router, hosted ones included, so it is asked without the
 *  password. */
export const fetchHealth = (server: string, timeoutMs = WAN_TIMEOUT_MS): Promise<Answer<Health>> =>
  getJson<Health>(`${server}/_router/health`, { timeoutMs });

/** Why a request got no HTTP answer at all, in words. */
export function describeFetchError(e: unknown, timeoutMs: number): string {
  const err = (e ?? {}) as { name?: string; message?: string; cause?: { code?: string; message?: string } };
  if (err.name === 'TimeoutError' || err.name === 'AbortError') {
    return `no answer within ${timeoutMs >= 1000 ? `${Math.round(timeoutMs / 1000)} s` : `${timeoutMs} ms`}`;
  }
  const code = err.cause?.code ?? '';
  switch (code) {
    case 'ENOTFOUND': case 'EAI_AGAIN': return 'its name does not resolve';
    case 'ECONNREFUSED': return 'the connection was refused';
    case 'ECONNRESET': return 'the connection was reset';
    case 'ETIMEDOUT': case 'UND_ERR_CONNECT_TIMEOUT': return 'the connection timed out';
    case 'EHOSTUNREACH': case 'ENETUNREACH': return 'the network cannot reach it';
  }
  if (/CERT|SIGNATURE|SELF_SIGNED|ALTNAME/.test(code)) { return `its TLS certificate was not accepted (${code})`; }
  return err.cause?.message || err.message || String(e);
}

/** The server's own sentence from an error answer: a JSON `error`, or the first line of
 *  a text body, without the `agent-router:` prefix the guard puts on its refusals. */
async function sentence(r: Response): Promise<string> {
  let text = '';
  try { text = await r.text(); } catch { return ''; }
  text = text.trim();
  if (text.startsWith('{')) {
    try {
      const v = JSON.parse(text) as { error?: unknown };
      if (typeof v.error === 'string') { return clip(v.error, 200); }
    } catch { /* not JSON after all */ }
  }
  const line = text.split(/\r?\n/, 1)[0] ?? '';
  return clip(line.replace(/^agent-router:\s*/, ''), 200);
}

function clip(s: string, n: number): string {
  return s.length > n ? `${s.slice(0, n - 1)}…` : s;
}

// ---------------------------------------------------------------------------------------
// The agent's command line (docs/hosting-machine-agent.md, "The agent")
// ---------------------------------------------------------------------------------------

/** The child that installs the agent gets the password in this variable, and no other
 *  process is ever given it: never an argument, so it is not in any process listing. */
export const PASSWORD_ENV = 'AGENT_ROUTER_PASSWORD';

/** The bundled programs are `.exe` files on Windows. */
export function exeName(name: string, platform: NodeJS.Platform = process.platform): string {
  return platform === 'win32' ? `${name}.exe` : name;
}

/** `agent-router agent install --server <url> [--no-service] [--no-config]`. */
export function agentInstallArgs(server: string, o: { service: boolean; config: boolean }): string[] {
  return ['agent', 'install', '--server', server, ...(o.service ? [] : ['--no-service']), ...(o.config ? [] : ['--no-config'])];
}

export const AGENT_ENSURE_ARGS: readonly string[] = ['agent', '--ensure'];
export const AGENT_UNINSTALL_ARGS: readonly string[] = ['agent', 'uninstall'];

/** The environment for an agent child. A password inherited from whatever started the
 *  editor is dropped (on Windows the name is matched without regard to case, as the
 *  system does), and only `password`, when given, is put back. HOME is pinned to the
 *  editor's idea of it, as the local router's children always were; not on Windows,
 *  where the agent resolves its profile itself, the same way its logon task will. */
export function agentEnv(base: NodeJS.ProcessEnv, homedir: string, platform: NodeJS.Platform, password?: string): NodeJS.ProcessEnv {
  const env: NodeJS.ProcessEnv = { ...base };
  for (const k of Object.keys(env)) {
    if (k === PASSWORD_ENV || (platform === 'win32' && k.toUpperCase() === PASSWORD_ENV)) { delete env[k]; }
  }
  if (platform !== 'win32') { env.HOME = homedir; }
  if (password !== undefined) { env[PASSWORD_ENV] = password; }
  return env;
}

/** The agent's error is one sentence on stderr. An older build that does not know the
 *  subcommand prints its sentence and then its whole usage text, so only the first
 *  paragraph is taken, on one line. */
export function firstParagraph(text: string): string {
  const para = text.replace(/\r\n/g, '\n').trim().split(/\n\s*\n/, 1)[0] ?? '';
  return para.replace(/\s+/g, ' ').trim();
}

/** The sentence to show from what a failed agent command wrote. A Rust `main` that
 *  returns an error prints anyhow's report, `Error: <what>` and then a `Caused by:`
 *  list; that is folded onto one line the way anyhow's own `{:#}` would, so the cause
 *  ("connection refused") is not lost behind the context ("reaching <server>"). */
export function agentSentence(stderr: string): string {
  const text = stderr.replace(/\r\n/g, '\n').trim();
  if (!text.startsWith('Error: ')) { return firstParagraph(text); }
  const [head = '', ...rest] = text.slice('Error: '.length).split(/\n\s*\n/);
  const parts = [head.replace(/\s+/g, ' ').trim()];
  const causes = rest.find((p) => p.startsWith('Caused by:'));
  for (const line of causes ? causes.split('\n').slice(1) : []) {
    const cause = line.trim().replace(/^\d+:\s*/, '');
    if (cause) { parts.push(cause); }
  }
  return parts.filter(Boolean).join(': ');
}

/** Why a router that took the password still cannot be this machine's server: the
 *  agent connects only to a hosted one (`serve --server`), and refuses the rest at
 *  install. Undefined when it is one. */
export function notHosted(auth: AuthStatus, host: string): string | undefined {
  if (auth.mode === 'server') { return undefined; }
  return typeof auth.mode === 'string'
    ? `${host} is not a hosted router: it runs in ${auth.mode} mode, on its own machine. Remote mode needs a router started with "agent-router serve --server".`
    : `${host} runs an Agent Router too old for remote mode.`;
}

/** Belt and braces before anything reaches the log: no output the agent writes should
 *  carry the password, and if one ever does, it is masked here. */
export function redact(text: string, secret: string | undefined): string {
  return secret ? text.split(secret).join('••••••') : text;
}
