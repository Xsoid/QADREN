import Foundation

/// Coordinates two independent persistence systems; deliberately does not
/// pretend that SQLite and Keychain share an atomic transaction.
enum ConnectionRemoval {
    enum Failure: Error {
        case unsupportedAdapter
    }

    /// Adapters whose connection owns a Keychain item.
    private static let credentialAdapters: Set<String> = ["yandex_disk", "smb_share", "ispmanager"]

    static func run(
        adapter: String,
        disable: () throws -> Void,
        ensureNoUnresolvedAttempts: () throws -> Void,
        deleteCredential: () throws -> Void,
        deleteRecord: () throws -> Void
    ) throws {
        guard credentialAdapters.contains(adapter) else { throw Failure.unsupportedAdapter }
        // Disable before credential cleanup so that failures remain visible
        // and future executors cannot schedule new work for this connection.
        try disable()
        // If the Job inventory is unavailable or points to an unresolved
        // attempt, fail closed. The disabled record and Keychain credential
        // remain available for recovery and read-only reconciliation.
        try ensureNoUnresolvedAttempts()
        // Keychain delete treats item-not-found as success, allowing retry.
        try deleteCredential()
        // SQLite deletion happens last. If it fails, the disabled row stays
        // visible and a subsequent attempt can finish the operation.
        try deleteRecord()
    }

    /// Local Filesystem has no credential to delete, but its Registry record
    /// is still an endpoint for durable Jobs. Apply the same disable and
    /// unresolved-attempt gate before removing only that local record.
    static func removeLocal(
        disable: () throws -> Void,
        ensureNoUnresolvedAttempts: () throws -> Void,
        deleteRecord: () throws -> Void
    ) throws {
        try disable()
        try ensureNoUnresolvedAttempts()
        try deleteRecord()
    }

    /// Disconnect only local credentials while preserving the connection for a
    /// later explicit reauthorization. As with removal, a failed step leaves
    /// the Registry record disabled so no new work can begin accidentally.
    static func disconnect(
        adapter: String,
        disable: () throws -> Void,
        ensureNoUnresolvedAttempts: () throws -> Void,
        deleteCredential: () throws -> Void,
        finalizeDisconnectedRecord: () throws -> Void
    ) throws {
        guard adapter == "yandex_disk" else { throw Failure.unsupportedAdapter }
        try disable()
        try ensureNoUnresolvedAttempts()
        try deleteCredential()
        try finalizeDisconnectedRecord()
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
