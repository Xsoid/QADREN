# D-027 — ISPmanager 6 Read-Only Backup Source

**Date:** 2026-11-01. **Status:** proposed. The owner must approve this decision before Gate 1 code starts. **Revises (after approval):** nothing in the synchronization model: the Backup/export mode is a one-way update of one current copy (section 5) and adds no BackupSet. **Work:** GitHub epic Xsoid/QADREN#36.

## Context

The owner asked for an ISPmanager 6 connector. The user connects a panel by URL, login, and password, selects exact resources, and exports them to a Local Filesystem or SMB connection.

Current contracts allow only synchronization. A one-way Scenario makes the destination equal to the source (D-025). A backup of a server needs other rules: a source change must never delete or replace an earlier backup, and a missing resource is a failure, not a deletion.

The ISPmanager API catalog is generated. The functions, parameters, and access levels depend on the installed panel, its modules, and the account level. The documentation alone does not prove any export path. See the [ISPmanager adapter](../architecture/ispmanager-adapter.md) for the documented functions and the capability states.

## Decision

### 1. Connection

1. `ispmanager` is a new adapter type. Each connection is an independent `StorageConnection` (D-016). More than one connection can use the same panel, also with the same login.
2. The Registry stores the display name, the panel URL, the verification state, and the last check time. The capability report of a check is shown in the interface; a later step stores it. The panel URL uses `https`. It can have a port and a path. It has no user information or query. A fragment, such as `#/` in the address that a browser shows, never reaches the panel; QADREN removes it.
3. The login and the password are one Keychain item bound to `connection_id`, as for an SMB share (D-021). The app gives it to the Worker in memory, the same as the Yandex Disk token (D-024 stage A, D-025 section 7).
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

The owner decided on 2026-11-02: the destination is **one current copy** that each run updates. A run does not make a new copy folder, and QADREN keeps no generations of old data.

1. An ISPmanager Scenario has the named mode **Backup/export**. It is a one-way update from the panel to the destination. The source always wins, as in a one-way Scenario (D-025 section 3). The user confirms this rule one time.
2. Each data type has its own subfolder in the destination root. The executor creates a subfolder only for a type that the Scenario exports:

   ```text
   <root>/
     manifest.json                     state of the last run
     files/<path below the home folder> selected files and folders
     databases/<database>.sql          one current dump for each selected database
     mailboxes/<mailbox>/              mailbox content and settings
     mail-domains/  cron/  dns/  ssl/  ftp/  accounts/  web-server/  php/
     mail-service/  network/  docker/  panel-settings/  logs/
   ```

   The subfolder names are the stable category keys of the [adapter contract](../architecture/ispmanager-adapter.md), in kebab case. They do not change with the interface language. `files/` keeps the path below the home folder of the account, so that objects from different branches cannot have the same destination name. The home folder is the one that the connection check finds (`.` on the SFTP server). It is the source root of the Scenario: the account has no use for paths above it, so the browser does not show them, and a rule outside it is refused (owner decision 2026-11-04).
