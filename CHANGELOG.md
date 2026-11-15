# Changelog

Each GitHub Release uses the section of its version as the release description ([D-026](docs/decisions/D-026-github-releases.md)). Write each section in English and ASD-STE100 style. Do not put user content or account data in it.

## [0.3.0] - 2026-11-15

This pre-release adds an ISPmanager 6 panel as a read-only backup source, a Daemons section in Settings, and lower CPU use in the monitoring panel.

### Added

- An ISPmanager 6 connection ([D-027](https://github.com/Xsoid/QADREN/blob/main/docs/decisions/D-027-ispmanager-backup-source.md)). QADREN only reads the panel. It never changes the panel or the server.
- A Backup/export Scenario from ISPmanager to a Mac folder or an SMB share. It keeps one current copy with a folder for each data type, and each run updates it.
- Export of site files over SFTP, with exclusions and a file browser. "All sites" also includes the sites that you add later.
- Database dumps over SSH, compressed during the transfer, with table rules (structure only). When SSH is not available or the SSH dump fails, QADREN uses the panel dump. A dump of 700 MiB or more is split into one file for each table, so you can restore one table.
- Export of panel settings as JSON files: sites, Cron jobs, DNS zones, mail domains and mailbox settings, SSL certificates, FTP users, plan limits, and PHP versions. Each DNS zone also gets a BIND zone file. The export never contains a password or a private key. Mail letters can be copied as Maildir files.
- A hidden database password can be entered in the Scenario editor. QADREN checks it and keeps it in the macOS Keychain.
- A "Демоны" (Daemons) section in Settings. It shows the state of the local Worker and has Restart, Stop, and launch at login. You can stop a run from Settings.
- The Scenario widget shows the last run result, the progress of each Backup/export category, and the stage "Ждёт повтора" (waiting for a retry) with the next retry time.
- A "Проверить план" (Check plan) button in the Scenario widget. It checks the plan and changes no file.
- The Connections widget shows the used and free space of each ready connection. Layout mode lets you select the connections that the widget shows.
- The System widget has an expanded size with Restart and Stop, and shows a JobStore read error.

### Changed

- The monitoring panel uses much less CPU during a run and at rest ([D-028](https://github.com/Xsoid/QADREN/blob/main/docs/decisions/D-028-energy-efficiency.md)). A hidden window stops its polls and its motion.
- The JobStore and the Registry use WAL, so a monitoring read never waits for a Worker write.
- During a plan check, the Scenario widget hides "Запустить" and "Проверить план" until the check ends.
- Without a running Worker, an active run shows "Не выполняется" and has no Stop button. A stop of the Worker first stops the active Scenario runs.

### Fixed

- A file manifest on an SMB share failed because the share refused a full flush.
- When the app and the Worker opened an older Registry at the same time, two migrations could break the Registry.
- A destination name that the SMB share refuses is now a permanent failure with a message. Before, the run waited for a retry without end.
- The removal of old database files after a split dump always failed.

### Known limitations

- Only Apple silicon Macs with macOS 13 or later.
- The app is not Developer ID signed and not notarized.
- Yandex Disk and ISPmanager can be only a source.
- iCloud is not supported.

## [0.2.0] - 2026-11-01

The first public pre-release of QADREN for macOS.

### Added

- Scenarios between "This Mac" folders and SMB network drives. QADREN mounts an SMB share when a run needs it.
- One-way Scenarios from Yandex Disk to a Mac folder or an SMB share.
- Schedules on selected days at a set time, or every N hours. A Scenario has at most one active run. A slot during an active run is skipped.
- Automatic create, update, and delete in a one-way Scenario: the source always wins. Replaced and deleted files go to the Trash.
- Two deletion modes: Paranoid (every deletion asks) and Simple synchronization (one approved rule).
- A background Worker. It starts at login, runs different Scenarios in parallel, and restarts only between files.
- A monitoring panel with System, Scenario, Scenarios, and Connections widgets.
- Reconciliation without mutation when proof of an effect is lost, also after an SMB remount. If the evidence is not sufficient, the result stays `UNKNOWN`.

### Known limitations

- Only Apple silicon Macs with macOS 13 or later.
- The app is not Developer ID signed and not notarized.
- Yandex Disk can be only a source.
- iCloud is not supported.
