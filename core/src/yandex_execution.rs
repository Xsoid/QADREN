//! Yandex Disk to Mac Scenario effects (D-023, stage 1).
//!
//! The source is a Yandex Disk file, read through a signed download link.
//! The destination is a Local Filesystem root (This Mac or an SMB mount), so
//! the destination rules of the local executor apply: durable attempt,
//! no-overwrite publish for a new file, the approved replacement protocol for
//! an update, and local-only reconciliation. Content is published only after
//! its byte count and MD5 match the provider metadata. The access token is
//! used in memory only and is never stored with the Job.
use crate::confirmation::Confirmation;
use crate::job_execution::{
    enabled_local_connection, local_root_identity_key, matches_conflict_snapshot,
    require_selected_path, run_scenario_rule_delete_after_remote_check,
    scenario_update_destination, valid_job_id, LocalDirectoryCreateError, LocalFileCopyError,
    LocalFileCopyStatus, LocalRuleDeleteStatus,
};
use crate::jobs::{JobStore, ScenarioCopyBaselineOutboxEntry};
use crate::local_fs::{LocalError, LocalFilesystem};
use crate::model::{ConfirmedFileVersion, JobState, ScenarioReference};
use crate::registry::Registry;
use crate::scenario_copy_dispatch::dispatch_pending_scenario_copy_baselines;
use crate::storage_access::{valid_yandex_root, YANDEX_DISK};
use crate::verification::VerifiedReceipt;
use crate::yandex::{YandexError, YandexMetadataClient};
use std::path::{Component, Path};

/// The plan metadata of a Yandex Disk file. `object_revision` of a Yandex
/// snapshot item is its MD5.
pub fn yandex_file_version(size: u64, md5: &str) -> ConfirmedFileVersion {
    ConfirmedFileVersion {
        object_revision: Some(md5.to_owned()),
        size,
        checksum_md5: Some(md5.to_owned()),
    }
}

/// A relative Scenario path with only normal segments.
fn plain_relative(path: &str) -> bool {
    !path.is_empty()
        && !path.contains(|ch: char| ch.is_control())
        && Path::new(path)
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
}

fn yandex_path(root: &str, relative: &str) -> String {
    if root.ends_with('/') {
        format!("{root}{relative}")
    } else {
        format!("{root}/{relative}")
    }
}

