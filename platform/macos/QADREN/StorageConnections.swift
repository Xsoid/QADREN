import Foundation
import SwiftUI

extension Notification.Name {
    static let qadrenStorageConnectionsDidChange = Notification.Name("qadren.storage-connections.did-change")
    /// A widget asks the Settings window to show the Connections section.
    static let qadrenShowConnectionSettings = Notification.Name("qadren.settings.show-connections")
}

@MainActor
final class StorageConnections: ObservableObject {
    @Published private(set) var entries: [StorageEntry] = []
    @Published private(set) var hasLegacyYandexToken = false
    @Published private(set) var error: String?

    init() {
        refresh()
        // Network drives are mounted at start, so that plans and scheduled
        // Jobs find their files (D-022).
        Task { await SMBMounts.mountAll(self) }
    }

    func refresh() {
        do {
            // "This Mac" always exists (D-020). This creates it on first use
            // and is a no-op afterwards.
            _ = try StorageRegistryBridge.ensureLocalComputer()
            entries = try StorageRegistryBridge.list()
            hasLegacyYandexToken = (try YandexKeychain.read("default")) != nil
            error = nil
        } catch {
            self.error = error.localizedDescription
        }
    }

    func addYandex() {
        do {
            _ = try StorageRegistryBridge.addYandex(name: "Yandex Disk")
            refresh()
            publishChange()
        } catch {
            self.error = error.localizedDescription
        }
    }

    /// Saves one SMB share connection with its login and password (D-021).
    /// The order fails closed, as `ConnectionAuthorization` does: the record
    /// stays `requires_auth` until the Keychain item is written and read back.
    /// `outcome` is the result of the login check; nil means not checked.
    func addSMBShare(
        name: String, address: String, credentials: SMBCredentials, outcome: SMBCheckOutcome?
    ) throws {
        let id = try StorageRegistryBridge.addSMBShare(name: name, address: address)
        refresh()
        publishChange()
        try saveSMBCredentials(id: id, credentials: credentials, outcome: outcome)
        Task { await SMBMounts.mountAll(self) }
    }

    /// Changes the address, the login, and the password of an SMB share.
    /// A Scenario that uses this connection keeps its roots and confirmed
    /// files; another share under the same roots could look like mass
    /// deletion. So the address of a connection in use does not change.
    func updateSMBShare(
        id: String, address: String, credentials: SMBCredentials, outcome: SMBCheckOutcome?
    ) throws {
        let current = try StorageRegistryBridge.list().first { $0.id == id }
        let addressChanged = current?.endpoint_url != SMBAddress.normalized(address)
        if addressChanged {
            if let scenario = try StorageRegistryBridge.listScenarios()
                .first(where: { $0.left_connection_id == id || $0.right_connection_id == id }) {
                throw StorageError.operation(
                    "Адрес нельзя изменить: подключение использует сценарий «\(scenario.name)». Создайте новое подключение для другой сетевой папки."
                )
            }
            // The QADREN mount still shows the old share.
            SMBMounts.release(id)
            try StorageRegistryBridge.setSMBEndpoint(id: id, address: address)
        }
        try saveSMBCredentials(id: id, credentials: credentials, outcome: outcome)
        Task { await SMBMounts.mountAll(self) }
    }

    /// Records the result of a login check with the saved credentials.
    func recordSMBCheck(id: String, outcome: SMBCheckOutcome) {
        do {
            try StorageRegistryBridge.setStatus(id: id, status: outcome.registryStatus)
            refresh()
            publishChange()
        } catch {
            self.error = error.localizedDescription
        }
    }

    private func saveSMBCredentials(
        id: String, credentials: SMBCredentials, outcome: SMBCheckOutcome?
    ) throws {
        defer {
            refresh()
            publishChange()
        }
        try StorageRegistryBridge.setStatus(id: id, status: "requires_auth")
        try SMBKeychain.store(credentials, for: id)
        guard try SMBKeychain.read(id) == credentials else {
            throw StorageError.operation("Keychain copy could not be verified")
        }
        try StorageRegistryBridge.setCredential(id: id)
        try StorageRegistryBridge.setStatus(id: id, status: outcome?.registryStatus ?? "disconnected")
        // The Worker mounts the share for scheduled runs (issue #32).
        try? WorkerIPC.provideSMBCredentials(
            connectionID: id, login: credentials.login, password: credentials.password
        )
    }

