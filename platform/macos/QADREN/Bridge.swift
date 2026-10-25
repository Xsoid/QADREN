import Foundation

// C ABI exported from qadren-core. These return static strings; never free them.
@_silgen_name("qadren_core_version")
private func coreVersionPointer() -> UnsafePointer<CChar>?

@_silgen_name("qadren_core_abi_version")
private func coreABIVersion() -> UInt32

@_silgen_name("qadren_core_status")
private func coreStatusPointer() -> UnsafePointer<CChar>?

@_silgen_name("qadren_worker_is_idle")
private func workerIsIdle() -> Bool

enum CoreBridge {
    static let supportedABI: UInt32 = 1

    static var isCompatible: Bool { coreABIVersion() == supportedABI }

    static var version: String {
        guard isCompatible, let pointer = coreVersionPointer() else {
            return "Incompatible core"
        }
        return String(cString: pointer)
    }

    static var status: String {
        guard isCompatible, let pointer = coreStatusPointer() else {
            return "Core ABI mismatch"
        }
        return String(cString: pointer)
    }

    static var workerStatus: String {
        guard isCompatible else { return "Unavailable" }
        return workerIsIdle() ? "Idle" : "Busy"
    }
}

@_silgen_name("qadren_yandex_disk_info")
private func yandexDiskInfo(_ token: UnsafePointer<CChar>?) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_yandex_list_root")
private func yandexRoot(_ token: UnsafePointer<CChar>?) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_yandex_list_folder")
private func yandexFolder(_ token: UnsafePointer<CChar>?, _ path: UnsafePointer<CChar>?) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_yandex_free_string")
private func yandexFree(_ pointer: UnsafeMutablePointer<CChar>?)

struct DiskMetadata: Decodable {
    let total_space: UInt64
    let used_space: UInt64
    let trash_size: UInt64
}
struct DiskItem: Decodable, Identifiable {
    var id: String { path }
    let name: String
    let path: String
    let kind: String
    let size: UInt64?

    enum CodingKeys: String, CodingKey {
        case name, path, size
        case kind = "type"
    }
}
struct DiskListing: Decodable {
    let items: [DiskItem]
    let total: UInt64
    let offset: UInt64
}
private struct RootResource: Decodable {
    let embedded: DiskListing?
    enum CodingKeys: String, CodingKey { case embedded = "_embedded" }
}
private struct MetadataResult<T: Decodable>: Decodable {
    let ok: Bool
    let data: T?
    let error: String?
}
enum DiskMetadataBridge {
    private static func request<T: Decodable>(
        token: String,
        function: (UnsafePointer<CChar>?) -> UnsafeMutablePointer<CChar>?
    ) throws -> T {
        guard CoreBridge.isCompatible else { throw BridgeFailure.message("Core ABI mismatch") }
        let pointer = token.withCString { function($0) }
        guard let pointer else { throw BridgeFailure.message("No response from Rust") }
        defer { yandexFree(pointer) }
        let result: MetadataResult<T>
        do {
            result = try JSONDecoder().decode(MetadataResult<T>.self, from: Data(String(cString: pointer).utf8))
        } catch let DecodingError.keyNotFound(key, context) {
            let path = context.codingPath.map(\.stringValue).joined(separator: ".")
            throw BridgeFailure.message("Missing metadata field: " + (path.isEmpty ? key.stringValue : path + "." + key.stringValue))
        } catch let DecodingError.typeMismatch(_, context) {
            throw BridgeFailure.message("Metadata type mismatch at: " + context.codingPath.map(\.stringValue).joined(separator: "."))
        } catch let DecodingError.valueNotFound(_, context) {
            throw BridgeFailure.message("Missing metadata value at: " + context.codingPath.map(\.stringValue).joined(separator: "."))
        } catch {
            throw BridgeFailure.message("Invalid metadata response")
        }
        guard result.ok, let value = result.data else {
            throw BridgeFailure.message(result.error ?? "Unknown provider error")
        }
        return value
    }
    static func quota(token: String) throws -> DiskMetadata {
        try request(token: token, function: yandexDiskInfo)
    }
    static func root(token: String) throws -> DiskListing {
        let response: RootResource = try request(token: token, function: yandexRoot)
        guard let listing = response.embedded else {
            throw BridgeFailure.message("Missing directory listing")
        }
        return listing
    }
    /// Recursive metadata of one Yandex Disk Scenario root (D-023). An error
    /// code comes back as a `yandex_` user message code.
    static func scenarioSnapshot(
        registryPath: String, connectionID: String, root: String, selection: [String], accessValue: String
    ) throws -> [ScenarioSnapshotItem] {
        let selectionJSON = String(decoding: try JSONEncoder().encode(selection), as: UTF8.self)
        let pointer = registryPath.withCString { registry in
            connectionID.withCString { connection in
                root.withCString { root in
                    selectionJSON.withCString { selection in
                        accessValue.withCString { value in
                            yandexScenarioSnapshot(registry, connection, root, selection, value)
                        }
                    }
                }
            }
        }
        guard let pointer else { throw BridgeFailure.message("No response from Rust") }
        defer { yandexFree(pointer) }
        guard let result = try? JSONDecoder().decode(
            MetadataResult<[ScenarioSnapshotItem]>.self, from: Data(String(cString: pointer).utf8)
        ) else {
            throw BridgeFailure.message("Invalid metadata response")
        }
        guard result.ok, let items = result.data else {
            let code = result.error ?? "provider_error"
            throw BridgeFailure.message(QADRENUserMessage.text(
                for: code.hasPrefix("local_") ? code : "yandex_" + code
            ))
        }
        return items
    }
    static func folder(token: String, path: String) throws -> DiskListing {
        try folderPage(accessValue: token, path: path, offset: 0)
    }
    static func folderPage(accessValue: String, path: String, offset: UInt64) throws -> DiskListing {
        let pointer = accessValue.withCString { value in
            path.withCString { path in yandexFolderPage(value, path, offset) }
        }
        guard let pointer else { throw BridgeFailure.message("No response from Rust") }
        defer { yandexFree(pointer) }
        let result: MetadataResult<RootResource>
        do {
            result = try JSONDecoder().decode(MetadataResult<RootResource>.self, from: Data(String(cString: pointer).utf8))
        } catch {
            throw BridgeFailure.message("Invalid metadata response")
        }
        guard result.ok, let listing = result.data?.embedded else {
            throw BridgeFailure.message(result.error ?? "Missing directory listing")
        }
        return listing
    }
}
private enum BridgeFailure: Error {
    case message(String)
}

@_silgen_name("qadren_yandex_scenario_metadata_snapshot")
private func yandexScenarioSnapshot(
    _ registry: UnsafePointer<CChar>?, _ connectionID: UnsafePointer<CChar>?,
    _ root: UnsafePointer<CChar>?, _ selection: UnsafePointer<CChar>?, _ value: UnsafePointer<CChar>?
) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_yandex_list_folder_page")
private func yandexFolderPage(_ value: UnsafePointer<CChar>?, _ path: UnsafePointer<CChar>?, _ offset: UInt64) -> UnsafeMutablePointer<CChar>?

extension BridgeFailure: LocalizedError {
    var errorDescription: String? {
        if case let .message(message) = self { return message }
        return nil
    }
}
