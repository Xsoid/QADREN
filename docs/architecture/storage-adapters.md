# Storage Adapters

## Connection model

An adapter type is not a connection instance. Each `StorageConnection` has an independent ID, settings, credential reference, status, and error history. The same provider account may have more than one independent connection. See [D-016](../decisions/D-016-multiple-connections.md).

## Capability contract

An adapter declares its supported OS, permissions, metadata access, content read and write operations, streaming, integrity evidence, change listing, deletion, Trash, and recovery capabilities. A capability declaration is not permission to perform I/O. The executor rechecks actual permissions, object state, and policy before every effect.

### Deletion into the Trash

The `trash` capability means that a deletion can move an object into a recoverable Trash of the storage. A Scenario deletion under the Simple synchronization rule uses the Trash when the adapter declares it and the target location has one. Otherwise, the deletion is permanent (D-019). **Every new connector design must examine deletion into the provider Trash** and declare `trash` only for a verified, recoverable path.

- Local Filesystem on macOS: the home `.Trash` for the home volume, and `.Trashes/<uid>` on another volume that has `.Trashes`. The move is descriptor-relative, does not follow links, and never replaces a Trash entry.
- Local Filesystem on Linux and Windows: no `trash` capability yet.
- Yandex Disk (post-release): the REST API moves a deleted resource into the Disk Trash unless `permanently=true`.

## Release 1 adapters

Local Filesystem is the only active Release 1 adapter. It reads selected source content without an extra `read_content` prompt. It must validate roots and paths, keep new destinations no-overwrite, use the approved replacement protocol for existing destinations, and reconcile uncertain effects without mutation.

Yandex Disk and iCloud are post-release. Do not add API, OAuth, transfer, entitlement, UI, or live end-to-end work without a new owner decision. See [Yandex Disk adapter](yandex-adapter.md) and [D-017](../decisions/D-017-defer-icloud.md). File Provider materialization caused by a required Local Filesystem read does not create an iCloud adapter and does not allow unrelated prefetch.

## New adapters

A new adapter design includes the Trash examination above.

New adapters are independent of Rust Core and declare their own platform limits. They must not create a second JobStore, bypass policy, or transfer macOS-specific dependencies to other platforms.
