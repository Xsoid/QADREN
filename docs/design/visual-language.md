# QADREN — visual language, brand and tokens

## 0. Monitoring style (owner decision, 2026-10-31)

The monitoring window uses the dark theme of [cybercn-ui](https://github.com/szvitek/cybercn-ui) (MIT), ported to SwiftUI in `platform/macos/QADREN/CyberDesign.swift`. The values are in the `monitoringCyber` group of [tokens.json](tokens.json). This section has priority over sections 1, 4, 5, and 6 for the monitoring window. The Settings window keeps the rules below until a separate decision.

- Canvas: near black. Do not use the yellow canvas of the library light theme.
- Yellow is an accent only: widget titles, primary values, and the active control.
- Widget frame: a cut top-left corner, a raised title tab, a bottom notch, and a 1.5 pt edge. Warning and failure color the edge, the title, and a top wash of 90 pt. They do not fill the whole widget.
- Fonts: Advent Pro for all words (it has Cyrillic); VT323 for digits, times, and IDs only (it has no Cyrillic). Both fonts are OFL and are in `assets/fonts/`.
- Status line: a large uppercase word with a soft glow and a `_` cursor.
- Buttons: a solid fill, a cut bottom-left corner, and a mint right edge.
- Activity motion (owner decision, 2026-10-31): a widget that shows a running process (for example a copying Scenario) shows constant motion: `QDCyberActivityBar` (marching segments) and a sweep arc on `QDCyberRing`. Motion shows only while the Worker reports the process as running. When the progress counters and the current file do not change for 3 minutes, the motion stops, turns to the warning color, and the widget shows "Нет прогресса N мин". Motion must never hide a hang. Reduce Motion draws the bar without motion.
- Window chrome (owner decision, 2026-10-31): the monitoring window hides the system title bar and keeps the system traffic lights. An empty unified `NSToolbar` without items makes the title bar as tall as the top bar, so macOS centers the traffic lights in it; a SwiftUI toolbar item is not used because it draws a separator. Full screen shows a toolbar as a separate bar over the content, so the toolbar is removed when the window enters full screen and comes back when it exits. `MonitoringHeader` draws the top bar: the logo, "Q.A.D.R.E.N." in accent with a glow, "// КОНТРОЛЬНАЯ ПАНЕЛЬ", and the layout actions as cyber buttons ("Компоновка" outline; in Layout mode "+ Виджет" outline and "Готово" filled). A thin edge with an accent bar under the title and a step at the trailing side closes the bar. `MonitoringScreenFrame` draws a thin edge and accent corner marks 6 pt inside the window corners. The window title is "Q.A.D.R.E.N. Контрольная панель". In full screen the bar drops the room of the traffic lights. The Settings window keeps the system title bar.
- Glitch effects: an optional application setting, off by default. When on, short bursts (about 300 ms, every 4–9 s) shift widget titles and the status line, and the cursor blinks. The purpose is to keep a panel that stays on screen for hours from showing a static image. Reduce Motion turns the effects off.

## 1. Artistic principle

**Cyberpunk Control Surface**: a technical dispatcher interface with modern native macOS mechanics. This is not a "gaming HUD": the content is read for hours; decorative light contours must not compete with data. Active operations, selected focus and blocking events are more prominent. Successful and historical events receive less attention. No constant artificial flickering or "scanning" lines.

The working window content occupies the maximum area. NO giant hero-headers, wide empty fields, constant right palette, side menu in monitoring, or decorative diagrams. One header + adaptive grid + narrow status bar. macOS titlebar/traffic lights remain native.

## 2. Brand — strictly from repository

The owner replaced the mark on 2026-10-31 with the cyber "plate" mark: a yellow `#F9F002` plate with cut top-left and bottom-right corners, a mint `#8AE66E` right edge, and a black `#0B0B0B` letter Q made of an octagon ring and a cut tail. It follows §0 (widget frame and button shapes). The former cyan/blue sign stays only in the Git history.

- Source: **`assets/brand/qadren-mark.svg`** (144×144 units). Do not change its geometry or colors without an owner decision.
- Raster: **`assets/qadren-logo.png`** (192×192 PNG, transparent background, 6% margin). SwiftUI loads this file; the HTML prototype uses `../../assets/qadren-logo.png`. The README uses it.
- App icon: **`assets/qadren.icns`**, made from the same geometry with a 10% margin (16–1024 px).
- Do not add another brand symbol and do not distort the proportions.

- Typical sizes: 20 pt next to the name in compact menu; 24–28 pt in titlebar/header; 48–64 pt only for the welcome screen/empty configuration.
- Margin around the sign: not less than 1/8 of the visible width, maintain aspect ratio, without cropping or clipping.
- Dark and light themes use the same file; if there is a contrast problem, change the background/margin, **not the logo colors**. The plate carries its own background, so it reads on both themes.
- The name "QADREN" — plain text SF Pro Semibold, without arbitrary decorative fonts or slogans ("SYNC WHAT MATTERS" is not approved by the brand book). Exception (owner decision, 2026-10-31): the monitoring top bar writes "Q.A.D.R.E.N." in Advent Pro (§0).
- System MenuBarExtra may use SF Symbol in template rendering if the color logo is unreadable there. This is a **system indicator**, not a change to the brand logo in windows.

## 3. Source of numerical values

Machine-readable table — [tokens.json](tokens.json). Below is the semantics; do not copy values from a randomly generated illustration.

| Group | Purpose |
|---|---|
| canvas, panel, raised, overlay | layer depth, not a stack of opaque cards |
| stroke, strokeHover, focus | 1 pt border and selected element accent |
| brandCyan, brandBlue | actions, progress and navigation accent of the Settings window (the monitoring window uses `monitoringCyber`, §0) |
| success, warning, danger, unknown, neutral | strictly semantic states |
| textPrimary, textSecondary, textDisabled | hierarchy without loss of readability |
| grid, gap, inset, header, corner | general rhythmic space |
| compact, regular, expanded | **content** density, not total window scale |

This paragraph applies to the Settings window; the monitoring window uses §0. The main dark palette is almost black with a blue undertone, layers are dark blue. The main accent is electric cyan `#0CB9F2`, additional blue `#2A80FF`. Red — only for danger/errors, amber — user decisions, lilac-grey — uncertainty; green — only for verified compliant status. Do not replace everything with green in `Worker report` without evidence. The light theme uses the separate darker `semantic.light` status tokens from `tokens.json`; it does not place the bright dark-theme accents as regular text on a white panel.

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
