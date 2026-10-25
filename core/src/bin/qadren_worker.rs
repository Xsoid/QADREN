//! Release 1 local Worker process.
//!
//! The Worker owns periodic schedule staging and JobStore dispatch. Its
//! private Unix socket is intentionally local-only. It does not expose a
//! network or remote-Worker protocol.

use qadren_core::confirmation::{Confirmation, EvidenceLevel};
use qadren_core::job_execution::{
    local_directory_reconciliation_candidates, local_file_copy_reconciliation_candidates,
    reconcile_local_directory_create, reconcile_local_file_copy, reconcile_local_rule_delete,
    remove_interrupted_staging, require_current_scenario_revision, run_local_directory_create,
    run_local_file_copy, run_local_file_replace, run_local_scenario_conflict_delete,
    run_local_scenario_conflict_replace, run_local_scenario_file_copy,
    run_local_scenario_file_replace_planned, run_local_scenario_rule_delete, LocalFileCopyError,
    LocalFileCopyReconciliationStatus, LocalFileCopyStatus, LocalRuleDeleteStatus,
};
use qadren_core::jobs::JobStore;
use qadren_core::local_fs::LocalFilesystem;
use qadren_core::model::{ConfirmedFileVersion, JobState, ScenarioReference};
use qadren_core::plan_store;
use qadren_core::registry::{BatchChildDue, DueBatchChild, Registry};
use qadren_core::scenario_planner::{ObservedFile, PlanAction};
use qadren_core::schedule_dispatch::dispatch_pending_schedule_jobs;
use qadren_core::snapshot::{
    walk_local_step, walk_yandex_step, SnapshotError, SnapshotKey, SnapshotStore,
};
use qadren_core::storage_access::YANDEX_DISK;
use qadren_core::yandex::{YandexError, YandexMetadataClient};
use qadren_core::yandex_execution::run_yandex_scenario_file_download;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::{FileTypeExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, SystemTime};

const POLL_INTERVAL: Duration = Duration::from_secs(60);
const POLL_OVERLAP: Duration = Duration::from_secs(90);
const MAX_SCHEDULE_SCAN_ENTRIES: usize = 10_000;

#[derive(Debug, Clone)]
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

/// File metadata that the user saw in a plan. Size and the endpoint-local
/// object revision must still match before a replacement starts.
#[derive(Debug, Deserialize)]
struct ExpectedFileVersion {
    object_revision: Option<String>,
    size: u64,
}

impl ExpectedFileVersion {
    fn confirmed(&self) -> ConfirmedFileVersion {
        ConfirmedFileVersion {
            object_revision: self.object_revision.clone(),
            size: self.size,
            checksum_md5: None,
        }
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
    /// Plan metadata for a Scenario Update. Required by `execute_replace`.
    #[serde(default)]
    expected_source: Option<ExpectedFileVersion>,
    #[serde(default)]
    expected_destination: Option<ExpectedFileVersion>,
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
    #[serde(default)]
    batch_id: Option<String>,
    /// Yandex Disk OAuth token from the app Keychain for a Yandex Disk to Mac
    /// Scenario (D-023). It lives in memory for this request only.
    #[serde(default)]
    access_token: Option<AccessToken>,
    /// The connection of `provide_access_token`.
    #[serde(default)]
    connection_id: Option<String>,
    /// Plan requests (D-024 stage 4).
    #[serde(default)]
    plan_id: Option<String>,
    #[serde(default)]
    action: Option<String>,
    #[serde(default)]
    search: Option<String>,
    #[serde(default)]
    offset: Option<u64>,
    #[serde(default)]
    limit: Option<u64>,
}

/// A secret that never appears in `Debug` output or in a log.
#[derive(Deserialize)]
#[serde(transparent)]
struct AccessToken(String);

impl std::fmt::Debug for AccessToken {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("AccessToken(<redacted>)")
    }
}

/// True when the source of this Scenario revision is a Yandex Disk
/// connection (D-023). Such a Scenario needs an access token from the app.
fn yandex_source(registry: &Registry, scenario_id: &str, revision: u64) -> bool {
    yandex_source_connection(registry, scenario_id, revision).is_some()
}

/// The Yandex Disk source connection of a one-way Scenario revision.
fn yandex_source_connection(
    registry: &Registry,
    scenario_id: &str,
    revision: u64,
) -> Option<String> {
    let scenario = registry
        .get_scenario_revision(scenario_id, revision)
        .ok()??;
    let source = match scenario.direction.as_str() {
        "left_to_right" => scenario.left_connection_id,
        "right_to_left" => scenario.right_connection_id,
        _ => return None,
    };
    matches!(registry.get(&source), Ok(Some(connection)) if connection.adapter == YANDEX_DISK)
        .then_some(source)
}

/// Run one Yandex Disk to Mac download. `None` replacement is a Create.
fn run_yandex_download(
    paths: &WorkerPaths,
    scenario_id: &str,
    revision: u64,
    relative_path: &str,
    job_id: &str,
    token: Option<&str>,
    replacement: Option<(&ConfirmedFileVersion, &ConfirmedFileVersion, bool)>,
) -> Result<LocalFileCopyStatus, LocalFileCopyError> {
    let Some(token) = token else {
        return Err(LocalFileCopyError::Remote(YandexError::InvalidToken));
    };
    let client = YandexMetadataClient::new().map_err(LocalFileCopyError::Remote)?;
    run_yandex_scenario_file_download(
        &paths.jobs,
        &paths.registry,
        scenario_id,
        revision,
        relative_path,
        job_id,
        token,
        &client,
        replacement,
    )
}

/// Stable codes for a Yandex Disk failure. They never contain a path or a
/// token.
fn yandex_error_code(error: &YandexError) -> &'static str {
    match error {
        YandexError::InvalidToken => "yandex_token_required",
        YandexError::Unauthorized => "yandex_unauthorized",
        YandexError::Forbidden => "yandex_forbidden",
        YandexError::RateLimited => "yandex_rate_limited",
        YandexError::NotFound => "yandex_not_found",
        YandexError::ContentMismatch => "yandex_content_mismatch",
        YandexError::ContentUnavailable(_) => "yandex_content_unavailable",
        YandexError::Transport(_) => "yandex_network_error",
        _ => "yandex_provider_error",
    }
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
    let overlap = chrono::Duration::from_std(POLL_OVERLAP)
        .map_err(|_| "worker_clock_unavailable".to_string())?;
    let registry =
        Registry::open(&paths.registry).map_err(|_| "registry_unavailable".to_string())?;
    reconcile_unknown_scenario_creates(paths, &registry)?;
    // D-024: the app cannot open the JobStore while the Worker owns it, so
    // the Worker delivers pending copy baselines to the Registry itself.
    if let Ok(mut jobs) = JobStore::open(&paths.jobs) {
        let _ = qadren_core::scenario_copy_dispatch::dispatch_pending_scenario_copy_baselines(
            &registry, &mut jobs,
        );
    }
    reconcile_unknown_rule_deletes(paths);
    let staged = registry
        .stage_schedule_catch_up(through, overlap)
        .map_err(|_| "schedule_stage_unavailable".to_string())?;
    let mut jobs = JobStore::open(&paths.jobs).map_err(|_| "job_store_unavailable".to_string())?;
    let dispatched = dispatch_pending_schedule_jobs(&registry, &mut jobs)
        .map_err(|_| "schedule_dispatch_unavailable".to_string())?;
    drop(jobs);
    execute_queued_schedule_jobs(paths, &registry)?;
    Ok((staged, dispatched))
}

/// Reconcile UNKNOWN deletions made under a simple deletion rule (D-019).
/// This is read-only: an object that is still at its path keeps the Job
/// UNKNOWN, and the deletion is never repeated.
fn reconcile_unknown_rule_deletes(paths: &WorkerPaths) {
    let Ok(attempts) = JobStore::open(&paths.jobs).and_then(|store| store.unresolved_attempts())
    else {
        return;
    };
    for attempt in attempts.into_iter().filter(|attempt| {
        attempt.status == "unknown"
            && matches!(
                attempt.operation_kind.as_deref(),
                Some("local_rule_trash" | "local_rule_delete")
            )
    }) {
        // One unresolvable Job must not stop the other checks.
        let _ = reconcile_local_rule_delete(&paths.jobs, &paths.registry, &attempt.job_id);
    }
}

/// Run every deletion that the plan of `reference` marks as covered by the
/// simple deletion rule. The plan applies the mass-deletion threshold, so a
/// caller cannot exceed it by asking for single paths. Child Job identities
/// are stable per path, so a repeated request does not repeat an effect.
fn run_rule_deletions(
    paths: &WorkerPaths,
    reference: &ScenarioReference,
    plan: &[qadren_core::scenario_planner::PlanItem],
    child_job_id: &dyn Fn(&str) -> Result<String, String>,
) -> Result<usize, String> {
    let mut done = 0;
    for item in plan {
        let PlanAction::Delete { on_left } = item.action else {
            continue;
        };
        let job_id = child_job_id(&item.relative_path)?;
        match run_local_scenario_rule_delete(
            &paths.jobs,
            &paths.registry,
            &reference.scenario_id,
            reference.revision,
            &item.relative_path,
            on_left,
            &job_id,
        ) {
            Ok(LocalRuleDeleteStatus::MovedToTrash | LocalRuleDeleteStatus::DeletedPermanently)
            | Ok(LocalRuleDeleteStatus::Existing(JobState::Completed)) => done += 1,
            Ok(LocalRuleDeleteStatus::Existing(_)) => {
                return Err("rule_delete_not_completed".to_string())
            }
            Err(LocalFileCopyError::OutcomeUnknown(_)) => return Err("outcome_unknown".to_string()),
            Err(LocalFileCopyError::ConflictStale) => return Err("plan_stale".to_string()),
            Err(_) => return Err("rule_delete_failed".to_string()),
        }
    }
    Ok(done)
}

