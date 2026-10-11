// Remote mode, as the person drives it: connecting this machine to a hosted router,
// setting it up for one, undoing that, and what a window does when it opens, including
// taking over a set-up the server's install script made before VS Code knew of it. The
// local flows stay in extension.ts, untouched; every command there hands over to the
// matching one here when `agentRouter.server` is set.
//
// The password is held in three places on a set-up machine, each because something
// needs it: VS Code's secret storage (the status bar and the connect check), the agent's
// own owner-only file (its connection to the server), and Claude Code's settings file
// (ANTHROPIC_AUTH_TOKEN, written by `agent install`). The install script writes the last
// two, and the first is copied from the agent's file when a window takes the set-up over.
// It is never written to VS Code's settings, an argument or the log.

import * as vscode from 'vscode';
import * as fs from 'fs';
import * as router from './router';
import * as remote from './remote';
import * as agent from './agent';
import * as claude from './claude';

const settings = () => vscode.workspace.getConfiguration('agentRouter');

function sentence(s: string): string {
  const t = s.trim();
  return /[.!?…]$/.test(t) ? t : `${t}.`;
}

function showFailure(message: string, log: vscode.OutputChannel): void {
  void vscode.window.showErrorMessage(message, 'Show log').then((c) => c && log.show());
}

function progress<T>(title: string, task: () => Thenable<T>): Thenable<T> {
  return vscode.window.withProgress({ location: vscode.ProgressLocation.Notification, title }, task);
}

function validateAddress(value: string): string | vscode.InputBoxValidationMessage | undefined {
  const n = remote.normaliseServerUrl(value);
  if (!n.ok) { return n.why; }
  if (!n.url) { return undefined; }
  if (n.note) { return { message: `Connects to ${n.url}: ${n.note}.`, severity: vscode.InputBoxValidationSeverity.Warning }; }
  if (n.url !== value.trim()) { return { message: `Connects to ${n.url}`, severity: vscode.InputBoxValidationSeverity.Info }; }
  return undefined;
}

/** Shown wherever local mode cannot help: on Windows, and for a server setting that
 *  is not an address. */
export async function offerConnect(message: string): Promise<void> {
  const pick = await vscode.window.showInformationMessage(message, 'Connect…');
  if (pick) { await vscode.commands.executeCommand('agentRouter.connect'); }
}

export const WINDOWS_LOCAL =
  'On Windows, Agent Router runs on a server: this machine runs only the agent, which connects to it. Connect to a server to use it.';

type SetUp =
  | { kind: 'done'; service: boolean; config: boolean; note: string }
  | { kind: 'cancelled' }
  | { kind: 'failed'; sentence: string };

type Wired = 'wired' | 'no-wrapper' | 'no-claude-code' | 'failed';

/** Hand Claude Code the wrapper, for a set-up that configures Claude Code. */
async function wireWrapper(log: vscode.OutputChannel): Promise<Wired> {
  // The wrapper is what lets the agent type into a session. Only a path that exists
  // is ever handed to Claude Code: a wrapper setting naming nothing stops every
  // session from starting.
  if (!fs.existsSync(router.wrapperPath())) {
    log.appendLine(`no wrapper at ${router.wrapperPath()}; claudeCode.claudeProcessWrapper left alone`);
    return 'no-wrapper';
  }
  if (!claude.claudeCodeInstalled()) {
    log.appendLine('the Claude Code extension is not installed; claudeCode.claudeProcessWrapper left alone');
    return 'no-claude-code';
  }
  try {
    await claude.configureWrapper(log);
    return 'wired';
  } catch (e) {
    log.appendLine(`could not set claudeCode.claudeProcessWrapper: ${(e as Error).message}`);
    return 'failed';
  }
}

/** The one-time set-up for a server: `agent install`, with the login item unless the
 *  person declines it, then the wrapper, then the agent itself. */
async function setUpMachine(context: vscode.ExtensionContext, log: vscode.OutputChannel, server: string, password: string): Promise<SetUp> {
  const host = remote.hostOf(server);
  const choice = await vscode.window.showInformationMessage(
    'Start the Agent Router agent at login?',
    {
      modal: true,
      detail: `The agent connects this machine to ${host}, so the phone and the server's live page can follow and steer the Claude Code sessions that run here. Started at login, it runs before any editor window opens; otherwise each VS Code window starts it.`,
    },
    'Start at Login', 'Only with VS Code');
  if (!choice) { return { kind: 'cancelled' }; }
  const service = choice === 'Start at Login';
  const config = settings().get<boolean>('configureClaudeCode', true);
  try {
    router.installForRemote(router.bundled(context));
  } catch (e) {
    log.appendLine(`could not install the agent: ${(e as Error).message}`);
    return { kind: 'failed', sentence: (e as Error).message };
  }
  const r = await progress(`Setting this machine up for ${host}…`, () => agent.install(server, password, { service, config }, log));
  if (!r.ok) { return { kind: 'failed', sentence: r.sentence }; }
  const w = config ? await wireWrapper(log) : undefined;
  const note =
    w === 'no-wrapper' ? ' This build carries no wrapper, so sessions can be followed but not typed into.'
    : w === 'failed' ? ' The wrapper setting could not be written; see the log.'
    : '';
  await agent.recordSetUp(context, server);
  agent.start(log);
  return { kind: 'done', service, config, note };
}

