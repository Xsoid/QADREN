# QADREN — component library (SwiftUI contract)

See [tokens](tokens.json), [widget system](widget-system.md), and [safety](../policies/data-safety.md). The names here are **target** component names. They are not a statement that the components exist in the code. Codex must find existing analogues and reuse them first.

## Layers

- Tokens/theme: `QDTheme`, `QDColorRole`, `QDTypography`, `QDSpacing`.
- Primitives: `QDIconButton`, `QDStatusLabel`, `QDBadge`, `QDMetric`, `QDProgress`, `QDPathLabel`, `QDEmptyState`, `QDNotice`, `QDConfirm`, `QDFilterChip`.
- Compose: `QDWidgetFrame` + header/body/footer, `QDDataRow`, `QDJobRow`, `QDConnectionRow`, `QDConflictRow`, `QDInspector`.
- Features: `QDMonitoringWorkspace`, `QDWidgetCatalog`, `QDLayoutEditor`, `QDSettingsWorkspace`; implementations of views for specific widgets.

SwiftUI layout/composition and Core/state bridge are separate: no visual component owns its own queue or performs file I/O.

## 1. Primitives

| Component / contract | States, variants | Behavior and constraints |
|---|---|---|
| `QDIconButton(action, symbol, label)` | default, hover, pressed, focused, disabled, busy; secondary/primary/destructive | 28×28 pt preferred; title/VoiceOver label; contextual action; disabled is not clickable; pending must not be shown as a completed command |
| `QDStatusLabel(kind, text, evidence)` | ready, active, waiting, paused, needsAction, failed, unknown, offline, unsupported | icon + text + semantic color; UNKNOWN requires reconciliation, not "error"; do not show "success" based only on indirect report |
| `QDBadge(count, severity)` | neutral/info/warning/danger/unknown | small numeric label; do not draw 0 if there is no value; severity is not determined by count |
| `QDMetric(value?, unit, freshness, provenance)` | loaded, stale, unknown, inaccessible | numbers with monospacedDigit, units; nil → "—", not a fictional 0; tooltip with source and time |
| `QDProgress(completed?, total?, verified, state)` | determinate/indeterminate/paused/unknown/completed | percentage only if total/completed are reliable; no ETA if unknown; 100% does not mean verified copy |
| `QDPathLabel(path, scope)` | truncated, full | full path in tooltip/inspector/copy; for dangerous operations, the full path is visible before confirmation |
| `QDFilterChip(filter, onChange)` | inherited, overridden, disabled | override is clearly visible, reset to panel filter, keyboard accessibility |
| `QDNotice(kind, text, action?)` | info/warning/error/unknown | embedded in context; important messages must not be hidden by "success" |
| `QDEmptyState(reason, action?)` | genuinelyEmpty, loading, unsupported, noPermission, filtered, offline, failure | distinguish between "nothing exists" and "cannot read"; action only if available |
| `QDConfirm(scope, operation, consequences)` | pending, declined, approved, expired | default-deny, name, source/destination, full path, count/bytes if known, consequences; confirmation is limited by scope and time |

Do not create custom input fields/popovers where the system `Button/Toggle/Menu/TextField/Picker/ConfirmationDialog/Alert` already provides accessibility; use styles and layout for branding.

## 2. Runtime semantics and evidence

Visible status and data acquisition status are **two independent axes**.

1. Fetch/availability: `loading | fresh | stale(lastSeen) | offline(lastSeen?) | unsupported(reason) | error(reason)`.
2. Job/execution: `queued | active | paused | awaitingConsent | conflict | failed | unknown | completed(reported/providerConfirmed/verified)`.

A "Verified" label is allowed only if there is sufficient evidence for the operation type. `providerConfirmed` and `workerReported` are shown with different labels. UNKNOWN does not include an unconditional "Retry" button; read-only reconciliation comes first. Offline is not equal to FAILED.

Control follows this chain: UI intent → availability/permission check → existing FFI command → durable Core state → UI refresh. Optimistic command display is allowed only as "Sent/Pending application", not as a completed Job.

## 3. Complex rows

**Job row:** short name; source → destination; operation/path; state/evidence; verified progress of files and bytes; active pause/resume/cancel buttons only if Core supports them for the current status. Context menu: "Details", "Open history", "Copy ID"; dangerous commands must pass Core Policy.

**Connection row:** connection name; adapter type; connected/offline/authNeeded/unsupported; quota based only on adapter data with `lastUpdated`; exact instance identifier. Multiple connections to one provider are separate rows. The "Refresh metadata" button must not download content.

**Conflict row:** two endpoints + path, attributes actually matched, what-is-unknown, rule scope. "More" and "Resolve" buttons only if a safe resolver is implemented. No "resolve all" without confirmation of boundaries and explicit consequences.

## 4. `QDWidgetFrame`: unified shell

The structure is strictly: `frame → header → content → optional footer → contextual overlay`.

| Part | Composition | UX |
|---|---|---|
| Frame | background, thin border, clip, focus | uniform radius 8pt; normal state without glow |
| Header (32pt) | 14pt icon, title 13pt semibold, optional count, trailing actions | menu "...", refresh, expand; actions may be hidden until hover, but VoiceOver/keyboard are available |
| Content | layout based on type and density | scroll only if content exists; no nested scroll-jank; empty state occupies a meaningful height |
| Footer | freshness, units, additional actions | do not duplicate global statusbar |
| Overlay | loading/stale/disabled, selected/edit handles | the message covers **only** the unavailable area and does not hide critical actions |

Border enhancement is allowed on hover; a widget selected in edit-mode draws a focus stroke + resize handles. For warning/error, highlight a narrow zone of the header/border; do not turn the whole widget into a red glowing panel.

### Widget context menu

`Refresh` for a specific provider → `Density` (compact/regular/expanded) → `Filters` (inherit/override) → `Expand` → `Open in window` (if supported) → `Remove from dashboard` (edit-mode only). This is independent of executing Jobs. If an action is not supported, it is disabled + an explanation is provided, without an empty menu.

## 5. Inspector, menus, and confirmations

The Inspector opens over/beside the current workspace without a navigation transition; on a narrow window it is a sheet, on a large window it is a temporary detail panel or a separate window. Closing returns focus to the selected row. For dangerous decisions, use a separate sheet/confirmation where the cancel/close button does not affect the Job. Do not automatically show a dialog for every event; an indicator collects pending items, and the user opens them intentionally.

All elements have correct `accessibilityLabel`, `accessibilityValue`, `accessibilityHint`; Job progress is announced without spam; keyboard control is mandatory.

## 6. Visual states matrix (minimum)

| Context | Design | Primary action |
|---|---|---|
| Working + fresh | cyan action/progress, neutral border | pause/open if actually available |
| Verified success | green short marker without intrusive lighting | inspect |
| Waiting | neutral/blue, no infinite animation | inspect/reorder if Core allows |
| Needs consent / conflict | amber border and alert icon | inspect → scoped confirm |
| Failed | red marker, reason details | inspect, retry only if safe |
| UNKNOWN | lilac/neutral distinct marker | read-only reconciliation |
| Offline/stale | muted data, text "updated ..." | refresh metadata when safe |
| Unsupported | neutral disable-area + reason | open settings |

This set is basic: when adding a widget, do not invent incompatible behavior for already defined states.
