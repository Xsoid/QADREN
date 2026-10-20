import Foundation
import ServiceManagement

/// Registers the bundled local Worker. The service owns schedule polling and
/// JobStore dispatch. This controller does not report it as healthy until a
/// later IPC status request can prove that it responds.
enum LocalWorkerController {
    private static let service = SMAppService.agent(plistName: "dev.qadren.worker.plist")
    private static var registrationFailed = false

    static func registerIfNeeded() {
        guard service.status == .notRegistered else {
            registrationFailed = false
            return
        }

        do {
            try service.register()
            registrationFailed = false
        } catch {
            // Do not expose a system error verbatim in the UI. It can include
            // installation-specific paths. The status must still distinguish
            // failed registration from a registration that was not attempted.
            registrationFailed = true
        }
    }

    static var statusText: String {
        switch service.status {
        case .enabled: return "Worker service registered"
        case .requiresApproval: return "Worker service requires approval"
        case .notRegistered:
            return registrationFailed
                ? "Worker service registration failed"
                : "Worker service is not registered"
        case .notFound: return "Worker service bundle is unavailable"
        @unknown default: return "Worker service status unavailable"
        }
    }
}
