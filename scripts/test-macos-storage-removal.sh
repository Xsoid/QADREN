#!/bin/bash
# Compile and run the native Swift-to-Rust Registry removal bridge test.
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
cargo build --release --manifest-path core/Cargo.toml --target "$TARGET"
TEST_ROOT="$(mktemp -d)"
trap 'rm -rf -- "$TEST_ROOT"' EXIT

xcrun swiftc -O -target "$ARCH-apple-macos13.0" \
  -framework Foundation -framework Security \
  platform/macos/QADREN/JobInventory.swift \
  platform/macos/QADREN/WorkerIPC.swift \
  platform/macos/QADREN/StorageRegistry.swift \
  platform/macos/Tests/StorageRegistryRemovalTests.swift \
  "$ROOT/core/target/$TARGET/release/libqadren_core.a" \
  -o "$TEST_ROOT/storage-registry-removal-tests"
"$TEST_ROOT/storage-registry-removal-tests"
