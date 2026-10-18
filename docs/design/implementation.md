# QADREN — UI Integration Plan for Codex and Acceptance

> **Current baseline / D-018:** `DesignSystem.swift`, `DashboardLayout.swift`, `DashboardLayoutTests.swift`, `MainView.swift`, `SettingsPanel.swift`, and `QADRENApp.swift` already implement the widget canvas, preservation of order, visibility, density, grid scale, Undo/Redo, multi-window support, and settings. The UI-A–F sequence below is a **verification and completion checklist**, not an instruction to recreate these classes. The independent service is later autonomy work. Yandex Disk is post-release; iCloud is out of scope. [Current State Audit](../development/current-state-audit.md).

## 0. Boundaries and Actual Baseline

This documentation is a **visual-behavioral contract**, not an order to create non-existent Core APIs. Historical baseline before the October 14 push (do not apply without verification): `QADRENApp.swift` has WindowGroup monitoring, `scenario-settings` window, and MenuBarExtra. `MainView.swift` contains dashboard status tiles, jobs list, two local metadata panels, and connections/OAuth setup; `StorageConnections.swift`, `YandexConnection.swift`, `LocalWorkspace.swift`, and `JobInventory.swift` exist. The first full Rust Job execution/independent worker service is still passing roadmap gates. **Check HEAD before implementation**; this list may become obsolete.

The UI shall not invent data pipelines, second queues, runtime history, or registries. The active controller for Release 1 is Local FS. Yandex UI and feature work are post-release; preserve existing records and tokens. iCloud connection, synchronization, and backup UI are excluded from active implementation (D-017). VoiceOver support is out of scope. Keyboard access and accurate SwiftUI accessibility metadata remain required. All changes occur in `main`. Do not create feature or documentation branches. Completion of documentation does not prove service readiness.

### Baseline Verification

- If a JobStore/Registry read error occurs, monitoring shall show a priority available warning and the error state of the corresponding widget; the error shall not be presented as a quiet empty/success state.
- If the saved layout is corrupted, `DashboardLayoutStore` shall show a safe default panel, shall not erase original bytes, and shall report this to monitoring; the notification shall disappear only after the user intentionally saves a new layout.
- The root view shall remove implicit animations when macOS enables Reduce Motion. Widget borders shall use a two point high-contrast treatment when macOS enables Increase Contrast.
- The user can hide all widgets: the canvas shall then show an available empty state with exact instructions to restore widgets via layout mode.
- In Release 1, Settings shall offer to create only a Local FS connection. An existing Yandex connection can be renamed, disabled, or locally deleted while preserving the rule "cloud objects are not touched," but the UI shall not offer OAuth or API calls without a new post-release solution.

## 1. Current Code Mapping Matrix

| Current File/Part | Target Role | To Do / NOT To Do |
|---|---|---|
| `platform/macos/QADREN/QADRENApp.swift` | monitoring/settings scenes, menu bar | preserve entrypoints, add panel identity/session; do not link Worker lifecycle to `Window` |
| `platform/macos/QADREN/MainView.swift` | composition root | gradually move tokens/components/widgets; do not leave "dashboard/jobs/files" switch as the primary UX |
| `StorageConnections.swift` + `YandexConnection.swift` | actual connection metadata + OAuth | adapt to Settings and Connection widget; do not rewrite Keychain/Registry |
| `JobInventory.swift` | read-only durable Job data + supported commands | do not replace mock-job with successful live transfer |
| `LocalWorkspace.swift` | local FS metadata and browse | open files via expanded workspace/separate window; do not use sneak-content-preview |
| Rust `jobs.rs`, `jobs_ffi.rs`, `model.rs`, `scenario_planner.rs`, `confirmation.rs` | source of truth | ViewModels subscribe to commands and durable state; Swift does not calculate policy and job outcomes |

Suggested SwiftUI type names (`QDTheme`, `QDWidgetFrame`, `QDLayoutStore`) are provided as decomposition directions; if existing types are suitable, **reuse** them.

## 2. Vertical Sequence (Not New Large Epics)

**UI-A — Design tokens and logo (PARTIAL IN MAIN; audit/finish).** Verify existing QDTheme/QDLogo and, if necessary, link existing `assets/qadren-logo.png` in Assets.xcassets/target without changing pixels; build one SwiftUI semantic theme; system light/dark variants and CSS/demo are independent. SwiftUI snapshot-previews for buttons, status, progress, and mini-widgets; perform contrast check.

**UI-B — Chrome and dashboard (PARTIAL IN MAIN; audit/finish).** Verify existing reusable `QDWidgetFrame`, standard headers, and states; hardcoded `PreviewFixtures` shall be in `#Preview`/tests only, not in runtime. Replace permanent section navigation with a single dashboard grid. Actual data providers shall display read-only/unsupported for now.

