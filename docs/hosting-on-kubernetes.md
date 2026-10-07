# Hosting Agent Router on Kubernetes

2026-10-05. Running the router as a hosted server, the way Northstar is hosted: a
distroless image, a Helm chart and a GitLab pipeline.

- **Phase 1** is built: server mode, the image, the chart and the pipeline. This document
  specifies it in detail.
- **Phase 2**, a per-machine agent for the live features, is outlined at the end.

Written for the maintainer. What the code does today is in [README.md](../README.md) and
[ARCHITECTURE.md](../ARCHITECTURE.md).

## Summary

- The router was built on one premise: it runs on the machine where Claude Code runs. A
  read of the whole tree on 2026-10-03 found 110 places that depend on it. They include
  loopback trust, `~/.claude` transcripts, `/tmp` wrapper sockets, the Keychain,
  `claude` subprocesses, launchd, and the port handover.
- **What a pod can serve on its own:** proxying and recording `/v1` for every machine,
  the dashboard and its analytics, search, quota and the forecast, token accounts, the
  password, push delivery, and project attribution taken from the request itself.
- **What needs a component on each developer machine (phase 2):**
  - the live session list, and following a session;
  - steering: send, answer, stop, and changing model, effort or mode;
  - `/btw`, prompt pushes, wrapper management, and Northstar's chat view.

  A pod cannot connect into a laptop, so that component dials out.
- **What is meaningless on a server:**
  - Claude Code's own Keychain login, profile accounts, and sign-in driven by the router;
  - the network switch, the port handover and login items;
  - writing `settings.json`.
- **Phase 1 adds `agent-router serve --server`.** Local mode (no arguments, what the Mac
  and the VS Code extension run) behaves as before, apart from three deliberate fixes that
  apply in both modes: forwarded headers are stripped, cross-site writes are refused, and
  project attribution falls back to the request.

## Owner's decisions (2026-10-03)

| Question | Answer |
| --- | --- |
| Who uses the hosted router | The owner alone: one password, one active account, every phone gets every prompt. Each developer machine gets its own identity (phase 2). |
| Scope | Phase 1, a hostable server, comes first. Phase 2, the per-machine agent, follows. Each phase is reviewed before the next starts. |
| Platforms for the live features | macOS and Windows first. Linux later. |
| Store | SQLite on a persistent volume. No Postgres. |
| Accounts on a server | Setup-token accounts only, pasted into the dashboard. The server never runs `claude`. |
| The password | Pinned from a Kubernetes Secret at every start, as in Northstar's ADR 0022. |

## Why the router assumed it was local

What a pod would hit today, in the order it hits them:

1. **No `serve` subcommand and no flags.** Configuration is environment-only, and
   `agent-router serve` exits 2.
2. **`HOME` decides where data lives.** With `HOME=/tmp` (an emptyDir), the store, the
   token file and the push keys vanish at the first restart.
3. **Loopback only.** The router listens beyond loopback only when an env var or a stored
   setting says so.
4. **The Host guard refuses every name.** Every request through the ingress, and every
   in-cluster call by Service name, gets 403.
5. **The first password can only be set from loopback.**
6. **The built-in "default" account is Claude Code's own login.** It needs the `claude`
   CLI and a Keychain or `~/.claude/.credentials.json`, so every `/v1` request answers 502.
7. **APNs signing shells out to `/usr/bin/openssl`.** The distroless image has no
   `openssl`.
8. **Ingress defaults break the proxy.** The 1 MiB body limit and the 60 s read timeout
   cut `/v1` and the event streams.

Where each feature can run once hosted:

| Feature | Pod alone | Machine agent (phase 2) | Off on a server |
| --- | --- | --- | --- |
| Proxy `/v1`, capture, analytics, search, quota, forecast, pricing | ✓ | | |
| Token accounts, the password, health, the pages themselves | ✓ | | |
| Push delivery, device registration, test push | ✓ | | |
| Project and branch attribution | ✓ (parsed from the request) | (or reported) | |
| Live list, following a session, session totals for effort and mode | | ✓ | |
| Send, answer, stop, model, effort, mode, `/btw` | | ✓ | |
| Prompt pushes and banner reconciliation | | ✓ (evidence) + pod (delivery) | |
| Wrapper install and auto-wrap, Claude Code settings, login item | | ✓ | ✓ |
| Keychain and profile accounts, router-driven sign-in | | | ✓ |
| Listen-on-your-network switch, port handover, `--ensure` | | | ✓ |

