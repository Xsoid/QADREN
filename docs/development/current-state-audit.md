# QADREN Current-State Audit

**Snapshot date:** 2026-10-19. This document describes the current code and recorded local evidence. It is not a task tracker or a release claim.

## Source and Verification Boundary

This audit uses source reading, the local Git history, and recorded local commands. Re-run relevant checks on the exact `HEAD` before making a release claim. A successful local command is not a hosted CI result. An unavailable check remains unavailable until it is run and recorded.

## Current Capability Matrix

| Area | Current implementation | Current boundary |
|---|---|---|
| Rust Core, SQLite, and FFI | `model.rs`, `registry.rs`, `jobs.rs`, `jobs_ffi.rs`, `confirmation.rs`, and `policy.rs` provide the shared model. Manual and Scenario Local-to-Local copies use durable Jobs. | A real File Provider materialization run with a user-selected provider item is not recorded. |
| App-owned mock Job | `job_execution.rs` has app-owned no-overwrite create, durable attempt, verification, and recovery tests. | It is a test boundary. Do not present it as user-file transfer. |
| Local Filesystem | `local_fs.rs`, `local_fs_unix.rs`, `local_fs_ffi.rs`, `jobs_ffi.rs`, and `LocalWorkspace.swift` provide metadata operations, local metadata revisions, no-overwrite file copy, controlled replacement, and safety guards. The Scenario plan enables replacement only after a current revision-bound plan reports an existing target and after a separate destructive confirmation. A requested copy reads only its selected source and has no separate content-read decision. Reconnecting a selected root automatically performs read-only reconciliation of bound UNKNOWN local operations. | A real File Provider materialization run with a user-selected provider item is not recorded. |
| Scenario | `scenario_planner.rs`, Registry revisions, `FileSyncState`, dry-run FFI, and the gated local metadata snapshot exist. User-confirmed `Create` items submit pinned no-overwrite copy Jobs. User-confirmed `Update` items submit pinned controlled-replacement Jobs in either permitted direction of a bidirectional Scenario. The completion transaction stores a baseline outbox. The app flushes that outbox before each dry-run. Delivery writes the revision-bound `FileSyncState` idempotently. The Registry has a revision-bound alternative conflict queue. The user must select a left or right source before a content or deletion decision is stored. Selected content tasks run only after both stored snapshots still match. `replace_if_newer` uses Local Filesystem mtime fields and does nothing if its selected source is not newer. A selected deletion task requires another destructive confirmation. It verifies an absent source and the saved destination snapshot, then creates a durable delete Job. An interrupted delete stays `UNKNOWN`. The queue is not a JobStore queue and cannot block ordinary transfers. A confirmed ordinary batch runs its existing per-file executors sequentially on a detached task. The app shows per-file progress and continues after an item failure. | End-to-end batch interaction and autonomous Scenario execution are not verified. Real File Provider delete evidence is not recorded. |
| Scheduler | `scheduler.rs`, `schedule_dispatch.rs`, and dispatch FFI create queued Jobs from durable configuration. While the QADREN process runs, `QADRENApp.swift` invokes the dispatch boundary once per minute on a serial utility queue with a 90-second idempotent overlap. This only queues revision-pinned Jobs. | No independent service after QADREN quits, event debounce, or Scenario execution is verified. |
| SwiftUI UI | `DesignSystem.swift`, `DashboardLayout.swift`, `QADRENApp.swift`, `MainView.swift`, and `SettingsPanel.swift` provide the native baseline. | Native visual acceptance, complete keyboard accessibility, and removal of legacy mock actions remain incomplete. VoiceOver support is outside the current scope. |
| Scenario plan and transfer UI | The Scenario editor is the only user configuration surface for endpoint roots, direction, and policies. Its plan view submits `Create` and `Update` items in a permitted direction and uses Rust JobStore and metadata verification. | No user-file File Provider materialization evidence is recorded. |
| Yandex Disk | `yandex.rs`, `yandex_ffi.rs`, and `YandexConnection.swift` remain in the source tree. | It is post-release work. Do not add API, OAuth, transfer, or live end-to-end work in Release 1. |
| iCloud | `ICloudAvailability.swift` contains legacy availability metadata. | It is post-release work. Do not add an adapter, entitlement, UI integration, or feasibility test in Release 1. Keep Local File Provider guards. |
| Background service | Native windows expose `workerStatus`. | An independent service after UI closure is not verified. |
| Distribution | `scripts/build-macos.sh` produces an unsigned development bundle. | Signing, notarization, clean installation, restart validation, and release packaging remain incomplete. |

## Native Scenario Execution Coverage

`platform/macos/Tests/ScenarioDryRunBridgeTests.swift` uses UUID-scoped disposable Local Filesystem roots. It calls the production Swift-to-Rust FFI bridge. It does not automate Settings UI controls or use a File Provider item.

