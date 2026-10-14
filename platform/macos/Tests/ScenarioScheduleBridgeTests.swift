import Foundation

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
@_silgen_name("qadren_scenario_schedules_create")
private func scheduleCreateForTest(
    _ path: UnsafePointer<CChar>?, _ scenarioID: UnsafePointer<CChar>?, _ timezone: UnsafePointer<CChar>?,
    _ weekday: UInt8, _ hour: UInt8, _ minute: UInt8
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

private func createScenario(database: String, leftID: String, rightID: String) throws -> String {
    try database.withCString { db in
        try "Swift schedule bridge".withCString { name in
            try leftID.withCString { left in
                try "/left".withCString { leftRoot in
                    try rightID.withCString { right in
                        try "/right".withCString { rightRoot in
                            try "left_to_right".withCString { direction in
                                try decode(
                                    scenarioCreateForTest(db, name, left, leftRoot, right, rightRoot, direction),
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

private func createSchedule(database: String, scenarioID: String) throws {
    try database.withCString { db in
        try scenarioID.withCString { scenario in
            try "UTC".withCString { timezone in
                _ = try decode(
                    scheduleCreateForTest(db, scenario, timezone, 0, 12, 0), as: CreatedID.self
                )
            }
        }
    }
}

private func utc(_ value: String) -> Date {
    ISO8601DateFormatter().date(from: value)!
}

@main
struct ScenarioScheduleBridgeTests {
    static func main() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("qadren-schedule-bridge-tests-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let database = root.appendingPathComponent("registry.sqlite3").path

        let leftID = try addConnection(database: database, name: "Left")
        let rightID = try addConnection(database: database, name: "Right")
        let scenarioID = try createScenario(database: database, leftID: leftID, rightID: rightID)
        try createSchedule(database: database, scenarioID: scenarioID)

        var automation = try StorageRegistryBridge.scenarioAutomation(
            scenarioID: scenarioID, databasePath: database
        )
        check(!automation.locally_paused, "new Scenario automation starts unpaused")
        try StorageRegistryBridge.setScenarioLocallyPaused(
            scenarioID: scenarioID, paused: true, databasePath: database
        )
        automation = try StorageRegistryBridge.scenarioAutomation(
            scenarioID: scenarioID, databasePath: database
        )
        check(automation.locally_paused, "Swift bridge persists local automatic-trigger pause")

        let pausedOccurrences = try StorageRegistryBridge.dueScenarioSchedules(
            after: utc("2026-10-18T00:00:00Z"),
            through: utc("2026-10-20T00:00:00Z"),
            databasePath: database
        )
        check(pausedOccurrences.isEmpty, "paused Scenario yields no automatic candidates")
        try StorageRegistryBridge.setScenarioLocallyPaused(
            scenarioID: scenarioID, paused: false, databasePath: database
        )

        let occurrences = try StorageRegistryBridge.dueScenarioSchedules(
            after: utc("2026-10-18T00:00:00Z"),
            through: utc("2026-10-20T00:00:00Z"),
            databasePath: database
        )
        check(occurrences.count == 1, "Swift bridge returns exactly one enabled weekly candidate")
        check(occurrences[0].scenario_id == scenarioID, "candidate retains the scenario identity")
        check(occurrences[0].occurrence_key == "2026-10-19 12:00@UTC", "candidate preserves Core slot identity")

        let repeated = try StorageRegistryBridge.dueScenarioSchedules(
            after: utc("2026-10-18T00:00:00Z"),
            through: utc("2026-10-20T00:00:00Z"),
            databasePath: database
        )
        check(repeated == occurrences, "read-only preview does not claim or mutate an occurrence")

        print("Scenario schedule native Swift/FFI bridge tests passed")
    }
}
