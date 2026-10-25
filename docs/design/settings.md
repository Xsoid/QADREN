# QADREN — settings window

## 1. Role

**Separate "Settings" window**, called from MenuBarExtra, widget inspector, and system menus. Navigation **is permitted here**; it DOES NOT become the monitoring panel Sidebar. Window id `scenario-settings` is kept for deep links. "Application Settings" is one section of the left menu, not a separate window.

The sidebar button has one fixed place: in the titlebar, right after the QADREN mark. It hides and shows the section sidebar and does not move when the sidebar is hidden. The system sidebar toggle is removed, because it moves between the sidebar and the toolbar and can fall into the toolbar overflow menu. Opening the window does not put the keyboard focus on a control. The Settings window has no toolbar actions: each action lives in the section that it changes ("Добавить подключение", "Создать сценарий"), and lists refresh when a section opens and after each action. The titlebar has one plain fill in both sidebar states: the detail content is clipped below the titlebar, without the scroll edge effect or the toolbar background separator. The window keeps the sidebar state.

Recommended 2-column layout: a narrow permanent navigation with sections on the left, and a list + inspector or a specific entity editor on the right. For small widths, use a navigation stack without two nested permanently visible sidebars. On macOS, use native `NavigationSplitView`, `Form`, `Table/List`, `Toolbar`, and native fields styled with semantic tokens; do not change panels into a glowing HUD.

## 2. Structure

| Section | Content | Before backend implementation |
|---|---|---|
| Connections | Flat list of named `StorageConnection`, adapter, status, credentials/capabilities, add/edit/remove | This Mac and SMB are Scenario endpoints; Yandex Disk is a one-way Scenario source (D-023); iCloud is out of scope |
| Scenarios | List with revision, source/destination, and status; create/rename/enable/disable/delete | Use existing Scenario model; do not show impossible operations as active |
| Automation | Triggers and schedules **inside a scenario** (may be a separate editor tab) | While the scheduler is missing: read-only disabled with explanation |
| Policies | Conflicts, deletions, scoped consents, space thresholds, event retention | Do not add false protection features or backup versions |
| Node and Service | Local Worker, start/pause, independent service status, launch management | Show unavailable signs if there is no independent service |
| Interface | Theme, window zoom, default density, panel restoration | Compatible with locally saved layouts |
| File transfer | Block of Application Settings: the switch "Повторять недоступные файлы до успеха" with a caption that states the retry schedule for the current value (D-024) | The value is in the Registry, so the Worker reads it; default on |
| About | Last block of Application Settings: app version and build, Core version, author (Xsoid), and a link to the developer site https://xsoid.net | Values come from `Info.plist` and the Core library |

These are not six independent app windows: this is the structure of **one** settings panel. Deep-link points have `section`, `entityID`, and `editorSubsection` with validation of the entity existence.

## 3. Connections

