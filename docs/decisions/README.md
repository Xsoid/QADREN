# Architecture Decision Record

Baseline date: **2026-10-08**. Records list agreements. They do not prove the feasibility of all integrations.

| ID | Status | Decision |
|---|---|---|
| D-001 | Revised v0.2 | QADREN is a compact system for backup of independent storages. It is not a permanent mirror of the main MacBook. |
| D-002 | Revised v0.2 | Rules at folder/BackupSet level; one-way and two-way directions. |
| D-003 | Revised D-017/D-018 | Initial integrations included iCloud and Yandex. Release 1 is Local Filesystem. Yandex is post-release. |
| D-004 | Accepted | Cross-platform core. OS limitations are a property of the adapter. |
| D-005 | Accepted | One application on several nodes with logical roles: Controller/Worker. |
| D-006 | Revised D-018 | A needed content read needs no separate confirmation. Destructive effects require policy-defined consent. |
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
| D-020 | Accepted (2026-10-24) | [One automatic "This Mac" connection per Mac; connector list in "Добавить подключение"; revises D-016 for `local_filesystem` only](D-020-single-this-mac-connection.md) |
| D-021 | Accepted (2026-10-24) | ["Сетевой диск (SMB)" connector: any number of connections, address in the Registry, login and password in the Keychain, login check through NetFS; no Scenario use yet](D-021-smb-network-drive-connector.md) |
| D-022 | Accepted (2026-10-24), rule 7 revised D-025 | [SMB network drive in Release 1 Scenarios: QADREN mount per connection, Local Filesystem executor, Core mount gate; revises D-018](D-022-smb-in-release-1-scenarios.md) |
| D-023 | Accepted (2026-10-24) | [Yandex Disk to Mac: a one-way Scenario with a Yandex Disk source and a Local Filesystem destination; two-way is stage 2; revises D-018 and D-022](D-023-yandex-disk-to-mac.md) |
| D-024 | Accepted (2026-10-25) | [Large transfers: Worker-owned Transfer Runs on a background thread, resumable snapshots, stored paged plans, retry classes, Worker token access (in memory now, shared Keychain group with a Team ID later), MD5 reconciliation](D-024-large-transfers.md) |
| D-019 | Accepted (2026-10-23), threshold revised D-025 | [Inline Scenario folder browser, selected objects, and Paranoid / Simple synchronization deletion modes; the 20-deletion threshold is removed by D-025](D-019-scenario-selection-and-deletion-modes.md) |
| D-025 | Accepted (2026-10-28) | [Autonomous scheduled Scenarios: one run path, one active run for each Scenario, automatic Update and Delete, one-way source wins, Worker SMB mount, Yandex schedules, launch at login, daily and every-N-hours schedules, parallel runs with a write-root lock; revises D-019, D-022, D-023, D-024](D-025-autonomous-scheduled-scenarios.md) |
| D-026 | Accepted (2026-11-01) | [GitHub Releases: a `vX.Y.Z` tag starts a macOS workflow that builds an ad-hoc signed Apple silicon pre-release; Semantic Versioning from `v0.2.0`; Yandex client ID from a repository variable](D-026-github-releases.md) |
| D-027 | Proposed (2026-11-01) | [ISPmanager 6 read-only backup source: `ispmanager` connector, POST transport, read-only allowlist, five capability states, Backup/export Scenario mode with a new run folder and manifest per run, typed selection](D-027-ispmanager-backup-source.md) |

## Revision 2026-10-09 — specification v0.2

QADREN focuses on synchronization. BackupSet and content versions are deferred. Scenario and Job, service history without content storage, protection for destructive operations, and an independent Local Filesystem adapter are current contracts. Release 1 is local-first. Remote Workers are post-release. Historical wording in D-001, D-002, and D-009 is not an active requirement. See [the synchronization model](../architecture/backup-model.md).

## Revision 2026-10-12 — iCloud deferred

[D-017](D-017-defer-icloud.md) overrides the iCloud part of D-003 and D-012: an iCloud Documents container, iCloud backup/sync, and native spike are not in Release 1. Local Filesystem is independent and active. A required Local Filesystem read can materialize a File Provider item without a separate prompt.

## Change rule

When a decision is revised, keep the ID. Describe the new decision, the reason, and the affected documents. Put new details in `docs/decisions/` as separate ADRs if necessary. A proposal is not approved without an explicit decision.
