#!/bin/bash
# Build a native macOS QADREN.app from SwiftUI + a Rust static library.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

for cmd in cargo xcrun plutil; do
  command -v "$cmd" >/dev/null || { echo "Missing build tool: $cmd" >&2; exit 1; }
done

ARCH="$(uname -m)"
case "$ARCH" in
  arm64) TARGET="aarch64-apple-darwin" ;;
  x86_64) TARGET="x86_64-apple-darwin" ;;
  *) echo "Unsupported macOS arch: $ARCH" >&2; exit 1 ;;
esac

# Apply one minimum deployment target to Rust, bundled C dependencies (SQLite) and Swift.\nexport MACOSX_DEPLOYMENT_TARGET=13.0\ncargo build --release --manifest-path core/Cargo.toml --target "$TARGET"
APP="$ROOT/dist/QADREN.app"
mkdir -p "$APP/Contents/MacOS"
cp platform/macos/Info.plist "$APP/Contents/Info.plist"
plutil -lint "$APP/Contents/Info.plist"

xcrun swiftc -O -target "$ARCH-apple-macos13.0" \
  -framework SwiftUI -framework AppKit -framework Foundation \
  platform/macos/QADREN/*.swift \
  "$ROOT/core/target/$TARGET/release/libqadren_core.a" \
  -o "$APP/Contents/MacOS/QADREN"

echo "Built $APP"
echo "Run: open '$APP'"
echo "Unsigned local developer build; no iCloud entitlement is configured."