Project attribution was already broken locally for most traffic: 108 of the 133 sessions
since 2026-09-24 had no project, because SDK sessions and the LAN machines leave no
transcript on the Mac. The request carries what is needed: Claude Code's environment
block (` - Primary working directory: <path>`) and git status (`Current branch: <name>`).
These sit in the request's `system` field or in its first user message.

## Phase 1: server mode

### Running it

```
agent-router                        run locally, exactly as before
agent-router serve [OPTIONS]        the same, with options

  --server                 hosted mode (everything below in "Server mode")
  --bind <ip>              local: also listen here, like AGENT_ROUTER_BIND
                           server: the only listener; required
  --port <n>               default 8787 (AGENT_ROUTER_PORT)
  --data-dir <dir>         where everything the router writes lives
                           (AGENT_ROUTER_DATA_DIR; default ~/.agent-router)
  --db <path>              the store (AGENT_ROUTER_DB; default <data-dir>/agent-router.db)
  --allow-host <name>      server: a name clients reach the router by; repeatable
  --public-url <url>       server: the https URL clients use; its host is allowed too
  --push-keys-dir <dir>    server: read-only directory holding AuthKey_<id>.p8 and/or a
                           Firebase service-account JSON (a mounted Secret)
  --drain-seconds <n>      server: how long SIGTERM waits for open requests (default 20)
```

- **Precedence.** Flags override env, and env overrides stored settings.
- **Unknown options.** An unknown option exits 2, so a binary that predates a flag
  refuses the chart's arguments instead of ignoring them.
- **The password variable.** `AGENT_ROUTER_SERVER_PASSWORD` is required in server mode and
  must be at least 8 characters. No client presents a variable of this name, so a shell
  that exports it for some other purpose cannot set a server's password by accident.

The data directory gets one root. Everything the router writes hangs off `--data-dir`:

- `credentials.json`
- `archived.json`
- `accounts/`
- `managed.json`
- `bin/`
- `logs/`
- dropped key files

`AGENT_ROUTER_DB` alone still moves only the store, as it does today, so test instances
and the live Mac router are unaffected.

### Server mode

**Listening.**
- One listener, on `--bind` (`0.0.0.0` in a pod).
- A failed bind stops the start.
- Off-host mode is fixed at startup, not flipped by the network switch.
- None of the local machinery runs: no separate loopback listener, no `SO_REUSEPORT`
  handover, no incumbent probe.
- `POST /api/network` refuses.

**Trust.**
- **Names.** The Host guard admits:
  - the `--allow-host` names and the `--public-url` host, matched exactly and
    case-insensitively, with the port ignored and one trailing dot dropped;
  - IP literals, so kubelet probes and `helm test` by IP pass;
  - `localhost`, for `kubectl port-forward`.

  Any other name gets 403, naming what the router does answer to.
- **No address is exempt from the password**, loopback included. In a pod, loopback means
  a port-forward or a mesh sidecar, not the owner at the keyboard. `/_router/health` stays
  open.
- **The password is pinned at every start.** If the stored record already verifies the
  variable, it is kept. Otherwise it is replaced.
- **`POST /api/auth` answers 409 while the password is pinned:** "the password is set by
  the deployment (AGENT_ROUTER_SERVER_PASSWORD); change the Secret and restart".

**Accounts.**
- No `default` account is seeded or activated, and no `claude` process is ever started:
  not at startup, not to list accounts, not to probe, not to sign in.
- `POST /api/accounts` accepts only `source: "token"`. The first account a store gets
  becomes the one that signs: a hosted store starts empty, with no discovered default to
  hold that place.
- `POST /api/accounts/activate` refuses an account that is not a token, so a Keychain
  row in a copied store can never become the one that signs.
- `POST /api/accounts/probe` and `/api/login/start` refuse.
- A `default` row in an imported store reads "not available on a hosted router" and can be
  removed.
- With no account to sign with, `/v1` answers 503 "no account to sign with — add one in
  the dashboard".

**Push.**
- APNs keys are handled in-process, in both modes: the public half that Apple's `.p8`
  omits is derived with the `p256` crate, and `/usr/bin/openssl` is no longer used.
