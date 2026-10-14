#!/bin/bash
# Compile and run local-only monitoring layout persistence tests.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

for cmd in xcrun mktemp; do
  command -v "$cmd" >/dev/null || { echo "Missing test tool: $cmd" >&2; exit 1; }
done

ARCH="$(uname -m)"
case "$ARCH" in
  arm64|x86_64) ;;
  *) echo "Unsupported macOS arch: $ARCH" >&2; exit 1 ;;
esac

TEST_ROOT="$(mktemp -d)"
trap 'rm -rf -- "$TEST_ROOT"' EXIT
xcrun swiftc -O -target "$ARCH-apple-macos13.0" \
  -framework Foundation \
  platform/macos/QADREN/DashboardLayout.swift \
  platform/macos/Tests/DashboardLayoutTests.swift \
  -o "$TEST_ROOT/dashboard-layout-tests"
"$TEST_ROOT/dashboard-layout-tests"
