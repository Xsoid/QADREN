#!/bin/bash
# Compile and run the native Swift-to-Worker Scenario Create and Update bridge test.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

for cmd in cargo xcrun mktemp; do
  command -v "$cmd" >/dev/null || { echo "Missing test tool: $cmd" >&2; exit 1; }
done

ARCH="$(uname -m)"
case "$ARCH" in
  arm64) TARGET="aarch64-apple-darwin" ;;
  x86_64) TARGET="x86_64-apple-darwin" ;;
  *) echo "Unsupported macOS arch: $ARCH" >&2; exit 1 ;;
esac

export MACOSX_DEPLOYMENT_TARGET=13.0
cargo build --release --manifest-path core/Cargo.toml --target "$TARGET" --bin qadren-worker --lib
TEST_ROOT="$(mktemp -d)"
trap 'rm -rf -- "$TEST_ROOT"' EXIT

xcrun swiftc -O -target "$ARCH-apple-macos13.0" \
  -framework Foundation -framework Security \
  platform/macos/QADREN/JobInventory.swift \
  platform/macos/QADREN/WorkerIPC.swift \
  platform/macos/QADREN/StorageRegistry.swift \
  platform/macos/Tests/WorkerIPCBridgeTests.swift \
  "$ROOT/core/target/$TARGET/release/libqadren_core.a" \
  -o "$TEST_ROOT/worker-ipc-tests"
"$TEST_ROOT/worker-ipc-tests" "$ROOT/core/target/$TARGET/release/qadren-worker"
