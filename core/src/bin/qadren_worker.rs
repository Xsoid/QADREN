//! Release 1 local Worker process.
//!
//! The Worker owns periodic schedule staging and JobStore dispatch. Its
//! private Unix socket is intentionally local-only. It does not expose a
//! network or remote-Worker protocol.

use qadren_core::confirmation::{Confirmation, EvidenceLevel};
use qadren_core::ispmanager_execution::{
    run_ispmanager_database_export, run_ispmanager_scenario_delete,
    run_ispmanager_scenario_file_download, run_ispmanager_scenario_update, DatabaseExportStatus,
};
use qadren_core::ispmanager_sftp::SftpConnection;
use qadren_core::job_execution::SftpFailure;
use qadren_core::job_execution::{
    local_directory_reconciliation_candidates, local_file_copy_reconciliation_candidates,
    reconcile_local_directory_create, reconcile_local_file_copy, reconcile_local_rule_delete,
    remove_interrupted_staging, require_current_scenario_revision, run_local_directory_create,
    run_local_file_copy, run_local_file_replace, run_local_scenario_conflict_delete,
    run_local_scenario_conflict_replace, run_local_scenario_file_copy,
    run_local_scenario_file_replace_planned, run_local_scenario_rule_delete,
    run_local_scenario_update, LocalFileCopyError, LocalFileCopyReconciliationStatus,
    LocalFileCopyStatus, LocalRuleDeleteStatus,
};
use qadren_core::jobs::JobStore;
use qadren_core::local_fs::LocalFilesystem;
use qadren_core::model::{ConfirmedFileVersion, JobState, ScenarioReference};
use qadren_core::plan_store;
use qadren_core::registry::{
    BackupDatabaseResult, BatchChildDue, BatchChildKind, DueBatchChild, Registry, RegistryError,
    RunWriteLock, ScenarioRun, ScheduleRunStart,
};
use qadren_core::scenario_planner::{ObservedFile, PlanAction};
use qadren_core::schedule_dispatch::dispatch_pending_schedule_jobs;
use qadren_core::snapshot::{
    walk_local_step, walk_sftp_step, walk_yandex_step, SnapshotError, SnapshotKey, SnapshotStore,
};
use qadren_core::storage_access::{ISPMANAGER, SMB_SHARE, YANDEX_DISK};
use qadren_core::yandex::{YandexError, YandexMetadataClient};
use qadren_core::yandex_execution::{
    run_yandex_scenario_delete, run_yandex_scenario_file_download, run_yandex_scenario_update,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::{FileTypeExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, SystemTime};

const POLL_INTERVAL: Duration = Duration::from_secs(60);
/// The longest time that `prepare_restart` holds new effects back. A restart
/// that the app abandons must not stop synchronization.
const RESTART_DRAIN_LIMIT: i64 = 600;
const POLL_OVERLAP: Duration = Duration::from_secs(90);
const MAX_RULE_DELETION_SCAN_ENTRIES: usize = 10_000;

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
    /// The connection of `provide_access_token` and `provide_smb_credentials`.
    #[serde(default)]
    connection_id: Option<String>,
    /// SMB login and password from the app Keychain (issue #32). The Worker
    /// keeps them in memory only.
    #[serde(default)]
    login: Option<String>,
    #[serde(default)]
    password: Option<AccessToken>,
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

/// The ISPmanager source connection of a Backup/export revision (D-027).
fn ispmanager_source_connection(
    registry: &Registry,
    scenario_id: &str,
    revision: u64,
) -> Option<String> {
    let scenario = registry
        .get_scenario_revision(scenario_id, revision)
        .ok()??;
    (scenario.direction == "left_to_right"
        && matches!(registry.get(&scenario.left_connection_id), Ok(Some(connection)) if connection.adapter == ISPMANAGER))
        .then_some(scenario.left_connection_id)
}

/// Open the SFTP session of an ISPmanager connection with the login that the
/// app gave in memory and the host key that the user confirmed.
fn open_sftp(paths: &WorkerPaths, connection_id: &str) -> Result<SftpConnection, &'static str> {
    let (login, password) = runner()
        .ispmanager_login(connection_id)
        .ok_or("ispmanager_login_required")?;
    let registry = Registry::open(&paths.registry).map_err(|_| "registry_unavailable")?;
    let connection = registry
        .get(connection_id)
        .ok()
        .flatten()
        .filter(|connection| connection.enabled && connection.adapter == ISPMANAGER)
        .ok_or("ispmanager_connection_unavailable")?;
    let url = connection
        .endpoint_url
        .as_deref()
        .and_then(|url| qadren_core::ispmanager::PanelUrl::parse(url).ok())
        .ok_or("ispmanager_connection_unavailable")?;
    let settings = registry
        .ispmanager_settings(connection_id)
        .map_err(|_| "registry_unavailable")?;
    let host_key = settings
        .host_key_sha256
        .ok_or("ispmanager_host_key_unconfirmed")?;
    drop(registry);
    SftpConnection::open(url.host(), settings.ssh_port, &login, &password, &host_key).map_err(
        |error| {
            use qadren_core::ispmanager_sftp::SftpCheck;
            let code = match error {
                SftpCheck::HostKeyChanged { .. } | SftpCheck::HostKeyUnconfirmed { .. } => {
                    "ispmanager_host_key_changed"
                }
                SftpCheck::AuthRefused => "ispmanager_sign_in_refused",
                _ => return "ispmanager_network_error",
            };
            // A changed host key or a refused login needs the user: the
            // Connections widget shows it with a way to the fix.
            if let Ok(registry) = Registry::open(&paths.registry) {
                let _ =
                    registry.mark_checked(connection_id, "requires_auth", unix_now(), Some(code));
            }
            code
        },
    )
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
    drop(registry);
    // A dispatched schedule slot is a new run.
    wake_runs();
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
                Some(
                    "local_rule_trash"
                        | "local_rule_delete"
                        | "local_rule_trash_folder"
                        | "local_rule_delete_folder"
                )
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

/// The pauses between the checks of one UNKNOWN copy Job. An interrupted
/// copy that never published its file stays UNKNOWN for good; before, each
/// poll read its destination folder again (found on 2026-11-06). The first
/// check after a Worker start is immediate.
const RECONCILE_BACKOFF: [Duration; 5] = [
    Duration::from_secs(60),
    Duration::from_secs(5 * 60),
    Duration::from_secs(30 * 60),
    Duration::from_secs(2 * 60 * 60),
    Duration::from_secs(6 * 60 * 60),
];

/// For each UNKNOWN copy Job: the time of its next check and its step.
fn reconcile_backoff() -> &'static Mutex<HashMap<String, (std::time::Instant, usize)>> {
    static BACKOFF: OnceLock<Mutex<HashMap<String, (std::time::Instant, usize)>>> =
        OnceLock::new();
    BACKOFF.get_or_init(Default::default)
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
    let now = std::time::Instant::now();
    let mut backoff = reconcile_backoff()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    backoff.retain(|job_id, _| candidates.iter().any(|candidate| &candidate.0 == job_id));
    for (job_id, scenario_id, revision, destination_connection_id) in candidates {
        if backoff.get(&job_id).is_some_and(|(next, _)| now < *next) {
            continue;
        }
        // A Job that stays UNKNOWN is checked less often (D-028).
        let step = backoff.get(&job_id).map_or(0, |(_, step)| step + 1);
        let delay = RECONCILE_BACKOFF[step.min(RECONCILE_BACKOFF.len() - 1)];
        backoff.insert(job_id.clone(), (now + delay, step));
        // One unresolvable Job must not stop schedules or other checks. It
        // stays UNKNOWN and is checked again after its pause.
        let Ok(Some(scenario)) = registry.get_scenario_revision(&scenario_id, revision) else {
            continue;
        };
        // Both endpoints can use the same Local connection ("This Mac"). Try
        // each root of that connection. Reconciliation is read-only and checks
        // the durable root identity, so a wrong root stays StillUnknown.
        let mut roots = vec![
            (&scenario.left_connection_id, scenario.left_root.clone()),
            (&scenario.right_connection_id, scenario.right_root.clone()),
        ];
        // A Backup/export database dump is next to `files/`, in the copy root
        // (D-027 section 7b).
        if registry.scenario_mode(&scenario_id).ok().as_deref() == Some("backup_export") {
            if let Some(root) = Path::new(&scenario.right_root).parent() {
                roots.push((
                    &scenario.right_connection_id,
                    root.to_string_lossy().into_owned(),
                ));
            }
        }
        for (_, destination_root) in roots
            .iter()
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
        .list_tree_with_skipped(Path::new(""), MAX_RULE_DELETION_SCAN_ENTRIES)
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

/// Finish the schedule Job of a finished run. The Job stays `Queued` while
/// its run is active: the restart rule makes a `Running` Job `UNKNOWN`.
fn finish_schedule_job_for_run(
    jobs: &mut JobStore,
    job_id: &str,
    run: &ScenarioRun,
    unknown: bool,
) -> Result<(), String> {
    match run.state.as_str() {
        "completed" => finish_schedule_job(jobs, job_id, JobState::Completed),
        "cancelled" => jobs
            .advance(
                job_id,
                JobState::Queued,
                JobState::Cancelled,
                Confirmation::Unknown,
            )
            .map_err(|_| "schedule_job_state_unavailable".to_string()),
        _ if unknown => finish_schedule_job(jobs, job_id, JobState::Unknown),
        _ => finish_schedule_job(jobs, job_id, JobState::Failed),
    }
}

/// D-025: start a run for each dispatched schedule Job, then move each
/// active run one step. A manual run and a scheduled run use the same
/// snapshots, plan, and batch.
fn advance_scenario_runs(paths: &WorkerPaths) -> Result<(), String> {
    let registry =
        Registry::open(&paths.registry).map_err(|_| "registry_unavailable".to_string())?;
    let queued = JobStore::open(&paths.jobs)
        .map_err(|_| "job_store_unavailable".to_string())?
        .jobs_in_state(JobState::Queued)
        .map_err(|_| "job_store_unavailable".to_string())?;
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
        let start = registry
            .begin_schedule_run(&job.id)
            .map_err(|_| "schedule_registry_unavailable".to_string())?;
        let run_id = match start {
            ScheduleRunStart::Started(id) => id,
            // D-025 section 2: the active run continues; this slot ends
            // without a run, a conflict, or a follow-up run.
            ScheduleRunStart::Skipped(active) => {
                eprintln!(
                    "qadren-worker: {} schedule Job {} skipped: run {active} is active",
                    log_time(),
                    job.id
                );
                JobStore::open(&paths.jobs)
                    .map_err(|_| "job_store_unavailable".to_string())?
                    .advance(
                        &job.id,
                        JobState::Queued,
                        JobState::Cancelled,
                        Confirmation::Unknown,
                    )
                    .map_err(|_| "schedule_job_state_unavailable".to_string())?;
                continue;
            }
        };
        let run = registry
            .scenario_run(&run_id)
            .map_err(|_| "schedule_registry_unavailable".to_string())?
            .ok_or_else(|| "schedule_registry_unavailable".to_string())?;
        // A crash after the run finished and before the Job did.
        if run.finished_at.is_some() {
            let unknown = run.error.as_deref() == Some("items_unknown");
            let mut jobs =
                JobStore::open(&paths.jobs).map_err(|_| "job_store_unavailable".to_string())?;
            finish_schedule_job_for_run(&mut jobs, &job.id, &run, unknown)?;
        }
    }
    let runs = registry
        .active_scenario_runs()
        .map_err(|_| "registry_unavailable".to_string())?;
    drop(registry);
    for run in runs {
        // One run that cannot advance must not stop the others.
        if let Err(error) = advance_scenario_run(paths, &run) {
            eprintln!(
                "qadren-worker: {} run {} waits: {error}",
                log_time(),
                run.id
            );
        }
    }
    Ok(())
}

/// A request for the plan IPC functions, made by the Worker itself.
fn internal_request(command: &str, scenario: &ScenarioReference) -> Option<Request> {
    serde_json::from_value(serde_json::json!({
        "command": command,
        "scenario_id": scenario.scenario_id,
        "scenario_revision": scenario.revision,
    }))
    .ok()
}

/// Pauses between mount attempts while the server does not answer (D-024).
const SMB_RETRY_SECONDS: [u64; 5] = [60, 300, 900, 3600, 21600];

/// Mount each SMB endpoint of a Scenario revision that is not mounted now
/// (issue #32). Returns the waiting reason when an endpoint stays absent.
fn ensure_smb_mounts(
    paths: &WorkerPaths,
    registry: &Registry,
    scenario: &ScenarioReference,
) -> Option<&'static str> {
    use qadren_core::smb_mount::{self, MountOutcome};
    let revision = registry
        .get_scenario_revision(&scenario.scenario_id, scenario.revision)
        .ok()??;
    for id in [revision.left_connection_id, revision.right_connection_id] {
        let Ok(Some(connection)) = registry.get(&id) else {
            continue;
        };
        if connection.adapter != SMB_SHARE || !connection.enabled {
            continue;
        }
        let dir = qadren_core::storage_access::smb_mount_dir(&paths.registry, &id);
        if qadren_core::storage_access::is_smb_mount(&dir) {
            continue;
        }
        let address = connection.endpoint_url.clone().unwrap_or_default();
        if smb_mount::mounted_elsewhere(&address, &dir).is_some() {
            return Some("smb_mounted_elsewhere");
        }
        let now = std::time::Instant::now();
        let paused = runner()
            .smb_retry
            .lock()
            .ok()
            .and_then(|retry| retry.get(&id).copied());
        if paused.is_some_and(|(next, _)| now < next) {
            return Some("smb_offline");
        }
        let login = runner()
            .smb_logins
            .lock()
            .ok()
            .and_then(|logins| logins.get(&id).cloned());
        let Some((login, password)) = login else {
            let refused = runner()
                .smb_refused
                .lock()
                .is_ok_and(|refused| refused.contains_key(&id));
            return Some(if refused {
                "smb_login_refused"
            } else {
                "needs_sign_in"
            });
        };
        if std::fs::create_dir_all(&dir).is_err() {
            return Some("smb_mount_failed");
        }
        let outcome = smb_mount::mount(&address, &dir, &login, &password);
        eprintln!(
            "qadren-worker: {} smb_mount connection={id} outcome={}",
            log_time(),
            match &outcome {
                MountOutcome::Mounted => "mounted".to_string(),
                MountOutcome::MountedElsewhere(_) => "mounted_elsewhere".to_string(),
                MountOutcome::LoginRefused => "login_refused".to_string(),
                MountOutcome::Offline => "offline".to_string(),
                MountOutcome::NotFound => "not_found".to_string(),
                MountOutcome::Failed(code) => format!("failed({code})"),
            }
        );
        match outcome {
            MountOutcome::Mounted if qadren_core::storage_access::is_smb_mount(&dir) => {
                if let Ok(mut retry) = runner().smb_retry.lock() {
                    retry.remove(&id);
                }
            }
            MountOutcome::Mounted => return Some("smb_mount_failed"),
            MountOutcome::MountedElsewhere(_) => return Some("smb_mounted_elsewhere"),
            // A refused login is not tried again until the app gives new
            // credentials: no retry loop against the server.
            MountOutcome::LoginRefused => {
                if let (Ok(mut logins), Ok(mut refused)) =
                    (runner().smb_logins.lock(), runner().smb_refused.lock())
                {
                    logins.remove(&id);
                    refused.insert(id.clone(), (login, password));
                }
                return Some("smb_login_refused");
            }
            MountOutcome::Offline => {
                if let Ok(mut retry) = runner().smb_retry.lock() {
                    let step = paused.map_or(0, |(_, step)| step + 1);
                    let pause = SMB_RETRY_SECONDS[(step as usize).min(SMB_RETRY_SECONDS.len() - 1)];
                    retry.insert(id.clone(), (now + Duration::from_secs(pause), step));
                }
                return Some("smb_offline");
            }
            MountOutcome::NotFound | MountOutcome::Failed(_) => return Some("smb_mount_failed"),
        }
    }
    None
}

