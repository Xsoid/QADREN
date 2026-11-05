import Darwin
import Foundation

struct WorkerScenarioCreateRequest: Encodable {
    let command = "execute_create"
    let scenario_id: String
    let scenario_revision: UInt64
    let source_root: String
    let source_connection_id: String
    let source_relative_path: String
    let destination_root: String
    let destination_connection_id: String
    let destination_relative_path: String
    let job_id: String
    /// Yandex Disk source only (D-023). Never stored or logged.
    var access_token: String? = nil
}

struct WorkerScenarioCreateBatchRequest: Encodable {
    let command = "execute_create_batch"
    let batch_id: String
    /// Yandex Disk source only (D-023). Never stored or logged.
    var access_token: String? = nil
}

struct WorkerScenarioCreateBatchCancellationRequest: Encodable {
    let command = "cancel_create_batch"
    let batch_id: String
}

struct WorkerScenarioReplaceRequest: Encodable {
    let command = "execute_replace"
    let scenario_id: String
    let scenario_revision: UInt64
    let source_root: String
    let source_connection_id: String
    let source_relative_path: String
    let destination_root: String
    let destination_connection_id: String
    let destination_relative_path: String
    let job_id: String
    let replacement_consent: Bool
    /// Yandex Disk source only (D-023). Never stored or logged.
    var access_token: String? = nil
    /// Metadata that the user saw in the plan. The Worker refuses the
    /// replacement if either file changed after planning.
    var expected_source: WorkerExpectedFileVersion?
    var expected_destination: WorkerExpectedFileVersion?
}

struct WorkerExpectedFileVersion: Encodable {
    let object_revision: String?
    let size: UInt64
}

struct WorkerManualCopyRequest: Encodable {
    let command: String
    let source_root: String
    let source_connection_id: String
    let source_relative_path: String
    let destination_root: String
    let destination_connection_id: String
    let destination_relative_path: String
    let job_id: String
    let replacement_consent: Bool?
}

struct WorkerConflictRequest: Encodable {
    let command: String
    let conflict_id: String
    let job_id: String
    let destructive_consent: Bool
}

/// A walk of one Scenario side (D-024 stage 3).
struct WorkerSnapshotInfo: Decodable, Equatable {
    let state: String
    let files: Int
    let folders_done: Int
    let folders_pending: Int
    let last_error: String?
    let retry_at: Int64?
}

struct WorkerPlanCount: Decodable, Equatable {
    let count: Int
    let bytes: Int64
}

/// A stored plan (D-024 stage 4).
struct WorkerPlanInfo: Decodable, Equatable {
    let id: String
    let state: String
    var finished_at: Int64? = nil
    let error: String?
    let counts: [String: WorkerPlanCount]?
}

struct WorkerPlanStatus: Decodable {
    let plan: WorkerPlanInfo?
    let left: WorkerSnapshotInfo?
    let right: WorkerSnapshotInfo?
}

/// One stored plan row with the file metadata of both sides.
struct WorkerPlanRow: Decodable {
    let relative_path: String
    let action: ScenarioPlanAction
    let left: String
    let right: String
    let left_file: ScenarioSnapshotItem?
    let right_file: ScenarioSnapshotItem?

    var item: ScenarioPlanItem {
        ScenarioPlanItem(relative_path: relative_path, left: left, right: right, action: action)
    }
}

/// `status`, or `prepare_restart` / `cancel_restart`, which answer like it.
struct WorkerStatusRequest: Encodable {
    var command = "status"
}

/// The answer to a `status` request.
struct WorkerStatus: Equatable {
    /// Batch files that the Worker runs now (D-024).
    let active: Int
    /// True while a restart waits: the Worker starts no new file.
    let draining: Bool
    /// The Unix time when the Worker process started; nil for a Worker of an
    /// earlier build.
    let started: Int64?
}

struct WorkerRegistryControlRequest: Encodable {
    let command: String
    let scenario_id: String?
    let locally_paused: Bool?
    let conflict_id: String?
    let source_is_left: Bool?
    let decision: String?
}

enum WorkerIPCError: Error, LocalizedError {
    case unavailable
    case invalidResponse
    case rejected(String)

