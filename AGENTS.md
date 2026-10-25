# QADREN — agent rules

## Authority and scope

- The owner's explicit product decisions, active ADRs, and safety policies have priority.
- Use `llms.txt` and `docs/README.md` as the project indexes. The linked canonical document owns each detailed contract.
- Do not duplicate architecture, UI, release, test, or workflow requirements in this file.
- If code and a canonical document disagree, report the conflict. Do not silently change the architecture.
- Work directly in `main`. Do not create branches, PRs, or deployment actions unless the owner asks.

## Safety invariants

- Keep Rust as the cross-platform core and SwiftUI as the macOS UI.
- Release 1 uses Local Filesystem and SMB network drives (D-022: QADREN mount, Local Filesystem executor, Core mount gate). A one-way Scenario can use Yandex Disk as the source and a Local Filesystem root as the destination (D-023). Yandex Disk as a destination and iCloud need a separate owner decision. Keep File Provider safety guards.
- Use one Rust JobStore. A `Scenario` defines synchronization rules. A `Job` is one execution or one manual file operation.
- Keep each `StorageConnection` independent. Bind OAuth and Keychain state to `connection_id`.
- Do not save secrets in SQLite, Git, logs, or HTTP URLs.
- Remove and Disconnect change only local state. They must not delete provider objects.
- If an operation cannot stay within selected files, disclose its real scope and limitation. Never claim narrow effects when broader files may change.
- Read needed content silently for an enabled/requested operation, including macOS File Provider materialization. Do not add a separate `read_content` consent prompt or unrelated prefetch.
- Before a modifying external effect, save a durable attempt; check capabilities, exact operation scope, destination state, no-overwrite for new files, and explicit consent for destructive replacement/deletion where required.
- When proof of an effect is lost, automatically perform non-mutating reconciliation. Persist a verified outcome if evidence suffices; otherwise keep `UNKNOWN`. Never blindly replay destructive effects.
- A Local Filesystem update can replace an existing destination. It must use the
  approved replacement protocol, a durable attempt, explicit operation scope,
  and `UNKNOWN` reconciliation. The owner removed the previous Release 1
  fail-closed block for this operation on 2026-10-17.
- Do not add content versioning, runtime BackupSet, scenario dependencies, or remote Worker protocol to Release 1.

## Documentation language

Write new and changed project documentation, `llms.txt`, and future external work-tracker text in English and ASD-STE100 style. Use short active sentences and exact technical names. Do not claim full ASD-STE100 compliance without an approved-dictionary check. Speak to the owner in Russian. Do not rewrite external work-tracker history unless the owner asks.

## macOS UI work

- Before a SwiftUI change, read the matching contract in `docs/design/` and follow the visual-development and skill-use procedure in `docs/design/implementation.md`.
- The QADREN design system remains the canonical source. External skills can inspect or implement the existing contract. They must not replace its tokens, components, SwiftUI implementation, or product decisions.

## AI Docs Hub boundaries

The Hub indexes, routes, verifies, and reviews. It does not define QADREN architecture. It can update only its managed blocks in this file and local project metadata. Do not index secrets, user content, transfer artifacts, or local databases.

<!-- BEGIN AI DOCS HUB PROJECT LIFECYCLE -->
## AI Docs Hub Project Lifecycle

- Before commit, push, or PR, first perform the Hub pre-publish gate for the current patch fingerprint.
- Code review and security review refer to the exact version of the diff; after the patch changes, they become stale and you must repeat them.
- The Hub performs deterministic verification and stores local-only metadata; semantic review starts explicitly through Codex.
- Do not publish a patch if there is failed verification, stale/incomplete review, or an unresolved blocking finding.
- For the qadren project, use project-scoped Hub tools and do not mix context from other projects.
<!-- END AI DOCS HUB PROJECT LIFECYCLE -->

<!-- BEGIN AI DOCS HUB CODEBASE MEMORY -->
## Codebase Memory

- For this project, Codebase Memory is used only with the project scope `qadren`.
- Search for documentation, ADR, runbooks, and project policies first through AI Docs Hub.
- For symbols, definitions, calls, dependencies, routes, and blast radius, first use the Codebase Memory MCP.
- Standard `rg` and file reading remain the fallback if graph coverage is incomplete or if the source of truth is needed.
- `manage_adr`, cross-project wildcard, `delete_project`, `ingest_traces`, and repository persistence are forbidden.
- If documentation and the code graph disagree, check the source code and explicitly report the conflict with paths.
- After the first connection or a change to the MCP configuration, restart Codex so the new session loads the server tools.
<!-- END AI DOCS HUB CODEBASE MEMORY -->