fn advance_scenario_run(paths: &WorkerPaths, run: &ScenarioRun) -> Result<(), String> {
    let registry =
        Registry::open(&paths.registry).map_err(|_| "registry_unavailable".to_string())?;
    // An SMB endpoint is mounted before a plan or an effect. While it stays
    // absent the run waits; the Core mount gate still checks each effect.
    if let Some(reason) = ensure_smb_mounts(paths, &registry, &run.scenario) {
        return registry
            .set_scenario_run_waiting(&run.id, Some(reason))
            .map_err(|_| "registry_unavailable".to_string());
    }
    // D-025 section 11: a run whose roots overlap the write root of another
    // active run waits until that run ends. It does not plan in the meantime.
    match registry.acquire_scenario_run_write_lock(&run.id) {
        Ok(RunWriteLock::Acquired) => {}
        Ok(RunWriteLock::WaitFor { .. }) => {
            return registry
                .set_scenario_run_waiting(&run.id, Some("waiting_for_scenario"))
                .map_err(|_| "registry_unavailable".to_string());
        }
        Err(_) => return Err("registry_unavailable".to_string()),
    }
    // A Yandex Disk run without a token from the app waits (D-024 stage A),
    // and a Backup/export run without its ISPmanager login (D-027).
    let needs_sign_in =
        yandex_source_connection(&registry, &run.scenario.scenario_id, run.scenario.revision)
            .is_some_and(|connection| runner().token(&connection).is_none())
            || ispmanager_source_connection(
                &registry,
                &run.scenario.scenario_id,
                run.scenario.revision,
            )
            .is_some_and(|connection| runner().ispmanager_login(&connection).is_none());
    let finished = match run.state.as_str() {
        "planning" => advance_planning_run(paths, &registry, run, needs_sign_in)?,
        "executing" => advance_executing_run(paths, &registry, run, needs_sign_in)?,
        _ => false,
    };
    drop(registry);
    if finished {
        write_backup_manifest(paths, &run.id);
        finish_run_job(paths, &run.id)?;
    }
    Ok(())
}

/// Write `manifest.json` into the root of a Backup/export copy after a run
/// (D-027 section 5.6). It replaces the previous manifest atomically and
/// does not go into the Trash: it describes the last run only. A failure
/// leaves the previous manifest and does not change the run result.
fn write_backup_manifest(paths: &WorkerPaths, run_id: &str) {
    let Ok(registry) = Registry::open(&paths.registry) else {
        return;
    };
    let Ok(Some(run)) = registry.scenario_run(run_id) else {
        return;
    };
    let (id, revision) = (&run.scenario.scenario_id, run.scenario.revision);
    if registry.scenario_mode(id).ok().as_deref() != Some("backup_export") {
        return;
    }
    let Ok(Some(scenario)) = registry.get_scenario_revision(id, revision) else {
        return;
    };
    if !qadren_core::storage_access::connection_id_ready(
        &registry,
        &paths.registry,
        &scenario.right_connection_id,
    ) {
        return;
    }
    let Some(root) = Path::new(&scenario.right_root).parent() else {
        return;
    };
    let resources = registry.backup_resources(id, revision).unwrap_or_default();
    let summary = run
        .batch_id
        .as_deref()
        .and_then(|batch| {
            registry
                .scenario_create_batch_summary(batch, unix_now())
                .ok()
        })
        .flatten();
    let result = match (run.state.as_str(), &summary) {
        ("completed", Some(summary)) if summary.failed + summary.unknown == 0 => "complete",
        ("completed", None) => "complete",
        ("cancelled", _) => "cancelled",
        (_, Some(summary)) if summary.completed > 0 => "partial",
        _ => "failed",
    };
    let finished_at = chrono::DateTime::<chrono::Utc>::from_timestamp(
        run.finished_at.unwrap_or_else(unix_now),
        0,
    )
    .map(|time| time.to_rfc3339());
    let selected_databases: Vec<_> = resources
        .iter()
        .filter(|resource| resource.category == "databases")
        .collect();
    let database_results = registry
        .backup_database_results(&run.id)
        .unwrap_or_default();
    let mut manifest = serde_json::json!({
        "format": "qadren-ispmanager-backup/1",
        "scenario_id": id,
        "scenario_revision": revision,
        "run_id": run.id,
        "trigger": run.trigger,
        "result": result,
        "error": run.error,
        "finished_at": finished_at,
        "categories": {
            "files": {
                "folder": "files",
                "artifact": "file",
                "transport": "sftp",
                "integrity": "byte count and MD5 checked before publish",
                "selected": resources
                    .iter()
                    .filter(|resource| resource.category == "files")
                    .collect::<Vec<_>>(),
                "items": summary.as_ref().map(|summary| serde_json::json!({
                    "total": summary.total,
                    "completed": summary.completed,
                    "failed": summary.failed,
                    "unknown": summary.unknown,
                    "cancelled": summary.cancelled,
                    "not_executed": run.deferred_items,
                })),
                "omissions": "symbolic links and special files are not copied",
            },
        },
    });
    if !selected_databases.is_empty() {
        manifest["categories"]["databases"] = serde_json::json!({
            "folder": "databases",
            "artifact": "database dump",
            "format": "mysqldump SQL, not compressed (<database>.sql)",
            "transport": "mysqldump over SSH, through gzip -1 when the server has gzip; access facts from the panel",
            "structure_only": "a selected table with rule structure_only has its structure and no data",
            "consistency": "one transaction for InnoDB tables; no table lock",
            "integrity": "exit status 0 and the final dump line checked before publish; md5 of the .sql file",
            "selected": selected_databases,
            "results": database_results,
        });
    }
    let Ok(text) = serde_json::to_vec_pretty(&manifest) else {
        return;
    };
    let staging = root.join(format!(".qadren-manifest-{}.tmp", std::process::id()));
    let written = (|| -> std::io::Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&staging)?;
        file.write_all(&text)?;
        file.sync_all()?;
        fs::rename(&staging, root.join("manifest.json"))
    })();
    if written.is_err() {
        let _ = fs::remove_file(&staging);
        eprintln!(
            "qadren-worker: {} manifest of run {run_id} not written",
            log_time()
        );
    }
}

/// True for a Backup/export revision without a selected file or folder.
fn backup_without_files(registry: &Registry, run: &ScenarioRun) -> bool {
    let (id, revision) = (&run.scenario.scenario_id, run.scenario.revision);
    registry.scenario_mode(id).ok().as_deref() == Some("backup_export")
        && registry
            .backup_resources(id, revision)
            .is_ok_and(|resources| {
                !resources
                    .iter()
                    .any(|r| r.category == "files" && r.rule == "include")
            })
}

