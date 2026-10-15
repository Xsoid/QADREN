# Repository Structure

Status: historical map of the initial structure; the current `main` contains more modules. Do not create new directories and packages without necessity. Yandex Disk and Local FS are active; iCloud is postponed.

```text
QADREN/
├── README.md
├── AGENTS.md
├── docs/
│   ├── README.md
│   ├── architecture/
│   │   ├── overview.md
│   │   ├── backup-model.md
│   │   ├── nodes-and-delegation.md
│   │   ├── storage-adapters.md
│   │   └── technology-stack.md
│   ├── decisions/
│   ├── development/
│   ├── policies/
│   └── roadmap.md
├── core/             # Rust workspace; create when coding starts
├── platform/macos/   # SwiftUI app + platform checks Local FS/Keychain
└── tests/            # general scenarios if necessary
```

This is a map of responsibility ownership, not a requirement to fill all directories in advance. You can combine crates if separation has no independent value. Do not create excessive bootstrap boilerplate.

## Dependency Direction

- UI -> thin layer of commands/state -> Core.
- Core -> abstract Storage Adapter contract, but not a concrete macOS UI.
- Postponed macOS iCloud implementation does not create a Release 1 dependency; see D-017. Current Local FS uses platform checks and directory access.
- Yandex adapter -> official API; does not depend on SwiftUI.
- Tests -> contract and behavior, not internal UI details.

## Source of Truth

Documentation is in **this** repository and is versioned with the code. An external AI controller can refer to it, but it must not maintain an alternative source of QADREN rules.
