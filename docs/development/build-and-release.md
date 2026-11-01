# Build and Release

## Development build

On a supported macOS developer machine, run:

```bash
bash scripts/test-core.sh
bash scripts/build-macos.sh
open dist/QADREN.app
```

`scripts/build-macos.sh` creates an ad-hoc signed developer bundle at `dist/QADREN.app`. The ad-hoc signature seals the bundle, so `SMAppService` accepts the bundled Worker LaunchAgent. It is not a Developer ID signature and not notarized. Use `./script/build_and_run.sh` to rebuild and start that bundle. Its `--verify` option checks the started process. These commands do not prove release readiness.

The ad-hoc signature gives the Worker a new cdhash with every build. `SMAppService` binds the Worker LaunchAgent to the cdhash of the registered build, and launchd refuses another Worker binary with `OS_REASON_CODESIGNING` (exit 78). Therefore every build gets a unique `CFBundleVersion`. When the app starts a new build, it first sends `prepare_restart`: the Worker finishes the files that run now and starts no new one, so a long copy never blocks the update and no file write is cut ([local Worker runtime](../architecture/local-worker-runtime.md)). When no file runs, the app unregisters and registers the LaunchAgent again and checks that the Worker answers the IPC `status` request. macOS applies the new cdhash only some time after the build (about 30 s on 2026-10-25), so the app repeats the registration with growing pauses for up to about 4 minutes. If the Worker still does not answer, the next app start tries again. A signature with a Team ID removes this cdhash binding. `scripts/build-macos.sh` therefore signs with the first valid "Apple Development" identity (a free Personal Team is enough), or with `QADREN_SIGN_IDENTITY`, and falls back to ad-hoc only without one. With a Team ID, launchd binds the LaunchAgent to the Team ID and the identifier (checked on 2026-10-25: a Worker with another cdhash started without a new registration), the first re-registration for a new build works, and macOS keeps privacy permissions and Keychain access between builds. The Apple Development certificate needs the "Apple Worldwide Developer Relations Certification Authority G3" intermediate in the login keychain.

The developer machine needs Rust and Xcode. The user distribution must include its Rust components and must not require Rust, Swift, Cargo, Xcode, Node, Python, or Docker.

## Release requirements

1. Build the selected release configuration from a fixed commit.
2. Run the applicable checks in [testing and quality](testing-and-quality.md).
3. Verify the bundle on a clean supported macOS installation.
4. Verify the selected signing, notarization, entitlement, installation, launch, update, and removal paths.
5. Record release evidence in the applicable GitHub Issue.

## GitHub Releases

[D-026](../decisions/D-026-github-releases.md) defines the distribution channel. To publish a release:

1. Set the same version in `CFBundleShortVersionString` of `platform/macos/Info.plist` and in `version` of `core/Cargo.toml`. Run `cargo update --workspace --manifest-path core/Cargo.toml` to update `Cargo.lock`. Add a `## [X.Y.Z] - YYYY-MM-DD` section to `CHANGELOG.md`. It becomes the release description. Commit and push to `main`.
2. Make sure that the repository variable `YANDEX_CLIENT_ID` has the public Yandex OAuth client ID.
3. Push a tag with the same version:

   ```bash
   git tag -a v0.2.0 -m "QADREN 0.2.0"
   git push origin v0.2.0
   ```

4. The `Release` workflow (`.github/workflows/release.yml`) checks the versions and the `CHANGELOG.md` section, runs `scripts/test-core.sh`, builds the bundle, and publishes a pre-release with `QADREN-<version>-macos-arm64.zip` and its SHA-256 file.

A release build has an ad-hoc signature. It is not Developer ID signed and not notarized. A Developer ID signature and notarization need a new owner decision. iCloud entitlements are not a Release 1 requirement. A release bundle must not introduce a second JobStore, Worker, or synchronization engine.

## Other platforms

The Rust core can support later Windows and Linux packages. Platform UI, credentials, and OS capabilities remain platform-specific. This does not expand Release 1 scope.
