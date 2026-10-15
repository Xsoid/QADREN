**Current Acceptance D-018 / 2026-10-14:** Release 1 P0 UI/foundation and P1–P2 local functionality **do not include the Yandex Disk controller** and do not require an available API, OAuth/live E2E, or Apple iCloud. Yandex is moved to post-release scope; iCloud is outside the active scope. Old tables below, marked cloud-first, refer to the historical plan and shall not block the local release; essential local tests/consent/no-overwrite/UNKNOWN/recovery remain mandatory. The current `main` after push contains more code than the baseline of this chronological matrix: see [current state audit](current-state-audit.md).

# Release 1 — Verification Matrix and Evidence

Status: working control document for [Epic #12](https://github.com/Xsoid/QADREN/issues/12), [#13](https://github.com/Xsoid/QADREN/issues/13), [#25](https://github.com/Xsoid/QADREN/issues/25).

A mark of "implemented" or successful compilation **does not** replace behavior verification. A test in a mock store is not a verification of the real Keychain/cloud.

**Historical Scope Release 1 (2026-10-12, replaced by D-018):** upload/download and synchronization between Yandex Disk and local directories. This is no longer the acceptance for the current local release: Release 1 is limited to Local↔Local; the Yandex Disk controller is post-release. iCloud adapter/container/sync/backup, native spike, and E2E are not acceptance. Mentions of iCloud File Provider below refer to Local FS security, not iCloud integration. [D-017](../decisions/D-017-defer-icloud.md), [D-018](../decisions/D-018-local-first-priorities.md).

## Local controller mkdir slice (2026-10-14)

- `qadren_jobs_run_local_directory_create` executes one explicit `local_mkdir_new` through the Rust `JobStore`: it persists a bound attempt before `mkdir`, fails on an existing target without overwrite, verifies the resulting directory metadata, and completes atomically with the receipt.
- `LocalWorkspace` offers `Create Folder` only after the user has selected both a local root and an enabled `StorageConnection`; the real `connection_id` is recorded in the Job attempt, so normal disconnect/remove gating can see it. Rust independently reopens the Registry and rejects missing, non-Local, or disabled connections before I/O. The native bridge test `bash scripts/test-macos-local-workspace.sh` creates a unique test root, verifies the listed directory, and confirms collision rejection.
- The legacy `qadren_local_mkdir` ABI symbol is fail-closed. It returns `local_mkdir_requires_job` and does not create a directory. New directory creation must use the durable JobStore command above.
- This is deliberately **not** content copy or Local↔Local synchronization. macOS content reads remain fail-closed because opening a File Provider placeholder could materialize it. Unknown post-effect outcomes are not retried blindly.

## Native UI acceptance utilities — #26 (2026-10-15)

- `./script/build_and_run.sh --verify --theme light` builds the real unsigned `dist/QADREN.app`, starts it with a process-only `light` override, and confirms the QADREN process. The override is a launch argument and does not change the user's durable theme preference; `dark` and `system` are available by the same path.
- A light-theme native dashboard inspection found that a nonempty Registry could incorrectly give the Connections widget a green ready indicator while all entries were `offline` or `disconnected`. The widget now derives its aggregate and row status from the enabled Registry entries: error and authorization states take priority; all unavailable entries are offline; mixed statuses are unknown; all disabled entries are paused.
- Current evidence is build/process verification and the deterministic Dashboard layout runner. Repeat native screenshots of the Monitoring window after the status correction, narrow-window review, and keyboard/VoiceOver checks remain **BLOCKED**, not passed, because the desktop automation could not retain that independently opened window in focus.

## Dispatcher workspace shell (2026-10-12)

- The macOS client no longer collects all technical elements on one scrollable page. Root navigation is divided into two spaces: `Dashboard` for monitoring and operational actions, and `Scenario Setup` for connections, future rules, and Worker assignment. The queue and dual-pane file manager open within the dashboard, rather than becoming competing root sections.
- Full paths of local panels are available for selection; two independent panel contexts and multiple selection are preserved. This is still a metadata-only view: content is not opened or previewed.
- The shell does not claim readiness of missing Release 1 parts: there is no Scenario execution, content synchronization, secure copy/move/rename/delete Jobs, saving of panel/window configurations, timeline, Job tree, or customizable widgets. These remain separate acceptance packages.
- Verification of the first version of this change: `bash scripts/test-core.sh` (208 Rust tests) and `bash scripts/build-macos.sh` on macOS are successful; the last command built `dist/QADREN.app`. The subsequent adjustment below replaced the original sidebar navigation.

### Independent Spaces and Windows (2026-10-12)

- The sidebar is removed. Switching spaces is performed via the `Space` menu in the macOS toolbar.
- `Dashboard` — `WindowGroup`: each open monitoring panel receives an independent SwiftUI state and can be created by the `New Panel` button, then placed by the user on another screen. QADREN does not synchronize the configuration of such windows between devices.
- `Scenario Setup` — a separate singleton window; it opens from the same menu and does not mix with the operational monitoring panel.
- Verification: after `bash scripts/build-macos.sh`, the accessibility tree of the open application confirmed the absence of a sidebar, the presence of the `Space` menu, and the `New Panel` button in the toolbar. There is no automatic test for placement on multiple physical displays.

### Menu Bar Menu (2026-10-12)

- The QADREN system menu uses the same SwiftUI `openWindow(id:)` as the toolbar. It does not search for arbitrary `NSApp.windows` and therefore does not depend on the presence of a legacy singleton window after the transition to `WindowGroup`.
- The menu provides `Open Dashboard`, `New Dashboard`, and `Scenario Setup`; these are the same independent scenes that open from the application interface.

| Area | Automatic Verification | Real Verification | State as of 2026-10-09 |
|---|---|---|---|
| Rust Registry | `scripts/test-core.sh`, migration v1→v2, FFI, different account IDs | Uncorrupted SQLite after installation update | Rust CI successful on `ff585c7`; update of real installation not verified |
| macOS SwiftUI | Compilation and .app verification in Actions | UI launch and window state restoration | CI successful; manual smoke test missing |
| Keychain remove | Swift fault injection: disable / Job attempt gate / Keychain / Registry failures, retry, second account | Forced failure of system Keychain and retry after restart | Local runner passes on 2026-10-12; system verification missing |
| Legacy OAuth | Code verification for legacy save until successful import | Import of old token on real Mac and rollback | Real verification missing in this cycle |
| Yandex metadata | Rust mock HTTP tests | Authorization, quota, 2 accounts, offline/401 | Historically confirmed in documentation; not re-verified in this cycle |
| Content / Yandex / Local FS / Worker | Partial Local FS and Yandex content primitives exist; no full Job execution | Real Yandex↔local directory and Local↔Local, consent, integrity and recovery | PARTIAL: full E2E not confirmed; iCloud deferred |
| Resources | CI records .app size, no idle RAM/CPU measurement | Activity Monitor, `ps`, network bytes, sleep/wake | Baseline not measured |
| Signing and Delivery | CI builds unsigned bundle | Gatekeeper, entitlement, notarization, clean install | Future Release 1 only |

## Mandatory Invariants

- QADREN does not upload cloud file content to the primary MacBook until explicit consent for a specific operation is given.
- `connection_id` is the identifier of an independent connection; `provider_account_id` may match.
- Removing a connection **never deletes provider files**; a failure between the Registry and Keychain does not lose the ability to safely complete local cleanup.
- An unfinished, unconfirmed effect does not automatically become `SUCCESS`.
- No runtime BackupSet and no internal file version history.
- Existing Apple system background downloads are not blocked.

## Before Closing #13

- [x] `main` CI passed on [ff585c7](https://github.com/Xsoid/QADREN/actions/runs/37857950583), including the new Swift test runner.
- [ ] Perform smoke test of signed/local UI on primary Mac without affecting user data.
- [ ] Confirm Keychain retry and legacy migration on real Mac with secure test connections.
- [ ] Measure idle CPU/RAM, disk/network, and record macOS, Xcode, and Rust versions.
- [ ] Enter discovered defects into separate Issues; do not present future iCloud support as current.

## Connection cleanup gate — #25 (2026-10-12)

- Remove first sets the connection to `disabled`, then executes a Rust read-only gate by `connection_id`. An active/UNKNOWN attempt, a legacy attempt without target binding, or JobStore unavailability stops removal before the Keychain: the disabled Registry record and credential remain for recovery/reconciliation.
- `bash scripts/test-core.sh` (fmt, Clippy, 197 Rust unit tests) and `bash scripts/build-macos.sh` passed locally. A separate Swift fault-injection runner verified the normal flow, failure at the Job gate, Keychain and Registry, retry after each failure, and independence of a second connection.
- Real macOS Keychain lifecycle is confirmed by a separate test-only runner: write, read, update duplicate item, delete, and repeated delete for an unpredictable `connection_id`; the runner does not access the Registry, provider, or user credentials. Forced system Keychain failure, restart between disable and cleanup, and live Scenario/adapter attempt were not performed. Therefore, #25 remains open and is not a confirmation of the readiness of the actual disconnect UX.
- The legacy `qadren_registry_remove` ABI symbol is fail-closed. It returns `registry_remove_requires_connection_workflow` and does not delete a Registry record. Remove and Disconnect must use the existing `ConnectionRemoval` workflow, which disables the connection and checks unresolved Jobs before local cleanup.

`YandexConnection.disconnect()` uses the same fail-closed protocol: disable → read-only attempt gate → Keychain delete → single SQLite finalization (`disconnected`, credential reference cleared, connection available again for explicit reauthorization). If a failure occurs before finalization, the record remains disabled; deletion of cloud files and access to the provider are absent. The actual results of this change are recorded with the SHA in #25.

The local `local_filesystem` connection no longer enters the Keychain-only branch: Remove uses disable → same read-only attempt gate → deletion of Registry record only. Keychain, local user files, and provider are not affected. In case of an unresolved attempt, the record remains disabled for reconciliation/retry. The Swift fault-injection runner additionally verifies normal and blocked local flows.

## Scenario metadata-only dry-run planner — #19 (2026-10-12)

- Rust `scenario_planner` builds a deterministic plan only from allowed metadata snapshots and the last confirmed `FileSyncState`; it does not perform I/O, does not create Jobs, and does not change the baseline.
- `Create`/`Update` remain proposals. Deletion always requires a separate policy decision; changing the destination and simultaneous changes result in a conflict. Equality of name/size is not considered byte equality: in the absence of a trusted checksum or provider revision, the result is `NeedsVerification`.
- `bash scripts/test-core.sh` with 202 Rust tests passed locally. There is no adapter scan, persistence, FFI/UI, Job submission, or E2E; #19 remains PARTIAL.

### Registry-backed preview composition — #19 (2026-10-13)

- `Registry::plan_scenario_dry_run` loads only the immutable Scenario revision and its revision-bound confirmed `FileSyncState`, then passes caller-supplied metadata snapshots to the pure planner. The test confirms that a subsequent change in direction does not change the preview of the old revision and the preview does not write the baseline.
- Rust C ABI `qadren_scenarios_dry_run` accepts only caller-supplied JSON metadata snapshots and exact `scenario_id`/revision, then returns a deterministic plan with explicit `create`, `update`, `conflict`, `needs_verification`, or `delete_requires_decision`. FFI limits each JSON payload to 1 MiB and a total of 10,000 entries, does not scan adapters, does not read content, does not create Jobs, does not change FileSyncState, and does not write policy decisions. Incorrect/too large JSON or revision 0 are rejected before Registry planning. UI for this FFI has not been added yet.

- The boundary is intentional: no adapter scan, UI, content reading, Job submission, policy decision, or FileSyncState writing. If the Registry or planner cannot prove a correct plan, the method returns an error; execution does not start. #19 remains PARTIAL until real authorized metadata scan and a separate execution path are implemented.

## Automation coalescing baseline — #21 (2026-10-12)

- A pure Rust state machine coalesces a burst of automatic triggers into one pending run and keeps exactly one follow-up while a Job is active. A disabled/locally paused Scenario ignores automatic triggers, while a manual request remains admissible.
- SQLite v5 stores only the local pause and one coalesced pending trigger. An active Job is deliberately not duplicated: its durable ownership remains with the JobStore and must be passed to the future scheduler when evaluating a trigger. Disable or local pause clears the pending automatic run; neither of these operations creates or changes a Job.
- A pure Rust evaluator for a weekly schedule based on IANA timezone is added. `chrono-tz` embeds only the timezone database into the Rust binary, not a network service or runtime for the user. A repeating DST slot receives one stable local key; a non-existent slot does not create a launch.
- SQLite v6 stores several validated schedules per Scenario (IANA timezone, weekday, time). This is a configuration-only layer: time evaluation, catch-up, and Job creation are not yet connected.
- SQLite v7 atomically claims evaluated occurrence keys. A repeated DST slot and a repeat after restart return an already occupied key and cannot produce a second future Job.
- `Registry::due_schedule_occurrences` reads enabled schedules only for enabled/unpaused Scenarios and deterministically returns DST-safe candidates for a given UTC interval. The C ABI accepts explicit RFC 3339 UTC bounds and also remains read-only; durable submission is added later via a separate Registry outbox.
- Settings shows up to five real candidates for the next 14 days for the selected Scenario via this C ABI. An empty list honestly means there is no suitable enabled/unpaused schedule in this window; the screen does not create Jobs, does not claim occurrences, and does not perform adapter/content I/O.
- Settings also shows and changes the durable local pause of Scenario automation via a narrow FFI. The pause clears only the pending automatic trigger, is saved in the Registry, and excludes the Scenario from candidate evaluation; existing Jobs are not changed, and manual requests remain a separate path.
- `scripts/test-macos-scenario-schedules.sh` creates a separate temporary Registry and verifies the Swift/FFI bridge: an enabled weekly UTC schedule returns exactly one expected slot, the pause is saved and excludes the candidate, and a repeated read-only request does not change the occurrence. `bash scripts/test-core.sh` with 231 Rust tests, native runner, `bash scripts/test-macos-job-inventory.sh`, `bash scripts/test-macos-keychain.sh`, and `bash scripts/build-macos.sh` passed locally. A restart test confirms the preservation of pending/pause. At that time, there was no catch-up orchestration, event debounce, priority aging, or Job submission; the outbox bridge is added below. #21 remains PARTIAL.

### Durable schedule outbox — #21 (2026-10-14)

- Registry schema v9 atomically records a DST-safe schedule receipt and one pending Job outbox item for the immutable current Scenario revision. A slot is not consumed without a durable future submission request.
- `schedule_dispatch::dispatch_pending_schedule_jobs` reads only this outbox and enqueues the exact Scenario-bound Job in the existing `JobStore`; it does not execute a Scenario, touch adapters, read content, or start a timer.
- Registry and JobStore deliberately remain separate SQLite databases. A restart after the JobStore commit but before the outbox acknowledgement observes the same Job identity/revision and marks the request dispatched without creating another Job. A mismatching existing Job fails closed.
- This is not a scheduler service or transfer path: catch-up policy/window ownership, event debounce, queue priority/aging, FFI/UI runtime, and Scenario execution remain unimplemented.

### Scheduler dispatch C ABI — #21 (2026-10-14)

- `qadren_scenario_schedules_dispatch` accepts explicit RFC 3339 UTC bounds plus Registry and JobStore paths, stages every due slot, then drains the durable outbox through the same Rust dispatcher.
- The ABI returns only `staged` and `dispatched` counts. It creates queued revision-pinned Jobs and does not execute a Scenario, touch any adapter, read content, or imply transfer consent.
- The ABI regression proves first invocation stages and queues one Job and a repeated invocation returns zero/zero; this preserves restart idempotency at the native boundary.

## Scoped destructive-operation consent — #20 (2026-10-12)

- Pure Rust policy gate permits destructive operation only with explicit accepted decision for exact operation, connection, relative path and term. No decision, deny, expiry or scope mismatch means fail-closed.
- `bash scripts/test-core.sh` with 208 Rust tests and `bash scripts/build-macos.sh` passed locally. No durable decisions, UI prompt, adapter pre-I/O recheck or E2E; #20 remains PARTIAL.

The next minimum durable layer stores the decision only for Job in `waiting_for_decision`: SQLite v8 links it to exact operation/connection/relative path and term. Accepted decision is consumed atomically exactly one time; denied, expired, repeated or scope-mismatched decision blocks the operation. Write/consumption does not perform adapter I/O; if the process terminates after consumption and before future effect, Job remains waiting, and a repeat requires a new explicit decision. UI prompt, adapter pre-I/O recheck and real destructive operations are still not implemented.

## Local metadata workspace — #22 (2026-10-12)

- User selects a local folder for the current session; Rust FFI shows only metadata entries and creates a subfolder. Content read, copy, move and delete are not available.

Workflow: small commits directly to `main`, mandatory waiting for CI and registration of constraints; documentation branches are not created.

## Local Filesystem Adapter — #15 (2026-10-11)

- User confirmed on macOS for `f5f6bab`: `bash scripts/test-core.sh`, 95/95 Rust unit tests, 0 failures; formatting and Clippy passed. This does not confirm future rename/move/delete operations and is not E2E iCloud.
- In `340ab81` bypass of directory check via `list_page(..., limit=0)` is removed: even an empty page requires full directory verification. Added test for missing path, regular file and symlink.
- `340ab81` has **not passed confirmed CI/macOS tests** yet. Consider it requiring verification, do not close #15.
- The following remain prohibited: automatic materialize/download on main MacBook, unsafe rename/move/delete and use of unfinished adapter for destructive Jobs.

### Root confinement hardening (2026-10-11)

- After confirmation of 95/95 unit tests on `98d79dd`, Unix root opening is changed in `a657bb4`: now path is traversed by components via `openat(O_DIRECTORY|O_NOFOLLOW)`, because `O_NOFOLLOW` on full path protects only the last component.
- `dc0bdbf` adds negative test for intermediate symlink in root path; upon failure, no traversal outside the specified directory shall occur.
- New commits require fmt/Clippy/macOS tests verification. Safe rename/move/delete and iCloud residency check are still not ready; prohibition of content read on main Mac is preserved.

### Root confinement confirmation and new publication race (2026-10-11)

- User executed `scripts/test-core.sh` on `acad913`: fmt and Clippy passed, 96/96 unit tests and 0 doc-tests successful.
- `744856c` adds test for **final name** substitution after `linkat`: operation shall return `OutcomeUnknown`, preserve foreign final file and original bytes by moved name. This is regression / fault injection, **not proof of atomicity against active attacker**.
- New test is not yet confirmed on macOS. Safe move/rename/delete and iCloud placeholder materialization remain blockers for #15.

### Verification after 97/97 and additional staging protection (2026-10-11)

- User confirmed `7aa841d` on MacBook: `scripts/test-core.sh` — fmt/Clippy clean, 97/97 unit tests passed, 0 doc-tests.
- `20b19bb`: after staging name removal and directory synchronization, inode of final name is additionally re-verified with expected link count 1. Mismatch returns `OutcomeUnknown`, not false success.
- `f00a0e8`: negative test of external hardlink to staging inode during read. Publication shall be rejected and foreign link shall not be deleted.
- Current two patches are **not yet confirmed by macOS tests**. Verifications are not a guarantee of protection against hostile process with concurrent write access to the same directory. Until its design, unsafe move/rename/delete and automatic content read on macOS remain disabled.

### Local adapter capabilities (2026-10-11)

- User checked `805ce3d` on MacBook: fmt/Clippy successful, 98/98 unit tests passed, doc-tests 0.
- `9deeb67` adds `LocalFilesystem::capabilities()` based on actually enabled functions: content read — Linux only, creation of new files and directories — Unix, delete/rename operations still prohibited. For Windows, only metadata is supported, others fail-closed.
- Added capabilities consistency test. New check is not yet performed on macOS; capabilities do not cancel requirements for individual consent for content read.

### Scoped local copy — #15 (2026-10-11)

- User confirmed `df2fff5`: `scripts/test-core.sh`, fmt/Clippy without errors, 99/99 Rust unit tests, 0 doc-test failures.
- `44909b5` adds `copy_file_new(from, to, consent)`: Linux reads via pinned descriptor and publishes only new file without overwrite; lack of consent prohibits read and creation. On macOS, method intentionally fail-closed even with consent before File Provider placeholder residency verification; do not consider this path ready for macOS.
- Added negative test for lack of consent, existing destination and data immutability; for Linux also successful local copy.
- New code is not yet confirmed by macOS CI/local run; operation is not connected to Job Engine. Rename/move/delete, macOS content residency and real fault-injection trials remain open.

### Local copy safety regression (2026-10-11)

- User confirmed `ca747a0`: `scripts/test-core.sh`, fmt/Clippy passed, **100/100** Rust tests, doc-tests (0) successful.
- `38d5c99` adds negative test `copy_file_new`: rejection of source/destination with `..`, missing source and source symlink (Unix), without target file publication or source content change.
- Additional test is not yet checked on macOS. On macOS content read/copy remain fail-closed until residency check; untrusted moves, renames and deletes are not implemented.

### Copy source consistency check (2026-10-11)

- User confirmed `58d4f9b`: `scripts/test-core.sh`, 101/101 unit tests, fmt and Clippy successful.
- `7821669`, `2358b8b`, `cf88625`: after Linux-only copy, size and modification time of original open file are checked. Change during copy is classified as `OutcomeUnknown`, target file may exist and shall not be automatically overwritten. Added Linux-only regression test for source file change.
- Metadata check **is not snapshot isolation or hash check**: rewriting with same size and modification time may remain unnoticed. New checks on Mac and Linux are not yet confirmed. macOS content read is still disabled.

### Source verification hardening during Linux copy (2026-10-11)

- User confirmed `8dfdcf5` on macOS: fmt/Clippy clean, 101/101 tests, doc-tests 0.
- `b6f265e` checks open source inode (`dev`, `ino`), length, `mtime` and `ctime` with nanoseconds and number of transferred bytes. Added Linux-only test for incomplete transfer detection.
- On mismatch after publication, `OutcomeUnknown` is returned; automatic repeat is not permitted.
- **Limit:** metadata checks do not provide snapshot and do not guarantee detection of byte substitution with restored metadata. New Linux tests are not verified in real Linux environment; macOS does not execute this `cfg(target_os = "linux")` path.

### Linux streaming read consistency (2026-10-11)

- User confirmed `c6f83fd` on macOS: `scripts/test-core.sh`, 101/101 Rust unit tests, fmt/Clippy without errors.
- `66a067d` adds check after Linux `read_to_writer` for preservation of metadata of **the same open source inode** and number of actually transferred bytes; detected mutation returns `OutcomeUnknown`, because receiver might have already accepted data.
- Added Linux-only test with Writer that changes source file during streaming read. This code and test **not executed on Linux**; macOS does not execute it and continues to prohibit file content read.
- Inode/mtime/ctime/length check is not a consistent snapshot and does not prove byte identity.

### Nested local directory creation (2026-10-11)

- User confirmed `571de26`: `scripts/test-core.sh`, 101/101 macOS Rust tests, fmt and Clippy passed.
- `1fc6ab4` / `346205b` add `LocalFilesystem::create_directory_tree` with step-by-step creation of missing nested folders inside pinned root without symlink traversal, without replacing existing objects. Result is number of created directories.
- If after creation of at least one directory the next operation ends with error, `OutcomeUnknown` is returned; already created folders are preserved for reconciliation, automatic rollback is absent. Tests check successful/repeated creation, failure for file in path, traversal and partial success with path component length 300 characters.
- New functional slice **not yet verified on macOS**; no destructive operations or macOS content read are included.

### Concurrent directory creation (2026-10-11)

- User confirmed `fdf43a9` on Mac: `scripts/test-core.sh`, 103/103 tests, formatting and Clippy passed.
- `e8f9c27`: `create_directory_tree` upon `AlreadyExists` after check for component absence re-verifies that a directory appeared in pinned root. Symbolic links and regular files are not considered successful creation.
- Added Unix test confirming that link inside root does not allow creation of nested directory outside it.
- Current commit is not yet verified on Mac; test does not prove absence of all races between individual steps. Move/rename/delete and read of File Provider placeholders on main Mac are still disabled.

### Partial streaming output safety (2026-10-11)

- User confirmed `6ac84ef`: `scripts/test-core.sh`, fmt/Clippy clean, 104/104 macOS tests passed, 0 doc-tests.
- `ddc96b0` changes Linux `read_to_writer`: error from `io::copy` is now `OutcomeUnknown`, because during transfer receiver might have accepted part of bytes and automatic repeat is unsafe. Added Linux-only test Writer that accepts one byte and fails on next call.
- New patch is not yet verified on macOS/Linux. Safe rename/move/delete, macOS placeholder guard and main acceptance tests #15 are not completed.

### Local filesystem capacity inspection (2026-10-11)

- User checked `e91f0fb` on macOS: `scripts/test-core.sh`, fmt/Clippy passed, 104/104 Rust unit tests, doc-tests 0.
- `94158a5`: read-only `LocalFilesystem::storage_space()` added with `fstatvfs` of pinned root descriptor on Unix. Returns total and available bytes without reading file content. Added test including check after root directory name removal with live pinned descriptor.
- Values are not disk space reservation and do not prove possibility of subsequent write; ENOSPC in real operation is still handled separately. On unsupported platforms, failure is returned. New patch is not yet verified on Mac.

### Capacity preflight with safety reserve (2026-10-11)

- User confirmed `1e12af3` on macOS: `scripts/test-core.sh`, fmt/Clippy successful, 105/105 unit tests, 0 doc-tests.
- `b9a954f` adds `require_free_space(incoming_bytes, reserve_bytes)` with sum overflow check and comparison with `storage_space()` via pinned descriptor, without reading file content and without creating an object. A new Unix test checks zero request, overflow, and requests exceeding available volume.
- This is a **preliminary rejection**, not a reservation or a transaction; parallel processes may occupy space, therefore ENOSPC handling during actual writing remains mandatory. The method is not yet integrated into Jobs execution.
- New changes are **not verified on macOS**; move/rename/delete operations and iCloud placeholder guards are not implemented.

### Reserved local file creation (2026-10-11)

- User confirmed `6ce4d3a` on macOS: `scripts/test-core.sh`, formatting and Clippy without errors, 106/106 unit tests, 0 doc-tests.
- `c867ecd` adds `create_with_space_reserve` — a check of required size and reserve immediately before `create_from_reader`. A pre-check error does not open the Reader and does not create the file. A Unix test `PanicReader` is added to prove no reading occurs during sum overflow.
- This DOES NOT guarantee space availability during writing and does not replace ENOSPC/OutcomeUnknown handling in the existing atomic writer. The actual size of the input stream is not yet checked against `expected_bytes`; the new patch is not yet verified on Mac.

### Exact declared size for reserved file creation (2026-10-11)

- User verified `2e3fa3f` on Mac: `scripts/test-core.sh` fmt/Clippy clean, **107/107** unit tests, 0 doc-tests failures.
- `d96b1c6`, `f5b7a91`: `create_with_space_reserve` applies a streaming `ExactSizeReader` to the passed data. It blocks publication if the source ends prematurely or is longer than `expected_bytes`. Upon error, the temporary write is cleared by the existing `create_from_reader` mechanism; the test covers short, long, and exact sources.
- The new test and changes are **not yet verified** on macOS. The method does not reserve free space and does not provide a content snapshot. Any change after publication still requires the standard reconciliation procedure.

### Guarded local copy with reserve (2026-10-11)

- User confirmed `7c6c5ad` on Mac: formatting and Clippy without errors, **108/108** unit tests, 0 doc-tests.
- `004f9f2` adds `copy_file_new_with_reserve`: Linux-only reading of the source file via pinned descriptor, free space check with reserve and exact length before atomic publication, and open source metadata check after creation. The test protects the existing destination and confirms fail-closed behavior without consent or on macOS.
- On macOS, reading content is still blocked even after consent. On Linux, source file mutations during copying are still checked only by metadata, not as a snapshot. The new patch code is not yet verified by local Linux/macOS tests; rename/move/delete are not yet implemented.

### Read-only move preflight (2026-10-11)

- User confirmed `4b385d4` on macOS: formatting, Clippy, compilation and **109/109 unit tests**, 0 doc-tests errors.
- `d422a7c` adds `preview_move_new(from, to)` as a read-only check: paths must remain inside the pinned root, source must be a regular file, destination must not exist, and destination parent must exist and be a directory. Source metadata is returned; no data is read and no object is moved. The test verifies preview, conflict, traversal, and the absence of changes in source/destination.
- The pre-check result does not guarantee the safety of a subsequent rename due to TOCTOU; `move_file_new` remains blocked until an identity-preserving implementation and adversarial tests are available. The new patch code is not yet verified on Mac.

### Descriptor-relative file identity (2026-10-11)

- User confirmed `15e7e0a` on macOS: formatting, Clippy and 110/110 Rust unit tests without errors.
- `28a8712`, `d9934c6`: `LocalFileIdentity` (device/inode/size) and `LocalFilesystem::file_identity(path)` are introduced. The Unix implementation uses `fstatat(AT_SYMLINK_NOFOLLOW)` relative to the pinned root and does not open content; the test verifies identity preservation after external renaming and traversal prohibition.
- The obtained identifier is suitable for matching results and reconciliation, but **does not provide an atomic right to rename/unlink**. Repeated comparison of inode and pathname does not eliminate TOCTOU/ABA. `move_file_new` remains fail-closed; destructive delete is not added. The new code is not yet confirmed on Mac.

### Move preview inode revalidation (2026-10-11)

- User confirmed `8c8d9e0` on macOS: fmt/Clippy without errors, **111/111** Rust tests passed, 0 doc-tests.
- `dd04ce5` adds read-only `revalidate_move_new(from, to, expected)` — a repeated check of source inode identity and absence of destination, without opening content and without changing files. A Unix test replaces the source after preview and verifies rejection while preserving data.
- Revalidation does not eliminate TOCTOU after return. `move_file_new` and destructive operations remain blocked. New changes are not yet verified on Mac.

### Case-collision guarded creation (2026-10-11)

- User confirmed `3085447` on Mac: fmt, Clippy and **112/112** unit tests without errors.
- `112b2f9` adds a separate `create_from_reader_case_guarded`: before atomic writing, the new method scans the destination directory (via descriptor-relative Unix list) and conservatively rejects names that match after Unicode lowercase. The test verifies `Report.TXT` against `report.txt` before reading the Reader, the invariance of the original object, and the creation of a free name.
- This is an **additional check, not a proof of the absence of all Unicode/locale/case-folding collisions**; the atomic no-overwrite writer still provides the final protection against overwriting. With unsafe objects in the directory, the list may return a rejection; the existing `create_from_reader` is not changed. The patch is not verified on Mac; move/rename/delete remain fail-closed.

### Early destination collision on atomic create (2026-10-11)

- User confirmed commit `7160aac` on Mac: `scripts/test-core.sh`, 113/113 tests, fmt and Clippy successful.
- `0f14aaa` changes the actual Unix write `secure::create`: destination name check via `fstatat(AT_SYMLINK_NOFOLLOW)` occurs before starting the data stream and creating the temporary file; an existing destination is rejected without reading the source. The final `linkat` **remains the mandatory atomic check** for concurrent creations.
- `5f7efde` adds a `PanicReader` test, confirming that if the destination is occupied, the source is not read and the file remains unchanged.
- Rename/move/delete and macOS content read are not unlocked in the changes; the patch awaits verification on Mac.

### Bounded metadata-only recursive inventory (2026-10-11)

- User confirmed `2dd94df` on Mac: formatting, Clippy and **114/114** tests without errors.
- `970ad18`, `3151ca4`: `LocalFilesystem::list_tree(path, max_entries)` lists the nested structure via already protected descriptor-relative `list` operations, returns relative paths and metadata, sorts the result, and refuses to return an incomplete set if the limit is exceeded. It does not open file content and does not initiate iCloud placeholder downloads.
- A new test verifies nested directories, the entry limit, and traversal prohibition. This does not replace the atomic snapshot: under concurrent changes, the tree may be inconsistent, and re-scanning is required. The patch is **not yet verified on Mac**. Move/rename/delete operations remain disabled.

### Recursive inventory depth and unsafe subtree handling (2026-10-11)

- User confirmed successful check run after `d0943c1` (115 tests expected); detailed output was not provided in this message.
- `4a2d67a` limits `list_tree` nesting to 256 levels regardless of the number of elements, with `UnsafeEntry` rejection instead of a partial result; a test for a symlink inside a subtree is added.
- The method still does not form an atomic tree snapshot during concurrent changes. The new patch is not yet verified on Mac; rename/move/delete are still blocked.

### Recursive inventory directory identity (2026-10-11)

- User confirmed successful local verification of patch `a175e8a` (116 tests expected, full log not provided).
- `e5733fd`, `3fa958b`: Unix `list_tree` tracks `(device,inode)` of each opened directory and rejects repeated visits to the same directory to avoid returning a tree with duplication or loops. The descriptor is opened without following symbolic links. `82e1f7f` verifies the preservation of the directory inode after renaming.
- This is not an atomic filesystem snapshot; concurrent changes are possible. New changes are not yet verified on Mac; rename/move/delete operations are still unavailable.

### Unix filenames without silent replacement (2026-10-11)

- User confirmed successful Mac run after `d11123f` (previous expected result — 117 tests).
- `cd25b02`, `9c40748`: Unix `list` and `stat` refuse to convert invalid UTF-8 names via `to_string_lossy()`; they return `UnsafeEntry` without changing files. A test for a name with an invalid UTF-8 byte is added, verifying rejection and content preservation.
- This is a fail-closed limitation of the current string metadata model: full support for arbitrary Unix byte-paths will require a separate name model instead of lossy Unicode. Changes are not yet confirmed on macOS; move/rename/delete and safe iCloud placeholder reading remain open.

### Mandatory collision guard in default local writer (2026-10-11)

- User confirmed successful verification of the previous patch `a4429f3` on macOS (118 tests expected; full output not provided).
- `353e42d`: the case-insensitive name conflict check is moved to the main `create_from_reader`, and `create_from_reader_case_guarded` becomes a compatible delegating method. All creation paths (including `create_with_space_reserve`) now use a single guard. A separate test verifies the main API, the absence of source reading during conflict, and the preservation of the existing file.
- The Unicode lowercase name check is conservative, **not** full Unicode normalization/casefold; a separate `linkat` remains the authoritative protection against overwrite during a race. The new patch requires fmt/Clippy/tests verification on Mac. Rename/move/delete and placeholder guards are not implemented.

### Directory case-alias safety (2026-10-11)

- After verifying 120 unit tests on macOS, the user instructed to continue #15 with the completion percentage and verification commands in each report.
- `9197381`, `b535fdd`: Unix `create_directory` performs a pre-operation check for case-insensitive name collisions; a separate test prohibits the creation of `assets` when `Assets` exists, including recursive paths.
- `f3a55c1`, `be9c931`: `create_directory_tree` now verifies the exact spelling of an **already existing** directory before proceeding, to avoid creating an unexpected subdirectory under an alias name on case-insensitive filesystems.
- These checks do not provide atomic protection against all Unicode-normalization collisions or concurrent directory changes. Rename/move/delete remain fail-closed due to TOCTOU of the source name identity. The new patch is **not verified** on macOS; percentages are not to be increased until confirmation.

### Concurrent mkdir alias revalidation (2026-10-11)

- User confirmed successful test run on macOS for `238d69a` (121 tests expected, detailed log not attached).
- `5288aaa`, `7a2a368`: when `create_directory_tree` receives `AlreadyExists` during a creation race, it now verifies not only the directory type but also the **exact spelling of the name** in the parent directory. This eliminates the bypass of the case-collision guard on case-insensitive filesystems when a competing process has created a directory with different casing.
- For a partially created tree, an undefined result is classified as `OutcomeUnknown`, without automatic rollback or retry. The new patch is not yet verified on Mac; rename/move/delete and iCloud placeholder handling remain blockers for closing #15.

### Recursive mkdir with unrelated symlink siblings (2026-10-11)

- User confirmed successful verification of the previous main state (`bc53814`, expected result 121 tests); detailed output was not provided.
- `fab2cbe`, `2ef5125`, `cf7cc16`: `has_exact_directory` compares names directly in `readdir` relative to the pinned directory fd and verifies the type only for the found exact name via `fstatat(AT_SYMLINK_NOFOLLOW)`. `create_directory_tree` uses this check for existing directories and during `AlreadyExists`, no longer requiring metadata reads for all unrelated neighbors. This allows safe creation of a nested directory next to symlinks unrelated to the operation.
- A Unix regression test `mkdir_tree_accepts_existing_directory_with_unrelated_symlink` is added. Actual rename/move/delete functionality and macOS placeholder read are still disabled. The patch is not yet verified on Mac.

### Non-destructive hard-link move staging (2026-10-11)

- User confirmed successful macOS run after `2993a28` (122 unit tests expected).
- `2bce13c`, `34a48c0`: Unix `stage_move_link_new(from,to)` publishes a new name via `linkat` without overwriting, does not read content, and does not delete the source name. Before the operation, it verifies that the source is a regular file and checks for destination conflict; after publication, it compares `dev/inode` and performs `sync_all` on the destination directory. Any error after publication returns `OutcomeUnknown`. A test is added for source preservation, existing destination protection, and inode identity.

- **This is not rename/move/delete.** Concurrent source pathname replacement cannot be fully excluded by a single check before `linkat`; reconciliation is required if a discrepancy occurs. Automatic unlink of the source and blind retry are prohibited. The operation is not integrated with Jobs and is not declared in capabilities. New code is not yet verified on macOS; the number of expected tests is 123.

### Hard-link staging source identity reconciliation (2026-10-11)

- The user confirmed successful macOS verification of the previous patch `31d1a4f`, with 123 expected tests (detailed log is not attached).
- `8f05d9c`: immediately after `linkat`, the source file name is re-verified using `fstatat(AT_SYMLINK_NOFOLLOW)` and a comparison of inode/device/type. Any discrepancy or error **after publication** returns `OutcomeUnknown`, prohibiting blind retry; the new destination is not automatically deleted.
- `4e883d5`: test for hard-link staging failure for a symlink source without destination creation.
- TOCTOU remains between the `linkat` command and the checks; the operation is not an atomic safe rename, does not perform unlink, and is not integrated into Job execution. The patch is not verified on Mac, with 124 expected tests; progress #15 remains at ~52%.

### Staging hard-link conflicts (2026-10-11)

- The user confirmed a successful test run of the previous patch `f113d5b` (124 expected tests).
- `82e4200`: regression test for case-insensitive destination name conflict for hard-link staging.
- `7370f63`: refuses to create an additional link to a source with `st_nlink != 1`, because an already shared inode requires explicit conflict resolution. `b330263`: negative test for such a source.
- Changes do not eliminate the TOCTOU race for the source; therefore, the method remains an auxiliary non-destructive operation, without integration into Jobs. The new patch is not yet verified on Mac; 126 tests are expected. Readiness estimate #15 ~52%.

### Hard-link publication race fault injection (2026-10-11)

- The user confirmed a successful macOS run of the previous patch `1f10dfa` (126 expected tests).
- `971e4c6` adds a test callback strictly after `linkat` in `link_file_new`, before the source check. Production uses no-op.
- `b6c186e` adds an adversarial test: after publication, the source name is renamed and replaced by another file. The result must be `OutcomeUnknown`, while original bytes are preserved in both the renamed source and the staged hard link; the replaced file is not deleted.
- This does not eliminate TOCTOU between preflight and `linkat`, does not implement delete/rename/move, and does not allow automatic retries. The new patch is not yet verified on Mac, 127 tests are expected; readiness ~52%.

### Read-only hard-link staging reconciliation (2026-10-11)

- The user confirmed a successful macOS run after `12037c8` (127 expected tests).
- `6039528`, `b035b43`: `reconcile_staged_link(from,to,expected)` matches the identity of both names with a fixed `(device,inode,size)` and returns `bool` without reading bytes or performing mutations. The test confirms a match after publication and a negative result after source replacement, as well as the preservation of the original inode.
- This is an **observation**, not a completion of move/delete and not a permission for blind retry after `OutcomeUnknown`; identifiers and size do not guarantee content integrity during concurrent modification. The new patch is not yet verified on Mac. Estimate #15 ~52%.

### Missing hard-link paths during reconciliation (2026-10-11)

- The user confirmed successful verification of `5b89d08` on Mac (128 tests).
- `5864520`: `reconcile_staged_link` returns `Ok(false)` when the source or published name is missing (`NotFound`), without recovery, republication, or content reading. Other error types are still returned to the calling code.
- The Unix test verifies a missing destination, then a missing source after an external rename, and the preservation of the original inode content. This is not a permission for automatic retry: any previously received `OutcomeUnknown` continues to require manual/managed state resolution.
- The new patch is not yet verified on Mac; 129 tests are expected; readiness #15 remains ≈52%.

### macOS metadata-only dataless identification (2026-10-11)

- The user confirmed successful macOS verification of the previous state `d53c679` (129 tests).
- `f3ad883`: macOS `fstatat(AT_SYMLINK_NOFOLLOW)` relative to a pinned descriptor checks Apple `SF_DATALESS = 0x40000000` (XNU `bsd/sys/stat.h`). It does not open content and does not initiate materialization.
- `eb8256e`: `content_residency(path)` returns `DatalessPlaceholder`, `NotDatalessFlagged` (does not mean guaranteed content availability!), or `Unknown` on Linux. macOS read-to-writer remains prohibited even after per-operation consent; the status does not grant read permission. macOS and Linux tests are added.
- Known limitation: `SF_DATALESS` does not replace a full evaluation of File Provider/iCloud `ubiquitousItemDownloadingStatus` and a robust per-operation no-hydration policy; a missing flag does not prove that a file read is safe. The new main state is not yet verified on Mac, 130 tests are expected on macOS (Linux: 130).

### Batch metadata inspection (2026-10-11)

- The user confirmed successful local verification of the previous state (`d9dda36`, 130 tests); GitHub-hosted Actions are blocked by billing, so local `scripts/test-core.sh` is used.
- `8a736f2`, `ecf2a61`: `inspect_content()` and `inspect_content_batch()` are added — a composite metadata-only preflight that checks inode/size before and after the residency request. Data collection from multiple files does not guarantee atomicity/consistent snapshot. If any element is missing, is a symlink, or is outside the root, the full call returns an error without a partial successful result.
- Three new Unix tests: batch for two regular files, failure on symlink and traversal, failure on missing entry without data creation. Expected local test result on macOS: 133 tests, not yet verified.
- The absence of `SF_DATALESS` does not permit hydration or content reading. macOS content reads remain prohibited. Extended File Provider statuses and safe rename/move/delete are not yet implemented. Estimate #15: ~58%.

### Metadata batch revalidation after preflight (2026-10-11)

- The user confirmed a successful local run of the previous package (`10faf08`; 133 tests).
- `db03c69`: `revalidate_content_inspection(path, expected)` and `revalidate_content_batch(paths, expected)` are added for re-performing metadata-only checks of stale preflight results before the next step. Inode/device/size and residency are compared. A missing file returns `Ok(false)`, symlinks/unsafe paths return an error; array length mismatch returns `InvalidPath`. The package is not declared as a consistent snapshot; verification operations do not grant read rights and do not eliminate TOCTOU.
- 3 Unix tests: successful re-observation, inode replacement and deletion, stale batch and incorrect length, symlink substitution and data preservation. New code requires a local macOS run; 136 tests are expected.
- Readiness #15 is about 60%; full safe move/rename/delete and File Provider statuses are not yet implemented.

### Same-size local modification detection (2026-10-11)

- The user confirmed a successful local run of the previous package (`4b1a9be`, 136 tests).
- `d31845d`, `ca49575`, `70c7a69`: `file_change_stamp()` obtains Unix `mtime`/`ctime` with the nanosecond part via `fstatat(AT_SYMLINK_NOFOLLOW)` in a pinned root; `LocalContentInspection` includes four time-stamp fields. `inspect_content` checks the stamp before/after residency and inode before/after; `revalidate_content_*` now detects changes of the same size without reading content. Unix tests for same-length rewrite of one file and in batch are added.
- Limitation: the modification time is not a cryptographic checksum and is not protection against intentional attribute substitution/micro-races. The inspection output is not proof of immutability and does not permit reading cloud content on Mac.
- The new package is not yet verified by the Mac compiler; 138 unit tests are expected. Readiness #15 is approximately 60%.

### Duplicate-source protection for metadata batches (2026-10-11)

- The user confirmed successful local verification of the previous package (`bd58430`, 138 tests).
- `4dc0d73`: `inspect_content_batch` fails if a path is repeated (including ASCII case aliases) or if `(device,inode)` is repeated under a different name. This prevents double processing of hardlinked files during the planning stage; it does not read content and does not modify files. Case-insensitivity rules for Unicode differ between file systems; final duplicate verification by inode concerns existing files.
- Two new Unix tests: exact/case duplicate and hard-link alias. 140 tests are expected during the next Mac verification. Not confirmed by compiler; rename/move/delete and File Provider remain incomplete.

### Pinned root overlap preflight (2026-10-11)

- The user confirmed local 140 tests for the previous package `10cd138`.
- `01d4ea0`, `96a22a5`, `0242ca7`: `LocalFilesystem::overlaps_root(&other)` checks for matching or nested file roots on Unix by `(st_dev, st_ino)` while traversing parent `..` via `openat` from pinned directory fds. Path or file content is not read. Tests: same root and parent/child, independent neighboring roots, moving (renaming) a parent directory after opening the fd. Automatic blocking of cross-root jobs is not performed; the calling code must prohibit scenarios with overlapping roots.
- Limitations: with active concurrent mount/rename changes, directory traversal remains observational, not transactional; a maximum depth of 1024 causes a fail-closed `UnsafeEntry`. The method is unavailable on non-Unix.
- The patch requires verification on Mac: 143 tests are expected. Readiness #15 is approximately 62%; full rename/move/delete and guaranteed no-hydration work remain open.

### Non-destructive staging: failure and unsafe-object matrix (2026-10-11)

- The user confirmed a local run of `f6fb0f8`: 143 tests.
- `2dfcb58`, `f98ae93`, `f36afa1`: four Unix regression tests are added for `stage_move_link_new` and `reconcile_staged_link`: failure after successful linkat is classified as `OutcomeUnknown` and does not delete either source or destination; retry with a busy destination is rejected; directory-source and symlink-destination are not changed; destination inode replacement is detected without file deletion.
- The new matrix verifies **data preservation**, but does not guarantee atomic rename/delete during concurrent replacement. Do not include actual source deletion and automatic retries based on these tests.
- The new package has not passed local fmt/clippy/tests; 147 tests are expected. Readiness #15 ≈62%: significant blockers for rename/move/delete and File Provider hydration are still open.

### Safe move preflight case collisions and directory guards (2026-10-11)

- The user confirmed a successful local run of the previous patch (`f9e49f7`, 147 tests).
- `400625a`, `33d5139`: metadata-only `preview_move_new` checks, when the destination is missing, via a pinned parent descriptor if a case-insensitive alias of the destination exists, and returns `AlreadyExists` instead of a false permission. `revalidate_move_new` inherits this protection. 3 Unix tests are added: case alias destination/revalidate, successful preview into an existing directory without writing, failure on symlink parent or missing parent.
- This does not permit actual move/rename/delete, does not eliminate TOCTOU after preflight, and does not initiate cloud content read. The new patch awaits local macOS verification: 150 tests. Readiness #15 ~62%; major blockers remain.

### POSIX permissions metadata (2026-10-11)

- The user confirmed successful local verification of the previous package `9677f8c` (150 tests).
- `6e8e8d2`, `760b2b2`: `LocalFilesystem::mode_bits(path)` is added, based on descriptor-relative `fstatat(AT_SYMLINK_NOFOLLOW)`. It returns POSIX bits `0o7777` for regular files/directories and fails on symlinks and invalid paths. It does not open content and does not initiate iCloud hydration.
- These bits are informative and do not prove effective access: ACL, process UID/GID, sandbox, TCC, and file policies are checked directly during operation execution. Two Unix tests are added: modes 0640/0750, failure on symlink and traversal.
- The current patch is not yet verified on Mac; 152 tests are expected. Readiness #15 ≈62%, safe rename/move/delete and full placeholder guard remain incomplete.

### Full-stage acceptance pass: macOS no-hydration contract (2026-10-11)

- As agreed with the user, #15 is henceforth verified as a single stage rather than sending every small change to Mac. The last confirmed local run refers to the previous state `7f3527f` (152 tests according to the expected counter; the user has not yet sent the output of the current run).
- `d063bb7` adds two macOS contract tests: both entry points for local copying (`copy_file_new`, `copy_file_new_with_reserve`) return `UnsafeEntry` and do not create a destination both for Denied and GrantedForThisOperation; metadata-only `inspect_content` does not permit `read_to_writer`, and capabilities do not promise `read_content`. This is not a test on a real iCloud placeholder and not an implementation of safe materialization.
- Before final acceptance of #15, it is mandatory to perform a single fmt/clippy/tests run on macOS and a separate manual placeholder check in a permitted test container, without QADREN-initiated downloading. Rename/move/delete functions are currently unsafe for public inclusion and must either receive a provably safe mechanism or remain an explicit blocker for closing the issue.

### Atomic no-overwrite rename candidate (2026-10-11)

- `30b42da`, `520f299`, `aa8bb35`: `rename_file_new` is added for regular files on Unix with an atomic system no-overwrite primitive: Linux `renameat2(RENAME_NOREPLACE)`, macOS `renameatx_np(RENAME_EXCL)`. Intermediate parent directories are pinned descriptor-relative, the source must have `st_nlink == 1`, the destination is pre-verified via `preview_move_new` and the case collision guard, and the inode is checked after movement; directory fds are synchronized. In case of an ambiguous outcome, `OutcomeUnknown` is returned; automatic retry is prohibited.
- Contract Unix tests: successful transfer to another directory, existing destination without overwrite, symlink in destination without changing source/target. Not verified in local Mac build. The call is **currently experimental**: the race with source inode change between preflight and rename is not fully eliminated; therefore, it cannot be presented as confirmed safe under hostile concurrent modifications or connected to automatic Jobs. Safe delete/replace are also not implemented, and verification of real File Provider placeholders has not been performed.
- Next step: local build of the full package and review of API/negative race tests, but by agreement, Mac runs are not requested after every commit; final closure of #15 requires all contracts and tests.

- `968e4a3`, `f9482b7`: source name recreated after `renameat*` is classified `OutcomeUnknown`, plus adversarial tests for hardlinked source, Unicode-independent case collision and intermediate symlink outside root. Added six total Unix rename tests. The rename API is available for isolated test use, but remains **not advertised in adapter capabilities** due unresolved concurrent source-swap race; this is not acceptance of deletion or general-purpose automatic move.

- `effb541`: `reconcile_renamed_file(from,to,expected)` performs only a metadata-only check of the result of a previous ambiguous rename: the expected destination inode matches and the source name is absent; it does not restore or delete anything. The test covers before/after rename and destination swap. This is an observational, not a transactional, guarantee.

### Interrupted rename handling (2026-10-11)

- `43e37c6`, `0385e93`, `c4492b6`: rename syscall failures with `Interrupted` now return `OutcomeUnknown` (not retryable `Io`); `AlreadyExists` remains a known collision; other error classes continue as `Io`. Three deterministic unit tests cover these classifications without performing destructive filesystem operations.
- Safety review: `renameat2(RENAME_NOREPLACE)` / `renameatx_np(RENAME_EXCL)` atomically protects **destination no-overwrite**, but it does **not** atomically bind the source inode observed before the syscall. A hostile concurrent source pathname replacement can cause a rename of a different inode. Postflight reconciliation detects many such outcomes only **after** mutation. Therefore, keep this API out of automatic Jobs/capabilities until a stronger trust/concurrency model or source-binding design is agreed and tested. The macOS placeholder policy remains fail-closed for content reads.
- Full-stage local validation is still outstanding; by agreement, do not request per-commit testing. No change to the percentage of issue #15.

- `6208400`: non-Unix local adapter no longer declares `Windows` as supported nor marks insecure path-based metadata listing as verified; `list_metadata` and prerequisites now require Unix. The existing non-Unix fallback is not a production-safe file adapter and must be replaced by an OS-specific secure implementation before enabling Windows.
- `2dbedec`: rename compares the second `fstatat` source `(dev,ino,size)` with the initial identity and requires `nlink==1`. This narrows the pre-rename race window but does not close it; the unresolved source-binding limitation still forbids automatic destructive jobs.
- Next gate: verify platform-specific `libc` symbols and fmt/clippy/tests on Mac as part of the agreed consolidated review, then decide the safe source-binding and deletion policy before declaring #15 accepted.

### Confirmed local package and next increment (2026-10-11)

- The user confirmed a successful `scripts/test-core.sh` after formatting `91f3a2a`. This confirms fmt/clippy/tests for this state, but does not resolve safety issues during concurrent file mutations.
- `12415b1`: metadata-only `preview_move_identity(from,to)` returns the observed identity of the source regular file after checking the destination, without changing files. The test verifies no changes on disk. This is a helper contract for future reconciliation, not an implementation of safe deletion or move; include the check of the new patch in the next consolidated acceptance without requesting a separate run.

### Regression baseline after macOS validation (2026-10-11)

- The user confirmed a successful Mac test-core.sh on `3772973` (including previous changes); the exact number of tests is not provided in the message.
- `993b81a`: two Unix negative tests — rename with a source symlink must fail without modifying the target, and a preview with a source inode replaced before revalidation must fail as `UnsafeEntry`. These tests have not yet been run on Mac.
- Until the full DoD #15 is met, a critical gap remains: the possibility of source pathname substitution **after** check and **before** the system rename, as well as the lack of safe delete, iCloud File Provider proof, and final general acceptance. There are currently insufficient grounds to mark the file movement item as complete.

### Checked rename preview binding (2026-10-11)

- Following the user-confirmed Mac run of `3772973`, a single development stage continues. New tests `993b81a` and `d41282c` are not yet verified.
- `d41282c`: `rename_file_new_checked(from,to,expected)` re-verifies source identity and destination preflight before a no-replace rename; a negative test with an inode change before the call does not move the substituted file; a positive test verifies a one-time transfer. This narrows the stale plan window but **does not exclude a race after the final check**. It must not be used for hostile concurrent directories or advertised as a completed `move/delete`.
- Final safety criterion: operations that potentially delete the source path are included in automatic Jobs only after a proven source binding guarantee or a documented and verified exclusive directory ownership model. No-hydration obligations on Mac cannot be dismissed yet.

### Capability gate for destructive operations (2026-10-11)

- `133dc04`: added a contract test confirming that `LocalFilesystem::capabilities()` does not advertise Rename/Delete and `CapabilityDeclaration::allows` returns `UnsupportedOperation` for both on Unix. This protects the planning layer from premature use of experimental rename, rather than eliminating the source pathname race.
- The last user-confirmed macOS check was before `993b81a`; combined changes after it have not yet passed fmt/clippy/test on Mac. As part of the agreed process, do not request a separate test run now.
- Stage closing blocker: ensure safe deletions/moves during allowed concurrent mutations or agree on a limited controlled directory model, plus File Provider/macOS hydration policy and full acceptance.

### macOS dataless mutation guard (2026-10-11)

- `da3474b`, `90a6ff3`: on macOS, `rename_file_noreplace` and `link_file_new_with_post_link_check` operations reject `SF_DATALESS` sources before namespace changes. A unit test for the flag is added. Compilation and testing results are not yet confirmed.
- The absence of the `SF_DATALESS` flag does not prove that a File Provider item is fully local; the metadata guard does not grant read rights. All content reads on macOS remain prohibited. Rename is not published in capabilities due to the source pathname race. Full DoD #15 remains open.

### Concurrent changes: fail-closed delete (2026-10-11)

- Decision accepted in #15: QADREN must be safe even with concurrent file changes by third-party applications.
- `c293901`: added an explicit `delete_file(path)` that checks path boundaries and rejects deletion as `UnsafeEntry`. Two negative tests verify source preservation and failure during traversal. The method intentionally deletes nothing: `fstatat` + `unlinkat` do not provide an atomic condition to "delete only the inode that was verified."
- Do not consider this as the delete implementation in the DoD. `move_file_new` also remains fail-closed; `rename_file_new*` must not be used in automatic jobs. Next step: a safe model without source deletion and verification of the entire chain in macOS, then a separate architectural implementation of source-bound destructive operations or documented unavailability.
- These two new tests are not verified in a local Mac run; do not request separate verification before general acceptance.

### Non-destructive staged copy under concurrent writers (2026-10-11)

- `df89eda`, `b2f50d4`, `cbb51ca`: added `prepare_move_copy_new(from,to,expected,consent)`, which on Linux matches the previous metadata-only source inspection, checks for a free destination, copies bytes via a no-overwrite writer, re-verifies the source, and **never deletes the source**. If the source changed after publication, it returns `OutcomeUnknown`, as a copy might have already appeared.
- Tests verify successful publication with source preservation, refusal of a second copy into an occupied destination, refusal upon stale inspection and lack of consent, and prohibition of reading/copying on macOS even with consent. This is only Linux preparation, **not a full move**; do not add rename/delete to capabilities.
- For a source pathname changed by a third-party writer during copying, checks are best-effort; a mandatory postflight `OutcomeUnknown` does not mean proven consistency. New changes await general macOS verification.

### Linux move-copy: checked opened source inode (2026-10-11)

- The user confirmed a successful local fmt/clippy/tests after `5c62e0d`.
- `3b55848`: `prepare_move_copy_new` no longer re-opens the source inside `copy_file_new` after a separate pathname preflight. Now, the source is first opened via pinned root/no-follow, the directly opened fd is verified by `(dev,ino,size,mtime,ctime)` against `LocalContentInspection` **before** destination creation; then the same open descriptor is passed to the no-overwrite writer. After copying, the metadata of the same fd and the source name are checked. If the result is unverifiable after publication, `OutcomeUnknown` is returned; the source is never deleted.
- This eliminates the replacement window between inspection and **opening the actual input** to prevent unnoticed copying of the wrong inode, but it does not guarantee a byte snapshot during concurrent writes to the open inode. Mac content reads remain fail-closed. The patch is not verified by a local compiler; execute during general acceptance.

### Exact-length move-copy publication (2026-10-11)

- The user confirmed a successful Mac run after `a7ad156`.
- `a937c72`: in Linux `prepare_move_copy_new`, the input descriptor is limited by `ExactSizeReader` to the fixed `before.len()` length. If the stream shrinks or grows, the temporary write must fail **before** destination publication; after publication, a separate verification of the same open fd and pathname remains. The source is not deleted. The package requires the next consolidated local check.
- This is not a guarantee of an unchanged byte snapshot during concurrent overwrite of the same length, nor is it the completion of move/delete. Mac cloud content safety remains fail-closed.

### Linux source verification before copy publication (2026-10-11)

- The user confirmed a Mac run after package `b30071b`.
- `24d38d0`, `b0e7ecc`: `VerifiedCopyReader` on Linux checks the open descriptor at the EOF point before the no-overwrite writer publishes the ready temporary file. If the inode changed size/mtime/ctime during reading, `create_from_reader` returns an error and does not publish the destination. A test for same-size source change before EOF is added. After publication, a separate check of the source and its pathname remains, with `OutcomeUnknown` upon discrepancy.
- Timestamp verification does not prove an atomic byte snapshot during concurrent writing; the source is never deleted. On macOS, content reads are prohibited. New code awaits compilation and local testing; do not consider #15 closed.

### Hardlink source guard for move-copy preparation (2026-10-11)

- The user confirmed a successful local run of the previous package `2bc6485`.
- `55c4955`: Linux move-copy preparation now fails if `nlink != 1` for the already opened source inode before destination publication. An additional test confirms that for a hard-linked source, no new file is created, and both the alias and the original are preserved. This is consistent with stricter safe movement requirements but does not apply to ordinary independent copying.
- The change has not yet passed macOS fmt/clippy/tests; request a new check together with the general package, not after this commit. Full delete/move remains closed for automatic Jobs.

### Move-copy publication regression (2026-10-11)

- The user confirmed the previous Mac run before the current set of changes.
- `e1beef4` adds a Linux end-to-end test: an already open source is changed after the initial `metadata`, and `VerifiedCopyReader` must abort the atomic publication of the destination. If the source is changed, the original is not deleted.
- Correction `6f233ac` removes an unnecessary explicit `drop` of the reader and relies on standard Rust NLL borrow checking. Compilation and testing on Mac are required during the consolidated stage; there is no basis to declare #15 complete.

### Reconciliation boundary: copy versus byte equality (2026-10-11)

- `c71bab1`: metadata-only `reconcile_move_copy` inspects a preserved source against the original inspection and checks that a destination of the expected length exists. It does **not** claim integrity, equality of content, recovery, completed move, or authorization to remove either file.
- Unix negative contract test deliberately uses same-sized but different bytes: metadata reconciliation may return true, illustrating why the caller MUST NOT mistake its result for verified copy integrity. A missing destination returns false. This interface is observational only.
- To mark a prepared copy as verified, execution needs a trustworthy content digest/producer-verified source and destination, alongside stable source identity and no forbidden macOS hydration. No automatic destructive step is enabled. Consolidated Mac tests remain pending for new code.

### End-to-end prepared copy byte verification (2026-10-11)

- `c71bab1`, `bcbaa41`: metadata-only copy reconciliation checks for the presence of two names and the size, **not** integrity; a negative contract demonstrates identical length with different bytes.
- `67c4a9c`, `a4ab00a`: a separate Linux API `verify_move_copy_bytes` opens the source and destination via pinned root + no-follow upon explicit permission, compares content streaming in 64 KiB blocks, and re-verifies both open inodes by metadata before/after. If detected metadata or the source pathname changes, it returns an error rather than a false success. The Linux test distinguishes equal-size corruption from an exact match; the macOS test requires fail-closed even with consent.
- With hostile writing to an open file, comparison + timestamps do not provide a transactional byte snapshot; automatic delete/move remains unavailable. These new changes are not verified by the Mac toolchain. Evaluate progress by completed functions, not by the number of tests.

### Release 1 Stage 04 start: Yandex streaming upload (2026-10-11)

- At the request to accelerate the project, independent work #16 has started without a dummy closing of #15. This is a parallel start, not a removal of blocking safety guarantees.
- `a7e874d`, `8738354`, `020c72b`, `e69891f`: a minimal upload path is implemented: GET for a signed resource `/resources/upload?overwrite=false`, verification of HTTPS/PUT/absence of URL credentials, streaming HTTP PUT of a fixed declared length without opening files on Mac, and a separate `UploadOutcome::Accepted/Pending`; HTTP redirects are prohibited in the production client. A PUT error is considered a possible unknown state of the remote file and requires external reconciliation; the method does not perform automatic retries.

- Upload reader now checks that actual durable length matches `expected_size` exactly: short source and trailing bytes are detected before the last chunk is transferred (for zero expected size, before PUT). A mismatch is not declared as a successful copy: the result remains `UploadOutcomeUnknown` and requires reconciliation. This does not compute MD5 source and does not replace remote metadata verification.

### Yandex upload metadata reconciliation contract — #16 (2026-10-13)

- `reconcile_uploaded_resource` only performs `GET /resources` for the exact expected path and matches path, size, and 32-hex provider MD5. An incorrect expected MD5 stops the check as `InsufficientEvidence` before network access; an incomplete or different provider response is not a success.
- Metadata-only browser retrieves subsequent pages only by explicit user action: Rust/FFI passes `offset` with a fixed limit of 50, SwiftUI shows "Show more" only when the provider reports more elements. Loopback HTTP verification records GET `/resources` with path/limit/offset and without content transfer. Missing or contradictory pagination fields do not trigger infinite loading.
- Observation of an existing durable `copy` attempt reuses this contract after verifying the registered `connection_id`, Keychain credential reference, and provider account UID. It remains read-only: it does not repeat PUT, does not read/download content, does not change Job/evidence/baseline, and is not connected to FFI/UI. Mock HTTP test proves read-only GET and metadata match; this is not live Yandex E2E and not confirmation of real transfer. #16 remains PARTIAL.
- For an already `UNKNOWN` copy attempt, a separate recovery boundary is added: after the same account-bound read-only verification, only an exact path/size/MD5 match atomically completes the attempt and Job. Mismatch, incomplete metadata, account/credential error, and provider failure leave the Job as `UNKNOWN`; PUT, download, retry, and Registry/FileSyncState mutation are absent. This path is not yet connected to UI/FFI and is not live Yandex E2E.
- End-to-end mock tests record the order account UID $\rightarrow$ resource metadata and prove that even an exact match does not change a `running` Job and does not resolve an attempt; a mismatching account UID stops the path before resource lookup. The token remains only in the closure test Keychain resolver and is not written to Registry/JobStore.
- Mock API test confirms the refusal to transfer to an HTTP URL before reading the entry. There is NO end-to-end verification with a real account, no verification of full size/MD5 after upload, no honest `OutcomeUnknown` in the error type, no integration with scenarios/queue, and no safe control of cancellation and retries/backoff; do not enable automatic Jobs or advertise as production-ready. New code has not yet passed local fmt/clippy/tests.

## Gate-focused acceptance matrix — 2026-10-11

Historical records above refer to the SHAs listed there and do not confirm the current HEAD. Every claim of passing tests must include the actual SHA and environment.

| Gate | Ready if and only if | Confirmation at preparation stage |
|---|---|---|
| G0 | Job owners, scope consent, adapter errors/capabilities, FFI, and UNKNOWN reconciliation are documented | Documented, implementations still require verification |
| G1 | Manual create-new passes SwiftUI $\rightarrow$ Rust JobStore $\rightarrow$ mock adapter $\rightarrow$ verified SQLite receipt $\rightarrow$ UI; no-overwrite, restart/duplicate/fault tests | PARTIAL: mock vertical slice, durable queue/cancel before effect, Rust/mock/ABI tests and macOS smoke/restart passed; real adapter, pause/resume, fair scheduling and limits remain |
| G2 | The same vertical works on a actually verified adapter pair; live provider evidence and consent | NOT performed |
| G3 | One-way and two-way Scenario diff rely on the last confirmed state; conflict is blocked | NOT performed |
| G4 | Mass delete/overwrite, concurrent mutation, Keychain disconnect, and uncertain retry fail-closed | Partially, full acceptance NOT performed |
| G5 | A single Rust engine services Jobs in an independent service; closing UI does not stop execution | NOT performed |
| G6 | Platform matrix, entitlement, selected distribution, signed install, resources, and fault tests are fixed | NOT performed |

For each increment, attach a test case ID, SHA, environment, executed command, PASS/FAIL/BLOCKED/SKIPPED, evidence source (real/mock/static), and remaining risk. Closing #15 based on the number of unit tests is prohibited. Recommended first E2E: no-overwrite create-new-file, stable attempt ID, and verification before terminal success. Original content must not be automatically downloaded to the main MacBook. Before release with incomplete destructive capabilities, request separate product approval.

### G1 in-process job inventory — 2026-10-11

Source code slice: `a39c392` (`JobStore::list_jobs`), `519810a`/`c581b9b` (read-only C ABI `qadren_jobs_list`), `58cd095` (Swift `JobInventory`), `33fc1f8` (read-only queue panel in `MainView`). Statuses are read directly from durable SQLite; no Job starts when the UI opens. A separate architectural boundary is maintained for the first runner, without an invented worker service.

**Verifications:** static path analysis of source code. Rust fmt/Clippy/tests, Swift compiler, native macOS smoke, and real recovery after restart on these commits were NOT performed here. Code is not considered verified until a confirmed run occurs. This intermediate stage **does not** satisfy G1: manual Job executor, no-overwrite adapter effect, production verifier, agreed result fixation, and end-to-end test are absent.

**Before next functional commit:** run `bash scripts/test-core.sh` and `bash scripts/build-macos.sh` on Mac. If unsuccessful, fix compilation first, do not expand functionality. The next change must relate to safe execution, not increase the read-only interface unnecessarily.

### G1 mock create execution package — 2026-10-11

`118cdf2` added a single in-process vertical slice: SwiftUI via a narrow C ABI creates a new **app-owned** mock artifact; `job_execution.rs` saves a `JobStore` Job and bound attempt with size/MD5 before writing, performs `create_new` without overwrite, then a mock verifier reads only this test object again and saves a metadata-verified receipt in SQLite. Swift does not write to SQLite and does not perform file business logic.

Verified on macOS: `bash scripts/test-core.sh` — Rust fmt, Clippy, and 188 unit tests passed; including consent-denial without effect, occupied destination without overwrite, invalid path, partial write, duplicate request, simulated crash after external effect $\rightarrow$ restart `UNKNOWN` with no replay, and successful C-ABI execution. `bash scripts/build-macos.sh` built an unsigned `dist/QADREN.app` and verified `Info.plist`.

`118c1f9` extends the mock slice with exactly one safe management operation: `Queue mock file` saves only a local SQLite Job; `Run` is a separate operation-scoped consent to create a test artifact; `Cancel` is allowed only if there is no active/unknown bound attempt. Terminal state `cancelled` and migration v6$\rightarrow$v7 are introduced, which preserve Jobs/events/attempts/evidence. An active or UNKNOWN attempt cannot be cancelled — a possible effect still requires reconciliation and does not receive a false `cancelled` status.

The next package adds durable `Pause`/`Resume` for a Job **before** external effect: `Pause` moves a queued Job to `paused`, `Resume` returns it to `queued`, and a separate `Run` again requires operation-scoped consent. Pause during an active/unknown bound attempt is intentionally rejected: checkpoint for streaming is not yet implemented, so the system does not report the stop of a possible effect as a verified fact. State is available via narrow Rust C ABI commands and displayed in SwiftUI without direct writing to SQLite.

Verified on macOS for this package: `bash scripts/test-core.sh` — PASS (fmt, Clippy, 192 Rust unit tests); `bash scripts/build-macos.sh` — PASS (unsigned `dist/QADREN.app`, `Info.plist`); UI smoke — PASS: queued mock Job appeared in UI, was cancelled before effect, after closing and restarting UI preserved `cancelled`. This is **mock evidence**, not G2 and not real Yandex/iCloud/Local FS transfer: no user or cloud content is read, no destructive operations, Background Worker, Scenario, pause/resume, fair scheduling, limits, or real provider reconciliation are added. G1 and #18 remain PARTIAL until these separate boundaries and final review on SHA.

A separate read-only reconciliation for already durable `UNKNOWN` G1 mock attempts is added: Rust compares exactly the saved `operation_kind`, `connection_id`, target, size, and MD5 with the app-owned test artifact; only after an exact match does it resolve that same attempt and move the Job to `completed`. Mismatch, absence of root/object, or ambiguous binding remain `UNKNOWN`; repeated writing, deletion, or automatic rollback are absent. This narrow mock recovery is not reconciliation for Yandex, Local FS, or iCloud and does not remove remaining G1/#18 restrictions.

Verified on macOS for this change: `bash scripts/test-core.sh` — PASS (fmt, Clippy, 212 Rust unit tests, including crash-after-effect $\rightarrow$ exact reconciliation and partial artifact $\rightarrow$ preservation of `UNKNOWN`); `bash scripts/build-macos.sh` — PASS (unsigned `dist/QADREN.app`, `Info.plist`).

`JobInventoryTests.swift` complements this mock-slice with a native Swift$\rightarrow$Rust check on a temporary app-owned root: empty inventory, Queue $\rightarrow$ Pause $\rightarrow$ Resume $\rightarrow$ Run $\rightarrow$ durable `completed`, then a separate Queue $\rightarrow$ Cancel without artifact. The command `bash scripts/test-macos-job-inventory.sh` builds the release Rust static library and this runner; it does not launch QADREN.app, does not use the user's Application Support, and does not read user/cloud content. This is ABI/bridge evidence, not visual UI smoke or real data transfer.

The next G1 safety package distinguishes between a known failure before effect and an ambiguous result: if app-owned `create_new` atomically reports that the target is already occupied, Rust moves the bound active attempt to `failed_before_effect` and the Job to `failed` in a single SQLite transaction. Receipts and successful verification are not fabricated; existing bytes remain untouched. Partial write, ordinary I/O error, crash after possible effect, and failed verification still become `UNKNOWN` after reopen and require only read-only reconciliation. Schema migration v9 preserves legacy active/unknown/resolved attempts; only active/unknown continue to block completion, cancel, pause, decisions, and credential deletion.

Successful completion of a verified attempt is now also a single SQLite transaction: `resolved` attempt, evidence level, `completed` Job, and both events are written together. Therefore, a crash cannot leave a resolved attempt under a `running`/`UNKNOWN` Job that can no longer be safely repeated or reconciled. Both normal mock completion and its read-only recovery use this common Rust boundary; it does not add real provider transfer or change the consent policy.

### Settings: connections and Scenario configuration — 2026-10-12

"Settings" remains a separate window and uses real sections: independent `StorageConnection`, saved Scenarios, and "App Settings". SwiftUI calls the existing Rust Registry FFI for list/create/enable/update Scenario; the form saves the name, two connection IDs, directories, direction, and revision 1. Changing the configuration transactionally increases the revision and saves a separate immutable snapshot of both revisions without changing the Scenario enablement; a pinned Job in the future will be able to take its specific configuration rather than the current string. Enabling/disabling also changes the durable Registry. "App Settings" contains a local theme in the same left menu and does not mix it with synchronization settings. In accordance with D-017, iCloud is absent from active interface items.

Scenario creation is an editor in the detail zone, not a sheet. Its upper part contains the name, two exchange points, and direction; below are two metadata-only directory browsers. "This Mac" is available as a real `local_filesystem` connection: an independent Registry record is created during the first save, then reused. Yandex Disk loads only the first page of metadata for the selected directory (not file bytes); scenario launch and file transfer are still absent.

Boundary: this is configuration-only UI. There is creation, enablement, and revision-safe change of Scenarios; there is no Scenario deletion, scheduler, dry-run launch, Job submission, or file transfer. The screen explicitly reports these limitations and does not simulate Run/copying. Connection deletion still uses the existing recoverable Keychain/attempt gate.

Verified on macOS for this change: `bash scripts/test-core.sh` — PASS (fmt, Clippy, 220 Rust unit tests, including C ABI create/list/enable/update Scenario and v3 migration with snapshot preservation); `bash scripts/build-macos.sh` — PASS (unsigned `dist/QADREN.app`, `Info.plist`). Native visual/VoiceOver smoke for this version is not confirmed: session UI automation became unavailable after application restart.

### File Manager: read-only metadata slice — #22 (2026-10-13)

"File Manager" is opened as a separate macOS window from the `Space` menu and Menu Bar, therefore it does not return a permanent Sidebar to the monitoring panel. In the window, there are two independent local panels with root selection, breadcrumb navigation, and row selection; they read only names, types, and sizes via the existing Local Filesystem FFI.

Boundary: this is a browsing-only slice. Content preview and download, directory creation, copying, moving, renaming, and deletion are disabled: the direct Swift call `qadren_local_mkdir` is removed. No Job, Scenario, consent, or file effect is created by opening/updating the window. Modification operations will be added only together with a safe Job Engine and its durable attempt/reconciliation contract.

`bash scripts/test-core.sh` and `bash scripts/build-macos.sh` were performed for this slice; native visual/VoiceOver smoke was not performed separately.

### Monitoring layout: per-window local composition — #23 (2026-10-14)

- Monitoring preserves the local order and visibility of four existing widgets (`System`, `Jobs`, `Queue`, `Connections`) separately for each SwiftUI scene via `@SceneStorage` panel ID and `UserDefaults`. In "Layout" mode, a widget can be rearranged or hidden; a temporary catalog returns only hidden standard widgets, and a reset restores the baseline. There is no permanent Sidebar, arbitrary HTML widgets, or second data/Job layer.
- In "Layout" mode, native toolbar actions Undo/Redo (`Cmd-Z` / `Shift-Cmd-Z`) are available for the last series of local changes. History is limited to 50 states, does not survive window restart, and does not affect Core data; each undo/redo saves the current presentation layout just as the original change did.
- For each widget, density is saved separately as `Compact` / `Normal` / `Detailed`; this changes only secondary information and the volume of actually available rows, does not scale the entire interface, and does not create metrics. A missing field in a layout saved by a previous version is safely read as `Normal`.
- In the same mode, an adaptive grid size is available separately for each panel as `Compact` / `Normal` / `Spacious`: it changes only the deterministic reflow threshold (`230` / `270` / `360` pt), does not scale chrome or widget content. This presentation-only field participates in Undo/Redo; layout in the old array format still opens with a normal grid.
- Only presentation fields are saved. Layout does not change Job, Scenario, StorageConnection, Worker, or files. Corrupted saved JSON opens with a safe default layout, while the original record is not automatically deleted.
- `bash scripts/test-macos-dashboard-layout.sh` verifies persistence of order/visibility, independence of two panel IDs, and fail-safe recovery of corrupted layout. The native app was opened for inspection of the monitoring surface; subsequent UI automation connection was interrupted, so visual/VoiceOver acceptance for the new layout remains BLOCKED and is not declared as PASS.

### Scenario Schedules: durable configuration UI — #21 (2026-10-13)

For an existing Scenario, several weekly schedules can be created in a separate settings panel, their IANA timezone can be viewed, each can be temporarily disabled individually, or deleted. Configuration is stored in the existing Rust Registry; SwiftUI accesses it only via new narrow C ABI list/create/enable/remove. The schedule deletion operation deletes its receipts for already evaluated slots first, then the configuration itself, in a single SQLite transaction, and does not affect Scenario, Job, or files. Disabling preserves receipts, so re-enabling should not reproduce an old slot.

Boundary: the interface explicitly states that the scheduler does not yet create Jobs. There is no `next run`, background timer, content I/O, Scenario execution, consent, or file transfer. Native visual/VoiceOver smoke for this SHA is not performed; static Core/FFI and macOS build checks are listed in evidence Issue #21.

### Confirmed FileSyncState — #19 (2026-10-13)

Registry schema v8 stores FileSyncState separately from the Scenario string: the key includes Scenario ID, immutable revision, and relative path; the record retains only metadata identity/size/MD5 for each side, without content or secrets. Writing is allowed only with `MetadataVerified` or stronger evidence; `Requested`, provider acknowledgement, and `UNKNOWN` fail-closed and do not advance the baseline. A new revision starts without an inherited baseline, so its dry-run cannot silently use the state of another configuration.

This package does not perform an adapter scan, does not call the planner via FFI/UI, does not create Jobs, and does not confirm real synchronization. It creates only a durable contract that a future executor must call after an independent result verification.
