# QADREN — аудит текущего `main` и расхождений с задачами

**Проверено 2026-10-14, после push пользователя.** Срез продуктового кода — commit [`1223e8f`](https://github.com/Xsoid/QADREN/commit/1223e8f29f94), в `main` также внесён D-018 и обновления документов. **Аудит по чтению кода и GitHub API**, а не новая нативная тестовая сборка. Ранний отчёт о SHA `44538fe` от 12 октября **устарел**: ранее локальные работы 13–14 октября теперь находятся в `main`.

## Границы проверки и CI

Осмотрены GitHub дерево исходников, документация, все issues, последовательность коммитов, файлы реализации, GitHub Actions runs. Последние проверенные CI jobs на `b1a3a22` — `rust: failure` и `macos: failure`, run [38599805324](https://github.com/Xsoid/QADREN/actions/runs/38599805324). API не вернул step details; скачивание логов ранее вернуло NOT_FOUND. **Причина отказа не установлена.** Локальные PASS из комментариев issues до push — свидетельства автора о проверках этих коммитов, но не заменяют независимую свежую проверку HEAD; не объявлять ни компиляцию, ни нативную визуальную приёмку доказанными.

## Матрица исходников и статусов

| Область | Наличие в `main` | Что реально осталось |
|---|---|---|
| Rust Core/SQLite/FFI | `model.rs`, `registry.rs`, `jobs.rs`, `jobs_ffi.rs`, `confirmation.rs`, `policy.rs` | Модули существуют; нет доказанной production Local↔Local execution через Job |
| Mock G1 Job | `job_execution.rs` — app-owned no-overwrite create + durable attempt/verification/recovery tests | Не повторять; перейти к безопасной **реальной Local FS** вертикали #18/#15 |
| Local FS | `local_fs.rs`, `local_fs_unix.rs`, `local_fs_ffi.rs`, `LocalWorkspace.swift` | API метаданных/операций/guards есть; UI→Job→Local FS→verified SQLite E2E не подтверждено |
| Scenario | `scenario_planner.rs`, Registry revisions/FileSyncState, dry-run FFI | Реальные metadata snapshots→Job submission→sync/recovery и конфликты не доказаны |
| Scheduler | `scheduler.rs`, `schedule_dispatch.rs` и dispatch FFI (`1223e8f`) | Outbox→queued Job есть; самостоятельного таймера, фонового жизненного цикла, event debounce и исполнения Scenario нет |
| UI theme/widget | `DesignSystem.swift`, `DashboardLayout.swift`, `DashboardLayoutTests.swift` | **Уже реализовано частично:** QDTheme/QDWidgetFrame/QDLogo, per-window order/visibility, density, grid scale, Undo/Redo/persistence. Нужна визуальная и нативная приёмка, не реализация с нуля |
| UI windows/settings | `QADRENApp.swift`, `MainView.swift`, `SettingsPanel.swift` | Dashboard adaptive grid, отдельные Monitoring/Settings/File Manager, MenuBarExtra. Расширение интеграции и доступность ещё частичны; `MainView` содержит legacy/mock actions |
| Yandex | `yandex.rs`, `yandex_ffi.rs`, `YandexConnection.swift` | Реальный код OAuth/metadata/upload и mock-тесты сохраняются, но контроллер **вынесен за Release 1**; live E2E не подтверждён; #26 убирает активные Add/Connect/API flows без удаления tokens/connections |
| iCloud | `ICloudAvailability.swift` (legacy metadata/status) | #17 `closed/not_planned`; не развивать, UI integration отсутствует/не нужна; Local FS guards остаются |
| Background service | Нативные окна и `workerStatus` | Независимая служба после закрытия UI **не доказана**, #23 P2 |
| Распространение | `scripts/build-macos.sh`, `.github/workflows/bootstrap.yml` | Unsigned dev bundle в скрипте; native install/sign/restart/packaging и CI диагностика ещё нужны |

## Issues — управляемая очередь (D-018)

- **P0 сейчас — только #26:** довести существующий `DesignSystem`/widget dashboard с нативной UI и build-приёмкой; **#13 архивируется как superseded/not planned**, а не VERIFIED: её baseline-аудит выполнен, остатки переданы в #26 (UI/macOS tests), #24 (GitHub Actions pre-runner failure, runtime resources/release), #25 (Keychain/legacy migration/recovery).
- **P1:** #15 Local FS; #18 Job engine; #20 policy; #25 remove/disconnect coordination. Acceptance — одна реальная локальная операция через единственный JobStore и проверяемое durable outcome. #22 двухпанельный UI и #19 локальные Scenario могут разрабатываться в независимых безопасных частях.
- **P2:** #21 scheduler runtime и catch-up; #23 независимая служба + завершающая интеграция UI; #24 безопасность/сборка/локальный дистрибутив.
- **P3 post-release, parked:** #16 контроллер Яндекс Диска. Сохранить исходники; он не является Release 1 gate.
- **Бессрочно parked:** #17 iCloud, закрыта. #2/#7/#14 уже закрыты, без повторной реализации.

Открытые issues **PARTIAL**, если не пройдён полный DoD; не закрывать по одному свежему коммиту или числу unit tests. #26 создана [отдельно](https://github.com/Xsoid/QADREN/issues/26), чтобы UI P0 не зависел от фоновой службы #23 P2.

## Приоритетные расхождения

1. Проектный `main` теперь содержит UI widget canvas/layout; старые UI-A/B/C «создать с нуля» неактуальны. Переиспользовать существующее.
2. Дизайн-система и логотип существуют, но не доказана native dark/light/VoiceOver, screenshot и макетная приёмка.
3. Яндекс-подключение всё ещё активно в UI (`SettingsPanel.swift` / `YandexConnectionCard`), хотя решение владельца — отложить до последнего этапа. Не запускать новые API-разработки. Скрытие/disable новых flows делать без уничтожения существующих credentials.
4. Старые Release 1 тексты связывают локальный выпуск с Yandex OAuth/content/E2E. D-018 **разрывает эту зависимость**, требует local-first.
5. CI на GitHub сейчас red (для docs и кодовых commits). Логи недоступны: оформить воспроизводимую диагностику, не гадать и не считать тесты пройденными.
6. Отсутствие независимого Worker и живой Local↔Local синхронизации нельзя компенсировать виджетом или mock demo.

**Следующий исполнимый результат:** #26 завершить визуально проверяемый P0 без переписывания текущих компонентов; инфраструктурная диагностика CI зафиксирована в #24, Keychain в #25; затем связка #18/#15/#20 с реальной Local FS. Действовать прямо в `main` с обязательной проверкой фактического SHA. Исторические G0–G6 и старые cloud requirements — не активная очередь.