/// Download one Yandex Disk file of a one-way Scenario into its Local
/// destination. `replacement` is `None` for a Create. For an Update it holds
/// the planned source and destination versions, and the user consent.
#[allow(clippy::too_many_arguments)]
pub fn run_yandex_scenario_file_download(
    database_path: &Path,
    registry_path: &Path,
    scenario_id: &str,
    scenario_revision: u64,
    relative_path: &str,
    job_id: &str,
    token: &str,
    client: &YandexMetadataClient,
    replacement: Option<(&ConfirmedFileVersion, &ConfirmedFileVersion, bool)>,
) -> Result<LocalFileCopyStatus, LocalFileCopyError> {
    if !valid_job_id(job_id) || !plain_relative(relative_path) || token.is_empty() {
        return Err(LocalFileCopyError::InvalidRequest);
    }
    if matches!(replacement, Some((_, _, false))) {
        return Err(LocalFileCopyError::ConsentRequired);
    }
    let registry =
        Registry::open(registry_path).map_err(|_| LocalFileCopyError::ScenarioUnavailable)?;
    let scenario = registry
        .get_scenario_revision(scenario_id, scenario_revision)
        .map_err(|_| LocalFileCopyError::ScenarioUnavailable)?
        .ok_or(LocalFileCopyError::ScenarioUnavailable)?;
    let adapter = |id: &str| {
        registry
            .get(id)
            .ok()
            .flatten()
            .filter(|connection| connection.enabled)
            .map(|connection| connection.adapter)
    };
    let (source_connection, source_root, destination_connection, destination_root, source_is_left) =
        match scenario.direction.as_str() {
            "left_to_right" => (
                &scenario.left_connection_id,
                &scenario.left_root,
                &scenario.right_connection_id,
                &scenario.right_root,
                true,
            ),
            "right_to_left" => (
                &scenario.right_connection_id,
                &scenario.right_root,
                &scenario.left_connection_id,
                &scenario.left_root,
                false,
            ),
            _ => return Err(LocalFileCopyError::ScenarioScopeMismatch),
        };
    if adapter(source_connection).as_deref() != Some(YANDEX_DISK) {
        return Err(LocalFileCopyError::ConnectionUnavailable);
    }
    if !valid_yandex_root(source_root) {
        return Err(LocalFileCopyError::ScenarioScopeMismatch);
    }
    if !enabled_local_connection(registry_path, destination_connection) {
        return Err(LocalFileCopyError::ConnectionUnavailable);
    }
    require_selected_path(
        &registry,
        scenario_id,
        scenario_revision,
        Path::new(relative_path),
    )?;

    // Read the source metadata again. The plan can be old.
    let remote_path = yandex_path(source_root, relative_path);
    let resource = client
        .resource_metadata(token, &remote_path)
        .map_err(LocalFileCopyError::Remote)?;
    let (Some(size), Some(md5)) = (resource.size, resource.md5.as_deref()) else {
        return Err(LocalFileCopyError::Remote(YandexError::ContentMismatch));
    };
    let md5 = md5.to_ascii_lowercase();
    if resource.kind != "file" || md5.len() != 32 || !md5.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(LocalFileCopyError::Remote(YandexError::ContentMismatch));
    }
    let source_version = yandex_file_version(size, &md5);

    let destination =
        LocalFilesystem::open(Path::new(destination_root)).map_err(LocalFileCopyError::Local)?;
    if !destination.capabilities().operations.write_content {
        return Err(LocalFileCopyError::CapabilityUnavailable);
    }
    let target = Path::new(relative_path);
    match (destination.stat(target), replacement) {
        (Err(LocalError::Io(error)), None) if error.kind() == std::io::ErrorKind::NotFound => {}
        (Ok(_), None) => return Err(LocalFileCopyError::TargetExists),
        (Ok(entry), Some((expected_source, expected_destination, _))) => {
            if source_version.size != expected_source.size
                || source_version.object_revision != expected_source.object_revision
                || !matches_conflict_snapshot(&entry, expected_destination)
            {
                return Err(LocalFileCopyError::ConflictStale);
            }
        }
        (Err(LocalError::Io(error)), Some(_)) if error.kind() == std::io::ErrorKind::NotFound => {
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
    // The local copy operation kinds keep local reconciliation and staging
    // cleanup unchanged: both check only the destination.
    let operation_kind = if replacement.is_some() {
        "local_copy_replace"
    } else {
        "local_copy_new"
    };
    store.prepare_bound_attempt_with_root_identity(
        job_id,
        &attempt_id,
        operation_kind,
        destination_connection,
        relative_path,
        size,
        &md5,
        &root_identity,
    )?;
    if !enabled_local_connection(registry_path, destination_connection)
        || adapter(source_connection).as_deref() != Some(YANDEX_DISK)
    {
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
    let mut download = match client.open_download(token, &remote_path, size, &md5) {
        Ok(download) => download,
        Err(error) => {
            store.fail_attempt_before_effect(job_id, &attempt_id)?;
            return Err(LocalFileCopyError::Remote(error));
        }
    };
    let written = if replacement.is_some() {
        destination.create_from_reader_to_replace(target, &mut download)
    } else {
        destination.create_from_reader(target, &mut download)
    };
    match written {
        Ok(bytes) if bytes == size && download.verified() => {}
        Ok(_) => return Err(LocalFileCopyError::VerificationFailed),
        // Only a publish error is UNKNOWN. Every other error, including a
        // network error or an MD5 mismatch, happens before the publish.
        Err(error @ LocalError::OutcomeUnknown(_)) => {
            return Err(LocalFileCopyError::OutcomeUnknown(error))
        }
        Err(LocalError::AlreadyExists) => {
            store.fail_attempt_before_effect(job_id, &attempt_id)?;
            return Err(LocalFileCopyError::TargetExists);
        }
        Err(error) => {
            store.fail_attempt_before_effect(job_id, &attempt_id)?;
            return Err(LocalFileCopyError::Local(error));
        }
    }
    let destination_entry = match destination.stat(target) {
        Ok(entry) if !entry.is_directory && entry.size == size => entry,
        _ => return Err(LocalFileCopyError::VerificationFailed),
    };
    let destination_version = ConfirmedFileVersion {
        object_revision: destination_entry.object_revision,
        size: destination_entry.size,
        checksum_md5: Some(md5.clone()),
    };
    let baseline = ScenarioCopyBaselineOutboxEntry {
        job_id: job_id.to_owned(),
        scenario: ScenarioReference {
            scenario_id: scenario_id.to_owned(),
            revision: scenario_revision,
        },
        relative_path: relative_path.to_owned(),
        left: if source_is_left {
            source_version.clone()
        } else {
            destination_version.clone()
        },
        right: if source_is_left {
            destination_version
        } else {
            source_version
        },
    };
    let receipt = VerifiedReceipt::metadata_verified(
        &attempt_id,
        operation_kind,
        destination_connection,
        relative_path,
    );
    store.complete_attempt_with_scenario_copy_baseline(
        job_id,
        &attempt_id,
        &receipt,
        Some(&baseline),
    )?;
    // Completion is already durable. A failed Registry write leaves the
    // outbox pending for a later, idempotent local-only delivery.
    if let Ok(registry) = Registry::open(registry_path) {
        let _ = dispatch_pending_scenario_copy_baselines(&registry, &mut store);
    }
    Ok(LocalFileCopyStatus::Completed)
}

/// The Yandex Disk source root of a one-way Scenario revision.
fn yandex_source_root(
    registry: &Registry,
    scenario: &crate::registry::Scenario,
) -> Result<String, LocalFileCopyError> {
    let (connection, root) = match scenario.direction.as_str() {
        "left_to_right" => (&scenario.left_connection_id, &scenario.left_root),
        "right_to_left" => (&scenario.right_connection_id, &scenario.right_root),
        _ => return Err(LocalFileCopyError::ScenarioScopeMismatch),
    };
    let adapter = registry
        .get(connection)
        .ok()
        .flatten()
        .filter(|connection| connection.enabled)
        .map(|connection| connection.adapter);
    if adapter.as_deref() != Some(YANDEX_DISK) {
        return Err(LocalFileCopyError::ConnectionUnavailable);
    }
    if !valid_yandex_root(root) {
        return Err(LocalFileCopyError::ScenarioScopeMismatch);
    }
    Ok(root.clone())
}

/// One Update of a Yandex Disk to Mac run (D-025 section 6.3). The source
/// metadata and the destination are read again here; the download then
/// checks the size and MD5 and replaces through the approved protocol.
#[allow(clippy::too_many_arguments)]
pub fn run_yandex_scenario_update(
    database_path: &Path,
    registry_path: &Path,
    scenario_id: &str,
    scenario_revision: u64,
    relative_path: &str,
    job_id: &str,
    token: &str,
    client: &YandexMetadataClient,
) -> Result<LocalFileCopyStatus, LocalFileCopyError> {
    if !plain_relative(relative_path) || token.is_empty() {
        return Err(LocalFileCopyError::InvalidRequest);
    }
    let registry =
        Registry::open(registry_path).map_err(|_| LocalFileCopyError::ScenarioUnavailable)?;
    let scenario = registry
        .get_scenario_revision(scenario_id, scenario_revision)
        .map_err(|_| LocalFileCopyError::ScenarioUnavailable)?
        .ok_or(LocalFileCopyError::ScenarioUnavailable)?;
    let source_root = yandex_source_root(&registry, &scenario)?;
    let source_is_left = scenario.direction == "left_to_right";
    let destination_root = if source_is_left {
        &scenario.right_root
    } else {
        &scenario.left_root
    };
    let resource = client
        .resource_metadata(token, &yandex_path(&source_root, relative_path))
        .map_err(LocalFileCopyError::Remote)?;
    let (Some(size), Some(md5)) = (resource.size, resource.md5.as_deref()) else {
        return Err(LocalFileCopyError::Remote(YandexError::ContentMismatch));
    };
    let source_version = yandex_file_version(size, &md5.to_ascii_lowercase());
    let destination =
        LocalFilesystem::open(Path::new(destination_root)).map_err(LocalFileCopyError::Local)?;
    let current = match destination.stat(Path::new(relative_path)) {
        Ok(entry) => entry,
        Err(LocalError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(LocalFileCopyError::ConflictStale)
        }
        Err(error) => return Err(LocalFileCopyError::Local(error)),
    };
    let expected_destination = scenario_update_destination(
        &registry,
        &scenario,
        relative_path,
        source_is_left,
        &current,
    )?;
    drop(registry);
    run_yandex_scenario_file_download(
        database_path,
        registry_path,
        scenario_id,
        scenario_revision,
        relative_path,
        job_id,
        token,
        client,
        Some((&source_version, &expected_destination, true)),
    )
}

/// One Delete of a Yandex Disk to Mac run (D-025 section 6.3). The provider
/// must answer that the source object is absent; any other answer stops
/// the deletion. The effect acts only on the Mac or SMB side.
#[allow(clippy::too_many_arguments)]
pub fn run_yandex_scenario_delete(
    database_path: &Path,
    registry_path: &Path,
    scenario_id: &str,
    scenario_revision: u64,
    relative_path: &str,
    job_id: &str,
    token: &str,
    client: &YandexMetadataClient,
) -> Result<LocalRuleDeleteStatus, LocalFileCopyError> {
    if !plain_relative(relative_path) || token.is_empty() {
        return Err(LocalFileCopyError::InvalidRequest);
    }
    let registry =
        Registry::open(registry_path).map_err(|_| LocalFileCopyError::ScenarioUnavailable)?;
    let scenario = registry
        .get_scenario_revision(scenario_id, scenario_revision)
        .map_err(|_| LocalFileCopyError::ScenarioUnavailable)?
        .ok_or(LocalFileCopyError::ScenarioUnavailable)?;
    let source_root = yandex_source_root(&registry, &scenario)?;
    let on_left = scenario.direction == "right_to_left";
    drop(registry);
    match client.resource_metadata(token, &yandex_path(&source_root, relative_path)) {
        Err(YandexError::NotFound) => {}
        Ok(_) => return Err(LocalFileCopyError::ConflictStale),
        Err(error) => return Err(LocalFileCopyError::Remote(error)),
    }
    run_scenario_rule_delete_after_remote_check(
        database_path,
        registry_path,
        scenario_id,
        scenario_revision,
        relative_path,
        on_left,
        job_id,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_paths_must_stay_inside_the_root() {
        assert!(plain_relative("a/b.txt"));
        assert!(!plain_relative(""));
        assert!(!plain_relative("/a"));
        assert!(!plain_relative("a/../b"));
        assert!(!plain_relative("./a"));
        assert_eq!(yandex_path("disk:/", "a/b"), "disk:/a/b");
        assert_eq!(yandex_path("disk:/Photos", "a"), "disk:/Photos/a");
    }
}