/// IPC: run the rule deletions of the current revision of one Scenario.
fn execute_rule_deletions(
    paths: &WorkerPaths,
    request: &Request,
) -> Result<(&'static str, usize), &'static str> {
    let (Some(scenario_id), Some(revision), Some(batch_id)) = (
        request.scenario_id.as_deref(),
        request.scenario_revision,
        request.batch_id.as_deref(),
    ) else {
        return Err("invalid_request");
    };
    if let Err(error) = require_current_scenario_revision(&paths.registry, scenario_id, revision) {
        return Err(match error {
            LocalFileCopyError::ScenarioRevisionStale => "scenario_revision_stale",
            _ => "scenario_unavailable",
        });
    }
    let registry = Registry::open(&paths.registry).map_err(|_| "registry_unavailable")?;
    let scenario = registry
        .get_scenario_revision(scenario_id, revision)
        .ok()
        .flatten()
        .ok_or("scenario_unavailable")?;
    // D-023 stage 1 copies from Yandex Disk only. A deletion on the Mac needs
    // a provider check that the file is gone, which is not built yet.
    if yandex_source(&registry, scenario_id, revision) {
        return Err("yandex_deletions_unsupported");
    }
    enabled_local_connection(&registry, &paths.registry, &scenario.left_connection_id)
        .and_then(|()| {
            enabled_local_connection(&registry, &paths.registry, &scenario.right_connection_id)
        })
        .map_err(|_| "local_connection_unavailable")?;
    let (left, skipped_left) =
        observed_files(Path::new(&scenario.left_root)).map_err(|_| "scan_unavailable")?;
    let (right, skipped_right) =
        observed_files(Path::new(&scenario.right_root)).map_err(|_| "scan_unavailable")?;
    let reference = ScenarioReference {
        scenario_id: scenario_id.to_owned(),
        revision,
    };
    let plan = registry
        .plan_scenario_dry_run_with_skipped(
            &reference,
            &left,
            &right,
            &skipped_left,
            &skipped_right,
        )
        .map_err(|_| "plan_unavailable")?;
    let child = |path: &str| Ok(format!("{batch_id}-{:x}", md5::compute(path.as_bytes())));
    match run_rule_deletions(paths, &reference, &plan, &child) {
        Ok(done) => Ok(("completed", done)),
        Err(error) => Err(match error.as_str() {
            "outcome_unknown" => "outcome_unknown",
            "plan_stale" => "plan_stale",
            _ => "rule_delete_failed",
        }),
    }
}

/// Reconcile UNKNOWN Scenario Create and replacement Jobs with an exact
/// persisted Scenario destination. This reads metadata and never retries an
/// effect. Deletion stays UNKNOWN because missing metadata cannot prove that
/// this Worker performed the deletion.
fn reconcile_unknown_scenario_creates(
    paths: &WorkerPaths,
    registry: &Registry,
) -> Result<(), String> {
    let candidates = {
        let store = JobStore::open(&paths.jobs).map_err(|_| "job_store_unavailable".to_string())?;
        let unknown = store
            .jobs_in_state(JobState::Unknown)
            .map_err(|_| "job_store_unavailable".to_string())?
            .into_iter()
            .filter_map(|job| {
                job.scenario_id
                    .zip(job.scenario_revision)
                    .map(|(scenario_id, revision)| (job.id, scenario_id, revision))
            })
            .collect::<Vec<_>>();
        let attempts = store
            .unresolved_attempts()
            .map_err(|_| "job_store_unavailable".to_string())?;
        unknown
            .into_iter()
            .filter_map(|(job_id, scenario_id, revision)| {
                attempts
                    .iter()
                    .find(|attempt| {
                        attempt.job_id == job_id
                            && attempt.status == "unknown"
                            && matches!(
                                attempt.operation_kind.as_deref(),
                                Some("local_copy_new") | Some("local_copy_replace")
                            )
                    })
                    .and_then(|attempt| attempt.target_connection_id.as_deref())
                    .map(|connection_id| (job_id, scenario_id, revision, connection_id.to_owned()))
            })
            .collect::<Vec<_>>()
    };
    for (job_id, scenario_id, revision, destination_connection_id) in candidates {
        // One unresolvable Job must not stop schedules or other checks. It
        // stays UNKNOWN and is checked again on the next poll.
        let Ok(Some(scenario)) = registry.get_scenario_revision(&scenario_id, revision) else {
            continue;
        };
        // Both endpoints can use the same Local connection ("This Mac"). Try
        // each root of that connection. Reconciliation is read-only and checks
        // the durable root identity, so a wrong root stays StillUnknown.
        let roots = [
            (&scenario.left_connection_id, &scenario.left_root),
            (&scenario.right_connection_id, &scenario.right_root),
        ];
        for (_, destination_root) in roots
            .into_iter()
            .filter(|(connection, _)| **connection == destination_connection_id)
        {
            // Remove junk staging of the interrupted write first. This never
            // touches the target file and never changes the Job state.
            let _ = remove_interrupted_staging(
                &paths.jobs,
                Path::new(destination_root),
                &destination_connection_id,
                &job_id,
            );
            if matches!(
                reconcile_local_file_copy(
                    &paths.jobs,
                    &paths.registry,
                    Path::new(destination_root),
                    &destination_connection_id,
                    &job_id,
                ),
                Ok(LocalFileCopyReconciliationStatus::Completed
                    | LocalFileCopyReconciliationStatus::Existing(_))
            ) {
                break;
            }
        }
    }
    Ok(())
}

/// Metadata of synchronizable files and the paths of skipped symbolic links
/// and special files below one Scenario root.
#[allow(clippy::type_complexity)]
fn observed_files(root: &Path) -> Result<(Vec<ObservedFile>, Vec<String>), String> {
    let filesystem =
        LocalFilesystem::open(root).map_err(|_| "schedule_root_unavailable".to_string())?;
    filesystem
        .list_tree_with_skipped(Path::new(""), MAX_SCHEDULE_SCAN_ENTRIES)
        .map_err(|_| "schedule_scan_unavailable".to_string())
        .map(|(entries, skipped)| {
            (
                entries
                    .into_iter()
                    .filter(|(_, entry)| !entry.is_directory)
                    .map(|(path, entry)| ObservedFile {
                        relative_path: path.to_string_lossy().into_owned(),
                        object_revision: entry.object_revision,
                        size: entry.size,
                        checksum_md5: None,
                    })
                    .collect(),
                skipped
                    .into_iter()
                    .map(|path| path.to_string_lossy().into_owned())
                    .collect(),
            )
        })
}

/// "This Mac", or an SMB share whose QADREN mount is present now (D-022).
/// The Worker does not mount a share itself; an absent mount refuses the Job.
fn enabled_local_connection(
    registry: &Registry,
    registry_path: &Path,
    connection_id: &str,
) -> Result<(), String> {
    let enabled = registry
        .list()
        .map_err(|_| "schedule_registry_unavailable".to_string())?
        .into_iter()
        .any(|connection| {
            connection.id == connection_id
                && qadren_core::storage_access::connection_ready(&connection, registry_path)
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
        .jobs_in_state(JobState::Queued)
        .map_err(|_| "job_store_unavailable".to_string())?
        .into_iter();
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
    enabled_local_connection(registry, &paths.registry, &scenario.left_connection_id)?;
    enabled_local_connection(registry, &paths.registry, &scenario.right_connection_id)?;
    let (left, skipped_left) = observed_files(Path::new(&scenario.left_root))?;
    let (right, skipped_right) = observed_files(Path::new(&scenario.right_root))?;
    let plan = registry
        .plan_scenario_dry_run_with_skipped(reference, &left, &right, &skipped_left, &skipped_right)
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
                            let conflict_id = registry
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
                            registry
                                .link_schedule_conflict(schedule_job_id, &conflict_id)
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
                let conflict_id = registry
                    .queue_conflict_task(
                        reference,
                        &item.relative_path,
                        "content",
                        Some(from_left),
                        Some(&left_version),
                        Some(&right_version),
                    )
                    .map_err(|_| "schedule_conflict_unavailable".to_string())?;
                registry
                    .link_schedule_conflict(schedule_job_id, &conflict_id)
                    .map_err(|_| "schedule_conflict_unavailable".to_string())?;
                waiting_for_decision = true;
            }
            PlanAction::Conflict { .. } => {
                let left_version = observed_version(&left, &item.relative_path)
                    .ok_or_else(|| "schedule_conflict_unavailable".to_string())?;
                let right_version = observed_version(&right, &item.relative_path)
                    .ok_or_else(|| "schedule_conflict_unavailable".to_string())?;
                let conflict_id = registry
                    .queue_conflict_task(
                        reference,
                        &item.relative_path,
                        "content",
                        None,
                        Some(&left_version),
                        Some(&right_version),
                    )
                    .map_err(|_| "schedule_conflict_unavailable".to_string())?;
                registry
                    .link_schedule_conflict(schedule_job_id, &conflict_id)
                    .map_err(|_| "schedule_conflict_unavailable".to_string())?;
                waiting_for_decision = true;
            }
            PlanAction::DeleteRequiresDecision { .. } => {
                let left_version = observed_version(&left, &item.relative_path);
                let right_version = observed_version(&right, &item.relative_path);
                let conflict_id = registry
                    .queue_conflict_task(
                        reference,
                        &item.relative_path,
                        "deletion",
                        None,
                        left_version.as_ref(),
                        right_version.as_ref(),
                    )
                    .map_err(|_| "schedule_conflict_unavailable".to_string())?;
                registry
                    .link_schedule_conflict(schedule_job_id, &conflict_id)
                    .map_err(|_| "schedule_conflict_unavailable".to_string())?;
                waiting_for_decision = true;
            }
            PlanAction::Delete { .. } => {
                let child = |path: &str| {
                    registry
                        .reserve_schedule_child_job_id(schedule_job_id, path)
                        .map_err(|_| "schedule_child_identity_unavailable".to_string())
                };
                match run_rule_deletions(paths, reference, std::slice::from_ref(&item), &child) {
                    Ok(_) => {}
                    Err(error) if error == "outcome_unknown" => {
                        return Err("schedule_outcome_unknown".to_string())
                    }
                    Err(_) => return Err("schedule_rule_delete_unavailable".to_string()),
                }
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
    if let Err(error) =
        require_current_scenario_revision(&paths.registry, scenario_id, scenario_revision)
    {
        return Err(match error {
            LocalFileCopyError::ScenarioRevisionStale => "scenario_revision_stale",
            _ => "scenario_unavailable",
        });
    }
    let registry = Registry::open(&paths.registry).map_err(|_| "registry_unavailable")?;
    let result = if yandex_source(&registry, scenario_id, scenario_revision) {
        if source_relative_path != destination_relative_path {
            return Err("invalid_request");
        }
        run_yandex_download(
            paths,
            scenario_id,
            scenario_revision,
            source_relative_path,
            job_id,
            request.access_token.as_ref().map(|token| token.0.as_str()),
            None,
        )
    } else {
        run_local_scenario_file_copy(
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
        )
    };
    match result {
        Err(LocalFileCopyError::Remote(error)) => Err(yandex_error_code(&error)),
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

/// The confirmed Create batches run on the background run thread (D-024).
/// This request only stores the Yandex Disk token of the batch source in
/// memory and answers at once.
fn execute_create_batch(
    paths: &WorkerPaths,
    request: &Request,
) -> Result<&'static str, &'static str> {
    let batch_id = request.batch_id.as_deref().ok_or("invalid_request")?;
    let registry = Registry::open(&paths.registry).map_err(|_| "registry_unavailable")?;
    let receipt = registry
        .scenario_create_batch(batch_id)
        .map_err(|_| "batch_unavailable")?
        .ok_or("batch_unavailable")?;
    if let (Some(connection), Some(AccessToken(token))) = (
        yandex_source_connection(
            &registry,
            &receipt.scenario.scenario_id,
            receipt.scenario.revision,
        ),
        request.access_token.as_ref(),
    ) {
        runner().remember_token(&connection, token);
    }
    Ok("accepted")
}

/// Pauses before the automatic retries of a child (D-024): 1 min, 5 min,
/// 30 min, 2 h, and 12 h.
const RETRY_DELAYS: [i64; 5] = [60, 300, 1_800, 7_200, 43_200];
/// Pause between the retries after the schedule when the "retry until
/// success" setting is on.
const RETRY_FOREVER_DELAY: i64 = 86_400;
/// Parallel metadata requests for each Yandex Disk snapshot (D-024).
const YANDEX_PARALLEL_METADATA: usize = 4;

/// The two snapshot keys of a Scenario revision: left and right side, each
/// with the Scenario selection (D-024 stage 3).
fn snapshot_keys(
    registry: &Registry,
    scenario_id: &str,
    revision: u64,
) -> Option<(SnapshotKey, SnapshotKey)> {
    let scenario = registry
        .get_scenario_revision(scenario_id, revision)
        .ok()??;
    let selection = registry
        .get_scenario_revision_scope(scenario_id, revision)
        .ok()?
        .selected_paths;
    let key = |connection_id: &str, root: &str| {
        let adapter = registry.get(connection_id).ok()??.adapter;
        Some(SnapshotKey {
            connection_id: connection_id.to_owned(),
            kind: if adapter == YANDEX_DISK {
                YANDEX_DISK.to_owned()
            } else {
                "local".to_owned()
            },
            root: root.to_owned(),
            selection: selection.clone(),
        })
    };
    Some((
        key(&scenario.left_connection_id, &scenario.left_root)?,
        key(&scenario.right_connection_id, &scenario.right_root)?,
    ))
}

/// The snapshot thread: continue every walking snapshot until the Worker
/// exits (D-024 stage 3).
fn run_snapshots_forever(paths: WorkerPaths) {
    loop {
        let worked = run_snapshot_pass(&paths);
        thread::sleep(if worked {
            Duration::from_millis(5)
        } else {
            Duration::from_secs(1)
        });
    }
}

/// One step of each walking snapshot. Returns true when a page was written.
fn run_snapshot_pass(paths: &WorkerPaths) -> bool {
    let Ok(store) = SnapshotStore::open(&SnapshotStore::path_for(&paths.registry)) else {
        return false;
    };
    let Ok(walking) = store.walking(unix_now()) else {
        return false;
    };
    let client = YandexMetadataClient::new().ok();
    let mut worked = false;
    for snapshot in walking {
        let result = if snapshot.key.kind == YANDEX_DISK {
            // Without a token from the app the walk waits.
            let (Some(token), Some(client)) =
                (runner().token(&snapshot.key.connection_id), client.as_ref())
            else {
                continue;
            };
            walk_yandex_step(
                &store,
                client,
                &token,
                &snapshot.id,
                &snapshot.key.root,
                YANDEX_PARALLEL_METADATA,
            )
        } else {
            // An SMB root is walked only while its QADREN mount is present.
            let ready = Registry::open(&paths.registry)
                .ok()
                .and_then(|registry| registry.get(&snapshot.key.connection_id).ok().flatten())
                .is_some_and(|connection| {
                    qadren_core::storage_access::root_ready(
                        &connection,
                        &paths.registry,
                        Path::new(&snapshot.key.root),
                    )
                });
            if !ready {
                let _ = store.delay(&snapshot.id, "local_connection_unavailable");
                continue;
            }
            walk_local_step(&store, &snapshot.id, Path::new(&snapshot.key.root))
        };
        match result {
            Ok(outcome) => worked |= outcome.pages > 0,
            Err(SnapshotError::Transient(code)) => {
                eprintln!(
                    "qadren-worker: {} snapshot {} waits: {code}",
                    log_time(),
                    snapshot.id
                );
                let _ = store.delay(&snapshot.id, &code);
            }
            Err(SnapshotError::Blocked(code)) if code == "yandex_unauthorized" => {
                runner().forget_token(&snapshot.key.connection_id);
                let _ = store.delay(&snapshot.id, &code);
            }
            Err(SnapshotError::Blocked(code)) => {
                eprintln!(
                    "qadren-worker: {} snapshot {} failed: {code}",
                    log_time(),
                    snapshot.id
                );
                let _ = store.fail(&snapshot.id, &code);
            }
            Err(_) => {
                let _ = store.delay(&snapshot.id, "snapshot_store_unavailable");
            }
        }
    }
    worked |= build_waiting_plans(paths, &store);
    if !worked {
        let _ = store.prune();
    }
    worked
}

/// Build each waiting plan whose two snapshots are complete; fail it when a
/// snapshot failed (D-024 stage 4).
fn build_waiting_plans(paths: &WorkerPaths, store: &SnapshotStore) -> bool {
    let Ok(waiting) = plan_store::waiting_plans(store.connection()) else {
        return false;
    };
    let mut built = false;
    for (id, _, left, right) in waiting {
        let states = (
            store.info(&left).ok().flatten().map(|info| info.state),
            store.info(&right).ok().flatten().map(|info| info.state),
        );
        match states {
            (Some(l), Some(r)) if l == "complete" && r == "complete" => {
                let Ok(registry) = Registry::open(&paths.registry) else {
                    continue;
                };
                let started = SystemTime::now();
                match plan_store::build_plan(
                    &SnapshotStore::path_for(&paths.registry),
                    &registry,
                    &id,
                ) {
                    Ok(_) => eprintln!(
                        "qadren-worker: {} plan {id} built in {} s",
                        log_time(),
                        started.elapsed().map_or(0, |elapsed| elapsed.as_secs())
                    ),
                    Err(_) => {
                        eprintln!("qadren-worker: {} plan {id} failed", log_time());
                        let _ = plan_store::fail_plan(store.connection(), &id, "plan_build_failed");
                    }
                }
                built = true;
            }
            (Some(l), _) | (_, Some(l)) if l == "failed" => {
                let _ = plan_store::fail_plan(store.connection(), &id, "snapshot_failed");
            }
            (None, _) | (_, None) => {
                let _ = plan_store::fail_plan(store.connection(), &id, "snapshot_missing");
            }
            _ => {}
        }
    }
    built
}

/// Start the snapshots of a Scenario revision and a plan that waits for
/// them (D-024 stage 4).
fn start_plan(paths: &WorkerPaths, request: &Request) -> serde_json::Value {
    let started = start_snapshots(paths, request);
    let (Some(left), Some(right)) = (started["left"].as_str(), started["right"].as_str()) else {
        return started;
    };
    let (Some(scenario_id), Some(revision)) =
        (request.scenario_id.as_deref(), request.scenario_revision)
    else {
        return serde_json::json!({"ok": false, "status": "invalid_request"});
    };
    let Ok(store) = SnapshotStore::open(&SnapshotStore::path_for(&paths.registry)) else {
        return serde_json::json!({"ok": false, "status": "snapshot_store_unavailable"});
    };
    let reference = ScenarioReference {
        scenario_id: scenario_id.to_owned(),
        revision,
    };
    match plan_store::request_plan(store.connection(), &reference, left, right) {
        Ok(plan) => serde_json::json!({"ok": true, "status": "accepted", "plan": plan}),
        Err(_) => serde_json::json!({"ok": false, "status": "snapshot_store_unavailable"}),
    }
}

/// The newest plan of a Scenario revision with the progress of its
/// snapshots.
fn plan_status(paths: &WorkerPaths, request: &Request) -> serde_json::Value {
    let (Some(scenario_id), Some(revision)) =
        (request.scenario_id.as_deref(), request.scenario_revision)
    else {
        return serde_json::json!({"ok": false, "status": "invalid_request"});
    };
    let Ok(store) = SnapshotStore::open_for_reading(&SnapshotStore::path_for(&paths.registry))
    else {
        return serde_json::json!({"ok": false, "status": "snapshot_store_unavailable"});
    };
    let reference = ScenarioReference {
        scenario_id: scenario_id.to_owned(),
        revision,
    };
    let plan = plan_store::latest_plan(store.connection(), &reference)
        .ok()
        .flatten();
    let (left, right) = match &plan {
        Some(plan) => (
            store.info(&plan.left_snapshot).ok().flatten(),
            store.info(&plan.right_snapshot).ok().flatten(),
        ),
        None => (None, None),
    };
    serde_json::json!({"ok": true, "status": "running", "plan": plan, "left": left, "right": right})
}

fn plan_page(paths: &WorkerPaths, request: &Request) -> serde_json::Value {
    let Some(plan) = request.plan_id.as_deref() else {
        return serde_json::json!({"ok": false, "status": "invalid_request"});
    };
    let Ok(store) = SnapshotStore::open_for_reading(&SnapshotStore::path_for(&paths.registry))
    else {
        return serde_json::json!({"ok": false, "status": "snapshot_store_unavailable"});
    };
    match plan_store::plan_page(
        store.connection(),
        plan,
        request.action.as_deref(),
        request.search.as_deref(),
        request.offset.unwrap_or(0),
        request.limit.unwrap_or(200),
    ) {
        Ok(rows) => serde_json::json!({"ok": true, "status": "running", "items": rows}),
        Err(_) => serde_json::json!({"ok": false, "status": "snapshot_store_unavailable"}),
    }
}

/// Make one Create batch of all Create rows of a complete plan, page by
/// page, and let the run thread start it (D-024 stage 4).
fn execute_plan_creates(paths: &WorkerPaths, request: &Request) -> serde_json::Value {
    let Some(plan) = request.plan_id.as_deref() else {
        return serde_json::json!({"ok": false, "status": "invalid_request"});
    };
    let Ok(store) = SnapshotStore::open_for_reading(&SnapshotStore::path_for(&paths.registry))
    else {
        return serde_json::json!({"ok": false, "status": "snapshot_store_unavailable"});
    };
    let Ok(Some(info)) = plan_store::plan_info(store.connection(), plan) else {
        return serde_json::json!({"ok": false, "status": "plan_unavailable"});
    };
    if info.state != "complete" {
        return serde_json::json!({"ok": false, "status": "plan_unavailable"});
    }
    let reference: Option<ScenarioReference> = store
        .connection()
        .query_row(
            "SELECT scenario_id,revision FROM plans WHERE id=?1",
            [plan],
            |row| {
                Ok(ScenarioReference {
                    scenario_id: row.get(0)?,
                    revision: row.get::<_, i64>(1)? as u64,
                })
            },
        )
        .ok();
    let Some(reference) = reference else {
        return serde_json::json!({"ok": false, "status": "plan_unavailable"});
    };
    let Ok(mut registry) = Registry::open(&paths.registry) else {
        return serde_json::json!({"ok": false, "status": "registry_unavailable"});
    };
    if let (Some(connection), Some(AccessToken(token))) = (
        yandex_source_connection(&registry, &reference.scenario_id, reference.revision),
        request.access_token.as_ref(),
    ) {
        runner().remember_token(&connection, token);
    }
    let batch = match registry.begin_scenario_create_batch(&reference) {
        Ok(batch) => batch,
        Err(_) => return serde_json::json!({"ok": false, "status": "scenario_revision_stale"}),
    };
    let mut after = 0;
    let mut added = 0;
    loop {
        let Ok(page) = plan_store::plan_creates_page(store.connection(), plan, after, 5_000) else {
            return serde_json::json!({"ok": false, "status": "snapshot_store_unavailable"});
        };
        let Some(last) = page.last() else { break };
        after = last.0;
        let children: Vec<(String, bool)> = page
            .into_iter()
            .map(|(_, path, from_left)| (path, from_left))
            .collect();
        match registry.append_scenario_create_batch_children(&batch, &children) {
            Ok(count) => added += count,
            Err(_) => return serde_json::json!({"ok": false, "status": "registry_unavailable"}),
        }
    }
    let _ = registry.set_scenario_create_batch_paused(&batch, false);
    serde_json::json!({"ok": true, "status": "accepted", "batch": batch, "files": added})
}

/// Start or continue the snapshots of both sides of a Scenario revision.
fn start_snapshots(paths: &WorkerPaths, request: &Request) -> serde_json::Value {
    let (Some(scenario_id), Some(revision)) =
        (request.scenario_id.as_deref(), request.scenario_revision)
    else {
        return serde_json::json!({"ok": false, "status": "invalid_request"});
    };
    let Ok(registry) = Registry::open(&paths.registry) else {
        return serde_json::json!({"ok": false, "status": "registry_unavailable"});
    };
    let Some((left, right)) = snapshot_keys(&registry, scenario_id, revision) else {
        return serde_json::json!({"ok": false, "status": "scenario_unavailable"});
    };
    if let (Some(connection), Some(AccessToken(token))) = (
        yandex_source_connection(&registry, scenario_id, revision),
        request.access_token.as_ref(),
    ) {
        runner().remember_token(&connection, token);
    }
    let Ok(store) = SnapshotStore::open(&SnapshotStore::path_for(&paths.registry)) else {
        return serde_json::json!({"ok": false, "status": "snapshot_store_unavailable"});
    };
    match (store.start(&left), store.start(&right)) {
        (Ok(left), Ok(right)) => {
            serde_json::json!({"ok": true, "status": "accepted", "left": left, "right": right})
        }
        _ => serde_json::json!({"ok": false, "status": "invalid_request"}),
    }
}

/// The newest snapshot of each side of a Scenario revision.
fn snapshot_status(paths: &WorkerPaths, request: &Request) -> serde_json::Value {
    let (Some(scenario_id), Some(revision)) =
        (request.scenario_id.as_deref(), request.scenario_revision)
    else {
        return serde_json::json!({"ok": false, "status": "invalid_request"});
    };
    let Some((left, right)) = Registry::open(&paths.registry)
        .ok()
        .and_then(|registry| snapshot_keys(&registry, scenario_id, revision))
    else {
        return serde_json::json!({"ok": false, "status": "scenario_unavailable"});
    };
    let Ok(store) = SnapshotStore::open_for_reading(&SnapshotStore::path_for(&paths.registry))
    else {
        return serde_json::json!({"ok": false, "status": "snapshot_store_unavailable"});
    };
    serde_json::json!({
        "ok": true,
        "status": "running",
        "left": store.latest(&left).ok().flatten(),
        "right": store.latest(&right).ok().flatten(),
    })
}

/// Batch children that one pass of the run thread reads (D-024).
const DUE_PAGE: usize = 256;
/// Parallel downloads for each Yandex Disk connection (D-024).
const YANDEX_PARALLEL_DOWNLOADS: usize = 2;

/// State of the run thread that the IPC thread can read (D-024).
struct Runner {
    /// Yandex Disk access tokens by connection ID, from the app. Memory only.
    tokens: Mutex<HashMap<String, String>>,
    /// Batch children that run now.
    active: AtomicUsize,
}

impl Runner {
    fn remember_token(&self, connection_id: &str, token: &str) {
        if let Ok(mut tokens) = self.tokens.lock() {
            tokens.insert(connection_id.to_owned(), token.to_owned());
        }
    }

    fn token(&self, connection_id: &str) -> Option<String> {
        self.tokens.lock().ok()?.get(connection_id).cloned()
    }

    fn forget_token(&self, connection_id: &str) {
        if let Ok(mut tokens) = self.tokens.lock() {
            tokens.remove(connection_id);
        }
    }
}

fn runner() -> &'static Runner {
    static RUNNER: OnceLock<Runner> = OnceLock::new();
    RUNNER.get_or_init(|| Runner {
        tokens: Mutex::new(HashMap::new()),
        active: AtomicUsize::new(0),
    })
}

fn unix_now() -> i64 {
    i64::try_from(log_time()).unwrap_or(i64::MAX)
}

/// The run thread: start due batch children until the Worker exits.
fn run_batches_forever(paths: WorkerPaths) {
    loop {
        run_due_batch_children(&paths);
        thread::sleep(Duration::from_secs(1));
    }
}

/// One pass over the batch children that are due now. A Yandex Disk batch
/// without a token waits until the app sends one.
fn run_due_batch_children(paths: &WorkerPaths) {
    let Ok(registry) = Registry::open(&paths.registry) else {
        return;
    };
    // D-024 scale rules: one bounded page per pass, read through an index.
    let Ok(due) = registry.due_scenario_create_batch_children(unix_now(), DUE_PAGE) else {
        return;
    };
    let mut sources: HashMap<String, Option<String>> = HashMap::new();
    let mut local = Vec::new();
    let mut remote: HashMap<String, Vec<DueBatchChild>> = HashMap::new();
    for child in due {
        if child.cancellation_requested {
            let _ = registry.finish_scenario_create_batch_child(
                &child.batch_id,
                &child.relative_path,
                "cancelled",
                None,
            );
            continue;
        }
        let source = sources
            .entry(child.batch_id.clone())
            .or_insert_with(|| {
                yandex_source_connection(
                    &registry,
                    &child.scenario.scenario_id,
                    child.scenario.revision,
                )
            })
            .clone();
        match source {
            Some(connection) => remote.entry(connection).or_default().push(child),
            None => local.push(child),
        }
    }
    drop(registry);
    for child in &local {
        if still_due(paths, child) {
            run_batch_child(paths, child, None);
        }
    }
    for (connection, children) in remote {
        if runner().token(&connection).is_none() {
            continue;
        }
        let queue = Mutex::new(std::collections::VecDeque::from(children));
        thread::scope(|scope| {
            for _ in 0..YANDEX_PARALLEL_DOWNLOADS {
                scope.spawn(|| loop {
                    let Some(child) = queue.lock().ok().and_then(|mut queue| queue.pop_front())
                    else {
                        break;
                    };
                    // A 401 removes the token; the batch then waits.
                    let Some(token) = runner().token(&connection) else {
                        break;
                    };
                    if still_due(paths, &child) {
                        run_batch_child(paths, &child, Some((&connection, &token)));
                    }
                });
            }
        });
    }
}

/// The child is still pending with the same Job ID and its batch is not
/// paused. A pause or a stop takes effect before the next child. This reads
/// one row.
fn still_due(paths: &WorkerPaths, child: &DueBatchChild) -> bool {
    let Ok(registry) = Registry::open(&paths.registry) else {
        return false;
    };
    match registry.batch_child_due(&child.batch_id, &child.relative_path, &child.job_id) {
        Ok(BatchChildDue::Due) => true,
        Ok(BatchChildDue::Cancelled) => {
            let _ = registry.finish_scenario_create_batch_child(
                &child.batch_id,
                &child.relative_path,
                "cancelled",
                None,
            );
            false
        }
        _ => false,
    }
}

fn job_state(paths: &WorkerPaths, job_id: &str) -> Option<JobState> {
    JobStore::open(&paths.jobs)
        .ok()?
        .get(job_id)
        .ok()?
        .map(|job| job.state)
}

/// Run one batch child and record its outcome, or schedule its retry.
fn run_batch_child(paths: &WorkerPaths, child: &DueBatchChild, yandex: Option<(&str, &str)>) {
    runner().active.fetch_add(1, Ordering::SeqCst);
    let result = (|| {
        // A resumed child can already have a Job from before a restart.
        // Report that Job; never start a second attempt for the same Job ID.
        if let Some(state) = job_state(paths, &child.job_id) {
            return Ok(LocalFileCopyStatus::Existing(state));
        }
        if let Some((_, token)) = yandex {
            return run_yandex_download(
                paths,
                &child.scenario.scenario_id,
                child.scenario.revision,
                &child.relative_path,
                &child.job_id,
                Some(token),
                None,
            );
        }
        let registry =
            Registry::open(&paths.registry).map_err(|_| LocalFileCopyError::ScenarioUnavailable)?;
        let scenario = registry
            .get_scenario_revision(&child.scenario.scenario_id, child.scenario.revision)
            .ok()
            .flatten()
            .ok_or(LocalFileCopyError::ScenarioUnavailable)?;
        let (source_root, source_connection, destination_root, destination_connection) =
            if child.source_is_left {
                (
                    scenario.left_root,
                    scenario.left_connection_id,
                    scenario.right_root,
                    scenario.right_connection_id,
                )
            } else {
                (
                    scenario.right_root,
                    scenario.right_connection_id,
                    scenario.left_root,
                    scenario.left_connection_id,
                )
            };
        run_local_scenario_file_copy(
            &paths.jobs,
            &paths.registry,
            &child.scenario.scenario_id,
            child.scenario.revision,
            Path::new(&source_root),
            &source_connection,
            Path::new(&child.relative_path),
            Path::new(&destination_root),
            &destination_connection,
            Path::new(&child.relative_path),
            &child.job_id,
        )
    })();
    record_batch_child_outcome(
        paths,
        child,
        yandex.map(|(connection, _)| connection),
        result,
    );
    runner().active.fetch_sub(1, Ordering::SeqCst);
}

/// What the Worker does after one failed child (D-024 retry classes).
enum Failure {
    /// Before any effect, and it can pass later.
    Transient,
    /// It cannot pass without a change by the user.
    Permanent,
    /// The effect is not proven. Never retry; reconciliation decides.
    Unproven,
    /// Yandex refused the token: pause the batch until a new sign-in.
    SignInRequired,
}

fn classify(error: &LocalFileCopyError) -> (Failure, String) {
    use LocalFileCopyError as E;
    match error {
        E::Remote(remote) => {
            let code = match remote {
                YandexError::Provider(code) | YandexError::ContentUnavailable(code) => {
                    format!("{}({code})", yandex_error_code(remote))
                }
                _ => yandex_error_code(remote).to_owned(),
            };
            let failure = match remote {
                YandexError::Unauthorized => Failure::SignInRequired,
                YandexError::Transport(_)
                | YandexError::RateLimited
                | YandexError::ContentUnavailable(_) => Failure::Transient,
                YandexError::Provider(status) if *status >= 500 => Failure::Transient,
                _ => Failure::Permanent,
            };
            (failure, code)
        }
        E::OutcomeUnknown(_) => (Failure::Unproven, "outcome_unknown".into()),
        E::VerificationFailed => (Failure::Unproven, "verification_failed".into()),
        E::ConnectionUnavailable => (Failure::Transient, "local_connection_unavailable".into()),
        E::Local(_) => (Failure::Transient, copy_error_kind(error)),
        E::Store(_) => (Failure::Transient, "job_store_unavailable".into()),
        E::TargetExists => (Failure::Permanent, "target_exists".into()),
        E::ConflictStale => (Failure::Permanent, "plan_stale".into()),
        E::ScenarioScopeMismatch => (Failure::Permanent, "scenario_scope_mismatch".into()),
        E::ScenarioUnavailable => (Failure::Permanent, "scenario_unavailable".into()),
        E::CapabilityUnavailable => (Failure::Permanent, "local_capability_unavailable".into()),
        _ => (Failure::Permanent, copy_error_kind(error)),
    }
}

fn record_batch_child_outcome(
    paths: &WorkerPaths,
    child: &DueBatchChild,
    yandex_connection: Option<&str>,
    result: Result<LocalFileCopyStatus, LocalFileCopyError>,
) {
    let Ok(registry) = Registry::open(&paths.registry) else {
        return;
    };
    let finish = |state: &str, error: Option<&str>| {
        let _ = registry.finish_scenario_create_batch_child(
            &child.batch_id,
            &child.relative_path,
            state,
            error,
        );
    };
    let error = match result {
        Ok(LocalFileCopyStatus::Completed)
        | Ok(LocalFileCopyStatus::Existing(JobState::Completed)) => {
            return finish("completed", None)
        }
        Ok(LocalFileCopyStatus::Existing(JobState::Cancelled)) => return finish("cancelled", None),
        Ok(LocalFileCopyStatus::Existing(JobState::Failed)) => return finish("failed", None),
        Ok(LocalFileCopyStatus::Existing(_)) => return finish("unknown", Some("outcome_unknown")),
        Err(error) => error,
    };
    let (failure, code) = classify(&error);
    // Diagnostics: the error kind only, never a path, a token, or a link.
    eprintln!(
        "qadren-worker: {} batch child {} failed: {code}",
        log_time(),
        child.job_id
    );
    // A retry needs proof that the failed Job made no effect: no Job, or a
    // Job closed as failed before any effect.
    let closed_before_effect = matches!(
        job_state(paths, &child.job_id),
        None | Some(JobState::Failed)
    );
    match failure {
        Failure::Unproven => finish("unknown", Some(&code)),
        Failure::Permanent => finish("failed", Some(&code)),
        Failure::SignInRequired => {
            if let Some(connection) = yandex_connection {
                runner().forget_token(connection);
            }
            let _ = registry.set_scenario_create_batch_paused(&child.batch_id, true);
            if closed_before_effect {
                let _ = registry.schedule_scenario_create_batch_child_retry(
                    &child.batch_id,
                    &child.relative_path,
                    &child.job_id,
                    unix_now(),
                    &code,
                );
            } else {
                finish("unknown", Some(&code));
            }
        }
        Failure::Transient => {
            let attempt = child.retries as usize;
            let delay = RETRY_DELAYS.get(attempt).copied().or_else(|| {
                registry
                    .retry_until_success()
                    .unwrap_or(false)
                    .then_some(RETRY_FOREVER_DELAY)
            });
            if let (true, Some(delay)) = (closed_before_effect, delay) {
                let _ = registry.schedule_scenario_create_batch_child_retry(
                    &child.batch_id,
                    &child.relative_path,
                    &child.job_id,
                    unix_now() + delay,
                    &code,
                );
            } else if closed_before_effect {
                finish("failed", Some(&format!("{code}_after_retries")));
            } else {
                finish("unknown", Some(&code));
            }
        }
    }
}

/// A stable, path-free name for a copy error, for the Worker log.
fn copy_error_kind(error: &LocalFileCopyError) -> String {
    match error {
        LocalFileCopyError::OutcomeUnknown(inner) => {
            format!("outcome_unknown({})", local_error_kind(inner))
        }
        LocalFileCopyError::Local(inner) => format!("local({})", local_error_kind(inner)),
        LocalFileCopyError::Remote(inner) => match inner {
            YandexError::Provider(code)
            | YandexError::UploadRejected(code)
            | YandexError::ContentUnavailable(code) => {
                format!("{}({code})", yandex_error_code(inner))
            }
            _ => yandex_error_code(inner).to_owned(),
        },
        other => format!("{other:?}")
            .chars()
            .take_while(|c| *c != '(')
            .collect(),
    }
}

fn local_error_kind(error: &qadren_core::local_fs::LocalError) -> String {
    use qadren_core::local_fs::LocalError;
    match error {
        LocalError::Io(io) | LocalError::OutcomeUnknown(io) => {
            // io::Error messages from QADREN are fixed strings without paths.
            format!(
                "{:?}:{}",
                io.kind(),
                io.raw_os_error()
                    .map_or_else(|| io.to_string(), |code| code.to_string())
            )
        }
        other => format!("{other:?}"),
    }
}

fn request_create_batch_cancellation(
    paths: &WorkerPaths,
    request: &Request,
) -> Result<&'static str, &'static str> {
    let batch_id = request.batch_id.as_deref().ok_or("invalid_request")?;
    Registry::open(&paths.registry)
        .map_err(|_| "registry_unavailable")?
        .request_scenario_create_batch_cancellation(batch_id)
        .map_err(|_| "batch_unavailable")?;
    Ok("cancellation_requested")
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
    if let Err(error) =
        require_current_scenario_revision(&paths.registry, scenario_id, scenario_revision)
    {
        return Err(match error {
            LocalFileCopyError::ScenarioRevisionStale => "scenario_revision_stale",
            _ => "scenario_unavailable",
        });
    }
    // Fail closed: a replacement without the planned metadata could overwrite
    // a file that changed after the user confirmed.
    let (Some(expected_source), Some(expected_destination)) = (
        request.expected_source.as_ref(),
        request.expected_destination.as_ref(),
    ) else {
        return Err("plan_snapshot_required");
    };
    let registry = Registry::open(&paths.registry).map_err(|_| "registry_unavailable")?;
    let result = if yandex_source(&registry, scenario_id, scenario_revision) {
        if source_relative_path != destination_relative_path {
            return Err("invalid_request");
        }
        run_yandex_download(
            paths,
            scenario_id,
            scenario_revision,
            source_relative_path,
            job_id,
            request.access_token.as_ref().map(|token| token.0.as_str()),
            Some((
                &expected_source.confirmed(),
                &expected_destination.confirmed(),
                request.replacement_consent.unwrap_or(false),
            )),
        )
    } else {
        run_local_scenario_file_replace_planned(
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
            &expected_source.confirmed(),
            &expected_destination.confirmed(),
        )
    };
    match result {
        Err(LocalFileCopyError::Remote(error)) => Err(yandex_error_code(&error)),
        Err(LocalFileCopyError::ConflictStale) => Err("plan_stale"),
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

fn execute_manual_mkdir(
    paths: &WorkerPaths,
    request: &Request,
) -> Result<&'static str, &'static str> {
    let (Some(root), Some(connection), Some(relative), Some(job_id)) = (
        request.source_root.as_deref(),
        request.source_connection_id.as_deref(),
        request.source_relative_path.as_deref(),
        request.job_id.as_deref(),
    ) else {
        return Err("invalid_request");
    };
    match run_local_directory_create(
        &paths.jobs,
        &paths.registry,
        Path::new(root),
        connection,
        Path::new(relative),
        job_id,
    ) {
        Ok(_) => Ok("completed"),
        Err(qadren_core::job_execution::LocalDirectoryCreateError::TargetExists) => {
            Err("target_exists")
        }
        Err(qadren_core::job_execution::LocalDirectoryCreateError::ConnectionUnavailable) => {
            Err("local_connection_unavailable")
        }
        Err(qadren_core::job_execution::LocalDirectoryCreateError::CapabilityUnavailable) => {
            Err("local_capability_unavailable")
        }
        Err(qadren_core::job_execution::LocalDirectoryCreateError::OutcomeUnknown(_)) => {
            Err("outcome_unknown")
        }
        Err(_) => Err("manual_mkdir_unavailable"),
    }
}

fn reconcile_manual(
    paths: &WorkerPaths,
    request: &Request,
    directory: bool,
) -> Result<&'static str, &'static str> {
    let (Some(root), Some(connection), Some(job_id)) = (
        request.source_root.as_deref(),
        request.source_connection_id.as_deref(),
        request.job_id.as_deref(),
    ) else {
        return Err("invalid_request");
    };
    if directory {
        match reconcile_local_directory_create(
            &paths.jobs,
            &paths.registry,
            Path::new(root),
            connection,
            job_id,
        ) {
            Ok(qadren_core::job_execution::LocalDirectoryReconciliationStatus::Completed)
            | Ok(qadren_core::job_execution::LocalDirectoryReconciliationStatus::Existing(
                JobState::Completed,
            )) => Ok("completed"),
            Ok(_) => Ok("unknown"),
            Err(_) => Err("reconciliation_unavailable"),
        }
    } else {
        match reconcile_local_file_copy(
            &paths.jobs,
            &paths.registry,
            Path::new(root),
            connection,
            job_id,
        ) {
            Ok(LocalFileCopyReconciliationStatus::Completed)
            | Ok(LocalFileCopyReconciliationStatus::Existing(JobState::Completed)) => {
                Ok("completed")
            }
            Ok(_) => Ok("unknown"),
            Err(_) => Err("reconciliation_unavailable"),
        }
    }
}

fn reconcile_reconnect(
    paths: &WorkerPaths,
    request: &Request,
) -> Result<&'static str, &'static str> {
    let (Some(root), Some(connection)) = (
        request.source_root.as_deref(),
        request.source_connection_id.as_deref(),
    ) else {
        return Err("invalid_request");
    };
    let root = Path::new(root);
    let directories =
        local_directory_reconciliation_candidates(&paths.jobs, &paths.registry, root, connection)
            .map_err(|_| "reconciliation_unavailable")?;
    for candidate in directories {
        let _ = reconcile_local_directory_create(
            &paths.jobs,
            &paths.registry,
            root,
            connection,
            &candidate.job_id,
        );
    }
    let copies =
        local_file_copy_reconciliation_candidates(&paths.jobs, &paths.registry, root, connection)
            .map_err(|_| "reconciliation_unavailable")?;
    for candidate in copies {
        let _ = reconcile_local_file_copy(
            &paths.jobs,
            &paths.registry,
            root,
            connection,
            &candidate.job_id,
        );
    }
    let remaining_directories =
        local_directory_reconciliation_candidates(&paths.jobs, &paths.registry, root, connection)
            .map_err(|_| "reconciliation_unavailable")?;
    let remaining_copies =
        local_file_copy_reconciliation_candidates(&paths.jobs, &paths.registry, root, connection)
            .map_err(|_| "reconciliation_unavailable")?;
    Ok(
        if remaining_directories.is_empty() && remaining_copies.is_empty() {
            "completed"
        } else {
            "unknown"
        },
    )
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
        Ok(LocalFileCopyStatus::Completed) => {
            complete_ready_schedule_jobs(paths, conflict_id)?;
            Ok("completed")
        }
        Ok(LocalFileCopyStatus::Existing(_)) => {
            complete_ready_schedule_jobs(paths, conflict_id)?;
            Ok("existing")
        }
        Err(LocalFileCopyError::ConsentRequired) => Err("consent_required"),
        Err(LocalFileCopyError::ConflictStale) => {
            // Report the stale snapshot even if the coordinator update fails.
            // The coordinator then stays waiting and is never shown as done.
            let _ = complete_ready_schedule_jobs(paths, conflict_id);
            Err("conflict_stale")
        }
        Err(LocalFileCopyError::ScenarioRevisionStale) => {
            let _ = complete_ready_schedule_jobs(paths, conflict_id);
            Err("scenario_revision_stale")
        }
        Err(LocalFileCopyError::ConflictUnavailable) => Err("conflict_unavailable"),
        Err(LocalFileCopyError::CapabilityUnavailable) => Err("local_capability_unavailable"),
        Err(LocalFileCopyError::OutcomeUnknown(_)) => Err("outcome_unknown"),
        Err(LocalFileCopyError::Store(_)) => Err("job_store_unavailable"),
        Err(LocalFileCopyError::Local(_)) => Err("local_operation_failed"),
        Err(_) => Err("conflict_operation_failed"),
    }
}

/// Move each coordinator linked to `conflict_id` out of
/// `WAITING_FOR_DECISION` once all its linked tasks are terminal. All completed
/// tasks complete the coordinator. Any stale task fails it, so the scheduled run
/// is never reported as a success. A coordinator that already left the waiting
/// state is skipped.
fn complete_ready_schedule_jobs(
    paths: &WorkerPaths,
    conflict_id: &str,
) -> Result<(), &'static str> {
    let registry = Registry::open(&paths.registry).map_err(|_| "registry_unavailable")?;
    let ready = registry
        .resolved_schedule_jobs_for_conflict(conflict_id)
        .map_err(|_| "schedule_conflict_unavailable")?;
    let mut jobs = JobStore::open(&paths.jobs).map_err(|_| "job_store_unavailable")?;
    for (job_id, succeeded) in ready {
        if jobs.state(&job_id).map_err(|_| "job_store_unavailable")?
            != Some(JobState::WaitingForDecision)
        {
            continue;
        }
        if !succeeded {
            jobs.advance(
                &job_id,
                JobState::WaitingForDecision,
                JobState::Failed,
                Confirmation::Unknown,
            )
            .map_err(|_| "schedule_job_state_unavailable")?;
            continue;
        }
        jobs.advance(
            &job_id,
            JobState::WaitingForDecision,
            JobState::Running,
            Confirmation::Confirmed(EvidenceLevel::Requested),
        )
        .map_err(|_| "schedule_job_state_unavailable")?;
        jobs.advance(
            &job_id,
            JobState::Running,
            JobState::Completed,
            Confirmation::Confirmed(EvidenceLevel::ProviderAcknowledged),
        )
        .map_err(|_| "schedule_job_state_unavailable")?;
    }
    Ok(())
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
            if matches!(decision, "skip_transfer" | "keep_file") {
                complete_ready_schedule_jobs(paths, id)?;
            }
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
        "status" => {
            // `active` counts batch children that run now (D-024).
            let _ = writeln!(
                stream,
                "{}",
                serde_json::json!({
                    "ok": true,
                    "status": "running",
                    "active": runner().active.load(Ordering::SeqCst),
                })
            );
        }
        "start_plan" => {
            let answer = start_plan(paths, &request);
            let _ = writeln!(stream, "{answer}");
        }
        "plan_status" => {
            let answer = plan_status(paths, &request);
            let _ = writeln!(stream, "{answer}");
        }
        "plan_page" => {
            let answer = plan_page(paths, &request);
            let _ = writeln!(stream, "{answer}");
        }
        "execute_plan_creates" => {
            let answer = execute_plan_creates(paths, &request);
            let _ = writeln!(stream, "{answer}");
        }
        "start_snapshot" => {
            let answer = start_snapshots(paths, &request);
            let _ = writeln!(stream, "{answer}");
        }
        "snapshot_status" => {
            let answer = snapshot_status(paths, &request);
            let _ = writeln!(stream, "{answer}");
        }
        "provide_access_token" => {
            let ok = match (
                request.connection_id.as_deref(),
                request.access_token.as_ref(),
            ) {
                (Some(connection), Some(AccessToken(token)))
                    if !connection.is_empty() && !token.is_empty() =>
                {
                    runner().remember_token(connection, token);
                    true
                }
                _ => false,
            };
            respond(
                &mut stream,
                &Response {
                    ok,
                    status: if ok { "updated" } else { "invalid_request" },
                    staged: None,
                    dispatched: None,
                },
            );
        }
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
        "execute_rule_deletions" => match execute_rule_deletions(paths, &request) {
            Ok((status, done)) => respond(
                &mut stream,
                &Response {
                    ok: true,
                    status,
                    staged: None,
                    dispatched: Some(done),
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
        "execute_create_batch" => match execute_create_batch(paths, &request) {
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
        "cancel_create_batch" => match request_create_batch_cancellation(paths, &request) {
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
        "execute_manual_mkdir" => match execute_manual_mkdir(paths, &request) {
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
        "reconcile_manual_mkdir" | "reconcile_manual_copy" => {
            match reconcile_manual(paths, &request, request.command == "reconcile_manual_mkdir") {
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
        "reconcile_local_reconnect" => match reconcile_reconnect(paths, &request) {
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
    // D-024: this process owns the JobStore. The claim applies the restart
    // rule once; the IPC thread and the run thread then share the store.
    JobStore::claim_for_process(&paths.jobs).map_err(|_| "job_store_unavailable".to_string())?;
    if once {
        dispatch_due(&paths)?;
        run_due_batch_children(&paths);
        return Ok(());
    }
    {
        let paths = paths.clone();
        thread::spawn(move || run_batches_forever(paths));
    }
    {
        let paths = paths.clone();
        thread::spawn(move || run_snapshots_forever(paths));
    }
    // D-024: the IPC thread answers at all times. Reconciliation and
    // schedules can wait for a network drive, so they poll on their own thread.
    {
        let paths = paths.clone();
        thread::spawn(move || loop {
            let _ = dispatch_due(&paths);
            thread::sleep(POLL_INTERVAL);
        });
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
    while !stop {
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
            // QADREN_DATA_ROOT moves the app and the Worker to the same
            // disposable data directory for native tests. It must be absolute.
            let base = match env::var_os("QADREN_DATA_ROOT").filter(|root| !root.is_empty()) {
                Some(root) if Path::new(&root).is_absolute() => PathBuf::from(root),
                Some(_) => usage(),
                None => {
                    let home = env::var_os("HOME").unwrap_or_else(|| usage());
                    PathBuf::from(home)
                        .join("Library")
                        .join("Application Support")
                        .join("QADREN")
                }
            };
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

/// Under launchd the Worker has no terminal, and its error lines were lost.
/// Write them to `worker.log` next to the JobStore. The lines hold error
/// kinds and Job IDs only, never a path, a token, or a link. A log over
/// 1 MB starts again empty.
fn redirect_stderr_to_log(paths: &WorkerPaths) {
    use std::os::fd::AsRawFd;
    if unsafe { libc::isatty(2) } == 1 {
        return;
    }
    let Some(directory) = paths.jobs.parent() else {
        return;
    };
    let log = directory.join("worker.log");
    if fs::metadata(&log).is_ok_and(|metadata| metadata.len() > 1_048_576) {
        let _ = fs::remove_file(&log);
    }
    let Ok(file) = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(&log)
    else {
        return;
    };
    unsafe { libc::dup2(file.as_raw_fd(), 2) };
}

/// Seconds since 1970 for a log line.
fn log_time() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

fn main() {
    let (paths, once) = parse_paths();
    redirect_stderr_to_log(&paths);
    eprintln!("qadren-worker: {} started", log_time());
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
        // Unix-domain socket paths have a small platform limit. Keep test
        // roots under the system temporary directory so an exported checkout
        // cannot make the private Worker socket unbindable.
        let temporary = std::env::temp_dir().canonicalize().unwrap();
        let path = temporary.join(format!(
            "qw-{name}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        path
    }

    fn wait_for_socket(socket: &Path) -> bool {
        // The Worker binds only after it obtains the exclusive lock and checks
        // the socket directory. A loaded macOS host can need more than one
        // second for that setup. Polling the real socket keeps the test tied to
        // the public readiness boundary instead of to thread scheduling.
        for _ in 0..500 {
            if socket.exists() {
                return true;
            }
            thread::sleep(Duration::from_millis(10));
        }
        false
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
    fn worker_restart_resumes_confirmed_create_batch_without_controller() {
        let root = root("create-batch-restart");
        let left_root = root.join("left");
        let right_root = root.join("right");
        fs::create_dir_all(&left_root).unwrap();
        fs::create_dir_all(&right_root).unwrap();
        fs::write(left_root.join("first.txt"), b"first").unwrap();
        fs::write(left_root.join("second.txt"), b"second").unwrap();
        let paths = WorkerPaths {
            socket: root.join("worker.sock"),
            registry: root.join("registry.sqlite3"),
            jobs: root.join("jobs.sqlite3"),
        };
        let mut registry = Registry::open(&paths.registry).unwrap();
        let left = registry.add("local_filesystem", "Left").unwrap();
        let right = registry.add("local_filesystem", "Right").unwrap();
        let scenario = registry
            .create_scenario(
                "Restart batch",
                &left,
                left_root.to_str().unwrap(),
                &right,
                right_root.to_str().unwrap(),
                "left_to_right",
            )
            .unwrap();
        let receipt = registry
            .create_scenario_create_batch(
                &ScenarioReference {
                    scenario_id: scenario,
                    revision: 1,
                },
                &[("first.txt".into(), true), ("second.txt".into(), true)],
            )
            .unwrap();
        drop(registry);

        // The receipt was saved by a controller that has now exited. A new
        // Worker process polls it from the existing Registry and owns effects.
        assert_eq!(dispatch_due(&paths).unwrap(), (0, 0));
        assert_eq!(fs::read(right_root.join("first.txt")).unwrap(), b"first");
        assert_eq!(fs::read(right_root.join("second.txt")).unwrap(), b"second");
        let registry = Registry::open(&paths.registry).unwrap();
        let stored = registry
            .scenario_create_batch(&receipt.id)
            .unwrap()
            .unwrap();
        assert!(stored
            .children
            .iter()
            .all(|child| child.state == "completed"));
        assert_eq!(
            JobStore::open(&paths.jobs)
                .unwrap()
                .list_jobs()
                .unwrap()
                .len(),
            2
        );

        // A subsequent restart finds no pending child and therefore creates no
        // duplicate Job or replacement effect.
        assert_eq!(dispatch_due(&paths).unwrap(), (0, 0));
        assert_eq!(
            JobStore::open(&paths.jobs)
                .unwrap()
                .list_jobs()
                .unwrap()
                .len(),
            2
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn worker_cancellation_marks_pending_create_batch_children_without_effect() {
        let root = root("create-batch-cancel");
        let left_root = root.join("left");
        let right_root = root.join("right");
        fs::create_dir_all(&left_root).unwrap();
        fs::create_dir_all(&right_root).unwrap();
        fs::write(left_root.join("first.txt"), b"first").unwrap();
        fs::write(left_root.join("second.txt"), b"second").unwrap();
        let paths = WorkerPaths {
            socket: root.join("worker.sock"),
            registry: root.join("registry.sqlite3"),
            jobs: root.join("jobs.sqlite3"),
        };
        let mut registry = Registry::open(&paths.registry).unwrap();
        let left = registry.add("local_filesystem", "Left").unwrap();
        let right = registry.add("local_filesystem", "Right").unwrap();
        let scenario = registry
            .create_scenario(
                "Cancelled batch",
                &left,
                left_root.to_str().unwrap(),
                &right,
                right_root.to_str().unwrap(),
                "left_to_right",
            )
            .unwrap();
        let receipt = registry
            .create_scenario_create_batch(
                &ScenarioReference {
                    scenario_id: scenario,
                    revision: 1,
                },
                &[("first.txt".into(), true), ("second.txt".into(), true)],
            )
            .unwrap();
        registry
            .request_scenario_create_batch_cancellation(&receipt.id)
            .unwrap();
        drop(registry);

        assert_eq!(dispatch_due(&paths).unwrap(), (0, 0));
        assert!(!right_root.join("first.txt").exists());
        assert!(!right_root.join("second.txt").exists());
        let stored = Registry::open(&paths.registry)
            .unwrap()
            .scenario_create_batch(&receipt.id)
            .unwrap()
            .unwrap();
        assert!(stored
            .children
            .iter()
            .all(|child| child.state == "cancelled"));
        assert!(JobStore::open(&paths.jobs)
            .unwrap()
            .list_jobs()
            .unwrap()
            .is_empty());
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
        registry
            .select_conflict_task(&conflicts[0].id, Some(true), "skip_transfer")
            .unwrap();
        complete_ready_schedule_jobs(&paths, &conflicts[0].id).unwrap();
        assert!(JobStore::open(&paths.jobs)
            .unwrap()
            .list_jobs()
            .unwrap()
            .iter()
            .any(|job| job.id.starts_with("schedule-") && job.state == JobState::Completed));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn worker_runs_one_catch_up_for_a_slot_missed_while_unavailable() {
        let root = root("scheduled-catch-up");
        let left_root = root.join("left");
        let right_root = root.join("right");
        fs::create_dir_all(&left_root).unwrap();
        fs::create_dir_all(&right_root).unwrap();
        fs::write(left_root.join("missed.txt"), b"missed slot").unwrap();
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
                "Catch-up",
                &left,
                left_root.to_str().unwrap(),
                &right,
                right_root.to_str().unwrap(),
                "left_to_right",
            )
            .unwrap();
        let current = now();
        let missed = current - chrono::Duration::hours(3);
        registry
            .create_scenario_schedule(
                &scenario,
                "UTC",
                missed.weekday().num_days_from_monday() as u8,
                missed.hour() as u8,
                missed.minute() as u8,
            )
            .unwrap();
        // The last poll happened one day ago. The Worker was then stopped or
        // the Mac was asleep, so the slot three hours ago was never observed.
        assert_eq!(
            registry
                .stage_schedule_catch_up(
                    current - chrono::Duration::days(1),
                    chrono::Duration::from_std(POLL_OVERLAP).unwrap(),
                )
                .unwrap(),
            0
        );
        drop(registry);

        assert_eq!(dispatch_due(&paths).unwrap(), (1, 1));
        assert_eq!(
            fs::read(right_root.join("missed.txt")).unwrap(),
            b"missed slot"
        );
        let jobs = JobStore::open(&paths.jobs).unwrap().list_jobs().unwrap();
        assert_eq!(jobs.len(), 2);
        assert!(jobs.iter().all(|job| job.state == JobState::Completed));
        assert_eq!(dispatch_due(&paths).unwrap(), (0, 0));
        assert_eq!(
            JobStore::open(&paths.jobs)
                .unwrap()
                .list_jobs()
                .unwrap()
                .len(),
            2
        );
        fs::remove_dir_all(root).unwrap();
    }

    /// Add one more weekly rule for the current minute. Each call produces one
    /// new due slot for the next `dispatch_due`.
    fn schedule_current_minute(paths: &WorkerPaths, scenario: &str) {
        let current = now();
        Registry::open(&paths.registry)
            .unwrap()
            .create_scenario_schedule(
                scenario,
                "UTC",
                current.weekday().num_days_from_monday() as u8,
                current.hour() as u8,
                current.minute() as u8,
            )
            .unwrap();
    }

    fn scheduled_fixture(name: &str) -> (PathBuf, PathBuf, PathBuf, WorkerPaths, String) {
        let root = root(name);
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
                "Decisions",
                &left,
                left_root.to_str().unwrap(),
                &right,
                right_root.to_str().unwrap(),
                "left_to_right",
            )
            .unwrap();
        (root, left_root, right_root, paths, scenario)
    }

    fn coordinator_states(paths: &WorkerPaths) -> Vec<JobState> {
        let mut jobs: Vec<_> = JobStore::open(&paths.jobs)
            .unwrap()
            .list_jobs()
            .unwrap()
            .into_iter()
            .filter(|job| job.id.starts_with("schedule-") && !job.id.starts_with("schedule-copy-"))
            .collect();
        jobs.sort_by(|left, right| left.id.cmp(&right.id));
        jobs.into_iter().map(|job| job.state).collect()
    }

    fn conflict_request(command: &str, conflict_id: &str, job_id: &str, consent: bool) -> Request {
        serde_json::from_value(serde_json::json!({
            "command": command,
            "conflict_id": conflict_id,
            "job_id": job_id,
            "destructive_consent": consent,
        }))
        .unwrap()
    }

    #[test]
    fn scheduled_deletion_waits_for_fresh_consent_and_completes_its_coordinator() {
        let (root, left_root, right_root, paths, scenario) =
            scheduled_fixture("scheduled-deletion");
        fs::write(left_root.join("report.txt"), b"baseline").unwrap();
        schedule_current_minute(&paths, &scenario);
        assert_eq!(dispatch_due(&paths).unwrap(), (1, 1));
        assert_eq!(
            fs::read(right_root.join("report.txt")).unwrap(),
            b"baseline"
        );

        // The source file is deleted. The next scheduled run must not delete
        // the destination by itself.
        fs::remove_file(left_root.join("report.txt")).unwrap();
        schedule_current_minute(&paths, &scenario);
        assert_eq!(dispatch_due(&paths).unwrap(), (1, 1));
        assert!(right_root.join("report.txt").exists());
        let registry = Registry::open(&paths.registry).unwrap();
        let reference = ScenarioReference {
            scenario_id: scenario.clone(),
            revision: 1,
        };
        let conflicts = registry.list_conflict_tasks(&reference).unwrap();
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].kind, "deletion");
        assert_eq!(conflicts[0].state, "pending");
        let states = coordinator_states(&paths);
        assert_eq!(states.len(), 2);
        assert!(states.contains(&JobState::Completed));
        assert!(coordinator_states(&paths).contains(&JobState::WaitingForDecision));

        // An unselected task cannot be executed, even with consent.
        assert_eq!(
            execute_conflict(
                &paths,
                &conflict_request(
                    "execute_conflict_delete",
                    &conflicts[0].id,
                    "delete-early",
                    true
                ),
                true,
            ),
            Err("conflict_unavailable")
        );
        registry
            .select_conflict_task(&conflicts[0].id, Some(true), "delete_as_source")
            .unwrap();
        assert_eq!(
            execute_conflict(
                &paths,
                &conflict_request(
                    "execute_conflict_delete",
                    &conflicts[0].id,
                    "delete-declined",
                    false
                ),
                true,
            ),
            Err("consent_required")
        );
        assert!(right_root.join("report.txt").exists());
        assert!(coordinator_states(&paths).contains(&JobState::WaitingForDecision));

        assert_eq!(
            execute_conflict(
                &paths,
                &conflict_request(
                    "execute_conflict_delete",
                    &conflicts[0].id,
                    "delete-confirmed",
                    true
                ),
                true,
            ),
            Ok("completed")
        );
        assert!(!right_root.join("report.txt").exists());
        assert!(coordinator_states(&paths)
            .iter()
            .all(|state| *state == JobState::Completed));
        // The used consent is not reusable for another effect.
        assert_eq!(
            execute_conflict(
                &paths,
                &conflict_request(
                    "execute_conflict_delete",
                    &conflicts[0].id,
                    "delete-again",
                    true
                ),
                true,
            ),
            Err("conflict_unavailable")
        );
        assert_eq!(dispatch_due(&paths).unwrap(), (0, 0));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn stale_scheduled_conflict_fails_its_coordinator_without_an_effect() {
        let (root, left_root, right_root, paths, scenario) =
            scheduled_fixture("scheduled-stale-conflict");
        fs::write(left_root.join("a.txt"), b"first a").unwrap();
        fs::write(left_root.join("b.txt"), b"first b").unwrap();
        schedule_current_minute(&paths, &scenario);
        assert_eq!(dispatch_due(&paths).unwrap(), (1, 1));

        fs::write(left_root.join("a.txt"), b"second version of a").unwrap();
        fs::write(left_root.join("b.txt"), b"second version of b").unwrap();
        schedule_current_minute(&paths, &scenario);
        assert_eq!(dispatch_due(&paths).unwrap(), (1, 1));
        let registry = Registry::open(&paths.registry).unwrap();
        let conflicts = registry
            .list_conflict_tasks(&ScenarioReference {
                scenario_id: scenario.clone(),
                revision: 1,
            })
            .unwrap();
        assert_eq!(conflicts.len(), 2);
        let task = |path: &str| {
            conflicts
                .iter()
                .find(|task| task.relative_path == path)
                .unwrap()
                .id
                .clone()
        };

        // One resolved task out of two keeps the coordinator waiting.
        registry
            .select_conflict_task(&task("b.txt"), Some(true), "skip_transfer")
            .unwrap();
        complete_ready_schedule_jobs(&paths, &task("b.txt")).unwrap();
        assert!(coordinator_states(&paths).contains(&JobState::WaitingForDecision));

        // The destination changes after the decision was offered. The selected
        // replacement becomes stale and changes no file.
        fs::write(right_root.join("a.txt"), b"edited at the destination").unwrap();
        registry
            .select_conflict_task(&task("a.txt"), Some(true), "replace_from_source")
            .unwrap();
        assert_eq!(
            execute_conflict(
                &paths,
                &conflict_request(
                    "execute_conflict_replace",
                    &task("a.txt"),
                    "replace-stale",
                    true
                ),
                false,
            ),
            Err("conflict_stale")
        );
        assert_eq!(
            fs::read(right_root.join("a.txt")).unwrap(),
            b"edited at the destination"
        );
        assert_eq!(fs::read(right_root.join("b.txt")).unwrap(), b"first b");
        let states = coordinator_states(&paths);
        assert!(states.contains(&JobState::Failed));
        assert!(!states.contains(&JobState::WaitingForDecision));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unresolvable_unknown_job_does_not_block_scheduled_runs() {
        let (root, left_root, right_root, paths, scenario) =
            scheduled_fixture("unresolvable-unknown");
        fs::write(left_root.join("report.txt"), b"scheduled").unwrap();
        let identity = LocalFilesystem::open(&right_root)
            .unwrap()
            .root_identity()
            .unwrap();
        let mut store = JobStore::open(&paths.jobs).unwrap();
        // The Job names a Scenario revision that the Registry cannot supply.
        store
            .enqueue("orphan-copy", Some(("missing-scenario", 1)))
            .unwrap();
        store
            .advance(
                "orphan-copy",
                JobState::Queued,
                JobState::Running,
                Confirmation::Confirmed(EvidenceLevel::Requested),
            )
            .unwrap();
        store
            .prepare_bound_attempt_with_root_identity(
                "orphan-copy",
                "orphan-copy-attempt",
                "local_copy_new",
                "missing-connection",
                "orphan.txt",
                6,
                "00000000000000000000000000000000",
                &format!("unix:{}:{}", identity.device, identity.inode),
            )
            .unwrap();
        drop(store);
        // Reopening after the simulated crash turns the running Job UNKNOWN.
        assert_eq!(
            JobStore::open(&paths.jobs)
                .unwrap()
                .state("orphan-copy")
                .unwrap(),
            Some(JobState::Unknown)
        );

        schedule_current_minute(&paths, &scenario);
        assert_eq!(dispatch_due(&paths).unwrap(), (1, 1));
        assert_eq!(
            fs::read(right_root.join("report.txt")).unwrap(),
            b"scheduled"
        );
        assert_eq!(
            JobStore::open(&paths.jobs)
                .unwrap()
                .state("orphan-copy")
                .unwrap(),
            Some(JobState::Unknown)
        );
        assert!(!right_root.join("orphan.txt").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn same_local_connection_on_both_sides_reconciles_the_destination_root() {
        let root = root("same-connection-reconciliation");
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
        // One "This Mac" connection is the usual Release 1 layout.
        let mac = registry.add("local_filesystem", "This Mac").unwrap();
        let scenario = registry
            .create_scenario(
                "Same connection",
                &mac,
                left_root.to_str().unwrap(),
                &mac,
                right_root.to_str().unwrap(),
                "left_to_right",
            )
            .unwrap();
        let identity = LocalFilesystem::open(&right_root)
            .unwrap()
            .root_identity()
            .unwrap();
        let root_identity = format!("unix:{}:{}", identity.device, identity.inode);
        {
            let mut store = JobStore::open(&paths.jobs).unwrap();
            store.enqueue("crashed-copy", Some((&scenario, 1))).unwrap();
            store
                .advance(
                    "crashed-copy",
                    JobState::Queued,
                    JobState::Running,
                    Confirmation::Confirmed(EvidenceLevel::Requested),
                )
                .unwrap();
            store
                .prepare_bound_attempt_with_root_identity(
                    "crashed-copy",
                    "crashed-copy-attempt",
                    "local_copy_new",
                    &mac,
                    "report.txt",
                    7,
                    "00000000000000000000000000000000",
                    &root_identity,
                )
                .unwrap();
        }
        // The next open applies the restart rule: running becomes UNKNOWN.
        reconcile_unknown_scenario_creates(&paths, &registry).unwrap();
        assert_eq!(
            JobStore::open(&paths.jobs)
                .unwrap()
                .state("crashed-copy")
                .unwrap(),
            Some(JobState::Completed)
        );
        assert_eq!(fs::read(right_root.join("report.txt")).unwrap(), b"payload");
        drop(registry);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn worker_reconciles_unknown_scenario_effects_after_restart_without_replay() {
        let root = root("scheduled-reconciliation");
        let left_root = root.join("left");
        let right_root = root.join("right");
        fs::create_dir_all(&left_root).unwrap();
        fs::create_dir_all(&right_root).unwrap();
        fs::write(left_root.join("report.txt"), b"payload").unwrap();
        fs::write(right_root.join("report.txt"), b"payload").unwrap();
        fs::write(left_root.join("manual.txt"), b"manual").unwrap();
        fs::write(right_root.join("manual.txt"), b"manual").unwrap();
        fs::write(right_root.join("replacement.txt"), b"replacement").unwrap();
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
        store
            .enqueue("manual-scenario-copy", Some((&scenario, 1)))
            .unwrap();
        store
            .advance(
                "manual-scenario-copy",
                JobState::Queued,
                JobState::Running,
                Confirmation::Confirmed(EvidenceLevel::Requested),
            )
            .unwrap();
        store
            .prepare_bound_attempt_with_root_identity(
                "manual-scenario-copy",
                "manual-scenario-copy-attempt",
                "local_copy_new",
                &right,
                "manual.txt",
                6,
                "00000000000000000000000000000000",
                &root_identity,
            )
            .unwrap();
        store
            .enqueue("replacement-scenario-copy", Some((&scenario, 1)))
            .unwrap();
        store
            .advance(
                "replacement-scenario-copy",
                JobState::Queued,
                JobState::Running,
                Confirmation::Confirmed(EvidenceLevel::Requested),
            )
            .unwrap();
        store
            .prepare_bound_attempt_with_root_identity(
                "replacement-scenario-copy",
                "replacement-scenario-copy-attempt",
                "local_copy_replace",
                &right,
                "replacement.txt",
                11,
                "00000000000000000000000000000000",
                &root_identity,
            )
            .unwrap();
        store
            .enqueue("delete-scenario-copy", Some((&scenario, 1)))
            .unwrap();
        store
            .advance(
                "delete-scenario-copy",
                JobState::Queued,
                JobState::Running,
                Confirmation::Confirmed(EvidenceLevel::Requested),
            )
            .unwrap();
        store
            .prepare_bound_attempt_with_root_identity(
                "delete-scenario-copy",
                "delete-scenario-copy-attempt",
                "local_delete",
                &right,
                "deleted.txt",
                7,
                "00000000000000000000000000000000",
                &root_identity,
            )
            .unwrap();
        drop(store);

        // dispatch_due is the first operation of a one-shot Worker start.
        // It must resolve only effects with positive metadata proof.
        dispatch_due(&paths).unwrap();
        assert_eq!(
            JobStore::open(&paths.jobs).unwrap().state(&child).unwrap(),
            Some(JobState::Completed)
        );
        assert_eq!(fs::read(right_root.join("report.txt")).unwrap(), b"payload");
        assert_eq!(
            JobStore::open(&paths.jobs)
                .unwrap()
                .state("manual-scenario-copy")
                .unwrap(),
            Some(JobState::Completed)
        );
        assert_eq!(fs::read(right_root.join("manual.txt")).unwrap(), b"manual");
        assert_eq!(
            JobStore::open(&paths.jobs)
                .unwrap()
                .state("replacement-scenario-copy")
                .unwrap(),
            Some(JobState::Completed)
        );
        assert_eq!(
            fs::read(right_root.join("replacement.txt")).unwrap(),
            b"replacement"
        );
        assert_eq!(
            JobStore::open(&paths.jobs)
                .unwrap()
                .state("delete-scenario-copy")
                .unwrap(),
            Some(JobState::Unknown),
            "missing metadata cannot prove that this Worker performed a deletion"
        );
        assert!(!right_root.join("deleted.txt").exists());
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
        assert!(wait_for_socket(&socket));

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
        assert!(wait_for_socket(&socket));
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
        // The UI sends the metadata that the user saw in the plan.
        let planned = |root: &Path| {
            let entry = LocalFilesystem::open(root)
                .unwrap()
                .stat(Path::new("report.txt"))
                .unwrap();
            serde_json::json!({"object_revision": entry.object_revision, "size": entry.size})
        };
        let expected_source = planned(&source_root);
        let expected_destination = planned(&destination_root);
        let missing_plan = serde_json::json!({
            "command":"execute_replace",
            "scenario_id":scenario,
            "scenario_revision":1,
            "source_root":source_root,
            "source_connection_id":left,
            "source_relative_path":"report.txt",
            "destination_root":destination_root,
            "destination_connection_id":right,
            "destination_relative_path":"report.txt",
            "job_id":"worker-scenario-replace-no-plan",
            "replacement_consent":true
        });
        let mut stream = UnixStream::connect(&socket).unwrap();
        writeln!(stream, "{missing_plan}").unwrap();
        response.clear();
        BufReader::new(stream).read_line(&mut response).unwrap();
        assert!(
            response.contains(r#""status":"plan_snapshot_required""#),
            "replacement without plan metadata must fail closed: {response}"
        );
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
            "replacement_consent":false,
            "expected_source":expected_source,
            "expected_destination":expected_destination
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
            "replacement_consent":true,
            "expected_source":expected_source,
            "expected_destination":expected_destination
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
        // The destination changes after the plan: the old consent is refused.
        fs::write(
            destination_root.join("report.txt"),
            b"edited after planning",
        )
        .unwrap();
        let stale_replace = serde_json::json!({
            "command":"execute_replace",
            "scenario_id":scenario,
            "scenario_revision":1,
            "source_root":source_root,
            "source_connection_id":left,
            "source_relative_path":"report.txt",
            "destination_root":destination_root,
            "destination_connection_id":right,
            "destination_relative_path":"report.txt",
            "job_id":"worker-scenario-replace-stale",
            "replacement_consent":true,
            "expected_source":planned(&source_root),
            "expected_destination":expected_destination
        });
        let mut stream = UnixStream::connect(&socket).unwrap();
        writeln!(stream, "{stale_replace}").unwrap();
        response.clear();
        BufReader::new(stream).read_line(&mut response).unwrap();
        assert!(
            response.contains(r#""status":"plan_stale""#),
            "a destination changed after planning must not be replaced: {response}"
        );
        assert_eq!(
            fs::read(destination_root.join("report.txt")).unwrap(),
            b"edited after planning"
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
            expected_source: None,
            expected_destination: None,
            conflict_id: Some(conflict.clone()),
            destructive_consent: Some(false),
            locally_paused: None,
            source_is_left: None,
            decision: None,
            batch_id: None,
            access_token: None,
            connection_id: None,
            plan_id: None,
            action: None,
            search: None,
            offset: None,
            limit: None,
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
            expected_source: None,
            expected_destination: None,
            conflict_id: None,
            destructive_consent: None,
            source_is_left: None,
            decision: None,
            batch_id: None,
            access_token: None,
            connection_id: None,
            plan_id: None,
            action: None,
            search: None,
            offset: None,
            limit: None,
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
