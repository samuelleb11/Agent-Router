// New for agent-router: the Settings pages' own rules, with no React and no DOM, beside the
// hosted-router and password decisions in ~/model/settings. Each is the router's rule or the
// dashboard's wording (assets/index.html, `renderAccounts`, `openEdit`, `wireAddForm`, `renderSetup`,
// `netForm`, `renderPush`), with the router's own numbers and sentences (src/api.rs,
// src/accounts.rs, src/quota.rs, src/notify.rs, src/hosted.rs), so a rule is checked before a
// request rather than learnt from its refusal (docs/web-console-contracts.md, "Settings").

import { ago, DASH, when } from '~/model/format';
import type { AccountRow, Device, LoginStatus } from '~/data/settings';
import type { Account } from '~/model/settings';

/// The router's integers and booleans for one flag: SQLite gives 1 and 0, JSON gives true and false.
export const flag = (v: number | boolean | null | undefined): boolean => v === true || (typeof v === 'number' && v !== 0);

// ---------- names and ids ----------

/// The longest id the router gives an account (src/accounts.rs, `slugify`).
export const ID_MAX = 48;

/// The id the router gives a name, worked out the way it does (src/accounts.rs, `slugify`): every
/// character that is not an ASCII letter or digit becomes a hyphen, runs of hyphens collapse and the
/// ends are trimmed, letters are lowercased, and the first 48 characters are kept. Null when nothing
/// is left, because the router then makes up a fresh id that no account has.
export function accountId(label: string): string | null {
  const mapped = [...label].map((c) => (/^[A-Za-z0-9]$/.test(c) ? c.toLowerCase() : '-')).join('');
  const joined = mapped
    .split('-')
    .filter((p) => p !== '')
    .join('-');
  return joined === '' ? null : joined.slice(0, ID_MAX);
}

/// What the router answers a sign-in started with no name (src/login.rs), said before anything is
/// sent; a token or a profile with no name would get a made-up id instead (ST-C41).
export const NAME_NEEDED = 'Give the account a name first.';

/// What keeps a name from being added, or nothing: no name at all (ST-C41), or a name whose id is
/// an existing account's, which the router would overwrite in place, credential, source and all
/// (ST-C42).
export function nameProblem(label: string, accounts: readonly Pick<Account, 'id' | 'label' | 'email'>[]): string | null {
  if (label.trim() === '') return NAME_NEEDED;
  const id = accountId(label);
  if (id === null) return null;
  const taken = accounts.find((a) => a.id === id);
  if (!taken) return null;
  const who = taken.email || taken.label || taken.id;
  return `That name is the id “${id}”, which ${who} already has: adding it would replace that account. Choose another name.`;
}

// ---------- the router's prefix and shape rules ----------

/// The router's rule for a credential (src/api.rs, `accounts_edit`): a `claude setup-token` value
/// starts `sk-ant-`, and a claude.ai session key is not one. A blank field is nobody's problem here:
/// the editor reads it as "keep", and the add form asks for a token on its own (ST-C11, ST-C48).
export function tokenProblem(token: string): string | null {
  const t = token.trim();
  if (t === '') return null;
  if (t.startsWith('sk-ant-sid')) return 'that is a claude.ai session key — it belongs in the session-key field, and cannot sign API requests';
  if (!t.startsWith('sk-ant-')) return 'expected a token starting sk-ant- (from `claude setup-token`)';
  return null;
}

/// What the router answers a token account added with no token (src/api.rs, `accounts_add`).
export const TOKEN_NEEDED = 'a token is required for this account type';

/// The router's rule for a claude.ai session key (src/quota.rs, `set_usage_token`). Blank means keep.
export function sessionKeyProblem(key: string): string | null {
  const k = key.trim();
  if (k === '' || k.startsWith('sk-ant-sid')) return null;
  return 'that does not look like a claude.ai session key (expected sk-ant-sid…)';
}

/// The router's test for an identity (src/api.rs, `looks_like_email`): something before an `@`, a
/// dot after it but not first, and no space.
export function looksLikeEmail(s: string): boolean {
  const t = s.trim();
  const at = t.indexOf('@');
  if (at < 0) return false;
  const user = t.slice(0, at);
  const host = t.slice(at + 1);
  return user !== '' && host.includes('.') && !host.startsWith('.') && !t.includes(' ');
}

/// The router's refusal of an identity that is not an email address, said before it is sent
/// (src/api.rs, `accounts_edit`). Blank means keep.
export function identityProblem(email: string): string | null {
  const e = email.trim();
  if (e === '' || looksLikeEmail(e)) return null;
  return `${JSON.stringify(e)} is not an email address`;
}

/// The router's rule for an APNs key (src/notify.rs, `config`): the three together, or nothing sent.
export function pushKeyProblem(teamId: string, keyId: string, p8: string): string | null {
  return teamId.trim() && keyId.trim() && p8.trim() ? null : 'team id, key id and the .p8 key are all needed';
}