The main area is a flat list of connection cards; do not group by provider as if there is only one connection. Each card has a visible boundary (a border and a left edge in the connector color), the connector icon and name, the connection name, a short ID, a status label ("Включено", "Выключено", "Заморожено"), the name field, the on/off switch, and only the actions that the connector supports now. "Этот Mac" exists automatically and exactly once; it has no removal action (D-020). A "Сетевой диск (SMB)" card also shows the share address, the saved login (never the password), "Проверить вход", and "Изменить адрес и вход" (D-021). Its status label shows the last check: "Подключено", "Нужен вход", "Нет связи", "Ошибка", or "Не проверено". The SMB form starts with a mouse choice: SMB shares that macOS has mounted, SMB servers found in the local network (a click opens the server in macOS, which asks for the login and the share), and "Выбрать папку…". Below it are the name, the `smb://` address for manual input, the login, and the password. "Проверить и сохранить" signs in first; after a failed check, "Сохранить без проверки" is available. The "Добавить подключение" button in the section header opens the connector list with the state of each connector. The Connections section is one navigation stack in the Settings window, not a set of sheets or windows: the list, "Добавить подключение", and the connector form (for example "Сетевой диск (SMB)") replace one another in the detail area. Breadcrumbs above a nested screen show the path, for example "Подключения › Добавить подключение › Сетевой диск (SMB)". Each crumb except the last one goes back to its screen; the back button and Command-[ go one level up. "Изменить адрес и вход" on a card opens "Подключения › <name> › Адрес и вход" in the same way. Two Yandex accounts can have the same displayName and **different** `connection_id`. A card contains: name, provider, enable/disable, auth state, metadata only upon explicit refresh, last status, and full paths/permissions. The inspector shows capabilities, settings, and the linked Job queue, but **not** secrets.

### Addition (wizard)

1. Select adapter from those available on the current OS / capability (explain unavailable ones).
2. User name and settings; check preconditions.
3. Authorization via existing OAuth/Keychain (where applicable) without showing the token.
4. Check metadata / permissions; result with timestamp.
5. Save independent connection ID; exit without a fake "Connected" message if an error occurs.

For deletion or disabling: provide explicit information that provider files are not deleted, but new Jobs can be blocked. Consider active attempts and recoverable SQLite and Keychain cleanup. Do not delete the registry and leave an orphan credential without managed recovery.

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

Creation does not open a small sheet: the editor occupies the detail zone of the "Settings" window. The name, two exchange points, and synchronization type are at the top. Below them, two equal panels show the folder structure of each endpoint connection directly in the editor (D-019). There is no separate folder window and no system picker step.

Each panel has:
- the connection picker;
- breadcrumbs from "Этот Mac" to the visible folder, with "up", "home", and "go to path" buttons;
- a list in the style of a two-pane file manager: icon, name, size; a double click or the chevron opens a folder;
- the "Сделать корнем «folder»" button, which makes the visible folder the Scenario root. "Отменить" restores the previous root and selection, "Сбросить" clears the root, and "Показать корень" opens it.

The source panel (the left panel in a two-way Scenario) has a checkbox before each object inside the root. Checked objects are the Scenario selection; an empty selection is the full root. A summary under the panel shows the selected objects and the selection rules. The browser reads names, types, and sizes only; it hides hidden entries and skips links. A "Сетевой диск (SMB)" connection is available in the point selection too (D-022). Selecting it mounts the share first ("Подключаю сетевой диск…"); its breadcrumbs start at the connection name, the browser does not go above the share, and the root reads as "<connection>: /folder". A mount error is shown in the panel with "Повторить"; a share that macOS mounted at another place offers "Отключить там и подключить для QADREN". A network drive endpoint states that a deletion there is usually permanent. "This Mac" is available directly in the point selection: when saving a scenario, a separate `local_filesystem` `StorageConnection` is created or reused. Cloud viewing is limited to directory metadata and does not download content.

Direction and deletion mode use choice cards (icon, title, one short line, accent border on the selected card), not segmented controls. A "Удаления" box selects the deletion mode: "Параноик" or "Простая синхронизация". Selecting Simple synchronization shows one confirmation with its consequences: Trash or permanent deletion, and the 20-deletion threshold. The Scenario list shows the selection and the deletion mode of each Scenario. The plan shows rule deletions and one "Удалить по правилу" action; the Worker plans again before it deletes.

Show the consequences of changes: current Jobs use the old revision, new Jobs use the new revision. `Run now` creates a new Job only through Core with correct permissions; settings do not copy files themselves.

## 5. Scheduling

Editing each trigger: type, schedule time/timezone, enabled, next valid run; DST ambiguous/missing states are shown honestly; manual trigger maintains priority (Core). Multiple schedules exist in one Scenario, and each is disabled separately. If the scheduler is not yet implemented, the interface may document parameters, but the action is disabled / Preview only, without a fake "scheduled" status.

## 6. Confirmations/Errors

Secrets are not visible in the UI, error details, logs, or clipboard. Authorization errors and API limitations are shown locally in the context of the Connection, with a code/proposed safe action. No hidden content read for "checking". Connection deletion requires confirmation by connection ID/name with safe rollback and a pending cleanup sign.

A Yandex Disk card shows the account login, the disk space, "Войти в Яндекс" or "Войти заново", and "Выйти" (local only; files on the Disk do not change). The status label uses the same words as an SMB card. In the Scenario editor a Yandex Disk connection is available only on the source side, and its Scenario is "В одну сторону" (D-023). The folder browser shows `disk:/` folders; a Yandex root reads as "<name>: /folder". iCloud Documents/backup/sync are postponed indefinitely (D-017/D-018): do not show as a working/connectable adapter, do not add a wizard, entitlement probes, or fake UI. Local FS still prohibits hidden materialization of File Provider/iCloud placeholders.

## 7. Visual Style of the Settings Window

Use the same palette/typography/logo/navigation focus, but **less glow**, more native form controls, and structured lists. Use `panel/raised` surfaces, highlighting only for focused/selected rows, verified warnings, and destructive confirmation. Content must not change height during network refresh. Full path and ID copying are available.

## 8. Acceptance

- Two connections of one provider can be independently opened, renamed, and enabled/disabled; secrets are not shown.
- A deep link from any widget returns the correct section and entity without changing the current monitoring panel.
- OAuth error, expired authorization, unavailable adapter, and lack of permissions have different states.
- Scenario parameters correspond to real Core capabilities and revisions; no fake Run/Save/Provider copy.
- After closing, the monitoring state is not reset, and the service does not change state.
- Keyboard, system controls, and min-size 760×560 work. VoiceOver support is out of scope.
