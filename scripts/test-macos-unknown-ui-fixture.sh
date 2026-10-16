#!/bin/bash
# Verify the feature-gated fixture used only to prepare native UNKNOWN UI review.
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
FIXTURE="$TEST_ROOT/unknown-job-fixture"
SUPPORT="$TEST_ROOT/home/Library/Application Support/QADREN"

xcrun swiftc -parse-as-library -O -target "$ARCH-apple-macos13.0" \
  -framework Foundation \
  platform/macos/Tests/UnknownJobFixture.swift \
  "$ROOT/core/target/$TARGET/release/libqadren_core.a" \
  -o "$FIXTURE"

mkdir -p "$SUPPORT"
"$FIXTURE" seed "$SUPPORT/jobs.sqlite3" "$SUPPORT/G1MockStorage" ui-unknown-fixture
"$FIXTURE" reconcile "$SUPPORT/jobs.sqlite3" "$SUPPORT/G1MockStorage" ui-unknown-fixture

echo "Unknown UI fixture native test passed"