- Key files are also read from `--push-keys-dir`. It is read-only, so the importers no
  longer require `chmod` to succeed.
- No wrapper janitor and no waiting-prompt sweep run; phase 2 feeds both.

**Live.**
- `GET /api/live/sessions` answers `[]`.
- Every other per-session route answers 503 "live sessions need the Agent Router agent on
  the machine that runs them; a hosted router does not have one yet".
- `/api/live/options` is unchanged.
- `/api/push/pending` answers `{"waiting": []}`.

**Shutdown.**
- The SIGTERM handler is installed first.
- On SIGTERM the router stops accepting, waits up to `--drain-seconds` for open requests,
  then gives capture 5 s. That fits inside the pod's 30 s grace period.

### How each changed route answers in server mode

The installed iPhone app requires every field of `/api/auth`, `/api/setup`,
`/api/setup/shim` and `/_router/health`, so these always return their full shape. The
Android app decodes leniently.

| Route | Server mode | Local mode |
| --- | --- | --- |
| `GET /api/auth` | adds `mode: "server"`, `password_pinned: true`, `public_url`. Also `remote_bind: true`, `lan_url: <public_url>`, `env_override: false` | adds `mode: "local"`, `password_pinned: false`, `public_url: null` |
| `POST /api/auth` | 409 while pinned | unchanged |
| `GET /api/setup` | `{service: {registered: false, supported: false, platform: "server"}, url: <public_url or "">, global_settings_path: "", managed: [], mode: "server"}`, with nothing written | unchanged |
| `POST /api/setup/service`, `/api/setup/config` | 409 `{error: "…not on a hosted router…"}` | unchanged |
| `GET /api/setup/shim` | `{available: false, installed: 0, targets: [], auto: false, auto_log: []}` | unchanged |
| `POST /api/setup/shim`, `/api/setup/shim/auto` | 409 | unchanged |
| `POST /api/network` | 409 | unchanged |
| `POST /api/login/start` | 409 | unchanged |
| `GET /api/login/status` | `{}` (no flow) | unchanged |
| `POST /api/accounts` | 409 unless `source: "token"` | unchanged |
| `POST /api/accounts/activate` | refuses an account that is not a token | unchanged |
| `POST /api/accounts/probe` | 409 | unchanged |
| `GET /api/live/sessions` | `[]` | unchanged |
| other `/api/live/*` except `options` | 503 `{error}` | unchanged |
| `GET /api/push/pending` | `{"waiting": []}` | unchanged |
| `GET /_router/health` | adds `build` | adds `build` |

### Changes that apply in both modes

- **Forwarded headers stripped.** Before a request goes upstream, `X-Forwarded-*`,
  `X-Real-IP`, `Forwarded`, `Via` and `X-Request-ID` are removed. An ingress adds them,
  and they would carry developers' addresses to Anthropic.
- **Cross-site writes refused.** A request that is not `GET`, `HEAD` or `OPTIONS` and
  carries an `Origin` whose host and port differ from the `Host` header gets 403. A
  `null` origin is refused too. The dashboard, `/live` and VS Code's Simple Browser are
  same-origin. The phones, Claude Code and Northstar send no `Origin`.
- **Cached credential dropped on 401.** An upstream 401 drops the cached signing
  credential, so the next request re-resolves it.
- **Project attribution falls back to the request.** When no transcript is found, cwd
  and branch come from the request:
  - **Where it looks:** the `system` field first, then the text blocks of the first user
    message.
  - **Directory line:** exactly ` - Primary working directory: <path>`, or a line
    starting `Working directory: ` (older builds).
  - **Branch line:** exactly `Current branch: <name>`.
  - **Path:** must be absolute, POSIX or a Windows drive path, with no newline.

  A transcript, when one exists, still wins. The `(unresolved)` project filter selects
  the sessions with no cwd instead of nothing.
- **Build id in health.** `/_router/health` reports `build: "tree:<sha>"`: the tree git
  would write over the sources it was compiled from, via Northstar's `build.rs`. It reads
  `unknown` without git.

### Web UI in server mode

Both pages read `mode`, `password_pinned` and `public_url` from `GET /api/auth`.

**What server mode hides:**
- Claude Code setup
- Start at login
- Type into editor sessions (the wrapper)
- Listen on your network
- the account options "Sign in from here" and "Separate profile": "Paste a token" is the
  only way to add an account
