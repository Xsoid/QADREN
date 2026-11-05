import AppKit
import Darwin
import Foundation
import NetFS
import Network
import Security

/// A login and a password of one connection: an SMB share (D-021) or an
/// ISPmanager panel (D-027). They live only in the Keychain item of that
/// `connection_id`, never in SQLite or the address.
struct LoginCredentials: Codable, Equatable {
    var login: String
    var password: String
}
typealias SMBCredentials = LoginCredentials

/// Keychain records are isolated by StorageConnection ID, as for Yandex Disk.
/// Each connector has its own Keychain service.
struct LoginKeychain: Sendable {
    let service: String

    private func query(_ connectionID: String) -> [String: Any] {
        [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: connectionID
        ]
    }

    func read(_ connectionID: String) throws -> LoginCredentials? {
        var q = query(connectionID)
        q[kSecReturnData as String] = true
        q[kSecMatchLimit as String] = kSecMatchLimitOne
        var item: CFTypeRef?
        let status = SecItemCopyMatching(q as CFDictionary, &item)
        if status == errSecItemNotFound { return nil }
        guard status == errSecSuccess, let data = item as? Data,
              let credentials = try? JSONDecoder().decode(LoginCredentials.self, from: data) else {
            throw StorageError.operation("Keychain read failed")
        }
        return credentials
    }

    func store(_ credentials: LoginCredentials, for connectionID: String) throws {
        guard !credentials.login.isEmpty && !connectionID.isEmpty else {
            throw StorageError.operation("Empty credentials")
        }
        let data = try JSONEncoder().encode(credentials)
        let key = query(connectionID)
        var item = key
        item[kSecValueData as String] = data
        item[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
        var result = SecItemAdd(item as CFDictionary, nil)
        if result == errSecDuplicateItem {
            result = SecItemUpdate(key as CFDictionary, [kSecValueData as String: data] as CFDictionary)
        }
        guard result == errSecSuccess else {
            throw StorageError.operation("Keychain write failed")
        }
    }

    func delete(_ connectionID: String) throws {
        let result = SecItemDelete(query(connectionID) as CFDictionary)
        guard result == errSecSuccess || result == errSecItemNotFound else {
            throw StorageError.operation("Keychain removal failed")
        }
    }
}

let SMBKeychain = LoginKeychain(service: "dev.qadren.smb.credentials")

/// The address of an SMB share: `smb://host[:port]/share[/folder]`. Core
/// stores and validates the same text; this type only builds the mount URL.
enum SMBAddress {
    /// The form that Core stores: trimmed, without a trailing slash.
    static func normalized(_ address: String) -> String {
        var value = address.trimmingCharacters(in: .whitespacesAndNewlines)
        while value.hasSuffix("/") { value.removeLast() }
        if value.lowercased().hasPrefix("smb://") { value = "smb://" + value.dropFirst(6) }
        return value
    }

    /// A percent-encoded URL for NetFS, or nil when the address has a login,
    /// a domain, a query, a fragment, or no share name.
    static func mountURL(_ address: String) -> URL? {
        let trimmed = address.trimmingCharacters(in: .whitespacesAndNewlines)
        guard trimmed.lowercased().hasPrefix("smb://") else { return nil }
        let rest = trimmed.dropFirst(6)
        guard !rest.contains(where: { "@;?#\\".contains($0) }),
              let slash = rest.firstIndex(of: "/") else { return nil }
        let authority = String(rest[..<slash])
        let segments = rest[rest.index(after: slash)...].split(separator: "/")
        guard !segments.isEmpty, !segments.contains(where: { $0 == "." || $0 == ".." }) else {
            return nil
        }
        var components = URLComponents()
        components.scheme = "smb"
        if let colon = authority.lastIndex(of: ":"), !authority.hasSuffix("]") {
            guard let port = Int(authority[authority.index(after: colon)...]), (1...65535).contains(port) else {
                return nil
            }
            components.host = String(authority[..<colon])
            components.port = port
        } else {
            components.host = authority
        }
        guard !(components.host ?? "").isEmpty else { return nil }
        components.path = "/" + segments.joined(separator: "/")
        return components.url
    }
}

/// The result of one login check. Each case maps to one Registry status.
enum SMBCheckOutcome: Equatable {
    case connected
    /// macOS already has this share mounted. The server is reachable, but
    /// macOS does not mount it again, so the saved login is not rechecked.
    case alreadyMounted
    case requiresAuth
    /// EACCES: the server refused this user for the share, or the login is
    /// wrong. macOS does not tell the two apart.
    case accessDenied
    case offline
    case notFound
    case failed(Int32)

