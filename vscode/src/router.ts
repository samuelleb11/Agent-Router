// The router on this machine: where it lives, whether it answers, and how this
// extension makes sure it is running and current. The extension is never the process
// that owns the router — every window would then own one. The daemon is alone on its
// port by construction: `--ensure` exits when a same-or-newer build already answers,
// and takes the port over without a gap when an older one does.

import * as vscode from 'vscode';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import { spawn, execFile } from 'child_process';

export interface Health { ok: boolean; pid: number; version: string }
export interface Bundled { version: string; router: string; wrapper: string }

const cfg = () => vscode.workspace.getConfiguration('agentRouter');
export const port = (): number => cfg().get<number>('port', 8787);
/** Where to listen, but only when the user has actually said.
 *
 *  The router stores this itself now, and the dashboard's switch writes it. An
 *  untouched default here must not outvote that: `AGENT_ROUTER_BIND` wins over the
 *  stored setting, so passing one on every start would silently undo the switch the
 *  next time a window opened. Undefined means "whatever the router already decided". */
export const bind = (): string | undefined => {
  const set = cfg().inspect<boolean>('listenOnNetwork');
  const chosen = set?.workspaceFolderValue ?? set?.workspaceValue ?? set?.globalValue;
  return chosen === undefined ? undefined : chosen ? '0.0.0.0' : '127.0.0.1';
};
export const base = (): string => `http://127.0.0.1:${port()}`;
export const dataDir = (): string => path.join(os.homedir(), '.agent-router');
export const binDir = (): string => path.join(dataDir(), 'bin');
export const routerPath = (): string => path.join(binDir(), 'agent-router');
export const wrapperPath = (): string => path.join(binDir(), 'claude-wrapper');

/** The binaries this build of the extension carries, and their version. */
export function bundled(context: vscode.ExtensionContext): Bundled {
  const dir = path.join(context.extensionPath, 'bin');
  let version = '';
  try { version = fs.readFileSync(path.join(dir, 'VERSION'), 'utf8').trim(); } catch { /* an unpackaged checkout */ }
  return { version, router: path.join(dir, 'agent-router'), wrapper: path.join(dir, 'claude-wrapper') };
}

export async function health(): Promise<Health | undefined> {
  try {
    const r = await fetch(`${base()}/_router/health`, { signal: AbortSignal.timeout(800) });
    if (!r.ok) { return undefined; }
    return (await r.json()) as Health;
  } catch {
    return undefined;
  }
}

/** Newer by dotted number; an unparseable or missing version counts as oldest. */
export function isNewer(ours: string, theirs: string): boolean {
  const parts = (v: string) => v.trim().split(/[.+-]/).slice(0, 3).map((p) => Number.parseInt(p, 10) || 0);
  const a = parts(ours), b = parts(theirs);
  for (let i = 0; i < 3; i++) {
    if ((a[i] ?? 0) !== (b[i] ?? 0)) { return (a[i] ?? 0) > (b[i] ?? 0); }
  }
  return false;
}

/** Copy beside, then rename over: a binary the kernel has mapped must never be
 *  overwritten in place — macOS kills the running process with a code-signature error. */
function installOne(from: string, to: string): void {
  fs.mkdirSync(path.dirname(to), { recursive: true });
  const fresh = `${to}.new`;
  fs.copyFileSync(from, fresh);
  fs.chmodSync(fresh, 0o755);
  fs.renameSync(fresh, to);
}

export function installBinaries(b: Bundled): void {
  if (!fs.existsSync(b.router)) { throw new Error(`this build of the extension carries no router binary (${b.router})`); }
  installOne(b.router, routerPath());
  if (fs.existsSync(b.wrapper)) { installOne(b.wrapper, wrapperPath()); }
}

function run(exe: string, args: string[], env: NodeJS.ProcessEnv): Promise<string> {
  return new Promise((resolve, reject) => {
    execFile(exe, args, { env, timeout: 30_000 }, (err, stdout, stderr) => {
      if (err) { reject(new Error(`${path.basename(exe)} ${args.join(' ')}: ${stderr || err.message}`)); }
      else { resolve(stdout); }
    });
  });
}

function routerEnv(): NodeJS.ProcessEnv {
  const b = bind();
  return { ...process.env, HOME: os.homedir(), AGENT_ROUTER_PORT: String(port()), ...(b ? { AGENT_ROUTER_BIND: b } : {}) };
}

/** Register the login item and point Claude Code's settings file at the router — the
 *  router's own `install` subcommand, which is also what undoes it. */
export async function setUp(log: vscode.OutputChannel): Promise<void> {
  const b = bind();
  const args = ['install', '--port', String(port()), ...(b ? ['--bind', b] : [])];
  if (!cfg().get<boolean>('manageService', true)) { args.push('--no-service'); }
  if (!cfg().get<boolean>('configureClaudeCode', true)) { args.push('--no-config'); }
  log.appendLine(`> agent-router ${args.join(' ')}`);
  log.appendLine(await run(routerPath(), args, routerEnv()));
}

export async function tearDown(log: vscode.OutputChannel): Promise<void> {
  log.appendLine('> agent-router uninstall');
  log.appendLine(await run(routerPath(), ['uninstall'], routerEnv()));
}

export type Ensured = 'running' | 'started' | 'upgraded' | 'failed';

/** Make sure a current router answers. Cheap when one does; otherwise install the
 *  bundled binaries into place and start one detached, so it outlives this window. */
export async function ensure(context: vscode.ExtensionContext, log: vscode.OutputChannel): Promise<Ensured> {
  const b = bundled(context);
  const before = await health();
  if (before && (!b.version || !isNewer(b.version, before.version))) {
    return 'running';
  }
  try {
    installBinaries(b);
  } catch (e) {
    log.appendLine(`could not install the router: ${(e as Error).message}`);
    return 'failed';
  }
  log.appendLine(before
    ? `router ${before.version} is older than ${b.version}; handing over`
    : `no router answering on ${base()}; starting ${b.version || 'the bundled build'}`);
  const child = spawn(routerPath(), ['--ensure'], { detached: true, stdio: 'ignore', env: routerEnv() });
  child.unref();
  for (let i = 0; i < 40; i++) {
    await new Promise((r) => setTimeout(r, 250));
    const now = await health();
    if (now && (!before || now.pid !== before.pid)) {
      log.appendLine(`router ${now.version} serving (pid ${now.pid})`);
      return before ? 'upgraded' : 'started';
    }
  }
  log.appendLine('the router did not answer within ten seconds; see ~/.agent-router/logs/router.log');
  return 'failed';
}
