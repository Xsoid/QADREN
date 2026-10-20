//! Release 1 local Worker process.
//!
//! The Worker owns periodic schedule staging and JobStore dispatch. Its
//! private Unix socket is intentionally local-only. It does not expose a
//! network or remote-Worker protocol.

use qadren_core::confirmation::{Confirmation, EvidenceLevel};
use qadren_core::job_execution::{
    reconcile_local_file_copy, run_local_file_copy, run_local_file_replace,
    run_local_scenario_conflict_delete, run_local_scenario_conflict_replace,
    run_local_scenario_file_copy, run_local_scenario_file_replace, LocalFileCopyError,
    LocalFileCopyReconciliationStatus, LocalFileCopyStatus,
};
use qadren_core::jobs::JobStore;
use qadren_core::local_fs::LocalFilesystem;
use qadren_core::model::{ConfirmedFileVersion, JobState, ScenarioReference};
use qadren_core::registry::Registry;
use qadren_core::scenario_planner::{ObservedFile, PlanAction};
use qadren_core::schedule_dispatch::dispatch_pending_schedule_jobs;
use serde::{Deserialize, Serialize};
use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, SystemTime};

const POLL_INTERVAL: Duration = Duration::from_secs(60);
const POLL_OVERLAP: Duration = Duration::from_secs(90);
const MAX_SCHEDULE_SCAN_ENTRIES: usize = 10_000;

#[derive(Debug)]
struct WorkerPaths {
    socket: PathBuf,
    registry: PathBuf,
    jobs: PathBuf,
}

#[derive(Debug)]
struct WorkerLock {
    _file: File,
}

impl WorkerLock {
    fn acquire(socket: &Path) -> Result<Self, String> {
        let parent = socket
            .parent()
            .ok_or_else(|| "worker_socket_parent_missing".to_string())?;
        let parent_existed = parent.exists();
        fs::create_dir_all(parent)
            .map_err(|_| "worker_socket_directory_unavailable".to_string())?;
        if !parent_existed {
            fs::set_permissions(parent, fs::Permissions::from_mode(0o700))
                .map_err(|_| "worker_socket_directory_unavailable".to_string())?;
        }
        let parent_metadata = fs::symlink_metadata(parent)
            .map_err(|_| "worker_socket_directory_unavailable".to_string())?;
        if !parent_metadata.file_type().is_dir()
            || parent_metadata.file_type().is_symlink()
            || parent_metadata.permissions().mode() & 0o077 != 0
        {
            return Err("worker_socket_directory_unsafe".to_string());
        }
        let lock_path = parent.join("worker.lock");
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(lock_path)
            .map_err(|_| "worker_lock_unavailable".to_string())?;
        let result = unsafe {
            libc::flock(
                std::os::unix::io::AsRawFd::as_raw_fd(&file),
                libc::LOCK_EX | libc::LOCK_NB,
            )
        };
        if result != 0 {
            return Err("worker_already_running".to_string());
        }
        Ok(Self { _file: file })
    }
}

#[derive(Debug, Deserialize)]
struct Request {
    command: String,
    #[serde(default)]
    scenario_id: Option<String>,
    #[serde(default)]
    scenario_revision: Option<u64>,
    #[serde(default)]
    source_root: Option<String>,
    #[serde(default)]
    source_connection_id: Option<String>,
    #[serde(default)]
    source_relative_path: Option<String>,
    #[serde(default)]
    destination_root: Option<String>,
    #[serde(default)]
    destination_connection_id: Option<String>,
    #[serde(default)]
    destination_relative_path: Option<String>,
    #[serde(default)]
    job_id: Option<String>,
    #[serde(default)]
    replacement_consent: Option<bool>,
    #[serde(default)]
    conflict_id: Option<String>,
    #[serde(default)]
    destructive_consent: Option<bool>,
    #[serde(default)]
    locally_paused: Option<bool>,
    #[serde(default)]
    source_is_left: Option<bool>,
    #[serde(default)]
    decision: Option<String>,
}

#[derive(Debug, Serialize)]
struct Response<'a> {
    ok: bool,
    status: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    staged: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dispatched: Option<usize>,
}

fn now() -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::<chrono::Utc>::from(SystemTime::now())
}

fn dispatch_due(paths: &WorkerPaths) -> Result<(usize, usize), String> {
    let through = now();
    let after = through
        - chrono::Duration::from_std(POLL_OVERLAP)
            .map_err(|_| "worker_clock_unavailable".to_string())?;
    let registry =
        Registry::open(&paths.registry).map_err(|_| "registry_unavailable".to_string())?;
    reconcile_unknown_scheduled_copies(paths, &registry)?;
    let staged = registry
        .stage_due_schedule_occurrences(after, through)
        .map_err(|_| "schedule_stage_unavailable".to_string())?;
    let mut jobs = JobStore::open(&paths.jobs).map_err(|_| "job_store_unavailable".to_string())?;
    let dispatched = dispatch_pending_schedule_jobs(&registry, &mut jobs)
        .map_err(|_| "schedule_dispatch_unavailable".to_string())?;
    drop(jobs);
    execute_queued_schedule_jobs(paths, &registry)?;
    Ok((staged, dispatched))
}

/// Reconcile only UNKNOWN child copies whose schedule mapping identifies an
/// exact Local destination. This reads metadata and never retries an effect.
fn reconcile_unknown_scheduled_copies(
    paths: &WorkerPaths,
    registry: &Registry,
) -> Result<(), String> {
    let candidates = {
        let store = JobStore::open(&paths.jobs).map_err(|_| "job_store_unavailable".to_string())?;
        let unknown = store
            .list_jobs()
            .map_err(|_| "job_store_unavailable".to_string())?
            .into_iter()
            .filter(|job| job.state == JobState::Unknown)
            .map(|job| job.id)
            .collect::<Vec<_>>();
        let attempts = store
            .unresolved_attempts()
            .map_err(|_| "job_store_unavailable".to_string())?;
        unknown
            .into_iter()
            .filter_map(|job_id| {
                attempts
                    .iter()
                    .find(|attempt| attempt.job_id == job_id && attempt.status == "unknown")
                    .and_then(|attempt| attempt.target_connection_id.as_deref())
                    .map(|connection_id| (job_id, connection_id.to_owned()))
            })
            .collect::<Vec<_>>()
    };
    for (job_id, destination_connection_id) in candidates {
        let Some(reference) = registry
            .schedule_child_job_scenario(&job_id)
            .map_err(|_| "schedule_registry_unavailable".to_string())?
        else {
            continue;
        };
        let scenario = registry
            .get_scenario_revision(&reference.scenario_id, reference.revision)
            .map_err(|_| "schedule_scenario_unavailable".to_string())?
            .ok_or_else(|| "schedule_scenario_unavailable".to_string())?;
        let destination_root = if destination_connection_id == scenario.left_connection_id {
            &scenario.left_root
        } else if destination_connection_id == scenario.right_connection_id {
            &scenario.right_root
        } else {
            continue;
        };
        match reconcile_local_file_copy(
            &paths.jobs,
            &paths.registry,
            Path::new(destination_root),
            &destination_connection_id,
            &job_id,
        ) {
            Ok(
                LocalFileCopyReconciliationStatus::Completed
                | LocalFileCopyReconciliationStatus::StillUnknown
                | LocalFileCopyReconciliationStatus::Existing(_),
            ) => {}
            Err(_) => {}
        }
    }
    Ok(())
}