    /// Saves one ISPmanager panel connection with its login and password
    /// (D-027), in the same fail-closed order as an SMB share.
    @discardableResult
    func addISPmanager(
        name: String, url: String, sshPort: UInt16, credentials: LoginCredentials,
        outcome: ISPmanagerCheckReport?
    ) throws -> String {
        let id = try StorageRegistryBridge.addISPmanager(name: name, url: url)
        refresh()
        publishChange()
        try StorageRegistryBridge.setISPmanagerSSHPort(id: id, port: sshPort)
        try saveISPmanagerCredentials(id: id, credentials: credentials, outcome: outcome)
        return id
    }

    /// No Scenario uses an ISPmanager connection yet, so its URL can change.
    /// Another URL or port clears the confirmed SSH host key in Core.
    func updateISPmanager(
        id: String, url: String, sshPort: UInt16, credentials: LoginCredentials,
        outcome: ISPmanagerCheckReport?
    ) throws {
        try StorageRegistryBridge.setISPmanagerEndpoint(id: id, url: url)
        try StorageRegistryBridge.setISPmanagerSSHPort(id: id, port: sshPort)
        try saveISPmanagerCredentials(id: id, credentials: credentials, outcome: outcome)
    }

    /// Stores the SSH host key fingerprint that the user confirmed (D-027
    /// section 7a).
    func confirmISPmanagerHostKey(id: String, fingerprint: String) throws {
        try StorageRegistryBridge.confirmISPmanagerHostKey(id: id, fingerprint: fingerprint)
        publishChange()
    }

    /// Records the status and keeps the report, so that it survives a restart.
    func recordISPmanagerCheck(id: String, outcome: ISPmanagerCheckReport) {
        do {
            try StorageRegistryBridge.recordISPmanagerCheck(id: id, report: outcome)
            try StorageRegistryBridge.setStatus(id: id, status: outcome.effectiveStatus)
            refresh()
            publishChange()
        } catch {
            self.error = error.localizedDescription
        }
    }

    private func saveISPmanagerCredentials(
        id: String, credentials: LoginCredentials, outcome: ISPmanagerCheckReport?
    ) throws {
        defer {
            refresh()
            publishChange()
        }
        try StorageRegistryBridge.setStatus(id: id, status: "requires_auth")
        try ISPmanagerKeychain.store(credentials, for: id)
        guard try ISPmanagerKeychain.read(id) == credentials else {
            throw StorageError.operation("Keychain copy could not be verified")
        }
        try StorageRegistryBridge.setCredential(id: id)
        try StorageRegistryBridge.setStatus(id: id, status: outcome?.effectiveStatus ?? "disconnected")
        if let outcome {
            try StorageRegistryBridge.recordISPmanagerCheck(id: id, report: outcome)
        }
    }

    @discardableResult
    func addLocalComputer() -> String? {
        do {
            let connectionID = try StorageRegistryBridge.ensureLocalComputer()
            refresh()
            publishChange()
            return connectionID
        } catch {
            self.error = error.localizedDescription
            return nil
        }
    }

    func rename(_ id: String, name: String) {
        do {
            try StorageRegistryBridge.rename(id: id, name: name)
            refresh()
            publishChange()
        } catch {
            self.error = error.localizedDescription
        }
    }

    @discardableResult
    func setEnabled(_ id: String, enabled: Bool) -> Bool {
        do {
            try StorageRegistryBridge.setEnabled(id: id, enabled: enabled)
            refresh()
            publishChange()
            return true
        } catch {
            self.error = error.localizedDescription
            return false
        }
    }

