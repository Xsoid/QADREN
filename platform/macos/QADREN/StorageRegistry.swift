import Foundation
import Security

@_silgen_name("qadren_registry_list")
private func registryList(_ path: UnsafePointer<CChar>?) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_scenarios_list")
private func scenarioList(_ path: UnsafePointer<CChar>?) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_scenarios_create")
private func scenarioCreate(
    _ path: UnsafePointer<CChar>?, _ name: UnsafePointer<CChar>?,
    _ leftID: UnsafePointer<CChar>?, _ leftRoot: UnsafePointer<CChar>?,
    _ rightID: UnsafePointer<CChar>?, _ rightRoot: UnsafePointer<CChar>?,
    _ direction: UnsafePointer<CChar>?
) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_scenarios_set_enabled")
private func scenarioSetEnabled(
    _ path: UnsafePointer<CChar>?, _ id: UnsafePointer<CChar>?, _ enabled: UInt8
) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_scenarios_update")
private func scenarioUpdate(
    _ path: UnsafePointer<CChar>?, _ id: UnsafePointer<CChar>?, _ name: UnsafePointer<CChar>?,
    _ leftID: UnsafePointer<CChar>?, _ leftRoot: UnsafePointer<CChar>?,
    _ rightID: UnsafePointer<CChar>?, _ rightRoot: UnsafePointer<CChar>?,
    _ direction: UnsafePointer<CChar>?
) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_registry_add")
private func registryAdd(_ path: UnsafePointer<CChar>?, _ adapter: UnsafePointer<CChar>?, _ name: UnsafePointer<CChar>?) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_registry_rename")
private func registryRename(_ path: UnsafePointer<CChar>?, _ id: UnsafePointer<CChar>?, _ name: UnsafePointer<CChar>?) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_registry_set_enabled")
private func registrySetEnabled(_ path: UnsafePointer<CChar>?, _ id: UnsafePointer<CChar>?, _ enabled: UInt8) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_registry_finalize_disconnect")
private func registryFinalizeDisconnect(_ path: UnsafePointer<CChar>?, _ id: UnsafePointer<CChar>?) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_registry_remove")
private func registryRemove(_ path: UnsafePointer<CChar>?, _ id: UnsafePointer<CChar>?) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_registry_set_credential")
private func registrySetCredential(_ path: UnsafePointer<CChar>?, _ id: UnsafePointer<CChar>?, _ ref: UnsafePointer<CChar>?) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_registry_set_status")
private func registrySetStatus(_ path: UnsafePointer<CChar>?, _ id: UnsafePointer<CChar>?, _ status: UnsafePointer<CChar>?, _ timestamp: Int64) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_registry_set_provider_account")
private func registrySetAccount(_ path: UnsafePointer<CChar>?, _ id: UnsafePointer<CChar>?, _ account: UnsafePointer<CChar>?) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_registry_free_string")
private func registryFree(_ ptr: UnsafeMutablePointer<CChar>?)

struct StorageEntry: Decodable, Identifiable {
    let id: String
    let adapter: String
    let name: String
    let enabled: Bool
    let status: String
    let credential_ref: String?
    let provider_account_id: String?
}
struct ScenarioEntry: Decodable, Identifiable {
    let id: String
    let name: String
    let left_connection_id: String
    let left_root: String
    let right_connection_id: String
    let right_root: String
    let direction: String
    let enabled: Bool
    let revision: UInt64
}
private struct NewConnection: Decodable {
    let id: String
}
private struct RegistryEnvelope<T: Decodable>: Decodable {
    let ok: Bool
    let data: T?
    let error: String?
}
enum StorageError: Error, LocalizedError {
    case operation(String)
    var errorDescription: String? {
        if case let .operation(message) = self { return message }
        return nil
    }
}
enum StorageRegistryBridge {
    private static var path: String {
        get throws {
            let base = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
                .appendingPathComponent("QADREN", isDirectory: true)
            try FileManager.default.createDirectory(
                at: base, withIntermediateDirectories: true,
                attributes: [.posixPermissions: NSNumber(value: 0o700)]
            )
            return base.appendingPathComponent("connections.sqlite3").path
        }
    }

