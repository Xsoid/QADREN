# D-025 — Autonomous Scheduled Scenarios

**Date:** 2026-10-28. **Status:** accepted. The owner gave the decisions on 2026-10-25. **Revises:** [D-019](D-019-scenario-selection-and-deletion-modes.md) rule 4 (20-deletion threshold), [D-022](D-022-smb-in-release-1-scenarios.md) rule 7 (the Worker does not mount), [D-023](D-023-yandex-disk-to-mac.md) rule 8 (the Worker refuses Yandex Jobs), and [D-024](D-024-large-transfers.md) section 5.1 and owner answer 4. **Work:** GitHub epic Xsoid/QADREN#27.

## Context

Facts from the source audit of `beecbb7` (2026-10-25):

- A schedule slot uses a separate synchronous path in the Worker (`execute_schedule_job`). It accepts only Local Filesystem connections, scans at most 10,000 entries, and executes only no-overwrite Create. Each Update, deletion, and conflict becomes a conflict task that waits for the user.
- A manual run uses the D-024 path: resumable snapshots, a stored plan, and a durable batch. Only Create executes as a batch.
- The Settings window disables schedules for a Yandex Disk Scenario.
- The schedule model has one kind: one weekday and one time.
- Slot staging does not check for an active run of the same Scenario.
- The run thread reads due batch children in `batch_id` order. One large batch fills each page, so other Scenarios wait. Local children of all batches execute one at a time.
- The Worker does not mount an SMB share. The SMB login and the Yandex Disk token are Keychain items of the app.

The owner goal: a Scenario of any Release 1 type runs on a schedule without the user, and keeps the destination equal to the source.

## Decision

### 1. One run path

A manual run and a scheduled run use the same durable **Scenario Run**: snapshots of both sides, a stored plan, then execution of the plan items as batch children (D-024). The legacy synchronous schedule path is removed. The schedule outbox and the slot claim stay: they make sure that one slot starts at most one run.

### 2. One active run for each Scenario

1. A Scenario has at most one active run. The Registry enforces this with a constraint, not only with code. A run is active while it takes snapshots, plans, executes, or waits.
2. When a slot arrives during an active run of the same Scenario, QADREN does not start a new run and does not make a conflict. The active run continues. The slot is recorded as "skipped: a run is active".
3. There is no follow-up run after the active run ends. The next slot starts the next run.
4. A manual request during an active run shows the active run. It does not start a second run.

### 3. A source change is not a conflict

The run creates, updates, and deletes files on the destination so that the destination matches the source.

1. **One-way Scenario: the source always wins.** The rules apply in this order:

   | Source | Destination | Action |
   |---|---|---|
   | Present, added or changed | Absent | Create |
   | Present, added or changed | Unchanged | Update |
   | Present, added or changed | Changed or added | Update: the source version replaces the destination version |
   | Unchanged | Changed | Update: the source version is restored |
   | Unchanged | Removed | Create: the source version is restored |
   | Removed | Present | Delete under the deletion mode |
   | Removed | Removed | No action; the confirmed state of the path is removed |
   | No confirmed state | Same content | No action; the confirmed state is recorded |

2. A destination object that is not in the source and has no confirmed state does not change. QADREN deletes only objects that it confirmed in an earlier synchronization (D-019 rule 4).
3. **Two-way Scenario.** An Update or a deletion on one side executes automatically when the other side did not change. A change on both sides stays a conflict.
4. A name that differs from another name only by letter case or Unicode form stays `NeedsVerification` and never executes.

### 4. Consent for the one-way rule

"The source always wins" is a durable, exact Scenario rule. Its scope is the selection of the revision on the destination root. The user confirms it one time when a one-way Scenario is saved for the first time and when a revision changes the destination root or the selection. The confirmation states the consequence for that destination:

