# syntax=docker/dockerfile:1
#
# Agent Router as a hosted server (docs/hosting-on-kubernetes.md): the router binary and
# its licence, and nothing else.
#
#   docker build -t agent-router .
#   docker run -p 8787:8787 -v agent-router:/data \
#     -e AGENT_ROUTER_SERVER_PASSWORD=<at least 8 characters> agent-router
#
# Two stages, because the toolchain that builds the router has no place in the image that
# runs it. Nothing else is needed at run time: a hosted router signs only with setup-token
# accounts pasted into the dashboard, so it never runs `claude`, and it handles APNs keys
# in-process, so it never runs `openssl`.

# ---------------------------------------------------------------- console (not yet)
# A web console built with Node will come first, as Northstar's does, and the runtime stage
# will then copy its `ui/dist` beside the binary:
#
#   FROM node:24-bookworm-slim AS console
#   WORKDIR /src/ui
#   COPY ui/package.json ui/pnpm-lock.yaml ./    # dependencies before sources
#   RUN <install with the frozen lockfile>
#   COPY ui/ ./
#   RUN <build>                                  # -> /src/ui/dist
#
# Today the dashboard and /live are compiled into the binary (assets/), so there is nothing
# to build here.

# ---------------------------------------------------------------- router
FROM rust:1.98-bookworm AS router
WORKDIR /src
# The WHOLE repository, `.git` included. The build id `/_router/health` reports is the tree
# git would write over these files (build.rs); without git it is `unknown`. So
# `.dockerignore` must never exclude a TRACKED file -- a missing one would give this image
# the id of a tree that was never committed. (A checkout made by `git worktree add` has a
# `.git` file pointing outside the context, and builds `unknown` for the same reason.)
COPY . .
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

# ---------------------------------------------------------------- runtime
# glibc, libgcc, libssl and CA certificates, and nothing else: no shell, no package manager.
# `nonroot` is uid 65532.
FROM gcr.io/distroless/cc-debian12:nonroot
COPY --from=router /out/agent-router /usr/local/bin/agent-router
# Where a Debian system keeps a package's licence.
COPY LICENSE /usr/local/share/doc/agent-router/
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