/// Request the plan of a planning run, or start the batch of its complete
/// plan. Returns true when the run finished.
fn advance_planning_run(
    paths: &WorkerPaths,
    registry: &Registry,
    run: &ScenarioRun,
    needs_sign_in: bool,
) -> Result<bool, String> {
    // A newer revision was saved: this run cannot execute, so it ends now
    // and does not walk for a plan that it would refuse (found on 2026-11-04).
    if !registry
        .is_current_scenario_revision(&run.scenario.scenario_id, run.scenario.revision)
        .map_err(|_| "registry_unavailable".to_string())?
    {
        return finish_run(registry, run, "failed", Some("scenario_revision_stale"));
    }
    // A Backup/export revision with databases and no files has no file plan:
    // an empty file selection would mean the whole home folder.
    if backup_without_files(registry, run) {
        return match advance_backup_databases(paths, registry, run)? {
            DatabaseStep::Running | DatabaseStep::None if needs_sign_in => registry
                .set_scenario_run_waiting(&run.id, Some("needs_sign_in"))
                .map(|_| false)
                .map_err(|_| "registry_unavailable".to_string()),
            DatabaseStep::Running => Ok(false),
            DatabaseStep::None => finish_run(registry, run, "completed", None),
            DatabaseStep::Done { failed: true, .. } => {
                finish_run(registry, run, "failed", Some("databases_failed"))
            }
            DatabaseStep::Done {
                cancelled: true, ..
            } => finish_run(registry, run, "cancelled", None),
            DatabaseStep::Done { .. }
                if cleanup_failures()
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .remove(&run.id) =>
            {
                finish_run(registry, run, "failed", Some("files_cleanup_failed"))
            }
            DatabaseStep::Done { .. } => finish_run(registry, run, "completed", None),
        };
    }
    let Some(plan_id) = run.plan_id.as_deref() else {
        let request = internal_request("start_plan", &run.scenario)
            .ok_or_else(|| "invalid_request".to_string())?;
        let response = start_plan(paths, &request);
        return match (response["plan"].as_str(), response["status"].as_str()) {
            (Some(plan), _) => registry
                .set_scenario_run_plan(&run.id, Some(plan))
                .map(|_| false)
                .map_err(|_| "registry_unavailable".to_string()),
            (None, Some("scenario_unavailable")) => {
                finish_run(registry, run, "failed", Some("scenario_unavailable"))
            }
            (None, status) => Err(status.unwrap_or("plan_unavailable").to_string()),
        };
    };
    let store = SnapshotStore::open_for_reading(&SnapshotStore::path_for(&paths.registry))
        .map_err(|_| "snapshot_store_unavailable".to_string())?;
    let info = plan_store::plan_info(store.connection(), plan_id)
        .map_err(|_| "snapshot_store_unavailable".to_string())?;
    drop(store);
    match info {
        None => registry.set_scenario_run_plan(&run.id, None),
        Some(info) if info.state == "complete" => {
            return match batch_from_plan(paths, plan_id) {
                Ok((_, batch, _, deferred)) => {
                    registry
                        .set_scenario_run_executing(&run.id, &batch, deferred)
                        .and_then(|_| registry.set_scenario_create_batch_paused(&batch, false))
                        .map_err(|_| "registry_unavailable".to_string())?;
                    Ok(false)
                }
                Err("scenario_revision_stale") => {
                    finish_run(registry, run, "failed", Some("scenario_revision_stale"))
                }
                Err(status) => Err(status.to_string()),
            };
        }
        // A newer plan of the same revision replaced this one; follow it.
        Some(info) if info.state == "failed" && info.error.as_deref() == Some("superseded") => {
            registry.set_scenario_run_plan(&run.id, None)
        }
        Some(info) if info.state == "failed" => {
            return finish_run(registry, run, "failed", Some("plan_failed"));
        }
        Some(_) => {
            registry.set_scenario_run_waiting(&run.id, needs_sign_in.then_some("needs_sign_in"))
        }
    }
    .map(|_| false)
    .map_err(|_| "registry_unavailable".to_string())
}

/// Finish an executing run when its batch has no pending child. Returns
/// true when the run finished.
fn advance_executing_run(
    paths: &WorkerPaths,
    registry: &Registry,
    run: &ScenarioRun,
    needs_sign_in: bool,
) -> Result<bool, String> {
    let summary = run
        .batch_id
        .as_deref()
        .map(|batch| registry.scenario_create_batch_summary(batch, unix_now()))
        .transpose()
        .map_err(|_| "registry_unavailable".to_string())?
        .flatten()
        .ok_or_else(|| "batch_unavailable".to_string())?;
    if summary.due + summary.retry_waiting > 0 {
        registry
            .set_scenario_run_waiting(&run.id, needs_sign_in.then_some("needs_sign_in"))
            .map_err(|_| "registry_unavailable".to_string())?;
        return Ok(false);
    }
    let (state, error) = if summary.unknown > 0 {
        ("failed", Some("items_unknown"))
    } else if summary.failed > 0 {
        ("failed", Some("items_failed"))
    } else if summary.cancelled > 0 || summary.cancellation_requested {
        ("cancelled", None)
    } else {
        ("completed", None)
    };
    // The databases of a Backup/export run follow its files (D-027 section
    // 7b). A stopped run starts no dump.
    if state != "cancelled" {
        match advance_backup_databases(paths, registry, run)? {
            DatabaseStep::Running => {
                registry
                    .set_scenario_run_waiting(&run.id, None)
                    .map_err(|_| "registry_unavailable".to_string())?;
                return Ok(false);
            }
            DatabaseStep::Done { failed: true, .. } if state == "completed" => {
                return finish_run(registry, run, "failed", Some("databases_failed"));
            }
            DatabaseStep::Done {
                cancelled: true, ..
            } if state == "completed" => {
                return finish_run(registry, run, "cancelled", None);
            }
            _ => {}
        }
    }
    finish_run(registry, run, state, error)
}

enum DatabaseStep {
    None,
    Running,
    Done { failed: bool, cancelled: bool },
}

fn run_wake() -> &'static (Mutex<bool>, std::sync::Condvar) {
    static WAKE: OnceLock<(Mutex<bool>, std::sync::Condvar)> = OnceLock::new();
    WAKE.get_or_init(Default::default)
}

/// Ask the run thread to advance the Scenario runs now, for example when a
/// run starts or the database export of a run ends. Without it a run waited
/// up to one poll pass (found on 2026-11-06).
fn wake_runs() {
    let (woken, signal) = run_wake();
    *woken.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = true;
    signal.notify_all();
}

/// Wait for `wake_runs` until `deadline`. True when it was called.
fn wait_for_run_wake(deadline: std::time::Instant) -> bool {
    let (woken, signal) = run_wake();
    let mut flag = woken.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    while !*flag {
        let Some(left) = deadline.checked_duration_since(std::time::Instant::now()) else {
            return false;
        };
        flag = signal
            .wait_timeout(flag, left)
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .0;
    }
    *flag = false;
    true
}

/// What the database step of an active run does now, for the Scenario
/// widget. It lives in memory: after a Worker restart the step starts again.
#[derive(Debug, Clone, Serialize)]
struct DatabaseProgress {
    scenario_id: String,
    run_id: String,
    /// `cleanup` of `files/`, `dump`, or `publish`.
    stage: &'static str,
    /// The current database, from 1; 0 during the cleanup.
    index: usize,
    total: usize,
    /// SQL bytes of the current dump so far.
    bytes: u64,
}

fn database_progress() -> &'static Mutex<HashMap<String, DatabaseProgress>> {
    static PROGRESS: OnceLock<Mutex<HashMap<String, DatabaseProgress>>> = OnceLock::new();
    PROGRESS.get_or_init(Default::default)
}

fn set_database_progress(run: &ScenarioRun, stage: &'static str, index: usize, total: usize, bytes: u64) {
    database_progress()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(
            run.id.clone(),
            DatabaseProgress {
                scenario_id: run.scenario.scenario_id.clone(),
                run_id: run.id.clone(),
                stage,
                index,
                total,
                bytes,
            },
        );
}

/// The database step of the active run of one Scenario, if any.
fn database_progress_status(request: &Request) -> serde_json::Value {
    let Some(scenario_id) = request.scenario_id.as_deref() else {
        return serde_json::json!({"ok": false, "status": "invalid_request"});
    };
    let progress = database_progress()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .values()
        .find(|progress| progress.scenario_id == scenario_id)
        .cloned();
    serde_json::json!({"ok": true, "progress": progress})
}

/// Runs whose `files/` cleanup failed in this Worker process.
fn cleanup_failures() -> &'static Mutex<std::collections::HashSet<String>> {
    static RUNS: OnceLock<Mutex<std::collections::HashSet<String>>> = OnceLock::new();
    RUNS.get_or_init(Default::default)
}

/// The `files/` copy of a revision without a file rule is a mirror of an
/// empty selection: every object in it is outside the source scope, so it
/// is deleted (D-027 section 5.6a, owner decision 2026-11-06). Each top
/// folder goes into the Trash as one entry, each top file as one file. No
/// server check is needed for a path outside the scope. Executor files,
/// links, and special files stay. Only the simple deletion rule deletes.
/// Returns false when an effect failed or its outcome is unknown.
fn clean_backup_files(paths: &WorkerPaths, run: &ScenarioRun) -> bool {
    let (id, revision) = (&run.scenario.scenario_id, run.scenario.revision);
    let Ok(registry) = Registry::open(&paths.registry) else {
        return false;
    };
    let simple = registry
        .get_scenario_revision_scope(id, revision)
        .is_ok_and(|scope| scope.deletion_mode == qadren_core::model::DeletionMode::Simple);
    let Ok(Some(scenario)) = registry.get_scenario_revision(id, revision) else {
        return false;
    };
    drop(registry);
    if !simple {
        return true;
    }
    let root = Path::new(&scenario.right_root);
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        // No `files/` yet: nothing to delete.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return true,
        Err(_) => return false,
    };
    let mut clean = true;
    for entry in entries {
        let Ok(entry) = entry else {
            clean = false;
            continue;
        };
        let Some(name) = entry.file_name().to_str().map(ToOwned::to_owned) else {
            clean = false;
            continue;
        };
        if name.starts_with(".qadren-") {
            continue;
        }
        let Ok(kind) = entry.file_type() else {
            clean = false;
            continue;
        };
        if !kind.is_dir() && !kind.is_file() {
            continue;
        }
        let digest = format!("{:x}", md5::compute(name.as_bytes()));
        let job_id = format!("ispclean-{}-{}", run.id, &digest[..16]);
        let outcome = if kind.is_dir() {
            qadren_core::job_execution::run_backup_mirror_delete_folder(
                &paths.jobs,
                &paths.registry,
                id,
                revision,
                &name,
                &job_id,
            )
        } else {
            qadren_core::job_execution::run_backup_mirror_delete(
                &paths.jobs,
                &paths.registry,
                id,
                revision,
                &name,
                &job_id,
            )
        };
        let ok = matches!(
            outcome,
            Ok(qadren_core::job_execution::LocalRuleDeleteStatus::MovedToTrash
                | qadren_core::job_execution::LocalRuleDeleteStatus::DeletedPermanently
                | qadren_core::job_execution::LocalRuleDeleteStatus::Existing(JobState::Completed))
        );
        if !ok {
            eprintln!(
                "qadren-worker: {} run {} files cleanup {}: {:?}",
                log_time(),
                run.id,
                if kind.is_dir() { "folder" } else { "file" },
                outcome.err()
            );
        }
        clean &= ok;
    }
    clean
}

/// Runs of which the databases are being exported now. A run has at most
/// one export thread.
fn database_runs() -> &'static Mutex<std::collections::HashSet<String>> {
    static RUNS: OnceLock<Mutex<std::collections::HashSet<String>>> = OnceLock::new();
    RUNS.get_or_init(Default::default)
}

