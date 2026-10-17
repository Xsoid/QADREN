# Release 1 Validation Record

Status: current validation contract and evidence summary. This document does not define product scope. [D-018](../decisions/D-018-local-first-priorities.md) and the [roadmap](../roadmap.md) own the product sequence.

## Scope

Release 1 is a local-first macOS product. It includes one Rust JobStore, Local Filesystem connections, manual local operations, local Scenarios, and later independent local scheduling. Yandex Disk and iCloud are post-release work.

## Evidence Rules

- Record the exact commit SHA, worktree state, operating-system version, and command for each result.
- A test result applies only to the executable source at that SHA.
- A local result does not prove hosted CI, a signed artifact, or a production release.
- A successful unit test does not prove user-file transfer, restart safety, or a destructive-operation guard.
- Record unavailable validation as unavailable. Do not replace it with inference.
- Keep external work-tracker links outside this document. They are not validation evidence.

## Current Recorded Local Evidence

The executable baseline is `f9121b2`.

| Check | Result | Evidence scope |
|---|---|---|
| `cargo clippy --manifest-path core/Cargo.toml --lib -- -D warnings` | PASS | Rust Core lint on `f9121b2` |
| `cargo test --manifest-path core/Cargo.toml --lib` | PASS, 269 tests | Rust Core on `f9121b2` |
| `scripts/test-macos-local-workspace.sh` | PASS | SwiftUI to Rust Local file-copy bridge on `f9121b2` |
| `scripts/test-macos-scenario-dry-run.sh` | PASS | SwiftUI Scenario metadata, conflict queue, and FFI bridge on `f9121b2` |
| `./script/build_and_run.sh --verify` | PASS | Unsigned local development bundle and running QADREN process on `f9121b2` |

`cargo fmt --manifest-path core/Cargo.toml --check` does not pass on this baseline. It reports existing formatting differences outside the Local Scenario update patch. Resolve or explicitly accept that debt before a Release claim.

## Current Capability Evidence Matrix

| Capability | Current evidence | Not yet established |
|---|---|---|
| Rust model and durable JobStore | Core test suite and manual and Scenario Local-to-Local no-overwrite copy paths pass on the recorded baseline. Rust core, FFI, and File Manager have a staged controlled replacement for an existing regular Local file. Persisted UNKNOWN copies, including replacements, can be listed for the bound destination root and checked through read-only exact metadata reconciliation. | Real File Provider materialization evidence is not recorded. |
| App-owned mock operation | No-overwrite, durable-attempt, verification, and recovery tests exist in `job_execution.rs`. | User content transfer. The mock is not a release substitute. |
| Local Filesystem metadata, `mkdir`, and copy | Rust and SwiftUI interfaces provide confirmed one-file no-overwrite copy through JobStore. Local metadata revisions support a revision-bound Scenario baseline. | A real File Provider materialization run. |
| macOS File Provider safety | A copy requires a user-confirmed, exact-scope durable decision before opening the source. | A real File Provider materialization run with a user-selected provider item. |
| Scenario planning and conflict queue | Dry-run FFI passes. Enabled local connections gate the metadata snapshot. User-confirmed `Create` and `Update` items submit pinned Jobs in a permitted direction and write `FileSyncState` through a durable outbox. A revision-bound Registry queue stores both conflict snapshots and a user-selected source and decision. Selected `replace_from_source` and `replace_if_newer` decisions create a pinned replacement Job only after a second snapshot check. The second action uses Local Filesystem mtime fields and makes no replacement when the source is not newer. A mismatch marks the task `stale` without an effect. The queue has no JobStore dependency and does not block ordinary transfers. | Batch and autonomous multi-item execution, deletion effects, and recovery. |
| Scheduler dispatch | Durable schedule dispatch can queue a Job. | Independent timer, app-exit lifecycle, event debounce, and real Scenario execution. |
| Native UI | Design system, widget canvas, settings, and local file navigation exist. | Full visual acceptance, removal of legacy mock actions, and real-copy interaction. |
| Distribution | The macOS build script creates an unsigned development bundle. | Signing, notarization, clean install, restart validation, and release package. |

## Mandatory Gates for a Real Local Copy

1. Create a durable attempt before the external filesystem effect.
2. Check capability, operation-specific consent, destination state, and no-overwrite for a new copy. A controlled replacement needs its approved replacement protocol.
3. Record and consume exact one-time consent before reading user content that can materialize a File Provider item.
4. Verify the copied destination after the effect and store the durable result.
5. Use `UNKNOWN` if proof is lost. Reconcile in read-only mode. Do not retry blindly.
6. Prove concurrent-change protection before any destructive Local Filesystem operation.
7. Exercise cancel, crash, restart, permission loss, destination conflict, and repeated request paths.

## Mandatory Gates for Release Claim

1. Run the Core, native, adapter, and integration checks that cover the final executable SHA.
2. Run the macOS build and verify the installed artifact on a clean user environment.
3. Verify UI behavior without mock success values.
4. Verify independent service behavior after UI exit and after restart.
5. Record signing and notarization status.
6. Run the project-scoped pre-publish gate, code review, and security review on the exact final patch before commit, push, or pull request.

Use [current-state audit](current-state-audit.md) for the code snapshot and [testing and quality](testing-and-quality.md) for test selection.
