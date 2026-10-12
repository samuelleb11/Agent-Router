#!/bin/sh
# Real OpenCode, three router providers, local fake Messages/Responses tool cycles.
# No credentials, global settings, package installation or provider calls.
#
#   OPENCODE_BIN=/path/to/opencode scripts/check-opencode-routing.sh
#
# Needs an installed OpenCode CLI, cargo, and a POSIX shell. Tested with stable 1.19.0.
set -eu

repo=$(cd "$(dirname "$0")/.." && pwd)
cli=${OPENCODE_BIN:-$(command -v opencode || true)}
[ -n "$cli" ] && [ -x "$cli" ] || {
  echo "OpenCode is not installed. Set OPENCODE_BIN to a CLI executable; this smoke never installs packages." >&2
  exit 127
}
case "$cli" in
  /*) ;;
  *) cli=$(cd "$(dirname "$cli")" && pwd)/$(basename "$cli") ;;
esac

cd "$repo"
test_name=proxy::tests::the_real_opencode_cli_routes_three_providers
listed=$(cargo test --locked -p agent-router --quiet -- --ignored --exact "$test_name" --list)
case "$listed" in
  *"$test_name: test"*) ;;
  *) echo "The native OpenCode routing test is missing." >&2; exit 1 ;;
esac
AR_OPENCODE_BIN="$cli" cargo test --locked -p agent-router --quiet -- \
  --ignored --exact --nocapture "$test_name"
