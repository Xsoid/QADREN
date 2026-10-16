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

struct LocalWorkspaceEntry: Decodable, Identifiable { let name: String; let is_directory: Bool; let size: UInt64; var id: String { name } }
private struct LocalEnvelope<T: Decodable>: Decodable { let ok: Bool; let data: T?; let error: String? }
private struct LocalJobResult: Decodable { let status: String; let reconciled: Bool? }
private struct LocalJobEnvelope: Decodable { let ok: Bool; let data: LocalJobResult?; let error: String? }
private struct LocalReconciliationCandidate: Decodable { let job_id: String }

@MainActor final class LocalWorkspace: ObservableObject {
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

    func chooseRoot() {
        let panel = NSOpenPanel(); panel.canChooseFiles = false; panel.canChooseDirectories = true; panel.allowsMultipleSelection = false
        guard panel.runModal() == .OK, let url = panel.url else { return }
        selectRoot(url.path)
    }
    func selectRoot(_ path: String) {
        root = path
        relativePath = ""
        selectedNames.removeAll()
        refresh()
    }
    var selectedDirectory: String? {
        guard let root else { return nil }
        return relativePath.isEmpty ? root : root + "/" + relativePath
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

    func findDirectoryReconciliations(connectionID: String, databasePath: String, registryPath: String) throws {
        guard let root else { return }
        guard !connectionID.isEmpty else {
            error = "Выберите локальное подключение"
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
