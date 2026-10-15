import Foundation

/// Presentation-only summary of the read-only Rust JobStore inventory. It does
/// not authorize, submit, retry, or reconcile Jobs.
enum JobPresentationStatus: Equatable {
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
        guard !states.isEmpty else { return .waiting }
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