fn observed_files(root: &Path) -> Result<Vec<ObservedFile>, String> {
    let filesystem =
        LocalFilesystem::open(root).map_err(|_| "schedule_root_unavailable".to_string())?;
    filesystem
        .list_tree(Path::new(""), MAX_SCHEDULE_SCAN_ENTRIES)
        .map_err(|_| "schedule_scan_unavailable".to_string())
        .map(|entries| {
            entries
                .into_iter()
                .filter(|(_, entry)| !entry.is_directory)
                .map(|(path, entry)| ObservedFile {
                    relative_path: path.to_string_lossy().into_owned(),
                    object_revision: entry.object_revision,
                    size: entry.size,
                    checksum_md5: None,
                })
                .collect()
        })
}

fn enabled_local_connection(registry: &Registry, connection_id: &str) -> Result<(), String> {
    let enabled = registry
        .list()
        .map_err(|_| "schedule_registry_unavailable".to_string())?
        .into_iter()
        .any(|connection| {
            connection.id == connection_id
                && connection.enabled
                && connection.adapter == "local_filesystem"
        });
    if enabled {
        Ok(())
    } else {
        Err("schedule_local_connection_unavailable".to_string())
    }
}

fn observed_version(files: &[ObservedFile], relative_path: &str) -> Option<ConfirmedFileVersion> {
    files
        .iter()
        .find(|file| file.relative_path == relative_path)
        .map(|file| ConfirmedFileVersion {
            object_revision: file.object_revision.clone(),
            size: file.size,
            checksum_md5: file.checksum_md5.clone(),
        })
}

fn finish_schedule_job(jobs: &mut JobStore, job_id: &str, next: JobState) -> Result<(), String> {
    jobs.advance(
        job_id,
        JobState::Queued,
        JobState::Running,
        Confirmation::Unknown,
    )
    .map_err(|_| "schedule_job_state_unavailable".to_string())?;
    jobs.advance(
        job_id,
        JobState::Running,
        next,
        if next == JobState::Completed {
            Confirmation::Confirmed(EvidenceLevel::ProviderAcknowledged)
        } else {
            Confirmation::Unknown
        },
    )
    .map_err(|_| "schedule_job_state_unavailable".to_string())
}

/// Execute each queued schedule coordinator once. It scans only metadata at
/// run time. A new destination is copied through the existing durable Create
/// executor. Replacements, deletions, and conflicting metadata become durable
/// conflict tasks and require a later explicit UI decision.
fn execute_queued_schedule_jobs(paths: &WorkerPaths, registry: &Registry) -> Result<(), String> {
    let queued = JobStore::open(&paths.jobs)
        .map_err(|_| "job_store_unavailable".to_string())?
        .list_jobs()
        .map_err(|_| "job_store_unavailable".to_string())?
        .into_iter()
        .filter(|job| job.state == JobState::Queued);
    for job in queued {
        let Some(reference) = registry
            .schedule_job_reference(&job.id)
            .map_err(|_| "schedule_registry_unavailable".to_string())?
        else {
            continue;
        };
        if job.scenario_id.as_deref() != Some(reference.scenario_id.as_str())
            || job.scenario_revision != Some(reference.revision)
        {
            return Err("schedule_job_identity_mismatch".to_string());
        }
        let result = execute_schedule_job(paths, registry, &reference, &job.id);
        let mut jobs =
            JobStore::open(&paths.jobs).map_err(|_| "job_store_unavailable".to_string())?;
        match result {
            Ok(waiting_for_decision) => finish_schedule_job(
                &mut jobs,
                &job.id,
                if waiting_for_decision {
                    JobState::WaitingForDecision
                } else {
                    JobState::Completed
                },
            )?,
            Err(error) if error == "schedule_outcome_unknown" => {
                finish_schedule_job(&mut jobs, &job.id, JobState::Unknown)?
            }
            Err(_) => finish_schedule_job(&mut jobs, &job.id, JobState::Failed)?,
        }
    }
    Ok(())
}

