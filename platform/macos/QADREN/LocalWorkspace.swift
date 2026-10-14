import AppKit
import Foundation

@_silgen_name("qadren_local_list") private func localList(_ root: UnsafePointer<CChar>?, _ relative: UnsafePointer<CChar>?) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_local_free_string") private func localFree(_ pointer: UnsafeMutablePointer<CChar>?)
@_silgen_name("qadren_jobs_run_local_directory_create")
private func createLocalDirectoryJob(
    _ database: UnsafePointer<CChar>?, _ registry: UnsafePointer<CChar>?, _ root: UnsafePointer<CChar>?, _ connection: UnsafePointer<CChar>?,
    _ relative: UnsafePointer<CChar>?, _ jobID: UnsafePointer<CChar>?
) -> UnsafeMutablePointer<CChar>?

struct LocalWorkspaceEntry: Decodable, Identifiable { let name: String; let is_directory: Bool; let size: UInt64; var id: String { name } }
private struct LocalEnvelope<T: Decodable>: Decodable { let ok: Bool; let data: T?; let error: String? }
private struct LocalJobEnvelope: Decodable { let ok: Bool; let error: String? }

@MainActor final class LocalWorkspace: ObservableObject {
    @Published private(set) var root: String?
    @Published private(set) var relativePath = ""
    @Published private(set) var entries: [LocalWorkspaceEntry] = []
    @Published private(set) var error: String?

    func chooseRoot() {
        let panel = NSOpenPanel(); panel.canChooseFiles = false; panel.canChooseDirectories = true; panel.allowsMultipleSelection = false
        guard panel.runModal() == .OK, let url = panel.url else { return }
        selectRoot(url.path)
    }
    func selectRoot(_ path: String) { root = path; relativePath = ""; refresh() }
    var selectedDirectory: String? {
        guard let root else { return nil }
        return relativePath.isEmpty ? root : root + "/" + relativePath
    }
    func refresh() { guard let root else { return }; do { entries = try call(localList, root: root, relative: relativePath); error = nil } catch { self.error = error.localizedDescription } }
    func open(_ entry: LocalWorkspaceEntry) { guard entry.is_directory else { return }; relativePath = relativePath.isEmpty ? entry.name : relativePath + "/" + entry.name; refresh() }
    func up() { guard !relativePath.isEmpty else { return }; relativePath = relativePath.split(separator: "/").dropLast().joined(separator: "/"); refresh() }
    /// The button that calls this is the explicit, operation-scoped user action.
    /// Rust owns the Job attempt, no-overwrite mkdir and final verification.
    func createDirectory(named name: String, connectionID: String) {
        do {
            try createDirectory(
                named: name,
                connectionID: connectionID,
                databasePath: JobInventory.paths().database,
                registryPath: StorageRegistryBridge.databasePath()
            )
        } catch {
            self.error = error.localizedDescription
        }
    }

    /// Test seam for the native bridge. Production uses the app-owned JobStore
    /// selected above; callers cannot bypass the Rust executor.
    func createDirectory(named name: String, connectionID: String, databasePath: String, registryPath: String) throws {
        guard let root else { return }
        guard !connectionID.isEmpty else {
            error = "Выберите активное локальное подключение"
            return
        }
        let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty, !trimmed.contains("/") else {
            error = "Имя папки не должно быть пустым или содержать /"
            return
        }
        let target = relativePath.isEmpty ? trimmed : relativePath + "/" + trimmed
        do {
            let pointer = databasePath.withCString { database in
                registryPath.withCString { registry in
                    root.withCString { root in
                        connectionID.withCString { connection in
                            target.withCString { target in
                                ("local-mkdir-" + UUID().uuidString.lowercased()).withCString { jobID in
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
            guard result.ok else { throw StorageError.operation(result.error ?? "Local Job failed") }
            error = nil
            refresh()
        } catch {
            self.error = error.localizedDescription
            throw error
        }
    }
    private func call<T: Decodable>(_ operation: (UnsafePointer<CChar>?, UnsafePointer<CChar>?) -> UnsafeMutablePointer<CChar>?, root: String, relative: String) throws -> T {
        let pointer = root.withCString { root in relative.withCString { relative in operation(root, relative) } }
        guard let pointer else { throw StorageError.operation("Local filesystem unavailable") }; defer { localFree(pointer) }
        let envelope = try JSONDecoder().decode(LocalEnvelope<T>.self, from: Data(String(cString: pointer).utf8))
        guard envelope.ok, let data = envelope.data else { throw StorageError.operation(envelope.error ?? "Local filesystem request failed") }; return data
    }
}
