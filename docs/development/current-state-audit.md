# QADREN Current-State Audit

**Snapshot date:** 2026-10-17. This document describes the current code and recorded local evidence. It is not a task tracker or a release claim.

## Source and Verification Boundary

This audit uses source reading, the local Git history, and recorded local commands. Re-run relevant checks on the exact `HEAD` before making a release claim. A successful local command is not a hosted CI result. An unavailable check remains unavailable until it is run and recorded.

## Current Capability Matrix

| Area | Current implementation | Current boundary |
|---|---|---|
| Rust Core, SQLite, and FFI | `model.rs`, `registry.rs`, `jobs.rs`, `jobs_ffi.rs`, `confirmation.rs`, and `policy.rs` provide the shared model. Manual and Scenario Local-to-Local copies use durable Jobs. | A real File Provider materialization run with a user-selected provider item is not recorded. |
| App-owned mock Job | `job_execution.rs` has app-owned no-overwrite create, durable attempt, verification, and recovery tests. | It is a test boundary. Do not present it as user-file transfer. |
| Local Filesystem | `local_fs.rs`, `local_fs_unix.rs`, `local_fs_ffi.rs`, `jobs_ffi.rs`, and `LocalWorkspace.swift` provide metadata operations, local metadata revisions, no-overwrite file copy, controlled replacement, and safety guards. The File Manager enables replacement only after a current preview reports an existing target and after a separate destructive confirmation. A content operation requires an exact durable `read_content` decision before opening the source. | A real File Provider materialization run with a user-selected provider item is not recorded. |
| Scenario | `scenario_planner.rs`, Registry revisions, `FileSyncState`, dry-run FFI, and the gated local metadata snapshot exist. User-confirmed `Create` items submit pinned no-overwrite copy Jobs. User-confirmed one-way `Update` items submit pinned controlled-replacement Jobs. The completion transaction stores a baseline outbox. The app flushes that outbox before each dry-run. Delivery writes the revision-bound `FileSyncState` idempotently. | Two-way execution and conflict execution are not verified. |
| Scheduler | `scheduler.rs`, `schedule_dispatch.rs`, and dispatch FFI create queued Jobs from durable configuration. | No independent timer, background lifecycle, event debounce, or Scenario execution is verified. |
| SwiftUI UI | `DesignSystem.swift`, `DashboardLayout.swift`, `QADRENApp.swift`, `MainView.swift`, and `SettingsPanel.swift` provide the native baseline. | Native visual acceptance, complete keyboard accessibility, and removal of legacy mock actions remain incomplete. VoiceOver support is outside the current scope. |
| Local file manager | File navigation, `mkdir`, copy preview, a confirmed one-file copy, Scenario `Create` and one-way `Update` submission, and persisted UNKNOWN-copy inventory with an explicit check action are present. Copies use Rust JobStore and metadata verification. | No user-file File Provider materialization evidence is recorded. |
| Yandex Disk | `yandex.rs`, `yandex_ffi.rs`, and `YandexConnection.swift` remain in the source tree. | It is post-release work. Do not add API, OAuth, transfer, or live end-to-end work in Release 1. |
| iCloud | `ICloudAvailability.swift` contains legacy availability metadata. | It is post-release work. Do not add an adapter, entitlement, UI integration, or feasibility test in Release 1. Keep Local File Provider guards. |
| Background service | Native windows expose `workerStatus`. | An independent service after UI closure is not verified. |
| Distribution | `scripts/build-macos.sh` produces an unsigned development bundle. | Signing, notarization, clean installation, restart validation, and release packaging remain incomplete. |

## Recorded Local Evidence

The executable code baseline is commit `a0b75f7`.

| Command | Recorded result | Scope |
|---|---|---|
| `cargo test --manifest-path core/Cargo.toml --lib` | PASS, 265 tests | Rust Core on `a0b75f7` |
| `scripts/test-macos-local-workspace.sh` | PASS | SwiftUI to Rust Local file-copy bridge on `a0b75f7` |
| `scripts/test-macos-scenario-dry-run.sh` | PASS | SwiftUI Scenario metadata and FFI bridge on `a0b75f7` |
| `./script/build_and_run.sh --verify` | PASS | Unsigned local development bundle and running QADREN process on `a0b75f7` |

These checks do not prove a signed release, an independent Worker, or File Provider materialization with a user-selected provider item.

## Active Product Boundaries

1. Reuse the existing native UI and app-owned mock. Do not restart baseline work.
2. Record an explicit one-time consent before any macOS read that can materialize a File Provider item.
3. Keep the durable Local-to-Local no-overwrite copy Job as the user-facing content-copy executor. Use controlled replacement only after separate destructive consent and exact Scenario or File Manager validation.
4. Keep local Scenario `Create` and one-way `Update` paths bound to their exact Scenario revision, with new consent for each source file.
5. Add two-way execution, conflict handling, independent local scheduling, service lifecycle, and release evidence.
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