    var registryStatus: String {
        switch self {
        case .connected, .alreadyMounted: "connected"
        case .requiresAuth, .accessDenied: "requires_auth"
        case .offline: "offline"
        case .notFound, .failed: "error"
        }
    }

    var message: String {
        switch self {
        case .connected:
            "Вход выполнен. Сетевая папка доступна с этим логином и паролем."
        case .alreadyMounted:
            "Сетевая папка уже подключена в macOS, сервер доступен. Повторно логин и пароль macOS не проверяет."
        case .requiresAuth:
            "Сервер не принял логин или пароль. Проверьте их и повторите."
        case .accessDenied:
            "Сервер отказал в доступе к общей папке. Проверьте логин, пароль и права этого пользователя на папку."
        case .offline:
            "Сервер не отвечает. Проверьте адрес, сеть и что сервер включён."
        case .notFound:
            "На сервере нет такой общей папки. Проверьте адрес после имени сервера."
        case .failed(EPERM):
            "macOS не разрешила подключить сетевую папку в это место (код 1)."
        case let .failed(code):
            "Не удалось подключиться к сетевой папке (код \(code))."
        }
    }
}

/// Checks a login by mounting the share through macOS NetFS without any
/// system dialog, then unmounts it. The check reads no file content and
/// changes nothing on the server.
enum SMBShareCheck {
    static func check(address: String, credentials: SMBCredentials) async -> SMBCheckOutcome {
        guard let url = SMBAddress.mountURL(address) else { return .notFound }
        return await Task.detached(priority: .userInitiated) {
            mountAndRelease(url: url, credentials: credentials)
        }.value
    }

    private static func mountAndRelease(url: URL, credentials: SMBCredentials) -> SMBCheckOutcome {
        // NetFS.h keys: kNAUIOptionKey/kNAUIOptionNoUI and kNetFSMountFlagsKey.
        let openOptions = NSMutableDictionary(dictionary: ["UIOption": "NoUI"])
        // The check mount does not appear in the Finder sidebar.
        let mountOptions = NSMutableDictionary(dictionary: ["MountFlags": MNT_DONTBROWSE, "AllowSubMounts": true])
        var mountpoints: Unmanaged<CFArray>?
        let status = NetFSMountURLSync(
            url as CFURL, nil,
            credentials.login as CFString, credentials.password as CFString,
            openOptions, mountOptions, &mountpoints
        )
        let paths = (mountpoints?.takeRetainedValue() as? [String]) ?? []
        switch status {
        case 0:
            // Release only the mount that this check made.
            for path in paths { _ = unmount(path, 0) }
            return .connected
        case EEXIST:
            return .alreadyMounted
        case EAUTH:
            return .requiresAuth
        case EACCES:
            return .accessDenied
        case ENOENT:
            return .notFound
        case EHOSTUNREACH, ENETUNREACH, ETIMEDOUT, ECONNREFUSED, EHOSTDOWN, ENETDOWN:
            return .offline
        default:
            return .failed(status)
        }
    }
}

/// One SMB share or folder that the user picked with the mouse. macOS
/// supplies the address; the login comes from the macOS mount, if any.
struct SMBShareChoice: Identifiable, Equatable {
    let address: String
    let login: String?
    /// Share or folder name and the server name, for the list row.
    let title: String
    let server: String
    var id: String { address }

    /// The SMB address of a folder on a mounted share, from the remount URL
    /// that macOS keeps for the volume. Nil for a folder that is not on SMB.
    static func forFolder(_ folder: URL) -> SMBShareChoice? {
        let keys: Set<URLResourceKey> = [.volumeURLKey, .volumeURLForRemountingKey]
        guard let values = try? folder.resourceValues(forKeys: keys),
              let remount = values.volumeURLForRemounting,
              remount.scheme?.lowercased() == "smb",
              let volume = values.volume,
              let host = remount.host(percentEncoded: false), !host.isEmpty else { return nil }
        let share = remount.path(percentEncoded: false)
            .split(separator: "/").map(String.init)
        guard !share.isEmpty else { return nil }
        let volumePath = volume.standardizedFileURL.path
        let folderPath = folder.standardizedFileURL.path
        guard folderPath == volumePath || folderPath.hasPrefix(volumePath + "/") else { return nil }
        let inner = folderPath.dropFirst(volumePath.count).split(separator: "/").map(String.init)
        let port = remount.port.map { ":\($0)" } ?? ""
        // Strip the login from the address: it goes to the Keychain only.
        let address = "smb://" + host + port + "/" + (share + inner).joined(separator: "/")
        let login = remount.user(percentEncoded: false).flatMap { $0.isEmpty ? nil : $0 }
        return SMBShareChoice(
            address: address, login: login,
            title: (share + inner).last ?? host, server: server(host)
        )
    }

