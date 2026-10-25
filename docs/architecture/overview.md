# Architectural Overview

QADREN synchronizes and transfers files between independent storage connections. It is not a permanent mirror or a content-version archive.

## Components

- **Core**: Rust models, policy, JobStore, state, and execution rules.
- **SwiftUI interface**: macOS configuration, consent, status, and monitoring.
- **Worker**: the only executor for a Job attempt.
- **Storage adapter**: storage-specific capability and I/O boundary.
- **Scenario**: synchronization rules and direction.
- **Job**: one Scenario execution or manual file operation.
- **Registry and SQLite**: durable local metadata, connection state, Job attempts, and synchronization state.

One application can combine Controller and Worker roles. A Controller never silently executes work assigned to another Worker.

## Release 1 boundary

Release 1 uses Local Filesystem, SMB network drives through a QADREN mount (D-022), and Yandex Disk as a one-way Scenario source (D-023). It uses Rust Core, SQLite, narrow FFI, the macOS SwiftUI interface, and one Rust JobStore. iCloud is post-release. Remote Worker transport, discovery, trust, distributed locking, and Controller re-election are deferred.

## Execution contract

1. Validate the connection, capability, exact roots, scope, and required consent.
2. Save a durable Job and attempt before a modifying external effect.
3. Keep new destinations no-overwrite.
4. Use the approved protocol for replacement and deletion.
5. Treat adapter acknowledgement as distinct from verified outcome.
6. Reconcile uncertain effects with non-mutating checks. Do not blindly replay an uncertain mutation.
7. Store durable state before SwiftUI presents completion.

See [Synchronization model](backup-model.md), [Storage adapters](storage-adapters.md), [Local Worker runtime](local-worker-runtime.md), and [Data safety](../policies/data-safety.md).
