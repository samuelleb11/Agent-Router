// Agent Router for VS Code. The router is a machine-wide daemon; this extension makes
// sure one is running and current, points Claude Code at it, and shows the quota. Every
// window activates its own copy of this code, which is why none of it owns anything:
// the daemon is alone on its port by construction (see router.ts).
//
// In remote mode (`agentRouter.server` set) no router runs here: a hosted one does the
// work, and this machine runs the agent instead (see connect.ts and agent.ts). Every
// command below first asks which mode it is in; the local branches are as they were.

import * as vscode from 'vscode';
import * as router from './router';
import * as claude from './claude';
import * as dashboard from './dashboard';
import * as remote from './remote';
import * as connect from './connect';
import { StatusBar } from './status';

export function activate(context: vscode.ExtensionContext): void {
  const log = vscode.window.createOutputChannel('Agent Router');
  const status = new StatusBar(context, log);
  context.subscriptions.push(log, status);

  // Which commands the palette offers: the local router's own only in local mode.
  const syncMode = () => void vscode.commands.executeCommand('setContext', 'agentRouter.remote', router.mode().kind !== 'local');
  syncMode();
  const localUnavailable = () => !remote.localModeAvailable();

  context.subscriptions.push(
    vscode.commands.registerCommand('agentRouter.openDashboard', () => dashboard.open('/', 'Agent Router')),
    vscode.commands.registerCommand('agentRouter.openLive', () => dashboard.open('/live', 'Live sessions')),
    vscode.commands.registerCommand('agentRouter.showLog', () => log.show()),
    vscode.commands.registerCommand('agentRouter.connect', () => connect.connect(context, log)),
    vscode.commands.registerCommand('agentRouter.disconnect', () => connect.disconnect(context, log)),
    // The status bar's "try again" when the server could not be reached.
    vscode.commands.registerCommand('agentRouter.refreshStatus', () => status.refresh()),
    vscode.commands.registerCommand('agentRouter.ensure', async () => {
      if (router.mode().kind !== 'local') { await connect.ensure(context, log); void status.refresh(); return; }
      if (localUnavailable()) { await connect.offerConnect(connect.WINDOWS_LOCAL); return; }
      const r = await router.ensure(context, log);
      void status.refresh();
      const said: Record<router.Ensured, string> = {
        running: 'The router is already running and current.',
        started: 'The router is running.',
        upgraded: 'The router was updated without dropping a request.',
        failed: 'The router could not be started. See the Agent Router log.',
      };
      void (r === 'failed' ? vscode.window.showErrorMessage(said[r], 'Show log').then((c) => c && log.show())
                          : vscode.window.showInformationMessage(said[r]));
    }),
    vscode.commands.registerCommand('agentRouter.setUp', async () => {
      if (router.mode().kind !== 'local') { await connect.setUp(context, log); void status.refresh(); return; }
      if (localUnavailable()) { await connect.offerConnect(connect.WINDOWS_LOCAL); return; }
      try {
        if ((await router.ensure(context, log)) === 'failed') { throw new Error('the router is not running'); }
        await router.setUp(log);
        if (vscode.workspace.getConfiguration('agentRouter').get<boolean>('configureClaudeCode', true)) {
          await claude.configureWrapper(log);
        }
        void vscode.window.showInformationMessage('Agent Router is set up: it starts at login, Claude Code goes through it, and new sessions can be followed from the phone.');
      } catch (e) {
        void vscode.window.showErrorMessage(`Set-up failed: ${(e as Error).message}`, 'Show log').then((c) => c && log.show());
      }
    }),
    vscode.commands.registerCommand('agentRouter.remove', async () => {
      if (router.mode().kind !== 'local') { await connect.remove(context, log); void status.refresh(); return; }
      if (localUnavailable()) { await connect.offerConnect(connect.WINDOWS_LOCAL); return; }
      const sure = await vscode.window.showWarningMessage(
        'Remove Agent Router from this machine? The login item is unregistered, Claude Code is pointed back at Anthropic, and the wrapper setting is cleared. Your data in ~/.agent-router is kept.',
        { modal: true }, 'Remove');
      if (sure !== 'Remove') { return; }
      try {
        await claude.unconfigureWrapper(log);
        await router.tearDown(log);
        void vscode.window.showInformationMessage('Agent Router was removed. The router keeps serving until it exits; your data is kept in ~/.agent-router.');
      } catch (e) {
        void vscode.window.showErrorMessage(`Removal failed: ${(e as Error).message}`, 'Show log').then((c) => c && log.show());
      }
    }),
    vscode.workspace.onDidChangeConfiguration((e) => {
      if (e.affectsConfiguration('agentRouter')) { syncMode(); void status.refresh(); }
    }),
    // Another window connected or disconnected: the password it saved or forgot is the
    // one this window's status bar reads.
    context.secrets.onDidChange((e) => {
      if (e.key === remote.PASSWORD_KEY) { void status.refresh(); }
    }),
  );

  status.start();

  // Not awaited: activation must not wait on a daemon. The status bar says how it went.
  void (async () => {
    const m = router.mode();
    if (m.kind === 'remote') {
      await connect.startUp(context, log, m.server);
      void status.refresh();
      return;
    }
    if (m.kind === 'invalid') {
      log.appendLine(`agentRouter.server is not usable (${m.why}); neither a router nor the agent was started`);
      return;
    }
    if (localUnavailable()) {
      log.appendLine('local mode is not available on Windows: connect to a server (Agent Router: Connect to a Server…)');
      return;
    }
    const r = await router.ensure(context, log);
    log.appendLine(`ensure: ${r}`);
    void status.refresh();
    if (r === 'started' || r === 'upgraded') {
      // A fresh install: offer the one-time set-up rather than doing it unasked.
      const set = context.globalState.get<boolean>('setUpDone');
      if (!set) {
        const pick = await vscode.window.showInformationMessage(
          'Agent Router is running. Set it up to start at login and to route Claude Code through it?', 'Set up', 'Not now');
        if (pick === 'Set up') {
          await vscode.commands.executeCommand('agentRouter.setUp');
          await context.globalState.update('setUpDone', true);
        }
      }
    }
    if (!claude.claudeCodeInstalled()) {
      log.appendLine('the Claude Code extension is not installed; the router still serves the CLI and the phone.');
    }
  })();
}

export function deactivate(): void {
  // Nothing: the router is not ours to stop, and neither is the agent.
}
