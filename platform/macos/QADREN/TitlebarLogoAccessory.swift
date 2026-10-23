import AppKit
import SwiftUI

/// SwiftUI's `.navigation` toolbar placement renders the QADREN mark as a
/// toolbar control. The native titlebar accessory keeps it a brand mark and
/// reserves space for the standard window title instead.
///
/// With `toggleSidebar`, the accessory also shows the sidebar button right
/// after the mark. It stays in this one place when the sidebar is visible
/// and when it is hidden, so the control never moves or overflows.
struct TitlebarLogoAccessory: NSViewRepresentable {
    var toggleSidebar: (() -> Void)?
    var sidebarVisible = true

    func makeCoordinator() -> Coordinator { Coordinator() }

    func makeNSView(context: Context) -> NSView {
        let host = NSView(frame: .zero)
        context.coordinator.toggle = toggleSidebar
        DispatchQueue.main.async {
            context.coordinator.install(in: host)
        }
        return host
    }

    func updateNSView(_ nsView: NSView, context: Context) {
        context.coordinator.toggle = toggleSidebar
        context.coordinator.sidebarVisible = sidebarVisible
        DispatchQueue.main.async {
            context.coordinator.install(in: nsView)
            context.coordinator.updateSidebarButton()
        }
    }

    static func dismantleNSView(_ nsView: NSView, coordinator: Coordinator) {
        coordinator.remove()
    }

    final class Coordinator: NSObject {
        private weak var window: NSWindow?
        private var accessory: NSTitlebarAccessoryViewController?
        private var sidebarButton: NSButton?
        var toggle: (() -> Void)?
        var sidebarVisible = true

        @objc private func toggleSidebar(_ sender: Any?) { toggle?() }

        func updateSidebarButton() {
            sidebarButton?.toolTip = sidebarVisible ? "Скрыть меню" : "Показать меню"
            sidebarButton?.setAccessibilityLabel(sidebarVisible ? "Скрыть меню" : "Показать меню")
        }

        func install(in host: NSView) {
            guard accessory == nil, let window = host.window else { return }
            let image = Bundle.main.image(forResource: "qadren-logo")
            let imageView = NSImageView(image: image ?? NSImage())
            imageView.imageScaling = .scaleProportionallyUpOrDown
            imageView.translatesAutoresizingMaskIntoConstraints = false

            let width: CGFloat = toggle == nil ? 28 : 64
            let container = NSView(frame: NSRect(x: 0, y: 0, width: width, height: 28))
            container.translatesAutoresizingMaskIntoConstraints = false
            container.addSubview(imageView)
            NSLayoutConstraint.activate([
                imageView.widthAnchor.constraint(equalToConstant: 20),
                imageView.heightAnchor.constraint(equalToConstant: 20),
                imageView.centerXAnchor.constraint(equalTo: container.leadingAnchor, constant: 14),
                imageView.centerYAnchor.constraint(equalTo: container.centerYAnchor),
                container.widthAnchor.constraint(equalToConstant: width),
                container.heightAnchor.constraint(equalToConstant: 28),
            ])
            if toggle != nil {
                let button = NSButton(
                    image: NSImage(systemSymbolName: "sidebar.left", accessibilityDescription: nil) ?? NSImage(),
                    target: self,
                    action: #selector(toggleSidebar(_:))
                )
                button.bezelStyle = .texturedRounded
                button.isBordered = false
                button.imagePosition = .imageOnly
                button.translatesAutoresizingMaskIntoConstraints = false
                container.addSubview(button)
                NSLayoutConstraint.activate([
                    button.widthAnchor.constraint(equalToConstant: 28),
                    button.heightAnchor.constraint(equalToConstant: 28),
                    button.leadingAnchor.constraint(equalTo: container.leadingAnchor, constant: 34),
                    button.centerYAnchor.constraint(equalTo: container.centerYAnchor),
                ])
                sidebarButton = button
                updateSidebarButton()
            }

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
