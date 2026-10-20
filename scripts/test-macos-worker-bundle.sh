#!/bin/bash
# Build the developer bundle and verify the immutable SMAppService payload.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

for cmd in bash plutil test; do
  command -v "$cmd" >/dev/null || { echo "Missing test tool: $cmd" >&2; exit 1; }
done

bash scripts/build-macos.sh

APP="$ROOT/dist/QADREN.app"
AGENT="$APP/Contents/Library/LaunchAgents/dev.qadren.worker.plist"
WORKER="$APP/Contents/Resources/qadren-worker"

[[ -x "$APP/Contents/MacOS/QADREN" ]] || { echo "Missing app executable" >&2; exit 1; }
[[ -x "$WORKER" ]] || { echo "Missing bundled Worker executable" >&2; exit 1; }
plutil -lint "$AGENT"
[[ "$(plutil -extract Label raw "$AGENT")" == "dev.qadren.worker" ]] || {
  echo "Unexpected LaunchAgent label" >&2
  exit 1
}
[[ "$(plutil -extract BundleProgram raw "$AGENT")" == "Contents/Resources/qadren-worker" ]] || {
  echo "Unexpected LaunchAgent Worker path" >&2
  exit 1
}
[[ "$(plutil -extract RunAtLoad raw "$AGENT")" == "true" ]] || {
  echo "LaunchAgent must start the Worker at load" >&2
  exit 1
}

echo "PASS: QADREN bundle contains the SMAppService LaunchAgent and Worker payload"
