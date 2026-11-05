import SwiftUI

/// The monitoring visual language. It ports the dark theme of cybercn-ui
/// (MIT, github.com/szvitek/cybercn-ui): a near-black canvas, yellow accents,
/// cut-corner frames, Advent Pro headings, and VT323 digits. The owner chose
/// it on 2026-10-31; the canvas stays dark, never the library's yellow fill.
/// The Settings window keeps `QDTheme` until a separate decision moves it.
enum QDCyber {
    enum Role {
        case canvas, card, edge, accent, onAccent, mint
        case primaryText, secondaryText, mutedText
    }

    static func color(_ role: Role, for scheme: ColorScheme) -> Color {
        Color(cyberHex: hex(role, dark: scheme == .dark), opacity: role == .edge ? 0.32 : 1)
    }

    /// Advent Pro has Cyrillic. Use it for all words.
    static func heading(_ size: CGFloat, weight: Font.Weight = .bold) -> Font {
        .custom("Advent Pro", fixedSize: size).weight(weight)
    }

    /// VT323 has Latin glyphs only. Use it for digits, times, and IDs.
    static func digits(_ size: CGFloat) -> Font {
        .custom("VT323", fixedSize: size)
    }

    private static func hex(_ role: Role, dark: Bool) -> UInt32 {
        switch role {
        case .canvas: dark ? 0x0B0B0B : 0xECECE4
        case .card: dark ? 0x151515 : 0xFFFFFF
        case .edge: dark ? 0xF9F002 : 0x1A1A1A
        case .accent: dark ? 0xF9F002 : 0x6B6400
        case .onAccent: 0x000000
        case .mint: dark ? 0x8AE66E : 0x2E8A1E
        case .primaryText: dark ? 0xECEBD9 : 0x161616
        case .secondaryText: dark ? 0x9A9A8A : 0x4A4A42
        case .mutedText: dark ? 0x6C6C62 : 0x6A6A60
        }
    }
}

extension QDStatusKind {
    /// Status colors of the cyber palette. Red is only for failures, orange
    /// for a required decision, neon green only for a verified ready state.
    func cyberColor(for scheme: ColorScheme) -> Color {
        let dark = scheme == .dark
        let hex: UInt32 = switch self {
        case .ready: dark ? 0x39FF14 : 0x1A8F00
        case .active, .waiting, .paused: dark ? 0x2F7BFF : 0x1F55C4
        case .needsAction: dark ? 0xFF9800 : 0xB35F00
        case .failed: dark ? 0xFF003C : 0xC8002F
        case .unknown: dark ? 0xB04CFF : 0x7A2BBF
        case .offline, .unsupported: dark ? 0x9A9A8A : 0x5A5A52
        }
        return Color(cyberHex: hex)
    }
}

/// The cybercn "paragraph" frame: a cut top-left corner, a raised title tab,
/// and a notch at the bottom edge.
struct QDCyberFrameShape: InsettableShape {
    static let tabHeight: CGFloat = 22
    static let tabFraction: CGFloat = 0.58
    var inset: CGFloat = 0

    func path(in rect: CGRect) -> Path {
        let r = rect.insetBy(dx: inset, dy: inset)
        let t = Self.tabHeight - inset
        let cut: CGFloat = 24 - inset
        let tab = r.minX + rect.width * Self.tabFraction
        let notch = r.minX + rect.width * 0.78
        var path = Path()
        path.move(to: CGPoint(x: r.minX, y: r.minY + cut))
        path.addLine(to: CGPoint(x: r.minX + cut, y: r.minY))
        path.addLine(to: CGPoint(x: tab, y: r.minY))
        path.addLine(to: CGPoint(x: tab + t, y: r.minY + t))
        path.addLine(to: CGPoint(x: r.maxX, y: r.minY + t))
        path.addLine(to: CGPoint(x: r.maxX, y: r.maxY - 10))
        path.addLine(to: CGPoint(x: notch - 10, y: r.maxY - 10))
        path.addLine(to: CGPoint(x: notch - 20, y: r.maxY))
        path.addLine(to: CGPoint(x: r.minX + 10, y: r.maxY))
        path.addLine(to: CGPoint(x: r.minX, y: r.maxY - 10))
        path.closeSubpath()
        return path
    }

