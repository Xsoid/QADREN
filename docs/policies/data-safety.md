# Data Security and Result Verification

Status: product policy v0.2, implementation of separate mechanisms requires tests.

## Guarantee Boundaries

QADREN performs synchronization. It **does not store its own previous versions of content** in MVP. It does not guarantee the recovery of overwritten or permanently deleted files. BackupSet, retention, and recovery of historical versions are deferred: [project of future capabilities](../architecture/backup-model.md).

## Risk Classification and User Decisions

QADREN shall not attempt to eliminate every theoretical risk before release. The user decides on **destructive consequences**, while QADREN must execute within a truthful, enforceable scope.

- **Ordinary read:** Read a needed file without an extra prompt. macOS may download a File Provider item as a side effect of reading. Do not present this as a risky operation or require per-file `read_content` consent. Do not prefetch unrelated content. See [local-downloads.md](local-downloads.md).
- **New file without overwrite:** Perform a requested or enabled Scenario operation using the JobStore, without an additional destructive-risk prompt if it stays within the configured scope. Do not silently overwrite an existing file.
- **Replacement:** Show the actual destination and loss-of-previous-content consequence; ask for confirmation unless a durable, explicit Scenario rule covers the exact destructive scope. A changed scope requires a new choice.
- **Deletion and mass deletion:** Show the actual files/count and irreversibility. Require a clear human confirmation for bulk deletion (for example, a proposal to remove 150 files), even in an automated Scenario. Do not infer consent from an enabled schedule.
- **Unknown result after failure:** Automatically reconcile by non-mutating checks without asking the user. Persist a verified outcome when evidence is sufficient; otherwise retain `UNKNOWN`. Never blindly replay a potentially destructive effect.
- **Unexpectedly broad scope:** If QADREN cannot limit an action to the selected files, tell the user exactly what is and is not controllable. Present the known broader effect and request approval when the effect is destructive. If the actual scope cannot be bounded or made truthful, do not execute it as a selected-file operation.

Consent is not a substitute for the implementation correctly targeting the described objects. Maintain operation/connection/root identity checks, durable attempts, outcome evidence, and permission checks. Explain limitations; do not fabricate `VERIFIED` results or claim that QADREN can restore overwritten content without version backups.

For scheduled or unattended work, carry out ordinary reads and unambiguously safe configured operations automatically. When new destructive consequences appear, save a `WAITING_FOR_DECISION` request and continue unaffected tasks.

## Security Requirements

- Apply scenario-based conflict and deletion policies; detect mass deletions, ambiguous overwrites, and dangerous discrepancies. Do not allow silent destructive actions.
- Ask only for approval of real destructive risks: replacement, deletion, bulk or conflict mutations beyond an approved rule. Do not interrupt ordinary reads. Mass resolution of similar conflicts and creation of scoped scenario rules are permitted.
- Use the provider trash where it is available; do not claim that it provides a backup history, and do not create a separate trash.
- Name/size/time metadata **is not proof of byte equality**. Use available remote identifiers, ETag/revisions/server hashes; explicitly indicate when reliable verification is unavailable.
- Store the last confirmed agreed state of synchronized files. This is a service history for change detection and idempotency, **not an archive of old content**.
- The executor distinguishes between its own report, write confirmation by the provider, and actually verified integrity. Do not replace UNKNOWN with FAILED or SUCCESS.
- After a failure, automatically check whether the previous effect occurred, using non-mutating reconciliation before considering a possibly destructive retry.
- When cleaning user history, do not delete information required for incomplete tasks, synchronization control, and action confirmation.

## Plan Freshness and Name Identity

- A confirmed replacement carries the metadata of both files from the plan that the user saw. The Worker refuses the replacement, before it creates a Job, if either file changed after planning. The user must check the plan again and give a new consent.
- An interactive request for a superseded Scenario revision is refused. Staged schedule Jobs and confirmed batch receipts keep their pinned revision.
- Two Local Filesystem roots of one Scenario must not be the same folder or be inside each other. The Registry refuses such a Scenario when it is saved. Executors still check the opened folders.
- Paths that differ only by letter case or Unicode form can name the same file on APFS. The plan marks them as "needs verification" and offers no Create or Update for them.

## Main MacBook Limitations

Follow the [content-read policy](local-downloads.md): file content needed by a requested/configured operation may be read without a separate prompt, including possible macOS File Provider materialization. This does not permit unrelated automatic mirroring or unsolicited prefetch. Do not interfere with native iCloud/macOS or other applications.
