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
            // In a window the empty title bar toolbar lies over this bar and
            // takes its clicks, so the controls are a toolbar item there
            // (found on 2026-11-04). Full screen has no toolbar.
            if fullScreen {
                controls
            }
        }
        .padding(.leading, leading)
        .padding(.trailing, 18)
        .frame(height: Self.height)
        .frame(maxWidth: .infinity)
        .background(QDCyber.color(.card, for: colorScheme))
        .overlay(alignment: .bottom) { bottomEdge }
        .background(WindowFullScreenReader(fullScreen: $fullScreen, controls: controls).frame(width: 0, height: 0))
    }

    private var controls: MonitoringLayoutControls {
        MonitoringLayoutControls(
            isEditing: isEditing,
            setEditing: { isEditing = $0 },
            onAdd: onAdd
        )
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

/// "+ Виджет", "Готово", and "Компоновка" of the monitoring top bar.
struct MonitoringLayoutControls: View {
    let isEditing: Bool
    let setEditing: (Bool) -> Void
    let onAdd: (MonitoringWidgetKind) -> Void
    @Environment(\.colorScheme) private var colorScheme

    var body: some View {
        // Both states take the same room, so the toolbar item keeps one width
        // and does not move when the mode changes (found on 2026-11-04).
        ZStack(alignment: .trailing) {
            editingControls
                .opacity(isEditing ? 1 : 0)
                .allowsHitTesting(isEditing)
                .accessibilityHidden(!isEditing)
            layoutButton
                .opacity(isEditing ? 0 : 1)
                .allowsHitTesting(!isEditing)
                .accessibilityHidden(isEditing)
        }
        // A toolbar shows only the icon of a Label; the bar shows both.
        .labelStyle(.titleAndIcon)
        .fixedSize()
    }

    private var editingControls: some View {
        HStack(spacing: 12) {
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
            Button {
                setEditing(false)
            } label: {
                Label("Готово", systemImage: "checkmark")
            }
            .buttonStyle(QDCyberButtonStyle(tint: QDCyber.color(.accent, for: colorScheme), label: QDCyber.color(.onAccent, for: colorScheme)))
            .keyboardShortcut(isEditing ? .defaultAction : nil)
            .help("Завершить компоновку")
        }
    }

    private var layoutButton: some View {
        Button {
            setEditing(true)
        } label: {
            Label("Компоновка", systemImage: "square.grid.2x2")
        }
        .buttonStyle(QDCyberOutlineButtonStyle())
        .help("Изменить компоновку панели")
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
    let controls: MonitoringLayoutControls

    func makeCoordinator() -> Coordinator { Coordinator(fullScreen: $fullScreen) }

    func makeNSView(context: Context) -> NSView {
        let view = NSView(frame: .zero)
        context.coordinator.controls.rootView = AnyView(Self.inToolbar(controls))
        DispatchQueue.main.async { context.coordinator.attach(to: view.window) }
        return view
    }

    func updateNSView(_ nsView: NSView, context: Context) {
        context.coordinator.controls.rootView = AnyView(Self.inToolbar(controls))
        DispatchQueue.main.async { context.coordinator.attach(to: nsView.window) }
    }

    /// The toolbar puts its last item close to the window edge; the header
    /// keeps 18 pt there in full screen, so the item adds the difference.
    private static func inToolbar(_ controls: MonitoringLayoutControls) -> some View {
        controls.padding(.trailing, 10)
    }

    @MainActor
    final class Coordinator: NSObject, NSToolbarDelegate {
        private var fullScreen: Binding<Bool>
        private weak var window: NSWindow?
        private var observers: [NSObjectProtocol] = []
        /// The layout controls as the one toolbar item, so that a click
        /// reaches them in a window.
        let controls = NSHostingView(rootView: AnyView(EmptyView()))
        private static let controlsItem = NSToolbarItem.Identifier("qadren.monitoring.layout")

        init(fullScreen: Binding<Bool>) { self.fullScreen = fullScreen }

        func toolbarDefaultItemIdentifiers(_ toolbar: NSToolbar) -> [NSToolbarItem.Identifier] {
            [.flexibleSpace, Self.controlsItem]
        }

        func toolbarAllowedItemIdentifiers(_ toolbar: NSToolbar) -> [NSToolbarItem.Identifier] {
            toolbarDefaultItemIdentifiers(toolbar)
        }

        func toolbar(
            _ toolbar: NSToolbar, itemForItemIdentifier itemIdentifier: NSToolbarItem.Identifier,
            willBeInsertedIntoToolbar flag: Bool
        ) -> NSToolbarItem? {
            guard itemIdentifier == Self.controlsItem else { return nil }
            let item = NSToolbarItem(itemIdentifier: itemIdentifier)
            item.view = controls
            item.isBordered = false
            item.label = "Компоновка"
            return item
        }

        func attach(to window: NSWindow?) {
            guard let window, window !== self.window else { return }
            self.window = window
            observers.forEach(NotificationCenter.default.removeObserver)
            let update: @Sendable (Notification) -> Void = { [weak self, weak window] _ in
                MainActor.assumeIsolated {
                    guard let self, let window else { return }
                    let full = window.styleMask.contains(.fullScreen)
                    self.fullScreen.wrappedValue = full
                    self.setTitlebarToolbar(window, present: !full)
                }
            }
            // Full screen shows a toolbar as a separate bar over the content,
            // so it goes before the transition and comes back after it.
            let leaving: @Sendable (Notification) -> Void = { [weak self, weak window] _ in
                MainActor.assumeIsolated {
                    if let self, let window { self.setTitlebarToolbar(window, present: false) }
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
            setTitlebarToolbar(window, present: !full)
        }

        /// A unified toolbar makes the title bar as tall as the header, so
        /// macOS centers the traffic lights in it. Its only item is the
        /// layout controls; the rest stays empty.
        private func setTitlebarToolbar(_ window: NSWindow, present: Bool) {
            if present, window.toolbar == nil {
                let toolbar = NSToolbar(identifier: "qadren.monitoring.titlebar")
                toolbar.delegate = self
                toolbar.allowsUserCustomization = false
                toolbar.displayMode = .iconOnly
                window.toolbar = toolbar
                window.toolbarStyle = .unified
            } else if !present, window.toolbar != nil {
                window.toolbar = nil
            }
        }

        deinit { observers.forEach(NotificationCenter.default.removeObserver) }
    }
}
