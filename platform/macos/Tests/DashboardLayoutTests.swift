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

        check(store.canUndo, "layout edits enable undo")
        store.undo()
        check(store.hiddenWidgets.isEmpty, "undo restores widget visibility")
        check(store.widgets.map(\.kind) == [.system, .jobs, .connections, .queue], "undo preserves earlier ordering edit")
        check(store.canRedo, "undo enables redo")
        store.undo()
        check(store.widgets.map(\.kind) == MonitoringWidgetKind.allCases, "second undo restores default ordering")
        store.redo()
        store.redo()
        check(store.hiddenWidgets.map(\.kind) == [.jobs], "redo restores the full edit sequence")

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

        let bounded = DashboardLayoutStore(defaults: defaults)
        bounded.load(panelID: "bounded-history")
        for index in 0..<55 {
            bounded.setVisible(.system, isVisible: !index.isMultiple(of: 2))
        }
        for _ in 0..<50 { bounded.undo() }
        check(!bounded.canUndo, "layout history retains no more than fifty states")

        print("Dashboard layout native tests passed")
    }
}