function doneText(lead: string, host: string, s: { service: boolean; config: boolean; note: string }): string {
  const claudeCode = s.config ? `new Claude Code sessions go through ${host}` : 'Claude Code was left alone (agentRouter.configureClaudeCode is off)';
  const agentLine = s.service ? 'the agent starts at login' : 'each VS Code window starts the agent';
  return `${lead}: ${claudeCode}, and ${agentLine}, so the phone can follow this machine's sessions.${s.note}`;
}

/** Agent Router: Connect to a Server… — the address, the password, a check of both
 *  with the server, then the set-up. Also how a changed password is given. */
export async function connect(context: vscode.ExtensionContext, log: vscode.OutputChannel, initial?: string): Promise<void> {
  const m = router.mode();
  const typed = await vscode.window.showInputBox({
    title: 'Connect to an Agent Router server (1/2)',
    prompt: 'The hosted router\'s address. What it records (captured requests and responses, accounts) is kept on that server.',
    placeHolder: 'https://router.example.com',
    value: initial ?? (m.kind === 'remote' ? m.server : m.kind === 'invalid' ? m.value : ''),
    ignoreFocusOut: true,
    validateInput: validateAddress,
  });
  if (typed === undefined) { return; }
  const n = remote.normaliseServerUrl(typed);
  if (!n.ok || !n.url) { return; }
  const server = n.url;
  const host = remote.hostOf(server);

  let refused = false;
  let password = '';
  for (;;) {
    const entered = await vscode.window.showInputBox({
      title: `Connect to ${host} (2/2)`,
      prompt: refused
        ? `${host} refused that credential. Enter a valid Machine access token or router password.`
        : 'A Machine access token from Settings → Access tokens (recommended), or the router password. Kept in VS Code’s secret storage, never in settings.',
      password: true,
      ignoreFocusOut: true,
    });
    // Spaces around a pasted password are not part of it: Claude Code presents it as a
    // bearer token, which the router trims, so a password that needed them could never
    // work there anyway.
    const candidate = entered?.trim();
    if (!candidate) { return; }
    const answer = await progress(`Checking the router credential with ${host}…`, () => remote.checkPassword(server, candidate));
    if (answer.kind === 'ok') {
      // The agent refuses any router that is not hosted, so this one would connect and
      // then fail to set up. Said now, before anything is stored.
      const problem = remote.notHosted(answer.body, host);
      if (problem) {
        log.appendLine(`connect: ${problem}`);
        void vscode.window.showErrorMessage(problem);
        return;
      }
      password = candidate;
      break;
    }
    if (answer.kind === 'rejected' && answer.status === 401) {
      log.appendLine(`connect: ${host} refused the credential (401)`);
      refused = true;
      continue;
    }
    const why =
      answer.kind === 'unreachable' ? `Could not reach ${host}: ${answer.detail}`
      : answer.kind === 'rejected' ? `${host} refused the connection (${answer.status})${answer.detail ? `: ${answer.detail}` : ''}`
      : `${host} did not answer as an Agent Router: ${answer.detail}`;
    log.appendLine(`connect: ${sentence(why)}`);
    const again = await vscode.window.showErrorMessage(sentence(why), 'Try Again');
    if (again) { await connect(context, log, server); }
    return;
  }

  // The secret first: the setting is what switches every window to remote mode, and
  // each of them reads the password as soon as it does.
  await remote.savePassword(context.secrets, server, password);
  await settings().update('server', server, vscode.ConfigurationTarget.Global);
  log.appendLine(`connected to ${server}`);

  const before = agent.setUpFor(context);
  const still = before && before !== server ? ` Until it succeeds, Claude Code may still go through ${remote.hostOf(before)}.` : '';
  const r = await setUpMachine(context, log, server, password);
  if (r.kind === 'done') {
    void vscode.window.showInformationMessage(doneText(`Connected to ${host}`, host, r));
  } else if (r.kind === 'cancelled') {
    void vscode.window.showInformationMessage(
      `Connected to ${host}, but this machine is not set up for it yet: run Agent Router: Set Up on This Machine when you are ready.${still}`);
  } else {
    showFailure(`Connected to ${host}, but setting this machine up failed: ${sentence(r.sentence)}${still}`, log);
  }
}

