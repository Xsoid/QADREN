import Foundation
import SwiftUI

extension Notification.Name {
    static let qadrenStorageConnectionsDidChange = Notification.Name("qadren.storage-connections.did-change")
}

@MainActor
final class StorageConnections: ObservableObject {
    @Published private(set) var entries: [StorageEntry] = []
    @Published private(set) var hasLegacyYandexToken = false
    @Published private(set) var error: String?

    init() {
        refresh()
    }

    func refresh() {
        do {
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

    @discardableResult
    func addLocalComputer() -> String? {
        do {
            let connectionID = try StorageRegistryBridge.addLocalComputer()
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
            let disable = { try StorageRegistryBridge.setEnabled(id: id, enabled: false) }
            let ensureNoUnresolvedAttempts = {
                try JobInventory.requireNoUnresolvedAttempts(connectionID: id)
            }
            let deleteRecord = {
                try StorageRegistryBridge.removeAfterJobGate(
                    id: id, jobsDatabasePath: JobInventory.paths().database
                )
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
