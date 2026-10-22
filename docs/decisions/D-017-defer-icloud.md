# D-017 — iCloud postponed beyond active Release 1

**Date:** 2026-10-12. **Status:** accepted. **Scope:** product scope, architecture, UI, and release gate.

## Decision

In the first QADREN release, **do not develop backup, synchronization, file transfer with iCloud, iCloud Documents adapter, own container, entitlement/provisioning, native feasibility spike, iCloud connection UI, or iCloud integration/E2E**. iCloud remains a potential future integration, not a current task and not a dependency. Do not start iCloud implementation without an explicit owner decision.

Release 1 uses Local Filesystem. Yandex Disk is also post-release under [D-018](D-018-local-first-priorities.md).

## Basis and return conditions

The owner decided to postpone iCloud because of limitations of the free integration method available to the owner. This is an accepted **product decision**, and not an independent verification of all APIs, Apple prices, and Apple Developer entitlements. Return to iCloud only after a new explicit decision, verification of a technically and economically available model, and evidence of cloud upload semantics; do not perform even a spike before this.

## Does not cancel

- System iCloud/macOS works independently; QADREN does not interfere.
- A required Local Filesystem read may materialize a File Provider item without a separate prompt. This does not mean the implementation of iCloud.
- Durable attempts, UNKNOWN/recovery, no-overwrite/conflict/delete policies, individual consent, independent connections, and metadata sync state remain active.

## Revision of decisions

Replaces the iCloud part of D-003 and D-012. Other decisions about architecture and local file systems remain.
