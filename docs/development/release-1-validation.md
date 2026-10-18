# Release 1 Validation Record

Status: current validation contract and evidence summary. This document does not define product scope. [D-018](../decisions/D-018-local-first-priorities.md) and the [roadmap](../roadmap.md) own the product sequence.

## Scope

Release 1 is a local-first macOS product. It includes one Rust JobStore, Local Filesystem connections, manual local operations, local Scenarios, and later independent local scheduling. Yandex Disk and iCloud are post-release work.

## Evidence Rules

- Record the exact commit SHA, worktree state, operating-system version, and command for each result.
- A test result applies only to the executable source at that SHA.
- A local result does not prove hosted CI, a signed artifact, or a production release.
- A successful unit test does not prove user-file transfer, restart safety, or a destructive-operation guard.
- Record unavailable validation as unavailable. Do not replace it with inference.
- Keep external work-tracker links outside this document. They are not validation evidence.

## Current Recorded Local Evidence

The initial recorded executable baseline is `f9121b2`. Later rows record newer checked commits.

| Check | Result | Evidence scope |
|---|---|---|
| `cargo clippy --manifest-path core/Cargo.toml --lib -- -D warnings` | PASS | Rust Core lint on `f9121b2` |
| `cargo test --manifest-path core/Cargo.toml --lib` | PASS, 269 tests | Rust Core on `f9121b2` |
| `scripts/test-macos-local-workspace.sh` | PASS | SwiftUI to Rust Local file-copy bridge on `f9121b2` |
| `scripts/test-macos-scenario-dry-run.sh` | PASS | SwiftUI Scenario metadata, conflict queue, and FFI bridge on `f9121b2` |
| `./script/build_and_run.sh --verify` | PASS | Unsigned local development bundle and running QADREN process on `f9121b2` |
| `scripts/test-macos-scenario-dry-run.sh` | PASS | Native Swift/FFI Scenario bridge build on `40bd428` |
| `./script/build_and_run.sh --verify` | PASS | Unsigned local development bundle and running QADREN process on `40bd428` |
| `cargo test --manifest-path core/Cargo.toml --lib` | PASS, 275 tests | Rust Core on `c5f479b` |
| `scripts/test-macos-scenario-schedules.sh` | PASS | Native schedule dispatch bridge and duplicate dispatch check on `c5f479b` |
| `scripts/test-macos-scenario-dry-run.sh` | PASS | Native Scenario bridge build on `c5f479b` |
| `./script/build_and_run.sh --verify` | PASS | Unsigned local development bundle and running QADREN process on `c5f479b` |
| `scripts/test-core.sh` | PASS, 275 tests | Rust format, Clippy, unit, and doc tests on `9a67258` |
| `scripts/test-macos-storage-removal.sh` | PASS | Native Local connection removal bridge on `9a67258` |
| `scripts/test-macos-keychain.sh` | PASS | Native Keychain smoke test on `9a67258` |

The Rust format gate now passes on `9a67258`. The earlier Core formatting debt is resolved.

## Manual macOS Evidence Required

Run these checks only in disposable folders. They do not authorize cloud adapter work.

1. Create two different Local Filesystem folders. Put a small known file in the source folder. In **Настройки → Сценарии**, create a one-way Local Filesystem Scenario. Select the source and destination roots in the Scenario editor. Open **Проверить план**, confirm that the file is a `Create` item, then select **Создать копию** and confirm. Verify the destination bytes in Finder or Terminal. Quit and reopen QADREN. Verify that the Job remains completed. Do not configure a second pair of roots outside the Scenario.
2. Repeat the first check with the same destination name. Verify that ordinary copy is unavailable and that QADREN requires the separate destructive replacement confirmation through **Заменить файл**. If you confirm replacement, verify the new destination bytes. If you cancel, verify that the destination bytes stay unchanged.
3. If a File Provider item is available in Finder, repeat the first check with one not-yet-local provider file. QADREN must show the exact materialization confirmation before it opens the file. On cancel, verify that no destination file appears. On confirm, record whether macOS materializes the item and whether QADREN completes or reports an error. Do not create an iCloud connection for this check.
4. Create a weekly Scenario schedule for one minute in the future. Close all QADREN windows without using **Выйти из QADREN**. Wait two minutes, reopen the monitoring window, and verify one queued revision-pinned Job. This check verifies the current app-resident timer only. It does not verify execution or operation after **Выйти из QADREN**.

