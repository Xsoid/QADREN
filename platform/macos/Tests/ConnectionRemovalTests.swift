import Foundation

enum SimulatedFailure: Error { case injected }

final class FakeConnection {
    var present = true
    var enabled = true
    var credentialPresent = true
    var events: [String] = []
    var failDisable = false
    var failKeychain = false
    var failRegistry = false

    func attempt(adapter: String = "yandex_disk") throws {
        try ConnectionRemoval.run(
            adapter: adapter,
            disable: {
                self.events.append("disable")
                if self.failDisable { throw SimulatedFailure.injected }
                self.enabled = false
            },
            deleteCredential: {
                self.events.append("keychain")
                if self.failKeychain { throw SimulatedFailure.injected }
                self.credentialPresent = false // Keychain not-found is also success.
            },
            deleteRecord: {
                self.events.append("registry")
                if self.failRegistry { throw SimulatedFailure.injected }
                self.present = false
            }
        )
    }
}

func check(_ condition: @autoclosure () -> Bool, _ message: String) {
    guard condition() else {
        fatalError("FAIL: " + message)
    }
}

let normal = FakeConnection()
try normal.attempt()
check(!normal.present && !normal.credentialPresent, "normal removal")
check(normal.events == ["disable", "keychain", "registry"], "operation ordering")

let keychainError = FakeConnection()
keychainError.failKeychain = true
do { try keychainError.attempt(); fatalError("keychain failure must throw") }
catch SimulatedFailure.injected {}
check(keychainError.present && !keychainError.enabled && keychainError.credentialPresent, "keychain failure retains disabled record")
check(keychainError.events == ["disable", "keychain"], "no registry deletion after keychain failure")
keychainError.failKeychain = false
try keychainError.attempt()
check(!keychainError.present && !keychainError.credentialPresent, "retry after keychain failure")

let registryError = FakeConnection()
registryError.failRegistry = true
do { try registryError.attempt(); fatalError("registry failure must throw") }
catch SimulatedFailure.injected {}
check(registryError.present && !registryError.enabled && !registryError.credentialPresent, "registry failure retains disabled record")
registryError.failRegistry = false
try registryError.attempt()
check(!registryError.present, "retry after registry failure")

let disableError = FakeConnection()
disableError.failDisable = true
do { try disableError.attempt(); fatalError("disable failure must throw") }
catch SimulatedFailure.injected {}
check(disableError.present && disableError.enabled && disableError.credentialPresent, "no secret deletion after disable failure")
check(disableError.events == ["disable"], "disable failure stops workflow")

let unsupported = FakeConnection()
do { try unsupported.attempt(adapter: "icloud_documents"); fatalError("unsupported adapter must throw") }
catch ConnectionRemoval.Failure.unsupportedAdapter {}
check(unsupported.events.isEmpty && unsupported.present && unsupported.credentialPresent, "unsupported adapter untouched")

let first = FakeConnection()
let second = FakeConnection()
try first.attempt()
check(second.present && second.credentialPresent && second.enabled, "other connection remains independent")
print("ConnectionRemoval: all injected-failure checks passed")
