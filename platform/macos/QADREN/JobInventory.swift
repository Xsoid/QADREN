import Foundation

@_silgen_name("qadren_jobs_list")
private func jobList(_ path: UnsafePointer<CChar>?) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_jobs_free_string")
private func jobFree(_ pointer: UnsafeMutablePointer<CChar>?)

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

/// Read-only representation: Rust owns JobStore and all execution transitions.
enum JobInventory {
    static func list() throws -> [JobRow] {
        let base = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("QADREN", isDirectory: true)
        try FileManager.default.createDirectory(
            at: base, withIntermediateDirectories: true,
            attributes: [.posixPermissions: NSNumber(value: 0o700)]
        )
        let path = base.appendingPathComponent("jobs.sqlite3").path
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
}
