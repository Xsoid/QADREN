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
}
private enum BridgeFailure: Error {
    case message(String)
}
extension BridgeFailure: LocalizedError {
    var errorDescription: String? {
        if case let .message(message) = self { return message }
        return nil
    }
}
