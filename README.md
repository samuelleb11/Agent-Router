# Agent Router

A local proxy that sits between Claude Code and Anthropic, so you can

* **keep several Claude accounts separate on one machine** and pick which one is
  serving, without `/login` and without restarting your agent; and
* **see where your tokens actually go** — per project, per session, per model, per
  tool, and what filled the context window.

Everything is local: one Rust binary, one SQLite file, no cloud, no telemetry, and a
socket that only ever listens on loopback.

> **Status:** proxy, capture, dashboard and multi-account switching are all working.
> See [ARCHITECTURE.md](ARCHITECTURE.md).

## Use it

**1. Build.**

```sh
cargo build --release
```

**2. Run it.** Leave it running in a terminal, or background it.

```sh
./target/release/agent-router
```

It prints where it is listening, which credential it picked up and when that expires:

```
agent-router listening on http://127.0.0.1:8787 (loopback only)
  upstream:     https://api.anthropic.com
  credential:   sk-ant-oat01 (subscription OAuth)
  subscription: max
  expires:      in 75 min
capture started db=/Users/you/.agent-router/agent-router.db
```

**3. Point a project at it.** Create `.claude/settings.local.json` in the project:

```json
{
  "env": {
    "ANTHROPIC_BASE_URL": "http://127.0.0.1:8787"
  }
}
```

Add that file to the project's `.gitignore` — it is machine-local. The same three
lines work in every project; there is nothing per-project to configure. Both the CLI
and the VSCode extension read it.

**4. Use Claude Code normally.** Nothing else changes. Delete the settings file and
the project goes straight back to talking to Anthropic directly.

**5. Open the dashboard** at <http://127.0.0.1:8787>.

Quota headroom for the active account with its history, the totals for whatever you
have scoped (prompt and output tokens, cache hit rate, time to first token, errors),
activity over time stacked by model, what the model read from cache versus paid in
full, what filled the context window, the main-thread / subagent split, tokens per
project and per session, which tools flood the context, and every request as an
expandable row. Click a project or a session and the whole page rescopes to it; the
breadcrumb and the time range (1h / 24h / 7d / 30d / all) sit in a sticky bar at the
top and are mirrored in the URL, so a link keeps its scope. It refreshes every five
seconds without flicker and is served from the same binary — no build step, no node
toolchain, no network, and it works down to phone width.

For anything the dashboard does not answer, the store is plain SQLite:

```sh
sqlite3 ~/.agent-router/agent-router.db < queries.sql
```

## What it records

Per request: the session and project it belongs to, the model, latency, status,
exact token usage (input, output, thinking, cache reads, and cache writes split by
5-minute and 1-hour tier), the tools the model called, the tool results in its
context, quota headroom, and a capped copy of both bodies with their true sizes.

Session and project attribution are exact, not inferred: Claude Code labels every
request with `x-claude-code-session-id`, and that id names its own transcript file
under `~/.claude/projects/<cwd-slug>/`, whose contents give the working directory
and git branch.

### One number worth looking at first

On a normal Claude Code turn, **tool definitions are around three quarters of the
context window** — far more than the conversation. Run the "where the context
actually goes" query and see what your own split looks like.

## Accounts

The account Claude Code is already logged into is picked up automatically — nothing to
configure. To add another, open the dashboard and use **+ add an account**:

* **Long-lived token (recommended).** Log into the other account in a terminal and run
  `claude setup-token`; paste what it prints. The token does not expire, so the router
  never has to refresh it and can never collide with a running Claude Code.
* **Separate Claude Code profile.** The router creates an isolated `CLAUDE_CONFIG_DIR`
  and hands you a `claude auth login` command to run in a terminal. It polls for
  completion and fills in the email, org and plan itself. The token is short-lived and
  only Claude Code can renew it, so the dashboard shows its expiry.

Switching is a click, takes effect on the **next request**, and needs no restart of the
router or of Claude Code. Tokens are held in the macOS Keychain, never in the database.

An isolated profile cannot disturb your main login: a fresh `CLAUDE_CONFIG_DIR` reports
"not logged in" even with the default profile's `.claude.json` copied into it.

## Upgrading without breaking sessions

Build the new binary and just start it — no need to stop the old one:

```sh
cargo build --release && ./target/release/agent-router
```

