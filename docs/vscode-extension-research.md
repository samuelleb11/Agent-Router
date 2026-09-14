# Shipping Agent Router as a VS Code extension

Research memo, 2026-09-13. Question: can the router ship as a VS Code extension,
installed in one click and updated on its own, with a single instance serving every
open window? Answer: yes. Below is what the platform allows, what it forbids, what
other extensions do, and the shape that fits. The structure described at the end was
built the next day: `vscode/`, `scripts/build-target.sh`, `scripts/package-vsix.sh` and
`.gitlab-ci.yml` in this repository.

## The answer in short

- **Packaging is well trodden.** The Claude Code extension itself is a platform-specific
  VSIX carrying a 207 MB signed binary, validated on the Marketplace. The router is 11 MB.
- **"One instance" is the daemon's job, not VS Code's.** Every window runs its own
  extension host and nothing is shared between them. So the extension is an installer, a
  supervisor and a UI for a machine-wide daemon that already knows how to be alone on
  its port.
- **The wrapper no longer needs to touch Anthropic's files.** The Claude Code extension
  has a documented, machine-scoped setting, `claudeCode.claudeProcessWrapper`, that
  launches Claude through an executable of our choosing and passes the bundled binary's
  path as an argument. That retires the shim-in-the-extension-folder, the re-wrap after
  every update, and the one behaviour a Marketplace reviewer could object to.
- **Route:** Marketplace first, for automatic updates; the same VSIX on Open VSX for
  Cursor, Windsurf and VSCodium; a GitHub or GitLab release VSIX as the no-account
  fallback. A Homebrew tap later for people who do not live in an editor.

| Figure | Value | Note |
| --- | --- | --- |
| Claude Code VSIX (darwin-arm64) | 93 MB | 207 MB binary inside, flagged validated |
| Router binary | 11 MB | plus a 0.6 MB wrapper |
| Marketplace update check | every 12 h | install delay 2 h by default |
| `deactivate()` budget | 5 s | the host is killed at 6 s |

## The wrapper without touching Anthropic's files

Until now the router wrapped Claude Code by renaming the extension's bundled `claude`
to `claude.real` and putting the shim in its place, then watching the extensions folder
to do it again after every update. It works, but it is file surgery on someone else's
extension, and Claude Code ships almost daily into a new folder each time.

The installed extension, version 2.1.270, declares `claudeCode.claudeProcessWrapper`:
"Executable path used to launch the Claude process." Anthropic's VS Code documentation
adds that the bundled binary path is passed as an argument when the setting is present,
and that this is how a process wrapper is meant to be supplied. The setting is
machine-scoped and the extension reads its global value.

What changes: the wrapper is set once, by configuration, and survives every Claude Code
update untouched. The wrapper takes the real binary's path from its first argument
instead of looking for `claude.real` beside itself (done; it still works as the shim
when run under the name `claude`). The auto-manager that re-wraps the extensions folder
becomes a fallback for extension versions that predate the setting.

Two caveats from the documentation: a wrapped setup starts new conversations in Manual
permission mode unless `claudeCode.initialPermissionMode` is set, so the extension sets
that too; and the exact argument order is documented in prose only, so the wrapper logs
the arguments it receives when `AGENT_ROUTER_WRAPPER_DEBUG` is set.

## One instance across windows

There is no shared extension host. VS Code's own August 2026 architecture post describes
the model as tying the runtime to one window, and a request for a cross-window shared
extension instance was closed without action. The consequences for a daemon: the
extension must never be the process that owns the router, and every window must be
able to activate without stepping on the others.

The router has the primitives: it binds with `SO_REUSEPORT`, asks the health endpoint who
is serving, and hands over by asking the incumbent to drain. `--ensure` (added) exits
when a same-or-newer build answers and serves otherwise. The port is the mutex; two
windows racing at start-up both try, and the loser sees a healthy peer and exits.

| Who keeps the daemon alive | Survives window close | Survives VS Code quit | Survives logout | Precedent | Role |
| --- | --- | --- | --- | --- | --- |
| Detached spawn from the extension | yes | yes on macOS and Linux with stdio detached; Windows needs the detached flag, job-object caveat unverified | no | orphaned language servers are a known phenomenon; Node documents the recipe | fallback |
| LaunchAgent, which `agent-router install` writes | yes | yes | restarts at next login; logout sends SIGTERM | VS Code's own CLI writes a LaunchAgent for its tunnel service from a plain process | owner |
| Lock file in global storage | decides who may spawn, not whether the daemon is alive | | | Remote-SSH uses pid files plus flock | optional |
| Extension `globalState` | shared across windows but eventually consistent, last writer wins | | | | not a mutex |

