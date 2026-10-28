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

Schedules are part of a Scenario ([D-025](../decisions/D-025-autonomous-scheduled-scenarios.md)). A schedule is "days at a time" (selected days and one local time in an IANA timezone) or "every N hours" (1 to 24 hours from an anchor, counted in UTC). A Worker records durable schedule claims before it dispatches work. It must prevent duplicate execution across restart, time change, or repeated notification. Missed slots merge into one run for each Scenario. A local pause persists until explicit resumption.

A manual run and a scheduled run use one path: snapshots, a stored plan, and a durable Scenario Run (D-024). A Scenario has at most one active run. A slot that arrives during an active run is recorded as skipped; the active run continues, and no follow-up run starts. Runs of different Scenarios execute in parallel, except runs whose write root overlaps a root of another run on the same storage: such a run waits.

A run creates, updates, and deletes destination objects so that the destination matches the source. In a one-way Scenario the source always wins. In a two-way Scenario a change on both sides is a conflict.

The Worker may continue permitted local work when the Controller is unavailable. Remote delegation requires a separate decision for transport, trust, and idempotency.

## Adapter boundary

An adapter declares capabilities. The scheduler must not assume an unconfirmed capability. Release 1 uses Local Filesystem, SMB network drives through a QADREN mount (D-022), and Yandex Disk as a one-way source (D-023). iCloud is post-release. See [Storage adapters](storage-adapters.md) and [Data safety](../policies/data-safety.md).