    /// SMB shares that macOS has mounted now, for example from Finder.
    static func mounted() -> [SMBShareChoice] {
        let volumes = FileManager.default.mountedVolumeURLs(
            includingResourceValuesForKeys: [.volumeURLForRemountingKey], options: []
        ) ?? []
        return volumes.compactMap(forFolder).sorted { $0.title.localizedStandardCompare($1.title) == .orderedAscending }
    }

    /// "My NAS._smb._tcp.local" reads as "My NAS".
    static func server(_ host: String) -> String {
        host.replacingOccurrences(of: "._smb._tcp.local", with: "", options: [.caseInsensitive])
    }
}

/// Finds SMB servers in the local network through Bonjour, as Finder shows
/// them under "Сеть". It reads only service names, not shares or files.
@MainActor
final class SMBServerBrowser: ObservableObject {
    @Published private(set) var servers: [String] = []
    private var browser: NWBrowser?

    func start() {
        guard browser == nil else { return }
        let browser = NWBrowser(for: .bonjour(type: "_smb._tcp", domain: nil), using: .tcp)
        let ownName = Host.current().localizedName
        browser.browseResultsChangedHandler = { [weak self] results, _ in
            let names = Set(results.compactMap { result -> String? in
                if case let .service(name, _, _, _) = result.endpoint, name != ownName { return name }
                return nil
            })
            DispatchQueue.main.async {
                self?.servers = names.sorted { $0.localizedStandardCompare($1) == .orderedAscending }
            }
        }
        browser.start(queue: .main)
        self.browser = browser
    }

    func stop() {
        browser?.cancel()
        browser = nil
    }

    /// Opens the server in macOS. macOS asks for the login and the share in
    /// its own dialog and mounts the share; the mounted list then shows it.
    static func open(_ server: String) {
        guard let host = (server + "._smb._tcp.local").addingPercentEncoding(withAllowedCharacters: .urlHostAllowed),
              let url = URL(string: "smb://" + host) else { return }
        NSWorkspace.shared.open(url)
    }

    /// The system folder panel, attached to `window` as a sheet, so that the
    /// form keeps the focus after the choice. A folder inside a mounted SMB
    /// share gives its address; any other folder gives nil. `completion` is
    /// not called when the user cancels.
    static func chooseFolder(attachedTo window: NSWindow?, completion: @escaping (SMBShareChoice?) -> Void) {
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.allowsMultipleSelection = false
        panel.prompt = "Выбрать"
        panel.message = "Выберите общую папку или папку внутри неё на сетевом диске."
        panel.directoryURL = URL(fileURLWithPath: "/Volumes", isDirectory: true)
        let finish = { (response: NSApplication.ModalResponse) in
            window?.makeKeyAndOrderFront(nil)
            guard response == .OK, let url = panel.url else { return }
            completion(SMBShareChoice.forFolder(url))
        }
        if let window {
            panel.beginSheetModal(for: window, completionHandler: finish)
        } else {
            panel.begin(completionHandler: finish)
        }
    }
}

/// The mount of one SMB share connection that QADREN owns (D-022):
/// `<data root>/mounts/<connection_id>`. Core computes the same directory and
/// refuses every plan and effect while it is not an `smbfs` mount point, so a
/// missing mount never turns into work on the local disk.
enum SMBMounts {
    enum Result: Equatable {
        case mounted
        /// macOS has this share mounted at another place, usually by Finder.
        /// macOS mounts one share only once, so QADREN cannot mount it again.
        case mountedElsewhere(String)
        case noCredentials
        case failed(SMBCheckOutcome)

