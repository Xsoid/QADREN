import Darwin
import Foundation

@_silgen_name("qadren_scenarios_create")
private func scenarioCreateForWorkerTest(
    _ path: UnsafePointer<CChar>?, _ name: UnsafePointer<CChar>?,
    _ leftID: UnsafePointer<CChar>?, _ leftRoot: UnsafePointer<CChar>?,
    _ rightID: UnsafePointer<CChar>?, _ rightRoot: UnsafePointer<CChar>?,
    _ direction: UnsafePointer<CChar>?
) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_scenario_create_batch")
private func scenarioCreateBatchForWorkerTest(
    _ path: UnsafePointer<CChar>?, _ scenarioID: UnsafePointer<CChar>?, _ revision: UInt64,
    _ items: UnsafePointer<CChar>?
) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_registry_free_string")
private func freeRegistryStringForWorkerTest(_ pointer: UnsafeMutablePointer<CChar>?)

private struct WorkerTestEnvelope<T: Decodable>: Decodable {
    let ok: Bool
    let data: T?
    let error: String?
}
private struct WorkerTestCreatedID: Decodable { let id: String }
private struct WorkerTestBatchReceipt: Decodable { let id: String }

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

private func createBatch(database: String, scenario: String, items: String) throws -> String {
    try database.withCString { database in
        try scenario.withCString { scenario in
            try items.withCString { items in
                guard let pointer = scenarioCreateBatchForWorkerTest(database, scenario, 1, items) else {
                    throw StorageError.operation("missing Rust batch response")
                }
                defer { freeRegistryStringForWorkerTest(pointer) }
                let response = try JSONDecoder().decode(
                    WorkerTestEnvelope<WorkerTestBatchReceipt>.self,
                    from: Data(String(cString: pointer).utf8)
                )
                guard response.ok, let receipt = response.data else {
                    throw StorageError.operation(response.error ?? "Rust batch request failed")
                }
                return receipt.id
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

        let batchSource = root.appendingPathComponent("batch-source", isDirectory: true)
        let batchDestination = root.appendingPathComponent("batch-destination", isDirectory: true)
        try FileManager.default.createDirectory(at: batchSource, withIntermediateDirectories: true)
        try FileManager.default.createDirectory(at: batchDestination, withIntermediateDirectories: true)
        try Data("first batch file".utf8).write(to: batchSource.appendingPathComponent("first.txt"))
        try Data("second batch file".utf8).write(to: batchSource.appendingPathComponent("second.txt"))
        let batchScenario = try createScenario(
            database: registry, leftID: left, leftRoot: batchSource.path,
            rightID: right, rightRoot: batchDestination.path
        )
        let batchID = try createBatch(
            database: registry, scenario: batchScenario,
            items: #"[{"relative_path":"first.txt","source_is_left":true},{"relative_path":"second.txt","source_is_left":true}]"#
        )
        let reopenedBatchScenario = ScenarioEntry(
            id: batchScenario,
            name: "Worker IPC batch",
            left_connection_id: left,
            left_root: batchSource.path,
            right_connection_id: right,
            right_root: batchDestination.path,
            direction: "left_to_right",
            enabled: true,
            revision: 1
        )
        let pendingAfterControllerReopen = try StorageRegistryBridge.pendingScenarioCreateBatches(
            scenario: reopenedBatchScenario, databasePath: registry
        )
        check(pendingAfterControllerReopen.count == 1, "reopened controller finds its pending Create batch")
        check(pendingAfterControllerReopen[0].id == batchID, "reopened controller reads the exact receipt")
        check(
            pendingAfterControllerReopen[0].children.allSatisfy { $0.state == "pending" },
            "reopened controller reports only durable pending children"
        )
        let batchStatus = try WorkerIPC.executeScenarioCreateBatch(receiptID: batchID, socketPath: socket)
        check(batchStatus == "accepted", "Worker accepts a confirmed Create batch receipt")
        // D-024: the Worker runs the batch on its run thread. Wait for it.
        var pendingAfterTerminalOutcome = try StorageRegistryBridge.pendingScenarioCreateBatches(
            scenario: reopenedBatchScenario, databasePath: registry
        )
        let batchDeadline = Date().addingTimeInterval(30)
        while !pendingAfterTerminalOutcome.isEmpty && Date() < batchDeadline {
            Thread.sleep(forTimeInterval: 0.2)
            pendingAfterTerminalOutcome = try StorageRegistryBridge.pendingScenarioCreateBatches(
                scenario: reopenedBatchScenario, databasePath: registry
            )
        }
        check(
            pendingAfterTerminalOutcome.isEmpty,
            "terminal batch is not restored as pending work"
        )
        let firstBatchBytes = try Data(contentsOf: batchDestination.appendingPathComponent("first.txt"))
        let secondBatchBytes = try Data(contentsOf: batchDestination.appendingPathComponent("second.txt"))
        check(
            firstBatchBytes == Data("first batch file".utf8),
            "Worker batch creates the first selected destination"
        )
        check(
            secondBatchBytes == Data("second batch file".utf8),
            "Worker batch creates the second selected destination"
        )

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
        check(stored.count == 3, "Worker batch and controller create three durable Jobs")
        let controllerJob = stored.first { $0.scenario_id == scenario }
        check(controllerJob?.scenario_id == scenario, "Worker IPC Job binds the Scenario identity")
        check(controllerJob?.state == "completed", "Worker IPC Job is verified complete")

        let caseSourceRoot = root.appendingPathComponent("case-source", isDirectory: true)
        let caseDestinationRoot = root.appendingPathComponent("case-destination", isDirectory: true)
        try FileManager.default.createDirectory(at: caseSourceRoot, withIntermediateDirectories: true)
        try FileManager.default.createDirectory(at: caseDestinationRoot, withIntermediateDirectories: true)
        try Data("new worker report".utf8).write(to: caseSourceRoot.appendingPathComponent("report.txt"))
        try Data("protected worker report".utf8).write(to: caseDestinationRoot.appendingPathComponent("REPORT.txt"))
        let caseScenario = try createScenario(
            database: registry, leftID: left, leftRoot: caseSourceRoot.path,
            rightID: right, rightRoot: caseDestinationRoot.path
        )
        let jobsBeforeCaseCollision = try JobInventory.list(databasePath: jobs).count
        do {
            _ = try WorkerIPC.executeScenarioCreate(WorkerScenarioCreateRequest(
                scenario_id: caseScenario, scenario_revision: 1,
                source_root: caseSourceRoot.path, source_connection_id: left,
                source_relative_path: "report.txt", destination_root: caseDestinationRoot.path,
                destination_connection_id: right, destination_relative_path: "report.txt",
                job_id: "swift-worker-ipc-case-collision"
            ), socketPath: socket)
            fatalError("Worker Scenario Create overwrote a case-colliding destination")
        } catch WorkerIPCError.rejected("target_exists") {
            let protectedBytes = try Data(contentsOf: caseDestinationRoot.appendingPathComponent("REPORT.txt"))
            check(protectedBytes == Data("protected worker report".utf8), "Worker preserves case-colliding destination bytes")
        }
        let jobsAfterCaseCollision = try JobInventory.list(databasePath: jobs).count
        check(jobsAfterCaseCollision == jobsBeforeCaseCollision, "Worker case collision creates no Job")

        let overlapRoot = root.appendingPathComponent("overlap", isDirectory: true)
        let nestedOverlapRoot = overlapRoot.appendingPathComponent("nested", isDirectory: true)
        try FileManager.default.createDirectory(at: nestedOverlapRoot, withIntermediateDirectories: true)
        try Data("must not copy".utf8).write(to: overlapRoot.appendingPathComponent("report.txt"))
        // The Registry refuses nested Local roots when the Scenario is saved,
        // so no executor request can be built for them.
        let jobsBeforeOverlap = try JobInventory.list(databasePath: jobs).count
        do {
            _ = try createScenario(
                database: registry, leftID: left, leftRoot: overlapRoot.path,
                rightID: right, rightRoot: nestedOverlapRoot.path
            )
            fatalError("Registry saved a Scenario with overlapping Local roots")
        } catch StorageError.operation("scenario_roots_overlap") {
            check(
                !FileManager.default.fileExists(atPath: nestedOverlapRoot.appendingPathComponent("report.txt").path),
                "refused overlapping roots do not write the nested destination"
            )
        }
        let jobsAfterOverlap = try JobInventory.list(databasePath: jobs).count
        check(jobsAfterOverlap == jobsBeforeOverlap, "Worker overlapping roots create no Job")

        try Data("Worker IPC replacement".utf8).write(to: sourceRoot.appendingPathComponent("report.txt"))
        func planned(_ root: URL, _ connection: String) throws -> WorkerExpectedFileVersion {
            guard let item = try StorageRegistryBridge.localScenarioSnapshot(
                root: root.path, connectionID: connection, databasePath: registry
            ).first(where: { $0.relative_path == "report.txt" }) else {
                throw StorageError.operation("missing planned file")
            }
            return WorkerExpectedFileVersion(object_revision: item.object_revision, size: item.size)
        }
        let plannedSource = try planned(sourceRoot, left)
        let plannedDestination = try planned(destinationRoot, right)
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
            replacement_consent: false,
            expected_source: plannedSource,
            expected_destination: plannedDestination
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
            replacement_consent: true,
            expected_source: plannedSource,
            expected_destination: plannedDestination
        )
        let replacementStatus = try WorkerIPC.executeScenarioReplace(confirmedReplacement, socketPath: socket)
        check(replacementStatus == "completed", "Worker returns completed Scenario Update status")
        let replacedBytes = try Data(contentsOf: destinationRoot.appendingPathComponent("report.txt"))
        check(
            replacedBytes == Data("Worker IPC replacement".utf8),
            "confirmed Worker replacement changes the exact destination bytes"
        )
        let replaced = try JobInventory.list(databasePath: jobs)
        check(replaced.count == 4, "confirmed Worker replacement creates one additional durable Job")
        check(replaced.filter { $0.scenario_id == scenario }.allSatisfy { $0.state == "completed" },
              "Worker replacement Job stays revision-bound and verified complete")

        try Data("Manual Worker copy".utf8).write(to: sourceRoot.appendingPathComponent("manual.txt"))
        let manualCreate = try WorkerIPC.executeManualCopy(WorkerManualCopyRequest(
            command: "execute_manual_copy", source_root: sourceRoot.path,
            source_connection_id: left, source_relative_path: "manual.txt",
            destination_root: destinationRoot.path, destination_connection_id: right,
            destination_relative_path: "manual.txt", job_id: "swift-worker-ipc-manual-create",
            replacement_consent: nil
        ), socketPath: socket)
        check(manualCreate == "completed", "Worker returns completed manual Create status")
        let manualCreatedBytes = try Data(contentsOf: destinationRoot.appendingPathComponent("manual.txt"))
        check(manualCreatedBytes == Data("Manual Worker copy".utf8),
              "Worker manual Create writes the exact requested destination bytes")
        let reconciledCompletedCopy = try WorkerIPC.reconcileManual(WorkerManualCopyRequest(
            command: "reconcile_manual_copy", source_root: destinationRoot.path,
            source_connection_id: right, source_relative_path: "", destination_root: "",
            destination_connection_id: "", destination_relative_path: "",
            job_id: "swift-worker-ipc-manual-create", replacement_consent: nil
        ), socketPath: socket)
        check(reconciledCompletedCopy == "completed", "Worker reports an already completed manual copy as completed")
        let reconnectStatus = try WorkerIPC.reconcileLocalReconnect(root: destinationRoot.path, connectionID: right, socketPath: socket)
        check(reconnectStatus == "completed", "Worker completes reconnect reconciliation without an FFI controller")

        try Data("Manual Worker replacement".utf8).write(to: sourceRoot.appendingPathComponent("manual.txt"))
        let manualReplace = try WorkerIPC.executeManualCopy(WorkerManualCopyRequest(
            command: "execute_manual_replace", source_root: sourceRoot.path,
            source_connection_id: left, source_relative_path: "manual.txt",
            destination_root: destinationRoot.path, destination_connection_id: right,
            destination_relative_path: "manual.txt", job_id: "swift-worker-ipc-manual-replace",
            replacement_consent: true
        ), socketPath: socket)
        check(manualReplace == "completed", "Worker returns completed manual Replace status")
        let manualReplacedBytes = try Data(contentsOf: destinationRoot.appendingPathComponent("manual.txt"))
        check(manualReplacedBytes == Data("Manual Worker replacement".utf8),
              "Worker manual Replace writes the exact requested destination bytes")
        let mkdirStatus = try WorkerIPC.executeManualDirectoryCreate(WorkerManualCopyRequest(
            command: "execute_manual_mkdir", source_root: destinationRoot.path,
            source_connection_id: right, source_relative_path: "worker-folder",
            destination_root: "", destination_connection_id: "", destination_relative_path: "",
            job_id: "swift-worker-ipc-manual-mkdir", replacement_consent: nil
        ), socketPath: socket)
        check(mkdirStatus == "completed", "Worker returns completed manual mkdir status")
        var mkdirIsDirectory: ObjCBool = false
        check(FileManager.default.fileExists(atPath: destinationRoot.appendingPathComponent("worker-folder").path, isDirectory: &mkdirIsDirectory) && mkdirIsDirectory.boolValue,
              "Worker manual mkdir creates the exact requested directory")
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
