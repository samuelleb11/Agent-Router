// @vitest-environment node
// New for agent-router: what Settings decides from /api/auth, beyond what the old checks pinned
// (docs/web-console-contracts.md, "Settings").
import { describe, expect, it } from 'vitest';
import { accountSources, ADD_PROVIDERS, byProvider, networkStatus, passwordProblem, passwordStatus, providerName, providerOf, readsSetup, settingsPages } from './settings';

const NOW = Date.UTC(2026, 9, 5, 12, 0, 0);

describe('settings', () => {
  it("ST-C73, ST-C74: a password's length is counted in characters, as the router counts it, where the page counted UTF-16 units", () => {
    // Four emoji are eight UTF-16 units and four characters, which the router refuses.
    expect(passwordProblem('😀😀😀😀', '😀😀😀😀')).toBe('use at least 8 characters');
    // Sent exactly as typed, spaces included.
    expect(passwordProblem('  spaced  ', '  spaced  ')).toBeNull();
    expect(passwordProblem('long enough', 'long enougH')).toBe('the two entries differ');
  });

  it('ST-C101, ST-C117, ST-C122, ST-C124: until /api/auth has said which kind of router this is, no machine-only control is offered, which the page offered; Models and Kimi Code, a provider asked for before Anthropic’s ways, are offered on every router', () => {
    expect(settingsPages(null)).toEqual(['accounts', 'models', 'clients', 'access-tokens', 'password', 'notifications', 'router']);
    expect([accountSources(null), readsSetup(null)]).toEqual([['token'], false]);
    // A router that predates `mode` is local, with nothing changed (ST-C100).
    expect(settingsPages({})).toEqual(['accounts', 'models', 'clients', 'claude-code', 'network', 'access-tokens', 'password', 'notifications', 'router']);
    expect(accountSources({})).toEqual(['signin', 'token', 'profile']);
    expect(settingsPages({ mode: 'server' })).toEqual(['accounts', 'models', 'clients', 'access-tokens', 'password', 'notifications', 'router', 'forges', 'library']);
    // The providers, Anthropic first and chosen as the form opens.
    expect(ADD_PROVIDERS).toEqual(['anthropic', 'kimi', 'openai']);
  });

  it('ST-C125: the accounts by provider — one group per provider that has any, Anthropic’s first, each in the router’s order; a row naming none is Anthropic’s', () => {
    const rows = [{ id: 'k1', provider: 'kimi' }, { id: 'a1' }, { id: 'k2', provider: 'kimi' }, { id: 'a2', provider: 'anthropic' }, { id: 'g1', provider: 'glm' }];
    expect(byProvider(rows).map((g) => [g.provider, g.accounts.map((a) => a.id)])).toEqual([
      ['anthropic', ['a1', 'a2']],
      ['kimi', ['k1', 'k2']],
      ['glm', ['g1']],
    ]);
    // A provider with no account has no group, Anthropic included.
    expect(byProvider([{ id: 'k1', provider: 'kimi' }]).map((g) => g.provider)).toEqual(['kimi']);
    expect(byProvider([])).toEqual([]);
  });

  it('ST-C119: an account names its provider as the router does, and one from a router that predates providers is Anthropic’s', () => {
    expect([providerName('anthropic'), providerName('kimi'), providerName(null), providerName('glm')]).toEqual(['Anthropic', 'Kimi Code', 'Anthropic', 'glm']);
    expect([providerOf({}), providerOf({ provider: 'kimi' })]).toEqual(['anthropic', 'kimi']);
  });

  it('ST-C77: the password heading says whether one is set and when, or why it matters', () => {
    expect(passwordStatus({ configured: true, updated_at: NOW - 3 * 60e3 }, NOW)).toBe('set · changed 3 min ago');
    expect(passwordStatus({ configured: false, remote_bind: true }, NOW)).toBe('not set — other devices are refused');
    expect(passwordStatus({ configured: false, remote_bind: false }, NOW)).toBe('not set — only matters on the network');
  });

  it('SH-C18, SH-C19: the network status shows only beyond loopback, unguarded in the alarm tone, with its explanation as text', () => {
    expect(networkStatus({ remote_bind: false, configured: true })).toBeNull();
    expect(networkStatus({ remote_bind: true, configured: false })).toEqual({
      tone: 'bad',
      text: 'network · no password',
      explanation: 'Reachable from your network, but other devices are refused until a password is set',
    });
    expect(networkStatus({ remote_bind: true, configured: true })?.explanation).toBe('Reachable from your network; other devices must give the password');
  });

  it('an active access token guards the network and makes a legacy password optional', () => {
    const auth = { configured: false, remote_bind: true, access_tokens: 1 };
    expect(networkStatus(auth)).toEqual({ tone: 'ok', text: 'network · access tokens', explanation: 'Reachable from your network; other devices must present a router credential' });
    expect(passwordStatus(auth)).toBe('not set — access tokens are configured');
    expect(networkStatus({ ...auth, access_tokens: 0 })?.tone).toBe('bad');
  });
});
