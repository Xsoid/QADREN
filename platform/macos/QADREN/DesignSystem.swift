import AppKit
import SwiftUI

enum QDThemeMode: String, CaseIterable, Identifiable {
    case dark
    case light
    case system

    var id: String { rawValue }

    var title: String {
        switch self {
        case .dark: "Тёмная"
        case .light: "Светлая"
        case .system: "Как в macOS"
        }
    }

    var preferredColorScheme: ColorScheme? {
        switch self {
        case .dark: .dark
        case .light: .light
        case .system: nil
        }
    }
}

enum QDColorRole {
    case canvas, panel, raised, stroke, strokeHover, focus
    case primaryText, secondaryText, disabledText
}

enum QDSemanticColor: CaseIterable {
    case active, ready, waiting, needsAction, failed, unknown, neutral
}

enum QDTheme {
    static func color(_ role: QDColorRole, for scheme: ColorScheme) -> Color {
        let dark = scheme == .dark
        return switch role {
        case .canvas: dark ? Color(hex: 0x06101B) : Color(hex: 0xF1F7FA)
        case .panel: dark ? Color(hex: 0x0B1B2A) : .white
        case .raised: dark ? Color(hex: 0x10273A) : Color(hex: 0xE9F2F7)
        case .stroke: dark ? Color(hex: 0x1B4058) : Color(hex: 0x9DB9C9)
        case .strokeHover: dark ? Color(hex: 0x2B617B) : Color(hex: 0x5688A4)
        case .focus: dark ? Color(hex: 0x19C5EF) : Color(hex: 0x006C98)
        case .primaryText: dark ? Color(hex: 0xECF7FF) : Color(hex: 0x112C3C)
        case .secondaryText: dark ? Color(hex: 0xA9C2D2) : Color(hex: 0x385869)
        case .disabledText: dark ? Color(hex: 0x7B91A2) : Color(hex: 0x617D8B)
        }
    }

    static let cyan = Color(hex: 0x0CB9F2)
    static let blue = Color(hex: 0x2A80FF)
    static let success = Color(hex: 0x24CE86)
    static let warning = Color(hex: 0xFFB347)
    static let danger = Color(hex: 0xFF6376)
    static let unknown = Color(hex: 0xB4A1F5)

    static func semanticColor(_ semantic: QDSemanticColor, for scheme: ColorScheme) -> Color {
        Color(hex: semanticHex(semantic, for: scheme))
    }

    static func semanticContrastRatio(_ semantic: QDSemanticColor, for scheme: ColorScheme) -> Double {
        contrastRatio(
            foreground: semanticHex(semantic, for: scheme),
            background: scheme == .dark ? 0x0B1B2A : 0xFFFFFF
        )
    }

    /// This is used for non-semantic widget borders when macOS asks for
    /// increased contrast. Semantic status borders keep their own color.
    static func widgetStroke(
        for scheme: ColorScheme, increasedContrast: Bool
    ) -> Color {
        Color(hex: widgetStrokeHex(for: scheme, increasedContrast: increasedContrast))
    }

    static func widgetStrokeContrastRatio(
        for scheme: ColorScheme, increasedContrast: Bool
    ) -> Double {
        contrastRatio(
            foreground: widgetStrokeHex(for: scheme, increasedContrast: increasedContrast),
            background: scheme == .dark ? 0x0B1B2A : 0xFFFFFF
        )
    }

    private static func widgetStrokeHex(
        for scheme: ColorScheme, increasedContrast: Bool
    ) -> UInt32 {
        guard increasedContrast else {
            return scheme == .dark ? 0x1B4058 : 0x9DB9C9
        }
        return scheme == .dark ? 0xECF7FF : 0x112C3C
    }

    private static func semanticHex(_ semantic: QDSemanticColor, for scheme: ColorScheme) -> UInt32 {
        if scheme == .dark {
            return switch semantic {
            case .active: 0x0CB9F2
            case .ready: 0x24CE86
            case .waiting: 0x2A80FF
            case .needsAction: 0xFFB347
            case .failed: 0xFF6376
            case .unknown: 0xB4A1F5
            case .neutral: 0xA9C2D2
            }
        }
        return switch semantic {
        case .active: 0x006C98
        case .ready: 0x087443
        case .waiting: 0x245E99
        case .needsAction: 0x8A4B00
        case .failed: 0xB3261E
        case .unknown: 0x6550A5
        case .neutral: 0x385869
        }
    }