3. **Update.** A new object is created with no-overwrite. A changed file, a new database dump, or changed metadata replaces the destination object through the approved replacement protocol (D-025 section 6). The replaced version goes into the Trash of the destination when it has a `trash` capability; an SMB share usually has none, so the loss is permanent.
4. **Deletion.** An object that is absent in the source scope is deleted from the destination (section 5.6a) under the deletion mode of the revision: Simple synchronization (the default, owner decision 2026-11-02) or Paranoid, where each deletion waits for a decision (D-019, D-025 section 5). The user confirms Simple synchronization together with the source-wins rule one time, when the Scenario is created.
5. **Guard.** A selected resource that is missing, renamed, or not accessible, a listing error, a permission error, or an incomplete snapshot is an error for that resource. It never gives a deletion or a replacement in the destination (D-025 section 5.3).
6. **Manifest.** The run writes `manifest.json` last, through a staging name, and replaces the previous one. It records for each selected resource: the category, the stable provider ID, the artifact type (file, database dump, mailbox content, configuration metadata), the export format, the time, the byte count and checksum where available, the result, and every omitted subpart with its reason.
6a. **Mirror of `files/`.** The `files/` subfolder is a mirror of the source scope (owner decision 2026-11-05). After a complete source snapshot, each object in `files/` that the source scope does not have is deleted under the deletion mode of the revision. This includes an object without a confirmed state, an object outside the selection, an object inside an exclusion, and an object left by an earlier source root. The destination snapshot covers all of `files/` for this reason. A path inside the scope is deleted only when the server answers that the source object is absent. A path outside the scope is not in the source by rule, so it needs no server check. These objects are never deleted: a `.qadren-` staging or replacement file of the local executor, a symbolic link or special file, a name collision, and a name variant of a selected object. The deletion goes into the Trash of the destination when it has a `trash` capability.
6b. **Folder deletion.** When every planned object under a destination folder is a deletion and the source has no folder with that name (by name key), the plan has one `delete_folder` row for the topmost such folder instead of its file rows (owner decision 2026-11-05). The row shows the file count and the byte count. The run deletes the folder as one batch child: one Job, one bound attempt (`local_rule_trash_folder` or `local_rule_delete_folder`), one effect. With a Trash, the folder moves into the Trash as one entry. Without a Trash, the folder is removed with all its content; every directory is opened without following a symbolic link, and an error after the first removal is `UNKNOWN`. Before the attempt, the executor reads the folder again. A symbolic link, a special file, a `.qadren-` file, an unresolved attempt under the folder, or a selected object inside it stops the deletion; the next plan then decides file by file. A folder inside the scope is deleted only when the server answers that it is absent. Reconciliation of a lost proof checks only that the folder is gone. A verified folder deletion removes the confirmed states of all paths under it. The mirror applies only to `files/`; the other subfolders and `manifest.json` are outside it. This rule replaces D-019 rule 4 ("only a confirmed object is deleted") for `files/` of a Backup/export Scenario only.
6c. **No file rule.** A revision with databases and no file rule has no file plan (section 7b.4). Its source scope is empty, so every object in `files/` is outside it (owner decision 2026-11-06). Under the simple deletion rule, the run deletes each top folder of `files/` as one folder deletion (item 6b) and each top file as one file deletion, before the database dumps. It needs no server check. A `.qadren-` file, a symbolic link, and a special file stay. Under the paranoid rule, nothing is deleted. A failed or `UNKNOWN` effect makes the run `failed` with `files_cleanup_failed`. The Scenario editor shows this effect when no file is selected.
7. The run result is `complete` only when each selected resource has a verified result. Otherwise it is `partial` or `failed`. A partial run keeps the earlier content of the failed resources and is never shown as a successful backup.
8. This mode adds no runtime BackupSet, no content version history, and no retention.

### 6. Selection

1. The selection is a typed list of resources: category, stable provider ID, display hierarchy, and a recursive flag for a folder. It is not a list of file paths. A database or a mailbox is never written as a fake path.
2. An empty selection means nothing. "Select all accessible resources" is a separate, explicit flag. The D-019 rule "an empty selection means the complete root" does not apply to this mode.
3. A selected folder includes new descendants. It excludes siblings. Inside a selected folder, the user can clear a check box: this stores an **exclude** rule for that folder or file with all its content (owner decision 2026-11-04). The most specific rule wins, so a selected object inside an excluded folder is in the Scenario again. The walk does not read an excluded object. An excluded object is not copied, and its earlier copy in the destination is deleted (section 5.6a).
4. A selected resource that is missing, renamed, or not accessible at run time fails as one item with a reason. It never becomes a deletion.
5. A resource that appears under more than one group, for example a database under a site, has one canonical ID. A run exports it one time.

### 7. Execution

1. The run uses the existing durable Scenario Run, the stored plan, batch children, and retry classes (D-024, D-025). There is no second scheduler or executor.
2. The destination effect uses the Local Filesystem executor: a durable attempt, a staging name, a check of the byte count, then no-overwrite publish. SMB uses the QADREN mount (D-022).
3. Remote reads can be retried. The connector makes no remote mutation, so no remote effect needs reconciliation. Reconciliation after a lost proof checks only the local destination. Insufficient evidence keeps `UNKNOWN`.
4. A file streams from the panel to the staging name. QADREN does not use the main MacBook as an unapproved full-content mirror.

### 7a. File transport: SFTP first, panel API as the fallback

The owner decided on 2026-11-01: file content uses the transport with the best stability and coverage. When the account has SSH access, that transport is SFTP. When it has no SSH access, the panel API (`file`, `file.download`) is the transport.

