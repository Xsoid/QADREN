# D-023 — Yandex Disk to Mac in Scenarios

**Date:** 2026-10-24. **Status:** accepted. **Revises:** D-018 (Yandex Disk is post-release and parked), D-022 ("Yandex Disk stays outside Release 1"), and the boundary in [Yandex Disk adapter](../architecture/yandex-adapter.md).

## Context

Release 1 is out. The current work is a set of Release 1 improvements. The owner reported that the Yandex Disk API restrictions after the October 2026 data-center incident are resolved. A read-only check on 2026-10-24 with the existing `yandex_disk` connection confirmed this: disk metadata, folder listing, and a file download through `/resources/download` worked, and the downloaded size and MD5 matched the provider metadata. The existing token has only the `cloud_api:disk.read` scope; a folder create returned `403`.

## Decision

1. **Stage 1: one way, Yandex Disk to Mac.** A one-way Scenario can use a `yandex_disk` connection as the source. The destination is "This Mac" or an SMB network drive (the Local Filesystem executor). The Yandex Disk connection is never a destination in this stage.
2. **Stage 2: two way.** A later step adds Mac to Yandex Disk and two-way Scenarios. It needs the `cloud_api:disk.write` scope, upload, folder create, and deletion to the Disk Trash. Stage 1 does not request the write scope.
3. **Download effect.** The Core Yandex client requests a download link, accepts only an `https` link, and sends no OAuth header to the link host. It follows at most one redirect, to an `https` host under `yandex.net` or `yandex.ru`. It streams the content and computes MD5 at the same time.
4. **Local write.** The executor uses the Local Filesystem executor rules for the destination: a durable attempt before the effect, a private staging name, a check of the byte count and MD5 against the provider metadata, then no-overwrite publish for a new file or the approved replacement protocol for an update. A mismatch publishes nothing.
5. **Recheck before the effect.** The executor reads the Yandex metadata of the source file again before the attempt. A different size or MD5 than the plan saw stops the file as stale.
6. **Reconciliation.** After lost effect proof, the executor checks only the local destination. It never downloads again blindly. Insufficient evidence keeps `UNKNOWN`.
7. **Plan snapshot.** Core lists the Yandex source folder recursively through bounded metadata pages. Each file gives its relative path, size, MD5, and modification time to the pure planner.
8. **Where it runs.** In stage 1 the app runs Yandex Jobs and gives the token to Core for each call. The background Worker refuses a Yandex Job before any effect, because it cannot read the app Keychain item. A shared Keychain access group is a separate step.
9. **Secrets.** The access token stays in the Keychain, bound to `connection_id`. Core receives it only in memory. Core never writes the token or a signed download link to SQLite, logs, or Job evidence.

## Consequences

- AGENTS.md, `llms.txt`, the roadmap, D-018, D-022, the storage adapter contract, and the Yandex Disk adapter contract state this scope.
- iCloud stays outside Release 1. D-017 is unchanged.
- The app does not store a refresh token. A token expires after about one year; then the user connects the account again.
- Native end-to-end evidence with a real Yandex Disk folder is open work for each change.
