# Synchronization Model: Scenario, Job, FileSyncState

Status: agreed product direction, 2026-10-09. This does not mean that the functionality is already implemented.

## Purpose and Scope

QADREN manages the transfer and one-way/two-way synchronization of files between independent StorageConnection, local directories, and cloud resources. **The first release is a full local version**, in which one application instance performs the roles of Controller and Worker. Remote Workers and inter-node transport are the next stage, pending separate approval of the protocol. Own backup archives, content versioning, recovery of historical copies, BackupSet, and dependencies between scenarios are out of scope for the MVP.

## Model

- **StorageConnection** — an independent adapter instance with its own ID/permissions; it can be a source and a destination.
- **Scenario** — a named configuration of two endpoints/directories, synchronization direction, filters, deletion and conflict policies, triggers, and an assigned Worker. It has revisions.
- **Job** — an execution unit with a fixed Scenario revision or a one-time internal operation. Release 1 exposes content transfer only from a Scenario plan.
- **Controller** — owns the configuration, assignments, queues, and general monitoring. **Worker** — the sole executor of a delegated task; it can operate autonomously and manage local execution, but it cannot edit the assigned scenario.
- **FileSyncState** — service metadata and the last confirmed agreed state of the parties to identify changes, conflicts, and idempotency; **it is not a content archive**. It is linked to a specific Scenario revision and is recorded only after independent confirmation of the result, not after a scan, request, or provider acknowledgement.
- **ExecutionEvent/Report** — events, provider confirmations, and check results; UNKNOWN is not equal to FAILED.

## Execution Rules

- One-way or two-way synchronization; filters by paths/folders/extensions; conflict and delete policies are defined by the scenario; unknown file identity is not considered equality.
- A scenario has several independent schedules, separate switches, IANA timezone (Controller by default), and one launch per DST period; if local time is missing, use the nearest allowable moment; if repeating, use one launch.
- Change triggers are merged intelligently; changes during execution result in one subsequent task; misses during Worker unavailability are merged into one catch-up. Schedule occurrence calculation is read-only. For schedule slots, the Registry atomically records the receipt and a durable outbox item; the dispatcher idempotently creates exactly the same pinned Job in a separate JobStore and then confirms the outbox. This prevents launch loss during a crash between SQLite stores, but it does not replace the catch-up policy or scheduler service.
- A completed local Scenario `Create` records source and destination metadata revisions in a JobStore outbox with the Job completion. A dispatcher writes the revision-bound `FileSyncState` to Registry and then acknowledges the outbox. A repeated delivery does not repeat the copy.
- Manual Jobs receive an initial high priority; long waiting increases the effective priority of automatic jobs; manual queue reordering is allowed. A repeated launch of a busy scenario is placed in the queue.
- A disabled Scenario prohibits automatic triggers but allows manual launch. When disabling, the user chooses to: terminate active Jobs or pause safely. When deleting: safe stop and confirmation from the Worker.
- Scenario changes apply only to future Jobs. Worker reassignment occurs only after confirmation that the previous execution has stopped. Forced removal of a Worker leaves undefined scenarios locked.
- The Worker saves a local pause across reboots until manual resumption. The Controller can store commands for an offline Worker, but "sent" is not equal to "applied".
- After a failure, automatically reconcile ambiguous results using non-mutating checks. Mark verified outcomes when supported by evidence, and retain `UNKNOWN` when inconclusive. Do not blindly repeat destructive effects. New destructive choices require human decisions.

## Adapters

**Update D-018 (2026-10-14):** Release 1: P0 — foundation/UI, P1 — verifiable Local Filesystem controller and safe Local↔Local synchronization, P2 — local service and distribution package. The Yandex Disk controller is moved to post-release P3, iCloud is indefinite. **The historical wording below is no longer the current scope:** downloading, uploading, and synchronization of Yandex Disk and local directories. The first path for Release 1 is only Local↔Local; cloud combinations require a new solution and real capability/E2E tests. Copying does not guarantee the storage of historical versions.

- **Yandex Disk** via official API; metadata/OAuth work, content primitives are partial, full upload/download and synchronization are not yet proven E2E.
- **iCloud Documents** — a deferred option outside of Release 1. Do not implement backup/sync, container, adapter, and Apple entitlements without a separate decision ([D-017](../decisions/D-017-defer-icloud.md)).
- **Local Filesystem** — **a separate adapter**, not part of iCloud Documents; it can be combined in a Scenario without mixing permissions.
- The adapter declares the OS, rights, and capabilities; the scheduler does not substitute missing capabilities.

## Security and History

- QADREN automatically reads file content required by an enabled/requested operation, including possible File Provider materialization on the main MacBook. No separate read or hydration consent is needed. Do not prefetch unrelated files; do not interfere with normal macOS/iCloud operations.
- Show the affected scope and request approval for replacement or deletion outside an exact preapproved rule. A mass-deletion proposal needs explicit confirmation of the proposed set. If QADREN cannot constrain an operation to selected files, disclose the actual possible broader effect. Use provider trash where available; QADREN has no internal content-versioning system or trash.
- Result data distinguishes between Worker report, provider confirmation, and actually verified integrity; do not claim a recovery guarantee without a backup version.
- Job/Event history has a configurable cleanup period, but unfinished states awaiting confirmation, unsent events, and service history necessary for correct synchronization are not deleted.

## UX/UI

- Dispatcher interface with an arbitrary number of local panels and separate windows, with full restoration of placement and context history.
- Standard widgets are added from a single list with search, without cloning; grid without overlaps; separate layout mode with auto-save and undo.
- Summary/individual resource widgets; queue; launch timeline with operational actions; tasks, file tree, progress; errors/conflicts; general health indicator. Speed and ETA are available only for the Job, and ETA is shown only when reliable.
- General panel filters with widget overrides; object selection updates linked widgets; back/forward and breadcrumbs; full paths without abbreviation.
- Three densities **for each widget**, scale **for each window**, light/dark/automatic themes. Global critical messages are not hidden by contextual filters and do not force windows to open.
- The Scenario editor owns endpoints, root folders, direction, filters, and conflict policies. The Scenario plan is the only Release 1 user surface for content transfer.
- The Job tree supports arbitrary multiple selection of files/directories from different branches and mass actions. It stably preserves selection/scroll during updates; a folder shows bytes **and** the number of files.
- Synchronization preview — on request, summary+tree, automatically updates, but is not mandatory before every launch. It is built only from allowed metadata snapshots, immutable Scenario revisions, and confirmed FileSyncState; it does not scan adapters itself, does not read content, does not create a Job, and does not change the baseline.
- Confirmation requests remain in the context of widgets/operations; a global indicator counts them; there is no separate center.
- Extended system indicator: active Jobs, progress, operations, and pause. The background service starts automatically; windows start manually; exiting the UI does not stop the service; explicit stop of the service occurs with a warning.
- The Worker has an independent local UI only for its own available tasks; panel configurations are not transferred between nodes.

## What not to do now

Do not implement iCloud sync/backup/adapter/native spike without separate decision D-017. Do not introduce BackupSet into runtime, mandatory backup of every file, graph dependency of scenarios, content versioning, quota analytics over time, space forecasts, global Worker speeds, graphical topology, search index, arbitrary custom widgets, automatic Worker selection, or UI synchronization between devices.

See [roadmap](../roadmap.md) and the history of the previous model in Git.
