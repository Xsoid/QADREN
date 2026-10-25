# D-024 — Large Transfers

**Date:** 2026-10-25. **Status:** accepted on 2026-10-25, with the owner answers in "Owner answers". **Builds on:** [D-023](D-023-yandex-disk-to-mac.md) (Yandex Disk to Mac), [D-022](D-022-smb-in-release-1-scenarios.md) (SMB network drive), [D-019](D-019-scenario-selection-and-deletion-modes.md) (selection and deletion modes).

## Context

The owner goal is to transfer very large volumes, much more than 10,000 files, from Yandex Disk to a Mac folder or an SMB network drive. The processes must not stop the transfer, and the product must not stop at fixed limits.

Facts from 2026-10-24 and 2026-10-25:

- **Volume.** The selected folder `disk:/Xsoid [Архив]` has more than 61,946 files in more than 1,806 folders, and more than 160 GB. The metadata walk stopped at a test cap of 60,000 files after 19 minutes.
- **Snapshot.** A plan snapshot is one recursive metadata walk in one FFI call. Any error discards the full walk. Nothing is kept for the next run.
- **Plan limits.** A dry run accepts at most 10,000 entries and 1 MB of JSON for each side (`registry_ffi`). The local snapshot, the Worker schedule scan, and the Yandex snapshot also stop at 10,000 entries. The plan screen shows one row for each file.
- **Worker.** The Worker handles IPC requests one at a time on one thread. During a batch it does not answer `status` and does not poll schedules. The app treats a Worker that does not answer as not started.
- **Token.** Only the app reads the Yandex Disk token from the Keychain. The background Worker cannot read it, so it refuses Yandex Jobs without a token from the app (D-023).
- **Provider errors.** In a batch of 111 files, Yandex Disk returned HTTP 500 for the content of 17 files, again and again. 4 other files failed once and downloaded correctly later. A connection timeout to the download host also occurred.
- **Retries.** A failed batch child stays failed. Nothing retries it.
- **Reconciliation.** Reconciliation of an `UNKNOWN` copy checks only the size of the destination file. On 2026-10-25 it marked 24 old Jobs as completed because a later batch had written files of the same size at those names.
- **Signing.** The developer build has an ad-hoc signature. Every build changes the Worker cdhash, and the app must register the LaunchAgent again (see [build and release](../development/build-and-release.md)).

## Decision

### 0. Scale target

The owner target (2026-10-25) is hundreds of thousands of files in one Scenario, and the system must not fail or stall at that size. The design target is **1,000,000 files for each Scenario side**. These rules apply to every part of a large transfer:

1. **No full list in memory.** No component loads all files, all plan rows, all batch children, or all Jobs into memory or into one FFI response. Each reads pages or streams rows in path order.
2. **Bounded memory.** The Worker uses bounded memory that does not grow with the number of files (target: less than 300 MB resident at 1,000,000 files). The app uses memory for one visible page, not for the full data set.
3. **Indexed queries.** Each periodic query (due items, unknown Jobs, unresolved attempts, progress counts) uses an index and reads only the rows it needs. A poll or a progress refresh never scans all items.
4. **Durable progress.** A walk, a plan, and a run continue after a crash or a restart from their stored state. No step must start again from the beginning because of an interruption.
5. **Responsive interface.** The IPC thread answers in less than 1 s at every size. Progress shows counts from the database, not from a list of items.
6. **Measured.** Each stage is checked with a generated data set of at least 300,000 files, with time and peak memory recorded.

### 1. The Worker owns long runs

1. A **Transfer Run** is one durable execution of one Scenario revision: its snapshots, its plan, and its items. Each item is one file effect and uses one Job with the existing durable attempt model. There is still one JobStore.
2. The Worker executes a run on a **background thread**. The IPC thread answers `status`, progress, pause, and cancel requests at all times.
3. A run continues after the app quits. After a Worker restart, the Worker continues a run from its durable state. It never starts a second attempt for an item that has a Job.
4. A run executes at most 2 items in parallel for each Yandex Disk connection. Each item has its own Job and attempt.
5. Pause and cancel take effect after the current file. A cancelled run keeps its completed items.
6. The app registers the Worker again for a new build only when no run is active. A stop of the Worker for any other reason makes the running item `UNKNOWN`, as now.

