# Yandex Disk Adapter

[D-023](../decisions/D-023-yandex-disk-to-mac.md) owns the scope. Stage 1: a one-way Scenario with a Yandex Disk source and a Local Filesystem destination. Stage 2: Yandex Disk as a destination and two-way Scenarios.

## Security contract

- Use OAuth Authorization Code with PKCE for a native public client.
- Do not embed or store a client secret in the application.
- Keep each credential in Keychain state bound to its `connection_id`.
- Do not store tokens, authorization headers, signed transfer links, or credential-bearing URLs in SQLite, Git, logs, or diagnostics.
- Stage 1 requests only `cloud_api:disk.read`.
- Treat provider acknowledgement as distinct from verified content integrity.

## Download (stage 1)

- `GET /resources/download` returns a signed link. Accept only method `GET` and scheme `https`.
- Send no OAuth header to the link host. Follow at most one redirect, to an `https` host under `yandex.net` or `yandex.ru`.
- Stream the body and compute MD5. A byte count or MD5 that differs from the provider metadata is a failed download.
- On 2026-10-24 the storage host returned HTTP 500 for some files after the October 2026 incident, while the link request succeeded and other files of the same folder downloaded. The adapter reports `ContentUnavailable`; the file stays in the plan, and nothing is published.
- The Local Filesystem executor owns the destination effect: durable attempt, staging name, no-overwrite publish or the approved replacement protocol, and local-only reconciliation.

## Snapshot

Large transfers follow [D-024](../decisions/D-024-large-transfers.md): a resumable, stored snapshot, at most 4 parallel metadata requests and 2 parallel downloads for each connection, and a pause on HTTP 429.


List the source folder recursively through bounded `GET /resources` pages. A file gives its relative path, size, MD5, and modification time. A listing error fails the whole snapshot; a partial snapshot must never produce a plan.

## Capability rules

The adapter must declare capabilities and platform prerequisites truthfully. Do not treat metadata access, quota display, or a successful API request as proof that a transfer is available. Before a modifying effect on Yandex Disk (stage 2), the adapter must receive a durable Job attempt, exact source and destination scope, capability checks, and required destructive consent. It must support non-mutating reconciliation after uncertain effects and must not blindly retry destructive operations.
