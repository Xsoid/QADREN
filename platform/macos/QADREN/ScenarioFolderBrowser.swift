import Foundation

@_silgen_name("qadren_local_list")
private func browserLocalList(_ root: UnsafePointer<CChar>?, _ relative: UnsafePointer<CChar>?) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_local_free_string")
private func browserLocalFree(_ pointer: UnsafeMutablePointer<CChar>?)

/// The selection and deletion rule that the Scenario editor saves with a new
/// revision (D-019).
struct ScenarioScopeDraft: Equatable {
    /// Relative paths under the source root. Empty means the full root.
    var selectedPaths: Set<String> = []
    var deletionMode: ScenarioDeletionMode = .paranoid
}

/// One directory level of the local disk for the Scenario editor. The Core
/// lists names, types, and sizes from the pinned "/" one level at a time. It
/// skips symbolic links and special files and never opens file content.
@MainActor
final class ScenarioFolderBrowser: ObservableObject {
    struct Crumb: Identifiable {
        let title: String
        let path: String
        var id: String { path }
    }

    private struct Envelope: Decodable {
        let ok: Bool
        let data: [LocalWorkspaceEntry]?
        let error: String?
    }

    /// Absolute path of the visible directory.
    @Published private(set) var path: String
    @Published private(set) var entries: [LocalWorkspaceEntry] = []
    @Published private(set) var error: String?

    init(path: String = NSHomeDirectory()) {
        self.path = path
    }

    var crumbs: [Crumb] {
        var result = [Crumb(title: "Этот Mac", path: "/")]
        var current = ""
        for part in path.split(separator: "/") {
            current += "/" + part
            result.append(Crumb(title: String(part), path: current))
        }
        return result
    }

    var canGoUp: Bool { path != "/" }

    func open(_ newPath: String) {
        path = newPath.isEmpty ? "/" : newPath
        refresh()
    }

    func openChild(_ name: String) {
        open(Self.join(path, name))
    }

    func up() {
        guard canGoUp else { return }
        open((path as NSString).deletingLastPathComponent)
    }

    func refresh() {
        let relative = String(path.drop { $0 == "/" })
        let pointer = "/".withCString { root in
            relative.withCString { relative in browserLocalList(root, relative) }
        }
        guard let pointer else {
            entries = []
            error = QADRENUserMessage.text(for: "local_metadata_unavailable")
            return
        }
        defer { browserLocalFree(pointer) }
        guard let envelope = try? JSONDecoder().decode(Envelope.self, from: Data(String(cString: pointer).utf8)),
              envelope.ok, let data = envelope.data
        else {
            entries = []
            error = QADRENUserMessage.text(for: "local_metadata_unavailable")
            return
        }
        // Hidden entries are system and tool data; a Scenario selects user
        // folders. The Core still synchronizes hidden files inside a root.
        entries = data
            .filter { !$0.name.hasPrefix(".") }
            .sorted {
                $0.is_directory != $1.is_directory
                    ? $0.is_directory
                    : $0.name.localizedStandardCompare($1.name) == .orderedAscending
            }
        error = nil
    }

    static func join(_ directory: String, _ name: String) -> String {
        directory == "/" ? "/" + name : directory + "/" + name
    }

    /// The path of `absolute` relative to `root`, or nil when it is not
    /// inside `root`. The root itself is not a selectable object.
    static func relative(_ absolute: String, to root: String) -> String? {
        guard !root.isEmpty else { return nil }
        let prefix = root == "/" ? "/" : root + "/"
        guard absolute.hasPrefix(prefix), absolute.count > prefix.count else { return nil }
        return String(absolute.dropFirst(prefix.count))
    }

    /// True when a selected ancestor directory already includes `path`.
    static func isImplied(_ path: String, by selection: Set<String>) -> Bool {
        selection.contains { selected in path.hasPrefix(selected + "/") }
    }
}
