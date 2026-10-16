# QADREN — audit of current `main` and discrepancies with tasks

**Verified 2026-10-14, after user push.** Product code snapshot — commit [`1223e8f`](https://github.com/Xsoid/QADREN/commit/1223e8f29f94). D-018 and document updates are also integrated into `main`. **Audit based on code reading and GitHub API**, not a new native test build. The early report for SHA `44538fe` from 12 October is **obsolete**: local work from 13–14 October is now in `main`.

## Verification Boundaries and CI

The following are inspected: GitHub source tree, documentation, all issues, commit sequence, implementation files, and GitHub Actions runs. The latest verified CI jobs on `b1a3a22` are `rust: failure` and `macos: failure`, run [38599805324](https://github.com/Xsoid/QADREN/actions/runs/38599805324). The API did not return step details; downloading logs previously returned NOT_FOUND. **The cause of failure is not determined.** Local PASS results from issue comments before the push are author statements regarding those commits, but they do not replace an independent fresh verification of HEAD; neither compilation nor native visual acceptance is declared proven.

## Source and Status Matrix

| Area | Presence in `main` | Actual Remaining Status |
|---|---|---|
| Rust Core/SQLite/FFI | `model.rs`, `registry.rs`, `jobs.rs`, `jobs_ffi.rs`, `confirmation.rs`, `policy.rs` | Modules exist; no proven production Local↔Local execution via Job |
| Mock G1 Job | `job_execution.rs` — app-owned no-overwrite create + durable attempt/verification/recovery tests | Do not repeat; move to secure **real Local FS** vertical #18/#15 |
| Local FS | `local_fs.rs`, `local_fs_unix.rs`, `local_fs_ffi.rs`, `LocalWorkspace.swift` | Metadata/operations/guards API exists; UI→Job→Local FS→verified SQLite E2E is not confirmed |
| Scenario | `scenario_planner.rs`, Registry revisions/FileSyncState, dry-run FFI | Real metadata snapshots→Job submission→sync/recovery and conflicts are not proven |
| Scheduler | `scheduler.rs`, `schedule_dispatch.rs` and dispatch FFI (`1223e8f`) | Outbox→queued Job exists; independent timer, background lifecycle, event debounce, and Scenario execution are absent |
| UI theme/widget | `DesignSystem.swift`, `DashboardLayout.swift`, `DashboardLayoutTests.swift` | **Partially implemented:** QDTheme/QDWidgetFrame/QDLogo, per-window order/visibility, density, grid scale, Undo/Redo/persistence. Visual and native acceptance is required, not implementation from scratch |
| UI windows/settings | `QADRENApp.swift`, `MainView.swift`, `SettingsPanel.swift` | Dashboard adaptive grid, separate Monitoring/Settings/File Manager, MenuBarExtra. Integration extension and keyboard accessibility are still partial; VoiceOver support is out of scope; `MainView` contains legacy/mock actions |
| Yandex | `yandex.rs`, `yandex_ffi.rs`, `YandexConnection.swift` | Real OAuth/metadata/upload code and mock tests remain, but the controller is **moved beyond Release 1**; live E2E is not confirmed; #26 removes active Add/Connect/API flows without deleting tokens/connections |
| iCloud | `ICloudAvailability.swift` (legacy metadata/status) | #17 `closed/not_planned`; do not develop, UI integration is absent/not required; Local FS guards remain |
| Background service | Native windows and `workerStatus` | Independent service after UI closure is **not proven**, #23 P2 |
| Distribution | `scripts/build-macos.sh`, `.github/workflows/bootstrap.yml` | Unsigned dev bundle in script; native install/sign/restart/packaging and CI diagnostics are still required |

## Issues — Managed Queue (D-018)

- **P0 now — only #26:** complete existing `DesignSystem`/widget dashboard with native UI and build acceptance; **#13 is archived as superseded/not planned**, not VERIFIED: its baseline audit is complete, remainders are transferred to #26 (UI/macOS tests), #24 (GitHub Actions pre-runner failure, runtime resources/release), and #25 (Keychain/legacy migration/recovery).
- **P1:** #15 Local FS; #18 Job engine; #20 policy; #25 remove/disconnect coordination. Acceptance — one real local operation via a single JobStore and a verifiable durable outcome. #22 dual-pane UI and #19 local Scenarios may be developed in independent secure parts.
- **P2:** #21 scheduler runtime and catch-up; #23 independent service + final UI integration; #24 security/build/local distribution.
- **P3 post-release, parked:** #16 Yandex Disk controller. Preserve sources; it is not a Release 1 gate.
- **Indefinitely parked:** #17 iCloud, closed. #2/#7/#14 are already closed, without re-implementation.

Open issues are **PARTIAL** if the full DoD is not passed; do not close based on a single fresh commit or number of unit tests. #26 is created [separately](https://github.com/Xsoid/QADREN/issues/26) so that UI P0 does not depend on background service #23 P2.

## Priority Discrepancies

1. Project `main` now contains UI widget canvas/layout; old UI-A/B/C "create from scratch" tasks are obsolete. Reuse existing components.
2. Design system and logo exist, but native dark/light, screenshot, and layout acceptance are not proven. VoiceOver support is out of scope.
3. The first real Local controller slice is `local_mkdir_new`: the file manager button calls the Rust JobStore, which saves the attempt before `mkdir`, uses no-overwrite, and confirms the type of created folder. This is not file transfer and does not confirm Local↔Local copy readiness.
4. Yandex connection is historically present in code/UI (`SettingsPanel.swift` / `YandexConnectionCard`), but the owner decision is to postpone the controller beyond Release 1. Do not start new API developments. Hide/disable new flows without destroying existing credentials.
5. Old Release 1 texts link the local release to Yandex OAuth/content/E2E. D-018 **breaks this dependency** and requires local-first.
6. CI on GitHub is currently red (for docs and code commits). Logs are unavailable: establish reproducible diagnostics, do not guess, and do not consider tests passed.
7. The absence of an independent Worker and live Local↔Local synchronization cannot be compensated by a widget or mock demo.

**Next executable result:** complete #26 visually verifiable P0 without rewriting current components; CI infrastructure diagnostics are recorded in #24, Keychain in #25; then link #18/#15/#20 with real Local FS. Work directly in `main` with mandatory verification of the actual SHA. Historical G0–G6 and old cloud requirements are not in the active queue.
