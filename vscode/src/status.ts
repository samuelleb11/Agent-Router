// The menubar app's job, inside the editor: the two quota windows, at a glance. Local
// mode reads the router on this machine; remote mode reads the server, with the password.

import * as vscode from 'vscode';
import { base, mode } from './router';
import * as remote from './remote';
import * as agent from './agent';

interface Window { utilization?: number | null; resets_at?: string | null }
interface Quota { five_hour?: Window; seven_day?: Window; severity?: string; error?: string }

const pct = (w?: Window) => (w?.utilization === null || w?.utilization === undefined ? '—' : `${Math.round(w.utilization)}%`);

const remoteOnlyTooltip =
  'On Windows, Agent Router runs on a server and this machine runs only the agent. Click to connect to a server.';

export class StatusBar implements vscode.Disposable {
  private readonly item: vscode.StatusBarItem;
  private timer: NodeJS.Timeout | undefined;
  /** Each refresh takes a number; one that finishes after a newer one started says
   *  nothing, so a slow answer from a server can never overwrite a fresher state. */
  private seq = 0;
  /** The last remote state written to the log, so the log gets transitions, not a line
   *  a minute. */
  private said = '';

  constructor(private readonly context: vscode.ExtensionContext, private readonly log: vscode.OutputChannel) {
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
    const seq = ++this.seq;
    if (!vscode.workspace.getConfiguration('agentRouter').get<boolean>('showStatusBar', true)) {
      this.item.hide();
      return;
    }
    const m = mode();
    if (m.kind === 'remote') { return this.refreshRemote(seq, m.server); }
    if (m.kind === 'invalid') {
      this.show('$(warning) Router: bad server address', `agentRouter.server is not usable: ${m.why} Click to connect to a server.`,
        'agentRouter.connect', 'statusBarItem.warningBackground');
      return;
    }
    if (!remote.localModeAvailable()) {
      this.show('$(plug) Agent Router: connect', remoteOnlyTooltip, 'agentRouter.connect');
      return;
    }
    return this.refreshLocal(seq);
  }

  private async refreshLocal(seq: number): Promise<void> {
    let q: Quota | undefined;
    try {
      const r = await fetch(`${base()}/api/quota`, { signal: AbortSignal.timeout(3000) });
      q = (await r.json()) as Quota;
    } catch {
      q = undefined;
    }
    if (seq !== this.seq) { return; }
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

  /** The server's quota, read with the saved password. Three outcomes are told apart:
   *  the password rejected (401 or 403), the server unreachable, and an answer. */
  private async refreshRemote(seq: number, server: string): Promise<void> {
    const host = remote.hostOf(server);
    const password = await remote.readPassword(this.context.secrets, server);
    if (seq !== this.seq) { return; }
    if (!password) {
      this.note(`no password saved for ${host}`);
      this.show('$(key) Router: connect', `Agent Router is set to use ${host}, and this machine holds no password for it. Click to connect.`,
        'agentRouter.connect');
      return;
    }
    const [q, h] = await Promise.all([remote.fetchQuota(server, password), remote.fetchHealth(server)]);
    if (seq !== this.seq) { return; }
    switch (q.kind) {
      case 'unreachable':
        this.note(`server unreachable: ${q.detail}`);
        this.show('$(debug-disconnect) Router: server unreachable', `Agent Router could not reach ${host}: ${q.detail}. Click to try again.`,
          'agentRouter.refreshStatus');
        return;
      case 'rejected':
        this.note(`password rejected (${q.status})${q.detail ? `: ${q.detail}` : ''}`);
        this.show('$(key) Router: password rejected',
          `${host} refused the saved password (${q.status}${q.detail ? `: ${q.detail}` : ''}). Click to connect again.`,
          'agentRouter.connect', 'statusBarItem.warningBackground');
        return;
      case 'unexpected':
        this.note(`unexpected answer: ${q.detail}`);
        this.show('$(warning) Router', `${host} answered unexpectedly: ${q.detail}. Click to open the dashboard in your browser.`,
          'agentRouter.openDashboard', 'statusBarItem.warningBackground');
        return;
    }
    const quota = q.body ?? {};
    if (quota.error) {
      this.note(`ok, with an error: ${quota.error}`);
      this.show('$(warning) Router', `${host}: ${quota.error}`, 'agentRouter.openDashboard', 'statusBarItem.warningBackground');
      return;
    }
    this.note('ok');
    const md = new vscode.MarkdownString();
    md.appendMarkdown('**Agent Router** on ');
    md.appendText(host);
    md.appendMarkdown('\n\n');
    for (const [label, w] of [['5-hour window', quota.five_hour], ['7-day window', quota.seven_day]] as const) {
      md.appendText(`${label}: ${pct(w)}${w?.resets_at ? `, resets ${w.resets_at}` : ''}`);
      md.appendMarkdown('\n\n');
    }
    if (h.kind === 'ok') {
      const version = h.body?.version;
      md.appendText(`Server${version ? ` ${version}` : ''}, build ${h.body?.build ?? 'unknown'}`);
      md.appendMarkdown('\n\n');
    }
    if (!agent.isSetUpFor(this.context, server)) {
      md.appendMarkdown('This machine is not set up for it yet: run **Agent Router: Set Up on This Machine**.\n\n');
    }
    md.appendMarkdown('Click to open the dashboard in your browser.');
    this.show(`$(cloud) 5h ${pct(quota.five_hour)} · 7d ${pct(quota.seven_day)}`, md, 'agentRouter.openDashboard',
      quota.severity === 'critical' ? 'statusBarItem.errorBackground'
      : quota.severity === 'warning' ? 'statusBarItem.warningBackground'
      : undefined);
  }

  private show(text: string, tooltip: string | vscode.MarkdownString, command: string, background?: string): void {
    this.item.text = text;
    this.item.tooltip = tooltip;
    this.item.command = command;
    this.item.backgroundColor = background ? new vscode.ThemeColor(background) : undefined;
    this.item.show();
  }

  private note(state: string): void {
    if (state !== this.said) {
      this.said = state;
      this.log.appendLine(`status: ${state}`);
    }
  }

  dispose(): void {
    this.stop();
    this.item.dispose();
  }
}
