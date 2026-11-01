# D-027 — ISPmanager 6 Read-Only Backup Source

**Date:** 2026-11-01. **Status:** proposed. The owner must approve this decision before Gate 1 code starts. **Revises (after approval):** the "runtime BackupSet is deferred" rule in the [synchronization model](../architecture/backup-model.md) and the [roadmap](../roadmap.md), only for the backup run namespace in section 5. **Work:** GitHub epic Xsoid/QADREN#36.

## Context

The owner asked for an ISPmanager 6 connector. The user connects a panel by URL, login, and password, selects exact resources, and exports them to a Local Filesystem or SMB connection.

Current contracts allow only synchronization. A one-way Scenario makes the destination equal to the source (D-025). A backup of a server needs other rules: a source change must never delete or replace an earlier backup, and a missing resource is a failure, not a deletion.

The ISPmanager API catalog is generated. The functions, parameters, and access levels depend on the installed panel, its modules, and the account level. The documentation alone does not prove any export path. See the [ISPmanager adapter](../architecture/ispmanager-adapter.md) for the documented functions and the capability states.

## Decision

### 1. Connection

1. `ispmanager` is a new adapter type. Each connection is an independent `StorageConnection` (D-016). More than one connection can use the same panel, also with the same login.
2. The Registry stores the display name, the panel URL, the login, the verification state, the last check time, and the detected capabilities. The panel URL uses `https`. It can have a port and a path. It has no user information, query, or fragment.
3. The password is a Keychain item bound to `connection_id`. The app gives it to the Worker in memory, the same as the Yandex Disk token (D-024 stage A, D-025 section 7).
4. The connection is a **source only**. A Scenario with an `ispmanager` connection as the destination is invalid. The destination is a Local Filesystem connection ("This Mac") or an SMB share.

### 2. Transport and secrets

1. Every API request is an HTTPS `POST` with a form body. The login, the password, and the session ID are in the body. They are never in a URL.
2. TLS uses the standard trust roots. QADREN does not accept an untrusted certificate, also not after a user prompt in this stage.
3. The client follows no redirect. A redirect response is an error.
4. A session ID stays in memory only. When the panel reports an expired session, the client authenticates again one time for that request. A second failure stops the work with an authentication error. There is no retry loop.
5. QADREN never sends `su`. It does not use a privilege that the configured account does not have.
6. Passwords, session IDs, authorization data, and signed download URLs never go into SQLite, logs, diagnostics, process arguments, Job evidence, or the work tracker.

### 3. Read-only function allowlist

1. The client calls only functions in a compiled read-only allowlist. A function that is not in the list is refused before any request.
2. The allowlist contains list and download functions only, for example `file`, `file.download`, `db`, `db.dump`, `email`, `scheduler`. A form function such as `*.edit` enters the allowlist only after a live check proves that a call without `sok` changes nothing.
3. These functions are never in the allowlist: any `*.delete`, `*.suspend`, `*.resume`, `*.new`, `*.add`, `*.upload`, `*.restore*`, `*.clear*`, `run`, `su`, `*.su`, `session.*`, `backups.new`, `backup.plan.run`, and every function that installs, enables, disables, or restarts something.

### 4. Capability states

Each resource category of a connection has one state. Discovery sets it with read-only calls on that panel.

| State | Meaning |
|---|---|
| `verified_export` | A live check exported exactly one selected object with this function on this kind of panel. |
| `verified_metadata_only` | A live check read the configuration or the list, but no content export exists. |
| `permission_denied` | The panel refused the list function for this account. |
| `unsupported` | The panel has no such function or module. |
| `unverified` | No live check exists yet. |

Only `verified_export` permits a content export. `verified_metadata_only` permits a metadata export that the manifest labels as metadata. The interface shows every category with its state and reason. A state that is not available never hides the category.

### 5. Backup/export Scenario mode

1. An ISPmanager Scenario has the named mode **Backup/export**. It is not a synchronization mode. It has no conflict rule, no deletion mode, and no replacement.
2. Each run writes into a new run folder below the destination root: `<root>/<run start, UTC, ISO 8601 basic>-<run ID>/`. The executor creates this folder with no-overwrite. A run never writes outside its own run folder.
3. A run never deletes, replaces, or prunes an earlier run folder. There is no retention, no deduplication, and no restore in this stage.
4. The run writes `manifest.json` last, through a staging name and no-overwrite publish. The manifest records for each selected resource: the category, the stable provider ID, the hierarchy, the artifact type (file, database dump, mailbox content, configuration metadata), the export format, the start and end time, the byte count and checksum, the result, and every omitted subpart with its reason.
5. The run result is `complete` only when each selected resource has a verified result. Otherwise it is `partial` or `failed`. A partial run is never shown as a successful backup.
6. A run folder with its manifest is the only new backup concept. This decision does not add a runtime BackupSet entity, content version history, or retention to the Registry.

### 6. Selection

1. The selection is a typed list of resources: category, stable provider ID, display hierarchy, and a recursive flag for a folder. It is not a list of file paths. A database or a mailbox is never written as a fake path.
2. An empty selection means nothing. "Select all accessible resources" is a separate, explicit flag. The D-019 rule "an empty selection means the complete root" does not apply to this mode.
3. A selected folder includes new descendants. It excludes siblings.
4. A selected resource that is missing, renamed, or not accessible at run time fails as one item with a reason. It never becomes a deletion.
5. A resource that appears under more than one group, for example a database under a site, has one canonical ID. A run exports it one time.

### 7. Execution

1. The run uses the existing durable Scenario Run, the stored plan, batch children, and retry classes (D-024, D-025). There is no second scheduler or executor.
2. The destination effect uses the Local Filesystem executor: a durable attempt, a staging name, a check of the byte count, then no-overwrite publish. SMB uses the QADREN mount (D-022).
3. Remote reads can be retried. The connector makes no remote mutation, so no remote effect needs reconciliation. Reconciliation after a lost proof checks only the local destination. Insufficient evidence keeps `UNKNOWN`.
4. A file streams from the panel to the staging name. QADREN does not use the main MacBook as an unapproved full-content mirror.

### 8. Native ISPmanager backups

This stage does not use native ISPmanager backups (`backups.*`, `backup.plan.*`, `backup2.*`). They create archives on the server, use server disk space, or bundle more data than the selection. A later owner decision can allow one of them after a live check of its exact content and side effects. The native backup does not include Docker.

### 9. Secrets in exported data

SSL private keys and other secrets that a panel can export are `unsupported` in this stage. A later owner decision can allow them through a verified, secure export method. QADREN never claims that a password is recoverable from a database grant or an account list.

## Owner questions

1. Approve the run folder name format in section 5.2.
2. Approve the exclusion of native backups (section 8) and of exported secrets (section 9) in this stage.
3. Give a test panel and test accounts (a user and an administrator) for Gate 0. The live checks run on the owner Mac with the read-only probe. Real names, paths, and counts stay outside the repository and the work tracker.

## Consequences (after approval)

- AGENTS.md, `llms.txt`, the roadmap, the synchronization model, the storage adapter contract, and the data-safety policy state this scope.
- The Settings design contract gets the ISPmanager connection form, the capability list, and the typed resource browser.
- Existing Local Filesystem, SMB, and Yandex Disk Scenarios do not change.
