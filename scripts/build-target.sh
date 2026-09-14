#!/bin/sh
# Build the router and the wrapper for one Rust target, into dist/<vsce target>/.
#
#   scripts/build-target.sh aarch64-apple-darwin
#   scripts/build-target.sh x86_64-apple-darwin           # from an Apple Silicon Mac too
#   scripts/build-target.sh x86_64-unknown-linux-musl      # from a Mac, through zig or musl-gcc
#   scripts/build-target.sh aarch64-unknown-linux-musl
#
# Linux builds are static (musl), so one binary runs on any distribution. From a Mac
# they go through `cargo zigbuild` when it is installed (cargo install cargo-zigbuild;
# brew install zig), else through a musl cross compiler (brew install
# filosottile/musl-cross/musl-cross). On a Linux runner they build natively with
# musl-tools.
set -eu
cd "$(dirname "$0")/.."
TRIPLE="${1:?usage: build-target.sh <rust target triple>}"
case "$TRIPLE" in
  aarch64-apple-darwin) VT=darwin-arm64 ;;
  x86_64-apple-darwin) VT=darwin-x64 ;;
  x86_64-unknown-linux-musl|x86_64-unknown-linux-gnu) VT=linux-x64 ;;
  aarch64-unknown-linux-musl|aarch64-unknown-linux-gnu) VT=linux-arm64 ;;
  *) echo "unknown target $TRIPLE" >&2; exit 2 ;;
esac
HOST=$(rustc -vV | sed -n 's/^host: //p')
# `cargo install` puts cargo-zigbuild in ~/.cargo/bin, which not every shell has on PATH.
[ -d "$HOME/.cargo/bin" ] && PATH="$HOME/.cargo/bin:$PATH"; export PATH
rustup target add "$TRIPLE" >/dev/null 2>&1 || true

BUILD="cargo build"
case "$TRIPLE" in
  *-linux-*)
    if [ "${HOST#*-}" != "${TRIPLE#*-}" ]; then
      if command -v cargo-zigbuild >/dev/null 2>&1; then
        BUILD="cargo zigbuild"
      else
        ARCH="${TRIPLE%%-*}"
        CC_VAR="CC_$(echo "$TRIPLE" | tr '-' '_')"
        LINK_VAR="CARGO_TARGET_$(echo "$TRIPLE" | tr '[:lower:]-' '[:upper:]_')_LINKER"
        if command -v "$ARCH-linux-musl-gcc" >/dev/null 2>&1; then
          export "$CC_VAR=$ARCH-linux-musl-gcc" "$LINK_VAR=$ARCH-linux-musl-gcc"
        else
          echo "no way to cross-build $TRIPLE from $HOST: install cargo-zigbuild or $ARCH-linux-musl-gcc" >&2
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
cp "$TDIR/$TRIPLE/release/agent-router" "$TDIR/$TRIPLE/release/claude-wrapper" "$OUT/"
ls -la "$OUT"
