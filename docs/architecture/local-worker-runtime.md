# Local Worker Runtime

Status: **accepted implementation contract for Issue #23**. The current
foundation implements the bundled process, private socket, exclusive start,
schedule dispatch, controlled stop, and one pinned Scenario `Create` command.
It does not yet migrate all Job execution or UI commands. This document
defines the Release 1 local process boundary. It does not define remote Worker
transport or node delegation.

## Decision

Release 1 uses one bundled `QADRENWorker` executable. A signed distribution
can use macOS `launchd` to start and restart this executable. The Worker is
the only process that opens a mutable `JobStore`, stages scheduled work, or
executes a Job. The SwiftUI application is a local controller and status
reader.

The bundle contains the Worker executable. A user does not need Rust, Swift,
Xcode, or another developer tool after installation. A development bundle can
run the same executable directly for local tests. An unsigned development
bundle is not release evidence for `launchd` registration or restart.

## Local Transport and Identity

The Worker listens only on a Unix domain socket below the QADREN Application
Support directory. The directory and socket use the current macOS user account
and mode `0700` or stricter. The Worker rejects a socket directory that is a
symlink, is not a directory, or permits group or other access. The protocol has
no TCP listener, HTTP endpoint, remote discovery, cloud credential transport,
or cross-user access.

Each request has one versioned command and one JSON result. The UI reconnects
after a socket or Worker restart. It must treat a lost response after a
mutating command as uncertain, reload durable state, and never resend a
destructive command merely because it did not receive a response.

Only the current-user private socket path identifies the local client in
Release 1. This is not a remote Worker authentication protocol. Do not extend
it to another host or user without a separate decision.

## Ownership and Commands

The completed Worker owns these operations:

- open and mutate the existing Registry and the one Rust `JobStore`;
- run startup and reconnect read-only reconciliation;
- stage and dispatch due schedule receipts;
- enqueue, pause, resume, stop, and report Jobs;
- hold a destructive request as `WAITING_FOR_DECISION` until the UI returns an
  explicit decision for the saved scope.

The UI can request `status`, `start`, `stop`, `queue`, `pause`, `resume`,
`pending_decisions`, and an explicit decision. It does not open a second
executor or schedule timer. `stop` is distinct from quitting the UI. The
Worker persists a controlled stopped state and does not discard queued or
UNKNOWN Jobs.

An ordinary content read needed by an enabled or requested operation needs no
decision. A replacement, deletion, or mass deletion uses the existing durable
attempt and exact scope checks. A new destructive consequence creates a
durable `WAITING_FOR_DECISION` record. The Worker never assumes approval while
the UI is closed. If it cannot constrain an operation to selected files, it
reports the actual bounded scope and limitation instead of executing it as a
selected-file action.

## Exclusivity and Recovery

`launchd` maintains one Worker instance. The Worker also acquires an advisory
lock in the private Application Support directory before it opens mutable
stores. A second start fails before it can issue I/O. The Worker releases the
lock only during controlled shutdown.

On startup after an unclean exit, the Worker performs only read-only
reconciliation for durable ambiguous attempts. For a scheduled Local Filesystem
child copy, the Registry mapping identifies the pinned destination root. The
Worker checks only the saved destination metadata. It persists a verified result
when evidence is sufficient. Otherwise it retains `UNKNOWN`. It never repeats
a copy, replacement, or deletion as recovery.

The Worker uses the existing Registry and JobStore files. It does not create a
second JobStore, queue, scenario engine, or business-logic copy in Swift.

## Lifecycle Boundaries

Closing every window or selecting Quit UI disconnects the controller only. An
active Worker continues under `launchd`. An explicit Stop Worker action asks
the Worker for a controlled stop; it does not silently terminate active work.
The UI reports only a confirmed socket response or persisted Worker state. It
does not invent a healthy status from a process launch attempt.

## Current Foundation Boundary

`qadren-worker` currently acquires the private lock, opens the existing
Registry and JobStore, stages due schedule receipts, dispatches their pinned
Jobs, scans the pinned Local Filesystem roots at run time, and serves `status`, `poll`, controlled `stop`, `execute_create`,
`execute_replace`, `execute_conflict_replace`, `execute_conflict_delete`,
`set_scenario_paused`, and `select_conflict_decision`
over the private socket. The Scenario commands accept one exact
Scenario revision, connection IDs, roots, relative source and destination
paths, and Job ID. `execute_create` uses the existing Rust no-overwrite
Scenario Create executor. `execute_replace` uses the existing Rust controlled
replacement executor and requires explicit replacement consent. Both executors
save the durable Job and attempt before they write the selected destination.
The conflict commands accept only a conflict ID, Job ID, and destructive
consent. Rust reloads the selected durable conflict task, its Scenario revision,
and its saved snapshots. It rejects missing consent, stale snapshots, and an
unselected task before it changes a file.
The macOS bundle includes the executable and the `SMAppService` LaunchAgent
definition. `QADRENApp` requests registration and reports the system
registration state without treating it as a health check. If the registration
request fails, it reports a failed registration. It does not display a system
error that can contain installation-specific paths.

Each schedule Job remains a coordinator receipt that stores a Scenario identity
and revision, not a durable multi-file execution plan. At execution time the
Worker reads only Local Filesystem metadata, composes the existing conservative
plan, and creates one durable child Job for each safe no-overwrite `Create`.
An `Update`, deletion, content conflict, or an unavailable metadata proof does
not change a destination. The Worker saves a durable conflict task and leaves
the coordinator Job `WAITING_FOR_DECISION` where it has complete observed
snapshots. A scan or local operation failure marks the coordinator Job failed.
An UNKNOWN child effect leaves the coordinator UNKNOWN. The Worker does not
blindly replay it. The production Swift Scenario `Create` and native-confirmed
`Update` paths send their exact scope to the Worker and do not
fall back to in-process FFI. If the private socket is unavailable before a
request is accepted, the UI reports the failure and does not write a file.
Native-confirmed conflict replacement and deletion also use the Worker. Scenario
pause and conflict-decision persistence also use the Worker. Mock-only
pause/resume and read-only Job status remain FFI test and inventory paths. Therefore this increment does not yet establish the Issue
#23 single-mutation-owner acceptance or the UI-quit execution acceptance.

This foundation does not prove a signed release bundle, service approval,
File Provider behavior, resource consumption, sleep/wake handling, or remote
Worker support. Signing and service approval are distribution acceptance work.
They are not a Release 1 Worker ownership requirement by owner decision on
2026-10-20. Issue #24 owns the resource measurements.

The native IPC fixture starts the Worker separately, starts a disposable native
controller process that submits one Scenario Create, and waits for that
controller to exit. The fixture then submits and verifies a confirmed
replacement through the still-running Worker. This proves controller-process
exit does not stop the direct development Worker. It does not prove a signed
`SMAppService` registration or a full SwiftUI Quit lifecycle.

`scripts/test-macos-worker-bundle.sh` builds the developer bundle and verifies
the bundled executable and LaunchAgent contract. It does not register the
service. A signed installed bundle can provide that distribution evidence.

## Rationale

The application-resident timer in `QADRENApp.swift` stops when the process
quits. A bundled `launchd` Worker permits local execution after UI exit while
keeping Rust as the sole execution engine. A private Unix socket is sufficient
for one user on one Mac and avoids a network listener or a premature remote
protocol. The lock protects the mutable SQLite stores even when a development
or recovery path attempts a duplicate start.