/// The selected databases of a Backup/export run and their state. It starts
/// one export thread for the databases without a result.
fn advance_backup_databases(
    paths: &WorkerPaths,
    registry: &Registry,
    run: &ScenarioRun,
) -> Result<DatabaseStep, String> {
    let (id, revision) = (&run.scenario.scenario_id, run.scenario.revision);
    if registry.scenario_mode(id).ok().as_deref() != Some("backup_export") {
        return Ok(DatabaseStep::None);
    }
    let databases: Vec<String> = registry
        .backup_resources(id, revision)
        .map_err(|_| "registry_unavailable".to_string())?
        .into_iter()
        .filter(|r| r.category == "databases" && r.rule == "include")
        .map(|r| r.provider_id)
        .collect();
    if databases.is_empty() {
        return Ok(DatabaseStep::None);
    }
    let results = registry
        .backup_database_results(&run.id)
        .map_err(|_| "registry_unavailable".to_string())?;
    let missing: Vec<String> = databases
        .iter()
        .filter(|database| {
            !results
                .iter()
                .any(|result| &result.provider_id == *database)
        })
        .cloned()
        .collect();
    if missing.is_empty() {
        return Ok(DatabaseStep::Done {
            failed: results.iter().any(|result| result.result == "failed"),
            cancelled: results.iter().any(|result| result.result == "cancelled"),
        });
    }
    let Some(connection_id) = ispmanager_source_connection(registry, id, revision) else {
        return Err("ispmanager_connection_unavailable".to_string());
    };
    if !database_runs()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(run.id.clone())
    {
        return Ok(DatabaseStep::Running);
    }
    let paths = paths.clone();
    let run = run.clone();
    let clean_files = backup_without_files(registry, &run);
    thread::spawn(move || {
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if clean_files {
                set_database_progress(&run, "cleanup", 0, missing.len(), 0);
            }
            if clean_files && !clean_backup_files(&paths, &run) {
                cleanup_failures()
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .insert(run.id.clone());
            }
            export_run_databases(&paths, &run, &connection_id, &missing)
        }));
        if outcome.is_err() {
            eprintln!(
                "qadren-worker: {} database export of run {} panicked",
                log_time(),
                run.id
            );
        }
        database_runs()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&run.id);
        database_progress()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&run.id);
        wake_runs();
    });
    Ok(DatabaseStep::Running)
}

/// True while the run is active and no stop was requested.
fn run_still_active(paths: &WorkerPaths, run: &ScenarioRun) -> bool {
    let Ok(registry) = Registry::open(&paths.registry) else {
        return true;
    };
    let Ok(Some(current)) = registry.scenario_run(&run.id) else {
        return false;
    };
    if !matches!(current.state.as_str(), "planning" | "executing") {
        return false;
    }
    !current.batch_id.as_deref().is_some_and(|batch| {
        matches!(
            registry.scenario_create_batch_summary(batch, unix_now()),
            Ok(Some(summary)) if summary.cancellation_requested
        )
    })
}

/// Export the databases of one run, one after another. Each database gets
/// a durable result. The access facts come from the panel for each
/// database and stay in memory.
fn export_run_databases(
    paths: &WorkerPaths,
    run: &ScenarioRun,
    connection_id: &str,
    databases: &[String],
) {
    let record =
        |database: &str, result: &str, error: Option<&str>, done: Option<(u64, String)>| {
            if let Ok(registry) = Registry::open(&paths.registry) {
                let (bytes, md5) =
                    done.map_or((None, None), |(bytes, md5)| (Some(bytes), Some(md5)));
                let _ = registry.record_backup_database_result(
                    &run.id,
                    &BackupDatabaseResult {
                        provider_id: database.to_owned(),
                        result: result.to_owned(),
                        error: error.map(str::to_owned),
                        bytes,
                        md5,
                        finished_at: unix_now(),
                    },
                );
            }
            eprintln!(
                "qadren-worker: {} run {} database export {result}{}",
                log_time(),
                run.id,
                error.map_or(String::new(), |error| format!(": {error}"))
            );
        };
    let fail_all = |code: &str| {
        for database in databases {
            record(database, "failed", Some(code), None);
        }
    };
    let Some((login, password)) = runner().ispmanager_login(connection_id) else {
        // The run waits for the login; the next advance starts again.
        return;
    };
    let url = Registry::open(&paths.registry)
        .ok()
        .and_then(|registry| registry.get(connection_id).ok().flatten())
        .and_then(|connection| connection.endpoint_url)
        .and_then(|url| qadren_core::ispmanager::PanelUrl::parse(&url).ok());
    let Some(url) = url else {
        return fail_all("ispmanager_connection_unavailable");
    };
    let Ok(mut session) = qadren_core::ispmanager::Session::new(url, login, password) else {
        return fail_all("ispmanager_network_error");
    };
    let connected = std::time::Instant::now();
    let connection = match open_sftp(paths, connection_id) {
        Ok(connection) => connection,
        Err(code) => return fail_all(code),
    };
    eprintln!(
        "qadren-worker: {} run {} database export ssh sign-in {:.1}s",
        log_time(),
        run.id,
        connected.elapsed().as_secs_f64()
    );
    let staging = paths
        .registry
        .parent()
        .unwrap_or(Path::new("."))
        .join("database-staging");
    let checked = Mutex::new(std::time::Instant::now());
    let keep_going = || {
        let mut last = checked
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if last.elapsed() < Duration::from_secs(5) {
            return true;
        }
        *last = std::time::Instant::now();
        run_still_active(paths, run)
    };
    for (index, database) in databases.iter().enumerate() {
        set_database_progress(run, "dump", index + 1, databases.len(), 0);
        if !run_still_active(paths, run) {
            record(database, "cancelled", Some("stopped_by_user"), None);
            continue;
        }
        let asked = std::time::Instant::now();
        let access = match qadren_core::ispmanager::mysql_access(&mut session, database) {
            Ok(access) => access,
            Err(code) => {
                record(database, "failed", Some(code), None);
                continue;
            }
        };
        let access_time = asked.elapsed();
        let mut timing = qadren_core::ispmanager_execution::DatabaseExportTiming::default();
        let status = run_ispmanager_database_export(
            &paths.jobs,
            &paths.registry,
            &run.scenario.scenario_id,
            run.scenario.revision,
            &run.id,
            &connection,
            &access,
            &staging,
            &|step| match step {
                qadren_core::ispmanager_execution::ExportProgress::Dump { bytes } => {
                    // Each chunk only changes the byte count.
                    if let Some(progress) = database_progress()
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .get_mut(&run.id)
                    {
                        progress.bytes = bytes;
                    }
                    keep_going()
                }
                qadren_core::ispmanager_execution::ExportProgress::Publish => {
                    set_database_progress(run, "publish", index + 1, databases.len(), 0);
                    true
                }
            },
            &mut timing,
        );
        drop(access);
        // The stages of one export, without the database name (D-028).
        eprintln!(
            "qadren-worker: {} run {} database export timing: panel access {:.1}s, dump {:.1}s, \
             sql {} bytes, network {} bytes, gzip {}, compare {:.1}s, publish {:.1}s",
            log_time(),
            run.id,
            access_time.as_secs_f64(),
            timing.dump.as_secs_f64(),
            timing.sql_bytes,
            timing.network_bytes,
            if timing.compressed { "on" } else { "off" },
            timing.compare.as_secs_f64(),
            timing.publish.as_secs_f64()
        );
        match status {
            Ok(DatabaseExportStatus::Created { bytes, md5 }) => {
                record(database, "created", None, Some((bytes, md5)))
            }
            Ok(DatabaseExportStatus::Updated { bytes, md5 }) => {
                record(database, "updated", None, Some((bytes, md5)))
            }
            Ok(DatabaseExportStatus::Unchanged { bytes, md5 }) => {
                record(database, "unchanged", None, Some((bytes, md5)))
            }
            Ok(DatabaseExportStatus::Existing(JobState::Completed)) => {
                record(database, "updated", None, None)
            }
            Ok(DatabaseExportStatus::Existing(_)) => {
                record(database, "failed", Some("items_unknown"), None)
            }
            Err("stopped_by_user") => record(database, "cancelled", Some("stopped_by_user"), None),
            Err(code) => record(database, "failed", Some(code), None),
        }
    }
}

fn finish_run(
    registry: &Registry,
    run: &ScenarioRun,
    state: &str,
    error: Option<&str>,
) -> Result<bool, String> {
    eprintln!(
        "qadren-worker: {} run {} {state}{}",
        log_time(),
        run.id,
        error.map_or(String::new(), |error| format!(": {error}"))
    );
    registry
        .finish_scenario_run(&run.id, state, error)
        .map(|_| true)
        .map_err(|_| "registry_unavailable".to_string())
}

/// After a run finished, finish its schedule Job. A manual run has none.
fn finish_run_job(paths: &WorkerPaths, run_id: &str) -> Result<(), String> {
    let registry =
        Registry::open(&paths.registry).map_err(|_| "registry_unavailable".to_string())?;
    let Some(run) = registry
        .scenario_run(run_id)
        .map_err(|_| "registry_unavailable".to_string())?
    else {
        return Ok(());
    };
    let Some(job_id) = run.schedule_job_id.as_deref() else {
        return Ok(());
    };
    let mut jobs = JobStore::open(&paths.jobs).map_err(|_| "job_store_unavailable".to_string())?;
    if jobs
        .state(job_id)
        .map_err(|_| "job_store_unavailable".to_string())?
        != Some(JobState::Queued)
    {
        return Ok(());
    }
    let unknown = run.error.as_deref() == Some("items_unknown");
    finish_schedule_job_for_run(&mut jobs, job_id, &run, unknown)
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
    let scope = registry
        .get_scenario_revision_scope(scenario_id, revision)
        .ok()?;
    let (selection, exclusions) = (scope.selected_paths, scope.excluded_paths);
    // The `files/` destination of a Backup/export Scenario is a mirror of
    // the source scope: its snapshot covers the whole folder, so an object
    // outside the selection or inside an exclusion becomes a deletion
    // (D-027 section 5.6a).
    let mirror = registry.scenario_mode(scenario_id).ok().as_deref() == Some("backup_export");
    let key = |connection_id: &str, root: &str, whole: bool| {
        let adapter = registry.get(connection_id).ok()??.adapter;
        Some(SnapshotKey {
            connection_id: connection_id.to_owned(),
            kind: if adapter == YANDEX_DISK || adapter == ISPMANAGER {
                adapter
            } else {
                "local".to_owned()
            },
            root: root.to_owned(),
            selection: if whole { vec![] } else { selection.clone() },
            exclusions: if whole { vec![] } else { exclusions.clone() },
        })
    };
    Some((
        key(&scenario.left_connection_id, &scenario.left_root, false)?,
        key(&scenario.right_connection_id, &scenario.right_root, mirror)?,
    ))
}

