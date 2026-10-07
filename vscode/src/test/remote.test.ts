// Remote mode's pure helpers, under plain Node: `npm test`. The server answers come from
// a throwaway HTTP server on a port the system picks, on loopback; nothing here reaches
// a real router.

import { test } from 'node:test';
import * as assert from 'node:assert/strict';
import * as http from 'node:http';
import type { AddressInfo } from 'node:net';
import * as remote from '../remote';

test('an address is reduced to the router\'s origin', () => {
  const ok = (input: string, url: string) => {
    const n = remote.normaliseServerUrl(input);
    assert.ok(n.ok, `${input}: ${n.ok ? '' : n.why}`);
    assert.equal(n.url, url, input);
  };
  ok('', '');
  ok('   ', '');
  ok('https://router.example.com', 'https://router.example.com');
  ok('https://router.example.com/', 'https://router.example.com');
  ok('  https://Router.Example.COM/  ', 'https://router.example.com');
  ok('router.example.com', 'https://router.example.com');
  ok('router.example.com:8443', 'https://router.example.com:8443');
  ok('https://router.example.com:443', 'https://router.example.com');
  ok('HTTPS://router.example.com', 'https://router.example.com');
  ok('http://192.0.2.10:8787', 'http://192.0.2.10:8787');
  ok('http://[::1]:8787/', 'http://[::1]:8787');
});

test('plain http is accepted with a warning, https without one', () => {
  const plain = remote.normaliseServerUrl('http://192.0.2.10:8787');
  assert.ok(plain.ok && plain.note && /unencrypted/.test(plain.note));
  const tls = remote.normaliseServerUrl('https://router.example.com');
  assert.ok(tls.ok && tls.note === undefined);
});

