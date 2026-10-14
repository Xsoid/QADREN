# QADREN — план интеграции UI для Codex и acceptance

> **Обновление после push 2026-10-14 / D-018:** существующие `DesignSystem.swift`, `DashboardLayout.swift`, `DashboardLayoutTests.swift`, `MainView.swift`, `SettingsPanel.swift`, `QADRENApp.swift` уже реализуют базовый виджетный canvas, сохранение порядка/видимости/плотности/gridScale, Undo/Redo, мультиоконность и настройки. Ниже исходная последовательность UI-A–F — **чек-лист проверки и завершения**, не указание повторно создать эти классы. Приоритет #26 P0, #23 служба P2. Яндекс P3/PARKED, iCloud вне scope. [Срез](../development/current-state-audit.md).

## 0. Границы и фактический baseline

Эта документация — **визуально-поведенческий контракт**, а не приказ сейчас создать несуществующие Core API. Исторический baseline до push 14 октября (не применять без проверки): `QADRENApp.swift` имеет WindowGroup monitoring, окно `scenario-settings`, MenuBarExtra. `MainView.swift` — dashboard status tiles, jobs list, две local metadata panels, connections/OAuth setup; `StorageConnections.swift`, `YandexConnection.swift`, `LocalWorkspace.swift`, `JobInventory.swift` существуют. Первый полноценный Rust Job execution/independent worker service ещё проходит дорожные gates. **Перед реализацией проверить HEAD**, этот список может устареть.

UI не должен изобретать data pipeline, вторую очередь, runtime history или registry. Активный адаптер локального релиза P0–P2 — Local FS; Yandex UI/feature-work P3/PARKED, existing records/tokens preserve. iCloud connection/sync/backup UI исключены из активной реализации (D-017). Все изменения в `main`; не создавай feature/docs branches, не предлагай автоматический переход Work. Завершение документации не закрывает Issue #23.

### Актуализированная P0-проверка #26

- При ошибке чтения JobStore/Registry мониторинг показывает приоритетное доступное предупреждение и ошибочное состояние соответствующего виджета; ошибку нельзя выдать за спокойный empty/success state.
- Если сохранённая раскладка повреждена, `DashboardLayoutStore` показывает безопасную стандартную панель, не стирает исходные байты и сообщает об этом в monitoring; уведомление исчезает только после осознанного сохранения новой раскладки.
- Пользователь может скрыть все виджеты: тогда canvas показывает доступное empty state с точной инструкцией вернуть виджеты через layout mode.
- В P0–P2 Settings предлагает создать только Local FS connection. Уже существующий Yandex connection можно переименовать, выключить или локально удалить с сохранением правила «облачные объекты не трогаются», но UI не предлагает OAuth или API calls до P3.

## 1. Матрица соответствия текущего кода

| Текущий файл/часть | Целевая роль | Что делать, что НЕ делать |
|---|---|---|
| `platform/macos/QADREN/QADRENApp.swift` | сцены monitoring/settings, menu bar | сохранить entrypoints, добавить panel identity/session; не связывать lifecycle Worker с `Window` |
| `platform/macos/QADREN/MainView.swift` | composition root | вынести tokens/components/widgets постепенно, не оставлять switch «dashboard/jobs/files» как главный UX |
| `StorageConnections.swift` + `YandexConnection.swift` | реальные connection metadata + OAuth | адаптировать к Settings и Connection widget, не переписывать Keychain/Registry |
| `JobInventory.swift` | read-only durable Job data + поддержанные команды | не подменять mock-job успешным live transfer |
| `LocalWorkspace.swift` | метаданные local FS и browse | файлы открывать через expanded workspace/отдельное окно, не использовать sneak-content-preview |
| Rust `jobs.rs`, `jobs_ffi.rs`, `model.rs`, `scenario_planner.rs`, `confirmation.rs` | source of truth | ViewModels подписаны на команды и durable state; Swift не вычисляет policy и job outcomes |

Условные имена SwiftUI types (`QDTheme`, `QDWidgetFrame`, `QDLayoutStore`) предлагаются как направление декомпозиции; если существующие типы подходят — **reuse**.

## 2. Вертикальная последовательность (не новые большие Epic)

**UI-A — дизайн-токены и logo (PARTIAL IN MAIN; audit/finish).** Проверить существующие QDTheme/QDLogo и при необходимости подключить существующий `assets/qadren-logo.png` в Assets.xcassets/target без изменения пикселей; построить один SwiftUI semantic theme, системные light/dark variants и CSS/demo независимы. SwiftUI snapshot-превью кнопки, статуса, progress, mini-widget; провести контрастную проверку.

**UI-B — chrome и dashboard (PARTIAL IN MAIN; audit/finish).** Проверить существующий reusable `QDWidgetFrame`, standard headers, states, hardcoded `PreviewFixtures` только в `#Preview`/тестах, не runtime. Заменить постоянную навигацию по разделам на single dashboard grid. Настоящие data providers пока отображают read-only/unsupported.

**UI-C — сохранение и multiwindow (PARTIAL IN MAIN; audit/finish).** Существующий DashboardLayoutStore, Dashboard `panelID`, type registry, grid sizing+reflow, layout editing with catalog, config/Undo/Redo, независимые window state/zoom/density/filters, persist+restore. Ни одного одновременно работающего второго Job runner.

**UI-D — реальные данные и safety.** Подключить активные Jobs, connections, events, Worker state через существующие Swift/FFI models с реальными evidence/freshness. При отсутствии API — honest `not available`. Commands только после capability check/Core policy. Consent/UNKNOWN/conflict из Rust contracts, никаких blind retries.