/** Agent Router: Disconnect from the Server — undo the set-up, forget the password, and
 *  go back to local mode. Nothing on the server is touched. */
export async function disconnect(context: vscode.ExtensionContext, log: vscode.OutputChannel): Promise<void> {
  const m = router.mode();
  if (m.kind === 'local') {
    void vscode.window.showInformationMessage('Agent Router is not connected to a server.');
    return;
  }
  const host = m.kind === 'remote' ? remote.hostOf(m.server) : m.value;
  const sure = await vscode.window.showWarningMessage(
    `Disconnect from ${host}?`,
    {
      modal: true,
      detail: 'Claude Code stops going through the server, the agent\'s login item is removed, and this machine forgets the credential. What the server keeps (captured requests and responses, accounts) stays on the server.',
    },
    'Disconnect');
  if (sure !== 'Disconnect') { return; }
  if (!fs.existsSync(router.routerPath())) {
    log.appendLine(`no agent-router at ${router.routerPath()}, so no set-up to undo`);
  } else {
    const r = await progress(`Undoing this machine's set-up for ${host}…`, () => agent.uninstall(log));
    if (!r.ok) {
      const anyway = await vscode.window.showWarningMessage(
        `This machine's set-up could not be undone: ${sentence(r.sentence)}`,
        {
          modal: true,
          detail: 'Disconnect anyway? Claude Code may still go through the server, and the agent may still start at login, until "agent-router agent uninstall" succeeds.',
        },
        'Disconnect Anyway');
      if (anyway !== 'Disconnect Anyway') { return; }
    }
  }
  await remote.forgetPassword(context.secrets);
  await agent.recordSetUp(context, undefined);
  await settings().update('server', undefined, vscode.ConfigurationTarget.Global);
  log.appendLine(`disconnected from ${host}`);
  void vscode.window.showInformationMessage(remote.localModeAvailable()
    ? `Disconnected from ${host}. This machine forgot the credential, and Agent Router is back in local mode.`
    : `Disconnected from ${host}. This machine forgot the credential.`);
}

/** Agent Router: Set Up on This Machine, in remote mode. */
export async function setUp(context: vscode.ExtensionContext, log: vscode.OutputChannel): Promise<void> {
  const m = router.mode();
  if (m.kind !== 'remote') { await connect(context, log); return; }
  const password = await remote.readPassword(context.secrets, m.server);
  if (!password) { await connect(context, log, m.server); return; }
  const host = remote.hostOf(m.server);
  const r = await setUpMachine(context, log, m.server, password);
  if (r.kind === 'done') {
    void vscode.window.showInformationMessage(doneText(`Agent Router is set up for ${host}`, host, r));
  } else if (r.kind === 'failed') {
    showFailure(`Set-up failed: ${sentence(r.sentence)}`, log);
  }
}

/** Agent Router: Remove from This Machine, in remote mode: the set-up goes, the
 *  connection stays, so the status bar still reads the server. Disconnect forgets it. */
export async function remove(context: vscode.ExtensionContext, log: vscode.OutputChannel): Promise<void> {
  const m = router.mode();
  const host = m.kind === 'remote' ? remote.hostOf(m.server) : 'the server';
  const sure = await vscode.window.showWarningMessage(
    `Remove Agent Router from this machine? The agent's login item is unregistered, Claude Code stops going through ${host}, and the wrapper setting is cleared. The server and what it keeps are not touched.`,
    { modal: true }, 'Remove');
  if (sure !== 'Remove') { return; }
  try {
    await claude.unconfigureWrapper(log);
    const r = await progress('Removing the agent\'s set-up…', () => agent.uninstall(log));
    if (!r.ok) { throw new Error(r.sentence); }
    await agent.recordSetUp(context, undefined);
    if (m.kind === 'remote') { await agent.recordDeclined(context, m.server); }
    void vscode.window.showInformationMessage(
      `Agent Router was removed from this machine. The status bar still reads ${host}; Disconnect from the Server forgets it.`);
  } catch (e) {
    showFailure(`Removal failed: ${sentence((e as Error).message)}`, log);
  }
}

/** Agent Router: Start or Update the Router, should it be run in remote mode: install
 *  the bundled agent where none is, and ask the agent to start. Updating an installed
 *  agent is its keeper's (docs/hosting-machine-agent.md, "Upgrading an agent"). */
