import AppKit
import Foundation
import ServiceManagement

/// Registers the bundled local Worker as a LaunchAgent through SMAppService.
/// The service owns schedule polling and JobStore dispatch, so it keeps
/// running after the user quits the UI. This controller does not report the
/// Worker as healthy; only the IPC status request proves that it responds.
enum LocalWorkerController {
    private static let service = SMAppService.agent(plistName: "dev.qadren.worker.plist")
    private static var registrationFailed = false

    /// A test instance with QADREN_DATA_ROOT must not register the production
    /// service unless the test asks for it with QADREN_REGISTER_AGENT=1.
    private static var registrationAllowed: Bool {
        let environment = ProcessInfo.processInfo.environment
        guard let root = environment[QADRENDataLocation.overrideVariable], !root.isEmpty else { return true }
        return environment["QADREN_REGISTER_AGENT"] == "1"
    }

    static func registerIfNeeded() {
        // Test cleanup hook: unregister the service of this bundle and quit.
        if UserDefaults.standard.bool(forKey: "QADRENUnregisterWorkerAgent") {
            try? service.unregister()
            exit(0)
        }
        // An explicit user stop is durable: do not register again silently.
        guard registrationAllowed, !UserDefaults.standard.bool(forKey: stoppedByUserKey) else { return }
        // Before the first registration, some macOS versions report .notFound
        // instead of .notRegistered. Both mean: not registered yet.
        guard service.status == .notRegistered || service.status == .notFound else {
            registrationFailed = false
            return
        }
        register()
    }

    private static let stoppedByUserKey = "qadren.worker.stopped-by-user"

    @discardableResult
    static func register() -> Bool {
        do {
            try service.register()
            registrationFailed = false
            UserDefaults.standard.set(false, forKey: stoppedByUserKey)
            return true
        } catch {
            // Do not expose a system error verbatim in the UI. It can include
            // installation-specific paths. The status must still distinguish
            // failed registration from a registration that was not attempted.
            registrationFailed = true
            return false
        }
    }

    /// Explicit stop by the user. launchd stops the Worker; queued and
    /// uncertain Jobs stay in the JobStore for the next start.
    static func stop() {
        try? service.unregister()
        UserDefaults.standard.set(true, forKey: stoppedByUserKey)
    }

    static var isRegistered: Bool { service.status == .enabled }

    static var statusText: String {
        switch service.status {
        case .enabled: return "фоновая служба зарегистрирована"
        case .requiresApproval: return "фоновую службу нужно разрешить в Системных настройках → Объекты входа"
        case .notRegistered:
            if UserDefaults.standard.bool(forKey: stoppedByUserKey) { return "фоновая служба остановлена пользователем" }
            return registrationFailed ? "фоновую службу не удалось зарегистрировать" : "фоновая служба не зарегистрирована"
        case .notFound: return "фоновая служба не найдена в приложении"
        @unknown default: return "состояние фоновой службы неизвестно"
        }
    }

    /// Confirm before an explicit stop. Quitting the UI is not a stop.
    @MainActor
    static func confirmAndStop() {
        let alert = NSAlert()
        alert.messageText = "Остановить фоновую службу QADREN?"
        alert.informativeText = "Расписания не будут выполняться, пока вы снова не запустите службу. Текущая операция с файлом может завершиться с результатом «неизвестно»; QADREN проверит его при следующем запуске и не повторит изменение вслепую."
        alert.addButton(withTitle: "Остановить")
        alert.addButton(withTitle: "Отмена")
        alert.alertStyle = .warning
        if alert.runModal() == .alertFirstButtonReturn { stop() }
    }
}
