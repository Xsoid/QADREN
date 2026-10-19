# QADREN documentation

Read [D-018: Local-first priorities](decisions/D-018-local-first-priorities.md) and the [current-state audit](development/current-state-audit.md) first.

The product specification is v0.2 (2026-10-09). Documents about BackupSet and content versions are historical. They do not apply to the MVP.

| Document | Purpose |
|---|---|
| [Design system and control surface](design/README.md) | Brand, visual tokens, SwiftUI components, widgets, windows, UX, and acceptance criteria |
| [Technology stack](architecture/technology-stack.md) | Rust, SwiftUI, SQLite, Yandex Disk, Local Filesystem, and distribution without an end-user runtime |
| [Engineering rules](development/engineering-rules.md) | Module ownership, reliability, and security rules |
| [Testing and quality](development/testing-and-quality.md) | Required checks and readiness criteria |
| [Build and release](development/build-and-release.md) | Native distribution and release rules |
| [Repository structure](development/repository-structure.md) | Code and documentation layout |
| [Agent instructions](../AGENTS.md) | Rules for Codex and other agents |
| [Architecture overview](architecture/overview.md) | Goals, components, and boundaries |
| [Scenario and execution model](architecture/backup-model.md) | Current execution model |
| [Nodes](architecture/nodes-and-delegation.md) | Controller, Worker, and delegation |
| [Local Worker runtime](architecture/local-worker-runtime.md) | Release 1 process lifecycle, local IPC, ownership, and recovery |
| [Storage adapters](architecture/storage-adapters.md) | Yandex Disk, Local Filesystem, and future iCloud status |
| [Local downloads](policies/local-downloads.md) | Consent rule for content downloads to the primary MacBook |
| [Data safety](policies/data-safety.md) | Copy proof, deletion, and recovery limits |
| [Decision record](decisions/README.md) | Accepted decisions and their reasons |
| [D-017: Defer iCloud](decisions/D-017-defer-icloud.md) | iCloud is outside the current release scope |
| [Roadmap](roadmap.md) | Current work queue |

## Status words

- **Accepted**: An agreed architecture rule.
- **Proposed**: An implementation option that is not accepted.
- **Requires verification**: An assumption about the system, an API, or an operating system.
- **Deferred**: Work outside the current design scope.

When you change an accepted decision, update the decision document and the decision record. Do not silently replace history.

## AI Docs Hub

The same rules apply with or without AI Docs Hub. Read [AGENTS.md](../AGENTS.md#unified-workflow-chat-codex-ai-docs-hub) and [engineering rules](development/engineering-rules.md#ai-docs-hub-integration-procedure-for-chat-and-codex).

Use the Hub only for navigation and quality checks. The Hub does not define QADREN product architecture. A project binding does not give permission to overwrite or generate project documentation.
