#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
cargo test -p agent-router --locked client_setup::tests:: -- --nocapture
