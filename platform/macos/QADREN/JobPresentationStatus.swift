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

    static func aggregate(_ states: [String], inventoryFailed: Bool) -> JobPresentationStatus {
        if inventoryFailed { return .failed }
        guard !states.isEmpty else { return .waiting }
        if states.contains("failed") { return .failed }
        if states.contains("unknown") { return .unknown }
        if states.contains("waiting_for_decision") { return .needsAction }
        if states.contains("running") { return .active }
        if states.contains("paused") { return .paused }
        if states.contains("queued") { return .waiting }
        return .ready
    }
}
