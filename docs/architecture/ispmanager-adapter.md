# ISPmanager Adapter

[D-027](../decisions/D-027-ispmanager-backup-source.md) owns the scope. Status: **accepted**. The adapter is a read-only backup source. ISPmanager is never a destination.

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

## Capability matrix

The live checks used user accounts only (up to 2026-11-14). No administrator account was available. A check with administrator rights is blocked: a category that a user account cannot read stays `unverified` or `permission_denied` until an administrator check. "Functions" gives the result of each read function for a user account: `items`, `empty`, `denied` (panel error `access`), `missing` (panel error `missed`), or `error`. "State" is the state of the category for a user account. `verified_export` means that a Backup/export run on a live panel copied the data to the destination. Live results go into the work tracker, without real data.

| # | Category | Functions (user account) | State | Export | Gaps and open questions |
|---|---|---|---|---|---|
| 1 | Files and directories | SFTP; `file`: `items` | `verified_export` | SFTP to `files/` with exclusions and the mirror rule (D-027 section 7a) | The `file` API without SSH access is not used. A symbolic link and a special file follow D-027 section 7a.7. |
| 2 | Websites and web domains | `webdomain`: `items`; `webproxy`: `denied` | `verified_export` | `sites/<site>.json` with the site options (D-027 section 7c) | `webproxy` needs higher rights. |
| 3 | Databases | `db`: `items`; `db.users`, `db.users.edit`: read; `db.server`: `denied` | `verified_export` | `mysqldump` over SSH to `databases/<database>.sql`, with table rules (D-027 section 7b); without SSH, or when the SSH dump is not possible, the panel dump `db.dump` (D-027 section 7b.9) | The panel dump keeps no table without its data. From 700 MiB, one file for each table in `databases/<database>/` (D-027 section 7b.11.1); no transfer of unchanged tables is a next stage. Passwords are never recoverable. |
| 4 | Mail domains | `emaildomain`: `items`; `blacklist`: `denied` | `verified_export` | `mail/<domain>.json` (D-027 section 7c) | `blacklist` needs higher rights. |
| 5 | Mailboxes | `email`: `items`; `email.edit`: read; `email.sorter`: `error` without `elid`, `empty` with a mailbox as `elid` | Settings: `verified_export`. Content: `verified_export` as Maildir files | Mailbox settings and `forward`, `maxsize`, `note` from `email.edit` in `mail/<domain>.json`. Letters as files from `email/<domain>` (option "письма") | Letters are in `email/<domain>/<mailbox>/.maildir` (`cur`, `new`, `tmp`, and one folder for each IMAP folder). One mailbox is selected as its folder. A live check on 2026-11-15 compared two mailboxes in a copy with the server: the same letter files and sizes, and the same MD5 for a sample; the only difference was letters that the user moved after the run. A letter that the mail server moves during the walk can be missing until the next run. A restore into a mail server is not tested. `email.sorter` with a mailbox as `elid` works; no filter was found, so the filter fields are not known. The form has no autoresponder text. |
| 6 | Cron | `scheduler`: `items` or `empty` | `verified_export` | `cron/jobs.json` (D-027 section 7c) | Never call `run`, `scheduler.suspend`, `scheduler.resume`, or `scheduler.delete`. |
| 7 | DNS | `domain`: `items`; `domain.record`, `domain.record.edit`: read; `techdomain`: `denied` or `missing` | `verified_export` | `dns/<zone>.json` and the BIND file `dns/<zone>.zone` (D-027 section 7c) | CAA, TLSA, and DS records are comments in the zone file; the live check found no such record to read. DNSSEC keys are not read. |
| 8 | SSL/TLS | `sslcert`: `items`; `sslcert.edit`: read | `verified_export` | The list in `ssl/certificates.json`, and the certificate and chain in `ssl/<certificate>.pem` (D-027 section 7c). Each PEM file has its own run result | The form field `crtkey` has the private key; it is `unsupported` in this stage (D-027 section 9). |
| 9 | FTP | `ftp.user`: `items` or `empty` | `verified_export` | `ftp/users.json` (D-027 section 7c) | FTP passwords are not exportable. |
| 10 | ISPmanager accounts | `userstat`: `items`; `user`, `admin`: `denied` | Limits: `verified_export` | Plan limits to `accounts/limits.json`, without the current use (D-027 section 7c) | `user` and `admin` need administrator rights. |
| 11 | SSH | No API function | `unverified` | `~/.ssh` can be selected as a folder in `files/` | No separate SSH option. Whether SFTP can read `~/.ssh` is not checked for each account. |
| 12 | Web server services | `services`: `denied` | `permission_denied` | None | Needs administrator rights. Configuration files are outside the account home. |
| 13 | PHP and runtimes | `phpversions`, `phpextensions`: `items` | `verified_export` | Versions and their extensions to `php/runtime.json` (D-027 section 7c) | `phpconf` and effective `php.ini` files are not checked. |
| 14 | Mail service settings | No API function | `unverified` | None | Configuration files need administrator rights. |
| 15 | Network and security | `firewall`, `ipdb`: `denied` | `permission_denied` | None | Needs administrator rights. |
| 16 | Docker | `docker_mysql`: `missing` | `unsupported` | None | No documented export of containers or volumes. The native backup does not include Docker. |
| 17 | Panel and backup settings | `backup.storages`, `backup.plan`: `missing` | `unsupported` | None | The panel versions in the checks have no such functions. `license.info`, `plugin`, and forms without `sok` are not checked. Storage settings can contain secrets; they are never exported. |
| 18 | Logs and statistics | `userlogs`, `authlog`: `items` | `verified_metadata_only` | None. Deferred (D-027 section 7c.7) | A log inside the account home can be selected as a file. Whether `userlogs.download` selects exactly one log, and the size of large logs, are not checked. |

"Functions" for one category can differ between panels. A check that reads a category on one panel does not prove it on another panel; each connection check shows its own result.

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
