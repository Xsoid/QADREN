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
case "$ARCH" in
  arm64) TARGET="aarch64-apple-darwin" ;;
  x86_64) TARGET="x86_64-apple-darwin" ;;
  *) echo "Unsupported macOS arch: $ARCH" >&2; exit 1 ;;
esac
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
  if [[ -n "$APP_PID" ]]; then
    "$DRIVER" "$APP_PID" dump Настройки >"$TEST_ROOT/failure-dump.txt" 2>/dev/null || true
    sed -n 1,80p "$TEST_ROOT/failure-dump.txt" >&2
  fi
  # Debug aid: keep the app open for inspection.
  [[ -n "${E2E_PAUSE_ON_FAIL:-}" ]] && { echo "paused: pid $APP_PID root $TEST_ROOT" >&2; sleep 600; }
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
  # The Scenario editor has one inline folder browser per endpoint. Open the
  # path field of panel $index, go to the folder, and make it the root. The
  # browser does not follow symbolic links, so use the physical path.
  local index="$1" folder
  folder="$(cd "$2" && pwd -P)"
  ax press Настройки AXButton "Перейти к папке" "$index"
  ax wait Настройки AXTextField "Путь к папке"
  ax set Настройки AXTextField "Путь к папке" "$folder"
  ax press Настройки AXButton "Открыть путь" 0
  ax gone Настройки AXTextField "Путь к папке"
  ax press Настройки AXButton "Сделать корнем" "$index"
  # A protected folder can show a one-time macOS access prompt; the owner
  # answers it. Wait long enough for that answer.
  ax wait Настройки AXStaticText "$folder" 120
}

# Create a Scenario in the open Scenario list. $4=two_way selects both directions.
create_scenario() {
  local name="$1" left="$2" right="$3" mode="${4:-one_way}"
  ax press Настройки AXButton "Создать сценарий" 0
  ax wait Настройки AXTextField "Название сценария"
  ax set Настройки AXTextField "Название сценария" "$name"
  [[ "$mode" == "two_way" ]] && ax press Настройки AXButton "В обе стороны"
  choose_folder 0 "$left"
  choose_folder 1 "$right"
  ax press Настройки AXButton "Создать сценарий" 0
  ax wait Настройки AXStaticText "$name"
}
open_plan() {
  ax within Настройки "$1" AXButton "Проверить план"
  ax wait Настройки AXStaticText "План сценария"
  ax wait Настройки AXStaticText "$1" 10
}
regtool_build() {
  [[ -x "$TEST_ROOT/settings-e2e-registry" ]] && return 0
  REGTOOL="$TEST_ROOT/settings-e2e-registry"
  xcrun swiftc -O -target "$ARCH-apple-macos13.0" -framework Foundation -framework Security \
    platform/macos/QADREN/JobInventory.swift platform/macos/QADREN/WorkerIPC.swift \
    platform/macos/QADREN/StorageRegistry.swift platform/macos/Tests/SettingsE2ERegistryTool.swift \
    "$ROOT/core/target/$TARGET/release/libqadren_core.a" -o "$REGTOOL"
}
regtool() { QADREN_DATA_ROOT="$DATA" "$TEST_ROOT/settings-e2e-registry" "$@"; }
# E2E_SECTIONS="negative" runs only the named sections after the shared setup.
section() { [[ -z "${E2E_SECTIONS:-}" || " $E2E_SECTIONS " == *" $1 "* ]]; }
to_scenario_list() {
  if ! "$DRIVER" "$APP_PID" dump Настройки >/dev/null 2>&1; then
    echo "   WARNING: the Settings window was not open; opening it again" >&2
    ax menu Window Настройки
    ax wait Настройки AXStaticText "Сценарии" 10
  fi
  "$DRIVER" "$APP_PID" press Настройки AXButton "Назад" 2>/dev/null || true
  ax select Настройки "Сценарии"
  ax wait Настройки AXButton "Создать сценарий"
}
jobs_count() { jobs_sql "select count(*) from jobs"; }
registry_sql() { sqlite3 "$DATA/connections.sqlite3" "$1"; }
has_files() { [[ -n "$(ls -A "$1")" ]]; }

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
  [[ -n "$APP_PID" ]] && ps eww -p "$APP_PID" | grep -F "QADREN_DATA_ROOT=$DATA" >/dev/null && break
  APP_PID=""; sleep 0.2
done
[[ -n "$APP_PID" ]] || fail "test app did not start"
ax wait monitoring AXStaticText "Worker отвечает" 15

step "This Mac exists by default, and no second one can be added"
ax menu Window Настройки
ax wait Настройки AXTextField "Этот Mac"
[[ "$(sqlite3 "$DATA/connections.sqlite3" "select count(*) from storage_connections where adapter='local_filesystem'")" == 1 ]] \
  || fail "This Mac is not exactly one Local Filesystem connection"
