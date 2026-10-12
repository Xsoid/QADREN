import SwiftUI

@main
struct QADRENApp: App {
    @AppStorage("qadren.theme") private var themeRaw = QDThemeMode.dark.rawValue

    var body: some Scene {
        WindowGroup("Контрольная панель", id: "monitoring") {
            MainView(workspace: .monitoring)
                .frame(minWidth: 900, minHeight: 620)
                .preferredColorScheme(QDThemeMode(rawValue: themeRaw)?.preferredColorScheme)
        }
        .defaultSize(width: 1180, height: 760)

        Window("Настройки", id: "scenario-settings") {
            MainView(workspace: .scenarios)
                .frame(minWidth: 760, minHeight: 560)
                .preferredColorScheme(QDThemeMode(rawValue: themeRaw)?.preferredColorScheme)
        }
        .defaultSize(width: 980, height: 700)

        Window("Файловый менеджер", id: "file-manager") {
            MainView(workspace: .files)
                .frame(minWidth: 900, minHeight: 620)
                .preferredColorScheme(QDThemeMode(rawValue: themeRaw)?.preferredColorScheme)
        }
        .defaultSize(width: 1180, height: 760)

        MenuBarExtra("QADREN", systemImage: "externaldrive.fill") {
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
        Button("Открыть контрольную панель") { open(id: "monitoring") }
        Button("Новая контрольная панель") { open(id: "monitoring") }
        Button("Файловый менеджер") { open(id: "file-manager") }
        Button("Настройки") { open(id: "scenario-settings") }
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
