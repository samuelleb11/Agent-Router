// The menubar app's job, inside the editor: the two quota windows, at a glance.

import * as vscode from 'vscode';
import { base } from './router';

interface Window { utilization?: number | null; resets_at?: string | null }
interface Quota { five_hour?: Window; seven_day?: Window; severity?: string; error?: string }

export class StatusBar implements vscode.Disposable {
  private readonly item: vscode.StatusBarItem;
  private timer: NodeJS.Timeout | undefined;

  constructor() {
    this.item = vscode.window.createStatusBarItem('agentRouter.quota', vscode.StatusBarAlignment.Right, 50);
    this.item.name = 'Agent Router';
    this.item.command = 'agentRouter.openDashboard';
  }

  start(): void {
    this.stop();
    void this.refresh();
    this.timer = setInterval(() => void this.refresh(), 60_000);
  }

  stop(): void {
    if (this.timer) { clearInterval(this.timer); this.timer = undefined; }
  }

  async refresh(): Promise<void> {
    if (!vscode.workspace.getConfiguration('agentRouter').get<boolean>('showStatusBar', true)) {
      this.item.hide();
      return;
    }
    let q: Quota | undefined;
    try {
      const r = await fetch(`${base()}/api/quota`, { signal: AbortSignal.timeout(3000) });
      q = (await r.json()) as Quota;
    } catch {
      q = undefined;
    }
    if (!q) {
      this.item.text = '$(circle-slash) Router off';
      this.item.tooltip = 'Agent Router is not answering. Click to start it.';
      this.item.command = 'agentRouter.ensure';
      this.item.backgroundColor = undefined;
      this.item.show();
      return;
    }
    this.item.command = 'agentRouter.openDashboard';
    if (q.error) {
      this.item.text = '$(warning) Router';
      this.item.tooltip = q.error;
      this.item.backgroundColor = new vscode.ThemeColor('statusBarItem.warningBackground');
      this.item.show();
      return;
    }
    const pct = (w?: Window) => (w?.utilization === null || w?.utilization === undefined ? '—' : `${Math.round(w.utilization)}%`);
    this.item.text = `$(pulse) 5h ${pct(q.five_hour)} · 7d ${pct(q.seven_day)}`;
    const md = new vscode.MarkdownString();
    md.appendMarkdown(`**Agent Router**\n\n`);
    md.appendMarkdown(`5-hour window: ${pct(q.five_hour)}${q.five_hour?.resets_at ? `, resets ${q.five_hour.resets_at}` : ''}\n\n`);
    md.appendMarkdown(`7-day window: ${pct(q.seven_day)}${q.seven_day?.resets_at ? `, resets ${q.seven_day.resets_at}` : ''}\n\n`);
    md.appendMarkdown(`Click to open the dashboard.`);
    this.item.tooltip = md;
    this.item.backgroundColor =
      q.severity === 'critical' ? new vscode.ThemeColor('statusBarItem.errorBackground')
      : q.severity === 'warning' ? new vscode.ThemeColor('statusBarItem.warningBackground')
      : undefined;
    this.item.show();
  }

  dispose(): void {
    this.stop();
    this.item.dispose();
  }
}