test('what is not a router address is refused, with a reason', () => {
  const refused = (input: string, why: RegExp) => {
    const n = remote.normaliseServerUrl(input);
    assert.ok(!n.ok, `${input} should be refused`);
    assert.match(n.why, why, input);
  };
  refused('ftp://router.example.com', /https:\/\//);
  refused('ws://router.example.com', /https:\/\//);
  refused('https://me:hunter22@router.example.com', /password/);
  refused('me@router.example.com', /password/);
  refused('https://router.example.com/agent-router', /path/);
  refused('https://router.example.com/?x=1', /query/);
  refused('https://router.example.com/#live', /fragment/);
  refused('https://', /not an address/);
  refused('https://router example.com', /not an address/);
});

test('the mode follows agentRouter.server', () => {
  assert.deepEqual(remote.decideMode(undefined), { kind: 'local' });
  assert.deepEqual(remote.decideMode(null), { kind: 'local' });
  assert.deepEqual(remote.decideMode(''), { kind: 'local' });
  assert.deepEqual(remote.decideMode('  '), { kind: 'local' });
  assert.deepEqual(remote.decideMode('https://Router.example.com/'), { kind: 'remote', server: 'https://router.example.com' });
  assert.deepEqual(remote.decideMode('router.example.com'), { kind: 'remote', server: 'https://router.example.com' });
  const bad = remote.decideMode('https://router.example.com/sub');
  assert.equal(bad.kind, 'invalid');
  assert.equal(remote.decideMode(8787).kind, 'invalid');
});

test('a set-up the install script made is taken over from local mode only, and only whole', () => {
  const saved = (o: unknown) => JSON.stringify(o, null, 2);
  const file = saved({ server: 'https://Router.example.com/', password: 'hunter22', machine: 'c0ffee', label: null });
  const taken = { server: 'https://router.example.com', password: 'hunter22' };
  assert.deepEqual(remote.adoptable(file, undefined), taken);
  assert.deepEqual(remote.adoptable(file, null), taken);
  assert.deepEqual(remote.adoptable(file, ''), taken);
  assert.deepEqual(remote.adoptable(file, '  '), taken);
  assert.deepEqual(remote.adoptable(saved({ server: 'http://192.0.2.10:8787', password: ' pass:with:colons ' }), undefined),
    { server: 'http://192.0.2.10:8787', password: ' pass:with:colons ' }, 'the password as the agent keeps it');

  // A setting that names a server, or is not usable, is the person's.
  assert.equal(remote.adoptable(file, 'https://router.example.com'), undefined);
  assert.equal(remote.adoptable(file, 'https://other.example.com'), undefined);
  assert.equal(remote.adoptable(file, 'https://router.example.com/sub'), undefined);
  assert.equal(remote.adoptable(file, 8787), undefined);

  // No file, or not the agent's.
  assert.equal(remote.adoptable(undefined, undefined), undefined);
  assert.equal(remote.adoptable('', undefined), undefined);
  assert.equal(remote.adoptable('{"server": "https://router.example.com", "password": ', undefined), undefined);
  assert.equal(remote.adoptable('null', undefined), undefined);
  assert.equal(remote.adoptable('"https://router.example.com"', undefined), undefined);
  assert.equal(remote.adoptable('[]', undefined), undefined);

  // After `agent uninstall` only the machine's id is left.
  assert.equal(remote.adoptable(saved({ server: null, password: null, machine: 'c0ffee', label: null }), undefined), undefined);
  assert.equal(remote.adoptable(saved({ machine: 'c0ffee' }), undefined), undefined);

  // Half a set-up, or not text where text belongs.
  assert.equal(remote.adoptable(saved({ server: 'https://router.example.com' }), undefined), undefined);
  assert.equal(remote.adoptable(saved({ password: 'hunter22' }), undefined), undefined);
  assert.equal(remote.adoptable(saved({ server: 'https://router.example.com', password: '' }), undefined), undefined);
  assert.equal(remote.adoptable(saved({ server: 'https://router.example.com', password: ' \n' }), undefined), undefined);
  assert.equal(remote.adoptable(saved({ server: 'https://router.example.com', password: 22 }), undefined), undefined);
  assert.equal(remote.adoptable(saved({ server: 42, password: 'hunter22' }), undefined), undefined);

  // An address the extension could not use either.
  assert.equal(remote.adoptable(saved({ server: '', password: 'hunter22' }), undefined), undefined);
  assert.equal(remote.adoptable(saved({ server: '  ', password: 'hunter22' }), undefined), undefined);
  assert.equal(remote.adoptable(saved({ server: 'https://router.example.com/agent', password: 'hunter22' }), undefined), undefined);
  assert.equal(remote.adoptable(saved({ server: 'ftp://router.example.com', password: 'hunter22' }), undefined), undefined);
  assert.equal(remote.adoptable(saved({ server: 'https://me:pw@router.example.com', password: 'hunter22' }), undefined), undefined);
});

test('the host names a server in messages', () => {
  assert.equal(remote.hostOf('https://router.example.com'), 'router.example.com');
  assert.equal(remote.hostOf('http://192.0.2.10:8787'), '192.0.2.10:8787');
});

test('local mode is not offered on Windows', () => {
  assert.equal(remote.localModeAvailable('darwin'), true);
  assert.equal(remote.localModeAvailable('linux'), true);
  assert.equal(remote.localModeAvailable('win32'), false);
});

test('a stored password is only ever given back for the server it was checked against', async () => {
  const raw = remote.encodeStored('https://a.example.com', 'hunter22');
  assert.equal(remote.decodeStored(raw, 'https://a.example.com'), 'hunter22');
  assert.equal(remote.decodeStored(raw, 'https://b.example.com'), undefined);
  assert.equal(remote.decodeStored('hunter22', 'https://a.example.com'), undefined);
  assert.equal(remote.decodeStored(undefined, 'https://a.example.com'), undefined);
  assert.equal(remote.decodeStored(JSON.stringify({ server: 'https://a.example.com', password: '' }), 'https://a.example.com'), undefined);

  const map = new Map<string, string>();
  const secrets: remote.Secrets = {
    get: async (k) => map.get(k),
    store: async (k, v) => { map.set(k, v); },
    delete: async (k) => { map.delete(k); },
  };
  await remote.savePassword(secrets, 'https://a.example.com', 'pass:with:colons');
  assert.ok(map.has(remote.PASSWORD_KEY));
  assert.equal(await remote.readPassword(secrets, 'https://a.example.com'), 'pass:with:colons');
  assert.equal(await remote.readPassword(secrets, 'https://b.example.com'), undefined);
  await remote.forgetPassword(secrets);
  assert.equal(await remote.readPassword(secrets, 'https://a.example.com'), undefined);
});

test('Basic auth carries the password whole, colons and all', () => {
  const header = remote.basicAuth('pässword:with:colons');
  assert.match(header, /^Basic /);
  const decoded = Buffer.from(header.slice('Basic '.length), 'base64').toString('utf8');
  const colon = decoded.indexOf(':');
  assert.equal(decoded.slice(colon + 1), 'pässword:with:colons');
});

test('the agent\'s command lines', () => {
  assert.deepEqual(remote.agentInstallArgs('https://r.example.com', { service: true, config: true }),
    ['agent', 'install', '--server', 'https://r.example.com']);
  assert.deepEqual(remote.agentInstallArgs('https://r.example.com', { service: false, config: false }),
    ['agent', 'install', '--server', 'https://r.example.com', '--no-service', '--no-config']);
  assert.deepEqual([...remote.AGENT_ENSURE_ARGS], ['agent', '--ensure']);
  assert.deepEqual([...remote.AGENT_UNINSTALL_ARGS], ['agent', 'uninstall']);
  assert.equal(remote.exeName('agent-router', 'win32'), 'agent-router.exe');
  assert.equal(remote.exeName('claude-wrapper', 'win32'), 'claude-wrapper.exe');
  assert.equal(remote.exeName('agent-router', 'darwin'), 'agent-router');
  assert.equal(remote.exeName('agent-router', 'linux'), 'agent-router');
});

test('only the installing child is given the password', () => {
  const base = { PATH: '/bin', AGENT_ROUTER_PASSWORD: 'inherited', HOME: '/elsewhere' };
  const ensure = remote.agentEnv(base, '/Users/me', 'darwin');
  assert.equal(ensure.AGENT_ROUTER_PASSWORD, undefined);
  assert.equal(ensure.HOME, '/Users/me');
  assert.equal(ensure.PATH, '/bin');
  const install = remote.agentEnv(base, '/Users/me', 'darwin', 'hunter22');
  assert.equal(install.AGENT_ROUTER_PASSWORD, 'hunter22');
  assert.equal(base.AGENT_ROUTER_PASSWORD, 'inherited', 'the base environment is not changed');

  const win = remote.agentEnv({ Path: 'C:\\Windows', agent_router_password: 'inherited' }, 'C:\\Users\\me', 'win32');
  assert.equal(win.agent_router_password, undefined);
  assert.equal(win.HOME, undefined, 'HOME is left to the agent on Windows');
});

test('a failed agent command\'s report, folded onto one line', () => {
  // `fn main() -> anyhow::Result<()>` prints the report this way.
  assert.equal(remote.agentSentence('Error: the router refused the password\n'), 'the router refused the password');
  assert.equal(
    remote.agentSentence('Error: reaching https://r.example.com\n\nCaused by:\n    0: client error (Connect)\n    1: tcp connect error: Connection refused (os error 61)\n'),
    'reaching https://r.example.com: client error (Connect): tcp connect error: Connection refused (os error 61)');
  assert.equal(
    remote.agentSentence('Error: that does not look like Agent Router\n\nCaused by:\n    expected value at line 1 column 1\n\nStack backtrace:\n   0: x\n'),
    'that does not look like Agent Router: expected value at line 1 column 1');
  // An older build, or a usage error: the sentence, then the usage text.
  assert.equal(remote.agentSentence('unknown command "agent"\n\nUSAGE:\n  agent-router ...\n'), 'unknown command "agent"');
  assert.equal(remote.agentSentence(''), '');
});

test('only a hosted router can be this machine\'s server', () => {
  assert.equal(remote.notHosted({ configured: true, mode: 'server' }, 'r.example.com'), undefined);
  assert.match(remote.notHosted({ configured: true, mode: 'local' }, '192.0.2.10:8787') ?? '', /not a hosted router: it runs in local mode/);
  assert.match(remote.notHosted({ configured: true }, 'r.example.com') ?? '', /too old/);
});

test('the agent\'s sentence, and masking', () => {
  assert.equal(remote.firstParagraph('the server refused the password\n'), 'the server refused the password');
  assert.equal(remote.firstParagraph('unknown command "agent"\n\nUSAGE:\n  agent-router ...'), 'unknown command "agent"');
  assert.equal(remote.firstParagraph('one sentence\r\nwrapped over two lines\r\n'), 'one sentence wrapped over two lines');
  assert.equal(remote.redact('token hunter22 and hunter22', 'hunter22'), 'token •••••• and ••••••');
  assert.equal(remote.redact('nothing to hide', undefined), 'nothing to hide');
  assert.equal(remote.redact('nothing to hide', ''), 'nothing to hide');
});

// ---------------------------------------------------------------------------------------
// Answers from a server
// ---------------------------------------------------------------------------------------

interface Seen { path: string; authorization?: string }

async function serve(handler: (req: http.IncomingMessage, res: http.ServerResponse) => void): Promise<{ url: string; seen: Seen[]; close: () => Promise<void> }> {
  const seen: Seen[] = [];
  const server = http.createServer((req, res) => {
    seen.push({ path: req.url ?? '', authorization: req.headers.authorization });
    handler(req, res);
  });
  await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve));
  const { port } = server.address() as AddressInfo;
  return {
    url: `http://127.0.0.1:${port}`,
    seen,
    close: () => new Promise<void>((resolve) => { server.closeAllConnections(); server.close(() => resolve()); }),
  };
}