1. **Scope.** The file transport serves the Files category and every category whose data exists only as files: SSH keys (`~/.ssh`), and service configuration files when the account can read them. Other categories use the panel API.
2. **Same account.** SFTP signs in with the login of the connection. By default it uses the same password and port 22. The connection form can set another port. QADREN never uses root, `su`, `sudo`, or another account.
3. **SFTP and one hash command.** The client opens the `sftp` subsystem for all reads. It never opens a shell. The owner allowed on 2026-11-04 one fixed command for a file hash: `md5sum -- '<path>'` on an `exec` channel, with the path in single quotes and every `'` escaped. The path has no control characters. QADREN runs no other command. When the command fails, the account has no shell, or the answer is not one MD5 for that path, QADREN reads the file one time through SFTP for its MD5. The download always checks the byte count and the MD5 before the publish, so a wrong answer publishes nothing.
4. **Read-only operations.** The client uses only open for read, read, readdir, stat, lstat, readlink, and close. It never writes, creates, removes, renames, or changes attributes.
5. **Host key.** The first connection shows the host key fingerprint. The user confirms it one time. The Registry stores the confirmed fingerprint for that `connection_id`; it is not a secret. A different key later stops all SFTP work for that connection with an error. QADREN never accepts a changed key silently.
6. **Choice per connection.** The connection check tries SFTP sign-in. The capability report states the file transport: `sftp`, or `panel_api` with the reason (no SSH access, sign-in refused, host key not confirmed). A run uses the transport that the last check confirmed and rechecks it before the effect.
7. **Links and special files.** SFTP gives `lstat` and `readlink`. A symbolic link is recorded as a link with its target; QADREN does not follow it out of the selection. A device, socket, or pipe is an omission with a reason. Nothing is skipped silently.
8. **Dependency.** SFTP needs an SSH client library in Rust Core. Choose a pure Rust library that uses the existing `tokio` and `ring` dependencies, and measure the binary size change (D-014) before the merge.

### 7b. Database dumps: panel facts, SSH dump, panel dump as the fallback

The owner decided on 2026-11-04: database servers differ in type, version, and configuration. QADREN reads these facts from the panel. It then makes the dump over SSH with the tool that matches the server. It uses the panel dump (`db.dump`) only when the SSH dump is not possible. QADREN acts only within the rights of the connection account. The owner does not enter a separate database password: QADREN gets every access fact from the panel.

