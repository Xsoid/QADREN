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
@_silgen_name("qadren_scenario_create_batch")
private func scenarioCreateBatch(
    _ path: UnsafePointer<CChar>?, _ scenarioID: UnsafePointer<CChar>?, _ revision: UInt64,
    _ items: UnsafePointer<CChar>?
) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_scenarios_dry_run")
private func scenarioDryRun(
    _ path: UnsafePointer<CChar>?, _ scenarioID: UnsafePointer<CChar>?, _ revision: UInt64,
    _ leftSnapshot: UnsafePointer<CChar>?, _ rightSnapshot: UnsafePointer<CChar>?
) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_scenario_conflicts_queue")
private func scenarioConflictQueue(
    _ path: UnsafePointer<CChar>?, _ scenarioID: UnsafePointer<CChar>?, _ revision: UInt64,
    _ relativePath: UnsafePointer<CChar>?, _ kind: UnsafePointer<CChar>?, _ sourceIsLeft: Int32,
    _ leftSnapshot: UnsafePointer<CChar>?, _ rightSnapshot: UnsafePointer<CChar>?
) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_scenario_conflicts_list")
private func scenarioConflictList(
    _ path: UnsafePointer<CChar>?, _ scenarioID: UnsafePointer<CChar>?, _ revision: UInt64
) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_scenario_conflicts_select")
private func scenarioConflictSelect(
    _ path: UnsafePointer<CChar>?, _ id: UnsafePointer<CChar>?, _ sourceIsLeft: Int32,
    _ decision: UnsafePointer<CChar>?
) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_local_scenario_metadata_snapshot")
private func localScenarioMetadataSnapshot(
    _ registry: UnsafePointer<CChar>?, _ connectionID: UnsafePointer<CChar>?, _ root: UnsafePointer<CChar>?
) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_jobs_run_local_scenario_file_copy")
private func runLocalScenarioCopyJob(
    _ jobs: UnsafePointer<CChar>?, _ registry: UnsafePointer<CChar>?, _ scenarioID: UnsafePointer<CChar>?, _ revision: UInt64,
    _ sourceRoot: UnsafePointer<CChar>?, _ sourceConnection: UnsafePointer<CChar>?, _ sourceRelative: UnsafePointer<CChar>?,
    _ destinationRoot: UnsafePointer<CChar>?, _ destinationConnection: UnsafePointer<CChar>?, _ destinationRelative: UnsafePointer<CChar>?,
    _ jobID: UnsafePointer<CChar>?
) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_jobs_run_local_scenario_file_replace")
private func runLocalScenarioReplaceJob(
    _ jobs: UnsafePointer<CChar>?, _ registry: UnsafePointer<CChar>?, _ scenarioID: UnsafePointer<CChar>?, _ revision: UInt64,
    _ sourceRoot: UnsafePointer<CChar>?, _ sourceConnection: UnsafePointer<CChar>?, _ sourceRelative: UnsafePointer<CChar>?,
    _ destinationRoot: UnsafePointer<CChar>?, _ destinationConnection: UnsafePointer<CChar>?, _ destinationRelative: UnsafePointer<CChar>?,
    _ jobID: UnsafePointer<CChar>?, _ consent: Bool
) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_jobs_run_local_scenario_conflict_replace")
private func runLocalScenarioConflictReplaceJob(
    _ jobs: UnsafePointer<CChar>?, _ registry: UnsafePointer<CChar>?,
    _ conflictID: UnsafePointer<CChar>?, _ jobID: UnsafePointer<CChar>?, _ consent: Bool
) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_jobs_run_local_scenario_conflict_delete")
private func runLocalScenarioConflictDeleteJob(
    _ jobs: UnsafePointer<CChar>?, _ registry: UnsafePointer<CChar>?,
    _ conflictID: UnsafePointer<CChar>?, _ jobID: UnsafePointer<CChar>?, _ consent: Bool
) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_jobs_dispatch_scenario_copy_baselines")
private func dispatchScenarioCopyBaselines(
    _ jobs: UnsafePointer<CChar>?, _ registry: UnsafePointer<CChar>?
) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_scenario_schedules_list")
private func scenarioSchedulesList(
    _ path: UnsafePointer<CChar>?, _ scenarioID: UnsafePointer<CChar>?
) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_scenario_schedules_due")
private func scenarioSchedulesDue(
    _ path: UnsafePointer<CChar>?, _ after: UnsafePointer<CChar>?, _ through: UnsafePointer<CChar>?
) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_scenario_schedules_dispatch")
private func scenarioSchedulesDispatch(
    _ registryPath: UnsafePointer<CChar>?, _ jobStorePath: UnsafePointer<CChar>?,
    _ after: UnsafePointer<CChar>?, _ through: UnsafePointer<CChar>?
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
struct ScenarioSnapshotItem: Codable {
    let relative_path: String
    let object_revision: String?
    let size: UInt64
    let checksum_md5: String?
}
private struct ScenarioBaselineDispatchStatus: Decodable {
    let dispatched: Int
}
private struct ScenarioScheduleDispatchStatus: Decodable {
    let staged: Int
    let dispatched: Int
}
struct ScenarioPlanAction: Decodable {
    let kind: String
    let left: ScenarioSnapshotItem?
    let right: ScenarioSnapshotItem?
    let from_left: Bool?
    let on_left: Bool?
    let reason: String?
}
struct ScenarioPlanItem: Decodable, Identifiable {
    let relative_path: String
    let left: String
    let right: String
    let action: ScenarioPlanAction
    var id: String { relative_path }
}

private struct ScenarioCreateBatchReceipt: Decodable { let id: String }
private struct ScenarioCreateBatchItem: Encodable {
    let relative_path: String
    let source_is_left: Bool
}
struct ScenarioConflictTask: Decodable, Identifiable {
    let id: String
    let scenario_id: String
    let scenario_revision: UInt64
    let relative_path: String
    let kind: String
    let source_is_left: Bool?
    let decision: String?
    let state: String
}
private struct NewConnection: Decodable {
    let id: String
}
private struct ScenarioJobStatus: Decodable {
    let status: String
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

    /// Read only metadata below one enabled Local Scenario endpoint. This does
    /// not open content, calculate a checksum, submit a Job, or change a
    /// Scenario baseline.
    static func localScenarioSnapshot(
        root: String, connectionID: String, databasePath: String? = nil
    ) throws -> [ScenarioSnapshotItem] {
        let database = try databasePath ?? path
        return try database.withCString { database in
            try connectionID.withCString { connectionID in
                try root.withCString { root in
                    try unwrap(
                        localScenarioMetadataSnapshot(database, connectionID, root),
                        as: [ScenarioSnapshotItem].self
                    )
                }
            }
        }
    }

    /// Deliver durable Scenario-copy baselines before a dry-run reads them.
    /// This has no adapter effect and never repeats a file copy.
    static func dispatchPendingScenarioCopyBaselines() throws {
        _ = try dispatchPendingScenarioCopyBaselines(
            registryPath: try path, jobsPath: try JobInventory.paths().database
        )
    }

    /// Test seam for the idempotent JobStore-to-Registry baseline hand-off.
    static func dispatchPendingScenarioCopyBaselines(
        registryPath: String, jobsPath: String
    ) throws -> Int {
        let pointer = jobsPath.withCString { jobs in
            registryPath.withCString { registry in
                dispatchScenarioCopyBaselines(jobs, registry)
            }
        }
        return try unwrap(
            pointer, as: ScenarioBaselineDispatchStatus.self, databasePath: registryPath
        ).dispatched
    }

    /// Produce a read-only plan for the exact stored Scenario revision from
    /// supplied metadata snapshots. No adapter scan or transfer occurs here.
    static func dryRunScenario(
        id: String, revision: UInt64, left: [ScenarioSnapshotItem], right: [ScenarioSnapshotItem],
        databasePath: String? = nil
    ) throws -> [ScenarioPlanItem] {
        let database = try databasePath ?? path
        let encoder = JSONEncoder()
        let leftJSON = String(decoding: try encoder.encode(left), as: UTF8.self)
        let rightJSON = String(decoding: try encoder.encode(right), as: UTF8.self)
        return try database.withCString { database in
            try id.withCString { id in
                try leftJSON.withCString { left in
                    try rightJSON.withCString { right in
                        try unwrap(
                            scenarioDryRun(database, id, revision, left, right),
                            as: [ScenarioPlanItem].self
                        )
                    }
                }
            }
        }
    }

    /// Add metadata-only conflict work to the independent Scenario queue.
    /// It neither creates a Job nor reads or changes file content.
    static func queueScenarioConflict(
        scenario: ScenarioEntry, relativePath: String, kind: String,
        left: ScenarioSnapshotItem?, right: ScenarioSnapshotItem?,
        sourceIsLeft: Bool? = nil, databasePath: String? = nil
    ) throws -> String {
        let database = try databasePath ?? path
        let source = sourceIsLeft.map { $0 ? Int32(1) : Int32(0) } ?? -1
        let encoder = JSONEncoder()
        let leftJSON = String(decoding: try encoder.encode(left), as: UTF8.self)
        let rightJSON = String(decoding: try encoder.encode(right), as: UTF8.self)
        let result: NewConnection = try database.withCString { database in
            try scenario.id.withCString { scenarioID in
                try relativePath.withCString { relativePath in
                    try kind.withCString { kind in
                        try leftJSON.withCString { leftJSON in
                            try rightJSON.withCString { rightJSON in
                                try unwrap(
                                    scenarioConflictQueue(database, scenarioID, scenario.revision, relativePath, kind, source, leftJSON, rightJSON),
                                    as: NewConnection.self
                                )
                            }
                        }
                    }
                }
            }
        }
        return result.id
    }

    /// Read the independent conflict queue for one exact Scenario revision.
    static func listScenarioConflicts(
        scenario: ScenarioEntry, databasePath: String? = nil
    ) throws -> [ScenarioConflictTask] {
        let database = try databasePath ?? path
        return try database.withCString { database in
            try scenario.id.withCString { scenarioID in
                try unwrap(
                    scenarioConflictList(database, scenarioID, scenario.revision),
                    as: [ScenarioConflictTask].self
                )
            }
        }
    }

    /// Store a user decision in the independent conflict queue. The selected
    /// source is mandatory. This method does not execute a transfer or delete.
    static func selectScenarioConflict(
        id: String, sourceIsLeft: Bool, decision: String, databasePath: String? = nil
    ) throws {
        if databasePath == nil {
            try WorkerIPC.updateRegistryControl(WorkerRegistryControlRequest(
                command: "select_conflict_decision", scenario_id: nil, locally_paused: nil,
                conflict_id: id, source_is_left: sourceIsLeft, decision: decision
            ))
            return
        }
        let database = try databasePath ?? path
        let source: Int32 = sourceIsLeft ? 1 : 0
        let _: Bool = try database.withCString { database in
            try id.withCString { id in
                try decision.withCString { decision in
                    try unwrap(
                        scenarioConflictSelect(database, id, source, decision), as: Bool.self
                    )
                }
            }
        }
    }

    static func executeScenarioConflictReplace(id: String) throws {
        _ = try WorkerIPC.executeConflict(
            WorkerConflictRequest(
                command: "execute_conflict_replace", conflict_id: id,
                job_id: "scenario-conflict-replace-" + UUID().uuidString.lowercased(),
                destructive_consent: true
            )
        )
    }

    /// Test seam for an explicitly confirmed destructive conflict replacement.
    /// Production always passes `true` only after the native confirmation alert.
    static func executeScenarioConflictReplace(
        id: String, registryPath: String, jobsPath: String, consent: Bool
    ) throws {
        let jobID = "scenario-conflict-replace-" + UUID().uuidString.lowercased()
        let pointer = jobsPath.withCString { jobs in registryPath.withCString { registry in
            id.withCString { id in jobID.withCString { jobID in
                runLocalScenarioConflictReplaceJob(jobs, registry, id, jobID, consent)
            }}
        }}
        let _: ScenarioJobStatus = try unwrap(
            pointer, as: ScenarioJobStatus.self, databasePath: registryPath
        )
    }

    static func executeScenarioConflictDelete(id: String) throws {
        _ = try WorkerIPC.executeConflict(
            WorkerConflictRequest(
                command: "execute_conflict_delete", conflict_id: id,
                job_id: "scenario-conflict-delete-" + UUID().uuidString.lowercased(),
                destructive_consent: true
            )
        )
    }

    /// Test seam for an explicitly confirmed destructive conflict deletion.
    /// Production always passes `true` only after the native confirmation alert.
    static func executeScenarioConflictDelete(
        id: String, registryPath: String, jobsPath: String, consent: Bool
    ) throws {
        let jobID = "scenario-conflict-delete-" + UUID().uuidString.lowercased()
        let pointer = jobsPath.withCString { jobs in registryPath.withCString { registry in
            id.withCString { id in jobID.withCString { jobID in
                runLocalScenarioConflictDeleteJob(jobs, registry, id, jobID, consent)
            }}
        }}
        let _: ScenarioJobStatus = try unwrap(
            pointer, as: ScenarioJobStatus.self, databasePath: registryPath
        )
    }

    /// Submit one approved non-destructive `create` proposal. The Core reads
    /// only the required source and validates the immutable Scenario revision
    /// and endpoint direction before it creates the durable copy Job.
    static func executeScenarioCreate(
        scenario: ScenarioEntry, item: ScenarioPlanItem
    ) throws {
        let request = try scenarioCreateRequest(scenario: scenario, item: item)
        _ = try WorkerIPC.executeScenarioCreate(request)
    }

    static func executeScenarioCreateBatch(scenario: ScenarioEntry, items: [ScenarioPlanItem]) throws {
        let children = try items.map { item -> ScenarioCreateBatchItem in
            guard item.action.kind == "create", let sourceIsLeft = item.action.from_left else {
                throw StorageError.operation("План не разрешает создание этого файла")
            }
            return ScenarioCreateBatchItem(relative_path: item.relative_path, source_is_left: sourceIsLeft)
        }
        guard !children.isEmpty else { throw StorageError.operation("Пакет не содержит файлов для создания") }
        let payload = try JSONEncoder().encode(children)
        guard let payloadString = String(data: payload, encoding: .utf8) else { throw StorageError.operation("Cannot encode Scenario batch") }
        let database = try path
        let receipt: ScenarioCreateBatchReceipt = try database.withCString { path in
            try scenario.id.withCString { id in try payloadString.withCString { items in
                try unwrap(scenarioCreateBatch(path, id, scenario.revision, items), as: ScenarioCreateBatchReceipt.self, databasePath: database)
            }}
        }
        _ = try WorkerIPC.executeScenarioCreateBatch(receiptID: receipt.id)
    }

    /// Test seam for the Rust-owned execution bridge. Production uses the
    /// app-owned Registry and JobStore paths above.
    static func executeScenarioCreate(
        scenario: ScenarioEntry, item: ScenarioPlanItem,
        registryPath: String, jobsPath: String
    ) throws {
        let request = try scenarioCreateRequest(scenario: scenario, item: item)
        let pointer = jobsPath.withCString { jobs in registryPath.withCString { registry in scenario.id.withCString { scenarioID in
            request.source_root.withCString { sourceRoot in request.source_connection_id.withCString { sourceConnection in request.source_relative_path.withCString { sourceRelative in
                request.destination_root.withCString { destinationRoot in request.destination_connection_id.withCString { destinationConnection in request.destination_relative_path.withCString { destinationRelative in request.job_id.withCString { jobID in
                    runLocalScenarioCopyJob(jobs, registry, scenarioID, scenario.revision, sourceRoot, sourceConnection, sourceRelative, destinationRoot, destinationConnection, destinationRelative, jobID)
                }}}}}}}}
        }}
        let _: ScenarioJobStatus = try unwrap(
            pointer, as: ScenarioJobStatus.self, databasePath: registryPath
        )
    }

    private static func scenarioCreateRequest(
        scenario: ScenarioEntry, item: ScenarioPlanItem
    ) throws -> WorkerScenarioCreateRequest {
        guard item.action.kind == "create", let fromLeft = item.action.from_left else {
            throw StorageError.operation("План не разрешает создание этого файла")
        }
        return WorkerScenarioCreateRequest(
            scenario_id: scenario.id,
            scenario_revision: scenario.revision,
            source_root: fromLeft ? scenario.left_root : scenario.right_root,
            source_connection_id: fromLeft ? scenario.left_connection_id : scenario.right_connection_id,
            source_relative_path: item.relative_path,
            destination_root: fromLeft ? scenario.right_root : scenario.left_root,
            destination_connection_id: fromLeft ? scenario.right_connection_id : scenario.left_connection_id,
            destination_relative_path: item.relative_path,
            job_id: "scenario-copy-" + UUID().uuidString.lowercased()
        )
    }

    /// Submit one `update` proposal after explicit replacement confirmation.
    /// Rust binds the replacement to this immutable Scenario revision.
    static func executeScenarioUpdate(
        scenario: ScenarioEntry, item: ScenarioPlanItem
    ) throws {
        let request = try scenarioReplaceRequest(scenario: scenario, item: item, consent: true)
        _ = try WorkerIPC.executeScenarioReplace(request)
    }

    /// Test seam for a destructive Scenario update. Production passes `true`
    /// only after the native replacement confirmation alert.
    static func executeScenarioUpdate(
        scenario: ScenarioEntry, item: ScenarioPlanItem,
        registryPath: String, jobsPath: String, consent: Bool
    ) throws {
        let request = try scenarioReplaceRequest(scenario: scenario, item: item, consent: consent)
        let pointer = jobsPath.withCString { jobs in registryPath.withCString { registry in scenario.id.withCString { scenarioID in
            request.source_root.withCString { sourceRoot in request.source_connection_id.withCString { sourceConnection in request.source_relative_path.withCString { sourceRelative in
                request.destination_root.withCString { destinationRoot in request.destination_connection_id.withCString { destinationConnection in request.destination_relative_path.withCString { destinationRelative in request.job_id.withCString { jobID in
                    runLocalScenarioReplaceJob(jobs, registry, scenarioID, scenario.revision, sourceRoot, sourceConnection, sourceRelative, destinationRoot, destinationConnection, destinationRelative, jobID, request.replacement_consent)
                }}}}}}}}
        }}
        let _: ScenarioJobStatus = try unwrap(
            pointer, as: ScenarioJobStatus.self, databasePath: registryPath
        )
    }

    private static func scenarioReplaceRequest(
        scenario: ScenarioEntry, item: ScenarioPlanItem, consent: Bool
    ) throws -> WorkerScenarioReplaceRequest {
        guard item.action.kind == "update", let fromLeft = item.action.from_left else {
            throw StorageError.operation("План не разрешает обновление этого файла")
        }
        return WorkerScenarioReplaceRequest(
            scenario_id: scenario.id,
            scenario_revision: scenario.revision,
            source_root: fromLeft ? scenario.left_root : scenario.right_root,
            source_connection_id: fromLeft ? scenario.left_connection_id : scenario.right_connection_id,
            source_relative_path: item.relative_path,
            destination_root: fromLeft ? scenario.right_root : scenario.left_root,
            destination_connection_id: fromLeft ? scenario.right_connection_id : scenario.left_connection_id,
            destination_relative_path: item.relative_path,
            job_id: "scenario-replace-" + UUID().uuidString.lowercased(),
            replacement_consent: consent
        )
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
        if databasePath == nil {
            try WorkerIPC.updateRegistryControl(WorkerRegistryControlRequest(
                command: "set_scenario_paused", scenario_id: scenarioID, locally_paused: paused,
                conflict_id: nil, source_is_left: nil, decision: nil
            ))
            return
        }
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

    /// Stage and dispatch due schedule receipts into the existing JobStore.
    /// This creates only revision-pinned queued Jobs. It does not read adapter
    /// metadata or content, execute a Scenario, or grant transfer consent.
    static func dispatchDueScenarioSchedules(
        after: Date, through: Date, databasePath: String? = nil, jobStorePath: String? = nil
    ) throws -> (staged: Int, dispatched: Int) {
        let database = try databasePath ?? path
        let jobs = try jobStorePath ?? JobInventory.paths().database
        let formatter = ISO8601DateFormatter()
        let afterValue = formatter.string(from: after)
        let throughValue = formatter.string(from: through)
        let result: ScenarioScheduleDispatchStatus = try database.withCString { database in
            try jobs.withCString { jobs in
                try afterValue.withCString { lowerBound in
                    try throughValue.withCString { upperBound in
                        try unwrap(
                            scenarioSchedulesDispatch(database, jobs, lowerBound, upperBound),
                            as: ScenarioScheduleDispatchStatus.self
                        )
                    }
                }
            }
        }
        return (result.staged, result.dispatched)
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
