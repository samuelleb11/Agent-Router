// Published native sidebar, attached through Agent Router's JSONL/socket adapter.
// Editor-host automation belongs to the isolated Linux CI job.
import assert from 'node:assert/strict';
import { _electron as electron } from 'playwright';
import { execFileSync } from 'node:child_process';
import { cp, mkdir, readFile, readdir, writeFile, rm } from 'node:fs/promises';
import path from 'node:path';
import { assertRolloutPreserved } from './history.mjs';

assert.equal(process.env.CI, 'true');
const setup = JSON.parse(await readFile(process.env.AR_SIDEBAR_SETUP, 'utf8'));
const scratch = process.env.AR_SIDEBAR_SCRATCH;
const home = path.join(scratch, 'home');
const codex = path.join(scratch, 'codex');
const project = path.join(scratch, 'project');
const server = process.env.AR_SIDEBAR_ROUTER;
const cases = JSON.parse(process.env.AR_SIDEBAR_MODELS);
const artifacts = path.resolve(process.env.AR_SIDEBAR_ARTIFACTS || '.ci/codex-artifacts');
await mkdir(artifacts, { recursive: true });
const environment = { PATH: process.env.PATH, HOME: home, CODEX_HOME: codex,
  DISPLAY: process.env.DISPLAY, XAUTHORITY: process.env.XAUTHORITY,
  TMPDIR: path.join(scratch, 'tmp'), XDG_CONFIG_HOME: path.join(scratch, 'config'),
  XDG_CACHE_HOME: path.join(scratch, 'cache'), XDG_DATA_HOME: path.join(scratch, 'data'),
  XDG_STATE_HOME: path.join(scratch, 'state'), LANG: 'en_US.UTF-8', CI: 'true' };
