# QADREN — monitoring window (Control Surface)

## 1. Purpose and entry point

The main window is a **working canvas of widgets**. The user configures this canvas one time and then monitors it without transitions between sections. It is NOT separate pages for "Overview/Queue/Files/History" and it does NOT have a permanent Sidebar.

Existing entry points in `QADRENApp.swift`: one `Window("Контрольная панель", id: "monitoring")` and one Settings window. The menu can open each window. There is no separate file-manager window.

## 2. Three horizontal zones

1. **Top bar** (`MonitoringHeader`, 52 pt, see [visual language §0](visual-language.md)): system traffic lights, actual logo asset (24 pt) + "Q.A.D.R.E.N. // Контрольная панель", layout button, and Layout-mode actions. Do not insert an 80px "banner".
2. **Canvas**: scrollable adaptive grid without a sidebar. All widget types reside here. Navigation is handled via context within a widget, an inspector, or a temporary separate window.
3. **Status bar**: one line containing the local Worker, service status, last update, and the number of unresolved critical conditions. Use values with provenance and freshness. Do not simulate "running in background" until the lifecycle is complete.

The Worker value comes only from the IPC `status` request to the local Worker socket. A registered service or the in-process Core library is not proof that the Worker responds. The panel reads the JobStore inventory and sends the `status` request every 5 seconds while the monitoring window is open. Both reads are non-mutating. If no inventory read succeeds for three intervals, the status bar marks the data as stale and shows the time of the last good read.

### Composition scheme (structure, not pixel dimensions)

| 12-column wide (conditional areas) | Content |
|---|---|
| First row: 3 + 5 + 4 | System · Scenario · Needs Attention |
| Second row: 4 + 4 + 4 | Connections · Worker Resources · Upcoming Runs |
| Third row: 4 + 4 + 4 | Events · Scenarios · Optional File Activity |
| Bottom narrow line | Worker · service · update · global problems |

This is the **default user layout without a saved configuration**. The fixed widget set supports order and density changes only. Use deterministic reflow for narrow windows; do not use horizontal scrolling for the entire canvas. Do not draw "file activity" without real aggregates.

## 3. Visual hierarchy

Priority from first to last: (1) blocking consent/conflict/UNKNOWN and errors; (2) current operations and progress; (3) connection availability; (4) queue and future starts; (5) history and diagnostic metrics. Do not turn "SUCCESS" into a huge green card. The panel can be completely quiet when there is zero activity.

**Cyberpunk** style follows [visual language §0](visual-language.md): cut-corner frames with a thin edge, a yellow accent, a mint edge on actions, glowing status lines, neat markers, and technical precision; the graphic background does not interfere with readability. When a widget is focused, adjacent widgets do not become dim automatically.

## 4. Interactions — without screen switching

- Click on Job row → `JobInspector` overlay or a separate window with Job ID, scenario/revision, endpoints, status, file tree/progress, attempts/evidence, and available commands.
- Click on Connection → `ConnectionInspector` with capabilities, quota freshness, and operations; to edit the profile, use the explicit "Configure" button to open the settings window **in the correct context**.
- Click on conflict/needs consent → context panel with consequences and a resolution. The interface is read-only and the action is disabled until the real Core Policy is available.
- Click on Scenario → brief information and status of current jobs; the "Configure Scenario" button opens settings for a specific ID.
- Global filter popover → connection/scenario/worker. Selecting an event sets a shared context only for widgets subscribed to it. Each widget has a local "Override" + "Return to Shared".
- In monitoring mode, top actions are only Filter, Layout, Window, and current health.

## 5. Behavior during risks

**Attention widget:** The card contains a number and the most dangerous problem. If the widget is absent, the global indicator in the title/statusbar remains. A critical notification does not disappear when you filter for a different Worker.

**UNKNOWN:** Use a separate "Result unknown" label. Detailed read-only reconciliation is available; do not provide a "Retry" button without information that the operation did not occur.

**Requested content read:** Read only the source files required by the configured or user-requested operation. Do not show a separate content-read or File Provider materialization prompt. macOS can materialize a provider item as part of that read. Do not prefetch unrelated content.

**Dangerous deletion/overwrite:** Show the full path, counters, and consequences. Do not provide a mass delete button without a policy. Separately show the use of the provider's trash if the API confirms it.

## 6. First visual options widget-by-widget

| Widget | Compact | Regular | Expanded |
|---|---|---|---|
| System | status + source/lastSeen | service status + Worker + last refresh | modules + provenance |
| Scenario | stage + progress ring of one selected Scenario | file counts, next run, Stop or Run | source and destination: connection, type, root, selected objects |
| Attention | count + severity | top conflict + "More details" action | resolutions, errors, list of pending confirmations |
| Scenarios | all Scenarios by nearest run: direction, schedule, next run | (one size; Layout mode sets the row count) | — |
| Connections | status line + online N/M + markers | up to 4 rows: marker, name, type, a status word only on a deviation | all rows + SMB share address |
| Resources | 2–3 actual metrics | CPU/RAM/disk + timestamp | telemetry, only if measured |
| UpcomingRuns | 1 nearest run | 3–5 schedule rows | timezone, trigger status |
| Timeline | last event | last 4–8 | lazy timeline with context filters |

Do not degrade critical status indication in compact mode; if the full text does not fit, leave the icon + available name and put details in the inspector.

The panel has no Queue widget (removed on 2026-10-31, see
`widget-system.md`). A widget never derives its status from terminal Job
history; old failed Jobs do not color a widget.

## 7. Mode specifications

**Monitoring:** layout fixed; data may refresh; view interactions active; no edit grid.
**Editing:** `Add widget` catalog, move controls, density choices, remove control, keyboard Undo/Redo, and save after each valid step. Entering this mode does not stop Jobs and does not change scenarios.
**Expanded widget:** expansion within the working area or in a separate window; the state of the original panel and scroll is preserved. Focus returns to the original widget upon return.
**Offline/read-only:** stale data is marked, no live claim; unavailable commands are disabled.

## 8. Control from macOS MenuBarExtra

The menu must quickly report health and actual values for active/waiting/attention. It must provide options to open/create the panel and configure it, pause the Worker (if Core allows), and explicitly stop the service with a warning. **Exiting the UI is not equal to stopping the service** in the target architecture; until the service is moved to a separate process, show the actual behavior and do not promise continuous execution. Do not present `NSApp.terminate` as a service stop.

## 9. UI acceptance

- 1180×760 and 900×620: no sidebar, no overflow/overlay, and no loss of critical indicators.
- One monitoring window: one local layout and one shared Core job truth, with no duplicate executions.
- Monitoring/edit switch: no dirty layout loss, add and remove widget instances (several instances of one type are permitted), undo/redo and recovery.
- 0 jobs, many jobs, auth failure, offline, unknown, consent, conflict: correct semantics, no false numbers/ETA.
- No focus loss on refresh; keyboard, contrast, and Reduced Motion are verified. VoiceOver support is out of scope.