1. **Panel facts.** The panel gives the database list (`db`), the users of one database (`db.users`, with `elid`), and the user form (`db.users.edit`, with `elid` and `plid`). The client opens the form without `sok`, so the panel saves nothing. The client never sends `sok`: the allowed parameters are `auth`, `username`, `password`, `elid`, and `plid`. The form enters the allowlist only for this read (section 3.2).
2. **Tool versions.** The connection check runs `mysqldump --version`, `mariadb-dump --version`, and `pg_dump --version` on an `exec` channel. These commands take no argument from the user. When the panel returns a password that is not a mask and `mysqldump` is present, the check signs in one time with `mysqldump --defaults-extra-file=/dev/stdin --no-data --single-transaction --skip-lock-tables --no-tablespaces --set-gtid-purged=OFF '<database>'` for the first database. It reads the table structure only, takes no lock, and discards the output. The user, the password, the host, and the port go on stdin as a `[client]` option file, never in the command line. These commands, the dump and its `gzip -1` pipe (item 6), and the table list query (item 10) are the only commands besides `md5sum` (section 7a.3).
3. **Live check.** The connection check records the result classes, the field names, the server type and version fields, the tool versions, whether the password field is a mask, and the result of the sign-in. It never stores, logs, or shows a password, a database name, or a user name. The live check on 2026-11-04 showed: the panel returns the password of a database user, and `mysqldump` signs in with it.
4. **Selection.** A database is a resource of category `databases`, kind `object`, with the database name as the provider ID. The name is also the file name, so it has only letters, digits, `_`, `-`, `.`, and `$`. A revision without a selected file or folder has no file plan: an empty file selection would mean the whole home folder.
5. **Access facts.** At the start of each export, QADREN reads the database, its first user with the read right, and that user's form from the panel. The facts stay in memory for this export only.
6. **Dump.** `mysqldump --defaults-extra-file=/dev/stdin --single-transaction --quick --skip-lock-tables --no-tablespaces --set-gtid-purged=OFF --routines --triggers --events --hex-blob --default-character-set=utf8mb4 --skip-dump-date '<database>'`. The dump is consistent for InnoDB tables and takes no table lock. Without the dump date, the same data gives the same bytes. The Mac writes the stream into a private staging file. The stored dump is not compressed: the owner decided on 2026-11-05 that compression of copies is part of a separate Scenario archive mode for files and dumps (#38). Only the transfer is compressed: when the probe `{ echo qadren; echo qadren-exit=0 >&2; } | gzip -1` gives the expected word, the command is `{ <mysqldump>; echo qadren-exit=$? >&2; } | gzip -1`. The Mac decompresses the stream and verifies the gzip check sum. The exit status of `mysqldump` must be the last stderr line, `qadren-exit=0`. Without `gzip` the stream is not compressed. The dump is complete only when `mysqldump` exits with 0 and the text ends with `-- Dump completed`.
7. **Publish.** The file is `databases/<database>.sql` in the copy root. When it has the same bytes as the destination file, there is no effect. Otherwise QADREN saves a durable attempt with the size and the MD5, then publishes with no-overwrite (a new file) or with the approved replacement protocol (an existing file). A lost proof gives `UNKNOWN` and local reconciliation of the copy root.
8. **Run.** The databases follow the files of the run, one after another, in their own thread. Each database gets a durable result: `created`, `updated`, `unchanged`, `failed`, or `cancelled`. A failed dump keeps the previous file and makes the run `failed` with `databases_failed`. A stop ends the current dump before its publish. `manifest.json` lists the selected databases and their results.
9. **Fallback.** When the SSH dump is not possible (no tool, no shell, or no access fact from the panel), the export will use `db.dump`. This fallback is not implemented yet. Until then the resource is an error with its reason (section 5.5).
10. **Table rules.** The owner decided on 2026-11-05: a table of a selected database can have the rule `structure_only`. The resource is `databases`, kind `object`, provider ID `<database>/<table>`. The rule needs its selected database. The Scenario editor lists the base tables with their estimated sizes from one read-only query: `mysql --defaults-extra-file=/dev/stdin --batch --skip-column-names -e "SELECT TABLE_NAME, DATA_LENGTH+INDEX_LENGTH, TABLE_ROWS FROM information_schema.TABLES WHERE TABLE_SCHEMA = DATABASE() AND TABLE_TYPE = 'BASE TABLE'" '<database>'`. The dump then has two parts in one file: the data dump with `--ignore-table='<database>.<table>'` for each rule, and a second `mysqldump --no-data` of only these tables. The tables without data have no consistency need, so the two parts do not need one transaction. Each part must end with `-- Dump completed`. Use: cache, sessions, logs, and statistics, which often make most of a large database. A table name with `/`, `\`, or a control character cannot have a rule.
11. **Large databases: next stage.** The owner accepted on 2026-11-05 the direction for databases of several gigabytes. One transaction over a long transfer has these problems: a break starts the dump again; InnoDB keeps old row versions for the open snapshot; DDL during the dump can stop it; the full SQL text goes over the network each run. Items 6 and 10 reduce them. The next stage, not implemented yet:
    1. **One file for each table.** The Mac splits the one consistent stream at the table markers of `mysqldump` into `databases/<database>/<table>.sql` and one file for the schema, views, routines, triggers, and events. Only a table file with a different MD5 is written. `manifest.json` records the load order. This saves destination writes and allows the restore of one table. It does not reduce the transfer.
    2. **No transfer of unchanged tables.** Before the dump, `CHECKSUM TABLE` gives a check sum for each table. One `--single-transaction` dump reads only the changed tables; the unchanged table files stay. After the dump, the check sums of the unchanged tables are read again; a table with a different check sum is dumped again. The result is a near-consistent copy, not a proven one: a table can change and change back between the checks. `CHECKSUM TABLE` reads each table on the server. The owner accepted this limitation for this stage; the user interface and `manifest.json` must disclose it.
    3. **Not in scope.** A dump into a temporary file on the server, with a resumable SFTP download, conflicts with section 7a.4 (no server writes) and uses server disk space. It needs a separate owner decision.

### 8. Native ISPmanager backups

This stage does not use native ISPmanager backups (`backups.*`, `backup.plan.*`, `backup2.*`). They create archives on the server, use server disk space, or bundle more data than the selection. A later owner decision can allow one of them after a live check of its exact content and side effects. The native backup does not include Docker.

### 9. Secrets in exported data

SSL private keys and other secrets that a panel can export are `unsupported` in this stage. A later owner decision can allow them through a verified, secure export method. QADREN never claims that a password is recoverable from a database grant or an account list.

## Owner questions

1. Approve the destination layout and the update rules in section 5.
2. Approve the exclusion of native backups (section 8) and of exported secrets (section 9) in this stage.
3. Approve the SSH client library for section 7a (D-014).
4. Give a test panel and test accounts (a user and an administrator) for Gate 0. The live checks run on the owner Mac with the read-only probe. Real names, paths, and counts stay outside the repository and the work tracker.

## Consequences (after approval)

- AGENTS.md, `llms.txt`, the roadmap, the synchronization model, the storage adapter contract, and the data-safety policy state this scope.
- The Settings design contract gets the ISPmanager connection form, the capability list, and the typed resource browser.
- Existing Local Filesystem, SMB, and Yandex Disk Scenarios do not change.
