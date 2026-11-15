#!/bin/bash
# Compile and run the native LocalWorkspace -> Rust Local Job controller test.
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
cargo build --release --features ui-test-fixtures --manifest-path core/Cargo.toml --target "$TARGET"
TEST_ROOT="$(mktemp -d)"
trap 'rm -rf -- "$TEST_ROOT"' EXIT
# A native test never opens the real QADREN data root.
mkdir -p "$TEST_ROOT/data-root"
export QADREN_DATA_ROOT="$(cd "$TEST_ROOT/data-root" && pwd -P)"
# A replaced file goes to this Trash, never to the Trash of the user.
mkdir -p "$TEST_ROOT/trash"
export QADREN_TEST_TRASH_DIR="$(cd "$TEST_ROOT/trash" && pwd -P)"

xcrun swiftc -O -target "$ARCH-apple-macos13.0" \
  -framework AppKit -framework Foundation -framework Security \
  platform/macos/QADREN/WorkerIPC.swift \
  platform/macos/QADREN/StorageRegistry.swift \
  platform/macos/QADREN/Bridge.swift \
  platform/macos/QADREN/LoginHandoff.swift \
  platform/macos/QADREN/ISPmanagerConnection.swift \
  platform/macos/QADREN/NetworkShare.swift \
  platform/macos/QADREN/DesignSystem.swift \
  platform/macos/QADREN/CyberDesign.swift \
  platform/macos/QADREN/WindowVisibility.swift \
  platform/macos/QADREN/StorageConnections.swift \
  platform/macos/QADREN/ConnectionRemoval.swift \
  platform/macos/QADREN/JobInventory.swift \
  platform/macos/QADREN/LocalWorkspace.swift \
  platform/macos/Tests/LocalWorkspaceTests.swift \
  "$ROOT/core/target/$TARGET/release/libqadren_core.a" \
  -o "$TEST_ROOT/local-workspace-tests"
"$TEST_ROOT/local-workspace-tests"
