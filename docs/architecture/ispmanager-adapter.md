# ISPmanager Adapter

[D-027](../decisions/D-027-ispmanager-backup-source.md) owns the scope. Status: **proposed**. The adapter is a read-only backup source. ISPmanager is never a destination.

## Sources

- [ISPmanager API](https://www.ispmanager.com/docs/ispmanager/ispmanager-api)
- [API authentication](https://www.ispmanager.com/docs/ispmanager/guide-to-ispmanager-software-api)
- [COREmanager API](https://www.ispmanager.com/docs/ispmanager/coremanager-api-functions)
- [Backup system](https://www.ispmanager.com/docs/ispmanager/backup-system-general-information)
- [Database export](https://www.ispmanager.com/docs/ispmanager/import-and-export-the-database)

The published catalog is generated. The documentation states that the function and parameter names depend on the panel configuration. A function in this document is a candidate. Only a live check on a panel proves it.

## Security contract

- Authenticate with `func=auth`, `username`, and `password` in an HTTPS `POST` form body. The panel returns a session ID. Send it as `auth` in the body of later requests. The documentation shows `authinfo=login:password` and credential-bearing GET URLs; QADREN does not use them.
- A session is valid for one hour without requests. On an expired session, authenticate again one time.
- Request `out=json`. Treat a response without the expected structure as a provider error.
- Use validated TLS with the standard trust roots. Follow no redirect.
- Never send `su`. An administrator can act as any user through `su`; QADREN uses only the rights of the configured account.
- Keep the password in the Keychain, bound to `connection_id`. Keep the session ID in memory only.
- Call only functions in the read-only allowlist (D-027 section 3).

## Capability states

`verified_export`, `verified_metadata_only`, `permission_denied`, `unsupported`, and `unverified` (D-027 section 4). The states are different. The interface and the manifest never treat one as another.

## Capability matrix (documentation only)

No live check exists yet. Every category is `unverified`. "Candidate functions" are read-only functions from the documentation. "Open questions" are the items that a live check must answer before a state changes. Live results go into the work tracker, without real data.

| # | Category | Candidate read functions | Content or metadata | Open questions |
|---|---|---|---|---|
| 1 | Files and directories | SFTP when the account has SSH access (D-027 section 7a); otherwise `file` (list, key `name`), `file.download` | Content | How `file` addresses a nested folder (`plid` or path). Pagination of large folders. Dotfiles, symlinks, special files, owner and mode fields. Whether `file.download` of one file streams directly or packs an archive on the server. Download of a folder. |
| 2 | Websites and web domains | `webdomain` (key `name`), `webproxy` | Metadata | Which fields the list returns. Whether a read-only view of site settings exists without `*.edit`. |
| 3 | Databases | `db` (key `pair`), `db.server`, `db.users`, `db.dump` | Content and metadata | Whether `db.dump` selects exactly one database. Whether it writes a temporary file on the server. Format and compression. Consistency of the dump. Grants of `db.users`. Passwords are never recoverable. |
| 4 | Mail domains | `emaildomain` (key `name`), `blacklist` | Metadata | Which delivery, DKIM, and DMARC fields are readable. |
| 5 | Mailboxes | `email` (key `name`), `email.sorter`, `email.responder`, `email.setting` | Metadata; content unknown | The documentation shows no mailbox content export. Mailbox content stays `unverified` until a live check finds an exact, read-only path. Whether `email.responder` and `email.setting` change state without `sok`. |
| 6 | Cron | `scheduler` (key `key`) | Metadata | Fields: command, schedule, status, owner. Never call `run`, `scheduler.suspend`, `scheduler.resume`, or `scheduler.delete`. |
| 7 | DNS | `domain` (key `name`), `domain.record` (key `rkey`), `domain.dnssecinfo`, `techdomain`, `techdomain.records` | Metadata | Record fields and TTL. Name servers. DNSSEC data. Whether `domain.record` needs `plid`. |
| 8 | SSL/TLS | `sslcert` (key `key`), `sslcert.csr` | Metadata | Whether the certificate and the chain are readable. Private keys are `unsupported` in this stage (D-027 section 9). |
| 9 | FTP | `ftp.user` (key `name`) | Metadata | Roots and rights. FTP passwords are not exportable. |
| 10 | ISPmanager accounts | `user` (key `name`), `admin`, `userrights`, `userstat`, `preset` | Metadata | Access level for each list. Template and limit fields. |
| 11 | SSH | No API function for SSH keys or shell settings. `file` for `~/.ssh` | Content through files | SSH access of the account does not give an API function. Whether `file` can read `~/.ssh/authorized_keys` for the account. Otherwise `unsupported`. |
| 12 | Web server services | `services` | Metadata | Configuration files are readable only through `file` with administrator rights. |
| 13 | PHP and runtimes | `phpversions`, `phpextensions`, `phpconf` | Metadata | Access level. Effective `php.ini` files through `file`. |
| 14 | Mail service settings | No dedicated read function found | Unknown | Configuration files through `file` with administrator rights. |
| 15 | Network and security | `firewall`, `ipdb`, `ipdb.addr`, `netactconn` | Metadata | Access level. |
| 16 | Docker | `docker_mysql` (MySQL versions only) | Unknown | No documented export of containers or volumes. The native backup does not include Docker. Expected `unsupported`. |
| 17 | Panel and backup settings | `license.info`, `plugin`, `backup.storages`, `backup.plan`, `backup.settings`, `emailnotify` | Metadata | Whether a form call without `sok` is read-only. Storage settings can contain secrets; they are never exported. |
| 18 | Logs and statistics | `userlogs`, `userlogs.download`, `authlog`, `traffic_report`, `accounting_stat` | Content and metadata | Whether `userlogs.download` selects exactly one log. Size of large logs. |

## Native backups

`backups.*`, `backup.plan.*`, and `backup2.*` are outside this stage (D-027 section 8). For example, `backups.new` makes a full backup on the server, and `backup.plan.run` runs a backup plan.

## Gate 0 probe

`qadren-ispmanager-probe` is a read-only command-line probe. The owner runs it on the owner Mac.

- It reads the password from standard input, never from an argument. On a terminal, echo is off. Example: `security find-generic-password -s <item> -a <login> -w | cargo run --manifest-path core/Cargo.toml --bin qadren-ispmanager-probe -- --url https://panel.example.com:1500/ispmgr --login <login>`.
- It assumes the documented `out=json` shape: `doc.elem` for list elements, `doc.error.$type` for an error, and `doc.auth` for the session ID. A `MalformedResponse` result means that the assumption is wrong for that panel.
- It authenticates through `POST`, then calls only the list functions of the allowlist.
- For each function it prints the result class (`ok`, `empty`, error type, HTTP status) and the field names of the first element. It prints no values, no counts, and no session ID.
- It downloads nothing in this version.

The probe output contains no user data by design. Check it before you post it to the work tracker.

## Destination effect

The Local Filesystem executor owns the destination effect (D-027 section 7). The destination is one current copy that each run updates. Each data type has its own subfolder in the destination root: `files/`, `databases/`, `mailboxes/`, `cron/`, and the other category keys (D-027 section 5). `files/` keeps the path below the home folder of the account, which is the source root of the Scenario.
