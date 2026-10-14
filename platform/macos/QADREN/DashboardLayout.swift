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

struct MonitoringWidgetLayout: Codable, Identifiable, Equatable {
    let kind: MonitoringWidgetKind
    var isVisible: Bool

    var id: MonitoringWidgetKind { kind }
}

/// Local-only presentation state for one monitoring window. It never owns Jobs,
/// connections, or file operations; a corrupt saved layout falls back safely
/// without overwriting the original data.
@MainActor
final class DashboardLayoutStore: ObservableObject {
    @Published private(set) var widgets = MonitoringWidgetKind.allCases.map {
        MonitoringWidgetLayout(kind: $0, isVisible: true)
    }

    private let defaults: UserDefaults
    private var panelID: String?
    private var undoStack: [[MonitoringWidgetLayout]] = []
    private var redoStack: [[MonitoringWidgetLayout]] = []
    private let historyLimit = 50

    init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
    }

    static func storageKey(for panelID: String) -> String {
        "qadren.monitoring.layout." + panelID
    }

    var visibleWidgets: [MonitoringWidgetLayout] {
        widgets.filter(\.isVisible)
    }

    var hiddenWidgets: [MonitoringWidgetLayout] {
        widgets.filter { !$0.isVisible }
    }

    var canUndo: Bool { !undoStack.isEmpty }

    var canRedo: Bool { !redoStack.isEmpty }

    func load(panelID: String) {
        self.panelID = panelID
        undoStack.removeAll()
        redoStack.removeAll()
        let key = Self.storageKey(for: panelID)
        guard let data = defaults.data(forKey: key),
              let decoded = try? JSONDecoder().decode([MonitoringWidgetLayout].self, from: data),
              isValid(decoded) else {
            widgets = defaultWidgets
            return
        }
        widgets = decoded
    }

    func move(_ kind: MonitoringWidgetKind, by offset: Int) {
        guard let index = widgets.firstIndex(where: { $0.kind == kind }) else { return }
        let destination = index + offset
        guard widgets.indices.contains(destination) else { return }
        recordUndoState()
        widgets.swapAt(index, destination)
        save()
    }

    func setVisible(_ kind: MonitoringWidgetKind, isVisible: Bool) {
        guard let index = widgets.firstIndex(where: { $0.kind == kind }) else { return }
        guard widgets[index].isVisible != isVisible else { return }
        recordUndoState()
        widgets[index].isVisible = isVisible
        save()
    }

    func reset() {
        guard widgets != defaultWidgets else { return }
        recordUndoState()
        widgets = defaultWidgets
        save()
    }

    func undo() {
        guard let previous = undoStack.popLast() else { return }
        redoStack.append(widgets)
        widgets = previous
        save()
    }

    func redo() {
        guard let next = redoStack.popLast() else { return }
        undoStack.append(widgets)
        widgets = next
        save()
    }

    private var defaultWidgets: [MonitoringWidgetLayout] {
        MonitoringWidgetKind.allCases.map { MonitoringWidgetLayout(kind: $0, isVisible: true) }
    }

    private func isValid(_ candidate: [MonitoringWidgetLayout]) -> Bool {
        candidate.count == MonitoringWidgetKind.allCases.count
            && Set(candidate.map(\.kind)) == Set(MonitoringWidgetKind.allCases)
    }

    private func recordUndoState() {
        undoStack.append(widgets)
        if undoStack.count > historyLimit {
            undoStack.removeFirst(undoStack.count - historyLimit)
        }
        redoStack.removeAll()
    }

    private func save() {
        guard let panelID, let data = try? JSONEncoder().encode(widgets) else { return }
        defaults.set(data, forKey: Self.storageKey(for: panelID))
    }
}
