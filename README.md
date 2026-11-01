# QADREN

<img src="assets/qadren-logo.png" alt="QADREN logo" width="144">

QADREN is a file transfer and synchronization system for independent storage connections. Rust is the cross-platform core. SwiftUI is the macOS control interface.

## Status

QADREN is in active development. [GitHub Releases](https://github.com/Xsoid/QADREN/releases) contain pre-release builds for Apple silicon Macs with macOS 13 or later ([D-026](docs/decisions/D-026-github-releases.md)). These builds are not Developer ID signed and not notarized.

## Install

1. Download `QADREN-<version>-macos-arm64.zip` from the [latest release](https://github.com/Xsoid/QADREN/releases).
2. Open the `.zip` file. Move `QADREN.app` to the Applications folder.
3. Open QADREN. macOS blocks the first start.
4. Open System Settings > Privacy & Security and click **Open Anyway**.

You can also remove the quarantine attribute in Terminal:

```bash
xattr -dr com.apple.quarantine /Applications/QADREN.app
```

After an update, macOS can ask again for permissions and Keychain access.

## Release 1 scope

[D-018](docs/decisions/D-018-local-first-priorities.md) defines the Release 1 scope. Release 1 is local-first:

- **Storage**: "This Mac" (Local Filesystem) and SMB network drives. QADREN mounts an SMB share at `~/.qadren/mounts/<connection_id>` and uses the Local Filesystem executor for it ([D-021](docs/decisions/D-021-smb-network-drive-connector.md), [D-022](docs/decisions/D-022-smb-in-release-1-scenarios.md)).
- **Yandex Disk**: a one-way Scenario source to a Local Filesystem or SMB destination ([D-023](docs/decisions/D-023-yandex-disk-to-mac.md)). Yandex Disk as a destination is a later stage.
- **Scenarios and Jobs**: a `Scenario` defines synchronization rules. A `Job` is one execution or one manual file operation. One Rust JobStore holds all Jobs.
- **Schedules**: scheduled and manual runs use one durable Scenario Run. A Scenario has at most one active run. Schedules can run on selected days at a time or every N hours ([D-025](docs/decisions/D-025-autonomous-scheduled-scenarios.md)).
- **Worker**: a local background process (`qadren-worker`) executes Jobs. It starts at login, mounts SMB shares on demand, and restarts only between files.
- **Monitoring panel**: the macOS window shows Scenarios, connections, active runs, and system state as widgets ([design system](docs/design/README.md)).

iCloud is post-release ([D-017](docs/decisions/D-017-defer-icloud.md)). Remote Workers, content versioning, and Scenario dependencies are not in Release 1. See the [roadmap](docs/roadmap.md).

## Safety model

- A new file never overwrites an existing destination. A replacement uses the approved replacement protocol.
- A Scenario has a deletion mode: Paranoid (every deletion asks) or Simple synchronization (one approved rule, Trash first) ([D-019](docs/decisions/D-019-scenario-selection-and-deletion-modes.md)).
- A missing source root or an incomplete source snapshot never causes a deletion or a replacement.
- QADREN saves a durable attempt before each modifying effect. When proof of an effect is lost, QADREN reconciles without mutation. If the evidence is not sufficient, the outcome stays `UNKNOWN`.
- Remove and Disconnect change only local state. They do not delete provider objects.
- Secrets stay in the macOS Keychain. QADREN does not write secrets to SQLite, logs, Git, or HTTP URLs.

See [data safety](docs/policies/data-safety.md).

## Build from source

Prerequisites: macOS 13 or later, a Rust toolchain, and Xcode or Xcode Command Line Tools.

```bash
bash scripts/test-core.sh
bash scripts/build-macos.sh
open dist/QADREN.app
```

`scripts/build-macos.sh` signs the bundle with the first valid "Apple Development" identity, or with `QADREN_SIGN_IDENTITY`. Without an identity, it uses an ad-hoc signature. The bundle is a developer artifact. It is not release evidence. `./script/build_and_run.sh` rebuilds and starts the bundle.

Yandex Disk connections need a registered OAuth client ID. Copy `config/oauth.example.env` to `config/oauth.local.env` and set `YANDEX_CLIENT_ID`. Do not commit the local file. Do not put a client secret in it.

See [build and release](docs/development/build-and-release.md) and [testing and quality](docs/development/testing-and-quality.md).

## Repository map

- `core/`: Rust core, JobStore, scheduler, storage adapters, and the `qadren-worker` binary.
- `platform/macos/`: SwiftUI app, LaunchAgents, and native tests.
- `scripts/`: build and validation commands.
- `script/`: local run entry point.
- `assets/`: logo, app icon, brand files, and fonts.
- `config/`: example local build configuration.
- `docs/`: canonical project documentation.

## Documentation

- [Documentation index](docs/README.md)
- [Architecture](docs/architecture/overview.md)
- [Decisions](docs/decisions/README.md)
- [Data safety](docs/policies/data-safety.md)
- [Design system](docs/design/README.md)
- [Build and release](docs/development/build-and-release.md)
- [Testing and quality](docs/development/testing-and-quality.md)
- [Roadmap](docs/roadmap.md)
- [Agent instructions](AGENTS.md)
- [Third-party notices](THIRD_PARTY_NOTICES.md)

## Contributing

GitHub Issues contain active work, audits, test results for a specific SHA, and closure criteria. Git history contains change history.

The repository and the issue tracker are public. Do not put real file or folder names, paths, counts, sizes, quota, account data, or screenshots of user content in issues, commits, or files.

## License

QADREN is available under the [MIT License](LICENSE). Third-party fonts, design sources, and agent skills keep their own licenses. See [third-party notices](THIRD_PARTY_NOTICES.md).
