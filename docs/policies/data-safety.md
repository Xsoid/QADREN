# Data Security and Result Verification

Status: product policy v0.2, implementation of separate mechanisms requires tests.

## Guarantee Boundaries

QADREN performs synchronization. It **does not store its own previous versions of content** in MVP. It does not guarantee the recovery of overwritten or permanently deleted files. BackupSet, retention, and recovery of historical versions are deferred: [project of future capabilities](../architecture/backup-model.md).

## Human Decision on Residual Risk

QADREN does not promise risk-free synchronization. The user chooses whether to proceed when the consequence is known or can be explained and bounded. Do not indefinitely block a feature to eliminate every theoretical OS, provider, or data-loss possibility.

- Present an accurate preview of the **actual selected operation**. State the affected source and destination, operation type, number of files and estimated bytes if known, whether existing content can be replaced or deleted, any potential local download, and whether QADREN can restore the old content.
- If risk is significant, request an explicit **Proceed / Cancel** choice before the effect. No response, stale consent, blanket terms acceptance, or an old decision is not permission. Make cancel the safe default. Present unresolved conflicts and bulk destructive changes as user decisions, not silent automatic actions.
- The user can approve a bounded batch based on its exact plan, roots, connections, and revisions. A changed plan or new destructive scope requires fresh consent. A potential QADREN-initiated download onto the main MacBook still needs its own per-source-file explicit `read_content` permission under [local-downloads.md](local-downloads.md).
- Explain uncertainty honestly. If a native File Provider read **may** materialize content, say so and ask permission. Do not require proof of an impossible negative before permitting a specifically authorized read. On refusal, do not read content.
- For unattended execution, only act within a previously approved, specific, durable policy and scope. Any new risk that needs a live choice remains `WAITING_FOR_DECISION` until the user responds.
- Consent authorizes a **known, bounded risk**. It does not authorize writes outside the chosen roots, execution with stale identities, permission bypass, unauthorized download, unbounded side effects, blind retry after `UNKNOWN`, or fabricated verification. If the system cannot enforce operation boundaries, block that operation and explain why.
- The user accepts the described residual consequence. QADREN remains responsible for showing truthful information, following the approved scope, and distinguishing VERIFIED, LIMITED, and UNKNOWN results. Approval is not a waiver of engineering responsibility.

## Security Requirements

- Apply scenario-based conflict and deletion policies; detect mass deletions, ambiguous overwrites, and dangerous discrepancies. Do not allow silent destructive actions.
- For a known, bounded risk, show the affected operation and request consent in its context. Stop only until a required decision is made or a technical operation boundary cannot be enforced. Mass resolution of similar conflicts and creation of scoped scenario rules are permitted.
- Use the provider trash where it is available; do not claim that it provides a backup history, and do not create a separate trash.
- Name/size/time metadata **is not proof of byte equality**. Use available remote identifiers, ETag/revisions/server hashes; explicitly indicate when reliable verification is unavailable.
- Store the last confirmed agreed state of synchronized files. This is a service history for change detection and idempotency, **not an archive of old content**.
- The executor distinguishes between its own report, write confirmation by the provider, and actually verified integrity. Do not replace UNKNOWN with FAILED or SUCCESS.
- After a failure, verify the fact of previous execution before repeating a potentially destructive operation.
- When cleaning user history, do not delete information required for incomplete tasks, synchronization control, and action confirmation.

## Main MacBook Limitations

Follow the [local downloads policy](local-downloads.md): QADREN does not download content to the main MacBook without the explicit consent of a specific operation. No diagnostics or hashing is an exception. Do not interfere with the native operation of iCloud/macOS and other applications.
