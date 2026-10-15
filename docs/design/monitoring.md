# QADREN — monitoring window (Control Surface)

## 1. Purpose and entry point

The main window is a **working canvas of widgets**. The user configures this canvas one time and then monitors it without transitions between sections. It is NOT separate pages for "Overview/Queue/Files/History" and it does NOT have a permanent Sidebar.

Existing entry points in `QADRENApp.swift`: `WindowGroup("QADREN", id: "monitoring")`, MenuBarExtra → "Monitoring Panel" / "New Working Panel". Maintain support for multiple windows with independent `panelID` and configurations. When you open the window again, do not create random duplicates by default: the "Open" action activates the existing main panel, and "New" adds a separate panel with a new ID. Select the technical method to manage SwiftUI instances after the inventory.

## 2. Three horizontal zones

1. **Native titlebar/toolbar**: traffic lights, actual logo asset (20–24pt) + panel name, compact system health, global filter popovers, layout button, and context actions. Do not insert an 80px "banner".
2. **Canvas**: scrollable adaptive grid without a sidebar. All widget types reside here. Navigation is handled via context within a widget, an inspector, or a temporary separate window.
3. **Status bar**: one line containing the local Worker, service status, last update, and the number of unresolved critical issues. Use values with provenance/freshness. Do not simulate "running in background" until the lifecycle is complete.

### Composition scheme (structure, not pixel dimensions)

| 12-column wide (conditional areas) | Content |
|---|---|
| First row: 3 + 5 + 2 + 2 | System · Active Jobs · Queue · Needs Attention |
| Second row: 4 + 4 + 4 | Connections · Worker Resources · Upcoming Runs |
| Third row: 4 + 4 + 4 | Events · Scenarios · Optional File Activity |
| Bottom narrow line | Worker · service · update · global problems |

This is the **default user layout without a saved configuration**, not a permanently fixed scheme. The user can remove or change sizes and repeat a widget type only in a different dashboard. Use deterministic reflow for narrow windows; do not use horizontal scrolling for the entire canvas. Do not draw "file activity" without real aggregates.

## 3. Visual hierarchy

Priority from first to last: (1) blocking consent/conflict/UNKNOWN and errors; (2) current operations and progress; (3) connection availability; (4) queue and future starts; (5) history and diagnostic metrics. Do not turn "SUCCESS" into a huge green card. The panel can be completely quiet when there is zero activity.

**Cyberpunk** style is implemented with thin lines/cyan focus, a neon accent edge for the selected/active state, neat markers, and technical precision; the graphic background does not interfere with readability. When a widget is focused, adjacent widgets do not become dim automatically.

## 4. Interactions — without screen switching

- Click on Job row → `JobInspector` overlay or a separate window with Job ID, scenario/revision, endpoints, status, file tree/progress, attempts/evidence, and available commands.
- Click on Connection → `ConnectionInspector` with capabilities, quota freshness, and operations; to edit the profile, use the explicit "Configure" button to open the settings window **in the correct context**.
- Click on conflict/needs consent → context panel with consequences and a resolution. The interface is read-only and the action is disabled until the real Core Policy is available.
- Click on Scenario → brief information and status of current jobs; the "Configure Scenario" button opens settings for a specific ID.
- Global filter popover → connection/scenario/worker. Selecting an event sets a shared context only for widgets subscribed to it. Each widget has a local "Override" + "Return to Shared".
- The "+ Widget" button is available only in layout mode; the catalog is a temporary drawer/popover and closes after adding a widget or exiting. In monitoring mode, top actions are only Filter, Refresh, Layout, Window, and current health.

## 5. Behavior during risks

**Attention widget:** The card contains a number and the most dangerous problem. If the widget is absent, the global indicator in the title/statusbar remains. A critical notification does not disappear when you filter for a different Worker.

**UNKNOWN:** Use a separate "Result unknown" label. Detailed read-only reconciliation is available; do not provide a "Retry" button without information that the operation did not occur.

**Content download consent:** Show the source, the destination "main MacBook", files/set, volume (if known), the reason for the download, and the scope of rights. The default is "Deny"; lack of reaction does not confirm the download. Do not consider a system iCloud download as part of the QADREN consent.

**Dangerous deletion/overwrite:** Show the full path, counters, and consequences. Do not provide a mass delete button without a policy. Separately show the use of the provider's trash if the API confirms it.

## 6. First visual options widget-by-widget

| Widget | Compact | Regular | Expanded |
|---|---|---|---|
| System | status + source/lastSeen | service status + Worker + last refresh | modules + provenance |
| ActiveJobs | count and 1 current | 2–3 operations, progress/files/bytes | lazy list and current file tree |
| Queue | active/waiting/unknown counters | 4–6 rows, badges | selection, priority, pause/resume |
| Attention | count + severity | top conflict + "More details" action | resolutions, errors, list of pending confirmations |
| Connections | 1 selected + availability | 2–4 rows quota/status | paths, quotas/limits, adapter capabilities |
| Resources | 2–3 actual metrics | CPU/RAM/disk + timestamp | telemetry, only if measured |
| UpcomingRuns | 1 nearest run | 3–5 schedule rows | timezone, trigger status |
| Timeline | last event | last 4–8 | lazy timeline with context filters |

Do not degrade critical status indication in compact mode; if the full text does not fit, leave the icon + available name and put details in the inspector.

The Queue derives its status and active/waiting/unknown counters only from the
read-only JobStore inventory. If that inventory cannot be read, it shows an
unavailable state instead of zero counters. Terminal history, failed Jobs, and
Jobs awaiting a decision are not presented as active or queued work.

## 7. Mode specifications

**Monitoring:** layout fixed; data may refresh; view interactions active; no edit grid.
**Editing:** dashed grid, snap, move/resize handles, catalog searchable by widget name, remove/undo, save-after-each-valid-step. Entering this mode does not stop Jobs and does not change scenarios.
**Expanded widget:** expansion within the working area or in a separate window; the state of the original panel and scroll is preserved. Focus returns to the original widget upon return.
**Offline/read-only:** stale data is marked, no live claim; unavailable commands are disabled.

## 8. Control from macOS MenuBarExtra

The menu must quickly report health and actual values for active/waiting/attention. It must provide options to open/create the panel and configure it, pause the Worker (if Core allows), and explicitly stop the service with a warning. **Exiting the UI is not equal to stopping the service** in the target architecture; until the service is moved to a separate process, show the actual behavior and do not promise continuous execution. Do not present `NSApp.terminate` as a service stop.

## 9. UI acceptance

- 1180×760 and 900×620: no sidebar, no overflow/overlay, and no loss of critical indicators.
- Two windows: different layouts/filters, one shared Core job truth, no duplicate executions.
- Monitoring/edit switch: no dirty layout loss, no widget duplicates in one panel, undo/redo and recovery.
- 0 jobs, many jobs, auth failure, offline, unknown, consent, conflict: correct semantics, no false numbers/ETA.
- No focus loss on refresh; keyboard, VoiceOver, contrast, and Reduced Motion are verified.
