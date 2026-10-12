import SwiftUI

@main
struct QADRENApp: App {
    var body: some Scene {
        WindowGroup("QADREN", id: "monitoring") {
            MainView(workspace: .monitoring)
                .frame(minWidth: 900, minHeight: 620)
        }
        .defaultSize(width: 1180, height: 760)

        Window("Настройка сценариев", id: "scenario-settings") {
            MainView(workspace: .scenarios)
                .frame(minWidth: 760, minHeight: 560)
        }
        .defaultSize(width: 980, height: 700)

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