        var message: String? {
            switch self {
            case .mounted:
                nil
            case let .mountedElsewhere(path) where SMBMounts.isTimeMachine(path):
                "Эту сетевую папку сейчас использует Time Machine. macOS подключает одну папку только один раз, поэтому QADREN подключит её после окончания резервного копирования."
            case let .mountedElsewhere(path):
                "Эта сетевая папка уже подключена в macOS (\(path)), например в Finder. macOS подключает одну папку только один раз. Отключите её там, и QADREN подключит её для себя."
            case .noCredentials:
                "Для сетевого диска не сохранены логин и пароль. Укажите их в разделе «Подключения»."
            case let .failed(outcome):
                outcome.message
            }
        }
    }

    /// The same rule as `storage_access::smb_mounts_root` in Core: macOS
    /// refuses a network mount inside `~/Library`, so the default data root
    /// uses `~/.qadren/mounts`; another data root uses `<data root>/mounts`.
    static func root() throws -> URL {
        let data = try QADRENDataLocation.baseDirectory().standardizedFileURL
        if data.path.hasSuffix("/Library/Application Support/QADREN") {
            return data.deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
                .appendingPathComponent(".qadren", isDirectory: true)
                .appendingPathComponent("mounts", isDirectory: true)
        }
        return data.appendingPathComponent("mounts", isDirectory: true)
    }

    static func directory(for connectionID: String) throws -> URL {
        let directory = try root()
            .appendingPathComponent(connectionID, isDirectory: true)
        try FileManager.default.createDirectory(
            at: directory, withIntermediateDirectories: true,
            attributes: [.posixPermissions: NSNumber(value: 0o700)]
        )
        return directory
    }

    /// The absolute path of the mount directory, for Scenario roots.
    static func path(for connectionID: String) -> String? {
        try? directory(for: connectionID).path
    }

    /// True when the directory is the mount point of an `smbfs` filesystem.
    static func isMounted(_ connectionID: String) -> Bool {
        guard let directory = try? directory(for: connectionID) else { return false }
        let resolved = directory.resolvingSymlinksInPath().path
        var info = statfs()
        guard statfs(resolved, &info) == 0 else { return false }
        let kind = withUnsafeBytes(of: info.f_fstypename) { String(decoding: $0.prefix { $0 != 0 }, as: UTF8.self) }
        let mountedOn = withUnsafeBytes(of: info.f_mntonname) { String(decoding: $0.prefix { $0 != 0 }, as: UTF8.self) }
        return kind == "smbfs" && mountedOn == resolved
    }

    /// One mount attempt per connection at a time. Two windows that start
    /// at once must not stack two mounts on one directory.
    @MainActor private static var inFlight: [String: Task<Result, Never>] = [:]

    /// Mounts the share of `entry` into its directory with the Keychain login,
    /// unless it is mounted there already. No system dialog appears.
    @MainActor
    static func ensureMounted(_ entry: StorageEntry) async -> Result {
        if let running = inFlight[entry.id] { return await running.value }
        let task = Task { await mountOnce(entry) }
        inFlight[entry.id] = task
        let result = await task.value
        inFlight[entry.id] = nil
        return result
    }

    private static func mountOnce(_ entry: StorageEntry) async -> Result {
        if isMounted(entry.id) { return .mounted }
        guard let address = entry.endpoint_url, let url = SMBAddress.mountURL(address) else {
            return .failed(.notFound)
        }
        // macOS mounts one share once. A second mount fails with a code that
        // looks like a refused login, so check the existing mounts first.
        if let elsewhere = mountedElsewhere(url) { return .mountedElsewhere(elsewhere) }
        guard let credentials = try? SMBKeychain.read(entry.id),
              let directory = try? directory(for: entry.id) else { return .noCredentials }
        return await Task.detached(priority: .userInitiated) {
            mount(url: url, at: directory, credentials: credentials)
        }.value
    }

    /// Mounts every enabled SMB connection with saved credentials, so that
    /// plans and scheduled Jobs find the files. Errors stay in the status.
    @MainActor
    static func mountAll(_ connections: StorageConnections) async {
        for entry in connections.entries where entry.adapter == "smb_share" && entry.enabled && entry.credential_ref != nil {
            let result = await ensureMounted(entry)
            if case let .failed(outcome) = result {
                connections.recordSMBCheck(id: entry.id, outcome: outcome)
            } else if result == .mounted, entry.status != "connected" {
                connections.recordSMBCheck(id: entry.id, outcome: .connected)
            }
        }
    }