    private static func contrastRatio(foreground: UInt32, background: UInt32) -> Double {
        let foregroundLuminance = relativeLuminance(foreground)
        let backgroundLuminance = relativeLuminance(background)
        let lighter = max(foregroundLuminance, backgroundLuminance)
        let darker = min(foregroundLuminance, backgroundLuminance)
        return (lighter + 0.05) / (darker + 0.05)
    }

    private static func relativeLuminance(_ hex: UInt32) -> Double {
        let channels = [
            Double((hex >> 16) & 0xFF) / 255,
            Double((hex >> 8) & 0xFF) / 255,
            Double(hex & 0xFF) / 255,
        ]
        let linear = channels.map { channel in
            channel <= 0.04045 ? channel / 12.92 : pow((channel + 0.055) / 1.055, 2.4)
        }
        return 0.2126 * linear[0] + 0.7152 * linear[1] + 0.0722 * linear[2]
    }
}

enum QDAccessibilityPresentation {
    static func widgetBorderWidth(increasedContrast: Bool) -> CGFloat {
        increasedContrast ? 2 : 1
    }

    static func permitsImplicitAnimation(reduceMotion: Bool) -> Bool {
        !reduceMotion
    }
}

private struct QDReduceMotion: ViewModifier {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    func body(content: Content) -> some View {
        content.transaction { transaction in
            if !QDAccessibilityPresentation.permitsImplicitAnimation(reduceMotion: reduceMotion) {
                transaction.animation = nil
            }
        }
    }
}

extension View {
    func qadrenAccessibilityPresentation() -> some View {
        modifier(QDReduceMotion())
    }
}

private extension Color {
    init(hex: UInt32) {
        self.init(
            .sRGB,
            red: Double((hex >> 16) & 0xFF) / 255,
            green: Double((hex >> 8) & 0xFF) / 255,
            blue: Double(hex & 0xFF) / 255,
            opacity: 1
        )
    }
}

enum QDStatusKind: Equatable {
    case ready, active, waiting, paused, needsAction, failed, unknown, offline, unsupported

    func color(for scheme: ColorScheme) -> Color {
        switch self {
        case .ready: QDTheme.semanticColor(.ready, for: scheme)
        case .active: QDTheme.semanticColor(.active, for: scheme)
        case .waiting, .paused: QDTheme.semanticColor(.waiting, for: scheme)
        case .needsAction: QDTheme.semanticColor(.needsAction, for: scheme)
        case .failed: QDTheme.semanticColor(.failed, for: scheme)
        case .unknown: QDTheme.semanticColor(.unknown, for: scheme)
        case .offline, .unsupported: QDTheme.semanticColor(.neutral, for: scheme)
        }
    }

    var symbol: String {
        switch self {
        case .ready: "checkmark.circle.fill"
        case .active: "arrow.triangle.2.circlepath.circle.fill"
        case .waiting: "clock"
        case .paused: "pause.circle"
        case .needsAction: "exclamationmark.triangle.fill"
        case .failed: "xmark.octagon.fill"
        case .unknown: "questionmark.diamond.fill"
        case .offline: "wifi.slash"
        case .unsupported: "minus.circle"
        }
    }

    /// Warning and failure states must stand out at the widget boundary.
    /// Other states keep the neutral chrome so that the dashboard does not
    /// become a field of permanent colored frames.
    var usesAttentionBorder: Bool {
        self == .needsAction || self == .failed
    }
}

struct QDStatusLabel: View {
    let kind: QDStatusKind
    let text: String
    @Environment(\.colorScheme) private var colorScheme

    var body: some View {
        Label(text, systemImage: kind.symbol)
            .font(.caption.weight(.medium))
            .foregroundStyle(kind.color(for: colorScheme))
            .accessibilityLabel(text)
    }
}

struct QDMetric: View {
    let value: String?
    let label: String
    let provenance: String

    var body: some View {
        VStack(alignment: .leading, spacing: 3) {
            Text(value ?? "—")
                .font(.system(size: 20, weight: .semibold, design: .default).monospacedDigit())
            Text(label).font(.caption.weight(.medium))
            Text(provenance).font(.caption2).foregroundStyle(.secondary)
        }
        .accessibilityElement(children: .combine)
    }
}

