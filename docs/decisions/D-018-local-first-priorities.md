# D-018 — Local-First Release 1 Priorities

**Date:** 2026-10-14. **Status:** accepted. **Overrides:** earlier Release 1 sequencing in D-017 and historical plans. Safety policies, D-016, and technical constraints remain in force.

## Decision

Release 1 is a local-first product. Build in this order:

1. Keep the existing native SwiftUI baseline.
2. Establish a consent-first macOS file-read boundary. Warn that a File Provider source may be materialized; do not demand proof that macOS can never hydrate an approved read.
3. Deliver one verified Local Filesystem to Local Filesystem no-overwrite copy through the Rust JobStore.
4. Connect local Scenario synchronization to that verified copy path.
5. Deliver independent local scheduling, service lifecycle, and macOS release evidence.

Yandex Disk and iCloud are post-release and parked. Do not add cloud API, OAuth, transfer, entitlement, or live end-to-end work without a separate owner decision. Preserve existing source code, local connection records, credentials, tests, and data. Offline regression checks that make no network request are permitted.

## Rationale

The current local product needs a verified end-to-end operation before remote-provider scope. The owner reported Yandex Disk emergency conditions and partial Disk/API availability. iCloud capability needs a separate access and platform decision. Neither provider is a Release 1 dependency.

## Constraints

Work in `main`. Verify the current code and SHA before each change. Use `VERIFIED` only for behavior checked on a specific SHA. Distinguish `PARTIAL`, `BLOCKED`, and `PARKED` in an evidence record.

Preserve the prohibition on unauthorized download to the primary MacBook, durable attempts, `UNKNOWN` with read-only reconciliation, scoped consent, and no proprietary content versioning in Release 1. New Local-to-Local copies remain no-overwrite. The owner approved a controlled replacement path for an existing Local destination on 2026-10-17. That path must use a durable attempt, explicit operation scope, destination identity checks where available, and `UNKNOWN` after lost effect proof. It must not add a content archive or a File Provider extension. A Local-to-Local file copy may materialize one File Provider source item only after an explicit one-time confirmation. Bind the durable `read_content` decision to the source `connection_id` and relative path.

## Owner Clarification — 2026-10-18: Risk Decisions

Release 1 follows **informed user control**, not absolute elimination of all possible risk. A known or explainable residual risk is an explicit user decision. Show the exact affected content and action, possible irreversible outcome, uncertainty, and available recovery before confirming. Permit the operation when the user explicitly accepts the bounded scope and runtime guards can enforce it.

Examples: potential File Provider hydration, controlled replacement, and proposed deletions may proceed only with their required specific approval. No autonomous destructive choice may be inferred from the absence of the user. The main MacBook download rule still requires per-operation explicit permission.

Do **not** turn missing evidence of a universally non-hydrating macOS file open into a perpetual release gate. Report it as a possible hydration consequence and get scoped approval. Do **not** relax directory/path isolation, permission gates, durable attempts, no-overwrite new copies, revision and destination checks, single ownership, UNKNOWN read-only recovery, or truthfulness of verification. These are technical integrity rules, not a risk that a generic confirmation can bypass.

Limit verification to supported behavior, credible negative scenarios, and honest documented residual risks. Do not expand Release 1 to solve every conceivable platform edge case. Unenforceable action boundaries remain BLOCKED, because the application cannot describe or contain the effect the user would be approving.

## Consequences

- The current local product sequence is in [the roadmap](../roadmap.md).
- The current implementation and evidence boundaries are in [the audit](../development/current-state-audit.md) and [validation record](../development/release-1-validation.md).
- Yandex Disk and iCloud remain future candidates without an automatic deadline or implementation commitment.
- A later owner decision must update this decision and the affected contracts before cloud work resumes.