**UI-C — Preservation and multiwindow (PARTIAL IN MAIN; audit/finish).** Existing DashboardLayoutStore, Dashboard `panelID`, type registry, grid sizing+reflow, layout editing with catalog, config/Undo/Redo, independent window state/zoom/density/filters, persist+restore. No two second Job runners shall operate simultaneously.

**UI-D — Real data and safety.** Connect active Jobs, connections, events, and Worker state via existing Swift/FFI models with real evidence/freshness. If the API is absent, show an honest `not available`. Commands shall execute only after capability check/Core policy. Use Consent/UNKNOWN/conflict from Rust contracts; no blind retries.

**UI-E — Settings and inspectors.** Settings panel with sections (connection/scenario/schedules/policies/node/UI), deep links from widgets, native forms, and recoverable connection removal. The Scenario editor is the only user configuration surface for endpoints and roots. Do not implement unfinished operations "for aesthetics."

**UI-F — Life cycle.** Connect the independent service only after the local execution path is verified. `MenuBarExtra` shall show real state. Closing windows shall not stop the Worker. Implement safe service stop. Verify restart, sleep, crash recovery, and absence of auto-open windows.

Each stage is one cohesive verifiable patch. There is no need to stop independent Core work on G1–G4: UI-B/C can be implemented with Preview fixtures and real read-only FFI.

## 3. File Structure (Recommended, Not Mandatory)

- `platform/macos/QADREN/DesignSystem/Theme.swift`, `Tokens.swift`, `Controls.swift`, `WidgetChrome.swift`
- `platform/macos/QADREN/Dashboard/MonitoringWorkspace.swift`, `WidgetRegistry.swift`, `LayoutStore.swift`, `Widgets/*.swift`
- `platform/macos/QADREN/Settings/SettingsWorkspace.swift`, `Connections*.swift`, `Scenarios*.swift`
- `platform/macos/QADREN/Inspectors/*.swift`

Do not create empty directories or a separate file for every icon; the actual structure is determined by readability and minimal dependencies. The `scripts/build-macos.sh` build may have an explicit list of Swift files — **update the manifest/script**; do not assume Xcode finds new files automatically.

## 4. Mandatory Security UX Invariants

- Do not interrupt a configured/requested file read with a download or File Provider hydration confirmation. Such reads may transparently materialize content; do not prefetch unrelated files. Browse/list/stat metadata views remain content-free unless an actual file read is required.
- The UI shall not confirm verified success without corresponding Rust evidence. UNKNOWN shall not have unconditional retry. Success of a Yandex API request is not unconditional proof of full delivery and integrity.
- Distinguish storages/jobs with the same display name by actual connection_id/Job ID.
- Do not delete user files via remove connection. Perform local credential cleanup with recoverability.
- No secrets, auth URLs, access tokens, or file content in diagnostics, screenshots/previews, or analytics.
- No preview/demo indicator shall enter runtime. Use a separate flag/fixture mode for UI tests only. The `ui-test-fixtures` Core feature is for native UNKNOWN acceptance only and is absent from the normal macOS build.

## 5. Pre-publication Verification

### Deterministic Checks

1. `python3 -m json.tool docs/design/tokens.json >/dev/null`; all Markdown links and logo assets must exist; JSON shall not contain secrets.
2. `bash scripts/test-core.sh` (if changes are Swift-only, record the check status; do not present it as a UI test).
3. `bash scripts/build-macos.sh` on a supported Mac + relevant Swift tests. If macOS is unavailable, mark as `BLOCKED`, not `PASS`.
4. Visual review of screenshots in `dark` and `light` at 1180×760, 900×620, settings 980×700, and 760×560; ensure no overflow or broken layout. Do not count an HTML prototype as a native SwiftUI screenshot.
5. `Reduce Motion`, `Increase Contrast`, and keyboard navigation; verify focus state, full path tooltips, and disabled features. VoiceOver support is out of scope.

### Visual Development and Skill Use

Use this procedure for each macOS UI patch.

1. Read the matching `docs/design/` contract and inspect the existing SwiftUI implementation. Use Codebase Memory first when it is available.
2. Use `ui-ux-pro-max` for UX hierarchy, state clarity, layout, contrast, keyboard use, and native-platform review. Its output is advisory. This design system remains the source of truth.
3. Use `swiftui-expert-skill` for SwiftUI implementation, native macOS layout, state flow, accessibility metadata, and test choices.
4. Use `design-system` only when a QADREN token or component-state contract needs a structured review. Do not generate CSS or Tailwind files for the native application. Do not use `ui-styling` for SwiftUI runtime code. The installed `design`, `brand`, `banner-design`, and `slides` skills are for separate brand or communication artifacts. They are not part of the macOS application workflow.
5. Build the app. Capture a real QADREN window with Peekaboo. Keep captures in a temporary directory outside Git. Review the required dark and light sizes, compact layouts, states, long paths, keyboard focus, and displayed-data truthfulness against this document.
6. Record concrete defects. Implement one bounded improvement. Rebuild and capture the affected state again. Run the applicable native and Core tests.