    func inset(by amount: CGFloat) -> QDCyberFrameShape {
        var shape = self
        shape.inset += amount
        return shape
    }
}

/// A rectangle with one cut corner at the bottom left, for buttons and chips.
struct QDCyberCutShape: Shape {
    var cut: CGFloat = 12

    func path(in rect: CGRect) -> Path {
        var path = Path()
        path.move(to: CGPoint(x: rect.minX, y: rect.minY))
        path.addLine(to: CGPoint(x: rect.maxX, y: rect.minY))
        path.addLine(to: CGPoint(x: rect.maxX, y: rect.maxY))
        path.addLine(to: CGPoint(x: rect.minX + cut, y: rect.maxY))
        path.addLine(to: CGPoint(x: rect.minX, y: rect.maxY - cut))
        path.closeSubpath()
        return path
    }
}

/// The cybercn button: a solid fill, a cut corner, and a mint right edge.
struct QDCyberButtonStyle: ButtonStyle {
    var tint: Color
    var label: Color = .white
    @Environment(\.isEnabled) private var isEnabled
    @Environment(\.colorScheme) private var colorScheme

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(QDCyber.heading(13))
            .tracking(1.8)
            .textCase(.uppercase)
            .foregroundStyle(configuration.isPressed ? QDCyber.color(.onAccent, for: colorScheme) : label)
            .padding(.leading, 22).padding(.trailing, 16).padding(.vertical, 7)
            .background(configuration.isPressed ? QDCyber.color(.accent, for: colorScheme) : tint)
            .overlay(alignment: .trailing) {
                Rectangle().fill(QDCyber.color(.mint, for: colorScheme)).frame(width: 3)
            }
            .clipShape(QDCyberCutShape())
            .opacity(isEnabled ? 1 : 0.4)
            .contentShape(QDCyberCutShape())
    }
}

/// The secondary cybercn button: a thin accent edge on the card color, the
/// same cut corner, and accent words. The pressed state fills with accent.
struct QDCyberOutlineButtonStyle: ButtonStyle {
    @Environment(\.isEnabled) private var isEnabled
    @Environment(\.colorScheme) private var colorScheme

    func makeBody(configuration: Configuration) -> some View {
        let accent = QDCyber.color(.accent, for: colorScheme)
        configuration.label
            .font(QDCyber.heading(13))
            .tracking(1.8)
            .textCase(.uppercase)
            .foregroundStyle(configuration.isPressed ? QDCyber.color(.onAccent, for: colorScheme) : accent)
            .padding(.leading, 22).padding(.trailing, 16).padding(.vertical, 7)
            .background(configuration.isPressed ? accent : accent.opacity(0.06))
            .overlay(QDCyberCutShape().stroke(accent.opacity(0.7), lineWidth: 1.5))
            .overlay(alignment: .trailing) {
                Rectangle().fill(QDCyber.color(.mint, for: colorScheme)).frame(width: 3)
            }
            .clipShape(QDCyberCutShape())
            .opacity(isEnabled ? 1 : 0.4)
            .contentShape(QDCyberCutShape())
    }
}

/// A quiet text action: accent uppercase words over a mint underline with a
/// soft glow, for a secondary action that must not compete with data.
struct QDCyberLinkStyle: ButtonStyle {
    @Environment(\.colorScheme) private var colorScheme

    func makeBody(configuration: Configuration) -> some View {
        let mint = QDCyber.color(.mint, for: colorScheme)
        configuration.label
            .font(QDCyber.heading(13, weight: .semibold))
            .tracking(1.6)
            .textCase(.uppercase)
            .foregroundStyle(QDCyber.color(.accent, for: colorScheme))
            .padding(.bottom, 3)
            .overlay(alignment: .bottom) {
                Rectangle().fill(mint).frame(height: 1.5).shadow(color: mint.opacity(0.7), radius: 3)
            }
            .opacity(configuration.isPressed ? 0.6 : 1)
            .contentShape(Rectangle())
    }
}

