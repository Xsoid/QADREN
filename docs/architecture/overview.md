# Architectural Overview

Status: product principles v0.2 accepted; release 1 — local version, remote Workers — after release 1.

## Purpose

QADREN is a system for synchronization and file exchange between local, cloud, and remote storages with autonomous job execution. It is not a mandatory local mirror of clouds.

## Logical Parts

- **Core**: scenarios, jobs, synchronization control metadata, states, scheduling, and log.
- **Controller**: rule configuration, task assignment, result retrieval, and monitoring.
- **Worker**: direct execution of assigned operations.
- **Storage Adapter**: compatibility and operations for a specific storage.
- **Scenario**: an independent synchronization configuration; **Job**: a specific execution or a one-time file operation. BackupSet is deferred (see backup-model.md).
- **Execution Report**: a Worker message about execution, errors, and evidence.
- **Local Download Policy**: a rule for downloading content on a specific node.

These roles are logical. One application instance can combine the Controller and Worker roles, but delegation does not give the controller the right to execute another worker's task secretly.

## Platformity

The architecture is **cross-platform**. Specific capabilities are determined by the OS, user rights, and the adapter. The active adapter for Release 1 is Local Filesystem; the existing Yandex Disk controller is kept in the code but moved to post-release P3 (D-018); Apple iCloud is a separate future capability that does not limit the macOS core.

## Limitations

- The Controller is not required to see or download file content.
- A Yandex API response acknowledging an operation is not evidence of an intact remote copy without a confirmation check.
- A separate device can execute jobs autonomously according to previously received rules.
- Communication protocol, node discovery, distributed locks, and controller re-election are **deferred**.
- The local release stack P0–P2 is **Rust Core + SwiftUI macOS + SQLite + Local Filesystem**; Yandex Disk remains in the source code, but integration work is P3. The iCloud adapter is deferred per [D-017](../decisions/D-017-defer-icloud.md). See [stack](technology-stack.md).

## Current Specification

[QADREN v0.2](backup-model.md) is the source for product requirements. Internal file version archives and mandatory BackupSet are not part of the MVP.

## Principle of Compactness

Minimum background activity, no permanent full mirrors on the main MacBook, and no mandatory heavy server stack. Establish specific resources and sizes after measurements.

## Actual Architectural Base (Audit 2026-10-11)

The `main` branch contains: Rust models `model.rs`, capabilities `capabilities.rs`, `JobStore` + attempts/events/expectations in `jobs.rs`, `confirmation.rs`, and `verification.rs`, `Registry` in `registry.rs`, Yandex metadata/OAuth/Keychain and initial streaming upload, protected partial Local FS, narrow Registry/Yandex FFI, and a SwiftUI connections screen. There is no proven end-to-end Job $\rightarrow$ adapter $\rightarrow$ durable verification $\rightarrow$ SwiftUI operation; scenario execution, scheduler, and independent service are not complete; the iCloud container is intentionally excluded from the scope. The implemented SQLite JobStore is the base; do not rewrite the second queue.

### Ownership and Dependencies

- **Controller** (local logical role): edits Scenario, sets schedules, forms Job, shows reports; does not execute a task assigned to another Worker.
- **Worker / Rust Job execution**: the sole executing component of each attempt; in the first increment, it is hosted inside the SwiftUI process without binding business logic to Swift; later, the same engine moves to a service. Ownership and durable attempt are recorded before the external effect.
- **Storage Adapters**: independent instances by `connection_id`; platform prerequisites and truthful capabilities, metadata without secret content reading, streaming, evidence, and deterministic classification of error/uncertainty; they do not make UI decisions.
- **Policy**: preliminary authorization/consent, conflict/mass deletion assessment, and post-check before modifying I/O; a preliminary stat is not an atomic guarantee during concurrent mutations.
- **Scenario**: a diff plan relative to the last confirmed `FileSyncState`, revisions, and directional rules; it does not perform I/O directly.
- **SQLite**: the source of truth for connections, Jobs/attempts/events, FileSyncState, decisions, and service history. Registry and JobStore exist separately; do not merge files/transactions without a proven requirement. On disconnect, prohibit new Jobs and synchronize already active attempts before deleting the Keychain reference.
- **FFI**: versioned commands/results and memory ownership; do not export arbitrary internal Rust structures or secrets. SwiftUI displays durable state and sends commands and confirmations.

### Integration Execution Invariants

1. Validate scope, connection, platform/capability, and consent; unconfirmed rights result in a refusal without I/O.
2. Stable `job_id`/`attempt_id` and target path/operation are saved in SQLite **before** the external effect.
3. Modifying I/O allows a validity check immediately before the effect; no-overwrite for the first vertical.
4. An adapter response is not necessarily evidence of integrity. Verification returns evidence levels; completion occurs only with a sufficient level and the corresponding attempt.
5. Failure after a possible external effect $\rightarrow$ `UNKNOWN`, then read-only reconciliation. Blind retry is prohibited, including timeout and crash.
6. Cancellation/pause after a possible effect does not erase the attempt; partial progress and the need for reconciliation are preserved.
7. Committing the Job result and events to SQLite is performed consistently; the UI rereads the actual durable state.
8. Platform limitations are recorded in capabilities based on actual implementation. `AdapterCapabilities` from `lib.rs` and `CapabilityDeclaration` from `capabilities.rs` must be synchronized without blind ABI changes.

### Explicit Decisions before Release 1

Execution starts inside SwiftUI, but Rust owns the engine and SQLite. Destructive Local FS is fail-closed until the safety of concurrent changes is proven; a decision to release with limitations is made separately. App Store versus direct signed/notarized delivery remains open; check service lifecycle and distribution independently, without an iCloud entitlement spike.
