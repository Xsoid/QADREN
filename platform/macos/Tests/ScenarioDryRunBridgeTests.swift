import Foundation
import Darwin

@_silgen_name("qadren_registry_add")
private func registryAddForTest(
    _ path: UnsafePointer<CChar>?, _ adapter: UnsafePointer<CChar>?, _ name: UnsafePointer<CChar>?
) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_scenarios_create")
private func scenarioCreateForTest(
    _ path: UnsafePointer<CChar>?, _ name: UnsafePointer<CChar>?,
    _ leftID: UnsafePointer<CChar>?, _ leftRoot: UnsafePointer<CChar>?,
    _ rightID: UnsafePointer<CChar>?, _ rightRoot: UnsafePointer<CChar>?,
    _ direction: UnsafePointer<CChar>?
) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_registry_free_string")
private func freeRegistryStringForTest(_ ptr: UnsafeMutablePointer<CChar>?)

private struct Response<T: Decodable>: Decodable {
    let ok: Bool
    let data: T?
    let error: String?
}
private struct CreatedID: Decodable { let id: String }

private func check(_ condition: @autoclosure () -> Bool, _ message: String) {
    guard condition() else { fatalError("FAIL: " + message) }
}

private func physicalPath(_ path: String) throws -> String {
    guard let resolved = realpath(path, nil) else {
        throw StorageError.operation("cannot resolve test root")
    }
    defer { free(resolved) }
    return String(cString: resolved)
}

private func decode<T: Decodable>(_ pointer: UnsafeMutablePointer<CChar>?, as: T.Type) throws -> T {
    guard let pointer else { throw StorageError.operation("missing Rust response") }
    defer { freeRegistryStringForTest(pointer) }
    let response = try JSONDecoder().decode(Response<T>.self, from: Data(String(cString: pointer).utf8))
    guard response.ok, let value = response.data else {
        throw StorageError.operation(response.error ?? "Rust request failed")
    }
    return value
}

private func addConnection(database: String, name: String) throws -> String {
    try database.withCString { db in
        try "local_filesystem".withCString { adapter in
            try name.withCString { label in
                try decode(registryAddForTest(db, adapter, label), as: CreatedID.self).id
            }
        }
    }
}

private func createScenario(
    database: String, leftID: String, leftRoot: String, rightID: String, rightRoot: String
) throws -> String {
    try database.withCString { db in
        try "Swift dry-run bridge".withCString { name in
            try leftID.withCString { left in
                try leftRoot.withCString { leftRoot in
                    try rightID.withCString { right in
                        try rightRoot.withCString { rightRoot in
                            try "left_to_right".withCString { direction in
                                try decode(
                                    scenarioCreateForTest(
                                        db, name, left, leftRoot, right, rightRoot, direction
                                    ),
                                    as: CreatedID.self
                                ).id
                            }
                        }
                    }
                }
            }
        }
    }
}

@main
struct ScenarioDryRunBridgeTests {
    static func main() throws {
        let temporary = URL(fileURLWithPath: try physicalPath(FileManager.default.temporaryDirectory.path))
            .appendingPathComponent("qadren-dry-run-bridge-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: temporary, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: temporary) }
        let leftRoot = temporary.appendingPathComponent("left", isDirectory: true)
        let rightRoot = temporary.appendingPathComponent("right", isDirectory: true)
        try FileManager.default.createDirectory(at: leftRoot, withIntermediateDirectories: true)
        try FileManager.default.createDirectory(at: rightRoot, withIntermediateDirectories: true)
        try Data("metadata only".utf8).write(to: leftRoot.appendingPathComponent("report.txt"))
        let database = temporary.appendingPathComponent("registry.sqlite3").path
        let leftID = try addConnection(database: database, name: "Left")
        let rightID = try addConnection(database: database, name: "Right")
        let scenarioID = try createScenario(
            database: database, leftID: leftID, leftRoot: leftRoot.path,
            rightID: rightID, rightRoot: rightRoot.path
        )

        let leftSnapshot = try StorageRegistryBridge.localScenarioSnapshot(root: leftRoot.path)
        let rightSnapshot = try StorageRegistryBridge.localScenarioSnapshot(root: rightRoot.path)
        check(leftSnapshot.count == 1, "metadata scan returns the source file")
        check(leftSnapshot[0].relative_path == "report.txt", "snapshot keeps a relative path")
        check(leftSnapshot[0].checksum_md5 == nil, "snapshot does not hash file content")
        let plan = try StorageRegistryBridge.dryRunScenario(
            id: scenarioID, revision: 1, left: leftSnapshot, right: rightSnapshot,
            databasePath: database
        )
        check(plan.count == 1, "dry run creates exactly one proposal")
        check(plan[0].action.kind == "create", "source-only file is a create proposal")
        check(plan[0].action.from_left == true, "proposal preserves source direction")
        check(!FileManager.default.fileExists(atPath: rightRoot.appendingPathComponent("report.txt").path),
              "dry run does not transfer content")
        print("Scenario dry-run native Swift/FFI bridge tests passed")
    }
}
