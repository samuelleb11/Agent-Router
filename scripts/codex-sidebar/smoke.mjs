// Drive the published native sidebar, including the visible model picker. No
// replacement webview, stub extension, executable override or mocked model/list.
import assert from 'node:assert/strict';
import { _electron as electron } from 'playwright';
import { execFileSync } from 'node:child_process';
import { cp, mkdir, readFile, readdir, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { assertRolloutPreserved } from './history.mjs';

assert.equal(process.env.CI, 'true', 'Editor hosts run only in CI');
const setup = JSON.parse(await readFile(process.env.AR_SIDEBAR_SETUP, 'utf8'));
const scratch = process.env.AR_SIDEBAR_SCRATCH;
const home = path.join(scratch, 'home');
const codex = path.join(scratch, 'codex');
const project = path.join(scratch, 'project');
const server = process.env.AR_SIDEBAR_ROUTER;
const token = process.env.AR_SIDEBAR_TOKEN;
const cases = JSON.parse(process.env.AR_SIDEBAR_MODELS);
const artifacts = path.resolve(process.env.AR_SIDEBAR_ARTIFACTS || '.ci/codex-artifacts');
await mkdir(artifacts, { recursive: true });
const configPath = path.join(codex, 'config.toml');
const config = await readFile(configPath, 'utf8');
await writeFile(configPath, `approval_policy = "never"\nsandbox_mode = "workspace-write"\n${config}\n[analytics]\nenabled = false\n`, { mode: 0o600 });
// An isolated fixture login lets the extension render without its login wizard.
// The provider still authenticates with the scoped Client token in config.toml.
await writeFile(path.join(codex, 'auth.json'), JSON.stringify({ OPENAI_API_KEY: 'fixture-key-never-use-upstream' }), { mode: 0o600 });
const userData = path.join(scratch, 'vscode-user');
await mkdir(path.join(userData, 'User'), { recursive: true });
await writeFile(path.join(userData, 'User/settings.json'), JSON.stringify({
  'security.workspace.trust.enabled': false, 'workbench.startupEditor': 'none',
  'telemetry.telemetryLevel': 'off', 'update.mode': 'none',
  'chatgpt.openOnStartup': true,
}));
const environment = {
  PATH: process.env.PATH, DISPLAY: process.env.DISPLAY, XAUTHORITY: process.env.XAUTHORITY,
  HOME: home, CODEX_HOME: codex,
  TMPDIR: path.join(scratch, 'tmp'), XDG_CONFIG_HOME: path.join(scratch, 'config'),
  XDG_CACHE_HOME: path.join(scratch, 'cache'), XDG_DATA_HOME: path.join(scratch, 'data'),
  XDG_STATE_HOME: path.join(scratch, 'state'), LANG: 'en_US.UTF-8', CI: 'true',
};
execFileSync('git', ['init', '--quiet', project], { env: environment });
let app;
let page;
async function rollouts(directory = path.join(codex, 'sessions')) {
  const result = new Map();
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    const file = path.join(directory, entry.name);
    if (entry.isDirectory()) for (const row of await rollouts(file)) result.set(...row);
    else if (entry.name.endsWith('.jsonl')) result.set(file, await readFile(file, 'utf8'));
  }
  return result;
}
async function sidebar() {
  for (let attempt = 0; attempt < 120; attempt++) {
    for (const frame of page.frames()) {
      // VS Code replaces its bootstrap webview frame while the extension loads.
      // A frame snapshot can become detached before the DOM query completes.
      const count = await frame.locator('[contenteditable="true"]').count().catch(error => {
        if (frame.isDetached()) return 0;
        throw error;
      });
      if (count) return frame;
    }
    await new Promise(resolve => setTimeout(resolve, 500));
  }
  throw new Error('Native Codex sidebar composer did not appear');
}
async function picker(frame) {
  // The toolbar button opens the power panel. Its menu action opens the model
  // list; inactive panels remain mounted, so use accessible roles throughout.
  // Startup can replace the labelled control with the icon while clicking.
  // One locator lets Playwright re-query either representation after detachment.
  const labels = (await models()).flatMap(row => [row.display_name, row.display_name.replace(/^gpt[- ]?/i, '')]);
  labels.push('Custom');
  const pattern = new RegExp(`^(?:${labels.filter(Boolean).map(label => label.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')).join('|')})(?:\\s|$)`);
  const toolbar = frame.getByRole('button', { name: 'Select model', exact: true })
    .or(frame.getByRole('button', { name: pattern })).first();
  await toolbar.click();
  const action = frame.getByRole('menuitem', { name: 'Select model', exact: true });
  const rows = frame.getByRole('menuitemradio');
  await action.or(rows.first()).first().waitFor({ state: 'visible', timeout: 15_000 });
  if (await action.isVisible()) await action.click();
  await rows.first().waitFor({ state: 'visible', timeout: 15_000 });
}
function modelRow(frame, label) {
  // The native UI strips GPT prefixes from labels in its model picker.
  const short = label.replace(/^gpt[- ]?/i, '');
  const text = frame.getByText(label, { exact: true }).or(frame.getByText(short, { exact: true }));
  return frame.getByRole('menuitemradio').filter({ has: text });
}
async function models() {
  const response = await fetch(`${server}/_router/models`, { headers: { Authorization: `Bearer ${token}` } });
  assert.equal(response.status, 200);
  return (await response.json()).models;
}
async function registry(method, suffix, body) {
  const response = await fetch(`${server}/api/provider-registry/models${suffix}`, {
    method, headers: { Authorization: `Bearer ${process.env.AR_SIDEBAR_ADMIN}`, 'Content-Type': 'application/json' },
    ...(body ? { body: JSON.stringify(body) } : {}),
  });
  assert.ok(response.ok, `Fixture registry mutation: ${response.status} ${await response.text()}`);
}
async function launch() {
  app = await electron.launch({ executablePath: setup.vscodeExecutable,
    args: ['--no-sandbox', '--disable-gpu', '--disable-dev-shm-usage', '--skip-welcome', '--skip-release-notes',
      '--user-data-dir', userData, '--extensions-dir', setup.extensionsDir, project],
    env: environment, timeout: 60_000,
  });
  page = await app.firstWindow();
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.locator('.monaco-workbench').waitFor({ timeout: 60_000 });
  // openOnStartup is a published native setting. Restored webview focus can
  // intercept workbench palette shortcuts, so let the extension open itself.
  const frame = await sidebar();
  console.log('NATIVE_SIDEBAR_PHASE composer-ready');
  return frame;
}
try {
  let frame = await launch();
  let catalogue = await models();
  for (const { model } of cases) {
    await picker(frame);
    const name = catalogue.find(row => row.id === model)?.display_name;
    assert.ok(name, `Missing server model ${model}`);
    await modelRow(frame, name).press('Enter', { timeout: 15_000 });
    console.log(`NATIVE_SIDEBAR_PHASE selected ${model}`);
    // A native sidebar turn: execute and replay a command through the router.
    const composer = frame.locator('[contenteditable="true"]').first();
    await composer.fill('Read fixture.txt using a command tool, then report the result.');
    await frame.getByRole('button', { name: 'Send', exact: true }).click();
    await frame.getByText(`SMOKE_OK ${model}`, { exact: true }).waitFor({ timeout: 45_000 });
    await frame.getByRole('button', { name: /^Stop/ }).waitFor({ state: 'hidden', timeout: 15_000 });
    console.log(`NATIVE_SIDEBAR_PHASE tool-turn-complete ${model}`);
    await frame.getByRole('button', { name: 'New chat', exact: true }).click();
    await frame.getByText(`SMOKE_OK ${model}`, { exact: true }).waitFor({ state: 'hidden', timeout: 15_000 });
    frame = await sidebar();
  }
  // Changes do not restart a live owner. Close the editor after the turns,
  // refresh the files, then reopen its same saved workspace/conversations.
  await app.close(); app = undefined;
  const savedConversations = await rollouts();
  assert.ok(savedConversations.size >= 6, 'Native CLI and sidebar conversations must be persisted');
  await writeFile(path.join(artifacts, 'history-before.json'), JSON.stringify(Object.fromEntries(savedConversations), null, 2));
  await registry('POST', '', { alias: 'team-sidebar-new', display_name: 'TEAM SERVER NEW',
    service: 'openai', account_id: 'openai', upstream_model: 'gpt-5.4' });
  execFileSync(process.env.AR_ROUTER_BIN, ['codex', 'models', 'refresh'], { env: environment, stdio: 'inherit', timeout: 30_000 });
  frame = await launch();
  await picker(frame);
  await modelRow(frame, 'TEAM SERVER NEW').waitFor({ timeout: 15_000 });
  await modelRow(frame, 'TEAM SERVER NEW').press('Enter');
  await app.close(); app = undefined;
  await registry('DELETE', '/team-sidebar-new');
  execFileSync(process.env.AR_ROUTER_BIN, ['codex', 'models', 'refresh'], { env: environment, stdio: 'inherit', timeout: 30_000 });
  frame = await launch();
  await picker(frame);
  assert.equal(await modelRow(frame, 'TEAM SERVER NEW').count(), 0, 'Removed server model persisted in native picker');
  catalogue = await models();
  assert.equal(catalogue.some(row => row.id === 'team-sidebar-new'), false);
  const resumedConversations = await rollouts();
  await writeFile(path.join(artifacts, 'history-after.json'), JSON.stringify(Object.fromEntries(resumedConversations), null, 2));
  const archived = await rollouts(path.join(codex, 'archived_sessions')).catch(() => new Map());
  await writeFile(path.join(artifacts, 'history-archived.json'), JSON.stringify(Object.fromEntries(archived), null, 2));
  for (const [file, history] of savedConversations) {
    const current = resumedConversations.get(file);
    if (!current?.startsWith(history)) {
      let at = 0;
      if (current) while (at < history.length && history[at] === current[at]) at++;
      console.error(JSON.stringify({ historyMismatch: path.basename(file), exists: current !== undefined,
        beforeBytes: history.length, afterBytes: current?.length, firstDifference: at,
        before: history.slice(Math.max(0, at - 100), at + 500), after: current?.slice(Math.max(0, at - 100), at + 500) }));
    }
    assertRolloutPreserved(history, current);
  }
  console.log('NATIVE_SIDEBAR_OK models=3 added=1 removed=1');
} catch (error) {
  if (page) {
    await page.screenshot({ path: path.join(artifacts, 'sidebar-failure.png') }).catch(() => {});
    for (const [index, frame] of page.frames().entries()) {
      await writeFile(path.join(artifacts, `frame-${index}.txt`), await frame.locator('body').innerText().catch(() => 'Frame inaccessible'));
      const buttons = await frame.getByRole('button').evaluateAll(elements => elements.map(element => ({
        label: element.getAttribute('aria-label'), title: element.getAttribute('title'), text: element.innerText,
      }))).catch(() => []);
      await writeFile(path.join(artifacts, `frame-${index}-buttons.json`), JSON.stringify(buttons, null, 2));
      const items = await frame.getByRole('menuitem').or(frame.getByRole('menuitemradio')).evaluateAll(elements => elements.map(element => ({
        role: element.getAttribute('role'), label: element.getAttribute('aria-label'), text: element.innerText,
      }))).catch(() => []);
      await writeFile(path.join(artifacts, `frame-${index}-menu.json`), JSON.stringify(items, null, 2));
    }
  }
  await cp(path.join(userData, 'logs'), path.join(artifacts, 'vscode-logs'), { recursive: true }).catch(() => {});
  throw error;
} finally {
  if (app) await app.close();
}
