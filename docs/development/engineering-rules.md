**D-018 (2026-10-14) — mandatory procedure:** first P0 UI/foundation and diagnostics, then P1 Local FS/Job controller, P2 independent service/local release. Yandex Disk controller is moved beyond Release 1; new Yandex API/OAuth/transfer implementation is frozen until a new decision from the owner; iCloud is indefinite. **Do not delete** existing adapters, mock tests, and user credentials. Old cloud-first instructions below do not override this decision. [Plan](../roadmap.md).

# QADREN Development Rules

Status: **mandatory rules for new patches**. Also read [AGENTS.md](../../AGENTS.md).

## 1. Minimalism

- Start with the simplest solution that covers the current scenario.
- Do not create abstractions for imagined integrations. Use a narrow, verifiable adapter contract for extensibility.
- Do not add a separate daemon/process, HTTP server, IPC, or scheduler without proven necessity.
- Do not duplicate Rust business logic in Swift UI. UI displays state and sends intentions.
- Separate logical modules; **do not create a separate crate for every entity**.
- For each external dependency, specify: why it is needed, the alternative, and the impact on binary size, maintenance, and security.

## 2. Module Boundaries

- General reservation logic, state, and rules are in Rust, without Apple API imports.
- Platform capabilities are in separate adapters/wrappers.
- Rust/Swift interaction is through a limited FFI: explicit memory ownership, error handling, and safe task cancellation.
- Adapter types and errors must describe **operation supportability**, not just success/failure.
- Do not treat an accepted Yandex API upload as a fully confirmed cloud copy. iCloud transfer and adapter work are postponed per D-017; do not implement them in the current release scope.
- Do not implement postponed network protocols and inter-node synchronization "just in case".

## 3. Preservation and Privacy

- No destructive operations without an explicitly provided policy, verified protection against concurrent changes, and sufficient confirmations. There is no file versioning in the current release; the operation remains fail-closed until safety is proven.
- No "last writer wins" for independently modified files.
- The execution of a delegated task belongs only to the Worker. The Controller does not re-verify content by downloading.
- Any download path **initiated by QADREN** on the main MacBook goes through a resolution mechanism. Do not interfere with macOS/iCloud and other programs.
- OAuth tokens and keys must not enter source code, logs, test fixtures, or Git.
- Logs do not record file contents, tokens, or private URLs with authorization by default.

## 4. Errors and Failures

- Errors must be typed and contextual.
- Any task survives a restart or is moved to an explicit recoverable state.
- Operation repetition is idempotent or prohibited until the remote result is determined.
- Do not treat a timeout as "task definitely not completed".
- Do not suppress errors and do not mark the absence of a check as success.

## 5. Project Changes

- One cohesive functional package equals one goal, its tests, and review. Work directly in `main`; PRs and additional branches are not mandatory.
- Before making changes, read the README, profile specification, decision log, and tests.
- If a patch changes an accepted principle, first propose an ADR change and explicitly state incompatibilities.
- Update documentation and tests within the same functional package. Do not leave new solutions exclusively in chat or an external work tracker.
- Do not claim a successful verification without actually running the command.

## Codex workflow for Release 1 (2026-10-11)

- **Start:** read AGENTS.md, `docs/roadmap.md`, the validation record, and specific code files. Identify existing APIs and baseline SHA. Do not create a second JobStore, Policy, Registry, or documentation branch.
- **Plan:** define the functional vertical, contracts on Rust/Swift/SQLite/FFI boundary, external effects, scope consent, UNKNOWN reconciliation, and completion criteria. If a platform limitation is not proven, the feature is fail-closed.
- **Implement:** one complete package with behaviour test + negative/fault matrix, not a series of cosmetic fixing tests without function. The first runner is hosted SwiftUI in-process; the Rust engine remains independent of the UI and suitable for service extraction.
- **Verify:** `bash scripts/test-core.sh`, applicable macOS Swift/FFI scripts, mock HTTP, and real platform tests where necessary. Do not confuse mock PASS with real macOS/cloud security.
- **Review:** code review for correctness, ownership, and ABI; security review for content policy, paths, races, secrets, overwrite, cancellation, and reconciliation. Perform explicit diff review, then commit directly to main. Do not declare Done without acceptance evidence.
- **Report:** commit SHA, verified and unverified criteria, environment version, and known risks. After completing a package, update the canonical documentation when the project state changed. Hosted CI is optional. Do not switch to Work automatically.

## AI-Docs-Hub Integration: procedure for chat and Codex

This is a supplement to the mandatory compatibility policy in `AGENTS.md`. AI-Docs-Hub is an optional dev-tool, not part of the QADREN runtime.

