# Testing and Quality

## Test layers

1. Rust unit tests cover Scenario, Job, `FileSyncState`, policy, state transition, conflict, and recovery rules.
2. Adapter contract tests cover incomplete metadata, authorization failure, timeout, repeated response, and capability limits.
3. Native integration tests cover Rust/Swift FFI, Keychain, Local Filesystem, and Worker boundaries.
4. End-to-end tests cover real selected roots, copies, failure, restart, and reconciliation.
5. Release validation covers the signed bundle, clean installation, launch, lifecycle, and resource use.

Do not treat a mock, unit test, or unsigned bundle as evidence for a different layer.

## Energy efficiency checks

[D-028](../decisions/D-028-energy-efficiency.md) is part of the test layers:

- A Rust test for a store read on a run path or a poll path counts the SQLite VM steps of the read at two data sizes with `energy_test::vm_steps`. `energy_test::assert_flat_cost` fails when the cost grows with the size of the data. Use a ratio of 100 or more between the sizes. The meter test `the_meter_detects_a_full_scan` proves that the check finds a full scan.
- A table of maintained counts has a test that compares it with a full count after each kind of change: insert, update, delete, and the migration of existing rows.
- A native test of a polling feed checks that a poll with unchanged data publishes no change, so the interface does not redraw.
- A change to a run path, a store, a polling loop, or continuous motion is checked with a CPU profile of the real app (`sample <pid>`). Record the CPU use before and after. Evidence states only the storage type and the result.

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

Each native test script sets `QADREN_DATA_ROOT` to its own temporary directory. A test never opens the real data root: an open of the real Registry by a newer build migrates it, and the Worker of the older build then cannot open it.

`scripts/test-macos-settings-e2e.sh` drives the rendered Settings window through macOS Accessibility. It starts the built app and a local Worker on a disposable data root. The app reads the data root from the `QADREN_DATA_ROOT` environment variable, so the run does not change the user's Registry, JobStore, or files. The script needs a logged-in session and Accessibility permission for the process that runs it. Do not use the Mac while the script runs: a click or key press in a test window changes the result. Without Accessibility permission the script exits with status 77 (`BLOCKED`). The opt-in section `E2E_SECTIONS=fileprovider` needs `FP_SOURCE`, an owner-approved provider folder with evicted placeholders, and `FP_ORIG`, local copies of the same bytes. The caller creates and removes these test files. `QADREN_AGENT_TEST=1 bash scripts/test-macos-worker-agent.sh` proves the LaunchAgent: registration, Worker operation after Quit, a scheduled run while the app is closed, restart by launchd after a kill, and unregister. It uses a test copy of the app with its own bundle identifier and service label on a disposable data root, and it always unregisters the service. Without the variable the script reports `SKIPPED`. The opt-in section `E2E_SECTIONS=schedule` waits for real wall-clock schedule slots and takes about 10 minutes. It needs the app under test in front, so nobody may use the Mac during the run. Run `python3 -m json.tool docs/design/tokens.json >/dev/null` when the token file changes. Native checks require a supported Mac.

## Publication and release evidence

Before commit, push, or PR, run the project-scoped AI Docs Hub pre-publish gate and repeat code and security review for the exact patch fingerprint. Record the base and final SHA, command, environment, result, evidence layer, and untested cases in the applicable GitHub Issue or handoff. Use `PASS`, `FAIL`, `BLOCKED`, or `SKIPPED`. Do not infer unavailable evidence. Evidence from real user storage contains no names, paths, counts, or sizes of user data ([data safety](../policies/data-safety.md)).
