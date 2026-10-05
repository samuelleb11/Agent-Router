// New for agent-router: what the dashboard decides from `/api/auth` and the accounts list, with no
// DOM (assets/index.html: `hosted`, `HOSTED_COPY`, `applyMode`, `applyAuth`, `pwForm`,
// `connectSnippet`, `renderRouterInfo`, `renderPush`, `openEdit`, `renderAccounts`). A hosted
// router (`agent-router serve --server`) has no Mac behind it, and says so in `mode`; a router that
// predates the field is local, and nothing changes (ST-C100). The words are the dashboard's,
// docs/web-console-contracts.md ST-C7 to ST-C103 and SH-C18, SH-C19.
//
// Where the page broke its own rule, the rule is kept:
// - a password's length is counted the way the router counts it, in characters, so four emoji
//   are four, not eight (ST-C74);
// - until `/api/auth` has said which kind of router this is, none of the machine-only controls
//   is offered: not the Claude Code or Network pages, nor the sign-in or profile ways to add an
//   account (ST-C101).

import { ago } from './format';

/// What `GET /api/auth` says (src/auth.rs). A local router that predates hosting sends no `mode`.
export interface Auth {
  readonly configured?: boolean | null;
  readonly updated_at?: number | null;
  readonly from_loopback?: boolean | null;
  readonly remote_bind?: boolean | null;
  readonly listening_on?: string | null;
  readonly lan_url?: string | null;
  readonly env_override?: boolean | null;
  readonly mode?: 'server' | (string & {}) | null;
  readonly password_pinned?: boolean | null;
  readonly public_url?: string | null;
}

/// A hosted router names itself; one that predates the field, or has not answered yet, is not one
/// (ST-C100, OV-C203, SE-C186).
export const isHosted = (auth: Auth | null | undefined): boolean => auth?.mode === 'server';

/// Whether the router has said which kind it is.
const known = (auth: Auth | null | undefined): auth is Auth => auth != null;

export type Page = 'accounts' | 'claude-code' | 'network' | 'password' | 'notifications' | 'router';

/// The Settings pages offered. A hosted router has no Claude Code page and no Network page
/// (OV-C204, ST-C53, ST-C72); nor does a router that has not said yet what it is (ST-C101).
export function settingsPages(auth: Auth | null | undefined): Page[] {
  const local = known(auth) && !isHosted(auth);
  return local ? ['accounts', 'claude-code', 'network', 'password', 'notifications', 'router'] : ['accounts', 'password', 'notifications', 'router'];
}

/// Whether the setup and the wrapper's state are read at all: never on a hosted router, which
/// would refuse (OV-C204, ST-C53).
export const readsSetup = (auth: Auth | null | undefined): boolean => known(auth) && !isHosted(auth);

export type Source = 'signin' | 'token' | 'profile';

/// The ways to add an account: on a local router sign-in first, then a pasted token, then a
/// separate profile (ST-C40); on a hosted router, which never runs `claude`, a pasted token only
/// (ST-C52, OV-C205). The first is the form's choice when it opens.
export function accountSources(auth: Auth | null | undefined): Source[] {
  return known(auth) && !isHosted(auth) ? ['signin', 'token', 'profile'] : ['token'];
}

/// The add form for a choice: the token field for a token, the sign-in explanation for sign-in,
/// and the button's words (ST-C43).
export function addForm(source: Source): { readonly token: boolean; readonly signin: boolean; readonly save: string; readonly help: string | null } {
  return {
    token: source === 'token',
    signin: source === 'signin',
    save: source === 'signin' ? 'Start sign-in' : 'Add account',
    help:
      source === 'profile'
        ? 'The router creates an isolated profile directory and gives you a command to run in a terminal, then detects when you finish and fills in the rest.'
        : null,
  };
}

// ---------- the password ----------

/// What a deployment's pinned password reads, with nothing to set, change or remove (ST-C79).
export const PINNED = 'Set by the deployment. Change the Secret and restart the router.';