## Current Capability Evidence Matrix

| Capability | Current evidence | Not yet established |
|---|---|---|
| Rust model and durable JobStore | Core test suite and Scenario Local-to-Local no-overwrite copy paths pass on the recorded baseline. Rust core and FFI have a staged controlled replacement for an existing regular Local file. The Scenario view submits it only from a current revision-bound plan. | Real File Provider materialization evidence is not recorded. |
| App-owned mock operation | No-overwrite, durable-attempt, verification, and recovery tests exist in `job_execution.rs`. | User content transfer. The mock is not a release substitute. |
| Local Filesystem metadata, `mkdir`, and copy | Rust and SwiftUI interfaces provide confirmed one-file no-overwrite copy through JobStore. Local metadata revisions support a revision-bound Scenario baseline. | A real File Provider materialization run. |
| macOS File Provider safety | A copy requires a user-confirmed, exact-scope durable decision before opening the source. | A real File Provider materialization run with a user-selected provider item. |
| Scenario planning and conflict queue | Dry-run FFI passes. Enabled local connections gate the metadata snapshot. User-confirmed `Create` and `Update` items submit pinned Jobs in a permitted direction and write `FileSyncState` through a durable outbox. A revision-bound Registry queue stores both conflict snapshots and a user-selected source and decision. Selected content decisions create a pinned replacement Job only after a second snapshot check. A selected deletion has a separate destructive confirmation, verifies an absent source and current destination snapshot, then creates a pinned delete Job. An interrupted delete is `UNKNOWN`. A mismatch marks the task `stale` without an effect. The queue has no JobStore dependency and does not block ordinary transfers. A user-confirmed ordinary batch uses the same per-file Jobs, runs away from the SwiftUI main actor, shows progress, and continues after one item fails. | End-to-end batch interaction, autonomous multi-item execution, delete recovery, and real File Provider delete evidence. |
| Scheduler dispatch | Durable schedule dispatch can queue a Job. The running QADREN process checks due slots once per minute and sends them through the same idempotent dispatch boundary. | Independent service after QADREN quits, event debounce, and real Scenario execution. |
| Native UI | Design system, widget canvas, settings, and local file navigation exist. | Full visual acceptance, removal of legacy mock actions, and real-copy interaction. |
| Distribution | The macOS build script creates an unsigned development bundle. | Signing, notarization, clean install, restart validation, and release package. |

## Mandatory Gates for a Real Local Copy

1. Create a durable attempt before the external filesystem effect.
2. Check capability, operation-specific consent, destination state, and no-overwrite for a new copy. A controlled replacement needs its approved replacement protocol.
3. Record and consume exact one-time consent before reading user content that can materialize a File Provider item.
4. Verify the copied destination after the effect and store the durable result.
5. Use `UNKNOWN` if proof is lost. Reconcile in read-only mode. Do not retry blindly.
6. Prove concurrent-change protection before any destructive Local Filesystem operation.
7. Exercise cancel, crash, restart, permission loss, destination conflict, and repeated request paths.

## Mandatory Gates for Release Claim

1. Run the Core, native, adapter, and integration checks that cover the final executable SHA.
2. Run the macOS build and verify the installed artifact on a clean user environment.
3. Verify UI behavior without mock success values.
4. Verify independent service behavior after UI exit and after restart.
5. Record signing and notarization status.
6. Run the project-scoped pre-publish gate, code review, and security review on the exact final patch before commit, push, or pull request.

Use [current-state audit](current-state-audit.md) for the code snapshot and [testing and quality](testing-and-quality.md) for test selection.
