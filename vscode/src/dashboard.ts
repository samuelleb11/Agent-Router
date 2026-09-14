// The router's own pages, inside the editor. VS Code's Simple Browser opens a loopback
// page in an editor tab, and hands off to the Integrated Browser where that exists; a
// webview with a frame is the fallback, and the system browser is always a click away.

import * as vscode from 'vscode';
import { base } from './router';

export async function open(pagePath: '/' | '/live', title: string): Promise<void> {
  const url = `${base()}${pagePath}`;
  try {
    await vscode.commands.executeCommand('simpleBrowser.api.open', vscode.Uri.parse(url), { viewColumn: vscode.ViewColumn.Active });
    return;
  } catch {
    /* not every build offers it */
  }
  const panel = vscode.window.createWebviewPanel('agentRouter.page', title, vscode.ViewColumn.Active, {
    enableScripts: true,
    enableForms: true,
    retainContextWhenHidden: true,
  });
  panel.webview.html = `<!DOCTYPE html><html><head>
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; frame-src ${base()}; style-src 'unsafe-inline';">
<style>html,body,iframe{margin:0;padding:0;width:100%;height:100%;border:0;background:#fff}</style>
</head><body><iframe src="${url}" sandbox="allow-scripts allow-forms allow-same-origin allow-downloads"></iframe></body></html>`;
}

export function openExternal(pagePath: '/' | '/live'): Thenable<boolean> {
  return vscode.env.openExternal(vscode.Uri.parse(`${base()}${pagePath}`));
}