    var errorDescription: String? {
        switch self {
        case .unavailable: return "Local Worker is unavailable"
        case .invalidResponse: return "Local Worker returned an invalid response"
        case let .rejected(status):
            let text = QADRENUserMessage.text(for: status)
            return text == status ? "Local Worker rejected the operation: \(status)" : text
        }
    }
}

enum WorkerIPC {
    private struct Response: Decodable {
        let ok: Bool
        let status: String
        /// `status` only: batch files that run now (D-024).
        var active: Int?
        var draining: Bool?
        var started: Int64?
        var dispatched: Int?
    }

    static func defaultSocketPath() throws -> String {
        try QADRENDataLocation.baseDirectory().appendingPathComponent("worker.sock").path
    }

    static func executeScenarioCreate(
        _ request: WorkerScenarioCreateRequest, socketPath: String? = nil
    ) throws -> String {
        let payload = try JSONEncoder().encode(request) + Data([0x0a])
        let response = try send(payload, socketPath: socketPath ?? defaultSocketPath())
        guard response.ok else { throw WorkerIPCError.rejected(response.status) }
        guard ["completed", "existing"].contains(response.status) else {
            throw WorkerIPCError.invalidResponse
        }
        return response.status
    }

    /// The Worker owns a confirmed batch receipt. The controller only sends
    /// its ID, so closing the settings window cannot cancel selected work.
    static func executeScenarioCreateBatch(
        receiptID: String, accessToken: String? = nil, socketPath: String? = nil
    ) throws -> String {
        let payload = try JSONEncoder().encode(
            WorkerScenarioCreateBatchRequest(batch_id: receiptID, access_token: accessToken)
        ) + Data([0x0a])
        // The Worker runs the batch on its run thread and answers at once
        // (D-024). The UI reads progress from the durable receipt.
        let response = try send(payload, socketPath: socketPath ?? defaultSocketPath())
        guard response.ok else { throw WorkerIPCError.rejected(response.status) }
        guard response.status == "accepted" else { throw WorkerIPCError.invalidResponse }
        return response.status
    }

    /// Give the Worker the Yandex Disk token of one connection. The Worker
    /// keeps it in memory only, so a run waits after a Worker restart until
    /// the app sends it again (D-024, stage A).
    static func provideAccessToken(connectionID: String, token: String, socketPath: String? = nil) throws {
        struct Request: Encodable {
            let command = "provide_access_token"
            let connection_id: String
            let access_token: String
        }
        let payload = try JSONEncoder().encode(Request(connection_id: connectionID, access_token: token)) + Data([0x0a])
        let response = try send(payload, socketPath: socketPath ?? defaultSocketPath(), timeoutSeconds: 2)
        guard response.ok, response.status == "updated" else { throw WorkerIPCError.invalidResponse }
    }

    /// Give the Worker the SMB login of one connection, so that it can mount
    /// the share without the app (issue #32). It stays in Worker memory only.
    static func provideSMBCredentials(
        connectionID: String, login: String, password: String, socketPath: String? = nil
    ) throws {
        struct Request: Encodable {
            let command = "provide_smb_credentials"
            let connection_id: String
            let login: String
            let password: String
        }
        let payload = try JSONEncoder().encode(
            Request(connection_id: connectionID, login: login, password: password)
        ) + Data([0x0a])
        let response = try send(payload, socketPath: socketPath ?? defaultSocketPath(), timeoutSeconds: 2)
        guard response.ok, response.status == "updated" else { throw WorkerIPCError.invalidResponse }
    }

    /// The ISPmanager login of one connection for SFTP reads (D-027). The
    /// Worker keeps it in memory only.
    static func provideISPmanagerCredentials(
        connectionID: String, login: String, password: String, socketPath: String? = nil
    ) throws {
        struct Request: Encodable {
            let command = "provide_ispmanager_credentials"
            let connection_id: String
            let login: String
            let password: String
        }
        let payload = try JSONEncoder().encode(
            Request(connection_id: connectionID, login: login, password: password)
        ) + Data([0x0a])
        let response = try send(payload, socketPath: socketPath ?? defaultSocketPath(), timeoutSeconds: 2)
        guard response.ok, response.status == "updated" else { throw WorkerIPCError.invalidResponse }
    }

