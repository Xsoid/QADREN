# Product Roadmap

This document defines approved product stages. It is not a work log or an acceptance report.

## Release 1: local-first product

1. Keep the native SwiftUI control interface and the Rust/SQLite/FFI foundation.
2. Deliver safe Local Filesystem to Local Filesystem operations through the single Rust JobStore.
3. Connect Local Filesystem Scenarios to those operations.
4. Deliver local scheduling, independent Worker lifecycle, and macOS distribution evidence.

Release 1 uses Local Filesystem and SMB network drives through a QADREN mount (D-022). New files use no-overwrite. Replacement follows the approved replacement protocol. A failed or uncertain effect is reconciled without mutation and remains `UNKNOWN` if proof is insufficient.

## Release 1 improvements

1. Yandex Disk to Mac: a one-way Scenario with a Yandex Disk source (D-023, stage 1).
2. Large transfers (D-024): Worker-owned Transfer Runs with retries, then stored snapshots, then stored paged plans, then MD5 reconciliation.
3. Autonomous scheduled Scenarios (D-025): one run path, one active run for each Scenario, automatic Update and Delete, Yandex Disk schedules, Worker SMB mount, launch at login, daily and every-N-hours schedules, parallel runs. This includes deletion on the Mac side for a Yandex Disk source.
4. Mac to Yandex Disk and two-way Yandex Disk Scenarios (D-023, stage 2).
5. Monitoring control panel (2026-10-31): cybercn style and the cyber plate mark, a widget catalog (System, Scenario, Scenarios, Connections), Worker restart between files, and `UNKNOWN` reconciliation after an SMB remount ([design](design/README.md), [D-022](decisions/D-022-smb-in-release-1-scenarios.md)).

## Post-release

iCloud requires a separate owner decision before new feature work, OAuth, transfer, entitlement, or live integration testing. Remote Workers require a separate decision for transport, trust, command delivery, acknowledgments, revisions, and idempotency.

Content version archives, runtime BackupSet, inter-Scenario dependencies, remote Worker protocol, proprietary search indexes, and Controller-driven Worker updates are deferred.

## Ownership

[D-018](decisions/D-018-local-first-priorities.md) owns Release 1 scope. Architecture documents own component contracts. Policies own safety rules. The design system owns UI contracts. GitHub Issues own active execution, audit results, and acceptance evidence.