### 2. Snapshot

1. A **snapshot** is a durable, resumable metadata walk of one Scenario side. The Worker stores it in a separate SQLite database next to the Registry. It is a metadata cache: it holds paths, sizes, MD5 values, and revisions, never file content and never a token.
2. The walk stores a queue of folders and each finished folder page. After an error or a restart, the walk continues from the last finished page.
3. A plan uses only a **complete** snapshot. A partial snapshot never produces a plan or an effect.
4. The walk lists only the selected objects of the Scenario (already in Core since 2026-10-25).
5. The Yandex walk uses pages of 1,000 entries and at most 4 parallel requests. HTTP 429 pauses all requests of that connection and increases the pause.
6. The executor still reads the source metadata of each file again before its effect. A snapshot is evidence for the plan, not for the effect.
7. Implementation (2026-10-25): `snapshots.sqlite3` next to the Registry, WAL mode, written only by the Worker snapshot thread. One snapshot is one key: connection, root, and Scenario selection. The selection applies to both sides, so a destination is not walked outside the selected objects. A selected object of unknown type is probed: a file becomes one row, a folder enters the queue. A transient error (network, HTTP 429 or 5xx, a missing SMB mount) delays the walk for 30 s, doubled up to 30 min; HTTP 401 waits for a new token; HTTP 403 fails the snapshot. A new walk of a key keeps the last complete snapshot until it completes; older snapshots are deleted. IPC: `start_snapshot` and `snapshot_status` for a Scenario revision.

### 3. Plan

1. Core computes the plan in the Worker from two complete snapshots and stores it in SQLite: one row for each path with an action, plus counts for each action.
2. The 10,000-entry and 1 MB limits do not apply to this path. The plan is computed as a stream: the Worker merges the two stored snapshots and the stored baselines in the order of the name key (NFC, lowercase), so names that can name one file are adjacent, and it writes plan rows in pages. The mass-deletion count is a database count, not a list. The supported size is 1,000,000 entries for each side.
3. The plan screen shows counts and the total size for each action, and the rows page by page, with a filter by action and a search by path. It does not build one view for each file.
4. "Создать все" confirms one run with the counts that the user sees. It does not send a list of paths through the app.
5. Replacements and deletions keep their confirmations (D-018, D-019). A mass deletion still needs its explicit confirmation of the proposed set.

### 4. Retries

Each item failure has one of these classes:

| Class | Examples | Rule |
|---|---|---|
| Transient, before any effect | HTTP 429, HTTP 5xx from the API or from the download host, connection or read timeout, network loss | Retry automatically after 1 min, 5 min, 30 min, 2 h, and 12 h. After that the item is "failed after retries". |
| Content not available | HTTP 500 from the download host for the same file at every retry | The same as transient. The run report shows the provider reason separately. |
| Permanent | HTTP 401 or 403, `not_found`, a source that changed after the plan, an existing destination for a new file | No retry. The item fails with its reason. 401 pauses the run until the user signs in again. |
| Effect not proven | `UNKNOWN` after a possible publish | Never retry. Run read-only reconciliation (section 6). |

A retry is safe only because the executor closes the attempt as "failed before effect" when no content was published. The retry uses a new Job and attempt for the same item.

**Retry until success.** A shared setting "Повторять недоступные файлы до успеха" in Application Settings (stored in the Registry, read by the Worker) continues the retries after the schedule: every 24 h, until the item succeeds or the user pauses or stops the run. It applies to the transient classes only, never to a permanent failure or an unproven effect. The owner requested it on 2026-10-25 because the Yandex Disk storage did not serve some files after the data-center incident; it is on by default.

