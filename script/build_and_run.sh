#!/usr/bin/env bash
# Local macOS development entrypoint for the already-bundled QADREN.app.
set -euo pipefail

MODE="run"
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP_NAME="QADREN"
APP_BUNDLE="$ROOT_DIR/dist/$APP_NAME.app"
APP_BINARY="$APP_BUNDLE/Contents/MacOS/$APP_NAME"
BUNDLE_ID="dev.qadren.bootstrap"
APP_ARGUMENTS=()

while (($#)); do
  case "$1" in
    run|--debug|debug|--logs|logs|--telemetry|telemetry|--verify|verify)
      MODE="$1"
      shift
      ;;
    --theme)
      [[ $# -ge 2 ]] || { echo "--theme requires dark, light, or system" >&2; exit 2; }
      case "$2" in
        dark|light|system) APP_ARGUMENTS=(--args -qadren.theme "$2") ;;
        *) echo "unsupported theme: $2" >&2; exit 2 ;;
      esac
      shift 2
      ;;
    *)
      echo "usage: $0 [run|--debug|--logs|--telemetry|--verify] [--theme dark|light|system]" >&2
      exit 2
      ;;
  esac
done

if [[ ${#APP_ARGUMENTS[@]} -gt 0 && ( "$MODE" == "--debug" || "$MODE" == "debug" ) ]]; then
  echo "--theme is supported only when launching the app bundle" >&2
  exit 2
fi

stop_running_app() {
  pkill -x "$APP_NAME" >/dev/null 2>&1 || true
}

build_app() {
  bash "$ROOT_DIR/scripts/build-macos.sh"
}

open_app() {
  /usr/bin/open -n "$APP_BUNDLE" "${APP_ARGUMENTS[@]}"
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
    echo "usage: $0 [run|--debug|--logs|--telemetry|--verify] [--theme dark|light|system]" >&2
    exit 2
    ;;
esac
