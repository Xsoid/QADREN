import Foundation
import Security

/// Descriptive entitlement gate. It never opens iCloud Drive or a container.
enum ICloudAvailability {
    static var status: String {
        guard let task = SecTaskCreateFromSelf(nil),
              let value = SecTaskCopyValueForEntitlement(
                task,
                "com.apple.developer.icloud-container-identifiers" as CFString,
                nil
              ) as? [String],
              !value.isEmpty else {
            return "Unavailable: this build has no signed iCloud container entitlement."
        }
        return "Entitlement present for \(value.count) container(s); container access is not implemented."
    }
}
