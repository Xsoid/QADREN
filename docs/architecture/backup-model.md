# Synchronization Model

## Scope

QADREN synchronizes and transfers files between independent StorageConnections. Release 1 is local-first and uses Local Filesystem connections. Content-version archives, runtime BackupSet, Scenario dependencies, and remote Worker transport are deferred.

## Model

- **StorageConnection**: one independent adapter configuration with its own ID and permissions.
- **Scenario**: named synchronization rules for two endpoints, direction, filters, conflict policy, deletion policy, and schedule.
- **Job**: one execution with a fixed Scenario revision or one manual file operation.
- **FileSyncState**: metadata about the last confirmed state. It supports change detection and idempotency. It is not a content archive.
- **Worker**: the sole executor of a Job attempt.
- **Controller**: configuration and monitoring. It does not execute work assigned to another Worker.

## Execution rules

- A Scenario has immutable revisions. A Job binds to one revision.
- New destinations remain no-overwrite. Replacement and deletion use the approved policy and exact disclosed scope.
- Save a durable attempt before a modifying effect. Recheck connection, capability, roots, paths, and required consent immediately before the effect.
- A required content read may occur without a separate prompt. Do not prefetch unrelated content.
- Keep Worker reports, provider acknowledgement, and verified integrity as distinct evidence levels.
- Reconcile a possibly completed effect with non-mutating checks. Do not blindly retry it. Keep `UNKNOWN` if evidence is inconclusive.
- Events received during a Job create a later Job. They do not overwrite the active Job state.

## Scheduling and autonomy

Schedules are part of a Scenario. A Worker records durable schedule claims before it dispatches work. It must prevent duplicate execution across restart, time change, or repeated notification. A local pause persists until explicit resumption.

The Worker may continue permitted local work when the Controller is unavailable. Remote delegation requires a separate decision for transport, trust, and idempotency.

## Adapter boundary

An adapter declares capabilities. The scheduler must not assume an unconfirmed capability. Local Filesystem is active in Release 1. Yandex Disk and iCloud are post-release. See [Storage adapters](storage-adapters.md) and [Data safety](../policies/data-safety.md).
