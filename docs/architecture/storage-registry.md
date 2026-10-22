# Storage Connection Registry

Status: Registry v2, SwiftUI integration, independent Keychain OAuth by connection_id and metadata-only Yandex Disk are implemented in `main`. The multiple connections model [D-016](../decisions/D-016-multiple-connections.md) is accepted. Content operations and the synchronization engine are not implemented.

The first multiple-connection UI test record is in [Multiple independent connections testing](../development/multiple-connections-testing.md).

## Purpose

`StorageConnection` is a local record of **one instance** of a connection to a provider. The number of instances of one `adapter` or one external account is not limited. The `id` is not equal to the login and does not depend on the connection name. In the current model, `Scenario` refers to independent connections by stable IDs; the registry does not perform transfers.

| Field | Content |
|---|---|
| `id` | Opaque 128-bit random identifier (hex) |
| `adapter` | Adapter type key, for example `yandex_disk` |
| `name` | User name |
| `enabled` | Boolean that indicates if the connection is allowed |
| `status` | disconnected / connected / requires_auth / offline / error |
| `credential_ref` | Reference to a Keychain record; **not an OAuth token** |
| `last_checked_at` | Time of the last check (integer, epoch seconds according to the future API layer convention) |
| `diagnosis` | Safe diagnostic text without secrets |

The adapter calculates `AdapterCapabilities` based on the execution environment/service version; these capabilities are not stored in the table permanently.

## API

`Registry::open(path)`, `in_memory()`, `add(adapter,name)`, `get(id)`, `list()`, `rename(id,name)`, `set_enabled(id,bool)`, `remove(id)`, `mark_checked(...)`, `set_credential_ref(...)`.

In the future integration, `credential_ref` must refer to the Keychain record **of this connection_id**. The global Keychain `account=default` is a legacy of an early prototype; the current UI provides a separate manual import with a check of the new token before the deletion of legacy data (D-016). Before the deletion of a connection, the system must check Scenario/Job references as these entities are implemented.

`remove` deletes **only the local record**. It does not access the cloud. The platform layer must delete the secret from the Keychain; before the connection of real secrets, the delete operation must be coordinated to avoid credential orphans. This operation is not a complete disconnect UX.

## Migrations and Integrity

SQLite `PRAGMA user_version=1`. During opening, the system executes `PRAGMA quick_check`; if damage is detected, the opening fails with an error. There are no automatic recovery or overwrite attempts. If the schema version is greater than the supported version, the system returns an explicit error without changing the database. Migration 0→1 is performed transactionally.

Future versions: sequential transactional migrations and a database backup before destructive changes. Future Scenario/Job schemas must include references to the connection and must be migrated transactionally.

## Safety

Only metadata and **non-secret references** are stored in SQLite. Do not write HTTP response bodies, Authorization headers, or URLs with tokens into `diagnosis`. The database path must be in the user Application Support with restricted permissions (implement in macOS UI/platform layer). Tests use `:memory:`.

## Not Implemented

Yandex Disk content operations, Scenario/Job, background executor, and Local Filesystem. iCloud Documents are not part of the active Release 1 (D-017). Metadata-only operations do not mean that synchronization is ready.

## Schema v2 — External Account Identity

The new optional field `provider_account_id` is the account identifier **at the provider** (not the OAuth application Client ID and not the `connection_id`). The system fills this field after authorization. This field is intentionally **not unique**: two connections of one external account are independent. Changing this field does not change the local ID, Keychain credential, or BackupSet links. Migration v1→v2 adds a nullable column in a transaction without destroying existing records. When transferring the legacy Keychain `default`, the system must use the separate verified procedure from D-016 and must not rewrite the token automatically during database opening.
