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
