import SwiftUI

/// Selects the view for one widget instance. Each widget type lives in its own
/// file in `Widgets/`; a new type adds a `MonitoringWidgetKind` case, a view
/// file, and one line here.
struct MonitoringWidgetView<Actions: View>: View {
    let widget: MonitoringWidgetLayout
    @ObservedObject var feed: MonitoringFeed
    @ObservedObject var connections: StorageConnections
    let isEditing: Bool
    let onSelectScenario: (String) -> Void
    @ViewBuilder let actions: Actions

    var body: some View {
        switch widget.kind {
        case .system:
            SystemWidget(feed: feed) { actions }
        case .scenario:
            ScenarioWidget(widget: widget, isEditing: isEditing, onSelectScenario: onSelectScenario) { actions }
        case .scenarios:
            ScenariosWidget(limit: widget.limit) { actions }
        case .connections:
            ConnectionsWidget(density: widget.density, connections: connections) { actions }
        }
    }
}