Recommended combination: launchd owns boot-time and crash recovery, the extension owns
"is it up and is it current". On activation, and every few minutes after, each window
asks the health endpoint. Absent: make sure the binary is in `~/.agent-router/bin`,
bootstrap the agent, or spawn detached where launchd is not available. Older than the
bundled version: copy the new binary beside the old one and rename over it, never
overwrite in place, then start it and let the handover take the port. One tension to
name: a supervisor that restarts on every exit would fight the zero-downtime handover,
and a handover leaves the running instance outside launchd until the next login. The
plist's `SuccessfulExit=false` rule was written with exactly that trade-off, and the
extension's periodic health check covers the crash-recovery gap.

## Installing and updating

| Route | Install | Updates | Needs | Role |
| --- | --- | --- | --- | --- |
| Visual Studio Marketplace | one click, or a `vscode:extension/…` link | automatic; clients poll every 12 h and install after a 2 h delay by default; users can pin or opt into pre-release | an Azure DevOps organization and a token (Entra ID sign-in after 1 Dec 2026); no human review, a malware scan gates publishing; the README must disclose the daemon at the top | primary |
| Open VSX | the same one click inside Cursor, Windsurf, VSCodium, Gitpod, Theia | automatic there | a GitHub-backed Eclipse account and a license file; 250 MiB cap; the same VSIX files | mirror |
| Release VSIX (GitHub or GitLab) | `code --install-extension` | none; VS Code disables auto-update for VSIX installs | nothing | fallback |
| Homebrew tap formula | `brew install`, then `brew services start` writes the LaunchAgent | `brew upgrade` | prebuilt tarballs per architecture and a formula with a service block; casks are the wrong tool (no services, signed and notarized artifacts required) | later |

Packaging is per platform: `vsce package --target darwin-arm64`, and again for
darwin-x64, linux-x64, linux-arm64. Package on macOS or Linux, because packaging on
Windows drops executable bits. There is no documented VSIX size limit; the 25 MB figure
people cite is Azure DevOps's, not VS Code's.

## What the extension shows inside the editor

- A status bar item with the headroom number: text with a codicon, a Markdown tooltip, a
  click command, and a warning background when a window is about to run out.
- The dashboard as an editor tab. A webview panel may embed an iframe of
  `http://127.0.0.1:8787/` on desktop VS Code; the built-in Simple Browser does exactly
  this. The page's content-security policy must allow that frame source, and the router
  must not send `X-Frame-Options`. The router asks no password of loopback callers, which
  matters because a nested frame cannot answer an authentication prompt.
- Or hand off to the Integrated Browser that shipped in VS Code 1.109, a real browser
  with cookies and developer tools, reachable through the Simple Browser's open command.
- Prompts through information messages with action buttons.

Limits: clipboard access inside a nested frame is probably blocked unless granted,
links inside the frame navigate in place, and none of this works on vscode.dev.

## macOS: quarantine, signing, Keychain

- **No quarantine.** VS Code's extension installer does not set the quarantine attribute
  on extracted files, verified on this Mac by stamping a VSIX with a browser-style
  quarantine and installing it: files extracted with executable bits intact and no
  quarantine on any of them. Gatekeeper and notarization checks run on quarantined items
  at first launch, so a binary spawned from an extension needs neither.
- **Apple silicon still requires a signature.** Ad-hoc is enough; the linker applies
  one. Claude Code's binary is Developer ID signed with the hardened runtime, which is
  polish, not a requirement.
- **Keychain prompts are about identity.** An ad-hoc signature's designated requirement
  is the build's hash, so every rebuild is a new identity to the Keychain. The router
  sidesteps this by reading through `/usr/bin/security`.
- **Overwriting a running binary kills it.** Write beside, then rename.
- **Since Ventura**, adding a LaunchAgent shows a "Background item added" notice and a
  toggle in Login Items.

## Remote windows

An extension with a `main` entry and no declared kind is treated as a workspace
extension, so under Remote-SSH, WSL or a dev container it would run on the remote, next
to the Claude Code extension, which behaves the same way. The daemon would then have to
run there too, with no Keychain and no launchd, and the dashboard would need
`env.asExternalUri`. The port-attributes API is still proposed and unavailable to
Marketplace extensions. Recommendation for a first release: declare the extension
`"ui"` kind so it always runs on the Mac, and state that Claude Code sessions in remote
windows are not proxied yet.

