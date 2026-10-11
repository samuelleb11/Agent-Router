#!/bin/bash
# Explicit, non-empty qualification gates. Fixtures never contact real providers.
set -euo pipefail
repo=$(cd "$(dirname "$0")/.." && pwd)
cd "$repo"

test_case() {
    local name=$1
    local listed
    listed=$(cargo test --locked -p agent-router --quiet -- --exact "$name" --list)
    [[ "$listed" == *"$name: test"* ]] || { echo "Missing acceptance case: $name" >&2; exit 1; }
    cargo test --locked -p agent-router --quiet "$name" -- --exact --nocapture "${@:2}"
}

case "${1:-}" in
    gateway)
        test_case models::tests::openai_model_discovery_uses_standard_objects
        # A private test environment; no packages installed in the user's Python.
        sdk_env=${AR_SDK_ENV:-${TMPDIR:-/tmp}/agent-router-openai-sdk-3.28.0}
        if [[ ! -x "$sdk_env/bin/python" ]]; then
            python3 -m venv "$sdk_env"
        fi
        "$sdk_env/bin/python" -m pip --disable-pip-version-check --quiet install -r scripts/openai-sdk-requirements.txt
        export AR_SDK_PYTHON="$sdk_env/bin/python"
        test_case proxy::tests::official_openai_sdk_routes_three_providers --ignored
        ;;
    catalogue|catalogue-core)
        test_case models::tests::client_model_metadata_has_capabilities_without_provider_secrets
        test_case codex::catalogue::tests::catalogue_ownership_and_capabilities
        cli=${CODEX_BIN:-$(command -v codex || true)}
        [[ -n "$cli" && -x "$cli" ]] || { echo "Set CODEX_BIN to the qualified Codex executable" >&2; exit 127; }
        export AR_CODEX_BIN="$cli"
        test_case proxy::tests::native_codex_catalogue_additions_and_removals --ignored
        if [[ "$1" == catalogue ]]; then
            # Actual editor-host qualification belongs in CI. An app-server test
            # alone cannot prove the native sidebar's picker.
            bash scripts/check-codex-sidebar.sh
        fi
        ;;
    runtime|frontends)
        test_case codex::runtime::tests::replies_match_ids_even_when_the_native_peer_reorders_them
        test_case codex::runtime::tests::disconnects_fail_unacknowledged_actions_and_clear_approval_ownership
        test_case codex::runtime::tests::approvals_are_bound_to_connection_and_thread_and_wait_for_resolution
        test_case codex::runtime::tests::native_refusals_and_malformed_frames_never_become_success
        cli=${CODEX_BIN:-$(command -v codex || true)}
        [[ -n "$cli" && -x "$cli" ]] || { echo "Set CODEX_BIN to the qualified native executable" >&2; exit 127; }
        export AR_CODEX_BIN="$cli"
        cargo build --locked -p agent-router --quiet
        export AR_ROUTER_BIN="$repo/target/debug/agent-router"
        if [[ "$1" == frontends ]]; then
            [[ "${CI:-}" == true ]] || { echo "Native frontend acceptance runs in CI only." >&2; exit 126; }
            export AR_CODEX_MANAGED_DRIVER="$repo/scripts/codex-sidebar/managed.mjs"
        fi
        test_case proxy::tests::managed_codex_controls_cross_the_machine_tunnel --ignored
        ;;
    *) echo "Usage: $0 gateway|catalogue|catalogue-core|runtime|frontends" >&2; exit 2 ;;
esac
