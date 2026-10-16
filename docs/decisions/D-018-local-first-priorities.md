# D-018 — Local-First Release 1 Priorities

**Date:** 2026-10-14. **Status:** accepted. **Overrides:** earlier Release 1 sequencing in D-017 and historical plans. Safety policies, D-016, and technical constraints remain in force.

## Decision

Release 1 is a local-first product. Build in this order:

1. Keep the existing native SwiftUI baseline.
2. Prove a safe macOS resident-file read boundary.
3. Deliver one verified Local Filesystem to Local Filesystem no-overwrite copy through the Rust JobStore.
4. Connect local Scenario synchronization to that verified copy path.
5. Deliver independent local scheduling, service lifecycle, and macOS release evidence.

Yandex Disk and iCloud are post-release and parked. Do not add cloud API, OAuth, transfer, entitlement, or live end-to-end work without a separate owner decision. Preserve existing source code, local connection records, credentials, tests, and data. Offline regression checks that make no network request are permitted.

## Rationale

The current local product needs a verified end-to-end operation before remote-provider scope. The owner reported Yandex Disk emergency conditions and partial Disk/API availability. iCloud capability needs a separate access and platform decision. Neither provider is a Release 1 dependency.

## Constraints

Work in `main`. Verify the current code and SHA before each change. Use `VERIFIED` only for behavior checked on a specific SHA. Distinguish `PARTIAL`, `BLOCKED`, and `PARKED` in an evidence record.

Preserve the prohibition on unauthorized download to the primary MacBook, durable attempts, no-overwrite, `UNKNOWN` with read-only reconciliation, scoped consent, and no proprietary content versioning in Release 1.

## Consequences

- The current local product sequence is in [the roadmap](../roadmap.md).
- The current implementation and evidence boundaries are in [the audit](../development/current-state-audit.md) and [validation record](../development/release-1-validation.md).
- Yandex Disk and iCloud remain future candidates without an automatic deadline or implementation commitment.
- A later owner decision must update this decision and the affected contracts before cloud work resumes.
