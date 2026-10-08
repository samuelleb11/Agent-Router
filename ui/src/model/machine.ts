// New for agent-router: the Machines screen's model, with no DOM and no React (docs/web-console-contracts.md,
// "Machines"). How a machine connects to a hosted router — the installer the router serves, one
// command per platform at the page's own origin (MA-C2) — and every machine the router has seen as
// the screen lists it: online first, then by label, each with its platform in words, how long it has
// been online or offline, and whether it runs the router's own build (MA-C3, MA-C4). A hosted
// router's empty session list says the same "no machine yet" sentence, and links to the screen
// (SE-C187).

import { span } from './format';

// ---------- connecting a machine ----------

/// The Machines screen's first section, and the link to it from a hosted router's empty session
/// list (MA-C2, SE-C187).
export const CONNECT = 'Connect a machine';

/// A hosted router serves the sessions of the machines connected to it, and a machine connects by
/// running the installer the router serves without a password, which installs the Agent Router
/// agent and asks there for the router's password (MA-C2).
export const CONNECT_HOW = 'Run this on a machine to connect it — it asks for this router’s password.';

/// What else the installer does where VS Code is: it adds the Agent Router extension, which wraps
/// Claude Code there, so the machine's sessions can be typed into and not only followed (MA-C2).
export const WITH_VS_CODE = 'With VS Code’s code command on the machine, the script also adds the Agent Router extension, '
  + 'so its sessions can be typed into from here and from the phone; without it, they can only be followed.';

export interface InstallCommand {
  /// The platforms it runs on, which label it.
  readonly label: string;
  readonly command: string;
}

/// The commands that connect a machine to the router at `origin`, one for each platform: a shell's
/// on macOS and Linux, PowerShell's on Windows (MA-C2).
export const installCommands = (origin: string): readonly InstallCommand[] => [
  { label: 'macOS and Linux', command: `curl -fsSL ${origin}/agent/install.sh | sh` },
  { label: 'Windows (PowerShell)', command: `irm ${origin}/agent/install.ps1 | iex` },
];

/// A hosted router that no machine has connected to: said under the commands here, and by a
/// hosted router's empty session list (MA-C5, SE-C187).
export const AGENTLESS = 'No machine has connected yet.';

/// A local router takes no machines: it serves the sessions of the machine it runs on, so the
/// screen says so and offers nothing to run (MA-C7).
export const ON_THIS_MACHINE = 'Machines connect to a hosted router. This router runs on this machine and serves its sessions itself, so there is nothing to connect.';

// ---------- the machines ----------

/// The list's heading: the router lists what it has seen since it last started, online or not
/// (MA-C3).
export const SEEN = 'Seen since this router started';

/// One machine as `GET /api/machines` lists it (src/machines.rs, `Seen`): every machine the router
/// has seen since it last started, online or not.
export interface MachineRow {
  readonly id: string;
  /// What its agent was given with `--label`, else its host name (src/agent.rs).
  readonly label: string;
  /// Its operating system as Rust names it: `macos`, `linux`, `windows`.
  readonly platform: string;
  readonly version: string;
  /// The source tree its agent was built from, `tree:<sha>`, as `/_router/health` names the
  /// router's own.
  readonly build: string;
  readonly online: boolean;
  /// When it connected, while it is online, and when it went away once it is not. Epoch
  /// milliseconds, on the router's clock.
  readonly since_ms: number;
  /// Where its Library stands (src/library/push.rs, `Standing`); absent from a router that
  /// keeps none.
  readonly library?: Standing;
}

/// A machine's build compared with the router's: quietly the same, or another, with what updates
/// it (MA-C4).
export const SAME_BUILD = 'Same build as this router';
export const OTHER_BUILD = 'Runs a different build from this router — run the installer on it again to update it.';

/// One machine as the screen lists it.
export interface MachineItem {
  readonly id: string;
  /// Its label, trimmed; a blank one names it by the start of its id, so no machine goes unnamed.
  readonly label: string;
  /// Its platform in words (MA-C3).
  readonly platform: string;
  readonly online: boolean;
  /// "online for 3h", "offline for 2d" (MA-C3).
  readonly presence: string;
  /// Whether it runs the router's own build, and the words for it; null until the router's build is
  /// known, or when the machine did not say its own (MA-C4).
  readonly build: { readonly same: boolean; readonly text: string } | null;
  /// Where its Library stands, in words, and whether it is something to act on (LB-C11).
  readonly library: { readonly text: string; readonly needs: boolean } | null;
}