ax press Настройки AXButton "Добавить подключение"
ax wait Настройки AXStaticText "Уже подключён" 5
ax press Настройки AXButton "Закрыть"
ax gone Настройки AXStaticText "Уже подключён" 5

if section one_way; then
step "Create a one-way Scenario in Settings"
to_scenario_list
create_scenario "E2E one-way" "$SRC" "$DST"

step "Preview the plan and run the Create batch"
open_plan "E2E one-way"
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

fi

if section two_way; then
step "Two-way: Create runs in both directions"
LEFT="$TEST_ROOT/left"; RIGHT="$TEST_ROOT/right"
mkdir -p "$LEFT" "$RIGHT"
printf 'left one\n' > "$LEFT/l1.txt"
printf 'right one\n' > "$RIGHT/r1.txt"
to_scenario_list
create_scenario "E2E two-way" "$LEFT" "$RIGHT" two_way
open_plan "E2E two-way"
ax wait Настройки AXButton "Создать все: 2"
ax press Настройки AXButton "Создать все: 2"
wait_for 20 test -f "$LEFT/r1.txt" || fail "right-to-left Create did not run"
wait_for 20 test -f "$RIGHT/l1.txt" || fail "left-to-right Create did not run"
ax gone Настройки "*" "Выполнение пакета" 20
cmp -s "$LEFT/l1.txt" "$RIGHT/l1.txt" && cmp -s "$LEFT/r1.txt" "$RIGHT/r1.txt" || fail "two-way bytes differ"

step "Two-way: a content conflict needs an explicit source and consent"
printf 'left edit, longer\n' > "$LEFT/l1.txt"
printf 'right edit\n' > "$RIGHT/l1.txt"
JOBS_BEFORE="$(jobs_sql "select count(*) from jobs")"
ax press Настройки AXButton "Проверить план" 0
ax wait Настройки AXStaticText "конфликт"
ax within Настройки "l1.txt" AXButton "Добавить в очередь конфликтов"
ax wait Настройки AXStaticText "Альтернативная очередь конфликтов"
ax pick Настройки "Выберите источник" 0 "Левая сторона"
ax press Настройки AXButton "Заменить из источника"
ax wait Настройки AXButton "Выполнить замену из источника"
ax press Настройки AXButton "Выполнить замену из источника"
ax wait Настройки AXStaticText "Выполнить решение конфликта?"
ax alert Настройки "Отмена"
[[ "$(cat "$RIGHT/l1.txt")" == "right edit" ]] || fail "declined conflict replacement changed the file"
[[ "$(jobs_sql "select count(*) from jobs")" == "$JOBS_BEFORE" ]] || fail "declined conflict replacement created a Job"
ax press Настройки AXButton "Выполнить замену из источника"
ax wait Настройки AXStaticText "Выполнить решение конфликта?"
ax alert Настройки "Заменить файл"
wait_for 20 cmp -s "$LEFT/l1.txt" "$RIGHT/l1.txt" || fail "confirmed conflict replacement did not copy the left bytes"

step "Two-way: a deletion needs an explicit decision and consent"
rm "$LEFT/r1.txt"
ax press Настройки AXButton "Проверить план" 0
ax wait Настройки AXStaticText "нужно отдельное решение"
ax within Настройки "r1.txt" AXButton "Добавить в очередь конфликтов"
ax wait Настройки AXStaticText "конфликт удаления"
ax pick Настройки "Выберите источник" 0 "Левая сторона"
ax press Настройки AXButton "Удалить как в источнике"
ax wait Настройки AXButton "Подтвердить удаление файла"
ax press Настройки AXButton "Подтвердить удаление файла"
ax wait Настройки AXStaticText "Удалить файл как в источнике?"
ax alert Настройки "Отмена"
[[ -f "$RIGHT/r1.txt" ]] || fail "declined deletion removed the file"
ax press Настройки AXButton "Подтвердить удаление файла"
ax wait Настройки AXStaticText "Удалить файл как в источнике?"
ax alert Настройки "Удалить файл"
wait_for 20 test ! -e "$RIGHT/r1.txt" || fail "confirmed deletion did not remove the selected file"
[[ -f "$RIGHT/l1.txt" ]] || fail "deletion changed another file"
[[ "$(jobs_sql "select count(*) from jobs where state not in ('completed')")" == 0 ]] \
  || fail "a Job is not completed: $(jobs_sql "select id, state from jobs where state<>'completed'")"

fi

