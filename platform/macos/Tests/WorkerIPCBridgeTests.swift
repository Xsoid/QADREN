import Darwin
import Foundation

@_silgen_name("qadren_scenarios_create")
private func scenarioCreateForWorkerTest(
    _ path: UnsafePointer<CChar>?, _ name: UnsafePointer<CChar>?,
    _ leftID: UnsafePointer<CChar>?, _ leftRoot: UnsafePointer<CChar>?,
    _ rightID: UnsafePointer<CChar>?, _ rightRoot: UnsafePointer<CChar>?,
    _ direction: UnsafePointer<CChar>?
) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_registry_free_string")
private func freeRegistryStringForWorkerTest(_ pointer: UnsafeMutablePointer<CChar>?)

private struct WorkerTestEnvelope<T: Decodable>: Decodable {
    let ok: Bool
    let data: T?
    let error: String?
}
private struct WorkerTestCreatedID: Decodable { let id: String }

private func check(_ condition: @autoclosure () -> Bool, _ message: String) {
    guard condition() else { fatalError("FAIL: \(message)") }
}

private func physicalPath(_ path: String) throws -> String {
    guard let resolved = realpath(path, nil) else {
        throw StorageError.operation("cannot resolve test root")
    }
    defer { free(resolved) }
    return String(cString: resolved)
}

private func decodeCreatedID(_ pointer: UnsafeMutablePointer<CChar>?) throws -> String {
    guard let pointer else { throw StorageError.operation("missing Rust response") }
    defer { freeRegistryStringForWorkerTest(pointer) }
    let response = try JSONDecoder().decode(
        WorkerTestEnvelope<WorkerTestCreatedID>.self, from: Data(String(cString: pointer).utf8)
    )
    guard response.ok, let id = response.data?.id else {
        throw StorageError.operation(response.error ?? "Rust request failed")
    }
    return id
}

private func createScenario(
    database: String, leftID: String, leftRoot: String, rightID: String, rightRoot: String
) throws -> String {
    try database.withCString { database in
        try "Worker IPC bridge".withCString { name in
            try leftID.withCString { leftID in
                try leftRoot.withCString { leftRoot in
                    try rightID.withCString { rightID in
                        try rightRoot.withCString { rightRoot in
                            try "left_to_right".withCString { direction in
                                try decodeCreatedID(
                                    scenarioCreateForWorkerTest(
                                        database, name, leftID, leftRoot, rightID, rightRoot, direction
                                    )
                                )
                            }
                        }
                    }
                }
            }
        }
    }
}