/// The snapshot thread: continue every walking snapshot until the Worker
/// exits (D-024 stage 3).
fn run_snapshots_forever(paths: WorkerPaths) {
    loop {
        // A panic in one pass must not stop every later walk (found on
        // 2026-11-04: one SFTP panic stopped all snapshots).
        let worked =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run_snapshot_pass(&paths)))
                .unwrap_or_else(|_| {
                    eprintln!("qadren-worker: {} snapshot pass panicked", log_time());
                    false
                });
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
        let result = if snapshot.key.kind == ISPMANAGER {
            // Without a login from the app the walk waits.
            if runner()
                .ispmanager_login(&snapshot.key.connection_id)
                .is_none()
            {
                continue;
            }
            match runner().checkout_sftp(paths, &snapshot.key.connection_id) {
                Ok(connection) => {
                    let result = walk_sftp_step(&store, &connection, &snapshot.key, &snapshot.id);
                    let healthy = !matches!(result, Err(SnapshotError::Transient(_)));
                    runner().checkin_sftp(&snapshot.key.connection_id, connection, healthy);
                    result
                }
                Err(code @ ("ispmanager_host_key_unconfirmed" | "ispmanager_host_key_changed")) => {
                    Err(SnapshotError::Blocked(code.into()))
                }
                Err(code) => Err(SnapshotError::Transient(code.into())),
            }
        } else if snapshot.key.kind == YANDEX_DISK {
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
        if let Ok(registry) = Registry::open(&paths.registry) {
            let _ = plan_store::remove_plans_of_removed_scenarios(store.connection(), &registry);
            // Snapshots of old revisions and removed Scenarios only take space.
            if let Ok(scenarios) = registry.list_scenarios() {
                let current: std::collections::HashSet<String> = scenarios
                    .iter()
                    .filter_map(|scenario| {
                        snapshot_keys(&registry, &scenario.id, scenario.revision)
                    })
                    .flat_map(|(left, right)| [left.fingerprint(), right.fingerprint()])
                    .collect();
                if !current.is_empty() {
                    let _ = store.prune_unused_keys(&current);
                }
            }
        }
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
    // A plan that already waits or is being built is the answer: a second
    // request must not start more walks (found on 2026-10-25).
    if let (Some(scenario_id), Some(revision), Ok(store)) = (
        request.scenario_id.as_deref(),
        request.scenario_revision,
        SnapshotStore::open(&SnapshotStore::path_for(&paths.registry)),
    ) {
        let reference = ScenarioReference {
            scenario_id: scenario_id.to_owned(),
            revision,
        };
        if let Ok(Some(plan)) = plan_store::active_plan(store.connection(), &reference) {
            let connection = Registry::open(&paths.registry)
                .ok()
                .and_then(|registry| yandex_source_connection(&registry, scenario_id, revision));
            if let (Some(connection), Some(AccessToken(token))) =
                (connection, request.access_token.as_ref())
            {
                runner().remember_token(&connection, token);
            }
            return serde_json::json!({"ok": true, "status": "accepted", "plan": plan});
        }
    }
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
    // A revision made before the carry-over fix has no confirmed states.
    if let Ok(registry) = Registry::open(&paths.registry) {
        let _ = registry.carry_forward_confirmed_states(scenario_id, revision);
    }
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
    // A screen follows its own plan by ID; without one, the newest plan.
    let plan = match request.plan_id.as_deref() {
        Some(id) => plan_store::plan_info(store.connection(), id).ok().flatten(),
        None => plan_store::latest_plan(store.connection(), &reference)
            .ok()
            .flatten(),
    };
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

/// Make one paused Create batch of all Create rows of a complete plan, page
/// by page. The caller records the run and then resumes the batch, so a
/// crash in between leaves a paused batch that never runs.
/// Make a paused batch of the executable rows of a complete plan: Create,
/// and Update and Delete when the run may execute them (D-025). Returns the
/// Scenario, the batch, the number of children, and the number of plan
/// items that the run does not execute.
fn batch_from_plan(
    paths: &WorkerPaths,
    plan: &str,
) -> Result<(ScenarioReference, String, usize, u64), &'static str> {
    let store = SnapshotStore::open_for_reading(&SnapshotStore::path_for(&paths.registry))
        .map_err(|_| "snapshot_store_unavailable")?;
    let Ok(Some(info)) = plan_store::plan_info(store.connection(), plan) else {
        return Err("plan_unavailable");
    };
    if info.state != "complete" {
        return Err("plan_unavailable");
    }
    let reference: ScenarioReference = store
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
        .map_err(|_| "plan_unavailable")?;
    let mut registry = Registry::open(&paths.registry).map_err(|_| "registry_unavailable")?;
    let actions = executable_actions(&registry, &reference)?;
    let batch = registry
        .begin_scenario_create_batch(&reference)
        .map_err(|_| "scenario_revision_stale")?;
    let mut after = 0;
    let mut added = 0;
    loop {
        let (page, last) =
            plan_store::plan_effects_page(store.connection(), plan, &actions, after, 5_000)
                .map_err(|_| "snapshot_store_unavailable")?;
        let Some(last) = last else { break };
        after = last;
        let children: Vec<(String, bool, BatchChildKind)> = page
            .into_iter()
            .filter_map(|(path, action, source_is_left)| {
                BatchChildKind::parse(&action).map(|kind| (path, source_is_left, kind))
            })
            .collect();
        added += registry
            .append_scenario_batch_children(&batch, &children)
            .map_err(|_| "registry_unavailable")?;
    }
    let planned: u64 = info
        .counts
        .as_ref()
        .and_then(|counts| counts.as_object())
        .map_or(0, |counts| {
            counts
                .iter()
                .filter(|(action, _)| action.as_str() != "no_action")
                .filter_map(|(_, value)| value["count"].as_u64())
                .sum()
        });
    Ok((
        reference,
        batch,
        added,
        planned.saturating_sub(added as u64),
    ))
}

/// The plan actions that a run of this revision executes. A one-way Update
/// or Delete replaces or removes data in the destination, so it needs the
/// one-time confirmation of "the source always wins" (D-025 section 4).
/// Without it these items stay in the plan and the run does not execute
/// them.
fn executable_actions(
    registry: &Registry,
    reference: &ScenarioReference,
) -> Result<Vec<&'static str>, &'static str> {
    let scenario = registry
        .get_scenario_revision(&reference.scenario_id, reference.revision)
        .ok()
        .flatten()
        .ok_or("scenario_unavailable")?;
    let confirmed = scenario.direction == "bidirectional"
        || registry
            .scenario_source_wins_confirmed(&reference.scenario_id, reference.revision)
            .map_err(|_| "registry_unavailable")?;
    Ok(if confirmed {
        vec!["create", "update", "delete", "delete_folder"]
    } else {
        vec!["create"]
    })
}

/// The Scenario of a stored plan.
fn plan_scenario_id(paths: &WorkerPaths, plan: &str) -> Option<String> {
    let store = SnapshotStore::open_for_reading(&SnapshotStore::path_for(&paths.registry)).ok()?;
    store
        .connection()
        .query_row("SELECT scenario_id FROM plans WHERE id=?1", [plan], |row| {
            row.get(0)
        })
        .ok()
}

fn run_active_answer(run: &ScenarioRun) -> serde_json::Value {
    serde_json::json!({
        "ok": false,
        "status": "run_active",
        "run": run.id,
        "state": run.state,
        "batch": run.batch_id,
    })
}

/// "Создать все" of a plan screen: a manual run of all Create rows of a
/// complete plan (D-024 stage 4, D-025).
fn execute_plan_creates(paths: &WorkerPaths, request: &Request) -> serde_json::Value {
    let Some(plan) = request.plan_id.as_deref() else {
        return serde_json::json!({"ok": false, "status": "invalid_request"});
    };
    // D-025 section 2: a manual request during an active run shows that run
    // and starts no second run.
    if let Some(active) = plan_scenario_id(paths, plan).and_then(|scenario| {
        Registry::open(&paths.registry)
            .ok()?
            .active_scenario_run(&scenario)
            .ok()?
    }) {
        return run_active_answer(&active);
    }
    let (reference, batch, added, deferred) = match batch_from_plan(paths, plan) {
        Ok(made) => made,
        Err(status) => return serde_json::json!({"ok": false, "status": status}),
    };
    let Ok(registry) = Registry::open(&paths.registry) else {
        return serde_json::json!({"ok": false, "status": "registry_unavailable"});
    };
    if let (Some(connection), Some(AccessToken(token))) = (
        yandex_source_connection(&registry, &reference.scenario_id, reference.revision),
        request.access_token.as_ref(),
    ) {
        runner().remember_token(&connection, token);
    }
    match registry.begin_manual_run(&reference, plan, &batch, deferred) {
        Ok(_) => {}
        // A run started between the check and now. Its paused batch never
        // executes; cancel it so that it does not wait forever.
        Err(RegistryError::ScenarioRunActive) => {
            let _ = registry.request_scenario_create_batch_cancellation(&batch);
            return match registry.active_scenario_run(&reference.scenario_id) {
                Ok(Some(active)) => run_active_answer(&active),
                _ => serde_json::json!({"ok": false, "status": "run_active"}),
            };
        }
        Err(_) => return serde_json::json!({"ok": false, "status": "registry_unavailable"}),
    }
    if registry
        .set_scenario_create_batch_paused(&batch, false)
        .is_err()
    {
        return serde_json::json!({"ok": false, "status": "registry_unavailable"});
    }
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
    // A Backup/export revision without files has no file plan: an empty file
    // selection would mean the whole home folder (D-027 section 7b).
    if left.kind == ISPMANAGER
        && registry
            .backup_resources(scenario_id, revision)
            .is_ok_and(|resources| {
                !resources
                    .iter()
                    .any(|r| r.category == "files" && r.rule == "include")
            })
    {
        return serde_json::json!({"ok": false, "status": "backup_without_files"});
    }
    // The `files/` folder of a Backup/export destination (D-027 section 5.2)
    // must exist before the destination walk: a missing root is an error.
    if left.kind == ISPMANAGER {
        let files_root = Path::new(&right.root);
        if let (Some(parent), Some(name)) = (files_root.parent(), files_root.file_name()) {
            let created = LocalFilesystem::open(parent)
                .and_then(|destination| destination.create_directory_tree(Path::new(name)));
            if created.is_err() && !files_root.is_dir() {
                return serde_json::json!({"ok": false, "status": "local_root_unavailable"});
            }
        }
    }
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

/// Parallel downloads for each Yandex Disk connection (D-024).
const YANDEX_PARALLEL_DOWNLOADS: usize = 2;

/// Idle SFTP sessions for each ISPmanager connection: one for the walk and
/// the parallel downloads (`YANDEX_PARALLEL_DOWNLOADS`).
const SFTP_POOL_SIZE: usize = 3;
/// A session idle longer than this closes: the SSH inactivity timeout of
/// the client is 60 s.
const SFTP_IDLE_LIMIT: Duration = Duration::from_secs(30);

struct PooledSftp {
    connection: SftpConnection,
    returned: std::time::Instant,
}

/// State of the run thread that the IPC thread can read (D-024).
struct Runner {
    /// Yandex Disk access tokens by connection ID, from the app. Memory only.
    tokens: Mutex<HashMap<String, String>>,
    /// SMB logins and passwords by connection ID, from the app. Memory only.
    smb_logins: Mutex<HashMap<String, (String, String)>>,
    /// ISPmanager logins and passwords by connection ID (D-027). Memory only.
    ispmanager_logins: Mutex<HashMap<String, (String, String)>>,
    /// Open SFTP sessions by connection ID that wait for the next file or
    /// walk step. One SSH sign-in for each file was too slow and fills the
    /// server log (2026-11-04).
    sftp_pool: Mutex<HashMap<String, Vec<PooledSftp>>>,
    /// The login that the server refused, by connection ID. The app sends
    /// the same login again every minute; it is not used again.
    smb_refused: Mutex<HashMap<String, (String, String)>>,
    /// The next allowed mount attempt and the current pause, by connection ID,
    /// after the server did not answer.
    smb_retry: Mutex<HashMap<String, (std::time::Instant, u64)>>,
    /// Batch children that run now.
    active: AtomicUsize,
    /// Until this Unix time the Worker starts no new effect, so that the app
    /// can restart it between files (`prepare_restart`). 0: no restart waits.
    drain_until: AtomicI64,
    /// The Unix time when this Worker process started. The app gives the
    /// credentials again when it sees a new value.
    started: i64,
}

impl Runner {
    /// True while a restart waits: running effects finish, new ones wait.
    fn draining(&self) -> bool {
        unix_now() < self.drain_until.load(Ordering::SeqCst)
    }

    fn remember_ispmanager_login(&self, connection_id: &str, login: &str, password: &str) {
        let credentials = (login.to_owned(), password.to_owned());
        let changed = self.ispmanager_logins.lock().is_ok_and(|mut logins| {
            logins.insert(connection_id.to_owned(), credentials.clone()) != Some(credentials)
        });
        // Sessions of an earlier login are not used again.
        if changed {
            if let Ok(mut pool) = self.sftp_pool.lock() {
                pool.remove(connection_id);
            }
        }
    }

    /// An idle session from the pool, or a new one.
    fn checkout_sftp(
        &self,
        paths: &WorkerPaths,
        connection_id: &str,
    ) -> Result<SftpConnection, &'static str> {
        let pooled = self.sftp_pool.lock().ok().and_then(|mut pool| {
            let sessions = pool.get_mut(connection_id)?;
            sessions.retain(|session| session.returned.elapsed() < SFTP_IDLE_LIMIT);
            sessions.pop()
        });
        match pooled {
            Some(session) => Ok(session.connection),
            None => open_sftp(paths, connection_id),
        }
    }

    /// Return a session after use. A session after a network or session
    /// error closes instead.
    fn checkin_sftp(&self, connection_id: &str, connection: SftpConnection, healthy: bool) {
        if !healthy {
            return;
        }
        if let Ok(mut pool) = self.sftp_pool.lock() {
            let sessions = pool.entry(connection_id.to_owned()).or_default();
            if sessions.len() < SFTP_POOL_SIZE {
                sessions.push(PooledSftp {
                    connection,
                    returned: std::time::Instant::now(),
                });
            }
        }
    }

    fn ispmanager_login(&self, connection_id: &str) -> Option<(String, String)> {
        self.ispmanager_logins
            .lock()
            .ok()?
            .get(connection_id)
            .cloned()
    }

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

    fn remember_smb_login(&self, connection_id: &str, login: &str, password: &str) {
        let credentials = (login.to_owned(), password.to_owned());
        if let Ok(mut refused) = self.smb_refused.lock() {
            if refused.get(connection_id) == Some(&credentials) {
                return;
            }
            refused.remove(connection_id);
        }
        let Ok(mut logins) = self.smb_logins.lock() else {
            return;
        };
        // Changed credentials allow an attempt now.
        if logins.get(connection_id) != Some(&credentials) {
            if let Ok(mut retry) = self.smb_retry.lock() {
                retry.remove(connection_id);
            }
        }
        logins.insert(connection_id.to_owned(), credentials);
    }
}