const PLATFORMS: Readonly<Record<string, string>> = { macos: 'macOS', linux: 'Linux', windows: 'Windows' };

/// A platform in the reader's words — macOS, Linux, Windows — and one the console does not know as
/// the agent sent it (MA-C3). A word like `constructor` cannot reach through the table's prototype.
export function platformOf(platform: string | null | undefined): string {
  const p = typeof platform === 'string' ? platform.trim() : '';
  const key = p.toLowerCase();
  return (Object.hasOwn(PLATFORMS, key) ? PLATFORMS[key] : undefined) ?? p;
}

/// Whether a machine is online, and for how long it has been online or offline, measured from
/// `since_ms` (MA-C3) on this page's clock: the list carries no `as_of_ms` of the router's, as a
/// session row does (SE-C43), and the seconds two clocks differ by do not move an hour or a day.
/// With no `since_ms`, only which.
export function presence(m: Pick<MachineRow, 'online' | 'since_ms'>, now: number): string {
  const word = m.online ? 'online' : 'offline';
  return typeof m.since_ms === 'number' && m.since_ms > 0 ? `${word} for ${span(now - m.since_ms)}` : word;
}

/// Labels in the alphabetical order of the locale, case aside, so a machine keeps its place; two
/// that differ only in case stay two, the capitalised one first, as the iPhone orders them. The
/// session list heads its machines in the same order (SE-C220).
export const machineOrder = (a: string, b: string): number => a.localeCompare(b, undefined, { sensitivity: 'accent' }) || (a < b ? -1 : a > b ? 1 : 0);

/// Every machine the router has seen, as the screen lists them: online first, then by label
/// (MA-C3), each with its build compared with the router's, `routerBuild`, which is null until
/// `/_router/health` has answered (MA-C4).
export function machineItems(rows: readonly MachineRow[], routerBuild: string | null, now: number): MachineItem[] {
  return rows
    .map((m): MachineItem => {
      const id = String(m.id ?? '');
      const build = typeof m.build === 'string' ? m.build : '';
      return {
        id,
        label: (typeof m.label === 'string' && m.label.trim()) || id.slice(0, 8),
        platform: platformOf(m.platform),
        online: m.online === true,
        presence: presence(m, now),
        build: routerBuild && build ? { same: build === routerBuild, text: build === routerBuild ? SAME_BUILD : OTHER_BUILD } : null,
        library: m.library ? standingWords(m.library) : null,
      };
    })
    .sort((a, b) => Number(b.online) - Number(a.online) || machineOrder(a.label, b.label));
}

/// How many are online and how many not, at a glance over the list: "2 online · 1 offline" (MA-C3).
export function tally(items: readonly Pick<MachineItem, 'online'>[]): string {
  const online = items.filter((m) => m.online).length;
  const offline = items.length - online;
  return [online ? `${online} online` : '', offline ? `${offline} offline` : ''].filter(Boolean).join(' · ');
}

// ---------- the Library ----------

/// Where a machine's Library stands, as `/api/machines` gives it (src/library/push.rs, `Standing`).
export interface Standing {
  readonly state: 'in_step' | 'behind' | 'failed' | 'replaced' | 'predates' | 'opted_out' | 'not_yet';
  readonly behind: number;
  readonly failed: number;
  readonly replaced: readonly string[];
  readonly at_ms: number | null;
}

/// A machine's Library in words, and whether it is something to act on (LB-C11).
export function standingWords(s: Standing | null | undefined): { text: string; needs: boolean } {
  switch (s?.state) {
    case 'in_step':
      return { text: 'Library in step', needs: false };
    case 'behind':
      return { text: `Library behind on ${s.behind} item${s.behind === 1 ? '' : 's'}`, needs: true };
    case 'failed':
      return { text: `Library: ${s.failed} item${s.failed === 1 ? '' : 's'} could not be placed`, needs: true };
    case 'replaced':
      return { text: `Library in step; it put back ${s.replaced.join(', ')} over a copy changed there`, needs: false };
    case 'predates':
      return { text: 'Its agent predates the Library — run the installer on it again', needs: true };
    case 'opted_out':
      return { text: 'Takes no part in the Library', needs: false };
    default:
      return { text: 'Nothing heard of its Library yet', needs: false };
  }
}
