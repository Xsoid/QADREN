# Yandex Disk Adapter

Yandex Disk is post-release. This document defines constraints for a future adapter. It does not authorize Release 1 feature work, OAuth changes, transfer work, or live end-to-end testing.

## Security contract

- Use OAuth Authorization Code with PKCE for a native public client.
- Do not embed or store a client secret in the application.
- Keep each credential in Keychain state bound to its `connection_id`.
- Do not store tokens, authorization headers, or credential-bearing URLs in SQLite, Git, logs, or diagnostics.
- Treat provider acknowledgement as distinct from verified content integrity.

## Future adapter requirements

The adapter must declare capabilities and platform prerequisites truthfully. Before a modifying effect, it must receive a durable Job attempt, exact source and destination scope, capability checks, and required destructive consent. It must support non-mutating reconciliation after uncertain effects and must not blindly retry destructive operations.

Do not treat metadata access, quota display, or a successful API request as proof that transfer or synchronization is available. Return to this work only after an owner decision updates the Release 1 boundary.
