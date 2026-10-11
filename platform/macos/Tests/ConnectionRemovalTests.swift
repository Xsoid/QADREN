import Foundation

enum SimulatedFailure: Error { case injected }

final class FakeConnection {
    var present = true
    var enabled = true
    var credentialPresent = true
    var events: [String] = []
    var failDisable = false
    var failAttemptCheck = false
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
            ensureNoUnresolvedAttempts: {
                self.events.append("attempts")
                if self.failAttemptCheck { throw SimulatedFailure.injected }
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

@main
struct ConnectionRemovalTests {
static func main() throws {
let normal = FakeConnection()
try normal.attempt()
check(!normal.present && !normal.credentialPresent, "normal removal")
check(normal.events == ["disable", "attempts", "keychain", "registry"], "operation ordering")

let unresolvedAttempt = FakeConnection()
unresolvedAttempt.failAttemptCheck = true
do { try unresolvedAttempt.attempt(); fatalError("unresolved attempt must stop removal") }
catch SimulatedFailure.injected {}
check(unresolvedAttempt.present && !unresolvedAttempt.enabled && unresolvedAttempt.credentialPresent, "attempt gate preserves recovery state")
check(unresolvedAttempt.events == ["disable", "attempts"], "attempt gate stops before Keychain")
unresolvedAttempt.failAttemptCheck = false
try unresolvedAttempt.attempt()
check(!unresolvedAttempt.present && !unresolvedAttempt.credentialPresent, "retry after reconciliation")

let keychainError = FakeConnection()
keychainError.failKeychain = true
do { try keychainError.attempt(); fatalError("keychain failure must throw") }
catch SimulatedFailure.injected {}
check(keychainError.present && !keychainError.enabled && keychainError.credentialPresent, "keychain failure retains disabled record")
check(keychainError.events == ["disable", "attempts", "keychain"], "no registry deletion after keychain failure")
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
// Inject failure at each step: never mark the record connected prematurely.
for failureAt in 0..<5 {
    var events: [String] = []
    var status = "connected"
    var credentialWritten = false
    let step: (Int, String) throws -> Void = { index, name in
        events.append(name)
        if index == failureAt { throw SimulatedFailure.injected }
    }
    do {
        try ConnectionAuthorization.commit(
            markRequiresAuth: {
                try step(0, "requires_auth")
                status = "requires_auth"
            },
            storeCredential: {
                try step(1, "keychain")
                credentialWritten = true
            },
            persistIdentity: { try step(2, "identity") },
            persistCredentialReference: { try step(3, "reference") },
            markConnected: {
                try step(4, "connected")
                status = "connected"
            }
        )
        fatalError("injected failure must throw")
    } catch SimulatedFailure.injected {}
    if failureAt == 0 {
        check(!credentialWritten && status == "connected", "preflight failure cannot write Keychain")
    } else {
        check(status == "requires_auth", "interrupted authorization must stay blocked")
    }
    check(events.count == failureAt + 1, "no operation after injected failure")
}
check(!ConnectionAuthorization.mayRefresh(enabled: true, status: "requires_auth", accountID: "1", credentialReference: "keychain:a"), "incomplete transaction blocked after restart")
check(!ConnectionAuthorization.mayRefresh(enabled: true, status: "connected", accountID: nil, credentialReference: "keychain:a"), "missing account binding blocked")
check(!ConnectionAuthorization.mayRefresh(enabled: true, status: "connected", accountID: "1", credentialReference: nil), "missing credential reference blocked")
check(ConnectionAuthorization.mayRefresh(enabled: true, status: "connected", accountID: "1", credentialReference: "keychain:a"), "complete record can refresh")
print("ConnectionRemoval: all injected-failure checks passed")
}
}
