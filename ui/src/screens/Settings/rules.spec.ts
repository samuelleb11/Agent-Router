// @vitest-environment node
// New for agent-router: the Settings pages' own rules, with the router's numbers and sentences
// (docs/web-console-contracts.md, "Settings").
import { describe, expect, it } from 'vitest';
import type { AccountRow } from '~/data/settings';
import {
  accountId,
  accountsHeading,
  activeWords,
  assignedWords,
  baseUrlProblem,
  credential,
  deviceRow,
  expiry,
  identityProblem,
  keyProblem,
  kimiKeyProblem,
  looksLikeEmail,
  modelChoices,
  nameProblem,
  networkWhere,
  nextSigner,
  projectRoot,
  pushHeading,
  pushKeyProblem,
  removeWords,
  running,
  servedBy,
  sessionKeyProblem,
  setupHeading,
  signin,
  signinWords,
  tokenProblem,
} from './rules';

const NOW = Date.UTC(2026, 9, 5, 12, 0, 0);

const row = (id: string, more: Partial<AccountRow> = {}): AccountRow => ({ id, label: id, source: 'token', active: 0, ...more });

describe('Settings rules', () => {
  it('ST-C42: the id a name becomes is worked out as the router works it out, so a clash is caught before anything is sent', () => {
    // src/accounts.rs, `slugify`: ASCII letters and digits, lowercased, runs of anything else one hyphen, ends trimmed.
    expect(accountId('Work')).toBe('work');
    expect(accountId('  My  Work account!! ')).toBe('my-work-account');
    expect(accountId('café crème')).toBe('caf-cr-me');
    expect(accountId('sample.person@example.com')).toBe('sample-person-example-com');
    // At most 48 characters, cut where the 48th falls, a trailing hyphen included, as the router cuts.
    expect(accountId('a'.repeat(47) + ' b')).toBe(`${'a'.repeat(47)}-`);
    expect(accountId('x'.repeat(60))).toHaveLength(48);
    // Nothing left: the router makes up a fresh id, which no account has.
    expect(accountId('!!!')).toBeNull();
    const existing = [row('work', { email: 'me@work.example' }), row('default', { source: 'keychain', label: 'default' })];
    expect(nameProblem('Work', existing)).toMatch(/“work”, which me@work\.example already has: adding it would replace that account/);
    // A new "Default" would turn the built-in account into a token account.
    expect(nameProblem('Default', existing)).toMatch(/“default”/);
    expect(nameProblem('work two', existing)).toBeNull();
    expect(nameProblem('???', existing)).toBeNull();
  });

  it('ST-C41: a name is required before anything is sent', () => {
    expect(nameProblem('', [])).toBe('Give the account a name first.');
    expect(nameProblem('   ', [])).toBe('Give the account a name first.');
  });

  it('ST-C11: the router’s own rules and sentences, checked before sending: a credential, a session key, an identity, a password and an APNs key', () => {
    // A credential starts sk-ant- and is not a claude.ai session key (src/api.rs, `accounts_edit`).
    expect(tokenProblem('sk-ant-sid01-abc')).toBe('that is a claude.ai session key — it belongs in the session-key field, and cannot sign API requests');
    expect(tokenProblem('oat01-abc')).toBe('expected a token starting sk-ant- (from `claude setup-token`)');
    expect(tokenProblem('  sk-ant-oat01-abc  ')).toBeNull();
    expect(tokenProblem(''), 'blank means keep').toBeNull();
    // A session key starts sk-ant-sid (src/quota.rs).
    expect(sessionKeyProblem('sk-ant-oat01-abc')).toBe('that does not look like a claude.ai session key (expected sk-ant-sid…)');
    expect(sessionKeyProblem('sk-ant-sid01-abc')).toBeNull();
    expect(sessionKeyProblem(''), 'blank means keep').toBeNull();
    // An identity looks like an email address (src/api.rs, `looks_like_email`).
    expect(looksLikeEmail('me@example.com')).toBe(true);
    for (const no of ['me', '@example.com', 'me@example', 'me@.example', 'me @example.com']) expect(looksLikeEmail(no), no).toBe(false);
    expect(identityProblem('bob')).toBe('"bob" is not an email address');
    expect(identityProblem('bob@example.com')).toBeNull();
    // The three parts of an APNs key together (src/notify.rs).
    expect(pushKeyProblem('ABCDE12345', 'ABC123DEF4', '')).toBe('team id, key id and the .p8 key are all needed');
    expect(pushKeyProblem('ABCDE12345', ' ', '-----BEGIN')).toBe('team id, key id and the .p8 key are all needed');
    expect(pushKeyProblem('ABCDE12345', 'ABC123DEF4', '-----BEGIN')).toBeNull();
  });

  it('ST-C118: each provider’s key is checked by its own rule in the router’s words, the sk-ant- rule being Anthropic’s alone, and a base URL must be https', () => {
    // src/accounts.rs, `Provider::key_problem`.
    expect(kimiKeyProblem('sk-ant-oat01-abc')).toBe('that is an Anthropic token — a Kimi Code account takes a key from the Kimi Code console');
    expect(kimiKeyProblem('sk-kimi-0123')).toBeNull();
    expect(kimiKeyProblem(''), 'blank is the add form’s to ask for, and the editor’s keep').toBeNull();
    expect(keyProblem('kimi', 'oat01-abc'), 'a Kimi Code key is not held to sk-ant-').toBeNull();
    expect(keyProblem('anthropic', 'sk-kimi-0123')).toBe('expected a token starting sk-ant- (from `claude setup-token`)');
    expect(keyProblem('kimi', 'sk-ant-sid01-abc')).toBe('that is an Anthropic token — a Kimi Code account takes a key from the Kimi Code console');
    // src/api.rs, `base_url_for`: a trailing slash is dropped, and a URL that is not https is named.
    expect(baseUrlProblem('')).toBeNull();
    expect(baseUrlProblem('https://api.kimi.com/coding/')).toBeNull();
    expect(baseUrlProblem('http://api.kimi.ai/coding')).toBe('"http://api.kimi.ai/coding" is not an https:// address');
    expect(baseUrlProblem('api.kimi.ai/coding/')).toBe('"api.kimi.ai/coding" is not an https:// address');
  });

  it('ST-C121: each provider has its own active account: the heading names each, and removing one names the oldest of its own provider as the next', () => {
    const accounts = [
      row('kimi-old', { provider: 'kimi', created_at: 5 }),
      row('kimi', { provider: 'kimi', active: 1, created_at: 15, label: 'Kimi' }),
      row('work', { active: 1, created_at: 10, email: 'me@example.com' }),
      row('spare', { created_at: 1 }),
    ];
    expect(accountsHeading(accounts)).toBe('4 accounts · me@example.com active · Kimi active for Kimi Code');
    expect(nextSigner(accounts, 'kimi')?.id, 'the oldest Kimi Code account, not the oldest of all').toBe('kimi-old');
    expect(nextSigner(accounts, 'work')?.id).toBe('spare');
    expect(nextSigner([row('kimi', { provider: 'kimi', active: 1 }), row('work', { active: 1 })], 'kimi'), 'no other Kimi Code account').toBeNull();
    expect(removeWords(accounts, 'kimi')).toEqual([
      'Remove Kimi from the router? Its stored key is deleted with it.',
      'It is the account signing Kimi Code’s requests now: kimi-old, the oldest one left, signs from the next request.',
    ]);
    expect(activeWords({ provider: 'kimi' })).toBe('Signs every Kimi Code request now, but for a model assigned another account.');
    expect(activeWords({})).toBe('Signs every Claude request now, but for a model assigned another account.');
  });

  it('ST-C122, ST-C123: a model’s list box offers its provider’s active account first, then its own provider’s accounts that can sign here, and says who serves it', () => {
    const accounts = [
      row('default', { source: 'keychain', active: 1, email: 'me@example.com' }),
      row('work'),
      row('kimi', { provider: 'kimi', active: 1, label: 'Kimi' }),
      row('kimi-2', { provider: 'kimi', label: 'Kimi two' }),
    ];
    const k3 = { id: 'k3', provider: 'kimi', display_name: 'Kimi K3', assigned: null, serving: 'kimi' } as const;
    expect(modelChoices(k3, accounts, false)).toEqual([
      { id: null, label: 'Kimi Code’s active account (Kimi)' },
      { id: 'kimi', label: 'Kimi' },
      { id: 'kimi-2', label: 'Kimi two' },
    ]);
    const opus = { id: 'claude-opus-5-5', provider: 'anthropic', display_name: 'claude-opus-5-5', assigned: 'default', serving: 'default' } as const;
    // A hosted router signs with a pasted key only, but the account a model is assigned is always shown.
    expect(modelChoices(opus, accounts, true).map((c) => c.id)).toEqual([null, 'default', 'work']);
    expect(modelChoices({ ...opus, assigned: null }, accounts, true).map((c) => c.id)).toEqual([null, 'work']);
    expect(servedBy(k3, accounts)).toBe('Kimi, Kimi Code’s active account');
    expect(servedBy(opus, accounts)).toBe('me@example.com, assigned to it');
    expect(servedBy({ ...k3, serving: null }, [])).toBe('nothing: Kimi Code has no active account');
    expect(assignedWords(k3, { id: 'kimi-2', label: 'Kimi two' })).toBe('Kimi K3 goes to Kimi two now.');
    expect(assignedWords(k3, { id: null, label: 'Kimi Code’s active account (Kimi)' })).toBe('Kimi K3 goes to Kimi Code’s active account again.');
  });

  it('ST-C20: a credential not checked reads "not checked", a ready one its kind, a failed one the router’s whole reason', () => {
    const long = `the profile is not logged in: ${'run the login again, '.repeat(8)}then come back`;
    expect(credential({})).toEqual({ kind: 'unchecked', text: 'not checked' });
    expect(credential({ ready: true, detail: 'sk-ant-oat01 (subscription OAuth)' })).toEqual({ kind: 'ready', text: 'sk-ant-oat01 (subscription OAuth)' });
    expect(credential({ ready: 0, detail: long })).toEqual({ kind: 'failed', text: long });
    expect(long.length).toBeGreaterThan(70);
  });

  it('ST-C21: expiry is "no expiry", a date, or "—" when it is not known', () => {
    expect(expiry({ ready: true, expires_at: 0 })).toBe('no expiry');
    expect(expiry({ ready: true, expires_at: NOW + 3_600_000 })).not.toMatch(/no expiry|—/);
    expect(expiry({})).toBe('—');
    expect(expiry({ ready: false, expires_at: 0 }), 'a credential that did not resolve has no known expiry').toBe('—');
  });

  it('ST-C22: the heading counts the accounts and names the active one, or says none is', () => {
    expect(accountsHeading([row('a', { email: 'a@example.com', active: 1 }), row('b')])).toBe('2 accounts · a@example.com active');
    expect(accountsHeading([row('solo')])).toBe('1 account · none active');
  });

  it('ST-C28: removing the active account names the one that signs next — the oldest one left', () => {
    const accounts = [row('newer', { created_at: 30 }), row('active', { active: 1, created_at: 20, email: 'me@example.com' }), row('oldest', { created_at: 10 })];
    expect(nextSigner(accounts, 'active')?.id).toBe('oldest');
    expect(nextSigner(accounts, 'newer'), 'removing one that does not sign changes nothing').toBeUndefined();
    expect(nextSigner([row('only', { active: 1 })], 'only')).toBeNull();
    const said = removeWords(accounts, 'active').join(' ');
    expect(said).toMatch(/^Remove me@example\.com from the router\? Its stored credential and its claude\.ai session key are deleted with it\./);
    expect(said).toMatch(/oldest, the oldest one left, signs from the next request/);
    expect(removeWords(accounts, 'newer')).toHaveLength(1);
  });

  it('ST-C58: Stop routing sends the project’s root, the settings file taken off with either separator', () => {
    expect(projectRoot('/Users/me/code/app/.claude/settings.local.json')).toBe('/Users/me/code/app');
    expect(projectRoot('C:\\Users\\me\\app\\.claude\\settings.local.json')).toBe('C:\\Users\\me\\app');
    expect(projectRoot('C:\\Users\\me\\app/.claude\\settings.local.json')).toBe('C:\\Users\\me\\app');
  });

  it('ST-C64: the Claude Code heading says whether the login item is on and whether every project or only some are routed', () => {
    expect(setupHeading(true, true, 0)).toBe('login item on · every project routed');
    expect(setupHeading(false, false, 2)).toBe('login item off · 2 projects routed');
    expect(setupHeading(false, false, 0)).toBe('login item off · no project routed');
  });

  it('ST-C67: on, the address other devices reach; off, "This Mac only" and where turning it on would put it', () => {
    expect(networkWhere(true, 'http://10.0.0.5:8787')).toBe('Reachable at http://10.0.0.5:8787');
    expect(networkWhere(false, 'http://10.0.0.5:8787')).toBe('This Mac only. Turning it on would put it at http://10.0.0.5:8787.');
    expect(networkWhere(false, null)).toBe('This Mac only.');
  });

  it('ST-C85: the Notifications heading names the key id and counts the phones, or says no phone can be told', () => {
    expect(pushHeading(true, 'ABC123DEF4', 2)).toBe('key ABC123DEF4 · 2 phones');
    expect(pushHeading(true, 'ABC123DEF4', 1)).toBe('key ABC123DEF4 · 1 phone');
    expect(pushHeading(false, null, 3)).toBe('no APNs key — phones cannot be told');
  });

  it('ST-C91: a phone is its name or "phone", its token’s tail, its environment and platform, then its last result and when, or when it registered', () => {
    expect(deviceRow({ token_tail: 'a1b2c3', name: 'Pocket', environment: 'production', platform: 'apns', last_result: 'delivered', last_seen_at: NOW - 120_000 }, NOW)).toEqual({
      who: 'Pocket · …a1b2c3 · production · iPhone',
      last: 'delivered · 2 min ago',
    });
    expect(deviceRow({ token_tail: 'd4e5f6', name: null, environment: 'development', platform: 'fcm', registered_at: NOW - 3 * 3_600_000 }, NOW)).toEqual({
      who: 'phone · …d4e5f6 · development · Android',
      last: 'registered 3 h ago',
    });
  });

  it('ST-C45, ST-C46: a sign-in’s stage is read from its status, and a status that names no flow is no flow, never a failure', () => {
    expect(signin({ phase: 'starting' })).toEqual({ stage: 'starting', url: null });
    expect(signin({ phase: 'awaiting_sign_in', url: 'https://claude.ai/oauth/x' })).toEqual({ stage: 'waiting', url: 'https://claude.ai/oauth/x' });
    expect(signin({ phase: 'done', label: 'work', email: 'me@example.com' })).toEqual({ stage: 'done', who: 'me@example.com' });
    expect(signin({ phase: 'failed', label: 'work', error: 'the browser was closed' })).toEqual({ stage: 'failed', why: 'the browser was closed' });
    // What the router answers before any sign-in has run, and what a hosted router answers.
    expect(signin({ phase: 'failed', label: '', error: null })).toEqual({ stage: 'none' });
    expect(signin({})).toEqual({ stage: 'none' });
    expect([running(signin({ phase: 'starting' })), running(signin({ phase: 'awaiting_sign_in' })), running(signin({ phase: 'done', label: 'w' }))]).toEqual([true, true, false]);
    expect(signinWords(signin({ phase: 'awaiting_sign_in' }))).toBe('Waiting for you to sign in…');
    expect(signinWords(signin({ phase: 'done', label: 'work' }))).toBe('Signed in as work. The account is ready.');
  });

  it('ST-C47: of a sign-in’s status only the link, the stage, the account and the error are kept; the terminal’s tail is not', () => {
    const tail = 'sk-ant-oat01-SECRET… printed on the terminal';
    const status = { phase: 'awaiting_sign_in', url: 'https://claude.ai/oauth/x', label: 'work', tail } as Parameters<typeof signin>[0];
    expect(JSON.stringify(signin(status))).not.toContain(tail);
    expect(JSON.stringify(signin({ ...status, phase: 'failed', error: 'no' }))).not.toContain(tail);
  });
});
