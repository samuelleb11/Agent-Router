#!/bin/sh
# How long the newest finished pipeline on a ref ran, against a ceiling: exit 0 when under it.
#
#   scripts/ci-minutes.sh dev 12
#
# Asks GitLab's API for the ref's newest pipeline that has finished (passed or failed) and
# compares its running time -- the jobs', not the queue's -- with the ceiling, in minutes. The
# token is GITLAB_TOKEN when set, else the vault's "GitLab (whnet.ca)" item, read here and
# never printed. curl rather than a script's HTTP library: the GitLab host refuses some
# libraries' default User-Agent.
set -eu

ref=${1:?usage: ci-minutes.sh <ref> <max-minutes>}
max=${2:?usage: ci-minutes.sh <ref> <max-minutes>}
api=${CI_API_V4_URL:-https://gitlab.whnet.ca/api/v4}
project=${CI_PROJECT_ID:-whnet%2Fapps%2Fagent-router}

token=${GITLAB_TOKEN:-}
if [ -z "$token" ]; then
  token=$("$HOME/.claude/skills/vault/scripts/vault" get 'GitLab (whnet.ca)' -f tok --raw)
fi

pipelines=$(curl -fsS -m 30 -H "PRIVATE-TOKEN: $token" "$api/projects/$project/pipelines?ref=$ref&per_page=20")
id=$(printf '%s' "$pipelines" | python3 -I -c '
import json, sys
done = [p for p in json.load(sys.stdin) if p["status"] in ("success", "failed")]
print(done[0]["id"] if done else "")')
[ -n "$id" ] || { echo "no finished pipeline on $ref"; exit 1; }

curl -fsS -m 30 -H "PRIVATE-TOKEN: $token" "$api/projects/$project/pipelines/$id" | python3 -I -c '
import json, sys
p, ceiling = json.load(sys.stdin), float(sys.argv[1])
minutes = (p.get("duration") or 0) / 60
pid, ref, sha, status = p["id"], p["ref"], p["sha"][:8], p["status"]
print(f"pipeline {pid} on {ref} at {sha}: {status}, {minutes:.1f} min (ceiling {ceiling:g})")
sys.exit(0 if minutes < ceiling else 1)' "$max"
