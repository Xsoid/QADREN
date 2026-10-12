import SwiftUI

@main
struct QADRENApp: App {
    var body: some Scene {
        Window("QADREN", id: "main") {
            MainView()
                .frame(minWidth: 900, minHeight: 620)
        }
        .defaultSize(width: 1180, height: 760)

        MenuBarExtra("QADREN", systemImage: "externaldrive.badge.icloud") {
            Text("QADREN \(CoreBridge.version)")
            Text(CoreBridge.status)
            Divider()
            Button("Open QADREN") {
                NSApp.activate(ignoringOtherApps: true)
                if let window = NSApp.windows.first(where: { $0.canBecomeMain }) {
                    window.makeKeyAndOrderFront(nil)
                }
            }
            Divider()
            Button("Quit QADREN") { NSApp.terminate(nil) }
        }
    }
}
