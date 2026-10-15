# QADREN — settings window

## 1. Role

**Separate "Settings" window**, called from MenuBarExtra, widget inspector, and system menus. Navigation **is permitted here**; it DOES NOT become the monitoring panel Sidebar. Window id `scenario-settings` is kept for deep links. "Application Settings" is one section of the left menu, not a separate window.

Recommended 2-column layout: a narrow permanent navigation with sections on the left, and a list + inspector or a specific entity editor on the right. For small widths, use a navigation stack without two nested permanently visible sidebars. On macOS, use native `NavigationSplitView`, `Form`, `Table/List`, `Toolbar`, and native fields styled with semantic tokens; do not change panels into a glowing HUD.

## 2. Structure

| Section | Content | Before backend implementation |
|---|---|---|
| Connections | Flat list of named `StorageConnection`, adapter, status, credentials/capabilities, add/edit/remove | Local FS is the only active controller for Release 1; Yandex OAuth/Keychain/Registry exist, but the controller is out of release (keep previous tokens); iCloud is out of scope |
| Scenarios | List with revision, source/destination, and status; create/rename/enable/disable/delete | Use existing Scenario model; do not show impossible operations as active |
| Automation | Triggers and schedules **inside a scenario** (may be a separate editor tab) | While the scheduler is missing: read-only disabled with explanation |
| Policies | Conflicts, deletions, scoped consents, space thresholds, event retention | Do not add false protection features or backup versions |
| Node and Service | Local Worker, start/pause, independent service status, launch management | Show unavailable signs if there is no independent service |
| Interface | Theme, window zoom, default density, panel restoration | Compatible with locally saved layouts |

These are not six independent app windows: this is the structure of **one** settings panel. Deep-link points have `section`, `entityID`, and `editorSubsection` with validation of the entity existence.

## 3. Connections

The main area is a flat list; do not group by provider as if there is only one connection. Two Yandex accounts can have the same displayName and **different** `connection_id`. A card contains: name, provider, enable/disable, auth state, metadata only upon explicit refresh, last status, and full paths/permissions. The inspector shows capabilities, settings, and the linked Job queue, but **not** secrets.

### Addition (wizard)

1. Select adapter from those available on the current OS / capability (explain unavailable ones).
2. User name and settings; check preconditions.
3. Authorization via existing OAuth/Keychain (where applicable) without showing the token.
4. Check metadata / permissions; result with timestamp.
5. Save independent connection ID; exit without a fake "Connected" message if an error occurs.

For deletion/disabling: provide explicit information that provider files are not deleted, but new jobs may be blocked; consider active attempts and recoverable cleanup of SQLite/Keychain according to issue #25. Do not delete the registry and leave an orphan credential without managed recovery.

## 4. Scenario: Master and Inspector

**List:** name, enabled, direction, source/destination, specific Worker, status/last result/next run (if reliable). No graph-based topology editor.

**Editor with sections inside a scenario:**
1. Name and basic parameters;
2. Source (connection + directory) and destination (connection + directory);
3. Synchronization type: one-way (from selected source to selected destination) or two-way; the order of points already defines the direction, therefore there is no separate "destination $\rightarrow$ source" in the UI;
4. Include/exclude by path/extension, without content scanning in the UI;
5. Conflict/overwrite/delete policies + thresholds;
6. Trigger schedules (multiple timezone-aware) and destination Worker;
7. Summary, **dry-run metadata diff** on request, revision creation/saving.

Creation does not open a small sheet: the editor occupies the detail zone of the "Settings" window. The name, two exchange points, and synchronization type are at the top. After selecting the points, two equal panels show the metadata-only tree of each source; the user selects the root directory in each panel. "This Mac" is available directly in the point selection: when saving a scenario, a separate `local_filesystem` `StorageConnection` is created or reused. Selection of a local folder is requested via a system picker; cloud viewing is limited to directory metadata and does not download content.

Show the consequences of changes: current Jobs use the old revision, new Jobs use the new revision. `Run now` creates a new Job only through Core with correct permissions; settings do not copy files themselves.

## 5. Scheduling

Editing each trigger: type, schedule time/timezone, enabled, next valid run; DST ambiguous/missing states are shown honestly; manual trigger maintains priority (Core). Multiple schedules exist in one Scenario, and each is disabled separately. If the scheduler is not yet implemented, the interface may document parameters, but the action is disabled / Preview only, without a fake "scheduled" status.

## 6. Confirmations/Errors

Secrets are not visible in the UI, error details, logs, or clipboard. Authorization errors and API limitations are shown locally in the context of the Connection, with a code/proposed safe action. No hidden content read for "checking". Connection deletion requires confirmation by connection ID/name with safe rollback and a pending cleanup sign.

The Yandex Disk controller is out of Release 1 (D-018): do not present new connections as current functionality; keep old records/tokens. iCloud Documents/backup/sync are postponed indefinitely (D-017/D-018): do not show as a working/connectable adapter, do not add a wizard, entitlement probes, or fake UI. Local FS still prohibits hidden materialization of File Provider/iCloud placeholders.

## 7. Visual Style of the Settings Window

Use the same palette/typography/logo/navigation focus, but **less glow**, more native form controls, and structured lists. Use `panel/raised` surfaces, highlighting only for focused/selected rows, verified warnings, and destructive confirmation. Content must not change height during network refresh. Full path and ID copying are available.

## 8. Acceptance

- Two connections of one provider can be independently opened, renamed, and enabled/disabled; secrets are not shown.
- A deep link from any widget returns the correct section and entity without changing the current monitoring panel.
- OAuth error, expired authorization, unavailable adapter, and lack of permissions have different states.
- Scenario parameters correspond to real Core capabilities and revisions; no fake Run/Save/Provider copy.
- After closing, the monitoring state is not reset, and the service does not change state.
- Keyboard/VoiceOver, system controls, and min-size 760×560 work.
