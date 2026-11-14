//! ISPmanager to Mac Backup/export effects (D-027 sections 5 and 7a).
//!
//! The source is a server file, read through the SFTP session of the
//! connection. The destination is a Local Filesystem root (This Mac or an
//! SMB mount), so the rules of the local executor apply: durable attempt,
//! no-overwrite publish for a new file, the approved replacement protocol for
//! an update, and local-only reconciliation. SFTP gives no checksum, so the
//! executor gets the MD5 before the attempt from `md5sum` on the server or,
//! without a shell, from one SFTP read. The download must give the same
//! bytes before the publish.
use crate::confirmation::Confirmation;
use crate::ispmanager_sftp::{SftpConnection, SftpReadError, SftpReader};
use crate::job_execution::{
    enabled_local_connection, local_root_identity_key, matches_conflict_snapshot,
    require_selected_path, run_backup_mirror_delete, run_backup_mirror_delete_folder,
    scenario_update_destination, valid_job_id, LocalDirectoryCreateError, LocalFileCopyError,
    LocalFileCopyStatus, LocalRuleDeleteStatus, SftpFailure,
};
use crate::jobs::{JobStore, ScenarioCopyBaselineOutboxEntry};
use crate::local_fs::{LocalError, LocalFilesystem};
use crate::model::{ConfirmedFileVersion, JobState, ScenarioReference, ScopeMatch};
use crate::registry::{Registry, Scenario};
use crate::scenario_copy_dispatch::dispatch_pending_scenario_copy_baselines;
use crate::scenario_planner::name_key;
use crate::storage_access::ISPMANAGER;
use crate::verification::VerifiedReceipt;
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

fn plain_relative(path: &str) -> bool {
    !path.is_empty()
        && !path.contains(|ch: char| ch.is_control())
        && Path::new(path)
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
}

/// The server path of a Scenario path: the source root is the home folder
/// of the account (D-027 section 6).
fn server_path(root: &str, relative: &str) -> String {
    format!("{}/{relative}", root.trim_end_matches('/'))
}

fn read_failure(error: SftpReadError) -> LocalFileCopyError {
    LocalFileCopyError::Sftp(match error {
        SftpReadError::NotFound => SftpFailure::NotFound,
        SftpReadError::PermissionDenied => SftpFailure::PermissionDenied,
        SftpReadError::Failed(_) => SftpFailure::Network,
    })
}

/// The Backup/export revision with an enabled ISPmanager source.
fn backup_scenario(
    registry: &Registry,
    scenario_id: &str,
    scenario_revision: u64,
) -> Result<Scenario, LocalFileCopyError> {
    let scenario = registry
        .get_scenario_revision(scenario_id, scenario_revision)
        .map_err(|_| LocalFileCopyError::ScenarioUnavailable)?
        .ok_or(LocalFileCopyError::ScenarioUnavailable)?;
    let mode = registry
        .scenario_mode(scenario_id)
        .map_err(|_| LocalFileCopyError::ScenarioUnavailable)?;
    if mode != "backup_export"
        || scenario.direction != "left_to_right"
        || !scenario.left_root.starts_with('/')
    {
        return Err(LocalFileCopyError::ScenarioScopeMismatch);
    }
    let source = registry
        .get(&scenario.left_connection_id)
        .ok()
        .flatten()
        .filter(|connection| connection.enabled && connection.adapter == ISPMANAGER);
    if source.is_none() {
        return Err(LocalFileCopyError::ConnectionUnavailable);
    }
    Ok(scenario)
}

/// A reader that fails at the end when the bytes differ from the first read.
/// The error comes before the publish, so nothing is published.
struct VerifyingReader<'a> {
    inner: SftpReader<'a>,
    context: md5::Context,
    total: u64,
    expected_size: u64,
    expected_md5: String,
    verified: bool,
}

impl Read for VerifyingReader<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let count = self.inner.read(buffer)?;
        if count == 0 {
            let md5 = format!("{:x}", self.context.clone().compute());
            if self.total != self.expected_size || md5 != self.expected_md5 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "source changed during the download",
                ));
            }
            self.verified = true;
            return Ok(0);
        }
        self.context.consume(&buffer[..count]);
        self.total += count as u64;
        if self.total > self.expected_size {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "source grew during the download",
            ));
        }
        Ok(count)
    }
}