fn runner() -> &'static Runner {
    static RUNNER: OnceLock<Runner> = OnceLock::new();
    RUNNER.get_or_init(|| Runner {
        tokens: Mutex::new(HashMap::new()),
        smb_logins: Mutex::new(HashMap::new()),
        ispmanager_logins: Mutex::new(HashMap::new()),
        sftp_pool: Mutex::new(HashMap::new()),
        smb_refused: Mutex::new(HashMap::new()),
        smb_retry: Mutex::new(HashMap::new()),
        active: AtomicUsize::new(0),
        drain_until: AtomicI64::new(0),
        started: unix_now(),
    })
}

fn unix_now() -> i64 {
    i64::try_from(log_time()).unwrap_or(i64::MAX)
}

/// The run thread: give every Scenario run with due files its own lane until
/// the Worker exits. One shared page let a run of long or failing downloads
/// hold back every other run (found on 2026-11-04).
fn run_batches_forever(paths: WorkerPaths) {
    loop {
        // A panic in one pass must not stop every later effect.
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| start_lanes(&paths))).is_err() {
            eprintln!("qadren-worker: {} batch pass panicked", log_time());
        }
        thread::sleep(Duration::from_secs(1));
    }
}

/// Batches whose lane runs now.
fn lanes() -> &'static Mutex<std::collections::HashSet<String>> {
    static LANES: OnceLock<Mutex<std::collections::HashSet<String>>> = OnceLock::new();
    LANES.get_or_init(Default::default)
}

/// Start a lane for each batch with due children that has none.
fn start_lanes(paths: &WorkerPaths) {
    if runner().draining() {
        return;
    }
    let Ok(batches) = Registry::open(&paths.registry)
        .and_then(|registry| registry.due_scenario_batches(unix_now()))
    else {
        return;
    };
    for batch in batches {
        let started = lanes()
            .lock()
            .is_ok_and(|mut lanes| lanes.insert(batch.clone()));
        if started {
            let paths = paths.clone();
            thread::spawn(move || {
                if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    run_lane(&paths, &batch)
                }))
                .is_err()
                {
                    eprintln!(
                        "qadren-worker: {} lane of batch {batch} panicked",
                        log_time()
                    );
                }
                if let Ok(mut lanes) = lanes().lock() {
                    lanes.remove(&batch);
                }
            });
        }
    }
}

/// Files of one run that a lane reads at a time.
const LANE_PAGE: usize = 32;
/// Parallel effects of one run.
const LANE_THREADS: usize = 2;

/// Run the due children of one batch, page by page, until none is due. A
/// batch that waits for a token or a login keeps its lane and checks again.
fn run_lane(paths: &WorkerPaths, batch: &str) {
    loop {
        if runner().draining() {
            return;
        }
        let Some((due, cancelled, queue)) = lane_items(paths, batch) else {
            return;
        };
        if due == 0 {
            return;
        }
        // A stopped run cancels page after page at once; only a run that
        // waits for a token or a login pauses (found on 2026-11-04).
        if queue.is_empty() && cancelled == 0 {
            thread::sleep(Duration::from_secs(5));
            continue;
        }
        if queue.is_empty() {
            continue;
        }
        run_pool(paths, queue, LANE_THREADS);
    }
}

/// All batches one after another, for one Worker pass (`--once`, tests).
fn run_due_batch_children(paths: &WorkerPaths) {
    let Ok(batches) = Registry::open(&paths.registry)
        .and_then(|registry| registry.due_scenario_batches(unix_now()))
    else {
        return;
    };
    for batch in batches {
        if let Some((_, _, queue)) = lane_items(paths, &batch) {
            run_pool(paths, queue, LANE_THREADS);
        }
    }
}

/// One page of a batch as pool items: the number of due children, the
/// number cancelled now, and the children that can run now. A Yandex Disk
/// batch without a token and an ISPmanager batch without a login wait for
/// the app.
fn lane_items(
    paths: &WorkerPaths,
    batch: &str,
) -> Option<(usize, usize, std::collections::VecDeque<PoolItem>)> {
    let registry = Registry::open(&paths.registry).ok()?;
    let due = registry
        .due_children_of_batch(batch, unix_now(), LANE_PAGE)
        .ok()?;
    let count = due.len();
    let Some(first) = due.first() else {
        return Some((0, 0, Default::default()));
    };
    if first.cancellation_requested {
        let cancelled = registry.cancel_pending_batch_children(batch).ok()?;
        return Some((count, cancelled, Default::default()));
    }
    let (scenario, revision) = (first.scenario.scenario_id.clone(), first.scenario.revision);
    let sides = registry
        .get_scenario_revision(&scenario, revision)
        .ok()
        .flatten()
        .map(|scenario| (scenario.left_connection_id, scenario.right_connection_id));
    let yandex = yandex_source_connection(&registry, &scenario, revision);
    let ispmanager = ispmanager_source_connection(&registry, &scenario, revision);
    let mut queue = std::collections::VecDeque::new();
    let mut cancelled = 0;
    for child in due {
        if child.cancellation_requested {
            let _ = registry.finish_scenario_create_batch_child(
                &child.batch_id,
                &child.relative_path,
                "cancelled",
                None,
            );
            cancelled += 1;
            continue;
        }
        let Some((left, right)) = sides.clone() else {
            continue;
        };
        let destination = if child.source_is_left { right } else { left };
        let mut slots = vec![format!("write:{destination}")];
        if let Some(connection) = &ispmanager {
            if runner().ispmanager_login(connection).is_none() {
                continue;
            }
            slots.push(format!("sftp:{connection}"));
        }
        if let Some(connection) = &yandex {
            if runner().token(connection).is_none() {
                continue;
            }
            slots.push(format!("yandex:{connection}"));
        }
        queue.push_back(PoolItem {
            child,
            yandex: yandex.clone(),
            ispmanager: ispmanager.clone(),
            slots,
        });
    }
    Some((count, cancelled, queue))
}

/// Parallel effects in one destination connection (Local Filesystem or SMB),
/// over all lanes. Each lane has `LANE_THREADS`, so two runs into one
/// destination both move: with 2, long downloads of one run held both slots
/// (found on 2026-11-04).
const DESTINATION_PARALLEL_EFFECTS: usize = 4;

struct PoolItem {
    child: DueBatchChild,
    yandex: Option<String>,
    /// The ISPmanager source of a Backup/export child (D-027).
    ispmanager: Option<String>,
    /// Limited resources that this child uses: its destination connection
    /// and, for a remote source, the download slots of that connection.
    slots: Vec<String>,
}

fn slot_limit(slot: &str) -> usize {
    if slot.starts_with("yandex:") || slot.starts_with("sftp:") {
        YANDEX_PARALLEL_DOWNLOADS
    } else {
        DESTINATION_PARALLEL_EFFECTS
    }
}

/// Connection slots that every lane shares, so that parallel runs keep the
/// limits of each connection.
struct SlotTable {
    busy: Mutex<HashMap<String, usize>>,
    freed: std::sync::Condvar,
}

fn slot_table() -> &'static SlotTable {
    static TABLE: OnceLock<SlotTable> = OnceLock::new();
    TABLE.get_or_init(|| SlotTable {
        busy: Mutex::new(HashMap::new()),
        freed: std::sync::Condvar::new(),
    })
}

/// Run one lane page on `threads` threads. A thread takes the first child
/// whose connections have a free slot; a full connection does not block
/// children that use other connections.
fn run_pool(paths: &WorkerPaths, queue: std::collections::VecDeque<PoolItem>, threads: usize) {
    let queue = Mutex::new(queue);
    let table = slot_table();
    thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| loop {
                let item = loop {
                    // A waiting restart takes effect before the next child.
                    if runner().draining() {
                        return;
                    }
                    let Ok(mut pending) = queue.lock() else {
                        return;
                    };
                    if pending.is_empty() {
                        return;
                    }
                    let Ok(mut busy) = table.busy.lock() else {
                        return;
                    };
                    let free = pending.iter().position(|item| {
                        item.slots
                            .iter()
                            .all(|slot| busy.get(slot).copied().unwrap_or(0) < slot_limit(slot))
                    });
                    if let Some(index) = free {
                        let item = pending.remove(index).expect("position is in range");
                        for slot in &item.slots {
                            *busy.entry(slot.clone()).or_default() += 1;
                        }
                        break item;
                    }
                    drop(pending);
                    // Another lane can free the slot; check again at least
                    // every second.
                    let _ = table.freed.wait_timeout(busy, Duration::from_secs(1));
                };
                run_pool_item(paths, &item);
                if let Ok(mut busy) = table.busy.lock() {
                    for slot in &item.slots {
                        if let Some(count) = busy.get_mut(slot) {
                            *count = count.saturating_sub(1);
                        }
                    }
                }
                table.freed.notify_all();
            });
        }
    });
}

