# Architecture Decision Record

Baseline date: **2026-10-08**. Records list agreements. They do not prove the feasibility of all integrations.

| ID | Status | Decision |
|---|---|---|
| D-001 | Revised v0.2 | QADREN is a compact system for backup of independent storages. It is not a permanent mirror of the main MacBook. |
| D-002 | Revised v0.2 | Rules at folder/BackupSet level; one-way and two-way directions. |
| D-003 | Revised D-017/D-018 | Initial integrations included iCloud and Yandex. Release 1 is Local Filesystem. Yandex is post-release. |
| D-004 | Accepted | Cross-platform core. OS limitations are a property of the adapter. |
| D-005 | Accepted | One application on several nodes with logical roles: Controller/Worker. |
| D-006 | Accepted | On the main MacBook, QADREN does not start a download of content without prior agreement. |
| D-007 | Accepted | Do not interfere with downloads of macOS, iCloud, and other applications. |
| D-008 | Accepted | Only the assigned Worker performs a delegated task. The Controller receives the report. |
| D-009 | Revised v0.2 | Do not apply automatic deletions/conflicts to the last protected version. |
| D-010 | Deferred | Communication protocol for instances, node discovery, and re-assignment mechanism. |
| D-011 | Replaced by D-012 | A stack was not selected initially. A specific decision was made later. |
| D-012 | Partially revised D-017 | Rust Core, SwiftUI macOS, SQLite, and narrow FFI remain. iCloud adapter is future-only. |
| D-013 | Accepted (2026-10-08) | Self-sufficient native installation packages without external runtimes for the user. OS system services are permissible. |
| D-014 | Accepted (2026-10-08) | Priority is the minimum number of dependencies, components, and background processes. Verify actual compactness by measurement. |
| D-015 | Accepted (2026-10-08) | Technical rules and agent context are stored in QADREN. The AGENTS.md instruction is mandatory when code is changed. |
| D-016 | Accepted (2026-10-08) | Unlimited number of independent StorageConnection per adapter and account. Do not confuse source/destination roles with Controller/Worker. See [ADR](D-016-multiple-connections.md). |
| D-018 | Accepted (2026-10-14) | [Local-first Release 1: P0 UI, P1 Local FS controller, P2 autonomous product; Yandex post-release P3, iCloud postponed](D-018-local-first-priorities.md) |
| D-017 | Accepted, partially revised D-018 | [iCloud future-only; historical cloud scope is replaced by local-first Release 1](D-017-defer-icloud.md) |

## Revision 2026-10-09 — specification v0.2

The following is agreed: QADREN focuses on synchronization. BackupSet/backup file versions are deferred. Scenario/Job, service history of the last confirmed states without content storage, enhanced protection of dangerous operations, a private iCloud Documents container, and an independent Local Filesystem adapter are current. Release 1 is a full local Controller+Worker. Release 2 is remote Workers. The historical meaning of D-001/D-002/D-009 is preserved, but the old wording **is not an active MVP requirement**. See [specification](../architecture/backup-model.md) and [BackupSet future](../architecture/backup-model.md).

## Revision 2026-10-12 — iCloud deferred

[D-017](D-017-defer-icloud.md) overrides the iCloud part of D-003 and D-012: a private iCloud Documents container, iCloud backup/sync, and native spike are not in Release 1. Local Filesystem is independent and active. Protection against accidental download of iCloud/File Provider placeholders through Local FS remains.

## Change rule

When a decision is revised, keep the ID. Describe the new decision, the reason, and the affected documents. Put new details in `docs/decisions/` as separate ADRs if necessary. A proposal is not approved without an explicit decision.