const launcher = execFileSync(process.env.AR_ROUTER_BIN, ['codex', 'configure', process.env.AR_CODEX_BIN], { env: environment, encoding: 'utf8' }).trim();
assert.ok(path.isAbsolute(launcher));
const userData = path.join(scratch, 'managed-vscode-user');
await mkdir(path.join(userData, 'User'), { recursive: true });
await writeFile(path.join(userData, 'User/settings.json'), JSON.stringify({
  'security.workspace.trust.enabled': false, 'workbench.startupEditor': 'none',
  'telemetry.telemetryLevel': 'off', 'update.mode': 'none',
  'chatgpt.openOnStartup': true, 'chatgpt.cliExecutable': launcher,
}));
await writeFile(path.join(project, 'fixture.txt'), 'MANAGED_SELECTION_CONTEXT_789\nLOCAL_TOOL_FIXTURE_OK\n');
execFileSync('git', ['init', '--quiet', project], { env: environment });
let app, page;
let editorStderr = '';
async function api(route, body) {
  const response = await fetch(`${server}${route}`, { method: body ? 'POST' : 'GET',
    headers: { Authorization: `Bearer ${process.env.AR_SIDEBAR_ADMIN}`, 'Content-Type': 'application/json' },
    ...(body ? { body: JSON.stringify(body) } : {}) });
  const value = await response.json();
  assert.ok(response.ok, `${route}: ${response.status} ${JSON.stringify(value)}`);
  return value;
}
async function catalogue() {
  const response = await fetch(`${server}/_router/models`, { headers: { Authorization: `Bearer ${process.env.AR_SIDEBAR_TOKEN}` } });
  assert.ok(response.ok); return (await response.json()).models;
}
async function sidebar() {
  for (let i = 0; i < 120; i++) {
    for (const frame of page.frames()) {
      const count = await frame.locator('[contenteditable="true"]').count().catch(error => {
        if (frame.isDetached()) return 0; throw error;
      });
      if (count) return frame;
    }
    await new Promise(resolve => setTimeout(resolve, 500));
  }
  throw new Error('Managed native sidebar did not appear');
}
async function picker(frame, rows) {
  const labels = rows.flatMap(row => [row.display_name, row.display_name.replace(/^gpt[- ]?/i, '')]);
  labels.push('Custom');
  const regex = new RegExp(labels.filter(Boolean).map(label => label.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')).join('|'));
  await frame.getByRole('button', { name: 'Select model', exact: true }).or(frame.getByRole('button', { name: regex })).first().click();
  const action = frame.getByRole('menuitem', { name: 'Select model', exact: true });
  const choices = frame.getByRole('menuitemradio');
  await action.or(choices.first()).first().waitFor({ state: 'visible', timeout: 15_000 });
  if (await action.isVisible()) await action.click();
  await choices.first().waitFor({ state: 'visible', timeout: 15_000 });
}
function modelRow(frame, label) {
  const text = frame.getByText(label, { exact: true }).or(frame.getByText(label.replace(/^gpt[- ]?/i, ''), { exact: true }));
  return frame.getByRole('menuitemradio').filter({ has: text });
}
async function launch() {
  editorStderr = '';
  app = await electron.launch({ executablePath: setup.vscodeExecutable,
    // Kubernetes job containers have a small /dev/shm. Use the isolated fixture
    // temp directory for Chromium's shared memory, including webview renderers.
    args: ['--no-sandbox', '--disable-gpu', '--disable-dev-shm-usage', '--skip-welcome', '--skip-release-notes',
      '--user-data-dir', userData, '--extensions-dir', setup.extensionsDir, project,
      '--goto', `${path.join(project, 'fixture.txt')}:1:1`], env: environment, timeout: 60_000 });
  app.process().stderr?.on('data', bytes => {
    editorStderr = (editorStderr + bytes.toString()).slice(-256 * 1024);
  });
  page = await app.firstWindow(); await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.locator('.monaco-workbench').waitFor({ timeout: 60_000 });
  const frame = await sidebar();
  // Real editor selection, consumed by the native extension's context sharing.
  await page.locator('.monaco-editor .view-lines').first().click();
  await page.keyboard.press('Control+a');
  console.log('MANAGED_SIDEBAR_PHASE composer-ready');
  return frame;
}
async function histories(dir = path.join(codex, 'sessions')) {
  const result = new Map();
  for (const entry of await readdir(dir, { withFileTypes: true })) {
    const file = path.join(dir, entry.name);
    if (entry.isDirectory()) for (const row of await histories(file)) result.set(...row);
    else if (entry.name.endsWith('.jsonl')) result.set(file, await readFile(file, 'utf8'));
  }
  return result;
}
async function threadFor(marker) {
  for (let i = 0; i < 100; i++) {
    const found = [...await histories()].filter(([,text]) => text.includes(marker));
    if (found.length === 1) {
      const first = JSON.parse(found[0][1].split('\n').find(Boolean));
      return `codex:${first.payload.id}`;
    }
    await new Promise(resolve => setTimeout(resolve, 100));
  }
  throw new Error(`No unique saved native thread for ${marker}`);
}
async function idle(session) {
  for (let i = 0; i < 100; i++) {
    const rows = await api('/api/live/sessions');
    const row = rows.find(row => row.session_id === session);
    if (row?.hosted === true && row.wrapped === true && row.active === false) return;
    await new Promise(resolve => setTimeout(resolve, 100));
  }
  throw new Error(`Managed native thread did not settle: ${session}`);
}
const sessions = [];
try {
  let frame = await launch(); const models = await catalogue();
  for (const { model } of cases) {
    await picker(frame, models);
    await modelRow(frame, models.find(row => row.id === model).display_name).press('Enter');
    const marker = `MANAGED_SIDEBAR_CONTEXT:${model}`;
    await frame.locator('[contenteditable="true"]').first().fill(`${marker}: Read fixture.txt using a command tool.`);
    await frame.getByRole('button', { name: 'Send', exact: true }).click();
    await frame.getByText(`SMOKE_OK ${model}`, { exact: true }).waitFor({ timeout: 45_000 });
    await frame.getByRole('button', { name: /^Stop/ }).waitFor({ state: 'hidden', timeout: 15_000 });
    const session = await threadFor(marker); await idle(session);
    sessions.push({ session, model, marker });
    const remote = `REMOTE_SIDEBAR_FOLLOWUP:${model}`;
    const sent = await api('/api/live/send', { session, text: `${remote}: Read fixture.txt again.` });
    assert.equal(sent.acknowledged, true);
    await frame.getByText(`${remote}: Read fixture.txt again.`, { exact: true }).waitFor({ timeout: 30_000 });
    await idle(session);
    console.log(`MANAGED_SIDEBAR_PHASE same-thread-remote-turn ${model} ${session}`);
    if (model === 'gpt-5.4') {
      await api('/api/live/model', { session, model: 'kimi-for-coding' });
      // The native UI must receive the backend setting change, rather than
      // silently overwrite it with a stale local selection on its next turn.
      await picker(frame, models);
      const selected = modelRow(frame, models.find(row => row.id === 'kimi-for-coding').display_name);
      await selected.waitFor({ state: 'visible', timeout: 15_000 });
      assert.equal(await selected.getAttribute('aria-checked'), 'true', 'remote model change must reach the native picker');
      await page.keyboard.press('Escape');
      await api('/api/live/mode', { session, mode: 'default' });
      await rm(path.join(project, 'approval-proof.txt'), { force: true });
      await frame.locator('[contenteditable="true"]').first().fill('CONTROL_APPROVAL: Write the approval proof using a command tool.');
      await frame.getByRole('button', { name: 'Send', exact: true }).click();
      let approval;
      for (let i = 0; i < 100; i++) {
        const value = await api(`/api/live/prompt?session=${encodeURIComponent(session)}`);
        if (value.pending) { approval = value; break; }
        await new Promise(resolve => setTimeout(resolve, 100));
      }
      assert.ok(approval, 'native sidebar approval must reach remote controller');
      const answer = await api('/api/live/answer', { session, request_id: approval.request_id, allow: true });
      assert.equal(answer.acknowledged, true); await idle(session);
      assert.equal(await readFile(path.join(project, 'approval-proof.txt'), 'utf8'), 'CONTROL_APPROVED');
      await api('/api/live/mode', { session, mode: 'acceptEdits' });
      await frame.locator('[contenteditable="true"]').first().fill('CONTROL_SLEEP: Wait using a command tool.');
      await frame.getByRole('button', { name: 'Send', exact: true }).click();
      await frame.getByRole('button', { name: /^Stop/ }).waitFor({ state: 'visible', timeout: 15_000 });
      const cancelled = await api('/api/live/interrupt', { session }); assert.equal(cancelled.acknowledged, true);
      await frame.getByRole('button', { name: /^Stop/ }).waitFor({ state: 'hidden', timeout: 15_000 });
      await idle(session);
      console.log('MANAGED_SIDEBAR_PHASE remote-model-approval-cancel');
    }
    await frame.getByRole('button', { name: 'New chat', exact: true }).click();
    await frame.getByText(`${remote}: Read fixture.txt again.`, { exact: true }).waitFor({ state: 'hidden', timeout: 15_000 });
    frame = await sidebar();
  }
  await app.close(); app = undefined;
  const before = await histories();
  await writeFile(path.join(artifacts, 'managed-history-before.json'), JSON.stringify(Object.fromEntries(before), null, 2));
  await launch(); await app.close(); app = undefined;
  const after = await histories();
  for (const [file,text] of before) assertRolloutPreserved(text, after.get(file));
  for (const { session } of sessions) { await idle(session); }
  await writeFile(path.join(artifacts, 'managed-sidebar-sessions.json'), JSON.stringify(sessions, null, 2));
  console.log('MANAGED_SIDEBAR_OK models=3 remote=3 model-change=1 approval=1 cancel=1 same-history=1');
} catch (error) {
  await writeFile(path.join(artifacts, 'managed-vscode-stderr.log'), editorStderr);
  if (page) {
    await page.screenshot({ path: path.join(artifacts, 'managed-sidebar-failure.png') }).catch(() => {});
    for (const [index,frame] of page.frames().entries()) {
      await writeFile(path.join(artifacts, `managed-frame-${index}.txt`), await frame.locator('body').innerText().catch(() => 'Frame inaccessible'));
    }
  }
  await cp(path.join(userData, 'logs'), path.join(artifacts, 'managed-vscode-logs'), { recursive: true }).catch(() => {});
  throw error;
} finally {
  if (app) await app.close();
}