- the router's process id

**What it changes:**
- **Password:** read-only, with "Set by the deployment. Change the Secret and restart
  the router."
- **Connection snippet** (in both modes, whenever the page is not on loopback):
  `ANTHROPIC_BASE_URL` set to `location.origin`, plus an `ANTHROPIC_AUTH_TOKEN` line.
- **`/live`:** an empty list explains that live sessions need the machine agent.
- **Copy:** text that says "this Mac" becomes neutral.

## Deploying

### Image

Two stages:

1. **Build.** A pinned `rust:1.98-bookworm` stage runs
   `cargo build --release --locked -p agent-router`. It builds glibc, and only the router:
   the wrapper and the menubar app have no use in a pod.
2. **Runtime.** `gcr.io/distroless/cc-debian12:nonroot`, running as uid 65532.

`/data` is created in the build stage and copied with its owner set. The build context
includes `.git` for the build id. `.dockerignore` mirrors `.gitignore` and never excludes
a tracked file. The image runs:

```
serve --server --bind 0.0.0.0 --port 8787 --data-dir /data
```

A Node stage for a future console slots in front of the build stage.

### Chart (`charts/agent-router`)

The chart is modelled on Northstar's.

**Shape.**
- One replica, with a `Recreate` rollout. The process holds the credential cache, the
  quota cache and the notifier's state, and two pods must never write one SQLite store.
- A `ReadWriteOnce` PVC for `/data`, kept on uninstall, default 20 Gi. Captured response
  bodies grow by about 100 MB a day. The PVC must be block storage, never NFS.
- An auth Secret, generated on first install, reused through `lookup`, and kept on
  uninstall. `auth.existingSecret` names one you manage.
- `push.existingSecret`: an optional Secret holding `AuthKey_<id>.p8` and/or the Firebase
  service account, mounted read-only and passed as `--push-keys-dir`.

**Allowed names.** The ingress host plus the Service names, so Northstar can call the
router in-cluster:
- `<svc>`
- `<svc>.<ns>`
- `<svc>.<ns>.svc`
- `<svc>.<ns>.svc.cluster.local`

**Probes and test.** All three probes hit `/_router/health` by pod IP. `helm test` curls
the same path through the Service IP.

**Two settings that are easy to miss.**
- `enableServiceLinks: false`. Otherwise Kubernetes gives the pod a variable for every
  Service in the namespace, and a Service named `agent-router` produces
  `AGENT_ROUTER_PORT=tcp://<ip>:8787`, the router's own port variable. `--port` outranks
  it, so this is a second guard.
- The chart refuses a `server.drainSeconds` that does not leave 5 s before
  `terminationGracePeriodSeconds`. The router writes its capture queue after the drain,
  and that must finish before the kubelet kills the pod.

**Ingress.** Off by default. TLS is on when it is enabled. The ingress-nginx annotations
the proxy needs are on by default, and other controllers ignore them:
- `proxy-body-size: 32m`
- `proxy-buffering: "off"`
- `proxy-read-timeout: "3600"` and `proxy-send-timeout: "3600"`

**Validation.** `values.schema.json` plus a `validate` helper fail a bad configuration at
render time, with a sentence saying what is wrong.

**NOTES.** After install, they print:
- how to read the password;
- the two lines that point Claude Code at the router;
- the phones' `https://` URL.

**Publishing.** `scripts/package-chart.sh` packages the chart for one build, as
Northstar's does: `major.minor.<pipeline iid>`, `appVersion` set to the commit SHA, and
the image repository filled in.

### Pipeline

`.gitlab-ci.yml` keeps its VS Code extension jobs (build, then release and publish on
`v*` tags). It adds Northstar's shape:

**Stages.**
- **check:** Rust and the chart.
  - Rust runs the tests and clippy, for the router and the wrapper. Clippy fails on every
    warning except the style and complexity groups, which hold about a dozen older lints
    from the current toolchain.
  - There is no format check: the tree has never been rustfmt-clean, and reformatting
    every file is not this change.
  - The chart is linted, and refused configurations are rendered on purpose to prove they
    fail.
- **build:** the image, through the shared template.
- **tag:** the commit SHA tag on `dev` and `main`, and `latest` on `main`.
- **publish:** the chart to the project's Helm registry, `dev` to the dev channel and
  `main` to stable.
