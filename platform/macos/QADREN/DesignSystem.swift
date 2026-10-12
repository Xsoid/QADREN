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

    var color: Color {
        switch self {
        case .ready: QDTheme.success
        case .active: QDTheme.cyan
        case .waiting, .paused: QDTheme.blue
        case .needsAction: QDTheme.warning
        case .failed: QDTheme.danger
        case .unknown: QDTheme.unknown
        case .offline, .unsupported: Color.secondary
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
}

struct QDStatusLabel: View {
    let kind: QDStatusKind
    let text: String

    var body: some View {
        Label(text, systemImage: kind.symbol)
            .font(.caption.weight(.medium))
            .foregroundStyle(kind.color)
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

struct QDWidgetFrame<Content: View, Actions: View>: View {
    let title: String
    let symbol: String
    let status: QDStatusKind?
    @ViewBuilder let actions: Actions
    @ViewBuilder let content: Content
    @Environment(\.colorScheme) private var colorScheme

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
        VStack(alignment: .leading, spacing: 12) {
            HStack(spacing: 8) {
                Image(systemName: symbol)
                    .font(.system(size: 14, weight: .medium))
                    .foregroundStyle(QDTheme.cyan)
                Text(title).font(.system(size: 13, weight: .semibold))
                if let status {
                    Image(systemName: status.symbol)
                        .font(.caption)
                        .foregroundStyle(status.color)
                }
                Spacer(minLength: 0)
                actions
            }
            .frame(height: 20)
            content
        }
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(QDTheme.color(.panel, for: colorScheme), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
        .overlay {
            RoundedRectangle(cornerRadius: 8, style: .continuous)
                .stroke(status == .needsAction ? QDTheme.warning : QDTheme.color(.stroke, for: colorScheme), lineWidth: 1)
        }
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
