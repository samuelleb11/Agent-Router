#!/bin/sh
# Real Codex, three router providers, local fake tool cycles. No package installs,
# user settings or real provider requests. Tested with 0.162.0-alpha.17.2.
set -eu

repo=$(cd "$(dirname "$0")/.." && pwd)
cli=${CODEX_BIN:-$(command -v codex || true)}
[ -n "$cli" ] && [ -x "$cli" ] || {
  echo "Codex is not installed. Set CODEX_BIN to an installed CLI executable." >&2
  exit 127
}
case "$cli" in
  /*) ;;
  *) cli=$(cd "$(dirname "$cli")" && pwd)/$(basename "$cli") ;;
esac

cd "$repo"
test_name=proxy::tests::the_real_codex_cli_routes_three_providers
listed=$(cargo test --locked -p agent-router --quiet -- --ignored --exact "$test_name" --list)
case "$listed" in
  *"$test_name: test"*) ;;
  *) echo "The native Codex routing test is missing." >&2; exit 1 ;;
esac
AR_CODEX_BIN="$cli" cargo test --locked -p agent-router --quiet -- \
  --ignored --exact --nocapture "$test_name"
