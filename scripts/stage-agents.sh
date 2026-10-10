#!/bin/sh
# Lay out what the package builds left in dist/ the way a hosted router hands it out
# (src/downloads.rs): dist/agents/<platform>/agent-router[.exe] and agent-router.vsix.
# The Dockerfile copies dist/agents into the image beside the router.
#
#   scripts/stage-agents.sh    # after build-target.sh and package-vsix.sh for all five
#
# Every platform, or it stops: the router's installer offers all five, and an image that
# lacked one would fail on someone's machine rather than here. Plain POSIX sh, as CI runs
# it in the docker job's image.
set -eu
cd "$(dirname "$0")/.."
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
rm -rf dist/agents
for vt in darwin-arm64 darwin-x64 linux-x64 linux-arm64 win32-x64; do
  exe=""
  [ "$vt" = win32-x64 ] && exe=".exe"
  bin="dist/$vt/agent-router$exe"
  vsix="dist/agent-router-$VERSION-$vt.vsix"
  [ -f "$bin" ] || { echo "no $bin: run scripts/build-target.sh for $vt first" >&2; exit 2; }
  [ -f "$vsix" ] || { echo "no $vsix: run scripts/package-vsix.sh $vt first" >&2; exit 2; }
  mkdir -p "dist/agents/$vt"
  cp "$bin" "dist/agents/$vt/agent-router$exe"
  chmod 755 "dist/agents/$vt/agent-router$exe"
  cp "$vsix" "dist/agents/$vt/agent-router.vsix"
done
ls -lR dist/agents
