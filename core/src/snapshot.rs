//! Stored, resumable metadata snapshots of Scenario sides (D-024, stage 3).
//!
//! A snapshot is a metadata cache in `snapshots.sqlite3` next to the
//! Registry. It holds relative paths, sizes, MD5 values, and revisions. It
//! never holds file content, a token, or a link. The Worker is the only
//! writer.
//!
//! A walk keeps a queue of folders. Each folder page is written in one
//! transaction: its files, its subfolders, and the offset of the next page.
//! After a crash or a restart the walk continues from the first unwritten
//! page; a page that is read again replaces the same rows. Memory does not
//! grow with the number of files.
//!
//! A plan uses only a complete snapshot. A folder that changes during the
//! walk can move entries between pages; the executor still reads the source
//! metadata of each file again before its effect, and the next walk sees
//! what this one missed.
use crate::local_fs::{LocalError, LocalFilesystem};
use crate::scenario_planner::name_key;
use crate::yandex::{YandexError, YandexMetadataClient};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;

/// Entries in one Yandex Disk folder page.
const YANDEX_PAGE: u32 = 1_000;
/// Local folders listed in one step.
const LOCAL_FOLDERS_PER_STEP: usize = 64;

#[derive(Debug)]
pub enum SnapshotError {
    Database(rusqlite::Error),
    NotFound,
    InvalidRequest,
    /// The walk can continue later: network, HTTP 429 or 5xx, a missing mount.
    Transient(String),
    /// The walk cannot continue without the user (401, 403, a broken root).
    Blocked(String),
}

impl From<rusqlite::Error> for SnapshotError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Database(error)
    }
}

pub type Result<T> = std::result::Result<T, SnapshotError>;

/// What a snapshot walks: one Scenario side with its selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotKey {
    pub connection_id: String,
    /// `yandex_disk`, or `local` for This Mac and an SMB mount.
    pub kind: String,
    /// `disk:/…` for Yandex Disk, an absolute path for a local root.
    pub root: String,
    /// Relative selected paths; empty means the full root.
    pub selection: Vec<String>,
}