/// What the router answers a Firebase key pasted empty (src/notify.rs, `fcm_config`).
export function serviceAccountProblem(json: string): string | null {
  return json.trim() ? null : 'paste the service-account JSON the Firebase console downloads';
}

// ---------- an account, as its card says it ----------

/// The account's name as the page says it: its identity, else its label, else its id.
export const accountName = (a: Pick<Account, 'id' | 'label' | 'email'>): string => a.email || a.label || a.id;

/// Where an account's credential comes from, in words.
export function sourceWord(source: string | null | undefined): string {
  if (source === 'keychain') return 'Claude Code login';
  if (source === 'profile') return 'Claude Code profile';
  if (source === 'token') return 'token';
  return source || DASH;
}

export type Credential =
  | { readonly kind: 'unchecked'; readonly text: string }
  | { readonly kind: 'ready'; readonly text: string }
  | { readonly kind: 'failed'; readonly text: string };

/// A credential's state: "not checked" when the read did not ask, never ready or failed; a ready
/// one names its kind; a failed one carries the router's whole reason (ST-C20).
export function credential(a: Pick<AccountRow, 'ready' | 'detail'>): Credential {
  if (a.ready == null) return { kind: 'unchecked', text: 'not checked' };
  if (flag(a.ready)) return { kind: 'ready', text: a.detail || 'ready' };
  return { kind: 'failed', text: a.detail || 'the router gave no reason' };
}

/// When the credential stops working: "no expiry" for one that never does, its date for one that
/// does, and "—" when that is not known — not checked, or checked and not working (ST-C21).
export function expiry(a: Pick<AccountRow, 'ready' | 'expires_at'>): string {
  if (a.expires_at == null || a.ready == null || !flag(a.ready)) return DASH;
  return a.expires_at > 0 ? when(a.expires_at) : 'no expiry';
}

/// The list's heading: how many, and which one signs (ST-C22).
export function accountsHeading(accounts: readonly Pick<AccountRow, 'id' | 'label' | 'email' | 'active'>[]): string {
  const active = accounts.find((a) => flag(a.active));
  const n = accounts.length;
  return `${n} account${n === 1 ? '' : 's'} · ${active ? `${accountName(active)} active` : 'none active'}`;
}

/// The account that signs once `id` is removed: the oldest one left when `id` is the active one,
/// because the router never leaves itself with nothing to sign with (src/accounts.rs, `remove`).
/// Undefined when `id` is not active, and null when nothing would be left.
export function nextSigner<T extends Pick<AccountRow, 'id' | 'active' | 'created_at'>>(accounts: readonly T[], id: string): T | null | undefined {
  const gone = accounts.find((a) => a.id === id);
  if (!gone || !flag(gone.active)) return undefined;
  const left = accounts.filter((a) => a.id !== id);
  // The router's list is oldest first already; the creation time decides where it is known.
  const oldest = [...left].sort((a, b) => (a.created_at ?? 0) - (b.created_at ?? 0));
  return (left.every((a) => a.created_at != null) ? oldest[0] : left[0]) ?? null;
}

/// Remove's confirmation: which account, what goes with it, and who signs next (ST-C28).
export function removeWords(accounts: readonly AccountRow[], id: string): string[] {
  const a = accounts.find((x) => x.id === id);
  const name = a ? accountName(a) : id;
  const said = [`Remove ${name} from the router? Its stored credential and its claude.ai session key are deleted with it.`];
  const next = nextSigner(accounts, id);
  if (next === null) said.push('It is the account signing requests now, and no other is left to sign with.');
  else if (next) said.push(`It is the account signing requests now: ${accountName(next)}, the oldest one left, signs from the next request.`);
  return said;
}

/// The router's sentence for an account a hosted router cannot sign with (src/accounts.rs,
/// `NOT_ON_A_SERVER`), for a card read before its credential was checked (ST-C27).
export const NOT_ON_A_SERVER =
  "this account is a Claude Code login on a developer's machine, which a hosted router cannot read — add an account with a `claude setup-token` value in the dashboard";

/// Whether an account can be made the one that signs from here: anything locally, and only a
/// pasted token on a hosted router, which refuses the rest (ST-C27).
export const usable = (a: Pick<Account, 'source'>, hosted: boolean): boolean => !hosted || a.source === 'token';

/// The explanation the "built in" marker carries as visible text (ST-C23).
export const BUILT_IN = 'Discovered from the Claude Code login on this machine, so the router does not own it.';

/// The explanation the "usage key" tag carries as visible text (ST-C23).
export const USAGE_KEY = 'A claude.ai session key is stored for this account, used only to read usage.';

/// Dots that look like a filled field, for a secret the page is never sent (ST-C17).
export const HELD = '•'.repeat(24);

// ---------- Claude Code ----------

/// A managed project record names its settings file, and the router wants the project's root:
/// `/.claude/settings.local.json` comes off, with either separator (ST-C58).
export const projectRoot = (settingsPath: string): string => String(settingsPath).replace(/[\\/]\.claude[\\/]settings\.local\.json$/, '');

