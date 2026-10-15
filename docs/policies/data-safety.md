# Data Security and Result Verification

Status: product policy v0.2, implementation of separate mechanisms requires tests.

## Guarantee Boundaries

QADREN performs synchronization. It **does not store its own previous versions of content** in MVP. It does not guarantee the recovery of overwritten or permanently deleted files. BackupSet, retention, and recovery of historical versions are deferred: [project of future capabilities](../architecture/backup-model.md).

## Security Requirements

- Apply scenario-based conflict and deletion policies; detect mass deletions, ambiguous overwrites, and dangerous discrepancies. Do not allow silent destructive actions.
- If there is a doubt, stop the affected operation and request permission in its context. Mass resolution of similar conflicts and creation of permanent scenario rules are permitted.
- Use the provider trash where it is available; do not claim that it provides a backup history, and do not create a separate trash.
- Name/size/time metadata **is not proof of byte equality**. Use available remote identifiers, ETag/revisions/server hashes; explicitly indicate when reliable verification is unavailable.
- Store the last confirmed agreed state of synchronized files. This is a service history for change detection and idempotency, **not an archive of old content**.
- The executor distinguishes between its own report, write confirmation by the provider, and actually verified integrity. Do not replace UNKNOWN with FAILED or SUCCESS.
- After a failure, verify the fact of previous execution before repeating a potentially destructive operation.
- When cleaning user history, do not delete information required for incomplete tasks, synchronization control, and action confirmation.

## Main MacBook Limitations

Follow the [local downloads policy](local-downloads.md): QADREN does not download content to the main MacBook without the explicit consent of a specific operation. No diagnostics or hashing is an exception. Do not interfere with the native operation of iCloud/macOS and other applications.