if section cancel; then
step "Batch cancellation stops after the current file; reopened Settings shows the result"
BIG="$TEST_ROOT/big"; BIGDST="$TEST_ROOT/bigdst"
mkdir -p "$BIG" "$BIGDST"
for i in $(seq -w 1 80); do dd if=/dev/urandom of="$BIG/f$i.bin" bs=1m count=32 2>/dev/null; done
to_scenario_list
create_scenario "E2E cancel" "$BIG" "$BIGDST"
open_plan "E2E cancel"
ax wait Настройки AXButton "Создать все: 80"
ax press Настройки AXButton "Создать все: 80"
ax press Настройки AXButton "Остановить после текущего файла"
ax gone Настройки "*" "Выполнение пакета" 120
ax wait Настройки AXStaticText "Пакет остановлен по запросу" 5
COPIED="$(ls "$BIGDST" | wc -l | tr -d ' ')"
CANCELLED="$(registry_sql "select count(*) from scenario_create_batch_children where state='cancelled'")"
echo "   copied $COPIED of 80, cancelled $CANCELLED"
(( COPIED < 80 && CANCELLED > 0 )) || fail "cancellation did not stop the batch (copied $COPIED, cancelled $CANCELLED)"
(( COPIED + CANCELLED == 80 )) || fail "batch children are not all completed or cancelled"
[[ "$(jobs_sql "select count(*) from jobs where state<>'completed'")" == 0 ]] || fail "cancellation left an unfinished Job"
for f in "$BIGDST"/*; do cmp -s "$f" "$BIG/$(basename "$f")" || fail "copied file differs: $f"; done
REMAINING=$((80 - COPIED))
ax wait Настройки AXButton "Создать все: $REMAINING" 10
ax close Настройки
ax menu Window Настройки
ax wait Настройки AXStaticText "Сценарии" 10
if ! "$DRIVER" "$APP_PID" wait Настройки AXStaticText "План сценария" 2 2>/dev/null; then
  ax select Настройки "Сценарии"
  open_plan "E2E cancel"
fi
ax press Настройки AXButton "Проверить план" 0
ax wait Настройки AXButton "Создать все: $REMAINING" 10
ax gone Настройки "*" "Выполнение пакета" 3
ax gone Настройки AXButton "Остановить после текущего файла" 3

fi

if section negative; then
regtool_build
NSRC="$TEST_ROOT/nsrc"; NDST="$TEST_ROOT/ndst"
mkdir -p "$NSRC" "$NDST"

NFC_NAME="$(printf 'caf\xc3\xa9.txt')"
NFD_NAME="$(printf 'cafe\xcc\x81.txt')"
# Byte-exact name check: ls prints names exactly as stored.
# grep reads all input (no -q), so pipefail never sees SIGPIPE from ls.
has_exact_name() { ls -1 "$1" | LC_ALL=C grep -Fx -- "$2" >/dev/null; }

step "Negative: case and Unicode name collisions are never copied over each other"
printf 'source upper\n' > "$NSRC/Report.txt"
printf 'destination lower\n' > "$NDST/report.txt"
printf 'source nfc\n' > "$NSRC/$NFC_NAME"
printf 'destination nfd\n' > "$NDST/$NFD_NAME"
printf 'plain\n' > "$NSRC/plain.txt"
to_scenario_list
create_scenario "E2E negative" "$NSRC" "$NDST"
open_plan "E2E negative"
ax wait Настройки AXStaticText "Имена отличаются только регистром"
ax wait Настройки AXButton "Создать все: 1"
ax press Настройки AXButton "Создать все: 1"
wait_for 20 test -f "$NDST/plain.txt" || fail "plain file was not created"
ax gone Настройки "*" "Выполнение пакета" 20
[[ "$(cat "$NDST/report.txt")" == "destination lower" ]] || fail "case collision changed the destination"
has_exact_name "$NDST" "report.txt" && ! has_exact_name "$NDST" "Report.txt" || fail "case collision renamed or added a file"
[[ "$(cat "$NDST/$NFD_NAME")" == "destination nfd" ]] || fail "Unicode collision changed the destination"
! has_exact_name "$NDST" "$NFC_NAME" || fail "Unicode collision added a second file"
ax gone Настройки AXButton "Создать копию" 3

step "Negative: a plan for a changed Scenario revision is refused"
printf 'late\n' > "$NSRC/late.txt"
NDST2="$TEST_ROOT/ndst2"; mkdir -p "$NDST2"
ax press Настройки AXButton "Проверить план" 0
ax wait Настройки AXStaticText "late.txt"
JOBS_BEFORE="$(jobs_count)"
regtool move-right-root "E2E negative" "$NDST2"
[[ "$(regtool revision "E2E negative")" == 2 ]] || fail "the second controller did not create revision 2"
ax within Настройки "late.txt" AXButton "Создать копию"
ax wait Настройки AXStaticText "Сценарий изменён после построения плана" 10
[[ ! -e "$NDST/late.txt" && ! -e "$NDST2/late.txt" ]] || fail "a stale revision wrote a file"
[[ "$(jobs_count)" == "$JOBS_BEFORE" ]] || fail "a stale revision created a Job"
ax press Настройки AXButton "Проверить план" 0
ax wait Настройки AXStaticText "Сценарий изменён (ревизия 2)" 10
ax gone Настройки AXButton "Создать копию" 3

step "Negative: overlapping Scenario folders are refused when saved"
mkdir -p "$NSRC/inner"
to_scenario_list
ax press Настройки AXButton "Создать сценарий" 0
ax wait Настройки AXTextField "Название сценария"
ax set Настройки AXTextField "Название сценария" "E2E overlap"
choose_folder 0 "$NSRC"
choose_folder 1 "$NSRC/inner"
ax press Настройки AXButton "Создать сценарий" 0
ax wait Настройки AXStaticText "Папки сценария пересекаются" 10
[[ "$(registry_sql "select count(*) from scenarios where name='E2E overlap'")" == 0 ]] || fail "an overlapping Scenario was saved"
ax press Настройки AXButton "Назад"

step "Negative: a disabled connection blocks execution, re-enabling restores it"
VSRC="$TEST_ROOT/vsrc"; VDST="$TEST_ROOT/vdst"; mkdir -p "$VSRC" "$VDST"
printf 'vanish\n' > "$VSRC/v.txt"
to_scenario_list
create_scenario "E2E vanished" "$VSRC" "$VDST"
open_plan "E2E vanished"
ax wait Настройки AXButton "Создать копию"
JOBS_BEFORE="$(jobs_count)"
regtool local-enabled false
ax press Настройки AXButton "Создать копию"
ax wait Настройки AXStaticText "Подключение выключено или недоступно" 10
[[ ! -e "$VDST/v.txt" ]] || fail "a disabled connection wrote a file"
[[ "$(jobs_count)" == "$JOBS_BEFORE" ]] || fail "a disabled connection created a Job"
regtool local-enabled true
ax press Настройки AXButton "Проверить план" 0
ax wait Настройки AXButton "Создать копию" 10
ax press Настройки AXButton "Создать копию"
wait_for 20 cmp -s "$VSRC/v.txt" "$VDST/v.txt" || fail "re-enabled connection did not copy"

step "Negative: a vanished source folder is reported, not planned as deletions"
mv "$VSRC" "$TEST_ROOT/vsrc-moved"
ax press Настройки AXButton "Проверить план" 0
ax wait Настройки AXStaticText "Не удалось прочитать метаданные" 10
ax gone Настройки AXButton "Добавить в очередь конфликтов" 2
[[ -f "$VDST/v.txt" ]] || fail "a vanished source changed the destination"
mv "$TEST_ROOT/vsrc-moved" "$VSRC"

step "Negative: concurrent changes after planning never overwrite newer data"
RL="$TEST_ROOT/race-left"; RR="$TEST_ROOT/race-right"; mkdir -p "$RL" "$RR"
printf 'shared v1\n' > "$RL/shared.txt"
to_scenario_list
create_scenario "E2E race" "$RL" "$RR" two_way
open_plan "E2E race"
ax wait Настройки AXButton "Создать все: 1"
ax press Настройки AXButton "Создать все: 1"
wait_for 20 test -f "$RR/shared.txt" || fail "initial two-way Create did not run"
ax gone Настройки "*" "Выполнение пакета" 20
# Create race: the plan proposes a Create, then the destination gets a file.
printf 'left new\n' > "$RL/new.txt"
ax press Настройки AXButton "Проверить план" 0
ax wait Настройки AXButton "Создать копию"
printf 'right arrived first\n' > "$RR/new.txt"
ax press Настройки AXButton "Создать копию"
ax wait Настройки AXStaticText "Файл назначения уже существует" 10
[[ "$(cat "$RR/new.txt")" == "right arrived first" ]] || fail "Create overwrote a file that appeared after planning"
# Update race: the plan proposes an Update, then the destination changes again.
printf 'shared v2 from the left side\n' > "$RL/shared.txt"
ax press Настройки AXButton "Проверить план" 0
ax wait Настройки AXButton "Заменить файл"
printf 'shared edited on the right after planning\n' > "$RR/shared.txt"
ax press Настройки AXButton "Заменить файл" 0
ax wait Настройки AXStaticText "Заменить существующий файл?"
ax alert Настройки "Заменить файл"
sleep 2
[[ "$(cat "$RR/shared.txt")" == "shared edited on the right after planning" ]] \
  || fail "a confirmed Update overwrote a destination that changed after planning"
ax wait Настройки AXStaticText "изменились после построения плана" 10
[[ "$(jobs_sql "select count(*) from jobs where state='unknown'")" == 0 ]] || fail "a refused action left an UNKNOWN Job"
fi

start_worker() {
  "$WORKER" --socket "$DATA/worker.sock" --registry "$DATA/connections.sqlite3" --jobs "$DATA/jobs.sqlite3" \
    >>"$TEST_ROOT/worker.log" 2>&1 &
  WORKER_PID=$!
  wait_for 10 test -S "$DATA/worker.sock" || fail "Worker socket did not appear"
}
make_big_source() {
  mkdir -p "$1"
  for i in $(seq -w 1 "$2"); do dd if=/dev/urandom of="$1/f$i.bin" bs=1m count=32 2>/dev/null; done
}
file_count() { ls -1 "$1" | wc -l | tr -d ' '; }
batch_children() { registry_sql "select state, count(*) from scenario_create_batch_children c join scenario_create_batches b on b.id=c.batch_id join scenarios s on s.id=b.scenario_id where s.name='$1' group by state"; }
pending_children() { registry_sql "select count(*) from scenario_create_batch_children c join scenario_create_batches b on b.id=c.batch_id join scenarios s on s.id=b.scenario_id where s.name='$1' and c.state='pending'"; }
no_pending_children() { [[ "$(pending_children "$1")" == 0 ]]; }

if section crash; then
step "Crash: the Worker is killed during a batch and recovers after restart"
CSRC="$TEST_ROOT/csrc"; CDST="$TEST_ROOT/cdst"; mkdir -p "$CDST"
make_big_source "$CSRC" 80
to_scenario_list
create_scenario "E2E crash" "$CSRC" "$CDST"
open_plan "E2E crash"
ax wait Настройки AXButton "Создать все: 80"
ax press Настройки AXButton "Создать все: 80"
wait_for 30 bash -c "(( \$(ls -1 '$CDST' | wc -l) >= 3 ))" || fail "batch did not start"
kill -9 "$WORKER_PID"; wait "$WORKER_PID" 2>/dev/null || true
COPIED_AT_CRASH="$(file_count "$CDST")"
echo "   killed the Worker after $COPIED_AT_CRASH files"
ax gone Настройки "*" "Выполнение пакета" 20
ax wait monitoring AXStaticText "Worker не запущен" 15
sleep 2
[[ "$(file_count "$CDST")" == "$COPIED_AT_CRASH" ]] || fail "files changed while the Worker was stopped"
start_worker
wait_for 120 no_pending_children "E2E crash" || fail "the restarted Worker did not resume the batch: $(batch_children "E2E crash")"
echo "   children after restart: $(batch_children "E2E crash" | tr '\n' ' ')"
for row in $(registry_sql "select c.relative_path||':'||c.job_id||':'||c.state from scenario_create_batch_children c join scenario_create_batches b on b.id=c.batch_id join scenarios s on s.id=b.scenario_id where s.name='E2E crash' and c.state<>'completed'"); do
  job="$(echo "$row" | cut -d: -f2)"
  echo "   unfinished child $row; job=$(jobs_sql "select state from jobs where id='$job'"); events=$(jobs_sql "select kind||'>'||state from job_events where job_id='$job' order by rowid" | tr '\n' ' '); attempts=$(jobs_sql "select status from job_attempts where job_id='$job'" | tr '\n' ' '); dest=$(ls -la "$CDST/$(echo "$row" | cut -d: -f1)" 2>&1 | awk '{print $5}')"
done
UNKNOWN_CHILDREN="$(registry_sql "select count(*) from scenario_create_batch_children c join scenario_create_batches b on b.id=c.batch_id join scenarios s on s.id=b.scenario_id where s.name='E2E crash' and c.state not in ('completed')")"
(( UNKNOWN_CHILDREN <= 1 )) || fail "more than the in-flight file is unfinished after restart"
[[ "$(jobs_sql "select count(*) from jobs where state='running'")" == 0 ]] || fail "a Job stays running after restart"
HIDDEN="$(ls -A "$CDST" | grep -v '^f[0-9]*\.bin$' || true)"
[[ -z "$HIDDEN" ]] || fail "the restarted Worker left staging files of the interrupted write: $HIDDEN"
for f in "$CDST"/*; do
  name="$(basename "$f")"
  [[ "$name" == f*.bin ]] || fail "an unexpected destination entry remains after the crash: $name"
  cmp -s "$f" "$CSRC/$name" || fail "a destination file differs from its source after the crash: $name"
done
echo "   unfinished Jobs: $(jobs_sql "select id, state from jobs where state<>'completed'" | tr '\n' ' ')"
MISSING=$((80 - $(file_count "$CDST")))
(( MISSING <= 1 )) || fail "more than one file is missing after restart"
ax press Настройки AXButton "Проверить план" 0
if (( MISSING == 0 )); then
  ax wait Настройки AXStaticText "без действия" 10
  ax gone Настройки AXButton "Создать копию" 3
else
  # The interrupted file was not published. The plan offers it again as a
  # new no-overwrite Create; nothing is replayed automatically.
  ax wait Настройки AXButton "Создать все: 1" 10
fi
fi

if section lifecycle; then
LSRC="$TEST_ROOT/lsrc"; LDST="$TEST_ROOT/ldst"; mkdir -p "$LDST"
make_big_source "$LSRC" 80
regtool_build
to_scenario_list
create_scenario "E2E lifecycle" "$LSRC" "$LDST"

step "Lifecycle: This Mac offers no removal"
# D-020: the single This Mac connection always exists. Rust tests cover the
# Registry refusal to remove a connection that a Scenario uses.
ax select Настройки "Подключения"
ax wait Настройки AXTextField "Этот Mac"
ax gone Настройки AXButton "Удалить подключение" 3
[[ "$(registry_sql "select count(*) from storage_connections where adapter='local_filesystem' and enabled=1")" == 1 ]] \
  || fail "This Mac is missing or disabled"

step "Lifecycle: disabling the connection during a batch stops new files"
to_scenario_list
open_plan "E2E lifecycle"
ax wait Настройки AXButton "Создать все: 80"
ax press Настройки AXButton "Создать все: 80"
wait_for 30 bash -c "(( \$(ls -1 '$LDST' | wc -l) >= 2 ))" || fail "batch did not start"
UNKNOWN_BEFORE="$(jobs_sql "select count(*) from jobs where state='unknown'")"
regtool local-enabled false
wait_for 120 no_pending_children "E2E lifecycle" || fail "the batch did not finish after disable"
AT_DISABLE="$(file_count "$LDST")"
echo "   children: $(batch_children "E2E lifecycle" | tr '\n' ' ')"
(( AT_DISABLE < 80 )) || fail "files were still copied after the connection was disabled"
[[ "$(jobs_sql "select count(*) from jobs where state='running'")" == 0 \
   && "$(jobs_sql "select count(*) from jobs where state='unknown'")" == "$UNKNOWN_BEFORE" ]] \
  || fail "disable left a running or new UNKNOWN Job: $(jobs_sql "select id, state from jobs where state in ('running','unknown')" | tr '\n' ' ')"
for f in "$LDST"/*; do cmp -s "$f" "$LSRC/$(basename "$f")" || fail "partial file after disable: $f"; done
ax gone Настройки "*" "Выполнение пакета" 20
ax wait Настройки AXStaticText "Не завершены файлы" 10
ax wait Настройки AXStaticText "План не построен" 10
ax gone Настройки AXStaticText "Изменения не найдены" 2
regtool local-enabled true
ax press Настройки AXButton "Проверить план" 0
ax wait Настройки AXButton "Создать все: $((80 - AT_DISABLE))" 10
fi

if section links; then
step "Links: symbolic links and special files are skipped and disclosed"
KSRC="$TEST_ROOT/ksrc"; KDST="$TEST_ROOT/kdst"; mkdir -p "$KSRC" "$KDST"
printf 'keep\n' > "$KSRC/keep.txt"
ln -s keep.txt "$KSRC/link.txt"
ln -s /private/tmp "$KSRC/outside-dir"
mkfifo "$KSRC/pipe"
to_scenario_list
create_scenario "E2E links" "$KSRC" "$KDST"
open_plan "E2E links"
ax wait Настройки AXStaticText "Пропущено объектов, которые QADREN не синхронизирует" 10
ax wait Настройки AXStaticText "в источнике 3, в назначении 0" 5
ax wait Настройки AXButton "Создать все: 1"
ax press Настройки AXButton "Создать все: 1"
wait_for 20 test -f "$KDST/keep.txt" || fail "the regular file was not copied"
ax gone Настройки "*" "Выполнение пакета" 20
[[ "$(ls -A "$KDST")" == "keep.txt" ]] || fail "a link or special file reached the destination: $(ls -A "$KDST" | tr '\n' ' ')"

step "Links: a confirmed file replaced by a link is not planned as a deletion"
rm "$KSRC/keep.txt"
ln -s /etc/hosts "$KSRC/keep.txt"
ax press Настройки AXButton "Проверить план" 0
ax wait Настройки AXStaticText "символическая ссылка или специальный файл" 10
ax gone Настройки AXButton "Добавить в очередь конфликтов" 2
ax gone Настройки AXButton "Создать копию" 2
[[ "$(cat "$KDST/keep.txt")" == "keep" ]] || fail "the destination file changed"
fi

# File Provider compatibility. Runs only when E2E_SECTIONS names it and
# FP_SOURCE points to an owner-approved provider folder with evicted
# (dataless) files; FP_ORIG holds local copies of the same bytes. The
# caller owns and removes FP_SOURCE; this section writes only to TEST_ROOT.
if [[ " ${E2E_SECTIONS:-} " == *" fileprovider "* ]]; then
step "File Provider: a metadata plan does not download, a Create reads the placeholder"
[[ -d "${FP_SOURCE:-}" && -d "${FP_ORIG:-}" ]] || fail "set FP_SOURCE and FP_ORIG for the fileprovider section"
dataless_count() { find "$FP_SOURCE" -type f -flags +dataless | wc -l | tr -d ' '; }
TOTAL="$(find "$FP_SOURCE" -type f | wc -l | tr -d ' ')"
[[ "$(dataless_count)" == "$TOTAL" ]] || fail "FP_SOURCE files must be evicted placeholders before the test"
FPDST="$TEST_ROOT/fpdst"; mkdir -p "$FPDST"
to_scenario_list
create_scenario "E2E file provider" "$FP_SOURCE" "$FPDST"
open_plan "E2E file provider"
ax wait Настройки AXButton "Создать все: $TOTAL" 20
[[ "$(dataless_count)" == "$TOTAL" ]] || fail "the metadata plan downloaded provider content"
ax press Настройки AXButton "Создать все: $TOTAL"
ax gone Настройки "*" "Выполнение пакета" 180
(cd "$FP_ORIG" && find . -type f) | while read -r f; do
  cmp -s "$FP_ORIG/$f" "$FPDST/$f" || fail "provider file was not copied exactly: $f"
done
FP_SCENARIO="$(registry_sql "select id from scenarios where name='E2E file provider'")"
[[ "$(jobs_sql "select count(*) from jobs where scenario_id='$FP_SCENARIO' and state='completed'")" == "$TOTAL" ]] \
  || { grep "qadren-worker:" "$TEST_ROOT/worker.log" >&2 || true
       fail "provider Jobs: $(jobs_sql "select id, state from jobs where scenario_id='$FP_SCENARIO'" | tr '\n' ' ')"; }
ax press Настройки AXButton "Проверить план" 0
ax wait Настройки AXStaticText "без действия" 20
ax gone Настройки AXButton "Создать копию" 3
# Evict the sources again. A download or eviction is not a content change,
# so the confirmed baseline must stay valid.
if command -v brctl >/dev/null; then
  (cd "$FP_SOURCE" && find . -type f | while read -r f; do brctl evict "$f" >/dev/null 2>&1 || true; done)
  wait_for 30 bash -c "[[ \$(find '$FP_SOURCE' -type f -flags +dataless | wc -l | tr -d ' ') == $TOTAL ]]" \
    || fail "provider files could not be evicted again"
  ax press Настройки AXButton "Проверить план" 0
  ax wait Настройки AXStaticText "без действия" 20
  ax gone Настройки AXStaticText "изменён" 3
  ax gone Настройки AXButton "Заменить файл" 2
  [[ "$(dataless_count)" == "$TOTAL" ]] || fail "the plan after eviction downloaded provider content"
fi
fi


# Scheduled execution. Opt-in: every slot needs real wall-clock time, so the
# section takes about 8 minutes. Run it with E2E_SECTIONS=schedule.
WEEKDAYS=(Понедельник Вторник Среда Четверг Пятница Суббота Воскресенье)
open_schedules() {
  to_scenario_list
  ax within Настройки "$1" AXButton "Расписания"
  ax wait Настройки AXButton "Добавить расписание"
}
# Add a weekly slot for today about 75 s from now, then return its minute.
add_next_slot() {
  ax date Настройки 0 75 >/dev/null
  local today="${WEEKDAYS[$(( $(date +%u) - 1 ))]}"
  local current
  current="$("$DRIVER" "$APP_PID" dump Настройки | grep -oE 'AXPopUpButton (Понедельник|Вторник|Среда|Четверг|Пятница|Суббота|Воскресенье)' | head -1 | awk '{print $2}')"
  [[ "$current" == "$today" ]] || ax pick Настройки "$current" 0 "$today"
  sleep 2
  ax press Настройки AXButton "Добавить расписание"
  sleep 1
}

if [[ " ${E2E_SECTIONS:-} " == *" schedule "* ]]; then
step "Schedule: a slot runs a safe Create without the Settings window"
regtool_build
SSRC="$TEST_ROOT/ssrc"; SDST="$TEST_ROOT/sdst"; mkdir -p "$SSRC" "$SDST"
printf 'update me\n' > "$SSRC/u.txt"
to_scenario_list
create_scenario "E2E schedule" "$SSRC" "$SDST"
open_plan "E2E schedule"
ax wait Настройки AXButton "Создать все: 1"
ax press Настройки AXButton "Создать все: 1"
wait_for 20 test -f "$SDST/u.txt" || fail "initial Create did not run"
printf 'source changed for the schedule\n' > "$SSRC/u.txt"
printf 'scheduled new\n' > "$SSRC/new.txt"
JOBS_BEFORE="$(jobs_count)"
open_schedules "E2E schedule"
add_next_slot
ax close Настройки
wait_for 240 test -f "$SDST/new.txt" || fail "the scheduled slot did not create the new file"
cmp -s "$SSRC/new.txt" "$SDST/new.txt" || fail "scheduled Create bytes differ"
sleep 3
[[ "$(cat "$SDST/u.txt")" == "update me" ]] || fail "a scheduled run replaced a file without a decision"
WAITING="$(jobs_sql "select count(*) from jobs where state='waiting_for_decision'")"
(( WAITING >= 1 )) || fail "the scheduled replacement is not waiting for a decision: $(jobs_sql "select id, state from jobs" | tail -5 | tr '\n' ' ')"
ax wait monitoring AXStaticText "Требует решения" 15

step "Schedule: the waiting decision is found and resolved in Settings"
ax menu Window Настройки
ax wait Настройки AXStaticText "Сценарии" 10
to_scenario_list
open_plan "E2E schedule"
ax wait Настройки AXStaticText "Альтернативная очередь конфликтов" 15
ax within Настройки "u.txt" AXButton "Заменить из источника"
ax wait Настройки AXButton "Выполнить замену из источника" 10
ax press Настройки AXButton "Выполнить замену из источника"
ax wait Настройки AXStaticText "Выполнить решение конфликта?"
ax alert Настройки "Заменить файл"
wait_for 20 cmp -s "$SSRC/u.txt" "$SDST/u.txt" || fail "the confirmed scheduled decision did not replace the file"
wait_for 20 bash -c "[[ \$(sqlite3 '$DATA/jobs.sqlite3' \"select count(*) from jobs where state='waiting_for_decision'\") == 0 ]]" \
  || fail "the schedule coordinator still waits after the decision: $(jobs_sql "select id, state from jobs where state='waiting_for_decision'" | tr '\n' ' ')"

step "Schedule: a paused Scenario does not run its slot, also after resume"
printf 'paused new\n' > "$SSRC/paused.txt"
open_schedules "E2E schedule"
ax toggle Настройки "Приостановить автоматические триггеры"
sleep 1
[[ "$(registry_sql "select locally_paused from scenario_automation where scenario_id=(select id from scenarios where name='E2E schedule')")" == 1 ]] \
  || fail "the pause switch did not save the paused state"
add_next_slot
ax close Настройки
sleep 150
[[ ! -e "$SDST/paused.txt" ]] || fail "a paused Scenario ran its slot"
ax menu Window Настройки
ax wait Настройки AXStaticText "Сценарии" 10
open_schedules "E2E schedule"
ax toggle Настройки "Приостановить автоматические триггеры"
sleep 70
[[ ! -e "$SDST/paused.txt" ]] || fail "a slot missed while paused ran after resume"

step "Schedule: a two-way slot missed while the Worker is stopped runs once after restart"
TL="$TEST_ROOT/sched-left"; TR="$TEST_ROOT/sched-right"; mkdir -p "$TL" "$TR"
printf 'from the left\n' > "$TL/left-new.txt"
printf 'from the right\n' > "$TR/right-new.txt"
to_scenario_list
create_scenario "E2E schedule two-way" "$TL" "$TR" two_way
open_schedules "E2E schedule two-way"
add_next_slot
ax close Настройки
kill "$WORKER_PID"; wait "$WORKER_PID" 2>/dev/null || true
sleep 100
[[ ! -e "$TR/left-new.txt" && ! -e "$TL/right-new.txt" ]] || fail "files changed while the Worker was stopped"
start_worker
wait_for 150 bash -c "[[ -f '$TR/left-new.txt' && -f '$TL/right-new.txt' ]]" \
  || fail "the missed two-way slot did not run after the Worker restart"
cmp -s "$TL/left-new.txt" "$TR/left-new.txt" && cmp -s "$TR/right-new.txt" "$TL/right-new.txt" \
  || fail "two-way scheduled bytes differ"
TW_ID="$(registry_sql "select id from scenarios where name='E2E schedule two-way'")"
sleep 65
COORDINATORS="$(jobs_sql "select count(*) from jobs where scenario_id='$TW_ID' and id like 'schedule-%' and id not like 'schedule-copy-%'")"
[[ "$COORDINATORS" == 1 ]] || fail "the missed slot ran $COORDINATORS times: $(jobs_sql "select id, state from jobs where scenario_id='$TW_ID'" | tr '\n' ' ')"
[[ "$(jobs_sql "select count(*) from jobs where scenario_id='$TW_ID' and state<>'completed'")" == 0 ]] \
  || fail "a two-way scheduled Job did not complete: $(jobs_sql "select id, state from jobs where scenario_id='$TW_ID'" | tr '\n' ' ')"
ax menu Window Настройки
ax wait Настройки AXStaticText "Сценарии" 10
open_schedules "E2E schedule"

step "Schedule: a disabled connection fails the slot before any effect"
add_next_slot
regtool local-enabled false
UNKNOWN_BEFORE="$(jobs_sql "select count(*) from jobs where state='unknown'")"
FAILED_BEFORE="$(jobs_sql "select count(*) from jobs where state='failed'")"
wait_for 240 bash -c "(( \$(sqlite3 '$DATA/jobs.sqlite3' \"select count(*) from jobs where state='failed'\") > $FAILED_BEFORE ))" \
  || fail "the slot with a disabled connection did not settle as failed"
[[ ! -e "$SDST/paused.txt" ]] || fail "a disabled connection wrote a file"
[[ "$(jobs_sql "select count(*) from jobs where state='unknown'")" == "$UNKNOWN_BEFORE" ]] || fail "a disabled connection left an UNKNOWN Job"
[[ "$(jobs_sql "select count(*) from jobs where state='running'")" == 0 ]] || fail "a Job stays running"
regtool local-enabled true
fi

echo "Settings native E2E passed"
