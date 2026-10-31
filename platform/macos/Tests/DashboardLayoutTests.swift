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
        check(store.widgets.map(\.kind) == MonitoringWidgetKind.allCases, "default widget set")

        guard let connections = store.widgets.first(where: { $0.kind == .connections })?.id else {
            fatalError("default connections widget")
        }
        store.move(connections, by: -1)
        store.setDensity(connections, density: .expanded)
        check(store.canUndo, "layout edits enable undo")
        store.undo()
        check(store.widgets.first(where: { $0.id == connections })?.density == .regular, "undo restores density")
        store.redo()
        check(store.widgets.first(where: { $0.id == connections })?.density == .expanded, "redo restores density")

        store.add(.connections)
        check(store.widgets.filter { $0.kind == .connections }.count == 2, "one type can have several instances")
        guard let system = store.widgets.first(where: { $0.kind == .system })?.id else {
            fatalError("default system widget")
        }
        store.remove(system)
        check(!store.widgets.contains { $0.kind == .system }, "remove takes the widget off the panel")
        store.undo()
        check(store.widgets.first?.id == system, "undo returns a removed widget")
        store.remove(system)

        let restored = DashboardLayoutStore(defaults: defaults)
        restored.load(panelID: panelID)
        check(restored.widgets.map(\.kind) == [.connections, .scenario, .connections], "order and instances persist")
        check(restored.widgets.first(where: { $0.id == connections })?.density == .expanded, "density persists")
    }

    private static func check(_ condition: @autoclosure () -> Bool, _ message: String) {
        guard condition() else { fatalError(message) }
    }
}
