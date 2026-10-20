# QADREN — widget system and layout

## 1. Product Contract

Monitoring is **not a list of sections**, but several independent local dashboard windows. Each window contains a set of standard interactive widgets. There is no permanent Sidebar, no right-side palette, and no permanently visible catalog. The user configures the panel, and then only observes and performs contextual actions. General filters are available from a compact header popover. Problems are displayed independently of filters.

Do not duplicate an instance of one `typeID` **within one panel**; the same type is permissible in another panel with a different configuration. Within a composite `Connection` type, you may show several connections or one selected connection—without adding a second identical widget type to the same panel.

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
| `widgetOptions` | only specific parameterized settings for the type |

Persist locally and atomically. If the layout is corrupted, open a safe default panel **without erasing the user configuration**; provide migration/recovery and diagnostics. Do not store OAuth secrets or file contents. Session restore restores windows, sizes, selected context, scroll, and selection where possible; it **does not open windows automatically on alarm**. When the panel closes, the Worker continues to operate according to its own lifecycle (after the background service is implemented).

## 3. Geometry

Wide window: base grid of **12 columns** for client widths from 1180pt. Narrow window: **8 columns** for 900–1179pt. For other sizes/zoom, recalculate; do not fix the pixel width of the widget. External margins are 12pt, gap is 10pt, with no overlaps and no negative coordinates.

- `widthSpan` is an integer from 2..12 (or less than the available number of columns on a narrow grid); height is determined by the integer grid or content with an agreed min height, without clipping commands.
- Initial typical spans for **wide**: SystemHealth 3×2, ActiveJobs 5×2, Queue 2×2, Attention 2×2, Connections 4×3, ResourceMetrics 4×3, UpcomingRuns 4×2, Timeline 4×3, Scenarios 4×3; this is an **example**, not a mandatory order.
- For a narrow screen, first adapt spans, then move widgets down; never overlap, create horizontal scroll for the whole panel, or clip mandatory actions.
- The layout engine searches for the nearest free area and performs deterministic reflow; the tab-navigation order matches the visual reading order.

## 4. Size ≠ Density

**Size** determines the allocated area on the canvas and the number of visible elements; **density** determines internal margins, row height, and the volume of secondary information. Do not change text with a forced 0.7× scale; density is not a global zoom. `compact` shows status + main number/2 lines; `regular` shows 3–5 lines/main actions; `expanded` shows details, tree, and contextual actions if the width is sufficient. If space is unavailable, hide secondary information by priority, but do not hide dangerous statuses.

Example for the queue: 2×2 — active/waiting/unknown (counters), 4×3 — several Job rows, 6×4 — detailed list; changing size does not change Core job ordering. For resources: compact shows 1 summary, expanded shows several separate metrics; do not insert fake sparklines.

## 5. Observation Mode and Layout Mode

**Observation** (default): widgets are locked; actions are available within them; no drag handles; zoom/filter context and details work without transitions.

**Layout** (explicitly enabled in toolbar): a grid line, drag handle, resize anchor, `+ Widget` button, and `Done` button appear. Only in this mode does the catalog drawer/popover open with search **by widget types**, not search for user files. Auto-save every valid change; Undo/Redo (Cmd-Z/Shift-Cmd-Z); optional layout recovery, but not without warning upon layout loss. Deleting a widget does not delete jobs, scenarios, connections, or their history.

Order: select → drag / resize → snap + safe reflow → validate bounds/type-uniqueness → persist view state. If save errors occur, show a message and the previous layout; do not report success. The catalog shows only supported widgets; for future types, show status "unavailable" with a reason.

## 6. Contextual Links

Global filters (connections, scenarios, worker/time window — only where applicable) apply to linked widgets; each widget can use `inherit` or `override`. When an object is selected from one widget, other widgets with `followSelection` enabled update without resetting scroll/selection. Back/forward history and breadcrumbs preserve a clear navigation context. **System critical signals and required consents are visible outside these filters**, even if the corresponding widget type is deleted (global status indication). There is no separate mandatory "confirmation center."

Metadata reads use only the permitted source. They do not start hidden content reads. The UI shows `lastFetched` and `lastVerified` as separate fields where applicable.

## 7. Standard Types Registry

| typeID | Compact | Expanded / Operations | Source / Constraint |
|---|---|---|---|
| `system-health` | Controller/Worker status + lastSeen | modular diagnostics, time sync | only confirmed statuses |
| `active-jobs` | current jobs + percent if known | per-job progress, pause/cancel only via Core | JobStore/FFI, not dummy runner |
| `job-queue` | counts active/waiting/paused/unknown | ordering + action menus | durable Jobs, manual-first if Core |
| `attention` | critical count + first item | conflict / consent / failure context | signal even if widget is hidden |
| `connections` | name/status/used if known | all connections or selected | per-connection, several of one provider |
| `resource-metrics` | CPU/RAM/disk for specific Worker | measurable metrics with timestamp | do not create history until telemetry exists |
| `upcoming-runs` | 1–3 future trigger times | timezone, enable/schedule context | `Scenario` scheduling only after implementation |
| `event-timeline` | latest events | filters, context, safe links | no content/secret logging |
| `scenarios` | enabled/disabled + next run | scenario summary and transition to settings | Core config changes only in settings |
| `worker-state` | local Worker state/paused | local diagnostics/commands | remote Worker UI later |
| `file-changes` | number of changes if known | chart ONLY if actual time-series | optional widget, may be unsupported |
| `connection-quota` | used/available if API knows | quota details per selected connection | missing quota → "no data" |

Do not include the following in MVP: topology graph, global Worker speed, quota analytics for previous periods without data, backup file versions, backup set, permanent search through files.

## 8. Default Layout

First start panel (if data is available): at the top, a brief system-health line and `attention`, `active-jobs`, `job-queue` widgets; below them, `connections`, `upcoming-runs`, `event-timeline`, `scenarios`. Widgets have an explicit `empty/unavailable` state if Core is not yet implemented. The primary layout must survive resizing while preserving user rearrangements.

## 9. Complex and Negative Cases

- 0 connections / 0 jobs → meaningful empty state, button to go to "Settings Panel", not fake success.
- 1000+ jobs → lazy rows/virtualization, stable IDs, bounded refresh; 0 memory leaks from monitoring.
- No network, auth expired, rate-limit → reason and data freshness, not "connected" via old cache.
- Job UNKNOWN, conflict, deleting connection with attempts → block dangerous commands, display reconciliation context.
- Window restart, resolution change, light/dark, zoom, and Reduce Motion → layout remains readable, without overlaps and without loss of commands. VoiceOver support is out of scope.