    static func cancelScenarioCreateBatch(receiptID: String, socketPath: String? = nil) throws {
        let payload = try JSONEncoder().encode(
            WorkerScenarioCreateBatchCancellationRequest(batch_id: receiptID)
        ) + Data([0x0a])
        let response = try send(payload, socketPath: socketPath ?? defaultSocketPath())
        guard response.ok, response.status == "cancellation_requested" else {
            throw WorkerIPCError.rejected(response.status)
        }
    }

    static func executeScenarioReplace(
        _ request: WorkerScenarioReplaceRequest, socketPath: String? = nil
    ) throws -> String {
        let payload = try JSONEncoder().encode(request) + Data([0x0a])
        let response = try send(payload, socketPath: socketPath ?? defaultSocketPath())
        guard response.ok else { throw WorkerIPCError.rejected(response.status) }
        guard ["completed", "existing"].contains(response.status) else {
            throw WorkerIPCError.invalidResponse
        }
        return response.status
    }

    static func executeManualCopy(
        _ request: WorkerManualCopyRequest, socketPath: String? = nil
    ) throws -> String {
        let payload = try JSONEncoder().encode(request) + Data([0x0a])
        let response = try send(payload, socketPath: socketPath ?? defaultSocketPath())
        guard response.ok else { throw WorkerIPCError.rejected(response.status) }
        guard ["completed", "existing"].contains(response.status) else {
            throw WorkerIPCError.invalidResponse
        }
        return response.status
    }

    static func executeManualDirectoryCreate(
        _ request: WorkerManualCopyRequest, socketPath: String? = nil
    ) throws -> String {
        try executeManualCopy(request, socketPath: socketPath)
    }

    static func reconcileManual(_ request: WorkerManualCopyRequest, socketPath: String? = nil) throws -> String {
        let payload = try JSONEncoder().encode(request) + Data([0x0a])
        let response = try send(payload, socketPath: socketPath ?? defaultSocketPath())
        guard response.ok, ["completed", "unknown"].contains(response.status) else {
            throw response.ok ? WorkerIPCError.invalidResponse : WorkerIPCError.rejected(response.status)
        }
        return response.status
    }

    static func reconcileLocalReconnect(root: String, connectionID: String, socketPath: String? = nil) throws -> String {
        try reconcileManual(WorkerManualCopyRequest(command: "reconcile_local_reconnect", source_root: root, source_connection_id: connectionID, source_relative_path: "", destination_root: "", destination_connection_id: "", destination_relative_path: "", job_id: "reconnect-reconciliation", replacement_consent: nil), socketPath: socketPath)
    }

    static func executeConflict(_ request: WorkerConflictRequest, socketPath: String? = nil) throws -> String {
        let payload = try JSONEncoder().encode(request) + Data([0x0a])
        let response = try send(payload, socketPath: socketPath ?? defaultSocketPath())
        guard response.ok else { throw WorkerIPCError.rejected(response.status) }
        guard ["completed", "existing"].contains(response.status) else {
            throw WorkerIPCError.invalidResponse
        }
        return response.status
    }

    static func updateRegistryControl(_ request: WorkerRegistryControlRequest) throws {
        let payload = try JSONEncoder().encode(request) + Data([0x0a])
        let response = try send(payload, socketPath: defaultSocketPath())
        guard response.ok, response.status == "updated" else {
            throw response.ok ? WorkerIPCError.invalidResponse : WorkerIPCError.rejected(response.status)
        }
    }

    /// Sends the non-mutating `status` request. A short timeout keeps the
    /// monitoring refresh responsive while the Worker runs a long effect.
    /// Returns the number of batch files that the Worker runs now (D-024).
    @discardableResult
    static func probeStatus(socketPath: String? = nil, timeoutSeconds: Int = 1) throws -> Int {
        let payload = try JSONEncoder().encode(WorkerStatusRequest()) + Data([0x0a])
        let response = try send(
            payload, socketPath: socketPath ?? defaultSocketPath(), timeoutSeconds: timeoutSeconds
        )
        guard response.ok, response.status == "running" else { throw WorkerIPCError.invalidResponse }
        return response.active ?? 0
    }

