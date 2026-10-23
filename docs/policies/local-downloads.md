# Local Content Read and Materialization Policy

Status: **accepted owner decision, 2026-10-18**. This policy supersedes the earlier per-file download confirmation rule for QADREN file reads on the primary MacBook.

## Read Requested Content Automatically

- When QADREN needs to read a file for an enabled or user-requested operation, it **may read the content without a confirmation dialog**, even if macOS File Provider transparently downloads/materializes the file onto the primary MacBook.
- This file read or incidental materialization is an ordinary execution step, **not a destructive risk requiring human approval**. The former mandatory per-file `read_content` consent is retired. Do not demand a reliable non-hydrating residency proof or a File Provider consent prompt just to open/read the requested source.
- The permission is limited to reading content **actually needed by a configured or requested operation**. It is not permission for unrelated background prefetch, unsolicited mirroring of entire cloud directories, or unrelated content downloads.
- Honor macOS and provider permissions and report actual read/network failures truthfully. Metadata scans may remain metadata-only when file content is not needed. Do not interfere with normal macOS/iCloud or other application sync.

## Separate Risky Effects From Ordinary Reads

- A read that results in a cloud download does **not** imply permission to replace, delete, move, or otherwise destructively modify user data. These effects follow [data-safety.md](data-safety.md).
- When replacing an existing file, explicitly ask the user to confirm the actual affected destination and possible loss of previous content unless that exact destructive scope has already been approved under a durable scenario rule.
- Before a mass deletion proposal, show affected files, count and known consequences, and require explicit confirmation; a general schedule switch is not approval of new mass-deletion effects.
- If an operation cannot be limited to the selected files, report the limitation and the **actual known scope** before execution. Do not describe an overbroad operation as a selected-file action. Obtain approval for the described broader scope if it can be bounded; otherwise explain the inability to enforce an exact boundary.

## File Provider Placeholders

- A Scenario root can be inside a File Provider folder, such as iCloud Drive with "Optimize Mac Storage". Files there can be dataless placeholders.
- A metadata plan does not download placeholders. Only a Create or an Update reads the needed source file, and macOS downloads it as part of that read.
- macOS can show a one-time system prompt for access to files that a provider manages. This is an operating-system permission, not a QADREN read-consent prompt. The user answers it.
- A download or an eviction by macOS changes the file ctime and the dataless flag, but not the content. iCloud Drive also restores the modification time from a double of epoch seconds, which can move it by up to about 120 ns. The Local object revision therefore uses device, inode, and the modification time as that double, not ctime. A copy accepts these changes only for a source that the read downloaded.

## Crash Recovery

- After an interrupted or uncertain effect, QADREN automatically performs available **non-mutating, read-only reconciliation**, including reading required metadata or content. This does not require another confirmation.
- If the effect can be verified, persist the proven result. If evidence is insufficient, retain `UNKNOWN` and explain it. Never silently re-run a possibly destructive action or mark success without evidence.
- A retry that may replace or delete content is a new effect and follows the applicable destructive-operation decision policy.

## Workers

This ordinary-read policy applies to the primary MacBook and local workers. A future remote Worker may have its own approved transfer/retention rules. Cloud adapters Yandex and iCloud remain post-release; this policy does not reactivate them.
