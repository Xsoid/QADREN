import Foundation

enum SimulatedFailure: Error { case injected }

struct FakeConnection {
    var present = true
    var enabled = true
    var credentialPresent = true
    var events: [String] = []
    var failDisable = false
    var failKeychain = false
    var failRegistry = false

    mutating func disable() throws {
        events.append("disable")
        if failDisable { throw SimulatedFailure.injected }
        enabled = false
    }

    mutating func removeKeychain() throws {
        events.append("keychain")
        if failKeychain { throw SimulatedFailure.injected }
        credentialPresent = false
    }

    mutating func removeRegistry() throws {
        events.append("registry")
        if failRegistry { throw SimulatedFailure.injected }
        present = false
    }

    mutating func attempt(adapter: String = "yandex_disk") throws {
        // These closures intentionally exercise the production orchestration,
        // not a copy of its step ordering.
        let state = UnsafeMutablePointer<FakeConnection>.allocate(capacity: 1)
        state.initialize(to: self)
        defer {
            self = state.move()
            state.deallocate()
        }
        try ConnectionRemoval.run(
            adapter: adapter,
            disable: { try state.pointee.disable() },
            deleteCredential: { try state.pointee.removeKeychain() },
            deleteRecord: { try state.pointee.removeRegistry() }
        )
    }
}

func check(_ expression: @autoclosure () -> Bool, _ message: String) {
    guard expression() else {
        fputs("FAIL: \\(message)\\n", stderr)
        exit(1)
    }
}

var normal = FakeConnection()
try normal.attempt()
check(!normal.present && !normal.credentialPresent, "normal removal")
check(normal.events == ["disable", "keychain", "registry"], "operation ordering")

var keychainError = FakeConnection(failKeychain: true)
do { try keychainError.attempt(); check(false, "keychain failure must throw") }
catch SimulatedFailure.injected {}
check(keychainError.present && !keychainError.enabled && keychainError.credentialPresent, "keychain failure retains disabled record")
check(keychainError.events == ["disable", "keychain"], "no registry delete after keychain failure")
keychainError.failKeychain = false
try keychainError.attempt()
check(!keychainError.present && !keychainError.credentialPresent, "retry after keychain failure")

var registryError = FakeConnection(failRegistry: true)
do { try registryError.attempt(); check(false, "registry failure must throw") }
catch SimulatedFailure.injected {}
check(registryError.present && !registryError.enabled && !registryError.credentialPresent, "registry failure retains disabled record")
registryError.failRegistry = false
try registryError.attempt()
check(!registryError.present, "retry after registry failure")

var disableError = FakeConnection(failDisable: true)
do { try disableError.attempt(); check(false, "disable failure must throw") }
catch SimulatedFailure.injected {}
check(disableError.present && disableError.enabled && disableError.credentialPresent, "no secret deletion after disable failure")
check(disableError.events == ["disable"], "disable failure stops workflow")

var unsupported = FakeConnection()
do { try unsupported.attempt(adapter: "icloud_documents"); check(false, "unsupported adapter must throw") }
catch ConnectionRemoval.Failure.unsupportedAdapter {}
check(unsupported.events.isEmpty && unsupported.present && unsupported.credentialPresent, "unsupported adapter untouched")

var first = FakeConnection()
var second = FakeConnection()
try first.attempt()
check(second.present && second.credentialPresent && second.enabled, "other connection remains independent")
print("ConnectionRemoval: all injected-failure checks passed")
