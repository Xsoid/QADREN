import SwiftUI

private final class AppScheduleDispatcher {
    static let shared = AppScheduleDispatcher()

    private let queue = DispatchQueue(label: "dev.qadren.schedule-dispatch", qos: .utility)
    private var timer: DispatchSourceTimer?

    private init() {}

    func start() {
        guard timer == nil else { return }
        let timer = DispatchSource.makeTimerSource(queue: queue)
        timer.schedule(deadline: .now(), repeating: .seconds(60), leeway: .seconds(5))
        timer.setEventHandler { [weak self] in self?.dispatchRecentSlots() }
        self.timer = timer
        timer.resume()
    }

    private func dispatchRecentSlots() {
        let through = Date()
        // The timer is not a catch-up service. The small overlap tolerates
        // ordinary timer drift, while Registry receipts deduplicate a slot.
        _ = try? StorageRegistryBridge.dispatchDueScenarioSchedules(
            after: through.addingTimeInterval(-90), through: through
        )
    }
}

@main
struct QADRENApp: App {
    @AppStorage("qadren.theme") private var themeRaw = QDThemeMode.dark.rawValue

    init() {
        AppScheduleDispatcher.shared.start()
    }

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
