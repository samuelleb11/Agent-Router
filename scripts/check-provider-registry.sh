#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
cargo test -p agent-router --locked provider_registry::tests:: -- --nocapture
cargo test -p agent-router --locked -- --exact db::tests::registered_route_attribution_survives_registry_changes