impl SnapshotKey {
    fn fingerprint(&self) -> String {
        let mut selection = self.selection.clone();
        selection.sort();
        let text =
            serde_json::json!([self.connection_id, self.kind, self.root, selection]).to_string();
        format!("{:x}", md5::compute(text.as_bytes()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SnapshotInfo {
    pub id: String,
    /// `walking`, `complete`, or `failed`.
    pub state: String,
    pub files: u64,
    pub folders_done: u64,
    pub folders_pending: u64,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    /// Stable code of the last walk error, never a path or a secret.
    pub last_error: Option<String>,
    /// Unix seconds before which the Worker does not continue the walk.
    pub retry_at: Option<i64>,
}

/// One file row of a snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotFile {
    pub relative_path: String,
    pub size: u64,
    /// The planner object revision: the MD5 for Yandex Disk, the local
    /// metadata revision for a local file.
    pub revision: Option<String>,
    pub md5: Option<String>,
    /// A symbolic link or special file that the walk skipped.
    pub special: bool,
}

/// A walking snapshot with the key the Worker needs to continue it.
#[derive(Debug, Clone)]
pub struct WalkingSnapshot {
    pub id: String,
    pub key: SnapshotKey,
    pub retry_at: Option<i64>,
}

struct PendingFolder {
    path: String,
    offset: u64,
    /// A selected object of unknown type: a folder or a file.
    probe: bool,
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs() as i64)
}

fn join(base: &str, name: &str) -> String {
    if base.is_empty() {
        name.to_owned()
    } else {
        format!("{base}/{name}")
    }
}

fn valid_relative(path: &str) -> bool {
    !path.is_empty()
        && !path.contains(|ch: char| ch.is_control())
        && path
            .split('/')
            .all(|segment| !segment.is_empty() && segment != "." && segment != "..")
}

pub struct SnapshotStore {
    db: Connection,
}

impl SnapshotStore {
    /// The snapshot database next to the Registry.
    pub fn path_for(registry_path: &Path) -> std::path::PathBuf {
        registry_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("snapshots.sqlite3")
    }

    pub fn open(path: &Path) -> Result<Self> {
        let db = Connection::open(path)?;
        db.busy_timeout(std::time::Duration::from_secs(30))?;
        // Readers (the app) never block the writer (the Worker).
        db.pragma_update(None, "journal_mode", "WAL")?;
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS snapshots (
                id TEXT PRIMARY KEY NOT NULL,
                fingerprint TEXT NOT NULL,
                connection_id TEXT NOT NULL,
                kind TEXT NOT NULL,
                root TEXT NOT NULL,
                selection TEXT NOT NULL,
                state TEXT NOT NULL CHECK(state IN ('walking','complete','failed')),
                files INTEGER NOT NULL DEFAULT 0,
                started_at INTEGER NOT NULL,
                finished_at INTEGER,
                last_error TEXT,
                retry_at INTEGER,
                failures INTEGER NOT NULL DEFAULT 0
             );
             CREATE INDEX IF NOT EXISTS snapshots_by_key ON snapshots(fingerprint, state);
             CREATE TABLE IF NOT EXISTS snapshot_folders (
                snapshot_id TEXT NOT NULL REFERENCES snapshots(id) ON DELETE CASCADE,
                path TEXT NOT NULL,
                next_offset INTEGER NOT NULL DEFAULT 0,
                probe INTEGER NOT NULL DEFAULT 0,
                done INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY(snapshot_id, path)
             ) WITHOUT ROWID;
             CREATE INDEX IF NOT EXISTS snapshot_folders_pending ON snapshot_folders(snapshot_id, done);
             CREATE TABLE IF NOT EXISTS snapshot_files (
                snapshot_id TEXT NOT NULL REFERENCES snapshots(id) ON DELETE CASCADE,
                relative_path TEXT NOT NULL,
                name_key TEXT NOT NULL,
                size INTEGER NOT NULL,
                revision TEXT,
                md5 TEXT,
                special INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY(snapshot_id, relative_path)
             ) WITHOUT ROWID;
             CREATE INDEX IF NOT EXISTS snapshot_files_by_key ON snapshot_files(snapshot_id, name_key);",
        )?;
        db.pragma_update(None, "foreign_keys", "ON")?;
        Ok(Self { db })
    }

    /// The connection, for the plan store in the same database.
    pub fn into_connection(self) -> Connection {
        self.db
    }

    pub fn connection(&self) -> &Connection {
        &self.db
    }

    /// Open for status reads only: no schema statements and no journal-mode
    /// change, which can wait for the write lock of the walking thread. The
    /// IPC thread uses this so that it always answers (D-024).
    pub fn open_for_reading(path: &Path) -> Result<Self> {
        let db = Connection::open(path)?;
        db.busy_timeout(std::time::Duration::from_secs(2))?;
        Ok(Self { db })
    }

    /// Start a walk of `key`, or return the walk of the same key that is in
    /// progress. A new walk keeps the last complete snapshot until it ends.
    pub fn start(&self, key: &SnapshotKey) -> Result<String> {
        if key.connection_id.is_empty() || key.root.is_empty() {
            return Err(SnapshotError::InvalidRequest);
        }
        if key.selection.iter().any(|path| !valid_relative(path)) {
            return Err(SnapshotError::InvalidRequest);
        }
        let fingerprint = key.fingerprint();
        let walking: Option<String> = self
            .db
            .query_row(
                "SELECT id FROM snapshots WHERE fingerprint=?1 AND state='walking'",
                [&fingerprint],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(id) = walking {
            return Ok(id);
        }
        let tx = self.db.unchecked_transaction()?;
        let id: String = tx.query_row("SELECT lower(hex(randomblob(16)))", [], |row| row.get(0))?;
        tx.execute(
            "INSERT INTO snapshots(id,fingerprint,connection_id,kind,root,selection,state,started_at)
             VALUES(?1,?2,?3,?4,?5,?6,'walking',?7)",
            params![
                id,
                fingerprint,
                key.connection_id,
                key.kind,
                key.root,
                serde_json::to_string(&key.selection).unwrap_or_else(|_| "[]".into()),
                unix_now()
            ],
        )?;
        if key.selection.is_empty() {
            tx.execute(
                "INSERT INTO snapshot_folders(snapshot_id,path,probe) VALUES(?1,'',0)",
                [&id],
            )?;
        } else {
            for path in &key.selection {
                tx.execute(
                    "INSERT OR IGNORE INTO snapshot_folders(snapshot_id,path,probe) VALUES(?1,?2,1)",
                    params![id, path],
                )?;
            }
        }
        tx.commit()?;
        Ok(id)
    }

    pub fn info(&self, id: &str) -> Result<Option<SnapshotInfo>> {
        let row = self
            .db
            .query_row(
                "SELECT state,files,started_at,finished_at,last_error,retry_at FROM snapshots WHERE id=?1",
                [id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)? as u64,
                        row.get::<_, i64>(2)?,
                        row.get::<_, Option<i64>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, Option<i64>>(5)?,
                    ))
                },
            )
            .optional()?;
        let Some((state, mut files, started_at, finished_at, last_error, retry_at)) = row else {
            return Ok(None);
        };
        if state == "walking" {
            files = self.db.query_row(
                "SELECT count(*) FROM snapshot_files WHERE snapshot_id=?1",
                [id],
                |row| row.get::<_, i64>(0),
            )? as u64;
        }
        let (done, pending): (i64, i64) = self.db.query_row(
            "SELECT coalesce(sum(done),0), coalesce(sum(1-done),0) FROM snapshot_folders WHERE snapshot_id=?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        Ok(Some(SnapshotInfo {
            id: id.to_owned(),
            state,
            files,
            folders_done: done as u64,
            folders_pending: pending as u64,
            started_at,
            finished_at,
            last_error,
            retry_at,
        }))
    }

    /// The newest snapshot of `key`, in any state.
    pub fn latest(&self, key: &SnapshotKey) -> Result<Option<SnapshotInfo>> {
        let id: Option<String> = self
            .db
            .query_row(
                "SELECT id FROM snapshots WHERE fingerprint=?1 ORDER BY started_at DESC, rowid DESC LIMIT 1",
                [key.fingerprint()],
                |row| row.get(0),
            )
            .optional()?;
        match id {
            Some(id) => self.info(&id),
            None => Ok(None),
        }
    }

    /// The newest complete snapshot of `key`, for a plan.
    pub fn latest_complete(&self, key: &SnapshotKey) -> Result<Option<String>> {
        Ok(self
            .db
            .query_row(
                "SELECT id FROM snapshots WHERE fingerprint=?1 AND state='complete'
                 ORDER BY finished_at DESC LIMIT 1",
                [key.fingerprint()],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// Walks that continue now: not waiting for a retry time.
    pub fn walking(&self, now: i64) -> Result<Vec<WalkingSnapshot>> {
        let mut statement = self.db.prepare(
            "SELECT id,connection_id,kind,root,selection,retry_at FROM snapshots
             WHERE state='walking' AND (retry_at IS NULL OR retry_at<=?1) ORDER BY started_at",
        )?;
        let rows = statement
            .query_map([now], |row| {
                let selection: String = row.get(4)?;
                Ok(WalkingSnapshot {
                    id: row.get(0)?,
                    key: SnapshotKey {
                        connection_id: row.get(1)?,
                        kind: row.get(2)?,
                        root: row.get(3)?,
                        selection: serde_json::from_str(&selection).unwrap_or_default(),
                    },
                    retry_at: row.get(5)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Files of a snapshot in name-key order, one page after `after_key`.
    pub fn files_page(&self, id: &str, after_key: &str, limit: usize) -> Result<Vec<SnapshotFile>> {
        let mut statement = self.db.prepare(
            "SELECT relative_path,size,revision,md5,special FROM snapshot_files
             WHERE snapshot_id=?1 AND name_key>?2 ORDER BY name_key, relative_path LIMIT ?3",
        )?;
        let rows = statement
            .query_map(params![id, after_key, limit as i64], |row| {
                Ok(SnapshotFile {
                    relative_path: row.get(0)?,
                    size: row.get::<_, i64>(1)? as u64,
                    revision: row.get(2)?,
                    md5: row.get(3)?,
                    special: row.get(4)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Delete old snapshots. Keeps, for each key, the newest snapshot in any
    /// state (a failed one keeps its error and its progress) and the newest
    /// complete one (the plan source).
    pub fn prune(&self) -> Result<usize> {
        Ok(self.db.execute(
            "DELETE FROM snapshots WHERE state!='walking'
               AND id NOT IN (SELECT id FROM snapshots s WHERE started_at=(
                   SELECT max(started_at) FROM snapshots t WHERE t.fingerprint=s.fingerprint))
               AND id NOT IN (SELECT id FROM snapshots s WHERE state='complete' AND finished_at=(
                   SELECT max(finished_at) FROM snapshots t
                   WHERE t.fingerprint=s.fingerprint AND t.state='complete'))",
            [],
        )?)
    }

    fn pending_folders(&self, id: &str, limit: usize) -> Result<Vec<PendingFolder>> {
        let mut statement = self.db.prepare(
            "SELECT path,next_offset,probe FROM snapshot_folders
             WHERE snapshot_id=?1 AND done=0 ORDER BY path LIMIT ?2",
        )?;
        let rows = statement
            .query_map(params![id, limit as i64], |row| {
                Ok(PendingFolder {
                    path: row.get(0)?,
                    offset: row.get::<_, i64>(1)? as u64,
                    probe: row.get(2)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Write one folder page in one transaction. `next_offset` is `None` when
    /// the folder is finished.
    fn record_page(
        &self,
        id: &str,
        folder: &str,
        files: &[SnapshotFile],
        subfolders: &[String],
        next_offset: Option<u64>,
    ) -> Result<()> {
        let tx = self.db.unchecked_transaction()?;
        {
            let mut insert_file = tx.prepare_cached(
                "INSERT OR REPLACE INTO snapshot_files(snapshot_id,relative_path,name_key,size,revision,md5,special)
                 VALUES(?1,?2,?3,?4,?5,?6,?7)",
            )?;
            for file in files {
                insert_file.execute(params![
                    id,
                    file.relative_path,
                    name_key(&file.relative_path),
                    file.size as i64,
                    file.revision,
                    file.md5,
                    file.special
                ])?;
            }
            let mut insert_folder = tx.prepare_cached(
                "INSERT OR IGNORE INTO snapshot_folders(snapshot_id,path) VALUES(?1,?2)",
            )?;
            for subfolder in subfolders {
                insert_folder.execute(params![id, subfolder])?;
            }
        }
        match next_offset {
            Some(offset) => tx.execute(
                "UPDATE snapshot_folders SET next_offset=?3, probe=0 WHERE snapshot_id=?1 AND path=?2",
                params![id, folder, offset as i64],
            )?,
            None => tx.execute(
                "UPDATE snapshot_folders SET done=1, probe=0 WHERE snapshot_id=?1 AND path=?2",
                params![id, folder],
            )?,
        };
        // A count after every page would be quadratic; `info` counts.
        tx.execute(
            "UPDATE snapshots SET last_error=NULL, retry_at=NULL, failures=0 WHERE id=?1",
            [id],
        )?;
        tx.commit()?;
        Ok(())
    }

    fn finish_if_done(&self, id: &str) -> Result<bool> {
        let pending: i64 = self.db.query_row(
            "SELECT count(*) FROM snapshot_folders WHERE snapshot_id=?1 AND done=0",
            [id],
            |row| row.get(0),
        )?;
        if pending > 0 {
            return Ok(false);
        }
        self.db.execute(
            "UPDATE snapshots SET state='complete', finished_at=?2,
                    files=(SELECT count(*) FROM snapshot_files WHERE snapshot_id=?1)
             WHERE id=?1 AND state='walking'",
            params![id, unix_now()],
        )?;
        Ok(true)
    }

    /// Record a transient walk error and the time of the next attempt:
    /// 30 s, then doubled up to 30 min.
    pub fn delay(&self, id: &str, code: &str) -> Result<i64> {
        let failures: i64 = self.db.query_row(
            "UPDATE snapshots SET failures=failures+1 WHERE id=?1 RETURNING failures",
            [id],
            |row| row.get(0),
        )?;
        let delay = (30_i64 << (failures - 1).clamp(0, 6)).min(1_800);
        let retry_at = unix_now() + delay;
        self.db.execute(
            "UPDATE snapshots SET last_error=?2, retry_at=?3 WHERE id=?1",
            params![id, code, retry_at],
        )?;
        Ok(retry_at)
    }

    pub fn fail(&self, id: &str, code: &str) -> Result<()> {
        self.db.execute(
            "UPDATE snapshots SET state='failed', last_error=?2, finished_at=?3 WHERE id=?1",
            params![id, code, unix_now()],
        )?;
        Ok(())
    }
}

/// The result of one walk step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StepOutcome {
    pub pages: usize,
    pub complete: bool,
}

fn yandex_failure(error: &YandexError) -> SnapshotError {
    let code = match error {
        YandexError::Unauthorized => "yandex_unauthorized",
        YandexError::Forbidden => "yandex_forbidden",
        YandexError::RateLimited => "yandex_rate_limited",
        YandexError::Transport(_) => "yandex_network_error",
        YandexError::Provider(_) | YandexError::ContentUnavailable(_) => "yandex_provider_error",
        YandexError::InvalidPath => "yandex_invalid_path",
        YandexError::InvalidToken => "yandex_token_required",
        _ => "yandex_provider_error",
    };
    match error {
        YandexError::Transport(_)
        | YandexError::RateLimited
        | YandexError::Provider(_)
        | YandexError::ContentUnavailable(_) => SnapshotError::Transient(code.into()),
        _ => SnapshotError::Blocked(code.into()),
    }
}

/// A file row. A file without an MD5 (for example, one that is still being
/// uploaded) has no revision; the plan then needs a verification for it.
fn yandex_file(relative_path: String, item: &crate::yandex::Resource) -> SnapshotFile {
    let md5 = item
        .md5
        .as_deref()
        .filter(|md5| md5.len() == 32 && md5.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .map(str::to_ascii_lowercase);
    SnapshotFile {
        relative_path,
        size: item.size.unwrap_or(0),
        revision: md5.clone(),
        md5,
        special: false,
    }
}

/// A name that QADREN cannot use as one path segment: empty, `.`, `..`, a
/// slash, or a control character. The walk keeps it as a skipped object
/// that the plan discloses; it never stops the walk (2026-10-25: one such
/// name stopped a 48-minute walk).
fn unusable_name(name: &str) -> bool {
    name.is_empty()
        || name == "."
        || name == ".."
        || name.contains('/')
        || name.contains(|ch: char| ch.is_control())
}

fn skipped(relative_path: String) -> SnapshotFile {
    SnapshotFile {
        relative_path,
        size: 0,
        revision: None,
        md5: None,
        special: true,
    }
}

/// Folders that one Yandex step takes from the queue.
const YANDEX_FOLDERS_PER_STEP: usize = 64;

/// Read up to 64 pending folder pages with `parallel` requests in flight at
/// all times, and write each page as soon as it arrives. A page error stops
/// the step; pages already written stay.
pub fn walk_yandex_step(
    store: &SnapshotStore,
    client: &YandexMetadataClient,
    token: &str,
    id: &str,
    root: &str,
    parallel: usize,
) -> Result<StepOutcome> {
    let folders = store.pending_folders(id, YANDEX_FOLDERS_PER_STEP)?;
    if folders.is_empty() {
        return Ok(StepOutcome {
            pages: 0,
            complete: store.finish_if_done(id)?,
        });
    }
    let remote = |relative: &str| {
        if relative.is_empty() {
            root.to_owned()
        } else if root.ends_with('/') {
            format!("{root}{relative}")
        } else {
            format!("{root}/{relative}")
        }
    };
    let queue = std::sync::Mutex::new(std::collections::VecDeque::from(folders));
    let stop = std::sync::atomic::AtomicBool::new(false);
    let mut pages = 0;
    let mut failure = None;
    std::thread::scope(|scope| -> Result<()> {
        let (sender, receiver) = std::sync::mpsc::channel();
        for _ in 0..parallel.max(1) {
            let sender = sender.clone();
            let (queue, stop, remote) = (&queue, &stop, &remote);
            scope.spawn(move || loop {
                if stop.load(std::sync::atomic::Ordering::SeqCst) {
                    break;
                }
                let Some(folder) = queue.lock().ok().and_then(|mut queue| queue.pop_front()) else {
                    break;
                };
                let result = client.list_folder_page(
                    token,
                    &remote(&folder.path),
                    folder.offset,
                    YANDEX_PAGE,
                );
                if sender.send((folder, result)).is_err() {
                    break;
                }
            });
        }
        drop(sender);
        for (folder, result) in receiver {
            if failure.is_some() {
                continue;
            }
            let page = match result {
                Ok(page) => page,
                // A selected object or a folder that is gone now has no files.
                Err(YandexError::NotFound) => {
                    store.record_page(id, &folder.path, &[], &[], None)?;
                    pages += 1;
                    continue;
                }
                Err(error) => {
                    stop.store(true, std::sync::atomic::Ordering::SeqCst);
                    failure = Some(yandex_failure(&error));
                    continue;
                }
            };
            if folder.probe && page.kind == "file" {
                let file = yandex_file(folder.path.clone(), &page);
                store.record_page(id, &folder.path, &[file], &[], None)?;
                pages += 1;
                continue;
            }
            let Some(list) = page.embedded else {
                stop.store(true, std::sync::atomic::Ordering::SeqCst);
                failure = Some(SnapshotError::Transient(
                    "yandex_incomplete_metadata".into(),
                ));
                continue;
            };
            let count = list.items.len() as u64;
            let mut files = Vec::new();
            let mut subfolders = Vec::new();
            for item in &list.items {
                let child = join(&folder.path, &item.name);
                if unusable_name(&item.name) {
                    files.push(skipped(child));
                    continue;
                }
                match item.kind.as_str() {
                    "dir" => subfolders.push(child),
                    "file" => files.push(yandex_file(child, item)),
                    _ => files.push(skipped(child)),
                }
            }
            let next = folder.offset + count;
            let finished = count == 0 || next >= list.total;
            store.record_page(
                id,
                &folder.path,
                &files,
                &subfolders,
                (!finished).then_some(next),
            )?;
            pages += 1;
        }
        Ok(())
    })?;
    if let Some(failure) = failure {
        return Err(failure);
    }
    Ok(StepOutcome {
        pages,
        complete: store.finish_if_done(id)?,
    })
}

/// List up to 64 pending local folders and write each one. The caller has
/// checked that the root is reachable (for SMB: the mount is present).
pub fn walk_local_step(store: &SnapshotStore, id: &str, root: &Path) -> Result<StepOutcome> {
    let filesystem = LocalFilesystem::open(root)
        .map_err(|_| SnapshotError::Transient("local_root_unavailable".into()))?;
    let folders = store.pending_folders(id, LOCAL_FOLDERS_PER_STEP)?;
    let mut pages = 0;
    for folder in folders {
        let relative = Path::new(&folder.path);
        if folder.probe {
            match filesystem.stat(relative) {
                Ok(entry) if !entry.is_directory => {
                    let file = SnapshotFile {
                        relative_path: folder.path.clone(),
                        size: entry.size,
                        revision: entry.object_revision,
                        md5: None,
                        special: false,
                    };
                    store.record_page(id, &folder.path, &[file], &[], None)?;
                    pages += 1;
                    continue;
                }
                Ok(_) => {}
                Err(LocalError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                    store.record_page(id, &folder.path, &[], &[], None)?;
                    pages += 1;
                    continue;
                }
                Err(LocalError::UnsafeEntry) => {
                    // A selected symbolic link or special file.
                    let file = SnapshotFile {
                        relative_path: folder.path.clone(),
                        size: 0,
                        revision: None,
                        md5: None,
                        special: true,
                    };
                    store.record_page(id, &folder.path, &[file], &[], None)?;
                    pages += 1;
                    continue;
                }
                Err(_) => {
                    return Err(SnapshotError::Transient(
                        "local_metadata_unavailable".into(),
                    ))
                }
            }
        }
        let (entries, skipped) = match filesystem.list_with_skipped(relative) {
            Ok(listing) => listing,
            Err(LocalError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                store.record_page(id, &folder.path, &[], &[], None)?;
                pages += 1;
                continue;
            }
            Err(_) => {
                return Err(SnapshotError::Transient(
                    "local_metadata_unavailable".into(),
                ))
            }
        };
        let mut files = Vec::new();
        let mut subfolders = Vec::new();
        for entry in entries {
            let child = join(&folder.path, &entry.name);
            if entry.is_directory {
                subfolders.push(child);
            } else {
                files.push(SnapshotFile {
                    relative_path: child,
                    size: entry.size,
                    revision: entry.object_revision,
                    md5: None,
                    special: false,
                });
            }
        }
        for name in skipped {
            files.push(SnapshotFile {
                relative_path: join(&folder.path, &name),
                size: 0,
                revision: None,
                md5: None,
                special: true,
            });
        }
        store.record_page(id, &folder.path, &files, &subfolders, None)?;
        pages += 1;
    }
    Ok(StepOutcome {
        pages,
        complete: store.finish_if_done(id)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_walk_resumes_and_completes() {
        let base = std::env::temp_dir().join(format!("qadren-snapshot-{}", std::process::id()));
        let root = base.join("root");
        std::fs::create_dir_all(root.join("a/b")).unwrap();
        std::fs::write(root.join("a/b/f.txt"), b"x").unwrap();
        std::fs::write(root.join("top.txt"), b"yy").unwrap();
        let root = std::fs::canonicalize(root).unwrap();
        let store = SnapshotStore::open(&base.join("snapshots.sqlite3")).unwrap();
        let key = SnapshotKey {
            connection_id: "c".into(),
            kind: "local".into(),
            root: root.to_string_lossy().into_owned(),
            selection: vec![],
        };
        let id = store.start(&key).unwrap();
        assert_eq!(store.start(&key).unwrap(), id, "a walking key resumes");
        while !walk_local_step(&store, &id, &root).unwrap().complete {}
        let info = store.info(&id).unwrap().unwrap();
        assert_eq!(info.state, "complete");
        assert_eq!(info.files, 2);
        let files = store.files_page(&id, "", 10).unwrap();
        assert_eq!(files.len(), 2);
        std::fs::remove_dir_all(base).unwrap();
    }
}