/// Download one server file of a Backup/export Scenario into its Local
/// destination. `replacement` is `None` for a Create. For an Update it holds
/// the planned source and destination versions.
#[allow(clippy::too_many_arguments)]
pub fn run_ispmanager_scenario_file_download(
    database_path: &Path,
    registry_path: &Path,
    scenario_id: &str,
    scenario_revision: u64,
    relative_path: &str,
    job_id: &str,
    connection: &SftpConnection,
    replacement: Option<(&ConfirmedFileVersion, &ConfirmedFileVersion)>,
) -> Result<LocalFileCopyStatus, LocalFileCopyError> {
    if !valid_job_id(job_id) || !plain_relative(relative_path) {
        return Err(LocalFileCopyError::InvalidRequest);
    }
    let registry =
        Registry::open(registry_path).map_err(|_| LocalFileCopyError::ScenarioUnavailable)?;
    let scenario = backup_scenario(&registry, scenario_id, scenario_revision)?;
    let destination_connection = scenario.right_connection_id.clone();
    if !enabled_local_connection(registry_path, &destination_connection) {
        return Err(LocalFileCopyError::ConnectionUnavailable);
    }
    require_selected_path(
        &registry,
        scenario_id,
        scenario_revision,
        Path::new(relative_path),
    )?;
    drop(registry);

    // Read the source metadata again. The plan can be old.
    let server_path = server_path(&scenario.left_root, relative_path);
    let meta = connection
        .lstat(&server_path)
        .map_err(read_failure)?
        .filter(|meta| meta.kind == "file")
        .ok_or(LocalFileCopyError::Sftp(SftpFailure::NotFound))?;
    if let Some((expected_source, _)) = replacement {
        if meta.size != expected_source.size
            || Some(meta.revision()) != expected_source.object_revision
        {
            return Err(LocalFileCopyError::ConflictStale);
        }
    }
    // One hash command when the account has a shell; otherwise one SFTP
    // read. The download checks both values before the publish.
    let (size, md5) = match connection.remote_md5(&server_path) {
        Some(md5) => (meta.size, md5),
        None => connection.md5(&server_path).map_err(read_failure)?,
    };
    if size != meta.size {
        return Err(LocalFileCopyError::Sftp(SftpFailure::Changed));
    }
    let source_version = ConfirmedFileVersion {
        object_revision: Some(meta.revision()),
        size,
        checksum_md5: Some(md5.clone()),
    };

    let destination = LocalFilesystem::open(Path::new(&scenario.right_root))
        .map_err(LocalFileCopyError::Local)?;
    if !destination.capabilities().operations.write_content {
        return Err(LocalFileCopyError::CapabilityUnavailable);
    }
    let target = Path::new(relative_path);
    match (destination.stat(target), replacement) {
        (Err(LocalError::Io(error)), None) if error.kind() == io::ErrorKind::NotFound => {}
        (Ok(_), None) => return Err(LocalFileCopyError::TargetExists),
        (Ok(entry), Some((_, expected_destination))) => {
            if !matches_conflict_snapshot(&entry, expected_destination) {
                return Err(LocalFileCopyError::ConflictStale);
            }
        }
        (Err(LocalError::Io(error)), Some(_)) if error.kind() == io::ErrorKind::NotFound => {
            return Err(LocalFileCopyError::ConflictStale)
        }
        (Err(error), _) => return Err(LocalFileCopyError::Local(error)),
    }
    let root_identity = local_root_identity_key(&destination).map_err(|error| match error {
        LocalDirectoryCreateError::Local(error) => LocalFileCopyError::Local(error),
        LocalDirectoryCreateError::Store(error) => LocalFileCopyError::Store(error),
        _ => LocalFileCopyError::InvalidRequest,
    })?;

    let mut store = JobStore::open(database_path)?;
    if let Some(existing) = store.get(job_id)? {
        return Ok(LocalFileCopyStatus::Existing(existing.state));
    }
    store.enqueue(job_id, Some((scenario_id, scenario_revision)))?;
    store.advance(
        job_id,
        JobState::Queued,
        JobState::Running,
        Confirmation::Unknown,
    )?;
    let attempt_id = format!("{job_id}-attempt-1");
    // The local copy kinds keep local reconciliation and staging cleanup.
    let operation_kind = if replacement.is_some() {
        "local_copy_replace"
    } else {
        "local_copy_new"
    };
    store.prepare_bound_attempt_with_root_identity(
        job_id,
        &attempt_id,
        operation_kind,
        &destination_connection,
        relative_path,
        size,
        &md5,
        &root_identity,
    )?;
    if !enabled_local_connection(registry_path, &destination_connection) {
        store.fail_attempt_before_effect(job_id, &attempt_id)?;
        return Err(LocalFileCopyError::ConnectionUnavailable);
    }
    if let Some(parent) = target
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        match destination.create_directory_tree(parent) {
            Ok(_) => {}
            Err(error @ LocalError::OutcomeUnknown(_)) => {
                return Err(LocalFileCopyError::OutcomeUnknown(error))
            }
            Err(error) => {
                store.fail_attempt_before_effect(job_id, &attempt_id)?;
                return Err(LocalFileCopyError::Local(error));
            }
        }
    }
    let inner = match connection.open_read(&server_path) {
        Ok(reader) => reader,
        Err(error) => {
            store.fail_attempt_before_effect(job_id, &attempt_id)?;
            return Err(read_failure(error));
        }
    };
    let mut download = VerifyingReader {
        inner,
        context: md5::Context::new(),
        total: 0,
        expected_size: size,
        expected_md5: md5.clone(),
        verified: false,
    };
    let written = if replacement.is_some() {
        destination.create_from_reader_to_replace(target, &mut download)
    } else {
        destination.create_from_reader(target, &mut download)
    };
    match written {
        Ok(bytes) if bytes == size && download.verified => {}
        Ok(_) => return Err(LocalFileCopyError::VerificationFailed),
        // Only a publish error is UNKNOWN. A read error or a changed source
        // happens before the publish.
        Err(error @ LocalError::OutcomeUnknown(_)) => {
            return Err(LocalFileCopyError::OutcomeUnknown(error))
        }
        Err(LocalError::AlreadyExists) => {
            store.fail_attempt_before_effect(job_id, &attempt_id)?;
            return Err(LocalFileCopyError::TargetExists);
        }
        Err(error) => {
            store.fail_attempt_before_effect(job_id, &attempt_id)?;
            return Err(match error {
                LocalError::Io(io) if io.kind() == io::ErrorKind::InvalidData => {
                    LocalFileCopyError::Sftp(SftpFailure::Changed)
                }
                other => LocalFileCopyError::Local(other),
            });
        }
    }
    let destination_entry = match destination.stat(target) {
        Ok(entry) if !entry.is_directory && entry.size == size => entry,
        _ => return Err(LocalFileCopyError::VerificationFailed),
    };
    let destination_version = ConfirmedFileVersion {
        object_revision: destination_entry.object_revision,
        size: destination_entry.size,
        checksum_md5: Some(md5),
    };
    let baseline = ScenarioCopyBaselineOutboxEntry {
        job_id: job_id.to_owned(),
        scenario: ScenarioReference {
            scenario_id: scenario_id.to_owned(),
            revision: scenario_revision,
        },
        relative_path: relative_path.to_owned(),
        left: source_version,
        right: destination_version,
    };
    let receipt = VerifiedReceipt::metadata_verified(
        &attempt_id,
        operation_kind,
        &destination_connection,
        relative_path,
    );
    store.complete_attempt_with_scenario_copy_baseline(
        job_id,
        &attempt_id,
        &receipt,
        Some(&baseline),
    )?;
    if let Ok(registry) = Registry::open(registry_path) {
        let _ = dispatch_pending_scenario_copy_baselines(&registry, &mut store);
    }
    Ok(LocalFileCopyStatus::Completed)
}

