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

In `core/src/capabilities.rs`, a declarative description of capabilities `CapabilityDeclaration`, `Operations`, `Platform` and `RequestedOperation` is defined. This model is for compatibility verification and is not a permission to perform I/O: the adapter must re-verify permissions, actual object availability, and for destructive mutations — explicit approval of the affected scope unless already covered by a durable approved Scenario rule. Reading needed content, including File Provider materialization, does not need an extra approval.

Current flags do not prove the implementation of content APIs or confirmation of cloud commit. Do not confuse `prerequisites_verified` with a stable permission. The actual state can change after verification.

## Yandex Disk

**Not part of Release 1.** This controller is preserved as a post-release code path: do not perform new OAuth/API/transfer/live E2E until a new decision from the owner.

OAuth Authorization Code + PKCE, independent Keychain credentials based on `connection_id`, and reading of quota and the first page of the directory via GET Disk REST API without retrieving file bytes are implemented. Statuses 401/403/404/429/5xx and network errors are handled; automatic retry is off, Refresh is on user demand. Mock HTTP tests include 200/401/403/429/503/timeout; two accounts and independent disconnection are verified on a real Mac. Behavior for large files, API limits, checksums, resuming downloads and live uploads is **not verified E2E**; partial streaming upload code exists, but full file transfer is not accepted. Do not consider server-to-server copy available without verification.

## iCloud Documents — deferred (D-017)

**Not part of Release 1.** Do not develop sync/backup, own Documents container, adapter, entitlements, native spike, connection UI or integration tests. The historical idea was limited to its own visible container, not arbitrary access to iCloud Drive/CloudKit. Previous architectural assumptions do not mean current capabilities. Return only by a separate decision on price, access and Apple APIs. [D-017](../decisions/D-017-defer-icloud.md).

Native File Provider/iCloud placeholder handling remains part of the **local** adapter: required file reads may transparently materialize content without a separate user question. This does not create an iCloud adapter or allow unrelated prefetch.

## Local Filesystem

A standalone, actively developed adapter for local file systems and directories. **Not part of iCloud Documents** and not a method for hidden access to all of iCloud Drive. A Local-to-Local copy reads its selected source without an additional `read_content` consent dialog and can transparently materialize a File Provider file. The Rust Local Filesystem API has no per-file content-read consent parameter. Source/target validation remains required. New destinations stay no-overwrite. Uncertain effects use non-mutating reconciliation; never blindly retry a mutation.

For `local_mkdir_new`, JobStore saves the opened root device/inode identity with the bound attempt before `mkdir`. When the user reconnects the selected root, the app automatically performs read-only reconciliation. It completes an UNKNOWN attempt only when the enabled Local connection, operation, relative path, directory type, and root identity all match. A missing legacy binding or a different root remains UNKNOWN. Release 1 does not expose a user interface for manual directory creation or its reconciliation.

For `local_copy_new`, JobStore saves the destination root identity and a durable attempt before the copy. Completion requires a regular destination file with the expected byte count. This is metadata verification, not a content hash proof. When the user reconnects the selected destination root, the app automatically performs read-only reconciliation. It completes an UNKNOWN copy only when the selected enabled Local connection, destination root identity, operation, target path, and expected byte count match the stored attempt. It does not open content, overwrite, delete, or retry.

The Rust core and FFI have `local_copy_replace` for one confirmed replacement of an existing regular Local file. The Scenario plan requires a current revision-bound `Update` item and shows a separate destructive confirmation. The core writes a private sibling staging file and then uses the pinned-root atomic replacement primitive. The durable attempt, enabled connections, exact destructive replacement approval, destination root identity, operation kind, target path, and expected byte count remain bound to the Job. Separate source-read consent is no longer required by product policy. A failed or ambiguous publication is `UNKNOWN`. Read-only reconciliation uses the recorded replacement operation and exact metadata. New copies remain no-overwrite.

For a Scenario `Create`, the source and destination relative paths must be equal and the Job stores the exact Scenario revision. The source and destination local metadata revisions are saved to a JobStore outbox in the same transaction as Job completion. The app delivers pending entries before a Scenario dry-run. Delivery records the revision-bound `FileSyncState` in Registry. Delivery can repeat after a crash. It does not repeat the copy.

For a Scenario `Update`, the planner must propose `update` from the permitted source to the unchanged destination. A bidirectional Scenario can propose either direction. The app requires separate destructive consent for each item. Rust reloads the exact Scenario revision and checks both endpoint roots, both connection IDs, and the permitted direction before it creates a `local_copy_replace` Job. The Job uses the controlled replacement protocol.

The Scenario view can submit all current `Create` items or all current `Update` items after one explicit batch confirmation. Each item still creates and executes its own scoped Job. A failed item does not prevent later ordinary items from being submitted. Conflict tasks are excluded from these batches.

The alternative conflict queue stores exact metadata snapshots for both sides. A selected `replace_from_source` decision can run only after a second snapshot check, immediately before its durable replacement attempt. `replace_if_newer` also compares the saved Local Filesystem mtime fields. It creates the replacement Job only when the selected source is newer. A selected `delete_as_source` decision requires separate destructive confirmation. It verifies that the source is still absent and that the destination still matches its snapshot, then creates a durable delete Job. An interrupted delete is `UNKNOWN` and is not retried. A changed source or destination marks the task `stale` and starts no effect. The user must refresh the Scenario plan before reopening a stale task with new snapshots. Completion marks the task complete. The queue remains independent from JobStore and does not block ordinary transfers. Autonomous multi-item execution is not implemented.

## New adapters

Integrations of new clouds and servers are added independently of the core; the macOS dependency of iCloud is not transferred to all of QADREN.