fn run_pool_item(paths: &WorkerPaths, item: &PoolItem) {
    if let Some(connection) = &item.ispmanager {
        if still_due(paths, &item.child) {
            run_ispmanager_batch_child(paths, &item.child, connection);
        }
        return;
    }
    match &item.yandex {
        Some(connection) => {
            // A 401 removes the token; the batch then waits.
            let Some(token) = runner().token(connection) else {
                return;
            };
            if still_due(paths, &item.child) {
                run_batch_child(paths, &item.child, Some((connection, &token)));
            }
        }
        None => {
            if still_due(paths, &item.child) {
                run_batch_child(paths, &item.child, None);
            }
        }
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

/// Run one child of a Backup/export batch on its own SFTP session and
/// record its outcome (D-027 section 7a).
fn run_ispmanager_batch_child(paths: &WorkerPaths, child: &DueBatchChild, connection_id: &str) {
    runner().active.fetch_add(1, Ordering::SeqCst);
    let result = (|| {
        if let Some(state) = job_state(paths, &child.job_id) {
            return Ok(LocalFileCopyStatus::Existing(state));
        }
        let connection = runner()
            .checkout_sftp(paths, connection_id)
            .map_err(|_| LocalFileCopyError::Sftp(SftpFailure::SessionUnavailable))?;
        let (scenario_id, revision) = (&child.scenario.scenario_id, child.scenario.revision);
        let result = match child.kind {
            BatchChildKind::Create => run_ispmanager_scenario_file_download(
                &paths.jobs,
                &paths.registry,
                scenario_id,
                revision,
                &child.relative_path,
                &child.job_id,
                &connection,
                None,
            ),
            BatchChildKind::Update => run_ispmanager_scenario_update(
                &paths.jobs,
                &paths.registry,
                scenario_id,
                revision,
                &child.relative_path,
                &child.job_id,
                &connection,
            ),
            BatchChildKind::Delete | BatchChildKind::DeleteFolder => {
                run_ispmanager_scenario_delete(
                    &paths.jobs,
                    &paths.registry,
                    scenario_id,
                    revision,
                    &child.relative_path,
                    &child.job_id,
                    &connection,
                    child.kind == BatchChildKind::DeleteFolder,
                )
                .map(|status| match status {
                    LocalRuleDeleteStatus::MovedToTrash
                    | LocalRuleDeleteStatus::DeletedPermanently => LocalFileCopyStatus::Completed,
                    LocalRuleDeleteStatus::Existing(state) => LocalFileCopyStatus::Existing(state),
                })
            }
        };
        let healthy = !matches!(
            result,
            Err(LocalFileCopyError::Sftp(
                SftpFailure::Network | SftpFailure::SessionUnavailable
            ))
        );
        runner().checkin_sftp(connection_id, connection, healthy);
        result
    })();
    record_batch_child_outcome(paths, child, None, result);
    runner().active.fetch_sub(1, Ordering::SeqCst);
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
        let (scenario_id, revision) = (&child.scenario.scenario_id, child.scenario.revision);
        let deleted = |status: LocalRuleDeleteStatus| match status {
            LocalRuleDeleteStatus::MovedToTrash | LocalRuleDeleteStatus::DeletedPermanently => {
                LocalFileCopyStatus::Completed
            }
            LocalRuleDeleteStatus::Existing(state) => LocalFileCopyStatus::Existing(state),
        };
        if let Some((_, token)) = yandex {
            let client = YandexMetadataClient::new().map_err(LocalFileCopyError::Remote)?;
            return match child.kind {
                BatchChildKind::Create => run_yandex_download(
                    paths,
                    scenario_id,
                    revision,
                    &child.relative_path,
                    &child.job_id,
                    Some(token),
                    None,
                ),
                BatchChildKind::Update => run_yandex_scenario_update(
                    &paths.jobs,
                    &paths.registry,
                    scenario_id,
                    revision,
                    &child.relative_path,
                    &child.job_id,
                    token,
                    &client,
                ),
                BatchChildKind::Delete => run_yandex_scenario_delete(
                    &paths.jobs,
                    &paths.registry,
                    scenario_id,
                    revision,
                    &child.relative_path,
                    &child.job_id,
                    token,
                    &client,
                )
                .map(deleted),
                // Only a Backup/export mirror deletes a whole folder.
                BatchChildKind::DeleteFolder => Err(LocalFileCopyError::ScenarioScopeMismatch),
            };
        }
        match child.kind {
            BatchChildKind::Create => {}
            BatchChildKind::Update => {
                return run_local_scenario_update(
                    &paths.jobs,
                    &paths.registry,
                    scenario_id,
                    revision,
                    &child.relative_path,
                    child.source_is_left,
                    &child.job_id,
                )
            }
            BatchChildKind::Delete => {
                return run_local_scenario_rule_delete(
                    &paths.jobs,
                    &paths.registry,
                    scenario_id,
                    revision,
                    &child.relative_path,
                    !child.source_is_left,
                    &child.job_id,
                )
                .map(deleted)
            }
            BatchChildKind::DeleteFolder => return Err(LocalFileCopyError::ScenarioScopeMismatch),
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
        E::Sftp(failure) => match failure {
            SftpFailure::NotFound => (Failure::Permanent, "ispmanager_not_found".into()),
            SftpFailure::PermissionDenied => {
                (Failure::Permanent, "ispmanager_permission_denied".into())
            }
            SftpFailure::SessionUnavailable => {
                (Failure::Transient, "ispmanager_session_unavailable".into())
            }
            SftpFailure::Network => (Failure::Transient, "ispmanager_network_error".into()),
            SftpFailure::Changed => (Failure::Transient, "ispmanager_source_changed".into()),
        },
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
        "qadren-worker: {} batch child {} ({}) failed: {code}",
        log_time(),
        child.job_id,
        child.kind.as_str()
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
                    .scenario_retry_until_success(&child.scenario.scenario_id)
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

/// "Остановить": stop the active run of a Scenario (D-025 section 2). A
/// planning run ends now. An executing run cancels its pending items and
/// ends when no item executes; completed items stay.
fn stop_scenario_run(paths: &WorkerPaths, request: &Request) -> serde_json::Value {
    let Some(scenario_id) = request.scenario_id.as_deref() else {
        return serde_json::json!({"ok": false, "status": "invalid_request"});
    };
    let Ok(registry) = Registry::open(&paths.registry) else {
        return serde_json::json!({"ok": false, "status": "registry_unavailable"});
    };
    let run = match registry.active_scenario_run(scenario_id) {
        Ok(Some(run)) => run,
        Ok(None) => return serde_json::json!({"ok": true, "status": "no_active_run"}),
        Err(_) => return serde_json::json!({"ok": false, "status": "registry_unavailable"}),
    };
    let stopped = match (run.state.as_str(), run.batch_id.as_deref()) {
        ("executing", Some(batch)) => registry
            .request_scenario_create_batch_cancellation(batch)
            .is_ok(),
        _ => finish_run(&registry, &run, "cancelled", Some("stopped_by_user")).is_ok(),
    };
    drop(registry);
    if !stopped {
        return serde_json::json!({"ok": false, "status": "registry_unavailable"});
    }
    if run.state == "planning" {
        let _ = finish_run_job(paths, &run.id);
    }
    serde_json::json!({"ok": true, "status": "stop_requested", "run": run.id})
}

/// "Запустить" of the monitoring Scenario widget: a manual run of the current
/// revision that plans and executes like a scheduled run. During an active
/// run it starts no second run and answers with the active one (D-025).
fn run_scenario_now(paths: &WorkerPaths, request: &Request) -> serde_json::Value {
    let Some(scenario_id) = request.scenario_id.as_deref() else {
        return serde_json::json!({"ok": false, "status": "invalid_request"});
    };
    let Ok(registry) = Registry::open(&paths.registry) else {
        return serde_json::json!({"ok": false, "status": "registry_unavailable"});
    };
    let scenario = match registry.list_scenarios() {
        Ok(scenarios) => scenarios
            .into_iter()
            .find(|scenario| scenario.id == scenario_id),
        Err(_) => return serde_json::json!({"ok": false, "status": "registry_unavailable"}),
    };
    let Some(scenario) = scenario else {
        return serde_json::json!({"ok": false, "status": "scenario_unavailable"});
    };
    if !scenario.enabled {
        return serde_json::json!({"ok": false, "status": "scenario_disabled"});
    }
    match registry.active_scenario_run(scenario_id) {
        Ok(Some(active)) => return run_active_answer(&active),
        Ok(None) => {}
        Err(_) => return serde_json::json!({"ok": false, "status": "registry_unavailable"}),
    }
    let reference = ScenarioReference {
        scenario_id: scenario.id,
        revision: scenario.revision,
    };
    if let (Some(connection), Some(AccessToken(token))) = (
        yandex_source_connection(&registry, &reference.scenario_id, reference.revision),
        request.access_token.as_ref(),
    ) {
        runner().remember_token(&connection, token);
    }
    match registry.begin_manual_planning_run(&reference) {
        Ok(run) => serde_json::json!({"ok": true, "status": "accepted", "run": run}),
        Err(RegistryError::ScenarioRunActive) => match registry.active_scenario_run(scenario_id) {
            Ok(Some(active)) => run_active_answer(&active),
            _ => serde_json::json!({"ok": false, "status": "run_active"}),
        },
        Err(_) => serde_json::json!({"ok": false, "status": "registry_unavailable"}),
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
                    "draining": runner().draining(),
                    "started": runner().started,
                })
            );
        }
        "prepare_restart" => {
            // Running effects finish; no new effect starts until the limit.
            // The app restarts the Worker when `active` is 0.
            runner()
                .drain_until
                .store(unix_now() + RESTART_DRAIN_LIMIT, Ordering::SeqCst);
            let _ = writeln!(
                stream,
                "{}",
                serde_json::json!({
                    "ok": true,
                    "status": "running",
                    "active": runner().active.load(Ordering::SeqCst),
                    "draining": true,
                    "started": runner().started,
                })
            );
        }
        "cancel_restart" => {
            runner().drain_until.store(0, Ordering::SeqCst);
            let _ = writeln!(
                stream,
                "{}",
                serde_json::json!({
                    "ok": true,
                    "status": "running",
                    "active": runner().active.load(Ordering::SeqCst),
                    "draining": false,
                    "started": runner().started,
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
        "database_progress" => {
            let answer = database_progress_status(&request);
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
        "stop_run" => {
            let answer = stop_scenario_run(paths, &request);
            wake_runs();
            let _ = writeln!(stream, "{answer}");
        }
        "run_now" => {
            let answer = run_scenario_now(paths, &request);
            wake_runs();
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
        "provide_ispmanager_credentials" => {
            let ok = match (
                request.connection_id.as_deref(),
                request.login.as_deref(),
                request.password.as_ref(),
            ) {
                (Some(connection), Some(login), Some(AccessToken(password)))
                    if !connection.is_empty() && !login.is_empty() =>
                {
                    runner().remember_ispmanager_login(connection, login, password);
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
        "provide_smb_credentials" => {
            let ok = match (
                request.connection_id.as_deref(),
                request.login.as_deref(),
                request.password.as_ref(),
            ) {
                (Some(connection), Some(login), Some(AccessToken(password)))
                    if !connection.is_empty() && !login.is_empty() =>
                {
                    runner().remember_smb_login(connection, login, password);
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
        advance_scenario_runs(&paths)?;
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
            if !runner().draining() {
                let _ = dispatch_due(&paths);
            }
            thread::sleep(POLL_INTERVAL);
        });
    }
    // The runs advance on their own thread. Reconciliation of UNKNOWN Jobs
    // can list large folders on a slow network drive for minutes; a manual
    // run waited for it before it started (found on 2026-11-06). A new run
    // or a finished step wakes this thread; otherwise it checks each poll
    // interval (D-028).
    {
        let paths = paths.clone();
        thread::spawn(move || loop {
            if !runner().draining() {
                let _ = advance_scenario_runs(&paths);
            }
            wait_for_run_wake(std::time::Instant::now() + POLL_INTERVAL);
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

    #[test]
    fn a_refused_smb_login_is_not_used_again_until_it_changes() {
        let runner = Runner {
            tokens: Mutex::new(HashMap::new()),
            smb_logins: Mutex::new(HashMap::new()),
            ispmanager_logins: Mutex::new(HashMap::new()),
            sftp_pool: Mutex::new(HashMap::new()),
            smb_refused: Mutex::new(HashMap::new()),
            smb_retry: Mutex::new(HashMap::new()),
            active: AtomicUsize::new(0),
            drain_until: AtomicI64::new(0),
            started: 0,
        };
        let login = |runner: &Runner| runner.smb_logins.lock().unwrap().get("nas").cloned();
        runner.remember_smb_login("nas", "alice", "wrong");
        assert_eq!(login(&runner), Some(("alice".into(), "wrong".into())));
        // The server refused it: the Worker moves it aside.
        let refused = runner.smb_logins.lock().unwrap().remove("nas").unwrap();
        runner
            .smb_refused
            .lock()
            .unwrap()
            .insert("nas".into(), refused);
        // The app sends the same login every minute: it is ignored.
        runner.remember_smb_login("nas", "alice", "wrong");
        assert_eq!(login(&runner), None);
        // A pause for an offline server stays for the same login...
        runner.remember_smb_login("nas", "alice", "right");
        runner
            .smb_retry
            .lock()
            .unwrap()
            .insert("nas".into(), (std::time::Instant::now(), 1));
        runner.remember_smb_login("nas", "alice", "right");
        assert!(runner.smb_retry.lock().unwrap().contains_key("nas"));
        // ...and a changed login allows an attempt now.
        runner.remember_smb_login("nas", "alice", "newer");
        assert!(!runner.smb_retry.lock().unwrap().contains_key("nas"));
        assert!(runner.smb_refused.lock().unwrap().is_empty());
    }

    fn root(name: &str) -> PathBuf {
        // Unix-domain socket paths have a small platform limit. Keep test
        // roots under the system temporary directory so an exported checkout
        // cannot make the private Worker socket unbindable.
        let temporary = std::env::temp_dir().canonicalize().unwrap();
        // A replacement keeps the previous version in the Trash. Worker tests
        // must never use the real Trash of the user.
        let trash = temporary.join("qw-trash");
        fs::create_dir_all(&trash).unwrap();
        qadren_core::local_fs::override_trash_directory_for_tests(trash);
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

    /// Drive every run to its end without the Worker threads: poll, walk
    /// and plan, then execute due batch children (D-025).
    /// One pass of the run thread. The Worker claims the JobStore for its
    /// process at start, so that the effect threads share it; do the same.
    fn run_pass(paths: &WorkerPaths) {
        let _ = JobStore::claim_for_process(&paths.jobs);
        run_due_batch_children(paths);
    }

    fn drive_runs(paths: &WorkerPaths) {
        for _ in 0..6 {
            dispatch_due(paths).unwrap();
            while run_snapshot_pass(paths) {}
            run_pass(paths);
        }
    }

    fn latest_run(paths: &WorkerPaths, scenario: &str) -> qadren_core::registry::ScenarioRun {
        Registry::open(&paths.registry)
            .unwrap()
            .latest_scenario_run(scenario)
            .unwrap()
            .unwrap()
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
        // The run thread executes batch children (D-024).
        assert_eq!(dispatch_due(&paths).unwrap(), (0, 0));
        run_pass(&paths);
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
        run_pass(&paths);
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
        run_pass(&paths);
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
    fn scheduled_run_copies_new_files_through_snapshots_plan_and_batch() {
        let (root, left_root, right_root, paths, scenario) =
            scheduled_fixture("scheduled-execution");
        fs::write(left_root.join("report.txt"), b"first version").unwrap();
        schedule_current_minute(&paths, &scenario);
        assert_eq!(dispatch_due(&paths).unwrap(), (1, 1));
        // The slot starts a run; the poll itself copies nothing.
        let run = latest_run(&paths, &scenario);
        assert_eq!(
            (run.trigger.as_str(), run.state.as_str()),
            ("schedule", "planning")
        );
        assert!(!right_root.join("report.txt").exists());
        assert_eq!(coordinator_states(&paths), vec![JobState::Queued]);

        drive_runs(&paths);
        assert_eq!(
            fs::read(right_root.join("report.txt")).unwrap(),
            b"first version"
        );
        let run = latest_run(&paths, &scenario);
        assert_eq!(run.state, "completed");
        assert_eq!(run.deferred_items, 0);
        assert_eq!(coordinator_states(&paths), vec![JobState::Completed]);

        // Without the confirmation of "the source always wins" a one-way
        // run does not replace a file (D-025 section 4). The run reports
        // the Update as deferred and changes no file.
        fs::write(left_root.join("report.txt"), b"second version").unwrap();
        schedule_current_minute(&paths, &scenario);
        drive_runs(&paths);
        assert_eq!(
            fs::read(right_root.join("report.txt")).unwrap(),
            b"first version"
        );
        let run = latest_run(&paths, &scenario);
        assert_eq!(run.state, "completed");
        assert_eq!(run.deferred_items, 1);
        assert_eq!(
            coordinator_states(&paths),
            vec![JobState::Completed, JobState::Completed]
        );
        fs::remove_dir_all(root).unwrap();
    }

    /// D-025 sections 3 to 6: a one-way run updates a changed file and
    /// restores a destination change after the one-time confirmation of
    /// "the source always wins". A deletion is deferred in Paranoid mode.
    /// The library tests check Delete with a test Trash; this binary would
    /// use the real Trash of the user.
    #[test]
    fn one_way_run_updates_after_the_source_wins_confirmation() {
        let root = root("source-wins");
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
            .create_scenario_with_scope(
                "Source wins",
                &left,
                left_root.to_str().unwrap(),
                &right,
                right_root.to_str().unwrap(),
                "left_to_right",
                &qadren_core::model::ScenarioScope::default(),
            )
            .unwrap();
        drop(registry);
        fs::write(left_root.join("changed.txt"), b"one").unwrap();
        fs::write(left_root.join("edited.txt"), b"source").unwrap();
        fs::write(left_root.join("removed.txt"), b"gone soon").unwrap();
        schedule_current_minute(&paths, &scenario);
        drive_runs(&paths);
        assert_eq!(latest_run(&paths, &scenario).state, "completed");
        assert!(right_root.join("removed.txt").exists());

        fs::write(left_root.join("changed.txt"), b"two, longer").unwrap();
        fs::write(right_root.join("edited.txt"), b"edited in the destination").unwrap();
        fs::remove_file(left_root.join("removed.txt")).unwrap();

        // Without the confirmation the run replaces and deletes nothing.
        schedule_current_minute(&paths, &scenario);
        drive_runs(&paths);
        let run = latest_run(&paths, &scenario);
        assert_eq!((run.state.as_str(), run.deferred_items), ("completed", 3));
        assert_eq!(fs::read(right_root.join("changed.txt")).unwrap(), b"one");
        assert!(right_root.join("removed.txt").exists());

        Registry::open(&paths.registry)
            .unwrap()
            .confirm_scenario_source_wins(&scenario, 1)
            .unwrap();
        schedule_current_minute(&paths, &scenario);
        drive_runs(&paths);
        let run = latest_run(&paths, &scenario);
        assert_eq!((run.state.as_str(), run.deferred_items), ("completed", 1));
        assert_eq!(
            fs::read(right_root.join("changed.txt")).unwrap(),
            b"two, longer"
        );
        assert_eq!(fs::read(right_root.join("edited.txt")).unwrap(), b"source");
        assert!(right_root.join("removed.txt").exists());
        // The replaced destination version is kept in the Trash.
        let trash = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join("qw-trash");
        assert!(fs::read_dir(trash).unwrap().any(|entry| {
            fs::read(entry.unwrap().path()).unwrap() == b"edited in the destination"
        }));
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
        drive_runs(&paths);
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
    #[test]
    fn two_runs_with_one_destination_execute_one_after_the_other() {
        let root = root("write-lock");
        let (photos, music, shared) =
            (root.join("photos"), root.join("music"), root.join("shared"));
        for dir in [&photos, &music, &shared] {
            fs::create_dir_all(dir).unwrap();
        }
        fs::write(photos.join("photo.jpg"), b"photo").unwrap();
        fs::write(music.join("song.mp3"), b"song").unwrap();
        let paths = WorkerPaths {
            socket: root.join("worker.sock"),
            registry: root.join("registry.sqlite3"),
            jobs: root.join("jobs.sqlite3"),
        };
        let registry = Registry::open(&paths.registry).unwrap();
        let mac = registry.add("local_filesystem", "Mac").unwrap();
        let create = |name: &str, source: &Path| {
            registry
                .create_scenario(
                    name,
                    &mac,
                    source.to_str().unwrap(),
                    &mac,
                    shared.to_str().unwrap(),
                    "left_to_right",
                )
                .unwrap()
        };
        let (first, second) = (create("Photos", &photos), create("Music", &music));
        drop(registry);
        schedule_current_minute(&paths, &first);
        schedule_current_minute(&paths, &second);
        dispatch_due(&paths).unwrap();
        let waiting = [&first, &second]
            .into_iter()
            .map(|scenario| latest_run(&paths, scenario))
            .filter(|run| run.waiting_reason.as_deref() == Some("waiting_for_scenario"))
            .count();
        assert_eq!(waiting, 1, "one run waits for the write-root lock");
        drive_runs(&paths);
        drive_runs(&paths);
        for scenario in [&first, &second] {
            let run = latest_run(&paths, scenario);
            assert_eq!((run.state.as_str(), run.error), ("completed", None));
        }
        assert_eq!(fs::read(shared.join("photo.jpg")).unwrap(), b"photo");
        assert_eq!(fs::read(shared.join("song.mp3")).unwrap(), b"song");
        fs::remove_dir_all(root).unwrap();
    }

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

    fn version_of(root: &Path, path: &str) -> Option<ConfirmedFileVersion> {
        observed_files(root)
            .unwrap()
            .0
            .into_iter()
            .find(|file| file.relative_path == path)
            .map(|file| ConfirmedFileVersion {
                object_revision: file.object_revision,
                size: file.size,
                checksum_md5: file.checksum_md5,
            })
    }

    /// A schedule coordinator that the legacy schedule path left waiting for
    /// decisions. Runs no longer make one (D-025), but such Jobs must still
    /// resolve. Returns the conflict IDs in the order of `tasks`.
    fn legacy_waiting_coordinator(
        paths: &WorkerPaths,
        scenario: &str,
        left_root: &Path,
        right_root: &Path,
        tasks: &[(&str, &str, Option<bool>)],
    ) -> Vec<String> {
        let job = "schedule-legacy-coordinator";
        let mut jobs = JobStore::open(&paths.jobs).unwrap();
        jobs.enqueue(job, Some((scenario, 1))).unwrap();
        jobs.advance(
            job,
            JobState::Queued,
            JobState::Running,
            Confirmation::Confirmed(EvidenceLevel::Requested),
        )
        .unwrap();
        jobs.advance(
            job,
            JobState::Running,
            JobState::WaitingForDecision,
            Confirmation::Unknown,
        )
        .unwrap();
        let registry = Registry::open(&paths.registry).unwrap();
        let reference = ScenarioReference {
            scenario_id: scenario.to_owned(),
            revision: 1,
        };
        tasks
            .iter()
            .map(|(path, kind, source_is_left)| {
                let id = registry
                    .queue_conflict_task(
                        &reference,
                        path,
                        kind,
                        *source_is_left,
                        version_of(left_root, path).as_ref(),
                        version_of(right_root, path).as_ref(),
                    )
                    .unwrap();
                registry.link_schedule_conflict(job, &id).unwrap();
                id
            })
            .collect()
    }

    #[test]
    fn scheduled_deletion_waits_for_fresh_consent_and_completes_its_coordinator() {
        let (root, left_root, right_root, paths, scenario) =
            scheduled_fixture("scheduled-deletion");
        fs::write(left_root.join("report.txt"), b"baseline").unwrap();
        schedule_current_minute(&paths, &scenario);
        drive_runs(&paths);
        assert_eq!(
            fs::read(right_root.join("report.txt")).unwrap(),
            b"baseline"
        );

        // The source file is deleted. In Paranoid mode the next scheduled
        // run must not delete the destination by itself.
        fs::remove_file(left_root.join("report.txt")).unwrap();
        schedule_current_minute(&paths, &scenario);
        drive_runs(&paths);
        assert!(right_root.join("report.txt").exists());
        assert_eq!(latest_run(&paths, &scenario).deferred_items, 1);

        let ids = legacy_waiting_coordinator(
            &paths,
            &scenario,
            &left_root,
            &right_root,
            &[("report.txt", "deletion", None)],
        );
        let registry = Registry::open(&paths.registry).unwrap();
        let reference = ScenarioReference {
            scenario_id: scenario.clone(),
            revision: 1,
        };
        let conflicts = registry.list_conflict_tasks(&reference).unwrap();
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].id, ids[0]);
        assert_eq!(conflicts[0].kind, "deletion");
        assert_eq!(conflicts[0].state, "pending");
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
        drive_runs(&paths);

        fs::write(left_root.join("a.txt"), b"second version of a").unwrap();
        fs::write(left_root.join("b.txt"), b"second version of b").unwrap();
        let ids = legacy_waiting_coordinator(
            &paths,
            &scenario,
            &left_root,
            &right_root,
            &[
                ("a.txt", "content", Some(true)),
                ("b.txt", "content", Some(true)),
            ],
        );
        let registry = Registry::open(&paths.registry).unwrap();
        let task = |path: &str| ids[if path == "a.txt" { 0 } else { 1 }].clone();

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
        drive_runs(&paths);
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
            login: None,
            password: None,
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
            login: None,
            password: None,
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
