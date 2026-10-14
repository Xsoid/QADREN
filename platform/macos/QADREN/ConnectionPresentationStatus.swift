import Foundation

/// Presentation-only state derived from the local Registry snapshot. It never
/// initiates a connection or changes its permissions.
enum ConnectionPresentationStatus: Equatable {
    case ready
    case needsAction
    case failed
    case offline
    case disconnected
    case paused
    case waiting
    case unknown

    init(enabled: Bool, registryStatus: String) {
        guard enabled else {
            self = .paused
            return
        }

        switch registryStatus {
        case "connected": self = .ready
        case "requires_auth": self = .needsAction
        case "error": self = .failed
        case "offline": self = .offline
        case "disconnected": self = .disconnected
        default: self = .unknown
        }
    }

    static func aggregate(_ entries: [ConnectionPresentationStatus], registryFailed: Bool) -> ConnectionPresentationStatus {
        if registryFailed { return .failed }
        guard !entries.isEmpty else { return .waiting }

        let enabled = entries.filter { $0 != .paused }
        guard !enabled.isEmpty else { return .paused }
        if enabled.contains(.failed) { return .failed }
        if enabled.contains(.needsAction) { return .needsAction }
        if enabled.allSatisfy({ $0 == .ready }) { return .ready }
        if enabled.allSatisfy({ $0 == .offline || $0 == .disconnected }) { return .offline }
        return .unknown
    }

    var title: String {
        switch self {
        case .ready: "подключено"
        case .needsAction: "нужна авторизация"
        case .failed: "ошибка"
        case .offline: "нет связи"
        case .disconnected: "отключено"
        case .paused: "выключено"
        case .waiting: "ожидание"
        case .unknown: "статус не подтверждён"
        }
    }
}