    private static func unwrap<T: Decodable>(
        _ pointer: UnsafeMutablePointer<CChar>?, as: T.Type
    ) throws -> T {
        guard let pointer else { throw StorageError.operation("Registry unavailable") }
        defer { registryFree(pointer) }
        let message = String(cString: pointer)
        let response = try JSONDecoder().decode(RegistryEnvelope<T>.self, from: Data(message.utf8))
        guard response.ok, let result = response.data else {
            throw StorageError.operation(response.error ?? "Registry request failed")
        }
        let database = try path
        try? FileManager.default.setAttributes(
            [.posixPermissions: NSNumber(value: 0o600)], ofItemAtPath: database
        )
        return result
    }

    static func list() throws -> [StorageEntry] {
        let database = try path
        return try database.withCString { try unwrap(registryList($0), as: [StorageEntry].self) }
    }

    static func listScenarios() throws -> [ScenarioEntry] {
        let database = try path
        return try database.withCString { try unwrap(scenarioList($0), as: [ScenarioEntry].self) }
    }

    static func createScenario(
        name: String,
        leftID: String,
        leftRoot: String,
        rightID: String,
        rightRoot: String,
        direction: String
    ) throws -> String {
        let database = try path
        let result: NewConnection = try database.withCString { db in
            try name.withCString { name in
                try leftID.withCString { leftID in
                    try leftRoot.withCString { leftRoot in
                        try rightID.withCString { rightID in
                            try rightRoot.withCString { rightRoot in
                                try direction.withCString { direction in
                                    try unwrap(
                                        scenarioCreate(db, name, leftID, leftRoot, rightID, rightRoot, direction),
                                        as: NewConnection.self
                                    )
                                }
                            }
                        }
                    }
                }
            }
        }
        return result.id
    }

    static func setScenarioEnabled(id: String, enabled: Bool) throws {
        let database = try path
        let _: Bool = try database.withCString { db in
            try id.withCString { value in
                try unwrap(scenarioSetEnabled(db, value, enabled ? 1 : 0), as: Bool.self)
            }
        }
    }

