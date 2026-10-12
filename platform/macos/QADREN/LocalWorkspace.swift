import AppKit
import Foundation

@_silgen_name("qadren_local_list") private func localList(_ root: UnsafePointer<CChar>?, _ relative: UnsafePointer<CChar>?) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_local_free_string") private func localFree(_ pointer: UnsafeMutablePointer<CChar>?)

struct LocalWorkspaceEntry: Decodable, Identifiable { let name: String; let is_directory: Bool; let size: UInt64; var id: String { name } }
private struct LocalEnvelope<T: Decodable>: Decodable { let ok: Bool; let data: T?; let error: String? }

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
    private func call<T: Decodable>(_ operation: (UnsafePointer<CChar>?, UnsafePointer<CChar>?) -> UnsafeMutablePointer<CChar>?, root: String, relative: String) throws -> T {
        let pointer = root.withCString { root in relative.withCString { relative in operation(root, relative) } }
        guard let pointer else { throw StorageError.operation("Local filesystem unavailable") }; defer { localFree(pointer) }
        let envelope = try JSONDecoder().decode(LocalEnvelope<T>.self, from: Data(String(cString: pointer).utf8))
        guard envelope.ok, let data = envelope.data else { throw StorageError.operation(envelope.error ?? "Local filesystem request failed") }; return data
    }
}