    /// Mounts the SMB endpoints of a Scenario before a plan. Returns a message
    /// when an endpoint cannot be mounted; nil when every endpoint is ready.
    @MainActor
    static func prepare(connectionIDs: [String]) async -> String? {
        guard let entries = try? StorageRegistryBridge.list() else { return nil }
        for id in Set(connectionIDs) {
            guard let entry = entries.first(where: { $0.id == id }), entry.adapter == "smb_share" else { continue }
            if let message = await ensureMounted(entry).message {
                return "«\(entry.name)»: " + message
            }
        }
        return nil
    }

    private static func mount(url: URL, at directory: URL, credentials: SMBCredentials) -> Result {
        let openOptions = NSMutableDictionary(dictionary: ["UIOption": "NoUI"])
        // NetFS.h: kNetFSMountAtMountDirKey mounts exactly at the given path.
        // kNetFSAllowSubMountsKey: an address with a folder mounts that folder.
        // The mount is visible in Finder, so that Finder and QADREN share one
        // mount: macOS mounts a share only once.
        let mountOptions = NSMutableDictionary(dictionary: [
            "MountAtMountDir": true,
            "AllowSubMounts": true
        ])
        var mountpoints: Unmanaged<CFArray>?
        let status = NetFSMountURLSync(
            url as CFURL, directory as CFURL,
            credentials.login as CFString, credentials.password as CFString,
            openOptions, mountOptions, &mountpoints
        )
        let paths = (mountpoints?.takeRetainedValue() as? [String]) ?? []
        switch status {
        case 0:
            return .mounted
        case EEXIST:
            if let elsewhere = mountedElsewhere(url) { return .mountedElsewhere(elsewhere) }
            return paths.isEmpty ? .failed(.failed(status)) : .mountedElsewhere(paths[0])
        case EAUTH:
            return .failed(.requiresAuth)
        case EACCES:
            return .failed(.accessDenied)
        case ENOENT:
            return .failed(.notFound)
        case EHOSTUNREACH, ENETUNREACH, ETIMEDOUT, ECONNREFUSED, EHOSTDOWN, ENETDOWN:
            return .failed(.offline)
        default:
            return .failed(.failed(status))
        }
    }

    /// Unmounts the QADREN mount of a connection and removes its directory
    /// when it is empty. `rmdir` never removes content, so no file on the
    /// share or on this Mac can be deleted here.
    static func release(_ connectionID: String) {
        guard let directory = try? root()
            .appendingPathComponent(connectionID, isDirectory: true) else { return }
        if isMounted(connectionID) { _ = unmount(directory.path, 0) }
        if !isMounted(connectionID) { _ = rmdir(directory.path) }
    }

    /// Unmounts the mount of this share at another place (after an explicit
    /// user click), then mounts it for QADREN. Files on the share stay as is.
    @MainActor
    static func takeOver(_ entry: StorageEntry, from path: String) async -> Result {
        guard !isTimeMachine(path) else { return .mountedElsewhere(path) }
        do {
            try await FileManager.default.unmountVolume(at: URL(fileURLWithPath: path), options: [])
        } catch {
            return .failed(.failed(Int32((error as NSError).code)))
        }
        return await ensureMounted(entry)
    }

    /// A Time Machine mount must never be unmounted by QADREN.
    static func isTimeMachine(_ path: String) -> Bool {
        path.hasPrefix("/Volumes/.timemachine/")
    }

    /// The local path where macOS has mounted the share of `url`, if any.
    /// Host names compare without a trailing dot ("local." is "local").
    static func mountedElsewhere(_ url: URL) -> String? {
        func hostKey(_ url: URL) -> String? {
            url.host(percentEncoded: false).map { name in
                var name = name.lowercased()
                while name.hasSuffix(".") { name.removeLast() }
                return name
            }
        }
        let host = hostKey(url)
        let share = url.path(percentEncoded: false).split(separator: "/").first.map { $0.lowercased() }
        let volumes = FileManager.default.mountedVolumeURLs(
            includingResourceValuesForKeys: [.volumeURLForRemountingKey], options: []
        ) ?? []
        for volume in volumes {
            guard let remount = try? volume.resourceValues(forKeys: [.volumeURLForRemountingKey]).volumeURLForRemounting,
                  remount.scheme?.lowercased() == "smb",
                  hostKey(remount) == host,
                  remount.path(percentEncoded: false).split(separator: "/").first.map({ $0.lowercased() }) == share
            else { continue }
            return volume.path
        }
        return nil
    }
}