/// One Update of a Backup/export run: the source metadata and the
/// destination are read again, then the approved replacement protocol runs.
#[allow(clippy::too_many_arguments)]
pub fn run_ispmanager_scenario_update(
    database_path: &Path,
    registry_path: &Path,
    scenario_id: &str,
    scenario_revision: u64,
    relative_path: &str,
    job_id: &str,
    connection: &SftpConnection,
) -> Result<LocalFileCopyStatus, LocalFileCopyError> {
    if !plain_relative(relative_path) {
        return Err(LocalFileCopyError::InvalidRequest);
    }
    let registry =
        Registry::open(registry_path).map_err(|_| LocalFileCopyError::ScenarioUnavailable)?;
    let scenario = backup_scenario(&registry, scenario_id, scenario_revision)?;
    let meta = connection
        .lstat(&server_path(&scenario.left_root, relative_path))
        .map_err(read_failure)?
        .filter(|meta| meta.kind == "file")
        .ok_or(LocalFileCopyError::Sftp(SftpFailure::NotFound))?;
    let source_version = ConfirmedFileVersion {
        object_revision: Some(meta.revision()),
        size: meta.size,
        checksum_md5: None,
    };
    let destination = LocalFilesystem::open(Path::new(&scenario.right_root))
        .map_err(LocalFileCopyError::Local)?;
    let current = match destination.stat(Path::new(relative_path)) {
        Ok(entry) => entry,
        Err(LocalError::Io(error)) if error.kind() == io::ErrorKind::NotFound => {
            return Err(LocalFileCopyError::ConflictStale)
        }
        Err(error) => return Err(LocalFileCopyError::Local(error)),
    };
    let expected_destination =
        scenario_update_destination(&registry, &scenario, relative_path, true, &current)?;
    drop(registry);
    run_ispmanager_scenario_file_download(
        database_path,
        registry_path,
        scenario_id,
        scenario_revision,
        relative_path,
        job_id,
        connection,
        Some((&source_version, &expected_destination)),
    )
}

/// One Delete of a Backup/export run, of a file or of a whole folder (D-027
/// sections 5.4, 5.6a, and 5.6b). For a path in the scope, the server must answer that the source object is
/// absent; any other answer stops the deletion. The effect acts only on the
/// Mac or SMB side.
#[allow(clippy::too_many_arguments)]
pub fn run_ispmanager_scenario_delete(
    database_path: &Path,
    registry_path: &Path,
    scenario_id: &str,
    scenario_revision: u64,
    relative_path: &str,
    job_id: &str,
    connection: &SftpConnection,
    folder: bool,
) -> Result<LocalRuleDeleteStatus, LocalFileCopyError> {
    if !plain_relative(relative_path) {
        return Err(LocalFileCopyError::InvalidRequest);
    }
    let registry =
        Registry::open(registry_path).map_err(|_| LocalFileCopyError::ScenarioUnavailable)?;
    let scenario = backup_scenario(&registry, scenario_id, scenario_revision)?;
    let scope = registry
        .get_scenario_revision_scope(scenario_id, scenario_revision)
        .map_err(|_| LocalFileCopyError::ScenarioUnavailable)?;
    drop(registry);
    // `files/` is a mirror of the source scope (D-027 section 5.6a). A path
    // outside the selection or inside an exclusion is not in the scope, so
    // its copy goes without a server check. A path in the scope goes only
    // when the server answers that the object is absent.
    match scope.classify(relative_path, name_key) {
        ScopeMatch::Outside => {}
        ScopeMatch::NameVariant => return Err(LocalFileCopyError::ScenarioScopeMismatch),
        ScopeMatch::Inside => {
            match connection.lstat(&server_path(&scenario.left_root, relative_path)) {
                Ok(None) => {}
                Ok(Some(_)) => return Err(LocalFileCopyError::ConflictStale),
                Err(error) => return Err(read_failure(error)),
            }
        }
    }
    if folder {
        run_backup_mirror_delete_folder(
            database_path,
            registry_path,
            scenario_id,
            scenario_revision,
            relative_path,
            job_id,
        )
    } else {
        run_backup_mirror_delete(
            database_path,
            registry_path,
            scenario_id,
            scenario_revision,
            relative_path,
            job_id,
        )
    }
}

/// The result of one database export (D-027 section 7b).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DatabaseExportStatus {
    Created {
        bytes: u64,
        md5: String,
    },
    Updated {
        bytes: u64,
        md5: String,
    },
    /// The new dump has the same bytes as the destination file: no effect.
    Unchanged {
        bytes: u64,
        md5: String,
    },
    /// The Job of this run and database exists already.
    Existing(JobState),
}

/// A writer that keeps the byte count and the MD5 of what it writes.
struct HashingWriter<W> {
    inner: W,
    context: md5::Context,
    total: u64,
}

impl<W: io::Write> io::Write for HashingWriter<W> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let count = self.inner.write(buffer)?;
        self.context.consume(&buffer[..count]);
        self.total += count as u64;
        Ok(count)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// A database name that is also a safe file name.
fn plain_database_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && !name.starts_with('.')
        && !name.starts_with('-')
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b'$'))
}

/// How long the stages of one database export took and how many bytes it
/// moved, for the Worker log (D-027 section 7b, D-028). It holds no name.
#[derive(Debug, Default, Clone, Copy)]
pub struct DatabaseExportTiming {
    pub compressed: bool,
    /// The panel reads before the dump: the access facts or the key.
    pub access: Duration,
    /// `mysqldump` on the server, the transfer, and the staging write.
    pub dump: Duration,
    pub sql_bytes: u64,
    pub network_bytes: u64,
    /// The read of the destination file for the MD5 comparison.
    pub compare: Duration,
    /// The durable attempt and the publish into the copy.
    pub publish: Duration,
}

/// The stage of one database export, for the progress of the run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportProgress {
    /// The dump runs; `bytes` of SQL text are in the staging file.
    Dump { bytes: u64 },
    /// The dump is complete: comparison with the copy and the publish.
    Publish,
}

