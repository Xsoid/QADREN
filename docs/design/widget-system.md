# QADREN — widget system and layout

## 1. Product Contract

Monitoring is **not a list of sections**. It is a local dashboard window with a set of standard interactive widgets. Release 1 has one monitoring window with one local layout, as [monitoring](monitoring.md) specifies. Several independent dashboard windows are **Deferred**. The panel model below keeps `panelID` so that a later decision can add more windows without a layout migration. There is no permanent Sidebar, no right-side palette, and no permanently visible catalog. The user configures the panel, and then only observes and performs contextual actions. General filters are available from a compact header popover. Problems are displayed independently of filters.

One panel can hold several instances of one `typeID` (owner decision, 2026-10-31). Each instance has its own `widgetID`, size, and density. The user adds instances from the widget catalog and removes them in Layout mode. Removing an instance changes only the layout. It does not change Jobs, scenarios, connections, or their history.

Each widget type declares the densities that it draws (`supportedDensities`). A type with one density has no density menu. The former `active-jobs` widget (a raw JobStore list) is removed (owner decision, 2026-10-31); a saved layout drops it and keeps its other widgets. The monitoring panel ID is an app preference, not window state, so a forced quit keeps the layout.

Each widget type is a separate SwiftUI view file in `platform/macos/QADREN/Widgets/`. `MonitoringWidgetKind` is the type registry, and `MonitoringWidgetView` selects the view for one instance. To add a type, add a `MonitoringWidgetKind` case, a view file, and one registry line.

## 2. Saved Representation Model

Proposed view-state contract (not Core domain schema):

| Field | Type / Meaning |
|---|---|
| `panelID` | stable UUID for a separate window/panel |
| `schemaVersion` | layout version for migration; must be independent of scenarios |
| `title` | user name for the panel, for example, "Main" |
| `windowFrame` | window placement/zoom; saved separately from Job/SQLite Core |
| `theme` | system/dark/light (may inherit global setting) |
| `panelFilters` | selected connections, scenarios, worker; `all` default |
| `widgetID`, `typeID` | instance ID, standard widget type |
| `gridRect` | integer `column,row,width,height`, span in cells |
| `density` | compact/regular/expanded, for a **specific** widget |
| `localFilters` | inherit or explicitly defined overrides |
| `widgetOptions` | only specific parameterized settings for the type; `scenario` stores `scenarioID`; `scenarios` stores `limit` (rows: 3, 5, 8, 12, or all) |

Persist locally and atomically. If the layout is corrupted, open a safe default panel **without erasing the user configuration**; provide migration/recovery and diagnostics. Do not store OAuth secrets or file contents. Session restore restores windows, sizes, selected context, scroll, and selection where possible; it **does not open windows automatically on alarm**. When the panel closes, the Worker continues to operate according to its own lifecycle (after the background service is implemented).

## 3. Geometry

Wide window: base grid of **12 columns** for client widths from 1180pt. Narrow window: **8 columns** for 900–1179pt. For other sizes/zoom, recalculate; do not fix the pixel width of the widget. External margins are 12pt, gap is 10pt, with no overlaps and no negative coordinates.

- `widthSpan` is an integer from 2..12 (or less than the available number of columns on a narrow grid); height is determined by the integer grid or content with an agreed min height, without clipping commands.
- Initial typical spans for **wide**: SystemHealth 3×2, Scenario 4×2, Connections 4×3, ResourceMetrics 4×3, UpcomingRuns 4×2, Timeline 4×3, Scenarios 4×3; this is an **example**, not a mandatory order.
- For a narrow screen, first adapt spans, then move widgets down; never overlap, create horizontal scroll for the whole panel, or clip mandatory actions.
- The layout engine searches for the nearest free area and performs deterministic reflow; the tab-navigation order matches the visual reading order.

## 4. Size ≠ Density

**Size** determines the allocated area on the canvas and the number of visible elements; **density** determines internal margins, row height, and the volume of secondary information. Do not change text with a forced 0.7× scale; density is not a global zoom. `compact` shows status + main number/2 lines; `regular` shows 3–5 lines/main actions; `expanded` shows details, tree, and contextual actions if the width is sufficient. If space is unavailable, hide secondary information by priority, but do not hide dangerous statuses.

Example for a Scenario: compact shows the stage and the progress ring, regular adds the file counts and Run or Stop, expanded adds the source and the destination; changing size does not change Core state. For resources: compact shows 1 summary, expanded shows several separate metrics; do not insert fake sparklines.

## 5. Observation Mode and Layout Mode

**Observation** (default): widgets are locked; actions are available within them; no drag handles; zoom/filter context and details work without transitions.

**Layout** (explicitly enabled in the top bar of the window, `MonitoringHeader`): the top bar shows `+ Виджет` and `Готово`. `Add widget` opens the widget catalog; a click on a type adds a new instance at the end of the panel. Each widget shows move controls, density choices, and a remove control. Auto-save every valid change; Undo/Redo use Cmd-Z/Shift-Cmd-Z without toolbar buttons; optional layout recovery, but not without warning upon layout loss. These changes do not delete jobs, scenarios, connections, or their history. A panel with no widgets shows an empty state that tells how to add a widget.

The monitoring top bar (`MonitoringHeader`) has only the layout controls. The panel has no status footer: Worker state lives in the `system-health` widget. If that widget is not on the panel, a Worker service failure shows in the panel attention banner. Data refreshes on its own interval, and the Settings window opens from the QADREN menu bar menu.

