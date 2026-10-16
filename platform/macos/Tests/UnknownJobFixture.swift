import Foundation

@_silgen_name("qadren_jobs_seed_unknown_ui_fixture")
private func seedUnknownFixture(
    _ databasePath: UnsafePointer<CChar>?,
    _ storageRoot: UnsafePointer<CChar>?,
    _ jobID: UnsafePointer<CChar>?
) -> UnsafeMutablePointer<CChar>?

@_silgen_name("qadren_jobs_reconcile_mock_create")
private func reconcileUnknownFixture(
    _ databasePath: UnsafePointer<CChar>?,
    _ storageRoot: UnsafePointer<CChar>?,
    _ jobID: UnsafePointer<CChar>?
) -> UnsafeMutablePointer<CChar>?

@_silgen_name("qadren_jobs_free_string")
private func freeFixtureString(_ pointer: UnsafeMutablePointer<CChar>?)

@main
struct UnknownJobFixture {
    static func main() {
        let arguments = CommandLine.arguments
        guard arguments.count == 5,
              ["seed", "reconcile"].contains(arguments[1]) else {
            fputs("usage: UnknownJobFixture <seed|reconcile> <jobs.sqlite3> <storage-root> <job-id>\n", stderr)
            exit(64)
        }

        let result = arguments[2].withCString { databasePath in
            arguments[3].withCString { storageRoot in
                arguments[4].withCString { jobID in
                    if arguments[1] == "seed" {
                        return seedUnknownFixture(databasePath, storageRoot, jobID)
                    }
                    return reconcileUnknownFixture(databasePath, storageRoot, jobID)
                }
            }
        }
        guard let result else {
            fputs("fixture returned no response\n", stderr)
            exit(1)
        }
        defer { freeFixtureString(result) }
        let response = String(cString: result)
        let expectedStatus = arguments[1] == "seed" ? "unknown" : "completed"
        guard response.contains("\"ok\":true"), response.contains("\"status\":\"" + expectedStatus + "\"") else {
            fputs(response + "\n", stderr)
            exit(1)
        }
        print(response)
    }
}
