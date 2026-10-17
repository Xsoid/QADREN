//! The first G1 execution slice: an explicit, app-owned mock destination.
//! It never contacts a provider or reads user/cloud content.
use crate::confirmation::{Confirmation, EvidenceLevel};
use crate::jobs::{JobStore, JobStoreError, ScenarioCopyBaselineOutboxEntry};
use crate::local_fs::{ContentReadConsent, LocalError, LocalFilesystem};
use crate::model::{ConfirmedFileVersion, JobState, ScenarioReference};
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

#[derive(Debug)]
pub enum LocalFileCopyError {
    InvalidRequest,
    ConsentRequired,
    TargetExists,
    CapabilityUnavailable,
    ConnectionUnavailable,
    ScenarioUnavailable,
    ScenarioScopeMismatch,
    VerificationFailed,
    OutcomeUnknown(LocalError),
    Local(LocalError),
    Store(JobStoreError),
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
    fn from(value: JobStoreError) -> Self { Self::Store(value) }
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

/// Execute one manually confirmed Local-to-Local copy. The confirmation is
/// written and consumed for this exact source connection and relative path
/// before the source file is opened. This permits macOS File Provider
/// materialization only after the user has approved that exact operation.
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
    consent: bool,
) -> Result<LocalFileCopyStatus, LocalFileCopyError> {
    run_local_file_copy_inner(
        database_path, registry_path, source_root, source_connection_id, source_relative,
        destination_root, destination_connection_id, destination_relative, job_id, consent, None,
    )
}