export async function ensure(context: vscode.ExtensionContext, log: vscode.OutputChannel): Promise<void> {
  const m = router.mode();
  if (m.kind !== 'remote') { await connect(context, log); return; }
  const host = remote.hostOf(m.server);
  try {
    router.installForRemote(router.bundled(context));
  } catch (e) {
    showFailure(`The agent could not be installed: ${sentence((e as Error).message)}`, log);
    return;
  }
  if (!agent.isSetUpFor(context, m.server)) {
    const pick = await vscode.window.showInformationMessage(`This machine is not set up for ${host} yet.`, 'Set Up');
    if (pick) { await vscode.commands.executeCommand('agentRouter.setUp'); }
    return;
  }
  agent.start(log);
  void vscode.window.showInformationMessage(`The agent was asked to start. It runs in the background, connected to ${host}.`);
}

/** What a window does first when it opens in local mode, on a machine the server's
 *  install script set up: `agent install` has run, so the agent's own file holds the
 *  server and its password, but VS Code was never told. That set-up is taken over as it
 *  is, never redone: the password goes into secret storage, `agentRouter.server` is set,
 *  and Claude Code is handed the wrapper. The server it switched to, or undefined when
 *  there was nothing to take over. The window then goes on in remote mode, where startUp
 *  finds the machine set up and starts the agent. */
export async function adopt(context: vscode.ExtensionContext, log: vscode.OutputChannel): Promise<string | undefined> {
  const a = remote.adoptable(agent.readSaved(), settings().get<unknown>('server'));
  if (!a) { return undefined; }
  const host = remote.hostOf(a.server);
  // The secret and the record first: the setting is what switches every window to
  // remote mode, and one that opens then must find the password saved and the machine
  // set up, or it would offer a set-up that is already done.
  await remote.savePassword(context.secrets, a.server, a.password);
  await agent.recordSetUp(context, a.server);
  // A window that opened at the same moment may have switched first: it says so, and
  // this one only follows.
  if (router.mode().kind !== 'local') { return undefined; }
  await settings().update('server', a.server, vscode.ConfigurationTarget.Global);
  log.appendLine(`connected to ${a.server}, taken over from ${agent.savedPath()}`);
  try {
    const b = router.bundled(context);
    if (router.installForRemote(b)) { log.appendLine(`installed ${b.version || 'the bundled build'} into ${router.binDir()}`); }
  } catch (e) {
    log.appendLine(`could not install the agent: ${(e as Error).message}`);
  }
  const w = settings().get<boolean>('configureClaudeCode', true) ? await wireWrapper(log) : 'off';
  const why =
    w === 'off' ? 'agentRouter.configureClaudeCode is off, so the wrapper was left alone'
    : w === 'no-wrapper' ? 'this build carries no wrapper'
    : w === 'no-claude-code' ? 'the Claude Code extension is not installed'
    : w === 'failed' ? 'the wrapper setting could not be written; see the log'
    : '';
  const lead = `This machine's agent connects to ${host}, so Agent Router now works through that server`;
  void vscode.window.showInformationMessage(why
    ? `${lead}. New Claude Code sessions can be followed from the phone but not typed into: ${why}.`
    : `${lead}: new Claude Code sessions can be followed and typed into from the phone.`);
  return a.server;
}

/** What a window does when it opens in remote mode: install the bundled agent where none
 *  is, then start the agent, or offer the set-up once. */
export async function startUp(context: vscode.ExtensionContext, log: vscode.OutputChannel, server: string): Promise<void> {
  const host = remote.hostOf(server);
  log.appendLine(`remote mode: ${server}`);
  try {
    const b = router.bundled(context);
    if (router.installForRemote(b)) { log.appendLine(`installed ${b.version || 'the bundled build'} into ${router.binDir()}`); }
  } catch (e) {
    log.appendLine(`could not install the agent: ${(e as Error).message}`);
    return;
  }
  if (agent.isSetUpFor(context, server)) { agent.start(log); return; }
  const password = await remote.readPassword(context.secrets, server);
  if (!password) {
    log.appendLine(`no credential saved for ${host}: run Agent Router: Connect to a Server`);
    return;
  }
  if (agent.declinedFor(context, server)) {
    log.appendLine(`this machine is not set up for ${host}; Agent Router: Set Up on This Machine does it`);
    return;
  }
  const pick = await vscode.window.showInformationMessage(
    `Agent Router is connected to ${host}, but this machine is not set up for it yet. Set it up, so Claude Code goes through the server and the phone can follow this machine's sessions?`,
    'Set up', 'Not now');
  if (pick === 'Set up') {
    await vscode.commands.executeCommand('agentRouter.setUp');
  } else if (pick === 'Not now') {
    await agent.recordDeclined(context, server);
  }
}
