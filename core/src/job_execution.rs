//! The first G1 execution slice: an explicit, app-owned mock destination.
//! It never contacts a provider or reads user/cloud content.
use crate::confirmation::{Confirmation, EvidenceLevel};
use crate::jobs::{JobStore, JobStoreError, ScenarioCopyBaselineOutboxEntry};
use crate::local_fs::{LocalError, LocalFilesystem};
use crate::model::{ConfirmedFileVersion, DeletionMode, JobState, ScenarioReference, ScopeMatch};
use crate::policy::{Decision, OperationScope, ProtectedOperation};
use crate::registry::Registry;
use crate::scenario_copy_dispatch::dispatch_pending_scenario_copy_baselines;
use crate::verification::{OperationVerifier, VerifiedReceipt};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};

pub const MOCK_CONNECTION_ID: &str = "qadren-g1-mock";
const MOCK_CONTENT: &[u8] = b"QADREN G1 mock create artifact\n";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MockCreateStatus {
    Queued,
    Completed,
    Existing(JobState),
}

/// Result of one non-destructive Local Filesystem controller action.
/// Directory creation is intentionally the first macOS-capable real operation:
/// it never opens user-file content and therefore cannot hydrate a File
/// Provider placeholder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalDirectoryCreateStatus {
    Completed,
    Existing(JobState),
}

/// Result of a read-only reconciliation for an UNKNOWN Local Filesystem
/// directory creation. It never retries or repairs the filesystem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalDirectoryReconciliationStatus {
    Completed,
    StillUnknown,
    Existing(JobState),
}

/// A durable Local Filesystem directory-create Job that can be inspected for
/// reconciliation in the currently opened root. This is inventory metadata;
/// it is not evidence that the directory was created.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalDirectoryReconciliationCandidate {
    pub job_id: String,
}

/// A durable Local Filesystem copy Job that can be inspected for
/// reconciliation in the currently opened destination root. This is inventory
/// metadata; it is not evidence that the copy completed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalFileCopyReconciliationCandidate {
    pub job_id: String,
}

/// Result of a read-only check of an UNKNOWN mock operation.  This never
/// retries a write: only an exact match with the durable expectation can
/// resolve the existing attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MockReconciliationStatus {
    Completed,
    StillUnknown,
    Existing(JobState),
}

#[derive(Debug)]
pub enum MockCreateError {
    ConsentRequired,
    InvalidJobId,
    OccupiedTarget,
    Io(std::io::Error),
    Store(JobStoreError),
    VerificationFailed,
    SimulatedCrashAfterEffect,
}

#[derive(Debug)]
pub enum LocalDirectoryCreateError {
    InvalidRequest,
    TargetExists,
    CapabilityUnavailable,
    ConnectionUnavailable,
    VerificationFailed,
    OutcomeUnknown(LocalError),
    Local(LocalError),
    Store(JobStoreError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalFileCopyStatus {
    Completed,
    Existing(JobState),
}

/// Result of a read-only reconciliation for an UNKNOWN Local copy Job.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalFileCopyReconciliationStatus {
    Completed,
    StillUnknown,
    Existing(JobState),
}

fn is_local_file_copy_operation(operation_kind: Option<&str>) -> bool {
    matches!(
        operation_kind,
        Some("local_copy_new" | "local_copy_replace")
    )
}

#[derive(Debug)]
pub enum LocalFileCopyError {
    InvalidRequest,
    ConsentRequired,
    TargetExists,
    CapabilityUnavailable,
    ConnectionUnavailable,
    ScenarioUnavailable,
    ScenarioScopeMismatch,
    ConflictUnavailable,
    ConflictStale,
    /// An interactive request was planned for a Scenario revision that is no
    /// longer current. The user must check the plan again.
    ScenarioRevisionStale,
    VerificationFailed,
    OutcomeUnknown(LocalError),
    Local(LocalError),
    Store(JobStoreError),
}

/// Refuse a new interactive Job for a superseded Scenario revision. Staged
/// schedule Jobs and confirmed batch receipts keep their pinned revision and do
/// not call this check.
pub fn require_current_scenario_revision(
    registry_path: &Path,
    scenario_id: &str,
    revision: u64,
) -> Result<(), LocalFileCopyError> {
    let current = Registry::open(registry_path)
        .and_then(|registry| registry.is_current_scenario_revision(scenario_id, revision))
        .map_err(|_| LocalFileCopyError::ScenarioUnavailable)?;
    if current {
        Ok(())
    } else {
        Err(LocalFileCopyError::ScenarioRevisionStale)
    }
}

fn matches_conflict_snapshot(
    entry: &crate::local_fs::LocalEntry,
    expected: &ConfirmedFileVersion,
) -> bool {
    !entry.is_directory
        && entry.size == expected.size
        && entry.object_revision.is_some()
        && entry.object_revision == expected.object_revision
}

/// Local metadata revisions contain mtime seconds and nanoseconds. This is
/// only an ordering signal for the same Local Filesystem adapter contract.
/// Modification time from a Local object revision, in epoch seconds. The
/// current format is `local:<dev>:<ino>:<hex bits of a double>`; older stored
/// revisions use `<seconds>:<nanoseconds>[:<ctime>...]`.
fn local_snapshot_modified_at(version: &ConfirmedFileVersion) -> Option<f64> {
    let mut fields = version.object_revision.as_deref()?.split(':');
    if fields.next()? != "local" {
        return None;
    }
    let _device = fields.next()?;
    let _inode = fields.next()?;
    let first = fields.next()?;
    match fields.next() {
        None => Some(f64::from_bits(u64::from_str_radix(first, 16).ok()?)),
        Some(nanoseconds) => {
            let seconds: i64 = first.parse().ok()?;
            let nanoseconds: u32 = nanoseconds.parse().ok()?;
            Some(seconds as f64 + f64::from(nanoseconds) / 1e9)
        }
    }
}

fn source_snapshot_is_newer(
    source: &ConfirmedFileVersion,
    destination: &ConfirmedFileVersion,
) -> Option<bool> {
    Some(local_snapshot_modified_at(source)? > local_snapshot_modified_at(destination)?)
}

fn enabled_local_connection(registry_path: &Path, connection_id: &str) -> bool {
    let Ok(registry) = Registry::open(registry_path) else {
        return false;
    };
    matches!(
        registry.get(connection_id),
        Ok(Some(connection)) if connection.adapter == "local_filesystem" && connection.enabled
    )
}

fn local_root_identity_key(local: &LocalFilesystem) -> Result<String, LocalDirectoryCreateError> {
    let identity = local
        .root_identity()
        .map_err(LocalDirectoryCreateError::Local)?;
    Ok(format!("unix:{}:{}", identity.device, identity.inode))
}

impl From<JobStoreError> for MockCreateError {
    fn from(value: JobStoreError) -> Self {
        Self::Store(value)
    }
}

impl From<std::io::Error> for MockCreateError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<JobStoreError> for LocalDirectoryCreateError {
    fn from(value: JobStoreError) -> Self {
        Self::Store(value)
    }
}

impl From<JobStoreError> for LocalFileCopyError {
    fn from(value: JobStoreError) -> Self {
        Self::Store(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MockFault {
    CrashAfterEffect,
    PartialWrite,
}

fn valid_job_id(job_id: &str) -> bool {
    !job_id.is_empty()
        && job_id.len() <= 96
        && job_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn target_name(job_id: &str) -> Result<String, MockCreateError> {
    if !valid_job_id(job_id) {
        return Err(MockCreateError::InvalidJobId);
    }
    Ok(format!("{job_id}.qadren-g1-test"))
}

fn md5_hex(content: &[u8]) -> String {
    format!("{:x}", md5::compute(content))
}

struct MockStorage {
    root: PathBuf,
}

impl MockStorage {
    fn open(root: &Path) -> Result<Self, MockCreateError> {
        // The root is application-owned. Refuse a symlink rather than follow a
        // caller-controlled alias into a user directory.
        match fs::symlink_metadata(root) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(MockCreateError::OccupiedTarget)
            }
            Ok(metadata) if !metadata.is_dir() => return Err(MockCreateError::OccupiedTarget),
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => fs::create_dir_all(root)?,
            Err(error) => return Err(error.into()),
        }
        Ok(Self {
            root: fs::canonicalize(root)?,
        })
    }

    /// Open an existing app-owned root without creating it.  Reconciliation
    /// must remain observational when a prior effect cannot be proven.
    fn open_existing(root: &Path) -> Result<Option<Self>, MockCreateError> {
        match fs::symlink_metadata(root) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(MockCreateError::OccupiedTarget)
            }
            Ok(metadata) if !metadata.is_dir() => return Err(MockCreateError::OccupiedTarget),
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        }
        Ok(Some(Self {
            root: fs::canonicalize(root)?,
        }))
    }

    fn target(&self, name: &str) -> Result<PathBuf, MockCreateError> {
        let relative = Path::new(name);
        if !relative
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
        {
            return Err(MockCreateError::InvalidJobId);
        }
        Ok(self.root.join(relative))
    }

    fn create_new(&self, name: &str, content: &[u8]) -> Result<(), MockCreateError> {
        let target = self.target(name)?;
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)
            .map_err(|error| {
                if error.kind() == std::io::ErrorKind::AlreadyExists {
                    MockCreateError::OccupiedTarget
                } else {
                    error.into()
                }
            })?;
        output.write_all(content)?;
        output.sync_all()?;
        Ok(())
    }
}

struct MockVerifier<'a> {
    storage: &'a MockStorage,
    target_name: &'a str,
    expected_size: u64,
    expected_md5: &'a str,
}

impl OperationVerifier for MockVerifier<'_> {
    fn verify(&self, attempt_id: &str) -> Option<VerifiedReceipt> {
        let path = self.storage.target(self.target_name).ok()?;
        let content = fs::read(path).ok()?;
        if content.len() as u64 != self.expected_size || md5_hex(&content) != self.expected_md5 {
            return None;
        }
        Some(VerifiedReceipt::metadata_verified(
            attempt_id,
            "mock_create_new",
            MOCK_CONNECTION_ID,
            self.target_name,
        ))
    }
}

/// Execute exactly one user-authorized mock create operation through JobStore.
/// The generated bytes belong to QADREN's app-owned test storage; no cloud or
/// user-file read occurs. A duplicate job is observed, never replayed.
pub fn run_mock_create(
    database_path: &Path,
    storage_root: &Path,
    job_id: &str,
    consent: bool,
) -> Result<MockCreateStatus, MockCreateError> {
    run_mock_create_inner(database_path, storage_root, job_id, consent, None)
}

/// Create one durable UNKNOWN mock attempt for a native UI test fixture.
///
/// This is compiled only with `ui-test-fixtures`. It writes only the
/// caller-supplied test paths and deliberately leaves the already-created
/// app-owned mock artifact for read-only reconciliation.
#[cfg(feature = "ui-test-fixtures")]
pub fn seed_unknown_mock_create_for_ui_fixture(
    database_path: &Path,
    storage_root: &Path,
    job_id: &str,
) -> Result<(), MockCreateError> {
    match run_mock_create_inner(
        database_path,
        storage_root,
        job_id,
        true,
        Some(MockFault::CrashAfterEffect),
    ) {
        Err(MockCreateError::SimulatedCrashAfterEffect) => Ok(()),
        Err(error) => Err(error),
        Ok(_) => Err(MockCreateError::VerificationFailed),
    }
}

/// Persist a mock Job without creating content. This is deliberately separate
/// from execution so a user can cancel it before any external effect starts.
pub fn queue_mock_create(
    database_path: &Path,
    job_id: &str,
) -> Result<MockCreateStatus, MockCreateError> {
    target_name(job_id)?;
    let mut store = JobStore::open(database_path)?;
    if let Some(existing) = store.get(job_id)? {
        return Ok(MockCreateStatus::Existing(existing.state));
    }
    store.enqueue(job_id, None)?;
    Ok(MockCreateStatus::Queued)
}

/// Cancel a mock Job only before an operation attempt is durable. This does
/// not touch test storage and cannot turn a possible effect into cancellation.
pub fn cancel_mock_create(database_path: &Path, job_id: &str) -> Result<(), MockCreateError> {
    target_name(job_id)?;
    let mut store = JobStore::open(database_path)?;
    store.cancel(job_id)?;
    Ok(())
}

/// Pause only a queued mock Job. No storage effect is started or interrupted.
pub fn pause_mock_create(database_path: &Path, job_id: &str) -> Result<(), MockCreateError> {
    target_name(job_id)?;
    let mut store = JobStore::open(database_path)?;
    store.pause(job_id)?;
    Ok(())
}

/// Return a safely paused mock Job to the queue; the later Run click remains
/// the separate operation-scoped consent for the file creation effect.
pub fn resume_mock_create(database_path: &Path, job_id: &str) -> Result<(), MockCreateError> {
    target_name(job_id)?;
    let mut store = JobStore::open(database_path)?;
    store.resume(job_id)?;
    Ok(())
}

/// Reconcile an UNKNOWN mock create without writing, deleting, or retrying.
///
/// The durable binding must name exactly this app-owned mock operation and
/// target.  A missing or mismatching object deliberately remains UNKNOWN.
pub fn reconcile_mock_create(
    database_path: &Path,
    storage_root: &Path,
    job_id: &str,
) -> Result<MockReconciliationStatus, MockCreateError> {
    let expected_target = target_name(job_id)?;
    let mut store = JobStore::open(database_path)?;
    let Some(job) = store.get(job_id)? else {
        return Err(MockCreateError::Store(JobStoreError::NotFound));
    };
    if job.state != JobState::Unknown {
        return Ok(MockReconciliationStatus::Existing(job.state));
    }

    let attempts = store.unresolved_attempts()?;
    let matching: Vec<_> = attempts
        .iter()
        .filter(|attempt| {
            attempt.job_id == job_id
                && attempt.status == "unknown"
                && attempt.operation_kind.as_deref() == Some("mock_create_new")
                && attempt.target_connection_id.as_deref() == Some(MOCK_CONNECTION_ID)
                && attempt.target_path.as_deref() == Some(expected_target.as_str())
        })
        .collect();
    // Any missing or ambiguous durable binding remains for operator review.
    if matching.len() != 1 {
        return Ok(MockReconciliationStatus::StillUnknown);
    }
    let attempt = matching[0];
    let Some(expected) = store.expected_metadata(job_id, &attempt.attempt_id)? else {
        return Ok(MockReconciliationStatus::StillUnknown);
    };
    if expected.operation_kind != "mock_create_new"
        || expected.target_connection_id != MOCK_CONNECTION_ID
        || expected.target_path != expected_target
        || expected.attempt_id != attempt.attempt_id
    {
        return Ok(MockReconciliationStatus::StillUnknown);
    }
    let Some(storage) = MockStorage::open_existing(storage_root)? else {
        return Ok(MockReconciliationStatus::StillUnknown);
    };
    let verifier = MockVerifier {
        storage: &storage,
        target_name: &expected.target_path,
        expected_size: expected.expected_size,
        expected_md5: &expected.expected_md5,
    };
    let Some(receipt) = verifier.verify(&attempt.attempt_id) else {
        return Ok(MockReconciliationStatus::StillUnknown);
    };
    store.complete_attempt(job_id, &attempt.attempt_id, &receipt)?;
    Ok(MockReconciliationStatus::Completed)
}