export type PasswordForm =
  | { readonly pinned: string }
  | { readonly save: 'Set password' | 'Change password'; readonly secondary: 'Not now' | 'Remove password' | null };

/// The password form, in Settings or as the first-boot prompt (ST-C73, ST-C75).
export function passwordForm(auth: Auth | null | undefined, firstBoot: boolean): PasswordForm {
  if (!firstBoot && auth?.password_pinned) return { pinned: PINNED };
  const configured = !!auth?.configured;
  return {
    save: configured && !firstBoot ? 'Change password' : 'Set password',
    secondary: firstBoot ? 'Not now' : configured ? 'Remove password' : null,
  };
}

/// The router's own minimum, in characters (src/auth.rs).
export const MIN_PASSWORD = 8;

/// What is wrong with a new password before it is sent, or nothing: its length counted in
/// characters as the router counts them, not UTF-16 units (ST-C74), and the two entries agreeing
/// (ST-C73). It is sent exactly as typed, spaces included.
export function passwordProblem(password: string, again: string): string | null {
  if ([...password].length < MIN_PASSWORD) return `use at least ${MIN_PASSWORD} characters`;
  if (password !== again) return 'the two entries differ';
  return null;
}

/// Whether the first-boot password prompt opens by itself: the router listens on the network, no
/// password is set, the console is read from this machine, the router is local, the password is
/// not pinned, the reader has not dismissed it in this load, and Settings is not on screen
/// (ST-C80).
export function asksFirstPassword(auth: Auth | null | undefined, now: { readonly dismissed: boolean; readonly settingsOpen: boolean }): boolean {
  return (
    !!auth?.remote_bind && !auth.configured && !!auth.from_loopback && !isHosted(auth) && !auth.password_pinned && !now.dismissed && !now.settingsOpen
  );
}

/// The password page's heading line: set and when, or why it matters (ST-C77).
export function passwordStatus(auth: Auth, now: number = Date.now()): string {
  if (auth.configured) return `set${auth.updated_at ? ` · changed ${ago(auth.updated_at, now)}` : ''}`;
  return auth.remote_bind ? 'not set — other devices are refused' : 'not set — only matters on the network';
}

export interface NetworkStatus {
  readonly tone: 'ok' | 'bad';
  readonly text: string;
  /// Its explanation, shown as text, never only a title (SH-C19).
  readonly explanation: string;
}

/// While the router listens beyond loopback, every screen says so and whether a password guards
/// it — guarded in a calm tone, unguarded in the alarm tone — and on loopback only, nothing
/// (SH-C18, SH-C19).
export function networkStatus(auth: Auth | null | undefined): NetworkStatus | null {
  if (!auth?.remote_bind) return null;
  const guarded = !!auth.configured;
  return {
    tone: guarded ? 'ok' : 'bad',
    text: guarded ? 'network · password set' : 'network · no password',
    explanation: isHosted(auth)
      ? 'Every caller must give the password'
      : guarded
        ? 'Reachable from your network; other devices must give the password'
        : 'Reachable from your network, but other devices are refused until a password is set',
  };
}

// ---------- the Router page ----------

/// What points a Claude Code at this router: the page's own origin, right behind an https ingress
/// and through a port-forward, with `ANTHROPIC_AUTH_TOKEN` wherever the router asks the caller for
/// the password — a hosted router, or a page read from another machine — and the single line it
/// always was at the machine itself (ST-C96). Until `/api/auth` has answered, it is drawn as for
/// this machine (ST-C97). The password itself is never in it.
export function connectSnippet(origin: string, auth: Auth | null | undefined): string {
  const url = `"ANTHROPIC_BASE_URL": ${JSON.stringify(origin)}`;
  return isHosted(auth) || auth?.from_loopback === false ? `{ "env": {\n  ${url},\n  "ANTHROPIC_AUTH_TOKEN": "<the router password>"\n} }` : `{ "env": { ${url} } }`;
}

