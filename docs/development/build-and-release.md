# Build and Release

## Development build

On a supported macOS developer machine, run:

```bash
bash scripts/test-core.sh
bash scripts/build-macos.sh
open dist/QADREN.app
```

`scripts/build-macos.sh` creates an unsigned developer bundle at `dist/QADREN.app`. Use `./script/build_and_run.sh` to rebuild and start that bundle. Its `--verify` option checks the started process. These commands do not prove release readiness.

The developer machine needs Rust and Xcode. The user distribution must include its Rust components and must not require Rust, Swift, Cargo, Xcode, Node, Python, or Docker.

## Release requirements

1. Build the selected release configuration from a fixed commit.
2. Run the applicable checks in [testing and quality](testing-and-quality.md).
3. Verify the bundle on a clean supported macOS installation.
4. Verify the selected signing, notarization, entitlement, installation, launch, update, and removal paths.
5. Record release evidence in the applicable GitHub Issue.

The distribution channel and its exact signing model require an owner decision. iCloud entitlements are not a Release 1 requirement. A release bundle must not introduce a second JobStore, Worker, or synchronization engine.

## Other platforms

The Rust core can support later Windows and Linux packages. Platform UI, credentials, and OS capabilities remain platform-specific. This does not expand Release 1 scope.
