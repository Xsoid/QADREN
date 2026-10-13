import Foundation

private func check(_ condition: @autoclosure () -> Bool, _ message: String) {
    guard condition() else { fatalError("FAIL: " + message) }
}

@main
struct YandexKeychainTests {
    static func main() throws {
        let connectionID = "qadren-native-keychain-test-" + UUID().uuidString.lowercased()
        defer { try? YandexKeychain.delete(connectionID) }

        // An independently generated ID makes this runner unable to read,
        // replace, or delete an existing user connection credential.
        try YandexKeychain.delete(connectionID)
        var stored = try YandexKeychain.read(connectionID)
        check(stored == nil, "fresh test item is absent")

        try YandexKeychain.store("qadren-keychain-test-v1", for: connectionID)
        stored = try YandexKeychain.read(connectionID)
        check(stored == "qadren-keychain-test-v1", "stored item can be read")

        try YandexKeychain.store("qadren-keychain-test-v2", for: connectionID)
        stored = try YandexKeychain.read(connectionID)
        check(stored == "qadren-keychain-test-v2", "duplicate item is safely updated")

        try YandexKeychain.delete(connectionID)
        stored = try YandexKeychain.read(connectionID)
        check(stored == nil, "delete removes only the test item")
        try YandexKeychain.delete(connectionID)

        print("YandexKeychain native macOS smoke tests passed")
    }
}
