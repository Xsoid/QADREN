# QADREN documentation

Read [D-018: Local-first priorities](decisions/D-018-local-first-priorities.md) first. It defines the Release 1 scope.

| Area | Canonical document |
|---|---|
| System design | [Architecture overview](architecture/overview.md) |
| Scenario and Job model | [Synchronization model](architecture/backup-model.md) |
| Storage connections | [Storage adapters](architecture/storage-adapters.md) |
| Local Worker | [Local Worker runtime](architecture/local-worker-runtime.md) |
| Product decisions | [Decision record](decisions/README.md) |
| Data safety | [Data safety](policies/data-safety.md) |
| Content reads | [Local content read policy](policies/local-downloads.md) |
| macOS interface | [Design system](design/README.md) |
| Build and delivery | [Build and release](development/build-and-release.md) |
| Validation | [Testing and quality](development/testing-and-quality.md) |
| Product sequence | [Roadmap](roadmap.md) |
| Agent rules | [AGENTS.md](../AGENTS.md) |
| Third-party licenses | [Third-party notices](../THIRD_PARTY_NOTICES.md) |

Use GitHub Issues for active work, audits, and evidence for a specific SHA. Do not add operational status reports to this documentation.

## Status words

- **Accepted**: An approved product or architecture rule.
- **Deferred**: Work outside the active scope.
- **Requires verification**: A behavior that needs evidence for a specific change.

When an accepted decision changes, update its decision record and the affected canonical contract. Do not replace history silently.
