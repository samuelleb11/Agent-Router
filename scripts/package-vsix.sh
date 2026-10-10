#!/bin/sh
# Package the extension for one platform, with the binaries scripts/build-target.sh
# left in dist/<vsce target>/, into dist/agent-router-<version>-<target>.vsix.
#
#   scripts/package-vsix.sh darwin-arm64
#   scripts/package-vsix.sh win32-x64       # agent-router.exe and claude-wrapper.exe
#
# The extension's version is the router's (Cargo.toml), so a Marketplace update always
# carries a router at least as new as the one it will find running.
set -eu
cd "$(dirname "$0")/.."
VT="${1:?usage: package-vsix.sh <darwin-arm64|darwin-x64|linux-x64|linux-arm64|win32-x64>}"
SRC="dist/$VT"
# Windows programs keep their .exe, which is the name the extension looks for there.
EXE=""
case "$VT" in win32-*) EXE=".exe" ;; esac
# -f rather than -x: a cross-built .exe need not carry the execute bit; chmod gives it.
[ -f "$SRC/agent-router$EXE" ] || { echo "no binaries in $SRC — run scripts/build-target.sh first" >&2; exit 2; }
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)

cd vscode
rm -rf bin && mkdir -p bin
cp "../$SRC/agent-router$EXE" "../$SRC/claude-wrapper$EXE" bin/
chmod 755 "bin/agent-router$EXE" "bin/claude-wrapper$EXE"
printf '%s\n' "$VERSION" > bin/VERSION
# The manifest version follows the router; edited in place and restored after.
cp package.json package.json.orig
node -e 'const fs=require("fs");const p=JSON.parse(fs.readFileSync("package.json"));p.version=process.argv[1];fs.writeFileSync("package.json",JSON.stringify(p,null,2)+"\n")' "$VERSION"
# vsce packages only this directory, so the licence has to be inside it. Open VSX
# refuses a package without one, and the Marketplace shows "no license" otherwise.
cp ../LICENSE LICENSE
trap 'mv -f package.json.orig package.json; rm -f LICENSE' EXIT

[ -d node_modules ] || npm ci --no-audit --no-fund
npm run --silent compile
ALLOW=""
git -C .. remote get-url origin >/dev/null 2>&1 || ALLOW="--allow-missing-repository"
npx --no-install @vscode/vsce package --target "$VT" --out "../dist/agent-router-$VERSION-$VT.vsix" $ALLOW
ls -la "../dist/agent-router-$VERSION-$VT.vsix"
