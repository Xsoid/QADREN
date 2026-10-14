import Foundation

@main
struct LocalWorkspaceTests {
    @MainActor
    static func main() throws {
        let root = URL(fileURLWithPath: FileManager.default.currentDirectoryPath)
            .appendingPathComponent(".qadren-local-workspace-test-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: root) }
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: false)

        let workspace = LocalWorkspace()
        workspace.selectRoot(root.path)
        let database = root.appendingPathComponent("jobs.sqlite3").path
        let registry = root.appendingPathComponent("connections.sqlite3").path
        let connection = try StorageRegistryBridge.addLocalComputer(
            name: "Test Local", databasePath: registry
        )
        try workspace.createDirectory(
            named: "created", connectionID: connection, databasePath: database, registryPath: registry
        )

        var isDirectory: ObjCBool = false
        let exists = FileManager.default.fileExists(
            atPath: root.appendingPathComponent("created").path,
            isDirectory: &isDirectory
        )
        let listed = workspace.entries.contains(where: { $0.name == "created" && $0.is_directory })
        guard exists, isDirectory.boolValue, listed else {
            throw NSError(domain: "QADRENTests", code: 1, userInfo: [
                NSLocalizedDescriptionKey: "local directory was not created and listed: exists=\(exists) directory=\(isDirectory.boolValue) listed=\(listed) workspaceError=\(workspace.error ?? "none")"
            ])
        }

        do {
            try workspace.createDirectory(
                named: "created", connectionID: connection, databasePath: database, registryPath: registry
            )
            throw NSError(domain: "QADRENTests", code: 2, userInfo: [NSLocalizedDescriptionKey: "existing directory was accepted"])
        } catch {
            guard FileManager.default.fileExists(atPath: root.appendingPathComponent("created").path) else {
                throw NSError(domain: "QADRENTests", code: 3, userInfo: [NSLocalizedDescriptionKey: "existing directory was changed"])
            }
        }
        print("LocalWorkspace native Swift/FFI controller tests passed")
    }
}
