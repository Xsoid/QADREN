#!/usr/bin/env bash
# Local macOS development entrypoint for the already-bundled QADREN.app.
set -euo pipefail

MODE="${1:-run}"
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP_NAME="QADREN"
APP_BUNDLE="$ROOT_DIR/dist/$APP_NAME.app"
APP_BINARY="$APP_BUNDLE/Contents/MacOS/$APP_NAME"
BUNDLE_ID="dev.qadren.bootstrap"

stop_running_app() {
  pkill -x "$APP_NAME" >/dev/null 2>&1 || true
}

build_app() {
  bash "$ROOT_DIR/scripts/build-macos.sh"
}

open_app() {
  /usr/bin/open -n "$APP_BUNDLE"
}

stop_running_app
build_app

case "$MODE" in
  run)
    open_app
    ;;
  --debug|debug)
    exec lldb -- "$APP_BINARY"
    ;;
  --logs|logs)
    open_app
    exec /usr/bin/log stream --info --style compact --predicate "process == \"$APP_NAME\""
    ;;
  --telemetry|telemetry)
    open_app
    exec /usr/bin/log stream --info --style compact --predicate "subsystem == \"$BUNDLE_ID\""
    ;;
  --verify|verify)
    open_app
    sleep 1
    pgrep -x "$APP_NAME" >/dev/null
    echo "Verified running $APP_NAME process"
    ;;
  *)
    echo "usage: $0 [run|--debug|--logs|--telemetry|--verify]" >&2
    exit 2
    ;;
esac
