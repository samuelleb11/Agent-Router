#!/bin/sh
# Native clients, local fake upstreams, scoped router tokens and real tool turns.
# Set CODEX_BIN and OPENCODE_BIN when the installed CLIs are outside PATH.
set -eu

repo=$(cd "$(dirname "$0")/.." && pwd)
cd "$repo"
cargo test --locked -p agent-router -- --exact proxy::tests::chatgpt_headerless_streams_translate_and_capture_usage
bash "$repo/scripts/check-claude-routing.sh"
bash "$repo/scripts/check-opencode-routing.sh"
bash "$repo/scripts/check-codex-routing.sh"
