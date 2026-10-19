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
@_silgen_name("qadren_registry_set_enabled")
private func registrySetEnabledForTest(
    _ path: UnsafePointer<CChar>?, _ id: UnsafePointer<CChar>?, _ enabled: UInt8
) -> UnsafeMutablePointer<CChar>?

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

private func setConnectionEnabled(database: String, id: String, enabled: Bool) throws {
    try database.withCString { database in
        try id.withCString { id in
            let _: Bool = try decode(
                registrySetEnabledForTest(database, id, enabled ? 1 : 0), as: Bool.self
            )
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
        try Data("second file".utf8).write(to: leftRoot.appendingPathComponent("notes.txt"))
        let database = temporary.appendingPathComponent("registry.sqlite3").path
        let leftID = try addConnection(database: database, name: "Left")
        let rightID = try addConnection(database: database, name: "Right")
        let scenarioID = try createScenario(
            database: database, leftID: leftID, leftRoot: leftRoot.path,
            rightID: rightID, rightRoot: rightRoot.path
        )

        let leftSnapshot = try StorageRegistryBridge.localScenarioSnapshot(
            root: leftRoot.path, connectionID: leftID, databasePath: database
        )
        let rightSnapshot = try StorageRegistryBridge.localScenarioSnapshot(
            root: rightRoot.path, connectionID: rightID, databasePath: database
        )
        check(leftSnapshot.count == 2, "metadata scan returns both source files")
        check(
            Set(leftSnapshot.map(\.relative_path)) == Set(["notes.txt", "report.txt"]),
            "snapshot keeps both relative paths"
        )
        check(leftSnapshot.allSatisfy { $0.checksum_md5 == nil }, "snapshot does not hash file content")
        let plan = try StorageRegistryBridge.dryRunScenario(
            id: scenarioID, revision: 1, left: leftSnapshot, right: rightSnapshot,
            databasePath: database
        )
        check(plan.count == 2, "dry run creates one proposal for each source file")
        check(plan.allSatisfy { $0.action.kind == "create" }, "source-only files are create proposals")
        check(plan.allSatisfy { $0.action.from_left == true }, "proposals preserve source direction")
        check(!FileManager.default.fileExists(atPath: rightRoot.appendingPathComponent("report.txt").path),
              "dry run does not transfer content")
        let scenario = ScenarioEntry(
            id: scenarioID,
            name: "Swift dry-run bridge",
            left_connection_id: leftID,
            left_root: leftRoot.path,
            right_connection_id: rightID,
            right_root: rightRoot.path,
            direction: "left_to_right",
            enabled: true,
            revision: 1
        )
        let jobs = temporary.appendingPathComponent("jobs.sqlite3").path
        for item in plan {
            try StorageRegistryBridge.executeScenarioCreate(
                scenario: scenario, item: item, registryPath: database, jobsPath: jobs
            )
        }
        let expectedContent = [
            "notes.txt": Data("second file".utf8),
            "report.txt": Data("metadata only".utf8),
        ]
        for (relativePath, expectedData) in expectedContent {
            let copiedData = try Data(contentsOf: rightRoot.appendingPathComponent(relativePath))
            check(copiedData == expectedData, "approved Scenario creates copy exactly \(relativePath)")
        }
        let createdJobs = try JobInventory.list(databasePath: jobs)
        check(createdJobs.count == 2, "each Scenario create records a durable Job")
        check(createdJobs.allSatisfy { $0.state == "completed" }, "Scenario creates record verified completion")
        check(createdJobs.allSatisfy { $0.scenario_id == scenarioID }, "Jobs preserve Scenario identity")
        check(createdJobs.allSatisfy { $0.scenario_revision == 1 }, "Jobs preserve Scenario revision")
        let updatedRightSnapshot = try StorageRegistryBridge.localScenarioSnapshot(
            root: rightRoot.path, connectionID: rightID, databasePath: database
        )
        let planAfterCopy = try StorageRegistryBridge.dryRunScenario(
            id: scenarioID, revision: 1, left: leftSnapshot, right: updatedRightSnapshot,
            databasePath: database
        )
        check(planAfterCopy.count == 2, "post-copy dry run keeps both tracked paths")
        check(planAfterCopy.allSatisfy { $0.action.kind == "no_action" }, "baseline prevents duplicate copy proposals")
        let conflictID = try StorageRegistryBridge.queueScenarioConflict(
            scenario: scenario, relativePath: "report.txt", kind: "content",
            left: leftSnapshot[0], right: leftSnapshot[0],
            databasePath: database
        )
        var conflicts = try StorageRegistryBridge.listScenarioConflicts(
            scenario: scenario, databasePath: database
        )
        check(conflicts.count == 1, "conflict is recorded in its independent queue")
        check(conflicts[0].id == conflictID, "queue returns its persistent task identifier")
        check(conflicts[0].source_is_left == nil, "a conflict has no implicit source")
        try StorageRegistryBridge.selectScenarioConflict(
            id: conflictID, sourceIsLeft: false, decision: "skip_transfer", databasePath: database
        )
        conflicts = try StorageRegistryBridge.listScenarioConflicts(scenario: scenario, databasePath: database)
        check(conflicts[0].source_is_left == false, "the selected source is durable")
        check(conflicts[0].decision == "skip_transfer", "the chosen policy is durable")
        try setConnectionEnabled(database: database, id: leftID, enabled: false)
        do {
            _ = try StorageRegistryBridge.localScenarioSnapshot(
                root: leftRoot.path, connectionID: leftID, databasePath: database
            )
            fatalError("disabled Local connection scanned a Scenario root")
        } catch {
            // The Registry gate runs before the Local Filesystem scan.
        }
        print("Scenario dry-run native Swift/FFI bridge tests passed")
    }
}
