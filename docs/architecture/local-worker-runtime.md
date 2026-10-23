# Local Worker Runtime

## Decision

Release 1 uses one bundled local Worker process. The Worker is the only process that mutates the Rust JobStore, stages scheduled work, or executes a Job. SwiftUI is a local controller and durable-state reader.

The local transport uses a private Unix domain socket below the QADREN Application Support directory. It has no TCP listener, HTTP endpoint, remote discovery, cloud credential transport, or cross-user access.

## Ownership

The Worker owns Job execution, reconciliation, scheduling, pause state, and destructive decisions. SwiftUI may request status, queue operations, pause, resume, stop, pending decisions, and an explicit decision. SwiftUI must not open a second executor or schedule timer.

The Worker answers one IPC request at a time. A long request, such as a confirmed Create batch, delays later requests until it ends. For this reason, a stop request for a confirmed Create batch is a durable cancellation flag on the batch receipt in the Registry. SwiftUI writes the flag. The Worker reads it before each next file, finishes the current file, and records the remaining files as `cancelled`. A status request that gets no answer in time means "busy or not responding", not "stopped".

An ordinary content read required by an enabled or requested operation needs no decision. Replacement, deletion, and mass deletion require the policy-defined durable consent and exact scope checks. If scope cannot be enforced, the Worker must disclose the actual bounded scope or refuse the action.

## Exclusivity and recovery

The Worker acquires a private lock before it opens mutable stores. A second start fails before it can issue I/O. SwiftUI reads the JobStore in read-only mode. A read takes no owner lock, runs no migration, and does not apply the restart rule, so a controller read never blocks the Worker or changes a Job state. The Worker uses the existing Registry and one JobStore. It must not create a second queue, Scenario engine, or Swift business-logic copy.

After an unclean exit, the Worker performs read-only reconciliation for ambiguous attempts. When both Scenario endpoints use the same Local connection, reconciliation checks each root of that connection against the durable root identity of the attempt. A resumed batch file that already has a Job reports that Job state and never starts a second attempt. The Worker removes QADREN staging files (`.qadren-<pid>-<n>.tmp`, `.qadren-replace-<leaf>-<pid>-<n>.stage`) that an interrupted copy of an UNKNOWN Job left next to its target, only in the bound destination root and only when the writer process no longer exists. It never removes the target file or a file with any other name. It persists a verified result only when evidence is sufficient. Otherwise it retains `UNKNOWN`. It never replays a copy, replacement, or deletion merely because a response was lost.

## Lifecycle

Closing the UI disconnects the controller. It does not silently stop active Worker work. An explicit stop requests a controlled Worker stop and preserves queued and uncertain Jobs. The UI reports only a confirmed response or persisted state.

The selected distribution method determines signing, service registration, and restart evidence. Those are release validation requirements, not proof of a separate Worker architecture.
