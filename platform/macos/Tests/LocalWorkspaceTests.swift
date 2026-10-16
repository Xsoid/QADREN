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
        let createdJobID = try workspace.createDirectory(
            named: "created", connectionID: connection, databasePath: database, registryPath: registry
        )
        guard let createdJobID else {
            throw NSError(domain: "QADRENTests", code: 4, userInfo: [
                NSLocalizedDescriptionKey: "local directory Job did not return an identifier"
            ])
        }

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

        guard let createdEntry = workspace.entries.first(where: { $0.name == "created" }) else {
            throw NSError(domain: "QADRENTests", code: 7, userInfo: [
                NSLocalizedDescriptionKey: "created directory is absent from the workspace selection source"
            ])
        }
        workspace.setSelected("created", isSelected: true)
        guard workspace.selectedNames == Set(["created"]) else {
            throw NSError(domain: "QADRENTests", code: 8, userInfo: [
                NSLocalizedDescriptionKey: "visible directory selection was not stored"
            ])
        }
        workspace.open(createdEntry)
        guard workspace.relativePath == "created", workspace.selectedNames.isEmpty else {
            throw NSError(domain: "QADRENTests", code: 9, userInfo: [
                NSLocalizedDescriptionKey: "directory navigation retained selection from its previous path"
            ])
        }
        workspace.up()
        guard workspace.relativePath.isEmpty, workspace.selectedNames.isEmpty else {
            throw NSError(domain: "QADRENTests", code: 10, userInfo: [
                NSLocalizedDescriptionKey: "up navigation retained selection from its previous path"
            ])
        }

        // The completed Job is not retried. This only exercises the same
        // narrow Swift-to-Rust reconciliation ABI used for UNKNOWN Jobs.
        try workspace.reconcileDirectoryCreate(
            jobID: createdJobID,
            connectionID: connection,
            databasePath: database,
            registryPath: registry
        )
        guard workspace.error == nil else {
            throw NSError(domain: "QADRENTests", code: 5, userInfo: [
                NSLocalizedDescriptionKey: "existing local Job reconciliation failed: \(workspace.error ?? "none")"
            ])
        }
        try workspace.findDirectoryReconciliations(
            connectionID: connection,
            databasePath: database,
            registryPath: registry
        )
        guard workspace.reconciliationJobIDs.isEmpty, workspace.error == nil else {
            throw NSError(domain: "QADRENTests", code: 6, userInfo: [
                NSLocalizedDescriptionKey: "completed local Job appeared in reconciliation inventory"
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