@main
struct WorkerIPCBridgeTests {
    static func main() throws {
        if CommandLine.arguments.dropFirst().first == "--create-controller" {
            try runCreateController(arguments: Array(CommandLine.arguments.dropFirst(2)))
            return
        }
        guard CommandLine.arguments.count == 2 else {
            throw StorageError.operation("worker executable path is required")
        }
        let root = URL(fileURLWithPath: try physicalPath(FileManager.default.temporaryDirectory.path))
            .appendingPathComponent("qadren-worker-ipc-\(UUID().uuidString)", isDirectory: true)
        let socketDirectory = URL(fileURLWithPath: "/tmp", isDirectory: true)
            .appendingPathComponent("qw-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(
            at: root, withIntermediateDirectories: true,
            attributes: [.posixPermissions: NSNumber(value: 0o700)]
        )
        try FileManager.default.createDirectory(
            at: socketDirectory, withIntermediateDirectories: false,
            attributes: [.posixPermissions: NSNumber(value: 0o700)]
        )
        defer {
            try? FileManager.default.removeItem(at: root)
            try? FileManager.default.removeItem(at: socketDirectory)
        }

        let sourceRoot = root.appendingPathComponent("source", isDirectory: true)
        let destinationRoot = root.appendingPathComponent("destination", isDirectory: true)
        try FileManager.default.createDirectory(at: sourceRoot, withIntermediateDirectories: true)
        try FileManager.default.createDirectory(at: destinationRoot, withIntermediateDirectories: true)
        try Data("Worker IPC content".utf8).write(to: sourceRoot.appendingPathComponent("report.txt"))
        let registry = root.appendingPathComponent("connections.sqlite3").path
        let jobs = root.appendingPathComponent("jobs.sqlite3").path
        let left = try StorageRegistryBridge.addLocalComputer(name: "Left", databasePath: registry)
        let right = try StorageRegistryBridge.addLocalComputer(name: "Right", databasePath: registry)
        let scenario = try createScenario(
            database: registry, leftID: left, leftRoot: sourceRoot.path,
            rightID: right, rightRoot: destinationRoot.path
        )

        let socket = socketDirectory.appendingPathComponent("worker.sock").path
        let request = WorkerScenarioCreateRequest(
            scenario_id: scenario,
            scenario_revision: 1,
            source_root: sourceRoot.path,
            source_connection_id: left,
            source_relative_path: "report.txt",
            destination_root: destinationRoot.path,
            destination_connection_id: right,
            destination_relative_path: "report.txt",
            job_id: "swift-worker-ipc-create"
        )
        do {
            _ = try WorkerIPC.executeScenarioCreate(request, socketPath: socket)
            fatalError("missing Worker socket executed a Scenario Create")
        } catch WorkerIPCError.unavailable {
            check(
                !FileManager.default.fileExists(atPath: destinationRoot.appendingPathComponent("report.txt").path),
                "missing Worker socket does not fall back to an in-process file write"
            )
        }
        let worker = Process()
        worker.executableURL = URL(fileURLWithPath: CommandLine.arguments[1])
        worker.arguments = ["--socket", socket, "--registry", registry, "--jobs", jobs]
        try worker.run()
        defer {
            if worker.isRunning { worker.terminate() }
            worker.waitUntilExit()
        }
        for _ in 0..<100 where !FileManager.default.fileExists(atPath: socket) {
            Thread.sleep(forTimeInterval: 0.01)
        }
        check(FileManager.default.fileExists(atPath: socket), "Worker socket is available")

        let controller = Process()
        controller.executableURL = URL(fileURLWithPath: CommandLine.arguments[0])
        controller.arguments = [
            "--create-controller", socket, scenario, sourceRoot.path, left,
            destinationRoot.path, right
        ]
        try controller.run()
        controller.waitUntilExit()
        check(controller.terminationStatus == 0, "independent controller submits Scenario Create")
        let destinationBytes = try Data(contentsOf: destinationRoot.appendingPathComponent("report.txt"))
        check(
            destinationBytes == Data("Worker IPC content".utf8),
            "Worker creates the exact requested destination bytes"
        )
        let stored = try JobInventory.list(databasePath: jobs)
        check(stored.count == 1, "Worker IPC creates one durable Job")
        check(stored.first?.scenario_id == scenario, "Worker IPC Job binds the Scenario identity")
        check(stored.first?.state == "completed", "Worker IPC Job is verified complete")

        try Data("Worker IPC replacement".utf8).write(to: sourceRoot.appendingPathComponent("report.txt"))
        let declinedReplacement = WorkerScenarioReplaceRequest(
            scenario_id: scenario,
            scenario_revision: 1,
            source_root: sourceRoot.path,
            source_connection_id: left,
            source_relative_path: "report.txt",
            destination_root: destinationRoot.path,
            destination_connection_id: right,
            destination_relative_path: "report.txt",
            job_id: "swift-worker-ipc-replace-declined",
            replacement_consent: false
        )
        do {
            _ = try WorkerIPC.executeScenarioReplace(declinedReplacement, socketPath: socket)
            fatalError("Worker replaced a file without native confirmation")
        } catch WorkerIPCError.rejected("consent_required") {
            let unchangedBytes = try Data(contentsOf: destinationRoot.appendingPathComponent("report.txt"))
            check(
                unchangedBytes == Data("Worker IPC content".utf8),
                "declined Worker replacement keeps the destination bytes"
            )
        }

        let confirmedReplacement = WorkerScenarioReplaceRequest(
            scenario_id: scenario,
            scenario_revision: 1,
            source_root: sourceRoot.path,
            source_connection_id: left,
            source_relative_path: "report.txt",
            destination_root: destinationRoot.path,
            destination_connection_id: right,
            destination_relative_path: "report.txt",
            job_id: "swift-worker-ipc-replace-confirmed",
            replacement_consent: true
        )
        let replacementStatus = try WorkerIPC.executeScenarioReplace(confirmedReplacement, socketPath: socket)
        check(replacementStatus == "completed", "Worker returns completed Scenario Update status")
        let replacedBytes = try Data(contentsOf: destinationRoot.appendingPathComponent("report.txt"))
        check(
            replacedBytes == Data("Worker IPC replacement".utf8),
            "confirmed Worker replacement changes the exact destination bytes"
        )
        let replaced = try JobInventory.list(databasePath: jobs)
        check(replaced.count == 2, "confirmed Worker replacement creates one additional durable Job")
        check(replaced.allSatisfy { $0.scenario_id == scenario && $0.state == "completed" },
              "Worker replacement Job stays revision-bound and verified complete")
        print("Worker IPC native bridge tests passed")
    }

    private static func runCreateController(arguments: [String]) throws {
        guard arguments.count == 6 else { throw StorageError.operation("invalid controller arguments") }
        let status = try WorkerIPC.executeScenarioCreate(
            WorkerScenarioCreateRequest(
                scenario_id: arguments[1], scenario_revision: 1,
                source_root: arguments[2], source_connection_id: arguments[3],
                source_relative_path: "report.txt", destination_root: arguments[4],
                destination_connection_id: arguments[5], destination_relative_path: "report.txt",
                job_id: "swift-worker-ipc-create"
            ), socketPath: arguments[0]
        )
        guard status == "completed" else { throw StorageError.operation("controller did not create Job") }
    }
}
