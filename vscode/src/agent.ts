// The agent on this machine, in remote mode: the same binary as the local router, run as
// `agent-router agent`, which dials out to the hosted router so the phone can follow and
// steer this machine's sessions (docs/hosting-machine-agent.md). It listens on nothing,
// so unlike the local router there is no health to ask: `agent --ensure` is told to
// start, and decides for itself whether one from this build is already running.
//
// The contract this file is built against:
//   agent install --server <url> [--no-service] [--no-config]
//       the password in AGENT_ROUTER_PASSWORD, never an argument; exit 0 means done,
//       otherwise stderr carries one sentence to show
//   agent --ensure      start detached unless one from this build runs; reads the
//                       configuration install saved
//   agent uninstall     undo install

import * as vscode from 'vscode';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import { spawn, execFile } from 'child_process';
import * as router from './router';
import * as remote from './remote';

/** The server this machine's agent was set up for, by this extension. */
const SET_UP_FOR = 'agentSetUpFor';
/** The server the person said "Not now" to, so the offer is not repeated in every window. */
const DECLINED_FOR = 'agentSetUpDeclined';

export const isSetUpFor = (context: vscode.ExtensionContext, server: string): boolean =>
  context.globalState.get<string>(SET_UP_FOR) === server;
export const setUpFor = (context: vscode.ExtensionContext): string | undefined =>
  context.globalState.get<string>(SET_UP_FOR);
export const declinedFor = (context: vscode.ExtensionContext, server: string): boolean =>
  context.globalState.get<string>(DECLINED_FOR) === server;

export async function recordSetUp(context: vscode.ExtensionContext, server: string | undefined): Promise<void> {
  await context.globalState.update(SET_UP_FOR, server);
  await context.globalState.update(DECLINED_FOR, undefined);
}

export async function recordDeclined(context: vscode.ExtensionContext, server: string | undefined): Promise<void> {
  await context.globalState.update(DECLINED_FOR, server);
}

const env = (password?: string): NodeJS.ProcessEnv => remote.agentEnv(process.env, os.homedir(), process.platform, password);

export interface Ran { ok: boolean; sentence: string }

/** Run one agent subcommand to completion. Its output goes to the log, masked for the
 *  password; on failure the result carries the one sentence to show. */
function runAgent(args: readonly string[], log: vscode.OutputChannel, timeoutMs: number, password?: string): Promise<Ran> {
  const exe = router.routerPath();
  log.appendLine(`> ${path.basename(exe)} ${args.join(' ')}`);
  return new Promise((resolve) => {
    const child = execFile(exe, [...args], { env: env(password), timeout: timeoutMs, windowsHide: true }, (err, stdout, stderr) => {
      const out = remote.redact(String(stdout ?? ''), password).trimEnd();
      const said = remote.redact(String(stderr ?? ''), password).trimEnd();
      if (out) { log.appendLine(out); }
      if (said) { log.appendLine(said); }
      if (!err) { resolve({ ok: true, sentence: '' }); return; }
      const e = err as NodeJS.ErrnoException & { killed?: boolean; signal?: string | null };
      const sentence =
        e.code === 'ENOENT' ? `there is no agent-router at ${exe}`
        : e.killed ? `agent-router ${args.slice(0, 2).join(' ')} did not finish within ${Math.round(timeoutMs / 1000)} s`
        : remote.agentSentence(said) || remote.redact(e.message, password);
      log.appendLine(`failed (${e.code ?? e.signal ?? 'error'}): ${sentence}`);
      resolve({ ok: false, sentence });
    });
    // The agent reads a password from standard input when the variable is empty; an
    // open pipe would hold it there until the timeout, where end-of-input fails at once.
    child.stdin?.end();
  });
}

/** `agent install`: checks the password with the server, saves it in the agent's own
 *  owner-only file, points Claude Code at the server and, with `service`, registers
 *  the login item. A minute, because it crosses the network and may register a task. */
export function install(server: string, password: string, o: { service: boolean; config: boolean }, log: vscode.OutputChannel): Promise<Ran> {
  return runAgent(remote.agentInstallArgs(server, o), log, 60_000, password);
}

export function uninstall(log: vscode.OutputChannel): Promise<Ran> {
  return runAgent(remote.AGENT_UNINSTALL_ARGS, log, 60_000);
}

/** Where the agent's own output goes: the file its login item writes too, since the
 *  agent keeps no log of its own and an agent this window starts would otherwise say
 *  nothing anywhere. */
export const logPath = (): string => path.join(router.dataDir(), 'logs', 'agent.log');

/** The file `agent install` saves, owner-only: the server, its password and this
 *  machine's id. Read here only to take over a set-up made without the editor. */
export const savedPath = (): string => path.join(router.dataDir(), 'agent.json');

/** Its text, or undefined: on most machines there is none, and one that cannot be read
 *  is as good as none. */
export function readSaved(): string | undefined {
  try { return fs.readFileSync(savedPath(), 'utf8'); } catch { return undefined; }
}

function openLog(): number | 'ignore' {
  try {
    fs.mkdirSync(path.dirname(logPath()), { recursive: true });
    return fs.openSync(logPath(), 'a');
  } catch {
    return 'ignore';
  }
}

/** `agent --ensure`, detached so it outlives this window, and without a console window
 *  on Windows. It exits at once when an agent is already running; there is nothing to
 *  wait for, since the agent has no address to ask. */
export function start(log: vscode.OutputChannel): void {
  const out = openLog();
  try {
    const child = spawn(router.routerPath(), [...remote.AGENT_ENSURE_ARGS], {
      detached: true,
      stdio: ['ignore', out, out],
      windowsHide: true,
      env: env(),
    });
    child.on('error', (e) => log.appendLine(`could not start the agent: ${e.message}`));
    child.unref();
    log.appendLine(`agent: asked to start (agent-router agent --ensure); its log is ${logPath()}`);
  } catch (e) {
    log.appendLine(`could not start the agent: ${(e as Error).message}`);
  } finally {
    // The child has its own handle on the file now.
    if (typeof out === 'number') { try { fs.closeSync(out); } catch { /* already closed */ } }
  }
}
