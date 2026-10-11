#!/bin/sh
# Installed Claude Code, three local fake providers and real scoped router auth.
# No package installs, user configuration changes or live provider requests.
set -eu
repo=$(cd "$(dirname "$0")/.." && pwd)
cli=${CLAUDE_BIN:-$(command -v claude || true)}
[ -n "$cli" ] && [ -x "$cli" ] || {
  echo "Claude Code is not installed. Set CLAUDE_BIN to an installed CLI executable." >&2
  exit 127
}
case "$cli" in
  /*) ;;
  *) cli=$(cd "$(dirname "$cli")" && pwd)/$(basename "$cli") ;;
esac
cd "$repo"
test_name=proxy::tests::the_real_claude_cli_routes_three_providers
listed=$(cargo test --locked -p agent-router --quiet -- --ignored --exact "$test_name" --list)
case "$listed" in
  *"$test_name: test"*) ;;
  *) echo "The native Claude Code routing test is missing." >&2; exit 1 ;;
esac
AR_CLAUDE_BIN="$cli" cargo test --locked -p agent-router --quiet -- --ignored --exact --nocapture "$test_name"
