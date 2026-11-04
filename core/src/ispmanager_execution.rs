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
    require_selected_path, run_scenario_rule_delete_after_remote_check,
    scenario_update_destination, valid_job_id, LocalDirectoryCreateError, LocalFileCopyError,
    LocalFileCopyStatus, LocalRuleDeleteStatus, SftpFailure,
};
use crate::jobs::{JobStore, ScenarioCopyBaselineOutboxEntry};
use crate::local_fs::{LocalError, LocalFilesystem};
use crate::model::{ConfirmedFileVersion, JobState, ScenarioReference};
use crate::registry::{Registry, Scenario};
use crate::scenario_copy_dispatch::dispatch_pending_scenario_copy_baselines;
use crate::storage_access::ISPMANAGER;
use crate::verification::VerifiedReceipt;
use std::io::{self, Read};
use std::path::{Component, Path};

fn plain_relative(path: &str) -> bool {
    !path.is_empty()
        && !path.contains(|ch: char| ch.is_control())
        && Path::new(path)
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
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
    if mode != "backup_export" || scenario.direction != "left_to_right" || scenario.left_root != "/"
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
    let server_path = format!("/{relative_path}");
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
        .lstat(&format!("/{relative_path}"))
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

/// One Delete of a Backup/export run (D-027 section 5.4). The server must
/// answer that the source object is absent; any other answer stops the
/// deletion. The effect acts only on the Mac or SMB side.
#[allow(clippy::too_many_arguments)]
pub fn run_ispmanager_scenario_delete(
    database_path: &Path,
    registry_path: &Path,
    scenario_id: &str,
    scenario_revision: u64,
    relative_path: &str,
    job_id: &str,
    connection: &SftpConnection,
) -> Result<LocalRuleDeleteStatus, LocalFileCopyError> {
    if !plain_relative(relative_path) {
        return Err(LocalFileCopyError::InvalidRequest);
    }
    let registry =
        Registry::open(registry_path).map_err(|_| LocalFileCopyError::ScenarioUnavailable)?;
    backup_scenario(&registry, scenario_id, scenario_revision)?;
    drop(registry);
    match connection.lstat(&format!("/{relative_path}")) {
        Ok(None) => {}
        Ok(Some(_)) => return Err(LocalFileCopyError::ConflictStale),
        Err(error) => return Err(read_failure(error)),
    }
    run_scenario_rule_delete_after_remote_check(
        database_path,
        registry_path,
        scenario_id,
        scenario_revision,
        relative_path,
        false,
        job_id,
    )
}
