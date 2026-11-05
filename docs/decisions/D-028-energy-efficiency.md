# D-028 — Energy efficiency

**Date:** 2026-11-04. **Status:** accepted. The owner gave the decision on 2026-11-04. **Extends:** D-014 (minimum components and background processes) and the resource targets in [technology stack](../architecture/technology-stack.md).

## Context

- QADREN runs long Scenario Runs on a MacBook, often on battery.
- A profile of a real run found that the Worker used two to three CPU cores. Most of the time went to `PRAGMA quick_check` on the Registry. Run code opened the Registry for each file, three or four times, and each open read every page of the database. The transfer itself used a small part of the time.
- The Registry used the `DELETE` journal mode. The Worker run threads waited for one another on the database lock.

## Decision

1. **Priority.** Energy efficiency is a system-wide priority in the design and the code of QADREN. It applies to the Rust core, the Worker, and the macOS app.
2. **Safety first.** Energy efficiency never overrides a safety invariant. Durable attempts, the operation scope, no-overwrite, and `UNKNOWN` reconciliation stay as they are. Durable writes keep `synchronous=FULL`.
3. **Cost for each item.** The work for one file or one plan item must not increase with the size of a database or the total count of items. A per-item path must not do a full scan, a full integrity check, a full sort, or a full rebuild.
4. **One-time work.** A check of a full database runs once for each path in a process. A store, a connection, or a provider session opens once for each run, or comes from a pool. It does not open again for each file.
5. **Contention.** A database that several threads or processes use at the same time uses the `WAL` journal mode. Parallel work is for I/O and network wait. Parallel threads must not compete for one lock or do the same bookkeeping.
6. **At rest.** Without an active run, CPU use is close to zero. Use events, timers with long intervals, or backoff. Do not poll tightly. The interface redraws only when its data changes.
7. **Growth.** Bookkeeping rows for a run are pruned when the run no longer needs them. A database does not grow without limit.
8. **Measurement.** A change to a run path, a store, or a polling loop is checked with a CPU profile of a real run (for example `sample` or Instruments). Evidence from real storage states only the storage type and the result ([data safety](../policies/data-safety.md)).

## Consequences

- `Registry::open` runs `quick_check` and sets the `WAL` journal mode once for each path in a process. SQLite still reports corruption that it finds later.
- New code review includes a check for per-item cost (rules 3 and 4).
- An existing per-file open of a store is a defect. Fix it when you change the related code.