- A change made in the destination is replaced or deleted.
- The replaced version or the deleted object goes into the Trash of the connection when the connection has a `trash` capability. Otherwise the loss is permanent. An SMB share usually has no Trash (D-022 rule 10).
- To keep the replaced version, the executor first puts a hard link to the destination file into the Trash, then does the atomic replace. The replace stays atomic, and an interrupted replace loses nothing. A volume without hard links moves the previous version into the Trash first; the destination name is then absent until the new version is published.

QADREN does not store old content versions. The Trash is not a backup history.

### 5. Deletion modes

1. **Simple synchronization**: every rule deletion executes. There is no mass-deletion threshold. This revises D-019 rule 4 ("21 or more deletions wait for an explicit decision").
2. **Paranoid**: unchanged. Every deletion waits for an explicit decision. The run continues all other items.
3. **Guard that stays:** a missing, unmounted, or unreadable source root, or a source snapshot that is not complete, is an error. It never gives a deletion and never gives a replacement. This guard protects against a mass deletion after a source failure, now that the threshold is removed.

### 6. Execution of Update and Delete

1. Update and Delete are batch children with their own Job and durable attempt, next to Create. Retries follow D-024 section 4.
2. Update uses the approved replacement protocol. Before the effect, the executor reads the source metadata and the destination metadata again. A destination that changed after the plan is replaced only in a one-way Scenario under rule 3.1. In a two-way Scenario the child is stale and fails with `plan_stale`.
3. For a Yandex Disk source, Update downloads to a staging name, checks the size and MD5 against the provider metadata, then replaces. Delete acts only on the Mac or SMB side.
4. Reconciliation of an `UNKNOWN` Update compares the destination MD5 with the expected MD5. Reconciliation of an `UNKNOWN` Delete checks that the path is absent. QADREN never replays an effect.
5. After a verified effect, the confirmed state of the path is updated or removed.

### 7. Yandex Disk schedules

A Yandex Disk Scenario can have schedules. The Worker executes its runs with the token that the app gave it (D-024 stage A). Without a token, the run waits with the reason "Нужен вход: откройте QADREN" and continues when the token arrives. This revises D-023 rule 8.

### 8. The Worker mounts SMB shares

1. When a run or a snapshot needs an SMB connection and its QADREN mount directory is not mounted, the Worker mounts the share there through the NetFS framework, without a dialog. This revises D-022 rule 7.
2. The app gives the SMB login and password to the Worker over the Worker socket. The Worker keeps them in memory only, bound to `connection_id`. They never enter SQLite, logs, a command line, or a URL.
3. The Worker never unmounts a mount that it did not make. When macOS has the share mounted at another place, or a Time Machine backup uses it, the run waits with the reason. The app offers "Отключить там и подключить для QADREN" as now.
4. A refused login (`EAUTH` or `EACCES`) makes the run wait with "Нужен вход" (`smb_login_refused`). The Worker forgets those credentials and does not use the same login again until the app gives a changed one. It does not retry the login in a loop.
5. The Core mount gate (`storage_access`) stays the last check before each effect.
6. When the server does not answer, the run waits with `smb_offline`. The Worker tries again after 1, 5, 15, and 60 minutes, then every 6 hours.
7. The mount is made before a plan or an effect of the run. The Worker does not unmount after a run; the mount stays visible in Finder (D-022).

### 9. Launch at login

1. A setting "Запускать QADREN при входе в систему" registers the login LaunchAgent `dev.qadren.login` through `SMAppService.agent`. The agent starts the app executable with the argument `--login-handoff`. The argument is the signal of a login start; it does not depend on an Apple Event property of a login item. The default is off.
2. A start at login opens no window and no Dock icon: the app does the handoff before any scene exists. It waits for the Worker socket, gives the Worker the Yandex Disk token of each enabled Yandex connection and the SMB login of each enabled SMB connection, then quits. If a Keychain item is missing or locked, the app posts the notification "Нужен вход" and quits; the affected runs wait.
3. The setting explains why it is necessary: the background service cannot read the Keychain items of QADREN without an Apple Developer ID signature (D-024 stage B is deferred). It keeps the token and the login in memory only, so after a Mac restart Scenarios with Yandex Disk or a network drive wait until QADREN opens.
4. When a Yandex or SMB Scenario has a schedule and the setting is off, the schedule editor shows this limitation and a button that turns the setting on.