fn execute_schedule_job(
    paths: &WorkerPaths,
    registry: &Registry,
    reference: &ScenarioReference,
    schedule_job_id: &str,
) -> Result<bool, String> {
    let scenario = registry
        .get_scenario_revision(&reference.scenario_id, reference.revision)
        .map_err(|_| "schedule_scenario_unavailable".to_string())?
        .ok_or_else(|| "schedule_scenario_unavailable".to_string())?;
    enabled_local_connection(registry, &scenario.left_connection_id)?;
    enabled_local_connection(registry, &scenario.right_connection_id)?;
    let left = observed_files(Path::new(&scenario.left_root))?;
    let right = observed_files(Path::new(&scenario.right_root))?;
    let plan = registry
        .plan_scenario_dry_run(reference, &left, &right)
        .map_err(|_| "schedule_plan_unavailable".to_string())?;
    let mut waiting_for_decision = false;
    for item in plan {
        match item.action {
            PlanAction::NoAction => {}
            PlanAction::Create { from_left } => {
                let (source_root, source_connection, destination_root, destination_connection) =
                    if from_left {
                        (
                            &scenario.left_root,
                            &scenario.left_connection_id,
                            &scenario.right_root,
                            &scenario.right_connection_id,
                        )
                    } else {
                        (
                            &scenario.right_root,
                            &scenario.right_connection_id,
                            &scenario.left_root,
                            &scenario.left_connection_id,
                        )
                    };
                let status = run_local_scenario_file_copy(
                    &paths.jobs,
                    &paths.registry,
                    &reference.scenario_id,
                    reference.revision,
                    Path::new(source_root),
                    source_connection,
                    Path::new(&item.relative_path),
                    Path::new(destination_root),
                    destination_connection,
                    Path::new(&item.relative_path),
                    &registry
                        .reserve_schedule_child_job_id(schedule_job_id, &item.relative_path)
                        .map_err(|_| "schedule_child_identity_unavailable".to_string())?,
                );
                match status {
                    Ok(
                        LocalFileCopyStatus::Completed
                        | LocalFileCopyStatus::Existing(JobState::Completed),
                    ) => {}
                    Ok(LocalFileCopyStatus::Existing(_)) => {
                        return Err("schedule_child_not_completed".to_string())
                    }
                    Err(LocalFileCopyError::OutcomeUnknown(_)) => {
                        return Err("schedule_outcome_unknown".to_string())
                    }
                    Err(LocalFileCopyError::TargetExists) => {
                        let source = observed_version(
                            if from_left { &left } else { &right },
                            &item.relative_path,
                        );
                        let destination = observed_version(
                            if from_left { &right } else { &left },
                            &item.relative_path,
                        );
                        if let (Some(source), Some(destination)) = (source, destination) {
                            registry
                                .queue_conflict_task(
                                    reference,
                                    &item.relative_path,
                                    "content",
                                    Some(from_left),
                                    Some(&if from_left {
                                        source.clone()
                                    } else {
                                        destination.clone()
                                    }),
                                    Some(&if from_left { destination } else { source }),
                                )
                                .map_err(|_| "schedule_conflict_unavailable".to_string())?;
                            waiting_for_decision = true;
                        } else {
                            return Err("schedule_plan_stale".to_string());
                        }
                    }
                    Err(_) => {
                        return Err("schedule_create_unavailable".to_string());
                    }
                }
            }
            PlanAction::Update { from_left } => {
                let left_version = observed_version(&left, &item.relative_path)
                    .ok_or_else(|| "schedule_conflict_unavailable".to_string())?;
                let right_version = observed_version(&right, &item.relative_path)
                    .ok_or_else(|| "schedule_conflict_unavailable".to_string())?;
                registry
                    .queue_conflict_task(
                        reference,
                        &item.relative_path,
                        "content",
                        Some(from_left),
                        Some(&left_version),
                        Some(&right_version),
                    )
                    .map_err(|_| "schedule_conflict_unavailable".to_string())?;
                waiting_for_decision = true;
            }
            PlanAction::Conflict { .. } => {
                let left_version = observed_version(&left, &item.relative_path)
                    .ok_or_else(|| "schedule_conflict_unavailable".to_string())?;
                let right_version = observed_version(&right, &item.relative_path)
                    .ok_or_else(|| "schedule_conflict_unavailable".to_string())?;
                registry
                    .queue_conflict_task(
                        reference,
                        &item.relative_path,
                        "content",
                        None,
                        Some(&left_version),
                        Some(&right_version),
                    )
                    .map_err(|_| "schedule_conflict_unavailable".to_string())?;
                waiting_for_decision = true;
            }
            PlanAction::DeleteRequiresDecision { .. } => {
                let left_version = observed_version(&left, &item.relative_path);
                let right_version = observed_version(&right, &item.relative_path);
                registry
                    .queue_conflict_task(
                        reference,
                        &item.relative_path,
                        "deletion",
                        None,
                        left_version.as_ref(),
                        right_version.as_ref(),
                    )
                    .map_err(|_| "schedule_conflict_unavailable".to_string())?;
                waiting_for_decision = true;
            }
            PlanAction::NeedsVerification { .. } => {
                return Err("schedule_metadata_needs_verification".to_string())
            }
        }
    }
    Ok(waiting_for_decision)
}