/// The Job of one database export in one run.
pub fn database_export_job_id(run_id: &str, database: &str) -> String {
    let digest = format!("{:x}", md5::compute(database.as_bytes()));
    format!("ispdb-{run_id}-{}", &digest[..16])
}

/// Where the dump of one database comes from (D-027 section 7b).
pub enum DumpSource<'a> {
    /// `mysqldump` over SSH with the access facts from the panel.
    Ssh {
        connection: &'a SftpConnection,
        access: &'a crate::ispmanager_sftp::MysqlAccess,
    },
    /// The panel function `db.dump` when SSH is not available. `key` comes
    /// from `ispmanager::database_key`.
    Panel {
        session: &'a mut crate::ispmanager::Session,
        key: &'a str,
    },
}

/// Dump one selected MySQL database into `databases/<name>.sql` of the
/// Backup/export copy (D-027 section 7b). Tables with the `structure_only`
/// rule keep only their structure; the panel dump cannot do this and stops.
/// When the server can pipe through `gzip -1`, only the SSH transfer is
/// compressed. The dump goes into a private
/// staging file first. Only a complete dump is published: as a new file
/// with no-overwrite, or through the approved replacement protocol with a
/// durable attempt. A dump with the same bytes as the destination file
/// gives no effect. `progress` is called during the dump; false stops it
/// before the publish.
#[allow(clippy::too_many_arguments)]
pub fn run_ispmanager_database_export(
    database_path: &Path,
    registry_path: &Path,
    scenario_id: &str,
    scenario_revision: u64,
    run_id: &str,
    database: &str,
    source: DumpSource<'_>,
    staging_dir: &Path,
    progress: &dyn Fn(ExportProgress) -> bool,
    timing: &mut DatabaseExportTiming,
) -> Result<DatabaseExportStatus, &'static str> {
    use crate::ispmanager::PanelDumpError;
    use crate::ispmanager_sftp::MysqlDumpError;
    use std::io::Write;
    let job_id = database_export_job_id(run_id, database);
    if !valid_job_id(&job_id) || !plain_database_name(database) {
        return Err("invalid_request");
    }
    let registry = Registry::open(registry_path).map_err(|_| "registry_unavailable")?;
    let scenario = backup_scenario(&registry, scenario_id, scenario_revision)
        .map_err(|_| "scenario_unavailable")?;
    let resources = registry
        .backup_resources(scenario_id, scenario_revision)
        .map_err(|_| "registry_unavailable")?;
    let selected = resources
        .iter()
        .any(|r| r.category == "databases" && r.rule == "include" && r.provider_id == database);
    if !selected {
        return Err("invalid_selection");
    }
    // The tables that the dump keeps without their data (section 7b.10).
    let prefix = format!("{database}/");
    let structure_only: Vec<String> = resources
        .iter()
        .filter(|r| r.category == "databases" && r.rule == "structure_only")
        .filter_map(|r| r.provider_id.strip_prefix(&prefix).map(ToOwned::to_owned))
        .collect();
    let destination_connection = scenario.right_connection_id.clone();
    if !enabled_local_connection(registry_path, &destination_connection) {
        return Err("local_scenario_connection_unavailable");
    }
    drop(registry);
    // `files/` is the right root; `databases/` is next to it.
    let root = Path::new(&scenario.right_root)
        .parent()
        .ok_or("local_root_unavailable")?;
    let destination = LocalFilesystem::open(root).map_err(|_| "local_root_unavailable")?;
    if !destination.capabilities().operations.write_content {
        return Err("local_root_unavailable");
    }
    let mut store = JobStore::open(database_path).map_err(|_| "job_store_unavailable")?;
    if let Some(existing) = store.get(&job_id).map_err(|_| "job_store_unavailable")? {
        return Ok(DatabaseExportStatus::Existing(existing.state));
    }

    // 1. The dump, read-only on the server, into a private staging file.
    std::fs::create_dir_all(staging_dir).map_err(|_| "local_root_unavailable")?;
    let staging = staging_dir.join(format!("{job_id}.sql"));
    let _ = std::fs::remove_file(&staging);
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&staging)
        .map_err(|_| "local_root_unavailable")?;
    let remove_staging = || {
        let _ = std::fs::remove_file(&staging);
    };
    // The dump stays plain SQL. Archives are a separate Scenario mode later.
    let mut hashing = HashingWriter {
        inner: io::BufWriter::new(file),
        context: md5::Context::new(),
        total: 0,
    };
    let mut write_failed = false;
    let started = Instant::now();
    let mut sink = |data: &[u8]| {
        write_failed = hashing.write_all(data).is_err();
        !write_failed
            && progress(ExportProgress::Dump {
                bytes: hashing.total,
            })
    };
    let dumped = match source {
        DumpSource::Ssh { connection, access } => {
            if access.database != database {
                remove_staging();
                return Err("invalid_request");
            }
            let compress = connection.gzip_pipe_available();
            timing.compressed = compress;
            connection
                .mysql_dump(access, &structure_only, compress, &mut sink)
                .map_err(|error| match error {
                    MysqlDumpError::Refused => "ispmanager_database_refused",
                    MysqlDumpError::Stopped => "stopped_by_user",
                    MysqlDumpError::Failed => "ispmanager_database_dump_failed",
                })
        }
        // The panel dump has all tables with their data: it cannot keep a
        // table without data (section 7b.9).
        DumpSource::Panel { .. } if !structure_only.is_empty() => {
            Err("ispmanager_database_structure_only_needs_ssh")
        }
        DumpSource::Panel { session, key } => session
            .database_dump(key, &mut sink)
            .map(|bytes| (bytes, bytes))
            .map_err(|error| match error {
                PanelDumpError::Refused => "ispmanager_database_panel_refused",
                PanelDumpError::Stopped => "stopped_by_user",
                PanelDumpError::Failed => "ispmanager_database_dump_failed",
            }),
    };
    match dumped {
        Ok((sql, network)) => (timing.sql_bytes, timing.network_bytes) = (sql, network),
        Err(code) => {
            remove_staging();
            return Err(if write_failed {
                "local_root_unavailable"
            } else {
                code
            });
        }
    }
    let written = (|| -> io::Result<(u64, String)> {
        hashing.inner.flush()?;
        let file = hashing
            .inner
            .into_inner()
            .map_err(|error| error.into_error())?;
        file.sync_all()?;
        Ok((hashing.total, format!("{:x}", hashing.context.compute())))
    })();
    let Ok((size, md5)) = written else {
        remove_staging();
        return Err("local_root_unavailable");
    };

    timing.dump = started.elapsed();
    let _ = progress(ExportProgress::Publish);
    // A large dump, and a database that is split already, get one file for
    // each table (section 7b.11.1). A split database stays split.
    let folder = Path::new("databases").join(database);
    let split = size >= SPLIT_FROM_BYTES
        || matches!(
            destination.stat(&folder.join(crate::sql_split::MANIFEST_FILE)),
            Ok(entry) if !entry.is_directory
        );
    if split {
        return publish_split_dump(
            database_path,
            registry_path,
            &mut store,
            &destination,
            &destination_connection,
            (scenario_id, scenario_revision),
            (run_id, database, &job_id),
            &staging,
            (size, md5),
            staging_dir,
            timing,
        );
    }
    publish_backup_file(
        registry_path,
        &mut store,
        &destination,
        &destination_connection,
        (scenario_id, scenario_revision),
        &job_id,
        &staging,
        (size, md5),
        &Path::new("databases").join(format!("{database}.sql")),
        timing,
    )
}

