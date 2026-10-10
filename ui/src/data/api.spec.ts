// New for agent-router: the router's refusals, in every shape it answers them, reach the Settings
// pages as failures carrying the router's words, through the one fetch layer (~/data/http) the
// writers in ./settings send through. None of them reads as success.
import { afterEach, describe, expect, it, vi } from 'vitest';
import { Refused, refusalText } from '~/lib/errors';
import { json, refusal, stubFetch } from '~/test-utils';
import {
  activateAccount,
  addAccount,
  clearPushKey,
  editAccount,
  readAccounts,
  readLogin,
  removeAccount,
  savePushKey,
  sendTest,
  setNetwork,
  setPassword,
  setRouting,
  setService,
  setSessionKey,
  setWrapper,
  setWrapperAuto,
  startSignin,
} from './settings';

afterEach(() => vi.unstubAllGlobals());

/// The class a call failed with and the words it carried; fails the test if the call succeeded.
async function failure(p: Promise<unknown>): Promise<{ error: unknown; words: string }> {
  try {
    await p;
  } catch (error) {
    return { error, words: refusalText(error) };
  }
  throw new Error('expected the call to fail');
}

const NO_MACHINE =
  "this router is hosted: it does not run on a developer's machine, so there is no Claude Code, login item, editor extension or network switch here to manage — set those up on each machine";

describe('the Settings writers, through the fetch layer', () => {
  it('a 200 carrying error is a failure in the router’s words: a refused password never reads "set"', async () => {
    stubFetch({ '/api/auth': () => refusal('use at least 8 characters') });
    const { error, words } = await failure(setPassword('😀😀😀😀'));
    expect(error).toBeInstanceOf(Refused);
    expect((error as Refused).status).toBe(200);
    expect(words).toBe('use at least 8 characters');

    stubFetch({ '/api/push/config': () => refusal('that key does not sign: invalid PEM') });
    expect((await failure(savePushKey({ team_id: 'T', key_id: 'K', p8: 'nope', bundle_id: '' }))).words).toBe('that key does not sign: invalid PEM');

    stubFetch({ '/api/setup/config': () => refusal('/nowhere is not a directory') });
    expect((await failure(setRouting(true, '/nowhere'))).words).toBe('/nowhere is not a directory');

    stubFetch({ '/api/usage-token': () => refusal('that does not look like a claude.ai session key (expected sk-ant-sid…)') });
    expect((await failure(setSessionKey('work', 'sk-ant-oat01-x'))).words).toBe('that does not look like a claude.ai session key (expected sk-ant-sid…)');

    stubFetch({ '/api/login/start': () => refusal('a sign-in is already in progress') });
    expect((await failure(startSignin('work'))).words).toBe('a sign-in is already in progress');

    stubFetch({ '/api/setup/service': () => refusal('launchctl bootstrap failed: 5') });
    expect((await failure(setService(true))).words).toBe('launchctl bootstrap failed: 5');
  });

  it('a 200 carrying ok:false is a failure, in the reason the wrapper switches give', async () => {
    stubFetch({ '/api/setup/shim/auto': () => json({ ok: false, reason: 'the wrapper binary is missing next to agent-router' }) });
    const auto = await failure(setWrapperAuto(true));
    expect(auto.error).toBeInstanceOf(Refused);
    expect((auto.error as Refused).status).toBe(200);
    expect(auto.words).toBe('the wrapper binary is missing next to agent-router');

    stubFetch({ '/api/setup/shim': () => json({ ok: false, reason: 'Not a known extension binary.' }) });
    expect((await failure(setWrapper('/elsewhere/claude', 'install'))).words).toBe('Not a known extension binary.');
  });

  it('a 409 carrying {ok:false, error} — a hosted router’s refusals — is a failure in its sentence', async () => {
    stubFetch({ '/api/network': () => json({ ok: false, error: NO_MACHINE }, 409) });
    const net = await failure(setNetwork(true));
    expect((net.error as Refused).status).toBe(409);
    expect(net.words).toBe(NO_MACHINE);

    const NO_PROFILE = "a profile account is a Claude Code login directory on the router's machine, which a hosted router does not have — run `claude setup-token` on your machine and paste the token";
    stubFetch({ '/api/accounts': () => json({ ok: false, error: NO_PROFILE }, 409) });
    expect((await failure(addAccount('side', { source: 'profile' }))).words).toBe(NO_PROFILE);

    const PINNED = 'the password is set by the deployment (AGENT_ROUTER_PASSWORD); change the Secret and restart the router';
    stubFetch({ '/api/auth': () => json({ ok: false, error: PINNED }, 409) });
    expect((await failure(setPassword('a long enough password'))).words).toBe(PINNED);
  });

  it('a 4xx or 5xx with an error body is a failure in the router’s words', async () => {
    stubFetch({ '/api/accounts/remove': () => refusal('the default account is discovered automatically and cannot be removed', 400) });
    const removed = await failure(removeAccount('default'));
    expect((removed.error as Refused).status).toBe(400);
    expect(removed.words).toBe('the default account is discovered automatically and cannot be removed');

    stubFetch({ '/api/accounts/activate': () => refusal('no account with id "gone"', 500) });
    expect((await failure(activateAccount('gone'))).words).toBe('no account with id "gone"');

    stubFetch({ '/api/accounts/edit': () => refusal('"bob" is not an email address', 422) });
    expect((await failure(editAccount('work', { email: 'bob' }))).words).toBe('"bob" is not an email address');

    stubFetch({ '/api/push/config': () => refusal('team id, key id and the .p8 key are all needed', 400) });
    expect((await failure(clearPushKey())).words).toBe('team id, key id and the .p8 key are all needed');
  });

  it('a test push nobody received is the phones’ answer, not a refusal with no words; one the router refused keeps its words', async () => {
    stubFetch({ '/api/push/test': () => json({ ok: true, results: [{ device: 'a1b2c3', ok: true, error: null }] }) });
    await expect(sendTest()).resolves.toEqual({ results: [{ device: 'a1b2c3', ok: true, error: null }] });

    stubFetch({ '/api/push/test': () => json({ ok: false, results: [{ device: 'a1b2c3', ok: false, error: 'BadDeviceToken' }] }) });
    await expect(sendTest()).resolves.toEqual({ undelivered: true });

    stubFetch({ '/api/push/test': () => refusal('no push credentials are set') });
    expect((await failure(sendTest())).words).toBe('no push credentials are set');
    stubFetch({ '/api/push/test': () => refusal('no push credentials are set', 409) });
    expect((await failure(sendTest())).words).toBe('no push credentials are set');
  });

  it('a failed sign-in’s status is the flow’s state, not a failed read; a refused read stays a failure', async () => {
    stubFetch({ '/api/login/status': () => json({ phase: 'failed', label: 'work', error: 'the browser was closed', url: null, tail: 'terminal' }) });
    await expect(readLogin()).resolves.toEqual({ phase: 'failed', error: 'the browser was closed' });

    stubFetch({ '/api/login/status': () => new Response('agent-router: a password is required from another machine', { status: 401 }) });
    expect((await failure(readLogin())).words).toBe('agent-router: a password is required from another machine');
  });

  it('asks for every credential to be checked only when told to', async () => {
    const fetch = stubFetch({ '/api/accounts': () => json([]) });
    await readAccounts(false);
    await readAccounts(true);
    expect(fetch.mock.calls.map(([input]) => String(input))).toEqual(['/api/accounts', '/api/accounts?probe=1']);
  });
});
