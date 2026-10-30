# Storage Adapters

## Connection model

An adapter type is not a connection instance. Each `StorageConnection` has an independent ID, settings, credential reference, status, and error history. The same provider account may have more than one independent connection. See [D-016](../decisions/D-016-multiple-connections.md).

## Capability contract

An adapter declares its supported OS, permissions, metadata access, content read and write operations, streaming, integrity evidence, change listing, deletion, Trash, and recovery capabilities. A capability declaration is not permission to perform I/O. The executor rechecks actual permissions, object state, and policy before every effect.

### Deletion into the Trash

The `trash` capability means that a deletion can move an object into a recoverable Trash of the storage. A Scenario deletion under the Simple synchronization rule uses the Trash when the adapter declares it and the target location has one. Otherwise, the deletion is permanent (D-019). **Every new connector design must examine deletion into the provider Trash** and declare `trash` only for a verified, recoverable path.

- Local Filesystem on macOS: the home `.Trash` for the home volume, and `.Trashes/<uid>` on another volume that has `.Trashes`. The move is descriptor-relative, does not follow links, and never replaces a Trash entry.
- Local Filesystem on Linux and Windows: no `trash` capability yet.
- SMB share ("Сетевой диск (SMB)", D-021): no verified recoverable Trash for a client. No `trash` capability.
- Yandex Disk (destination is stage 2 of D-023): the REST API moves a deleted resource into the Disk Trash unless `permanently=true`.

## Release 1 adapters

Local Filesystem is the only Release 1 executor. An SMB share reaches it through a QADREN mount (D-022). It reads selected source content without an extra `read_content` prompt. It must validate roots and paths, keep new destinations no-overwrite, use the approved replacement protocol for existing destinations and keep the replaced version in the Trash of the volume where it exists (D-025), and reconcile uncertain effects without mutation.

The SMB share connector (`smb_share`, [D-021](../decisions/D-021-smb-network-drive-connector.md), [D-022](../decisions/D-022-smb-in-release-1-scenarios.md)) stores an address, a Keychain login and password, and a status. Scenarios use it through a QADREN mount at `~/.qadren/mounts/<connection_id>` and the Local Filesystem executor. Core refuses an SMB plan or effect unless that directory is an `smbfs` mount point now (`storage_access`). Write rules for smbfs (flush, publish by reservation, name and identity checks) are in D-022, item 9. Large transfers (Transfer Runs, stored snapshots and plans, retry classes) follow [D-024](../decisions/D-024-large-transfers.md). The Worker mounts the share itself when a run needs it, with a login that the app gives it in memory ([D-025](../decisions/D-025-autonomous-scheduled-scenarios.md)).

Yandex Disk is a one-way Scenario source to a Local Filesystem root ([D-023](../decisions/D-023-yandex-disk-to-mac.md), [Yandex Disk adapter](yandex-adapter.md)). Yandex Disk as a destination needs stage 2 of D-023. iCloud is post-release ([D-017](../decisions/D-017-defer-icloud.md)). File Provider materialization caused by a required Local Filesystem read does not create an iCloud adapter and does not allow unrelated prefetch.

## New adapters

A new adapter design includes the Trash examination above.

New adapters are independent of Rust Core and declare their own platform limits. They must not create a second JobStore, bypass policy, or transfer macOS-specific dependencies to other platforms.
