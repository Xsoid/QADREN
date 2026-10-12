//! The first G1 execution slice: an explicit, app-owned mock destination.
//! It never contacts a provider or reads user/cloud content.
use crate::confirmation::{Confirmation, EvidenceLevel};
use crate::jobs::{JobStore, JobStoreError};
use crate::model::JobState;
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
    store.resolve_attempt(job_id, &attempt.attempt_id, &receipt)?;
    store.advance(
        job_id,
        JobState::Unknown,
        JobState::Completed,
        receipt.confirmation(),
    )?;
    Ok(MockReconciliationStatus::Completed)
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
    storage.create_new(&target_name, written_content)?;
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
    store.resolve_attempt(job_id, &attempt_id, &receipt)?;
    store.advance(
        job_id,
        JobState::Running,
        JobState::Completed,
        receipt.confirmation(),
    )?;
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
        assert!(matches!(
            run_mock_create(&database, &storage, "job-1", true),
            Err(MockCreateError::OccupiedTarget)
        ));
        assert_eq!(fs::read(target).unwrap(), b"existing bytes");
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
}
