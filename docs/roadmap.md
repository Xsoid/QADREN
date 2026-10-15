# Roadmap and Acceptance Criteria

Status: target plan, not a performance report.

## Phase 0 — Documentation Normalization

- README and AGENTS identify synchronization as the primary goal; BackupSet is marked as future-only.
- The existing metadata-only Yandex adapter is not presented as a completed transfer.
- Do not change working code for terminology only without a migration plan.

## Updated Executable Queue D-018 — 2026-10-14

**New owner priority: foundation and interfaces → local operations → autonomous local product.** The Yandex Disk controller is moved outside of Release 1; iCloud is deferred indefinitely. This overrides the previous Release 1 cloud-first scope. [Decision](decisions/D-018-local-first-priorities.md) · [audit](development/current-state-audit.md).

| Priority | Issues | Executable Result / Gate |
|---|---|---|
| **P0 / NOW** | #26 | Finalize existing SwiftUI design system and widget monitoring; local macOS build/visual check on a specific SHA. Archive #13 is closed as superseded; unresolved CI and release issues are in #24, Keychain in #25 |
| **P1 / LOCAL CORE** | #15, #18, #20, #25 | Verifiable secure chain UI→Rust JobStore→Local FS→verification→SQLite→UI; no-overwrite, durable attempts, consent, recovery; no second Job engine |
| **P1 / LOCAL PRODUCT** | #19, #22 | Existing Scenario and dual-pane manager: start with metadata/dry-run and UI; implement secure execution via #18/#20 as they become ready. One issue per function; complex criteria may finish later, but priority remains P1 |
| **P2 / AUTONOMY** | #21, #23, #24 | Scheduler/queue, independent background service, stable multi-pane UI, local distribution, and Local↔Local security check |
| **P3 / POST-RELEASE, PARKED** | #16 | Yandex Disk controller is moved outside of Release 1; no new API/transfer/OAuth/live E2E work until a new owner decision and API availability check. Preserve code and mock tests |
| **INDEFINITE PARKED** | #17 (closed) | iCloud: do not implement, do not create dependencies, do not perform entitlements/spike |

### Task Stability and Requirement Sources

**Issue numbering is stable.** A change in priority `P0/P1/P2/P3` is an **edit to an existing task**, not a replacement, re-issue, or change of number. Do not use ranges such as `P1–P2` for child issues: each has one current starting priority; subsequent acceptance criteria remain in the same issue. Epic #12 may cover all phases. **The only new thematic P0 issue after the audit is #26** for the existing UI; the original #13 is archived as `not planned / superseded`, and its unfinished requirements are recorded in #26/#24/#25. Do not create new "audit", "stabilization", or "new plan" issues.

**Hierarchy:** product decisions and safety policies → D-018 (priorities) → this executable roadmap (allocation) → profile architecture and design (how it must work) → specific issue (scope of work) → actual code and verified tests (readiness stage). **Historical wording below the D-018 block in issues does not create a repeat mandatory scope.** When changing the sequence, edit existing documents and issues without creating parallel specifications. Regulatory safety guarantees and confirmed functions must not be removed from requirements for the purpose of reordering the queue.

