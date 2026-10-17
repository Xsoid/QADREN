import AppKit
import Foundation

@_silgen_name("qadren_local_list") private func localList(_ root: UnsafePointer<CChar>?, _ relative: UnsafePointer<CChar>?) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_local_free_string") private func localFree(_ pointer: UnsafeMutablePointer<CChar>?)
@_silgen_name("qadren_jobs_run_local_directory_create")
private func createLocalDirectoryJob(
    _ database: UnsafePointer<CChar>?, _ registry: UnsafePointer<CChar>?, _ root: UnsafePointer<CChar>?, _ connection: UnsafePointer<CChar>?,
    _ relative: UnsafePointer<CChar>?, _ jobID: UnsafePointer<CChar>?
) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_jobs_reconcile_local_directory_create")
private func reconcileLocalDirectoryJob(
    _ database: UnsafePointer<CChar>?, _ registry: UnsafePointer<CChar>?, _ root: UnsafePointer<CChar>?, _ connection: UnsafePointer<CChar>?,
    _ jobID: UnsafePointer<CChar>?
) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_jobs_list_local_directory_reconciliations")
private func listLocalDirectoryReconciliations(
    _ database: UnsafePointer<CChar>?, _ registry: UnsafePointer<CChar>?, _ root: UnsafePointer<CChar>?, _ connection: UnsafePointer<CChar>?
) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_jobs_list_local_file_copy_reconciliations")
private func listLocalFileCopyReconciliations(
    _ database: UnsafePointer<CChar>?, _ registry: UnsafePointer<CChar>?, _ root: UnsafePointer<CChar>?, _ connection: UnsafePointer<CChar>?
) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_local_copy_preview")
private func localCopyPreview(
    _ sourceRoot: UnsafePointer<CChar>?, _ sourceRelative: UnsafePointer<CChar>?,
    _ destinationRoot: UnsafePointer<CChar>?, _ destinationRelative: UnsafePointer<CChar>?
) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_jobs_run_local_file_copy")
private func runLocalFileCopyJob(
    _ database: UnsafePointer<CChar>?, _ registry: UnsafePointer<CChar>?,
    _ sourceRoot: UnsafePointer<CChar>?, _ sourceConnection: UnsafePointer<CChar>?, _ sourceRelative: UnsafePointer<CChar>?,
    _ destinationRoot: UnsafePointer<CChar>?, _ destinationConnection: UnsafePointer<CChar>?, _ destinationRelative: UnsafePointer<CChar>?,
    _ jobID: UnsafePointer<CChar>?, _ consent: Bool
) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_jobs_run_local_file_replace")
private func runLocalFileReplaceJob(
    _ database: UnsafePointer<CChar>?, _ registry: UnsafePointer<CChar>?,
    _ sourceRoot: UnsafePointer<CChar>?, _ sourceConnection: UnsafePointer<CChar>?, _ sourceRelative: UnsafePointer<CChar>?,
    _ destinationRoot: UnsafePointer<CChar>?, _ destinationConnection: UnsafePointer<CChar>?, _ destinationRelative: UnsafePointer<CChar>?,
    _ jobID: UnsafePointer<CChar>?, _ consent: Bool
) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_jobs_reconcile_local_file_copy")
private func reconcileLocalFileCopyJob(
    _ database: UnsafePointer<CChar>?, _ registry: UnsafePointer<CChar>?, _ root: UnsafePointer<CChar>?, _ connection: UnsafePointer<CChar>?,
    _ jobID: UnsafePointer<CChar>?
) -> UnsafeMutablePointer<CChar>?

struct LocalWorkspaceEntry: Decodable, Identifiable { let name: String; let is_directory: Bool; let size: UInt64; var id: String { name } }
private struct LocalEnvelope<T: Decodable>: Decodable { let ok: Bool; let data: T?; let error: String? }
private struct LocalJobResult: Decodable { let status: String; let reconciled: Bool? }
private struct LocalJobEnvelope: Decodable { let ok: Bool; let data: LocalJobResult?; let error: String? }
private struct LocalReconciliationCandidate: Decodable { let job_id: String }
struct LocalCopyPreview: Decodable, Equatable {
    let source_relative: String
    let destination_relative: String
    let size: UInt64
    let destination_exists: Bool
}

