# syntax=docker/dockerfile:1
#
# Agent Router as a hosted server (docs/hosting-on-kubernetes.md): the router binary, its
# licence, and the machine agent it hands out to the developer machines it serves.
#
# The agent comes from the package builds, not from here: CI runs scripts/stage-agents.sh
# over their output before this, which puts every platform's agent and editor package in
# dist/agents. A build without them still works; its installer then says it has nothing
# to hand out (src/downloads.rs).
#
#   docker build -t agent-router .
#   docker run -p 8787:8787 -v agent-router:/data \
#     -e AGENT_ROUTER_SERVER_PASSWORD=<at least 8 characters> agent-router
#
# Three stages, because the toolchains that build the console and the router have no place
# in the image that runs it. Nothing else is needed at run time: the console is compiled into
# the binary, a hosted router signs only with setup-token accounts pasted into the console, so
# it never runs `claude`, and it handles APNs keys in-process, so it never runs `openssl`.

# ---------------------------------------------------------------- console
# The web console, built with Node as Northstar's is. The router stage embeds its ui/dist, so
# nothing of it reaches the runtime image but the bytes inside the binary.
FROM node:24-bookworm-slim AS console
WORKDIR /src/ui
RUN npm i -g pnpm@10.33.0
# Dependencies before sources, so a change to the console alone reuses the installed layer.
COPY ui/package.json ui/pnpm-lock.yaml ./
RUN pnpm install --frozen-lockfile
COPY ui/ ./
# tsc, Vite, the first load's budget and the notices check -> /src/ui/dist
RUN pnpm run build

# ---------------------------------------------------------------- router
FROM rust:1.98-bookworm AS router
WORKDIR /src
# The WHOLE repository, `.git` included. The build id `/_router/health` reports is the tree
# git would write over these files (build.rs); without git it is `unknown`. So
# `.dockerignore` must never exclude a TRACKED file -- a missing one would give this image
# the id of a tree that was never committed. (A checkout made by `git worktree add` has a
# `.git` file pointing outside the context, and builds `unknown` for the same reason.)
COPY . .
# The console, where build.rs looks for it: a release build stops without it. Git ignores
# ui/dist, so it leaves the build id alone, and .dockerignore keeps a local one out of the
# context, so this is the only ui/dist there is.
COPY --from=console /src/ui/dist ui/dist
# `-p agent-router`, never `--workspace`: the workspace also holds the macOS menubar app,
# which pulls in gtk on Linux, and the editor wrapper, which has no use in a pod.
# git comes with this base image (buildpack-deps); asking for its version first fails the
# build if that ever changes, rather than stamping the image `unknown`.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    git --version \
 && cargo build --release --locked -p agent-router \
 && install -D target/release/agent-router /out/agent-router \
 && mkdir -p /out/data
# The agent's files, when this build was given them: an empty directory otherwise.
RUN mkdir -p /out/agents \
 && if [ -d dist/agents ]; then cp -R dist/agents/. /out/agents/; fi \
 && ls -lR /out/agents

# ---------------------------------------------------------------- runtime
# glibc, libgcc, libssl and CA certificates, and nothing else: no shell, no package manager.
# `nonroot` is uid 65532.
FROM gcr.io/distroless/cc-debian12:nonroot
COPY --from=router /out/agent-router /usr/local/bin/agent-router
# Where a Debian system keeps a package's licence.
COPY LICENSE /usr/local/share/doc/agent-router/
# Where the router looks for them by default, relative to itself (`--agents-dir`).
COPY --from=router /out/agents /usr/local/share/agent-router/agents
# The directory `--data-dir` names below: the store, the token file, dropped key files.
# Created here because the image has no shell to create it at run time, and owned by the
# user the router runs as.
COPY --from=router --chown=65532:65532 /out/data /data
EXPOSE 8787
ENTRYPOINT ["/usr/local/bin/agent-router"]
# Hosted mode, on every address in the container. Every caller, loopback included, then
# presents the password, which the router reads from AGENT_ROUTER_SERVER_PASSWORD and will
# not start without (the chart sets it from a Secret).
CMD ["serve", "--server", "--bind", "0.0.0.0", "--port", "8787", "--data-dir", "/data"]
