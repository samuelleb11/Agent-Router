#!/usr/bin/env bash
# Real authenticated inference. Uses temporary client settings; never installs or
# changes a person's client configuration. Existing login files are read only.
set -euo pipefail
cd "$(dirname "$0")/.."
for client in CODEX OPENCODE CLAUDE; do
  variable="${client}_BIN"
  case "$client" in CODEX) program=codex ;; OPENCODE) program=opencode ;; CLAUDE) program=claude ;; esac
  executable="${!variable:-}"
  if [[ -z "$executable" ]]; then executable=$(command -v "$program" || true); fi
  if [[ -z "$executable" || ! -x "$executable" ]]; then
    echo "Set $variable to an installed $program executable." >&2
    exit 127
  fi
  export "AR_LIVE_${client}_BIN=$executable"
done
test_name=proxy::tests::native_clients_complete_live_provider_acceptance
listed=$(cargo test --locked -p agent-router --quiet -- --ignored --exact "$test_name" --list)
if [[ "$listed" != *"$test_name: test"* ]]; then
  echo "Live client acceptance test is missing." >&2
  exit 1
fi
cargo test --locked -p agent-router --quiet -- --ignored --exact --nocapture "$test_name"
