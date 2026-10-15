import Foundation

@_silgen_name("qadren_registry_add")
private func registryAdd(
    _ path: UnsafePointer<CChar>?, _ adapter: UnsafePointer<CChar>?, _ name: UnsafePointer<CChar>?
) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_registry_set_enabled")
private func registrySetEnabled(
    _ path: UnsafePointer<CChar>?, _ id: UnsafePointer<CChar>?, _ enabled: UInt8
) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_registry_list")
private func registryList(_ path: UnsafePointer<CChar>?) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_registry_free_string")
private func registryFree(_ pointer: UnsafeMutablePointer<CChar>?)
@_silgen_name("qadren_jobs_list")
private func jobsList(_ path: UnsafePointer<CChar>?) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_jobs_free_string")
private func jobsFree(_ pointer: UnsafeMutablePointer<CChar>?)

private struct Envelope<T: Decodable>: Decodable {
    let ok: Bool
    let data: T?
    let error: String?
}

private struct NewConnection: Decodable {
    let id: String
}

private struct JobListResponse: Decodable {
    let ok: Bool
}

private func decodeRegistry<T: Decodable>(_ pointer: UnsafeMutablePointer<CChar>?, as: T.Type) throws -> T {
    guard let pointer else { throw StorageError.operation("Registry unavailable") }
    defer { registryFree(pointer) }
    let response = try JSONDecoder().decode(Envelope<T>.self, from: Data(String(cString: pointer).utf8))
    guard response.ok, let data = response.data else {
        throw StorageError.operation(response.error ?? "Registry request failed")
    }
    return data
}

private func check(_ condition: @autoclosure () -> Bool, _ message: String) {
    guard condition() else { fatalError("FAIL: " + message) }
}

@main
struct StorageRegistryRemovalTests {
    static func main() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("qadren-storage-removal-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }

        let registryPath = root.appendingPathComponent("connections.sqlite3").path
        let jobsPath = root.appendingPathComponent("jobs.sqlite3").path
        let connectionID: String = try registryPath.withCString { database in
            try "local_filesystem".withCString { adapter in
                try "Test Local".withCString { name in
                    let value: NewConnection = try decodeRegistry(
                        registryAdd(database, adapter, name), as: NewConnection.self
                    )
                    return value.id
                }
            }
        }

        let jobPointer = jobsPath.withCString { jobsList($0) }
        guard let jobPointer else { throw StorageError.operation("JobStore unavailable") }
        defer { jobsFree(jobPointer) }
        let jobStore = try JSONDecoder().decode(
            JobListResponse.self, from: Data(String(cString: jobPointer).utf8)
        )
        check(jobStore.ok, "clean temporary JobStore opens")

        do {
            try StorageRegistryBridge.removeAfterJobGate(
                id: connectionID, jobsDatabasePath: jobsPath, databasePath: registryPath
            )
            fatalError("enabled connection must not be removed")
        } catch StorageError.operation {
        }
        let before: [StorageEntry] = try registryPath.withCString {
            try decodeRegistry(registryList($0), as: [StorageEntry].self)
        }
        check(before.map(\.id) == [connectionID], "enabled record remains after refusal")

        let _: Bool = try registryPath.withCString { database in
            try connectionID.withCString { id in
                try decodeRegistry(registrySetEnabled(database, id, 0), as: Bool.self)
            }
        }
        try StorageRegistryBridge.removeAfterJobGate(
            id: connectionID, jobsDatabasePath: jobsPath, databasePath: registryPath
        )
        let after: [StorageEntry] = try registryPath.withCString {
            try decodeRegistry(registryList($0), as: [StorageEntry].self)
        }
        check(after.isEmpty, "disabled clean connection is removed through the Swift bridge")
        print("StorageRegistry native removal bridge tests passed")
    }
}
