# Testing and Quality

## Test layers

1. Rust unit tests cover Scenario, Job, `FileSyncState`, policy, state transition, conflict, and recovery rules.
2. Adapter contract tests cover incomplete metadata, authorization failure, timeout, repeated response, and capability limits.
3. Native integration tests cover Rust/Swift FFI, Keychain, Local Filesystem, and Worker boundaries.
4. End-to-end tests cover real selected roots, copies, failure, restart, and reconciliation.
5. Release validation covers the signed bundle, clean installation, launch, lifecycle, and resource use.

Do not treat a mock, unit test, or unsigned bundle as evidence for a different layer.

## Required operation coverage

For each modifying operation, test durable attempt creation before the effect, exact scope and capability checks, new-destination no-overwrite, destructive confirmation where required, interruption, restart, repeated request, destination conflict, permission loss, and reconciliation. Verify that an uncertain outcome remains `UNKNOWN` when evidence is insufficient. Do not test only the success path.

An enabled or requested operation may read necessary content without a separate read-consent test. Test File Provider compatibility when a suitable fixture exists. Do not prefetch unrelated content.

## Commands

Run only the commands relevant to the patch. The standard baseline is:

```bash
bash scripts/test-core.sh
bash scripts/build-macos.sh
```

Use the relevant `scripts/test-macos-*.sh` command for changed macOS, Worker, storage, schedule, Keychain, or Scenario behavior.

`scripts/test-macos-settings-e2e.sh` drives the rendered Settings window through macOS Accessibility. It starts the built app and a local Worker on a disposable data root. The app reads the data root from the `QADREN_DATA_ROOT` environment variable, so the run does not change the user's Registry, JobStore, or files. The script needs a logged-in session and Accessibility permission for the process that runs it. Do not use the Mac while the script runs: a click or key press in a test window changes the result. Without Accessibility permission the script exits with status 77 (`BLOCKED`). Run `python3 -m json.tool docs/design/tokens.json >/dev/null` when the token file changes. Native checks require a supported Mac.

## Publication and release evidence

Before commit, push, or PR, run the project-scoped AI Docs Hub pre-publish gate and repeat code and security review for the exact patch fingerprint. Record the base and final SHA, command, environment, result, evidence layer, and untested cases in the applicable GitHub Issue or handoff. Use `PASS`, `FAIL`, `BLOCKED`, or `SKIPPED`. Do not infer unavailable evidence.
