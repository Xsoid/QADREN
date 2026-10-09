import Foundation

/// Coordinates two independent persistence systems; deliberately does not
/// pretend that SQLite and Keychain share an atomic transaction.
enum ConnectionRemoval {
    enum Failure: Error {
        case unsupportedAdapter
    }

    static func run(
        adapter: String,
        disable: () throws -> Void,
        deleteCredential: () throws -> Void,
        deleteRecord: () throws -> Void
    ) throws {
        guard adapter == "yandex_disk" else { throw Failure.unsupportedAdapter }
        // Disable before credential cleanup so that failures remain visible
        // and future executors cannot schedule new work for this connection.
        try disable()
        // Keychain delete treats item-not-found as success, allowing retry.
        try deleteCredential()
        // SQLite deletion happens last. If it fails, the disabled row stays
        // visible and a subsequent attempt can finish the operation.
        try deleteRecord()
    }
}

 
/// Fail-closed authorization commit across Registry and Keychain.
/// Interrupted commits remain requires_auth; automatic refresh must not promote them.
enum ConnectionAuthorization {
    static func commit(
        markRequiresAuth: () throws -> Void,
        storeCredential: () throws -> Void,
        persistIdentity: () throws -> Void,
        persistCredentialReference: () throws -> Void,
        markConnected: () throws -> Void
    ) throws {
        try markRequiresAuth()
        try storeCredential()
        try persistIdentity()
        try persistCredentialReference()
        try markConnected()
    }

    static func mayRefresh(
        enabled: Bool,
        status: String,
        accountID: String?,
        credentialReference: String?
    ) -> Bool {
        enabled && status == "connected"
            && !(accountID ?? "").isEmpty
            && !(credentialReference ?? "").isEmpty
    }
}
