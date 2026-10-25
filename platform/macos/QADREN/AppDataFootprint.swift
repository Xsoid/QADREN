import Darwin
import Foundation

/// The places on this Mac where QADREN keeps its own state: databases,
/// caches, preferences, and mount points. Synchronized user files are not
/// part of it. The scan reads names and allocated sizes only; it never
/// enters a mounted network share.
struct AppDataLocation: Identifiable, Equatable {
    enum Size: Equatable {
        case bytes(Int64)
        /// The location exists, but its size is not counted (for example, a
        /// mount point or the Keychain).
        case notCounted(String)
        case missing
    }

    let id: String
    let title: String
    let symbol: String
    /// Nil for a store without a file path (the Keychain).
    let url: URL?
    let size: Size
    /// A visible one-line breakdown.
    var breakdown: String? = nil
    /// An explanation shown on hover.
    var note: String? = nil
}

enum AppDataFootprint {
    static func scan(fileManager: FileManager = .default) -> [AppDataLocation] {
        let mountPoints = activeMountPoints()
        let library = fileManager.urls(for: .libraryDirectory, in: .userDomainMask)[0]
        let bundleID = Bundle.main.bundleIdentifier ?? "dev.qadren.bootstrap"
        var locations: [AppDataLocation] = []

        if let data = try? QADRENDataLocation.baseDirectory() {
            locations.append(dataRoot(data, mountPoints: mountPoints, fileManager: fileManager))
        }
        if let mounts = try? SMBMounts.root() {
            let active = mountPoints.filter { $0.hasPrefix(mounts.path + "/") }.count
            let exists = fileManager.fileExists(atPath: mounts.path)
            locations.append(AppDataLocation(
                id: "mounts", title: "Точки подключения SMB", symbol: "externaldrive.connected.to.line.below",
                url: mounts,
                size: exists ? .notCounted("Подключено: \(active)") : .missing,
                note: "Содержимое сетевых дисков в размер не входит"
            ))
        }
        let caches = library.appendingPathComponent("Caches/\(bundleID)", isDirectory: true)
        locations.append(AppDataLocation(
            id: "caches", title: "Кэш сетевых запросов", symbol: "archivebox",
            url: caches, size: measure(caches, mountPoints: mountPoints, fileManager: fileManager),
            note: "Можно удалить, macOS создаст заново"
        ))
        let httpStorage = library.appendingPathComponent("HTTPStorages/\(bundleID)", isDirectory: true)
        let cookies = library.appendingPathComponent("HTTPStorages/\(bundleID).binarycookies")
        locations.append(AppDataLocation(
            id: "http", title: "HTTP-хранилище и cookies", symbol: "network",
            url: httpStorage,
            size: sum(
                measure(httpStorage, mountPoints: mountPoints, fileManager: fileManager),
                measure(cookies, mountPoints: mountPoints, fileManager: fileManager)
            )
        ))
        let preferences = library.appendingPathComponent("Preferences/\(bundleID).plist")
        locations.append(AppDataLocation(
            id: "preferences", title: "Настройки интерфейса", symbol: "slider.horizontal.3",
            url: preferences, size: measure(preferences, mountPoints: mountPoints, fileManager: fileManager),
            note: "Тема, раскладка мониторинга, размеры окон"
        ))
        let savedState = library.appendingPathComponent("Saved Application State/\(bundleID).savedState", isDirectory: true)
        if fileManager.fileExists(atPath: savedState.path) {
            locations.append(AppDataLocation(
                id: "saved-state", title: "Состояние окон", symbol: "macwindow",
                url: savedState, size: measure(savedState, mountPoints: mountPoints, fileManager: fileManager)
            ))
        }
        locations.append(AppDataLocation(
            id: "keychain", title: "Связка ключей macOS", symbol: "key",
            url: nil, size: .notCounted("Не учитывается"),
            note: "Токены Яндекса и пароли SMB, по одной записи на подключение"
        ))
        return locations
    }

