# Testing and Quality Criteria

Status: mandatory validation strategy. Bootstrap CI is implemented in `.github/workflows/bootstrap.yml`. Add integration checks with the feature that they validate.

## Test Pyramid

1. **Rust unit tests:** Scenario, Job, `FileSyncState`, revision, conflict, deletion, state-transition, and policy rules.
2. **Adapter contract tests:** API mocks with incomplete metadata, delays, authorization errors, and repeated responses.
3. **Integration tests:** Rust/Swift FFI, Keychain, Yandex API, and Local Filesystem with test credentials and directories. iCloud adapter, entitlement, and integration checks are outside Release 1.
4. **End-to-end tests:** two folders or storage connections, real copies, failures, and execution recovery from service state. Do not store historical content versions.
5. **Resource checks:** idle CPU and RAM, local disk, network bytes, and signed application size.

## Mandatory Scenarios for Each Data Operation

- Transfer interruption and under-downloaded temporary object.
- Restart between transfer and confirmation.
- Double start of one Job and repeat after timeout.
- Token expiration, rate limit, and cloud unavailability.
- Independent-version conflict and mass deletion.
- Incorrect or missing checksum.
- A QADREN-initiated download on the primary MacBook does not start without confirmation, including verification and recovery.
- A Controller does not download content or start the same delegated Job.
- A successful Yandex Disk API response does not prove integrity of the written copy. Local Filesystem rejects hidden iCloud or File Provider materialization.

## Before Publication to `main` or Release

- Run `bash scripts/test-core.sh` for Rust formatting, clippy, and tests.
- Run applicable `cargo test`, platform Swift tests, and integration scenarios in available environments.
- For the Swift-to-Rust Job inventory bridge, run `bash scripts/test-macos-job-inventory.sh`. It uses only a temporary app-owned mock root and does not start `QADREN.app`.
- For the Local Filesystem `mkdir` bridge, run `bash scripts/test-macos-local-workspace.sh`. It creates and deletes only a unique test root in the checkout and verifies no-overwrite.
- For the macOS Keychain lifecycle, run `bash scripts/test-macos-keychain.sh`. It uses an unpredictable test-only connection ID and removes only that test item.
- Perform dependency and security review. Verify no secrets and updated canonical documentation.
- Build the macOS artifact and verify `.app` completeness and dependencies.
- Before a user release, verify signing, notarization, and a clean-install smoke test.
- If a test set is incomplete, list every not-performed item in the validation record and final handoff. Publication directly to `main` does not remove code-review or security-review requirements.

Establish performance thresholds and supported OS versions after the baseline prototype. iCloud synchronization, backup, entitlement spike, and end-to-end work are outside the active release scope per D-017. Maintain standard macOS File Provider safety-guard checks for the local adapter.

## Acceptance Evidence

Record for every gate: base SHA and final `HEAD`, command, OS and toolchain version, `PASS`, `FAIL`, `BLOCKED`, or `SKIPPED`, evidence source (unit, mock, native, or live), commit link, and untested scenarios. Do not extrapolate Rust test results to macOS Keychain, Apple entitlements, or a real cloud API. Mac checks performed by the owner are valid only for the recorded SHA and commands.

If a Mac is unavailable, run available Rust and static checks. Stop only the corresponding native gate and do not claim release readiness. If the Hub is unavailable, run standard commands and review without it. Hosted CI does not prove real integration by itself.

## Context Transfer Check

Before an independent implementation session, perform a read-only context check. Do not edit files or commit during this check.

1. Record current `HEAD` and `git status -sb` when the local tree is available.
2. Map existing `JobStore`, `jobs_ffi.rs`, `JobInventory.swift`, `verification.rs`, and `local_fs.rs` APIs. State the difference between read-only inventory and execution.
3. Define one minimal functional acceptance scope: no-overwrite create, durable attempt before effect, verified or `UNKNOWN` outcome, FFI command, real UI status, and negative tests.
4. List conditions that require owner input: new data-loss risk, policy conflict, missing Apple entitlement, production credentials or consent, or an undefined distribution decision.
5. State the code and security review plan, supported test commands, and unconfirmed real-Mac or live-cloud checks.

The context check passes only when it matches current code and preserves safety policy. It fails if it proposes a second JobStore, disables consent, treats a mock as production end-to-end behavior, creates BackupSet, introduces a normal branch workflow, or starts a file effect without durable attempt preparation. If repository access is unavailable, record the check as unavailable and do not infer the missing state.
