#!/bin/bash
# Compile and run the native metadata-only Scenario dry-run bridge test.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

# A selected Scenario Create is already the user's command. Ordinary source
# reads, including File Provider materialization, must not add a second prompt.
if rg -q 'Разрешить загрузку и создание копии|Загрузить и создать|pendingCreate|executePendingCreate' \
  platform/macos/QADREN/SettingsPanel.swift; then
  echo "Scenario Create still contains an obsolete ordinary-read confirmation" >&2
  exit 1
fi
# The owner removed the standalone file-manager window. Local folder selection
# stays in Scenario Settings through LocalWorkspace.
if rg -q 'file-manager|FileManagerPane|filesWorkspace' platform/macos/QADREN; then
  echo "The removed standalone file-manager window is back" >&2
  exit 1
fi
rg -q 'Button\("Создать копию"\) \{ executeCreate\(item\) \}' \
  platform/macos/QADREN/SettingsPanel.swift
rg -Fq 'Button("Создать все: \(creates.count)") {' \
  platform/macos/QADREN/SettingsPanel.swift
rg -q 'resumePendingCreateBatches\(\)' platform/macos/QADREN/SettingsPanel.swift
rg -q 'pendingScenarioCreateBatches\(scenario: scenario\)' \
  platform/macos/QADREN/SettingsPanel.swift
rg -q 'deleteConflictDestinationDescription' platform/macos/QADREN/SettingsPanel.swift

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
  platform/macos/Tests/ScenarioDryRunBridgeTests.swift \
  "$ROOT/core/target/$TARGET/release/libqadren_core.a" \
  -o "$TEST_ROOT/scenario-dry-run-tests"
"$TEST_ROOT/scenario-dry-run-tests"
