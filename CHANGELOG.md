# Changelog

Each GitHub Release uses the section of its version as the release description ([D-026](docs/decisions/D-026-github-releases.md)). Write each section in English and ASD-STE100 style. Do not put user content or account data in it.

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
