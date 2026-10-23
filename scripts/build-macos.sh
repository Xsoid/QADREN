#!/bin/bash
# Build a native macOS QADREN.app from SwiftUI + a Rust static library.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

for cmd in cargo xcrun plutil codesign; do
  command -v "$cmd" >/dev/null || { echo "Missing build tool: $cmd" >&2; exit 1; }
done

ARCH="$(uname -m)"
case "$ARCH" in
  arm64) TARGET="aarch64-apple-darwin" ;;
  x86_64) TARGET="x86_64-apple-darwin" ;;
  *) echo "Unsupported macOS arch: $ARCH" >&2; exit 1 ;;
esac

# Apply one minimum deployment target to Rust, bundled C dependencies (SQLite) and Swift.
export MACOSX_DEPLOYMENT_TARGET=13.0
cargo build --release --manifest-path core/Cargo.toml --target "$TARGET"
cargo build --release --manifest-path core/Cargo.toml --target "$TARGET" --bin qadren-worker
APP="$ROOT/dist/QADREN.app"
mkdir -p "$APP/Contents/MacOS"
mkdir -p "$APP/Contents/Resources"
mkdir -p "$APP/Contents/Library/LaunchAgents"
cp platform/macos/Info.plist "$APP/Contents/Info.plist"
cp assets/qadren.icns "$APP/Contents/Resources/qadren.icns"
cp assets/qadren-logo.png "$APP/Contents/Resources/qadren-logo.png"
# Replace the executable with a new inode. An in-place overwrite keeps the old
# code-signature cache for the inode, and the kernel then kills the new Worker
# with SIGKILL at launch.
rm -f "$APP/Contents/Resources/qadren-worker"
cp "$ROOT/core/target/$TARGET/release/qadren-worker" "$APP/Contents/Resources/qadren-worker"
cp platform/macos/LaunchAgents/dev.qadren.worker.plist "$APP/Contents/Library/LaunchAgents/dev.qadren.worker.plist"
# Embed public OAuth Client ID from an untracked local build config.
CONFIG="$ROOT/config/oauth.local.env"
if [[ -f "$CONFIG" ]]; then
  CLIENT_ID=""
  while IFS= read -r line || [[ -n "$line" ]]; do
    [[ -z "$line" || "$line" =~ ^[[:space:]]*# ]] && continue
    if [[ "$line" =~ ^YANDEX_CLIENT_ID=([a-zA-Z0-9_-]+)$ ]]; then
      CLIENT_ID="${BASH_REMATCH[1]}"
    else
      echo "Invalid local OAuth config line; expected YANDEX_CLIENT_ID=..." >&2
      exit 1
    fi
  done < "$CONFIG"
  if [[ -z "$CLIENT_ID" ]]; then
    echo "Missing YANDEX_CLIENT_ID in $CONFIG" >&2
    exit 1
  fi
  plutil -insert QADRENYandexClientID -string "$CLIENT_ID" "$APP/Contents/Info.plist"
fi
plutil -lint "$APP/Contents/Info.plist"

xcrun swiftc -O -target "$ARCH-apple-macos13.0" \
  -framework SwiftUI -framework AppKit -framework Foundation -framework ServiceManagement \
  platform/macos/QADREN/*.swift \
  "$ROOT/core/target/$TARGET/release/libqadren_core.a" \
  -o "$APP/Contents/MacOS/QADREN"

# Seal the bundle with an ad-hoc signature. Without a sealed bundle macOS does
# not accept Contents/Library/LaunchAgents as part of the app, and
# SMAppService reports the Worker service as not found. Sign the nested Worker
# first, then the app. This is an internal build, not a Developer ID release.
codesign --force --sign - --identifier dev.qadren.worker "$APP/Contents/Resources/qadren-worker"
codesign --force --sign - --identifier "$(plutil -extract CFBundleIdentifier raw "$APP/Contents/Info.plist")" "$APP"
codesign --verify --strict "$APP"

# Finder uses the bundle directory modification date to invalidate its icon cache.
touch "$APP"

echo "Built $APP"
echo "Run: open '$APP'"
echo "Ad-hoc signed local developer build (not Developer ID); no iCloud entitlement is configured."