/// Execute one explicitly confirmed Local-to-Local create proposed by an exact
/// Scenario revision. The immutable Scenario identity is stored with the Job.
/// This does not enable automatic content reads: every source still needs its
/// own confirmation.
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
    consent: bool,
) -> Result<LocalFileCopyStatus, LocalFileCopyError> {
    let registry = Registry::open(registry_path).map_err(|_| LocalFileCopyError::ScenarioUnavailable)?;
    let scenario = registry
        .get_scenario_revision(scenario_id, scenario_revision)
        .map_err(|_| LocalFileCopyError::ScenarioUnavailable)?
        .ok_or(LocalFileCopyError::ScenarioUnavailable)?;
    let source_root = source_root.to_str().ok_or(LocalFileCopyError::InvalidRequest)?;
    let destination_root = destination_root.to_str().ok_or(LocalFileCopyError::InvalidRequest)?;
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
    run_local_file_copy_inner(
        database_path, registry_path, Path::new(source_root), source_connection_id, source_relative,
        Path::new(destination_root), destination_connection_id, destination_relative, job_id, consent,
        Some((scenario_id, scenario_revision, is_left_to_right)),
    )
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
    if !consent { return Err(LocalFileCopyError::ConsentRequired); }
    if !enabled_local_connection(registry_path, source_connection_id)
        || !enabled_local_connection(registry_path, destination_connection_id)
    { return Err(LocalFileCopyError::ConnectionUnavailable); }
    let source = LocalFilesystem::open(source_root).map_err(LocalFileCopyError::Local)?;
    let destination = LocalFilesystem::open(destination_root).map_err(LocalFileCopyError::Local)?;
    if !source.capabilities().operations.read_content || !destination.capabilities().operations.write_content {
        return Err(LocalFileCopyError::CapabilityUnavailable);
    }
    if source.overlaps_root(&destination).map_err(LocalFileCopyError::Local)? {
        return Err(LocalFileCopyError::InvalidRequest);
    }
    let source_entry = source.stat(source_relative).map_err(LocalFileCopyError::Local)?;
    if source_entry.is_directory { return Err(LocalFileCopyError::InvalidRequest); }
    match destination.stat(destination_relative) {
        Ok(_) => return Err(LocalFileCopyError::TargetExists),
        Err(LocalError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(LocalFileCopyError::Local(error)),
    }
    let source_path = source_relative.to_str().ok_or(LocalFileCopyError::InvalidRequest)?;
    let target_path = destination_relative.to_str().ok_or(LocalFileCopyError::InvalidRequest)?;
    let root_identity = local_root_identity_key(&destination).map_err(|error| match error {
        LocalDirectoryCreateError::Local(error) => LocalFileCopyError::Local(error),
        LocalDirectoryCreateError::Store(error) => LocalFileCopyError::Store(error),
        _ => LocalFileCopyError::InvalidRequest,
    })?;
    let mut store = JobStore::open(database_path)?;
    if let Some(existing) = store.get(job_id)? { return Ok(LocalFileCopyStatus::Existing(existing.state)); }
    store.enqueue(job_id, scenario.map(|(id, revision, _)| (id, revision)))?;
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
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| LocalFileCopyError::InvalidRequest)?.as_secs();
    let scope = OperationScope {
        operation: ProtectedOperation::ReadContent,
        connection_id: source_connection_id.to_owned(),
        relative_path: source_path.to_owned(),
        expires_at: now.saturating_add(60),
    };
    let decision_id = format!("{job_id}-source-read");
    store.record_operation_decision(&decision_id, job_id, &scope, Decision::Accepted)?;
    store.consume_operation_decision(&decision_id, job_id, &scope, now)?;
    store.advance(job_id, JobState::WaitingForDecision, JobState::Running, Confirmation::Confirmed(EvidenceLevel::Requested))?;
    let attempt_id = format!("{job_id}-attempt-1");
    store.prepare_bound_attempt_with_root_identity(
        job_id, &attempt_id, "local_copy_new", destination_connection_id, target_path,
        source_entry.size, "00000000000000000000000000000000", &root_identity,
    )?;
    if !enabled_local_connection(registry_path, source_connection_id)
        || !enabled_local_connection(registry_path, destination_connection_id)
    {
        store.fail_attempt_before_effect(job_id, &attempt_id)?;
        return Err(LocalFileCopyError::ConnectionUnavailable);
    }
    match source.copy_file_to_new(source_relative, &destination, destination_relative, ContentReadConsent::GrantedForThisOperation) {
        Ok(bytes) if bytes == source_entry.size => {}
        Ok(_) => return Err(LocalFileCopyError::VerificationFailed),
        Err(LocalError::AlreadyExists) => { store.fail_attempt_before_effect(job_id, &attempt_id)?; return Err(LocalFileCopyError::TargetExists); }
        Err(error @ LocalError::OutcomeUnknown(_)) => return Err(LocalFileCopyError::OutcomeUnknown(error)),
        Err(error) => return Err(LocalFileCopyError::Local(error)),
    }
    let destination_entry = match destination.stat(destination_relative) {
        Ok(entry) if !entry.is_directory && entry.size == source_entry.size => entry,
        _ => return Err(LocalFileCopyError::VerificationFailed),
    };
    let baseline = if let Some((scenario_id, scenario_revision, source_is_left)) = scenario {
        let source_after = source.stat(source_relative).map_err(LocalFileCopyError::Local)?;
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
            left: if source_is_left { source_version.clone() } else { destination_version.clone() },
            right: if source_is_left { destination_version } else { source_version },
        })
    } else {
        None
    };
    let receipt = VerifiedReceipt::metadata_verified(&attempt_id, "local_copy_new", destination_connection_id, target_path);
    store.complete_attempt_with_scenario_copy_baseline(job_id, &attempt_id, &receipt, baseline.as_ref())?;
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
    let identity = destination.root_identity().map_err(LocalFileCopyError::Local)?;
    let root_identity = format!("unix:{}:{}", identity.device, identity.inode);
    let mut store = JobStore::open(database_path)?;
    let Some(job) = store.get(job_id)? else {
        return Err(LocalFileCopyError::Store(JobStoreError::NotFound));
    };
    if job.state != JobState::Unknown {
        return Ok(LocalFileCopyReconciliationStatus::Existing(job.state));
    }
    let attempts = store.unresolved_attempts()?;
    let matching: Vec<_> = attempts.iter().filter(|attempt| {
        attempt.job_id == job_id
            && attempt.status == "unknown"
            && attempt.operation_kind.as_deref() == Some("local_copy_new")
            && attempt.target_connection_id.as_deref() == Some(destination_connection_id)
    }).collect();
    if matching.len() != 1 { return Ok(LocalFileCopyReconciliationStatus::StillUnknown); }
    let attempt = matching[0];
    let Some(expected) = store.expected_metadata(job_id, &attempt.attempt_id)? else {
        return Ok(LocalFileCopyReconciliationStatus::StillUnknown);
    };
    if expected.operation_kind != "local_copy_new"
        || expected.target_connection_id != destination_connection_id
        || store.attempt_root_identity(job_id, &attempt.attempt_id)?.as_deref() != Some(root_identity.as_str())
    {
        return Ok(LocalFileCopyReconciliationStatus::StillUnknown);
    }
    if !matches!(destination.stat(Path::new(&expected.target_path)), Ok(entry) if !entry.is_directory && entry.size == expected.expected_size) {
        return Ok(LocalFileCopyReconciliationStatus::StillUnknown);
    }
    let receipt = VerifiedReceipt::metadata_verified(&attempt.attempt_id, "local_copy_new", destination_connection_id, &expected.target_path);
    store.complete_attempt(job_id, &attempt.attempt_id, &receipt)?;
    Ok(LocalFileCopyReconciliationStatus::Completed)
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
    let identity = destination.root_identity().map_err(LocalFileCopyError::Local)?;
    let root_identity = format!("unix:{}:{}", identity.device, identity.inode);
    let store = JobStore::open(database_path)?;
    let mut candidates = Vec::new();
    for attempt in store.unresolved_attempts()? {
        if attempt.status != "unknown"
            || attempt.operation_kind.as_deref() != Some("local_copy_new")
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
    fn local_file_copy_persists_consent_and_never_overwrites() {
        let (database, registry_path, source_root, source_connection) = local_fixture("copy");
        let destination_root = source_root.parent().unwrap().join("destination-root");
        fs::create_dir(&source_root).unwrap();
        fs::create_dir(&destination_root).unwrap();
        fs::write(source_root.join("source.txt"), b"copy me").unwrap();
        let registry = Registry::open(&registry_path).unwrap();
        let destination_connection = registry.add("local_filesystem", "Destination Local").unwrap();

        assert!(matches!(
            run_local_file_copy(
                &database, &registry_path, &source_root, &source_connection, Path::new("source.txt"),
                &destination_root, &destination_connection, Path::new("source.txt"), "local-copy-denied", false,
            ),
            Err(LocalFileCopyError::ConsentRequired)
        ));
        assert!(!database.exists());
        assert_eq!(
            run_local_file_copy(
                &database, &registry_path, &source_root, &source_connection, Path::new("source.txt"),
                &destination_root, &destination_connection, Path::new("source.txt"), "local-copy-1", true,
            ).unwrap(),
            LocalFileCopyStatus::Completed
        );
        assert_eq!(fs::read(destination_root.join("source.txt")).unwrap(), b"copy me");
        assert_eq!(fs::read(source_root.join("source.txt")).unwrap(), b"copy me");
        assert_eq!(JobStore::open(&database).unwrap().state("local-copy-1").unwrap(), Some(JobState::Completed));
        assert!(matches!(
            run_local_file_copy(
                &database, &registry_path, &source_root, &source_connection, Path::new("source.txt"),
                &destination_root, &destination_connection, Path::new("source.txt"), "local-copy-2", true,
            ),
            Err(LocalFileCopyError::TargetExists)
        ));
        assert_eq!(fs::read(destination_root.join("source.txt")).unwrap(), b"copy me");
    }

    #[test]
    fn local_file_copy_reconciliation_requires_exact_destination_evidence() {
        let (database, registry_path, root, _) = local_fixture("copy-reconcile");
        let destination_root = root.parent().unwrap().join("copy-reconcile-destination");
        fs::create_dir(&destination_root).unwrap();
        let registry = Registry::open(&registry_path).unwrap();
        let connection = registry.add("local_filesystem", "Copy destination").unwrap();
        let destination = LocalFilesystem::open(&destination_root).unwrap();
        let identity = destination.root_identity().unwrap();
        let root_identity = format!("unix:{}:{}", identity.device, identity.inode);
        let mut store = JobStore::open(&database).unwrap();
        store.enqueue("copy-unknown", None).unwrap();
        store.advance(
            "copy-unknown", JobState::Queued, JobState::Running,
            Confirmation::Confirmed(EvidenceLevel::Requested),
        ).unwrap();
        store.prepare_bound_attempt_with_root_identity(
            "copy-unknown", "copy-unknown-attempt", "local_copy_new", &connection,
            "copied.txt", 7, "00000000000000000000000000000000", &root_identity,
        ).unwrap();
        drop(store);
        fs::write(destination_root.join("copied.txt"), b"payload").unwrap();
        assert_eq!(
            reconcile_local_file_copy(&database, &registry_path, &destination_root, &connection, "copy-unknown").unwrap(),
            LocalFileCopyReconciliationStatus::Completed
        );
        assert_eq!(JobStore::open(&database).unwrap().state("copy-unknown").unwrap(), Some(JobState::Completed));
    }

    #[test]
    fn local_file_copy_reconciliation_inventory_requires_the_bound_destination_root() {
        let (database, registry_path, root, _) = local_fixture("copy-reconcile-inventory");
        let destination_root = root.parent().unwrap().join("copy-reconcile-inventory-destination");
        let other_root = root.parent().unwrap().join("copy-reconcile-inventory-other");
        fs::create_dir(&destination_root).unwrap();
        fs::create_dir(&other_root).unwrap();
        let registry = Registry::open(&registry_path).unwrap();
        let connection = registry.add("local_filesystem", "Copy destination").unwrap();
        let destination = LocalFilesystem::open(&destination_root).unwrap();
        let identity = destination.root_identity().unwrap();
        let root_identity = format!("unix:{}:{}", identity.device, identity.inode);
        let mut store = JobStore::open(&database).unwrap();
        store.enqueue("copy-inventory-unknown", None).unwrap();
        store.advance(
            "copy-inventory-unknown", JobState::Queued, JobState::Running,
            Confirmation::Confirmed(EvidenceLevel::Requested),
        ).unwrap();
        store.prepare_bound_attempt_with_root_identity(
            "copy-inventory-unknown", "copy-inventory-unknown-attempt", "local_copy_new", &connection,
            "copied.txt", 7, "00000000000000000000000000000000", &root_identity,
        ).unwrap();
        drop(store);

        assert_eq!(
            local_file_copy_reconciliation_candidates(
                &database, &registry_path, &destination_root, &connection,
            ).unwrap(),
            vec![LocalFileCopyReconciliationCandidate {
                job_id: "copy-inventory-unknown".to_owned(),
            }]
        );
        assert!(local_file_copy_reconciliation_candidates(
            &database, &registry_path, &other_root, &connection,
        ).unwrap().is_empty());
    }

    #[test]
    fn scenario_copy_binds_the_exact_revision_and_endpoint_scope() {
        use crate::scenario_planner::{ObservedFile, PlanAction};
        let (database, registry_path, source_root, source_connection) = local_fixture("scenario-copy");
        let destination_root = source_root.parent().unwrap().join("scenario-destination-root");
        fs::create_dir(&source_root).unwrap();
        fs::create_dir(&destination_root).unwrap();
        fs::write(source_root.join("source.txt"), b"copy me").unwrap();
        let registry = Registry::open(&registry_path).unwrap();
        let destination_connection = registry.add("local_filesystem", "Scenario Destination").unwrap();
        let scenario = registry.create_scenario(
            "Scenario copy", &source_connection, source_root.to_str().unwrap(),
            &destination_connection, destination_root.to_str().unwrap(), "left_to_right",
        ).unwrap();
        assert_eq!(
            run_local_scenario_file_copy(
                &database, &registry_path, &scenario, 1, &source_root, &source_connection, Path::new("source.txt"),
                &destination_root, &destination_connection, Path::new("source.txt"), "scenario-copy-1", true,
            ).unwrap(),
            LocalFileCopyStatus::Completed
        );
        let stored = JobStore::open(&database).unwrap().get("scenario-copy-1").unwrap().unwrap();
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
        assert!(baseline[0].right.as_ref().unwrap().object_revision.is_some());
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
                &database, &registry_path, &scenario, 1, &destination_root, &source_connection, Path::new("source.txt"),
                &source_root, &destination_connection, Path::new("other.txt"), "scenario-copy-bad", true,
            ),
            Err(LocalFileCopyError::ScenarioScopeMismatch)
        ));
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