/// Execute one explicit Local Filesystem create-directory Job through the
/// durable Rust JobStore. The target is created with no-overwrite semantics.
/// This is deliberately not a content-copy controller: macOS content reads
/// remain fail-closed until residency protection has a proven safe boundary.
pub fn run_local_directory_create(
    database_path: &Path,
    registry_path: &Path,
    local_root: &Path,
    connection_id: &str,
    relative_path: &Path,
    job_id: &str,
) -> Result<LocalDirectoryCreateStatus, LocalDirectoryCreateError> {
    if !valid_job_id(job_id) || connection_id.is_empty() || relative_path.as_os_str().is_empty() {
        return Err(LocalDirectoryCreateError::InvalidRequest);
    }
    if !enabled_local_connection(registry_path, connection_id) {
        return Err(LocalDirectoryCreateError::ConnectionUnavailable);
    }
    let local = LocalFilesystem::open(local_root).map_err(LocalDirectoryCreateError::Local)?;
    if !local.capabilities().operations.create_directory {
        return Err(LocalDirectoryCreateError::CapabilityUnavailable);
    }
    let root_identity = local_root_identity_key(&local)?;
    let target_path = relative_path
        .to_str()
        .filter(|path| !path.is_empty())
        .ok_or(LocalDirectoryCreateError::InvalidRequest)?;
    let mut store = JobStore::open(database_path)?;
    if let Some(existing) = store.get(job_id)? {
        return Ok(LocalDirectoryCreateStatus::Existing(existing.state));
    }
    store.enqueue(job_id, None)?;
    store.advance(
        job_id,
        JobState::Queued,
        JobState::Running,
        Confirmation::Confirmed(EvidenceLevel::Requested),
    )?;
    let attempt_id = format!("{job_id}-attempt-1");
    // A directory has no content checksum, but JobStore requires a fixed
    // expectation shape. The empty-content MD5 is an immutable sentinel;
    // verification below proves directory type, not file-content integrity.
    store.prepare_bound_attempt_with_root_identity(
        job_id,
        &attempt_id,
        "local_mkdir_new",
        connection_id,
        target_path,
        0,
        "d41d8cd98f00b204e9800998ecf8427e",
        &root_identity,
    )?;
    // Disconnect may have raced with Job preparation. No I/O has begun, so
    // record the proven pre-effect refusal instead of touching the root.
    if !enabled_local_connection(registry_path, connection_id) {
        store.fail_attempt_before_effect(job_id, &attempt_id)?;
        return Err(LocalDirectoryCreateError::ConnectionUnavailable);
    }
    match local.create_directory(relative_path) {
        Ok(()) => {}
        Err(LocalError::AlreadyExists) => {
            store.fail_attempt_before_effect(job_id, &attempt_id)?;
            return Err(LocalDirectoryCreateError::TargetExists);
        }
        Err(error @ LocalError::OutcomeUnknown(_)) => {
            // Keep the active durable attempt. Reopening JobStore changes it to
            // UNKNOWN; no retry or cleanup is attempted by this controller.
            return Err(LocalDirectoryCreateError::OutcomeUnknown(error));
        }
        Err(error) => return Err(LocalDirectoryCreateError::Local(error)),
    }
    match local.stat(relative_path) {
        Ok(entry) if entry.is_directory => {}
        _ => return Err(LocalDirectoryCreateError::VerificationFailed),
    }
    let receipt = VerifiedReceipt::metadata_verified(
        &attempt_id,
        "local_mkdir_new",
        connection_id,
        target_path,
    );
    store.complete_attempt(job_id, &attempt_id, &receipt)?;
    Ok(LocalDirectoryCreateStatus::Completed)
}

/// Execute one Local-to-Local copy to a new destination. Reading content that
/// is necessary for this requested operation needs no separate consent. macOS
/// can materialize the selected File Provider item as part of that read.
#[allow(clippy::too_many_arguments)]
pub fn run_local_file_copy(
    database_path: &Path,
    registry_path: &Path,
    source_root: &Path,
    source_connection_id: &str,
    source_relative: &Path,
    destination_root: &Path,
    destination_connection_id: &str,
    destination_relative: &Path,
    job_id: &str,
) -> Result<LocalFileCopyStatus, LocalFileCopyError> {
    run_local_file_copy_inner(
        database_path,
        registry_path,
        source_root,
        source_connection_id,
        source_relative,
        destination_root,
        destination_connection_id,
        destination_relative,
        job_id,
        true,
        None,
        false,
        None,
    )
}

/// Execute one explicitly confirmed replacement of an existing Local file.
/// The confirmation covers replacement only. It does not authorize the
/// necessary source read.
#[allow(clippy::too_many_arguments)]
pub fn run_local_file_replace(
    database_path: &Path,
    registry_path: &Path,
    source_root: &Path,
    source_connection_id: &str,
    source_relative: &Path,
    destination_root: &Path,
    destination_connection_id: &str,
    destination_relative: &Path,
    job_id: &str,
    consent: bool,
) -> Result<LocalFileCopyStatus, LocalFileCopyError> {
    run_local_file_copy_inner(
        database_path,
        registry_path,
        source_root,
        source_connection_id,
        source_relative,
        destination_root,
        destination_connection_id,
        destination_relative,
        job_id,
        consent,
        None,
        true,
        None,
    )
}

/// Execute one Local-to-Local create proposed by an exact Scenario revision.
/// The immutable Scenario identity is stored with the Job. Necessary content
/// reads are ordinary operation work and need no separate confirmation.
#[allow(clippy::too_many_arguments)]
pub fn run_local_scenario_file_copy(
    database_path: &Path,
    registry_path: &Path,
    scenario_id: &str,
    scenario_revision: u64,
    source_root: &Path,
    source_connection_id: &str,
    source_relative: &Path,
    destination_root: &Path,
    destination_connection_id: &str,
    destination_relative: &Path,
    job_id: &str,
) -> Result<LocalFileCopyStatus, LocalFileCopyError> {
    let registry =
        Registry::open(registry_path).map_err(|_| LocalFileCopyError::ScenarioUnavailable)?;
    let scenario = registry
        .get_scenario_revision(scenario_id, scenario_revision)
        .map_err(|_| LocalFileCopyError::ScenarioUnavailable)?
        .ok_or(LocalFileCopyError::ScenarioUnavailable)?;
    let source_root = source_root
        .to_str()
        .ok_or(LocalFileCopyError::InvalidRequest)?;
    let destination_root = destination_root
        .to_str()
        .ok_or(LocalFileCopyError::InvalidRequest)?;
    let is_left_to_right = scenario.left_connection_id == source_connection_id
        && scenario.left_root == source_root
        && scenario.right_connection_id == destination_connection_id
        && scenario.right_root == destination_root;
    let is_right_to_left = scenario.direction == "bidirectional"
        && scenario.right_connection_id == source_connection_id
        && scenario.right_root == source_root
        && scenario.left_connection_id == destination_connection_id
        && scenario.left_root == destination_root;
    if !is_left_to_right && !is_right_to_left {
        return Err(LocalFileCopyError::ScenarioScopeMismatch);
    }
    require_selected_path(&registry, scenario_id, scenario_revision, source_relative)?;
    run_local_file_copy_inner(
        database_path,
        registry_path,
        Path::new(source_root),
        source_connection_id,
        source_relative,
        Path::new(destination_root),
        destination_connection_id,
        destination_relative,
        job_id,
        true,
        Some((scenario_id, scenario_revision, is_left_to_right)),
        false,
        None,
    )
}

/// Execute one explicitly confirmed replacement proposed by an exact Scenario
/// revision. A bidirectional Scenario may use either permitted direction. The
/// immutable Scenario endpoint binding is checked before the durable
/// replacement attempt is created.
#[allow(clippy::too_many_arguments)]
pub fn run_local_scenario_file_replace(
    database_path: &Path,
    registry_path: &Path,
    scenario_id: &str,
    scenario_revision: u64,
    source_root: &Path,
    source_connection_id: &str,
    source_relative: &Path,
    destination_root: &Path,
    destination_connection_id: &str,
    destination_relative: &Path,
    job_id: &str,
    consent: bool,
) -> Result<LocalFileCopyStatus, LocalFileCopyError> {
    run_local_scenario_file_replace_checked(
        database_path,
        registry_path,
        scenario_id,
        scenario_revision,
        source_root,
        source_connection_id,
        source_relative,
        destination_root,
        destination_connection_id,
        destination_relative,
        job_id,
        consent,
        None,
    )
}

/// Execute one confirmed replacement only if both files still match the
/// metadata the user saw in the plan. A change after planning returns
/// `ConflictStale` before any Job or attempt exists, so new data on either
/// side is never overwritten by an older consent.
#[allow(clippy::too_many_arguments)]
pub fn run_local_scenario_file_replace_planned(
    database_path: &Path,
    registry_path: &Path,
    scenario_id: &str,
    scenario_revision: u64,
    source_root: &Path,
    source_connection_id: &str,
    source_relative: &Path,
    destination_root: &Path,
    destination_connection_id: &str,
    destination_relative: &Path,
    job_id: &str,
    consent: bool,
    expected_source: &ConfirmedFileVersion,
    expected_destination: &ConfirmedFileVersion,
) -> Result<LocalFileCopyStatus, LocalFileCopyError> {
    run_local_scenario_file_replace_checked(
        database_path,
        registry_path,
        scenario_id,
        scenario_revision,
        source_root,
        source_connection_id,
        source_relative,
        destination_root,
        destination_connection_id,
        destination_relative,
        job_id,
        consent,
        Some((expected_source, expected_destination)),
    )
}

#[allow(clippy::too_many_arguments)]
fn run_local_scenario_file_replace_checked(
    database_path: &Path,
    registry_path: &Path,
    scenario_id: &str,
    scenario_revision: u64,
    source_root: &Path,
    source_connection_id: &str,
    source_relative: &Path,
    destination_root: &Path,
    destination_connection_id: &str,
    destination_relative: &Path,
    job_id: &str,
    consent: bool,
    expected_snapshots: Option<(&ConfirmedFileVersion, &ConfirmedFileVersion)>,
) -> Result<LocalFileCopyStatus, LocalFileCopyError> {
    let registry =
        Registry::open(registry_path).map_err(|_| LocalFileCopyError::ScenarioUnavailable)?;
    let scenario = registry
        .get_scenario_revision(scenario_id, scenario_revision)
        .map_err(|_| LocalFileCopyError::ScenarioUnavailable)?
        .ok_or(LocalFileCopyError::ScenarioUnavailable)?;
    let source_root = source_root
        .to_str()
        .ok_or(LocalFileCopyError::InvalidRequest)?;
    let destination_root = destination_root
        .to_str()
        .ok_or(LocalFileCopyError::InvalidRequest)?;
    let is_left_to_right = scenario.left_connection_id == source_connection_id
        && scenario.left_root == source_root
        && scenario.right_connection_id == destination_connection_id
        && scenario.right_root == destination_root;
    let is_right_to_left = scenario.direction == "bidirectional"
        && scenario.right_connection_id == source_connection_id
        && scenario.right_root == source_root
        && scenario.left_connection_id == destination_connection_id
        && scenario.left_root == destination_root;
    if !is_left_to_right && !is_right_to_left {
        return Err(LocalFileCopyError::ScenarioScopeMismatch);
    }
    require_selected_path(&registry, scenario_id, scenario_revision, source_relative)?;
    run_local_file_copy_inner(
        database_path,
        registry_path,
        Path::new(source_root),
        source_connection_id,
        source_relative,
        Path::new(destination_root),
        destination_connection_id,
        destination_relative,
        job_id,
        consent,
        Some((scenario_id, scenario_revision, is_left_to_right)),
        true,
        expected_snapshots,
    )
}

/// Execute the selected replacement decision from the independent conflict
/// queue. Both dry-run snapshots must still match before a Job can be created.
#[allow(clippy::too_many_arguments)]
pub fn run_local_scenario_conflict_replace(
    database_path: &Path,
    registry_path: &Path,
    conflict_id: &str,
    job_id: &str,
    consent: bool,
) -> Result<LocalFileCopyStatus, LocalFileCopyError> {
    let registry =
        Registry::open(registry_path).map_err(|_| LocalFileCopyError::ConflictUnavailable)?;
    let task = registry
        .selected_conflict_task(conflict_id)
        .map_err(|_| LocalFileCopyError::ConflictUnavailable)?
        .ok_or(LocalFileCopyError::ConflictUnavailable)?;
    if task.kind != "content"
        || !matches!(
            task.decision.as_deref(),
            Some("replace_from_source" | "replace_if_newer")
        )
    {
        return Err(LocalFileCopyError::ConflictUnavailable);
    }
    let (Some(source_is_left), Some(left), Some(right)) =
        (task.source_is_left, task.left.as_ref(), task.right.as_ref())
    else {
        return Err(LocalFileCopyError::ConflictUnavailable);
    };
    if let Err(error) = require_current_scenario_revision(
        registry_path,
        &task.scenario.scenario_id,
        task.scenario.revision,
    ) {
        if matches!(error, LocalFileCopyError::ScenarioRevisionStale) {
            // The task describes a superseded revision. Make it stale, so the
            // user builds a new task from the current plan.
            registry
                .mark_conflict_task_stale(conflict_id)
                .map_err(|_| LocalFileCopyError::ConflictUnavailable)?;
        }
        return Err(error);
    }
    let scenario = registry
        .get_scenario_revision(&task.scenario.scenario_id, task.scenario.revision)
        .map_err(|_| LocalFileCopyError::ConflictUnavailable)?
        .ok_or(LocalFileCopyError::ConflictUnavailable)?;
    require_selected_path(
        &registry,
        &task.scenario.scenario_id,
        task.scenario.revision,
        Path::new(&task.relative_path),
    )?;
    let (
        source_root,
        source_connection,
        destination_root,
        destination_connection,
        source_expected,
        destination_expected,
    ) = if source_is_left {
        (
            &scenario.left_root,
            &scenario.left_connection_id,
            &scenario.right_root,
            &scenario.right_connection_id,
            left,
            right,
        )
    } else {
        (
            &scenario.right_root,
            &scenario.right_connection_id,
            &scenario.left_root,
            &scenario.left_connection_id,
            right,
            left,
        )
    };
    let path = Path::new(&task.relative_path);
    let source =
        LocalFilesystem::open(Path::new(source_root)).map_err(LocalFileCopyError::Local)?;
    let destination =
        LocalFilesystem::open(Path::new(destination_root)).map_err(LocalFileCopyError::Local)?;
    let current_source = match source.stat(path) {
        Ok(entry) => entry,
        Err(LocalError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            registry
                .mark_conflict_task_stale(conflict_id)
                .map_err(|_| LocalFileCopyError::ConflictUnavailable)?;
            return Err(LocalFileCopyError::ConflictStale);
        }
        Err(error) => return Err(LocalFileCopyError::Local(error)),
    };
    let current_destination = match destination.stat(path) {
        Ok(entry) => entry,
        Err(LocalError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            registry
                .mark_conflict_task_stale(conflict_id)
                .map_err(|_| LocalFileCopyError::ConflictUnavailable)?;
            return Err(LocalFileCopyError::ConflictStale);
        }
        Err(error) => return Err(LocalFileCopyError::Local(error)),
    };
    if !matches_conflict_snapshot(&current_source, source_expected)
        || !matches_conflict_snapshot(&current_destination, destination_expected)
    {
        registry
            .mark_conflict_task_stale(conflict_id)
            .map_err(|_| LocalFileCopyError::ConflictUnavailable)?;
        return Err(LocalFileCopyError::ConflictStale);
    }
    if task.decision.as_deref() == Some("replace_if_newer") {
        let Some(source_is_newer) = source_snapshot_is_newer(source_expected, destination_expected)
        else {
            return Err(LocalFileCopyError::ConflictUnavailable);
        };
        if !source_is_newer {
            registry
                .complete_conflict_task(conflict_id)
                .map_err(|_| LocalFileCopyError::ConflictUnavailable)?;
            return Ok(LocalFileCopyStatus::Completed);
        }
    }
    let result = run_local_scenario_file_replace_checked(
        database_path,
        registry_path,
        &task.scenario.scenario_id,
        task.scenario.revision,
        Path::new(source_root),
        source_connection,
        path,
        Path::new(destination_root),
        destination_connection,
        path,
        job_id,
        consent,
        Some((source_expected, destination_expected)),
    );
    let result = match result {
        Err(LocalFileCopyError::ConflictStale) => {
            registry
                .mark_conflict_task_stale(conflict_id)
                .map_err(|_| LocalFileCopyError::ConflictUnavailable)?;
            return Err(LocalFileCopyError::ConflictStale);
        }
        result => result?,
    };
    if result == LocalFileCopyStatus::Completed {
        registry
            .complete_conflict_task(conflict_id)
            .map_err(|_| LocalFileCopyError::ConflictUnavailable)?;
    }
    Ok(result)
}

