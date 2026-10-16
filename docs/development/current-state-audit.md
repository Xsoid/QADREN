# QADREN Current-State Audit

**Snapshot date:** 2026-10-17. This document describes the current code and recorded local evidence. It is not a task tracker or a release claim.

## Source and Verification Boundary

This audit uses source reading, the local Git history, and recorded local commands. Re-run relevant checks on the exact `HEAD` before making a release claim. A successful local command is not a hosted CI result. An unavailable check remains unavailable until it is run and recorded.

## Current Capability Matrix

| Area | Current implementation | Current boundary |
|---|---|---|
| Rust Core, SQLite, and FFI | `model.rs`, `registry.rs`, `jobs.rs`, `jobs_ffi.rs`, `confirmation.rs`, and `policy.rs` provide the shared model. | No verified user Local-to-Local content copy runs through the complete durable path. |
| App-owned mock Job | `job_execution.rs` has app-owned no-overwrite create, durable attempt, verification, and recovery tests. | It is a test boundary. Do not present it as user-file transfer. |
| Local Filesystem | `local_fs.rs`, `local_fs_unix.rs`, `local_fs_ffi.rs`, and `LocalWorkspace.swift` provide metadata operations and safety guards. | Arbitrary user content reads on macOS remain fail-closed until public residency proof is available for the selected item. |
| Scenario | `scenario_planner.rs`, Registry revisions, `FileSyncState`, dry-run FFI, and the gated local metadata snapshot exist. | No real metadata-snapshot to Job submission, synchronization, recovery, or conflict execution is verified. |
| Scheduler | `scheduler.rs`, `schedule_dispatch.rs`, and dispatch FFI create queued Jobs from durable configuration. | No independent timer, background lifecycle, event debounce, or Scenario execution is verified. |
| SwiftUI UI | `DesignSystem.swift`, `DashboardLayout.swift`, `QADRENApp.swift`, `MainView.swift`, and `SettingsPanel.swift` provide the native baseline. | Native visual acceptance, complete keyboard accessibility, and removal of legacy mock actions remain incomplete. VoiceOver support is outside the current scope. |
| Local file manager | File navigation, `mkdir` Job, and copy metadata preview are present. | A UI action does not yet perform a verified user-file copy. |
| Yandex Disk | `yandex.rs`, `yandex_ffi.rs`, and `YandexConnection.swift` remain in the source tree. | It is post-release work. Do not add API, OAuth, transfer, or live end-to-end work in Release 1. |
| iCloud | `ICloudAvailability.swift` contains legacy availability metadata. | It is post-release work. Do not add an adapter, entitlement, UI integration, or feasibility test in Release 1. Keep Local File Provider guards. |
| Background service | Native windows expose `workerStatus`. | An independent service after UI closure is not verified. |
| Distribution | `scripts/build-macos.sh` produces an unsigned development bundle. | Signing, notarization, clean installation, restart validation, and release packaging remain incomplete. |

## Recorded Local Evidence

The executable code baseline is commit `03e67df`. Later commits in this sequence changed documentation only.

| Command | Recorded result | Scope |
|---|---|---|
| `bash scripts/test-core.sh` | PASS, 252 tests | Rust Core on `03e67df` |
| `bash scripts/test-macos-scenario-dry-run.sh` | PASS | SwiftUI to Rust Scenario dry-run boundary on `03e67df` |
| `bash scripts/build-macos.sh` | PASS | Unsigned macOS development bundle on `03e67df` |

These checks do not prove a signed release, an independent Worker, or user-file content transfer.

## Active Product Boundaries

1. Reuse the existing native UI and app-owned mock. Do not restart baseline work.
2. Establish a safe macOS resident-file read boundary before enabling user content copy.
3. Build one durable Local-to-Local no-overwrite copy Job through the existing Rust JobStore.
4. Connect the local File Manager and Scenario planner only to that verified Job path.
5. Add independent local scheduling, service lifecycle, and release evidence after the copy path is safe.
6. Keep Yandex Disk and iCloud parked until a separate owner decision.

## Required Evidence Before a Release Claim

- Record the exact SHA and a clean or explicitly described worktree.
- Run Core, native, adapter, and integration checks that cover the changed behavior.
- Demonstrate durable attempt creation before each external effect.
- Demonstrate capability, consent, destination-state, and no-overwrite checks.
- Demonstrate `UNKNOWN` and read-only reconciliation when effect proof is lost.
- Demonstrate restart and concurrent-change safety for destructive local operations.
- Build, install, restart, and verify the macOS artifact. Record signing and notarization status.

Read [D-018](../decisions/D-018-local-first-priorities.md), the [roadmap](../roadmap.md), and the matching policy before changing this sequence.
