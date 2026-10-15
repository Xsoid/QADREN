# D-017 — iCloud postponed beyond active Release 1

**Date:** 2026-10-12. **Status:** accepted. **Scope:** product scope, Codex issues, architecture, UI, release gate.

## Decision

In the first QADREN release, **do not develop backup, synchronization, file transfer with iCloud, iCloud Documents adapter, own container, entitlement/provisioning, native feasibility spike, iCloud connection UI and iCloud integration/E2E**. iCloud remains a potential future integration, not a current task and not a dependency. Existing #17 is closed as not planned. Codex does not create new iCloud implementation tasks without an explicit decision by the owner.

**Historical order from October 12, REVISED by D-018 from October 14:** the Yandex↔Local requirement stated below is no longer a gate for the local release; the priority is now foundation/UI → Local↔Local → autonomous local version, and only then Yandex. [D-018](D-018-local-first-priorities.md).

Active endpoints in the historical scope: **Yandex Disk and Local Filesystem**. Former priority — Local→Yandex upload, Yandex→Local download (with individual consent when downloading to the main MacBook), Local↔Local; other pairs, including Yandex↔Yandex, only after capabilities verification. Then one-way and two-way synchronization and safe conflicts. Do not consider a simple copy as a guaranteed backup version: there is no own BackupSet and no content versioning.

## Basis and return conditions

The owner decided to postpone iCloud because of limitations of the free integration method available to the owner. This is an accepted **product decision**, and not an independent verification of all APIs, Apple prices, and Apple Developer entitlements. Return to iCloud only after a new explicit decision, verification of a technically and economically available model, and evidence of cloud upload semantics; do not perform even a spike before this.

## Does not cancel

- System iCloud/macOS works independently; QADREN does not interfere.
- Guards in Local Filesystem against implicit materialization of File Provider/iCloud placeholders and downloading of content to the main MacBook **remain** as safety requirements. This does not mean the implementation of iCloud.
- Durable attempts, UNKNOWN/recovery, no-overwrite/conflict/delete policies, individual consent, independent connections, and metadata sync state remain active.

## Revision of decisions

Replaces the iCloud part of D-003 and D-012; other decisions about architecture and local file systems remain. Links: `README.md`, `AGENTS.md`, `docs/roadmap.md`, `docs/architecture/*`, `docs/development/*`, `docs/design/settings.md`, Epic #12.
