#!/bin/bash
# Native proof that the bundled Worker runs as an SMAppService LaunchAgent
# and keeps working after the user quits QADREN.
#
# Opt-in: it registers a temporary Login Item. Run it with
#   QADREN_AGENT_TEST=1 bash scripts/test-macos-worker-agent.sh
# It uses a test copy of the app (bundle id dev.qadren.e2e.agent, service
# label dev.qadren.e2e.worker) on a disposable data root, never the user's
# real service or data, and always unregisters the service at the end.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

if [[ "${QADREN_AGENT_TEST:-}" != "1" ]]; then
  echo "SKIPPED: set QADREN_AGENT_TEST=1 to run the LaunchAgent proof"
  exit 0
fi
for cmd in xcrun codesign plutil launchctl nc sqlite3 open osascript; do
  command -v "$cmd" >/dev/null || { echo "Missing test tool: $cmd" >&2; exit 1; }
done
ARCH="$(uname -m)"
case "$ARCH" in
  arm64) TARGET="aarch64-apple-darwin" ;;
  x86_64) TARGET="x86_64-apple-darwin" ;;
  *) echo "Unsupported macOS arch: $ARCH" >&2; exit 1 ;;
esac
[[ "${SKIP_BUILD:-0}" == "1" ]] || bash scripts/build-macos.sh >/dev/null

BUNDLE_ID="dev.qadren.e2e.agent"
LABEL="dev.qadren.e2e.worker"
DOMAIN="gui/$(id -u)"
TEST_ROOT="$(cd "$(mktemp -d /tmp/qa.XXXXXX)" && pwd -P)"
DATA="$TEST_ROOT/data"
mkdir -m 700 "$DATA"
APP="$TEST_ROOT/QADRENAgentTest.app"
AGENT_PLIST="$APP/Contents/Library/LaunchAgents/dev.qadren.worker.plist"
ditto "$ROOT/dist/QADREN.app" "$APP"
plutil -replace CFBundleIdentifier -string "$BUNDLE_ID" "$APP/Contents/Info.plist"
plutil -replace Label -string "$LABEL" "$AGENT_PLIST"
plutil -insert EnvironmentVariables -json "{\"QADREN_DATA_ROOT\":\"$DATA\"}" "$AGENT_PLIST"
codesign --force --sign - --identifier dev.qadren.worker "$APP/Contents/Resources/qadren-worker"
codesign --force --sign - --identifier "$BUNDLE_ID" "$APP"

DRIVER="$TEST_ROOT/settings-e2e-driver"
REGTOOL="$TEST_ROOT/settings-e2e-registry"
xcrun swiftc -O -target "$ARCH-apple-macos13.0" -framework AppKit -framework ApplicationServices \
  platform/macos/Tests/SettingsE2EDriver.swift -o "$DRIVER"
xcrun swiftc -O -target "$ARCH-apple-macos13.0" -framework Foundation -framework Security \
  platform/macos/QADREN/JobInventory.swift platform/macos/QADREN/WorkerIPC.swift \
  platform/macos/QADREN/StorageRegistry.swift platform/macos/Tests/SettingsE2ERegistryTool.swift \
  "$ROOT/core/target/$TARGET/release/libqadren_core.a" -o "$REGTOOL"

APP_PID=""
step() { echo "== $*"; }
fail() { echo "FAIL: $*" >&2; exit 1; }
app_pid() { pgrep -n -f "$APP/Contents/MacOS/QADREN" || true; }
worker_pid() { launchctl print "$DOMAIN/$LABEL" 2>/dev/null | awk '/^\tpid = / {print $3}'; }
worker_status() { echo '{"command":"status"}' | nc -U -w 3 "$DATA/worker.sock" 2>/dev/null || true; }
wait_for() {
  local seconds="$1"; shift
  for _ in $(seq 1 $((seconds * 5))); do "$@" && return 0; sleep 0.2; done
  "$@"
}
worker_running() { [[ -n "$(worker_pid)" ]] && worker_status | grep -F '"status":"running"' >/dev/null; }
launch_app() {
  open -n -F --env "QADREN_DATA_ROOT=$DATA" "$@"
  for _ in $(seq 1 50); do APP_PID="$(app_pid)"; [[ -n "$APP_PID" ]] && break; sleep 0.2; done
  [[ -n "$APP_PID" ]] || fail "the app did not start"
}
quit_app() {
  osascript -e "tell application id \"$BUNDLE_ID\" to quit" >/dev/null 2>&1 || true
  wait_for 15 bash -c "[[ -z \"\$(pgrep -f '$APP/Contents/MacOS/QADREN')\" ]]" || fail "the app did not quit"
  APP_PID=""
}
cleanup() {
  set +e
  [[ -n "$(app_pid)" ]] && kill "$(app_pid)" 2>/dev/null
  # The app unregisters the service of its own bundle and exits.
  if [[ -n "$(worker_pid)" ]] || launchctl print "$DOMAIN/$LABEL" >/dev/null 2>&1; then
    open -n -W -F --env "QADREN_DATA_ROOT=$DATA" "$APP" --args -QADRENUnregisterWorkerAgent YES >/dev/null 2>&1 || true
    sleep 2
  fi
  launchctl print "$DOMAIN/$LABEL" >/dev/null 2>&1 && launchctl bootout "$DOMAIN/$LABEL" 2>/dev/null
  rm -rf -- "$TEST_ROOT"
}
trap cleanup EXIT

