import Foundation
import ServiceManagement
import UserNotifications

/// "Запускать QADREN при входе в систему" (D-025, issue #33).
///
/// The Worker cannot read the Keychain items of QADREN: the app has no
/// Developer ID signature (D-024, stage B is deferred). The app gives the
/// Worker the credentials, and the Worker keeps them in memory only. After a
/// Mac restart that memory is empty. This setting registers a LaunchAgent
/// that starts the app executable at login with `--login-handoff`. That
/// process opens no window, gives the credentials to the Worker, and quits.
enum LoginHandoff {
    static let argument = "--login-handoff"
    private static let service = SMAppService.agent(plistName: "dev.qadren.login.plist")

    enum State {
        case enabled
        /// Registered, but the user must allow it in System Settings › Login Items.
        case requiresApproval
        case off
    }

    static var state: State {
        switch service.status {
        case .enabled: .enabled
        case .requiresApproval: .requiresApproval
        default: .off
        }
    }

    static func setEnabled(_ enabled: Bool) throws {
        if enabled {
            try service.register()
        } else {
            try service.unregister()
        }
    }

    static func openLoginItemsSettings() {
        SMAppService.openSystemSettingsLoginItems()
    }

    /// The login start: give the credentials to the Worker, then quit before
    /// any scene exists, so there is no window and no Dock icon.
    static func runIfRequested() {
        guard CommandLine.arguments.contains(argument) else { return }
        // The Worker LaunchAgent also starts at login. Wait for its socket.
        let deadline = Date().addingTimeInterval(120)
        while (try? WorkerIPC.probeStatus(timeoutSeconds: 2)) == nil, Date() < deadline {
            Thread.sleep(forTimeInterval: 2)
        }
        if !WorkerCredentials.deliverAll() {
            notifySignInRequired()
        }
        exit(0)
    }

    /// A quiet notification: provisional authorization needs no prompt.
    private static func notifySignInRequired() {
        let done = DispatchSemaphore(value: 0)
        let center = UNUserNotificationCenter.current()
        center.requestAuthorization(options: [.provisional, .alert]) { granted, _ in
            guard granted else {
                done.signal()
                return
            }
            let content = UNMutableNotificationContent()
            content.title = "Нужен вход"
            content.body = "QADREN не передал фоновой службе данные для входа. Сценарии с Яндекс Диском или сетевым диском ждут, пока вы откроете QADREN."
            let request = UNNotificationRequest(identifier: "qadren.login-handoff", content: content, trigger: nil)
            center.add(request) { _ in done.signal() }
        }
        _ = done.wait(timeout: .now() + 10)
    }
}

/// Gives the Worker the credentials of every enabled Yandex Disk and SMB
/// connection from the Keychain. The Worker keeps them in memory only.
enum WorkerCredentials {
    /// Returns false when a Keychain item is missing or locked, or when the
    /// Worker did not take a credential.
    static func deliverAll() -> Bool {
        var complete = true
        for entry in (try? StorageRegistryBridge.list()) ?? [] where entry.enabled {
            switch entry.adapter {
            case "yandex_disk":
                guard let token = try? YandexKeychain.read(entry.id) else {
                    complete = false
                    continue
                }
                if (try? WorkerIPC.provideAccessToken(connectionID: entry.id, token: token)) == nil {
                    complete = false
                }
            case "smb_share" where entry.credential_ref != nil:
                guard let credentials = try? SMBKeychain.read(entry.id) else {
                    complete = false
                    continue
                }
                if (try? WorkerIPC.provideSMBCredentials(
                    connectionID: entry.id, login: credentials.login, password: credentials.password
                )) == nil {
                    complete = false
                }
            default:
                continue
            }
        }
        return complete
    }
}
