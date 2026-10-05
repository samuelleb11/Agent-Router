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
import { decideMode, exeName, Mode } from './remote';

export interface Health { ok: boolean; pid: number; version: string }
export interface Bundled { version: string; router: string; wrapper: string }

const cfg = () => vscode.workspace.getConfiguration('agentRouter');
/** Local, or remote when `agentRouter.server` names a hosted router (see remote.ts).
 *  Everything below that reads a port or a bind is local mode only. */
export const mode = (): Mode => decideMode(cfg().get<unknown>('server'));
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
/** `~/.agent-router`; on Windows `os.homedir()` is the user's profile (%USERPROFILE%). */
export const dataDir = (): string => path.join(os.homedir(), '.agent-router');
export const binDir = (): string => path.join(dataDir(), 'bin');
export const routerPath = (): string => path.join(binDir(), exeName('agent-router'));
export const wrapperPath = (): string => path.join(binDir(), exeName('claude-wrapper'));

/** The binaries this build of the extension carries, and their version. */
export function bundled(context: vscode.ExtensionContext): Bundled {
  const dir = path.join(context.extensionPath, 'bin');
  let version = '';
  try { version = fs.readFileSync(path.join(dir, 'VERSION'), 'utf8').trim(); } catch { /* an unpackaged checkout */ }
  return { version, router: path.join(dir, exeName('agent-router')), wrapper: path.join(dir, exeName('claude-wrapper')) };
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
 *  overwritten in place — macOS kills the running process with a code-signature error.
 *  The copy is named for this process, so two windows installing at once never write
 *  into one file. */
function installOne(from: string, to: string): void {
  fs.mkdirSync(path.dirname(to), { recursive: true });
  const fresh = `${to}.new-${process.pid}`;
  try {
    fs.copyFileSync(from, fresh);
    fs.chmodSync(fresh, 0o755);
    replace(fresh, to);
  } catch (e) {
    try { fs.unlinkSync(fresh); } catch { /* already renamed, or never made */ }
    throw e;
  }
}

/** Windows will not let a running program's file be replaced, but it will let it be
 *  renamed: the running copy is moved aside and removed by a later install, once
 *  nothing runs it. Elsewhere this is the plain rename it always was. */
function replace(fresh: string, to: string): void {
  try {
    fs.renameSync(fresh, to);
  } catch (e) {
    if (process.platform !== 'win32') { throw e; }
    const aside = `${to}.old-${process.pid}-${Date.now()}`;
    let moved = false;
    try { fs.renameSync(to, aside); moved = true; } catch { /* another window moved it already */ }
    try {
      fs.renameSync(fresh, to);
    } catch (again) {
      // Never leave the place empty: put back the copy that was there.
      if (moved) { try { fs.renameSync(aside, to); } catch { /* the next install fills it */ } }
      throw again;
    }
  }
}

/** The copies `replace` moved aside on Windows, removed when nothing runs them any more. */
function sweepAside(): void {
  if (process.platform !== 'win32') { return; }
  let names: string[] = [];
  try { names = fs.readdirSync(binDir()); } catch { return; }
  for (const n of names) {
    if (/\.old-\d+-\d+$/.test(n)) {
      try { fs.unlinkSync(path.join(binDir(), n)); } catch { /* still running */ }
    }
  }
}

export function installBinaries(b: Bundled): void {
  if (!fs.existsSync(b.router)) { throw new Error(`this build of the extension carries no router binary (${b.router})`); }
  installOne(b.router, routerPath());
  if (fs.existsSync(b.wrapper)) { installOne(b.wrapper, wrapperPath()); }
}

function sameBytes(a: string, b: string): boolean {
  try {
    if (fs.statSync(a).size !== fs.statSync(b).size) { return false; }
    return fs.readFileSync(a).equals(fs.readFileSync(b));
  } catch {
    return false;
  }
}

/** Remote mode's install. There is no local router to ask for its version, so the
 *  installed files are compared with the bundled ones and replaced only when they
 *  differ: a running agent or wrapper is never touched for nothing, which on Windows
 *  is what keeps the moved-aside copies from piling up. True when anything changed. */
export function installBinariesIfChanged(b: Bundled): boolean {
  if (!fs.existsSync(b.router)) { throw new Error(`this build of the extension carries no router binary (${b.router})`); }
  sweepAside();
  let changed = false;
  if (!sameBytes(b.router, routerPath())) { installOne(b.router, routerPath()); changed = true; }
  if (fs.existsSync(b.wrapper) && !sameBytes(b.wrapper, wrapperPath())) { installOne(b.wrapper, wrapperPath()); changed = true; }
  return changed;
}

function run(exe: string, args: string[], env: NodeJS.ProcessEnv): Promise<string> {
  return new Promise((resolve, reject) => {
    execFile(exe, args, { env, timeout: 30_000, windowsHide: true }, (err, stdout, stderr) => {
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
  const child = spawn(routerPath(), ['--ensure'], { detached: true, stdio: 'ignore', windowsHide: true, env: routerEnv() });
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
