# D-018 — Local-First Release 1 Priorities

**Date:** 2026-10-14. **Status:** accepted. **Overrides:** earlier Release 1 sequencing in D-017 and historical plans. Safety policies, D-016, and technical constraints remain in force.

## Decision

Release 1 is a local-first product. Build in this order:

1. Keep the existing native SwiftUI baseline.
2. Establish an ordinary macOS content-read path, including transparent File Provider materialization, without a separate read/transfer-hydration prompt. Preserve requested operation scope and durable Job effects.
3. Deliver one verified Local Filesystem to Local Filesystem no-overwrite copy through the Rust JobStore.
4. Connect local Scenario synchronization to that verified copy path.
5. Deliver independent local scheduling, service lifecycle, and macOS release evidence.

Yandex Disk and iCloud are post-release and parked. Do not add cloud API, OAuth, transfer, entitlement, or live end-to-end work without a separate owner decision. Preserve existing source code, local connection records, credentials, tests, and data. Offline regression checks that make no network request are permitted.

## Rationale

The current local product needs a verified end-to-end operation before remote-provider scope. The owner reported Yandex Disk emergency conditions and partial Disk/API availability. iCloud capability needs a separate access and platform decision. Neither provider is a Release 1 dependency.

## Constraints

Work in `main`. Verify the current code and SHA before each change. Use `VERIFIED` only for behavior checked on a specific SHA. Distinguish `PARTIAL`, `BLOCKED`, and `PARKED` in evidence.

**This is the latest product decision and overrides the earlier one-time `read_content` consent policy:**

1. **Read automatically:** When an enabled/configured operation needs file content, QADREN reads it without another user question. macOS may transparently download a File Provider item onto the main MacBook. This is not classified as a destructive or risky user action; no dedicated hydration warning or per-file `read_content` decision is needed. Do not prefetch unrelated content or turn this into general cloud mirroring.
2. **Ask before destructive effects:** Replacement of an existing file is a risk: show the target and ask unless a preapproved, exact, durable Scenario replacement policy covers the effect. Mass deletion (for example 150 files) requires an explicit, clear confirmation of the current proposed set. No silent destructive choices or assumed consent.
3. **Recheck automatically after failure:** Perform available read-only reconciliation of `UNKNOWN` results on restart/reconnect without prompting. Do not blindly repeat an uncertain write/delete. Retain `UNKNOWN` and report it if evidence remains insufficient.
4. **Disclose broad effects:** If the executor cannot restrict an action to selected files, disclose the actual known impact and technical limitation. Never imply selection precision it cannot enforce. Obtain approval for a different bounded destructive scope; if impact cannot be bounded or explained truthfully, do not run it as a selected-file operation.
5. **Keep technical invariants:** Retain durable attempts, supported permissions, exact roots/connections/revisions, truthful evidence, no-overwrite on new files, and single Worker ownership. The user takes responsibility for clearly described destructive consequences; a click cannot excuse execution outside the disclosed scope.

Controlled replacement remains an approved capability path (owner decision 2026-10-17), with durable attempt, exact destination checks where available, and `UNKNOWN` on lost effect proof. The local product has no content-version archive. Yandex and iCloud are still post-release.

Do not block release simply because it is impossible to prove a file read will never cause OS-managed materialization; that is an allowed ordinary read. File Provider testing can establish compatibility, but no download consent dialogue is required.

## Consequences

- The local product sequence is in [the roadmap](../roadmap.md).
- GitHub Issues contain implementation and evidence status for a specific SHA.
- Yandex Disk and iCloud remain future candidates without an automatic deadline or implementation commitment.
- A later owner decision must update this decision and the affected contracts before cloud work resumes.