    func remove(_ id: String) {
        do {
            // Verify the record before touching credentials. The connection must
            // remain discoverable until Keychain cleanup has succeeded.
            guard let entry = try StorageRegistryBridge.list().first(where: { $0.id == id }) else {
                throw StorageError.operation("Connection not found; credentials were not touched")
            }
            // Check Scenario references before any change. A refused removal
            // must not leave the connection disabled as a side effect.
            if let scenarioName = try StorageRegistryBridge.listScenarios()
                .first(where: { $0.left_connection_id == id || $0.right_connection_id == id })?.name {
                self.error = "Нельзя удалить подключение: его использует сценарий «\(scenarioName)». Сначала измените или удалите сценарий. Подключение и файлы не изменены."
                return
            }
            let disable = { try StorageRegistryBridge.setEnabled(id: id, enabled: false) }
            let ensureNoUnresolvedAttempts = {
                try JobInventory.requireNoUnresolvedAttempts(connectionID: id)
            }
            let deleteRecord = {
                try StorageRegistryBridge.removeAfterJobGate(
                    id: id, jobsDatabasePath: JobInventory.paths().database
                )
            }
            // "This Mac" always exists (D-020). Only an extra record from
            // older data can be removed, never the last one.
            if entry.adapter == "local_filesystem",
               try StorageRegistryBridge.list().filter({ $0.adapter == "local_filesystem" }).count < 2 {
                throw StorageError.operation(QADRENUserMessage.text(for: "this_mac_required"))
            }
            switch entry.adapter {
            case "yandex_disk":
                try ConnectionRemoval.run(
                    adapter: entry.adapter,
                    disable: disable,
                    ensureNoUnresolvedAttempts: ensureNoUnresolvedAttempts,
                    deleteCredential: { try YandexKeychain.delete(id) },
                    deleteRecord: deleteRecord
                )
            case "smb_share":
                try ConnectionRemoval.run(
                    adapter: entry.adapter,
                    disable: disable,
                    ensureNoUnresolvedAttempts: ensureNoUnresolvedAttempts,
                    deleteCredential: { try SMBKeychain.delete(id) },
                    deleteRecord: deleteRecord
                )
                // Local cleanup only: unmount and remove the empty directory.
                SMBMounts.release(id)
            case "ispmanager":
                try ConnectionRemoval.run(
                    adapter: entry.adapter,
                    disable: disable,
                    ensureNoUnresolvedAttempts: ensureNoUnresolvedAttempts,
                    deleteCredential: { try ISPmanagerKeychain.delete(id) },
                    deleteRecord: deleteRecord
                )
            case "local_filesystem":
                try ConnectionRemoval.removeLocal(
                    disable: disable,
                    ensureNoUnresolvedAttempts: ensureNoUnresolvedAttempts,
                    deleteRecord: deleteRecord
                )
            default:
                throw StorageError.operation("Unsupported connection adapter")
            }
            refresh()
            publishChange()
        } catch {
            // Refresh the listing without discarding the failure message.
            entries = (try? StorageRegistryBridge.list()) ?? entries
            if error.localizedDescription == "connection_referenced_by_scenario" {
                let scenarioName = (try? StorageRegistryBridge.listScenarios())?
                    .first(where: { $0.left_connection_id == id || $0.right_connection_id == id })?
                    .name
                if let scenarioName {
                    self.error = "Нельзя удалить подключение: его использует сценарий «\(scenarioName)». Сначала измените или удалите сценарий. Файлы на этом Mac не изменены."
                } else {
                    self.error = "Нельзя удалить подключение, пока его использует сценарий. Сначала измените или удалите сценарий. Файлы на этом Mac не изменены."
                }
            } else {
                self.error = "Connection removal incomplete; retry Remove: " + error.localizedDescription
            }
        }
    }

    /// Another object changed a connection status in the Registry, for
    /// example a Yandex Disk sign-in. Read it again and tell the dashboard.
    func statusDidChange() {
        refresh()
        publishChange()
    }

    private func publishChange() {
        NotificationCenter.default.post(name: .qadrenStorageConnectionsDidChange, object: nil)
    }

    /// Copies the existing singleton credential into a new isolated connection.
    /// The legacy credential is deleted ONLY after the new item can be read back.
    /// Repeated invocation after interruption reuses the already copied item.
    func importLegacyYandex() {
        do {
            guard let legacyToken = try YandexKeychain.read("default") else {
                hasLegacyYandexToken = false
                return
            }
            let current = try StorageRegistryBridge.list()
            for entry in current where entry.adapter == "yandex_disk" {
                if (try YandexKeychain.read(entry.id)) == legacyToken {
                    try StorageRegistryBridge.setCredential(id: entry.id)
                    try YandexKeychain.delete("default")
                    refresh()
                    publishChange()
                    return
                }
            }
            // Never adopt an unrelated empty user-created connection by name.
            // Interrupted migrations that already wrote a token are found above.
            let id = try StorageRegistryBridge.addYandex(name: "Imported Yandex Disk")
            try YandexKeychain.store(legacyToken, for: id)
            guard try YandexKeychain.read(id) == legacyToken else {
                throw StorageError.operation("Legacy token copy could not be verified")
            }
            try StorageRegistryBridge.setCredential(id: id)
            try YandexKeychain.delete("default")
            refresh()
            publishChange()
        } catch {
            self.error = "Legacy migration incomplete; old token retained: " + error.localizedDescription
            // Never delete legacy token on failure. Safe to retry after restart.
            hasLegacyYandexToken = true
        }
    }
}