/// Execute an explicitly confirmed deletion decision from the conflict queue.
/// The selected source must still be absent and the destination must still
/// match its saved metadata. An interrupted delete remains UNKNOWN.
pub fn run_local_scenario_conflict_delete(
    database_path: &Path,
    registry_path: &Path,
    conflict_id: &str,
    job_id: &str,
    consent: bool,
) -> Result<LocalFileCopyStatus, LocalFileCopyError> {
    let registry =
        Registry::open(registry_path).map_err(|_| LocalFileCopyError::ConflictUnavailable)?;
    let task = registry
        .selected_conflict_task(conflict_id)
        .map_err(|_| LocalFileCopyError::ConflictUnavailable)?
        .ok_or(LocalFileCopyError::ConflictUnavailable)?;
    if task.kind != "deletion" || task.decision.as_deref() != Some("delete_as_source") || !consent {
        return Err(if consent {
            LocalFileCopyError::ConflictUnavailable
        } else {
            LocalFileCopyError::ConsentRequired
        });
    }
    let source_is_left = task
        .source_is_left
        .ok_or(LocalFileCopyError::ConflictUnavailable)?;
    if let Err(error) = require_current_scenario_revision(
        registry_path,
        &task.scenario.scenario_id,
        task.scenario.revision,
    ) {
        if matches!(error, LocalFileCopyError::ScenarioRevisionStale) {
            // The task describes a superseded revision. Make it stale, so the
            // user builds a new task from the current plan.
            registry
                .mark_conflict_task_stale(conflict_id)
                .map_err(|_| LocalFileCopyError::ConflictUnavailable)?;
        }
        return Err(error);
    }
    let scenario = registry
        .get_scenario_revision(&task.scenario.scenario_id, task.scenario.revision)
        .map_err(|_| LocalFileCopyError::ConflictUnavailable)?
        .ok_or(LocalFileCopyError::ConflictUnavailable)?;
    require_selected_path(
        &registry,
        &task.scenario.scenario_id,
        task.scenario.revision,
        Path::new(&task.relative_path),
    )?;
    let (
        source_root,
        source_connection,
        destination_root,
        destination_connection,
        source_expected,
        destination_expected,
    ) = if source_is_left {
        (
            &scenario.left_root,
            &scenario.left_connection_id,
            &scenario.right_root,
            &scenario.right_connection_id,
            task.left.as_ref(),
            task.right.as_ref(),
        )
    } else {
        (
            &scenario.right_root,
            &scenario.right_connection_id,
            &scenario.left_root,
            &scenario.left_connection_id,
            task.right.as_ref(),
            task.left.as_ref(),
        )
    };
    if source_expected.is_some()
        || destination_expected.is_none()
        || !enabled_local_connection(registry_path, source_connection)
        || !enabled_local_connection(registry_path, destination_connection)
    {
        return Err(LocalFileCopyError::ConflictUnavailable);
    }
    let source =
        LocalFilesystem::open(Path::new(source_root)).map_err(LocalFileCopyError::Local)?;
    let destination =
        LocalFilesystem::open(Path::new(destination_root)).map_err(LocalFileCopyError::Local)?;
    if !destination.capabilities().operations.delete {
        return Err(LocalFileCopyError::CapabilityUnavailable);
    }
    let path = Path::new(&task.relative_path);
    if !matches!(source.stat(path), Err(LocalError::Io(ref error)) if error.kind() == std::io::ErrorKind::NotFound)
    {
        registry
            .mark_conflict_task_stale(conflict_id)
            .map_err(|_| LocalFileCopyError::ConflictUnavailable)?;
        return Err(LocalFileCopyError::ConflictStale);
    }
    let current = match destination.stat(path) {
        Ok(entry) if matches_conflict_snapshot(&entry, destination_expected.unwrap()) => entry,
        Ok(_) | Err(LocalError::Io(_)) => {
            registry
                .mark_conflict_task_stale(conflict_id)
                .map_err(|_| LocalFileCopyError::ConflictUnavailable)?;
            return Err(LocalFileCopyError::ConflictStale);
        }
        Err(error) => return Err(LocalFileCopyError::Local(error)),
    };
    let target_path = path.to_str().ok_or(LocalFileCopyError::InvalidRequest)?;
    let root_identity =
        local_root_identity_key(&destination).map_err(|_| LocalFileCopyError::InvalidRequest)?;
    let mut store = JobStore::open(database_path)?;
    if let Some(existing) = store.get(job_id)? {
        return Ok(LocalFileCopyStatus::Existing(existing.state));
    }
    store.enqueue(
        job_id,
        Some((&task.scenario.scenario_id, task.scenario.revision)),
    )?;
    store.advance(
        job_id,
        JobState::Queued,
        JobState::Running,
        Confirmation::Unknown,
    )?;
    store.advance(
        job_id,
        JobState::Running,
        JobState::WaitingForDecision,
        Confirmation::Unknown,
    )?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| LocalFileCopyError::InvalidRequest)?
        .as_secs();
    let scope = OperationScope {
        operation: ProtectedOperation::Delete,
        connection_id: destination_connection.to_owned(),
        relative_path: target_path.to_owned(),
        expires_at: now.saturating_add(60),
    };
    let decision_id = format!("{job_id}-delete");
    store.record_operation_decision(&decision_id, job_id, &scope, Decision::Accepted)?;
    store.consume_operation_decision(&decision_id, job_id, &scope, now)?;
    store.advance(
        job_id,
        JobState::WaitingForDecision,
        JobState::Running,
        Confirmation::Confirmed(EvidenceLevel::Requested),
    )?;
    let attempt_id = format!("{job_id}-attempt-1");
    store.prepare_bound_attempt_with_root_identity(
        job_id,
        &attempt_id,
        "local_delete",
        destination_connection,
        target_path,
        current.size,
        "00000000000000000000000000000000",
        &root_identity,
    )?;
    match destination.delete_file(path) {
        Ok(()) => {}
        Err(error @ LocalError::OutcomeUnknown(_)) => {
            return Err(LocalFileCopyError::OutcomeUnknown(error))
        }
        Err(error) => {
            store.fail_attempt_before_effect(job_id, &attempt_id)?;
            return Err(LocalFileCopyError::Local(error));
        }
    }
    if !matches!(destination.stat(path), Err(LocalError::Io(ref error)) if error.kind() == std::io::ErrorKind::NotFound)
    {
        return Err(LocalFileCopyError::VerificationFailed);
    }
    let receipt = VerifiedReceipt::metadata_verified(
        &attempt_id,
        "local_delete",
        destination_connection,
        target_path,
    );
    store.complete_attempt(job_id, &attempt_id, &receipt)?;
    registry
        .complete_conflict_task(conflict_id)
        .map_err(|_| LocalFileCopyError::ConflictUnavailable)?;
    Ok(LocalFileCopyStatus::Completed)
}

/// Result of one deletion under the simple deletion rule of a revision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalRuleDeleteStatus {
    MovedToTrash,
    /// The volume has no usable Trash. The owner rule (D-019) allows a
    /// permanent delete in this case.
    DeletedPermanently,
    Existing(JobState),
}

/// Execute one deletion that the simple deletion rule of the current
/// revision covers. The caller must get `relative_path` from a plan that
/// marked it `PlanAction::Delete`; this function rechecks every condition:
/// the rule, the selection, the confirmed baseline, that the object is still
/// absent on the other side, and that it is unchanged on `on_left`'s side.
/// It moves the object into the Trash of its volume when possible. An
/// interrupted effect remains UNKNOWN and is never repeated.
pub fn run_local_scenario_rule_delete(
    database_path: &Path,
    registry_path: &Path,
    scenario_id: &str,
    scenario_revision: u64,
    relative_path: &str,
    on_left: bool,
    job_id: &str,
) -> Result<LocalRuleDeleteStatus, LocalFileCopyError> {
    if !valid_job_id(job_id) {
        return Err(LocalFileCopyError::InvalidRequest);
    }
    require_current_scenario_revision(registry_path, scenario_id, scenario_revision)?;
    let registry =
        Registry::open(registry_path).map_err(|_| LocalFileCopyError::ScenarioUnavailable)?;
    let scope = registry
        .get_scenario_revision_scope(scenario_id, scenario_revision)
        .map_err(|_| LocalFileCopyError::ScenarioUnavailable)?;
    if scope.deletion_mode != DeletionMode::Simple {
        return Err(LocalFileCopyError::ConsentRequired);
    }
    let path = Path::new(relative_path);
    require_selected_path(&registry, scenario_id, scenario_revision, path)?;
    let scenario = registry
        .get_scenario_revision(scenario_id, scenario_revision)
        .map_err(|_| LocalFileCopyError::ScenarioUnavailable)?
        .ok_or(LocalFileCopyError::ScenarioUnavailable)?;
    let reference = ScenarioReference {
        scenario_id: scenario_id.to_owned(),
        revision: scenario_revision,
    };
    let (target_root, target_connection, other_root, other_connection) = if on_left {
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
    if !enabled_local_connection(registry_path, target_connection)
        || !enabled_local_connection(registry_path, other_connection)
    {
        return Err(LocalFileCopyError::ConnectionUnavailable);
    }
    let baseline = registry
        .list_confirmed_file_states(&reference)
        .map_err(|_| LocalFileCopyError::ScenarioUnavailable)?
        .into_iter()
        .find(|state| state.relative_path == relative_path)
        .ok_or(LocalFileCopyError::ConflictStale)?;
    let (Some(confirmed_target), Some(_)) = (
        if on_left { &baseline.left } else { &baseline.right },
        if on_left { &baseline.right } else { &baseline.left },
    ) else {
        return Err(LocalFileCopyError::ConflictStale);
    };
    let other = LocalFilesystem::open(Path::new(other_root)).map_err(LocalFileCopyError::Local)?;
    if !matches!(other.stat(path), Err(LocalError::Io(ref error)) if error.kind() == std::io::ErrorKind::NotFound)
    {
        return Err(LocalFileCopyError::ConflictStale);
    }
    let target =
        LocalFilesystem::open(Path::new(target_root)).map_err(LocalFileCopyError::Local)?;
    if !target.capabilities().operations.delete {
        return Err(LocalFileCopyError::CapabilityUnavailable);
    }
    let current = match target.stat(path) {
        Ok(entry) if matches_conflict_snapshot(&entry, confirmed_target) => entry,
        Ok(_) | Err(LocalError::Io(_)) => return Err(LocalFileCopyError::ConflictStale),
        Err(error) => return Err(LocalFileCopyError::Local(error)),
    };
    let to_trash = target.capabilities().operations.trash
        && target
            .trash_available(path)
            .map_err(LocalFileCopyError::Local)?;
    let root_identity =
        local_root_identity_key(&target).map_err(|_| LocalFileCopyError::InvalidRequest)?;
    let mut store = JobStore::open(database_path)?;
    if let Some(existing) = store.get(job_id)? {
        return Ok(LocalRuleDeleteStatus::Existing(existing.state));
    }
    store.enqueue(job_id, Some((scenario_id, scenario_revision)))?;
    store.advance(
        job_id,
        JobState::Queued,
        JobState::Running,
        Confirmation::Unknown,
    )?;
    store.advance(
        job_id,
        JobState::Running,
        JobState::WaitingForDecision,
        Confirmation::Unknown,
    )?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| LocalFileCopyError::InvalidRequest)?
        .as_secs();
    let operation_scope = OperationScope {
        operation: ProtectedOperation::Delete,
        connection_id: target_connection.to_owned(),
        relative_path: relative_path.to_owned(),
        expires_at: now.saturating_add(60),
    };
    // The durable consent is the revision's simple deletion rule. This
    // records its exact use for this one path.
    let decision_id = format!("{job_id}-rule-delete");
    store.record_operation_decision(&decision_id, job_id, &operation_scope, Decision::Accepted)?;
    store.consume_operation_decision(&decision_id, job_id, &operation_scope, now)?;
    store.advance(
        job_id,
        JobState::WaitingForDecision,
        JobState::Running,
        Confirmation::Confirmed(EvidenceLevel::Requested),
    )?;
    let attempt_id = format!("{job_id}-attempt-1");
    let operation_kind = if to_trash {
        "local_rule_trash"
    } else {
        "local_rule_delete"
    };
    store.prepare_bound_attempt_with_root_identity(
        job_id,
        &attempt_id,
        operation_kind,
        target_connection,
        relative_path,
        current.size,
        "00000000000000000000000000000000",
        &root_identity,
    )?;
    let effect = if to_trash {
        match target.trash_file(path) {
            Ok(true) => Ok(()),
            // The Trash disappeared after the check. Nothing changed.
            Ok(false) => Err(LocalError::UnsafeEntry),
            Err(error) => Err(error),
        }
    } else {
        target.delete_file(path)
    };
    match effect {
        Ok(()) => {}
        Err(error @ LocalError::OutcomeUnknown(_)) => {
            return Err(LocalFileCopyError::OutcomeUnknown(error))
        }
        Err(error) => {
            store.fail_attempt_before_effect(job_id, &attempt_id)?;
            return Err(LocalFileCopyError::Local(error));
        }
    }
    if !matches!(target.stat(path), Err(LocalError::Io(ref error)) if error.kind() == std::io::ErrorKind::NotFound)
    {
        return Err(LocalFileCopyError::VerificationFailed);
    }
    let receipt =
        VerifiedReceipt::metadata_verified(&attempt_id, operation_kind, target_connection, relative_path);
    store.complete_attempt(job_id, &attempt_id, &receipt)?;
    registry
        .forget_confirmed_file_state(&reference, relative_path)
        .map_err(|_| LocalFileCopyError::ScenarioUnavailable)?;
    Ok(if to_trash {
        LocalRuleDeleteStatus::MovedToTrash
    } else {
        LocalRuleDeleteStatus::DeletedPermanently
    })
}

/// Result of a read-only check of an UNKNOWN rule deletion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalRuleDeleteReconciliationStatus {
    Completed,
    StillUnknown,
}

