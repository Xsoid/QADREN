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
        // A new build has a new bundle version. Register it again, so that
        // launchd starts the Worker of this build. With an ad-hoc signature
        // SMAppService also binds the Worker to the cdhash of the registered
        // build and refuses another one (OS_REASON_CODESIGNING); with a Team
        // ID signature the binding is the Team ID and the first attempt works.
        if service.status == .enabled, registeredBuild != currentBuild {
            // D-024: never stop a Worker while it writes a file. The running
            // Worker continues; the next app start registers the new build.
            if let active = try? WorkerIPC.activeItems(), active > 0 { return }
            registerVerified(build: currentBuild, attempt: 0, unregisterFirst: true)
            return
        }
        // Before the first registration, some macOS versions report .notFound
        // instead of .notRegistered. Both mean: not registered yet.
        guard service.status == .notRegistered || service.status == .notFound else {
            registrationFailed = false
            return
        }
        registerVerified(build: currentBuild, attempt: 0, unregisterFirst: false)
    }

    /// Pauses before each repeat of the registration.
    private static let retryDelays: [Double] = [10, 20, 40, 60, 90]

    /// Register, and then prove that the Worker answers. An ad-hoc signed
    /// build gets a new Worker cdhash, and macOS applies it to a LaunchAgent
    /// registration only some time after the build: on 2026-10-25 a
    /// registration within the first ~30 s kept the old cdhash, and the same
    /// steps about a minute later worked. So repeat unregister and register
    /// with growing pauses. The build counts as registered only after the
    /// Worker answers; otherwise the next launch tries again.
    private static func registerVerified(build: String, attempt: Int, unregisterFirst: Bool) {
        let registerAndProbe = {
            // The unregistration is applied a moment after the call returns.
            DispatchQueue.main.asyncAfter(deadline: .now() + (unregisterFirst ? 3 : 0)) {
                guard register() else { return }
                DispatchQueue.global(qos: .utility).asyncAfter(deadline: .now() + 5) {
                    let answers = (try? WorkerIPC.probeStatus(timeoutSeconds: 2)) != nil
                    DispatchQueue.main.async {
                        if answers {
                            registeredBuild = build
                        } else if attempt < retryDelays.count {
                            DispatchQueue.main.asyncAfter(deadline: .now() + retryDelays[attempt]) {
                                registerVerified(build: build, attempt: attempt + 1, unregisterFirst: true)
                            }
                        }
                    }
                }
            }
        }
        if unregisterFirst {
            service.unregister { _ in registerAndProbe() }
        } else {
            registerAndProbe()
        }
    }

    private static let registeredBuildKey = "qadren.worker.registered-build"
    private static var currentBuild: String {
        Bundle.main.object(forInfoDictionaryKey: "CFBundleVersion") as? String ?? ""
    }
    private static var registeredBuild: String {
        get { UserDefaults.standard.string(forKey: registeredBuildKey) ?? "" }
        set { UserDefaults.standard.set(newValue, forKey: registeredBuildKey) }
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