1. **Onboard without writing:** run `python3.11 scripts/onboard-project --project <qadren-project-id>` from the installed AI-Docs-Hub (substitute the actual ID after creating the project config). Similarly, run `scripts/scaffold-project-docs --project <id>` without `--write`. Review actions and diff; do not accept the generation of parallel architectural directories.
2. **Write only consciously:** ensure the Hub adds a maximum of idempotent managed blocks to the existing root `AGENTS.md`; if QADREN documents are not empty, write is prohibited if it changes their meaning or placement. Clarify inclusion/exclusion configurations, exclude credentials, temporary transfer files, SQLite, and local artifacts.
3. **Context:** the agent starts with QADREN `AGENTS.md` and current `docs/`, using Hub RAG/Codebase Memory only as a navigation index with a freshness check relative to HEAD. Chat without the Hub performs exactly the same steps directly on the repository.
4. **Verification of changes:** canonical QADREN commands (`bash scripts/test-core.sh`, `bash scripts/build-macos.sh` on macOS plus profile tests) remain primary. Hub verify/review adds evidence but does not replace native or live provider E2E. Code review and security review are mandatory before publication, regardless of Hub availability.
5. **Result:** update the validation record when its current evidence changes. Record SHA, environment, PASS/FAIL/SKIPPED/BLOCKED, mock-vs-real evidence, and known risks. Never write secrets, user files, or transfer contents into the Hub. Avoid Git-tracked Hub-generated snapshots inside QADREN.

Installing the Hub on a machine and creating a local project binding is a separate action **not** performed automatically by changes to this document. QADREN directives take priority over the standard Hub template.

## Autonomous Transfer between sessions (Chat → Codex → Chat)

**Purpose:** the agent can change at any moment. Chat memory and the AI-Docs-Hub index are not sources of truth. Continue work from the current `main`, canonical documentation, and recorded validation evidence. Do not create a parallel state file or a second task board.

### Session Start

1. Check `git status -sb`, `git rev-parse HEAD`, and `git pull --ff-only origin main` **only if the tree is clean**. Do not overwrite others' uncommitted changes and do not perform force or reset. Compare the current code with the audit and validation record.
2. Read `AGENTS.md`, `docs/roadmap.md`, profile policies/ADR, and `docs/development/release-1-validation.md`. If AI-Docs-Hub is connected, check context freshness relative to HEAD; if Hub is absent, read the repository directly.
3. Perform **existing-code inventory**: find actual public APIs, migrations, tests, and already implemented parts. Do not trust old line numbers or old SHAs. `JobStore::list_jobs`, `jobs_ffi.rs`, and Swift `JobInventory` are **read-only**, not execution.
4. Select one dependency-ready product boundary from the roadmap. Do not write new product code only for the sake of activity.

### Before Commit

- Specify target user behavior, input/output, change boundary, existing APIs being reused, fail-closed risks, and negative scenarios.
- Do not perform destructive operations on user data, live transfers, or content downloads on the main MacBook without required consent; separate product permission is not required for safe unit/mock fixtures.
- Check scripts and save a report for each: `PASS`, `FAIL`, `BLOCKED` (environment/secret/permission missing), `SKIPPED` (not applicable). `NOT RUN` does not mean `PASS`.
- Perform an independent code review and security review. If no independent verification mechanism exists, explicitly note the limitation. Do not bypass prohibitions by disabling lint or faking confirmations.

### Session Completion / Transfer

Leave a compact handoff in the user report and update the canonical validation record when its state changed. Include `HEAD/base SHA`, commits, confirmed ready behavior, exact commands and results, unverified properties and risks, the next specific acceptance step, and owner decisions. Do not present a read-only UI change as a completed Job Engine. Do not create a separate session-log document.

### When the Project Owner is Needed

Request a decision only when changing product invariants or architecture, when there is an inevitable risk of data loss, for distribution selection, for approving a release without mandatory features, for actual authorization or secrets, or for access to a machine unavailable to the agent. Ordinary project decisions, safe mocks, and refactoring according to rules do not require separate approval. If one stream cannot proceed, continue an independent stream from `docs/roadmap.md`.

**Hub Policy:** onboarding and scaffold only after dry-run and diff; preservation of existing documents. Hub checks are additional sources of evidence but do not replace native tests and published `main`.

## Computational Economy Policy for Plus

- Base recommended Codex model: **GPT-5.6 Terra**, moderate reasoning; if the user selects the model mode, adhere to this default. The model is not part of the QADREN runtime architecture.
- Break work into functional packages with specific acceptance criteria. Limit context to necessary source files. Do not reread the entire repository or run a full deep architectural analysis for every patch.
- Spend increased reasoning only on narrow security concerns, SQLite recovery, parallelism, FFI ownership, and potential data loss. Upgrading to GPT-6 Astra or another expensive mode requires separate owner approval. Do not escalate automatically.
- Do not reduce mandatory tests, code/security reviews, or fail-closed checks to save tokens. Instead, reuse verification results with the same SHA when they are truly applicable.
- Model configuration and actual limit usage are managed by the Codex interface/settings, not `AGENTS.md`. The agent follows a best-effort policy and does not claim to have switched the model without the real ability to do so.
