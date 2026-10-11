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
    *) echo "Usage: $0 gateway|catalogue|catalogue-core" >&2; exit 2 ;;
esac
