# Product Roadmap

This document defines approved product stages. It is not a work log or an acceptance report.

## Release 1: local-first product

1. Keep the native SwiftUI control interface and the Rust/SQLite/FFI foundation.
2. Deliver safe Local Filesystem to Local Filesystem operations through the single Rust JobStore.
3. Connect Local Filesystem Scenarios to those operations.
4. Deliver local scheduling, independent Worker lifecycle, and macOS distribution evidence.

Release 1 uses Local Filesystem only. New files use no-overwrite. Replacement follows the approved replacement protocol. A failed or uncertain effect is reconciled without mutation and remains `UNKNOWN` if proof is insufficient.

## Post-release

Yandex Disk and iCloud require a separate owner decision before new feature work, OAuth, transfer, entitlement, or live integration testing. Remote Workers require a separate decision for transport, trust, command delivery, acknowledgments, revisions, and idempotency.

Content version archives, runtime BackupSet, inter-Scenario dependencies, remote Worker protocol, proprietary search indexes, and Controller-driven Worker updates are deferred.

## Ownership

[D-018](decisions/D-018-local-first-priorities.md) owns Release 1 scope. Architecture documents own component contracts. Policies own safety rules. The design system owns UI contracts. GitHub Issues own active execution, audit results, and acceptance evidence.