/// The dump size from which a database gets one file for each table
/// (owner decision of 2026-11-14: 700 MB).
pub const SPLIT_FROM_BYTES: u64 = 700 * 1024 * 1024;

/// The Job of one file of a split database dump in one run.
fn database_file_job_id(run_id: &str, database: &str, path: &str) -> String {
    let digest = format!(
        "{:x}",
        md5::compute(format!("{database}/{path}").as_bytes())
    );
    format!("ispdb-{run_id}-{}", &digest[..16])
}

/// Split a complete dump into `databases/<database>/` (section 7b.11.1).
/// Each file is published like a dump: only a file with other bytes is
/// written, with its own durable attempt. `manifest.json` is published
/// last, with the Job of the database, so that it names only files that are
/// in the copy. After it, the old `databases/<database>.sql` and the file
/// of a table that is gone are deleted under the simple deletion rule
/// (owner decision 2026-11-15); in Paranoid mode they stay.
#[allow(clippy::too_many_arguments)]
fn publish_split_dump(
    database_path: &Path,
    registry_path: &Path,
    store: &mut JobStore,
    destination: &LocalFilesystem,
    destination_connection: &str,
    scenario: (&str, u64),
    (run_id, database, job_id): (&str, &str, &str),
    staging: &Path,
    (size, md5): (u64, String),
    staging_dir: &Path,
    timing: &mut DatabaseExportTiming,
) -> Result<DatabaseExportStatus, &'static str> {
    use crate::sql_split::{split_dump, split_manifest, MANIFEST_FILE};
    let dir = staging_dir.join(format!("{job_id}.split"));
    let _ = std::fs::remove_dir_all(&dir);
    let split = std::fs::File::open(staging)
        .and_then(|file| split_dump(io::BufReader::with_capacity(1 << 20, file), &dir));
    let _ = std::fs::remove_file(staging);
    let remove_dir = || {
        let _ = std::fs::remove_dir_all(&dir);
    };
    let files = match split {
        Ok(files) => files,
        Err(error) => {
            remove_dir();
            return Err(if error.kind() == io::ErrorKind::InvalidData {
                "ispmanager_database_dump_failed"
            } else {
                "local_root_unavailable"
            });
        }
    };
    let folder = Path::new("databases").join(database);
    let mut changed = false;
    for file in &files {
        let file_job = database_file_job_id(run_id, database, &file.path);
        match store.get(&file_job) {
            Ok(None) => {}
            // This run published the file already.
            Ok(Some(job)) if job.state == JobState::Completed => {
                changed = true;
                continue;
            }
            Ok(Some(job)) => {
                remove_dir();
                return Ok(DatabaseExportStatus::Existing(job.state));
            }
            Err(_) => {
                remove_dir();
                return Err("job_store_unavailable");
            }
        }
        match publish_backup_file(
            registry_path,
            store,
            destination,
            destination_connection,
            scenario,
            &file_job,
            &dir.join(&file.path),
            (file.bytes, file.md5.clone()),
            &folder.join(&file.path),
            timing,
        ) {
            Ok(DatabaseExportStatus::Unchanged { .. }) => {}
            Ok(_) => changed = true,
            Err(code) => {
                remove_dir();
                return Err(code);
            }
        }
    }
    let manifest = split_manifest(database, &files);
    let manifest_path = dir.join(MANIFEST_FILE);
    if std::fs::write(&manifest_path, &manifest).is_err() {
        remove_dir();
        return Err("local_root_unavailable");
    }
    let published = publish_backup_file(
        registry_path,
        store,
        destination,
        destination_connection,
        scenario,
        job_id,
        &manifest_path,
        (
            manifest.len() as u64,
            format!("{:x}", md5::compute(&manifest)),
        ),
        &folder.join(MANIFEST_FILE),
        timing,
    );
    remove_dir();
    let published = published?;
    // The copy keeps one current dump: the old one-file dump and the files
    // of tables that are gone are deleted under the simple deletion rule.
    // In Paranoid mode they stay.
    let current: std::collections::BTreeSet<&str> =
        files.iter().map(|file| file.path.as_str()).collect();
    let mut stale = Vec::new();
    let one_file = format!("databases/{database}.sql");
    if matches!(destination.stat(Path::new(&one_file)), Ok(entry) if !entry.is_directory) {
        stale.push(one_file);
    }
    match destination.list(&folder.join("tables")) {
        Ok(entries) => stale.extend(
            entries
                .into_iter()
                .filter(|entry| !entry.is_directory && entry.name.ends_with(".sql"))
                .map(|entry| format!("tables/{}", entry.name))
                .filter(|path| !current.contains(path.as_str()))
                .map(|path| format!("databases/{database}/{path}")),
        ),
        Err(LocalError::Io(error)) if error.kind() == io::ErrorKind::NotFound => {}
        Err(_) => return Err("ispmanager_database_cleanup_failed"),
    }
    for path in &stale {
        let digest = format!("{:x}", md5::compute(path.as_bytes()));
        let delete_job = format!("ispdbdel-{run_id}-{}", &digest[..16]);
        match crate::job_execution::run_backup_database_file_delete(
            database_path,
            registry_path,
            scenario.0,
            scenario.1,
            path,
            &delete_job,
        ) {
            Ok(crate::job_execution::LocalRuleDeleteStatus::Existing(state))
                if state != JobState::Completed =>
            {
                return Err("ispmanager_database_cleanup_failed")
            }
            Ok(_) | Err(LocalFileCopyError::ConsentRequired) => {}
            Err(_) => return Err("ispmanager_database_cleanup_failed"),
        }
    }
    Ok(match published {
        DatabaseExportStatus::Created { .. } => DatabaseExportStatus::Created { bytes: size, md5 },
        _ if changed => DatabaseExportStatus::Updated { bytes: size, md5 },
        _ => DatabaseExportStatus::Unchanged { bytes: size, md5 },
    })
}

