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
        case let .rejected(status): return "Local Worker rejected the operation: \(status)"
        }
    }
}

enum WorkerIPC {
    private struct Response: Decodable {
        let ok: Bool
        let status: String
    }

    static func defaultSocketPath() throws -> String {
        let base = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("QADREN", isDirectory: true)
        try FileManager.default.createDirectory(
            at: base, withIntermediateDirectories: true,
            attributes: [.posixPermissions: NSNumber(value: 0o700)]
        )
        return base.appendingPathComponent("worker.sock").path
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

    private static func send(_ payload: Data, socketPath: String) throws -> Response {
        let descriptor = Darwin.socket(AF_UNIX, SOCK_STREAM, 0)
        guard descriptor >= 0 else { throw WorkerIPCError.unavailable }
        defer { Darwin.close(descriptor) }

        var timeout = timeval(tv_sec: 5, tv_usec: 0)
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
