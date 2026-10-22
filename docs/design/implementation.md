# UI Implementation and Verification

This document defines the permanent implementation procedure for the QADREN SwiftUI interface. The [design system](README.md) owns visual decisions. Rust Core owns business logic, policy, and durable outcomes.

## Boundaries

- Use the QADREN design tokens, components, widget contract, and approved logo.
- Keep monitoring as a widget canvas without a permanent Sidebar. Settings can use navigation.
- Present durable state and supported intents only. Do not create a second queue, registry, policy engine, or runtime history in SwiftUI.
- Keep Yandex Disk and iCloud out of new Release 1 interface work.
- Do not present mock data, a failed read, or an unknown state as confirmed success.

## Safety UX

- A required file read may materialize a File Provider item without a separate confirmation. Do not prefetch unrelated content.
- Destructive replacement and deletion show the policy-defined scope and require the required consent.
- `UNKNOWN` has no unconditional mutation retry.
- Remove or Disconnect must not delete provider objects.
- Do not display secrets, authorization URLs, tokens, or file content in the interface, diagnostics, or test captures.

## Procedure for a SwiftUI change

1. Read the matching documents in `docs/design/` and inspect the existing SwiftUI implementation.
2. Use Codebase Memory first when available. Use `ui-ux-pro-max` for UX review and `swiftui-expert-skill` for SwiftUI implementation. These skills are advisory; the QADREN design system remains canonical.
3. Keep the change within the existing Rust/SwiftUI boundary. Reuse components and tokens before adding new ones.
4. Build the application and run relevant native checks.
5. Capture real QADREN windows in dark and light appearance. Check compact layouts, long paths, keyboard focus, contrast, Reduce Motion, and displayed-data truthfulness.
6. Keep captures outside Git. Record patch-specific results and open work in the applicable GitHub Issue.

An accessibility tree, HTML prototype, source review, or preview does not replace a native screenshot for visual acceptance.

## Verification

Run the applicable commands from [testing and quality](../development/testing-and-quality.md). When design tokens change, run:

```bash
python3 -m json.tool docs/design/tokens.json >/dev/null
```

Review only states that the changed interface can present. Mark unavailable native checks as `BLOCKED` or `SKIPPED`.
