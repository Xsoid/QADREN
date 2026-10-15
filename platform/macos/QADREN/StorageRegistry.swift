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
@_silgen_name("qadren_scenario_schedules_list")
private func scenarioSchedulesList(
    _ path: UnsafePointer<CChar>?, _ scenarioID: UnsafePointer<CChar>?
) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_scenario_schedules_due")
private func scenarioSchedulesDue(
    _ path: UnsafePointer<CChar>?, _ after: UnsafePointer<CChar>?, _ through: UnsafePointer<CChar>?
) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_scenario_automation_get")
private func scenarioAutomationGet(
    _ path: UnsafePointer<CChar>?, _ scenarioID: UnsafePointer<CChar>?
) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_scenario_automation_set_paused")
private func scenarioAutomationSetPaused(
    _ path: UnsafePointer<CChar>?, _ scenarioID: UnsafePointer<CChar>?, _ paused: UInt8
) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_scenario_schedules_create")
private func scenarioSchedulesCreate(
    _ path: UnsafePointer<CChar>?, _ scenarioID: UnsafePointer<CChar>?, _ timezone: UnsafePointer<CChar>?,
    _ weekday: UInt8, _ hour: UInt8, _ minute: UInt8
) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_scenario_schedules_set_enabled")
private func scenarioSchedulesSetEnabled(
    _ path: UnsafePointer<CChar>?, _ scenarioID: UnsafePointer<CChar>?, _ scheduleID: UnsafePointer<CChar>?,
    _ enabled: UInt8
) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_scenario_schedules_remove")
private func scenarioSchedulesRemove(
    _ path: UnsafePointer<CChar>?, _ scenarioID: UnsafePointer<CChar>?, _ scheduleID: UnsafePointer<CChar>?
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
@_silgen_name("qadren_registry_remove_after_job_gate")
private func registryRemoveAfterJobGate(
    _ registryPath: UnsafePointer<CChar>?, _ jobsPath: UnsafePointer<CChar>?,
    _ id: UnsafePointer<CChar>?
) -> UnsafeMutablePointer<CChar>?
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
struct ScenarioScheduleEntry: Decodable, Identifiable {
    let id: String
    let scenario_id: String
    let timezone: String
    let weekday: UInt8
    let hour: UInt8
    let minute: UInt8
    let enabled: Bool
}
struct ScenarioScheduleOccurrence: Decodable, Identifiable, Equatable {
    let scenario_id: String
    let schedule_id: String
    let occurrence_key: String

    var id: String { scenario_id + ":" + schedule_id + ":" + occurrence_key }
}
struct ScenarioAutomationEntry: Decodable, Equatable {
    let locally_paused: Bool
    let pending_automatic_run: Bool
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

    static func databasePath() throws -> String { try path }

    private static func unwrap<T: Decodable>(
        _ pointer: UnsafeMutablePointer<CChar>?, as: T.Type, databasePath: String? = nil
    ) throws -> T {
        guard let pointer else { throw StorageError.operation("Registry unavailable") }
        defer { registryFree(pointer) }
        let message = String(cString: pointer)
        let response = try JSONDecoder().decode(RegistryEnvelope<T>.self, from: Data(message.utf8))
        guard response.ok, let result = response.data else {
            throw StorageError.operation(response.error ?? "Registry request failed")
        }
        let database = try databasePath ?? path
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

    static func listScenarioSchedules(scenarioID: String) throws -> [ScenarioScheduleEntry] {
        let database = try path
        return try database.withCString { db in
            try scenarioID.withCString { id in
                try unwrap(scenarioSchedulesList(db, id), as: [ScenarioScheduleEntry].self)
            }
        }
    }

    static func scenarioAutomation(
        scenarioID: String, databasePath: String? = nil
    ) throws -> ScenarioAutomationEntry {
        let database = try databasePath ?? path
        return try database.withCString { db in
            try scenarioID.withCString { id in
                try unwrap(scenarioAutomationGet(db, id), as: ScenarioAutomationEntry.self)
            }
        }
    }

    static func setScenarioLocallyPaused(
        scenarioID: String, paused: Bool, databasePath: String? = nil
    ) throws {
        let database = try databasePath ?? path
        let _: Bool = try database.withCString { db in
            try scenarioID.withCString { id in
                try unwrap(
                    scenarioAutomationSetPaused(db, id, paused ? 1 : 0), as: Bool.self
                )
            }
        }
    }

    /// Returns scheduler candidates only. The Core deliberately does not claim
    /// these slots or create Jobs until it has an atomic JobStore/outbox boundary.
    static func dueScenarioSchedules(
        after: Date, through: Date, databasePath: String? = nil
    ) throws -> [ScenarioScheduleOccurrence] {
        let database = try databasePath ?? path
        let formatter = ISO8601DateFormatter()
        let afterValue = formatter.string(from: after)
        let throughValue = formatter.string(from: through)
        return try database.withCString { db in
            try afterValue.withCString { lowerBound in
                try throughValue.withCString { upperBound in
                    try unwrap(
                        scenarioSchedulesDue(db, lowerBound, upperBound),
                        as: [ScenarioScheduleOccurrence].self
                    )
                }
            }
        }
    }

    static func createScenarioSchedule(
        scenarioID: String, timezone: String, weekday: UInt8, hour: UInt8, minute: UInt8
    ) throws {
        let database = try path
        let _: NewConnection = try database.withCString { db in
            try scenarioID.withCString { id in
                try timezone.withCString { timezone in
                    try unwrap(
                        scenarioSchedulesCreate(db, id, timezone, weekday, hour, minute),
                        as: NewConnection.self
                    )
                }
            }
        }
    }

    static func setScenarioScheduleEnabled(
        scenarioID: String, scheduleID: String, enabled: Bool
    ) throws {
        let database = try path
        let _: Bool = try database.withCString { db in
            try scenarioID.withCString { scenario in
                try scheduleID.withCString { schedule in
                    try unwrap(
                        scenarioSchedulesSetEnabled(db, scenario, schedule, enabled ? 1 : 0),
                        as: Bool.self
                    )
                }
            }
        }
    }

    static func removeScenarioSchedule(scenarioID: String, scheduleID: String) throws {
        let database = try path
        let _: Bool = try database.withCString { db in
            try scenarioID.withCString { scenario in
                try scheduleID.withCString { schedule in
                    try unwrap(scenarioSchedulesRemove(db, scenario, schedule), as: Bool.self)
                }
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

    static func addLocalComputer(name: String = "Этот Mac", databasePath: String? = nil) throws -> String {
        let database = try databasePath ?? path
        let result: NewConnection = try database.withCString { db in
            try "local_filesystem".withCString { adapter in
                try name.withCString { label in
                    try unwrap(registryAdd(db, adapter, label), as: NewConnection.self)
                }
            }
        }
        return result.id
    }

    /// Final local record deletion for the recoverable removal workflow. Rust
    /// rechecks that the record is disabled and has no unresolved Jobs.
    static func removeAfterJobGate(
        id: String, jobsDatabasePath: String, databasePath: String? = nil
    ) throws {
        let database = try databasePath ?? path
        let _: Bool = try database.withCString { db in
            try jobsDatabasePath.withCString { jobDatabase in
                try id.withCString { value in
                    try unwrap(
                        registryRemoveAfterJobGate(db, jobDatabase, value),
                        as: Bool.self,
                        databasePath: database
                    )
                }
            }
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
