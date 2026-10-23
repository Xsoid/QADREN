import Foundation

/// Presentation-only summary of the read-only Rust JobStore inventory. It does
/// not authorize, submit, retry, or reconcile Jobs.
enum JobPresentationStatus: Equatable {
    /// The JobStore was read and holds no Jobs. This is not "queued".
    case empty
    case ready
    case active
    case waiting
    case paused
    case needsAction
    case failed
    case unknown
    case completed
    case cancelled

    init(state: String) {
        switch state {
        case "queued": self = .waiting
        case "running": self = .active
        case "paused": self = .paused
        case "waiting_for_decision": self = .needsAction
        case "completed": self = .completed
        case "failed": self = .failed
        case "cancelled": self = .cancelled
        default: self = .unknown
        }
    }

    static func aggregate(_ states: [String], inventoryFailed: Bool) -> JobPresentationStatus {
        if inventoryFailed { return .failed }
        guard !states.isEmpty else { return .empty }
        let presentations = states.map(JobPresentationStatus.init(state:))
        if presentations.contains(.failed) { return .failed }
        if presentations.contains(.unknown) { return .unknown }
        if presentations.contains(.needsAction) { return .needsAction }
        if presentations.contains(.active) { return .active }
        if presentations.contains(.paused) { return .paused }
        if presentations.contains(.waiting) { return .waiting }
        return .ready
    }

    var title: String {
        switch self {
        case .empty: "Нет заданий"
        case .ready: "Готово"
        case .active: "Выполняется"
        case .waiting: "В очереди"
        case .paused: "Приостановлено"
        case .needsAction: "Требует решения"
        case .failed: "Ошибка"
        case .unknown: "Результат неизвестен"
        case .completed: "Завершено"
        case .cancelled: "Отменено"
        }
    }
}

/// Counts only the states that affect the live Queue presentation. The source
/// is the read-only JobStore inventory; terminal history is intentionally not
/// presented as active work or a risk.
struct JobQueueSummary: Equatable {
    let active: Int
    let waiting: Int
    let unknown: Int

    init(active: Int, waiting: Int, unknown: Int) {
        self.active = active
        self.waiting = waiting
        self.unknown = unknown
    }

    init(states: [String]) {
        let presentations = states.map(JobPresentationStatus.init(state:))
        active = presentations.filter { $0 == .active }.count
        waiting = presentations.filter { $0 == .waiting }.count
        unknown = presentations.filter { $0 == .unknown }.count
    }
}
