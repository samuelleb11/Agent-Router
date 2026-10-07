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
# AGENT_ROUTER_PASSWORD holds it. Where VS Code is installed, or the VS Code Server that a
# Remote - SSH window from another computer runs here, it also installs the Agent Router
# extension, which lets the router type into the sessions VS Code starts; without it they
# can be followed. On Windows, use install.ps1.
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

# The extension goes where Claude Code runs, which is the machine a window's files are on:
# into VS Code itself through its `code` command, and into the VS Code Server that a
# Remote - SSH window from another computer runs here, through the server's own CLI. A
# headless machine has only the server. Every version of a server shares one extensions
# folder, so the first to take it is enough; in the server's own terminals `code` is a
# shim into that same server, so it is left to the server's turn.
by_hand="install $SERVER/agent/$platform/agent-router.vsix from VS Code (Extensions, ..., Install from VSIX)"
servers=${VSCODE_AGENT_FOLDER:-$HOME/.vscode-server}
vsix=
install_with() { # <where> <its command>
  [ -n "$vsix" ] || { fetch agent-router.vsix; vsix="$dir/agent-router.vsix"; }
  say "Installing the Agent Router extension into $1"
  "$2" --install-extension "$vsix" --force >&2 && return
  say "$1 did not take the extension: $by_hand"
  return 1
}

none=1
code=$(command -v code || true)
case $code in "$servers"/*) code= ;; esac
if [ -n "$code" ]; then
  none=
  install_with "VS Code" "$code" || :
fi
for cli in "$servers"/cli/servers/*/server/bin/code-server "$servers"/bin/*/bin/code-server; do
  [ -x "$cli" ] || continue
  none=
  if install_with "the VS Code Server in $servers" "$cli"; then
    say "Reload the VS Code windows open on this machine (Developer: Reload Window) to start it."
    break
  fi
done
[ -z "$none" ] || say "Neither VS Code's \`code\` command nor a VS Code Server was found, so the extension was not installed. This machine's sessions can be followed; to type into the ones VS Code starts, $by_hand."
