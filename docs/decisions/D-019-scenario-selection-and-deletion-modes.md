# D-019 — Scenario Object Selection and Deletion Modes

**Date:** 2026-10-23. **Status:** accepted; rule 4 partially revised by [D-025](D-025-autonomous-scheduled-scenarios.md) (2026-10-28). **Extends:** D-018 rules 2 and 4. Safety policies, D-016, and D-018 remain in force.

## Decision

1. **Inline folder browser.** The Scenario editor shows the folder structure of each endpoint connection in the editor itself. It has breadcrumbs and a list. The user makes the visible folder the Scenario root. There is no separate folder window and no system picker step. The browser reads names, types, and sizes only.
2. **Selected objects.** In the source panel, checkboxes select objects inside the root. The selection belongs to the immutable Scenario revision.
   - An empty selection means the complete root folder.
   - A selected folder includes all its content, also new content.
   - A selected file is that file only.
   - A renamed object and a new object next to selected objects are not in the Scenario.
   - In a two-way Scenario, the same relative paths apply to both sides. Only the left panel has checkboxes.
   - The destination keeps the folder structure: `Source/Docs/a.txt` becomes `Destination/Docs/a.txt`.
3. **Deletion modes.** Each revision has one deletion mode:
   - **Paranoid** (default): every deletion waits for an explicit decision and consent.
   - **Simple synchronization**: when a side deletes an object, QADREN deletes the same object on the other side. This is a durable, preapproved Scenario rule. The user confirms it one time when the mode is selected.
4. **Conditions for an automatic deletion.** All conditions must be true:
   - QADREN confirmed the object in an earlier synchronization of this revision.
   - The object is inside the selection.
   - The object did not change on the other side after that synchronization. Otherwise, the plan shows a conflict.
   - The plan has 20 deletions or fewer. With 21 or more deletions, all deletions of that plan wait for an explicit decision. The threshold is fixed. *Revised by D-025 section 5: Simple synchronization has no threshold. A missing or unreadable source root, or a source snapshot that is not complete, never gives a deletion.*
   - A missing root folder, for example a disconnected disk, is an error. It is never a deletion.
5. **Trash of the connection.** An automatic deletion moves the object into the Trash of the connection when the adapter has a `trash` capability. When the connection or volume has no Trash, the deletion is permanent. Every connector design must examine deletion into the provider Trash.

## Rationale

The owner asked for one window that shows the folder structure and lets the user select several objects for synchronization. The owner also asked for a mode where a deletion in the source deletes the file in the destination, next to a mode that asks for every deletion. The data-safety policy allows deletion under an approved, exact, durable rule. It requires explicit confirmation of a mass deletion.

## Implementation boundaries

- The planner removes paths outside the selection from the plan. A path that differs from a selected object only by letter case or Unicode form is `NeedsVerification` and never executes.
- Every Scenario executor rechecks the selection of its revision before an effect. The rule-deletion executor also rechecks the mode, the confirmed baseline, the absence of the object on the other side, and the unchanged object on its own side.
- The interface does not send paths for rule deletions. It asks the Worker to run the rule deletions of the current revision. The Worker plans again, so the threshold cannot be bypassed.
- A rule deletion has a durable attempt and a recorded scoped decision. An interrupted effect is `UNKNOWN`. Read-only reconciliation completes it only when the object is absent from its path. QADREN never repeats a deletion.
- After a verified deletion, QADREN removes the confirmed baseline row of that path.
- Local Filesystem on macOS moves a file into the home `.Trash`, or into `.Trashes/<uid>` of another volume when that volume has `.Trashes`. On other operating systems, the Local Filesystem adapter has no `trash` capability yet.

## Affected documents

- [Settings window](../design/settings.md)
- [Data safety](../policies/data-safety.md)
- [Storage adapters](../architecture/storage-adapters.md)