/// The shared widget shell in the cyber language (`QDCyber`): a cut-corner
/// frame with the title in a raised tab. Warning and failure states color the
/// edge, the title, and a narrow wash at the top; other states stay quiet.
struct QDWidgetFrame<Content: View, Actions: View>: View {
    let title: String
    let symbol: String
    let status: QDStatusKind?
    @ViewBuilder let actions: Actions
    @ViewBuilder let content: Content
    @Environment(\.colorScheme) private var colorScheme
    @State private var increasedContrast = NSWorkspace.shared.accessibilityDisplayShouldIncreaseContrast
    @State private var titleWidth: CGFloat? = nil

    init(
        title: String,
        symbol: String,
        status: QDStatusKind? = nil,
        @ViewBuilder actions: () -> Actions = { EmptyView() },
        @ViewBuilder content: () -> Content
    ) {
        self.title = title
        self.symbol = symbol
        self.status = status
        self.actions = actions()
        self.content = content()
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text(title)
                .font(QDCyber.heading(14))
                .tracking(2.2)
                .textCase(.uppercase)
                .foregroundStyle(accent)
                .lineLimit(1)
                .qdGlitch()
                .frame(maxWidth: titleWidth, alignment: .leading)
                .padding(.leading, 26 - 12)
                .frame(maxWidth: .infinity, maxHeight: QDCyberFrameShape.tabHeight, alignment: .leading)
                // Layout controls ride above the edge, right of the tab, and
                // never move the title or the content.
                .overlay(alignment: .trailing) {
                    HStack(spacing: 4) { actions }
                        .buttonStyle(.borderless)
                        .menuIndicator(.hidden)
                        .controlSize(.small)
                        .foregroundStyle(QDCyber.color(.secondaryText, for: colorScheme))
                        .fixedSize()
                }
                .frame(height: QDCyberFrameShape.tabHeight)
            content
                .padding(.bottom, 12)
        }
        .padding(.horizontal, 12)
        .padding(.bottom, 10)
        .frame(maxWidth: .infinity, alignment: .topLeading)
        // A long title stays inside the raised tab.
        .onGeometryChange(for: CGFloat.self) { $0.size.width } action: { width in
            titleWidth = max(40, width * QDCyberFrameShape.tabFraction - 30)
        }
        .background {
            ZStack {
                QDCyberFrameShape().fill(QDCyber.color(.card, for: colorScheme))
                if let attention {
                    // A narrow wash at the top, not a filled widget.
                    LinearGradient(colors: [attention.opacity(0.2), .clear], startPoint: .top, endPoint: .bottom)
                        .frame(height: 90)
                        .frame(maxHeight: .infinity, alignment: .top)
                        .clipShape(QDCyberFrameShape())
                }
                QDCyberFrameShape()
                    .strokeBorder(edge, lineWidth: increasedContrast ? 2 : 1.5)
            }
        }
        .onReceive(
            NotificationCenter.default.publisher(
                for: NSWorkspace.accessibilityDisplayOptionsDidChangeNotification
            )
        ) { _ in
            increasedContrast = NSWorkspace.shared.accessibilityDisplayShouldIncreaseContrast
        }
    }

    private var attention: Color? {
        guard let status, status.usesAttentionBorder else { return nil }
        return status.cyberColor(for: colorScheme)
    }

    private var accent: Color {
        attention ?? QDCyber.color(.accent, for: colorScheme)
    }

    private var edge: Color {
        if let attention { return attention }
        return increasedContrast ? QDCyber.color(.accent, for: colorScheme) : QDCyber.color(.edge, for: colorScheme)
    }
}

struct QDLogo: View {
    let size: CGFloat

    var body: some View {
        Group {
            if let image = Bundle.main.image(forResource: "qadren-logo") {
                Image(nsImage: image)
                    .resizable()
                    .interpolation(.high)
                    .scaledToFit()
            } else {
                Color.clear
            }
        }
        .frame(width: size, height: size)
        .accessibilityLabel("QADREN")
    }
}

struct QDEmptyState: View {
    let title: String
    let detail: String
    let symbol: String

    var body: some View {
        VStack(spacing: 10) {
            Image(systemName: symbol).font(.system(size: 28)).foregroundStyle(.secondary)
            Text(title).font(.headline)
            Text(detail).font(.callout).foregroundStyle(.secondary).multilineTextAlignment(.center)
        }
        .frame(maxWidth: .infinity, minHeight: 150)
        .padding()
    }
}
