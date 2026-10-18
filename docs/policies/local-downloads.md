# Local Download Policy

Status: **accepted**. This policy is valid only for the **main MacBook** and **QADREN** operations.

## Basic Prohibition

QADREN **does not start the download of cloud file content to the local storage of the main MacBook** without previous explicit agreement from the user. File size is not important. Integrity checks, backup, and recovery are not exceptions.

Before the request, specify the files/set, source, reason, purpose, expected volume, and operation authority if possible. No answer is not agreement. Permission has a limited scope; it does not become a global permanent permission.

## Local File Copy Consent

For one Scenario Local Filesystem file copy or controlled replacement, the Scenario plan can ask the user to allow macOS to materialize the selected source item. The dialog names the materialization risk and states the selected source and destination operation. The Rust JobStore records and consumes one accepted `read_content` decision for the exact source `connection_id` and relative path before it opens the file. A new operation needs a new confirmation. A preview, a metadata read, or a previous confirmation is not consent.

## Native File Provider Uncertainty

QADREN may be unable to prove that a macOS file is resident or that opening it cannot cause File Provider materialization. In that case, describe the possibility as a **potential download**, even if metadata suggests that the file is local. Offer the user a choice to allow that exact content read or cancel. An explicit accepted `read_content` decision permits the attempted read, including possible OS-managed materialization. Lack of a reliable non-hydrating proof is **not, by itself, a release blocker once that risk is disclosed and the required consent is recorded**.

The application must not secretly probe content before that decision. It must still enforce scoped paths and connections, a durable attempt before external effects, and honest UNKNOWN/recovery handling. A refused or missing decision causes no QADREN-initiated content read. A changed source or new operation requires another decision.

## Allowed Automatic Actions

Retrieval of metadata, lists, sizes, dates, identifiers, and available server hashes **without downloading content**. If the API requires a content download to get information, the basic prohibition applies.

## Rule Boundaries

- Do not try to block or change the normal operation of iCloud Drive in macOS.
- Do not prevent other applications from accessing files.
- Do not consider system synchronization already performed by macOS as a violation.
- QADREN reading of a file that is already local, and operations that can cause its materialization, require the separate consent described above.
- On **another Mac designated as a Worker**, different explicitly established policies may apply: it can download data automatically for its tasks.

## Delegation

Agreement for download to the main MacBook is not required for a transfer that another authorized Worker performs independently within its rules. The Controller does not perform the delegated task and does not download its content for re-verification.
