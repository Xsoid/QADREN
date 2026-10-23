# QADREN — UI/UX Design System

Status: **mandatory source specification for SwiftUI implementation**. The visual design direction is "cyberpunk instrument panel". Token values define the approved baseline. Patch-specific visual acceptance belongs in GitHub Issues.

![Original QADREN logo](../../assets/qadren-logo.png)

## Entry Point and Priority

1. [Visual Language and Brand](visual-language.md) + [machine-readable tokens](tokens.json).
2. [Atomic and composite components](components.md).
3. [Grid, registry and widget life cycle](widget-system.md).
4. [Monitoring panel](monitoring.md) and [settings panel](settings.md).
5. [Implementation and acceptance contract](implementation.md).
6. [Static visual prototype](prototype.html) — **demonstration**, not application behavior and not an alternative specification.

In case of discrepancy: safety from `docs/policies/` and domain solutions from `docs/architecture/`/ADR are higher priority than appearance. Within the design system, Markdown behavioral contracts have priority over the HTML example. The actual current `main` is verified before the code; the absence of a function is shown as unavailable/stub, not as "ready".

## Do Not Change During Implementation

- Monitoring — **a canvas of interactive widgets without a permanent Sidebar or section navigation**. The fixed widget set supports order and density changes in layout mode.
- Two work windows are defined: monitoring and settings. Do not add a separate file-manager window. Scenario Settings owns local folder selection through `LocalWorkspace`. The macOS menu is the entry point and for quick actions. Closing the UI is not equal to stopping the background service after its implementation.
- Dark cyberpunk theme — the primary artistic baseline: dark depth, electric cyan/blue, controlled glow; light and system themes are also provided.
- **Use only** `assets/qadren-logo.png` as the QADREN mark; do not create a "similar" mark, do not replace SF Symbols with a third-party product logo.
- Fictitious graphs/numbers, replacing UNKNOWN with error/success, unrelated content prefetch to the main MacBook, and decorating an unavailable function with an active button are prohibited.
- First the existing `MainView.swift`, `QADRENApp.swift`, `StorageConnections.swift`, `JobInventory.swift`, `LocalWorkspace.swift`, not a second UI layer and not a second Rust Job Engine.

## Separation of Decisions

| Type | Source of Truth |
|---|---|
| Synchronization functions, local uploads, confirmations | `docs/architecture/backup-model.md`, `docs/policies/` |
| UI composition and interaction | `docs/design/*.md` files |
| Spacing, palette, sizes, typography | `docs/design/tokens.json` + explanations in `visual-language.md` |
| Existing API and runtime readiness | Current source code, tests, and the applicable GitHub Issue |
| HTML demonstration | only stylistic and compositional reference |

Do not change accepted documents "according to Codex taste"; describe and agree on any conflict. Documentation is not a permission to activate dangerous commands prematurely.

## Recommended Path for Codex

Read the documents in the given order, inventory the current components, create a general theme/token layer and widget chrome, then move the current panel to the widget surface without changing the domain logic. After the visual baseline, integrate real read-only data, then permitted commands via FFI in the order of release gates. Verify each increment according to [implementation.md](implementation.md). Work in `main`, without new branches and without a separate design repository.