/// The Job of one Cron, DNS, or mail export in one run.
pub fn object_export_job_id(run_id: &str, category: &str, provider_id: &str) -> String {
    let digest = format!(
        "{:x}",
        md5::compute(format!("{category}:{provider_id}").as_bytes())
    );
    format!("ispobj-{run_id}-{}", &digest[..16])
}

/// The file of one exported object in the copy root (D-027 section 10).
fn object_target(category: &str, provider_id: &str) -> Option<PathBuf> {
    match category {
        "cron" if provider_id == "all" => Some(PathBuf::from("cron/jobs.json")),
        "ssl" if provider_id == "all" => Some(PathBuf::from("ssl/certificates.json")),
        "ftp" if provider_id == "all" => Some(PathBuf::from("ftp/users.json")),
        "accounts" if provider_id == "all" => Some(PathBuf::from("accounts/limits.json")),
        "php" if provider_id == "all" => Some(PathBuf::from("php/runtime.json")),
        "ssl_pem" if crate::registry::domain_name(provider_id) => {
            Some(Path::new("ssl").join(format!("{provider_id}.pem")))
        }
        "dns_zone" if crate::registry::domain_name(provider_id) => {
            Some(Path::new("dns").join(format!("{provider_id}.zone")))
        }
        "dns" | "mail" | "sites" if crate::registry::domain_name(provider_id) => {
            Some(Path::new(category).join(format!("{provider_id}.json")))
        }
        _ => None,
    }
}

/// Write the panel export of one selected Cron, DNS, or mail object into
/// its file of the Backup/export copy (D-027 section 10). `content` comes
/// from `ispmanager::export_object`. The same bytes as the copy give no
/// effect; otherwise the file is published like a database dump.
#[allow(clippy::too_many_arguments)]
pub fn run_ispmanager_object_export(
    database_path: &Path,
    registry_path: &Path,
    scenario_id: &str,
    scenario_revision: u64,
    run_id: &str,
    category: &str,
    provider_id: &str,
    content: &[u8],
    staging_dir: &Path,
) -> Result<DatabaseExportStatus, &'static str> {
    use std::io::Write;
    let job_id = object_export_job_id(run_id, category, provider_id);
    let target = object_target(category, provider_id).ok_or("invalid_request")?;
    if !valid_job_id(&job_id) {
        return Err("invalid_request");
    }
    let registry = Registry::open(registry_path).map_err(|_| "registry_unavailable")?;
    let scenario = backup_scenario(&registry, scenario_id, scenario_revision)
        .map_err(|_| "scenario_unavailable")?;
    // A site is selected by name or through `all` (D-027 section 7c).
    let selected = registry
        .backup_resources(scenario_id, scenario_revision)
        .map_err(|_| "registry_unavailable")?
        .iter()
        .any(|r| {
            // The zone file goes with its selected DNS zone, and a PEM
            // file with the selected certificate list.
            if category == "ssl_pem" {
                return r.category == "ssl" && r.rule == "include" && r.provider_id == "all";
            }
            r.category
                == if category == "dns_zone" {
                    "dns"
                } else {
                    category
                }
                && r.rule == "include"
                && (r.provider_id == provider_id || (category == "sites" && r.provider_id == "all"))
        });
    if !selected {
        return Err("invalid_selection");
    }
    let destination_connection = scenario.right_connection_id.clone();
    if !enabled_local_connection(registry_path, &destination_connection) {
        return Err("local_scenario_connection_unavailable");
    }
    drop(registry);
    let root = Path::new(&scenario.right_root)
        .parent()
        .ok_or("local_root_unavailable")?;
    let destination = LocalFilesystem::open(root).map_err(|_| "local_root_unavailable")?;
    if !destination.capabilities().operations.write_content {
        return Err("local_root_unavailable");
    }
    let mut store = JobStore::open(database_path).map_err(|_| "job_store_unavailable")?;
    if let Some(existing) = store.get(&job_id).map_err(|_| "job_store_unavailable")? {
        return Ok(DatabaseExportStatus::Existing(existing.state));
    }
    std::fs::create_dir_all(staging_dir).map_err(|_| "local_root_unavailable")?;
    let staging = staging_dir.join(format!("{job_id}.json"));
    let _ = std::fs::remove_file(&staging);
    let written = (|| -> io::Result<()> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&staging)?;
        file.write_all(content)?;
        file.sync_all()
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&staging);
        return Err("local_root_unavailable");
    }
    publish_backup_file(
        registry_path,
        &mut store,
        &destination,
        &destination_connection,
        (scenario_id, scenario_revision),
        &job_id,
        &staging,
        (content.len() as u64, format!("{:x}", md5::compute(content))),
        &target,
        &mut DatabaseExportTiming::default(),
    )
}

