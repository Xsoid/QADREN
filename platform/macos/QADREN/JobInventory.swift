import Foundation

@_silgen_name("qadren_jobs_list")
private func jobList(_ path: UnsafePointer<CChar>?) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_jobs_free_string")
private func jobFree(_ pointer: UnsafeMutablePointer<CChar>?)
@_silgen_name("qadren_jobs_run_mock_create")
private func runMockCreate(
    _ databasePath: UnsafePointer<CChar>?, _ storageRoot: UnsafePointer<CChar>?, _ jobID: UnsafePointer<CChar>?
) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_jobs_reconcile_mock_create")
private func reconcileMockCreate(
    _ databasePath: UnsafePointer<CChar>?, _ storageRoot: UnsafePointer<CChar>?, _ jobID: UnsafePointer<CChar>?
) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_jobs_queue_mock_create")
private func queueMockCreate(
    _ databasePath: UnsafePointer<CChar>?, _ jobID: UnsafePointer<CChar>?
) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_jobs_cancel_mock_create")
private func cancelMockCreate(
    _ databasePath: UnsafePointer<CChar>?, _ jobID: UnsafePointer<CChar>?
) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_jobs_pause_mock_create")
private func pauseMockCreate(
    _ databasePath: UnsafePointer<CChar>?, _ jobID: UnsafePointer<CChar>?
) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_jobs_resume_mock_create")
private func resumeMockCreate(
    _ databasePath: UnsafePointer<CChar>?, _ jobID: UnsafePointer<CChar>?
) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_jobs_connection_has_unresolved_attempts")
private func connectionHasUnresolvedAttempts(
    _ databasePath: UnsafePointer<CChar>?, _ connectionID: UnsafePointer<CChar>?
) -> UnsafeMutablePointer<CChar>?

struct JobRow: Decodable, Identifiable {
    let id: String
    let state: String
    let scenario_id: String?
    let scenario_revision: UInt64?
}

private struct JobEnvelope: Decodable {
    let ok: Bool
    let data: [JobRow]?
    let error: String?
}

private struct MockCreateEnvelope: Decodable {
    let ok: Bool
    let error: String?
}

private struct ConnectionAttemptData: Decodable {
    let unresolved: Bool
}

private struct ConnectionAttemptEnvelope: Decodable {
    let ok: Bool
    let data: ConnectionAttemptData?
    let error: String?
}

/// Read-only representation: Rust owns JobStore and all execution transitions.
enum JobInventory {
    static func paths() throws -> (database: String, mockStorage: String) {
        let base = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("QADREN", isDirectory: true)
        try FileManager.default.createDirectory(
            at: base, withIntermediateDirectories: true,
            attributes: [.posixPermissions: NSNumber(value: 0o700)]
        )
        return (base.appendingPathComponent("jobs.sqlite3").path, base.appendingPathComponent("G1MockStorage", isDirectory: true).path)
    }

    static func list() throws -> [JobRow] {
        let path = try paths().database
        let ptr = path.withCString { jobList($0) }
        guard let ptr else { throw StorageError.operation("Job inventory unavailable") }
        defer { jobFree(ptr) }
        let result = try JSONDecoder().decode(JobEnvelope.self, from: Data(String(cString: ptr).utf8))
        guard result.ok, let jobs = result.data else {
            throw StorageError.operation(result.error ?? "Job inventory failed")
        }
        try? FileManager.default.setAttributes(
            [.posixPermissions: NSNumber(value: 0o600)], ofItemAtPath: path
        )
        return jobs
    }

    /// Fails closed when the durable Job inventory cannot prove this local
    /// connection is clear. This check never retries or reconciles a Job.
    static func requireNoUnresolvedAttempts(connectionID: String) throws {
        let database = try paths().database
        let pointer = database.withCString { database in
            connectionID.withCString { identifier in
                connectionHasUnresolvedAttempts(database, identifier)
            }
        }
        guard let pointer else { throw StorageError.operation("Job safety check unavailable") }
        defer { jobFree(pointer) }
        let result = try JSONDecoder().decode(
            ConnectionAttemptEnvelope.self,
            from: Data(String(cString: pointer).utf8)
        )
        guard result.ok, let data = result.data else {
            throw StorageError.operation(result.error ?? "Job safety check failed")
        }
        guard !data.unresolved else {
            throw StorageError.operation(
                "Connection has unresolved Jobs; credentials were not touched"
            )
        }
    }

    private static func command(
        _ invoke: (UnsafePointer<CChar>?, UnsafePointer<CChar>?) -> UnsafeMutablePointer<CChar>?,
        jobID: String
    ) throws {
        let database = try paths().database
        let pointer = database.withCString { database in
            jobID.withCString { identifier in invoke(database, identifier) }
        }
        guard let pointer else { throw StorageError.operation("Mock Job command unavailable") }
        defer { jobFree(pointer) }
        let result = try JSONDecoder().decode(MockCreateEnvelope.self, from: Data(String(cString: pointer).utf8))
        guard result.ok else { throw StorageError.operation(result.error ?? "Mock Job failed") }
    }

    /// Queuing changes only QADREN's local SQLite metadata; it creates no file.
    static func queueMockArtifact() throws {
        try command(queueMockCreate, jobID: "g1-" + UUID().uuidString.lowercased())
    }

    /// The button invoking this method is the operation-scoped consent for an
    /// app-owned test artifact. No user or cloud content is read.
    static func runMockArtifact(jobID: String) throws {
        let paths = try paths()
        let pointer = paths.database.withCString { database in
            paths.mockStorage.withCString { storage in
                jobID.withCString { identifier in runMockCreate(database, storage, identifier) }
            }
        }
        guard let pointer else { throw StorageError.operation("Mock Job command unavailable") }
        defer { jobFree(pointer) }
        let result = try JSONDecoder().decode(MockCreateEnvelope.self, from: Data(String(cString: pointer).utf8))
        guard result.ok else { throw StorageError.operation(result.error ?? "Mock Job failed") }
    }

    /// Read-only recovery for an existing UNKNOWN app-owned mock artifact.
    /// Rust verifies the bound target and metadata; this never retries a write.
    static func reconcileMockArtifact(jobID: String) throws {
        let paths = try paths()
        let pointer = paths.database.withCString { database in
            paths.mockStorage.withCString { storage in
                jobID.withCString { identifier in reconcileMockCreate(database, storage, identifier) }
            }
        }
        guard let pointer else { throw StorageError.operation("Mock reconciliation unavailable") }
        defer { jobFree(pointer) }
        let result = try JSONDecoder().decode(MockCreateEnvelope.self, from: Data(String(cString: pointer).utf8))
        guard result.ok else {
            throw StorageError.operation(result.error ?? "Mock reconciliation failed")
        }
    }

    static func cancelMockArtifact(jobID: String) throws {
        try command(cancelMockCreate, jobID: jobID)
    }

    static func pauseMockArtifact(jobID: String) throws {
        try command(pauseMockCreate, jobID: jobID)
    }

    static func resumeMockArtifact(jobID: String) throws {
        try command(resumeMockCreate, jobID: jobID)
    }
}
