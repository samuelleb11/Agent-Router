// New for agent-router: the Settings pages' own rules, with no React and no DOM, beside the
// hosted-router and password decisions in ~/model/settings. Each is the router's rule or the
// dashboard's wording (assets/index.html, `renderAccounts`, `openEdit`, `wireAddForm`, `renderSetup`,
// `netForm`, `renderPush`), with the router's own numbers and sentences (src/api.rs,
// src/accounts.rs, src/quota.rs, src/notify.rs, src/hosted.rs), so a rule is checked before a
// request rather than learnt from its refusal (docs/web-console-contracts.md, "Settings").

import { ago, DASH, when } from '~/model/format';
import type { AccountRow, Device, LoginStatus, ModelRow } from '~/data/settings';
import { providerName, providerOf, type Account, type Provider } from '~/model/settings';

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

/// The router's rule for a Kimi Code key (src/accounts.rs, `Provider::key_problem`): anything but an
/// Anthropic token, which could only be refused upstream at every request it signed. Blank is
/// nobody's problem here, as for a token (ST-C118).
export function kimiKeyProblem(key: string): string | null {
  const k = key.trim();
  if (k === '' || !k.startsWith('sk-ant-')) return null;
  return 'that is an Anthropic token — a Kimi Code account takes a key from the Kimi Code console';
}

/// A credential checked by its account's provider's rule: the `sk-ant-` rule is Anthropic's alone
/// (ST-C11, ST-C118).
export function openaiKeyProblem(key: string): string | null {
  const k = key.trim();
  if (k === '') return null;
  if (k.startsWith('sk-ant-')) return 'that is an Anthropic token — an OpenAI account takes an OpenAI API key';
  return k.startsWith('sk-') ? null : 'expected an OpenAI API key starting sk-; use the Codex login source for a ChatGPT subscription';
}

export const keyProblem = (provider: Provider, key: string): string | null => provider === 'kimi' ? kimiKeyProblem(key) : provider === 'openai' ? openaiKeyProblem(key) : tokenProblem(key);

/// What the router answers a token account added with no token (src/api.rs, `accounts_add`), a Kimi
/// Code account's key included.
export const TOKEN_NEEDED = 'a token is required for this account type';

/// The router's rule for a Kimi Code account's base URL (src/api.rs, `base_url_for`): an https
/// address with a host, a trailing slash dropped, and named as typed when it is not one. Blank means
/// Kimi Code's own (ST-C118).
export function baseUrlProblem(url: string): string | null {
  const u = url.trim().replace(/\/+$/, '');
  if (u === '') return null;
  let parsed: URL | null = null;
  try {
    parsed = new URL(u);
  } catch {
    parsed = null;
  }
  return parsed?.protocol === 'https:' && parsed.host !== '' ? null : `${JSON.stringify(u)} is not an https:// address`;
}

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
  if (source === 'codex') return 'Codex login';
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

/// The list's heading: how many, and which one signs (ST-C22). Each provider has its own active
/// account, so an Anthropic one is "active" and any other "active for" its provider, Anthropic's
/// first (ST-C121).
export function accountsHeading(accounts: readonly Pick<AccountRow, 'id' | 'label' | 'email' | 'active' | 'provider'>[]): string {
  const active = accounts.filter((a) => flag(a.active)).sort((a, b) => Number(providerOf(a) !== 'anthropic') - Number(providerOf(b) !== 'anthropic'));
  const said = active.map((a) => (providerOf(a) === 'anthropic' ? `${accountName(a)} active` : `${accountName(a)} active for ${providerName(providerOf(a))}`));
  const n = accounts.length;
  return `${n} account${n === 1 ? '' : 's'} · ${said.length ? said.join(' · ') : 'none active'}`;
}

/// The account that signs once `id` is removed: the oldest one of its provider left when `id` is
/// that provider's active one, because the router never leaves a provider with nothing to sign
/// with (src/accounts.rs, `remove`; ST-C121). Undefined when `id` is not active, and null when
/// nothing of its provider would be left.
export function nextSigner<T extends Pick<AccountRow, 'id' | 'active' | 'created_at' | 'provider'>>(accounts: readonly T[], id: string): T | null | undefined {
  const gone = accounts.find((a) => a.id === id);
  if (!gone || !flag(gone.active)) return undefined;
  const left = accounts.filter((a) => a.id !== id && providerOf(a) === providerOf(gone));
  // The router's list is oldest first already; the creation time decides where it is known.
  const oldest = [...left].sort((a, b) => (a.created_at ?? 0) - (b.created_at ?? 0));
  return (left.every((a) => a.created_at != null) ? oldest[0] : left[0]) ?? null;
}

/// Remove's confirmation: which account, what goes with it, and who signs next (ST-C28). A Kimi
/// Code account has a key and no claude.ai session key, and the one that signs next is of its own
/// provider (ST-C121).
export function removeWords(accounts: readonly AccountRow[], id: string): string[] {
  const a = accounts.find((x) => x.id === id);
  const name = a ? accountName(a) : id;
  const keyed = a !== undefined && providerOf(a) !== 'anthropic';
  const said = [
    keyed ? `Remove ${name} from the router? Its stored key is deleted with it.` : `Remove ${name} from the router? Its stored credential and its claude.ai session key are deleted with it.`,
  ];
  const whose = keyed ? `signing ${providerName(providerOf(a!))}’s requests` : 'signing requests';
  const next = nextSigner(accounts, id);
  if (next === null) said.push(`It is the account ${whose} now, and no other is left to sign with.`);
  else if (next) said.push(`It is the account ${whose} now: ${accountName(next)}, the oldest one left, signs from the next request.`);
  return said;
}

