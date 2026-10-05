// Claude Code's side of the arrangement, done through its own documented settings and
// nothing else: `claudeCode.claudeProcessWrapper` launches Claude through the router's
// wrapper, which is what makes a session answerable from the phone. The base URL goes
// through Claude Code's settings file, written by the router's `install` (see router.ts),
// or in remote mode by `agent install` with the server's password beside it (agent.ts).

import * as vscode from 'vscode';
import { wrapperPath } from './router';

const CLAUDE_CODE = 'anthropic.claude-code';

export const claudeCodeInstalled = (): boolean => vscode.extensions.getExtension(CLAUDE_CODE) !== undefined;

export async function configureWrapper(log: vscode.OutputChannel): Promise<void> {
  const cfg = vscode.workspace.getConfiguration('claudeCode');
  const want = wrapperPath();
  if (cfg.inspect<string>('claudeProcessWrapper')?.globalValue !== want) {
    await cfg.update('claudeProcessWrapper', want, vscode.ConfigurationTarget.Global);
    log.appendLine(`claudeCode.claudeProcessWrapper = ${want}`);
  }
  // Anthropic's documentation: a wrapped setup starts new conversations in Manual
  // permission mode unless this is set. Keep whatever the person chose; otherwise
  // name the default explicitly so nothing changes for them.
  if (cfg.inspect<string>('initialPermissionMode')?.globalValue === undefined) {
    await cfg.update('initialPermissionMode', 'default', vscode.ConfigurationTarget.Global);
    log.appendLine('claudeCode.initialPermissionMode = default');
  }
}

export async function unconfigureWrapper(log: vscode.OutputChannel): Promise<void> {
  const cfg = vscode.workspace.getConfiguration('claudeCode');
  if (cfg.inspect<string>('claudeProcessWrapper')?.globalValue === wrapperPath()) {
    await cfg.update('claudeProcessWrapper', undefined, vscode.ConfigurationTarget.Global);
    log.appendLine('claudeCode.claudeProcessWrapper cleared');
  }
}