/// `GET /_router/health`, as much of it as the page shows.
export interface Health {
  readonly version?: string | null;
  readonly pid?: number | null;
}

export interface Fact {
  readonly label: string;
  readonly value: string;
}

/// The Router page's facts: where the console is read from, the version and, on a local router
/// only, the process id — a hosted router's is nothing a reader can act on (ST-C95, OV-C210).
export function routerFacts(health: Health | null | undefined, auth: Auth | null | undefined, host: string): Fact[] {
  const facts: Fact[] = [
    { label: 'Listening on', value: host },
    { label: 'Version', value: health?.version || '—' },
  ];
  if (!isHosted(auth)) facts.push({ label: 'Process', value: health?.pid != null ? String(health.pid) : '—' });
  return facts;
}

/// `GET /api/push`, as much of it as the hint reads.
export interface Push {
  readonly configured?: boolean | null;
  readonly key_file?: string | null;
}

/// Where the APNs key came from, or where to put one: a key file the router took in names its
/// path; with none, `~/.agent-router` on a local router and the deployment's push Secret on a
/// hosted one, or paste it (ST-C86, OV-C210).
export function pushKeyHint(push: Push | null | undefined, auth: Auth | null | undefined): string | null {
  if (push?.key_file) return `key file: ${push.key_file}`;
  if (push?.configured) return null;
  return isHosted(auth)
    ? 'Put the AuthKey_<key id>.p8 that developer.apple.com hands out in the deployment’s push Secret — or paste it here.'
    : 'Drop the AuthKey_<key id>.p8 that developer.apple.com hands out into ~/.agent-router and the router takes it in by itself — or paste it here.';
}

// ---------- the words that speak of this Mac ----------

/// A note's words: text, and the code and emphasis inside it, never markup.
export type Segment = string | { readonly code: string } | { readonly em: string };

export type NoteId = 'store' | 'push' | 'password' | 'usageKey' | 'token';

/// The notes as a local router says them: the dashboard's own markup, `#storenote`, `#pushnote`,
/// `#pwnote`, `#e-usage-note` and `#a-token-note`.
const LOCAL_NOTES: Readonly<Record<NoteId, readonly Segment[]>> = {
  store: [
    "Everything above is stored locally. The router's own settings (port, database location, upstream, body cap) are environment variables read at startup — see the README.",
  ],
  push: [
    "The iPhone app is told the moment a session waits on you — a permission, a question — through Apple's push service. That needs an APNs key from your developer account (Keys → + → Apple Push Notifications service); paste the ",
    { code: '.p8' },
    " here once. It stays in the router's secrets file, never in the database.",
  ],
  password: [
    'Asked of any device that is not this Mac — a browser gets the usual prompt (any username), and a Claude Code elsewhere sets ',
    { code: 'ANTHROPIC_AUTH_TOKEN' },
    ' to it. Forgotten? Set a new one from here; this Mac never needs the old one.',
  ],
  usageKey: [
    'Copied from your claude.ai browser cookie. Used ',
    { em: 'only' },
    ' to read usage — it is never sent to api.anthropic.com and never signs a request, and it lives in a separate Keychain entry from the credential that does.',
  ],
  token: [
    'Run ',
    { code: 'claude setup-token' },
    ' in a terminal that is logged into the other account and paste the ',
    { code: 'sk-ant-oat01-…' },
    ' value it prints. A ',
    { code: 'sk-ant-sid…' },
    ' session key copied from claude.ai is a browser cookie, not an API credential, and will not authenticate.',
  ],
};

