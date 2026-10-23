#!/bin/bash
# Native E2E of the rendered Settings window through macOS Accessibility.
# It runs a built QADREN.app and a local Worker on a disposable data root
# (QADREN_DATA_ROOT). The user's Registry, JobStore, and files stay untouched.
# Requirements: a logged-in macOS session and Accessibility permission for the
# process that runs this script. The script takes keyboard focus briefly.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

for cmd in xcrun sqlite3 defaults open pgrep; do
  command -v "$cmd" >/dev/null || { echo "Missing test tool: $cmd" >&2; exit 1; }
done

ARCH="$(uname -m)"
[[ "${SKIP_BUILD:-0}" == "1" ]] || bash scripts/build-macos.sh >/dev/null

# The Unix socket path must stay short, so use /tmp.
TEST_ROOT="$(cd "$(mktemp -d /tmp/qe.XXXXXX)" && pwd -P)"
# Run a private copy of the bundle. Another check can rebuild dist/ meanwhile.
APP="$TEST_ROOT/QADREN.app"
ditto "$ROOT/dist/QADREN.app" "$APP"
WORKER="$APP/Contents/Resources/qadren-worker"
BUNDLE_ID="$(plutil -extract CFBundleIdentifier raw "$APP/Contents/Info.plist")"
DATA="$TEST_ROOT/data"
SRC="$TEST_ROOT/src"
DST="$TEST_ROOT/dst"
mkdir -m 700 "$DATA"
mkdir -p "$SRC/sub" "$DST"
DRIVER="$TEST_ROOT/settings-e2e-driver"
xcrun swiftc -O -target "$ARCH-apple-macos13.0" -framework AppKit -framework ApplicationServices \
  platform/macos/Tests/SettingsE2EDriver.swift -o "$DRIVER"

# The Finder open panel starts in NSNavLastRootDirectory. Keep the user's value.
NAV_KEY=NSNavLastRootDirectory
OLD_NAV="$(defaults read "$BUNDLE_ID" "$NAV_KEY" 2>/dev/null || true)"
APP_PID=""
WORKER_PID=""
cleanup() {
  [[ -n "$APP_PID" ]] && kill "$APP_PID" 2>/dev/null || true
  [[ -n "$WORKER_PID" ]] && kill "$WORKER_PID" 2>/dev/null || true
  if [[ -n "$OLD_NAV" ]]; then defaults write "$BUNDLE_ID" "$NAV_KEY" "$OLD_NAV"
  else defaults delete "$BUNDLE_ID" "$NAV_KEY" 2>/dev/null || true; fi
  rm -rf -- "$TEST_ROOT"
}
trap cleanup EXIT

step() { echo "== $*"; }
fail() {
  echo "FAIL: $*" >&2
  [[ -n "$APP_PID" ]] && "$DRIVER" "$APP_PID" dump Настройки 2>/dev/null | head -80 >&2
  exit 1
}
ax() { "$DRIVER" "$APP_PID" "$@" || fail "step failed: $*"; }
jobs_sql() { sqlite3 "$DATA/jobs.sqlite3" "$1"; }
wait_for() {
  local seconds="$1"; shift
  for _ in $(seq 1 $((seconds * 5))); do "$@" && return 0; sleep 0.2; done
  "$@"
}

# Choose a folder through the in-app browser and the Finder open panel.
choose_folder() {
  local index="$1" folder="$2"
  defaults write "$BUNDLE_ID" "$NAV_KEY" "$folder"
  ax press Настройки AXButton "Открыть браузер папок" "$index"
  ax wait Настройки AXButton "Выбрать папку в Finder"
  ax press Настройки AXButton "Выбрать папку в Finder"
  ax wait Настройки AXButton "OKButton"
  ax press Настройки AXButton "OKButton"
  ax wait Настройки AXStaticText "$folder"
  ax press Настройки AXButton "Использовать эту папку"
  ax gone Настройки AXButton "Использовать эту папку"
}

printf 'alpha\n' > "$SRC/a.txt"
printf 'bravo\n' > "$SRC/b.txt"
printf 'charlie\n' > "$SRC/sub/c.txt"

step "Start the Worker and the app on the disposable data root"
"$WORKER" --socket "$DATA/worker.sock" --registry "$DATA/connections.sqlite3" --jobs "$DATA/jobs.sqlite3" \
  >"$TEST_ROOT/worker.log" 2>&1 &