The previous detailed roadmap and G0–G6 are preserved in [Git history at `44538fe`](https://github.com/Xsoid/QADREN/blob/44538fe480ab17e3ee0c6114aaa3762a1c35d8c9/docs/roadmap.md) as an archived plan, not a second active specification.

**For P0:** #26 is the only active P0 issue; #13 is archived, and its open checks are distributed among #26/#24/#25. Do not wait for G1/G2/Yandex completion; do not invent runtime data. **For P1:** The G1 app-owned mock already exists in `job_execution.rs`; do not rewrite it as if it were missing. The next criterion is a **real local, secure, Job-linked operation**, not a new mock. In P2, do not mix background service readiness with interface component readiness.

**Local release gates:** successful build and tests on a fixed SHA; UI without mock metrics; Local Filesystem controller and Local↔Local scenario with recovery/consent; working independent service and installation. The Yandex Disk controller is a separate post-release scope, not a cloud gate for this release. Policies from `docs/policies/` remain mandatory.

### Local Release Acceptance Scenarios P0–P2

- Independent connection_id are preserved for local connections; cloud independence of credentials remains an invariant for future P3 and does not block the local release.
- One-way and two-way detect changes based on the last agreed state and signal ambiguous conflicts; identical name/size does not prove byte equality.
- Events received during a Job create one subsequent Job; the state of the old Job and its revision are not overwritten.
- A restart, power failure, and subsequent start do not launch a dangerous operation twice without checking its result.
- A manual task receives initial priority; an automatic task does not starve indefinitely; a local pause is preserved after reboot.
- A preview request does not transfer content to the main MacBook; content downloads require explicit individual consent.
- Mass deletion and an unresolved conflict do not result in silent data destruction.
- Multiselect and operations on nested files work in the Job tree without double counting; progress is shown in bytes and files.
- Closing the UI does not interrupt the service; reopening restores windows/panels/navigation and re-reads current states.
- Unknown quota/ETA/Worker state is not displayed as a verified success.
- Yandex Disk and iCloud are not conditions for the local release; new Yandex connections in P0–P2 are marked as deferred, and previous connections/secrets are preserved. No unconfirmed cloud operation is presented as available.

### UI/UX Contract Release 1

For the implementation of the visual layer (#22, #23), the [design system](design/README.md), [widget layout](design/widget-system.md), and [UI checks](design/implementation.md) are mandatory. Monitoring is a customizable **widget canvas without a permanent Sidebar**; a separate settings panel may have navigation. The brand logo is taken from `assets/qadren-logo.png`. The design prototype is for demonstration and is not a confirmation of Core/Worker readiness or permission to activate insecure operations.

## Release 2 — Remote Workers

- Develop and separately approve transport, mutual trust, command delivery, acknowledgments, revisions, logs, and idempotency.
- The Worker handles local permissions/authorization and operational tasks; the Controller changes scenarios.
- The Controller assigns a scenario manually to a compatible Worker; remote unavailability does not mean stoppage.
- Reassignment requires confirmation of the release of the previous Worker; a forced disconnect leaves UNKNOWN Jobs blocked until reconciliation.
- During re-pairing, state matching is performed; the Worker UI remains local.
- Verify execution with the Controller disconnected and subsequent report loading without repeating effects.

## Deferred

**iCloud Documents, synchronization and backup with iCloud:** future capability, not Release 1. Do not develop the container/adapter, UI, entitlements/native spike, or iCloud E2E; the original #17 is closed as not planned. Resume only by a separate product decision after checking the free/paid access method, platform requirements, and cloud confirmation. See [D-017](decisions/D-017-defer-icloud.md).

BackupSet, backup versions and recovery of old content; inter-scenario dependencies; custom Worker ACLs; proprietary search index; quota change analytics; remote UI Controller; Worker software update from Controller.

## Negative Tests

The happy-path alone cannot be tested for preservation. Connection breaks, API 429/5xx, token expiration, partial writes, repeat notifications, time/DST changes, destination unavailability, and race conditions between delete/reconnect and local pause are required. Record test results separately from plans.

## Currency Rule

**Fact after push from 2026-10-14:** the baseline UI already includes `DesignSystem.swift`, `DashboardLayout.swift`, and a composable `LazyVGrid` with per-window layout, density, grid scale, and Undo/Redo; the scheduler has `schedule_dispatch.rs` and dispatch FFI. Re-verify before development. #26 is now created; it is **not** a task from a blank sheet. [Code slice](development/current-state-audit.md).

The historical plan G0–G6 from 2026-10-11 is replaced by the executable queue D-018 above. Reliability contracts (bound attempts, UNKNOWN, read-only reconciliation, consent, no-overwrite) are preserved but do not impose a cloud order. Verify statuses and SHAs via the [audit](development/current-state-audit.md), GitHub `main`, and the local working tree before any Codex package.