fn remove_stale_socket(socket: &Path) -> Result<(), String> {
    match fs::symlink_metadata(socket) {
        Ok(metadata) if metadata.file_type().is_socket() => {
            fs::remove_file(socket).map_err(|_| "worker_socket_unavailable".to_string())
        }
        Ok(_) => Err("worker_socket_path_unsafe".to_string()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err("worker_socket_unavailable".to_string()),
    }
}

fn execute_create(paths: &WorkerPaths, request: &Request) -> Result<&'static str, &'static str> {
    let (
        Some(scenario_id),
        Some(scenario_revision),
        Some(source_root),
        Some(source_connection_id),
        Some(source_relative_path),
        Some(destination_root),
        Some(destination_connection_id),
        Some(destination_relative_path),
        Some(job_id),
    ) = (
        request.scenario_id.as_deref(),
        request.scenario_revision,
        request.source_root.as_deref(),
        request.source_connection_id.as_deref(),
        request.source_relative_path.as_deref(),
        request.destination_root.as_deref(),
        request.destination_connection_id.as_deref(),
        request.destination_relative_path.as_deref(),
        request.job_id.as_deref(),
    )
    else {
        return Err("invalid_request");
    };
    match run_local_scenario_file_copy(
        &paths.jobs,
        &paths.registry,
        scenario_id,
        scenario_revision,
        Path::new(source_root),
        source_connection_id,
        Path::new(source_relative_path),
        Path::new(destination_root),
        destination_connection_id,
        Path::new(destination_relative_path),
        job_id,
    ) {
        Ok(LocalFileCopyStatus::Completed) => Ok("completed"),
        Ok(LocalFileCopyStatus::Existing(_)) => Ok("existing"),
        Err(LocalFileCopyError::TargetExists) => Err("target_exists"),
        Err(LocalFileCopyError::ConnectionUnavailable) => Err("local_connection_unavailable"),
        Err(LocalFileCopyError::ScenarioUnavailable) => Err("scenario_unavailable"),
        Err(LocalFileCopyError::ScenarioScopeMismatch) => Err("scenario_scope_mismatch"),
        Err(LocalFileCopyError::CapabilityUnavailable) => Err("local_capability_unavailable"),
        Err(LocalFileCopyError::VerificationFailed) => Err("verification_failed"),
        Err(LocalFileCopyError::OutcomeUnknown(_)) => Err("outcome_unknown"),
        Err(LocalFileCopyError::InvalidRequest) => Err("invalid_request"),
        Err(LocalFileCopyError::Local(_)) => Err("local_operation_failed"),
        Err(LocalFileCopyError::Store(_)) => Err("job_store_unavailable"),
        Err(_) => Err("scenario_create_unavailable"),
    }
}

fn execute_replace(paths: &WorkerPaths, request: &Request) -> Result<&'static str, &'static str> {
    let (
        Some(scenario_id),
        Some(scenario_revision),
        Some(source_root),
        Some(source_connection_id),
        Some(source_relative_path),
        Some(destination_root),
        Some(destination_connection_id),
        Some(destination_relative_path),
        Some(job_id),
    ) = (
        request.scenario_id.as_deref(),
        request.scenario_revision,
        request.source_root.as_deref(),
        request.source_connection_id.as_deref(),
        request.source_relative_path.as_deref(),
        request.destination_root.as_deref(),
        request.destination_connection_id.as_deref(),
        request.destination_relative_path.as_deref(),
        request.job_id.as_deref(),
    )
    else {
        return Err("invalid_request");
    };
    match run_local_scenario_file_replace(
        &paths.jobs,
        &paths.registry,
        scenario_id,
        scenario_revision,
        Path::new(source_root),
        source_connection_id,
        Path::new(source_relative_path),
        Path::new(destination_root),
        destination_connection_id,
        Path::new(destination_relative_path),
        job_id,
        request.replacement_consent.unwrap_or(false),
    ) {
        Ok(LocalFileCopyStatus::Completed) => Ok("completed"),
        Ok(LocalFileCopyStatus::Existing(_)) => Ok("existing"),
        Err(LocalFileCopyError::ConsentRequired) => Err("consent_required"),
        Err(LocalFileCopyError::TargetExists) => Err("target_exists"),
        Err(LocalFileCopyError::ConnectionUnavailable) => Err("local_connection_unavailable"),
        Err(LocalFileCopyError::ScenarioUnavailable) => Err("scenario_unavailable"),
        Err(LocalFileCopyError::ScenarioScopeMismatch) => Err("scenario_scope_mismatch"),
        Err(LocalFileCopyError::CapabilityUnavailable) => Err("local_capability_unavailable"),
        Err(LocalFileCopyError::VerificationFailed) => Err("verification_failed"),
        Err(LocalFileCopyError::OutcomeUnknown(_)) => Err("outcome_unknown"),
        Err(LocalFileCopyError::InvalidRequest) => Err("invalid_request"),
        Err(LocalFileCopyError::Local(_)) => Err("local_operation_failed"),
        Err(LocalFileCopyError::Store(_)) => Err("job_store_unavailable"),
        Err(_) => Err("scenario_replace_unavailable"),
    }
}

fn execute_manual_copy(
    paths: &WorkerPaths,
    request: &Request,
    replace: bool,
) -> Result<&'static str, &'static str> {
    let (
        Some(source_root),
        Some(source_connection_id),
        Some(source_relative_path),
        Some(destination_root),
        Some(destination_connection_id),
        Some(destination_relative_path),
        Some(job_id),
    ) = (
        request.source_root.as_deref(),
        request.source_connection_id.as_deref(),
        request.source_relative_path.as_deref(),
        request.destination_root.as_deref(),
        request.destination_connection_id.as_deref(),
        request.destination_relative_path.as_deref(),
        request.job_id.as_deref(),
    )
    else {
        return Err("invalid_request");
    };
    let result = if replace {
        run_local_file_replace(
            &paths.jobs,
            &paths.registry,
            Path::new(source_root),
            source_connection_id,
            Path::new(source_relative_path),
            Path::new(destination_root),
            destination_connection_id,
            Path::new(destination_relative_path),
            job_id,
            request.replacement_consent.unwrap_or(false),
        )
    } else {
        run_local_file_copy(
            &paths.jobs,
            &paths.registry,
            Path::new(source_root),
            source_connection_id,
            Path::new(source_relative_path),
            Path::new(destination_root),
            destination_connection_id,
            Path::new(destination_relative_path),
            job_id,
        )
    };
    match result {
        Ok(LocalFileCopyStatus::Completed) => Ok("completed"),
        Ok(LocalFileCopyStatus::Existing(_)) => Ok("existing"),
        Err(LocalFileCopyError::ConsentRequired) => Err("consent_required"),
        Err(LocalFileCopyError::TargetExists) => Err("target_exists"),
        Err(LocalFileCopyError::ConnectionUnavailable) => Err("local_connection_unavailable"),
        Err(LocalFileCopyError::CapabilityUnavailable) => Err("local_capability_unavailable"),
        Err(LocalFileCopyError::VerificationFailed) => Err("verification_failed"),
        Err(LocalFileCopyError::OutcomeUnknown(_)) => Err("outcome_unknown"),
        Err(LocalFileCopyError::InvalidRequest) => Err("invalid_request"),
        Err(LocalFileCopyError::Local(_)) => Err("local_operation_failed"),
        Err(LocalFileCopyError::Store(_)) => Err("job_store_unavailable"),
        Err(_) => Err("manual_copy_unavailable"),
    }
}

fn execute_conflict(
    paths: &WorkerPaths,
    request: &Request,
    delete: bool,
) -> Result<&'static str, &'static str> {
    let (Some(conflict_id), Some(job_id)) =
        (request.conflict_id.as_deref(), request.job_id.as_deref())
    else {
        return Err("invalid_request");
    };
    let result = if delete {
        run_local_scenario_conflict_delete(
            &paths.jobs,
            &paths.registry,
            conflict_id,
            job_id,
            request.destructive_consent.unwrap_or(false),
        )
    } else {
        run_local_scenario_conflict_replace(
            &paths.jobs,
            &paths.registry,
            conflict_id,
            job_id,
            request.destructive_consent.unwrap_or(false),
        )
    };
    match result {
        Ok(LocalFileCopyStatus::Completed) => Ok("completed"),
        Ok(LocalFileCopyStatus::Existing(_)) => Ok("existing"),
        Err(LocalFileCopyError::ConsentRequired) => Err("consent_required"),
        Err(LocalFileCopyError::ConflictStale) => Err("conflict_stale"),
        Err(LocalFileCopyError::ConflictUnavailable) => Err("conflict_unavailable"),
        Err(LocalFileCopyError::CapabilityUnavailable) => Err("local_capability_unavailable"),
        Err(LocalFileCopyError::OutcomeUnknown(_)) => Err("outcome_unknown"),
        Err(LocalFileCopyError::Store(_)) => Err("job_store_unavailable"),
        Err(LocalFileCopyError::Local(_)) => Err("local_operation_failed"),
        Err(_) => Err("conflict_operation_failed"),
    }
}

fn update_registry_control(
    paths: &WorkerPaths,
    request: &Request,
) -> Result<&'static str, &'static str> {
    let registry = Registry::open(&paths.registry).map_err(|_| "registry_unavailable")?;
    match request.command.as_str() {
        "set_scenario_paused" => {
            let (Some(id), Some(paused)) = (request.scenario_id.as_deref(), request.locally_paused)
            else {
                return Err("invalid_request");
            };
            registry
                .set_scenario_locally_paused(id, paused)
                .map_err(|_| "scenario_unavailable")?;
            Ok("updated")
        }
        "select_conflict_decision" => {
            let (Some(id), Some(source_is_left), Some(decision)) = (
                request.conflict_id.as_deref(),
                request.source_is_left,
                request.decision.as_deref(),
            ) else {
                return Err("invalid_request");
            };
            registry
                .select_conflict_task(id, Some(source_is_left), decision)
                .map_err(|_| "conflict_unavailable")?;
            Ok("updated")
        }
        _ => Err("unsupported_command"),
    }
}

fn respond(stream: &mut UnixStream, response: &Response<'_>) {
    if let Ok(json) = serde_json::to_string(response) {
        let _ = writeln!(stream, "{json}");
    }
}

fn handle_client(mut stream: UnixStream, paths: &WorkerPaths, stop: &mut bool) {
    let request = {
        let mut line = String::new();
        let mut reader = BufReader::new(&stream);
        if reader.read_line(&mut line).is_err() {
            return;
        }
        serde_json::from_str::<Request>(&line).ok()
    };
    let Some(request) = request else {
        respond(
            &mut stream,
            &Response {
                ok: false,
                status: "invalid_request",
                staged: None,
                dispatched: None,
            },
        );
        return;
    };
    match request.command.as_str() {
        "status" => respond(
            &mut stream,
            &Response {
                ok: true,
                status: "running",
                staged: None,
                dispatched: None,
            },
        ),
        "poll" => match dispatch_due(paths) {
            Ok((staged, dispatched)) => respond(
                &mut stream,
                &Response {
                    ok: true,
                    status: "running",
                    staged: Some(staged),
                    dispatched: Some(dispatched),
                },
            ),
            Err(status) => respond(
                &mut stream,
                &Response {
                    ok: false,
                    status: &status,
                    staged: None,
                    dispatched: None,
                },
            ),
        },
        "execute_create" => match execute_create(paths, &request) {
            Ok(status) => respond(
                &mut stream,
                &Response {
                    ok: true,
                    status,
                    staged: None,
                    dispatched: None,
                },
            ),
            Err(status) => respond(
                &mut stream,
                &Response {
                    ok: false,
                    status,
                    staged: None,
                    dispatched: None,
                },
            ),
        },
        "execute_replace" => match execute_replace(paths, &request) {
            Ok(status) => respond(
                &mut stream,
                &Response {
                    ok: true,
                    status,
                    staged: None,
                    dispatched: None,
                },
            ),
            Err(status) => respond(
                &mut stream,
                &Response {
                    ok: false,
                    status,
                    staged: None,
                    dispatched: None,
                },
            ),
        },
        "execute_manual_copy" | "execute_manual_replace" => {
            match execute_manual_copy(paths, &request, request.command == "execute_manual_replace")
            {
                Ok(status) => respond(
                    &mut stream,
                    &Response {
                        ok: true,
                        status,
                        staged: None,
                        dispatched: None,
                    },
                ),
                Err(status) => respond(
                    &mut stream,
                    &Response {
                        ok: false,
                        status,
                        staged: None,
                        dispatched: None,
                    },
                ),
            }
        }
        "execute_conflict_replace" => match execute_conflict(paths, &request, false) {
            Ok(status) => respond(
                &mut stream,
                &Response {
                    ok: true,
                    status,
                    staged: None,
                    dispatched: None,
                },
            ),
            Err(status) => respond(
                &mut stream,
                &Response {
                    ok: false,
                    status,
                    staged: None,
                    dispatched: None,
                },
            ),
        },
        "execute_conflict_delete" => match execute_conflict(paths, &request, true) {
            Ok(status) => respond(
                &mut stream,
                &Response {
                    ok: true,
                    status,
                    staged: None,
                    dispatched: None,
                },
            ),
            Err(status) => respond(
                &mut stream,
                &Response {
                    ok: false,
                    status,
                    staged: None,
                    dispatched: None,
                },
            ),
        },
        "set_scenario_paused" | "select_conflict_decision" => {
            match update_registry_control(paths, &request) {
                Ok(status) => respond(
                    &mut stream,
                    &Response {
                        ok: true,
                        status,
                        staged: None,
                        dispatched: None,
                    },
                ),
                Err(status) => respond(
                    &mut stream,
                    &Response {
                        ok: false,
                        status,
                        staged: None,
                        dispatched: None,
                    },
                ),
            }
        }
        "stop" => {
            *stop = true;
            respond(
                &mut stream,
                &Response {
                    ok: true,
                    status: "stopping",
                    staged: None,
                    dispatched: None,
                },
            );
        }
        _ => respond(
            &mut stream,
            &Response {
                ok: false,
                status: "unsupported_command",
                staged: None,
                dispatched: None,
            },
        ),
    }
}

fn run(paths: WorkerPaths, once: bool) -> Result<(), String> {
    let _lock = WorkerLock::acquire(&paths.socket)?;
    if once {
        dispatch_due(&paths)?;
        return Ok(());
    }
    remove_stale_socket(&paths.socket)?;
    let listener =
        UnixListener::bind(&paths.socket).map_err(|_| "worker_socket_unavailable".to_string())?;
    fs::set_permissions(&paths.socket, fs::Permissions::from_mode(0o600))
        .map_err(|_| "worker_socket_unavailable".to_string())?;
    listener
        .set_nonblocking(true)
        .map_err(|_| "worker_socket_unavailable".to_string())?;
    let mut stop = false;
    let mut next_poll = SystemTime::now();
    while !stop {
        if SystemTime::now() >= next_poll {
            let _ = dispatch_due(&paths);
            next_poll = SystemTime::now() + POLL_INTERVAL;
        }
        match listener.accept() {
            Ok((stream, _)) => handle_client(stream, &paths, &mut stop),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(50))
            }
            Err(_) => return Err("worker_socket_unavailable".to_string()),
        }
    }
    let _ = fs::remove_file(&paths.socket);
    Ok(())
}

