import Foundation

private func check(_ condition: @autoclosure () -> Bool, _ message: String) {
    guard condition() else { fatalError("FAIL: " + message) }
}

private func state(of jobID: String, in jobs: [JobRow]) -> String? {
    jobs.first(where: { $0.id == jobID })?.state
}

@main
struct JobInventoryTests {
    static func main() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("qadren-job-inventory-tests-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: root) }

        let paths = try JobInventory.paths(in: root)
        let queuedID = "swift-g1-queued"
        let cancelledID = "swift-g1-cancelled"

        var jobs = try JobInventory.list(databasePath: paths.database)
        check(jobs.isEmpty, "new inventory is empty")

        try JobInventory.queueMockArtifact(jobID: queuedID, databasePath: paths.database)
        jobs = try JobInventory.list(databasePath: paths.database)
        check(
            state(of: queuedID, in: jobs) == "queued",
            "queue persists through the Swift bridge"
        )

        try JobInventory.pauseMockArtifact(jobID: queuedID, databasePath: paths.database)
        jobs = try JobInventory.list(databasePath: paths.database)
        check(
            state(of: queuedID, in: jobs) == "paused",
            "pause persists through the Swift bridge"
        )

        try JobInventory.resumeMockArtifact(jobID: queuedID, databasePath: paths.database)
        jobs = try JobInventory.list(databasePath: paths.database)
        check(
            state(of: queuedID, in: jobs) == "queued",
            "resume persists through the Swift bridge"
        )

        try JobInventory.runMockArtifact(
            jobID: queuedID, databasePath: paths.database, mockStoragePath: paths.mockStorage
        )
        jobs = try JobInventory.list(databasePath: paths.database)
        check(
            state(of: queuedID, in: jobs) == "completed",
            "run completes through the Swift bridge"
        )
        check(
            FileManager.default.fileExists(
                atPath: URL(fileURLWithPath: paths.mockStorage)
                    .appendingPathComponent(queuedID + ".qadren-g1-test").path
            ),
            "completed mock artifact remains inside the supplied app-owned root"
        )

        try JobInventory.queueMockArtifact(jobID: cancelledID, databasePath: paths.database)
        try JobInventory.cancelMockArtifact(jobID: cancelledID, databasePath: paths.database)
        jobs = try JobInventory.list(databasePath: paths.database)
        check(
            state(of: cancelledID, in: jobs) == "cancelled",
            "cancel changes only the queued Job"
        )
        check(
            !FileManager.default.fileExists(
                atPath: URL(fileURLWithPath: paths.mockStorage)
                    .appendingPathComponent(cancelledID + ".qadren-g1-test").path
            ),
            "cancelled Job creates no artifact"
        )

        print("JobInventory native Swift/FFI bridge tests passed")
    }
}