/// Read-only reconciliation of an UNKNOWN rule deletion. The object is gone
/// from its path only if the effect happened or someone else removed it; in
/// both cases the Scenario goal holds and the attempt completes. If the
/// object is still there, the Job stays UNKNOWN and nothing is repeated.
pub fn reconcile_local_rule_delete(
    database_path: &Path,
    registry_path: &Path,
    job_id: &str,
) -> Result<LocalRuleDeleteReconciliationStatus, LocalFileCopyError> {
    let still_unknown = Ok(LocalRuleDeleteReconciliationStatus::StillUnknown);
    let mut store = JobStore::open(database_path)?;
    let Some(job) = store.get(job_id)? else {
        return Err(LocalFileCopyError::InvalidRequest);
    };
    let (JobState::Unknown, Some(scenario_id), Some(revision)) =
        (job.state, job.scenario_id.clone(), job.scenario_revision)
    else {
        return still_unknown;
    };
    let Some(attempt) = store.unresolved_attempts()?.into_iter().find(|attempt| {
        attempt.job_id == job_id
            && matches!(
                attempt.operation_kind.as_deref(),
                Some("local_rule_trash" | "local_rule_delete")
            )
    }) else {
        return still_unknown;
    };
    let (Some(kind), Some(connection), Some(relative_path), Some(root_identity)) = (
        attempt.operation_kind.clone(),
        attempt.target_connection_id.clone(),
        attempt.target_path.clone(),
        store.attempt_root_identity(job_id, &attempt.attempt_id)?,
    ) else {
        return still_unknown;
    };
    let registry =
        Registry::open(registry_path).map_err(|_| LocalFileCopyError::ScenarioUnavailable)?;
    let scenario = registry
        .get_scenario_revision(&scenario_id, revision)
        .map_err(|_| LocalFileCopyError::ScenarioUnavailable)?
        .ok_or(LocalFileCopyError::ScenarioUnavailable)?;
    // Both sides can use one connection. The stored root identity selects
    // the side that the attempt opened.
    for root in [&scenario.left_root, &scenario.right_root] {
        let Ok(local) = LocalFilesystem::open(Path::new(root)) else {
            continue;
        };
        if local_root_identity_key(&local).ok().as_deref() != Some(root_identity.as_str()) {
            continue;
        }
        if !matches!(local.stat(Path::new(&relative_path)), Err(LocalError::Io(ref error)) if error.kind() == std::io::ErrorKind::NotFound)
        {
            return still_unknown;
        }
        let receipt =
            VerifiedReceipt::metadata_verified(&attempt.attempt_id, &kind, &connection, &relative_path);
        store.complete_attempt(job_id, &attempt.attempt_id, &receipt)?;
        registry
            .forget_confirmed_file_state(
                &ScenarioReference {
                    scenario_id,
                    revision,
                },
                &relative_path,
            )
            .map_err(|_| LocalFileCopyError::ScenarioUnavailable)?;
        return Ok(LocalRuleDeleteReconciliationStatus::Completed);
    }
    still_unknown
}

/// Refuse a Scenario effect on a path outside the revision's selection or on
/// a name variant of a selected object. An empty selection is the full root.
fn require_selected_path(
    registry: &Registry,
    scenario_id: &str,
    scenario_revision: u64,
    relative: &Path,
) -> Result<(), LocalFileCopyError> {
    let scope = registry
        .get_scenario_revision_scope(scenario_id, scenario_revision)
        .map_err(|_| LocalFileCopyError::ScenarioUnavailable)?;
    let path = relative.to_str().ok_or(LocalFileCopyError::InvalidRequest)?;
    if scope.classify(path, crate::scenario_planner::name_key) == ScopeMatch::Inside {
        Ok(())
    } else {
        Err(LocalFileCopyError::ScenarioScopeMismatch)
    }
}

#[allow(clippy::too_many_arguments)]
fn run_local_file_copy_inner(
    database_path: &Path,
    registry_path: &Path,
    source_root: &Path,
    source_connection_id: &str,
    source_relative: &Path,
    destination_root: &Path,
    destination_connection_id: &str,
    destination_relative: &Path,
    job_id: &str,
    consent: bool,
    scenario: Option<(&str, u64, bool)>,
    replace_existing: bool,
    expected_snapshots: Option<(&ConfirmedFileVersion, &ConfirmedFileVersion)>,
) -> Result<LocalFileCopyStatus, LocalFileCopyError> {
    if !valid_job_id(job_id)
        || source_connection_id.is_empty()
        || destination_connection_id.is_empty()
        || source_relative.as_os_str().is_empty()
        || destination_relative.as_os_str().is_empty()
        || (scenario.is_some() && source_relative != destination_relative)
    {
        return Err(LocalFileCopyError::InvalidRequest);
    }
    if replace_existing && !consent {
        return Err(LocalFileCopyError::ConsentRequired);
    }
    if !enabled_local_connection(registry_path, source_connection_id)
        || !enabled_local_connection(registry_path, destination_connection_id)
    {
        return Err(LocalFileCopyError::ConnectionUnavailable);
    }
    let source = LocalFilesystem::open(source_root).map_err(LocalFileCopyError::Local)?;
    let destination = LocalFilesystem::open(destination_root).map_err(LocalFileCopyError::Local)?;
    if !source.capabilities().operations.read_content
        || !destination.capabilities().operations.write_content
    {
        return Err(LocalFileCopyError::CapabilityUnavailable);
    }
    if source
        .overlaps_root(&destination)
        .map_err(LocalFileCopyError::Local)?
    {
        return Err(LocalFileCopyError::InvalidRequest);
    }
    let source_entry = source
        .stat(source_relative)
        .map_err(LocalFileCopyError::Local)?;
    if source_entry.is_directory {
        return Err(LocalFileCopyError::InvalidRequest);
    }
    let destination_entry = match destination.stat(destination_relative) {
        Ok(entry) if replace_existing && !entry.is_directory => Some(entry),
        Ok(_) if replace_existing => return Err(LocalFileCopyError::InvalidRequest),
        Ok(_) => return Err(LocalFileCopyError::TargetExists),
        Err(LocalError::Io(error))
            if !replace_existing && error.kind() == std::io::ErrorKind::NotFound =>
        {
            None
        }
        Err(error) => return Err(LocalFileCopyError::Local(error)),
    };
    if let Some((source_expected, destination_expected)) = expected_snapshots {
        if !matches_conflict_snapshot(&source_entry, source_expected)
            || !destination_entry
                .as_ref()
                .is_some_and(|entry| matches_conflict_snapshot(entry, destination_expected))
        {
            return Err(LocalFileCopyError::ConflictStale);
        }
    }
    let source_path = source_relative
        .to_str()
        .ok_or(LocalFileCopyError::InvalidRequest)?;
    let target_path = destination_relative
        .to_str()
        .ok_or(LocalFileCopyError::InvalidRequest)?;
    let root_identity = local_root_identity_key(&destination).map_err(|error| match error {
        LocalDirectoryCreateError::Local(error) => LocalFileCopyError::Local(error),
        LocalDirectoryCreateError::Store(error) => LocalFileCopyError::Store(error),
        _ => LocalFileCopyError::InvalidRequest,
    })?;
    let mut store = JobStore::open(database_path)?;
    if let Some(existing) = store.get(job_id)? {
        return Ok(LocalFileCopyStatus::Existing(existing.state));
    }
    store.enqueue(job_id, scenario.map(|(id, revision, _)| (id, revision)))?;
    store.advance(
        job_id,
        JobState::Queued,
        JobState::Running,
        Confirmation::Unknown,
    )?;
    let attempt_id = format!("{job_id}-attempt-1");
    let operation_kind = if replace_existing {
        "local_copy_replace"
    } else {
        "local_copy_new"
    };
    store.prepare_bound_attempt_with_root_identity(
        job_id,
        &attempt_id,
        operation_kind,
        destination_connection_id,
        target_path,
        source_entry.size,
        "00000000000000000000000000000000",
        &root_identity,
    )?;
    if !enabled_local_connection(registry_path, source_connection_id)
        || !enabled_local_connection(registry_path, destination_connection_id)
    {
        store.fail_attempt_before_effect(job_id, &attempt_id)?;
        return Err(LocalFileCopyError::ConnectionUnavailable);
    }
    // A Scenario file can be in a folder that does not exist yet in the
    // destination. Create only the missing parent folders, inside the
    // prepared attempt. Existing entries are never replaced. A failure
    // before any folder is created closes the attempt as not applied.
    if let Some(parent) = destination_relative
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
    let copy_result = if replace_existing {
        source.copy_file_to_replace(source_relative, &destination, destination_relative)
    } else {
        source.copy_file_to_new(source_relative, &destination, destination_relative)
    };
    match copy_result {
        Ok(bytes) if bytes == source_entry.size => {}
        Ok(_) => return Err(LocalFileCopyError::VerificationFailed),
        Err(LocalError::AlreadyExists) => {
            store.fail_attempt_before_effect(job_id, &attempt_id)?;
            return Err(LocalFileCopyError::TargetExists);
        }
        Err(error @ LocalError::OutcomeUnknown(_)) => {
            return Err(LocalFileCopyError::OutcomeUnknown(error))
        }
        Err(error) => return Err(LocalFileCopyError::Local(error)),
    }
    let destination_entry = match destination.stat(destination_relative) {
        Ok(entry) if !entry.is_directory && entry.size == source_entry.size => entry,
        _ => return Err(LocalFileCopyError::VerificationFailed),
    };
    let baseline = if let Some((scenario_id, scenario_revision, source_is_left)) = scenario {
        let source_after = source
            .stat(source_relative)
            .map_err(LocalFileCopyError::Local)?;
        if source_after.is_directory
            || source_after.object_revision != source_entry.object_revision
            || source_after.size != source_entry.size
        {
            return Err(LocalFileCopyError::VerificationFailed);
        }
        let source_version = ConfirmedFileVersion {
            object_revision: source_after.object_revision,
            size: source_after.size,
            checksum_md5: None,
        };
        let destination_version = ConfirmedFileVersion {
            object_revision: destination_entry.object_revision,
            size: destination_entry.size,
            checksum_md5: None,
        };
        Some(ScenarioCopyBaselineOutboxEntry {
            job_id: job_id.to_owned(),
            scenario: ScenarioReference {
                scenario_id: scenario_id.to_owned(),
                revision: scenario_revision,
            },
            relative_path: source_path.to_owned(),
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
        })
    } else {
        None
    };
    let receipt = VerifiedReceipt::metadata_verified(
        &attempt_id,
        operation_kind,
        destination_connection_id,
        target_path,
    );
    store.complete_attempt_with_scenario_copy_baseline(
        job_id,
        &attempt_id,
        &receipt,
        baseline.as_ref(),
    )?;
    if baseline.is_some() {
        // Completion is already durable. A failed Registry write leaves the
        // outbox pending for a later, idempotent local-only delivery.
        if let Ok(registry) = Registry::open(registry_path) {
            let _ = dispatch_pending_scenario_copy_baselines(&registry, &mut store);
        }
    }
    Ok(LocalFileCopyStatus::Completed)
}

/// Reconcile one UNKNOWN Local copy without reading content, writing, or
/// retrying. Completion requires the durable destination-root binding and an
/// existing regular file with the exact stored size.
pub fn reconcile_local_file_copy(
    database_path: &Path,
    registry_path: &Path,
    destination_root: &Path,
    destination_connection_id: &str,
    job_id: &str,
) -> Result<LocalFileCopyReconciliationStatus, LocalFileCopyError> {
    if !valid_job_id(job_id) || destination_connection_id.is_empty() {
        return Err(LocalFileCopyError::InvalidRequest);
    }
    if !enabled_local_connection(registry_path, destination_connection_id) {
        return Ok(LocalFileCopyReconciliationStatus::StillUnknown);
    }
    let destination = LocalFilesystem::open(destination_root).map_err(LocalFileCopyError::Local)?;
    let identity = destination
        .root_identity()
        .map_err(LocalFileCopyError::Local)?;
    let root_identity = format!("unix:{}:{}", identity.device, identity.inode);
    let mut store = JobStore::open(database_path)?;
    let Some(job) = store.get(job_id)? else {
        return Err(LocalFileCopyError::Store(JobStoreError::NotFound));
    };
    if job.state != JobState::Unknown {
        return Ok(LocalFileCopyReconciliationStatus::Existing(job.state));
    }
    let attempts = store.unresolved_attempts()?;
    let matching: Vec<_> = attempts
        .iter()
        .filter(|attempt| {
            attempt.job_id == job_id
                && attempt.status == "unknown"
                && is_local_file_copy_operation(attempt.operation_kind.as_deref())
                && attempt.target_connection_id.as_deref() == Some(destination_connection_id)
        })
        .collect();
    if matching.len() != 1 {
        return Ok(LocalFileCopyReconciliationStatus::StillUnknown);
    }
    let attempt = matching[0];
    let Some(expected) = store.expected_metadata(job_id, &attempt.attempt_id)? else {
        return Ok(LocalFileCopyReconciliationStatus::StillUnknown);
    };
    let Some(operation_kind) = attempt.operation_kind.as_deref() else {
        return Ok(LocalFileCopyReconciliationStatus::StillUnknown);
    };
    if expected.operation_kind != operation_kind
        || expected.target_connection_id != destination_connection_id
        || store
            .attempt_root_identity(job_id, &attempt.attempt_id)?
            .as_deref()
            != Some(root_identity.as_str())
    {
        return Ok(LocalFileCopyReconciliationStatus::StillUnknown);
    }
    if !matches!(destination.stat(Path::new(&expected.target_path)), Ok(entry) if !entry.is_directory && entry.size == expected.expected_size)
    {
        return Ok(LocalFileCopyReconciliationStatus::StillUnknown);
    }
    let receipt = VerifiedReceipt::metadata_verified(
        &attempt.attempt_id,
        operation_kind,
        destination_connection_id,
        &expected.target_path,
    );
    store.complete_attempt(job_id, &attempt.attempt_id, &receipt)?;
    Ok(LocalFileCopyReconciliationStatus::Completed)
}

/// Return the writer process id of a QADREN staging name:
/// `.qadren-<pid>-<serial>.tmp` or `.qadren-replace-<leaf>-<pid>-<serial>.stage`.
/// Any other name is not QADREN staging and is never touched.
fn staging_writer_pid(name: &str) -> Option<i32> {
    let rest = name.strip_prefix(".qadren-")?;
    let (body, replace) = if let Some(body) = rest.strip_suffix(".tmp") {
        (body, false)
    } else {
        (rest.strip_suffix(".stage")?, true)
    };
    let mut parts = body.rsplitn(3, '-');
    parts.next()?.parse::<u64>().ok()?;
    let pid = parts.next()?.parse::<i32>().ok()?;
    match (replace, parts.next()) {
        (false, None) => Some(pid),
        (true, Some(prefix)) if prefix.starts_with("replace-") => Some(pid),
        _ => None,
    }
}

/// True only when the process provably no longer exists. A live process,
/// including this one, keeps its staging file.
fn writer_process_is_gone(pid: i32) -> bool {
    if pid <= 0 || pid == std::process::id() as i32 {
        return false;
    }
    #[cfg(unix)]
    {
        // SAFETY: signal 0 only checks that the process exists.
        let result = unsafe { libc::kill(pid, 0) };
        result == -1 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
    }
    #[cfg(not(unix))]
    {
        false
    }
}