## Marketplace policy

No clause names daemons, proxies or wrappers. Three apply: the terms of use forbid
causing "unexpected behavior that is not clearly documented at the top of both the
package description and package README"; the publisher agreement says an offering
"must not install or launch executable code on the user's environment beyond what is
identified in or may reasonably be expected from the Listing Information", and allows
removal for any reason; extensions run with the same permissions as VS Code itself.
The design consequence is transparency: the first paragraph of the listing says a local
proxy is installed and runs in the background, that Claude Code is pointed at it through
the documented base-URL setting, and that Claude Code's documented process-wrapper
setting is set. Uninstall has a hook, `vscode:uninstall`, a Node-only script that runs at
the next restart and has misfired before; an explicit "Remove" command exists alongside.

## The shape that fits

```
window activates (onStartupFinished)
  → GET 127.0.0.1:8787/_router/health
      no answer          → install binaries into ~/.agent-router/bin, bootstrap the
                           LaunchAgent (or spawn detached), then `agent-router --ensure`
      older than bundled → copy beside, rename over, `agent-router --ensure` → handover
      same or newer      → nothing
  → status bar reads /api/quota; commands open the dashboard and live pages
  → every few minutes: health again
```

Claude Code is wired through its own settings and nothing else: `ANTHROPIC_BASE_URL` in
its settings file, written by `agent-router install`, and `claudeCode.claudeProcessWrapper`
pointing at `claude-wrapper`. The router, the phone app, the menubar app and the
dashboard are unchanged; the extension is optional at runtime rather than load-bearing.

## Spike plan

1. Wrap through the setting, with no extension: the wrapper takes the real binary's path
   from its first argument (done); set `claudeCode.claudeProcessWrapper` and
   `initialPermissionMode`; confirm a new session's control socket answers.
2. Make the daemon version-aware: `--ensure` (done).
3. A minimal extension: activate on startup finished, ensure, status bar; package with
   `vsce package --target darwin-arm64`; install by hand; close every window; the daemon
   still serves (scaffold done; the in-window run is the next step).
4. The dashboard tab.
5. Publish a pre-release; let one Mac update through the Marketplace path.
6. Mirror to Open VSX and attach the VSIX files to a release.

## Unverified

- The exact argument list the extension passes to a process wrapper (documented in
  prose; the wrapper logs it).
- Which Claude Code version introduced the wrapper setting; present in 2.1.270.
- Whether a detached child of the extension host escapes the job object on Windows.
- Whether clipboard works inside the nested dashboard frame.
- How Claude Code behaves under Remote-SSH.
- The Marketplace's VSIX size limit; undocumented, 93 MB accepted in practice.

## Sources

