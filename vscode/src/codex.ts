// Reversible attachment of the published native Codex sidebar to the managed
// app-server. Inference configuration is supplied by Settings -> Clients.
import * as vscode from 'vscode';
import * as fs from 'fs';
import * as path from 'path';
import { execFile } from 'child_process';
import { routerPath } from './router';

const KEY = 'managedCodexLauncher';
const QUALIFIED_EXTENSION = '26.1007.21434';
interface Kept { readonly launcher: string; readonly previous?: string }

function execute(executable: string, args: readonly string[]): Promise<string> {
  return new Promise((resolve, reject) => {
    execFile(executable, [...args], { timeout: 30_000, windowsHide: true }, (error, stdout, stderr) => {
      if (error) reject(new Error(stderr.trim() || error.message));
      else resolve(stdout.trim());
    });
  });
}

export async function configure(context: vscode.ExtensionContext, log: vscode.OutputChannel): Promise<void> {
  const extension = vscode.extensions.getExtension('openai.chatgpt');
  if (!extension) throw new Error('Install the native Codex extension first.');
  if (extension.packageJSON.version !== QUALIFIED_EXTENSION) throw new Error(`Managed attachment currently requires Codex extension ${QUALIFIED_EXTENSION}.`);
  const cfg = vscode.workspace.getConfiguration('chatgpt');
  if (cfg.get<boolean>('runCodexInWindowsSubsystemForLinux', false)) throw new Error('Configure Agent Router from the WSL workspace to use a managed Linux Codex backend.');
  const kept = context.globalState.get<Kept>(KEY);
  const current = cfg.inspect<string>('cliExecutable')?.globalValue;
  const platform = process.platform === 'darwin' ? 'macos' : process.platform === 'win32' ? 'windows' : 'linux';
  const arch = process.arch === 'arm64' ? 'aarch64' : 'x86_64';
  const bundled = path.join(extension.extensionPath, 'bin', `${platform}-${arch}`, process.platform === 'win32' ? 'codex.exe' : 'codex');
  const backend = current && current !== kept?.launcher ? current : bundled;
  if (!fs.existsSync(backend)) throw new Error('The native Codex backend is missing. Reinstall the qualified extension.');
  const launcher = await execute(routerPath(), ['codex', 'configure', backend]);
  if (!path.isAbsolute(launcher) || !fs.existsSync(launcher)) throw new Error('Agent Router did not create its Codex launcher.');
  // Preserve the original choice across repeated installs and launcher updates.
  const previous = kept && current === kept.launcher ? kept.previous : current;
  await context.globalState.update(KEY, { launcher, previous } satisfies Kept);
  await cfg.update('cliExecutable', launcher, vscode.ConfigurationTarget.Global);
  log.appendLine(`Codex connected to the managed backend through ${launcher}`);
  void vscode.window.showInformationMessage('Codex is connected to Agent Router. Finish active turns, then reload this window to reconnect its sidebar.');
}

export async function restore(context: vscode.ExtensionContext, log: vscode.OutputChannel): Promise<void> {
  const kept = context.globalState.get<Kept>(KEY);
  if (!kept) return;
  const cfg = vscode.workspace.getConfiguration('chatgpt');
  if (cfg.inspect<string>('cliExecutable')?.globalValue === kept.launcher) {
    await cfg.update('cliExecutable', kept.previous, vscode.ConfigurationTarget.Global);
    log.appendLine('Codex’s previous launcher restored.');
  } else {
    log.appendLine('Codex launcher was edited; the current choice was preserved.');
  }
  await context.globalState.update(KEY, undefined);
  void vscode.window.showInformationMessage('Codex launch configuration restored. Finish active turns, then reload this window.');
}
