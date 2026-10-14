import Foundation

private func check(_ condition: @autoclosure () -> Bool, _ message: String) {
    guard condition() else { fatalError("FAIL: " + message) }
}

@main
struct DashboardLayoutTests {
    @MainActor
    static func main() {
        let suite = "qadren-dashboard-layout-tests-" + UUID().uuidString
        let defaults = UserDefaults(suiteName: suite)!
        defer { defaults.removePersistentDomain(forName: suite) }

        let panelID = "panel-a"
        let store = DashboardLayoutStore(defaults: defaults)
        store.load(panelID: panelID)
        check(store.visibleWidgets.map(\.kind) == MonitoringWidgetKind.allCases, "new panel has each standard widget")

        store.move(.connections, by: -1)
        store.setVisible(.jobs, isVisible: false)

        let restored = DashboardLayoutStore(defaults: defaults)
        restored.load(panelID: panelID)
        check(restored.widgets.map(\.kind) == [.system, .jobs, .connections, .queue], "widget order persists per panel")
        check(restored.hiddenWidgets.map(\.kind) == [.jobs], "hidden widget persists per panel")

        let otherPanel = DashboardLayoutStore(defaults: defaults)
        otherPanel.load(panelID: "panel-b")
        check(otherPanel.visibleWidgets.map(\.kind) == MonitoringWidgetKind.allCases, "separate panel starts independently")

        let corruptPanel = "corrupt"
        defaults.set(Data("not-json".utf8), forKey: DashboardLayoutStore.storageKey(for: corruptPanel))
        let recovered = DashboardLayoutStore(defaults: defaults)
        recovered.load(panelID: corruptPanel)
        check(recovered.visibleWidgets.count == MonitoringWidgetKind.allCases.count, "corrupt layout falls back safely")
        check(defaults.data(forKey: DashboardLayoutStore.storageKey(for: corruptPanel)) == Data("not-json".utf8), "corrupt layout is not erased during recovery")

        print("Dashboard layout native tests passed")
    }
}
