# D-026 — GitHub Releases

**Date:** 2026-11-01. **Status:** accepted. The owner gave the decisions on 2026-11-01. **Revises:** the open distribution-channel question in [build and release](../development/build-and-release.md).

## Context

- The repository is public and has the MIT License.
- `scripts/build-macos.sh` builds `dist/QADREN.app`. It signs with an "Apple Development" identity when one is available. Without one, it uses an ad-hoc signature.
- QADREN has no Developer ID certificate and no notarization.
- The `v1.0.0-internal` tag marks an internal ad-hoc build. It is not a public release.
- `Info.plist` and `core/Cargo.toml` have version `0.1.0`.

## Decision

1. **Channel.** QADREN publishes macOS builds as GitHub Releases of `Xsoid/QADREN`.
2. **Trigger.** A pushed tag `vX.Y.Z` starts the `Release` workflow (`.github/workflows/release.yml`). The workflow builds on a GitHub-hosted macOS runner from the tagged commit. A person does not upload a local build.
3. **Version.** QADREN uses Semantic Versioning. The first public release is `v0.2.0`. The tag, `CFBundleShortVersionString` in `platform/macos/Info.plist`, and `version` in `core/Cargo.toml` must be equal. The workflow stops if they are not equal. `CFBundleVersion` stays a unique build number for each build.
4. **Signing.** A release build has an ad-hoc signature. It is not Developer ID signed and not notarized. Each release is marked as a pre-release. The release notes tell the user how to open the app the first time.
5. **Gate.** The workflow runs `scripts/test-core.sh` before the build. A failed check stops the release.
6. **Yandex Disk.** The build includes the public Yandex OAuth client ID from the repository variable `YANDEX_CLIENT_ID`. The workflow stops if the variable is empty. A client secret is never part of a build, a variable, or a log.
7. **Asset.** Each release has `QADREN-<version>-macos-arm64.zip` (made with `ditto`) and its SHA-256 file. Release 1 builds only for Apple silicon.
8. **Description.** The release description is the `## [X.Y.Z]` section of `CHANGELOG.md`, followed by the install steps. The workflow stops if the section is missing. The workflow does not use GitHub generated notes.
9. **Public content.** Release titles and notes follow [data safety](../policies/data-safety.md). They contain no user content or account data.

## Consequences

- At the first start, macOS blocks the downloaded app. The user allows it in System Settings > Privacy & Security > Open Anyway, or removes the quarantine attribute.
- An ad-hoc signature changes with every build. After an update, macOS can ask again for privacy permissions and Keychain access. The app registers the Worker LaunchAgent again ([build and release](../development/build-and-release.md)).
- A Developer ID signature, notarization, an Intel build, and automatic updates need a new owner decision.
- GitHub adds "Source code" archives to each release. They cannot be removed. The app asset is only the `.zip` file of `QADREN.app`.