/// Remove staging files that an interrupted Local copy of this UNKNOWN Job
/// left next to its target. Only QADREN staging names in the target folder of
/// the bound destination root are removed, and only when their writer process
/// is gone. The target file, other files, and the Job state are not changed.
pub fn remove_interrupted_staging(
    database_path: &Path,
    destination_root: &Path,
    destination_connection_id: &str,
    job_id: &str,
) -> Result<usize, LocalFileCopyError> {
    if !valid_job_id(job_id) || destination_connection_id.is_empty() {
        return Err(LocalFileCopyError::InvalidRequest);
    }
    let destination = LocalFilesystem::open(destination_root).map_err(LocalFileCopyError::Local)?;
    let identity = destination
        .root_identity()
        .map_err(LocalFileCopyError::Local)?;
    let root_identity = format!("unix:{}:{}", identity.device, identity.inode);
    let store = JobStore::open(database_path)?;
    if store.get(job_id)?.map(|job| job.state) != Some(JobState::Unknown) {
        return Ok(0);
    }
    let mut removed = 0;
    for attempt in store.unresolved_attempts()?.iter().filter(|attempt| {
        attempt.job_id == job_id
            && attempt.status == "unknown"
            && is_local_file_copy_operation(attempt.operation_kind.as_deref())
            && attempt.target_connection_id.as_deref() == Some(destination_connection_id)
    }) {
        if store
            .attempt_root_identity(job_id, &attempt.attempt_id)?
            .as_deref()
            != Some(root_identity.as_str())
        {
            continue;
        }
        let Some(expected) = store.expected_metadata(job_id, &attempt.attempt_id)? else {
            continue;
        };
        let folder = Path::new(&expected.target_path)
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .to_path_buf();
        let Ok(entries) = destination.list(&folder) else {
            continue;
        };
        for entry in entries {
            if entry.is_directory
                || !staging_writer_pid(&entry.name).is_some_and(writer_process_is_gone)
            {
                continue;
            }
            if destination.delete_file(&folder.join(&entry.name)).is_ok() {
                removed += 1;
            }
        }
    }
    Ok(removed)
}

/// List UNKNOWN Local Filesystem copy Jobs bound to exactly this opened
/// destination root and Local connection. This does not inspect target paths,
/// read content, retry an effect, or complete a Job. Opening JobStore may
/// preserve its existing restart rule that turns an interrupted active attempt
/// into UNKNOWN before this inventory is returned.
pub fn local_file_copy_reconciliation_candidates(
    database_path: &Path,
    registry_path: &Path,
    destination_root: &Path,
    destination_connection_id: &str,
) -> Result<Vec<LocalFileCopyReconciliationCandidate>, LocalFileCopyError> {
    if destination_connection_id.is_empty() {
        return Err(LocalFileCopyError::InvalidRequest);
    }
    if !enabled_local_connection(registry_path, destination_connection_id) {
        return Err(LocalFileCopyError::ConnectionUnavailable);
    }
    let destination = LocalFilesystem::open(destination_root).map_err(LocalFileCopyError::Local)?;
    let identity = destination
        .root_identity()
        .map_err(LocalFileCopyError::Local)?;
    let root_identity = format!("unix:{}:{}", identity.device, identity.inode);
    let store = JobStore::open(database_path)?;
    let mut candidates = Vec::new();
    for attempt in store.unresolved_attempts()? {
        if attempt.status != "unknown"
            || !is_local_file_copy_operation(attempt.operation_kind.as_deref())
            || attempt.target_connection_id.as_deref() != Some(destination_connection_id)
            || store
                .attempt_root_identity(&attempt.job_id, &attempt.attempt_id)?
                .as_deref()
                != Some(root_identity.as_str())
        {
            continue;
        }
        candidates.push(LocalFileCopyReconciliationCandidate {
            job_id: attempt.job_id,
        });
    }
    candidates.sort_by(|left, right| left.job_id.cmp(&right.job_id));
    candidates.dedup_by(|left, right| left.job_id == right.job_id);
    Ok(candidates)
}

/// Reconcile an UNKNOWN Local Filesystem directory creation without writing,
/// deleting, or retrying. A different root, disabled connection, missing
/// binding, or mismatched target remains UNKNOWN.
pub fn reconcile_local_directory_create(
    database_path: &Path,
    registry_path: &Path,
    local_root: &Path,
    connection_id: &str,
    job_id: &str,
) -> Result<LocalDirectoryReconciliationStatus, LocalDirectoryCreateError> {
    if !valid_job_id(job_id) || connection_id.is_empty() {
        return Err(LocalDirectoryCreateError::InvalidRequest);
    }
    if !enabled_local_connection(registry_path, connection_id) {
        return Ok(LocalDirectoryReconciliationStatus::StillUnknown);
    }
    let local = LocalFilesystem::open(local_root).map_err(LocalDirectoryCreateError::Local)?;
    let root_identity = local_root_identity_key(&local)?;
    let mut store = JobStore::open(database_path)?;
    let Some(job) = store.get(job_id)? else {
        return Err(LocalDirectoryCreateError::Store(JobStoreError::NotFound));
    };
    if job.state != JobState::Unknown {
        return Ok(LocalDirectoryReconciliationStatus::Existing(job.state));
    }
    let attempts = store.unresolved_attempts()?;
    let matching: Vec<_> = attempts
        .iter()
        .filter(|attempt| {
            attempt.job_id == job_id
                && attempt.status == "unknown"
                && attempt.operation_kind.as_deref() == Some("local_mkdir_new")
                && attempt.target_connection_id.as_deref() == Some(connection_id)
        })
        .collect();
    if matching.len() != 1 {
        return Ok(LocalDirectoryReconciliationStatus::StillUnknown);
    }
    let attempt = matching[0];
    let Some(expected) = store.expected_metadata(job_id, &attempt.attempt_id)? else {
        return Ok(LocalDirectoryReconciliationStatus::StillUnknown);
    };
    if expected.operation_kind != "local_mkdir_new"
        || expected.target_connection_id != connection_id
        || expected.attempt_id != attempt.attempt_id
        || store
            .attempt_root_identity(job_id, &attempt.attempt_id)?
            .as_deref()
            != Some(root_identity.as_str())
    {
        return Ok(LocalDirectoryReconciliationStatus::StillUnknown);
    }
    let target = Path::new(&expected.target_path);
    if !matches!(local.stat(target), Ok(entry) if entry.is_directory) {
        return Ok(LocalDirectoryReconciliationStatus::StillUnknown);
    }
    let receipt = VerifiedReceipt::metadata_verified(
        &attempt.attempt_id,
        "local_mkdir_new",
        connection_id,
        &expected.target_path,
    );
    store.complete_attempt(job_id, &attempt.attempt_id, &receipt)?;
    Ok(LocalDirectoryReconciliationStatus::Completed)
}

/// List UNKNOWN Local Filesystem directory-create Jobs bound to exactly this
/// opened root and Local connection. This does not inspect target paths,
/// create directories, retry effects, or complete a Job. Opening JobStore may
/// preserve its existing restart rule that turns an interrupted active attempt
/// into UNKNOWN before this inventory is returned.
pub fn local_directory_reconciliation_candidates(
    database_path: &Path,
    registry_path: &Path,
    local_root: &Path,
    connection_id: &str,
) -> Result<Vec<LocalDirectoryReconciliationCandidate>, LocalDirectoryCreateError> {
    if connection_id.is_empty() {
        return Err(LocalDirectoryCreateError::InvalidRequest);
    }
    if !enabled_local_connection(registry_path, connection_id) {
        return Err(LocalDirectoryCreateError::ConnectionUnavailable);
    }
    let local = LocalFilesystem::open(local_root).map_err(LocalDirectoryCreateError::Local)?;
    let root_identity = local_root_identity_key(&local)?;
    let store = JobStore::open(database_path)?;
    let mut candidates = Vec::new();
    for attempt in store.unresolved_attempts()? {
        if attempt.status != "unknown"
            || attempt.operation_kind.as_deref() != Some("local_mkdir_new")
            || attempt.target_connection_id.as_deref() != Some(connection_id)
            || store
                .attempt_root_identity(&attempt.job_id, &attempt.attempt_id)?
                .as_deref()
                != Some(root_identity.as_str())
        {
            continue;
        }
        candidates.push(LocalDirectoryReconciliationCandidate {
            job_id: attempt.job_id,
        });
    }
    candidates.sort_by(|left, right| left.job_id.cmp(&right.job_id));
    candidates.dedup_by(|left, right| left.job_id == right.job_id);
    Ok(candidates)
}

