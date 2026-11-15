import AppKit
import SwiftUI

/// Whether the window of a view is visible on screen: not minimized, not
/// hidden, not on another Space, and not fully covered by other windows.
/// Continuous motion stops while the window is not visible, so that a panel
/// that nobody sees costs no CPU time.
private struct QDWindowVisibleKey: EnvironmentKey {
    static let defaultValue = true
}

extension EnvironmentValues {
    var qdWindowVisible: Bool {
        get { self[QDWindowVisibleKey.self] }
        set { self[QDWindowVisibleKey.self] = newValue }
    }
}

extension View {
    /// Publishes `qdWindowVisible` for the window that shows this view.
    func qdTracksWindowVisibility() -> some View {
        modifier(QDWindowVisibilityModifier())
    }
}

private struct QDWindowVisibilityModifier: ViewModifier {
    @State private var visible = true

    func body(content: Content) -> some View {
        content
            .environment(\.qdWindowVisible, visible)
            .background(QDWindowVisibilityReader(visible: $visible).frame(width: 0, height: 0))
    }
}

/// Reads `NSWindow.occlusionState`, which macOS updates when the window is
/// minimized, hidden, covered, or the display sleeps.
private struct QDWindowVisibilityReader: NSViewRepresentable {
    @Binding var visible: Bool

    func makeCoordinator() -> Coordinator { Coordinator(visible: $visible) }

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
        private var visible: Binding<Bool>
        private weak var window: NSWindow?
        private var observer: NSObjectProtocol?

        init(visible: Binding<Bool>) { self.visible = visible }

        func attach(to window: NSWindow?) {
            guard let window, window !== self.window else { return }
            self.window = window
            if let observer { NotificationCenter.default.removeObserver(observer) }
            observer = NotificationCenter.default.addObserver(
                forName: NSWindow.didChangeOcclusionStateNotification, object: window, queue: .main
            ) { [weak self, weak window] _ in
                MainActor.assumeIsolated {
                    if let self, let window { self.publish(window) }
                }
            }
            publish(window)
        }

        private func publish(_ window: NSWindow) {
            let now = window.occlusionState.contains(.visible)
            if visible.wrappedValue != now { visible.wrappedValue = now }
        }

        deinit {
            if let observer { NotificationCenter.default.removeObserver(observer) }
        }
    }
}
