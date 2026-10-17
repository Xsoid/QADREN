# Roadmap and Acceptance Criteria

Status: current product sequence. This document is not a work log.

## Phase 0 — Documentation Normalization

- README and AGENTS identify file synchronization as the primary product goal.
- BackupSet is future-only.
- The metadata-only Yandex adapter is not a completed transfer feature.
- Do not change working code for terminology only without a migration plan.

## Current Product Sequence — 2026-10-17

**Current sequence: native UI baseline → verified Local Filesystem file copy → local Scenario synchronization → autonomous local product.** Yandex Disk and iCloud are post-release work. They are not Release 1 gates. See [D-018](decisions/D-018-local-first-priorities.md).

| Priority | Product result and gate |
|---|---|
| **P0 / COMPLETE** | The SwiftUI design system, widget canvas, and monitoring baseline exist. Do not reopen general UI-baseline work. |
| **P1 / CRITICAL PATH** | The manual Local-to-Local no-overwrite copy path exists through UI → Rust JobStore → Local Filesystem → verification → SQLite → UI. Record a real File Provider materialization run. Preserve scoped consent and `UNKNOWN`. Do not build a second engine. |
| **P1 / LOCAL PRODUCT** | A user-confirmed Scenario `Create` item now uses the verified copy Job and records a revision-bound baseline. An immediate UNKNOWN copy result has an explicit read-only check action. Add one-way updates, two-way execution, conflict handling, and persisted UNKNOWN-copy inventory. |
| **P2 / AUTONOMY** | Run real local Scenarios on schedule in one independent Worker after UI exit. Test crash and restart recovery. Produce a verified macOS release. |
| **POST-RELEASE / PARKED** | Yandex Disk and iCloud require a separate owner decision before reactivation. Preserve existing code, credentials, and Local File Provider guards. Do not add cloud adapter, API, entitlement, or live end-to-end work in Release 1. |

## Requirement Ownership

Product decisions and safety policies define required behavior. D-018 defines the product order. This roadmap allocates the current product sequence. Architecture and design documents define implementation contracts. Code and recorded verification establish readiness.

An external work tracker can record discussion and delivery history. It does not define requirements, architecture, readiness, or the active product sequence. The project remains usable when that tracker is unavailable.

Do not create a parallel specification. When the sequence changes, update the canonical decision and the affected contract documents. Do not remove a safety guarantee or a confirmed function only to reorder work.

**P0 is complete.** Reuse the existing app-owned mock in `job_execution.rs`. The next criterion is a real, secure, Job-linked Local Filesystem operation, not another mock. Do not mix background-service readiness with interface-component readiness.

**Local release gates:** successful build and tests on a fixed SHA; UI without mock metrics; Local Filesystem controller and Local-to-Local Scenario with recovery and consent; working independent service and installation. Cloud adapters are separate post-release scope. Policies in `docs/policies/` remain mandatory.

## Local Release Acceptance Scenarios P0–P2

- Preserve independent `connection_id` values for local connections. Cloud credential independence remains an invariant for later remote work and does not block the local release.
- One-way and two-way synchronization detect changes from the last agreed state and signal ambiguous conflicts. An identical name and size do not prove byte equality.
- Events received during a Job create one subsequent Job. Do not overwrite the old Job state or revision.
- After restart or power failure, do not start a dangerous operation twice before checking its result.
- A manual task receives initial priority. An automatic task does not starve indefinitely. A local pause persists after reboot.
- A preview does not transfer content to the primary MacBook. Content download requires explicit individual consent.
- Mass deletion and unresolved conflict do not silently destroy data.
- Multiselect and nested-file operations work in the Job tree without double counting. Show progress in bytes and files.
- Closing the UI does not interrupt the service. Reopening restores windows, panels, navigation, and current states.
- Do not display unknown quota, ETA, or Worker state as verified success.
- Yandex Disk and iCloud are not conditions for the local release. Do not present an unconfirmed cloud operation as available.

## UI/UX Contract Release 1

The [design system](design/README.md), [widget layout](design/widget-system.md), and [UI checks](design/implementation.md) are mandatory for visual-layer changes. Monitoring is a customizable **widget canvas without a permanent Sidebar**. A separate settings panel can have navigation. The brand logo is `assets/qadren-logo.png`. A design prototype demonstrates interface behavior only. It does not confirm Core or Worker readiness and does not permit unsafe operations.

## Release 2 — Remote Workers

- Develop and separately approve transport, mutual trust, command delivery, acknowledgments, revisions, logs, and idempotency.
- The Worker handles local permissions, authorization, and operational tasks. The Controller changes Scenarios.
- The Controller assigns a Scenario manually to a compatible Worker. Remote unavailability does not mean stoppage.
- Reassignment requires confirmation that the previous Worker was released. A forced disconnect leaves `UNKNOWN` Jobs blocked until reconciliation.
- During re-pairing, match state. Keep the Worker UI local.
- Verify execution with the Controller disconnected. Load the later report without repeating effects.

## Deferred

**iCloud Documents, synchronization, and backup with iCloud:** future capability, not Release 1. Do not develop the container, adapter, UI, entitlement, native spike, or iCloud end-to-end flow. Resume only after a separate product decision that checks the access method, platform requirements, and cloud confirmation. See [D-017](decisions/D-017-defer-icloud.md).

BackupSet, backup versions, recovery of old content, inter-Scenario dependencies, custom Worker ACLs, proprietary search index, quota-change analytics, remote UI Controller, and Worker software update from Controller are also deferred.

## Negative Tests

Do not validate preservation only on a happy path. Test connection breaks, API `429` and `5xx`, token expiration, partial writes, repeated notifications, time and DST changes, destination unavailability, and races between delete, reconnect, and local pause. Record results in the validation record, not in this plan.

## Currency Rule

Re-verify the current `HEAD` before each patch. The native UI, layout, scheduler-dispatch foundation, File Manager copy path, Scenario metadata snapshot, user-confirmed Scenario `Create` path, and immediate UNKNOWN-copy check action exist in `main`. Updates, two-way execution, conflict handling, persisted UNKNOWN-copy inventory, autonomy, real File Provider evidence, and release evidence remain incomplete. See the [current-state audit](development/current-state-audit.md).

This roadmap replaces the earlier G0–G6 plan. It preserves reliability contracts: durable attempts, `UNKNOWN`, read-only reconciliation, consent, and no-overwrite. These contracts do not impose a cloud-first order. Verify the current worktree and recorded local evidence before starting work.