fn run_mock_create_inner(
    database_path: &Path,
    storage_root: &Path,
    job_id: &str,
    consent: bool,
    #[allow(unused_variables)] fault: Option<MockFault>,
) -> Result<MockCreateStatus, MockCreateError> {
    if !consent {
        return Err(MockCreateError::ConsentRequired);
    }
    let target_name = target_name(job_id)?;
    let mut store = JobStore::open(database_path)?;
    if let Some(existing) = store.get(job_id)? {
        if existing.state != JobState::Queued {
            return Ok(MockCreateStatus::Existing(existing.state));
        }
    } else {
        store.enqueue(job_id, None)?;
    }

    let storage = MockStorage::open(storage_root)?;
    let expected_md5 = md5_hex(MOCK_CONTENT);
    store.advance(
        job_id,
        JobState::Queued,
        JobState::Running,
        Confirmation::Confirmed(EvidenceLevel::Requested),
    )?;
    let attempt_id = format!("{job_id}-attempt-1");
    store.prepare_bound_attempt(
        job_id,
        &attempt_id,
        "mock_create_new",
        MOCK_CONNECTION_ID,
        &target_name,
        MOCK_CONTENT.len() as u64,
        &expected_md5,
    )?;
    let written_content = if fault == Some(MockFault::PartialWrite) {
        &MOCK_CONTENT[..1]
    } else {
        MOCK_CONTENT
    };
    match storage.create_new(&target_name, written_content) {
        Ok(()) => {}
        Err(MockCreateError::OccupiedTarget) => {
            // `create_new` failed atomically before this attempt could publish
            // a destination. Persist the known no-effect outcome instead of
            // converting it to UNKNOWN on the next process open.
            store.fail_attempt_before_effect(job_id, &attempt_id)?;
            return Err(MockCreateError::OccupiedTarget);
        }
        Err(error) => return Err(error),
    }
    if fault == Some(MockFault::CrashAfterEffect) {
        // Deliberately return before resolution: reopening the durable store
        // classifies the active effect as UNKNOWN and forbids blind replay.
        return Err(MockCreateError::SimulatedCrashAfterEffect);
    }
    let verifier = MockVerifier {
        storage: &storage,
        target_name: &target_name,
        expected_size: MOCK_CONTENT.len() as u64,
        expected_md5: &expected_md5,
    };
    let receipt = verifier
        .verify(&attempt_id)
        .ok_or(MockCreateError::VerificationFailed)?;
    store.complete_attempt(job_id, &attempt_id, &receipt)?;
    Ok(MockCreateStatus::Completed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn fixture(name: &str) -> (PathBuf, PathBuf) {
        let base = std::env::temp_dir().join(format!(
            "qadren-g1-{name}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&base).unwrap();
        (base.join("jobs.sqlite3"), base.join("mock-storage"))
    }

    // `temp_dir()` may be a symlinked macOS path (`/var` → `/private/var`),
    // which the Local Filesystem adapter correctly rejects. Keep Local FS
    // controller fixtures under Cargo's physical target directory instead.
    fn local_fixture(name: &str) -> (PathBuf, PathBuf, PathBuf, String) {
        let base = std::env::current_dir()
            .unwrap()
            .join("target")
            .join(format!(
                "qadren-local-controller-{name}-{}",
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
        fs::create_dir_all(&base).unwrap();
        let registry_path = base.join("connections.sqlite3");
        let registry = Registry::open(&registry_path).unwrap();
        let connection = registry.add("local_filesystem", "Test Local").unwrap();
        (
            base.join("jobs.sqlite3"),
            registry_path,
            base.join("local-root"),
            connection,
        )
    }

    #[test]
    fn completes_no_overwrite_mock_create_with_verified_evidence() {
        let (database, storage) = fixture("complete");
        assert_eq!(
            run_mock_create(&database, &storage, "job-1", true).unwrap(),
            MockCreateStatus::Completed
        );
        assert_eq!(
            fs::read(storage.join("job-1.qadren-g1-test")).unwrap(),
            MOCK_CONTENT
        );
        let store = JobStore::open(&database).unwrap();
        assert_eq!(store.state("job-1").unwrap(), Some(JobState::Completed));
        assert!(store.unresolved_attempts().unwrap().is_empty());
    }

    #[test]
    fn consent_denial_has_no_store_or_content_effect() {
        let (database, storage) = fixture("consent");
        assert!(matches!(
            run_mock_create(&database, &storage, "job-1", false),
            Err(MockCreateError::ConsentRequired)
        ));
        assert!(!database.exists());
        assert!(!storage.exists());
    }

    #[test]
    fn occupied_destination_never_overwrites_bytes() {
        let (database, storage) = fixture("occupied");
        fs::create_dir(&storage).unwrap();
        let target = storage.join("job-1.qadren-g1-test");
        fs::write(&target, b"existing bytes").unwrap();
        let result = run_mock_create(&database, &storage, "job-1", true);
        assert!(
            matches!(result, Err(MockCreateError::OccupiedTarget)),
            "expected a known pre-effect collision, got {result:?}"
        );
        assert_eq!(fs::read(target).unwrap(), b"existing bytes");
        let store = JobStore::open(&database).unwrap();
        assert_eq!(store.state("job-1").unwrap(), Some(JobState::Failed));
        assert!(store.unresolved_attempts().unwrap().is_empty());
    }

    #[test]
    fn duplicate_request_is_observed_not_replayed() {
        let (database, storage) = fixture("duplicate");
        assert_eq!(
            run_mock_create(&database, &storage, "job-1", true).unwrap(),
            MockCreateStatus::Completed
        );
        assert_eq!(
            run_mock_create(&database, &storage, "job-1", true).unwrap(),
            MockCreateStatus::Existing(JobState::Completed)
        );
    }

    #[test]
    fn queued_mock_job_can_be_cancelled_without_storage_effect() {
        let (database, storage) = fixture("cancel");
        assert_eq!(
            queue_mock_create(&database, "job-1").unwrap(),
            MockCreateStatus::Queued
        );
        cancel_mock_create(&database, "job-1").unwrap();
        assert!(!storage.exists());
        let store = JobStore::open(&database).unwrap();
        assert_eq!(store.state("job-1").unwrap(), Some(JobState::Cancelled));
        drop(store);
        assert_eq!(
            run_mock_create(&database, &storage, "job-1", true).unwrap(),
            MockCreateStatus::Existing(JobState::Cancelled)
        );
        assert!(!storage.exists());
    }

    #[test]
    fn paused_mock_job_survives_resume_without_storage_effect() {
        let (database, storage) = fixture("pause");
        queue_mock_create(&database, "job-1").unwrap();
        pause_mock_create(&database, "job-1").unwrap();
        assert!(!storage.exists());
        let store = JobStore::open(&database).unwrap();
        assert_eq!(store.state("job-1").unwrap(), Some(JobState::Paused));
        drop(store);
        resume_mock_create(&database, "job-1").unwrap();
        assert!(!storage.exists());
        let store = JobStore::open(&database).unwrap();
        assert_eq!(store.state("job-1").unwrap(), Some(JobState::Queued));
    }

    #[test]
    fn crash_after_effect_reopens_as_unknown_and_does_not_replay() {
        let (database, storage) = fixture("crash");
        assert!(matches!(
            run_mock_create_inner(
                &database,
                &storage,
                "job-1",
                true,
                Some(MockFault::CrashAfterEffect)
            ),
            Err(MockCreateError::SimulatedCrashAfterEffect)
        ));
        assert_eq!(
            fs::read(storage.join("job-1.qadren-g1-test")).unwrap(),
            MOCK_CONTENT
        );
        let store = JobStore::open(&database).unwrap();
        assert_eq!(store.state("job-1").unwrap(), Some(JobState::Unknown));
        drop(store);
        assert_eq!(
            run_mock_create(&database, &storage, "job-1", true).unwrap(),
            MockCreateStatus::Existing(JobState::Unknown)
        );
    }

    #[test]
    fn partial_write_is_not_completed_and_reopens_as_unknown() {
        let (database, storage) = fixture("partial");
        assert!(matches!(
            run_mock_create_inner(
                &database,
                &storage,
                "job-1",
                true,
                Some(MockFault::PartialWrite)
            ),
            Err(MockCreateError::VerificationFailed)
        ));
        assert_eq!(
            fs::read(storage.join("job-1.qadren-g1-test")).unwrap(),
            &MOCK_CONTENT[..1]
        );
        let store = JobStore::open(&database).unwrap();
        assert_eq!(store.state("job-1").unwrap(), Some(JobState::Unknown));
        assert_eq!(store.unresolved_attempts().unwrap().len(), 1);
    }

    #[test]
    fn reconciliation_completes_only_exact_crash_after_effect_artifact() {
        let (database, storage) = fixture("reconcile-complete");
        assert!(matches!(
            run_mock_create_inner(
                &database,
                &storage,
                "job-1",
                true,
                Some(MockFault::CrashAfterEffect)
            ),
            Err(MockCreateError::SimulatedCrashAfterEffect)
        ));
        assert_eq!(
            reconcile_mock_create(&database, &storage, "job-1").unwrap(),
            MockReconciliationStatus::Completed
        );
        assert_eq!(
            fs::read(storage.join("job-1.qadren-g1-test")).unwrap(),
            MOCK_CONTENT
        );
        let store = JobStore::open(&database).unwrap();
        assert_eq!(store.state("job-1").unwrap(), Some(JobState::Completed));
        assert!(store.unresolved_attempts().unwrap().is_empty());
    }

    #[test]
    fn reconciliation_keeps_partial_or_missing_artifacts_unknown_without_writing() {
        let (database, storage) = fixture("reconcile-partial");
        assert!(matches!(
            run_mock_create_inner(
                &database,
                &storage,
                "job-1",
                true,
                Some(MockFault::PartialWrite)
            ),
            Err(MockCreateError::VerificationFailed)
        ));
        let target = storage.join("job-1.qadren-g1-test");
        let before = fs::read(&target).unwrap();
        assert_eq!(
            reconcile_mock_create(&database, &storage, "job-1").unwrap(),
            MockReconciliationStatus::StillUnknown
        );
        assert_eq!(fs::read(&target).unwrap(), before);
        let store = JobStore::open(&database).unwrap();
        assert_eq!(store.state("job-1").unwrap(), Some(JobState::Unknown));
        assert_eq!(store.unresolved_attempts().unwrap().len(), 1);
    }

    #[test]
    fn rejects_path_like_job_identifier_before_storage_access() {
        let (database, storage) = fixture("path");
        assert!(matches!(
            run_mock_create(&database, &storage, "../job", true),
            Err(MockCreateError::InvalidJobId)
        ));
        assert!(!database.exists());
        assert!(!storage.exists());
    }

    #[test]
    fn local_directory_controller_persists_and_verifies_no_overwrite_creation() {
        let (database, registry, root, connection) = local_fixture("complete");
        fs::create_dir(&root).unwrap();
        assert_eq!(
            run_local_directory_create(
                &database,
                &registry,
                &root,
                &connection,
                Path::new("created"),
                "local-mkdir-1",
            )
            .unwrap(),
            LocalDirectoryCreateStatus::Completed
        );
        assert!(root.join("created").is_dir());
        let store = JobStore::open(&database).unwrap();
        assert_eq!(
            store.state("local-mkdir-1").unwrap(),
            Some(JobState::Completed)
        );
        assert!(store.unresolved_attempts().unwrap().is_empty());
    }

    #[test]
    fn local_file_copy_reads_selected_source_without_consent_and_never_overwrites() {
        let (database, registry_path, source_root, source_connection) = local_fixture("copy");
        let destination_root = source_root.parent().unwrap().join("destination-root");
        fs::create_dir(&source_root).unwrap();
        fs::create_dir(&destination_root).unwrap();
        fs::write(source_root.join("source.txt"), b"copy me").unwrap();
        let registry = Registry::open(&registry_path).unwrap();
        let destination_connection = registry
            .add("local_filesystem", "Destination Local")
            .unwrap();

        assert_eq!(
            run_local_file_copy(
                &database,
                &registry_path,
                &source_root,
                &source_connection,
                Path::new("source.txt"),
                &destination_root,
                &destination_connection,
                Path::new("source.txt"),
                "local-copy-1",
            )
            .unwrap(),
            LocalFileCopyStatus::Completed
        );
        assert_eq!(
            fs::read(destination_root.join("source.txt")).unwrap(),
            b"copy me"
        );
        assert_eq!(
            fs::read(source_root.join("source.txt")).unwrap(),
            b"copy me"
        );
        assert_eq!(
            JobStore::open(&database)
                .unwrap()
                .state("local-copy-1")
                .unwrap(),
            Some(JobState::Completed)
        );
        assert!(matches!(
            run_local_file_copy(
                &database,
                &registry_path,
                &source_root,
                &source_connection,
                Path::new("source.txt"),
                &destination_root,
                &destination_connection,
                Path::new("source.txt"),
                "local-copy-2",
            ),
            Err(LocalFileCopyError::TargetExists)
        ));
        assert_eq!(
            fs::read(destination_root.join("source.txt")).unwrap(),
            b"copy me"
        );
    }

    #[test]
    fn local_file_replace_persists_attempt_and_replaces_existing_file() {
        let (database, registry_path, source_root, source_connection) = local_fixture("replace");
        let destination_root = source_root
            .parent()
            .unwrap()
            .join("replace-destination-root");
        fs::create_dir(&source_root).unwrap();
        fs::create_dir(&destination_root).unwrap();
        fs::write(source_root.join("source.txt"), b"new contents").unwrap();
        fs::write(destination_root.join("destination.txt"), b"old contents").unwrap();
        let registry = Registry::open(&registry_path).unwrap();
        let destination_connection = registry
            .add("local_filesystem", "Destination Local")
            .unwrap();

        assert!(matches!(
            run_local_file_replace(
                &database,
                &registry_path,
                &source_root,
                &source_connection,
                Path::new("source.txt"),
                &destination_root,
                &destination_connection,
                Path::new("destination.txt"),
                "local-replace-denied",
                false,
            ),
            Err(LocalFileCopyError::ConsentRequired)
        ));
        assert_eq!(
            fs::read(destination_root.join("destination.txt")).unwrap(),
            b"old contents"
        );

        assert_eq!(
            run_local_file_replace(
                &database,
                &registry_path,
                &source_root,
                &source_connection,
                Path::new("source.txt"),
                &destination_root,
                &destination_connection,
                Path::new("destination.txt"),
                "local-replace-1",
                true,
            )
            .unwrap(),
            LocalFileCopyStatus::Completed
        );
        assert_eq!(
            fs::read(destination_root.join("destination.txt")).unwrap(),
            b"new contents"
        );
        let store = JobStore::open(&database).unwrap();
        assert_eq!(
            store.state("local-replace-1").unwrap(),
            Some(JobState::Completed)
        );
        assert!(store.unresolved_attempts().unwrap().is_empty());
    }

    #[test]
    fn local_file_copy_creates_missing_destination_parents_without_unknown() {
        let (database, registry_path, source_root, source_connection) =
            local_fixture("copy-nested");
        let destination_root = source_root.parent().unwrap().join("nested-destination");
        fs::create_dir_all(source_root.join("sub/deeper")).unwrap();
        fs::create_dir(&destination_root).unwrap();
        fs::write(source_root.join("sub/deeper/c.txt"), b"nested").unwrap();
        fs::write(source_root.join("blocked.txt"), b"blocked").unwrap();
        let registry = Registry::open(&registry_path).unwrap();
        let destination_connection = registry
            .add("local_filesystem", "Nested Destination")
            .unwrap();

        assert_eq!(
            run_local_file_copy(
                &database,
                &registry_path,
                &source_root,
                &source_connection,
                Path::new("sub/deeper/c.txt"),
                &destination_root,
                &destination_connection,
                Path::new("sub/deeper/c.txt"),
                "local-copy-nested",
            )
            .unwrap(),
            LocalFileCopyStatus::Completed
        );
        assert_eq!(
            fs::read(destination_root.join("sub/deeper/c.txt")).unwrap(),
            b"nested"
        );

        // A file where a parent folder must be is a failure before any
        // effect: the Job fails and does not become UNKNOWN.
        fs::write(destination_root.join("file-parent"), b"keep").unwrap();
        assert!(matches!(
            run_local_file_copy(
                &database,
                &registry_path,
                &source_root,
                &source_connection,
                Path::new("blocked.txt"),
                &destination_root,
                &destination_connection,
                Path::new("file-parent/blocked.txt"),
                "local-copy-file-parent",
            ),
            Err(LocalFileCopyError::Local(_))
        ));
        assert_eq!(
            fs::read(destination_root.join("file-parent")).unwrap(),
            b"keep"
        );
        let state = JobStore::open(&database)
            .unwrap()
            .state("local-copy-file-parent")
            .unwrap();
        assert_ne!(state, Some(JobState::Unknown));
    }

    #[test]
    fn interrupted_staging_is_removed_only_for_a_dead_writer() {
        assert_eq!(staging_writer_pid(".qadren-123-4.tmp"), Some(123));
        assert_eq!(
            staging_writer_pid(".qadren-replace-report.txt-123-4.stage"),
            Some(123)
        );
        assert_eq!(staging_writer_pid(".qadren-notes.tmp"), None);
        assert_eq!(staging_writer_pid(".qadren-1-2-3.tmp"), None);
        assert_eq!(staging_writer_pid("qadren-123-4.tmp"), None);

        let (database, registry_path, source_root, _) = local_fixture("staging-cleanup");
        let destination_root = source_root.parent().unwrap().join("staging-destination");
        fs::create_dir_all(destination_root.join("sub")).unwrap();
        let registry = Registry::open(&registry_path).unwrap();
        let destination_connection = registry
            .add("local_filesystem", "Staging Destination")
            .unwrap();
        let destination = LocalFilesystem::open(&destination_root).unwrap();
        let identity = destination.root_identity().unwrap();
        let root_identity = format!("unix:{}:{}", identity.device, identity.inode);
        // PID 2147483646 cannot exist on macOS; this process is alive.
        let dead = "sub/.qadren-2147483646-7.tmp";
        let alive = format!("sub/.qadren-{}-8.tmp", std::process::id());
        for (path, bytes) in [
            (dead.to_owned(), b"partial".as_slice()),
            (alive.clone(), b"in use".as_slice()),
            ("sub/.qadren-notes.tmp".to_owned(), b"user file".as_slice()),
            ("sub/other.txt".to_owned(), b"other".as_slice()),
        ] {
            fs::write(destination_root.join(path), bytes).unwrap();
        }
        {
            let mut store = JobStore::open(&database).unwrap();
            store.enqueue("staging-job", None).unwrap();
            store
                .advance(
                    "staging-job",
                    JobState::Queued,
                    JobState::Running,
                    Confirmation::Unknown,
                )
                .unwrap();
            store
                .prepare_bound_attempt_with_root_identity(
                    "staging-job",
                    "staging-attempt",
                    "local_copy_new",
                    &destination_connection,
                    "sub/report.txt",
                    7,
                    "00000000000000000000000000000000",
                    &root_identity,
                )
                .unwrap();
        }
        // The next open applies the restart rule: the Job becomes UNKNOWN.
        assert_eq!(
            remove_interrupted_staging(
                &database,
                &destination_root,
                &destination_connection,
                "staging-job"
            )
            .unwrap(),
            1
        );
        assert!(!destination_root.join(dead).exists());
        assert!(destination_root.join(&alive).exists());
        assert!(destination_root.join("sub/.qadren-notes.tmp").exists());
        assert!(destination_root.join("sub/other.txt").exists());
        assert_eq!(
            JobStore::open(&database)
                .unwrap()
                .state("staging-job")
                .unwrap(),
            Some(JobState::Unknown)
        );
    }

    #[test]
    fn local_file_copy_reconciliation_requires_exact_destination_evidence() {
        let (database, registry_path, root, _) = local_fixture("copy-reconcile");
        let destination_root = root.parent().unwrap().join("copy-reconcile-destination");
        fs::create_dir(&destination_root).unwrap();
        let registry = Registry::open(&registry_path).unwrap();
        let connection = registry
            .add("local_filesystem", "Copy destination")
            .unwrap();
        let destination = LocalFilesystem::open(&destination_root).unwrap();
        let identity = destination.root_identity().unwrap();
        let root_identity = format!("unix:{}:{}", identity.device, identity.inode);
        let mut store = JobStore::open(&database).unwrap();
        store.enqueue("copy-unknown", None).unwrap();
        store
            .advance(
                "copy-unknown",
                JobState::Queued,
                JobState::Running,
                Confirmation::Confirmed(EvidenceLevel::Requested),
            )
            .unwrap();
        store
            .prepare_bound_attempt_with_root_identity(
                "copy-unknown",
                "copy-unknown-attempt",
                "local_copy_new",
                &connection,
                "copied.txt",
                7,
                "00000000000000000000000000000000",
                &root_identity,
            )
            .unwrap();
        drop(store);
        fs::write(destination_root.join("copied.txt"), b"payload").unwrap();
        assert_eq!(
            reconcile_local_file_copy(
                &database,
                &registry_path,
                &destination_root,
                &connection,
                "copy-unknown"
            )
            .unwrap(),
            LocalFileCopyReconciliationStatus::Completed
        );
        assert_eq!(
            JobStore::open(&database)
                .unwrap()
                .state("copy-unknown")
                .unwrap(),
            Some(JobState::Completed)
        );
    }

    #[test]
    fn local_file_replace_reconciliation_uses_the_replacement_attempt_kind() {
        let (database, registry_path, root, _) = local_fixture("replace-reconcile");
        let destination_root = root.parent().unwrap().join("replace-reconcile-destination");
        fs::create_dir(&destination_root).unwrap();
        let registry = Registry::open(&registry_path).unwrap();
        let connection = registry
            .add("local_filesystem", "Replacement destination")
            .unwrap();
        let destination = LocalFilesystem::open(&destination_root).unwrap();
        let identity = destination.root_identity().unwrap();
        let root_identity = format!("unix:{}:{}", identity.device, identity.inode);
        let mut store = JobStore::open(&database).unwrap();
        store.enqueue("replace-unknown", None).unwrap();
        store
            .advance(
                "replace-unknown",
                JobState::Queued,
                JobState::Running,
                Confirmation::Confirmed(EvidenceLevel::Requested),
            )
            .unwrap();
        store
            .prepare_bound_attempt_with_root_identity(
                "replace-unknown",
                "replace-unknown-attempt",
                "local_copy_replace",
                &connection,
                "replaced.txt",
                7,
                "00000000000000000000000000000000",
                &root_identity,
            )
            .unwrap();
        drop(store);
        fs::write(destination_root.join("replaced.txt"), b"payload").unwrap();

        assert_eq!(
            reconcile_local_file_copy(
                &database,
                &registry_path,
                &destination_root,
                &connection,
                "replace-unknown"
            )
            .unwrap(),
            LocalFileCopyReconciliationStatus::Completed
        );
        assert_eq!(
            JobStore::open(&database)
                .unwrap()
                .state("replace-unknown")
                .unwrap(),
            Some(JobState::Completed)
        );
    }

    #[test]
    fn local_file_copy_reconciliation_inventory_requires_the_bound_destination_root() {
        let (database, registry_path, root, _) = local_fixture("copy-reconcile-inventory");
        let destination_root = root
            .parent()
            .unwrap()
            .join("copy-reconcile-inventory-destination");
        let other_root = root
            .parent()
            .unwrap()
            .join("copy-reconcile-inventory-other");
        fs::create_dir(&destination_root).unwrap();
        fs::create_dir(&other_root).unwrap();
        let registry = Registry::open(&registry_path).unwrap();
        let connection = registry
            .add("local_filesystem", "Copy destination")
            .unwrap();
        let destination = LocalFilesystem::open(&destination_root).unwrap();
        let identity = destination.root_identity().unwrap();
        let root_identity = format!("unix:{}:{}", identity.device, identity.inode);
        let mut store = JobStore::open(&database).unwrap();
        store.enqueue("copy-inventory-unknown", None).unwrap();
        store
            .advance(
                "copy-inventory-unknown",
                JobState::Queued,
                JobState::Running,
                Confirmation::Confirmed(EvidenceLevel::Requested),
            )
            .unwrap();
        store
            .prepare_bound_attempt_with_root_identity(
                "copy-inventory-unknown",
                "copy-inventory-unknown-attempt",
                "local_copy_new",
                &connection,
                "copied.txt",
                7,
                "00000000000000000000000000000000",
                &root_identity,
            )
            .unwrap();
        drop(store);

        assert_eq!(
            local_file_copy_reconciliation_candidates(
                &database,
                &registry_path,
                &destination_root,
                &connection,
            )
            .unwrap(),
            vec![LocalFileCopyReconciliationCandidate {
                job_id: "copy-inventory-unknown".to_owned(),
            }]
        );
        assert!(local_file_copy_reconciliation_candidates(
            &database,
            &registry_path,
            &other_root,
            &connection,
        )
        .unwrap()
        .is_empty());
    }

    #[test]
    fn scenario_copy_binds_the_exact_revision_and_endpoint_scope() {
        use crate::scenario_planner::{ObservedFile, PlanAction};
        let (database, registry_path, source_root, source_connection) =
            local_fixture("scenario-copy");
        let destination_root = source_root
            .parent()
            .unwrap()
            .join("scenario-destination-root");
        fs::create_dir(&source_root).unwrap();
        fs::create_dir(&destination_root).unwrap();
        fs::write(source_root.join("source.txt"), b"copy me").unwrap();
        let registry = Registry::open(&registry_path).unwrap();
        let destination_connection = registry
            .add("local_filesystem", "Scenario Destination")
            .unwrap();
        let scenario = registry
            .create_scenario(
                "Scenario copy",
                &source_connection,
                source_root.to_str().unwrap(),
                &destination_connection,
                destination_root.to_str().unwrap(),
                "left_to_right",
            )
            .unwrap();
        assert_eq!(
            run_local_scenario_file_copy(
                &database,
                &registry_path,
                &scenario,
                1,
                &source_root,
                &source_connection,
                Path::new("source.txt"),
                &destination_root,
                &destination_connection,
                Path::new("source.txt"),
                "scenario-copy-1",
            )
            .unwrap(),
            LocalFileCopyStatus::Completed
        );
        let stored = JobStore::open(&database)
            .unwrap()
            .get("scenario-copy-1")
            .unwrap()
            .unwrap();
        assert_eq!(stored.scenario_id.as_deref(), Some(scenario.as_str()));
        assert_eq!(stored.scenario_revision, Some(1));
        let baseline = Registry::open(&registry_path)
            .unwrap()
            .list_confirmed_file_states(&ScenarioReference {
                scenario_id: scenario.clone(),
                revision: 1,
            })
            .unwrap();
        assert_eq!(baseline.len(), 1);
        assert_eq!(baseline[0].relative_path, "source.txt");
        assert_eq!(baseline[0].left.as_ref().unwrap().size, 7);
        assert_eq!(baseline[0].right.as_ref().unwrap().size, 7);
        assert!(baseline[0].left.as_ref().unwrap().object_revision.is_some());
        assert!(baseline[0]
            .right
            .as_ref()
            .unwrap()
            .object_revision
            .is_some());
        assert!(JobStore::open(&database)
            .unwrap()
            .pending_scenario_copy_baseline_outbox()
            .unwrap()
            .is_empty());
        let source_entry = LocalFilesystem::open(&source_root)
            .unwrap()
            .stat(Path::new("source.txt"))
            .unwrap();
        let destination_entry = LocalFilesystem::open(&destination_root)
            .unwrap()
            .stat(Path::new("source.txt"))
            .unwrap();
        let plan = Registry::open(&registry_path)
            .unwrap()
            .plan_scenario_dry_run(
                &ScenarioReference {
                    scenario_id: scenario.clone(),
                    revision: 1,
                },
                &[ObservedFile {
                    relative_path: "source.txt".into(),
                    object_revision: source_entry.object_revision,
                    size: source_entry.size,
                    checksum_md5: None,
                }],
                &[ObservedFile {
                    relative_path: "source.txt".into(),
                    object_revision: destination_entry.object_revision,
                    size: destination_entry.size,
                    checksum_md5: None,
                }],
            )
            .unwrap();
        assert_eq!(plan[0].action, PlanAction::NoAction);
        assert!(matches!(
            run_local_scenario_file_copy(
                &database,
                &registry_path,
                &scenario,
                1,
                &destination_root,
                &source_connection,
                Path::new("source.txt"),
                &source_root,
                &destination_connection,
                Path::new("other.txt"),
                "scenario-copy-bad",
            ),
            Err(LocalFileCopyError::ScenarioScopeMismatch)
        ));
    }

    #[test]
    fn scenario_replace_binds_the_exact_revision_and_replaces_existing_file() {
        let (database, registry_path, source_root, source_connection) =
            local_fixture("scenario-replace");
        let destination_root = source_root
            .parent()
            .unwrap()
            .join("scenario-replace-destination-root");
        fs::create_dir(&source_root).unwrap();
        fs::create_dir(&destination_root).unwrap();
        fs::write(source_root.join("report.txt"), b"new report").unwrap();
        fs::write(destination_root.join("report.txt"), b"old report").unwrap();
        let registry = Registry::open(&registry_path).unwrap();
        let destination_connection = registry
            .add("local_filesystem", "Scenario Destination")
            .unwrap();
        let scenario = registry
            .create_scenario(
                "Scenario update",
                &source_connection,
                source_root.to_str().unwrap(),
                &destination_connection,
                destination_root.to_str().unwrap(),
                "left_to_right",
            )
            .unwrap();

        assert_eq!(
            run_local_scenario_file_replace(
                &database,
                &registry_path,
                &scenario,
                1,
                &source_root,
                &source_connection,
                Path::new("report.txt"),
                &destination_root,
                &destination_connection,
                Path::new("report.txt"),
                "scenario-replace-1",
                true,
            )
            .unwrap(),
            LocalFileCopyStatus::Completed
        );
        assert_eq!(
            fs::read(destination_root.join("report.txt")).unwrap(),
            b"new report"
        );
        let stored = JobStore::open(&database)
            .unwrap()
            .get("scenario-replace-1")
            .unwrap()
            .unwrap();
        assert_eq!(stored.scenario_id.as_deref(), Some(scenario.as_str()));
        assert_eq!(stored.scenario_revision, Some(1));
    }

    #[test]
    fn selected_conflict_replace_revalidates_snapshots_and_completes_its_task() {
        let (database, registry_path, source_root, source_connection) =
            local_fixture("conflict-replace");
        let destination_root = source_root
            .parent()
            .unwrap()
            .join("conflict-replace-destination");
        fs::create_dir(&source_root).unwrap();
        fs::create_dir(&destination_root).unwrap();
        fs::write(source_root.join("report.txt"), b"new report").unwrap();
        fs::write(destination_root.join("report.txt"), b"old report").unwrap();
        let registry = Registry::open(&registry_path).unwrap();
        let destination_connection = registry.add("local_filesystem", "Destination").unwrap();
        let scenario_id = registry
            .create_scenario(
                "Conflict",
                &source_connection,
                source_root.to_str().unwrap(),
                &destination_connection,
                destination_root.to_str().unwrap(),
                "left_to_right",
            )
            .unwrap();
        let source = LocalFilesystem::open(&source_root)
            .unwrap()
            .stat(Path::new("report.txt"))
            .unwrap();
        let destination = LocalFilesystem::open(&destination_root)
            .unwrap()
            .stat(Path::new("report.txt"))
            .unwrap();
        let task = registry
            .queue_conflict_task(
                &ScenarioReference {
                    scenario_id: scenario_id.clone(),
                    revision: 1,
                },
                "report.txt",
                "content",
                None,
                Some(&ConfirmedFileVersion {
                    object_revision: source.object_revision,
                    size: source.size,
                    checksum_md5: None,
                }),
                Some(&ConfirmedFileVersion {
                    object_revision: destination.object_revision,
                    size: destination.size,
                    checksum_md5: None,
                }),
            )
            .unwrap();
        registry
            .select_conflict_task(&task, Some(true), "replace_from_source")
            .unwrap();
        assert_eq!(
            run_local_scenario_conflict_replace(
                &database,
                &registry_path,
                &task,
                "conflict-replace-1",
                true
            )
            .unwrap(),
            LocalFileCopyStatus::Completed
        );
        assert_eq!(
            fs::read(destination_root.join("report.txt")).unwrap(),
            b"new report"
        );
        assert_eq!(registry.selected_conflict_task(&task).unwrap(), None);
    }

    #[test]
    fn selected_conflict_replace_if_newer_uses_local_snapshot_mtime() {
        let (database, registry_path, source_root, source_connection) =
            local_fixture("conflict-replace-newer");
        let destination_root = source_root
            .parent()
            .unwrap()
            .join("conflict-replace-newer-destination");
        fs::create_dir(&source_root).unwrap();
        fs::create_dir(&destination_root).unwrap();
        fs::write(destination_root.join("report.txt"), b"old report").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(10));
        fs::write(source_root.join("report.txt"), b"new report").unwrap();
        let registry = Registry::open(&registry_path).unwrap();
        let destination_connection = registry.add("local_filesystem", "Destination").unwrap();
        let scenario_id = registry
            .create_scenario(
                "Conflict",
                &source_connection,
                source_root.to_str().unwrap(),
                &destination_connection,
                destination_root.to_str().unwrap(),
                "left_to_right",
            )
            .unwrap();
        let source = LocalFilesystem::open(&source_root)
            .unwrap()
            .stat(Path::new("report.txt"))
            .unwrap();
        let destination = LocalFilesystem::open(&destination_root)
            .unwrap()
            .stat(Path::new("report.txt"))
            .unwrap();
        let task = registry
            .queue_conflict_task(
                &ScenarioReference {
                    scenario_id,
                    revision: 1,
                },
                "report.txt",
                "content",
                None,
                Some(&ConfirmedFileVersion {
                    object_revision: source.object_revision,
                    size: source.size,
                    checksum_md5: None,
                }),
                Some(&ConfirmedFileVersion {
                    object_revision: destination.object_revision,
                    size: destination.size,
                    checksum_md5: None,
                }),
            )
            .unwrap();
        registry
            .select_conflict_task(&task, Some(true), "replace_if_newer")
            .unwrap();

        assert_eq!(
            run_local_scenario_conflict_replace(
                &database,
                &registry_path,
                &task,
                "conflict-replace-newer-1",
                true
            )
            .unwrap(),
            LocalFileCopyStatus::Completed
        );
        assert_eq!(
            fs::read(destination_root.join("report.txt")).unwrap(),
            b"new report"
        );
        assert_eq!(registry.selected_conflict_task(&task).unwrap(), None);
    }

    #[test]
    fn selected_conflict_replace_if_newer_keeps_a_newer_destination_without_a_job() {
        let (database, registry_path, source_root, source_connection) =
            local_fixture("conflict-keep-newer-destination");
        let destination_root = source_root
            .parent()
            .unwrap()
            .join("conflict-keep-newer-destination-root");
        fs::create_dir(&source_root).unwrap();
        fs::create_dir(&destination_root).unwrap();
        fs::write(source_root.join("report.txt"), b"source report").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(10));
        fs::write(destination_root.join("report.txt"), b"newer destination").unwrap();
        let registry = Registry::open(&registry_path).unwrap();
        let destination_connection = registry.add("local_filesystem", "Destination").unwrap();
        let scenario_id = registry
            .create_scenario(
                "Conflict",
                &source_connection,
                source_root.to_str().unwrap(),
                &destination_connection,
                destination_root.to_str().unwrap(),
                "left_to_right",
            )
            .unwrap();
        let source = LocalFilesystem::open(&source_root)
            .unwrap()
            .stat(Path::new("report.txt"))
            .unwrap();
        let destination = LocalFilesystem::open(&destination_root)
            .unwrap()
            .stat(Path::new("report.txt"))
            .unwrap();
        let task = registry
            .queue_conflict_task(
                &ScenarioReference {
                    scenario_id,
                    revision: 1,
                },
                "report.txt",
                "content",
                None,
                Some(&ConfirmedFileVersion {
                    object_revision: source.object_revision,
                    size: source.size,
                    checksum_md5: None,
                }),
                Some(&ConfirmedFileVersion {
                    object_revision: destination.object_revision,
                    size: destination.size,
                    checksum_md5: None,
                }),
            )
            .unwrap();
        registry
            .select_conflict_task(&task, Some(true), "replace_if_newer")
            .unwrap();

        assert_eq!(
            run_local_scenario_conflict_replace(
                &database,
                &registry_path,
                &task,
                "conflict-keep-newer-destination-1",
                true
            )
            .unwrap(),
            LocalFileCopyStatus::Completed
        );
        assert_eq!(
            fs::read(destination_root.join("report.txt")).unwrap(),
            b"newer destination"
        );
        assert_eq!(
            JobStore::open(&database)
                .unwrap()
                .get("conflict-keep-newer-destination-1")
                .unwrap(),
            None
        );
        assert_eq!(registry.selected_conflict_task(&task).unwrap(), None);
    }

    #[test]
    fn selected_conflict_delete_removes_the_destination_after_confirmation() {
        let (database, registry_path, left_root, left_connection) =
            local_fixture("conflict-delete");
        let right_root = left_root.parent().unwrap().join("conflict-delete-right");
        fs::create_dir(&left_root).unwrap();
        fs::create_dir(&right_root).unwrap();
        fs::write(right_root.join("report.txt"), b"remove me").unwrap();
        let registry = Registry::open(&registry_path).unwrap();
        let right_connection = registry.add("local_filesystem", "Right").unwrap();
        let scenario_id = registry
            .create_scenario(
                "Conflict",
                &left_connection,
                left_root.to_str().unwrap(),
                &right_connection,
                right_root.to_str().unwrap(),
                "left_to_right",
            )
            .unwrap();
        let destination = LocalFilesystem::open(&right_root)
            .unwrap()
            .stat(Path::new("report.txt"))
            .unwrap();
        let task = registry
            .queue_conflict_task(
                &ScenarioReference {
                    scenario_id,
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
            .select_conflict_task(&task, Some(true), "delete_as_source")
            .unwrap();

        assert_eq!(
            run_local_scenario_conflict_delete(
                &database,
                &registry_path,
                &task,
                "conflict-delete-1",
                true
            )
            .unwrap(),
            LocalFileCopyStatus::Completed
        );
        assert!(!right_root.join("report.txt").exists());
        assert_eq!(registry.selected_conflict_task(&task).unwrap(), None);
        assert_eq!(
            JobStore::open(&database)
                .unwrap()
                .state("conflict-delete-1")
                .unwrap(),
            Some(JobState::Completed)
        );
    }

    /// A Scenario whose left side deleted `report.txt` after a confirmed
    /// synchronization. Returns the paths, the Scenario ID and a test Trash.
    #[cfg(target_os = "macos")]
    fn rule_delete_fixture(
        name: &str,
        scope: &crate::model::ScenarioScope,
    ) -> (PathBuf, PathBuf, PathBuf, String, PathBuf) {
        let (database, registry_path, left_root, left_connection) = local_fixture(name);
        let right_root = left_root.parent().unwrap().join(format!("{name}-right"));
        let trash = left_root.parent().unwrap().join(format!("{name}-trash"));
        fs::create_dir(&left_root).unwrap();
        fs::create_dir(&right_root).unwrap();
        fs::create_dir(&trash).unwrap();
        fs::write(right_root.join("report.txt"), b"synchronized").unwrap();
        let registry = Registry::open(&registry_path).unwrap();
        let right_connection = registry.add("local_filesystem", "Right").unwrap();
        let scenario_id = registry
            .create_scenario_with_scope(
                "Rule",
                &left_connection,
                left_root.to_str().unwrap(),
                &right_connection,
                right_root.to_str().unwrap(),
                "left_to_right",
                scope,
            )
            .unwrap();
        let entry = LocalFilesystem::open(&right_root)
            .unwrap()
            .stat(Path::new("report.txt"))
            .unwrap();
        let version = ConfirmedFileVersion {
            object_revision: entry.object_revision,
            size: entry.size,
            checksum_md5: None,
        };
        registry
            .record_confirmed_file_state(
                &crate::model::FileSyncState {
                    scenario: ScenarioReference {
                        scenario_id: scenario_id.clone(),
                        revision: 1,
                    },
                    relative_path: "report.txt".into(),
                    left: Some(version.clone()),
                    right: Some(version),
                },
                Confirmation::Confirmed(EvidenceLevel::MetadataVerified),
            )
            .unwrap();
        crate::local_fs::tests::TEST_TRASH.with(|value| *value.borrow_mut() = Some(trash.clone()));
        (database, registry_path, right_root, scenario_id, trash)
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn simple_rule_moves_the_deleted_object_to_the_trash_and_forgets_its_baseline() {
        let scope = crate::model::ScenarioScope {
            selected_paths: vec!["report.txt".into()],
            deletion_mode: DeletionMode::Simple,
        };
        let (database, registry_path, right_root, scenario_id, trash) =
            rule_delete_fixture("rule-delete", &scope);
        assert_eq!(
            run_local_scenario_rule_delete(
                &database,
                &registry_path,
                &scenario_id,
                1,
                "report.txt",
                false,
                "rule-delete-1",
            )
            .unwrap(),
            LocalRuleDeleteStatus::MovedToTrash
        );
        assert!(!right_root.join("report.txt").exists());
        assert_eq!(fs::read(trash.join("report.txt")).unwrap(), b"synchronized");
        assert_eq!(
            JobStore::open(&database)
                .unwrap()
                .state("rule-delete-1")
                .unwrap(),
            Some(JobState::Completed)
        );
        let reference = ScenarioReference {
            scenario_id,
            revision: 1,
        };
        assert!(Registry::open(&registry_path)
            .unwrap()
            .list_confirmed_file_states(&reference)
            .unwrap()
            .is_empty());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn rule_delete_is_refused_without_the_simple_rule_or_outside_the_selection() {
        let paranoid = crate::model::ScenarioScope::default();
        let (database, registry_path, right_root, scenario_id, _) =
            rule_delete_fixture("rule-delete-paranoid", &paranoid);
        assert!(matches!(
            run_local_scenario_rule_delete(
                &database,
                &registry_path,
                &scenario_id,
                1,
                "report.txt",
                false,
                "rule-delete-2",
            ),
            Err(LocalFileCopyError::ConsentRequired)
        ));
        assert!(right_root.join("report.txt").exists());

        let elsewhere = crate::model::ScenarioScope {
            selected_paths: vec!["Docs".into()],
            deletion_mode: DeletionMode::Simple,
        };
        let (database, registry_path, right_root, scenario_id, _) =
            rule_delete_fixture("rule-delete-outside", &elsewhere);
        assert!(matches!(
            run_local_scenario_rule_delete(
                &database,
                &registry_path,
                &scenario_id,
                1,
                "report.txt",
                false,
                "rule-delete-3",
            ),
            Err(LocalFileCopyError::ScenarioScopeMismatch)
        ));
        assert!(right_root.join("report.txt").exists());
        assert_eq!(JobStore::open(&database).unwrap().state("rule-delete-3").unwrap(), None);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn rule_delete_never_removes_an_object_changed_after_synchronization() {
        let scope = crate::model::ScenarioScope {
            selected_paths: Vec::new(),
            deletion_mode: DeletionMode::Simple,
        };
        let (database, registry_path, right_root, scenario_id, _) =
            rule_delete_fixture("rule-delete-changed", &scope);
        fs::write(right_root.join("report.txt"), b"edited after the last synchronization").unwrap();
        assert!(matches!(
            run_local_scenario_rule_delete(
                &database,
                &registry_path,
                &scenario_id,
                1,
                "report.txt",
                false,
                "rule-delete-4",
            ),
            Err(LocalFileCopyError::ConflictStale)
        ));
        assert_eq!(
            fs::read(right_root.join("report.txt")).unwrap(),
            b"edited after the last synchronization"
        );
    }

    #[test]
    fn selected_conflict_replace_marks_task_stale_when_the_destination_changed() {
        let (database, registry_path, source_root, source_connection) =
            local_fixture("conflict-replace-stale");
        let destination_root = source_root
            .parent()
            .unwrap()
            .join("conflict-replace-stale-destination");
        fs::create_dir(&source_root).unwrap();
        fs::create_dir(&destination_root).unwrap();
        fs::write(source_root.join("report.txt"), b"source version").unwrap();
        fs::write(destination_root.join("report.txt"), b"queued destination").unwrap();
        let registry = Registry::open(&registry_path).unwrap();
        let destination_connection = registry.add("local_filesystem", "Destination").unwrap();
        let scenario_id = registry
            .create_scenario(
                "Conflict",
                &source_connection,
                source_root.to_str().unwrap(),
                &destination_connection,
                destination_root.to_str().unwrap(),
                "left_to_right",
            )
            .unwrap();
        let source = LocalFilesystem::open(&source_root)
            .unwrap()
            .stat(Path::new("report.txt"))
            .unwrap();
        let destination = LocalFilesystem::open(&destination_root)
            .unwrap()
            .stat(Path::new("report.txt"))
            .unwrap();
        let task = registry
            .queue_conflict_task(
                &ScenarioReference {
                    scenario_id: scenario_id.clone(),
                    revision: 1,
                },
                "report.txt",
                "content",
                None,
                Some(&ConfirmedFileVersion {
                    object_revision: source.object_revision,
                    size: source.size,
                    checksum_md5: None,
                }),
                Some(&ConfirmedFileVersion {
                    object_revision: destination.object_revision,
                    size: destination.size,
                    checksum_md5: None,
                }),
            )
            .unwrap();
        registry
            .select_conflict_task(&task, Some(true), "replace_from_source")
            .unwrap();
        fs::write(
            destination_root.join("report.txt"),
            b"changed after dry run",
        )
        .unwrap();

        assert!(matches!(
            run_local_scenario_conflict_replace(
                &database,
                &registry_path,
                &task,
                "conflict-replace-stale-1",
                true
            ),
            Err(LocalFileCopyError::ConflictStale)
        ));
        assert_eq!(
            fs::read(destination_root.join("report.txt")).unwrap(),
            b"changed after dry run"
        );
        let stored = registry
            .list_conflict_tasks(&ScenarioReference {
                scenario_id,
                revision: 1,
            })
            .unwrap();
        assert_eq!(
            stored
                .into_iter()
                .find(|entry| entry.id == task)
                .unwrap()
                .state,
            "stale"
        );
    }

    #[test]
    fn bidirectional_scenario_replace_accepts_a_right_to_left_update() {
        let (database, registry_path, left_root, left_connection) =
            local_fixture("scenario-replace-reverse");
        let right_root = left_root
            .parent()
            .unwrap()
            .join("scenario-replace-reverse-right-root");
        fs::create_dir(&left_root).unwrap();
        fs::create_dir(&right_root).unwrap();
        fs::write(left_root.join("report.txt"), b"old report").unwrap();
        fs::write(right_root.join("report.txt"), b"new report").unwrap();
        let registry = Registry::open(&registry_path).unwrap();
        let right_connection = registry.add("local_filesystem", "Scenario Right").unwrap();
        let scenario = registry
            .create_scenario(
                "Bidirectional scenario update",
                &left_connection,
                left_root.to_str().unwrap(),
                &right_connection,
                right_root.to_str().unwrap(),
                "bidirectional",
            )
            .unwrap();

        assert_eq!(
            run_local_scenario_file_replace(
                &database,
                &registry_path,
                &scenario,
                1,
                &right_root,
                &right_connection,
                Path::new("report.txt"),
                &left_root,
                &left_connection,
                Path::new("report.txt"),
                "scenario-replace-reverse-1",
                true,
            )
            .unwrap(),
            LocalFileCopyStatus::Completed
        );
        assert_eq!(
            fs::read(left_root.join("report.txt")).unwrap(),
            b"new report"
        );
        let stored = JobStore::open(&database)
            .unwrap()
            .get("scenario-replace-reverse-1")
            .unwrap()
            .unwrap();
        assert_eq!(stored.scenario_id.as_deref(), Some(scenario.as_str()));
        assert_eq!(stored.scenario_revision, Some(1));
        let baseline = Registry::open(&registry_path)
            .unwrap()
            .list_confirmed_file_states(&ScenarioReference {
                scenario_id: scenario,
                revision: 1,
            })
            .unwrap();
        assert_eq!(baseline.len(), 1);
        assert_eq!(baseline[0].relative_path, "report.txt");
        assert_eq!(baseline[0].left.as_ref().unwrap().size, 10);
        assert_eq!(baseline[0].right.as_ref().unwrap().size, 10);
    }

    #[test]
    fn local_directory_reconciliation_requires_the_durable_root_identity() {
        let (database, registry, root, connection) = local_fixture("reconcile-root");
        fs::create_dir(&root).unwrap();
        let local = LocalFilesystem::open(&root).unwrap();
        let root_identity = local_root_identity_key(&local).unwrap();
        let mut store = JobStore::open(&database).unwrap();
        store.enqueue("local-mkdir-unknown", None).unwrap();
        store
            .advance(
                "local-mkdir-unknown",
                JobState::Queued,
                JobState::Running,
                Confirmation::Confirmed(EvidenceLevel::Requested),
            )
            .unwrap();
        store
            .prepare_bound_attempt_with_root_identity(
                "local-mkdir-unknown",
                "local-mkdir-unknown-attempt-1",
                "local_mkdir_new",
                &connection,
                "created",
                0,
                "d41d8cd98f00b204e9800998ecf8427e",
                &root_identity,
            )
            .unwrap();
        drop(store);
        fs::create_dir(root.join("created")).unwrap();

        assert_eq!(
            reconcile_local_directory_create(
                &database,
                &registry,
                &root,
                &connection,
                "local-mkdir-unknown",
            )
            .unwrap(),
            LocalDirectoryReconciliationStatus::Completed
        );
        let store = JobStore::open(&database).unwrap();
        assert_eq!(
            store.state("local-mkdir-unknown").unwrap(),
            Some(JobState::Completed)
        );
    }

    #[test]
    fn local_directory_reconciliation_refuses_a_different_root() {
        let (database, registry, root, connection) = local_fixture("reconcile-wrong-root");
        let other_root = root.parent().unwrap().join("other-root");
        fs::create_dir(&root).unwrap();
        fs::create_dir(&other_root).unwrap();
        let local = LocalFilesystem::open(&root).unwrap();
        let root_identity = local_root_identity_key(&local).unwrap();
        let mut store = JobStore::open(&database).unwrap();
        store.enqueue("local-mkdir-wrong-root", None).unwrap();
        store
            .advance(
                "local-mkdir-wrong-root",
                JobState::Queued,
                JobState::Running,
                Confirmation::Confirmed(EvidenceLevel::Requested),
            )
            .unwrap();
        store
            .prepare_bound_attempt_with_root_identity(
                "local-mkdir-wrong-root",
                "local-mkdir-wrong-root-attempt-1",
                "local_mkdir_new",
                &connection,
                "created",
                0,
                "d41d8cd98f00b204e9800998ecf8427e",
                &root_identity,
            )
            .unwrap();
        drop(store);
        fs::create_dir(root.join("created")).unwrap();
        fs::create_dir(other_root.join("created")).unwrap();

        assert_eq!(
            reconcile_local_directory_create(
                &database,
                &registry,
                &other_root,
                &connection,
                "local-mkdir-wrong-root",
            )
            .unwrap(),
            LocalDirectoryReconciliationStatus::StillUnknown
        );
        let store = JobStore::open(&database).unwrap();
        assert_eq!(
            store.state("local-mkdir-wrong-root").unwrap(),
            Some(JobState::Unknown)
        );
    }

    #[test]
    fn local_directory_controller_never_replaces_existing_entry() {
        let (database, registry, root, connection) = local_fixture("occupied");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("occupied"), b"preserve").unwrap();
        assert!(matches!(
            run_local_directory_create(
                &database,
                &registry,
                &root,
                &connection,
                Path::new("occupied"),
                "local-mkdir-occupied",
            ),
            Err(LocalDirectoryCreateError::TargetExists)
        ));
        assert_eq!(fs::read(root.join("occupied")).unwrap(), b"preserve");
        let store = JobStore::open(&database).unwrap();
        assert_eq!(
            store.state("local-mkdir-occupied").unwrap(),
            Some(JobState::Failed)
        );
        assert!(store.unresolved_attempts().unwrap().is_empty());
    }

    #[test]
    fn local_directory_controller_refuses_disabled_or_missing_connection_before_io() {
        let (database, registry_path, root, connection) = local_fixture("disabled");
        let registry = Registry::open(&registry_path).unwrap();
        registry.set_enabled(&connection, false).unwrap();
        assert!(matches!(
            run_local_directory_create(
                &database,
                &registry_path,
                &root,
                &connection,
                Path::new("must-not-exist"),
                "local-mkdir-disabled",
            ),
            Err(LocalDirectoryCreateError::ConnectionUnavailable)
        ));
        assert!(!database.exists());
        assert!(!root.exists());
    }
}
