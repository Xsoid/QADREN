# Storage Adapters

## Connection model

An adapter type is not a connection instance. Each `StorageConnection` has an independent ID, settings, credential reference, status, and error history. The same provider account may have more than one independent connection. See [D-016](../decisions/D-016-multiple-connections.md).

## Capability contract

An adapter declares its supported OS, permissions, metadata access, content read and write operations, streaming, integrity evidence, change listing, deletion, and recovery capabilities. A capability declaration is not permission to perform I/O. The executor rechecks actual permissions, object state, and policy before every effect.

## Release 1 adapters

Local Filesystem is the only active Release 1 adapter. It reads selected source content without an extra `read_content` prompt. It must validate roots and paths, keep new destinations no-overwrite, use the approved replacement protocol for existing destinations, and reconcile uncertain effects without mutation.

Yandex Disk and iCloud are post-release. Do not add API, OAuth, transfer, entitlement, UI, or live end-to-end work without a new owner decision. See [Yandex Disk adapter](yandex-adapter.md) and [D-017](../decisions/D-017-defer-icloud.md). File Provider materialization caused by a required Local Filesystem read does not create an iCloud adapter and does not allow unrelated prefetch.

## New adapters

New adapters are independent of Rust Core and declare their own platform limits. They must not create a second JobStore, bypass policy, or transfer macOS-specific dependencies to other platforms.
