# D-020 — One "This Mac" Connection and the Connector List

**Date:** 2026-10-24. **Status:** accepted. **Revises:** D-016 for the `local_filesystem` adapter only. D-016 stays in force for every other adapter.

## Decision

1. **Connector and connection.** A connector is a kind of storage. A connection is one configured record of a connector. Release 1 knows two connectors: "Этот Mac" (`local_filesystem`) and "Яндекс.Диск" (`yandex_disk`).
2. **One "This Mac" connection.** Each Mac has exactly one Local Filesystem connection. QADREN creates it automatically when the Registry has none. The product never creates a second one. A second one has no use: one connection reaches every folder and volume of the Mac, and a Scenario can use it on both sides with different roots.
3. **No removal of the last "This Mac".** The user can rename and disable it. Only an extra Local record from older data can be removed; the interface reports such duplicates.
4. **Yandex Disk.** Any number of Yandex Disk connections can exist (D-016). The connector is parked until P3 (D-018), so the interface does not create new ones now. Existing records stay manageable.
5. **Connector list.** "Добавить подключение" shows every connector with its state: available, already connected ("Этот Mac"), or parked ("Яндекс.Диск").

## Implementation boundaries

- `Registry::ensure_local_computer` returns the existing Local Filesystem connection or creates one in one immediate transaction. The product calls only this path (`qadren_registry_ensure_local_computer`).
- The generic `Registry::add` keeps no single-record rule. Tests use it to model two independent local endpoints.
- The removal workflow refuses to remove the last Local Filesystem connection.

## Affected documents

- [D-016](D-016-multiple-connections.md)
- [Settings window](../design/settings.md)