const json = (res: http.ServerResponse, status: number, body: unknown) => {
  res.writeHead(status, { 'content-type': 'application/json' });
  res.end(JSON.stringify(body));
};

test('the connect check: the right password, a wrong one, and a refusal', async () => {
  const s = await serve((req, res) => {
    const pw = remote.basicAuth('hunter22');
    if (req.url !== '/api/auth') { res.writeHead(404); res.end(); return; }
    if (req.headers.authorization === pw) { json(res, 200, { configured: true, mode: 'server', password_pinned: true }); return; }
    if (req.headers.authorization === remote.basicAuth('forbidden')) {
      res.writeHead(403); res.end('agent-router: Host header is not an address we serve\n'); return;
    }
    res.writeHead(401, { 'www-authenticate': 'Basic realm="agent-router"' });
    res.end('agent-router: password required\n');
  });
  try {
    const ok = await remote.checkPassword(s.url, 'hunter22');
    assert.equal(ok.kind, 'ok');
    assert.equal(ok.kind === 'ok' && ok.body.mode, 'server');
    assert.equal(s.seen[0].authorization, remote.basicAuth('hunter22'));

    const wrong = await remote.checkPassword(s.url, 'nope');
    assert.deepEqual(wrong, { kind: 'rejected', status: 401, detail: 'password required' });

    const refused = await remote.checkPassword(s.url, 'forbidden');
    assert.equal(refused.kind, 'rejected');
    assert.equal(refused.kind === 'rejected' && refused.status, 403);
    assert.equal(refused.kind === 'rejected' && refused.detail, 'Host header is not an address we serve');
  } finally {
    await s.close();
  }
});

