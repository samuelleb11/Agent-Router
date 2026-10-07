#!/bin/sh
# Connects this machine to an Agent Router. It installs the machine agent, which lets the
# router follow and steer the Claude Code sessions that run here, and starts it at login.
# The router serves this script with its own address filled in below:
#
#   curl -fsSL <router>/agent/install.sh | sh
#   curl -fsSL <router>/agent/install.sh | sh -s -- --label work-laptop
#
# Options after `--` go to `agent-router agent install`: --label <name>, --project <dir>,
# --no-service, --no-config. It asks for the router's password unless
# AGENT_ROUTER_PASSWORD holds it. Where VS Code's `code` command is installed it also
# installs the Agent Router extension, which lets the router type into the sessions VS
# Code starts; without it they can be followed. On Windows, use install.ps1.
set -eu

SERVER=@SERVER@

say() { printf '%s\n' "$*" >&2; }
fail() { say "agent-router: $*"; exit 1; }

case "$(uname -s)" in
  Darwin) os=darwin ;;
  Linux) os=linux ;;
  *) fail "this script is for macOS and Linux; on Windows, run: irm $SERVER/agent/install.ps1 | iex" ;;
esac
case "$(uname -m)" in
  arm64 | aarch64) arch=arm64 ;;
  x86_64 | amd64) arch=x64 ;;
  *) fail "no agent is built for $(uname -m)" ;;
esac
platform="$os-$arch"
command -v curl >/dev/null 2>&1 || fail "curl is needed"

dir=$(mktemp -d)
trap 'rm -rf "$dir"' EXIT
trap 'exit 130' INT TERM

fetch() {
  curl -fsSL --retry 2 -o "$dir/$1" "$SERVER/agent/$platform/$1" \
    || fail "could not download $1 for $platform from $SERVER"
}

say "Downloading the agent for $platform from $SERVER"
fetch agent-router
chmod 755 "$dir/agent-router"

# `agent install` copies itself into ~/.agent-router/bin, so the copy here can go. The
# password is read from the terminal: this script's own input is the pipe it came in by.
if [ -n "${AGENT_ROUTER_PASSWORD:-}" ]; then
  "$dir/agent-router" agent install --server "$SERVER" "$@"
elif (: </dev/tty) 2>/dev/null; then
  "$dir/agent-router" agent install --server "$SERVER" "$@" </dev/tty
else
  fail "there is no terminal to ask for the router's password on: set AGENT_ROUTER_PASSWORD and run this again"
fi

by_hand="install $SERVER/agent/$platform/agent-router.vsix from VS Code (Extensions, ..., Install from VSIX)"
if command -v code >/dev/null 2>&1; then
  say "Installing the Agent Router extension into VS Code"
  fetch agent-router.vsix
  code --install-extension "$dir/agent-router.vsix" --force >&2 \
    || say "VS Code did not take the extension: $by_hand"
else
  say "VS Code's \`code\` command was not found, so the extension was not installed. This machine's sessions can be followed; to type into the ones VS Code starts, $by_hand."
fi
