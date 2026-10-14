# Release 1 — матрица проверок и доказательства

Статус: рабочий контрольный документ для [Epic #12](https://github.com/Xsoid/QADREN/issues/12), [#13](https://github.com/Xsoid/QADREN/issues/13), [#25](https://github.com/Xsoid/QADREN/issues/25).

Отметка «реализовано» или успешная компиляция **не** заменяет проверку поведения. Тест в mock-хранилище не является проверкой настоящего Keychain/облака.

**Scope Release 1 (2026-10-12):** загрузка/скачивание и синхронизация между Яндекс Диском и локальными директориями, только реально подтверждённые capabilities. iCloud adapter/container/sync/backup, native spike и E2E не являются acceptance. Упоминания iCloud File Provider ниже — безопасность Local FS, не iCloud integration. [D-017](../decisions/D-017-defer-icloud.md).

## Dispatcher workspace shell (2026-10-12)

- macOS-клиент больше не собирает все технические элементы в одну прокручиваемую страницу. Корневая навигация разделена на два пространства: `Рабочая панель` для мониторинга и оперативных действий и `Настройка сценариев` для подключений, будущих правил и назначения Worker. Очередь и двухпанельный файловый менеджер открываются внутри рабочей панели, а не становятся конкурирующими корневыми разделами.
- Полные пути локальных панелей доступны для выделения, сохраняются два независимых контекста панелей и множественный выбор. Это всё ещё metadata-only просмотр: содержимое не открывается и не предпросматривается.
- Оболочка не заявляет о готовности отсутствующих частей Release 1: нет исполнения Scenario, синхронизации содержимого, безопасных copy/move/rename/delete Jobs, сохранения конфигурации панелей/окон, timeline, дерева Job или настраиваемых виджетов. Они остаются отдельными acceptance-пакетами.
- Проверка первой версии этого изменения: `bash scripts/test-core.sh` (208 Rust tests) и `bash scripts/build-macos.sh` на macOS успешны; последняя команда собрала `dist/QADREN.app`. Последующая корректировка ниже заменила первоначальную sidebar-навигацию.

### Независимые пространства и окна (2026-10-12)

- Боковая панель удалена. Смена пространства выполняется через меню `Пространство` в toolbar macOS.
- `Рабочая панель` — `WindowGroup`: каждая открытая панель мониторинга получает независимое состояние SwiftUI и может быть создана кнопкой `Новая панель`, а затем размещена пользователем на другом экране. QADREN не синхронизирует конфигурацию таких окон между устройствами.
- `Настройка сценариев` — отдельное singleton-окно; оно открывается из того же меню и не смешивается с оперативной мониторинговой панелью.
- Проверка: после `bash scripts/build-macos.sh` accessibility tree открытого приложения подтвердил отсутствие sidebar, меню `Пространство` и кнопку `Новая панель` в toolbar. Автоматического теста размещения на нескольких физических дисплеях нет.

### Меню строки меню (2026-10-12)

- Системное меню QADREN использует тот же SwiftUI `openWindow(id:)`, что и toolbar. Оно не ищет произвольное `NSApp.windows` и поэтому не зависит от наличия устаревшего singleton-окна после перехода на `WindowGroup`.
- В меню доступны `Открыть рабочую панель`, `Новая рабочая панель` и `Настройка сценариев`; это те же независимые сцены, что открываются из интерфейса приложения.

| Область | Автоматическая проверка | Реальная проверка | Состояние на 2026-10-09 |
|---|---|---|---|
| Rust Registry | `scripts/test-core.sh`, migration v1→v2, FFI, разные account IDs | Неповреждённая SQLite после обновления установки | Rust CI успешно на `ff585c7`; обновление реальной установки не проверялось |
| macOS SwiftUI | Компиляция и проверка .app в Actions | Запуск UI и восстановление состояния окон | CI успешно; ручной smoke test отсутствует |
| Keychain remove | Swift fault injection: disable / Job attempt gate / Keychain / Registry failures, retry, второй аккаунт | Принудительный отказ системного Keychain и повтор после перезапуска | Локальный runner проходит на 2026-10-12; системная проверка отсутствует |
| Legacy OAuth | Проверка кода сохранения legacy до успешного импорта | Импорт старого токена на реальном Mac и rollback | Реальная проверка в этом цикле отсутствует |
| Yandex metadata | Rust mock HTTP тесты | Авторизация, квота, 2 аккаунта, offline/401 | Исторически подтверждено в документации; в этом цикле не перепроверено |
| Content / Yandex / Local FS / Worker | Частичные Local FS и Yandex content primitives существуют; полного Job execution нет | Реальные Яндекс↔локальный каталог и Local↔Local, consent, integrity и recovery | PARTIAL: полный E2E не подтверждён; iCloud отложен |
| Ресурсы | CI фиксирует размер .app, без замера idle RAM/CPU | Activity Monitor, `ps`, network bytes, sleep/wake | Baseline не измерен |
| Подпись и поставка | CI собирает неподписанный bundle | Gatekeeper, entitlement, notarization, clean install | Только future Release 1 |

## Обязательные инварианты

- Содержимое облачных файлов QADREN не загружает на основной MacBook до явного согласия конкретной операции.
- `connection_id` — идентификатор независимого подключения, `provider_account_id` может совпадать.
- Удаление подключения **никогда не удаляет файлы провайдера**; сбой между Registry и Keychain не теряет возможность безопасно завершить локальную очистку.
- Незавершённый, неподтверждённый эффект не превращается в `SUCCESS` автоматически.
- Никакого runtime BackupSet и собственной истории версий файлов.
- Существующие Apple-системные фоновые загрузки не блокируются.

## Перед завершением #13

- [x] Пройден `main` CI на [ff585c7](https://github.com/Xsoid/QADREN/actions/runs/37857950583), в том числе новый Swift test runner.
- [ ] Выполнить smoke test подписанного/локального UI на основном Mac без воздействия на пользовательские данные.
- [ ] Подтвердить Keychain retry и legacy migration на реальном Mac с безопасными тестовыми подключениями.
- [ ] Снять idle CPU/RAM, диск/сеть и записать версии macOS, Xcode и Rust.
- [ ] Внести обнаруженные дефекты в отдельные Issues; не выдавать будущую поддержку iCloud за действующую.

## Connection cleanup gate — #25 (2026-10-12)

- Remove сначала переводит connection в `disabled`, затем выполняет Rust read-only gate по `connection_id`. Активная/UNKNOWN попытка, legacy attempt без target binding либо недоступность JobStore останавливают удаление до Keychain: disabled Registry record и credential остаются для recovery/reconciliation.
- Локально пройдены `bash scripts/test-core.sh` (fmt, Clippy, 197 Rust unit tests) и `bash scripts/build-macos.sh`. Отдельный Swift fault-injection runner проверил normal flow, отказ на Job gate, Keychain и Registry, повтор после каждого отказа и независимость второй connection.
- Реальный macOS Keychain lifecycle подтверждён отдельным test-only runner: запись, чтение, update duplicate item, delete и повторный delete для непредсказуемого `connection_id`; runner не обращается к Registry, provider или пользовательским credentials. Не выполнены проверка принудительного отказа системного Keychain, restart между disable и cleanup и live Scenario/adapter attempt. Поэтому #25 остаётся открытой и не является подтверждением готовности настоящего disconnect UX.

`YandexConnection.disconnect()` использует тот же fail-closed протокол: disable → read-only attempt gate → Keychain delete → единый SQLite finalization (`disconnected`, credential reference cleared, connection снова доступно для явной reauthorization). При сбое до finalization запись остаётся disabled; удаление облачных файлов и обращение к провайдеру отсутствуют. Фактические результаты этого изменения фиксируются вместе с SHA в #25.

Локальное `local_filesystem` подключение теперь не попадает в Keychain-only ветку: Remove использует disable → тот же read-only attempt gate → удаление только Registry record. Keychain, локальные пользовательские файлы и provider не затрагиваются. При unresolved attempt запись остаётся disabled для reconciliation/retry. Swift fault-injection runner дополнительно проверяет normal и blocked local flow.

## Scenario metadata-only dry-run planner — #19 (2026-10-12)

- Rust `scenario_planner` строит детерминированный план только из разрешённых metadata snapshots и последнего подтверждённого `FileSyncState`; он не выполняет I/O, не создаёт Job и не меняет baseline.
- `Create`/`Update` остаются предложениями. Удаление всегда требует отдельного policy decision; изменение destination и одновременные изменения дают conflict. Равенство имени/размера не считается равенством байтов: при отсутствии trusted checksum или provider revision результат `NeedsVerification`.
- Локально пройден `bash scripts/test-core.sh` с 202 Rust tests. Нет adapter scan, persistence, FFI/UI, Job submission или E2E; #19 остаётся PARTIAL.

### Registry-backed preview composition — #19 (2026-10-13)

- `Registry::plan_scenario_dry_run` загружает только immutable Scenario revision и его revision-bound подтверждённый `FileSyncState`, после чего передаёт caller-supplied metadata snapshots в pure planner. Тест подтверждает, что последующая смена direction не меняет preview старой revision и preview не записывает baseline.
- Rust C ABI `qadren_scenarios_dry_run` принимает только caller-supplied JSON metadata snapshots и exact `scenario_id`/revision, затем возвращает deterministic plan с явными `create`, `update`, `conflict`, `needs_verification` или `delete_requires_decision`. FFI ограничивает каждый JSON payload 1 MiB и суммарно 10 000 entries, не сканирует adapters, не читает content, не создаёт Job, не меняет FileSyncState и не записывает policy decision. Некорректный/слишком большой JSON или revision 0 отклоняются до Registry planning. UI для этого FFI ещё не добавлен.

- Граница намеренная: нет adapter scan, UI, чтения контента, Job submission, policy decision или записи FileSyncState. Если Registry или planner не может доказать корректный план, метод возвращает ошибку; выполнение не начинается. #19 остаётся PARTIAL до реального авторизованного metadata scan и отдельного execution path.

## Automation coalescing baseline — #21 (2026-10-12)

- Pure Rust state machine coalesces a burst of automatic triggers into one pending run and keeps exactly one follow-up while a Job is active. Disabled/locally paused Scenario ignores automatic triggers, while a manual request remains admissible.
- SQLite v5 хранит только локальную паузу и один coalesced pending trigger. Active Job намеренно не дублируется: его durable ownership остаётся у JobStore и должен передаваться будущим scheduler при оценке trigger. Disable или local pause очищают pending automatic run; ни одна из этих операций не создаёт и не меняет Job.
- Добавлен pure Rust evaluator weekly schedule на IANA timezone. `chrono-tz` встраивает только базу timezone в Rust binary, не добавляет сетевой сервис или runtime для пользователя. Повторяющийся DST slot получает один стабильный local key; несуществующий slot не создаёт запуск.
- SQLite v6 хранит несколько валидированных расписаний на Scenario (IANA timezone, weekday, время). Это configuration-only слой: оценка времени, catch-up и создание Job ещё не подключены.
- SQLite v7 atomically claims evaluated occurrence keys. Повторный DST slot и повтор после restart возвращают уже занятый key и не могут породить второй будущий Job.
- `Registry::due_schedule_occurrences` читает enabled schedules только enabled/unpaused Scenario и детерминированно возвращает DST-safe кандидаты за заданный UTC interval. C ABI принимает явные RFC 3339 UTC bounds и также остаётся read-only; durable submission добавлен позднее отдельным Registry outbox.
- Settings показывает для выбранного Scenario до пяти реальных кандидатов на следующие 14 дней через этот C ABI. Пустой список честно означает, что в этом окне нет подходящего enabled/unpaused расписания; экран не создаёт Job, не claim-ит occurrence и не выполняет adapter/content I/O.
- Settings также показывает и меняет durable локальную паузу автоматизации Scenario через узкий FFI. Пауза очищает только ожидающий automatic trigger, сохраняется в Registry и исключает Scenario из candidate evaluation; существующие Job не меняются, а ручные запросы остаются отдельным путём.
- `scripts/test-macos-scenario-schedules.sh` создаёт отдельный временный Registry и проверяет Swift/FFI bridge: enabled weekly UTC schedule возвращает ровно один ожидаемый slot, пауза сохраняется и исключает candidate, а повторный read-only запрос не меняет occurrence. Локально пройдены `bash scripts/test-core.sh` с 231 Rust tests, native runner, `bash scripts/test-macos-job-inventory.sh`, `bash scripts/test-macos-keychain.sh` и `bash scripts/build-macos.sh`. Restart-тест подтверждает сохранение pending/pause. На тот момент не было catch-up orchestration, event debounce, priority aging или Job submission; outbox bridge добавлен ниже. #21 остаётся PARTIAL.

### Durable schedule outbox — #21 (2026-10-14)

- Registry schema v9 atomically records a DST-safe schedule receipt and one pending Job outbox item for the immutable current Scenario revision. A slot is not consumed without a durable future submission request.
- `schedule_dispatch::dispatch_pending_schedule_jobs` reads only this outbox and enqueues the exact Scenario-bound Job in the existing `JobStore`; it does not execute a Scenario, touch adapters, read content or start a timer.
- Registry and JobStore deliberately remain separate SQLite databases. A restart after the JobStore commit but before the outbox acknowledgement observes the same Job identity/revision and marks the request dispatched without creating another Job. A mismatching existing Job fails closed.
- This is not a scheduler service or transfer path: catch-up policy/window ownership, event debounce, queue priority/aging, FFI/UI runtime and Scenario execution remain unimplemented.

### Scheduler dispatch C ABI — #21 (2026-10-14)

- `qadren_scenario_schedules_dispatch` accepts explicit RFC 3339 UTC bounds plus Registry and JobStore paths, stages every due slot, then drains the durable outbox through the same Rust dispatcher.
- The ABI returns only `staged` and `dispatched` counts. It creates queued revision-pinned Jobs and does not execute a Scenario, touch any adapter, read content, or imply transfer consent.
- The ABI regression proves first invocation stages and queues one Job and a repeated invocation returns zero/zero; this preserves restart idempotency at the native boundary.

## Scoped destructive-operation consent — #20 (2026-10-12)

- Pure Rust policy gate допускает destructive operation только с explicit accepted decision для точных operation, connection, relative path и срока. Нет решения, deny, expiry или несовпадение scope означают fail-closed.
- Локально пройдены `bash scripts/test-core.sh` с 208 Rust tests и `bash scripts/build-macos.sh`. Нет durable decisions, UI prompt, adapter pre-I/O recheck или E2E; #20 остаётся PARTIAL.

Следующий минимальный durable слой хранит решение только для Job в `waiting_for_decision`: SQLite v8 связывает его с точными operation/connection/relative path и сроком. Accepted decision расходуется атомарно ровно один раз; denied, expired, повторное или scope-mismatched решение блокируют операцию. Запись/расходование не выполняют adapter I/O; если процесс завершится после расходования и до будущего effect, Job остаётся в ожидании, а для повтора потребуется новое явное решение. UI prompt, adapter pre-I/O recheck и реальные destructive operations по-прежнему не реализованы.

## Local metadata workspace — #22 (2026-10-12)

- Пользователь выбирает локальную папку на текущую сессию; Rust FFI показывает только metadata entries и создаёт подпапку. Content read, copy, move и delete не доступны.

Рабочий процесс: небольшие коммиты напрямую в `main`, обязательное ожидание CI и регистрация ограничений; ветки документации не создаются.

## Local Filesystem Adapter — #15 (2026-10-11)

- Подтверждено пользователем на macOS для `f5f6bab`: `bash scripts/test-core.sh`, 95/95 Rust unit tests, 0 failures; форматирование и Clippy пройдены. Это не подтверждает будущие операции rename/move/delete и не является E2E iCloud.
- В `340ab81` устранён обход проверки каталога через `list_page(..., limit=0)`: даже пустая страница требует полноценной проверки директории. Добавлен тест отсутствующего пути, обычного файла и symlink.
- `340ab81` пока **не прошёл подтверждённые CI/macOS тесты**. Считать его требующим проверки, не закрывать #15.
- Продолжают оставаться запрещёнными: автоматическое materialize/download на основном MacBook, небезопасные rename/move/delete и использование незавершённого адаптера для destructive Jobs.

### Root confinement hardening (2026-10-11)

- После подтверждения 95/95 unit tests на `98d79dd`, в `a657bb4` изменено открытие Unix root: теперь путь обходится по компонентам через `openat(O_DIRECTORY|O_NOFOLLOW)`, поскольку `O_NOFOLLOW` на полном пути защищает только последний компонент.
- `dc0bdbf` добавляет негативный тест на промежуточную symlink в пути root; при отказе не должно происходить обхода за пределами заданной директории.
- Новые коммиты требуют проверки fmt/Clippy/macOS tests. Безопасные rename/move/delete и проверка iCloud residency по-прежнему не готовы; запрет чтения контента на основном Mac сохранён.

### Подтверждение root confinement и новая гонка публикации (2026-10-11)

- Пользователь выполнил `scripts/test-core.sh` на `acad913`: fmt и Clippy пройдены, 96/96 unit tests и 0 doc-tests успешны.
- `744856c` добавляет тест подмены **конечного имени** уже после `linkat`: операция должна вернуть `OutcomeUnknown`, сохранить чужой конечный файл и исходные байты по перемещённому имени. Это regression / fault injection, **не доказательство атомарности против активного злоумышленника**.
- Новый тест ещё не подтверждён на macOS. Безопасные move/rename/delete и iCloud placeholder materialization остаются блокерами #15.

### Проверка после 97/97 и дополнительная защита staging (2026-10-11)

- Пользователь подтвердил на MacBook `7aa841d`: `scripts/test-core.sh` — fmt/Clippy clean, 97/97 unit tests passed, 0 doc-tests.
- `20b19bb`: после удаления staging name и синхронизации каталога дополнительно перепроверяется inode конечного имени с ожидаемым числом ссылок 1. Несовпадение возвращает `OutcomeUnknown`, а не ложный успех.
- `f00a0e8`: негативный тест внешнего hardlink на staging inode во время чтения. Публикация должна быть отвергнута и чужая ссылка не должна удаляться.
- Текущие два патча ещё **не подтверждены тестами macOS**. Проверки не являются гарантией защиты против враждебного процесса с конкурентным доступом на запись в тот же каталог. До её проектирования небезопасные move/rename/delete и автоматическое чтение content на macOS остаются отключёнными.

### Capabilities локального адаптера (2026-10-11)

- Пользователь проверил `805ce3d` на MacBook: fmt/Clippy успешны, 98/98 unit tests passed, doc-tests 0.
- `9deeb67` добавляет `LocalFilesystem::capabilities()` на основе реально включённых функций: чтение содержимого — только Linux, создание новых файлов и каталогов — Unix, операции delete/rename по-прежнему запрещены. Для Windows поддержка только metadata, остальное fail-closed.
- Добавлен тест непротиворечивости capabilities. Новая проверка пока не выполнена на macOS; capabilities не отменяют требования индивидуального согласия на чтение содержимого.

### Scoped local copy — #15 (2026-10-11)

- Пользователь подтвердил `df2fff5`: `scripts/test-core.sh`, fmt/Clippy без ошибок, 99/99 Rust unit tests, 0 doc-test failures.
- `44909b5` добавляет `copy_file_new(from, to, consent)`: Linux читает через pinned descriptor и публикует только новый файл без overwrite; отказ в consent запрещает чтение и создание. На macOS метод намеренно fail-closed даже при consent до верификации File Provider placeholder residency; не считать этот путь готовым для macOS.
- Добавлен негативный тест отсутствия согласия, существующего destination и неизменности данных; для Linux также успешная локальная копия.
- Новый код ещё не подтверждён macOS CI/локальным запуском; операция не подключена к Job Engine. Rename/move/delete, macOS content residency и настоящие fault-injection испытания остаются открытыми.

### Local copy safety regression (2026-10-11)

- Пользователь подтвердил `ca747a0`: `scripts/test-core.sh`, fmt/Clippy пройдены, **100/100** Rust tests, doc-tests (0) успешны.
- `38d5c99` добавляет негативный тест `copy_file_new`: отклонение source/destination с `..`, отсутствующего source и symlink источника (Unix), без публикации целевого файла или изменения исходного содержимого.
- Дополнительный тест ещё не проверен на macOS. На macOS content read/copy остаются fail-closed до residency проверки; недоверенные перемещения, переименования и удаления не реализованы.

### Copy source consistency check (2026-10-11)

- Пользователь подтвердил `58d4f9b`: `scripts/test-core.sh`, 101/101 unit tests, fmt и Clippy успешны.
- `7821669`, `2358b8b`, `cf88625`: после Linux-only copy выполняется контроль размера и времени модификации исходного открытого файла. Изменение во время копирования классифицируется `OutcomeUnknown`, целевой файл может существовать и не должен автоматически перезаписываться. Добавлен Linux-only regression test изменения исходного файла.
- Контроль метаданных **не является snapshot isolation или проверкой хеша**: переписывание с тем же размером и временем модификации может остаться незамеченным. Новые проверки на Mac и Linux ещё не подтверждены. macOS content read по-прежнему отключён.

### Усиление проверки источника при Linux-копировании (2026-10-11)

- Пользователь подтвердил `8dfdcf5` на macOS: fmt/Clippy clean, 101/101 tests, doc-tests 0.
- `b6f265e` проверяет открытый исходный inode (`dev`, `ino`), длину, `mtime` и `ctime` с наносекундами и число переданных байтов. Добавлен Linux-only тест обнаружения неполной передачи.
- При несоответствии после публикации возвращается `OutcomeUnknown`; автоматический повтор недопустим.
- **Предел:** проверки метаданных не обеспечивают snapshot и не гарантируют обнаружение подмены байтов с восстановленными метаданными. Новые тесты Linux не проверены в реальном Linux-окружении; macOS не исполняет этот `cfg(target_os = "linux")` путь.

### Linux streaming read consistency (2026-10-11)

- Пользователь подтвердил `c6f83fd` на macOS: `scripts/test-core.sh`, 101/101 Rust unit tests, fmt/Clippy без ошибок.
- `66a067d` добавляет после Linux `read_to_writer` проверку сохранности метаданных **того же открытого исходного inode** и количества реально переданных байтов; обнаруженная мутация возвращает `OutcomeUnknown`, поскольку получатель уже мог принять данные.
- Добавлен Linux-only тест с Writer, изменяющим исходный файл во время потокового чтения. Этот код и тест **не выполнены на Linux**; macOS не исполняет его и продолжает запрещать чтение файлового содержимого.
- Проверка inode/mtime/ctime/длины не является консистентным снапшотом и не доказывает идентичность байтов.

### Nested local directory creation (2026-10-11)

- Пользователь подтвердил `571de26`: `scripts/test-core.sh`, 101/101 macOS Rust tests, fmt и Clippy пройдены.
- `1fc6ab4` / `346205b` добавляют `LocalFilesystem::create_directory_tree` с пошаговым созданием отсутствующих вложенных папок внутри pinned root без обхода symlink, без замены существующих объектов. Результат — количество созданных каталогов.
- Если после создания хотя бы одного каталога следующая операция завершается ошибкой, возвращается `OutcomeUnknown`; уже созданные папки сохраняются для сверки, автоматический rollback отсутствует. Тесты проверяют успешное/повторное создание, отказ для файла в пути, traversal и частичный успех при компоненте пути длиной 300 символов.
- Новый функциональный срез **ещё не проверен на macOS**; никакие destructive операции или macOS content read не включены.

### Конкурентное создание каталогов (2026-10-11)

- Пользователь подтвердил `fdf43a9` на Mac: `scripts/test-core.sh`, 103/103 tests, форматирование и Clippy пройдены.
- `e8f9c27`: `create_directory_tree` при `AlreadyExists` после проверки отсутствия компонента повторно проверяет, что появился именно каталог в pinned root. Символические ссылки и обычные файлы не считаются успешным созданием.
- Добавлен Unix-тест, подтверждающий, что ссылка внутри корня не позволяет создать вложенный каталог за его пределами.
- Текущий коммит ещё не проверен на Mac; тест не доказывает отсутствие всех гонок между отдельными шагами. Move/rename/delete и чтение File Provider placeholders на главном Mac по-прежнему отключены.

### Partial streaming output safety (2026-10-11)

- Пользователь подтвердил `6ac84ef`: `scripts/test-core.sh`, fmt/Clippy clean, 104/104 macOS tests passed, 0 doc-tests.
- `ddc96b0` изменяет Linux `read_to_writer`: ошибка из `io::copy` теперь `OutcomeUnknown`, потому что при передаче получатель мог принять часть байтов и автоматический повтор небезопасен. Добавлен Linux-only тест Writer, который принимает один байт и отказывает на следующем вызове.
- Новый патч ещё не проверен на macOS/Linux. Не завершены safe rename/move/delete, macOS placeholder guard и основные acceptance-тесты #15.

### Local filesystem capacity inspection (2026-10-11)

- Пользователь проверил `e91f0fb` на macOS: `scripts/test-core.sh`, fmt/Clippy passed, 104/104 Rust unit tests, doc-tests 0.
- `94158a5`: добавлен read-only `LocalFilesystem::storage_space()` с `fstatvfs` закреплённого дескриптора корня на Unix. Возвращает общий и доступный объём байтов без чтения содержимого файлов. Добавлен тест, включающий проверку после удаления имени корневого каталога при живом pinned descriptor.
- Значения не являются резервированием дискового пространства и не доказывают возможность последующей записи; ENOSPC в реальной операции всё равно обрабатывается отдельно. На неподдерживаемых платформах возвращается отказ. Новый патч пока не проверен на Mac.

### Capacity preflight with safety reserve (2026-10-11)

- Пользователь подтвердил `1e12af3` на macOS: `scripts/test-core.sh`, fmt/Clippy successful, 105/105 unit tests, doc-tests 0.
- `b9a954f` добавляет `require_free_space(incoming_bytes, reserve_bytes)` с проверкой переполнения суммы и сравнением с `storage_space()` по pinned descriptor, без чтения содержимого файлов и без создания объекта. Новый Unix тест проверяет нулевой запрос, переполнение и запрос сверх доступного объёма.
- Это **предварительный отказ**, не резервирование и не транзакция; параллельные процессы могут занять пространство, поэтому обработка ENOSPC во время фактической записи остаётся обязательной. Метод пока не интегрирован в исполнение Jobs.
- Новые изменения **не проверены на macOS**; операции move/rename/delete и guard iCloud placeholders не реализованы.

### Reserved local file creation (2026-10-11)

- Пользователь подтвердил `6ce4d3a` на macOS: `scripts/test-core.sh`, форматирование и Clippy без ошибок, 106/106 unit tests, 0 doc-tests.
- `c867ecd` добавляет `create_with_space_reserve` — проверку требуемого размера и резерва непосредственно перед `create_from_reader`. Ошибка предварительной проверки не открывает Reader и не создаёт файл. Добавлен Unix-тест `PanicReader`, доказывающий отсутствие чтения при переполнении суммы.
- Это НЕ гарантирует наличие места во время записи и не заменяет ENOSPC/OutcomeUnknown обработку в существующем атомарном writer. Реальный размер входного потока пока не проверяется против `expected_bytes`; новый патч ещё не проверен на Mac.

### Exact declared size for reserved file creation (2026-10-11)

- Пользователь проверил `2e3fa3f` на Mac: `scripts/test-core.sh` fmt/Clippy clean, **107/107** unit tests, 0 doc-tests failures.
- `d96b1c6`, `f5b7a91`: `create_with_space_reserve` применяет streaming `ExactSizeReader` к передаваемым данным. Он блокирует публикацию, если источник раньше времени завершился либо оказался длиннее `expected_bytes`. При ошибке временная запись очищается существующим механизмом `create_from_reader`; тест покрывает короткий, длинный и точный источник.
- Новый тест и изменения ещё **не проверены** на macOS. Метод не резервирует свободное место и не обеспечивает snapshot содержимого. Любое изменение после публикации по-прежнему требует обычной процедуры reconciliation.

### Guarded local copy with reserve (2026-10-11)

- Пользователь подтвердил `7c6c5ad` на Mac: форматирование и Clippy без ошибок, **108/108** unit tests, doc-tests 0.
- `004f9f2` добавляет `copy_file_new_with_reserve`: Linux-only чтение исходного файла через закреплённый дескриптор, проверка свободного места с резервом и точной длины до атомарной публикации, проверка метаданных открытого source после создания. Тест защищает существующий destination и подтверждает fail-closed без согласия или на macOS.
- На macOS чтение содержимого всё ещё блокируется даже после consent. На Linux файловые мутации источника при копировании по-прежнему проверяются только по метаданным, не как snapshot. Код нового патча пока не проверен локальными тестами Linux/macOS; rename/move/delete ещё не реализованы.

### Read-only move preflight (2026-10-11)

- Пользователь подтвердил `4b385d4` на macOS: форматирование, Clippy, компиляция и **109/109 unit tests**, 0 doc-tests ошибок.
- `d422a7c` добавляет `preview_move_new(from, to)` как read-only проверку: пути остаются внутри pinned root, source должен быть обычным файлом, destination отсутствует, parent destination существует и является каталогом. Возвращаются метаданные source, никакие данные не читаются и ни один объект не перемещается. Тест проверяет предпросмотр, конфликт, traversal и отсутствие изменений в source/destination.
- Результат предварительной проверки не гарантирует безопасности последующего rename из-за TOCTOU; `move_file_new` остаётся заблокированным до identity-preserving реализации и adversarial tests. Код нового патча ещё не проверен на Mac.

### Descriptor-relative file identity (2026-10-11)

- Пользователь подтвердил `15e7e0a` на macOS: форматирование, Clippy и 110/110 Rust unit tests без ошибок.
- `28a8712`, `d9934c6`: введены `LocalFileIdentity` (device/inode/size) и `LocalFilesystem::file_identity(path)`. Unix реализация использует `fstatat(AT_SYMLINK_NOFOLLOW)` относительно pinned root, не открывает содержимое; тест проверяет сохранение идентичности после внешнего переименования и запрет traversal.
- Полученный идентификатор пригоден для сопоставления результатов и reconciliation, **не предоставляет атомарное право на rename/unlink**. Повторное сравнение inode и pathname само по себе не устраняет TOCTOU/ABA. `move_file_new` остаётся fail-closed, destructive delete не добавлен. Новый код ещё не подтверждён на Mac.

### Move preview inode revalidation (2026-10-11)

- Пользователь подтвердил `8c8d9e0` на macOS: fmt/Clippy без ошибок, **111/111** Rust tests passed, 0 doc-tests.
- `dd04ce5` добавляет read-only `revalidate_move_new(from, to, expected)` — повторная проверка идентичности inode source и отсутствия destination, без открытия содержимого и без изменения файлов. Unix-тест заменяет source после предпросмотра и проверяет отказ с сохранением данных.
- Повторная проверка сама по себе не устраняет TOCTOU после возврата. `move_file_new` и destructive operations остаются заблокированными. Новые изменения пока не проверены на Mac.

### Case-collision guarded creation (2026-10-11)

- Пользователь подтвердил `3085447` на Mac: fmt, Clippy и **112/112** unit tests без ошибок.
- `112b2f9` добавляет отдельный `create_from_reader_case_guarded`: перед атомарной записью новый метод сканирует каталог назначения (через descriptor-relative Unix list) и консервативно отклоняет имена, совпадающие после Unicode lowercase. Тест проверяет `Report.TXT` против `report.txt` до чтения Reader, неизменность исходного объекта и создание свободного имени.
- Это **дополнительная проверка, не доказательство отсутствия всех Unicode/locale/case-folding коллизий**; финальную защиту от перезаписи по-прежнему обеспечивает атомарный no-overwrite writer. При небезопасных объектах в каталоге list может вернуть отказ; 기존 `create_from_reader` не изменён. Патч не проверен на Mac; move/rename/delete остаются fail-closed.

### Early destination collision on atomic create (2026-10-11)

- Пользователь подтвердил на Mac коммит `7160aac`: `scripts/test-core.sh`, 113/113 tests, fmt and Clippy successful.
- `0f14aaa` меняет реальную Unix запись `secure::create`: проверка имени назначения через `fstatat(AT_SYMLINK_NOFOLLOW)` до запуска потока данных и создания временного файла; существующее назначение отклоняется без чтения источника. Конечный `linkat` **остаётся обязательной атомарной проверкой** на конкурентные создания.
- `5f7efde` добавляет тест `PanicReader`, подтверждающий, что при занятом назначении источник вообще не читается и файл остаётся неизменным.
- В изменениях не разблокированы rename/move/delete и macOS content read; патч ожидает проверки на Mac.

### Bounded metadata-only recursive inventory (2026-10-11)

- Пользователь подтвердил `2dd94df` на Mac: форматирование, Clippy и **114/114** тестов без ошибок.
- `970ad18`, `3151ca4`: `LocalFilesystem::list_tree(path, max_entries)` перечисляет вложенную структуру через уже защищённые descriptor-relative `list` операции, возвращает относительные пути и метаданные, сортирует результат и отказывается возвращать неполный набор при превышении ограничения. Не открывает содержимое файла и не инициирует загрузку iCloud placeholders.
- Новый тест проверяет вложенные каталоги, лимит записей и запрет traversal. Не заменяет атомарный snapshot: при конкурентных изменениях дерево может быть непоследовательным, требуется повторное сканирование. Патч **ещё не проверен на Mac**. Операции move/rename/delete остаются отключены.

### Recursive inventory depth and unsafe subtree handling (2026-10-11)

- Пользователь подтвердил успешный запуск проверок после `d0943c1` (ожидавшиеся 115 тестов); детальный вывод в этом сообщении не предоставлялся.
- `4a2d67a` ограничивает вложенность `list_tree` 256 уровнями независимо от количества элементов, с отказом `UnsafeEntry` вместо частичного результата; добавлен тест на symlink внутри поддерева.
- Метод по-прежнему не формирует атомарный снимок дерева при конкурентных изменениях. Новый патч ещё не проверен на Mac, rename/move/delete пока заблокированы.

### Recursive inventory directory identity (2026-10-11)

- Пользователь подтвердил успешную локальную проверку патча `a175e8a` (ожидалось 116 тестов, полный журнал не предоставлен).
- `e5733fd`, `3fa958b`: Unix `list_tree` отслеживает `(device,inode)` каждого открытого каталога, отказывает при повторном посещении одного каталога, чтобы не возвращать дерево с дублированием или циклом. Дескриптор открывается без следования символическим ссылкам. `82e1f7f` проверяет сохранение inode каталога после переименования.
- Это не атомарный снимок файловой системы; конкурентные изменения возможны. Новые изменения ещё не проверены на Mac, операции rename/move/delete по-прежнему недоступны.

### Unix filenames without silent replacement (2026-10-11)

- Пользователь подтвердил успешный Mac-прогон после `d11123f` (предыдущий ожидаемый итог — 117 тестов).
- `cd25b02`, `9c40748`: Unix `list` и `stat` отказываются преобразовывать невалидное UTF-8 имя через `to_string_lossy()`; возвращают `UnsafeEntry` без изменений файлов. Добавлен тест на имя с недопустимым UTF-8 байтом, проверяет отказ и сохранность содержимого.
- Это fail-closed ограничение текущей строковой модели метаданных: для полноценной поддержки произвольных Unix byte-path потребуется отдельная модель имён, а не lossy Unicode. Изменения ещё не подтверждены macOS; move/rename/delete и безопасное чтение iCloud placeholders остаются открытыми.

### Mandatory collision guard in default local writer (2026-10-11)

- Пользователь подтвердил успешную проверку предыдущего патча `a4429f3` на macOS (ожидалось 118 тестов; полный вывод не предоставлен).
- `353e42d`: проверка регистронезависимого конфликта имён перенесена в основной `create_from_reader`, а `create_from_reader_case_guarded` стал совместимым делегирующим методом. Все пути создания (включая `create_with_space_reserve`) теперь используют единый guard. Отдельный тест проверяет основной API, отсутствие чтения source при конфликте и сохранность existing file.
- Проверка имен Unicode lowercase консервативная, **не** полноценная Unicode normalization/casefold; отдельный `linkat` остаётся авторитетной защитой от перезаписи при гонке. Новый патч требует проверки fmt/Clippy/tests на Mac. Rename/move/delete и placeholder guard не реализованы.

### Directory case-alias safety (2026-10-11)

- После проверки macOS 120 unit tests пользователь поручил продолжить #15 с процентом готовности и командами проверки в каждом отчёте.
- `9197381`, `b535fdd`: Unix `create_directory` выполняет предоперационную проверку регистронезависимой коллизии имён; отдельный test запрещает создание `assets` при существующем `Assets`, включая рекурсивный путь.
- `f3a55c1`, `be9c931`: `create_directory_tree` теперь проверяет точное написание **уже существующего** каталога перед продолжением, чтобы на case-insensitive ФС не создать неожиданный подкаталог под именем-алиасом.
- Эти проверки не обеспечивают атомарную защиту от всех Unicode-normalization коллизий или конкурентной смены каталогов. Rename/move/delete пока остаются fail-closed из-за TOCTOU идентичности исходного имени. Новый патч **не проверен** на macOS, проценты до подтверждения не увеличивать.

### Concurrent mkdir alias revalidation (2026-10-11)

- Пользователь подтвердил успешный тестовый запуск на macOS для `238d69a` (ожидался 121 тест, подробный журнал не приложен).
- `5288aaa`, `7a2a368`: когда `create_directory_tree` получает `AlreadyExists` при гонке создания, теперь проверяется не только тип директории, но и **точное написание имени** в родительском каталоге. Это устраняет обход case-collision guard на case-insensitive ФС, когда конкурирующий процесс создал каталог с иным регистром.
- При частично созданном дереве неопределённый итог классифицируется как `OutcomeUnknown`, без автоматического отката или повтора. Новый патч ещё не проверен на Mac; rename/move/delete и iCloud placeholder handling остаются блокерами закрытия #15.

### Recursive mkdir with unrelated symlink siblings (2026-10-11)

- Пользователь подтвердил успешную проверку предыдущего состояния main (`bc53814`, ожидаемый итог 121 тест); детальный вывод не предоставлялся.
- `fab2cbe`, `2ef5125`, `cf7cc16`: `has_exact_directory` сравнивает имена прямо в `readdir` относительно закреплённого directory fd и проверяет тип только найденного точного имени через `fstatat(AT_SYMLINK_NOFOLLOW)`. `create_directory_tree` использует эту проверку для существующих каталогов и при `AlreadyExists`, больше не требуя читать метаданные всех посторонних соседей. Это позволяет безопасно создавать вложенный каталог рядом с не относящимися к операции symlink.
- Добавлен Unix-регрессионный тест `mkdir_tree_accepts_existing_directory_with_unrelated_symlink`. Реальная rename/move/delete функциональность и macOS placeholder read по-прежнему отключены. Патч пока не проверен на Mac.

### Non-destructive hard-link move staging (2026-10-11)

- Пользователь подтвердил успешный macOS запуск после `2993a28` (ожидалось 122 unit tests).
- `2bce13c`, `34a48c0`: Unix `stage_move_link_new(from,to)` публикует новое имя посредством `linkat` без перезаписи, не читает содержимое и не удаляет исходное имя. Перед операцией проверяет, что source — regular file, и конфликт destination; после публикации сверяет `dev/inode` и выполняет `sync_all` каталога назначения. Любая ошибка после публикации возвращает `OutcomeUnknown`. Добавлен тест сохранения source, защиты существующего destination и идентичности inode.
- **Это не rename/move/delete.** Concurrent source pathname replacement не может быть полностью исключён одной проверкой перед `linkat`; в случае расхождения требуется reconciliation. Запрещены автоматический unlink исходника и blind retry. Операция не интегрирована с Jobs и не заявлена в capabilities. Новый код ещё не проверен macOS; число ожидаемых тестов — 123.

### Hard-link staging source identity reconciliation (2026-10-11)

- Пользователь подтвердил успешную macOS проверку предыдущего патча `31d1a4f`, ожидавшиеся 123 теста (подробный лог не приложен).
- `8f05d9c`: сразу после `linkat` повторно проверяется имя исходного файла через `fstatat(AT_SYMLINK_NOFOLLOW)` и сопоставление inode/device/type. Любое расхождение или ошибка **после публикации** возвращает `OutcomeUnknown`, запрещая слепой повтор; новый destination не удаляется автоматически.
- `4e883d5`: тест отказа hard-link staging для symlink-источника без создания назначения.
- Остаётся TOCTOU между самой командой `linkat` и проверками; операция не является атомарным безопасным rename, не выполняет unlink, не интегрирована в Job execution. Патч не проверен на Mac, ожидаемый итог 124 теста; прогресс #15 сохраняется на ~52%.

### Staging hard-link conflicts (2026-10-11)

- Пользователь подтвердил успешный тестовый запуск предыдущего патча `f113d5b` (ожидаемые 124 теста).
- `82e4200`: регрессионный тест регистронезависимого конфликта имени destination для hard-link staging.
- `7370f63`: отказывается создавать дополнительную ссылку на source с `st_nlink != 1`, поскольку уже разделяемый inode требует явного разрешения конфликта. `b330263`: негативный тест такого источника.
- Изменения не устраняют гонку TOCTOU для source, поэтому метод остаётся вспомогательной неразрушающей операцией, без интеграции в Jobs. Новый патч ещё не проверен на Mac; ожидаются 126 тестов. Оценка готовности #15 ~52%.

### Hard-link publication race fault injection (2026-10-11)

- Пользователь подтвердил успешный macOS-прогон предыдущего патча `1f10dfa` (ожидались 126 тестов).
- `971e4c6` добавляет тестовый callback строго после `linkat` в `link_file_new`, до проверки source. Production использует no-op.
- `b6c186e` добавляет adversarial-тест: после публикации исходное имя переименовывается и замещается другим файлом. Результат обязан быть `OutcomeUnknown`, при этом оригинальные bytes сохранены и в renamed source, и в staged hard link; подменённый файл не удаляется.
- Это не устраняет TOCTOU между preflight и `linkat`, не реализует delete/rename/move и не разрешает автоматические повторы. Новый патч пока не проверен на Mac, ожидается 127 тестов; готовность ~52%.

### Read-only hard-link staging reconciliation (2026-10-11)

- Пользователь подтвердил успешный прогон macOS после `12037c8` (ожидаемые 127 тестов).
- `6039528`, `b035b43`: `reconcile_staged_link(from,to,expected)` сопоставляет identity обоих имён с зафиксированным `(device,inode,size)` и возвращает `bool`, не читая байты и не выполняя мутаций. Тест подтверждает совпадение после публикации и отрицательный результат после подмены source, а также сохранность исходного inode.
- Это **наблюдение**, а не завершение move/delete и не разрешение на blind retry после `OutcomeUnknown`; идентификаторы и размер не гарантируют целостность содержимого при конкурентной модификации. Новый патч ещё не проверен на Mac. Оценка #15 ~52%.

### Missing hard-link paths during reconciliation (2026-10-11)

- Пользователь подтвердил успешную проверку `5b89d08` на Mac (128 тестов).
- `5864520`: `reconcile_staged_link` возвращает `Ok(false)`, когда исходное или опубликованное имя отсутствует (`NotFound`), без восстановления, повторной публикации или чтения содержимого. Ошибки другого типа по-прежнему возвращаются вызывающему коду.
- Unix-тест проверяет отсутствующий destination, затем отсутствующий source после внешнего rename, и сохранение содержимого исходного inode. Это не разрешение на автоматический повтор: любой ранее полученный `OutcomeUnknown` продолжает требовать ручного/управляемого разбора состояния.
- Новый патч ещё не проверен на Mac; ожидается 129 тестов; готовность #15 остаётся ≈52%.

### macOS metadata-only dataless identification (2026-10-11)

- Пользователь подтвердил успешную macOS проверку предыдущего состояния `d53c679` (129 тестов).
- `f3ad883`: macOS `fstatat(AT_SYMLINK_NOFOLLOW)` относительно закреплённого descriptor проверяет Apple `SF_DATALESS = 0x40000000` (XNU `bsd/sys/stat.h`). Не открывает контент и не инициирует материализацию.
- `eb8256e`: `content_residency(path)` возвращает `DatalessPlaceholder`, `NotDatalessFlagged` (не означает гарантированную доступность содержимого!) либо `Unknown` на Linux. macOS read-to-writer остаётся запрещённым даже после per-operation consent; статус не выдаёт разрешения на чтение. Добавлены macOS и Linux тесты.
- Известное ограничение: `SF_DATALESS` не заменяет полную оценку File Provider/iCloud `ubiquitousItemDownloadingStatus` и устойчивую per-operation no-hydration политику; отсутствующий флаг не доказывает, что file read безопасен. Новое состояние main пока не проверено на Mac, ожидаются 130 тестов на macOS (Linux: 130).

### Batch metadata inspection (2026-10-11)

- Пользователь подтвердил успешную локальную проверку предыдущего состояния (`d9dda36`, 130 тестов); GitHub-hosted Actions заблокированы биллингом, используем локальный `scripts/test-core.sh`.
- `8a736f2`, `ecf2a61`: добавлены `inspect_content()` и `inspect_content_batch()` — составной metadata-only preflight с проверкой inode/size до и после запроса residency. Сбор данных из нескольких файлов атомарности/консистентного snapshot не гарантирует. Если любой элемент отсутствует, является symlink или выходит за пределы root — полный вызов возвращает ошибку без частичного успешного результата.
- Три новых Unix-теста: batch для двух regular files, отказ на symlink и traversal, отказ на missing entry без создания данных. Ожидаемый локальный тестовый итог на macOS: 133 теста, ещё не проверено.
- Отсутствие `SF_DATALESS` не разрешает hydration или чтение содержимого. macOS content reads остаются запрещены. Расширенные File Provider statuses и безопасные rename/move/delete ещё не реализованы. Оценка #15: ~58%.

### Metadata batch revalidation after preflight (2026-10-11)

- Пользователь подтвердил успешный локальный прогон предыдущего пакета (`10faf08`; 133 теста).
- `db03c69`: добавлены `revalidate_content_inspection(path, expected)` и `revalidate_content_batch(paths, expected)` для повторной metadata-only проверки устаревших результатов preflight перед следующим шагом. Сверяются inode/device/size и residency. Отсутствующий файл возвращает `Ok(false)`, symlink/небезопасные пути возвращают ошибку; несоответствие длины массивов — `InvalidPath`. Пакет не объявляется консистентным снимком, операции проверки не дают права чтения и не устраняют TOCTOU.
- 3 Unix-теста: успешное повторное наблюдение, замена inode и удаление, stale batch и неверная длина, подмена symlink и сохранность данных. Новый код требует локального запуска macOS; ожидаются 136 тестов.
- Готовность #15 около 60%; полноценные безопасные move/rename/delete и File Provider statuses ещё не реализованы.

### Same-size local modification detection (2026-10-11)

- Пользователь подтвердил успешный локальный прогон предыдущего пакета (`4b1a9be`, 136 тестов).
- `d31845d`, `ca49575`, `70c7a69`: `file_change_stamp()` получает Unix `mtime`/`ctime` с наносекундной частью через `fstatat(AT_SYMLINK_NOFOLLOW)` в закреплённом root; `LocalContentInspection` включает четыре поля временной подписи. `inspect_content` проверяет подпись до/после residency и inode до/после, `revalidate_content_*` теперь выявляет изменения одинакового размера, не читая контент. Добавлены Unix-тесты same-length rewrite одного файла и в batch.
- Ограничение: время изменения не криптографическая контрольная сумма и не защита от намеренной подмены атрибутов / микрогонок. Вывод инспекции не является доказательством неизменности и не разрешает чтение cloud-content на Mac.
- Новый пакет ещё не проверен компилятором Mac; ожидается 138 unit-тестов. Готовность #15 приблизительно 60%.

### Duplicate-source protection for metadata batches (2026-10-11)

- Пользователь подтвердил успешную локальную проверку предыдущего пакета (`bd58430`, 138 тестов).
- `4dc0d73`: `inspect_content_batch` отказывает при повторе пути (включая ASCII case aliases) или повторном `(device,inode)` под другим именем. Это предотвращает двойную обработку hardlinked файлов на этапе планирования; не читает содержимое и не изменяет файлы. Правила регистронезависимости для Unicode отличаются между файловыми системами; окончательная проверка дубликатов по inode касается существующих файлов.
- Два новых Unix-теста: exact/case duplicate и hard-link alias. Ожидаются 140 тестов при следующей проверке на Mac. Не подтверждено компилятором; rename/move/delete и File Provider остаются незавершёнными.

### Pinned root overlap preflight (2026-10-11)

- Пользователь подтвердил локальные 140 тестов для предыдущего пакета `10cd138`.
- `01d4ea0`, `96a22a5`, `0242ca7`: `LocalFilesystem::overlaps_root(&other)` проверяет совпадение / вложенность файловых корней на Unix по `(st_dev, st_ino)` при обходе parent `..` через `openat` от закреплённых directory fd. Путь или содержимое файлов не читаются. Тесты: одинаковый корень и parent/child, независимые соседние корни, перемещение (rename) родительского каталога после открытия fd. Не выполняется автоматическое блокирование cross-root jobs; вызывающий код должен запрещать сценарии с пересекающимися корнями.
- Ограничения: при активных конкурентных mount/rename изменениях обход каталогов остаётся observational, а не транзакционным; предельная глубина 1024 вызывает fail-closed UnsafeEntry. На non-Unix метод недоступен.
- Патч требует проверки на Mac: ожидаемые 143 теста. Готовность #15 приблизительно 62%; полноценные rename/move/delete и гарантированная no-hydration работа остаются открытыми.

### Non-destructive staging: failure and unsafe-object matrix (2026-10-11)

- Пользователь подтвердил локальный прогон `f6fb0f8`: 143 теста.
- `2dfcb58`, `f98ae93`, `f36afa1`: добавлены четыре Unix-регрессионных теста для `stage_move_link_new` и `reconcile_staged_link`: сбой после успешного linkat классифицируется как `OutcomeUnknown` и не удаляет ни source, ни destination; повтор с занятым destination отклоняется; directory-source и symlink-destination не изменяются; замена inode destination обнаруживается без удаления файлов.
- Новая матрица проверяет **сохранность данных**, но не даёт гарантии атомарного rename/delete при конкурентной подмене. Не включать реальное удаление источника и автоматические повторы по итогам этих тестов.
- Новый пакет не проходил локальный fmt/clippy/tests; ожидаемый итог 147 тестов. Готовность #15 ≈62%: существенные блокеры rename/move/delete и File Provider hydration ещё открыты.

### Safe move preflight case collisions and directory guards (2026-10-11)

- Пользователь подтвердил успешный локальный прогон предыдущего патча (`f9e49f7`, 147 тестов).
- `400625a`, `33d5139`: metadata-only `preview_move_new` при отсутствующем destination проверяет через закреплённый descriptor родителя, нет ли case-insensitive alias назначения, и возвращает `AlreadyExists` вместо ложного разрешения. `revalidate_move_new` наследует защиту. Добавлены 3 Unix-теста: case alias destination/revalidate, успешная preview в существующий каталог без записи, отказ на symlink parent или отсутствующий parent.
- Это не разрешает реальный move/rename/delete, не устраняет TOCTOU после preflight и не инициирует cloud content read. Новый патч ожидает локальной macOS проверки: 150 тестов. Готовность #15 ~62%; крупные блокеры сохраняются.

### POSIX permissions metadata (2026-10-11)

- Пользователь подтвердил успешную локальную проверку предыдущего пакета `9677f8c` (150 тестов).
- `6e8e8d2`, `760b2b2`: добавлена `LocalFilesystem::mode_bits(path)`, основанная на descriptor-relative `fstatat(AT_SYMLINK_NOFOLLOW)`. Возвращает POSIX bits `0o7777` для regular files/directories, отказывает на symlink и недопустимых путях. Не открывает содержимое и не инициирует iCloud hydration.
- Эти биты информативны и не доказывают эффективный доступ: ACL, процессный UID/GID, sandbox, TCC и файловые политики проверяются непосредственно при выполнении операции. Добавлены два Unix-теста: режимы 0640/0750, отказ на symlink и traversal.
- Текущий патч ещё не проверен на Mac; ожидаются 152 теста. Готовность #15 ≈62%, безопасные rename/move/delete и полный placeholder guard остаются незавершёнными.

### Full-stage acceptance pass: macOS no-hydration contract (2026-10-11)

- По согласованию с пользователем далее проверяем #15 как единый этап, а не отправляем на Mac каждое небольшое изменение. Последний подтверждённый локальный прогон относится к предыдущему состоянию `7f3527f` (152 теста по ожидаемому счётчику; пользователь ещё не прислал вывод текущего прогона).
- `d063bb7` добавляет два macOS contract tests: оба entry point для локального копирования (`copy_file_new`, `copy_file_new_with_reserve`) возвращают `UnsafeEntry` и не создают destination как при Denied, так и при GrantedForThisOperation; metadata-only `inspect_content` не разрешает `read_to_writer`, а capabilities не обещают read_content. Это не проверка на реальном iCloud placeholder и не реализация безопасной materialization.
- До финальной приёмки #15 обязательно выполнить единый fmt/clippy/tests на macOS и отдельную ручную проверку placeholder в разрешённом тестовом контейнере, без QADREN-инициированного скачивания. Функции rename/move/delete пока небезопасны для публичного включения и должны либо получить доказуемо безопасный механизм, либо остаться явным блокером закрытия issue.

### Atomic no-overwrite rename candidate (2026-10-11)

- `30b42da`, `520f299`, `aa8bb35`: добавлен `rename_file_new` для regular file на Unix с атомарным системным no-overwrite primitive: Linux `renameat2(RENAME_NOREPLACE)`, macOS `renameatx_np(RENAME_EXCL)`. Промежуточные parent directories закрепляются descriptor-relative, source должен иметь `st_nlink == 1`, destination предварительно проходит `preview_move_new` и case collision guard, после перемещения проверяется inode; directory fds синхронизируются. При неоднозначном исходе возвращается `OutcomeUnknown`, автоматический retry запрещён.
- Контрактные Unix-тесты: успешный перенос в другой каталог, existing destination без перезаписи, symlink в назначении без изменения source/target. Не проверено в локальном Mac build. Вызов **пока экспериментальный**: race со сменой source inode между preflight и rename не устранён полностью, поэтому нельзя выдавать его за подтверждённо безопасный при враждебных конкурентных модификациях или подключать к автоматическим Jobs. Также не реализованы безопасные delete/replace и не проведена проверка реальных File Provider placeholders.
- Следующий шаг: локальная сборка полного пакета и review API/негативных race-тестов, но по договорённости не запрашиваем Mac прогон после каждого коммита; окончательное закрытие #15 требует всех контрактов и тестов.

- `968e4a3`, `f9482b7`: source name recreated after `renameat*` is classified `OutcomeUnknown`, plus adversarial tests for hardlinked source, Unicode-independent case collision and intermediate symlink outside root. Added six total Unix rename tests. The rename API is available for isolated test use, but remains **not advertised in adapter capabilities** due unresolved concurrent source-swap race; this is not acceptance of deletion or general-purpose automatic move.

- `effb541`: `reconcile_renamed_file(from,to,expected)` делает только metadata-only проверку результата ранее неоднозначного rename: ожидаемый destination inode совпадает, source name отсутствует; ничего не восстанавливает и не удаляет. Тест покрывает before/after rename и destination swap. Это observational, не транзакционная гарантия.

### Interrupted rename handling (2026-10-11)

- `43e37c6`, `0385e93`, `c4492b6`: rename syscall failures with `Interrupted` now return `OutcomeUnknown` (not retryable `Io`); `AlreadyExists` stays a known collision; other error classes continue as `Io`. Three deterministic unit tests cover these classifications without performing destructive filesystem operations.
- Safety review: `renameat2(RENAME_NOREPLACE)` / `renameatx_np(RENAME_EXCL)` atomically protects **destination no-overwrite**, but it does **not** atomically bind the source inode observed before syscall. A hostile concurrent source pathname replacement can cause rename of a different inode. Postflight reconciliation detects many such outcomes only **after** mutation. Therefore keep this API out of automatic Jobs/capabilities until a stronger trust/concurrency model or source-binding design is agreed and tested. macOS placeholder policy remains fail closed for content reads.
- Full-stage local validation still outstanding, by agreement do not ask for per-commit testing. No change to percentage of issue #15.

- `6208400`: non-Unix local adapter no longer declares `Windows` as supported nor marks insecure path-based metadata listing as verified; `list_metadata` and prerequisites now require Unix. Existing non-Unix fallback is not a production-safe file adapter and should be replaced by OS-specific secure implementation before enabling Windows.
- `2dbedec`: rename compares second `fstatat` source `(dev,ino,size)` with initial identity and requires `nlink==1`. This narrows the pre-rename race window but does not close it; unresolved source-binding limitation still forbids automatic destructive jobs.
- Next gate: verify platform-specific `libc` symbols and fmt/clippy/tests on Mac as part of the agreed consolidated review, then decide safe source-binding and deletion policy before declaring #15 accepted.

### Подтверждённый локальный пакет и следующий инкремент (2026-10-11)

- Пользователь подтвердил успешный `scripts/test-core.sh` после форматирования `91f3a2a`. Это подтверждает fmt/clippy/tests для данного состояния, но не закрывает вопросы сохранности при конкурентных файловых мутациях.
- `12415b1`: metadata-only `preview_move_identity(from,to)` возвращает наблюдаемую идентичность исходного regular file после проверки назначения, не меняя файлов. Тест проверяет отсутствие изменений на диске. Это вспомогательный контракт для будущего reconciliation, не реализация безопасного удаления или move; проверку нового патча включить в следующую объединённую приёмку, не запрашивая отдельный запуск.

### Regression baseline after macOS validation (2026-10-11)

- Пользователь подтвердил успешный Mac test-core.sh на `3772973` (включая предыдущие изменения); точное число тестов в сообщении не приведено.
- `993b81a`: два Unix negative tests — rename со source symlink должен отказать без модификации target, а preview с исходным inode, заменённым до revalidate, должен отказать как UnsafeEntry. Эти тесты ещё не запускались на Mac.
- До полного DoD #15 сохраняется критический пробел: возможность подмены source pathname **после** проверки и **до** самого системного rename, а также отсутствие безопасного delete, iCloud File Provider proof и финальной общей приёмки. Пока нет достаточных оснований отмечать пункт файловых перемещений завершённым.

### Checked rename preview binding (2026-10-11)

- После подтверждённого пользователем Mac-прогона `3772973` продолжается единый этап разработки. Новые тесты `993b81a` и `d41282c` ещё не проверены.
- `d41282c`: `rename_file_new_checked(from,to,expected)` повторно проверяет source identity и destination preflight перед no-replace rename; негативный тест со сменой inode до вызова не перемещает подменённый файл; позитивный тест проверяет одноразовый перенос. Это сужает окно устаревшего плана, но **не исключает гонку после последней проверки**. Нельзя использовать для враждебных конкурентных директорий или рекламировать как завершённый `move/delete`.
- Итоговый критерий безопасности: операции, которые потенциально удаляют исходный путь, включаются в автоматические Jobs только после доказанной гарантии source binding либо документированной и проверенной модели эксклюзивного владения каталогом. Пока нельзя отметать no-hydration обязательства на Mac.

### Capability gate for destructive operations (2026-10-11)

- `133dc04`: добавлен contract test, подтверждающий, что `LocalFilesystem::capabilities()` не рекламирует Rename/Delete и `CapabilityDeclaration::allows` возвращает `UnsupportedOperation` для обоих на Unix. Это защита слоя планирования от преждевременного использования experimental rename, а не устранение source pathname race.
- Последняя подтверждённая пользователем macOS проверка была до `993b81a`; объединённые изменения после неё ещё не прошли fmt/clippy/test на Mac. В рамках согласованного процесса не требуем отдельного тестового прогона сейчас.
- Блокер закрытия этапа: обеспечить безопасные удаления/перемещения при допускаемых конкурентных мутациях или согласовать ограниченную модель контролируемой директории, плюс File Provider/macOS hydration policy и полная приёмка.

### macOS dataless mutation guard (2026-10-11)

- `da3474b`, `90a6ff3`: на macOS операции `rename_file_noreplace` и `link_file_new_with_post_link_check` отклоняют `SF_DATALESS` source до изменения namespace. Добавлен unit-тест флага. Результат компиляции и тестирования ещё не подтверждён.
- Отсутствие флага `SF_DATALESS` не доказывает, что File Provider item полностью локальный; metadata guard не выдаёт право на чтение. Все content reads на macOS остаются запрещены. Rename не публикуется в capabilities из-за source pathname race. Полный DoD #15 ещё открыт.

### Конкурентные изменения: fail-closed delete (2026-10-11)

- Принятое в #15 решение: QADREN должен быть безопасным даже при конкурирующих изменениях файлов сторонними приложениями.
- `c293901`: добавлен явный `delete_file(path)`, который проверяет границы пути и отклоняет удаление как `UnsafeEntry`. Два негативных теста проверяют сохранность исходника и отказ при traversal. Метод намеренно ничего не удаляет: `fstatat` + `unlinkat` не дают атомарного условия «удалить только inode, который проверен».
- Не считать это реализацией delete в DoD. `move_file_new` также остаётся fail-closed; `rename_file_new*` нельзя использовать в автоматических заданиях. Следующий шаг: безопасная модель без удаления исходника и проверка всей цепочки в macOS, затем отдельная архитектурная реализация source-bound destructive operations или задокументированная недоступность.
- Эти два новых теста не проверены в локальном Mac прогоне; не запрашивать отдельную проверку до общей приёмки.

### Non-destructive staged copy under concurrent writers (2026-10-11)

- `df89eda`, `b2f50d4`, `cbb51ca`: добавлен `prepare_move_copy_new(from,to,expected,consent)`, который на Linux сверяет прежнюю metadata-only инспекцию источника, проверяет свободное назначение, копирует байты через no-overwrite writer, повторно сверяет исходник и **никогда не удаляет source**. Если исходник изменился после публикации, возвращает `OutcomeUnknown`, так как копия могла уже появиться.
- Тесты проверяют успешную публикацию с сохранением исходника, отказ от второго копирования в занятое назначение, отказ при устаревшей инспекции и отсутствии согласия, запрет чтения/копирования на macOS даже с согласием. Это только Linux preparation, **не полноценный move**; не добавлять rename/delete в capabilities.
- Для source pathname, изменённого сторонним writer в ходе копирования, проверки являются best-effort; обязательный postflight `OutcomeUnknown` не означает доказанную консистентность. Новые изменения ждут общей macOS проверки.

### Linux move-copy: checked opened source inode (2026-10-11)

- Пользователь подтвердил успешный локальный fmt/clippy/tests после `5c62e0d`.
- `3b55848`: `prepare_move_copy_new` больше не открывает source повторно внутри `copy_file_new` после отдельного pathname preflight. Теперь источник сначала открывается через pinned root/no-follow, непосредственно открытый fd сверяется по `(dev,ino,size,mtime,ctime)` с `LocalContentInspection` **до** создания destination; затем тот же открытый дескриптор передаётся в no-overwrite writer. После копирования проверяются метаданные того же fd и исходное имя. При непроверяемом результате после публикации возвращается `OutcomeUnknown`; исходник никогда не удаляется.
- Это устраняет окно замены между инспекцией и **открытием фактического input** для незаметного копирования не того inode, но не гарантирует snapshot байтов при конкурентных записях в открытый inode. Mac content reads по-прежнему fail-closed. Патч не проверен локальным компилятором; выполнить в общей приёмке.

### Exact-length move-copy publication (2026-10-11)

- Пользователь подтвердил успешный Mac-прогон после `a7ad156`.
- `a937c72`: в Linux `prepare_move_copy_new` входной descriptor ограничен `ExactSizeReader` на зафиксированную `before.len()` длину. При сокращении или росте потока временная запись должна отказать **до** публикации назначения; после публикации остаётся отдельная сверка того же открытого fd и pathname. Исходник не удаляется. Пакет требует следующей объединённой локальной проверки.
- Это не гарантия неизменного снимка байтов при конкурентной перезаписи такой же длины и не завершение move/delete. Безопасность Mac cloud content остаётся fail-closed.

### Linux source verification before copy publication (2026-10-11)

- Пользователь подтвердил Mac-прогон после пакета `b30071b`.
- `24d38d0`, `b0e7ecc`: `VerifiedCopyReader` на Linux проверяет открытый descriptor в точке EOF, прежде чем no-overwrite writer публикует готовый temporary file. Если inode поменял размер/mtime/ctime во время чтения, `create_from_reader` возвращает ошибку и не публикует destination. Добавлен тест same-size изменения источника перед EOF. После публикации остаётся отдельная проверка источника и его pathname с `OutcomeUnknown` при расхождении.
- Проверка timestamp не доказывает атомарный snapshot байтов при конкурентной записи; исходник никогда не удаляется. На macOS content reads запрещены. Новый код ожидает компиляции и локального теста, не считать #15 закрытым.

### Hardlink source guard for move-copy preparation (2026-10-11)

- Пользователь подтвердил успешный локальный прогон предыдущего пакета `2bc6485`.
- `55c4955`: Linux move-copy preparation теперь отказывает при `nlink != 1` у уже открытого source inode перед публикацией destination. Дополнительный тест подтверждает, что для hard-linked source новый файл не создаётся, а alias и оригинал сохраняются. Это согласовано с более строгими требованиями безопасного перемещения, но не распространяется на обычное самостоятельное копирование.
- Изменение ещё не прошло macOS fmt/clippy/tests; новую проверку запрашивать вместе с общим пакетом, не после этого коммита. Полное delete/move остаётся закрытым для автоматических Jobs.

### Move-copy publication regression (2026-10-11)

- Пользователь подтвердил предыдущий Mac-прогон до текущего набора изменений.
- `e1beef4` добавляет Linux end-to-end тест: уже открытый source изменён после исходного `metadata`, а `VerifiedCopyReader` должен сорвать атомарную публикацию destination. Если source изменён, оригинал не удаляется.
- Коррекция `6f233ac` убирает ненужный явный `drop` reader-а и полагается на стандартную NLL-проверку областей заимствования Rust. Нужна компиляция и тестирование на Mac на объединённом этапе; нет основания объявлять #15 завершённым.

### Reconciliation boundary: copy versus byte equality (2026-10-11)

- `c71bab1`: metadata-only `reconcile_move_copy` inspects a preserved source against the original inspection and checks that a destination of the expected length exists. It does **not** claim integrity, equality of content, recovery, completed move, or authorization to remove either file.
- Unix negative contract test deliberately uses same-sized but different bytes: metadata reconciliation may return true, illustrating why caller MUST NOT mistake its result for verified copy integrity. A missing destination returns false. This interface is observational only.
- To mark prepared copy as verified, execution needs a trustworthy content digest/producer-verified source and destination, alongside stable source identity and no forbidden macOS hydration. No automatic destructive step is enabled. Consolidated Mac tests remain pending for new code.

### End-to-end prepared copy byte verification (2026-10-11)

- `c71bab1`, `bcbaa41`: metadata-only copy reconciliation проверяет наличие двух имён и размер, **не** целостность; отрицательный контракт демонстрирует одинаковую длину при разных байтах.
- `67c4a9c`, `a4ab00a`: отдельный Linux API `verify_move_copy_bytes` по явному разрешению открывает исходник и назначение через pinned root + no-follow, сравнивает содержимое потоково блоками 64 KiB и перепроверяет оба открытых inode по metadata до/после. При изменении обнаруженных metadata или исходного pathname возвращает ошибку, а не мнимый успех. Linux-тест различает equal-size corruption от точного совпадения, macOS-тест требует fail-closed даже с consent.
- При враждебной записи в открытый файл сравнение + timestamps не обеспечивает транзакционный байтовый snapshot; автоматический delete/move остаётся недоступен. Эти новые изменения не проверены Mac toolchain. Прогресс оценивать по готовым функциям, не количеству тестов.

### Release 1 Stage 04 start: Yandex streaming upload (2026-10-11)

- По просьбе ускорить проект начата независимая работа #16 без фиктивного закрытия #15. Это параллельное начало, а не снятие блокирующих гарантий безопасности.
- `a7e874d`, `8738354`, `020c72b`, `e69891f`: реализован минимальный путь upload: GET подписанного ресурса `/resources/upload?overwrite=false`, проверка HTTPS/PUT/отсутствия URL credentials, потоковый HTTP PUT фиксированной заявленной длины без открытия файлов на Mac, отдельный `UploadOutcome::Accepted/Pending`; запрещены HTTP redirects в производственном клиенте. Ошибка PUT считается возможным неизвестным состоянием удалённого файла и требует внешнего reconciliation; метод не выполняет автоматических retries.
- Upload reader теперь проверяет точное соответствие фактической длины durable `expected_size`: short source и trailing bytes обнаруживаются до передачи последнего chunk (для нулевого expected size — до PUT). Несовпадение не объявляется успешной копией: результат остаётся `UploadOutcomeUnknown` и требует reconciliation. Это не вычисляет MD5 source и не заменяет remote metadata verification.

### Yandex upload metadata reconciliation contract — #16 (2026-10-13)

- `reconcile_uploaded_resource` делает только `GET /resources` по точному ожидаемому path и сопоставляет path, size и 32-hex provider MD5. Некорректный ожидаемый MD5 останавливает проверку как `InsufficientEvidence` ещё до сети; неполный или иной ответ провайдера не становится успехом.
- Metadata-only browser получает следующие страницы только по явному действию пользователя: Rust/FFI передаёт `offset` при фиксированном limit 50, SwiftUI показывает «Показать ещё» только когда provider сообщил ещё элементы. Проверка loopback HTTP фиксирует GET `/resources` с path/limit/offset и без content transfer. Отсутствие или противоречивые pagination fields не открывают бесконечную загрузку.
- Наблюдение existing durable `copy` attempt переиспользует этот contract после проверки зарегистрированного `connection_id`, Keychain credential reference и provider account UID. Оно остаётся read-only: не повторяет PUT, не читает/скачивает контент, не изменяет Job/evidence/baseline и не подключено к FFI/UI. Mock HTTP test доказывает read-only GET и match metadata; это не live Yandex E2E и не подтверждение реальной передачи. #16 остаётся PARTIAL.
- Для уже `UNKNOWN` copy attempt добавлен отдельный recovery boundary: после той же account-bound read-only проверки только точное совпадение path/size/MD5 атомарно завершает attempt и Job. Mismatch, неполная metadata, account/credential error и provider failure оставляют Job `UNKNOWN`; PUT, download, retry, Registry/FileSyncState mutation отсутствуют. Этот путь пока не подключён к UI/FFI и не является live Yandex E2E.
- Сквозные mock tests фиксируют порядок account UID → resource metadata и доказывают, что даже точное совпадение не меняет `running` Job и не resolves attempt; несовпадающий account UID останавливает путь до lookup ресурса. Токен остаётся только в closure test Keychain resolver и не записывается в Registry/JobStore.
- Тест с mock API подтверждает отказ от передачи на HTTP URL до чтения входа. НЕТ end-to-end проверки реальным аккаунтом, проверки полного размера/MD5 после загрузки, честного `OutcomeUnknown` в типе ошибки, интеграции со сценариями/очередью, безопасного контроля отмены и retries/backoff; пока не включать автоматические Jobs или рекламировать production-ready. Новый код ещё не прошёл локальный fmt/clippy/tests.


## Gate-focused acceptance matrix — 2026-10-11

Исторические записи выше относятся к указанным там SHA и не подтверждают нынешний HEAD. Каждому утверждению о прохождении тестов обязателен фактический SHA и среда.

| Gate | Готово тогда и только тогда, когда | Подтверждение на этапе подготовки |
|---|---|---|
| G0 | Документированы владельцы Job, scope consent, adapter errors/capabilities, FFI и UNKNOWN reconciliation | Документировано, реализации ещё требуют проверки |
| G1 | Manual create-new проходит SwiftUI → Rust JobStore → mock adapter → verified SQLite receipt → UI; no-overwrite, restart/duplicate/fault tests | PARTIAL: mock vertical slice, durable queue/cancel before effect, Rust/mock/ABI tests and macOS smoke/restart passed; real adapter, pause/resume, fair scheduling and limits remain |
| G2 | Та же вертикаль работает на реально подтверждённой паре адаптеров; live provider evidence и consent | НЕ выполнено |
| G3 | One-way и two-way Scenario diff опираются на последнее подтверждённое состояние; конфликт блокируется | НЕ выполнено |
| G4 | Mass delete/overwrite, concurrent mutation, Keychain disconnect и uncertain retry fail-closed | Частично, полная приёмка НЕ выполнена |
| G5 | Единый Rust engine обслуживает Job в независимой службе; закрытие UI не прекращает выполнение | НЕ выполнено |
| G6 | Матрица платформы, entitlement, выбранная distribution, signed install, ресурсы и fault tests зафиксированы | НЕ выполнено |

Для каждого инкремента приложить тестовый case ID, SHA, environment, executed command, PASS/FAIL/BLOCKED/SKIPPED, источник evidence (real/mock/static) и оставшийся риск. Закрытие #15 по количеству unit tests запрещено. Рекомендуемый первый E2E: no-overwrite create-new-file, stable attempt ID и verification перед terminal success. Исходный контент нельзя автоматически скачивать на главный MacBook. Перед выпуском с неполными destructive capabilities запросить отдельное продуктовое согласование.


### G1 in-process job inventory — 2026-10-11

Срез исходного кода: `a39c392` (`JobStore::list_jobs`), `519810a`/`c581b9b` (read-only C ABI `qadren_jobs_list`), `58cd095` (Swift `JobInventory`), `33fc1f8` (read-only queue panel in `MainView`). Статусы считываются непосредственно из durable SQLite; никакой Job не запускается при открытии UI. Для первого runner сохраняется отдельная архитектурная граница, без придуманной worker службы.

**Проверки:** статический анализ путей по исходному коду. Rust fmt/Clippy/tests, Swift compiler, native macOS smoke и реальное восстановление после restart на этих коммитах здесь НЕ выполнялись. Код не считается проверенным, пока нет подтверждённого запуска. Этот промежуточный этап **не** удовлетворяет G1: отсутствуют manual Job executor, no-overwrite adapter effect, production verifier, согласованная фиксация результата и end-to-end test.

**Перед следующим функциональным коммитом:** запустить `bash scripts/test-core.sh` и `bash scripts/build-macos.sh` на Mac. При неуспехе сначала исправить компиляцию, не расширять функциональность. Следующее изменение должно быть связано с безопасным исполнением, не наращивать read-only интерфейс без необходимости.

### G1 mock create execution package — 2026-10-11

`118cdf2` добавил единственный in-process vertical slice: SwiftUI через узкий C ABI создаёт новый **app-owned** mock artifact; `job_execution.rs` сохраняет `JobStore` Job и bound attempt с размером/MD5 до записи, выполняет `create_new` без overwrite, затем mock verifier заново читает только этот тестовый объект и сохраняет metadata-verified receipt в SQLite. Swift не пишет SQLite и не выполняет файловую бизнес-логику.

Проверено на macOS: `bash scripts/test-core.sh` — Rust fmt, Clippy и 188 unit tests passed; в том числе consent-denial без эффекта, occupied destination без overwrite, invalid path, partial write, duplicate request, simulated crash after external effect → restart `UNKNOWN` with no replay, а также successful C-ABI execution. `bash scripts/build-macos.sh` собрал unsigned `dist/QADREN.app` и проверил `Info.plist`.

`118c1f9` расширяет mock slice ровно одной безопасной операцией управления: `Queue mock file` сохраняет только local SQLite Job; `Run` является отдельным operation-scoped согласием на создание test artifact; `Cancel` разрешён только если нет active/unknown bound attempt. Введён terminal state `cancelled` и migration v6→v7, которая сохраняет Jobs/events/attempts/evidence. Active или UNKNOWN attempt отменить нельзя — возможный эффект по-прежнему требует reconciliation, не получает ложный статус `cancelled`.

Следующий пакет добавляет durable `Pause`/`Resume` для Job **до** внешнего эффекта: `Pause` переводит queued Job в `paused`, `Resume` возвращает его в `queued`, а отдельный `Run` вновь требует operation-scoped consent. Пауза при active/unknown bound attempt сознательно отклоняется: checkpoint для потоковой передачи ещё не реализован, поэтому система не выдаёт остановку возможного эффекта за проверенный факт. Состояние доступно через узкие Rust C ABI команды и отображается SwiftUI без прямой записи в SQLite.

Проверено на macOS для этого пакета: `bash scripts/test-core.sh` — PASS (fmt, Clippy, 192 Rust unit tests); `bash scripts/build-macos.sh` — PASS (unsigned `dist/QADREN.app`, `Info.plist`); UI smoke — PASS: queued mock Job появился в UI, был отменён до effect, после закрытия и повторного запуска UI сохранил `cancelled`. Это **mock evidence**, не G2 и не реальная передача Yandex/iCloud/Local FS: не читается пользовательское или облачное содержимое, не добавлены destructive operations, Background Worker, Scenario, pause/resume, fair scheduling, limits или реальный provider reconciliation. G1 и #18 остаются PARTIAL до этих отдельных границ и final review на SHA.

Добавлена отдельная read-only reconciliation для уже durable `UNKNOWN` G1 mock attempt: Rust сверяет ровно сохранённые `operation_kind`, `connection_id`, target, size и MD5 с app-owned test artifact, лишь после точного совпадения разрешает тот же attempt и переводит Job в `completed`. Несовпадение, отсутствие root/object или неоднозначная binding остаются `UNKNOWN`; повторная запись, удаление или автоматический rollback отсутствуют. Этот узкий mock recovery не является reconciliation для Yandex, Local FS или iCloud и не снимает оставшиеся G1/#18 ограничения.

Проверено на macOS для этого изменения: `bash scripts/test-core.sh` — PASS (fmt, Clippy, 212 Rust unit tests, включая crash-after-effect → exact reconciliation и partial artifact → сохранение `UNKNOWN`); `bash scripts/build-macos.sh` — PASS (unsigned `dist/QADREN.app`, `Info.plist`).

`JobInventoryTests.swift` дополняет этот mock-slice native Swift→Rust проверкой на временном app-owned root: пустой inventory, Queue → Pause → Resume → Run → durable `completed`, затем отдельный Queue → Cancel без artifact. Команда `bash scripts/test-macos-job-inventory.sh` собирает release Rust static library и этот runner; она не запускает QADREN.app, не использует Application Support пользователя и не читает пользовательское/облачное содержимое. Это ABI/bridge evidence, а не visual UI smoke или реальная передача данных.

Следующий G1 safety-пакет различает известный отказ до эффекта и неоднозначный результат: если app-owned `create_new` атомарно сообщает, что target уже занят, Rust в одной SQLite-транзакции переводит bound active attempt в `failed_before_effect`, а Job — в `failed`. Receipt и успешная verification при этом не фабрикуются; существующие bytes остаются нетронутыми. Partial write, обычная I/O-ошибка, crash после возможного эффекта и неудачная verification по-прежнему становятся `UNKNOWN` после reopen и требуют только read-only reconciliation. Миграция schema v9 сохраняет legacy active/unknown/resolved attempts; только active/unknown продолжают блокировать completion, cancel, pause, decisions и удаление credentials.

Успешное завершение verified attempt теперь также является единой SQLite-транзакцией: `resolved` attempt, evidence level, `completed` Job и оба event записываются вместе. Поэтому crash не может оставить resolved attempt под `running`/`UNKNOWN` Job, который уже нельзя безопасно повторить или reconciliate. Этот общий Rust boundary используют как normal mock completion, так и его read-only recovery; он не добавляет реальный provider transfer или меняет политику согласия.

### Настройки: подключения и Scenario configuration — 2026-10-12

«Настройки» остаются отдельным окном и используют реальные секции: независимые `StorageConnection`, сохранённые Scenario и «Настройки приложения». SwiftUI вызывает существующий Rust Registry FFI для list/create/enable/update Scenario; форма сохраняет имя, два connection ID, каталоги, направление и ревизию 1. Изменение конфигурации транзакционно увеличивает revision и сохраняет отдельный immutable snapshot обеих ревизий, не меняя включённость Scenario; закреплённый Job в будущем сможет взять именно свою конфигурацию, а не текущую строку. Включение/выключение также меняет durable Registry. «Настройки приложения» содержат локальную тему в том же левом меню и не смешивают её с настройками синхронизации. В соответствии с D-017 iCloud отсутствует из активных пунктов интерфейса.

Создание Scenario — редактор в detail-зоне, не sheet. В его верхней части находятся имя, две точки обмена и направление; ниже — два metadata-only браузера каталогов. «Этот Mac» доступен как реальное `local_filesystem` подключение: при первом сохранении создаётся независимый Registry record, затем он переиспользуется. Яндекс Диск загружает только первую страницу метаданных выбранного каталога (не байты файлов); запуск сценария и передача файлов по-прежнему отсутствуют.

Граница: это configuration-only UI. Есть создание, включение и revision-safe изменение Scenario; нет удаления Scenario, scheduler, dry-run запуска, Job submission или передачи файлов. Экран явно сообщает об этих ограничениях и не имитирует Run/копирование. Удаление connection по-прежнему использует существующий recoverable Keychain/attempt gate.

Проверено на macOS для этого изменения: `bash scripts/test-core.sh` — PASS (fmt, Clippy, 220 Rust unit tests, включая C ABI create/list/enable/update Scenario и миграцию v3 с сохранением snapshot); `bash scripts/build-macos.sh` — PASS (unsigned `dist/QADREN.app`, `Info.plist`). Native visual/VoiceOver smoke для этой версии не подтверждён: session UI automation стала недоступна после перезапуска приложения.

### Файловый менеджер: read-only metadata slice — #22 (2026-10-13)

«Файловый менеджер» открыт отдельным macOS-окном из меню `Пространство` и Menu Bar, поэтому не возвращает постоянный Sidebar на мониторинговую панель. В окне две независимые локальные панели с выбором корня, breadcrumb-навигацией и выделением строк; они читают только имена, типы и размеры через существующий Local Filesystem FFI.

Граница: это browsing-only срез. Предпросмотр и загрузка содержимого, создание каталогов, копирование, перемещение, переименование и удаление отключены: прямой Swift вызов `qadren_local_mkdir` удалён. Никакой Job, Scenario, consent или файловый эффект не создаётся открытием/обновлением окна. Операции изменения будут добавляться только вместе с безопасным Job Engine и его durable attempt/reconciliation контрактом.

Для этого среза выполнены `bash scripts/test-core.sh` и `bash scripts/build-macos.sh`; native visual/VoiceOver smoke отдельно не выполнялся.

### Monitoring layout: per-window local composition — #23 (2026-10-14)

- Monitoring сохраняет локальный порядок и видимость четырёх существующих виджетов (`Система`, `Задания`, `Очередь`, `Подключения`) отдельно для каждого SwiftUI scene через `@SceneStorage` panel ID и `UserDefaults`. В режиме «Компоновка» можно переставить или скрыть виджет; временный каталог возвращает только скрытые стандартные виджеты, а сброс восстанавливает baseline. Постоянного Sidebar, произвольных HTML-виджетов и второго data/Job слоя нет.
- В режиме «Компоновка» доступны native toolbar actions Undo/Redo (`Cmd-Z` / `Shift-Cmd-Z`) для последней серии локальных изменений. История ограничена 50 состояниями, не переживает перезапуск окна и не затрагивает данные Core; каждый undo/redo сохраняет текущий presentation layout так же, как исходное изменение.
- Для каждого виджета отдельно сохраняется плотность `Компактно` / `Обычно` / `Подробно`; это меняет только вторичную информацию и объём реально доступных строк, не масштабирует весь интерфейс и не создаёт показатели. Отсутствующее поле в layout, сохранённом предыдущей версией, безопасно читается как `Обычно`.
- В том же режиме отдельно для каждой панели доступен размер адаптивной сетки `Компактная` / `Обычная` / `Просторная`: он меняет только порог deterministic reflow (`230` / `270` / `360` pt), не масштабирует chrome или содержимое виджетов. Это presentation-only поле участвует в Undo/Redo; layout старого формата-массива по-прежнему открывается с обычной сеткой.
- Сохранены только presentation fields. Layout не меняет Job, Scenario, StorageConnection, Worker или файлы. Повреждённый сохранённый JSON открывается с безопасным default layout, при этом исходная запись не удаляется автоматически.
- `bash scripts/test-macos-dashboard-layout.sh` проверяет persistence порядка/visibility, независимость двух panel ID и fail-safe recovery повреждённого layout. Native app был открыт для inspection monitoring surface; последующее UI automation connection прервалось, поэтому visual/VoiceOver acceptance для нового layout остаётся BLOCKED и не заявляется как PASS.

### Расписания Scenario: durable configuration UI — #21 (2026-10-13)

Для существующего Scenario в отдельной панели настроек можно создать несколько weekly расписаний, просмотреть их IANA timezone, временно выключить каждое отдельно или удалить. Конфигурация хранится в существующем Rust Registry; SwiftUI обращается к ней только через новые узкие C ABI list/create/enable/remove. Операция удаления расписания в одной SQLite-транзакции сначала удаляет его receipts уже оценённых слотов, затем саму конфигурацию, и не затрагивает Scenario, Job или файлы. Выключение сохраняет receipts, поэтому повторное включение не должно воспроизвести старый слот.

Граница: интерфейс прямо сообщает, что scheduler ещё не создаёт Job. Нет `next run`, фонового таймера, content I/O, Scenario execution, consent или передачи файлов. Native visual/VoiceOver smoke для этого SHA не выполнен; статические Core/FFI и macOS build проверки перечислены в evidence Issue #21.

### Подтверждённый FileSyncState — #19 (2026-10-13)

Registry schema v8 хранит FileSyncState отдельно от строки Scenario: ключ включает Scenario ID, immutable revision и относительный путь; в записи остаются только metadata identity/size/MD5 каждой стороны, без контента и секретов. Запись допускается только при `MetadataVerified` либо более сильном evidence; `Requested`, provider acknowledgement и `UNKNOWN` fail-closed и baseline не продвигают. Новая revision начинает без унаследованного baseline, поэтому её dry-run не может молча использовать состояние другой конфигурации.

Этот пакет не делает adapter scan, не вызывает planner через FFI/UI, не создаёт Job и не подтверждает реальную синхронизацию. Он создаёт только durable contract, который будущий executor обязан вызывать после независимой проверки результата.