/// The exact metadata inputs used to produce a copy preview. A preview is not
/// an authorization and it is invalid as soon as the selected source or either
/// panel location changes. This prevents the UI from presenting an old plan as
/// the plan for a later selection.
struct LocalCopyPreviewRequest: Equatable {
    let sourceConnectionID: String
    let sourceRoot: String
    let sourceRelativePath: String
    let destinationConnectionID: String
    let destinationRoot: String
    let destinationDirectoryRelativePath: String
    let destinationRelativePath: String

    @MainActor func isCurrent(source: LocalWorkspace, destination: LocalWorkspace) -> Bool {
        source.boundConnectionID == sourceConnectionID
            && source.root == sourceRoot
            && source.selectedRegularFilePaths == [sourceRelativePath]
            && destination.boundConnectionID == destinationConnectionID
            && destination.root == destinationRoot
            && destination.relativePath == destinationDirectoryRelativePath
    }
}

@MainActor final class LocalWorkspace: ObservableObject {
    /// A root is valid only for the independently selected Local connection in
    /// this panel. Switching connections clears all path-derived state before
    /// any Job can use it.
    @Published private(set) var boundConnectionID: String?
    @Published private(set) var root: String?
    @Published private(set) var relativePath = ""
    @Published private(set) var entries: [LocalWorkspaceEntry] = []
    /// Selection belongs to the directory that is visible now. Navigation
    /// clears it so a name selected in one directory cannot appear selected
    /// in another directory with the same name.
    @Published private(set) var selectedNames: Set<String> = []
    @Published private(set) var error: String?
    /// This exists only after Rust reports a potentially effected mkdir as
    /// UNKNOWN. It is an explicit user action, never an automatic retry.
    @Published private(set) var reconciliationJobIDs: [String] = []
    @Published private(set) var copyReconciliationJobIDs: [String] = []

    func chooseRoot(for connectionID: String) {
        guard !connectionID.isEmpty else {
            error = "Выберите активное локальное подключение"
            return
        }
        let panel = NSOpenPanel(); panel.canChooseFiles = false; panel.canChooseDirectories = true; panel.allowsMultipleSelection = false
        guard panel.runModal() == .OK, let url = panel.url else { return }
        selectRoot(url.path, connectionID: connectionID)
    }
    /// Bind this panel to a connection before browsing its root. A different
    /// connection never inherits a path, selection, or UNKNOWN Job inventory.
    func bind(connectionID: String) {
        guard boundConnectionID != connectionID else { return }
        boundConnectionID = connectionID
        root = nil
        relativePath = ""
        entries = []
        selectedNames.removeAll()
        reconciliationJobIDs = []
        copyReconciliationJobIDs = []
        error = nil
    }
    func selectRoot(_ path: String, connectionID: String? = nil) {
        if let connectionID { bind(connectionID: connectionID) }
        root = path
        relativePath = ""
        selectedNames.removeAll()
        reconciliationJobIDs = []
        copyReconciliationJobIDs = []
        refresh()
    }
    var selectedDirectory: String? {
        guard let root else { return nil }
        return relativePath.isEmpty ? root : root + "/" + relativePath
    }
    /// Relative paths for selected regular files in the currently visible
    /// directory. This is metadata only; it never opens their content.
    var selectedRegularFilePaths: [String] {
        entries.compactMap { entry in
            guard selectedNames.contains(entry.name), !entry.is_directory else { return nil }
            return relativePath.isEmpty ? entry.name : relativePath + "/" + entry.name
        }
    }
    func refresh() {
        guard let root else { return }
        do {
            entries = try call(localList, root: root, relative: relativePath)
            selectedNames.formIntersection(Set(entries.map(\.name)))
            error = nil
        } catch {
            self.error = error.localizedDescription
        }
    }
    func open(_ entry: LocalWorkspaceEntry) {
        guard entry.is_directory else { return }
        relativePath = relativePath.isEmpty ? entry.name : relativePath + "/" + entry.name
        selectedNames.removeAll()
        refresh()
    }
    func up() {
        guard !relativePath.isEmpty else { return }
        relativePath = relativePath.split(separator: "/").dropLast().joined(separator: "/")
        selectedNames.removeAll()
        refresh()
    }
    func setSelected(_ name: String, isSelected: Bool) {
        guard entries.contains(where: { $0.name == name }) else { return }
        if isSelected {
            selectedNames.insert(name)
        } else {
            selectedNames.remove(name)
        }
    }
    /// The button that calls this is the explicit, operation-scoped user action.
    /// Rust owns the Job attempt, no-overwrite mkdir and final verification.
    @discardableResult func createDirectory(named name: String, connectionID: String) -> String? {
        do {
            return try createDirectory(
                named: name,
                connectionID: connectionID,
                databasePath: JobInventory.paths().database,
                registryPath: StorageRegistryBridge.databasePath()
            )
        } catch {
            self.error = error.localizedDescription
            return nil
        }
    }

