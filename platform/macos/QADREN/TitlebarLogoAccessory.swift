import AppKit
import SwiftUI

/// SwiftUI's `.navigation` toolbar placement renders the QADREN mark as a
/// toolbar control. The native titlebar accessory keeps it a brand mark and
/// reserves space for the standard window title instead.
struct TitlebarLogoAccessory: NSViewRepresentable {
    func makeCoordinator() -> Coordinator { Coordinator() }

    func makeNSView(context: Context) -> NSView {
        let host = NSView(frame: .zero)
        DispatchQueue.main.async {
            context.coordinator.install(in: host)
        }
        return host
    }

    func updateNSView(_ nsView: NSView, context: Context) {
        DispatchQueue.main.async {
            context.coordinator.install(in: nsView)
        }
    }

    static func dismantleNSView(_ nsView: NSView, coordinator: Coordinator) {
        coordinator.remove()
    }

    final class Coordinator {
        private weak var window: NSWindow?
        private var accessory: NSTitlebarAccessoryViewController?

        func install(in host: NSView) {
            guard accessory == nil, let window = host.window else { return }
            let image = Bundle.main.image(forResource: "qadren-logo")
            let imageView = NSImageView(image: image ?? NSImage())
            imageView.imageScaling = .scaleProportionallyUpOrDown
            imageView.translatesAutoresizingMaskIntoConstraints = false

            let container = NSView(frame: NSRect(x: 0, y: 0, width: 28, height: 28))
            container.translatesAutoresizingMaskIntoConstraints = false
            container.addSubview(imageView)
            NSLayoutConstraint.activate([
                imageView.widthAnchor.constraint(equalToConstant: 20),
                imageView.heightAnchor.constraint(equalToConstant: 20),
                imageView.centerXAnchor.constraint(equalTo: container.centerXAnchor),
                imageView.centerYAnchor.constraint(equalTo: container.centerYAnchor),
                container.widthAnchor.constraint(equalToConstant: 28),
                container.heightAnchor.constraint(equalToConstant: 28),
            ])

            let controller = NSTitlebarAccessoryViewController()
            controller.view = container
            controller.layoutAttribute = .leading
            window.addTitlebarAccessoryViewController(controller)
            self.window = window
            accessory = controller
        }

        func remove() {
            guard let window, let accessory,
                  let index = window.titlebarAccessoryViewControllers.firstIndex(of: accessory)
            else { return }
            window.removeTitlebarAccessoryViewController(at: index)
            self.accessory = nil
        }
    }
}
