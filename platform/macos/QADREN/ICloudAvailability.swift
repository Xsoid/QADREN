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
            return "Недоступно: в этой сборке нет подписанного entitlement для контейнера iCloud."
        }
        return "Entitlement задан для контейнеров: \(value.count); доступ к контейнеру ещё не реализован."
    }
}