    /// Test seam for the native bridge. Production uses the app-owned JobStore
    /// selected above; callers cannot bypass the Rust executor.
    @discardableResult func createDirectory(named name: String, connectionID: String, databasePath: String, registryPath: String) throws -> String? {
        guard let root else { return nil }
        guard !connectionID.isEmpty else {
            error = "Выберите активное локальное подключение"
            return nil
        }
        guard boundConnectionID == nil || boundConnectionID == connectionID else {
            error = "Выберите папку для текущего локального подключения"
            return nil
        }
        let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty, !trimmed.contains("/") else {
            error = "Имя папки не должно быть пустым или содержать /"
            return nil
        }
        let target = relativePath.isEmpty ? trimmed : relativePath + "/" + trimmed
        let jobID = "local-mkdir-" + UUID().uuidString.lowercased()
        do {
            let pointer = databasePath.withCString { database in
                registryPath.withCString { registry in
                    root.withCString { root in
                        connectionID.withCString { connection in
                            target.withCString { target in
                                jobID.withCString { jobID in
                                    createLocalDirectoryJob(database, registry, root, connection, target, jobID)
                                }
                            }
                        }
                    }
                }
            }
            guard let pointer else { throw StorageError.operation("Local Job unavailable") }
            defer { localFree(pointer) }
            let result = try JSONDecoder().decode(LocalJobEnvelope.self, from: Data(String(cString: pointer).utf8))
            guard result.ok else {
                if result.error == "outcome_unknown" {
                    reconciliationJobIDs = [jobID]
                    throw StorageError.operation(
                        "Результат создания папки не подтверждён. Проверьте операцию до повторной попытки."
                    )
                }
                throw StorageError.operation(result.error ?? "Local Job failed")
            }
            reconciliationJobIDs = []
            error = nil
            refresh()
            return jobID
        } catch {
            self.error = error.localizedDescription
            throw error
        }
    }

    /// Inspect the stored UNKNOWN mkdir attempt through Rust. This does not
    /// create a directory, repeat a request, or read file contents.
    func reconcileDirectoryCreate(jobID: String, connectionID: String) {
        do {
            try reconcileDirectoryCreate(
                jobID: jobID,
                connectionID: connectionID,
                databasePath: JobInventory.paths().database,
                registryPath: StorageRegistryBridge.databasePath()
            )
        } catch {
            self.error = error.localizedDescription
        }
    }

    func reconcileDirectoryCreate(jobID: String, connectionID: String, databasePath: String, registryPath: String) throws {
        guard let root else { return }
        guard !jobID.isEmpty, !connectionID.isEmpty else {
            error = "Выберите исходное локальное подключение"
            return
        }
        guard boundConnectionID == nil || boundConnectionID == connectionID else {
            error = "Выберите папку для текущего локального подключения"
            return
        }
        let pointer = databasePath.withCString { database in
            registryPath.withCString { registry in
                root.withCString { root in
                    connectionID.withCString { connection in
                        jobID.withCString { identifier in
                            reconcileLocalDirectoryJob(database, registry, root, connection, identifier)
                        }
                    }
                }
            }
        }
        guard let pointer else { throw StorageError.operation("Local reconciliation unavailable") }
        defer { localFree(pointer) }
        let result = try JSONDecoder().decode(LocalJobEnvelope.self, from: Data(String(cString: pointer).utf8))
        guard result.ok, let data = result.data else {
            throw StorageError.operation(result.error ?? "Local reconciliation failed")
        }
        if data.reconciled == true || data.status == "completed" {
            reconciliationJobIDs = []
            error = nil
            refresh()
        } else {
            reconciliationJobIDs = [jobID]
            error = "Результат операции всё ещё не подтверждён. Повтор не выполнен."
        }
    }