The new instance binds the port alongside the old one, tells it to stand down, and the
old one finishes the requests and streams it is already holding before exiting. Nothing
is refused and no stream is cut, so a Claude Code session running through the router
does not notice. Measured across a live handover: 575 consecutive probes with zero
failures, and a 58-second streaming response begun on the old instance completed intact.

## Configuration

| Variable | Default | Purpose |
| --- | --- | --- |
| `AGENT_ROUTER_PORT` | `8787` | Port to listen on |
| `AGENT_ROUTER_DB` | `~/.agent-router/agent-router.db` | Store location |
| `AGENT_ROUTER_BODY_CAP` | `65536` | Bytes of each body kept; true size is always recorded |
| `AGENT_ROUTER_UPSTREAM` | `https://api.anthropic.com` | Upstream API |
| `AGENT_ROUTER_ACCESS_TOKEN` | *(unset)* | Use this token instead of the Keychain one |
| `AGENT_ROUTER_LOG` | `agent_router=info` | `tracing` filter |
| `AGENT_ROUTER_DUMP_HEADERS` | `1` | Requests to dump full headers for, at startup |
| `AGENT_ROUTER_BIND` | *(unset — loopback)* | An IP to listen on instead, to follow sessions from another device. Turns the password on; see below |

## Security

Agent Router holds a live subscription credential and signs everything it forwards,
so reaching it is kept as hard as it can be:

1. The socket binds to `127.0.0.1` unless `AGENT_ROUTER_BIND` says otherwise.
2. The connecting peer must itself be on loopback — again, unless the bind was widened.
3. The `Host` header must parse as a loopback address, or a bare IP when the bind was
   widened. This is what defeats DNS rebinding — a web page that resolves its own domain
   to `127.0.0.1` so your browser will send credentialed requests here. Layers 1 and 2
   both *pass* in that attack, and rebinding needs a *name*, so bare IPs stay safe.
4. Only `/v1/` paths are proxied, and only for callers that already presented a
   credential of their own, so nothing can borrow the token by simply finding the port.
5. Anything that is not on this machine must present the **router password**.

### The password

Only devices other than this one are ever asked; Claude Code, the menubar app and your
own browser talk to the router over loopback and never see a prompt. The first time the
dashboard opens on a network bind it asks you to set one; afterwards it lives under
Settings → Password. Until one is set, other devices are refused outright.

- A browser gets the usual prompt. The username is ignored.
- A Claude Code on another machine sets `ANTHROPIC_AUTH_TOKEN` to the password (and
  `ANTHROPIC_BASE_URL` to this router). The router checks it and swaps in the real one.
- Forgot it? Set a new one from this machine; it never asks for the old one.

It is stored hashed (PBKDF2-HMAC-SHA256, 600k rounds), never in the clear. It *travels*
in the clear: this is HTTP, and the network bind already assumes a network you trust.

Tokens live in the macOS Keychain and are never written to the database or to config.
They are read and written through `/usr/bin/security`, and the items the router creates
trust it (`-T /usr/bin/security`). That is a deliberate trade. A locally built binary is
ad-hoc signed, so every `cargo build` produces a new code identity, "Always Allow"
whitelists only the build that asked, and the prompt returns on the next rebuild — with
no button that makes it stop. The cost is that any process running as you can read those
items through `security` without a prompt, which is the same footing as Claude Code's own
credential entry. If you would rather have a per-binary ACL, sign the binary with a
stable identity (`codesign -s <your-cert> target/release/agent-router`) and the Security
framework path becomes practical again.
Inbound `authorization` and `x-api-key` headers are discarded, never stored. Captured
bodies may contain your source code — the database is local and gitignored.

## The iPhone app

