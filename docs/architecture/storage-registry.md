# Storage Connection Registry

## Purpose

`StorageConnection` is one local configuration instance for one adapter. Its stable `connection_id` is distinct from an account name and a display name. A Scenario refers to connections by stable ID.

Multiple independent connections may use the same adapter, provider account, endpoint, or credentials. The system must not merge or delete them automatically. See [D-016](../decisions/D-016-multiple-connections.md).

## Data and safety

The Registry stores metadata and non-secret credential references only. OAuth tokens, authorization headers, and credential-bearing URLs must not enter SQLite, Git, logs, or diagnostics.

OAuth state, Keychain state, status, errors, and capability information belong to one `connection_id`. Removing or disconnecting a connection changes local state only. It does not delete provider objects. Before local credential cleanup, the executor must preserve references required for incomplete or uncertain work.

Registry migrations are sequential and transactional. A newer unsupported schema fails without mutation. Do not overwrite a damaged database automatically.

## Boundaries

The Registry does not execute transfers. Job execution and durable effect attempts belong to the one Rust JobStore. Capability information is not a permission to perform I/O; the executor rechecks the actual connection, scope, permissions, and policy before each effect.
