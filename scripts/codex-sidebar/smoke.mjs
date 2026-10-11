// Drive the published native sidebar, including the visible model picker. No
// replacement webview, stub extension, executable override or mocked model/list.
import assert from 'node:assert/strict';
import { _electron as electron } from 'playwright';
import { execFileSync } from 'node:child_process';
import { mkdir, readFile, readdir, writeFile } from 'node:fs/promises';
import path from 'node:path';

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
  PATH: process.env.PATH, DISPLAY: process.env.DISPLAY, HOME: home, CODEX_HOME: codex,
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
async function palette(command) {
  await page.keyboard.press('Control+Shift+P');
  const input = page.locator('.quick-input-widget input');
  await input.fill(command);
  await page.keyboard.press('Enter');
}
async function sidebar() {
  for (let attempt = 0; attempt < 60; attempt++) {
    for (const frame of page.frames()) {
      if (await frame.locator('[contenteditable="true"]').count()) return frame;
    }
    await new Promise(resolve => setTimeout(resolve, 500));
  }
  throw new Error('Native Codex sidebar composer did not appear');
}
async function picker(frame) {
  // The published UI exposes Ctrl+Shift+M and a "Select model" action.
  await frame.locator('[contenteditable="true"]').first().click();
  await page.keyboard.press('Control+Shift+M');
  const select = frame.getByRole('button', { name: 'Select model', exact: true });
  await select.waitFor({ state: 'visible', timeout: 15_000 });
  await select.click();
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
    args: ['--no-sandbox', '--disable-gpu', '--skip-welcome', '--skip-release-notes',
      '--user-data-dir', userData, '--extensions-dir', setup.extensionsDir, project],
    env: environment, timeout: 60_000,
  });
  page = await app.firstWindow();
  await page.locator('.monaco-workbench').waitFor({ timeout: 60_000 });
  await palette('Codex: Open Codex Sidebar');
  return sidebar();
}
try {
  let frame = await launch();
  let catalogue = await models();
  for (const { model } of cases) {
    await picker(frame);
    const name = catalogue.find(row => row.id === model)?.display_name;
    assert.ok(name, `Missing server model ${model}`);
    await frame.getByText(name, { exact: true }).last().click({ timeout: 15_000 });
    // A native sidebar turn: execute and replay a command through the router.
    const composer = frame.locator('[contenteditable="true"]').first();
    await composer.fill('Read fixture.txt using a command tool, then report the result.');
    await composer.press('Enter');
    await frame.getByText(`SMOKE_OK ${model}`, { exact: true }).waitFor({ timeout: 45_000 });
    await frame.getByRole('button', { name: /^Stop/ }).waitFor({ state: 'hidden', timeout: 15_000 });
    await palette('Codex: New Chat in ChatGPT Sidebar');
    frame = await sidebar();
  }
  // Changes do not restart a live owner. Close the editor after the turns,
  // refresh the files, then reopen its same saved workspace/conversations.
  await app.close(); app = undefined;
  const savedConversations = await rollouts();
  assert.ok(savedConversations.size >= 6, 'Native CLI and sidebar conversations must be persisted');
  await registry('POST', '', { alias: 'team-sidebar-new', display_name: 'TEAM SERVER NEW',
    service: 'openai', account_id: 'openai', upstream_model: 'gpt-5.4' });
  execFileSync(process.env.AR_ROUTER_BIN, ['codex', 'models', 'refresh'], { env: environment, stdio: 'inherit', timeout: 30_000 });
  frame = await launch();
  await picker(frame);
  await frame.getByText('TEAM SERVER NEW', { exact: true }).waitFor({ timeout: 15_000 });
  await frame.getByText('TEAM SERVER NEW', { exact: true }).click();
  await app.close(); app = undefined;
  await registry('DELETE', '/team-sidebar-new');
  execFileSync(process.env.AR_ROUTER_BIN, ['codex', 'models', 'refresh'], { env: environment, stdio: 'inherit', timeout: 30_000 });
  frame = await launch();
  await picker(frame);
  assert.equal(await frame.getByText('TEAM SERVER NEW', { exact: true }).count(), 0, 'Removed server model persisted in native picker');
  catalogue = await models();
  assert.equal(catalogue.some(row => row.id === 'team-sidebar-new'), false);
  const resumedConversations = await rollouts();
  for (const [file, history] of savedConversations) {
    assert.ok(resumedConversations.get(file)?.startsWith(history), `Refresh lost or rewrote conversation ${path.basename(file)}`);
  }
  console.log('NATIVE_SIDEBAR_OK models=3 added=1 removed=1');
} catch (error) {
  if (page) {
    await page.screenshot({ path: path.join(artifacts, 'sidebar-failure.png') }).catch(() => {});
    for (const [index, frame] of page.frames().entries()) {
      await writeFile(path.join(artifacts, `frame-${index}.txt`), await frame.locator('body').innerText().catch(() => 'Frame inaccessible'));
    }
  }
  throw error;
} finally {
  if (app) await app.close();
}