test('a 200 that is not the router\'s answer is not taken as a yes', async () => {
  const s = await serve((req, res) => {
    if (req.url === '/api/auth') { res.writeHead(200, { 'content-type': 'text/html' }); res.end('<html>Sign in to the Wi-Fi</html>'); return; }
    json(res, 200, { hello: 'world' });
  });
  try {
    const html = await remote.checkPassword(s.url, 'hunter22');
    assert.deepEqual(html, { kind: 'unexpected', status: 200, detail: 'the answer was not JSON' });
    const other = await remote.getJson(`${s.url}/elsewhere`, { timeoutMs: 2000 });
    assert.equal(other.kind, 'ok', 'getJson itself takes any JSON');
  } finally {
    await s.close();
  }
});

test('quota with the password, health without it', async () => {
  const s = await serve((req, res) => {
    if (req.url === '/api/quota') {
      if (req.headers.authorization !== remote.basicAuth('hunter22')) { res.writeHead(401); res.end(); return; }
      json(res, 200, { five_hour: { utilization: 12.4 }, seven_day: { utilization: 40 }, severity: 'ok' });
      return;
    }
    if (req.url === '/_router/health') { json(res, 200, { ok: true, pid: 1, version: '0.1.0', build: 'tree:abc' }); return; }
    res.writeHead(404); res.end();
  });
  try {
    const q = await remote.fetchQuota(s.url, 'hunter22');
    assert.equal(q.kind, 'ok');
    assert.equal(q.kind === 'ok' && q.body.five_hour?.utilization, 12.4);
    const rejected = await remote.fetchQuota(s.url, 'old-password');
    assert.equal(rejected.kind, 'rejected');
    const h = await remote.fetchHealth(s.url);
    assert.equal(h.kind === 'ok' && h.body.build, 'tree:abc');
    const healthSeen = s.seen.find((x) => x.path === '/_router/health');
    assert.equal(healthSeen?.authorization, undefined, 'health is asked without the password');
  } finally {
    await s.close();
  }
});

