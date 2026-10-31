import AppKit
import SwiftUI

/// The top bar of the monitoring window in the cyber style. The window hides
/// the system title bar, so this bar draws the title and the layout actions;
/// the system traffic lights stay at the leading edge.
struct MonitoringHeader: View {
    @Binding var isEditing: Bool
    let onAdd: (MonitoringWidgetKind) -> Void
    @Environment(\.colorScheme) private var colorScheme
    /// Full screen hides the traffic lights, so their room is not needed.
    @State private var fullScreen = false

    static let height: CGFloat = 52
    /// Room for the system traffic lights.
    private static let trafficLights: CGFloat = 100

    var body: some View {
        HStack(spacing: 12) {
            logo
            HStack(alignment: .firstTextBaseline, spacing: 10) {
                Text("Q.A.D.R.E.N.")
                    .font(QDCyber.heading(19))
                    .tracking(3)
                    .foregroundStyle(QDCyber.color(.accent, for: colorScheme))
                    .shadow(color: QDCyber.color(.accent, for: colorScheme).opacity(0.45), radius: 5)
                    .qdGlitch()
                Text("//")
                    .font(QDCyber.digits(20))
                    .foregroundStyle(QDCyber.color(.mutedText, for: colorScheme))
                Text("Контрольная панель")
                    .font(QDCyber.heading(15, weight: .semibold))
                    .tracking(2.2)
                    .textCase(.uppercase)
                    .foregroundStyle(QDCyber.color(.secondaryText, for: colorScheme))
            }
            .accessibilityElement(children: .ignore)
            .accessibilityLabel("Q.A.D.R.E.N. Контрольная панель")
            .accessibilityAddTraits(.isHeader)
            Spacer(minLength: 12)
            if isEditing {
                Menu {
                    ForEach(MonitoringWidgetKind.allCases) { kind in
                        Button {
                            onAdd(kind)
                        } label: {
                            Label(kind.title, systemImage: kind.symbol)
                        }
                        .help(kind.summary)
                    }
                } label: {
                    Label("Виджет", systemImage: "plus")
                }
                .menuStyle(.button)
                .menuIndicator(.hidden)
                .buttonStyle(QDCyberOutlineButtonStyle())
                .fixedSize()
                .help("Добавить виджет на панель")
            }
            if isEditing {
                Button {
                    isEditing = false
                } label: {
                    Label("Готово", systemImage: "checkmark")
                }
                .buttonStyle(QDCyberButtonStyle(tint: QDCyber.color(.accent, for: colorScheme), label: QDCyber.color(.onAccent, for: colorScheme)))
                .keyboardShortcut(.defaultAction)
                .help("Завершить компоновку")
            } else {
                Button {
                    isEditing = true
                } label: {
                    Label("Компоновка", systemImage: "square.grid.2x2")
                }
                .buttonStyle(QDCyberOutlineButtonStyle())
                .help("Изменить компоновку панели")
            }
        }
        .padding(.leading, leading)
        .padding(.trailing, 18)
        .frame(height: Self.height)
        .frame(maxWidth: .infinity)
        .background(QDCyber.color(.card, for: colorScheme))
        .overlay(alignment: .bottom) { bottomEdge }
        .background(WindowFullScreenReader(fullScreen: $fullScreen).frame(width: 0, height: 0))
    }

    private var leading: CGFloat { fullScreen ? 18 : Self.trafficLights }

    @ViewBuilder
    private var logo: some View {
        if let image = Bundle.main.image(forResource: "qadren-logo") {
            Image(nsImage: image)
                .resizable()
                .interpolation(.high)
                .aspectRatio(contentMode: .fit)
                .frame(width: 26, height: 26)
                .accessibilityHidden(true)
        }
    }

    /// A thin edge with a bright accent under the title and a step down at
    /// the trailing side, like the notch of a widget frame.
    private var bottomEdge: some View {
        GeometryReader { proxy in
            let width = proxy.size.width
            let accent = QDCyber.color(.accent, for: colorScheme)
            ZStack(alignment: .topLeading) {
                Path { path in
                    path.move(to: CGPoint(x: 0, y: 0.5))
                    path.addLine(to: CGPoint(x: width * 0.72, y: 0.5))
                    path.addLine(to: CGPoint(x: width * 0.72 + 8, y: 4.5))
                    path.addLine(to: CGPoint(x: width, y: 4.5))
                }
                .stroke(QDCyber.color(.edge, for: colorScheme).opacity(0.32), lineWidth: 1)
                Rectangle()
                    .fill(accent)
                    .frame(width: 150, height: 2)
                    .shadow(color: accent.opacity(0.7), radius: 4)
                    .offset(x: leading, y: -0.5)
            }
        }
        .frame(height: 5)
        .offset(y: 4)
        .allowsHitTesting(false)
    }
}

