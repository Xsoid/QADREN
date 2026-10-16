# D-018 — Priority of local foundation and interfaces

**Date:** 2026-10-14. **Status:** accepted. **Overrides:** Release 1 queue in D-017, roadmap, and historical issues. Safety policies, D-016, and technical constraints remain.

## Development Sequence

- **P0 — foundation and UX/UI.** First, remove identified build errors and discrepancies between `main` and local work; bring Rust Core, SQLite, narrow FFI, and SwiftUI to a stable baseline. Develop the native design system, components, monitoring widget canvas without a constant sidebar, settings window and MenuBarExtra, and empty queue/error/UNKNOWN states. Use `assets/qadren-logo.png` and `docs/design/`. Do not create a second Job Engine or fictitious indicators.
- **P1 — local functional core.** Implement and verify UI → Rust JobStore → Local Filesystem → verification → SQLite → UI. Then implement local scenarios, crash recovery, consent policy, and a two-pane file manager. Start with metadata/dry-run and no-overwrite; perform dangerous operations only through safety-gates.
- **P2 — autonomous local product.** Scheduler, independent Worker/service, session saving, verifiable Local↔Local operations, and distribution package. The absence of a cloud API does not block the local version.
- **P3 / POST-RELEASE — Yandex Disk controller.** By direct decision of the owner, this is **removed from Release 1**. The owner reported an emergency state of Yandex and partial unavailability of Disk/API. **Do not perform new tasks, OAuth/API/transfer, or live E2E with Yandex Disk until a new separate decision is made.** Preserve existing OAuth/metadata/upload source code, connections, tests, and data; independent offline/mock regression checks without network requests are permitted. #16 is not a stage or gate of Release 1.
- **P3 / POST-RELEASE, PARKED — iCloud without date.** According to the owner, the desired path requires an unavailable paid Apple Developer account. The connector, entitlement, UI, and integration research are not developed. #17 remains closed. The protection of Local FS against hidden File Provider materialization is not canceled.

## Status clarification — 2026-10-16

P0 interface baseline [#26](https://github.com/Xsoid/QADREN/issues/26) has been closed as completed; do not treat P0 as an active development stage. Both [Yandex #16](https://github.com/Xsoid/QADREN/issues/16) and [iCloud #17](https://github.com/Xsoid/QADREN/issues/17) are closed as **not planned for Release 1**, solely to keep post-release work out of the active queue. They are future candidates without an automatic deadline or implementation commitment. Reopen the existing relevant Issue only after a separate owner decision. For local execution use the active [Epic #12](https://github.com/Xsoid/QADREN/issues/12) gates: safe native read → real no-overwrite copy → Scenario sync → autonomous Worker → release evidence.

## Management

Work only in `main`; no new branches and no mandatory PRs. Verify actual code and SHA before each task; do not count local unpushed commits from issue comments as part of origin/main. Set `VERIFIED` only after checks on a specific SHA; distinguish `PARTIAL`, `BLOCKED`, `PARKED`. Develop UI using existing read-only APIs and separately isolated preview fixtures without waiting for the cloud. The prohibition of unauthorized downloading to the main MacBook, durable attempts, no-overwrite, UNKNOWN/reconciliation, scoped consent, and the absence of proprietary versioning remain unchanged.

Release 1 consists only of P0–P2: native UI and a verifiable **Local Filesystem** controller (JobStore, Policy, recovery, Local↔Local), then an autonomous local service and distribution package. Cloud integration is a separate post-release scope and is not a criterion for the local release.

See [roadmap](../roadmap.md) and [audit](../development/current-state-audit.md).