    /// Batch files that the Worker runs now, from the `status` request.
    static func activeItems(socketPath: String? = nil) throws -> Int {
        try status(socketPath: socketPath).active
    }

    static func status(command: String = "status", socketPath: String? = nil) throws -> WorkerStatus {
        let payload = try JSONEncoder().encode(WorkerStatusRequest(command: command)) + Data([0x0a])
        let response = try send(payload, socketPath: socketPath ?? defaultSocketPath(), timeoutSeconds: 2)
        guard response.ok, response.status == "running" else { throw WorkerIPCError.invalidResponse }
        return WorkerStatus(active: response.active ?? 0, draining: response.draining ?? false, started: response.started)
    }

    /// Asks the Worker to finish the files that run now and to start no new
    /// one for at most 10 minutes, so that it can restart between files.
    /// A Worker of an earlier build refuses the command (`invalidResponse`).
    @discardableResult
    static func prepareRestart() throws -> WorkerStatus {
        try status(command: "prepare_restart")
    }

    /// Lets the Worker start new files again after an abandoned restart.
    static func cancelRestart() {
        _ = try? status(command: "cancel_restart")
    }

    private static func send(_ payload: Data, socketPath: String, timeoutSeconds: Int = 5) throws -> Response {
        let response = try sendRaw(payload, socketPath: socketPath, timeoutSeconds: timeoutSeconds, limit: 64 * 1024)
        guard let decoded = try? JSONDecoder().decode(Response.self, from: response) else {
            throw WorkerIPCError.invalidResponse
        }
        return decoded
    }

    /// Send one request line and read one answer line of at most `limit`
    /// bytes. A plan page can be larger than a status answer (D-024).
    private static func sendRaw(
        _ payload: Data, socketPath: String, timeoutSeconds: Int, limit: Int
    ) throws -> Data {
        let descriptor = Darwin.socket(AF_UNIX, SOCK_STREAM, 0)
        guard descriptor >= 0 else { throw WorkerIPCError.unavailable }
        defer { Darwin.close(descriptor) }

        var timeout = timeval(tv_sec: timeoutSeconds, tv_usec: 0)
        guard setsockopt(
            descriptor, SOL_SOCKET, SO_RCVTIMEO, &timeout, socklen_t(MemoryLayout<timeval>.size)
        ) == 0,
        setsockopt(
            descriptor, SOL_SOCKET, SO_SNDTIMEO, &timeout, socklen_t(MemoryLayout<timeval>.size)
        ) == 0 else { throw WorkerIPCError.unavailable }

        var address = sockaddr_un()
        address.sun_len = UInt8(MemoryLayout<sockaddr_un>.size)
        address.sun_family = sa_family_t(AF_UNIX)
        let bytes = Array(socketPath.utf8)
        guard bytes.count < MemoryLayout.size(ofValue: address.sun_path) else {
            throw WorkerIPCError.unavailable
        }
        withUnsafeMutableBytes(of: &address.sun_path) { destination in
            destination.copyBytes(from: bytes)
        }
        let connected = withUnsafePointer(to: &address) { pointer in
            pointer.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                Darwin.connect(descriptor, $0, socklen_t(MemoryLayout<sockaddr_un>.size))
            }
        }
        guard connected == 0 else { throw WorkerIPCError.unavailable }

