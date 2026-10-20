import Combine
import Foundation

enum MonitoringWidgetKind: String, CaseIterable, Codable, Identifiable {
    case system
    case jobs
    case queue
    case connections

    var id: String { rawValue }

    var title: String {
        switch self {
        case .system: "Система"
        case .jobs: "Задания"
        case .queue: "Очередь"
        case .connections: "Подключения"
        }
    }
}

/// The amount of secondary information shown by one widget. This is local
/// presentation state, not a request to recalculate or mutate Core state.
enum MonitoringWidgetDensity: String, CaseIterable, Codable, Identifiable {
    case compact
    case regular
    case expanded

    var id: String { rawValue }

    var title: String {
        switch self {
        case .compact: "Компактно"
        case .regular: "Обычно"
        case .expanded: "Подробно"
        }
    }
}

struct MonitoringWidgetLayout: Codable, Identifiable, Equatable {
    let kind: MonitoringWidgetKind
    var density: MonitoringWidgetDensity

    var id: MonitoringWidgetKind { kind }

    init(
        kind: MonitoringWidgetKind,
        density: MonitoringWidgetDensity = .regular
    ) {
        self.kind = kind
        self.density = density
    }

    private enum CodingKeys: String, CodingKey {
        case kind
        case density
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        kind = try container.decode(MonitoringWidgetKind.self, forKey: .kind)
        density = try container.decodeIfPresent(MonitoringWidgetDensity.self, forKey: .density) ?? .regular
    }
}

private struct MonitoringLayoutState: Codable, Equatable {
    var widgets: [MonitoringWidgetLayout]
}

/// Local-only presentation state for one monitoring window. It never owns Jobs,
/// connections, or file operations; a corrupt saved layout falls back safely
/// without overwriting the original data.
@MainActor
final class DashboardLayoutStore: ObservableObject {
    @Published private(set) var widgets = MonitoringWidgetKind.allCases.map {
        MonitoringWidgetLayout(kind: $0)
    }
    /// A saved layout can be unreadable after an interrupted preference write or
    /// an incompatible manual edit. The original bytes are intentionally kept;
    /// this flag lets the UI explain why the safe default is on screen.
    @Published private(set) var recoveredFromInvalidLayout = false

    private let defaults: UserDefaults
    private var panelID: String?
    private var undoStack: [MonitoringLayoutState] = []
    private var redoStack: [MonitoringLayoutState] = []
    private let historyLimit = 50

    init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
    }

    static func storageKey(for panelID: String) -> String {
        "qadren.monitoring.layout." + panelID
    }

    var canUndo: Bool { !undoStack.isEmpty }

    var canRedo: Bool { !redoStack.isEmpty }

    func load(panelID: String) {
        self.panelID = panelID
        undoStack.removeAll()
        redoStack.removeAll()
        recoveredFromInvalidLayout = false
        let key = Self.storageKey(for: panelID)
        guard let data = defaults.data(forKey: key) else {
            widgets = defaultWidgets
            return
        }
        if let decoded = try? JSONDecoder().decode(MonitoringLayoutState.self, from: data),
           isValid(decoded.widgets) {
            restore(decoded)
            return
        }
        widgets = defaultWidgets
        recoveredFromInvalidLayout = true
    }

    func move(_ kind: MonitoringWidgetKind, by offset: Int) {
        guard let index = widgets.firstIndex(where: { $0.kind == kind }) else { return }
        let destination = index + offset
        guard widgets.indices.contains(destination) else { return }
        recordUndoState()
        widgets.swapAt(index, destination)
        save()
    }

    func setDensity(_ kind: MonitoringWidgetKind, density: MonitoringWidgetDensity) {
        guard let index = widgets.firstIndex(where: { $0.kind == kind }) else { return }
        guard widgets[index].density != density else { return }
        recordUndoState()
        widgets[index].density = density
        save()
    }

    func undo() {
        guard let previous = undoStack.popLast() else { return }
        redoStack.append(currentState)
        restore(previous)
        save()
    }

    func redo() {
        guard let next = redoStack.popLast() else { return }
        undoStack.append(currentState)
        restore(next)
        save()
    }

    private var defaultWidgets: [MonitoringWidgetLayout] {
        MonitoringWidgetKind.allCases.map { MonitoringWidgetLayout(kind: $0) }
    }

    private var currentState: MonitoringLayoutState {
        MonitoringLayoutState(widgets: widgets)
    }

    private func isValid(_ candidate: [MonitoringWidgetLayout]) -> Bool {
        candidate.count == MonitoringWidgetKind.allCases.count
            && Set(candidate.map(\.kind)) == Set(MonitoringWidgetKind.allCases)
    }

    private func recordUndoState() {
        undoStack.append(currentState)
        if undoStack.count > historyLimit {
            undoStack.removeFirst(undoStack.count - historyLimit)
        }
        redoStack.removeAll()
    }

    private func save() {
        guard let panelID, let data = try? JSONEncoder().encode(currentState) else { return }
        defaults.set(data, forKey: Self.storageKey(for: panelID))
        recoveredFromInvalidLayout = false
    }

    private func restore(_ state: MonitoringLayoutState) {
        widgets = state.widgets
    }
}