`ios/AgentRouter/` is a SwiftUI app for iOS 26 that watches any number of routers from
your phone: each router's quota and forecast, every account it holds, spend across
accounts, and a switcher to change which account serves — the same
`POST /api/accounts/activate` the dashboard uses, so a switch takes effect on the next
request. It talks to the router over the network bind and authenticates with the router
password (HTTP Basic; kept in the phone's Keychain).

It is also the dashboard in your pocket. Router → **Usage** is the dashboard page — the
range buttons, the totals tiles, activity over time, what the model read, what fills the
context window, models, main thread vs subagents, accounts — with Projects, Sessions,
Tools and Requests beneath it, each narrowing the page when tapped. **Manage accounts**
and **Settings** are the settings modal: add an account (sign in through the router,
paste a token, or point at a profile), rename, rotate, remove; the login item, routing,
per-project routes, the wrapper per extension (and whether the router keeps it
installed on its own), and the password.

It is also the `/live` page in your pocket. Router → Live sessions lists every session
on that Mac the way the web view does (by project, recent first, older and archived
folded away); open one and the feed streams as it happens — prose, tool calls and their
output, thinking, and questions. A session marked **live** runs under the wrapper and
takes a typed message, answers to an AskUserQuestion from the form the prompt renders,
and a Stop beside Send while it is working — Send stays, because a message typed while
a turn runs is queued by the session itself and taken up when the turn ends (the status
line counts what is waiting); one marked **view** is read-only, which is what every
session started before the wrapper was installed (or after an extension update) is,
until it is restarted. The strip above the feed switches between a project's sessions,
the clock button recalls what you already sent, and the menu archives the session — or
changes its **model** and **effort** for the next turn, from the phone or from the web
page's facts strip. The model goes in as the editor's own `set_model` control request,
and the CLI's confirmation or refusal is what you see; effort is `/effort <level>` typed
into the session. Both need the wrapper.

**Side questions.** The menu's **Ask on the side…** (the **btw** button on the web page)
is `/btw` for a session you are not sitting at: the CLI's own `/btw` is an interactive
command the editor's stream refuses, so the router does what it does instead — resumes
the session *forked*, with no tools and an instruction to answer briefly, and shows you
the answer. The session never sees the question or the answer; nothing is interrupted
and nothing is queued. It costs one replay of the session's context (mostly cached, so
a few cents and about ten seconds on a small session, longer on a big one), and the
fork it leaves is shelved so the list stays clean.

**The wrapper keeps itself installed.** An extension update lands a fresh binary the
wrapper is not in, and every session the new version starts is view-only until it is.
Settings → Wrapper → **Manage it automatically** (dashboard) or **Manage automatically**
(phone) has the router watch the extension directory and wrap any version's binary it is not
in — a fresh update included — once the download has settled. The switch lives in the router's database — flipped
once, it holds across router restarts and app versions, and it is off until you turn it
on. Sessions already running keep the binary they started with; the reload the extension
asks for after an update is the only step left, and new sessions after it are live. The
phone is told each time it happens, and the Setup section keeps the log.

The Xcode project is generated from `project.yml`, so the `.xcodeproj` is not committed:

```
brew install xcodegen
cd ios/AgentRouter && xcodegen generate
xcodebuild -scheme AgentRouter -destination 'platform=iOS,id=<device id>' -derivedDataPath build build
xcrun devicectl device install app --device <device id> build/Build/Products/Debug-iphoneos/AgentRouter.app
xcrun devicectl device process launch --device <device id> ca.whnet.agent-router
```

`xcrun devicectl list devices` prints the device id. Unit tests run on the simulator:
`xcodebuild test -scheme AgentRouter -destination 'platform=iOS Simulator,name=iPhone 17 Pro'`.
Two of them talk to a real router: `LiveNetworkTests` (the app's own networking, stream
included — set `TEST_RUNNER_ROUTER_BASE`, `_PASSWORD` and `_SESSION`, and run it on the
phone with `-destination 'platform=iOS,id=…' CODE_SIGNING_ALLOWED=YES` to cross the
password guard) and the UI tests, which drive add-router → sessions → a chat against a
router on `127.0.0.1:8799`, and tap a notification carrying a router push's payload with
the app in front, in the background, and not running, checking that each lands on the
session (`--ui-test-notify` schedules it locally, since nothing can push to a simulator).

**Notifications.** The router tells the phone the moment a session waits on a person —
a permission, an AskUserQuestion — through Apple's push service, once per prompt; a tap
opens that session. Everything the feature needs is built in: the app carries the push
entitlement, the router speaks to Apple, and the app tells the router its team and
bundle id when it registers. What cannot be built in is Apple's part. A push-enabled
App ID and a signing key are issued only to the account holder, by the developer
portal, and no API or Xcode feature creates the key. Two one-time steps, both on the
account holder's login:

