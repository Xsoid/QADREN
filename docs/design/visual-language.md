# QADREN — visual language, brand and tokens

## 1. Artistic principle

**Cyberpunk Control Surface**: a technical dispatcher interface with modern native macOS mechanics. This is not a "gaming HUD": the content is read for hours; decorative light contours must not compete with data. Active operations, selected focus and blocking events are more prominent. Successful and historical events receive less attention. No constant artificial flickering or "scanning" lines.

The working window content occupies the maximum area. NO giant hero-headers, wide empty fields, constant right palette, side menu in monitoring, or decorative diagrams. One header + adaptive grid + narrow status bar. macOS titlebar/traffic lights remain native.

## 2. Brand — strictly from repository

Canonical graphic resource: **`assets/qadren-logo.png`** (192×192 PNG, transparent background, existing cyan/blue sign). In SwiftUI — load asset catalog image from **this file without redrawing**; for HTML prototype — `../../assets/qadren-logo.png`. It is used in the project README. Do not add another brand symbol, do not make an SVG "replica" and do not distort the original proportions.

- Typical sizes: 20 pt next to the name in compact menu; 24–28 pt in titlebar/header; 48–64 pt only for the welcome screen/empty configuration.
- Margin around the sign: not less than 1/8 of the visible width, maintain aspect ratio, without cropping or clipping.
- Dark and light themes use the same file; if there is a contrast problem, change the background/margin, **not the logo colors**.
- The name "QADREN" — plain text SF Pro Semibold, without arbitrary decorative fonts or slogans ("SYNC WHAT MATTERS" is not approved by the brand book).
- System MenuBarExtra may use SF Symbol in template rendering if the color logo is unreadable there. This is a **system indicator**, not a change to the brand logo in windows.

## 3. Source of numerical values

Machine-readable table — [tokens.json](tokens.json). Below is the semantics; do not copy values from a randomly generated illustration.

| Group | Purpose |
|---|---|
| canvas, panel, raised, overlay | layer depth, not a stack of opaque cards |
| stroke, strokeHover, focus | 1 pt border and selected element accent |
| brandCyan, brandBlue | actions, progress and navigation accent, visually compatible with logo |
| success, warning, danger, unknown, neutral | strictly semantic states |
| textPrimary, textSecondary, textDisabled | hierarchy without loss of readability |
| grid, gap, inset, header, corner | general rhythmic space |
| compact, regular, expanded | **content** density, not total window scale |

The main dark palette is almost black with a blue undertone, layers are dark blue. The main accent is electric cyan `#0CB9F2`, additional blue `#2A80FF`. Red — only for danger/errors, amber — user decisions, lilac-grey — uncertainty; green — only for verified compliant status. Do not replace everything with green in `Worker report` without evidence. The light theme uses the separate darker `semantic.light` status tokens from `tokens.json`; it does not place the bright dark-theme accents as regular text on a white panel.

## 4. Geometry and grid

- Base layout unit **4 pt**, most common intervals 4/8/12/16/24.
- Canvas edge margin 12 pt, intervals between widgets 10 pt, widget padding 12 pt, header 32 pt. System titlebar — native.
- Widget frame: 8 pt corner radius, thin 1 pt border, no constant volumetric shadows. Controls 6 pt; small pill labels 4–6 pt.
- Between lines of a dense table 4–6 pt, row height 28/34/42 pt by density. Do not reduce tap targets on touch-devices based on assumptions; for macOS pointer, the minimum reasonable target is 24×24 pt, preferred available hit area 28–32 pt.
- Do not make a sharp "circuitry" frame for every element: maximum one thin line/small corner cut for **key** accent states, only if it does not affect clipping or accessibility.

## 5. Typography, numbers, long paths

SF Pro Text via system SwiftUI font. Scale in tokens: interface 13 pt regular; widget header 13 pt semibold; secondary 11–12 pt; metrics 20 pt semibold; inspector header 16 pt semibold. Technical IDs and structured dates — SF Mono, numerical indicators — monospacedDigit(). Do not apply monospaced to all headers.

Overflow rule: a name may be truncated in compact preview, **full path is mandatory** in context (tooltip, inspector, copy) and in detailed representation. It is forbidden to show a partial path as a full path. Sizes and speeds include units, quotas include source/time status; "—" for unknown, not "0".

## 6. Glow and motion

- Effects `glowIdle=0`, `glowFocus` limited outer contour <= 3 pt blur; for warning/danger, edge highlighting is possible without filling the whole widget.
- No infinite animated decorative pulsations. For undefined progress — system indicator, with a static replacement for `Reduce Motion`.
- Hover: 100–150 ms, focus: system keyboard focus is always visible, inspector opening maximum 160–200 ms or without animation.
- Data updates do not reset scroll, selection and focus, and do not jump the card size. No random "digital noise".

## 7. Themes, contrast, accessibility

Support `dark`, `light`, `system`. Dark theme is the baseline; light theme uses separate explicitly defined semantic tokens from JSON, not CSS invert. Tokens must account for system High Contrast, Reduce Transparency, Reduce Motion, Dynamic Type, and keyboard use. VoiceOver support is out of scope. Minimum contrast for regular text 4.5:1, for large text 3:1; if a specific color pair fails — correct the implementation colors and document the difference. Icon + short caption are mandatory for dangerous statuses; color and glow are **not** the only signals.

## 8. Iconography and graphics

Use SF Symbols with a uniform size of 13–15 pt and predictable names. Cloud brands in connection rows — only if a licensed asset is available, otherwise use a neutral symbol. Ring scales, graphs and grid diagrams are shown **only** for measured data; if there is no metrics history, do not draw a fake "CPU for 24 hours" line. Do not invent Worker ETA and total speed. Tooltip contains meaning, units, data source and freshness. A diagram is a tool, not a decoration.
