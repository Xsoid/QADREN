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

/// Run every deletion that the simple deletion rule of the current revision
/// covers. The Worker builds the plan itself, so it applies the selection and
/// the mass-deletion threshold. `batch_id` makes child Job IDs stable.
struct WorkerScenarioRuleDeletionsRequest: Encodable {
    let command = "execute_rule_deletions"
    let scenario_id: String
    let scenario_revision: UInt64
    let batch_id: String
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

struct WorkerStatusRequest: Encodable {
    let command = "status"
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

    /// Rule deletions still run inside the IPC request, so wait long enough
    /// for a large set instead of reporting a false failure after 5 s.
    static let batchAnswerTimeoutSeconds = 6 * 60 * 60

    /// Returns the number of objects that the rule deleted or moved to Trash.
    static func executeScenarioRuleDeletions(
        _ request: WorkerScenarioRuleDeletionsRequest, socketPath: String? = nil
    ) throws -> Int {
        let payload = try JSONEncoder().encode(request) + Data([0x0a])
        let response = try send(
            payload, socketPath: socketPath ?? defaultSocketPath(), timeoutSeconds: batchAnswerTimeoutSeconds
        )
        guard response.ok else { throw WorkerIPCError.rejected(response.status) }
        guard response.status == "completed", let done = response.dispatched else {
            throw WorkerIPCError.invalidResponse
        }
        return done
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
    static func probeStatus(socketPath: String? = nil, timeoutSeconds: Int = 1) throws {
        let payload = try JSONEncoder().encode(WorkerStatusRequest()) + Data([0x0a])
        let response = try send(
            payload, socketPath: socketPath ?? defaultSocketPath(), timeoutSeconds: timeoutSeconds
        )
        guard response.ok, response.status == "running" else { throw WorkerIPCError.invalidResponse }
    }

    /// Batch files that the Worker runs now, from the `status` request.
    static func activeItems(socketPath: String? = nil) throws -> Int {
        let payload = try JSONEncoder().encode(WorkerStatusRequest()) + Data([0x0a])
        let response = try send(payload, socketPath: socketPath ?? defaultSocketPath(), timeoutSeconds: 2)
        guard response.ok, response.status == "running" else { throw WorkerIPCError.invalidResponse }
        return response.active ?? 0
    }

    private static func send(_ payload: Data, socketPath: String, timeoutSeconds: Int = 5) throws -> Response {
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
        while response.count < 64 * 1024 {
            var byte: UInt8 = 0
            let count = Darwin.read(descriptor, &byte, 1)
            guard count > 0 else { break }
            if byte == 0x0a { break }
            response.append(byte)
        }
        guard !response.isEmpty, let decoded = try? JSONDecoder().decode(Response.self, from: response) else {
            throw WorkerIPCError.invalidResponse
        }
        return decoded
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