1. Sign into Xcode with the Apple ID (Xcode → Settings → Accounts). Automatic signing
   then registers the App ID `ca.whnet.agent-router` with the Push Notifications
   capability and fetches a profile that carries it — the team's wildcard profile
   already on the Mac cannot. Until then, `PUSH=0 ios/AgentRouter/scripts/device-build.sh`
   builds and installs a variant without the entitlement (a build-setting override is
   not enough — Xcode also reads the capability off the project), and the app says so
   under Settings → Notifications. To keep the Apple ID out of Xcode altogether, do the
   same by hand in the portal instead — Identifiers → + → App → explicit
   `ca.whnet.agent-router` → Push Notifications → Register; then Profiles → + → iOS App
   Development → that App ID, the Mac's Apple Development certificate, the phone →
   Generate → Download, double-click it — and build with
   `PROFILE="<its name>" ios/AgentRouter/scripts/device-build.sh`.
2. Make one APNs key: developer.apple.com → Certificates, IDs & Profiles → Keys → + →
   tick Apple Push Notifications service → Register → Download. Apple hands the `.p8`
   out exactly once, so keep the file. Drop it, name unchanged (`AuthKey_<key id>.p8`),
   into `~/.agent-router/` on the Mac: the router takes it in by itself — the key id
   from the file name, the team id from the app's first registration — and says so in
   the dashboard and under Settings → Notifications on the phone. Pasting it into either
   still works. Either way it stays in the router's secrets file and never comes back
   out. One key serves every app on the team, sandbox and production, and does not
   expire; the older per-app push certificate has none of those properties and is not
   supported.

Then Settings → Notifications → Enable on the phone, and "Send test".

Signing is automatic under team `ABCDE12345`. It needs no Apple ID in Xcode as long as
the team's wildcard development profile is on the Mac and lists the phone (it is, and it
does); see the comment in `project.yml` for the one wrong turn that makes Xcode report
"No profiles found".

## Building and packaging for Mac and Linux

The router builds for four platforms and ships as a VS Code extension per platform,
with the router and the wrapper inside the package. The extension is not the router:
every editor window runs its own copy of the extension, so each one only makes sure a
current router is serving on this machine — `agent-router --ensure` exits when a
same-or-newer build already answers, and takes the port over without dropping a
request when an older one does — then shows the quota in the status bar and opens the
dashboard and the live page in an editor tab. Claude Code is pointed at the router
through its settings file and launched through the wrapper by way of Claude Code's own
`claudeCode.claudeProcessWrapper` setting, so nothing inside Anthropic's extension is
ever touched.

```
scripts/build-target.sh aarch64-apple-darwin        # or x86_64-apple-darwin,
scripts/package-vsix.sh darwin-arm64                #    x86_64-unknown-linux-musl, aarch64-unknown-linux-musl
code --install-extension dist/agent-router-0.1.0-darwin-arm64.vsix
```

Linux builds are static (musl). From a Mac they cross-build through `cargo zigbuild`
(`brew install zig && cargo install cargo-zigbuild`) or a musl cross compiler; on Linux
they build natively. `.gitlab-ci.yml` does all four on a tag — the Linux job in Docker
through zig, the Mac job on a shell runner tagged `macos` that has rustup, node and zig
— puts the VSIX files in the package registry, makes a release, and publishes to the
Marketplace and Open VSX when the `VSCE_PAT` and `OVSX_PAT` variables are set. The
extension's manifest lives in `vscode/`; its version follows `Cargo.toml`. The publisher
id in it, `whnet`, must match the Marketplace publisher you create.

`agent-router install` grew two options for the same reason: `--bind <addr>` writes the
listen address into the login item, so a router that serves the phone keeps doing so
after a reboot, and `--no-config` registers the login item without touching Claude
Code's settings.

## Findings and research

Research and decisions that are not descriptions of the code — what Apple push needs,
how the router can ship as a VS Code extension and what that costs, the product
assessment — live in [docs/](docs/README.md), one dated document per finding with its
sources.

## Current limitations

* **Adding an account needs a terminal once.** Claude Code's login is a terminal
  program and produces no output when driven headlessly, so the router cannot run it
  inside the page. It sets everything up either side of that one command and detects
  when you finish.
* **No cost figures.** `usage.cost` is nullable and unset — there is no pricing table
  yet, and on a subscription the quota utilization the router already records is the
  more meaningful number.
* **Context composition is an estimate.** Totals are exact; the split across system /
  tools / history / tool results apportions that exact total by character share,
  because there is no local tokenizer for current Claude models.
