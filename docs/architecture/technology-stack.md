# QADREN Technology Stack

Status: **accepted** 2026-10-08. This is a selection of technologies. It is not a statement that the implementation is complete. The current product specification is `docs/architecture/backup-model.md`.

## Mandatory Solutions

- **Rust** — general cross-platform core, Scenario/Job/FileSyncState logic, scheduling, tasks, Worker, and general cloud adapters.
- **Swift + SwiftUI** — native compact interface for the first platform **macOS**.
- **C ABI / FFI** — narrow, versioned boundary between Rust and Swift. Do not pass arbitrary internal Rust structures.
- **SQLite** — embedded local storage for registry, metadata, and task log; the schema and the specific Rust library are selected during implementation.
- **macOS Keychain** — secrets and OAuth tokens; platform secret stores for other OS in the future.
- **Official Yandex Disk REST API** — existing partially implemented network adapter; the controller is moved beyond Release 1 to post-release P3 and is not required for local-first P0–P2 (D-018).
- **Cargo and Xcode** — developer build tools. These are not user system requirements.

## Deferred

Swift/Foundation + Apple iCloud Documents — only a possible future adapter; not part of the mandatory Release 1 stack, current tasks, build, or release acceptance ([D-017](../decisions/D-017-defer-icloud.md)).

## Preliminary Libraries, Non-mandatory Dependencies

- Tokio — asynchronous background operations.
- reqwest — HTTP client; select TLS configuration/dependencies based on real need.
- serde — serialization of configurations and messages at module boundaries.
- rusqlite — candidate for SQLite.
- sha2 — candidate for SHA-256.

Include these libraries only when a specific use case occurs. Do not introduce two libraries that solve one task. Fix versions in the lockfile.

## User Result

The finished application is a **self-sufficient compiled .app package**. The user does not need to install Rust, Swift, Xcode, Python, Node.js, Docker, DBMS, or an external runtime. The application uses system libraries and services of the supported macOS version. On other platforms, use a corresponding native package with verified system requirements.

SwiftUI is available only on supported Apple OS: when the core is ported to Windows/Linux, the UI implementation may differ, but the task semantics, policies, and adapter contract must not change.

## Not Used (By Design)

Electron, Chromium bundle, Node.js/Python runtime in production, JVM, Docker, microservices, mandatory local HTTP server, separate DB server, heavy plugin host. Do not add a third-party framework for a few screens.

## Minimum Resources — Measurable Targets, Not Guarantees

- Installed macOS package: desired target up to **20 MB**.
- RAM at rest: desired target up to **40 MB**.
- CPU without tasks: close to zero, without active polling.
- No permanent local mirror on the main MacBook.
- No unrelated content prefetch or mirroring to the main MacBook.

Measure size on the **release, signed** build; verify RAM, energy, disk activity, and network bytes with tests. Do not promise these values in advance.

## Not Yet Selected

macOS/Rust/Swift/Xcode versions, deployment target, CPU architectures, packaging/signing/notarization path, update scheme, final crates structure, exact SQLite configuration, and API adapter. Fix these after the prototype and verification of Apple support.
