# D-016 — Unlimited number of independent connections to one adapter

Status: **accepted**, 2026-10-08.

## Solution
One adapter type (`yandex_disk`, `s3`, `webdav`, `icloud_documents`, etc.) can be used by **an unlimited number of independent connection instances** (`StorageConnection`) within the limits of the specific provider/OS. You must not introduce artificial limits, singletons, or uniqueness constraints based on adapter type or account.

**Adapter** is the implementation of the protocol and capabilities. **Connection** is a separate configuration instance with a constant `connection_id`, a provider account identifier (if applicable), a name, a state, and its own credentials. **Source or destination** is the role of the connection in a specific `Scenario`, not a separate account type. One `StorageConnection` can be part of several Scenarios and have both roles.

**Controller** is the logical role of a *QADREN node* when it assigns and coordinates tasks. A data source is not automatically a Controller node: these are different entities. One Controller manages many connections and Workers; it does not "own" a single cloud.

## Isolation Rules
- Two connections can have the same `adapter`, provider account ID, URL/endpoint, and even the same external credentials; this is not a reason to merge them or delete them automatically.
- The UI can warn about duplicates, but the user decides.
- OAuth `state`, PKCE verifier, session, Keychain credential, status update, and errors are linked to a **specific connection_id**, not to a global `default`.
- The deletion/disconnection of one connection does not reset the state of adjacent connections and **does not change remote data**; delete from the Keychain only the secret of that specific record after a safe reference check.
- A `Scenario` refers to the `connection_id` of the source and destination; changing the connection name does not change the link. The deletion of a connection in use must be blocked or require an explicit change to the affected rules.
- The location, capabilities, and rights of a connection relate to a specific **execution node**; a Worker cloud token must not be copied automatically to the Controller or other nodes. The inter-node access protocol is deferred to D-010.
- For macOS iCloud Documents, availability depends on the system Apple ID/container: "unlimited connection instances" do not promise the technical capability to authorize several Apple IDs independently in one user session. The adapter must declare limitations honestly.

## Acceptance
- Two different Yandex Disk accounts and two separate connections of **the same account** coexist; they have independent statuses, Keychains, and actions.
- The disconnection of one instance does not affect another.
- After a restart, all instances are restored.
- There are no uniqueness constraints for `adapter`, `provider_account_id`, or their pair.
- Source/destination rules refer to the connection instance; Controller/Worker separately.
- Credential migration preserves the old credential until the new connection-bound credential is verified.
- A needed content read does not require a separate consent prompt. Destructive effects follow the data-safety policy.