/// Publish one complete private staging file at `target` of the copy root
/// (D-027 sections 7b.7 and 10). The same bytes as the destination file give
/// no effect. Otherwise a durable attempt with the size and the MD5 comes
/// first, then a no-overwrite create or the approved replacement protocol.
/// A lost proof gives `outcome_unknown`; local reconciliation checks it.
#[allow(clippy::too_many_arguments)]
fn publish_backup_file(
    registry_path: &Path,
    store: &mut JobStore,
    destination: &LocalFilesystem,
    destination_connection: &str,
    (scenario_id, scenario_revision): (&str, u64),
    job_id: &str,
    staging: &Path,
    (size, md5): (u64, String),
    target: &Path,
    timing: &mut DatabaseExportTiming,
) -> Result<DatabaseExportStatus, &'static str> {
    // The same bytes as the destination file give no effect.
    let compared = Instant::now();
    let remove_staging = || {
        let _ = std::fs::remove_file(staging);
    };
    let target_text = target.to_string_lossy().into_owned();
    let replace = match destination.stat(target) {
        Ok(entry) if entry.is_directory => {
            remove_staging();
            return Err("target_exists");
        }
        Ok(entry) => {
            if entry.size == size {
                let mut hashing = HashingWriter {
                    inner: io::sink(),
                    context: md5::Context::new(),
                    total: 0,
                };
                if destination.read_to_writer(target, &mut hashing).is_ok()
                    && format!("{:x}", hashing.context.compute()) == md5
                {
                    remove_staging();
                    timing.compare = compared.elapsed();
                    return Ok(DatabaseExportStatus::Unchanged { bytes: size, md5 });
                }
            }
            true
        }
        Err(LocalError::Io(error)) if error.kind() == io::ErrorKind::NotFound => false,
        Err(_) => {
            remove_staging();
            return Err("local_metadata_unavailable");
        }
    };

    timing.compare = compared.elapsed();
    let publish_started = Instant::now();
    // The publish: a durable attempt, then no-overwrite or replacement.
    let root_identity = local_root_identity_key(destination).map_err(|_| {
        remove_staging();
        "local_root_unavailable"
    })?;
    let store_error = |_| "job_store_unavailable";
    store
        .enqueue(job_id, Some((scenario_id, scenario_revision)))
        .map_err(store_error)?;
    store
        .advance(
            job_id,
            JobState::Queued,
            JobState::Running,
            Confirmation::Unknown,
        )
        .map_err(store_error)?;
    let attempt_id = format!("{job_id}-attempt-1");
    let operation_kind = if replace {
        "local_copy_replace"
    } else {
        "local_copy_new"
    };
    store
        .prepare_bound_attempt_with_root_identity(
            job_id,
            &attempt_id,
            operation_kind,
            destination_connection,
            &target_text,
            size,
            &md5,
            &root_identity,
        )
        .map_err(store_error)?;
    let fail_before_effect = |store: &mut JobStore, code: &'static str| {
        let _ = store.fail_attempt_before_effect(job_id, &attempt_id);
        let _ = std::fs::remove_file(staging);
        Err(code)
    };
    if !enabled_local_connection(registry_path, destination_connection) {
        return fail_before_effect(store, "local_scenario_connection_unavailable");
    }
    match destination.create_directory_tree(target.parent().unwrap_or(Path::new(""))) {
        Ok(_) => {}
        Err(LocalError::OutcomeUnknown(_)) => return Err("outcome_unknown"),
        Err(_) => return fail_before_effect(store, "local_root_unavailable"),
    }
    let Ok(mut input) = std::fs::File::open(staging) else {
        return fail_before_effect(store, "local_root_unavailable");
    };
    let published = if replace {
        destination.create_from_reader_to_replace(target, &mut input)
    } else {
        destination.create_from_reader(target, &mut input)
    };
    match published {
        Ok(bytes) if bytes == size => {}
        Ok(_) => return Err("outcome_unknown"),
        Err(LocalError::OutcomeUnknown(_)) => return Err("outcome_unknown"),
        Err(LocalError::AlreadyExists) => return fail_before_effect(store, "target_exists"),
        Err(_) => return fail_before_effect(store, "local_root_unavailable"),
    }
    remove_staging();
    match destination.stat(target) {
        Ok(entry) if !entry.is_directory && entry.size == size => {}
        _ => return Err("outcome_unknown"),
    }
    let receipt = VerifiedReceipt::metadata_verified(
        &attempt_id,
        operation_kind,
        destination_connection,
        &target_text,
    );
    store
        .complete_attempt_with_scenario_copy_baseline(job_id, &attempt_id, &receipt, None)
        .map_err(store_error)?;
    timing.publish = publish_started.elapsed();
    Ok(if replace {
        DatabaseExportStatus::Updated { bytes: size, md5 }
    } else {
        DatabaseExportStatus::Created { bytes: size, md5 }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DeletionMode;
    use crate::registry::{BackupResource, BackupScenarioDraft};

    fn object(category: &str, provider_id: &str) -> BackupResource {
        BackupResource {
            category: category.into(),
            provider_id: provider_id.into(),
            kind: "object".into(),
            rule: "include".into(),
        }
    }

    #[test]
    fn object_target_stays_in_its_category_folder() {
        assert_eq!(
            object_target("ssl_pem", "example.test"),
            Some(PathBuf::from("ssl/example.test.pem"))
        );
        assert_eq!(
            object_target("accounts", "all"),
            Some(PathBuf::from("accounts/limits.json"))
        );
        assert_eq!(
            object_target("php", "all"),
            Some(PathBuf::from("php/runtime.json"))
        );
        assert_eq!(
            object_target("dns_zone", "example.test"),
            Some(PathBuf::from("dns/example.test.zone"))
        );
        for (category, provider) in [
            ("ssl_pem", "../example.test"),
            ("ssl_pem", "a/b"),
            ("ssl_pem", ""),
            ("dns", ".."),
            ("mail", "/etc/passwd"),
            ("accounts", "example.test"),
            ("php", "../all"),
            ("files", "/x"),
            ("unknown", "all"),
        ] {
            assert_eq!(
                object_target(category, provider),
                None,
                "{category} {provider}"
            );
        }
    }

    #[test]
    fn object_export_publishes_once_and_follows_the_selection() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::fs::canonicalize(std::env::temp_dir())
            .unwrap()
            .join(format!("qadren-ispobj-{}-{nanos}", std::process::id()));
        let copy = dir.join("copy");
        std::fs::create_dir_all(&copy).unwrap();
        let registry_path = dir.join("connections.sqlite3");
        let jobs_path = dir.join("jobs.sqlite3");
        let staging = dir.join("staging");
        let registry = Registry::open(&registry_path).unwrap();
        let source = registry
            .add_ispmanager("Panel", "https://panel.example.test:1500/ispmgr")
            .unwrap();
        let destination = registry.ensure_local_computer("Mac").unwrap();
        let resources = [object("ssl", "all"), object("dns", "example.test")];
        let scenario = registry
            .create_backup_scenario(BackupScenarioDraft {
                name: "Backup",
                source_connection_id: &source,
                destination_connection_id: &destination,
                destination_root: copy.to_str().unwrap(),
                source_root: "/home/user",
                resources: &resources,
                deletion_mode: DeletionMode::Paranoid,
            })
            .unwrap();
        drop(registry);
        let export = |run: &str, category: &str, provider: &str, content: &[u8]| {
            run_ispmanager_object_export(
                &jobs_path,
                &registry_path,
                &scenario,
                1,
                run,
                category,
                provider,
                content,
                &staging,
            )
        };
        let pem = b"-----BEGIN CERTIFICATE-----\nMIIB\n-----END CERTIFICATE-----\n";

        // A new file is created with its size and MD5.
        let created = export("run1", "ssl_pem", "example.test", pem).unwrap();
        let expected_md5 = format!("{:x}", md5::compute(pem));
        assert!(
            matches!(&created, DatabaseExportStatus::Created { bytes, md5 }
            if *bytes == pem.len() as u64 && *md5 == expected_md5)
        );
        assert_eq!(
            std::fs::read(copy.join("ssl/example.test.pem")).unwrap(),
            pem
        );

        // The same run and object does not publish again.
        assert!(matches!(
            export("run1", "ssl_pem", "example.test", pem),
            Ok(DatabaseExportStatus::Existing(JobState::Completed))
        ));
        // A later run with the same bytes makes no effect.
        assert!(matches!(
            export("run2", "ssl_pem", "example.test", pem),
            Ok(DatabaseExportStatus::Unchanged { .. })
        ));
        // The zone file follows its selected DNS zone; another zone is refused.
        assert!(matches!(
            export("run2", "dns_zone", "example.test", b"; zone\n"),
            Ok(DatabaseExportStatus::Created { .. })
        ));
        assert_eq!(
            export("run2", "dns_zone", "other.test", b"; zone\n"),
            Err("invalid_selection")
        );
        // Objects that the revision does not select are refused.
        assert_eq!(
            export("run2", "cron", "all", b"{}"),
            Err("invalid_selection")
        );
        assert_eq!(
            export("run2", "mail", "example.test", b"{}"),
            Err("invalid_selection")
        );
        // A name that is not a file name never reaches the destination.
        assert_eq!(
            export("run2", "ssl_pem", "../escape", pem),
            Err("invalid_request")
        );
        assert!(!dir.join("escape.pem").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn backup_scenario_refuses_a_wrong_selection_or_endpoint() {
        use crate::registry::RegistryError;
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("qadren-ispsel-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let registry = Registry::open(&dir.join("connections.sqlite3")).unwrap();
        let panel = registry
            .add_ispmanager("Panel", "https://panel.example.test:1500/ispmgr")
            .unwrap();
        let other_panel = registry
            .add_ispmanager("Panel 2", "https://panel.example.test:1500/ispmgr")
            .unwrap();
        let mac = registry.ensure_local_computer("Mac").unwrap();
        let create = |source: &str, destination: &str, resources: &[BackupResource]| {
            registry.create_backup_scenario(BackupScenarioDraft {
                name: "Backup",
                source_connection_id: source,
                destination_connection_id: destination,
                destination_root: "/Volumes/copy",
                source_root: "/home/user",
                resources,
                deletion_mode: DeletionMode::Paranoid,
            })
        };
        let file = |path: &str, rule: &str| BackupResource {
            category: "files".into(),
            provider_id: path.into(),
            kind: "dir".into(),
            rule: rule.into(),
        };
        // An empty selection means nothing, so it cannot be saved.
        assert!(matches!(
            create(&panel, &mac, &[]),
            Err(RegistryError::InvalidSelection)
        ));
        // A file rule outside the home folder, or with `..`, is refused.
        for path in ["/etc", "/home/username", "/home/user/../other", "relative"] {
            assert!(
                matches!(
                    create(&panel, &mac, &[file(path, "include")]),
                    Err(RegistryError::InvalidSelection)
                ),
                "{path}"
            );
        }
        // An exclusion needs a selected folder above it.
        assert!(matches!(
            create(&panel, &mac, &[file("/home/user/www/cache", "exclude")]),
            Err(RegistryError::InvalidSelection)
        ));
        // Whole-list objects use `all` only; names must be file names.
        for (category, provider) in [
            ("accounts", "user"),
            ("php", "8.2"),
            ("ssl", "example.test"),
            ("cron", "1"),
            ("dns", "../zone"),
            ("mail", "a/b"),
            ("sites", ""),
        ] {
            assert!(
                matches!(
                    create(&panel, &mac, &[object(category, provider)]),
                    Err(RegistryError::InvalidSelection)
                ),
                "{category} {provider}"
            );
        }
        // ISPmanager is never a destination, and the source must be ISPmanager.
        assert!(matches!(
            create(&panel, &other_panel, &[object("cron", "all")]),
            Err(RegistryError::UnsupportedScenarioEndpoint)
        ));
        assert!(matches!(
            create(&mac, &mac, &[object("cron", "all")]),
            Err(RegistryError::UnsupportedScenarioEndpoint)
        ));
        // A valid selection is saved.
        assert!(create(
            &panel,
            &mac,
            &[
                file("/home/user/www", "include"),
                file("/home/user/www/cache", "exclude"),
                object("accounts", "all"),
                object("php", "all"),
                object("sites", "all")
            ]
        )
        .is_ok());
        drop(registry);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
