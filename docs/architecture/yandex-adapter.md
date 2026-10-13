# Яндекс Диск: metadata и ограниченный upload-контракт

Статус: metadata/OAuth интеграция и ограниченный streaming upload-контракт существуют, но полноценная передача и live E2E не приняты. Issue #16 (частично).

## Основания
Официальные документы:
- REST API: https://yandex.com/dev/disk/rest/
- OAuth authorization code + PKCE: https://yandex.ru/dev/id/doc/ru/codes/code-url
- Регистрация приложения: https://yandex.ru/dev/id/doc/en/register-client

## Выбранная модель
- OAuth Authorization Code с PKCE S256, системный браузер и защита параметром state.
- Для native public client **не хранить client_secret в приложении**; Яндекс документирует возможность PKCE без секрета.
- Client ID / redirect URI необходимо зарегистрировать и подтвердить до полноценного OAuth.
- Токен хранится в Keychain; Rust-клиент получает его только на время API-операции из платформенного слоя. Ни в Git, ни в SQLite, ни в логах токен хранить нельзя.
- Первоначально запрашивать права чтения, точный scope проверить в кабинете Яндекс OAuth перед добавлением авторизации.

## Уже реализовано
- `YandexMetadataClient` на Rust через HTTPS, 15-секундный timeout.
- `GET /v1/disk/` (квота) и `GET /v1/disk/resources` (пагинация, метаданные).
- `StorageAdapter::capabilities()`: metadata=true, read_content=false, write_content=false.
- Явное различение ошибок 401/403/404/429 и остальных неуспешных HTTP ответов.
- Для нового удалённого объекта есть изолированный `upload_from_reader`: сначала получает upload URL с `overwrite=false`, принимает только HTTPS `PUT` без URL credentials и не следует redirect. Он не открывает локальный файл сам, не выполняет retry и различает provider `Accepted`/`Pending` от подтверждённого результата.
- `reconcile_uploaded_resource` повторно читает только metadata указанного ресурса и сопоставляет точные path, size и валидный provider MD5. Неполные/некорректные ожидания дают `InsufficientEvidence`; метод ничего не записывает, не повторяет upload и не меняет Job.
- Нет download, delete, move, rename, copy или готового Job execution path.
- Вспомогательные unit tests с JSON фикстурами, без реального сетевого запроса.

## Ограничения и этапы проверки
- Upload не подключён к JobStore/FFI/SwiftUI и не является разрешением запускать передачу. До него требуются durable bound attempt, source/destination consent и capability checks.
- Нет download: на основном MacBook он остаётся запрещён без отдельного согласия конкретной операции.
- Token refresh/повторная авторизация и реальные upload/reconciliation надо проверить на тестовом account. API-ответы и metadata нельзя выдавать за live integrity evidence, пока не пройдена такая проверка.


## Проверяемый UI: квота и корень Диска

В PR #4 добавлены C ABI функции для **только читающих** запросов Rust:
- `qadren_yandex_disk_info`: `GET /v1/disk/` — объём.
- `qadren_yandex_list_root`: `GET /v1/disk/resources?path=disk:/&limit=50&offset=0` — первые 50 записей корня.
- `qadren_yandex_free_string` освобождает JSON-ответ, полученный от Rust.

Swift передаёт токен из Keychain в Rust **в памяти**, запускает блокирующий запрос не в UI-потоке и отображает общий/занятый/свободный объём и список имён. Ответы API и секреты не логируются. Ошибки передаются как безопасные коды, например `unauthorized`, `forbidden`, `network_error`.

Важно: это только первая страница каталога, а не полное дерево и не подтверждение резервного копирования. Для проверки на MacBook: обновить ветку, собрать приложение, войти (или использовать Keychain), убедиться в появлении квоты и имён, нажать Refresh. Не присылать в GitHub токены или приватные имена файлов в логах/скриншотах без маскировки.