step "The app registers the bundled Worker as a LaunchAgent"
launch_app --env QADREN_REGISTER_AGENT=1 "$APP" --args -ApplePersistenceIgnoreState YES
if ! wait_for 30 worker_running; then
  "$DRIVER" "$APP_PID" dump monitoring 2>/dev/null | grep -E "Служба|Worker" >&2 || true
  fail "launchd did not start a responding Worker: $(launchctl print "$DOMAIN/$LABEL" 2>&1 | head -3)"
fi
"$DRIVER" "$APP_PID" wait monitoring AXStaticText "фоновая служба зарегистрирована" 15 || fail "the UI does not show the registered service"
"$DRIVER" "$APP_PID" wait monitoring AXStaticText "Worker отвечает" 15 || fail "the UI does not see the Worker"
FIRST_PID="$(worker_pid)"
echo "   launchd Worker pid $FIRST_PID"

step "A schedule is set and the user quits QADREN"
SRC="$TEST_ROOT/src"; DST="$TEST_ROOT/dst"; mkdir -p "$SRC" "$DST"
printf 'made by the background Worker\n' > "$SRC/after-quit.txt"
TZ_NAME="$(readlink /etc/localtime | sed 's#.*/zoneinfo/##')"
SLOT="$(date -v+90S '+%u %H %M')"
read -r DOW HOUR MINUTE <<<"$SLOT"
QADREN_DATA_ROOT="$DATA" "$REGTOOL" setup-scheduled "E2E agent" "$SRC" "$DST" "$TZ_NAME" $((DOW - 1)) $((10#$HOUR)) $((10#$MINUTE)) >/dev/null \
  || fail "could not prepare the scheduled Scenario"
quit_app
[[ "$(worker_pid)" == "$FIRST_PID" ]] || fail "quitting the app stopped or replaced the Worker"
worker_running || fail "the Worker does not answer after the app quit"
echo "   the app quit; Worker pid $FIRST_PID still answers"

step "The schedule runs while the app is not running"
if ! wait_for 240 test -f "$DST/after-quit.txt"; then
  echo "   slot $SLOT tz $TZ_NAME now $(date '+%u %H:%M:%S')" >&2
  sqlite3 "$DATA/connections.sqlite3" "select id, enabled, adapter from storage_connections; select weekday, hour, minute, timezone, enabled from scenario_schedules; select * from schedule_job_outbox;" >&2 || true
  sqlite3 "$DATA/jobs.sqlite3" "select id, state from jobs" >&2 || true
  fail "the slot did not run while the app was closed"
fi
cmp -s "$SRC/after-quit.txt" "$DST/after-quit.txt" || fail "scheduled bytes differ"
[[ -z "$(app_pid)" ]] || fail "the app was started during the check"

step "launchd restarts the Worker after a crash"
kill -9 "$FIRST_PID"
wait_for 30 bash -c "p=\$(launchctl print '$DOMAIN/$LABEL' 2>/dev/null | awk '/^\tpid = / {print \$3}'); [[ -n \"\$p\" && \"\$p\" != '$FIRST_PID' ]]" \
  || fail "launchd did not restart the crashed Worker"
wait_for 20 worker_running || fail "the restarted Worker does not answer"
echo "   restarted Worker pid $(worker_pid)"

step "Reopened QADREN sees the Worker and the scheduled result"
launch_app "$APP" --args -ApplePersistenceIgnoreState YES
"$DRIVER" "$APP_PID" wait monitoring AXStaticText "Worker отвечает" 20 || fail "the reopened UI does not see the Worker"
"$DRIVER" "$APP_PID" wait monitoring AXStaticText "Задания: Готово" 20 || fail "the reopened UI does not show the completed scheduled Jobs"
quit_app

step "Unregister removes the service"
open -n -W -F --env "QADREN_DATA_ROOT=$DATA" "$APP" --args -QADRENUnregisterWorkerAgent YES >/dev/null 2>&1 || true
wait_for 20 bash -c "! launchctl print '$DOMAIN/$LABEL' >/dev/null 2>&1" || fail "the service is still loaded after unregister"

echo "PASS: the bundled Worker runs as a LaunchAgent and keeps working after Quit"
