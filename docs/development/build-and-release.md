> **D-018 / 2026-10-14:** локальный релиз P0–P2 строится и тестируется без контроллера Яндекс Диска, живого API, новых OAuth и iCloud entitlements. Yandex feature-work/live E2E вынесены за Release 1, iCloud — бессрочно. Ниже старый G1/G2 cloud sequence исторический: локальный Job→Local FS и safety проверяются раньше. Подпись/распространение относятся к **локальному** релизу и не требуют облачного сервиса. [D-018](../decisions/D-018-local-first-priorities.md).

# Сборка и выпуск

Статус: developer build scripts реализованы, выпускной конвейер и модель распространения ещё не утверждены.

## Архитектура поставки

- На macOS итог — `QADREN.app`, самодостаточный нативный пакет.
- Rust-компоненты компилируются в машинный код и **поставляются внутри приложения** (библиотека или исполняемый компонент — решить по прототипу).
- SwiftUI и Apple APIs используют штатные framework macOS.
- Никакой отдельной установки Rust/Swift/Cargo/Xcode/Node/Python/Docker на компьютер конечного пользователя.
- Для работы облаков потребуются учётные записи, сеть, системные разрешения и подходящая версия ОС.

## Разработка

Локальная сборка: `bash scripts/test-core.sh` для Rust fmt/Clippy/tests; `bash scripts/build-macos.sh` для unsigned macOS developer bundle `dist/QADREN.app`. Команды не заменяют реальный E2E. Локальная сборка требует Rust toolchain и Xcode на Mac разработчика. Сборку Rust и Swift держать воспроизводимой: зафиксировать toolchain в манифестах после выбора версий, использовать lockfile и минимальный build script. Не загружать бинарники зависимостей из непроверенных источников в release pipeline.

## Релизный конвейер (проектируется)

1. Проверить tests, lint, контроль лицензий и зависимостей.
2. Собрать оптимизированную release-конфигурацию, минимизировать символы отладки только в пользовательском артефакте.
3. Сформировать .app с корректными подписыванием и sandbox/entitlements, только если они нужны выбранной модели распространения и реальным функциям Яндекс Диска/Local FS; iCloud entitlements не требуются.
4. Проверить корректную загрузку Rust-компонента на чистой поддерживаемой macOS.
5. Подписать Developer ID и выполнить notarization (для распространения вне Mac App Store); проверить Gatekeeper.
6. Проверить установку, запуск, обновление и удаление без сторонних runtime.

Выбор распространения через App Store или напрямую **ещё не принят**. App Sandbox, entitlements и background execution зависят от этого выбора и должны быть проверены отдельно.

## Другие ОС

В будущем общее Rust-ядро компилируется под Windows/Linux. Платформенный UI и секреты — отдельные модули. iCloud Documents adapter отложен (D-017) и не входит в release criteria для других ОС. Цель — нативный пакет без стороннего runtime, но системные библиотеки и требования к версии ОС допустимы.


## Сквозной integration runbook (2026-10-11)

1. Сохранить SHA `main`, версии macOS/Xcode/Swift/rustc и test fixture; не использовать пользовательские файлы.
2. `bash scripts/test-core.sh`: fmt, Clippy, Rust tests. На Mac также `bash scripts/build-macos.sh`, проверить `dist/QADREN.app`; это unsigned developer bundle, не релиз.
3. Для G1 подготовить тестовый исходный поток и *отсутствующий* целевой путь. Submit idempotent manual Job через тот же Rust JobStore, независимо от frontend. Никакого Scenario для разовой операции.
4. Перед I/O проверить connection/capability/consent и durable attempt. Провести no-overwrite write, metadata verification, сохранить receipt в SQLite, отобразить его в SwiftUI. Отличать adapter ACK от cloud integrity evidence.
5. Fault matrix: отсутствует consent (нулевой content I/O), existing destination (не меняется), short/long stream, adapter 401/429/5xx, timeout после возможной записи, restart после external effect до DB commit, duplicate attempt, cancel/pause, отключение connection. UNKNOWN не retry и не success без read-only reconciliation.
6. Сверить дерево файлов, object metadata и строки SQLite до/после; убедиться в отсутствии секретов и скачиваний на основной MacBook без операции-specific consent.
7. Для G2 заменить mock реальными поддерживаемыми адаптерами; выделить реальные проверки macOS Keychain, Яндекс-аккаунта и разрешений Local FS отдельно от mock tests; не проверять iCloud entitlement. Результаты записать в `release-1-validation.md` с SHA, командами и непройденными сценариями.
8. Провести code/security review и только затем отметить соответствующую клетку acceptance и Issue. Если шаг не воспроизводится, указать BLOCKED, не Done.

**Рабочее решение по архитектуре:** первый runner работает in-process SwiftUI → Rust через узкий FFI. В release gate остаётся независимое выполнение после закрытия UI: после G1/G2 тот же Rust engine переносится в background service без новых JobStore или конкурирующих владельцев. Выбор Mac App Store/прямой signed distribution открыт до macOS signing/sandbox/service feasibility проверки, независимо от iCloud. GitHub Actions необязательны.
