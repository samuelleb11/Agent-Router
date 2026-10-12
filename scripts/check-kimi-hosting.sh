#!/bin/sh
# The real `kimi web` hosting a session for the machine agent, on this machine: started in a
# directory, prompted, its approval answered, its model and mode switched and a second turn
# interrupted, by the agent's own code (`kimi::web::Host`).
#
#   scripts/check-kimi-hosting.sh
#
# Kimi Code runs on a scratch home, its model scripts/kimi-standin.py on a free loopback
# port: no tokens are spent, and nothing of this machine's own Kimi Code is read or written.
# Needs Kimi Code (`kimi` on PATH, or KIMI_BIN, or ~/.kimi-code/bin/kimi), python3 and cargo.
set -eu

root=$(cd "$(dirname "$0")/.." && pwd)
kimi=${KIMI_BIN:-$(command -v kimi || echo "$HOME/.kimi-code/bin/kimi")}
[ -x "$kimi" ] || { echo "no Kimi Code here: set KIMI_BIN"; exit 1; }

scratch=$(mktemp -d "${TMPDIR:-/tmp}/ar-kimi-hosting.XXXXXX")
standin=
cleanup() {
  [ -n "$standin" ] && kill "$standin" 2>/dev/null
  rm -rf "$scratch"
}
trap cleanup EXIT INT TERM

port=$(python3 -I -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])')
python3 -I "$root/scripts/kimi-standin.py" "$port" &
standin=$!

mkdir -p "$scratch/home/.kimi-code" "$scratch/work"
cat > "$scratch/home/.kimi-code/config.toml" <<TOML
default_model = "agent-router/k3"

[providers.agent-router]
type = "anthropic"
base_url = "http://127.0.0.1:$port"
api_key = "agent-router"

[models."agent-router/k3"]
provider = "agent-router"
model = "k3"
max_context_size = 262144
capabilities = ["thinking", "image_in", "tool_use"]
display_name = "K3 via Agent Router"

[models."agent-router/k3-256k"]
provider = "agent-router"
model = "k3-256k"
max_context_size = 262144
capabilities = ["thinking", "image_in", "tool_use"]
display_name = "K3 (256K) via Agent Router"
TOML

echo "kimi $("$kimi" --version) on a scratch home, its model a stand-in on port $port"
cd "$root"
AR_KIMI_BIN="$kimi" AR_KIMI_HOME="$scratch/home/.kimi-code" AR_KIMI_OS_HOME="$scratch/home" AR_KIMI_WORK="$scratch/work" \
  cargo test -p agent-router --quiet -- --ignored --exact --nocapture kimi::web::tests::the_real_kimi_web_hosts_a_session \
  > "$scratch/test.log" 2>&1 || true
grep -E '^(started|prompted|model|interrupted|stopped)' "$scratch/test.log" || true
if grep -q 'test result: ok. 1 passed' "$scratch/test.log"; then
  echo "hosted: started, prompted, approval answered, model and mode switched, interrupted"
else
  tail -n 30 "$scratch/test.log"
  exit 1
fi
