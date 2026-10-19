import Foundation
import ServiceManagement

/// Registers the bundled local Worker. The service owns schedule polling and
/// JobStore dispatch. This controller does not report it as healthy until a
/// later IPC status request can prove that it responds.
enum LocalWorkerController {
    private static let service = SMAppService.agent(plistName: "dev.qadren.worker.plist")

    static func registerIfNeeded() {
        guard service.status == .notRegistered else { return }
        try? service.register()
    }

    static var statusText: String {
        switch service.status {
        case .enabled: return "Worker service registered"
        case .requiresApproval: return "Worker service requires approval"
        case .notRegistered: return "Worker service is not registered"
        case .notFound: return "Worker service bundle is unavailable"
        @unknown default: return "Worker service status unavailable"
        }
    }
}
