# Storage Adapters

Status: **model accepted; first Local controller vertical slice implemented.** Local Filesystem and its Release 1 controller are under active development. Yandex Disk is post-release and its code is preserved. iCloud is future-only (D-018). `local_mkdir_new` creates only a new folder through Rust JobStore, durable attempt, and metadata verification. It does not read user content. Yandex Disk metadata and macOS UI source exist; content end-to-end behavior is not proven.

## Adapter type ≠ connection instance

Any adapter can serve multiple independent `StorageConnection` instances: this includes different accounts of one provider or repeated connections of one account. The adapter type does not store a global token or status of a specific user; each instance has its own ID, settings, Keychain credentials, errors and independent life history. Applicable technical limitations of Apple ID and a specific platform must be reflected through capabilities. Details: [D-016](../decisions/D-016-multiple-connections.md).

## Capabilities contract

Each adapter reports:
- supported OS and required permissions;
- availability of metadata without downloading content;
- reading and writing of content;
- streaming capability;
- confirmation of remote write completion;
- available version identifiers and checksums;
- capabilities for listing changes, deletion, and recovery.

The scheduler must not build a job based on an unconfirmed adapter function.

## Preliminary Rust Contract

In `core/src/capabilities.rs`, a declarative description of capabilities `CapabilityDeclaration`, `Operations`, `Platform` and `RequestedOperation` is defined. This model is for compatibility verification and is not a permission to perform I/O: the adapter must re-verify permissions, actual object availability, and for the main MacBook — consent to download content immediately before the operation.

Current flags do not prove the implementation of content APIs or confirmation of cloud commit. Do not confuse `prerequisites_verified` with a stable permission. The actual state can change after verification.

## Yandex Disk

**Not part of Release 1.** This controller is preserved as a post-release code path: do not perform new OAuth/API/transfer/live E2E until a new decision from the owner.

OAuth Authorization Code + PKCE, independent Keychain credentials based on `connection_id`, and reading of quota and the first page of the directory via GET Disk REST API without retrieving file bytes are implemented. Statuses 401/403/404/429/5xx and network errors are handled; automatic retry is off, Refresh is on user demand. Mock HTTP tests include 200/401/403/429/503/timeout; two accounts and independent disconnection are verified on a real Mac. Behavior for large files, API limits, checksums, resuming downloads and live uploads is **not verified E2E**; partial streaming upload code exists, but full file transfer is not accepted. Do not consider server-to-server copy available without verification.

## iCloud Documents — deferred (D-017)

**Not part of Release 1.** Do not develop sync/backup, own Documents container, adapter, entitlements, native spike, connection UI or integration tests. The historical idea was limited to its own visible container, not arbitrary access to iCloud Drive/CloudKit. Previous architectural assumptions do not mean current capabilities. Return only by a separate decision on price, access and Apple APIs. [D-017](../decisions/D-017-defer-icloud.md).

Native File Provider/iCloud placeholder checks when the **local** adapter operates are preserved: they prohibit hidden materialization and do not create an iCloud adapter.

## Local Filesystem

A standalone, actively developed adapter for local file systems and directories. **Not part of iCloud Documents** and not a method for hidden access to all of iCloud Drive. A Local-to-Local file copy can open a File Provider source only after the user confirms that macOS can materialize that exact source item. The Rust JobStore records and consumes a one-time `read_content` decision for its source `connection_id` and relative path before the open. The destination is no-overwrite. Uncertain post-effect state is `UNKNOWN`; it is never retried blindly.

For `local_mkdir_new`, JobStore saves the opened root device/inode identity with the bound attempt before `mkdir`. Read-only reconciliation completes an UNKNOWN attempt only when the enabled Local connection, operation, relative path, directory type, and root identity all match. A missing legacy binding or a different root remains UNKNOWN. The File Manager can list UNKNOWN mkdir Jobs bound to the selected root and connection, including after restart. It shows an explicit check action for each matching Job. Inventory and the check action do not create, remove, retry, or read a directory.

For `local_copy_new`, JobStore saves the destination root identity and a durable attempt before the copy. Completion requires a regular destination file with the expected byte count. This is metadata verification, not a content hash proof. Read-only reconciliation can complete an UNKNOWN copy only when the selected enabled Local connection, destination root identity, operation, target path, and expected byte count match the stored attempt. It does not open content, overwrite, delete, or retry. The File Manager lists persisted UNKNOWN copy Jobs for the selected destination root and connection, including after restart. It shows an explicit check action for each matching Job. Inventory does not inspect target paths, read content, complete a Job, or repeat a copy.

The Rust core, FFI, and File Manager have `local_copy_replace` for one manually confirmed replacement of an existing regular Local file. The File Manager requires a current metadata preview that reports an existing destination and shows a separate destructive confirmation. The core writes a private sibling staging file and then uses the pinned-root atomic replacement primitive. The durable attempt, exact source-read consent, enabled connections, destination root identity, operation kind, target path, and expected byte count remain bound to the Job. A failed or ambiguous publication is `UNKNOWN`. Read-only reconciliation uses the recorded replacement operation and exact metadata. New copies remain no-overwrite.

For a Scenario `Create`, the source and destination relative paths must be equal and the Job stores the exact Scenario revision. The source and destination local metadata revisions are saved to a JobStore outbox in the same transaction as Job completion. The app delivers pending entries before a Scenario dry-run. Delivery records the revision-bound `FileSyncState` in Registry. Delivery can repeat after a crash. It does not repeat the copy.

For a Scenario `Update`, the planner must propose `update` from the permitted source to the unchanged destination. A bidirectional Scenario can propose either direction. The app requires separate destructive consent for each item. Rust reloads the exact Scenario revision and checks both endpoint roots, both connection IDs, and the permitted direction before it creates a `local_copy_replace` Job. The Job uses the controlled replacement protocol. Batch and autonomous multi-item execution and conflict execution are not implemented.

## New adapters

Integrations of new clouds and servers are added independently of the core; the macOS dependency of iCloud is not transferred to all of QADREN.