/// The large status line: Advent Pro, uppercase, a soft glow, and a cursor.
/// The cursor blinks and the line glitches only with glitch effects on.
struct QDCyberHeadline: View {
    let text: String
    let color: Color
    var size: CGFloat = 32
    @AppStorage(QDGlitchSetting.key) private var glitchEnabled = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        TimelineView(.periodic(from: .now, by: 0.6)) { context in
            let blinkOff = glitchEnabled && !reduceMotion
                && Int(context.date.timeIntervalSinceReferenceDate / 0.6) % 2 == 1
            (Text(text) + Text("_").foregroundColor(blinkOff ? .clear : color))
                .font(QDCyber.heading(size))
                .textCase(.uppercase)
                .foregroundStyle(color)
                .shadow(color: color.opacity(0.55), radius: 6)
                .lineLimit(2)
                .minimumScaleFactor(0.7)
        }
        .qdGlitch()
        .accessibilityLabel(text)
    }
}

/// A three-zone progress ring: done (ready color), failed (failure color),
/// and the rest (a dashed track). An unknown total draws the track only.
struct QDCyberRing: View {
    let total: Int?
    let done: Int
    let failed: Int
    var size: CGFloat = 96
    /// A running process: a short arc sweeps around the ring.
    var active = false
    /// A running process without progress: the sweep stops.
    var stalled = false
    @Environment(\.colorScheme) private var colorScheme
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        let line = size * 0.09
        // A batch with nothing to do is complete.
        let doneEnd = total == 0 ? 1 : fraction(done)
        let failedEnd = min(1, doneEnd + fraction(failed))
        ZStack {
            Circle()
                .stroke(QDCyber.color(.edge, for: colorScheme), style: StrokeStyle(lineWidth: line, dash: [2, 3]))
            Circle()
                .trim(from: 0, to: doneEnd)
                .stroke(QDStatusKind.ready.cyberColor(for: colorScheme), style: StrokeStyle(lineWidth: line))
                .shadow(color: QDStatusKind.ready.cyberColor(for: colorScheme).opacity(0.5), radius: 3)
            Circle()
                .trim(from: doneEnd, to: failedEnd)
                .stroke(QDStatusKind.failed.cyberColor(for: colorScheme), style: StrokeStyle(lineWidth: line))
        }
        // The arcs start at 12 o'clock; the labels stay upright.
        .rotationEffect(.degrees(-90))
        .padding(line / 2)
        .overlay {
            if active && !stalled && !reduceMotion {
                TimelineView(.animation(minimumInterval: 1 / 30)) { context in
                    let turn = context.date.timeIntervalSinceReferenceDate.truncatingRemainder(dividingBy: 2.4) / 2.4
                    Circle()
                        .trim(from: 0, to: 0.1)
                        .stroke(QDStatusKind.active.cyberColor(for: colorScheme), style: StrokeStyle(lineWidth: line * 0.45, lineCap: .round))
                        .shadow(color: QDStatusKind.active.cyberColor(for: colorScheme), radius: 4)
                        .rotationEffect(.degrees(turn * 360 - 90))
                        .padding(line * 1.6)
                }
            }
        }
        .overlay {
            VStack(spacing: 0) {
                Text(percentText)
                    .font(QDCyber.digits(size * 0.34))
                    .foregroundStyle(QDCyber.color(.primaryText, for: colorScheme))
                Text(total.map { Self.short(done) + "/" + Self.short($0) } ?? "—")
                    .font(QDCyber.digits(size * 0.17))
                    .foregroundStyle(QDCyber.color(.mutedText, for: colorScheme))
            }
        }
        .frame(width: size, height: size)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(total.map { "Скопировано \(done) из \($0), ошибок \(failed)" } ?? "Прогресс неизвестен")
    }

    /// Tenths below 10%, so that a long run does not read 0% for hours.
    private var percentText: String {
        guard let total else { return "—" }
        guard total > 0 else { return "100%" }
        let percent = Double(done) / Double(total) * 100
        if percent > 0, percent < 10 {
            return String(format: "%.1f%%", (percent * 10).rounded(.down) / 10)
        }
        return "\(Int(percent.rounded(.down)))%"
    }

    /// 190143 → 190K, 2400000 → 2.4M: the center has room for a few digits.
    static func short(_ value: Int) -> String {
        switch value {
        case ..<10_000: String(value)
        case ..<1_000_000: "\(value / 1000)K"
        default: String(format: "%.1fM", Double(value) / 1_000_000)
        }
    }

    private func fraction(_ value: Int) -> CGFloat {
        guard let total, total > 0 else { return 0 }
        return CGFloat(value) / CGFloat(total)
    }
}