- Publishing extensions, platform-specific targets, publisher setup, pre-release: https://code.visualstudio.com/api/working-with-extensions/publishing-extension
- Extension Marketplace: auto-update, pinning, pre-release, VSIX installs: https://code.visualstudio.com/docs/configure/extensions/extension-marketplace
- Update poll interval in source: https://github.com/microsoft/vscode/blob/main/src/vs/workbench/contrib/extensions/browser/extensionsWorkbenchService.ts
- VSIX size question: https://github.com/microsoft/vsmarketplace/issues/1541
- Open VSX limits and publishing: https://open-vsx.org/api/version, https://github.com/eclipse/openvsx/wiki/Publishing-Extensions
- Cursor and Windsurf use Open VSX: https://cursor.com/help/customization/extensions, https://docs.windsurf.com/windsurf/recommended-plugins
- Marketplace terms of use: https://aka.ms/vsmarketplace-ToU; publisher agreement: https://cdn.vsassets.io/v/M187_20210610.3/_content/Visual-Studio-Marketplace-Publisher-Agreement.pdf; runtime security: https://code.visualstudio.com/docs/configure/extensions/extension-runtime-security
- Extension manifest, activation events, contribution points, workspace trust: https://code.visualstudio.com/api/references/extension-manifest, https://code.visualstudio.com/api/references/activation-events, https://code.visualstudio.com/api/references/contribution-points, https://code.visualstudio.com/api/extension-guides/workspace-trust
- Extension host and remote extensions: https://code.visualstudio.com/api/advanced-topics/extension-host, https://code.visualstudio.com/api/advanced-topics/remote-extensions; kind inference: https://github.com/microsoft/vscode/blob/main/src/vs/workbench/services/extensions/common/extensionManifestPropertiesService.ts
- One host per window: https://code.visualstudio.com/blogs/2026/08/26/agent-host-architecture, https://github.com/microsoft/vscode/issues/120490
- Deactivate budget and host kill: https://github.com/microsoft/vscode/blob/main/src/vs/workbench/api/common/extHostExtensionService.ts, https://github.com/microsoft/vscode/blob/main/src/vs/workbench/services/extensions/electron-browser/localProcessExtensionHost.ts
- Detached children: https://nodejs.org/api/child_process.html#optionsdetached; Windows job objects: https://github.com/libuv/libuv/blob/v1.x/src/win/process.c
- Orphaned servers as precedent: https://github.com/microsoft/vscode-languageserver-node/issues/726, https://github.com/microsoft/vscode-mypy/issues/291
- Lock files: https://www.npmjs.com/package/proper-lockfile; Remote-SSH locking: https://github.com/microsoft/vscode-remote-release/issues/7162
- VS Code CLI service installers: https://github.com/microsoft/vscode/blob/main/cli/src/tunnels/service_macos.rs
- Shared daemons elsewhere: https://nx.dev/docs/concepts/nx-daemon, https://github.com/wakatime/vscode-wakatime/blob/master/src/dependencies.ts, https://github.com/Exafunction/codeium/issues/13, https://github.com/codota/TabNine/issues/360
- Webviews: https://code.visualstudio.com/api/extension-guides/webview; Simple Browser: https://github.com/microsoft/vscode/blob/main/extensions/simple-browser/src/simpleBrowserView.ts, https://github.com/microsoft/vscode/blob/main/extensions/simple-browser/src/extension.ts; outer CSP: https://github.com/microsoft/vscode/blob/main/src/vs/workbench/contrib/webview/browser/pre/index.html
- Integrated Browser: https://code.visualstudio.com/updates/v1_109, https://code.visualstudio.com/docs/debugtest/integrated-browser; loopback as trustworthy: https://w3c.github.io/webappsec-secure-contexts/
- Port attributes API still proposed: https://github.com/microsoft/vscode/blob/main/src/vscode-dts/vscode.proposed.portsAttributes.d.ts
- Claude Code in VS Code (bundled binary, process wrapper, environment variables): https://code.claude.com/docs/en/vs-code; base URL and gateways: https://code.claude.com/docs/en/llm-gateway, https://code.claude.com/docs/en/llm-gateway-connect; settings files: https://code.claude.com/docs/en/settings
- Claude Code release cadence: https://github.com/anthropics/claude-code/issues/42818; Remote-SSH reports: https://github.com/anthropics/claude-code/issues/20226
- Quarantine is per writing app: https://developer.apple.com/documentation/bundleresources/information-property-list/lsfilequarantineenabled, https://eclecticlight.co/2025/12/08/who-decides-to-quarantine-files/; Gatekeeper and notarization: https://eclecticlight.co/2024/08/10/gatekeeper-and-notarization-in-sequoia/, https://support.apple.com/en-us/102445; Apple silicon signatures: https://eclecticlight.co/2020/08/22/apple-silicon-macs-will-require-signed-code/
- Keychain and code-signing requirements: https://developer.apple.com/documentation/technotes/tn3127-inside-code-signing-requirements, https://developer.apple.com/forums/thread/98484, https://keith.github.io/xcode-man-pages/security.1.html
- Replacing a running binary: https://developer.apple.com/forums/thread/758098, https://docs.rs/self-replace/latest/self_replace/; launchd.plist: https://keith.github.io/xcode-man-pages/launchd.plist.5.html; Ventura login items: https://www.kandji.io/blog/macos-ventura-login-background-items
- Homebrew: https://docs.brew.sh/Formula-Cookbook, https://docs.brew.sh/rubydoc/Homebrew/Service.html, https://docs.brew.sh/Acceptable-Casks, https://brew.sh/2025/11/12/homebrew-5.0.0/; an extension delegating to brew: https://sweetpad.hyzyla.dev/docs/format/
- Self-updaters: https://docs.rs/self_update/latest/self_update/, https://github.com/axodotdev/axoupdater, https://github.com/cargo-bins/cargo-binstall
- Uninstall hook: https://github.com/microsoft/vscode/issues/45474, https://github.com/microsoft/vscode/issues/72375; malicious background installs: https://www.stepsecurity.io/blog/malicious-iolitelabs-vscode-extensions-target-solidity-developers-on-windows-macos-and-linux-with-backdoor