/// The border of the monitoring window: a thin edge and bright corner marks,
/// so that the panel reads as one instrument surface.
struct MonitoringScreenFrame: View {
    @Environment(\.colorScheme) private var colorScheme
    private let corner: CGFloat = 18

    var body: some View {
        GeometryReader { proxy in
            // Inside the rounded corners of the window.
            let rect = CGRect(origin: .zero, size: proxy.size).insetBy(dx: 6, dy: 6)
            let accent = QDCyber.color(.accent, for: colorScheme)
            ZStack {
                Path { path in path.addRect(rect) }
                    .stroke(QDCyber.color(.edge, for: colorScheme).opacity(0.14), lineWidth: 1)
                Path { path in
                    for (x, y, dx, dy) in [
                        (rect.minX, rect.minY, 1.0, 1.0), (rect.maxX, rect.minY, -1.0, 1.0),
                        (rect.minX, rect.maxY, 1.0, -1.0), (rect.maxX, rect.maxY, -1.0, -1.0),
                    ] {
                        path.move(to: CGPoint(x: x + dx * corner, y: y))
                        path.addLine(to: CGPoint(x: x, y: y))
                        path.addLine(to: CGPoint(x: x, y: y + dy * corner))
                    }
                }
                .stroke(accent.opacity(0.75), lineWidth: 2)
            }
        }
        .allowsHitTesting(false)
        .accessibilityHidden(true)
    }
}

/// Reports whether the window of this view is in full screen now, and sets up
/// its title bar for `MonitoringHeader`.
private struct WindowFullScreenReader: NSViewRepresentable {
    @Binding var fullScreen: Bool

    func makeCoordinator() -> Coordinator { Coordinator(fullScreen: $fullScreen) }

    func makeNSView(context: Context) -> NSView {
        let view = NSView(frame: .zero)
        DispatchQueue.main.async { context.coordinator.attach(to: view.window) }
        return view
    }

    func updateNSView(_ nsView: NSView, context: Context) {
        DispatchQueue.main.async { context.coordinator.attach(to: nsView.window) }
    }

    @MainActor
    final class Coordinator {
        private var fullScreen: Binding<Bool>
        private weak var window: NSWindow?
        private var observers: [NSObjectProtocol] = []

        init(fullScreen: Binding<Bool>) { self.fullScreen = fullScreen }

        func attach(to window: NSWindow?) {
            guard let window, window !== self.window else { return }
            self.window = window
            observers.forEach(NotificationCenter.default.removeObserver)
            let update: @Sendable (Notification) -> Void = { [weak self, weak window] _ in
                MainActor.assumeIsolated {
                    guard let self, let window else { return }
                    let full = window.styleMask.contains(.fullScreen)
                    self.fullScreen.wrappedValue = full
                    Self.setTitlebarToolbar(window, present: !full)
                }
            }
            // Full screen shows a toolbar as a separate bar over the content,
            // so it goes before the transition and comes back after it.
            let leaving: @Sendable (Notification) -> Void = { [weak window] _ in
                MainActor.assumeIsolated {
                    if let window { Self.setTitlebarToolbar(window, present: false) }
                }
            }
            observers = [NSWindow.didEnterFullScreenNotification, NSWindow.didExitFullScreenNotification].map {
                NotificationCenter.default.addObserver(forName: $0, object: window, queue: .main, using: update)
            } + [NotificationCenter.default.addObserver(forName: NSWindow.willEnterFullScreenNotification, object: window, queue: .main, using: leaving)]
            let full = window.styleMask.contains(.fullScreen)
            fullScreen.wrappedValue = full
            window.titleVisibility = .hidden
            window.titlebarAppearsTransparent = true
            window.titlebarSeparatorStyle = .none
            Self.setTitlebarToolbar(window, present: !full)
        }

        /// An empty unified toolbar makes the title bar as tall as the
        /// header, so macOS centers the traffic lights in it. It has no
        /// items, so it draws no item separators.
        @MainActor
        private static func setTitlebarToolbar(_ window: NSWindow, present: Bool) {
            if present, window.toolbar == nil {
                window.toolbar = NSToolbar(identifier: "qadren.monitoring.titlebar")
                window.toolbarStyle = .unified
            } else if !present, window.toolbar != nil {
                window.toolbar = nil
            }
        }

        deinit { observers.forEach(NotificationCenter.default.removeObserver) }
    }
}