D-024 stage A stays in force. Stage B stays deferred.

### 10. Schedule kinds

1. **Days at a time**: selected days of the week and one local time in an IANA timezone. "Every day" selects all 7 days. Existing weekly schedules become this kind with one day. The current DST rules stay: a missing local time gives no slot, a repeated local time gives one slot.
2. **Every N hours**: N from 1 to 24, from an anchor that the user selects. Slots are the anchor plus multiples of N hours in UTC, so a DST change does not move or repeat them. Optional: active days of the week.
3. Missed slots of all kinds merge into one run for each Scenario, as now.
4. File-change monitoring is out of scope. It needs a new owner decision.

### 11. Parallel runs

1. Runs of different Scenarios execute in parallel. The run thread selects due items from each active run in turn, through an index, so that one large run does not stop the others.
2. A fixed pool of effect threads executes the items. Each connection has a limit for parallel effects. Yandex Disk keeps the D-024 limits (2 downloads, 4 metadata requests) for all runs of one connection together. *Revised 2026-11-04 (owner decision): each active run has its own lane: it reads pages of 32 of its own due items and executes them on 2 threads of its own. One shared page made a run of long or failing downloads hold back every other run for hours. The connection limits are shared by all lanes.*
3. **Write-root lock.** Two runs must not execute at the same time when the write root of one is equal to, inside, or around any root of the other on the same storage. QADREN compares canonical paths. The second run waits with the reason "ждёт сценарий <name>". The lock is durable and is released when the run ends.
4. The Scenario editor warns when the roots of a new Scenario overlap the roots of an existing Scenario.
5. Initial values: 2 effect threads for each run lane (before 2026-11-04: 4 effect threads for all runs); 4 parallel effects for each destination connection (Local Filesystem or SMB) over all lanes (before 2026-11-04: 2), and 2 downloads for each ISPmanager SFTP connection. Measure before a setting.
6. A run takes the write-root lock before it plans. The lock is the `write_lock` column of its active row in `scenario_runs`, so the end of the run releases it and a Worker restart keeps it. A waiting run stores the Scenario that it waits for (`waiting_for_scenario_id`) and has the reason `waiting_for_scenario`. A one-way Scenario writes only to its destination root; a two-way Scenario writes to both roots. Two runs that only read the same root do not conflict. Paths compare canonical and lowercased; a Yandex Disk root compares only with roots of the same Yandex connection, and an ISPmanager server path only with paths of the same ISPmanager connection.
7. JobStore and Registry write transactions start with `BEGIN IMMEDIATE`, so that parallel effect threads wait for each other instead of a failure.

## Rationale

The owner requested on 2026-10-25:

1. Schedules for Yandex Disk Scenarios.
2. A slot during an active run must not make a conflict or a restart; the active run continues.
3. A changed, deleted, or added source file must update, delete, or create the destination file. This is normal and not a conflict.
4. The Worker must mount a network drive that is not mounted.
5. Daily schedules and schedules every N hours.
6. Many Scenarios can run in parallel.

Owner answers on 2026-10-25: in a one-way Scenario the source always wins; the mass-deletion threshold is removed; a slot during an active run is skipped without a follow-up run; a launch-at-login setting with an explanation is added.

## Safety invariants that stay

A durable attempt before each effect, no-overwrite for new files, the approved replacement protocol, `UNKNOWN` reconciliation without a blind replay, the Trash of the connection where it exists, no secrets in SQLite, Git, logs, or URLs, and a missing root is an error and never a deletion.

## Affected documents

- [Data safety](../policies/data-safety.md)
- [Synchronization model](../architecture/backup-model.md)
- [Local Worker runtime](../architecture/local-worker-runtime.md)
- [Storage adapters](../architecture/storage-adapters.md)
- [Settings window](../design/settings.md)
- [Roadmap](../roadmap.md)
- `AGENTS.md`, `llms.txt`