### 5. Worker access to the token

1. **Now (stage A).** The app sends the token to the Worker when it starts or resumes a Yandex run. The Worker keeps it in memory only, for that run. After a Worker restart the run waits with the state "Нужен вход: откройте QADREN". When the app opens, it sends the token again and the run continues. A scheduled Yandex run also waits for the app.
2. **Target (stage B, deferred).** The owner has no Apple Developer Program account (2026-10-25), so stage A stays in force until a new owner decision. Stage B: sign the app and the Worker with one Apple Team ID (Developer ID). Store the token in a Keychain access group that the app and the Worker share. Then the Worker reads the token itself, and runs and schedules continue without the app. The same signature removes the cdhash binding of the LaunchAgent to each ad-hoc build.
3. QADREN stores the OAuth refresh token in the Keychain, bound to `connection_id`. The Worker renews the access token before it expires.
4. Rejected: a token file on disk, a token in SQLite, and a Keychain ACL that names an ad-hoc Worker binary (it breaks at every build).

### 6. Integrity and reconciliation

1. A Yandex download publishes a file only after its size and MD5 match the provider metadata (already true).
2. Reconciliation of an `UNKNOWN` Yandex download reads the local destination file and compares its MD5 with the expected MD5 of the attempt. Only a match completes the Job. A size match alone keeps `UNKNOWN`. This read is an ordinary content read (D-018).
3. On smbfs the inode number follows the name (D-022). Reconciliation must not use it as file identity there.

### 7. Observability

1. The Worker writes error kinds to `worker.log` (since 2026-10-25), never paths, tokens, or links.
2. Each run has a report: counts for each class, the failed items with their reasons, and the retry schedule. The report stays after the run ends.

## Order of work

1. Done (2026-10-25): Worker background run thread, durable runs with retries, token in memory (stage A), no re-registration during a run.
2. Done (2026-10-25): scale pass on stage 1: indexed and paged due items, progress counts from the database, a single-row check before each item, the newest 200 Jobs and counts for the monitoring screen, indexed unknown-Job and attempt queries. Checked with 300,000 batch children and 300,000 Jobs.
3. Done (2026-10-25): snapshot store and resumable Yandex and local walks. Checked: 300,000 generated local files in 3,031 folders in about 7 s; the real Yandex Disk folder "Xsoid [Архив]" (143,426 files, 8,197 folders) in 16.7 min with 4 requests in flight; `kill -9` during each walk, then the same snapshot continued without loss; Worker memory 11 MB. One name with a control character became a skipped object instead of a failed walk.
4. Streaming plan, plan store, and the paged plan screen; a run starts from the stored plan, not from a path list in the app.
5. MD5 reconciliation for Yandex downloads.
6. Deferred: Developer ID signing and the shared Keychain access group (stage B). It needs an Apple Developer account and a new owner decision.

Each step keeps the safety invariants of AGENTS.md: durable attempt before an effect, no-overwrite for new files, the replacement protocol, and `UNKNOWN` without a blind retry.

## Owner answers (2026-10-25)

1. There is no Apple Developer Program account. The Worker keeps the token in memory (stage A). Stage B is deferred.
2. 4 parallel metadata requests and 2 parallel downloads for each Yandex Disk connection are accepted.
3. The retry schedule (1 min, 5 min, 30 min, 2 h, 12 h) is accepted.
4. Deletion on the Mac side for a Yandex Disk source is a separate step, outside this decision.
5. A setting must let the Worker retry an unavailable file until it succeeds (added on 2026-10-25; see "Retry until success" in section 4).

## Consequences

- The storage adapter contract, the Yandex Disk adapter contract, the Worker runtime document, and the roadmap refer to this decision.
- Without stage B, a Yandex Disk run continues only while the Worker that received the token runs. After a Worker restart or a Mac restart it waits until the app opens.
- The existing batch receipt stays for small plans until the run model replaces it.
