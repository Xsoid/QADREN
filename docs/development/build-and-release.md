**D-018 / 2026-10-14:** the local release P0–P2 is built and tested without the Yandex Disk controller, live API, new OAuth, and iCloud entitlements. Yandex feature-work/live E2E is moved after Release 1; iCloud is moved indefinitely. The old G1/G2 cloud sequence is historical below: local Job→Local FS and safety are checked first. Signing and distribution relate to the **local** release and do not require a cloud service. [D-018](../decisions/D-018-local-first-priorities.md).

# Build and Release

Status: developer build scripts are implemented; the release pipeline and distribution model are not approved.

## Delivery Architecture

- On macOS, the result is `QADREN.app`, which is a self-contained native package.
- Rust components are compiled into machine code and **delivered inside the application** (as a library or an executable component — decide based on the prototype).
- SwiftUI and Apple APIs use standard macOS frameworks.
- The end-user computer does not require a separate installation of Rust/Swift/Cargo/Xcode/Node/Python/Docker.
- Cloud operation requires accounts, network, system permissions, and a compatible OS version.

## Development

Local build: `bash scripts/test-core.sh` for Rust fmt/Clippy/tests; `bash scripts/build-macos.sh` for the unsigned macOS developer bundle `dist/QADREN.app`. For a standard developer launch, use `./script/build_and_run.sh`: it stops the previous process, builds this bundle, and starts it as a macOS app; `--verify` additionally confirms the process. For temporary native theme acceptance, you can add `--theme dark`, `--theme light`, or `--theme system`: the override is passed only to the started process and does not change the user setting. These commands do not replace real E2E. Local build requires the Rust toolchain and Xcode on the developer Mac. Keep the Rust and Swift build reproducible: fix the toolchain in the manifests after selecting versions, use a lockfile, and use a minimum build script. Do not upload dependency binaries from unverified sources into the release pipeline.

## Release Pipeline (in design)

1. Check tests, lint, license control, and dependencies.
2. Build an optimized release configuration; minimize debug symbols only in the user artifact.
3. Form the .app with correct signing and sandbox/entitlements only if they are required for the selected distribution model and real Yandex Disk/Local FS functions; iCloud entitlements are not required.
4. Check the correct loading of the Rust component on a clean supported macOS.
5. Sign with Developer ID and perform notarization (for distribution outside the Mac App Store); check Gatekeeper.
6. Check installation, launch, update, and removal without third-party runtimes.

The choice of distribution via the App Store or directly **is not yet decided**. App Sandbox, entitlements, and background execution depend on this choice and must be checked separately.

## Other OS

In the future, the common Rust core is compiled for Windows/Linux. Platform UI and secrets are separate modules. The iCloud Documents adapter is postponed (D-017) and is not part of the release criteria for other OS. The goal is a native package without a third-party runtime, but system libraries and OS version requirements are acceptable.

## End-to-End Integration Runbook (2026-10-11)

1. Save the `main` SHA, versions of macOS/Xcode/Swift/rustc, and the test fixture; do not use user files.
2. `bash scripts/test-core.sh`: fmt, Clippy, Rust tests. On Mac, also run `bash scripts/build-macos.sh` and check `dist/QADREN.app`; this is an unsigned developer bundle, not a release.
3. For G1, prepare a test source stream and a *missing* destination path. Submit an idempotent manual Job through the same Rust JobStore, independent of the frontend. No Scenario for a one-time operation.
4. Before a modifying effect, check connection/capability/actual scope and any required destructive-operation consent; persist a durable attempt. Do not ask separately for ordinary source reads or File Provider materialization. Perform a no-overwrite new-file write and metadata verification; save the receipt in SQLite and display it in SwiftUI. Distinguish an adapter ACK from proven integrity.
5. Fault matrix: missing destructive replacement/deletion consent (zero destructive I/O), existing destination (no unapproved overwrite), short/long stream, adapter 401/429/5xx, timeout after possible write, restart after effect before DB commit, duplicate attempt, cancel/pause, connection disconnection. Automatic read-only reconciliation should determine the effect when possible; inconclusive remains UNKNOWN. Never replay a destructive operation blindly.
6. Compare the file tree, object metadata, and SQLite before and after; ensure no leaked secrets or out-of-scope mutations. A necessary source read may download File Provider content without a prompt. Do not prefetch unrelated files.
7. For G2, replace the mock with real supported adapters; isolate real checks of macOS Keychain, Yandex account, and Local FS permissions from mock tests; do not check iCloud entitlement. Record the results in `release-1-validation.md` with SHA, commands, and failed scenarios.
8. Perform a code and security review. Then update the corresponding validation record. If a step is not reproducible, specify `BLOCKED`, not done.

**Architectural Working Solution:** the first runner works in-process SwiftUI → Rust through a narrow FFI. The release gate retains independent execution after the UI closes: after G1/G2, the same Rust engine is moved to a background service without new JobStores or competing owners. The choice of Mac App Store/direct signed distribution remains open until the macOS signing/sandbox/service feasibility check, independent of iCloud. GitHub Actions are optional.
