#!/bin/sh
# Build the router and the wrapper for one Rust target, into dist/<vsce target>/.
#
#   scripts/build-target.sh aarch64-apple-darwin
#   scripts/build-target.sh x86_64-apple-darwin           # from an Apple Silicon Mac too
#   scripts/build-target.sh x86_64-unknown-linux-musl      # from a Mac, through zig or musl-gcc
#   scripts/build-target.sh aarch64-unknown-linux-musl
#   scripts/build-target.sh x86_64-pc-windows-gnu          # from a Mac, through zig or mingw-w64
#
# Linux builds are static (musl), so one binary runs on any distribution. From a Mac
# they go through `cargo zigbuild` when it is installed (cargo install cargo-zigbuild;
# brew install zig), else through a musl cross compiler (brew install
# filosottile/musl-cross/musl-cross). On a Linux runner they build natively with
# musl-tools.
#
# Windows builds are the GNU ABI, so a Mac can make them: through `cargo zigbuild`
# as above, else through mingw-w64 (brew install mingw-w64). Their programs are
# agent-router.exe and claude-wrapper.exe, and only the agent runs there
# (docs/hosting-machine-agent.md).
set -eu
cd "$(dirname "$0")/.."
TRIPLE="${1:?usage: build-target.sh <rust target triple>}"
case "$TRIPLE" in
  aarch64-apple-darwin) VT=darwin-arm64 ;;
  x86_64-apple-darwin) VT=darwin-x64 ;;
  x86_64-unknown-linux-musl|x86_64-unknown-linux-gnu) VT=linux-x64 ;;
  aarch64-unknown-linux-musl|aarch64-unknown-linux-gnu) VT=linux-arm64 ;;
  x86_64-pc-windows-gnu) VT=win32-x64 ;;
  *) echo "unknown target $TRIPLE" >&2; exit 2 ;;
esac
EXE=""
case "$TRIPLE" in *-windows-*) EXE=".exe" ;; esac
HOST=$(rustc -vV | sed -n 's/^host: //p')
# `cargo install` puts cargo-zigbuild in ~/.cargo/bin, which not every shell has on PATH.
[ -d "$HOME/.cargo/bin" ] && PATH="$HOME/.cargo/bin:$PATH"; export PATH
rustup target add "$TRIPLE" >/dev/null 2>&1 || true

BUILD="cargo build"
case "$TRIPLE" in
  *-linux-*|*-windows-gnu)
    if [ "${HOST#*-}" != "${TRIPLE#*-}" ]; then
      ARCH="${TRIPLE%%-*}"
      case "$TRIPLE" in
        *-windows-gnu) GCC="$ARCH-w64-mingw32-gcc" ;;
        *) GCC="$ARCH-linux-musl-gcc" ;;
      esac
      if command -v cargo-zigbuild >/dev/null 2>&1; then
        BUILD="cargo zigbuild"
      else
        CC_VAR="CC_$(echo "$TRIPLE" | tr '-' '_')"
        LINK_VAR="CARGO_TARGET_$(echo "$TRIPLE" | tr '[:lower:]-' '[:upper:]_')_LINKER"
        if command -v "$GCC" >/dev/null 2>&1; then
          export "$CC_VAR=$GCC" "$LINK_VAR=$GCC"
        else
          echo "no way to cross-build $TRIPLE from $HOST: install cargo-zigbuild or $GCC" >&2
          exit 3
        fi
      fi
    fi ;;
esac

echo "==> $BUILD --release --target $TRIPLE (agent-router, claude-wrapper)"
$BUILD --release --target "$TRIPLE" -p agent-router -p claude-wrapper
OUT="dist/$VT"
mkdir -p "$OUT"
TDIR="${CARGO_TARGET_DIR:-target}"
cp "$TDIR/$TRIPLE/release/agent-router$EXE" "$TDIR/$TRIPLE/release/claude-wrapper$EXE" "$OUT/"
ls -la "$OUT"