    static func updateScenario(
        id: String,
        name: String,
        leftID: String,
        leftRoot: String,
        rightID: String,
        rightRoot: String,
        direction: String
    ) throws {
        let database = try path
        let _: Bool = try database.withCString { db in
            try id.withCString { id in
                try name.withCString { name in
                    try leftID.withCString { leftID in
                        try leftRoot.withCString { leftRoot in
                            try rightID.withCString { rightID in
                                try rightRoot.withCString { rightRoot in
                                    try direction.withCString { direction in
                                        try unwrap(
                                            scenarioUpdate(
                                                db, id, name, leftID, leftRoot, rightID, rightRoot, direction
                                            ),
                                            as: Bool.self
                                        )
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    static func addYandex(name: String) throws -> String {
        let database = try path
        let result: NewConnection = try database.withCString { db in
            try "yandex_disk".withCString { adapter in
                try name.withCString { label in
                    try unwrap(registryAdd(db, adapter, label), as: NewConnection.self)
                }
            }
        }
        return result.id
    }

    static func addLocalComputer(name: String = "Этот Mac") throws -> String {
        let database = try path
        let result: NewConnection = try database.withCString { db in
            try "local_filesystem".withCString { adapter in
                try name.withCString { label in
                    try unwrap(registryAdd(db, adapter, label), as: NewConnection.self)
                }
            }
        }
        return result.id
    }

    static func remove(id: String) throws {
        let database = try path
        let _: Bool = try database.withCString { db in
            try id.withCString { value in try unwrap(registryRemove(db, value), as: Bool.self) }
        }
    }

    static func rename(id: String, name: String) throws {
        let database = try path
        let _: Bool = try database.withCString { db in
            try id.withCString { value in
                try name.withCString { newName in
                    try unwrap(registryRename(db, value, newName), as: Bool.self)
                }
            }
        }
    }

    static func setEnabled(id: String, enabled: Bool) throws {
        let database = try path
        let _: Bool = try database.withCString { db in
            try id.withCString { value in
                try unwrap(registrySetEnabled(db, value, enabled ? 1 : 0), as: Bool.self)
            }
        }
    }

    /// Final local Registry update after the matching Keychain item was
    /// removed. This never contacts a provider or removes remote objects.
    static func finalizeDisconnect(id: String) throws {
        let database = try path
        let _: Bool = try database.withCString { db in
            try id.withCString { value in
                try unwrap(registryFinalizeDisconnect(db, value), as: Bool.self)
            }
        }
    }

    static func setCredential(id: String) throws {
        let database = try path
        let _: Bool = try database.withCString { db in
            try id.withCString { value in
                try ("keychain:" + id).withCString { reference in
                    try unwrap(registrySetCredential(db, value, reference), as: Bool.self)
                }
            }
        }
    }

    static func clearCredential(id: String) throws {
        let database = try path
        let _: Bool = try database.withCString { db in
            try id.withCString { value in
                try unwrap(registrySetCredential(db, value, nil), as: Bool.self)
            }
        }
    }

    static func setStatus(id: String, status: String) throws {
        let database = try path
        let timestamp = Int64(Date().timeIntervalSince1970)
        let _: Bool = try database.withCString { db in
            try id.withCString { value in
                try status.withCString { state in
                    try unwrap(registrySetStatus(db, value, state, timestamp), as: Bool.self)
                }
            }
        }
    }

    static func setProviderAccount(id: String, providerID: String) throws {
        let database = try path
        let _: Bool = try database.withCString { db in
            try id.withCString { value in
                try providerID.withCString { provider in
                    try unwrap(registrySetAccount(db, value, provider), as: Bool.self)
                }
            }
        }
    }
}

/// Keychain records are isolated by StorageConnection ID; no token reaches SQLite.
enum YandexKeychain {
    private static let service = "dev.qadren.yandex.oauth"

    private static func query(_ connectionID: String) -> [String: Any] {
        [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: connectionID
        ]
    }

    static func read(_ connectionID: String) throws -> String? {
        var q = query(connectionID)
        q[kSecReturnData as String] = true
        q[kSecMatchLimit as String] = kSecMatchLimitOne
        var item: CFTypeRef?
        let status = SecItemCopyMatching(q as CFDictionary, &item)
        if status == errSecItemNotFound { return nil }
        guard status == errSecSuccess, let data = item as? Data,
              let token = String(data: data, encoding: .utf8) else {
            throw StorageError.operation("Keychain read failed")
        }
        return token
    }

    static func store(_ token: String, for connectionID: String) throws {
        guard !token.isEmpty && !connectionID.isEmpty else {
            throw StorageError.operation("Empty credentials")
        }
        let key = query(connectionID)
        var item = key
        item[kSecValueData as String] = Data(token.utf8)
        item[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
        var result = SecItemAdd(item as CFDictionary, nil)
        if result == errSecDuplicateItem {
            result = SecItemUpdate(key as CFDictionary,
                                   [kSecValueData as String: Data(token.utf8)] as CFDictionary)
        }
        guard result == errSecSuccess else {
            throw StorageError.operation("Keychain write failed")
        }
    }

    static func delete(_ connectionID: String) throws {
        let result = SecItemDelete(query(connectionID) as CFDictionary)
        guard result == errSecSuccess || result == errSecItemNotFound else {
            throw StorageError.operation("Keychain removal failed")
        }
    }
}
