# QADREN — аудит состояния и реестр несоответствий

**Срез GitHub:** 2026-10-14; проверен `main` at `44538fe480ab17e3ee0c6114aaa3762a1c35d8c9` (последний commit в GitHub — 2026-10-12). **Статус:** документированный аудит репозитория, не подтверждение локального рабочего дерева и не release certification.

## Источники и границы доказательств

Проверены: дерево `main`, Rust/Swift исходники, build/test scripts, README/AGENTS/roadmap/design, все 16 GitHub issues, история PR, последние обсуждения issues, результаты GitHub Actions. **Локальный Mac и непушенные коммиты недоступны через GitHub**. В комментариях от 13–14 октября есть SHA с явной пометкой `local, no push`, поэтому не переносить их возможности и успешные тесты в колонку `main VERIFIED` до push, сопоставления commit и повторной проверки.

GitHub Actions `Bootstrap checks` для HEAD `44538fe`: `failure` как у `rust`, так и у `macos` (run 38230893539). Логи job скачать не удалось (GitHub вернул NOT_FOUND для blob); **точная причина неизвестна**. Не трактовать CI failure как доказательство некомпилируемости без лога; также нельзя считать pipeline зелёным. Нужны local `bash scripts/test-core.sh`, `bash scripts/build-macos.sh` на Mac и/или новая диагностика CI с сохранённым выводом.

## Состояние компонентов, подтверждённое чтением main

| Компонент | Наличие в коде | Статус готовности и ограничение |
|---|---|---|
| Rust Core/SQLite Registry и C ABI | `model.rs`, `registry.rs`, `registry_ffi.rs`, `jobs.rs`, `jobs_ffi.rs` | Основа существует; не путать с production Job execution |
| G1 mock execution | `job_execution.rs`: no-overwrite app-owned mock, durable Job, receipt/verification, fault tests | **PARTIAL**: тестовый storage, не Local↔Local/Yandex E2E |
| Local Filesystem | `local_fs.rs`, `local_fs_unix.rs`, `local_fs_ffi.rs` | Существенный API (list/stat/copy/read/guards) существует; интегрированное UI→Job→Local FS доказательство не получено |
| Scenario | `scenario_planner.rs`, `automation.rs`, `registry.rs` | Консервативное dry-run моделирование; нет доказанной сквозной синхронизации |
| Яндекс Диск | `yandex.rs`, `yandex_ffi.rs`, `YandexConnection.swift` | Metadata, OAuth и upload-from-reader код есть; live E2E не подтверждён; **PARKED LAST** |
| macOS UI | `QADRENApp.swift`, `MainView.swift` | WindowGroup + Settings + MenuBarExtra, dashboard cards/jobs/read-only files. На проверенном HEAD нет общего `DesignSystem/` или persisted widget engine из поздних локальных комментариев |
| iCloud | `ICloudAvailability.swift` и упоминания в `MainView.swift` | Legacy informational/status UI; **PARKED**, не коннектор. Убрать iCloud из активного UI в составе P0, сохранив safety guards |
| Независимая служба | SwiftUI scene и `CoreBridge.workerStatus` | Независимый background Worker/сервис не доказан; закрытие UI пока нельзя считать независимым от engine |
| Build/CI | `scripts/test-core.sh`, `scripts/build-macos.sh`, `.github/workflows/bootstrap.yml` | Скрипты есть; последние удалённые CI Rust/macOS — failure с недоступными логами |

**Важное расхождение:** комментарии issues #18–#23, #16, #25 сообщают о более новых локальных проверках и функциональных срезах (включая schedules, revisions, UI layout). Это полезный follow-up inventory, **не** подтверждение присутствия в `origin/main`. До начала Codex разработчик выполняет `git status -sb; git log --oneline -20; git rev-parse HEAD; git log origin/main..HEAD --oneline`, затем либо безопасно синхронизирует уже проверенное, либо указывает реальные причины расхождения. Никакого reset/force push/перезаписи локальной работы.

## Issues — фактическое состояние GitHub на срезе

| Issue | Тема | Статус GitHub / наблюдение | Приоритет D-018 |
|---|---|---|---|
| #2, #7 | Registry и мультиподключения | closed completed; исходники есть | baseline, не переписывать |
| #12 | Epic R1 | open, содержал cloud-first plan | координация P0–P3 |
| #13 | Audit/stabilization | open; отдельные проверки не закрыты | P0 |
| #14 | Core contracts | closed completed, но runtime contracts уточняются | baseline |
| #15 | Local FS | open, large historical checklist | P1 |
| #16 | Yandex content | open; live E2E не доказан | **P3 PARKED LAST** |
| #17 | iCloud | closed not_planned | **PARKED indefinitely** |
| #18 | Job engine | open; G1 mock существует | P1 |
| #19 | Scenario sync | open; pure planning существует | P1–P2 |
| #20 | Policy/recovery | open; consent guards частично есть | P1 |
| #21 | Scheduler | open; core structures есть | P2 |
| #22 | File manager | open; read-only local panels в main | P1–P2 |
| #23 | SwiftUI/service | open; UI baseline в main, service нет | P0 UI и P2 service: разделить фокус |
| #24 | Release/distribution | open, cloud E2E было hard dependency | P2 local release; P3 provider optional |
| #25 | Connection removal safety | open; fault tests есть | P1, provider live validation в P3 |
| #26 | UI foundation (при создании) | новая управляемая единица P0 | P0 |

Нет доказательства выполнения полного acceptance у открытых issues; **не закрывать их** формально из-за наличия исходников и локально заявленных тестов.

## Главные проблемы порядка работы

1. Старые D-017/AGENTS/README/Epic/roadmap всё ещё требуют Яндекс контент в текущем Release 1 и подталкивают Codex к #16. **Переопределены D-018**.
2. G1 описан как предстоящий, но первый mock executor уже есть — дальнейшая работа должна делать **реальную локальную вертикаль**, а не повторный mock или второй JobStore.
3. UI требования хорошо описаны в `docs/design/`, но `MainView.swift` в GitHub до сих пор монолитен и с navigation-switch вместо widget canvas; P0 нужен отдельный конечный acceptance.
4. #23 объединяет собственно интерфейс и позднюю независимую службу; UI не должен ждать P2 сервиса.
5. Дублирующиеся огромные исторические checklists issues создают противоречивую линейную очередь; D-018 и приоритетные заголовки считаются последним управляющим статусом, старый body — история и детализация.
6. CI на удалённом HEAD красный, причина неизвестна; нельзя делать новые заявления о verification без pass с конкретным SHA.

## Условия продолжения

**Сейчас:** (1) выяснить локальные непушенные работы и падение CI, (2) #26 P0 дизайн-система + native dashboard, (3) #13 завершить baseline/обязательные тесты. Затем #15/#18/#20/#25 локальные безопасные Job и #22, после чего #19/#21/#23 background, #24 local packaging. **Последним**, только по новому разрешению владельца, #16/Yandex. iCloud не планировать.

Все задачи вести в `main`, без обязательного PR. Действующие политики сохранности данных имеют абсолютный приоритет над очередностью.