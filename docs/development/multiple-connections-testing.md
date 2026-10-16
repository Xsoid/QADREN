# Multiple Independent Connections — First Working UI

The branch implements D-016 for Yandex Disk using a common Rust SQLite registry. Each `StorageConnection` instance has an independent ID, name, provider account ID, and Keychain secret. No unique constraint for the provider account ID is implemented.

## Local storage
- SQLite: `~/Library/Application Support/QADREN/connections.sqlite3` (metadata only), protected directory; the file is created/marked with 0600 permissions on macOS.
- Keychain: service `dev.qadren.yandex.oauth`, `account=connection_id`, tokens are never written to SQLite. The OAuth Client ID is embedded via local configuration; the Client Secret is not used.
- Each instance has its own `YandexConnection`, OAuth state, PKCE verifier, session, status, account identity, and metadata.

## Migration of previous account
On the first start, the new version detects the old Keychain `account=default` and shows **Import existing connection**. The user does not need to perform OAuth again. During import, the token is copied to a new independent ID; the system verifies the copy read and the credential-ref write; the original `default` key is deleted **only after success**. If the process is interrupted, the existing new record with the same token is reused.

Migration does not change the Yandex Disk content. If the old key cannot be read, the previous connection is not deleted. In case of failed copying, the new incomplete registry can be restored or deleted manually.

## Limitations before final acceptance
- Rust tests for the model and v1→v2 migration are verified; macOS UI, Keychain copy-once, and multiple real accounts **require manual testing**.
- The UI uses a single registered OAuth Client ID for all connections of this provider.
- The UI currently supports only Yandex Disk; other adapters are added to the same registry as they are implemented.
- BackupSet, deleted nodes, and links are not implemented; therefore, a warning about connections used by rules is required before BackupSet implementation.
- Yandex may require the user to explicitly log out or change the account in the browser when authorizing a second connection; this is to be confirmed by manual testing.
- No downloading of file contents, writing, or deleting in the cloud is performed.

## Acceptance
1. Update the application, see the legacy banner, and import the token: verify the same user and quota without repeated OAuth.
2. Add two separate connections, authorize different accounts, and repeat one of them.
3. Verify independent statuses and the Refresh operation for each connection.
4. Delete/disconnect only one connection: verify that others remain active.
5. Restart the application: verify that connected instances are available again.
6. Verify the handling of erroneous/cancelled OAuth, offline state, token preservation in Keychain, and absence of leaks in logs.

## Final security / error review

- No tokens or OAuth codes are written to SQLite, logs, Disk REST API request URLs, or git. The Keychain is linked to `connection_id`.
- Legacy import is performed after manual user confirmation; the previous `default` is deleted only after copying, reading, and saving the reference. Interruption of the process before this point leaves the original token. Import does not capture a random empty record based only on the name.
- During Remove, the connection is first set to disabled. Then the Rust JobStore (read-only) checks for incomplete attempts for this `connection_id`; if the JobStore is unavailable, if there is an active/UNKNOWN attempt, or a legacy attempt without target binding, the cleanup is fail-closed: the record and Keychain credential remain for reconciliation/retry. The credential and local Registry record are deleted only after a clean check. This does not delete cloud data and does not affect adjacent connections.
- Rust HTTP tests responses 200, 401, 403, 429, 503, and timeout via loopback without real clouds and without requesting file contents. 401/403 and 429 are classified separately; automatic request repetition is intentionally disabled; manual Refresh is available. Retry/backoff requires a separate policy before implementing background tasks to avoid violating API limits.
- Remaining limitations: no distributed management of accounts/nodes, iCloud Documents is unavailable, no BackupSet, automatic refresh of an expired OAuth token and token operation after revocation in Yandex were not verified. These functions are not part of the current read-only prototype.
