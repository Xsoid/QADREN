import Foundation

@main
struct DashboardLayoutTests {
    static func main() {
        let suite = "qadren-dashboard-layout-tests-\(UUID().uuidString)"
        guard let defaults = UserDefaults(suiteName: suite) else { fatalError("defaults unavailable") }
        defer { defaults.removePersistentDomain(forName: suite) }

        let panelID = "main"
        let store = DashboardLayoutStore(defaults: defaults)
        store.load(panelID: panelID)
        check(store.widgets.map(\.kind) == MonitoringWidgetKind.allCases, "fixed widget set")

        store.move(.connections, by: -1)
        store.setDensity(.connections, density: .expanded)
        check(store.canUndo, "layout edits enable undo")
        store.undo()
        check(store.widgets.first(where: { $0.kind == .connections })?.density == .regular, "undo restores density")
        store.redo()
        check(store.widgets.first(where: { $0.kind == .connections })?.density == .expanded, "redo restores density")

        let restored = DashboardLayoutStore(defaults: defaults)
        restored.load(panelID: panelID)
        check(restored.widgets.map(\.kind) == [.system, .jobs, .connections, .queue], "order persists")
        check(restored.widgets.first(where: { $0.kind == .connections })?.density == .expanded, "density persists")
    }

    private static func check(_ condition: @autoclosure () -> Bool, _ message: String) {
        guard condition() else { fatalError(message) }
    }
}