fn usage() -> ! {
    eprintln!("usage: qadren-worker [--socket PATH --registry PATH --jobs PATH] [--once]");
    std::process::exit(64);
}

fn parse_paths() -> (WorkerPaths, bool) {
    let mut socket = None;
    let mut registry = None;
    let mut jobs = None;
    let mut once = false;
    let mut args = env::args().skip(1);
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--socket" => socket = args.next().map(PathBuf::from),
            "--registry" => registry = args.next().map(PathBuf::from),
            "--jobs" => jobs = args.next().map(PathBuf::from),
            "--once" => once = true,
            _ => usage(),
        }
    }
    match (socket, registry, jobs) {
        (None, None, None) => {
            let home = env::var_os("HOME").unwrap_or_else(|| usage());
            let base = PathBuf::from(home)
                .join("Library")
                .join("Application Support")
                .join("QADREN");
            (
                WorkerPaths {
                    socket: base.join("worker.sock"),
                    registry: base.join("connections.sqlite3"),
                    jobs: base.join("jobs.sqlite3"),
                },
                once,
            )
        }
        (Some(socket), Some(registry), Some(jobs)) => (
            WorkerPaths {
                socket,
                registry,
                jobs,
            },
            once,
        ),
        _ => usage(),
    }
}

fn main() {
    let (paths, once) = parse_paths();
    if let Err(error) = run(paths, once) {
        eprintln!("qadren-worker: {error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Datelike, Timelike};
    use qadren_core::local_fs::LocalFilesystem;
    use qadren_core::model::{ConfirmedFileVersion, JobState, ScenarioReference};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn root(name: &str) -> PathBuf {
        let path = std::env::current_dir()
            .unwrap()
            .join("target")
            .join(format!(
                "qadren-worker-{name}-{}",
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
        fs::create_dir_all(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        path
    }

    #[test]
    fn duplicate_worker_lock_is_denied_before_store_access() {
        let root = root("lock");
        let socket = root.join("worker.sock");
        let first = WorkerLock::acquire(&socket).unwrap();
        assert_eq!(
            WorkerLock::acquire(&socket).unwrap_err(),
            "worker_already_running"
        );
        drop(first);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn one_poll_stages_and_dispatches_a_due_scenario_job() {
        let root = root("poll");
        let left_root = root.join("left");
        let right_root = root.join("right");
        fs::create_dir_all(&left_root).unwrap();
        fs::create_dir_all(&right_root).unwrap();
        let paths = WorkerPaths {
            socket: root.join("worker.sock"),
            registry: root.join("registry.sqlite3"),
            jobs: root.join("jobs.sqlite3"),
        };
        let registry = Registry::open(&paths.registry).unwrap();
        let left = registry.add("local_filesystem", "Left").unwrap();
        let right = registry.add("local_filesystem", "Right").unwrap();
        let scenario = registry
            .create_scenario(
                "Documents",
                &left,
                left_root.to_str().unwrap(),
                &right,
                right_root.to_str().unwrap(),
                "left_to_right",
            )
            .unwrap();
        let current = now();
        registry
            .create_scenario_schedule(
                &scenario,
                "UTC",
                current.weekday().num_days_from_monday() as u8,
                current.hour() as u8,
                current.minute() as u8,
            )
            .unwrap();
        let (staged, dispatched) = dispatch_due(&paths).unwrap();
        assert_eq!((staged, dispatched), (1, 1));
        assert_eq!(
            JobStore::open(&paths.jobs)
                .unwrap()
                .list_jobs()
                .unwrap()
                .len(),
            1
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn scheduled_run_scans_roots_copies_new_files_and_queues_existing_file_updates() {
        let root = root("scheduled-execution");
        let left_root = root.join("left");
        let right_root = root.join("right");
        fs::create_dir_all(&left_root).unwrap();
        fs::create_dir_all(&right_root).unwrap();
        fs::write(left_root.join("report.txt"), b"first version").unwrap();
        let paths = WorkerPaths {
            socket: root.join("worker.sock"),
            registry: root.join("registry.sqlite3"),
            jobs: root.join("jobs.sqlite3"),
        };
        let registry = Registry::open(&paths.registry).unwrap();
        let left = registry.add("local_filesystem", "Left").unwrap();
        let right = registry.add("local_filesystem", "Right").unwrap();
        let scenario = registry
            .create_scenario(
                "Scheduled",
                &left,
                left_root.to_str().unwrap(),
                &right,
                right_root.to_str().unwrap(),
                "left_to_right",
            )
            .unwrap();
        let current = now();
        registry
            .create_scenario_schedule(
                &scenario,
                "UTC",
                current.weekday().num_days_from_monday() as u8,
                current.hour() as u8,
                current.minute() as u8,
            )
            .unwrap();
        drop(registry);

        assert_eq!(dispatch_due(&paths).unwrap(), (1, 1));
        assert_eq!(
            fs::read(right_root.join("report.txt")).unwrap(),
            b"first version"
        );
        let first_jobs = JobStore::open(&paths.jobs).unwrap().list_jobs().unwrap();
        assert_eq!(first_jobs.len(), 2);
        assert!(first_jobs
            .iter()
            .any(|job| job.id.starts_with("schedule-") && job.state == JobState::Completed));
        assert!(first_jobs
            .iter()
            .any(|job| job.id.starts_with("schedule-copy-") && job.state == JobState::Completed));

        fs::write(left_root.join("report.txt"), b"second version").unwrap();
        let registry = Registry::open(&paths.registry).unwrap();
        registry
            .create_scenario_schedule(
                &scenario,
                "UTC",
                current.weekday().num_days_from_monday() as u8,
                current.hour() as u8,
                current.minute() as u8,
            )
            .unwrap();
        drop(registry);
        assert_eq!(dispatch_due(&paths).unwrap(), (1, 1));
        assert_eq!(
            fs::read(right_root.join("report.txt")).unwrap(),
            b"first version"
        );
        let registry = Registry::open(&paths.registry).unwrap();
        let conflicts = registry
            .list_conflict_tasks(&ScenarioReference {
                scenario_id: scenario,
                revision: 1,
            })
            .unwrap();
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].kind, "content");
        assert_eq!(conflicts[0].state, "pending");
        let jobs = JobStore::open(&paths.jobs).unwrap().list_jobs().unwrap();
        assert!(jobs
            .iter()
            .any(|job| job.state == JobState::WaitingForDecision));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn worker_reconciles_unknown_scheduled_copy_after_restart_without_replay() {
        let root = root("scheduled-reconciliation");
        let left_root = root.join("left");
        let right_root = root.join("right");
        fs::create_dir_all(&left_root).unwrap();
        fs::create_dir_all(&right_root).unwrap();
        fs::write(left_root.join("report.txt"), b"payload").unwrap();
        fs::write(right_root.join("report.txt"), b"payload").unwrap();
        let paths = WorkerPaths {
            socket: root.join("worker.sock"),
            registry: root.join("registry.sqlite3"),
            jobs: root.join("jobs.sqlite3"),
        };
        let registry = Registry::open(&paths.registry).unwrap();
        let left = registry.add("local_filesystem", "Left").unwrap();
        let right = registry.add("local_filesystem", "Right").unwrap();
        let scenario = registry
            .create_scenario(
                "Reconcile",
                &left,
                left_root.to_str().unwrap(),
                &right,
                right_root.to_str().unwrap(),
                "left_to_right",
            )
            .unwrap();
        let current = now();
        registry
            .create_scenario_schedule(
                &scenario,
                "UTC",
                current.weekday().num_days_from_monday() as u8,
                current.hour() as u8,
                current.minute() as u8,
            )
            .unwrap();
        registry
            .stage_due_schedule_occurrences(current - chrono::Duration::seconds(90), current)
            .unwrap();
        let parent = registry
            .pending_schedule_job_outbox()
            .unwrap()
            .pop()
            .unwrap();
        let child = registry
            .reserve_schedule_child_job_id(&parent.job_id, "report.txt")
            .unwrap();
        let destination = LocalFilesystem::open(&right_root).unwrap();
        let identity = destination.root_identity().unwrap();
        let root_identity = format!("unix:{}:{}", identity.device, identity.inode);
        drop(registry);
        let mut store = JobStore::open(&paths.jobs).unwrap();
        store.enqueue(&child, Some((&scenario, 1))).unwrap();
        store
            .advance(
                &child,
                JobState::Queued,
                JobState::Running,
                Confirmation::Confirmed(EvidenceLevel::Requested),
            )
            .unwrap();
        store
            .prepare_bound_attempt_with_root_identity(
                &child,
                "scheduled-copy-attempt",
                "local_copy_new",
                &right,
                "report.txt",
                7,
                "00000000000000000000000000000000",
                &root_identity,
            )
            .unwrap();
        drop(store);

        let registry = Registry::open(&paths.registry).unwrap();
        reconcile_unknown_scheduled_copies(&paths, &registry).unwrap();
        assert_eq!(
            JobStore::open(&paths.jobs).unwrap().state(&child).unwrap(),
            Some(JobState::Completed)
        );
        assert_eq!(fs::read(right_root.join("report.txt")).unwrap(), b"payload");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn private_socket_reports_status_and_stops_the_worker() {
        let root = root("socket");
        fs::create_dir_all(&root).unwrap();
        let paths = WorkerPaths {
            socket: root.join("worker.sock"),
            registry: root.join("registry.sqlite3"),
            jobs: root.join("jobs.sqlite3"),
        };
        let socket = paths.socket.clone();
        let handle = thread::spawn(move || run(paths, false));
        for _ in 0..100 {
            if socket.exists() {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(socket.exists());

        let mut status = UnixStream::connect(&socket).unwrap();
        writeln!(status, r#"{{"command":"status"}}"#).unwrap();
        let mut response = String::new();
        BufReader::new(status).read_line(&mut response).unwrap();
        assert!(response.contains(r#""status":"running"#));

        let mut stop = UnixStream::connect(&socket).unwrap();
        writeln!(stop, r#"{{"command":"stop"}}"#).unwrap();
        response.clear();
        BufReader::new(stop).read_line(&mut response).unwrap();
        assert!(response.contains(r#""status":"stopping"#));
        assert!(handle.join().unwrap().is_ok());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_non_socket_at_the_socket_path_is_not_deleted() {
        let root = root("unsafe-socket");
        fs::create_dir_all(&root).unwrap();
        let socket = root.join("worker.sock");
        fs::write(&socket, b"not a socket").unwrap();
        assert_eq!(
            remove_stale_socket(&socket).unwrap_err(),
            "worker_socket_path_unsafe"
        );
        assert_eq!(fs::read(&socket).unwrap(), b"not a socket");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn private_socket_executes_pinned_scenario_create_and_confirmed_replace() {
        let root = root("scenario-create");
        let source_root = root.join("source");
        let destination_root = root.join("destination");
        let socket_directory = std::env::temp_dir().join(format!(
            "qw-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&socket_directory).unwrap();
        fs::set_permissions(&socket_directory, fs::Permissions::from_mode(0o700)).unwrap();
        let socket = socket_directory.join("worker.sock");
        fs::create_dir_all(&source_root).unwrap();
        fs::create_dir_all(&destination_root).unwrap();
        fs::write(source_root.join("report.txt"), b"worker content").unwrap();
        let paths = WorkerPaths {
            socket,
            registry: root.join("registry.sqlite3"),
            jobs: root.join("jobs.sqlite3"),
        };
        let registry = Registry::open(&paths.registry).unwrap();
        let left = registry.add("local_filesystem", "Left").unwrap();
        let right = registry.add("local_filesystem", "Right").unwrap();
        let scenario = registry
            .create_scenario(
                "Documents",
                &left,
                source_root.to_str().unwrap(),
                &right,
                destination_root.to_str().unwrap(),
                "left_to_right",
            )
            .unwrap();
        drop(registry);
        let socket = paths.socket.clone();
        let handle = thread::spawn(move || run(paths, false));
        for _ in 0..100 {
            if socket.exists() {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        let request = serde_json::json!({
            "command":"execute_create",
            "scenario_id":scenario,
            "scenario_revision":1,
            "source_root":source_root,
            "source_connection_id":left,
            "source_relative_path":"report.txt",
            "destination_root":destination_root,
            "destination_connection_id":right,
            "destination_relative_path":"report.txt",
            "job_id":"worker-scenario-create"
        });
        let mut stream = UnixStream::connect(&socket).unwrap();
        writeln!(stream, "{request}").unwrap();
        let mut response = String::new();
        BufReader::new(stream).read_line(&mut response).unwrap();
        assert!(
            response.contains(r#""status":"completed"#),
            "unexpected Worker response: {response}"
        );
        assert_eq!(
            fs::read(destination_root.join("report.txt")).unwrap(),
            b"worker content"
        );
        fs::write(source_root.join("report.txt"), b"replacement content").unwrap();
        let declined_replace = serde_json::json!({
            "command":"execute_replace",
            "scenario_id":scenario,
            "scenario_revision":1,
            "source_root":source_root,
            "source_connection_id":left,
            "source_relative_path":"report.txt",
            "destination_root":destination_root,
            "destination_connection_id":right,
            "destination_relative_path":"report.txt",
            "job_id":"worker-scenario-replace-declined",
            "replacement_consent":false
        });
        let mut stream = UnixStream::connect(&socket).unwrap();
        writeln!(stream, "{declined_replace}").unwrap();
        response.clear();
        BufReader::new(stream).read_line(&mut response).unwrap();
        assert!(
            response.contains(r#""status":"consent_required""#),
            "unexpected declined replacement response: {response}"
        );
        assert_eq!(
            fs::read(destination_root.join("report.txt")).unwrap(),
            b"worker content"
        );
        let confirmed_replace = serde_json::json!({
            "command":"execute_replace",
            "scenario_id":scenario,
            "scenario_revision":1,
            "source_root":source_root,
            "source_connection_id":left,
            "source_relative_path":"report.txt",
            "destination_root":destination_root,
            "destination_connection_id":right,
            "destination_relative_path":"report.txt",
            "job_id":"worker-scenario-replace-confirmed",
            "replacement_consent":true
        });
        let mut stream = UnixStream::connect(&socket).unwrap();
        writeln!(stream, "{confirmed_replace}").unwrap();
        response.clear();
        BufReader::new(stream).read_line(&mut response).unwrap();
        assert!(
            response.contains(r#""status":"completed""#),
            "unexpected confirmed replacement response: {response}"
        );
        assert_eq!(
            fs::read(destination_root.join("report.txt")).unwrap(),
            b"replacement content"
        );
        assert_eq!(
            JobStore::open(&root.join("jobs.sqlite3"))
                .unwrap()
                .list_jobs()
                .unwrap()
                .len(),
            2
        );
        let mut stop = UnixStream::connect(&socket).unwrap();
        writeln!(stop, r#"{{"command":"stop"}}"#).unwrap();
        assert!(handle.join().unwrap().is_ok());
        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(socket_directory).unwrap();
    }

    #[test]
    fn conflict_delete_requires_consent_and_uses_the_selected_snapshot() {
        let root = root("conflict-delete");
        let source_root = root.join("source");
        let destination_root = root.join("destination");
        fs::create_dir(&source_root).unwrap();
        fs::create_dir(&destination_root).unwrap();
        fs::write(destination_root.join("report.txt"), b"remove me").unwrap();
        let paths = WorkerPaths {
            socket: root.join("worker.sock"),
            registry: root.join("registry.sqlite3"),
            jobs: root.join("jobs.sqlite3"),
        };
        let registry = Registry::open(&paths.registry).unwrap();
        let left = registry.add("local_filesystem", "Left").unwrap();
        let right = registry.add("local_filesystem", "Right").unwrap();
        let scenario = registry
            .create_scenario(
                "Conflict",
                &left,
                source_root.to_str().unwrap(),
                &right,
                destination_root.to_str().unwrap(),
                "left_to_right",
            )
            .unwrap();
        let destination = LocalFilesystem::open(&destination_root)
            .unwrap()
            .stat(Path::new("report.txt"))
            .unwrap();
        let conflict = registry
            .queue_conflict_task(
                &ScenarioReference {
                    scenario_id: scenario.clone(),
                    revision: 1,
                },
                "report.txt",
                "deletion",
                None,
                None,
                Some(&ConfirmedFileVersion {
                    object_revision: destination.object_revision,
                    size: destination.size,
                    checksum_md5: None,
                }),
            )
            .unwrap();
        registry
            .select_conflict_task(&conflict, Some(true), "delete_as_source")
            .unwrap();
        let declined = Request {
            command: "execute_conflict_delete".into(),
            scenario_id: None,
            scenario_revision: None,
            source_root: None,
            source_connection_id: None,
            source_relative_path: None,
            destination_root: None,
            destination_connection_id: None,
            destination_relative_path: None,
            job_id: Some("worker-conflict-delete-declined".into()),
            replacement_consent: None,
            conflict_id: Some(conflict.clone()),
            destructive_consent: Some(false),
            locally_paused: None,
            source_is_left: None,
            decision: None,
        };
        assert_eq!(
            execute_conflict(&paths, &declined, true),
            Err("consent_required")
        );
        assert!(destination_root.join("report.txt").exists());
        let confirmed = Request {
            job_id: Some("worker-conflict-delete-confirmed".into()),
            destructive_consent: Some(true),
            ..declined
        };
        assert_eq!(execute_conflict(&paths, &confirmed, true), Ok("completed"));
        assert!(!destination_root.join("report.txt").exists());
        assert_eq!(
            JobStore::open(&paths.jobs)
                .unwrap()
                .state("worker-conflict-delete-confirmed")
                .unwrap(),
            Some(JobState::Completed)
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn registry_control_updates_pause_and_selected_conflict_decision() {
        let root = root("registry-control");
        let paths = WorkerPaths {
            socket: root.join("worker.sock"),
            registry: root.join("registry.sqlite3"),
            jobs: root.join("jobs.sqlite3"),
        };
        let registry = Registry::open(&paths.registry).unwrap();
        let left = registry.add("local_filesystem", "Left").unwrap();
        let right = registry.add("local_filesystem", "Right").unwrap();
        let scenario = registry
            .create_scenario("Control", &left, "/left", &right, "/right", "left_to_right")
            .unwrap();
        let pause = Request {
            command: "set_scenario_paused".into(),
            scenario_id: Some(scenario.clone()),
            locally_paused: Some(true),
            scenario_revision: None,
            source_root: None,
            source_connection_id: None,
            source_relative_path: None,
            destination_root: None,
            destination_connection_id: None,
            destination_relative_path: None,
            job_id: None,
            replacement_consent: None,
            conflict_id: None,
            destructive_consent: None,
            source_is_left: None,
            decision: None,
        };
        assert_eq!(update_registry_control(&paths, &pause), Ok("updated"));
        assert!(
            registry
                .scenario_automation_snapshot(&scenario)
                .unwrap()
                .locally_paused
        );
        let conflict = registry
            .queue_conflict_task(
                &ScenarioReference {
                    scenario_id: scenario.clone(),
                    revision: 1,
                },
                "report.txt",
                "deletion",
                None,
                None,
                Some(&ConfirmedFileVersion {
                    object_revision: None,
                    size: 1,
                    checksum_md5: None,
                }),
            )
            .unwrap();
        let decision = Request {
            command: "select_conflict_decision".into(),
            conflict_id: Some(conflict.clone()),
            source_is_left: Some(true),
            decision: Some("keep_file".into()),
            ..pause
        };
        assert_eq!(update_registry_control(&paths, &decision), Ok("updated"));
        assert_eq!(
            registry
                .list_conflict_tasks(&ScenarioReference {
                    scenario_id: scenario,
                    revision: 1
                })
                .unwrap()[0]
                .state,
            "completed"
        );
        fs::remove_dir_all(root).unwrap();
    }
}
