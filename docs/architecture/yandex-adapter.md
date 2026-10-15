# Yandex Disk: metadata and limited upload contract

Status: metadata/OAuth integration and limited streaming upload contract exist, but full transfer and live E2E are not accepted. Issue #16 (partial).

## Basis
Official documents:
- REST API: https://yandex.com/dev/disk/rest/
- OAuth authorization code + PKCE: https://yandex.ru/dev/id/doc/ru/codes/code-url
- Application registration: https://yandex.ru/dev/id/doc/en/register-client

## Selected model
- OAuth Authorization Code with PKCE S256, system browser, and state parameter protection.
- For native public client: **do not store client_secret in the application**; Yandex documents the possibility of PKCE without a secret.
- Client ID / redirect URI must be registered and confirmed before full OAuth.
- The token is stored in the Keychain; the Rust client receives it from the platform layer only for the duration of the API operation. Do not store the token in Git, SQLite, or logs.
- Request read permissions first; verify the exact scope in the Yandex OAuth cabinet before adding authorization.

## Implementation status
- `YandexMetadataClient` in Rust via HTTPS, 15-second timeout.
- `GET /v1/disk/` (quota) and `GET /v1/disk/resources` (pagination, metadata).
- `StorageAdapter::capabilities()`: metadata=true, read_content=false, write_content=false.
- Explicit distinction between 401/403/404/429 errors and other unsuccessful HTTP responses.
- For a new remote object, there is an isolated `upload_from_reader`: it first obtains an upload URL with `overwrite=false`, accepts only HTTPS `PUT` without URL credentials, and does not follow redirects. It does not open the local file itself, does not perform retries, and distinguishes between provider `Accepted`/`Pending` and a confirmed result.
- `reconcile_uploaded_resource` reads only the metadata of the specified resource and matches the exact path, size, and valid provider MD5. Incomplete/incorrect expectations result in `InsufficientEvidence`; the method does not write data, does not repeat the upload, and does not change the Job.
- No download, delete, move, rename, copy, or completed Job execution path.
- Auxiliary unit tests with JSON fixtures, without real network requests.

## Limitations and verification steps
- Upload is not connected to JobStore/FFI/SwiftUI and does not permit the start of a transfer. Before upload, durable bound attempt, source/destination consent, and capability checks are required.
- No download: on the main MacBook, it remains prohibited without separate consent for a specific operation.
- Token refresh/re-authorization and real upload/reconciliation must be verified on a test account. API responses and metadata cannot be used as live integrity evidence until this verification is complete.

## UI under verification: quota and Disk root

PR #4 adds C ABI functions for **read-only** Rust requests:
- `qadren_yandex_disk_info`: `GET /v1/disk/` — volume.
- `qadren_yandex_list_root`: `GET /v1/disk/resources?path=disk:/&limit=50&offset=0` — first 50 entries of the root.
- `qadren_yandex_free_string` frees the JSON response received from Rust.

Swift passes the token from the Keychain to Rust **in memory**, starts a blocking request not in the UI thread, and displays the total/used/free volume and the list of names. API responses and secrets are not logged. Errors are passed as safe codes, for example: `unauthorized`, `forbidden`, `network_error`.

Important: this is only the first page of the catalog, not the full tree and not confirmation of a backup. For verification on MacBook: update the branch, build the application, log in (or use Keychain), verify that the quota and names appear, and press Refresh. Do not send tokens or private file names in logs/screenshots to GitHub without masking.
