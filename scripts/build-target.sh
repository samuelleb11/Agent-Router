#!/bin/sh
# Build the router and the wrapper for one Rust target, into dist/<vsce target>/.
#
#   scripts/build-target.sh aarch64-apple-darwin          # from Linux too, through zig
#   scripts/build-target.sh x86_64-apple-darwin           # from an Apple Silicon Mac too
#   scripts/build-target.sh x86_64-unknown-linux-musl      # from a Mac, through zig or musl-gcc
#   scripts/build-target.sh aarch64-unknown-linux-musl
#   scripts/build-target.sh x86_64-pc-windows-gnu          # from a Mac, through zig or mingw-w64
#
# macOS builds are native on a Mac. Anywhere else they go through `cargo zigbuild`, with
# no Apple SDK: both programs link only libSystem and libiconv, no frameworks, and zig
# and cargo-zigbuild carry stubs for those. CI builds them this way, on Linux.
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
#
# The router embeds the web console, ui/dist, and a release build stops without it. This
# builds it first, with pnpm (Node 22 or later; npm i -g pnpm@10.33.0), unless it is
# already there: CI's build:ui job hands it over, as does an earlier run. It is built once
# and never per target, since targets build in parallel with their own CARGO_TARGET_DIR
# and would race on ui/dist.
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
  *-apple-darwin)
    case "$HOST" in
      *-apple-darwin) ;;
      *)
        command -v cargo-zigbuild >/dev/null 2>&1 || {
          echo "no way to cross-build $TRIPLE from $HOST: install cargo-zigbuild and zig" >&2
          exit 3
        }
        BUILD="cargo zigbuild" ;;
    esac ;;
esac

# The console, before the first cargo step. A run started beside this one waits on the lock
# for its build rather than starting a second over the same ui/dist, and a build that fails
# leaves no ui/dist behind, so the next run does not embed one its checks refused.
if [ ! -f ui/dist/index.html ]; then
  LOCK=ui/.build-target.lock
  until mkdir "$LOCK" 2>/dev/null; do
    echo "waiting for another run's console build (remove $LOCK if none is running)" >&2
    sleep 5
  done
  trap 'rmdir "$LOCK"' EXIT
  if [ ! -f ui/dist/index.html ]; then
    command -v pnpm >/dev/null 2>&1 || { echo "building the console needs pnpm: npm i -g pnpm@10.33.0" >&2; exit 4; }
    echo "==> pnpm run build (the web console, into ui/dist)"
    (cd ui && pnpm install --frozen-lockfile && pnpm run build) || { rm -rf ui/dist; exit 5; }
  fi
  rmdir "$LOCK"
  trap - EXIT
fi

echo "==> $BUILD --release --target $TRIPLE (agent-router, claude-wrapper)"
$BUILD --release --target "$TRIPLE" -p agent-router -p claude-wrapper
OUT="dist/$VT"
mkdir -p "$OUT"
TDIR="${CARGO_TARGET_DIR:-target}"

# Apple Silicon kills an unsigned program the moment it starts, and only on the user's Mac.
# A Mac's linker signs ad hoc, and so does zig's; this holds a cross build to it, so a zig
# that stopped signing fails here rather than there. The test is the LC_CODE_SIGNATURE load
# command (0x1d) in the 64-bit Mach-O header.
if [ "$TRIPLE" = aarch64-apple-darwin ] && [ "$BUILD" = "cargo zigbuild" ]; then
  for prog in agent-router claude-wrapper; do
    python3 -c '
import struct, sys
data = open(sys.argv[1], "rb").read()
magic, ncmds = struct.unpack_from("<I12xI", data, 0)
off = 32
for _ in range(ncmds if magic == 0xFEEDFACF else 0):
    cmd, size = struct.unpack_from("<2I", data, off)
    if cmd == 0x1D:
        sys.exit(0)
    off += size
sys.exit(1)
' "$TDIR/$TRIPLE/release/$prog" || {
      echo "$prog for $TRIPLE carries no code signature, and macOS would kill it at launch" >&2
      exit 6
    }
  done
fi

cp "$TDIR/$TRIPLE/release/agent-router$EXE" "$TDIR/$TRIPLE/release/claude-wrapper$EXE" "$OUT/"
ls -la "$OUT"
