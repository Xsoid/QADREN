# Testing and Quality Criteria

Status: mandatory strategy. Bootstrap CI is implemented in `.github/workflows/bootstrap.yml`; additional integration checks are added as the corresponding features are implemented.

## Test Pyramid

1. **Rust unit tests:** Scenario/Job/FileSyncState rules, revisions, conflicts, deletions, state transitions, and policy engine (as implemented).
2. **Adapter contract tests:** API mocks with incomplete metadata, delays, authorization errors, and repeated responses.
3. **Integration tests:** Rust/Swift FFI, Keychain, Yandex API, and Local Filesystem with test credentials/directories. iCloud adapter, entitlement, and integration checks are excluded from Release 1.
4. **End-to-end:** two folders/two storages, real copies, failures, execution recovery from service state (without storage of historical content versions).
5. **Resources:** idle CPU/RAM, local disk, network bytes, signed application size.

## Mandatory Scenarios for Each Data Operation

- Transfer interruption; under-downloaded temporary object.
- Restart between transfer and confirmation.
- Double start of one job, repeat after timeout.
- Token expiration, rate limit, cloud unavailability.
- Conflict of independent versions, mass deletion.
- Incorrect/missing checksum.
- Primary MacBook: download initiated by QADREN **does not start without confirmation**, including verification and recovery.
- Delegation: Controller does not download content and does not start the same job.
- Yandex Disk: a successful API response does not prove the integrity of the written copy; Local FS prohibits hidden materialization of iCloud/File Provider placeholders.

## Before Publication to main / Release

- Rust formatting and static analysis: `bash scripts/test-core.sh` (Rust fmt, clippy, and tests); CI already runs this script.
- `cargo test`, platform Swift tests, and integration scenarios for available environments. For native Swift→Rust Job inventory bridge: `bash scripts/test-macos-job-inventory.sh`; it uses only a temporary app-owned mock root and does not start QADREN.app. For real Local controller `mkdir` bridge: `bash scripts/test-macos-local-workspace.sh`; it creates and deletes only a unique test root in the checkout and verifies no-overwrite. For actual macOS Keychain lifecycle: `bash scripts/test-macos-keychain.sh`; the runner uses an unpredictable test-only connection ID, verifies store/read/update/delete/idempotent delete, and removes only this test item.
- Dependency/security review, verification of no secrets, verification of updated documents.
- Release compilation for macOS, verification of .app completeness and dependencies.
- Signing/notarization and clean-install smoke test during release to users.
- If the test set is incomplete, the report in the working Issue and the final handoff must contain an explicit list of **not performed** items. Publication directly to `main` does not remove the requirement for code review and security review.

Establish performance thresholds and supported OS versions after the baseline prototype. iCloud sync/backup, entitlement spike, and iCloud E2E are outside active tasks and the release gate per D-017. Checks for standard macOS File Provider safety guards for the local adapter are maintained.

## Acceptance Evidence for Autonomous Agent

Record for each gate and Issue: `base SHA` and final `HEAD`, command, OS/toolchain version, status PASS/FAIL/BLOCKED/SKIPPED, evidence source (unit/mock/native/live), commit link, and untested scenarios. Do not extrapolate Rust test results to macOS Keychain, Apple entitlements, or real cloud API. Mac checks performed by the user are valid only for the presented SHA and commands.

If a Mac is unavailable, perform available Rust/static checks and stop **only the corresponding native gate**; do not declare release-ready. If the Hub is unavailable, run standard commands and review without the Hub. CI is optional and does not prove real integration by itself.

## Codex Transfer Verification (without changing product code)

Before independent execution of #18, perform one **read-only rehearsal** in a new Codex session. This is a check of the transferred context quality, not a request to start implementation.

**Rehearsal Task:** read `AGENTS.md`, Epic #12, Issue #18, `docs/roadmap.md`, profile policies, and current code. Without commits or file editing, provide:
1. Exact current HEAD and `git status -sb` (if the local tree is available); if the environment is unavailable, state that the check is not performed.
2. A map of **existing** `JobStore`, `jobs_ffi.rs`, `JobInventory.swift`, `verification.rs`, `local_fs.rs` APIs with the exact difference between read-only inventory and execution.
3. One minimal functional G1 acceptance scope: no-overwrite mock create, durable attempt until effect, verified/UNKNOWN outcome, FFI command, and real UI status. List negative tests.
4. A list of conditions where the owner must be contacted: new data loss risk, policy conflict, missing Apple entitlement, production credentials/consent, undefined distribution decision.
5. Code/security review plan, supported test commands, and explicitly unconfirmed real-Mac/live-cloud checks; no new JobStore, Backend Worker, or mandatory PRs.

**PASS:** all five points match the current repository, real APIs are listed, safety policy is maintained, and false DONE is avoided. **FAIL:** Codex proposes to rewrite the queue, disable consent, treat mock as production-E2E, create BackupSet, assign another branch as the norm, or start file operations without preparing an attempt. **BLOCKED:** cannot access the repository/Issue. In case of FAIL, correct context sources in existing documents instead of inventing a separate memory system.

Only after a successful rehearsal can independent implementation of the Issue start. Record the rehearsal result (SHA, PASS/FAIL/BLOCKED, deficiencies) in Issue #18. The rehearsal as part of current documentary preparation **is not yet performed**.
