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
            MenuBarWorkspaceMenu()
        }
    }
}

private struct MenuBarWorkspaceMenu: View {
    @Environment(\.openWindow) private var openWindow

    var body: some View {
        Text("QADREN \(CoreBridge.version)")
        Text(coreStatus).foregroundStyle(.secondary)
        Divider()
        Button("Открыть рабочую панель") { open(id: "monitoring") }
        Button("Новая рабочая панель") { open(id: "monitoring") }
        Button("Настройка сценариев") { open(id: "scenario-settings") }
        Divider()
        Button("Выйти из QADREN") { NSApp.terminate(nil) }
    }

    private var coreStatus: String {
        switch CoreBridge.status {
        case "ready - worker idle", "Core ready - worker idle": "Готово — Worker простаивает"
        default: CoreBridge.status
        }
    }

    private func open(id: String) {
        openWindow(id: id)
        NSApp.activate(ignoringOtherApps: true)
    }
}