/// The router's sentence for an account a hosted router cannot sign with (src/accounts.rs,
/// `NOT_ON_A_SERVER`), for a card read before its credential was checked (ST-C27).
export const NOT_ON_A_SERVER =
  "this account is a Claude Code login on a developer's machine, which a hosted router cannot read — add an account with a `claude setup-token` value in the dashboard";

/// Whether an account can be made the one that signs from here: anything locally, and only a
/// pasted token on a hosted router, which refuses the rest (ST-C27).
export const usable = (a: Pick<Account, 'source'>, hosted: boolean): boolean => !hosted || a.source === 'token';

/// One choice of a provider's "Requests go to" list box: one of that provider's accounts.
export interface ActiveChoice {
  readonly id: string;
  readonly label: string;
}

/// What a provider's "Requests go to" list box offers, from that provider's accounts (ST-C125): each
/// that can be made the one that signs here — a hosted router refuses all but a pasted key (ST-C27) —
/// and whichever is active now, so the box always shows what the router reports.
export function activeChoices(own: readonly Pick<Account, 'id' | 'label' | 'email' | 'source' | 'active'>[], hosted: boolean): ActiveChoice[] {
  return own.filter((a) => usable(a, hosted) || flag(a.active)).map((a) => ({ id: a.id, label: accountName(a) }));
}

/// What an active card says it does. Each provider has its own active account, which signs that
/// provider's requests, all but a model's assigned to another account (ST-C121, ST-C123).
export function activeWords(a: Pick<Account, 'provider'>): string {
  const whose = providerOf(a) === 'anthropic' ? 'Claude' : providerName(providerOf(a));
  return `Signs every ${whose} request now, but for a model assigned another account.`;
}

/// The explanation the "built in" marker carries as visible text (ST-C23).
export const BUILT_IN = 'Discovered from the Claude Code login on this machine, so the router does not own it.';
export const CODEX_BUILT_IN = 'Discovered from the Codex login on this machine, so the router does not own it.';
export const CODEX_NOT_ON_A_SERVER = "a hosted router cannot read a machine's Codex login — add an OpenAI API key as a token account";

/// The explanation the "usage key" tag carries as visible text (ST-C23).
export const USAGE_KEY = 'A claude.ai session key is stored for this account, used only to read usage.';

/// Dots that look like a filled field, for a secret the page is never sent (ST-C17).
export const HELD = '•'.repeat(24);

// ---------- models ----------

/// One choice of a model's list box: an account of the model's provider, or null for whichever
/// account is that provider's active one.
export interface ModelChoice {
  readonly id: string | null;
  readonly label: string;
}

/// A model as the page names it: the provider's name for it, else its id.
export const modelName = (m: Pick<ModelRow, 'id' | 'display_name'>): string => m.display_name || m.id;

/// What a model's list box offers (ST-C123): its provider's active account first, naming which one
/// that is now; then each account of the model's own provider that can sign on this router — the
/// router refuses another provider's (409), and a hosted router signs with a pasted key only
/// (ST-C27) — and whatever account it is assigned now, so the box always shows what the router
/// reports.
export function modelChoices(model: Pick<ModelRow, 'provider' | 'assigned'>, accounts: readonly AccountRow[], hosted: boolean): [ModelChoice, ...ModelChoice[]] {
  const own = accounts.filter((a) => providerOf(a) === providerOf(model));
  const active = own.find((a) => flag(a.active));
  const first: ModelChoice = { id: null, label: `${providerName(providerOf(model))}’s active account${active ? ` (${accountName(active)})` : ''}` };
  const rest: ModelChoice[] = own.filter((a) => usable(a, hosted) || a.id === model.assigned).map((a) => ({ id: a.id, label: accountName(a) }));
  if (model.assigned && !rest.some((c) => c.id === model.assigned)) rest.push({ id: model.assigned, label: model.assigned });
  return [first, ...rest];
}

/// Who a model's next request goes to (ST-C122): the account it was assigned, else its provider's
/// active account, by name; or that its provider has none.
export function servedBy(model: Pick<ModelRow, 'provider' | 'assigned' | 'serving'>, accounts: readonly AccountRow[]): string {
  const provider = providerName(providerOf(model));
  if (!model.serving) return `nothing: ${provider} has no active account`;
  const a = accounts.find((x) => x.id === model.serving);
  const name = a ? accountName(a) : model.serving;
  return model.assigned ? `${name}, assigned to it` : `${name}, ${provider}’s active account`;
}

/// What an assignment did, once the router took it (ST-C123).
export function assignedWords(model: Pick<ModelRow, 'id' | 'display_name' | 'provider'>, choice: ModelChoice): string {
  return choice.id === null
    ? `${modelName(model)} goes to ${providerName(providerOf(model))}’s active account again.`
    : `${modelName(model)} goes to ${choice.label} now.`;
}

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
