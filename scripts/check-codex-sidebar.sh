#!/bin/bash
# Never start an editor host on the developer's Mac. The native editor is
# qualified in a disposable Linux CI container with a separate home/workspace.
set -euo pipefail
[[ "${CI:-}" == true ]] || { echo "Native Codex sidebar acceptance runs in CI only." >&2; exit 126; }
repo=$(cd "$(dirname "$0")/.." && pwd)
cd "$repo"
[[ -f "${AR_SIDEBAR_SETUP:-}" ]] || { echo "Prepare the pinned native extension and VS Code first." >&2; exit 127; }
node --test scripts/codex-sidebar/history.spec.mjs
cargo build --locked -p agent-router --quiet
export AR_ROUTER_BIN="$repo/target/debug/agent-router"
export AR_CODEX_SIDEBAR_DRIVER="$repo/scripts/codex-sidebar/smoke.mjs"
bash scripts/check-codex-routing.sh
