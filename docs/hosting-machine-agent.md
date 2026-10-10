# The machine agent: live sessions through a hosted router

2026-10-05. Design for phase 2 of [hosting](hosting-on-kubernetes.md). A hosted router
proxies and records every machine's traffic, but it cannot see the sessions themselves.
They are transcripts, a session registry and wrapper sockets on each developer's
machine, and a pod cannot open a connection into a laptop. The agent is the piece that
can: the same binary, run on each machine, connecting out to the router.

Written for the maintainer, and as the contract the pieces of phase 2 are built against.
Built the same day; [Status](#status) says what has run where, and what still has to be
tried on a Windows machine.

## Decisions (owner, 2026-10-05)

| Question | Answer |
| --- | --- |
| How an agent proves itself | The router password, which every machine already holds for Claude Code (`ANTHROPIC_AUTH_TOKEN`). No per-machine tokens. |
| What keeps it running | A login item, from `agent-router agent install --server <url>` (launchd on macOS, a launcher in the Startup folder on Windows). The VS Code extension also starts it when a window opens. |
| Platforms | macOS and Windows. Windows ships with both following and steering working, not one before the other. Linux later. |
| How a machine gets the agent (2026-10-07) | From the hosted router itself: `install.sh` and `install.ps1`, which fetch the agent and the editor package from the router's own image. No release or store is needed. Windows ships on every route, though it has not run on Windows yet. |
| Linux, for a dev host (2026-10-07) | The agent's login item is a systemd user unit, `agent-router-agent.service`, beside the router's own `agent-router.service`. It runs `agent --ensure`, logs where the editor reads, and restarts only after a crash. |

## The shape

```
phone / browser ──https──▶ hosted router ◀──wss, outbound── agent ── wrapper ── claude
                            (pod)                            (each developer machine)
```

- **The agent connects out.** It keeps one WebSocket to the router at
  `wss://<host>/_router/agent`, authenticated with the router password, and reconnects
  with backoff when it drops. It listens on nothing.
- **The router forwards live requests to the agent.** Each request for a session (follow
  it, answer a prompt, send a message, stop, change the model) goes down that socket
  as an HTTP request. The agent runs it against the same `live` handlers a local router
  serves, in-process. The response, event streams included, comes back up the same
  socket.

This needs no new handler code: every `live` handler is stateless, and takes only its
query or JSON body plus the machine it runs on.

**What the router does itself:**
- **Session list.** It asks every connected agent for its list and merges the answers,
  labelling each session with its machine. It remembers which machine holds which
  session.
- **Per-session routing.** Session ids are UUIDs, so they are a global key. The router
  sends each per-session request to the machine holding that session.
- **Prompt notifications.** It decides when to announce a waiting prompt and when to take
  the banner down, as today. The evidence comes from the agents.
- **The archive shelf.** The shelf lives on the router, so every phone sees the same one.

**Offline machines.** A machine whose agent is offline reads as offline, never as closed:
- its sessions leave the list;
- requests for them answer 503, naming the machine;
- its banners stay on the phones until it comes back and says otherwise.

## Connecting a machine

A hosted router hands out its own agent, so a machine needs nothing but the router's
address and password:

```
curl -fsSL https://<router>/agent/install.sh | sh     # macOS and Linux
irm https://<router>/agent/install.ps1 | iex           # Windows, in PowerShell
```

The `/live` page shows both, with the router's address in them, until a machine connects.

- **What the script does.** It downloads the agent for the machine's platform from the
  router and runs `agent-router agent install --server <the router>`, which asks for the
  password. It also installs the Agent Router extension from the router: into VS Code
  through its `code` command, and into the VS Code Server that a Remote - SSH window from
  another computer runs on this machine (`$VSCODE_AGENT_FOLDER`, by default
  `~/.vscode-server`), through the server's own `code-server`. A headless machine has
  only the server; a window open at the time picks the extension up when reloaded.
  `sh -s -- <options>` passes options on to `agent install`.
- **The extension takes the set-up over.** Its first window finds the server and the
  password in `agent.json` and switches to remote mode instead of starting a router: the
  password goes into secret storage, `agentRouter.server` is set, and Claude Code is
  handed the wrapper, which is what lets the phone type into a session.
- **Without VS Code** a machine's sessions can be followed but not typed into: `claude`
  in a terminal is not wrapped.
- **Open without the password.** The two scripts and the files they fetch are the only
  addresses besides `/_router/health` that a hosted router answers without it
  (`src/downloads.rs`). Nothing in them is secret, and a machine being connected has
  nothing to present yet.
- **The router's own build.** The image carries every platform's agent and editor package
  from the pipeline that built it (`scripts/stage-agents.sh`, then the Dockerfile), in
  `/usr/local/share/agent-router/agents` beside the router, so an agent always speaks its
  router's protocol. `--agents-dir` names another directory.

## The agent

```
agent-router agent --server <url> [--label <name>] [--data-dir <dir>]
agent-router agent --ensure            start only if no agent's keeper runs here
agent-router agent install --server <url> [--project <dir>] [--no-service] [--no-config]
                                       [--no-library] [--no-auto-update]
agent-router agent uninstall
agent-router agent status
```

**The password.** It comes from `AGENT_ROUTER_PASSWORD`, or from `<data-dir>/agent.json`,
which `install` writes. That file holds `{server, password, machine, label}`, is
owner-only (0600), and on Windows sits in the user's profile.

**The machine id.** A random UUID made on first run and kept in `agent.json`. It
survives a rename. The label defaults to the host name.

**`install` does five things:**
1. Checks the password against `GET <server>/api/auth`.
2. Writes `agent.json`.
3. Retires the local router's login item, on a machine that ran the router itself. That
   item starts `bin/agent-router`, the path the agent's binary is about to take, so at
   the next login it would start an empty local router. It is taken out of launchd (or
   systemd, with its shell-profile line) and its file kept in the data directory as
   `*.retired`.
4. Points Claude Code at the server: `env.ANTHROPIC_BASE_URL` and
   `env.ANTHROPIC_AUTH_TOKEN` in `~/.claude/settings.json`, or in one project's
   `.claude/settings.local.json`. The bookkeeping is the same as `install` uses today,
   extended to two keys: touch only our own keys, adopt an existing value, and leave a
   changed one alone.
5. Registers the login item.

`uninstall` reverts all of it but the retired login item, which stays retired: the
local `agent-router install` registers it again.

**Single instance.** `<data-dir>/agent.lock` is held by the agent's keeper, and
`<data-dir>/agent.pid` says who holds it: its pid, its build and `keeper`. The lock file
itself stays empty, because Windows refuses to read a file another process holds locked.
`--ensure` exits when a keeper holds the lock, whatever its build, since a keeper keeps its
agent at the right build itself (see [Upgrading an agent](#upgrading-an-agent)). An agent
from before keepers wrote its pid and build into `agent.lock`; it is asked to stop, then
replaced. On Windows such an agent cannot be named, and a keeper takes its place at the
next logon.

**What it runs:**
- **Request routes.** Only these answer tunneled requests:
  - the `live` routes;
  - two for the notifier, `GET /agent/waiting` and `GET /agent/still-waiting`;
  - one for the Library, `POST /agent/library/sync`
    ([library.md](library.md#push)). It syncs now and answers with its report.

  That last route is the one through which a router can cause a write on a machine. It
  carries nothing: the agent pulls the manifest and blobs over HTTPS and checks every
  digest. So all it can place in `~/.claude` is what the router's Library lists. A machine
  installed with `--no-library` answers it 409. Nothing else is reachable through the
  tunnel: no setup, no shim install, no files.
- **Housekeeping.** The wrapper janitor runs every 60 s, as the router does locally today.
- **Clock.** It derives activity on its own clock. Rows carry `as_of_ms` and the router
  never re-derives them, because a few seconds of clock skew would misread turn
  boundaries.

### Upgrading an agent

A new build takes over in place, on the machine alone: the router and its rollout are not
involved, and the machine never shows offline.

**The keeper and its worker.** What the login item and the editor start (`agent`,
`agent --ensure`) is the keeper. It holds the lock, runs no tunnel, and keeps one worker
running: `<data-dir>/bin/agent-router agent --worker`, started from the installed binary
as it is at that moment. The worker is the agent described above: the tunnel and
everything it serves.
- The worker's standard input is a pipe the keeper never writes to. The worker stops when
  that pipe closes, which is how the keeper tells it to go, and what happens when the
  keeper goes away, whatever stopped it.
- On its standard output the worker says `ready <build>` once its tunnel is first up, and
  `connected` at each reconnect after. Its logs go to standard error, into `agent.log`.
- A worker that exits on its own is started again, waiting from 1 s up to 60 s.

**The handover.** Every 5 s the keeper looks at `bin/agent-router` (its size and time,
then its sha256 when those change). When it is a new build:
1. The keeper starts a second worker from it.
2. It waits up to 30 s for that worker's `ready`. The router routes to a machine's newest
   tunnel the moment it connects, and the older one's closing neither unregisters it nor
   marks the machine offline (`machines::Machines::serve`).
3. It closes the first worker's pipe, and kills it after 5 s.
4. It logs `handed over from <build> to <build>`.

What the first worker was relaying at that instant fails with "the machine reconnected",
and a stream someone is following reconnects. A worker that is not up in time is stopped,
and the first one carries on; that build is tried again after ten minutes, in case what
stopped it was the router being away. Two workers may overlap for a moment: the Library
sync holds its own lock, and a places report says the same thing twice.

The keeper itself stays on its build until it is next started (a login, a reboot, or the
installer replacing an agent from before keepers). It does nothing but this.

**Self-update.** At start, at each worker reconnect (a router deploy replaces the pod, so
every worker reconnects), and hourly, the keeper reads `/_router/health`'s build. Once per
router build:
1. It fetches `/agent/<platform>/agent-router`, which the router already serves for its
   installer.
2. When its sha256 differs from `bin/`'s, it writes it beside as `agent-router.new-<pid>`,
   runs it once with `--build` (it must exit 0 within 10 s and say a build, which catches a
   wrong architecture or a binary macOS will not start), and renames it over `bin/`'s,
   never writing over a binary in place.
3. It records the router build in `agent.json` (`checked_build`).

The handover then takes it from there. Only the installed agent updates itself: a keeper
not running from `bin/` (one run from a build directory) never does, and neither does a
machine installed with `--no-auto-update`, which writes `"update": false` into `agent.json`
(`--auto-update` undoes it).

**The installer and the editor.** Re-running the installer puts the new binary in `bin/`
by rename; with a keeper running, the login item is rewritten but not restarted, and the
keeper hands over. An agent from before keepers is restarted once (launchd `bootout` and
`bootstrap`, systemd `restart`, the Windows Startup script). In remote mode the VS Code
extension installs its bundled agent only where none is installed, so it never copies an
older build over one the keeper keeps current.

## The tunnel

One WebSocket, binary frames. Each frame is `kind: u8`, `id: u32` (big-endian, the
request it belongs to), then the payload:

| kind | direction | payload |
| --- | --- | --- |
| 1 `HELLO` | agent → router | JSON `{machine, label, platform, version, build}`; id 0, first frame |
| 2 `REQUEST` | router → agent | JSON `{method, path, headers: [[name, value]], body}`; path includes the query, body is base64 |
| 3 `HEAD` | agent → router | JSON `{status, headers: [[name, value]]}` |
| 4 `BODY` | agent → router | raw bytes, at most 64 KiB per frame |
| 5 `END` | agent → router | empty: the response is complete |
| 6 `CANCEL` | router → agent | empty: nobody wants this response any more (a viewer closed the page) |
| 7 `FAIL` | agent → router | UTF-8 text: the request failed before any `HEAD` |

**Requests.** The router numbers them; ids are not reused while live.

**Bodies.** A wrapper frame can be up to 1 MiB, so a request body can too. The WebSocket
message limit is 4 MiB.

**Keepalive.** The router pings every 20 s. A side that hears nothing for 60 s closes
the socket, and the agent reconnects with backoff (1 s, then up to 30 s, with jitter).

**The ingress.** It passes WebSocket upgrades as it is (ingress-nginx does by default).
The `proxy-read-timeout` already documented for event streams also covers this socket,
and the pings keep it under that timeout.

**Authentication** is the router's guard, unchanged: `Authorization: Bearer <password>`
on the upgrade request. Only a hosted router serves `/_router/agent`; a local router
answers it 404.

## What the router answers

| Route | Hosted router with agents |
| --- | --- |
| `GET /api/live/sessions` | Every connected agent's list, merged and filtered by the shelf. Each row gains `machine: {id, label}`, and carries `repository` from its agent where the session's directory has a git remote. Old phones ignore both keys. |
| `GET /api/live/stream`, `/stats`, `/prompt` | Tunneled to the session's machine. A stream is relayed byte for byte. |
| `POST /api/live/send`, `/interrupt`, `/answer`, `/model`, `/effort`, `/mode`, `/aside` | Tunneled to the session's machine. |
| `GET /api/live/project?project=<key>` | Every agent's answer for that project, summed: the sessions whose repository is the key, else whose directory is. One repository's clones and worktrees on every machine add up. |
| `POST /api/live/archive` | On the router: the shelf. |
| `GET /api/push/pending` | The waiting keys of connected agents, plus the announced keys of machines that are offline. |
| `POST /api/machines/places` | Each agent's report, at every connection and every minute: the sessions it lists (Claude Code's from its registry and transcripts, Kimi Code's from its session files and hooks, `src/kimi/session.rs`), the directory each is in, and each directory's git remotes, credentials taken out (`src/places.rs`), and the coding clients the machine has: Claude Code and Kimi Code, each as a command, an editor extension and a home folder, with versions (`src/clients.rs`). It places the sessions on that machine and gives each its repository, which is its project (`sessions.project`). `scripts/backfill-repositories.py` posts the same report for past sessions, by hand. |
| `GET /api/machines` | New: every machine seen, with label, platform, build, online, and since when, `library`: where its Library stands ([library.md](library.md#each-machines-state)), and `clients`: the clients it last reported. |
| `POST /api/library/sync` | Tells every connected agent, or `{machine}`, to sync the Library now, through its tunnel. |

**Errors:**
- A session no agent knows answers 404.
- A session whose machine is offline answers 503: "the machine running this session
  (<label>) is offline".
- With no agent connected at all, the list is `[]`, and the pages say how to install one.

## Windows

Everything above has to work on Windows, steering included. Six things are Unix-only today.

1. **The router crate must build for Windows** (`x86_64-pc-windows-msvc`, or `-gnu`
   from the Mac). Agent mode is the only mode it runs there. Local and server mode refuse
   with a sentence; they do not compile in half-working form.
2. **Process liveness.** The `registry::Procs` trait gets a Windows implementation:
   - liveness from `OpenProcess` plus `GetExitCodeProcess`;
   - start time from `GetProcessTimes`, matched against the record's `procStart`;
   - the parent pid from a toolhelp snapshot.
3. **File identity.** The live view keys transcripts by `(dev, ino)`. Windows keys them
   by volume serial and file index (`GetFileInformationByHandle`), not by creation time:
   NTFS gives a file made under a name deleted moments before the old file's creation
   time.
4. **The wrapper's control transport.** A named pipe,
   `\\.\pipe\agent-router-<user>-<session>`, carries the same frames, the same
   whole-frame-or-nothing rule and the same `{ok}` receipt.
   - `.pending` and `.ctl` live in `%LOCALAPPDATA%\agent-router\sessions\`. There is
     no `.sock`: a session is wrapped while its pipe name is held.
   - The pipe admits only its own user, and an elevated editor's only elevated processes,
     so steering an editor run as administrator needs an agent run the same way.
   - Before writing, the agent checks that the process serving the pipe runs as the same
     user, by its token's SID, and it connects at identification level: a pipe whose name
     another account took never receives what someone typed, and cannot act as the agent.
   - A stop is the same `interrupt` control request as on macOS, never a console control
     event, which would reach every process on that console.
   - Installed in place of the extension's binary, the wrapper parks the real one as
     `claude.real.exe`, a name that is still a program.
5. **Paths.** `%USERPROFILE%\.claude`, `claude.exe`, and the extension's
   `native-binary\claude.exe`.
6. **Packaging.**
   - A `win32-x64` VS Code package carries `agent-router.exe` and `claude-wrapper.exe`,
     cross-built from a Mac or the Linux runner through zig (`build:windows` in CI). It
     ships wherever the other packages do, and a hosted router hands it out
     (`install.ps1`).
   - The login item is a script in the user's Startup folder, `Agent Router agent.vbs`,
     that starts `agent-router.exe agent --ensure` without a console window. Unlike a
     logon task it needs no administrator rights, and it touches no registry. It is
     written in UTF-16, which Windows Script Host reads whatever the profile folder is
     called. Where policy turns Script Host off, the editor still starts the agent.

## The clients

- **VS Code extension.**
  - It runs on the machine Claude Code runs on (extension kind `workspace`): in a
    Remote - SSH window, the remote one. Claude Code reads `claudeProcessWrapper` from
    that machine's settings, so an extension on the client would hand it a path that
    exists only on the client, and no session there could start.
  - A remote mode: `agentRouter.server` (a URL) plus the password in the editor's secret
    storage.
  - In remote mode it installs and supervises the agent instead of a router
    (`agent-router agent --ensure`).
  - It keeps `claudeCode.claudeProcessWrapper`.
  - It reads quota from the server with the password, and opens the pages in the
    browser.
- **Phones and `/live`.** They show the machine label on each session, and group by
  machine when there is more than one.
- **Northstar.** It needs a URL and password pinned from its deployment. That is handed
  to a Northstar session separately.

## Build order

1. **The tunnel and the router side, built and tested on macOS.** The agent mode, the
   protocol, the merged list, per-session routing, notifications from agents, and the
   machine label.
2. **`agent install` and the login items.** Then the extension's remote mode.
3. **Windows.** The crate for Windows, liveness and identity, the wrapper on named
   pipes, the login item, and the `win32-x64` package.
4. **The machine label on the phones.**

Each step is tested on its own: unit tests for the frame codec, and a real router with a
real agent over loopback. The Windows pieces build on the Mac with a cross toolchain and
run on the Windows machine.

## Status

**On macOS, tried end to end** with a router in server mode and an agent over loopback:
- the merged list, stats, prompts, an event stream relayed as it is written, sending, the
  shelf, and pending prompts;
- a session nobody holds (404), a machine gone offline (503), and a wrong password;
- `agent install`, `status`, `--ensure` and `uninstall`, with Claude Code's settings put
  back as they were and the password kept only as a fingerprint.

**The phones** show the machine on each session. iOS is installed; Android is built and
waits for a device.

**On Windows, built and not yet run.** Both programs cross-build and pass clippy, and
the wrapper's own tests compile for Windows. On the Windows machine, in this order:

1. `cargo test -p claude-wrapper`: 52 unit tests and 9 lab tests (build the lab's fake
   CLI first with `cargo build -p claude-wrapper --example fake_cli`).
2. Claude Code's records: `%USERPROFILE%\.claude\sessions\<pid>.json` and
   `projects\<slug>\<uuid>.jsonl` in the shapes macOS writes, and how a Windows path is
   slugged. The session list is built from them.
3. The extension honours `claudeCode.claudeProcessWrapper`, and the arguments survive
   being quoted again: compare `Get-CimInstance Win32_Process -Filter
   "Name='claude.exe'" | select CommandLine` for a wrapped and an unwrapped session.
4. The pipe is there under the expected name:
   `[IO.Directory]::GetFiles('\\.\pipe\') | ? { $_ -like '*agent-router-*' }`.
5. From a phone through the hosted router: follow a session, send a message, stop a
   turn, answer a permission prompt (its `.pending` goes), then `/clear` (the pipe and
   the records move to the new id).
6. Closing panels: an idle one leaves nothing behind; a busy one leaves no `claude.exe`
   two seconds later; no console window flashes.
7. Access: only the user can open the pipe (`accesschk -v \pipe\agent-router-…`);
   another account is refused; with the editor run as administrator, an ordinary agent
   is refused and says so.
8. The login item: `agent install --server <url>` writes the Startup script and starts
   the agent hidden; it comes back after logging off and on; `agent uninstall` removes
   it and stops the agent.
9. Defender lets the unsigned, zig-built programs run.