/// The Claude Code page's heading: whether the login item is on, and whether every project or only
/// some are routed (ST-C64).
export function setupHeading(registered: boolean, everyProject: boolean, projects: number): string {
  const routed = everyProject ? 'every project routed' : projects > 0 ? `${projects} project${projects === 1 ? '' : 's'} routed` : 'no project routed';
  return `login item ${registered ? 'on' : 'off'} · ${routed}`;
}

/// An extension binary's state, in words (src/shim.rs, `State`).
export function wrapperState(state: string): string {
  if (state === 'installed') return 'installed';
  if (state === 'not_installed') return 'not installed';
  if (state === 'inconsistent') return 'needs reinstalling';
  return state;
}

/// What a wrapper change means for the reader, since a wrapper only reaches new sessions (ST-C63).
export const WRAPPER_SAID = {
  install: 'Installed — restart a session to use it.',
  remove: 'Removed.',
  on: 'On — new extension versions are wrapped as they arrive.',
  off: 'Off — the buttons below still work.',
} as const;

// ---------- the network ----------

/// Where the router can be reached, as the Network page says it (ST-C67).
export function networkWhere(on: boolean, lanUrl: string | null | undefined): string {
  if (on) return lanUrl ? `Reachable at ${lanUrl}` : 'Reachable from your network.';
  return `This Mac only.${lanUrl ? ` Turning it on would put it at ${lanUrl}.` : ''}`;
}

/// The router's sentence for a page a hosted router does not have (src/hosted.rs, `NO_MACHINE`),
/// for a bookmarked address that reaches one (ST-C103).
export const NO_MACHINE =
  "this router is hosted: it does not run on a developer's machine, so there is no Claude Code, login item, editor extension or network switch here to manage — set those up on each machine";

// ---------- notifications ----------

/// The Notifications heading: the key id and the phones, or that no phone can be told (ST-C85).
export function pushHeading(configured: boolean, keyId: string | null | undefined, phones: number): string {
  if (!configured) return 'no APNs key — phones cannot be told';
  return `key ${keyId ?? DASH} · ${phones} phone${phones === 1 ? '' : 's'}`;
}

/// A platform, in the reader's words.
export function platformWord(platform: string | null | undefined): string {
  if (platform === 'apns') return 'iPhone';
  if (platform === 'fcm') return 'Android';
  return platform || DASH;
}

/// A phone's compact row: its name or "phone", the tail of its token, its environment and its
/// platform; then its last result and when, or when it registered (ST-C91).
export function deviceRow(d: Device, now: number = Date.now()): { readonly who: string; readonly last: string } {
  const who = [d.name || 'phone', `…${d.token_tail}`, d.environment || DASH, platformWord(d.platform)].join(' · ');
  const last = d.last_result ? `${d.last_result} · ${ago(d.last_seen_at, now)}` : `registered ${ago(d.registered_at, now)}`;
  return { who, last };
}

/// How to register a phone, when none has (ST-C91).
export const NO_PHONES = 'No phone has registered yet — in the app, Settings → Notifications → Enable.';

// ---------- the sign-in flow ----------

export type Signin =
  | { readonly stage: 'none' }
  | { readonly stage: 'starting'; readonly url: string | null }
  | { readonly stage: 'waiting'; readonly url: string | null }
  | { readonly stage: 'done'; readonly who: string }
  | { readonly stage: 'failed'; readonly why: string };

/// What a sign-in's status says, and only that: its stage, its link, who it signed in as, and why it
/// failed (ST-C45, ST-C47). A status that names no flow — failed, with no name and no error, which is
/// what the router says before any sign-in has run, or `{}` from a hosted router — is no flow, never
/// a failure (ST-C46).
export function signin(s: LoginStatus | null | undefined): Signin {
  if (!s?.phase) return { stage: 'none' };
  switch (s.phase) {
    case 'starting':
      return { stage: 'starting', url: s.url || null };
    case 'awaiting_sign_in':
      return { stage: 'waiting', url: s.url || null };
    case 'done':
      return { stage: 'done', who: s.email || s.label || s.account_id || 'the new account' };
    case 'failed':
      if (!s.label && !s.error) return { stage: 'none' };
      return { stage: 'failed', why: s.error || 'the sign-in did not complete' };
    default:
      return { stage: 'none' };
  }
}

/// Whether a sign-in is still running, and the page keeps reading its status.
export const running = (s: Signin): boolean => s.stage === 'starting' || s.stage === 'waiting';

/// The stage in words (ST-C45).
export function signinWords(s: Signin): string {
  switch (s.stage) {
    case 'starting':
      return 'Starting the sign-in…';
    case 'waiting':
      return 'Waiting for you to sign in…';
    case 'done':
      return `Signed in as ${s.who}. The account is ready.`;
    case 'failed':
      return s.why;
    default:
      return '';
  }
}

/// How often a running sign-in's status is read, and how often a new profile is re-checked
/// (ST-C45, ST-C51).
export const SIGNIN_POLL_MS = 1500;
export const PROFILE_POLL_MS = 3000;
