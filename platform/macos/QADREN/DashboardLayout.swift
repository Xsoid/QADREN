import Combine
import Foundation

/// The widget type registry. The raw value is the saved `typeID`; the view for
/// each type lives in `Widgets/` and is selected by `MonitoringWidgetView`.
enum MonitoringWidgetKind: String, CaseIterable, Codable, Identifiable {
    case system
    case scenario
    case scenarios
    case connections

    var id: String { rawValue }

    var title: String {
        switch self {
        case .system: "Система"
        case .scenario: "Сценарий"
        case .scenarios: "Сценарии"
        case .connections: "Подключения"
        }
    }

    var symbol: String {
        switch self {
        case .system: "cpu"
        case .scenario: "arrow.triangle.2.circlepath"
        case .scenarios: "list.bullet.rectangle"
        case .connections: "externaldrive"
        }
    }

    /// The sizes that the widget type draws. One size means no choice.
    var supportedDensities: [MonitoringWidgetDensity] {
        switch self {
        case .system, .scenarios: [.regular]
        case .scenario, .connections: MonitoringWidgetDensity.allCases
        }
    }

    /// One line for the widget catalog.
    var summary: String {
        switch self {
        case .system: "Фоновая служба Worker: работает ли синхронизация"
        case .scenario: "Один выбранный сценарий: этап, прогресс, запуск и остановка"
        case .scenarios: "Все сценарии по ближайшему запуску: направление и расписание"
        case .connections: "Подключения хранилищ и их состояние"
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

/// One widget instance on the panel. A panel can hold several instances of
/// one type, so the instance `id` and not the `kind` identifies it.
struct MonitoringWidgetLayout: Codable, Identifiable, Equatable {
    let id: UUID
    let kind: MonitoringWidgetKind
    var density: MonitoringWidgetDensity
    /// The Scenario that a `scenario` widget shows; `nil` until selected.
    var scenarioID: String? = nil
    /// The rows that a `scenarios` widget shows; `nil` shows all.
    var limit: Int? = nil

    init(
        id: UUID = UUID(),
        kind: MonitoringWidgetKind,
        density: MonitoringWidgetDensity = .regular
    ) {
        self.id = id
        self.kind = kind
        self.density = density
    }

    private enum CodingKeys: String, CodingKey {
        case id
        case kind
        case density
        case scenarioID
        case limit
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        // Layouts saved before widget instances had no `id`.
        id = try container.decodeIfPresent(UUID.self, forKey: .id) ?? UUID()
        kind = try container.decode(MonitoringWidgetKind.self, forKey: .kind)
        density = try container.decodeIfPresent(MonitoringWidgetDensity.self, forKey: .density) ?? .regular
        scenarioID = try container.decodeIfPresent(String.self, forKey: .scenarioID)
        limit = try container.decodeIfPresent(Int.self, forKey: .limit).map { max(1, $0) }
    }
}

private struct MonitoringLayoutState: Codable, Equatable {
    var widgets: [MonitoringWidgetLayout]

    init(widgets: [MonitoringWidgetLayout]) {
        self.widgets = widgets
    }

    /// A widget of a removed type (for example the former `jobs`) is dropped;
    /// the rest of the saved layout stays.
    init(from decoder: Decoder) throws {
        struct Element: Decodable {
            let widget: MonitoringWidgetLayout?
            init(from decoder: Decoder) throws {
                widget = try? MonitoringWidgetLayout(from: decoder)
            }
        }
        let container = try decoder.container(keyedBy: CodingKeys.self)
        widgets = try container.decode([Element].self, forKey: .widgets).compactMap(\.widget)
    }

    private enum CodingKeys: String, CodingKey {
        case widgets
    }
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

    func add(_ kind: MonitoringWidgetKind) {
        recordUndoState()
        widgets.append(MonitoringWidgetLayout(kind: kind))
        save()
    }

    /// Removes the widget from the panel only. Jobs, connections, and their
    /// history do not change.
    func remove(_ id: UUID) {
        guard let index = widgets.firstIndex(where: { $0.id == id }) else { return }
        recordUndoState()
        widgets.remove(at: index)
        save()
    }

    func move(_ id: UUID, by offset: Int) {
        guard let index = widgets.firstIndex(where: { $0.id == id }) else { return }
        let destination = index + offset
        guard widgets.indices.contains(destination) else { return }
        recordUndoState()
        widgets.swapAt(index, destination)
        save()
    }

    func setDensity(_ id: UUID, density: MonitoringWidgetDensity) {
        guard let index = widgets.firstIndex(where: { $0.id == id }) else { return }
        guard widgets[index].density != density else { return }
        recordUndoState()
        widgets[index].density = density
        save()
    }

    func setScenario(_ id: UUID, scenarioID: String) {
        guard let index = widgets.firstIndex(where: { $0.id == id }) else { return }
        guard widgets[index].scenarioID != scenarioID else { return }
        recordUndoState()
        widgets[index].scenarioID = scenarioID
        save()
    }

    func setLimit(_ id: UUID, limit: Int?) {
        guard let index = widgets.firstIndex(where: { $0.id == id }) else { return }
        guard widgets[index].limit != limit else { return }
        recordUndoState()
        widgets[index].limit = limit
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
        Set(candidate.map(\.id)).count == candidate.count
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