Use `peekaboo permissions` before the first capture in a session. Prefer a configured Peekaboo MCP tool when Codex exposes it. Otherwise, use the approved Peekaboo CLI. If Peekaboo cannot return a real screenshot, record visual acceptance as `BLOCKED`; an Accessibility tree, an HTML prototype, or a source review does not replace a screenshot.

The minimal native UI skill set is `ui-ux-pro-max` and `swiftui-expert-skill`. `design-system` is conditional. Keep the other installed skills unchanged unless a separate task proves that they are obsolete and have no dependency.

### Recorded Visual Audit — 2026-10-18

This audit used real QADREN windows and Peekaboo CLI on commit `32cc794`. The captures are local temporary files. They are not Git artifacts. The accessibility tree was available. Peekaboo MCP tools were not exposed in the Codex session.

| Priority | Finding or validation gap | Evidence | Disposition |
|---|---|---|---|
| P1 | A `failed` widget used the neutral frame. This did not meet the warning/error border contract. | `QDWidgetFrame` previously gave an attention border only to `needsAction`. | Fixed in `32cc794`. `failed` and `needsAction` now use their semantic border. `DashboardLayoutTests` checks the rule. |
| P2 | The macOS settings window did not accept a 760×560 frame during the audit. It stopped at 760×612. | The real 760×612 window had no overflow or clipped text. | This is a test-size coverage gap, not a confirmed layout defect. Re-check 760×560 when the window can use that size. |
| P2 | Fresh native evidence for keyboard traversal, Reduce Motion, and Increase Contrast was not recorded in this audit. | The visual captures do not prove these interaction states. | Keep this as an acceptance check for the next UI patch. Do not report it as passed from source review. |

The review captured the dashboard in dark 1180×760, dark 900×620, and light 1180×760. It captured Settings in dark and light 980×700 and in light 760×612. The reviewed states had no detected clipping, overflow, permanent dashboard sidebar, fabricated success state, or inconsistent widget grid reflow. No P0 finding was detected.

GitHub Issue review on 2026-10-18 found that Issue #26, the native UI acceptance issue, was closed. Open Issues #15 and #18 through #25 concern Local Filesystem execution, safety, Worker, Scheduler, and release evidence. They do not authorize a second dashboard or a replacement design-system implementation.

### Feature and Negative-State Matrix

| Case | Expected |
|---|---|
| Fresh install, no storage | honest empty state; link to settings, not "all synchronized" |
| 2 Yandex connections | separate rows/credentials, no mixed quotas |
| Job running and 1000 queued | lazy list; progress based on real data only; stable scroll/selection |
| UNKNOWN after crash | automatically attempt read-only reconciliation after restart; show the verified result or a truthful persistent UNKNOWN state if inconclusive, without blind mutation retry |
| iCloud out of scope | iCloud is absent from the wizard and active actions; no connection buttons, entitlement probes, or fake quotas. If legacy data contains an iCloud connection, display as disabled/unsupported without I/O |
| Auth revoked / no network | stale timestamp + reason; not green "connected" |
| Global filter excludes critical job | global attention remains visible |
| Edit grid, resize, undo, close/open | deterministic layout and persistence, no overlap |
| Two independent dashboards | independent local layouts, one execution state |
| Requested file content read / File Provider hydration | no extra prompt; transparently read only files needed by the selected or configured action |
| Destructive replacement / mass deletion | show precise destination or file set and require informed confirmation unless a matching approved scenario rule covers replacement; mass deletion always gets an explicit prompt |
| Operation exceeds selected files | disclose actual possible scope, do not falsely claim selective action; require approval before broader destructive effect |
| Close all UI while service running | after G5, Worker continues execution; before G5, show honest limitation |
| System sleeps/timezone changes | UI reflects real schedule; does not calculate DST via surrogates |

## 6. Readiness and Reporting

For each UI validation record `HEAD/base SHA`, list components, actual APIs, screenshots or preview snapshots, `PASS/FAIL/BLOCKED/SKIPPED`, commit SHA, known residual risks, and owner decisions. Perform code and security review. AI-Docs-Hub is only an additional project-scoped index and review. It is not the source of UI architecture.

**DONE Criterion for the design system in code:** not simply a similar screenshot. The theme is applied via one token layer, every widget uses a common frame/states, layout/filters/session are preserved, real data is not substituted, dangerous actions without Core authorization are unavailable, and accessibility and native build are actually passed.

## 7. What Codex Shall Not Do

Do not draw a new logo. Do not return a standard dashboard with a permanent Sidebar. Do not add a universal system of arbitrary HTML widgets, an embedded browser, WebView, or a separate frontend runtime. Do not rewrite Rust for decoration. Do not create an additional documentation branch/duplicate skill. Do not "execute" actions by simply changing SwiftUI state without durable Core confirmation. Do not turn attractive demo diagrams into product indicators.
