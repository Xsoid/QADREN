import Foundation
import SwiftUI

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
        } catch {
            self.error = error.localizedDescription
        }
    }

    func rename(_ id: String, name: String) {
        do {
            try StorageRegistryBridge.rename(id: id, name: name)
            refresh()
        } catch {
            self.error = error.localizedDescription
        }
    }

    func setEnabled(_ id: String, enabled: Bool) {
        do {
            try StorageRegistryBridge.setEnabled(id: id, enabled: enabled)
            refresh()
        } catch {
            self.error = error.localizedDescription
        }
    }

    func remove(_ id: String) {
        do {
            // Verify the record before touching credentials. The connection must
            // remain discoverable until Keychain cleanup has succeeded.
            guard let entry = try StorageRegistryBridge.list().first(where: { $0.id == id }) else {
                throw StorageError.operation("Connection not found; credentials were not touched")
            }
            guard entry.adapter == "yandex_disk" else {
                throw StorageError.operation("Unsupported credential cleanup for this adapter")
            }

            // Fail closed: prevent new work before removing credentials. If
            // Keychain deletion fails, the disabled record remains for retry.
            try StorageRegistryBridge.setEnabled(id: id, enabled: false)
            try YandexKeychain.delete(id) // Not-found is an idempotent success.
            // If SQLite removal fails, the disabled record remains visible;
            // retrying deletion is safe even though the token is already gone.
            try StorageRegistryBridge.remove(id: id)
            refresh()
        } catch {
            // Refresh the listing without discarding the failure message.
            entries = (try? StorageRegistryBridge.list()) ?? entries
            self.error = "Connection removal incomplete; retry Remove: " + error.localizedDescription
        }
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
        } catch {
            self.error = "Legacy migration incomplete; old token retained: " + error.localizedDescription
            // Never delete legacy token on failure. Safe to retry after restart.
            hasLegacyYandexToken = true
        }
    }
}
