# D-021 — "Сетевой диск (SMB)" Connector

**Date:** 2026-10-24. **Status:** accepted. **Revises:** D-020 item 1 (the list of known connectors). D-016 applies to this connector without change.

## Decision

1. **Connector.** QADREN has a third connector: "Сетевой диск (SMB)" (`smb_share`). It connects one SMB share: a NAS, a Windows computer, or another Mac. The connector name states the protocol. A WebDAV share will be a separate connector.
2. **Any number of connections.** Each SMB connection is an independent `StorageConnection` (D-016). Two connections can use the same server, share, or login.
3. **Address.** The Registry stores the share address `smb://host[:port]/share[/folder]` in `storage_connections.endpoint_url`. Core refuses an address that has a login (`user@`), a domain (`domain;`), a query, a fragment, a backslash, or no share name.
4. **Mouse choice.** The form shows the SMB shares that macOS has mounted and the SMB servers that Bonjour (`_smb._tcp`) finds in the local network, as Finder does. A click on a mounted share fills the address and the login from the macOS remount URL; the address never keeps the login. A click on a server opens it in macOS: macOS asks for the login and the share in its own dialog and mounts the share. "Выбрать папку…" opens the system folder panel; a folder inside a mounted share gives its address. Discovery reads service names only. The app declares `NSLocalNetworkUsageDescription` and `NSBonjourServices`. A Bonjour host name can contain spaces, for example `smb://My NAS._smb._tcp.local/share`.
5. **Login and password.** QADREN remembers the login and the password of each connection and uses them to sign in. They are in one Keychain item of that `connection_id` (service `dev.qadren.smb.credentials`). They never enter SQLite, the address, logs, or the interface after input.
6. **Login check.** "Проверить и сохранить" and "Проверить вход" mount the share through macOS NetFS without a system dialog. The check before the first save uses `MNT_DONTBROWSE` and unmounts its mount; "Проверить вход" uses the QADREN mount of [D-022](D-022-smb-in-release-1-scenarios.md). The check reads no file content and changes nothing on the server. When macOS already has the share mounted, the result is "connected, login not rechecked". A failed check can be saved without the check; the status then shows the real result.
7. **Statuses.** `connected`, `requires_auth` (the server refused the login), `offline` (no answer), `error` (no share or another error), `disconnected` (not checked).
8. **Removal.** Remove deletes only the local record and its Keychain item, through the same disable, unresolved-attempt gate, and Keychain-first order as Yandex Disk. It never changes files on the share.

## Boundaries of this step

- Revised by [D-022](D-022-smb-in-release-1-scenarios.md): Scenarios can use the connector through a QADREN mount.
- Trash examination (required for a new connector): an SMB share has no verified recoverable Trash that a client can use. The connector does not declare `trash`.

## Affected documents

- [D-020](D-020-single-this-mac-connection.md)
- [Storage adapters](../architecture/storage-adapters.md)
- [Settings window](../design/settings.md)
