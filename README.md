# Agent Router

A local proxy that sits between Claude Code and Anthropic, so you can

* **keep several Claude accounts separate on one machine** and pick which one is
  serving, without `/login` and without restarting your agent;
* **see where your tokens actually go** — per project, per session, per model, per
  tool, and what filled the context window; and
* **follow and steer your sessions from your phone** — read the live feed, answer a
  permission prompt, send a message, switch the model.

Everything is local: one Rust binary, one SQLite file, no cloud, no telemetry, and a
socket that only ever listens on loopback until you tell it otherwise. The same binary
can also run as a server you host yourself, behind HTTPS, for several machines at once:
see [Running it as a server](#running-it-as-a-server).

> **Status:** working, and used daily by its author. Proxy, capture, dashboard,
> multi-account switching, live sessions, the VS Code extension and both phone apps
> are all in place. See [ARCHITECTURE.md](ARCHITECTURE.md) for how it fits together.

## Platforms

| | macOS | Linux | Windows |
| --- | --- | --- | --- |
| Router and dashboard | yes | yes | — |
| Credential store | Keychain, via `/usr/bin/security` | `~/.claude/.credentials.json` | — |
| Run at login | launchd agent | systemd user service | — |
| VS Code extension | arm64, x64 | x64, arm64 (static musl) | x64, agent only |
| Hosted server | — | container image (glibc, distroless) | — |
| Machine agent, for a hosted router | yes, at login through launchd | yes, at login through a systemd user unit | yes, at login from the Startup folder |

On Windows only the machine agent runs: it serves a hosted router the sessions on that
machine, to follow and to steer, and the router itself refuses to start there. The
Windows build is cross-compiled and has not yet run on Windows; the checklist for its
first run is in [docs/hosting-machine-agent.md](docs/hosting-machine-agent.md#status).

The phone apps are **iOS 26+** (SwiftUI) and **Android 10+** (API 29, Jetpack Compose).
Either one can watch any number of routers.

## Install

### From source

```sh
(cd ui && pnpm install && pnpm run build)   # the web console, which the binary embeds
cargo build --release
./target/release/agent-router
```

The console needs Node 22 or later and pnpm 10 to build (`npm i -g pnpm@10.33.0`); the
router itself needs only Rust. A release build without `ui/dist` stops and says so, rather
than ship a router without its console (`AGENT_ROUTER_ALLOW_NO_UI=1` builds one that serves a
placeholder instead), and a debug build serves that placeholder with a warning. Either way it
is still one binary: the console is compiled into it.

### As a VS Code extension

The extension bundles the router and the wrapper for your platform, keeps a current
router running, shows quota in the status bar, and opens the dashboard and the live
page in editor tabs. Build one with `scripts/package-vsix.sh` (see
[Building and packaging](#building-and-packaging)), then:

```sh
code --install-extension dist/agent-router-0.1.0-darwin-arm64.vsix
```

The extension is not the router. Every editor window runs its own copy of the
extension, so each one only makes sure a current router is serving on this machine —
`agent-router --ensure` exits when a same-or-newer build already answers, and takes the
port over without dropping a request when an older one does.

Connected to a hosted router instead (**Agent Router: Connect to a Server…**, which sets
`agentRouter.server` and keeps the password in the editor's secret storage), it runs the
machine agent rather than a router, and points Claude Code at the server. That is the
only way it runs on Windows.

## Quick start

**1. Run the router.** Leave it in a terminal, or background it.

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

**2. Point a project at it.** Create `.claude/settings.local.json` in the project:

```json
{
  "env": {
    "ANTHROPIC_BASE_URL": "http://127.0.0.1:8787"
  }
}
```

Add that file to the project's `.gitignore` — it is machine-local. The same three
lines work in every project; there is nothing per-project to configure. Both the CLI
and the VS Code extension read it.

**3. Use Claude Code normally.** Nothing else changes. Delete the settings file and
the project goes straight back to talking to Anthropic directly.

**4. Open the dashboard** at <http://127.0.0.1:8787>.

## The dashboard

Quota headroom for the active account with its history, the totals for whatever you
have scoped (prompt and output tokens, spend, cache hit rate, time to first token,
errors), activity over time stacked by model, what the model read from cache versus paid in
full, what filled the context window, the main-thread / subagent split, tokens per
project and per session, which tools flood the context, and every request as an
expandable row.

Click a project or a session and the whole page rescopes to it; the scope and the time
range (1h / 24h / 7d / 30d / all) are in the URL, so a link keeps its scope. It refreshes
every five seconds without flicker and works down to phone width. The live sessions are at
`/live` and the settings under `/settings`, in the same app: a React and IBM Carbon console
(`ui/`, built with pnpm as above), compiled into the binary and served by it, so there is
nothing to install beside the router and nothing is loaded from the network.

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
context window** — far more than the conversation. Run the "where the context actually
goes" query and see what your own split looks like.

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
router or of Claude Code.

An isolated profile cannot disturb your main login: a fresh `CLAUDE_CONFIG_DIR` reports
"not logged in" even with the default profile's `.claude.json` copied into it.

### Other providers' models

An account can also be at another Anthropic-compatible provider. The first is **Kimi
Code**: add its console key with the API (`"provider": "kimi"`), and its models (`k3`,
`k3-256k`, `kimi-for-coding`, `kimi-for-coding-highspeed`, as your plan allows) join the
router's catalogue. Each request goes to the account its model is assigned to, so switching
provider is switching model: `/model k3` in Claude Code, and `/model opus` to come back.
`GET /api/models` lists the catalogue and `POST /api/models/assign` gives a model to
another account of its provider. The plan is [docs/sources-and-models.md](docs/sources-and-models.md).

* **The picker.** Claude Code's `/model` picker lists only Claude's models by itself, so
  the installer and the machine agent add a row for each of the router's other models to
  `modelPicker` in `~/.claude/settings.json`. Your own rows are left as they are.
* **Background work follows the main model.** Titles and summaries run on the model the
  session uses, so a session on `k3` spends Kimi Code's quota on them too. To keep them on
  Claude, set `ANTHROPIC_DEFAULT_HAIKU_MODEL` to a Claude model id in your settings' `env`.
* Anthropic does not support Claude Code against non-Claude models through a gateway; Kimi
  Code's own guide points Claude Code at it directly, and it works in practice.

## Upgrading without dropping sessions

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
| `AGENT_ROUTER_DATA_DIR` | `~/.agent-router` | Where everything the router writes lives: the store, the secrets file, the archive shelf |
| `AGENT_ROUTER_DB` | `<data dir>/agent-router.db` | Store location, if it should live elsewhere |
| `AGENT_ROUTER_BODY_CAP` | `65536` | Bytes of each body kept; true size is always recorded |
| `AGENT_ROUTER_UPSTREAM` | `https://api.anthropic.com` | Upstream API |
| `AGENT_ROUTER_ACCESS_TOKEN` | *(unset)* | Use this token instead of the stored one |
| `AGENT_ROUTER_LOG` | `agent_router=info` | `tracing` filter |
| `AGENT_ROUTER_DUMP_HEADERS` | `1` | Requests to dump full headers for, at startup |
| `AGENT_ROUTER_BIND` | *(unset)* | An IP to also listen on, overriding the stored setting for this run. See below |
| `AGENT_ROUTER_SERVER_PASSWORD` | *(unset)* | A hosted router's password, pinned at every start. Server mode only |

`agent-router serve` takes the same settings as flags (`--port`, `--bind`, `--data-dir`,
`--db`), and flags win over the environment. `agent-router --help` lists them.

### Listening on your network

The router always answers on `127.0.0.1`. To let your phone — or a Claude Code on
another machine — reach it, turn on **Listen on your network** in the dashboard's
settings. It adds a second listener beside the loopback one, so it takes effect at
once: nothing restarts, and no session in flight is cut. The answer is stored, so it
survives a reboot whatever starts the router next — a login item is handed almost no
environment, which is why a variable alone was never enough.

A password must be set first; without one, callers from other machines are refused
anyway. `AGENT_ROUTER_BIND` still works and still wins, for a hand-launch or a dev box,
but it only lasts as long as that process.

`agent-router install` registers the login item. `--bind <addr>` writes the same stored
setting the switch does (`127.0.0.1` puts it back), and `--no-config` registers the
login item without touching Claude Code's settings.

## Running it as a server

`agent-router serve --server` runs the router as a hosted server: in a container,
behind an HTTPS ingress, for Claude Code on several machines, the phones and a browser.

```
agent-router serve --server --bind 0.0.0.0 --data-dir /data \
    --public-url https://agent-router.example.com
```

**What changes in server mode:**
- **One listener.** The router listens on `--bind` only. There is no handover, and no
  network switch.
- **Names.** It answers to the names it is given (`--public-url`, `--allow-host`) and to
  IP addresses, and refuses every other name.
- **The password.** Every caller presents the password, loopback included. It comes from
  `AGENT_ROUTER_SERVER_PASSWORD` at every start, and the dashboard cannot change it.
- **Accounts.** Accounts are `claude setup-token` values pasted into the dashboard. The
  server never runs `claude`.
- **Nothing of a developer's machine.** No `~/.claude`, no Keychain, no wrapper sockets.
  The settings that act on a machine (Claude Code setup, start at login, the wrapper) are
  not offered.
- **Live sessions come from the machines.** The live view, steering and prompt
  notifications need the machine a session runs on. Each machine runs the same binary as
  an agent that dials out to the router over one WebSocket and answers for its sessions;
  the router lists every machine's sessions, labelled with the machine, and sends each
  request to the one that holds it. A machine that goes offline reads as offline, not as
  closed.

**Joining a machine.** The router hands out its own agent. On the machine, run
`curl -fsSL https://<router>/agent/install.sh | sh` (macOS, Linux) or
`irm https://<router>/agent/install.ps1 | iex` (Windows); the live page shows both with
the router's address filled in. The script downloads the agent from the router and runs
`agent install`, and it installs this extension from the router too, into VS Code and
into the VS Code Server a Remote - SSH window runs on that machine; the extension takes
the set-up over and wires the wrapper so sessions can be typed into. See [docs/hosting-machine-agent.md](docs/hosting-machine-agent.md#connecting-a-machine).

`agent-router agent install --server <url>` asks for the password
and checks it against the server, points Claude Code at the server
(`ANTHROPIC_BASE_URL` and `ANTHROPIC_AUTH_TOKEN` under `env` in `~/.claude/settings.json`,
or in one project's `.claude/settings.local.json` with `--project <dir>`), and starts the
agent now and at every login. `agent-router agent uninstall` puts all of it back. The VS
Code extension does the same from its remote mode. By hand, the two variables are all
Claude Code needs; the agent is what the live view needs.

**Deploying it.** The image is built from the `Dockerfile`. The Helm chart is in
`charts/agent-router`. The pipeline that checks, builds, publishes and deploys both is
`.gitlab-ci.yml`. The design, the decisions behind it and the first-install steps are in
[docs/hosting-on-kubernetes.md](docs/hosting-on-kubernetes.md); the agent, its protocol
and the Windows port are in [docs/hosting-machine-agent.md](docs/hosting-machine-agent.md).

## Security

Agent Router holds a live subscription credential and signs everything it forwards,
so reaching it is kept as hard as it can be:

1. The socket binds to `127.0.0.1` only, unless you widen it deliberately — the switch
   in the dashboard, or `AGENT_ROUTER_BIND`.
2. The connecting peer must itself be on loopback — again, unless the bind was widened.
3. The `Host` header must parse as a loopback address, or a bare IP when the bind was
   widened. This is what defeats DNS rebinding — a web page that resolves its own domain
   to `127.0.0.1` so your browser will send credentialed requests here. Layers 1 and 2
   both *pass* in that attack, and rebinding needs a *name*, so bare IPs stay safe.
4. Only `/v1/` paths are proxied, and only for callers that already presented a
   credential of their own, so nothing can borrow the token by simply finding the port.
5. Anything that is not on this machine must present the **router password**.
6. A write whose `Origin` names another site is refused. That is a page elsewhere driving
   your browser, which would otherwise pass every check above with your saved password.

A hosted router changes layers 1, 3 and 5. It listens only where it was told. It answers
to the names it was given as well as to IP addresses. It asks everyone for the password,
loopback included, because in a container loopback is a port-forward, not you. Besides
health, only the agent's installer is open (`/agent/install.sh`, `/agent/install.ps1` and
the files they fetch): nothing in it is secret, and a machine being connected has nothing
to present yet.

Inbound `authorization` and `x-api-key` headers are discarded, never stored. Captured
bodies may contain your source code — the database is local and gitignored.

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

### Where tokens live

Tokens are never written to the database or to config. On macOS they are held in the
Keychain, read and written through `/usr/bin/security`, and the items the router creates
trust it (`-T /usr/bin/security`). That is a deliberate trade. A locally built binary is
ad-hoc signed, so every `cargo build` produces a new code identity, "Always Allow"
whitelists only the build that asked, and the prompt returns on the next rebuild — with
no button that makes it stop. The cost is that any process running as you can read those
items through `security` without a prompt, which is the same footing as Claude Code's own
credential entry. If you would rather have a per-binary ACL, sign the binary with a
stable identity (`codesign -s <your-cert> target/release/agent-router`) and the Security
framework path becomes practical again.

On Linux there is no Keychain; the router reads Claude Code's own
`~/.claude/.credentials.json`, which is that platform's existing store, and only ever
reads it.

## The phone apps

Two apps, the same job: watch any number of routers, and steer the sessions running
through them. `ios/AgentRouter/` is SwiftUI for iOS 26; `android/` is Jetpack Compose
for Android 10 and up. They talk to the router over the network bind and authenticate
with the router password — kept in the iOS Keychain, and on Android encrypted with an
AES-GCM key held in the AndroidKeyStore (hardware-backed where the phone has a TEE,
never extractable either way) into ordinary `SharedPreferences`.

**The dashboard in your pocket.** Router → **Usage** is the dashboard page — the range
buttons, the totals tiles, activity over time, what the model read, what fills the
context window, models, main thread vs subagents, accounts — with Projects, Sessions,
Tools and Requests beneath it, each narrowing the page when tapped. **Manage accounts**
and **Settings** are the settings modal: add an account (sign in through the router,
paste a token, or point at a profile), rename, rotate, remove; the login item, routing,
per-project routes, the wrapper per extension (and whether the router keeps it
installed on its own), and the password. Switching which account serves is the same
`POST /api/accounts/activate` the dashboard uses, so it takes effect on the next request.

**The `/live` page in your pocket.** Router → Live sessions lists every session on that
machine the way the web view does (by project, recent first, older and archived folded
away); open one and the feed streams as it happens — prose, tool calls and their output,
thinking, and questions.

A session marked **live** runs under the wrapper and takes a typed message, answers to
an AskUserQuestion from the form the prompt renders, and a Stop beside Send while it is
working — Send stays, because a message typed while a turn runs is queued by the session
itself and taken up when the turn ends (the status line counts what is waiting). One
marked **view** is read-only, which is what every session started before the wrapper was
installed (or after an extension update) is, until it is restarted.

The strip above the feed switches between a project's sessions, the clock button recalls
what you already sent, and the menu archives the session — or changes its **model** and
**effort** for the next turn, from the phone or from the web page's facts strip. The
model goes in as the editor's own `set_model` control request, and the CLI's confirmation
or refusal is what you see; effort is `/effort <level>` typed into the session. Both need
the wrapper.

### Side questions

The menu's **Ask on the side…** (the **btw** button on the web page) is `/btw` for a
session you are not sitting at: the CLI's own `/btw` is an interactive command the
editor's stream refuses, so the router does what it does instead — resumes the session
*forked*, with no tools and an instruction to answer briefly, and shows you the answer.
The session never sees the question or the answer; nothing is interrupted and nothing is
queued. It costs one replay of the session's context (mostly cached, so a few cents and
about ten seconds on a small session, longer on a big one), and the fork it leaves is
shelved so the list stays clean.

### The wrapper keeps itself installed

An extension update lands a fresh binary the wrapper is not in, and every session the new
version starts is view-only until it is. Settings → Wrapper → **Manage it automatically**
(dashboard) or **Manage automatically** (phone) has the router watch the extension
directory and wrap any version's binary it is not in — a fresh update included — once the
download has settled. The switch lives in the router's database, so once flipped it holds
across router restarts and app versions, and it is off until you turn it on. Sessions
already running keep the binary they started with; the reload the extension asks for
after an update is the only step left, and new sessions after it are live. The phone is
told each time it happens, and the Setup section keeps the log.

### Notifications

The router tells the phone the moment a session waits on a person — a permission, an
AskUserQuestion — once per prompt; a tap opens that session. Answer it anywhere else —
the terminal, the editor, another phone — and the notification takes itself down: the
prompt leaves the router's waiting set, and the router wakes the app with a silent push
naming it. A silent push is dropped whenever the app has been force quit, so the app also
asks `/api/push/pending` what is still waiting every time it comes to the front, and
clears up whatever the push missed.

Everything on this side is built in: both apps carry the push entitlement, the router
speaks to Apple (`src/apns.rs`) and to Google (`src/fcm.rs`), and the app tells the router
its team and bundle id when it registers. What cannot be built in is the platform's part.

**Apple** issues a push-enabled App ID and a signing key only to the account holder,
through the developer portal; no API or Xcode feature creates the key. Two one-time steps:

1. Sign into Xcode with the Apple ID (Xcode → Settings → Accounts). Automatic signing
   then registers the App ID with the Push Notifications capability and fetches a profile
   that carries it — a team's plain wildcard profile cannot. Until then,
   `PUSH=0 ios/AgentRouter/scripts/device-build.sh` builds and installs a variant without
   the entitlement (a build-setting override is not enough — Xcode also reads the
   capability off the project), and the app says so under Settings → Notifications.
   To keep the Apple ID out of Xcode altogether, do the same by hand in the portal —
   Identifiers → + → App → an explicit bundle id → Push Notifications → Register; then
   Profiles → + → iOS App Development → that App ID, the Mac's Apple Development
   certificate, the phone → Generate → Download, double-click it — and build with
   `PROFILE="<its name>" ios/AgentRouter/scripts/device-build.sh`.
2. Make one APNs key: developer.apple.com → Certificates, IDs & Profiles → Keys → + →
   tick Apple Push Notifications service → Register → Download. **Apple hands the `.p8`
   out exactly once**, so keep the file somewhere safe. Drop it, name unchanged
   (`AuthKey_<key id>.p8`), into `~/.agent-router/`: the router takes it in by itself —
   the key id from the file name, the team id from the app's first registration — and
   says so in the dashboard and under Settings → Notifications on the phone. Pasting it
   into either still works. Either way it stays in the router's secrets file and never
   comes back out. One key serves every app on the team, sandbox and production, and does
   not expire; the older per-app push certificate has none of those properties and is not
   supported.

**Google** is shorter, and free and re-issuable throughout: a Firebase project on the
Spark plan, `google-services.json` into `android/app/`, and a service-account key dropped
into `~/.agent-router/` — the router imports any `*.json` there whose `"type"` is
`"service_account"`, reads the project id out of it, and makes the file `0600`. Without
`google-services.json` the Android app still builds and runs, and says so under
Settings → Notifications.

Then Settings → Notifications → Enable on the phone, and "Send test".

See [docs/apple-push-requirements.md](docs/apple-push-requirements.md) and
[docs/google-push-requirements.md](docs/google-push-requirements.md) for the full
account of what each platform requires and why.

### Building the iPhone app

The Xcode project is generated from `project.yml`, so the `.xcodeproj` is not committed.
Two values are **not** kept in the repo and must come from the environment — your Apple
Developer team id, and the UDID of the device you are installing to:

```sh
export DEVELOPMENT_TEAM=<your team id>       # developer.apple.com → Membership
export DEVICE=<udid>                         # xcrun devicectl list devices

brew install xcodegen
cd ios/AgentRouter && xcodegen generate
scripts/device-build.sh                      # build, install and launch on $DEVICE
```

`xcodegen` expands `${DEVELOPMENT_TEAM}` in `project.yml` unless you pass `--no-env`.
Signing is automatic and needs no Apple ID in Xcode as long as the team's wildcard
development profile is on the Mac and lists the phone; see the comment in `project.yml`
for the one wrong turn that makes Xcode report "No profiles found".

Unit tests run on the simulator:

```sh
xcodebuild test -scheme AgentRouter -destination 'platform=iOS Simulator,name=iPhone 17 Pro'
```

Two suites talk to a real router. `LiveNetworkTests` covers the app's own networking,
stream included — set `TEST_RUNNER_ROUTER_BASE`, `_PASSWORD` and `_SESSION`, and run it
on the phone with `-destination 'platform=iOS,id=…' CODE_SIGNING_ALLOWED=YES` to cross
the password guard. The UI tests drive add-router → sessions → a chat against a router on
`127.0.0.1:8799`, and tap a notification carrying a router push's payload with the app in
front, in the background, and not running, checking that each lands on the session
(`--ui-test-notify` schedules it locally, since nothing can push to a simulator). One
more checks the opposite: a notification whose prompt the router no longer lists as
waiting is gone from Notification Center once the app has been to the front.

### Building the Android app

Gradle needs a JDK 17+ and an Android SDK with `compileSdk 37`. `scripts/device-build.sh`
falls back to the JDK that Android Studio ships and writes `local.properties` for you:

```sh
cd android
scripts/device-build.sh            # build, install and launch on whatever adb sees
scripts/device-build.sh test       # unit tests only
DEVICE=<serial> scripts/device-build.sh
```

Firebase is optional — see Notifications above.

## Building and packaging

The router builds for five platforms and ships as a VS Code extension per platform, with
the router and the wrapper inside the package:

```sh
scripts/build-target.sh aarch64-apple-darwin        # or x86_64-apple-darwin,
scripts/package-vsix.sh darwin-arm64                #    x86_64-unknown-linux-musl,
                                                    #    aarch64-unknown-linux-musl,
                                                    #    x86_64-pc-windows-gnu (win32-x64)
```

`build-target.sh` builds the web console first, once, with pnpm (`ui/dist`, which every
binary embeds), unless it is already there. Linux builds are static (musl). From a Mac they
cross-build through `cargo zigbuild` (`brew install zig && cargo install cargo-zigbuild`) or
a musl cross compiler; on Linux they build natively. Windows builds are the GNU ABI and
cross-build the same way, through zig or mingw-w64. macOS builds are native on a Mac and
cross-build from anywhere else through zig, with no Apple SDK: the programs link only
libSystem and libiconv.

Claude Code is pointed at the router through its settings file and launched through the
wrapper by way of Claude Code's own `claudeCode.claudeProcessWrapper` setting, so nothing
inside Anthropic's extension is ever touched.

`.gitlab-ci.yml` does all of them on a tag — the console once, in its `build:ui` job, which
hands `ui/dist` to the rest; the Mac, Linux and Windows jobs in Docker on Linux, through
zig — puts
the VSIX files in the package registry, makes a release, and publishes to the
Marketplace and Open VSX when the `VSCE_PAT` and `OVSX_PAT` variables are set. The
Windows package stays off the marketplaces until it has run on Windows. The extension's manifest lives in
`vscode/`; its version follows `Cargo.toml`. **Change the `publisher` id in it to your
own** before publishing — it must match the Marketplace publisher you create.

## Findings and research

Research and decisions that are not descriptions of the code — what Apple and Google push
need, how the router can ship as a VS Code extension and what that costs, where the
subscription quota actually goes — live in [docs/](docs/README.md), one dated document
per finding with its sources.

## Current limitations

* **Adding an account needs a terminal once.** Claude Code's login is a terminal
  program and produces no output when driven headlessly, so the router cannot run it
  inside the page. It sets everything up either side of that one command and detects
  when you finish.
* **Context composition is an estimate.** Totals are exact; the split across system /
  tools / history / tool results apportions that exact total by character share,
  because there is no local tokenizer for current Claude models.
* **Windows is untried.** The agent and the wrapper build for Windows and pass their
  checks on a Mac, and have not run on Windows yet. See [Platforms](#platforms).
* **The agent is untried on Linux.** It shares its code with macOS and starts at login
  from a systemd user unit, but has not run on Linux yet. That unit stops at logout
  unless the user lingers (`loginctl enable-linger`), which a machine reached over SSH
  needs.

## License

MIT — see [LICENSE](LICENSE).