/// Motion that shows a running process: skewed segments march along a bar.
/// Draw it only while the Worker reports the process as running. A stalled
/// process stops the motion and turns it to the warning color, so motion
/// never hides a hang. Reduce Motion draws the same bar without motion.
struct QDCyberActivityBar: View {
    var stalled = false
    var height: CGFloat = 4
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.colorScheme) private var colorScheme

    var body: some View {
        let color = (stalled ? QDStatusKind.needsAction : QDStatusKind.active).cyberColor(for: colorScheme)
        TimelineView(.animation(minimumInterval: 1 / 30, paused: stalled || reduceMotion)) { context in
            let phase = stalled || reduceMotion ? 0 : context.date.timeIntervalSinceReferenceDate.truncatingRemainder(dividingBy: 1)
            Canvas { canvas, size in
                let step: CGFloat = 12
                var x = -step + CGFloat(phase) * step
                while x < size.width {
                    var segment = Path()
                    segment.move(to: CGPoint(x: x + 3, y: 0))
                    segment.addLine(to: CGPoint(x: x + 9, y: 0))
                    segment.addLine(to: CGPoint(x: x + 6, y: size.height))
                    segment.addLine(to: CGPoint(x: x, y: size.height))
                    segment.closeSubpath()
                    canvas.fill(segment, with: .color(color))
                    x += step
                }
            }
        }
        .frame(height: height)
        .mask(LinearGradient(colors: [.clear, .black, .black, .clear], startPoint: .leading, endPoint: .trailing))
        .shadow(color: color.opacity(0.6), radius: 3)
        .accessibilityLabel(stalled ? "Нет прогресса" : "Процесс идёт")
    }
}

/// The user setting for glitch effects. Off by default. Short, rare bursts
/// keep a panel that stays on screen for hours from showing a static image.
enum QDGlitchSetting {
    static let key = "qadren.monitoring.glitch-effects"
}

private struct QDGlitchSlice: Equatable {
    let shift: CGFloat
    let split: CGFloat
    let bandY: CGFloat
    let bandHeight: CGFloat
}

/// A cybercn-style glitch: every 4–9 s, a burst of about 300 ms shifts the
/// view and splits a horizontal band into red and cyan copies. Off without
/// the setting and with Reduce Motion.
private struct QDGlitchModifier: ViewModifier {
    @AppStorage(QDGlitchSetting.key) private var enabled = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var slice: QDGlitchSlice?

    func body(content: Content) -> some View {
        content
            .offset(x: slice?.shift ?? 0)
            .overlay {
                if let slice {
                    GeometryReader { proxy in
                        let band = Rectangle()
                            .frame(height: proxy.size.height * slice.bandHeight)
                            .offset(y: proxy.size.height * slice.bandY)
                            .frame(maxHeight: .infinity, alignment: .top)
                        ZStack {
                            content.colorMultiply(Color(red: 1, green: 0, blue: 0.24)).offset(x: slice.split)
                            content.colorMultiply(Color(red: 0, green: 0.9, blue: 1)).offset(x: -slice.split)
                        }
                        .mask(band)
                        .blendMode(.screen)
                    }
                    .allowsHitTesting(false)
                }
            }
            .task(id: enabled && !reduceMotion) {
                guard enabled, !reduceMotion else {
                    slice = nil
                    return
                }
                while !Task.isCancelled {
                    try? await Task.sleep(for: .seconds(Double.random(in: 4...9)))
                    for _ in 0..<Int.random(in: 3...5) {
                        slice = QDGlitchSlice(
                            shift: .random(in: -4...4),
                            split: .random(in: 2...6),
                            bandY: .random(in: 0...0.7),
                            bandHeight: .random(in: 0.15...0.45)
                        )
                        try? await Task.sleep(for: .milliseconds(Int.random(in: 40...90)))
                    }
                    slice = nil
                }
            }
    }
}

extension View {
    func qdGlitch() -> some View {
        modifier(QDGlitchModifier())
    }
}

private extension Color {
    init(cyberHex hex: UInt32, opacity: Double = 1) {
        self.init(
            .sRGB,
            red: Double((hex >> 16) & 0xFF) / 255,
            green: Double((hex >> 8) & 0xFF) / 255,
            blue: Double(hex & 0xFF) / 255,
            opacity: opacity
        )
    }
}