WORKER_PID=$!
wait_for 10 test -S "$DATA/worker.sock" || fail "Worker socket did not appear"
open -n -F --env "QADREN_DATA_ROOT=$DATA" "$APP" --args -ApplePersistenceIgnoreState YES
for _ in $(seq 1 50); do
  APP_PID="$(pgrep -n -f "$APP/Contents/MacOS/QADREN" || true)"
  [[ -n "$APP_PID" ]] && ps eww -p "$APP_PID" | grep -q "QADREN_DATA_ROOT=$DATA" && break
  APP_PID=""; sleep 0.2
done
[[ -n "$APP_PID" ]] || fail "test app did not start"
ax wait monitoring AXStaticText "Worker отвечает" 15

step "Add a Local connection and create a one-way Scenario in Settings"
ax menu Window Настройки
ax wait Настройки AXButton "Добавить этот Mac"
ax press Настройки AXButton "Добавить этот Mac"
ax wait Настройки AXTextField "Этот Mac"
ax select Настройки "Сценарии"
ax wait Настройки AXButton "Создать сценарий"
ax press Настройки AXButton "Создать сценарий"
ax wait Настройки AXTextField "Название сценария"
ax set Настройки AXTextField "Название сценария" "E2E one-way"
choose_folder 0 "$SRC"
choose_folder 1 "$DST"
ax press Настройки AXButton "Создать сценарий" 0
ax wait Настройки AXStaticText "Ревизия 1"

step "Preview the plan and run the Create batch"
ax press Настройки AXButton "Проверить план" 0
ax wait Настройки AXButton "Создать все: 3"
ax press Настройки AXButton "Создать все: 3"
wait_for 20 test -f "$DST/sub/c.txt" || fail "nested file was not created"
cmp -s "$SRC/a.txt" "$DST/a.txt" && cmp -s "$SRC/b.txt" "$DST/b.txt" && cmp -s "$SRC/sub/c.txt" "$DST/sub/c.txt" \
  || fail "destination bytes differ from the source"
ax gone Настройки "*" "Выполнение пакета" 20
ax wait Настройки AXStaticText "без действия" 10
ax gone Настройки AXButton "Создать копию" 10
[[ "$(jobs_sql "select count(*) from jobs where state='completed' and scenario_id is not null")" == 3 ]] \
  || fail "expected 3 completed Scenario Jobs: $(jobs_sql 'select id, state from jobs')"
[[ "$(jobs_sql "select count(*) from jobs where state<>'completed'")" == 0 ]] || fail "a Job is not completed"

step "Update: declining the replacement changes nothing"
printf 'alpha changed by the source\n' > "$SRC/a.txt"
ax press Настройки AXButton "Проверить план" 0
ax wait Настройки AXButton "Заменить файл"
ax press Настройки AXButton "Заменить файл" 0
ax wait Настройки AXStaticText "Заменить существующий файл?"
ax alert Настройки "Отмена"
ax gone Настройки AXStaticText "Заменить существующий файл?"
[[ "$(cat "$DST/a.txt")" == "alpha" ]] || fail "declined replacement changed the destination"
[[ "$(jobs_sql "select count(*) from jobs")" == 3 ]] || fail "declined replacement created a Job"

step "Update: confirmed replacement writes the exact source bytes"
ax press Настройки AXButton "Заменить файл" 0
ax wait Настройки AXStaticText "Заменить существующий файл?"
ax alert Настройки "Заменить файл"
wait_for 20 cmp -s "$SRC/a.txt" "$DST/a.txt" || fail "confirmed replacement did not write the source bytes"
ax gone Настройки AXButton "Заменить файл" 20
[[ "$(jobs_sql "select count(*) from jobs where state='completed'")" == 4 ]] \
  || fail "expected 4 completed Jobs: $(jobs_sql 'select id, state from jobs')"

step "Close and reopen Settings: the Scenario and its state persist"
ax close Настройки
ax menu Window Настройки
ax wait Настройки AXStaticText "Сценарии" 10
ax select Настройки "Сценарии"
ax wait Настройки AXStaticText "E2E one-way"
ax press Настройки AXButton "Проверить план" 0
ax wait Настройки AXStaticText "без действия" 20
ax gone Настройки AXButton "Создать копию" 2
ax gone Настройки AXButton "Заменить файл" 2

step "Monitoring shows the completed Jobs from the same JobStore"
ax wait monitoring AXStaticText "Задания: Готово" 15

echo "Settings native E2E passed"