- **deploy:** `helm upgrade --install` at the commit's image tag on `main`, then
  `helm test`.

**Workflow rules.** One pipeline per change. Tag pipelines still run.

**Variables, set in Settings → CI/CD:**
- `CI_TEMPLATES_PROJECT` (the shared templates are included only through this variable,
  never by path, because the repository is published)
- `DEPLOY_TOKEN_USER` / `DEPLOY_TOKEN`
- `AGENT_ROUTER_VALUES`: a file variable with this cluster's ingress host, class, TLS
  issuer, storage class and pull secret
- `KUBE_CONTEXT` (optional)

### First install

1. Set the variables above, then merge `dev` into `main`. The deploy job installs the
   release.
2. Read the password:
   `kubectl get secret <release>-auth -o jsonpath='{.data.password}' | base64 -d`.
3. Open `https://<host>/`, give the password, and paste a `claude setup-token` value
   under Accounts.
4. On each developer machine, put these two lines under `env` in `~/.claude/settings.json`,
   or a project's `.claude/settings.local.json`:
   `ANTHROPIC_BASE_URL=https://<host>` and `ANTHROPIC_AUTH_TOKEN=<password>`.
   Phase 2's installer will write them.

**Moving the Mac's history over** (optional). Scale the Deployment to 0. Run a pod with a
shell that mounts the PVC, then `kubectl cp` both store files and `credentials.json` into
`/data`. Then delete the rows that describe the Mac:

- `DELETE FROM settings WHERE key IN ('bind', 'auto_wrapper')`
- `DELETE FROM push_devices`

Scale back to 1.

## Phase 2: the machine agent (outline)

**What it is.**
- The same binary runs as `agent-router agent --server https://<host>` on each developer
  machine. It is supervised by the VS Code extension, or by launchd or a Windows
  equivalent.
- It holds one outbound connection to the server: a WebSocket through the ingress,
  authenticated with a per-machine token.
- The server routes commands by session id, which is a global key.

**What runs on the agent.** These already take every local input as a parameter:
- `live::discover_in`, `observe` and `stream`;
- `registry::Reader`;
- `activity::derive` (on the agent's clock);
- the `peer::*_in` wrapper calls;
- `/btw`.

**What the server keeps.** A per-machine snapshot of sessions and waiting prompts. A
machine whose agent is unreachable reads as unknown, never as closed. Its prompts stay on
the phones.

**Windows** needs:
- the wrapper and the agent built for Windows;
- a named-pipe control transport in place of the Unix socket;
- process liveness through the Windows API;
- `%USERPROFILE%\.claude` paths.

**Clients.**
- The phones learn `https://` URLs and a machine dimension.
- The VS Code extension learns a remote mode that supervises the agent instead of a
  router.
- The menubar app learns a URL and a password.

**Northstar.** Its Agent Router URL is checked to be loopback, and it sends no
credential. It needs its own change to accept an in-cluster URL and a Secret-held
credential.

## Not verified

- **The image build.** This Mac has no container runtime. The image is first built by
  CI, and the Linux binary is checked here with `cargo zigbuild`.
- **The tests on Linux.** The suite has only run on macOS. The CI job is its first Linux
  run.
- **The phones against a hosted router.** Both apps build `http://host:port`, so neither
  can add an `https://` router yet. That is listed under phase 2, and could come sooner.
- **The target cluster.** Its ingress controller and storage classes, which decide the
  annotations and the PVC class, are not known.
- **Egress.** Whether the cluster can reach `api.anthropic.com`, `api.push.apple.com` and
  `fcm.googleapis.com` directly. `HTTPS_PROXY` is not honoured.

## Sources

- Northstar's hosting: its `Dockerfile`, `charts/northstar`, `.gitlab-ci.yml`,
  `scripts/package-chart.sh`, and ADRs 0013 (LAN access with a password), 0016 (a named
  host may be allowed) and 0022 (a deployment may pin the password).
- The shared CI templates: `.docker-build` runs
  `docker build -t $CI_REGISTRY_IMAGE:$CI_COMMIT_REF_SLUG .` at the repository root. The
  shared rules name `develop`, while this repository works on `dev`.
- Distroless `cc-debian12` ships glibc, libssl and CA certificates, with no shell and no
  `openssl` binary: https://github.com/GoogleContainerTools/distroless/blob/main/base/README.md
