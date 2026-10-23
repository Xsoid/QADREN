import AppKit
import SwiftUI

/// AppKit gives the first focusable control the keyboard focus when a window
/// opens. On the monitoring canvas that is an arbitrary widget button, which
/// then shows a permanent focus ring. This view clears the first responder
/// one time, when the window first becomes key. Tab still moves the focus
/// into the window, and the system focus ring stays visible.
struct InitialFocusReset: NSViewRepresentable {
    func makeCoordinator() -> Coordinator { Coordinator() }

    func makeNSView(context: Context) -> NSView {
        let host = NSView(frame: .zero)
        DispatchQueue.main.async {
            context.coordinator.attach(to: host.window)
        }
        return host
    }

    func updateNSView(_ nsView: NSView, context: Context) {}

    static func dismantleNSView(_ nsView: NSView, coordinator: Coordinator) {
        coordinator.detach()
    }

    final class Coordinator {
        private var observer: NSObjectProtocol?
        private var done = false

        func attach(to window: NSWindow?) {
            guard !done, observer == nil, let window else { return }
            if window.isKeyWindow {
                clear(window)
                return
            }
            observer = NotificationCenter.default.addObserver(
                forName: NSWindow.didBecomeKeyNotification,
                object: window,
                queue: .main
            ) { [weak self] _ in
                self?.clear(window)
            }
        }

        private func clear(_ window: NSWindow) {
            done = true
            detach()
            // SwiftUI assigns the initial focus after the window becomes key.
            DispatchQueue.main.async {
                window.makeFirstResponder(nil)
            }
        }

        func detach() {
            if let observer {
                NotificationCenter.default.removeObserver(observer)
            }
            observer = nil
        }
    }
}