    /// Find only UNKNOWN local mkdir Jobs bound to this selected root and
    /// connection. This is an inventory request; it does not inspect files.
    func findDirectoryReconciliations(connectionID: String) {
        do {
            try findDirectoryReconciliations(
                connectionID: connectionID,
                databasePath: JobInventory.paths().database,
                registryPath: StorageRegistryBridge.databasePath()
            )
        } catch {
            self.error = error.localizedDescription
        }
    }

    /// Produce one read-only Local-to-Local candidate. It does not create a
    /// Job, request consent, reserve space, read bytes, or write a file.
    func previewCopy(
        sourceRelativePath: String, to destination: LocalWorkspace
    ) throws -> LocalCopyPreview {
        guard let root, let destinationRoot = destination.root else {
            throw StorageError.operation("Выберите папки в обеих панелях")
        }
        let pointer = root.withCString { sourceRoot in
            sourceRelativePath.withCString { sourceRelative in
                destinationRoot.withCString { destinationRoot in
                    destination.relativePath.withCString { destinationRelative in
                        localCopyPreview(sourceRoot, sourceRelative, destinationRoot, destinationRelative)
                    }
                }
            }
        }
        guard let pointer else { throw StorageError.operation("Local copy preview unavailable") }
        defer { localFree(pointer) }
        let result = try JSONDecoder().decode(
            LocalEnvelope<LocalCopyPreview>.self,
            from: Data(String(cString: pointer).utf8)
        )
        guard result.ok, let data = result.data else {
            throw StorageError.operation(
                "Предпросмотр недоступен: выберите разные непересекающиеся локальные папки."
            )
        }
        return data
    }

    /// Capture the exact panel state that a metadata copy preview describes.
    /// The caller must check this again before it displays or later uses the
    /// preview. This does not grant consent or reserve the destination.
    func copyPreviewRequest(
        sourceRelativePath: String, to destination: LocalWorkspace
    ) throws -> LocalCopyPreviewRequest {
        guard let sourceConnectionID = boundConnectionID,
              let sourceRoot = root,
              let destinationConnectionID = destination.boundConnectionID,
              let destinationRoot = destination.root,
              selectedRegularFilePaths == [sourceRelativePath]
        else {
            throw StorageError.operation("Предпросмотр устарел: выберите один файл и папки в обеих панелях")
        }
        let destinationLeaf = (sourceRelativePath as NSString).lastPathComponent
        let destinationRelativePath = destination.relativePath.isEmpty
            ? destinationLeaf
            : destination.relativePath + "/" + destinationLeaf
        return LocalCopyPreviewRequest(
            sourceConnectionID: sourceConnectionID,
            sourceRoot: sourceRoot,
            sourceRelativePath: sourceRelativePath,
            destinationConnectionID: destinationConnectionID,
            destinationRoot: destinationRoot,
            destinationDirectoryRelativePath: destination.relativePath,
            destinationRelativePath: destinationRelativePath
        )
    }

    /// Executes only after the confirmation dialog has explained that macOS can
    /// materialize the selected File Provider item. Rust persists and consumes
    /// the one-time decision before opening the source file.
    @discardableResult func copyFile(
        request: LocalCopyPreviewRequest, to destination: LocalWorkspace
    ) throws -> String {
        try copyFile(
            request: request,
            to: destination,
            databasePath: try JobInventory.paths().database,
            registryPath: try StorageRegistryBridge.databasePath()
        )
    }

