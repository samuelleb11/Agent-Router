#!/bin/sh
# Local fake-provider coverage for portable Chat Completions, no external keys.
set -eu
repo=$(cd "$(dirname "$0")/.." && pwd)
cd "$repo"
test_name=chat_completions::tests::portable_chat_completions_cross_provider_matrix
listed=$(cargo test --locked -p agent-router --quiet -- --exact "$test_name" --list)
case "$listed" in
  *"$test_name: test"*) ;;
  *) echo "The Chat Completions acceptance matrix is missing." >&2; exit 1 ;;
esac
cargo test --locked -p agent-router --quiet chat_completions::tests:: -- --nocapture
