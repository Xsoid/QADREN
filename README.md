# QADREN

<img src="assets/qadren-logo.png" alt="QADREN logo" width="144">

QADREN is a cross-platform system for file transfer and synchronization between independent storage connections.

## Release 1

Release 1 is local-first. It uses Local Filesystem connections, one Rust JobStore, and a SwiftUI macOS control interface. A `Scenario` defines synchronization rules. A `Job` is one execution or one manual file operation.

Yandex Disk is a one-way Scenario source to a Mac folder ([D-023](docs/decisions/D-023-yandex-disk-to-mac.md)). iCloud is post-release ([D-017](docs/decisions/D-017-defer-icloud.md)).

QADREN reads content needed by an enabled or requested operation without a separate prompt. A destructive replacement or deletion requires the policy-defined confirmation. After uncertain effects, QADREN performs non-mutating reconciliation and retains `UNKNOWN` when proof is insufficient.

## Quick start on macOS

Developer prerequisites: a supported Rust toolchain and Xcode Command Line Tools or Xcode.

```bash
bash scripts/test-core.sh
bash scripts/build-macos.sh
open dist/QADREN.app
```

The generated bundle is an unsigned developer artifact. It is not release evidence.

## Repository map

- `core/`: Rust core and JobStore.
- `platform/macos/`: SwiftUI macOS interface and platform integration.
- `scripts/`: build and validation commands.
- `docs/`: canonical project documentation.

## Documentation

- [Documentation index](docs/README.md)
- [Architecture](docs/architecture/overview.md)
- [Decisions](docs/decisions/README.md)
- [Safety policies](docs/policies/data-safety.md)
- [Design system](docs/design/README.md)
- [Development guide](docs/development/build-and-release.md)
- [Testing and quality](docs/development/testing-and-quality.md)
- [Roadmap](docs/roadmap.md)
- [Agent instructions](AGENTS.md)

GitHub Issues contain active work, audits, test results for a specific SHA, and closure criteria. Git history contains change history.