test('errors other than a refusal are unexpected, with the server\'s sentence', async () => {
  const s = await serve((_req, res) => json(res, 503, { error: 'no account to sign with' }));
  try {
    const a = await remote.fetchQuota(s.url, 'x');
    assert.equal(a.kind, 'unexpected');
    assert.match(a.kind === 'unexpected' ? a.detail : '', /^503 .*no account to sign with$/);
  } finally {
    await s.close();
  }
});

test('a redirect is reported, not followed, and the password does not travel with it', async () => {
  const target = await serve((_req, res) => json(res, 200, { configured: true }));
  const s = await serve((_req, res) => { res.writeHead(301, { location: `${target.url}/api/auth` }); res.end(); });
  try {
    const a = await remote.checkPassword(s.url, 'hunter22');
    assert.equal(a.kind, 'unexpected');
    assert.match(a.kind === 'unexpected' ? a.detail : '', /redirects to http:\/\/127\.0\.0\.1:\d+\/api\/auth/);
    assert.equal(target.seen.length, 0, 'the redirect target was never asked');
  } finally {
    await s.close();
    await target.close();
  }
});

test('no server, and a server that does not answer in time, are unreachable', async () => {
  const gone = await serve(() => undefined);
  const url = gone.url;
  await gone.close();
  const refused = await remote.fetchQuota(url, 'x', 2000);
  assert.deepEqual(refused, { kind: 'unreachable', detail: 'the connection was refused' });

  const slow = await serve(() => { /* never answers */ });
  try {
    const late = await remote.fetchQuota(slow.url, 'x', 300);
    assert.deepEqual(late, { kind: 'unreachable', detail: 'no answer within 300 ms' });
  } finally {
    await slow.close();
  }
});

test('fetch errors in words', () => {
  const t = new DOMException('The operation was aborted due to timeout', 'TimeoutError');
  assert.equal(remote.describeFetchError(t, 10_000), 'no answer within 10 s');
  const dns = Object.assign(new TypeError('fetch failed'), { cause: { code: 'ENOTFOUND', message: 'getaddrinfo ENOTFOUND x' } });
  assert.equal(remote.describeFetchError(dns, 10_000), 'its name does not resolve');
  const cert = Object.assign(new TypeError('fetch failed'), { cause: { code: 'UNABLE_TO_VERIFY_LEAF_SIGNATURE', message: 'unable to verify' } });
  assert.match(remote.describeFetchError(cert, 10_000), /TLS certificate was not accepted \(UNABLE_TO_VERIFY_LEAF_SIGNATURE\)/);
  const other = Object.assign(new TypeError('fetch failed'), { cause: { code: 'EWHATEVER', message: 'something odd' } });
  assert.equal(remote.describeFetchError(other, 10_000), 'something odd');
});