    static func total(_ locations: [AppDataLocation]) -> Int64 {
        locations.reduce(0) { total, location in
            if case .bytes(let value) = location.size { return total + value }
            return total
        }
    }

    private static func dataRoot(_ url: URL, mountPoints: Set<String>, fileManager: FileManager) -> AppDataLocation {
        let groups: [(prefix: String, title: String)] = [
            ("connections.sqlite3", "подключения и сценарии"),
            ("jobs.sqlite3", "задания"),
            ("snapshots.sqlite3", "снимки"),
            ("worker.log", "журнал"),
        ]
        var sizes = Array(repeating: Int64(0), count: groups.count)
        var other: Int64 = 0
        var total: Int64 = 0
        let names = (try? fileManager.contentsOfDirectory(atPath: url.path)) ?? []
        for name in names {
            let child = url.appendingPathComponent(name)
            guard case .bytes(let size) = measure(child, mountPoints: mountPoints, fileManager: fileManager) else { continue }
            total += size
            if let index = groups.firstIndex(where: { name.hasPrefix($0.prefix) }) {
                sizes[index] += size
            } else {
                other += size
            }
        }
        var parts = groups.indices.compactMap { index in
            sizes[index] > 0 ? "\(groups[index].title) \(format(sizes[index]))" : nil
        }
        if other > 0 { parts.append("прочее \(format(other))") }
        return AppDataLocation(
            id: "data", title: "Базы данных и журнал", symbol: "cylinder.split.1x2",
            url: url, size: .bytes(total),
            breakdown: parts.isEmpty ? nil : parts.joined(separator: " · ").capitalizedFirst
        )
    }

    /// Allocated size of a file or a folder tree, like `du -x`. A mount point
    /// is skipped before any stat call, so a stalled network share cannot
    /// block the scan.
    private static func measure(_ url: URL, mountPoints: Set<String>, fileManager: FileManager) -> AppDataLocation.Size {
        let path = url.standardizedFileURL.path
        guard !mountPoints.contains(path) else { return .bytes(0) }
        var info = stat()
        guard lstat(path, &info) == 0 else { return .missing }
        var total = Int64(info.st_blocks) * 512
        guard (info.st_mode & S_IFMT) == S_IFDIR else { return .bytes(total) }
        for name in (try? fileManager.contentsOfDirectory(atPath: path)) ?? [] {
            if case .bytes(let size) = measure(url.appendingPathComponent(name), mountPoints: mountPoints, fileManager: fileManager) {
                total += size
            }
        }
        return .bytes(total)
    }

    private static func sum(_ first: AppDataLocation.Size, _ second: AppDataLocation.Size) -> AppDataLocation.Size {
        switch (first, second) {
        case (.bytes(let a), .bytes(let b)): return .bytes(a + b)
        case (.bytes, _): return first
        case (_, .bytes): return second
        default: return .missing
        }
    }

    /// The kernel mount table without a status refresh (`MNT_NOWAIT`), so a
    /// stalled network share does not block.
    private static func activeMountPoints() -> Set<String> {
        var buffer: UnsafeMutablePointer<statfs>?
        let count = getmntinfo(&buffer, MNT_NOWAIT)
        guard count > 0, let buffer else { return [] }
        var result = Set<String>()
        for index in 0..<Int(count) {
            var entry = buffer[index]
            let path = withUnsafePointer(to: &entry.f_mntonname) {
                $0.withMemoryRebound(to: CChar.self, capacity: Int(MAXPATHLEN)) { String(cString: $0) }
            }
            result.insert(URL(fileURLWithPath: path).standardizedFileURL.path)
        }
        return result
    }

    static func format(_ bytes: Int64) -> String {
        ByteCountFormatter.string(fromByteCount: bytes, countStyle: .file)
    }
}

private extension String {
    var capitalizedFirst: String { prefix(1).uppercased() + dropFirst() }
}