Order: select → move → choose density → validate bounds/unique `widgetID` → persist view state. If save errors occur, show a message and the previous layout; do not report success. Future widget types remain unavailable until an approved product decision adds them.

## 6. Contextual Links

Global filters (connections, scenarios, worker/time window — only where applicable) apply to linked widgets; each widget can use `inherit` or `override`. When an object is selected from one widget, other widgets with `followSelection` enabled update without resetting scroll/selection. Back/forward history and breadcrumbs preserve a clear navigation context. **System critical signals and required consents are visible outside these filters**, even if the corresponding widget type is deleted (global status indication). There is no separate mandatory "confirmation center."

Metadata reads use only the permitted source. They do not start hidden content reads. The UI shows `lastFetched` and `lastVerified` as separate fields where applicable.

## 7. Standard Types Registry

| typeID | Compact | Expanded / Operations | Source / Constraint |
|---|---|---|---|
| `system-health` | one fixed size: Worker service state, active operations, last check, app version | start, restart (the Worker first finishes the files that run now and starts no new one; the status line shows "Перезапуск" meanwhile), open Login Items | launchd registration + IPC `status`; attention state tints the status zone and border |
| `scenario` | one selected Scenario: stage, three-zone progress ring (copied, remaining, errors) | regular: copied/remaining/error counts, next run when idle, Stop during a run, Run when idle | Registry `scenario_runs` + batch summary; expanded adds the source and destination (connection, type, root, selected objects); Run uses Worker `run_now` (a manual run that plans and executes like a scheduled run, D-025); failed files never color the frame, only a failed run does; a file that waits for a retry after a failed transfer (for example Yandex Disk HTTP 500) is in the error zone until a retry copies it, and regular shows "Ждут повтора: N" with the next retry time; a file that still fails after all retries is planned again by the next run |
| `scenarios` | one fixed size: all Scenarios in the order of the nearest run (running, waiting for a write root, by next scheduled run, manual, paused, disabled); each row: marker, name, direction (source → destination; ⇄ for two-way), first schedule rule (+N), next run time with today/tomorrow/date, or "идёт", "ждёт", "вручную", "пауза", "выкл" | Layout mode sets the row count; a running row shows `QDCyberActivityBar`; a failed last run colors only its marker, never the frame | Registry: Scenarios, schedules (`next_at`), run state, local automation pause; read every 5 s |
| `connections` | compact: status line ("В сети", "Нужен вход", "Ошибка", "Нет связи"), online count N/M of enabled connections, one cut marker per connection | regular: up to 4 rows (marker, name, type; a status word only when the state is not ready) + "Ещё N"; expanded: all rows and the SMB share address (no login) | local Registry; a switched-off or empty registry keeps the frame quiet; a connection that needs sign-in or failed colors the frame |

The `job-queue` widget was removed on 2026-10-31 (owner decision). Its counts repeated other widgets: a Scenario has at most one active run with one batch at a time, so "active" was the number of copying Scenarios and "waiting" was almost always 0. Its status came from all Job history, so old failed Jobs colored it red permanently. Each signal goes to the widget that it belongs to: Worker service state to `system-health`, file outcomes (including `UNKNOWN`) to the `scenario` error zone. A saved layout drops a `queue` widget when it loads.
| `resource-metrics` | CPU/RAM/disk for specific Worker | measurable metrics with timestamp | do not create history until telemetry exists |
| `upcoming-runs` | 1–3 future trigger times | timezone, enable/schedule context | `Scenario` scheduling only after implementation |
| `event-timeline` | latest events | filters, context, safe links | no content/secret logging |
| `scenarios` | enabled/disabled + next run | scenario summary and transition to settings | Core config changes only in settings |
| `worker-state` | local Worker state/paused | local diagnostics/commands | remote Worker UI later |
| `file-changes` | number of changes if known | chart ONLY if actual time-series | optional widget, may be unsupported |
| `connection-quota` | used/available if API knows | quota details per selected connection | missing quota → "no data" |

Do not include the following in MVP: topology graph, global Worker speed, quota analytics for previous periods without data, backup file versions, backup set, permanent search through files.

## 8. Default Layout

First start panel: `system-health`, `scenario`, `scenarios`, `connections` in catalog order. No catch-all attention widget (owner decision, 2026-10-31): each signal stays in the widget that it belongs to. Worker service state is in `system-health`, file outcomes (including `UNKNOWN` and retries) are in `scenario`, sign-in and connection failures are in `connections`. The panel attention banner shows only a Worker service failure when no `system-health` widget is on the panel, a store read error, or a recovered layout. `upcoming-runs` and `event-timeline` stay planned. Widgets have an explicit `empty/unavailable` state if Core is not yet implemented. The primary layout must survive resizing while preserving user rearrangements.

## 9. Complex and Negative Cases

- 0 connections / 0 jobs → meaningful empty state, button to go to "Settings Panel", not fake success.
- 1000+ jobs → lazy rows/virtualization, stable IDs, bounded refresh; 0 memory leaks from monitoring.
- No network, auth expired, rate-limit → reason and data freshness, not "connected" via old cache.
- Job UNKNOWN, conflict, deleting connection with attempts → block dangerous commands, display reconciliation context.
- Window restart, resolution change, light/dark, zoom, and Reduce Motion → layout remains readable, without overlaps and without loss of commands. VoiceOver support is out of scope.