| Flow | Recorded native evidence | Current boundary |
|---|---|---|
| One-way Create and multiple items | Two source-only files produce two pinned `Create` Jobs. Exact destination bytes, completed Jobs, Scenario revision, and no-action follow-up plan are checked. | This is bridge evidence. It does not prove the detached Settings batch progress UI. |
| Baseline and one-way Update | Completed Creates have already delivered their idempotent `FileSyncState` baselines. A later source-only change produces `Update`. A declined replacement creates no Job or file effect. A confirmed replacement records one completed Job. | The native alert interaction is not automated. |
| Alternative content replacement | A selected conflict with current metadata rejects a declined replacement. A confirmed replacement changes only the selected destination, records one Job, and keeps a completed conflict record. | The Settings conflict presentation is not automated. |
| Alternative deletion | A selected absent-source deletion rejects a declined delete. A confirmed delete removes only the selected destination and records one completed Job. | Mass-deletion presentation is not covered. |
| Two-way Create and Update | A right-only file creates a `from_left=false` proposal. A later right-only change creates a right-to-left `Update`. The Rust bridge validates the permitted bidirectional endpoint direction. | The fixture covers one file per direction. |
| Concurrent two-way changes | Two changed endpoints produce `conflict`. The dry run creates no Job and changes no file. | The fixture does not automate a later user decision for this planned conflict. |
| Negative bridge gates | An unavailable Scenario revision creates no Job or destination file. A disabled Local connection blocks a metadata snapshot before a scan. | Case and Unicode collision evidence is in Core tests, not this native fixture. |
| UNKNOWN, cancellation, and restart | Core tests cover durable UNKNOWN, cancellation state limits, and read-only reconciliation. | Native Scenario bridge restart, cancellation, and Settings reopen evidence are not recorded. |

The recorded native Scenario fixture passed on `6e596e5`. Run it again on the exact release candidate. Do not treat this matrix as proof of Worker autonomy, File Provider compatibility, or a signed release.

## Recorded Local Evidence

The initial recorded executable code baseline is commit `f9121b2`. Later rows record newer checked commits.

| Command | Recorded result | Scope |
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
| `scripts/test-macos-scenario-dry-run.sh` | PASS | Native one-way/two-way Scenario Create and Update, conflict replacement/delete, stale revision, disabled connection, and concurrent conflict on `6e596e5` |
| `scripts/test-core.sh` | PASS, 275 tests | Rust Core on `6e596e5` |
| `scripts/build-macos.sh` | PASS | Unsigned local macOS development bundle on `6e596e5` |

The Rust format gate now passes on `9a67258`. The earlier Core formatting debt is resolved.

These checks do not prove a signed release, an independent Worker, or File Provider materialization with a user-selected provider item.

## Policy / Code Mismatch (2026-10-18)

The source snapshot above recorded the historical one-time `read_content` decision path. The owner authorizes silent reads and possible File Provider hydration for content required by a requested or configured operation. The active Local copy path has no separate content-read decision. Keep separate confirmation only for destructive effects. Reconnecting a selected Local root automatically runs only non-mutating reconciliation. No real File Provider materialization test has been recorded.

## Active Product Boundaries

1. Reuse the existing native UI and app-owned mock. Do not restart baseline work.
2. **Approved policy:** read file content required by an active/configured operation without an extra per-file prompt, even if File Provider materializes it. The Rust Local Filesystem copy path has no `read_content` decision. Do not prefetch unrelated content. Real File Provider compatibility evidence is still pending.
3. Keep the durable Local-to-Local no-overwrite copy Job as the user-facing content-copy executor. Use controlled replacement only after separate destructive consent and exact Scenario validation.
4. Keep Scenario `Create` and `Update` paths bound to their exact Scenario revision. Ask before destructive replacement (unless already approved for that precise scope) and always confirm mass-deletion plans. Source reads require no separate consent under the latest owner decision.
5. Verify end-to-end batch interaction. Add autonomous multi-item Scenario execution, independent local scheduling, service lifecycle, and release evidence. Keep conflict decisions in the alternative Registry queue.
6. Keep Yandex Disk and iCloud parked until a separate owner decision.

## Required Evidence Before a Release Claim

- Record the exact SHA and a clean or explicitly described worktree.
- Run Core, native, adapter, and integration checks that cover the changed behavior.
- Demonstrate durable attempt creation before each external effect.
- Demonstrate capability, correct actual scope, destructive-consent checks where applicable, destination-state, and no-overwrite for new files.
- Demonstrate **automatic** non-mutating reconciliation after an uncertain effect and honest persistent `UNKNOWN` if checks cannot prove the result.
- Demonstrate restart and concurrent-change safety for destructive local operations.
- Build, install, restart, and verify the macOS artifact. Record signing and notarization status.

Read [D-018](../decisions/D-018-local-first-priorities.md), the [roadmap](../roadmap.md), and the matching policy before changing this sequence.
