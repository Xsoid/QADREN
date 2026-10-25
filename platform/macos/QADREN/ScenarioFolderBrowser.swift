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
    /// D-024 retry mode. It is outside the revision: a change of this value
    /// alone does not make a new revision.
    var retryUntilSuccess = true
}

/// One directory level of the local disk for the Scenario editor. The Core
/// lists names, types, and sizes from the pinned "/" one level at a time. It
/// skips symbolic links and special files and never opens file content.
/// For a Yandex Disk connection (D-023) the paths are `disk:/…` and the Core
/// reads metadata pages of the Yandex Disk API instead.
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
    /// The top directory: "/" for "This Mac", or the QADREN mount directory
    /// of an SMB share (D-022). The browser never goes above it.
    @Published private(set) var base = "/"
    private var baseTitle = "Этот Mac"
    /// The Yandex Disk connection whose folders are shown, or nil for a local
    /// directory.
    private var yandexConnectionID: String?
    /// Ignores a Yandex page that arrives after the user opened another folder.
    private var generation = 0
    /// At most this many entries of one Yandex folder are shown.
    private static let yandexEntryLimit: UInt64 = 1_000

    init(path: String = NSHomeDirectory()) {
        self.path = path
    }

    /// Show another top directory, for example when the panel changes from
    /// "This Mac" to a network drive.
    func rebase(
        _ newBase: String, title: String, open newPath: String? = nil, yandexConnectionID: String? = nil
    ) {
        base = newBase
        baseTitle = title
        self.yandexConnectionID = yandexConnectionID
        open(newPath ?? newBase)
    }

    var crumbs: [Crumb] {
        var result = [Crumb(title: baseTitle, path: base)]
        var current = base == "/" ? "" : base
        for part in path.dropFirst(base == "/" ? 0 : base.count).split(separator: "/") {
            current = current.hasSuffix("/") ? current + part : current + "/" + part
            result.append(Crumb(title: String(part), path: current))
        }
        return result
    }

    var canGoUp: Bool { path != base }

    func open(_ newPath: String) {
        let candidate = newPath.isEmpty ? base : newPath
        let prefix = base.hasSuffix("/") ? base : base + "/"
        // Paths outside the top directory open the top directory.
        path = base == "/" || candidate == base || candidate.hasPrefix(prefix) ? candidate : base
        refresh()
    }

    func openChild(_ name: String) {
        open(Self.join(path, name))
    }

    func up() {
        guard canGoUp else { return }
        // Not NSString path methods: they change "disk:/a" into "disk:".
        guard let slash = path.lastIndex(of: "/") else { return open(base) }
        let parent = String(path[..<slash])
        open(parent.count < base.count ? base : parent)
    }

    func refresh() {
        generation += 1
        if let connectionID = yandexConnectionID {
            refreshYandex(connectionID: connectionID, generation: generation)
            return
        }
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

    /// Read one Yandex Disk folder off the main thread, page by page. The
    /// token is read from the Keychain for this call only.
    private func refreshYandex(connectionID: String, generation: Int) {
        let folder = path
        let limit = Self.yandexEntryLimit
        entries = []
        error = nil
        Task.detached(priority: .userInitiated) { [weak self] in
            let result: Result<[LocalWorkspaceEntry], Error> = Result {
                guard let token = try YandexKeychain.read(connectionID) else {
                    throw StorageError.operation(QADRENUserMessage.text(for: "yandex_token_required"))
                }
                var items: [LocalWorkspaceEntry] = []
                var offset: UInt64 = 0
                while offset < limit {
                    let page = try DiskMetadataBridge.folderPage(accessValue: token, path: folder, offset: offset)
                    items += page.items.map {
                        LocalWorkspaceEntry(name: $0.name, is_directory: $0.kind == "dir", size: $0.size ?? 0)
                    }
                    offset += UInt64(page.items.count)
                    if page.items.isEmpty || offset >= page.total { break }
                }
                return items
            }
            await MainActor.run { [weak self] in
                guard let self, self.generation == generation else { return }
                switch result {
                case let .success(items):
                    self.entries = Self.sorted(items)
                    self.error = nil
                case let .failure(failure):
                    self.entries = []
                    self.error = failure.localizedDescription
                }
            }
        }
    }

    private static func sorted(_ items: [LocalWorkspaceEntry]) -> [LocalWorkspaceEntry] {
        items
            .filter { !$0.name.hasPrefix(".") }
            .sorted {
                $0.is_directory != $1.is_directory
                    ? $0.is_directory
                    : $0.name.localizedStandardCompare($1.name) == .orderedAscending
            }
    }

    static func join(_ directory: String, _ name: String) -> String {
        directory.hasSuffix("/") ? directory + name : directory + "/" + name
    }

    /// The path of `absolute` relative to `root`, or nil when it is not
    /// inside `root`. The root itself is not a selectable object.
    static func relative(_ absolute: String, to root: String) -> String? {
        guard !root.isEmpty else { return nil }
        let prefix = root.hasSuffix("/") ? root : root + "/"
        guard absolute.hasPrefix(prefix), absolute.count > prefix.count else { return nil }
        return String(absolute.dropFirst(prefix.count))
    }

    /// True when a selected ancestor directory already includes `path`.
    static func isImplied(_ path: String, by selection: Set<String>) -> Bool {
        selection.contains { selected in path.hasPrefix(selected + "/") }
    }
}
