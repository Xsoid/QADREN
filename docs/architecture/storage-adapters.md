# Storage Adapters

Status: **model accepted; first Local controller vertical slice implemented.** Local Filesystem and its Release 1 controller are under active development; Yandex Disk controller is moved to post-release P3, the code is preserved; iCloud is future-only for an indefinite period (D-018). `local_mkdir_new` creates only a new folder via Rust JobStore, durable attempt and metadata verification, without reading user content. Yandex Disk metadata and macOS UI are historically verified (Issue #2, PR #4/#9); content E2E is not yet proven.

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

## Preliminary Rust contract (Issue #14)

In `core/src/capabilities.rs`, a declarative description of capabilities `CapabilityDeclaration`, `Operations`, `Platform` and `RequestedOperation` is defined. This model is for compatibility verification and is not a permission to perform I/O: the adapter must re-verify permissions, actual object availability, and for the main MacBook — consent to download content immediately before the operation.

Current flags do not prove the implementation of content APIs or confirmation of cloud commit. Their detail, errors, capabilities completeness and FFI compatibility will continue in #14. Do not confuse `prerequisites_verified` with a stable permission: the actual state can change after verification.

## Yandex Disk

**Not part of Release 1.** This controller is preserved as a post-release code path: do not perform new OAuth/API/transfer/live E2E until a new decision from the owner.

OAuth Authorization Code + PKCE, independent Keychain credentials based on `connection_id`, and reading of quota and the first page of the directory via GET Disk REST API without retrieving file bytes are implemented. Statuses 401/403/404/429/5xx and network errors are handled; automatic retry is off, Refresh is on user demand. Mock HTTP tests include 200/401/403/429/503/timeout; two accounts and independent disconnection are verified on a real Mac. Behavior for large files, API limits, checksums, resuming downloads and live uploads is **not verified E2E**; partial streaming upload code exists, but full file transfer is not accepted. Do not consider server-to-server copy available without verification.

## iCloud Documents — deferred (D-017)

**Not part of Release 1.** Do not develop sync/backup, own Documents container, adapter, entitlements, native spike, connection UI or integration tests. The historical idea was limited to its own visible container, not arbitrary access to iCloud Drive/CloudKit. Previous architectural assumptions do not mean current capabilities. Return only by a separate decision on price, access and Apple APIs. [D-017](../decisions/D-017-defer-icloud.md).

Native File Provider/iCloud placeholder checks when the **local** adapter operates are preserved: they prohibit hidden materialization and do not create an iCloud adapter.

## Local Filesystem

A standalone, actively developed adapter for local file systems and directories. **Not part of iCloud Documents** and not a method for hidden access to all of iCloud Drive. Before reading a file in an iCloud-managed directory, it is necessary to determine if the operation initiates forbidden materialization of content on the main MacBook.

For `local_mkdir_new`, JobStore saves the opened root device/inode identity with the bound attempt before `mkdir`. Read-only reconciliation completes an UNKNOWN attempt only when the enabled Local connection, operation, relative path, directory type, and root identity all match. A missing legacy binding or a different root remains UNKNOWN. Reconciliation does not create, remove, or retry a directory.

## New adapters

Integrations of new clouds and servers are added independently of the core; the macOS dependency of iCloud is not transferred to all of QADREN.
