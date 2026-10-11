// CI only: download published binaries; never alter the developer's VS Code.
import { downloadAndUnzipVSCode, resolveCliArgsFromVSCodeExecutablePath } from '@vscode/test-electron';
import { execFileSync } from 'node:child_process';
import { mkdir, readdir, readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';

if (process.env.CI !== 'true' || process.platform !== 'linux') throw new Error('Native editor qualification requires Linux CI');
const root = path.resolve('.ci/codex-editor');
await mkdir(root, { recursive: true });
const version = '26.1007.21434';
const vscodeExecutable = await downloadAndUnzipVSCode({ version: '1.141.0', cachePath: path.join(root, 'vscode') });
const vsix = path.join(root, 'codex.vsix');
const url = `https://openai.gallery.vsassets.io/_apis/public/gallery/publisher/openai/extension/chatgpt/version/${version}/assetbyname/Microsoft.VisualStudio.Services.VSIXPackage?targetPlatform=linux-x64`;
const response = await fetch(url, { signal: AbortSignal.timeout(120_000) });
if (!response.ok) throw new Error(`Pinned Codex VSIX download: HTTP ${response.status}`);
await writeFile(vsix, Buffer.from(await response.arrayBuffer()));
const extensionsDir = path.join(root, 'extensions');
const [cli, ...args] = resolveCliArgsFromVSCodeExecutablePath(vscodeExecutable);
execFileSync(cli, [...args, '--no-sandbox', '--user-data-dir', path.join(root, 'installer'), '--extensions-dir', extensionsDir, '--install-extension', vsix, '--force'], { stdio: 'inherit', timeout: 120_000 });
const extension = (await readdir(extensionsDir)).find(name => name.startsWith(`openai.chatgpt-${version}`));
if (!extension) throw new Error('Pinned native Codex extension was not installed');
const codexBin = path.join(extensionsDir, extension, 'bin/linux-x86_64/codex');
const nativeVersion = execFileSync(codexBin, ['--version'], { encoding: 'utf8' }).trim();
if (nativeVersion !== 'codex-cli 0.162.0-alpha.17.2') throw new Error(`CLI/sidebar version mismatch: ${nativeVersion}`);
const pkg = JSON.parse(await readFile(path.join(extensionsDir, extension, 'package.json'), 'utf8'));
if (pkg.version !== version) throw new Error('Native extension version mismatch');
await writeFile(path.join(root, 'setup.json'), JSON.stringify({ vscodeExecutable, extensionsDir, codexBin, nativeVersion, extensionVersion: version }, null, 2));
console.log(`Prepared VS Code 1.141.0, Codex extension ${version}, ${nativeVersion}`);
