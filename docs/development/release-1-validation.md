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

The executable baseline is `03e67df`. Later commits in this sequence changed documentation only.

| Check | Result | Evidence scope |
|---|---|---|
| `bash scripts/test-core.sh` | PASS, 252 tests | Rust Core on `03e67df` |
| `bash scripts/test-macos-scenario-dry-run.sh` | PASS | SwiftUI to Rust Scenario dry-run boundary on `03e67df` |
| `bash scripts/build-macos.sh` | PASS | Unsigned macOS development bundle on `03e67df` |

## Current Capability Evidence Matrix

| Capability | Current evidence | Not yet established |
|---|---|---|
| Rust model and durable JobStore | Core test suite passes on the recorded baseline. | A real user Local-to-Local copy through the complete UI, JobStore, filesystem, verification, SQLite, and UI path. |
| App-owned mock operation | No-overwrite, durable-attempt, verification, and recovery tests exist in `job_execution.rs`. | User content transfer. The mock is not a release substitute. |
| Local Filesystem metadata and `mkdir` | Rust and SwiftUI Local Filesystem interfaces exist. `mkdir` uses a durable Job path. | Generic arbitrary user-file content read on macOS and a verified no-overwrite copy. |
| macOS File Provider safety | Local File Provider guards reject known nonresident values. | A public generic residency proof for arbitrary user-selected provider content. Keep user content read fail-closed until that proof exists. |
| Scenario planning | Dry-run FFI passes. Enabled local connections gate the metadata snapshot. | Snapshot-to-Job submission, one-way and two-way execution, conflict handling, and recovery. |
| Scheduler dispatch | Durable schedule dispatch can queue a Job. | Independent timer, app-exit lifecycle, event debounce, and real Scenario execution. |
| Native UI | Design system, widget canvas, settings, and local file navigation exist. | Full visual acceptance, removal of legacy mock actions, and real-copy interaction. |
| Distribution | The macOS build script creates an unsigned development bundle. | Signing, notarization, clean install, restart validation, and release package. |

## Mandatory Gates for a Real Local Copy

1. Create a durable attempt before the external filesystem effect.
2. Check capability, operation-specific consent, destination state, and no-overwrite.
3. Check macOS file residency before reading user content. Reject an item when the required proof is absent.
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