        try writeAll(payload, descriptor: descriptor)
        var response = Data()
        var buffer = [UInt8](repeating: 0, count: 64 * 1024)
        while response.count < limit {
            let count = Darwin.read(descriptor, &buffer, buffer.count)
            guard count > 0 else { break }
            response.append(contentsOf: buffer[0..<count])
            if buffer[count - 1] == 0x0a { break }
        }
        if response.last == 0x0a { response.removeLast() }
        guard !response.isEmpty else { throw WorkerIPCError.invalidResponse }
        return response
    }

    private struct PlanAnswerStatus: Decodable { let ok: Bool; let status: String }

    /// A request to the plan commands of the Worker (D-024 stage 4).
    private struct PlanRequest: Encodable {
        let command: String
        var scenario_id: String? = nil
        var scenario_revision: UInt64? = nil
        var plan_id: String? = nil
        var action: String? = nil
        var search: String? = nil
        var offset: Int? = nil
        var limit: Int? = nil
        var access_token: String? = nil
    }

    private static func plan<T: Decodable>(_ request: PlanRequest, as type: T.Type) throws -> T {
        let payload = try JSONEncoder().encode(request) + Data([0x0a])
        let data = try sendRaw(payload, socketPath: defaultSocketPath(), timeoutSeconds: 30, limit: 16 * 1024 * 1024)
        if let status = try? JSONDecoder().decode(PlanAnswerStatus.self, from: data), !status.ok {
            throw WorkerIPCError.rejected(status.status)
        }
        guard let decoded = try? JSONDecoder().decode(T.self, from: data) else {
            throw WorkerIPCError.invalidResponse
        }
        return decoded
    }

    /// Start the snapshots and the plan of a Scenario revision.
    static func startPlan(scenarioID: String, revision: UInt64, accessToken: String?) throws -> String {
        struct Answer: Decodable { let plan: String }
        return try plan(
            PlanRequest(command: "start_plan", scenario_id: scenarioID, scenario_revision: revision, access_token: accessToken),
            as: Answer.self
        ).plan
    }

    static func planStatus(scenarioID: String, revision: UInt64, planID: String? = nil) throws -> WorkerPlanStatus {
        try plan(
            PlanRequest(command: "plan_status", scenario_id: scenarioID, scenario_revision: revision, plan_id: planID),
            as: WorkerPlanStatus.self
        )
    }

    /// The database step of the active run of one Scenario: the cleanup of
    /// `files/`, the dump, or the publish (D-027 section 7b). Nil when the
    /// run has no database step now.
    static func databaseProgress(scenarioID: String) throws -> WorkerDatabaseProgress? {
        struct Answer: Decodable { let progress: WorkerDatabaseProgress? }
        return try plan(PlanRequest(command: "database_progress", scenario_id: scenarioID), as: Answer.self).progress
    }

    static func planPage(planID: String, action: String?, search: String?, offset: Int, limit: Int) throws -> [WorkerPlanRow] {
        struct Answer: Decodable { let items: [WorkerPlanRow] }
        return try plan(
            PlanRequest(command: "plan_page", plan_id: planID, action: action, search: search, offset: offset, limit: limit),
            as: Answer.self
        ).items
    }

    /// "Остановить": stop the active run of a Scenario. Completed items stay.
    static func stopRun(scenarioID: String) throws {
        struct Answer: Decodable { let status: String }
        _ = try plan(PlanRequest(command: "stop_run", scenario_id: scenarioID), as: Answer.self)
    }

    /// "Запустить" of the Scenario widget: a manual run that plans and
    /// executes like a scheduled run. During an active run it starts nothing.
    static func runNow(scenarioID: String, accessToken: String?) throws {
        struct Answer: Decodable { let status: String }
        _ = try plan(PlanRequest(command: "run_now", scenario_id: scenarioID, access_token: accessToken), as: Answer.self)
    }

    /// Make one Create batch of all Create rows of a stored plan.
    static func executePlanCreates(planID: String, accessToken: String?) throws -> (batch: String, files: Int) {
        struct Answer: Decodable { let batch: String; let files: Int }
        let answer = try plan(PlanRequest(command: "execute_plan_creates", plan_id: planID, access_token: accessToken), as: Answer.self)
        return (answer.batch, answer.files)
    }

    private static func writeAll(_ data: Data, descriptor: Int32) throws {
        try data.withUnsafeBytes { bytes in
            guard let start = bytes.baseAddress else { return }
            var written = 0
            while written < bytes.count {
                let count = Darwin.write(descriptor, start.advanced(by: written), bytes.count - written)
                guard count > 0 else { throw WorkerIPCError.unavailable }
                written += count
            }
        }
    }
}

/// What the database step of a run does now.
struct WorkerDatabaseProgress: Decodable, Equatable {
    let run_id: String
    /// `cleanup`, `dump`, or `publish`.
    let stage: String
    /// The current database, from 1; 0 during the cleanup.
    let index: Int
    let total: Int
    /// SQL bytes of the current dump so far.
    let bytes: UInt64
}