**UI-E — настройки и инспекторы.** Панель настроек с sections (connection/scenario/schedules/policies/node/UI), глубокие ссылки из виджетов, нативные формы, recoverable connection removal. Файловый менеджер — как expanded/отдельная панель из мониторинга, без постоянной навигации. Не реализовывать неготовые операции «для красоты».

**UI-F — жизненный цикл.** Подключить независимую службу в соответствии с #23, не до завершения G5; MenuBarExtra показ реального состояния, закрытие окон не гасит Worker, безопасная остановка службы. Проверить перезапуск, сон, аварийное восстановление, отсутствие auto-open windows.

Каждый этап — один связный проверяемый patch. Нет нужды останавливать независимую работу Core на G1–G4: UI-B/C можно реализовать с Preview fixtures и реальным read-only FFI.

## 3. Файловая структура (рекомендуемая, не навязанная)

- `platform/macos/QADREN/DesignSystem/Theme.swift`, `Tokens.swift`, `Controls.swift`, `WidgetChrome.swift`
- `platform/macos/QADREN/Dashboard/MonitoringWorkspace.swift`, `WidgetRegistry.swift`, `LayoutStore.swift`, `Widgets/*.swift`
- `platform/macos/QADREN/Settings/SettingsWorkspace.swift`, `Connections*.swift`, `Scenarios*.swift`
- `platform/macos/QADREN/Inspectors/*.swift`

Не создавать пустые каталоги или отдельный файл для каждой иконки; фактическую структуру определяет читаемость и минимальные зависимости. Сборка `scripts/build-macos.sh` может иметь явный список Swift-файлов — **обновить manifest/script**, не предполагать, что Xcode сам найдёт новые файлы.

## 4. Обязательные security UX-инварианты

- Никаких скачиваний содержимого на основной MacBook через QADREN без отдельного explicit scoped consent. Browse/list/stat/metadata не включают content.
- UI не подтверждает verified success без соответствующего evidence Rust. UNKNOWN не имеет безусловного retry. Успех запроса Яндекс API не является безусловным доказательством полной доставки и целостности.
- Хранилища/задачи с одинаковым display name различать по реальному connection_id/Job ID.
- Не удалять пользовательские файлы через remove connection; локальная чистка credentials с recoverability #25.
- No secret, auth URL, access token, file content in diagnostics, screenshots/previews, analytics.
- Ни один preview/demo-показатель не попадает в runtime. Отдельный флаг/fixture mode только для UI tests.

## 5. Проверка перед публикацией

### Детерминированные проверки

1. `python3 -m json.tool docs/design/tokens.json >/dev/null`; все ссылки Markdown и logo asset существуют, JSON не содержит secrets.
2. `bash scripts/test-core.sh` (если изменения Swift-only, фиксировать статус проверки, не выдавать её за UI test).
3. `bash scripts/build-macos.sh` на поддерживаемом Mac + Swift relevant tests. Если macOS недоступна, пометка `BLOCKED`, не `PASS`.
4. Visual review screenshots в `dark` и `light` при 1180×760, 900×620, settings 980×700 и 760×560; без overflow и нарушенной компоновки. Не считать HTML-прототип скриншотом нативного SwiftUI.
5. `Reduce Motion`, `Increase Contrast`, VoiceOver keyboard navigation; состояние focus, тултипы полных путей и disabled features.

### Feature и negative-state matrix

| Case | Expected |
|---|---|
| Fresh install, no storage | честный empty state; ссылка на настройки, не «всё синхронизировано» |
| 2 Yandex connections | отдельные строки/credentials, никаких смешанных квот |
| Job running and 1000 queued | lazy list; progress только по реальным данным; стабилен scroll/selection |
| UNKNOWN after crash | отдельная цвет/label семантика, read-only inspect + reconciliation |
| iCloud out of scope | iCloud отсутствует в мастере и активных действиях; без кнопок подключения, entitlement probe и fake quota. Если в legacy данных есть iCloud connection, отображать как disabled/unsupported без I/O |
| Auth revoked / no network | stale timestamp + причина; не зелёное «подключено» |
| Global filter excludes critical job | глобальный attention остаётся виден |
| Edit grid, resize, undo, close/open | deterministic layout и персистентность, без overlap |
| Two independent dashboards | независимые local layouts, один execution state |
| Request content preview/download on primary MacBook | explicit scoped consent до QADREN-инициированного чтения |
| Close all UI while service running | после G5 Worker продолжает исполнять; до G5 честное ограничение |
| System sleeps/timezone changes | UI отражает реальное расписание, не вычисляет DST суррогатами |

## 6. Готовность и отчёт

Для каждой Issue фиксировать `HEAD/base SHA`, список компонентов, реальные API, скриншоты/preview snapshots, `PASS/FAIL/BLOCKED/SKIPPED`, commit SHA, известные остаточные риски и owner decisions. Выполнять code review + security review. AI-Docs-Hub — только дополнительный project-scoped индекс/review; не источник UI-архитектуры.

**Критерий DONE для дизайн-системы в коде:** не просто похожий screenshot. Тема применена через один token layer, каждый виджет использует общий frame/states, layout/filters/session сохраняются, реальные данные не подменены, опасные действия без Core authorization недоступны, accessibility и native build реально пройдены.

## 7. Чего Codex не должен делать

Не рисовать новый логотип. Не возвращать обычный dashboard с постоянным Sidebar. Не добавлять универсальную систему произвольных HTML виджетов, встроенный браузер, WebView или отдельный frontend runtime. Не переписывать Rust ради декора. Не создавать дополнительную документационную ветку/дублирующий skill. Не «выполнять» действия простым изменением SwiftUI state без durable Core confirmation. Не превращать симпатичные демо-диаграммы в показатели продукта.
