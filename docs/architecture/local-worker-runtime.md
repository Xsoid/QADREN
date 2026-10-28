# Local Worker Runtime

## Decision

Release 1 uses one bundled local Worker process. The Worker is the only process that mutates the Rust JobStore, stages scheduled work, or executes a Job. SwiftUI is a local controller and durable-state reader.

The local transport uses a private Unix domain socket below the QADREN Application Support directory. It has no TCP listener, HTTP endpoint, remote discovery, cloud credential transport, or cross-user access.

## Ownership

The Worker owns Job execution, reconciliation, scheduling, pause state, and destructive decisions. SwiftUI may request status, queue operations, pause, resume, stop, pending decisions, and an explicit decision. SwiftUI must not open a second executor or schedule timer.

The Worker has three threads ([D-024](../decisions/D-024-large-transfers.md)). [D-025](../decisions/D-025-autonomous-scheduled-scenarios.md) changes the run thread: a schedule slot starts the same durable Scenario Run as a manual request, the run executes Create, Update, and Delete children, due items are selected from each active run in turn, a pool of effect threads with limits for each connection executes them, and a write-root lock keeps overlapping runs apart. Until that work is complete, the text below states the current behavior. The IPC thread answers every request, including `status` with the count of batch files that run now. The run thread executes confirmed Create batches: `execute_create_batch` only stores the batch token and answers `accepted`. The poll thread runs reconciliation and schedules every 60 s. The Worker process claims the JobStore once at start (owner lock and restart rule); its threads then open plain connections that wait up to 30 s for a write of another thread. The Registry waits up to 10 s.

A confirmed batch is a durable receipt in the Registry. Its children run in path order, at most 2 at a time for a Yandex Disk source. Pause and cancellation are durable flags on the receipt; the run thread reads them before each next file. A child that fails before any effect with a transient error (network, HTTP 429, HTTP 5xx, content not served, a missing mount) stays `pending` with a new Job ID and a retry time: 1 min, 5 min, 30 min, 2 h, 12 h. After the last retry it is `failed` with the code `<reason>_after_retries`, unless the Scenario parameter `retry_until_success` is on (the default; a column of the `scenarios` row, outside the revision): then the Worker retries it every 24 h until it succeeds or the user pauses or stops the batch. A permanent error makes it `failed`. An unproven effect makes it `unknown` and is never retried. HTTP 401 pauses the batch and removes the token. The receipt keeps the retry count, the next retry time, and the last failure code of each child. A status request that gets no answer in time means "busy or not responding", not "stopped".

An ordinary content read required by an enabled or requested operation needs no decision. Replacement, deletion, and mass deletion require the policy-defined durable consent and exact scope checks. If scope cannot be enforced, the Worker must disclose the actual bounded scope or refuse the action.

## Exclusivity and recovery

The Worker acquires a private lock before it opens mutable stores. A second start fails before it can issue I/O. A Yandex Disk run gets its OAuth token from the app over the Worker socket and keeps it in memory only (D-023, D-024 stage A). After a Worker restart such a run waits until the app sends the token again. D-025 adds the SMB login: the app gives it to the Worker in the same way, and the Worker mounts the share itself through NetFS at the QADREN mount directory. A launch-at-login setting starts the app without a window after a Mac restart, so the Worker gets the token and the login without the user. SwiftUI reads the JobStore in read-only mode. A read takes no owner lock, runs no migration, and does not apply the restart rule, so a controller read never blocks the Worker or changes a Job state. The Worker uses the existing Registry and one JobStore. It must not create a second queue, Scenario engine, or Swift business-logic copy.

After an unclean exit, the Worker performs read-only reconciliation for ambiguous attempts. When both Scenario endpoints use the same Local connection, reconciliation checks each root of that connection against the durable root identity of the attempt. A resumed batch file that already has a Job reports that Job state and never starts a second attempt. The Worker removes QADREN staging files (`.qadren-<pid>-<n>.tmp`, `.qadren-replace-<leaf>-<pid>-<n>.stage`) that an interrupted copy of an UNKNOWN Job left next to its target, only in the bound destination root and only when the writer process no longer exists. It never removes the target file or a file with any other name. It persists a verified result only when evidence is sufficient. Otherwise it retains `UNKNOWN`. It never replays a copy, replacement, or deletion merely because a response was lost.

## Lifecycle

The app registers the bundled Worker as a LaunchAgent through `SMAppService` (`Contents/Library/LaunchAgents/dev.qadren.worker.plist`). launchd starts it at load and restarts it after any unsuccessful exit, including a kill by a signal. Quitting the UI does not stop it. The menu offers "Остановить фоновую службу…" with a warning, which unregisters the service, and "Запустить фоновую службу". A stop by the user is durable: the app does not register the service again silently. A test instance with `QADREN_DATA_ROOT` registers the service only when `QADREN_REGISTER_AGENT=1` is set, and the Worker honors the same `QADREN_DATA_ROOT`.

Closing the UI disconnects the controller. It does not silently stop active Worker work. An explicit stop requests a controlled Worker stop and preserves queued and uncertain Jobs. The UI reports only a confirmed response or persisted state.

The selected distribution method determines signing, service registration, and restart evidence. Those are release validation requirements, not proof of a separate Worker architecture.