    /// Test seam for the native bridge. Production obtains the app-owned paths
    /// above, while tests can use isolated stores.
    @discardableResult func copyFile(
        request: LocalCopyPreviewRequest, to destination: LocalWorkspace,
        databasePath: String, registryPath: String
    ) throws -> String {
        guard request.isCurrent(source: self, destination: destination) else {
            throw StorageError.operation("План копирования устарел. Проверьте его ещё раз.")
        }
        let jobID = "local-copy-" + UUID().uuidString.lowercased()
        let pointer = databasePath.withCString { database in
            registryPath.withCString { registry in
                request.sourceRoot.withCString { sourceRoot in
                    request.sourceConnectionID.withCString { sourceConnection in
                        request.sourceRelativePath.withCString { sourceRelative in
                            request.destinationRoot.withCString { destinationRoot in
                                request.destinationConnectionID.withCString { destinationConnection in
                                    request.destinationRelativePath.withCString { destinationRelative in
                                        jobID.withCString { identifier in
                                            runLocalFileCopyJob(database, registry, sourceRoot, sourceConnection, sourceRelative, destinationRoot, destinationConnection, destinationRelative, identifier, true)
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        guard let pointer else { throw StorageError.operation("Local copy Job unavailable") }
        defer { localFree(pointer) }
        let result = try JSONDecoder().decode(LocalJobEnvelope.self, from: Data(String(cString: pointer).utf8))
        guard result.ok else {
            if result.error == "outcome_unknown" {
                destination.copyReconciliationJobIDs = [jobID]
                throw StorageError.operation("Результат копирования не подтверждён. Проверьте файлы до повторной попытки.")
            }
            if result.error == "target_exists" {
                throw StorageError.operation("Файл с таким именем уже есть в папке назначения.")
            }
            throw StorageError.operation(result.error ?? "Local copy Job failed")
        }
        error = nil
        refresh()
        destination.refresh()
        return jobID
    }

    /// Replaces one existing regular destination file only after the same
    /// exact-source confirmation required for a new Local copy.
    @discardableResult func replaceFile(
        request: LocalCopyPreviewRequest, to destination: LocalWorkspace
    ) throws -> String {
        try replaceFile(
            request: request,
            to: destination,
            databasePath: try JobInventory.paths().database,
            registryPath: try StorageRegistryBridge.databasePath()
        )
    }

    /// Test seam for the controlled replacement bridge.
    @discardableResult func replaceFile(
        request: LocalCopyPreviewRequest, to destination: LocalWorkspace,
        databasePath: String, registryPath: String
    ) throws -> String {
        guard request.isCurrent(source: self, destination: destination) else {
            throw StorageError.operation("План замены устарел. Проверьте его ещё раз.")
        }
        let jobID = "local-replace-" + UUID().uuidString.lowercased()
        let pointer = databasePath.withCString { database in
            registryPath.withCString { registry in
                request.sourceRoot.withCString { sourceRoot in
                    request.sourceConnectionID.withCString { sourceConnection in
                        request.sourceRelativePath.withCString { sourceRelative in
                            request.destinationRoot.withCString { destinationRoot in
                                request.destinationConnectionID.withCString { destinationConnection in
                                    request.destinationRelativePath.withCString { destinationRelative in
                                        jobID.withCString { identifier in
                                            runLocalFileReplaceJob(database, registry, sourceRoot, sourceConnection, sourceRelative, destinationRoot, destinationConnection, destinationRelative, identifier, true)
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        guard let pointer else { throw StorageError.operation("Local replacement Job unavailable") }
        defer { localFree(pointer) }
        let result = try JSONDecoder().decode(LocalJobEnvelope.self, from: Data(String(cString: pointer).utf8))
        guard result.ok else {
            if result.error == "outcome_unknown" {
                destination.copyReconciliationJobIDs = [jobID]
                throw StorageError.operation("Результат замены не подтверждён. Проверьте файлы до повторной попытки.")
            }
            throw StorageError.operation(result.error ?? "Local replacement Job failed")
        }
        error = nil
        refresh()
        destination.refresh()
        return jobID
    }

    /// Inspect one UNKNOWN destination copy without reading content or retrying.
    func reconcileFileCopy(jobID: String, connectionID: String) {
        guard let root else { return }
        do {
            let database = try JobInventory.paths().database
            let registry = try StorageRegistryBridge.databasePath()
            let pointer = database.withCString { database in registry.withCString { registry in root.withCString { root in connectionID.withCString { connection in jobID.withCString { jobID in
                reconcileLocalFileCopyJob(database, registry, root, connection, jobID)
            }}}} }
            guard let pointer else { throw StorageError.operation("Local copy reconciliation unavailable") }
            defer { localFree(pointer) }
            let result = try JSONDecoder().decode(LocalJobEnvelope.self, from: Data(String(cString: pointer).utf8))
            guard result.ok, let data = result.data else { throw StorageError.operation(result.error ?? "Local copy reconciliation failed") }
            if data.reconciled == true || data.status == "completed" {
                copyReconciliationJobIDs = []
                error = nil
                refresh()
            } else {
                copyReconciliationJobIDs = [jobID]
                error = "Результат копирования всё ещё не подтверждён. Повтор не выполнен."
            }
        } catch { self.error = error.localizedDescription }
    }

    /// Find only UNKNOWN local-copy Jobs bound to this selected destination
    /// root and connection. This is an inventory request; it does not inspect
    /// files or repeat a copy.
    func findFileCopyReconciliations(connectionID: String) {
        guard let root else { return }
        guard !connectionID.isEmpty else {
            error = "Выберите локальное подключение"
            return
        }
        guard boundConnectionID == nil || boundConnectionID == connectionID else {
            error = "Выберите папку для текущего локального подключения"
            return
        }
        do {
            let databasePath = try JobInventory.paths().database
            let registryPath = try StorageRegistryBridge.databasePath()
            let pointer = databasePath.withCString { database in
                registryPath.withCString { registry in
                    root.withCString { root in
                        connectionID.withCString { connection in
                            listLocalFileCopyReconciliations(database, registry, root, connection)
                        }
                    }
                }
            }
            guard let pointer else { throw StorageError.operation("Local copy reconciliation inventory unavailable") }
            defer { localFree(pointer) }
            let result = try JSONDecoder().decode(
                LocalEnvelope<[LocalReconciliationCandidate]>.self,
                from: Data(String(cString: pointer).utf8)
            )
            guard result.ok, let candidates = result.data else {
                throw StorageError.operation(result.error ?? "Local copy reconciliation inventory failed")
            }
            copyReconciliationJobIDs = candidates.map(\.job_id)
            error = nil
        } catch {
            self.error = error.localizedDescription
        }
    }

    func findDirectoryReconciliations(connectionID: String, databasePath: String, registryPath: String) throws {
        guard let root else { return }
        guard !connectionID.isEmpty else {
            error = "Выберите локальное подключение"
            return
        }
        guard boundConnectionID == nil || boundConnectionID == connectionID else {
            error = "Выберите папку для текущего локального подключения"
            return
        }
        let pointer = databasePath.withCString { database in
            registryPath.withCString { registry in
                root.withCString { root in
                    connectionID.withCString { connection in
                        listLocalDirectoryReconciliations(database, registry, root, connection)
                    }
                }
            }
        }
        guard let pointer else { throw StorageError.operation("Local reconciliation inventory unavailable") }
        defer { localFree(pointer) }
        let result = try JSONDecoder().decode(
            LocalEnvelope<[LocalReconciliationCandidate]>.self,
            from: Data(String(cString: pointer).utf8)
        )
        guard result.ok, let candidates = result.data else {
            throw StorageError.operation(result.error ?? "Local reconciliation inventory failed")
        }
        reconciliationJobIDs = candidates.map(\.job_id)
        error = nil
    }
    private func call<T: Decodable>(_ operation: (UnsafePointer<CChar>?, UnsafePointer<CChar>?) -> UnsafeMutablePointer<CChar>?, root: String, relative: String) throws -> T {
        let pointer = root.withCString { root in relative.withCString { relative in operation(root, relative) } }
        guard let pointer else { throw StorageError.operation("Local filesystem unavailable") }; defer { localFree(pointer) }
        let envelope = try JSONDecoder().decode(LocalEnvelope<T>.self, from: Data(String(cString: pointer).utf8))
        guard envelope.ok, let data = envelope.data else { throw StorageError.operation(envelope.error ?? "Local filesystem request failed") }; return data
    }
}