/// The same notes as a hosted router says them: of its deployment, not of this Mac (ST-C102).
const HOSTED_NOTES: Readonly<Record<NoteId, readonly Segment[]>> = {
  store: ['Everything above is kept in the router’s data directory. Its own settings (port, database location, upstream, body cap) are set by the deployment and read at startup.'],
  push: [
    'The iPhone app is told the moment a session waits on you — a permission, a question — through Apple’s push service. That needs an APNs key from your developer account (Keys → + → Apple Push Notifications service): put the ',
    { code: '.p8' },
    ' in the deployment’s push Secret, or paste it here once. A pasted key stays in the router’s secrets file, never in the database.',
  ],
  password: ['Asked of every caller — a browser gets the usual prompt (any username), and Claude Code sets ', { code: 'ANTHROPIC_AUTH_TOKEN' }, ' to it.'],
  usageKey: ['Copied from your claude.ai browser cookie. Used ', { em: 'only' }, ' to read usage — it is never sent to api.anthropic.com and never signs a request.'],
  token: [
    'Run ',
    { code: 'claude setup-token' },
    ' in a terminal on any machine, sign in as the account to add, and paste the ',
    { code: 'sk-ant-oat01-…' },
    ' value it prints. A ',
    { code: 'sk-ant-sid…' },
    ' session key copied from claude.ai is a browser cookie, not an API credential, and will not authenticate.',
  ],
};

/// The notes that speak of where things live, in the words of the router they describe (ST-C38,
/// ST-C48, ST-C76, ST-C94, ST-C99, ST-C102).
export const notes = (auth: Auth | null | undefined): Readonly<Record<NoteId, readonly Segment[]>> => (isHosted(auth) ? HOSTED_NOTES : LOCAL_NOTES);

/// A note's words as plain text.
export const noteText = (segments: readonly Segment[]): string => segments.map((s) => (typeof s === 'string' ? s : 'code' in s ? s.code : s.em)).join('');

// ---------- accounts ----------

/// One row of `GET /api/accounts` (src/api.rs), as much as Settings decides from.
export interface Account {
  readonly id: string;
  readonly label?: string | null;
  readonly email?: string | null;
  /// `keychain` is the built-in Claude Code login; `profile` a separate Claude Code profile;
  /// `token` a pasted long-lived token.
  readonly source?: string | null;
  readonly active?: number | boolean | null;
  /// Absent when the credential was not checked; true or 1 when it works.
  readonly ready?: boolean | number | null;
  readonly detail?: string | null;
  readonly expires_at?: number | null;
}

const on = (v: number | boolean | null | undefined): boolean => v === true || (typeof v === 'number' && v !== 0);

/// The note under an account's identity: a Claude Code login's identity is read, not typed —
/// locally from this machine's login, and on a hosted router a login it cannot sign with (ST-C31);
/// a token's identity is what labels its traffic (ST-C32).
export function identityNote(account: Pick<Account, 'source'>, auth: Auth | null | undefined): string {
  if (account.source !== 'keychain') {
    return 'A long-lived token cannot report which account it belongs to, so setting the identity here is what labels this account’s traffic in the charts.';
  }
  return isHosted(auth)
    ? 'Claude Code’s own login, from the machine this store was copied from. A hosted router cannot sign with it.'
    : 'Identity is read from the Claude Code login on this machine and refreshes on every request, so it cannot be set by hand.';
}

/// Whether Settings → Accounts opens by itself: when nothing can sign, or an account that could
/// sign reports a credential that does not work (ST-C7). On a hosted router only the accounts it
/// could sign with count — tokens, and whichever account is active — so a Claude Code login or a
/// profile copied in with a Mac's store interrupts only while it is the one in use (ST-C8). At most
/// once per load, which the screen keeps.
export function opensSettings(accounts: readonly Account[], auth: Auth | null | undefined): boolean {
  const signers = isHosted(auth) ? accounts.filter((a) => (a.source !== 'keychain' && a.source !== 'profile') || on(a.active)) : accounts;
  return !signers.length || signers.some((a) => a.ready != null && !on(a.ready));
}

/// Whether an account can be removed: locally the built-in Claude Code account cannot, since the
/// router would discover it again; on a hosted router, which seeds none, it goes like any other
/// (ST-C26).
export const removable = (account: Pick<Account, 'source'>, auth: Auth | null | undefined): boolean => account.source !== 'keychain' || isHosted(auth);
