//! Durable Job metadata. No content bytes, credentials or transfer side effects.
//! Stored independently from StorageConnection Registry until cross-links are specified.
use crate::confirmation::Confirmation;
use crate::model::JobState;
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;

#[derive(Debug)]
pub enum JobStoreError {
    Sqlite(rusqlite::Error),
    UnsupportedSchema(i32),
    InvalidId,
    InvalidTransition,
    MissingEvidence,
    UntrustedEvidence,
    NotFound,
    DuplicateJob,
}
impl From<rusqlite::Error> for JobStoreError {
    fn from(value: rusqlite::Error) -> Self {
        Self::Sqlite(value)
    }
}
pub type Result<T> = std::result::Result<T, JobStoreError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredJob {
    pub id: String,
    pub state: JobState,
    pub scenario_id: Option<String>,
    pub scenario_revision: Option<u64>,
}

/// Read-only reconciliation inventory; no content, credentials or side effects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnresolvedAttempt {
    pub attempt_id: String,
    pub job_id: String,
    pub status: String,
    pub operation_kind: Option<String>,
    pub target_connection_id: Option<String>,
    pub target_path: Option<String>,
    pub is_legacy_unbound: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReconciliationDisposition {
    /// The recorded target and operation exist; an adapter verifier is still required.
    NeedsAdapterVerification,
    /// Missing legacy binding prevents trustworthy automatic verification.
    NeedsManualInvestigation,
}

impl UnresolvedAttempt {
    pub fn disposition(&self) -> ReconciliationDisposition {
        if self.is_legacy_unbound {
            ReconciliationDisposition::NeedsManualInvestigation
        } else {
            ReconciliationDisposition::NeedsAdapterVerification
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredExpectation {
    pub attempt_id: String,
    pub job_id: String,
    pub operation_kind: String,
    pub target_connection_id: String,
    pub target_path: String,
    pub expected_size: u64,
    pub expected_md5: String,
}

pub struct JobStore {
    db: Connection,
}

fn label(state: JobState) -> &'static str {
    match state {
        JobState::Queued => "queued",
        JobState::Running => "running",
        JobState::Paused => "paused",
        JobState::WaitingForDecision => "waiting_for_decision",
        JobState::Completed => "completed",
        JobState::Failed => "failed",
        JobState::Unknown => "unknown",
    }
}

fn parse_state(value: &str) -> rusqlite::Result<JobState> {
    match value {
        "queued" => Ok(JobState::Queued),
        "running" => Ok(JobState::Running),
        "paused" => Ok(JobState::Paused),
        "waiting_for_decision" => Ok(JobState::WaitingForDecision),
        "completed" => Ok(JobState::Completed),
        "failed" => Ok(JobState::Failed),
        "unknown" => Ok(JobState::Unknown),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}

impl JobStore {
    pub fn open(path: &Path) -> Result<Self> {
        Self::initialize(Connection::open(path)?)
    }

    pub fn in_memory() -> Result<Self> {
        Self::initialize(Connection::open_in_memory()?)
    }

    fn initialize(db: Connection) -> Result<Self> {
        let version: i32 = db.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version > 6 {
            return Err(JobStoreError::UnsupportedSchema(version));
        }
        if version == 0 {
            db.execute_batch(
                "BEGIN IMMEDIATE;
                 CREATE TABLE jobs (
                   id TEXT PRIMARY KEY NOT NULL,
                   state TEXT NOT NULL CHECK (state IN
                     ('queued','running','paused','waiting_for_decision','completed','failed','unknown')),
                   scenario_id TEXT,
                   scenario_revision INTEGER,
                   CHECK ((scenario_id IS NULL AND scenario_revision IS NULL) OR
                          (scenario_id IS NOT NULL AND scenario_revision IS NOT NULL AND scenario_revision > 0))
                 );
                 PRAGMA user_version = 1;
                 COMMIT;",
            )?;
        }
        if version <= 1 {
            db.execute_batch(
                "BEGIN IMMEDIATE;
                 CREATE TABLE job_events (
                   sequence INTEGER PRIMARY KEY AUTOINCREMENT,
                   job_id TEXT NOT NULL REFERENCES jobs(id),
                   state TEXT NOT NULL,
                   kind TEXT NOT NULL
                 );
                 PRAGMA user_version = 2;
                 COMMIT;",
            )?;
        }
        if version <= 2 {
            db.execute_batch(
                "BEGIN IMMEDIATE;
                 CREATE TABLE job_attempts (
                   attempt_id TEXT PRIMARY KEY NOT NULL,
                   job_id TEXT NOT NULL REFERENCES jobs(id),
                   status TEXT NOT NULL CHECK (status IN ('active','unknown','resolved')),
                   UNIQUE(job_id, attempt_id)
                 );
                 CREATE UNIQUE INDEX one_active_attempt_per_job
                   ON job_attempts(job_id) WHERE status='active';
                 PRAGMA user_version = 3;
                 COMMIT;",
            )?;
        }
        if version <= 3 {
            db.execute_batch(
                "BEGIN IMMEDIATE;
                 CREATE TABLE attempt_verifications (
                   attempt_id TEXT PRIMARY KEY NOT NULL REFERENCES job_attempts(attempt_id),
                   evidence_level TEXT NOT NULL CHECK (evidence_level IN
                     ('metadata_verified','content_verified'))
                 );
                 PRAGMA user_version = 4;
                 COMMIT;",
            )?;
        }
        if version <= 4 {
            db.execute_batch(
                "BEGIN IMMEDIATE;
                 ALTER TABLE job_attempts ADD COLUMN operation_kind TEXT;
                 ALTER TABLE job_attempts ADD COLUMN target_connection_id TEXT;
                 ALTER TABLE job_attempts ADD COLUMN target_path TEXT;
                 PRAGMA user_version = 5;
                 COMMIT;",
            )?;
        }
        if version <= 5 {
            db.execute_batch(
                "BEGIN IMMEDIATE;
                 CREATE TABLE attempt_expectations (
                   attempt_id TEXT PRIMARY KEY NOT NULL REFERENCES job_attempts(attempt_id),
                   expected_size INTEGER NOT NULL CHECK(expected_size >= 0),
                   expected_md5 TEXT NOT NULL CHECK(length(expected_md5) = 32)
                 );
                 PRAGMA user_version = 6;
                 COMMIT;",
            )?;
        }
        db.pragma_update(None, "foreign_keys", "ON")?;
        // A previous process may have written remote effects before dying.
        // Reconciliation is required; never resubmit these Jobs blindly.
        db.execute_batch(
            "BEGIN IMMEDIATE;
             INSERT INTO job_events(job_id,state,kind)
               SELECT id,'unknown','restart_uncertain' FROM jobs WHERE state='running';
             UPDATE job_attempts SET status='unknown' WHERE status='active';
             UPDATE jobs SET state='unknown' WHERE state='running';
             COMMIT;",
        )?;
        Ok(Self { db })
    }

    pub fn enqueue(&mut self, id: &str, scenario: Option<(&str, u64)>) -> Result<()> {
        if id.is_empty()
            || scenario
                .is_some_and(|(name, rev)| name.is_empty() || rev == 0 || rev > i64::MAX as u64)
        {
            return Err(JobStoreError::InvalidId);
        }
        let (scenario_id, revision) = match scenario {
            Some((name, rev)) => (Some(name), Some(rev as i64)),
            None => (None, None),
        };
        let tx = self.db.transaction()?;
        let result = tx.execute(
            "INSERT OR IGNORE INTO jobs (id,state,scenario_id,scenario_revision)
             VALUES (?1,'queued',?2,?3)",
            params![id, scenario_id, revision],
        )?;
        if result == 0 {
            return Err(JobStoreError::DuplicateJob);
        }
        tx.execute(
            "INSERT INTO job_events(job_id,state,kind) VALUES (?1,'queued','enqueued')",
            [id],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn get(&self, id: &str) -> Result<Option<StoredJob>> {
        self.db
            .query_row(
                "SELECT id,state,scenario_id,scenario_revision FROM jobs WHERE id=?1",
                [id],
                |r| {
                    let state: String = r.get(1)?;
                    let rev: Option<i64> = r.get(3)?;
                    Ok(StoredJob {
                        id: r.get(0)?,
                        state: parse_state(&state)?,
                        scenario_id: r.get(2)?,
                        scenario_revision: rev.map(|v| v as u64),
                    })
                },
            )
            .optional()
            .map_err(JobStoreError::from)
    }

    /// Compare-and-set: only the expected persisted state may advance.
    /// The caller remains responsible for verifying the evidence's provenance.
    pub fn advance(
        &mut self,
        id: &str,
        expected: JobState,
        next: JobState,
        evidence: Confirmation,
    ) -> Result<()> {
        match expected.transition(next, evidence) {
            Ok(_) => {}
            Err(crate::confirmation::StateTransitionError::InvalidTransition) => {
                return Err(JobStoreError::InvalidTransition);
            }
            Err(crate::confirmation::StateTransitionError::InsufficientEvidence) => {
                return Err(JobStoreError::MissingEvidence);
            }
        }
        let tx = self.db.transaction()?;
        // A Job must never be declared complete while an operation attempt
        // is active or has an unresolved outcome.
        if next == JobState::Completed {
            let unresolved: i64 = tx.query_row(
                "SELECT count(*) FROM job_attempts WHERE job_id=?1 AND status!='resolved'",
                [id],
                |row| row.get(0),
            )?;
            if unresolved > 0 {
                return Err(JobStoreError::InvalidTransition);
            }
        }
        let changed = tx.execute(
            "UPDATE jobs SET state=?1 WHERE id=?2 AND state=?3",
            params![label(next), id, label(expected)],
        )?;
        if changed == 0 {
            drop(tx);
            if self.get(id)?.is_none() {
                Err(JobStoreError::NotFound)
            } else {
                Err(JobStoreError::InvalidTransition)
            }
        } else {
            tx.execute(
                "INSERT INTO job_events(job_id,state,kind) VALUES (?1,?2,'transition')",
                params![id, label(next)],
            )?;
            tx.commit()?;
            Ok(())
        }
    }

    /// Bind the operation before it may produce an external side effect.
    /// Legacy attempts have no binding and remain ineligible for bound receipts.
    pub fn begin_bound_attempt(
        &mut self,
        job_id: &str,
        attempt_id: &str,
        operation_kind: &str,
        target_connection_id: &str,
        target_path: &str,
    ) -> Result<()> {
        if operation_kind.is_empty() || target_connection_id.is_empty() || target_path.is_empty() {
            return Err(JobStoreError::InvalidId);
        }
        let tx = self.db.transaction()?;
        let state: Option<String> = tx
            .query_row("SELECT state FROM jobs WHERE id=?1", [job_id], |row| {
                row.get(0)
            })
            .optional()?;
        if state.as_deref() != Some("running") {
            return Err(JobStoreError::InvalidTransition);
        }
        let pending: i64 = tx.query_row(
            "SELECT count(*) FROM job_attempts WHERE job_id=?1 AND status!='resolved'",
            [job_id],
            |row| row.get(0),
        )?;
        if pending != 0 {
            return Err(JobStoreError::InvalidTransition);
        }
        tx.execute(
            "INSERT INTO job_attempts
               (attempt_id,job_id,status,operation_kind,target_connection_id,target_path)
             VALUES (?1,?2,'active',?3,?4,?5)",
            params![
                attempt_id,
                job_id,
                operation_kind,
                target_connection_id,
                target_path
            ],
        )?;
        tx.execute(
            "INSERT INTO job_events(job_id,state,kind) VALUES (?1,'running','attempt_started')",
            [job_id],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Store expected metadata before any effect. This operation is write-once
    /// and only permitted for an active attempt bound to the exact target.
    pub fn bind_expected_metadata(
        &mut self,
        job_id: &str,
        attempt_id: &str,
        expected_size: u64,
        expected_md5: &str,
    ) -> Result<()> {
        if expected_size > i64::MAX as u64
            || expected_md5.len() != 32
            || !expected_md5.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err(JobStoreError::InvalidId);
        }
        let tx = self.db.transaction()?;
        let eligible: i64 = tx.query_row(
            "SELECT count(*) FROM job_attempts
             WHERE job_id=?1 AND attempt_id=?2 AND status='active'
             AND operation_kind IS NOT NULL AND target_connection_id IS NOT NULL
             AND target_path IS NOT NULL",
            params![job_id, attempt_id],
            |row| row.get(0),
        )?;
        if eligible != 1 {
            return Err(JobStoreError::InvalidTransition);
        }
        tx.execute(
            "INSERT INTO attempt_expectations(attempt_id,expected_size,expected_md5)
             VALUES (?1,?2,?3)",
            params![
                attempt_id,
                expected_size as i64,
                expected_md5.to_ascii_lowercase()
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Retrieve only a fully bound expectation; no disk content is read.
    pub fn expected_metadata(
        &self,
        job_id: &str,
        attempt_id: &str,
    ) -> Result<Option<StoredExpectation>> {
        self.db
            .query_row(
                "SELECT a.attempt_id,a.job_id,a.operation_kind,a.target_connection_id,
                    a.target_path,e.expected_size,e.expected_md5
             FROM job_attempts a INNER JOIN attempt_expectations e
               ON e.attempt_id=a.attempt_id
             WHERE a.job_id=?1 AND a.attempt_id=?2
               AND a.operation_kind IS NOT NULL AND a.target_connection_id IS NOT NULL
               AND a.target_path IS NOT NULL",
                params![job_id, attempt_id],
                |row| {
                    let size: i64 = row.get(5)?;
                    Ok(StoredExpectation {
                        attempt_id: row.get(0)?,
                        job_id: row.get(1)?,
                        operation_kind: row.get(2)?,
                        target_connection_id: row.get(3)?,
                        target_path: row.get(4)?,
                        expected_size: size as u64,
                        expected_md5: row.get(6)?,
                    })
                },
            )
            .optional()
            .map_err(JobStoreError::from)
    }

    /// Compare a read-only Yandex observation to persisted expectations.
    /// Does not issue a VerifiedReceipt, change Job state, or retry an operation.
    pub fn inspect_yandex_metadata(
        &self,
        job_id: &str,
        attempt_id: &str,
        target_connection_id: &str,
        observed: &crate::yandex::Resource,
    ) -> Result<Option<crate::yandex::MetadataMatch>> {
        let Some(expected) = self.expected_metadata(job_id, attempt_id)? else {
            return Ok(None);
        };
        if expected.target_connection_id != target_connection_id {
            return Err(JobStoreError::UntrustedEvidence);
        }
        let current: Option<String> = self
            .db
            .query_row(
                "SELECT status FROM job_attempts WHERE job_id=?1 AND attempt_id=?2",
                params![job_id, attempt_id],
                |row| row.get(0),
            )
            .optional()?;
        if !matches!(current.as_deref(), Some("active" | "unknown")) {
            return Err(JobStoreError::InvalidTransition);
        }
        let expected_resource = crate::yandex::ExpectedResourceMetadata {
            path: expected.target_path,
            size: expected.expected_size,
            md5: expected.expected_md5,
        };
        Ok(Some(crate::yandex::compare_resource_metadata(
            &expected_resource,
            observed,
        )))
    }

    /// Records explicit resolution without claiming the remote operation succeeded.
    /// A receipt must be generated by the adapter's verification boundary.
    /// The public API does not accept a caller-supplied evidence level.
    pub fn resolve_attempt(
        &mut self,
        job_id: &str,
        attempt_id: &str,
        receipt: &crate::verification::VerifiedReceipt,
    ) -> Result<()> {
        if receipt.attempt_id() != attempt_id {
            return Err(JobStoreError::UntrustedEvidence);
        }
        let stored_binding: Option<(Option<String>, Option<String>, Option<String>)> = self
            .db
            .query_row(
                "SELECT operation_kind,target_connection_id,target_path
                     FROM job_attempts WHERE job_id=?1 AND attempt_id=?2",
                params![job_id, attempt_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        if let Some((Some(kind), Some(connection), Some(path))) = stored_binding {
            if receipt.operation_kind() != Some(kind.as_str())
                || receipt.target_connection_id() != Some(connection.as_str())
                || receipt.target_path() != Some(path.as_str())
            {
                return Err(JobStoreError::UntrustedEvidence);
            }
        } else {
            // Legacy attempts have no trusted operation/target binding.
            // Preserve them for investigation, but never resolve blindly.
            return Err(JobStoreError::UntrustedEvidence);
        }
        let evidence = receipt.confirmation();
        // Acknowledging a prompt or requesting an operation is not proof of outcome.
        // This is a minimum guard, not a substitute for adapter-side verification.
        if !evidence.meets(crate::confirmation::EvidenceLevel::MetadataVerified) {
            return Err(JobStoreError::MissingEvidence);
        }
        let tx = self.db.transaction()?;
        let updated = tx.execute(
            "UPDATE job_attempts SET status='resolved'
             WHERE job_id=?1 AND attempt_id=?2 AND status IN ('active','unknown')",
            params![job_id, attempt_id],
        )?;
        if updated == 0 {
            return Err(JobStoreError::InvalidTransition);
        }
        let level = match evidence {
            Confirmation::Confirmed(crate::confirmation::EvidenceLevel::ContentVerified) => {
                "content_verified"
            }
            _ => "metadata_verified",
        };
        tx.execute(
            "INSERT INTO attempt_verifications(attempt_id,evidence_level) VALUES (?1,?2)",
            params![attempt_id, level],
        )?;
        let state: String =
            tx.query_row("SELECT state FROM jobs WHERE id=?1", [job_id], |row| {
                row.get(0)
            })?;
        tx.execute(
            "INSERT INTO job_events(job_id,state,kind) VALUES (?1,?2,'attempt_resolved')",
            params![job_id, state],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Inventory for operator investigation. Never changes or retries attempts.
    pub fn unresolved_attempts(&self) -> Result<Vec<UnresolvedAttempt>> {
        let mut statement = self.db.prepare(
            "SELECT attempt_id,job_id,status,operation_kind,target_connection_id,target_path
             FROM job_attempts WHERE status!='resolved' ORDER BY job_id,attempt_id",
        )?;
        let rows = statement.query_map([], |row| {
            let operation_kind: Option<String> = row.get(3)?;
            let target_connection_id: Option<String> = row.get(4)?;
            let target_path: Option<String> = row.get(5)?;
            let is_legacy_unbound =
                operation_kind.is_none() || target_connection_id.is_none() || target_path.is_none();
            Ok(UnresolvedAttempt {
                attempt_id: row.get(0)?,
                job_id: row.get(1)?,
                status: row.get(2)?,
                operation_kind,
                target_connection_id,
                target_path,
                is_legacy_unbound,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(JobStoreError::from)
    }

    pub fn state(&self, id: &str) -> Result<Option<JobState>> {
        Ok(self.get(id)?.map(|job| job.state))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::confirmation::EvidenceLevel;

    #[test]
    fn manual_and_scenario_jobs_remain_separate() {
        let mut db = JobStore::in_memory().unwrap();
        db.enqueue("manual", None).unwrap();
        db.enqueue("sync", Some(("scenario-1", 4))).unwrap();
        assert_eq!(db.get("manual").unwrap().unwrap().scenario_id, None);
        assert_eq!(db.get("sync").unwrap().unwrap().scenario_revision, Some(4));
        assert!(matches!(
            db.enqueue("sync", Some(("other", 8))),
            Err(JobStoreError::DuplicateJob)
        ));
        assert_eq!(
            db.get("sync").unwrap().unwrap().scenario_id.as_deref(),
            Some("scenario-1")
        );
    }

    #[test]
    fn completion_requires_evidence_and_cas() {
        let mut db = JobStore::in_memory().unwrap();
        db.enqueue("j", None).unwrap();
        db.advance(
            "j",
            JobState::Queued,
            JobState::Running,
            Confirmation::Unknown,
        )
        .unwrap();
        assert!(matches!(
            db.advance(
                "j",
                JobState::Running,
                JobState::Completed,
                Confirmation::Unknown
            ),
            Err(JobStoreError::MissingEvidence)
        ));
        db.advance(
            "j",
            JobState::Running,
            JobState::Completed,
            Confirmation::Confirmed(EvidenceLevel::ProviderAcknowledged),
        )
        .unwrap();
        assert!(matches!(
            db.advance(
                "j",
                JobState::Running,
                JobState::Failed,
                Confirmation::Unknown
            ),
            Err(JobStoreError::InvalidTransition)
        ));
    }

    #[test]
    fn restart_marks_running_unknown_and_never_resubmits() {
        use std::time::{SystemTime, UNIX_EPOCH};
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "qadren-jobs-{}-{unique}.sqlite",
            std::process::id()
        ));
        {
            let mut db = JobStore::open(&path).unwrap();
            db.enqueue("interrupted", Some(("scenario", 2))).unwrap();
            db.enqueue("waiting", None).unwrap();
            db.advance(
                "interrupted",
                JobState::Queued,
                JobState::Running,
                Confirmation::Unknown,
            )
            .unwrap();
        }
        {
            let mut db = JobStore::open(&path).unwrap();
            assert_eq!(db.state("interrupted").unwrap(), Some(JobState::Unknown));
            assert_eq!(db.state("waiting").unwrap(), Some(JobState::Queued));
            assert!(matches!(
                db.advance(
                    "interrupted",
                    JobState::Unknown,
                    JobState::Running,
                    Confirmation::Unknown
                ),
                Err(JobStoreError::InvalidTransition)
            ));
        }
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn transitions_record_events_without_duplicates() {
        let mut db = JobStore::in_memory().unwrap();
        db.enqueue("job", None).unwrap();
        db.advance(
            "job",
            JobState::Queued,
            JobState::Running,
            Confirmation::Unknown,
        )
        .unwrap();
        let count: i64 = db
            .db
            .query_row(
                "SELECT count(*) FROM job_events WHERE job_id='job'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 2);
        assert!(matches!(
            db.advance(
                "job",
                JobState::Queued,
                JobState::Paused,
                Confirmation::Unknown
            ),
            Err(JobStoreError::InvalidTransition)
        ));
        let after: i64 = db
            .db
            .query_row(
                "SELECT count(*) FROM job_events WHERE job_id='job'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(after, 2);
    }

    #[test]
    fn migrates_v1_jobs_without_losing_records() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE jobs (
                   id TEXT PRIMARY KEY NOT NULL,
                   state TEXT NOT NULL,
                   scenario_id TEXT,
                   scenario_revision INTEGER
                 );
                 INSERT INTO jobs VALUES ('existing','running','scenario-a',3);
                 PRAGMA user_version = 1;",
            )
            .unwrap();
        let store = JobStore::initialize(connection).unwrap();
        let job = store.get("existing").unwrap().unwrap();
        assert_eq!(job.state, JobState::Unknown);
        assert_eq!(job.scenario_revision, Some(3));
        let event_count: i64 = store
            .db
            .query_row("SELECT count(*) FROM job_events", [], |row| row.get(0))
            .unwrap();
        assert_eq!(event_count, 1);
    }

    #[test]
    fn unresolved_attempt_blocks_duplicate_and_survives_restart() {
        let mut db = JobStore::in_memory().unwrap();
        db.enqueue("job", None).unwrap();
        db.advance(
            "job",
            JobState::Queued,
            JobState::Running,
            Confirmation::Unknown,
        )
        .unwrap();
        db.begin_bound_attempt("job", "attempt-1", "copy", "destination-1", "/safe")
            .unwrap();
        assert!(matches!(
            db.resolve_attempt(
                "job",
                "attempt-1",
                &crate::verification::test_bound_receipt("other", "copy", "destination-1", "/safe")
            ),
            Err(JobStoreError::UntrustedEvidence)
        ));

        assert!(matches!(
            db.begin_bound_attempt("job", "attempt-2", "copy", "destination-1", "/safe"),
            Err(JobStoreError::InvalidTransition)
        ));
        db.resolve_attempt(
            "job",
            "attempt-1",
            &crate::verification::test_bound_receipt("attempt-1", "copy", "destination-1", "/safe"),
        )
        .unwrap();
        db.begin_bound_attempt("job", "attempt-2", "copy", "destination-1", "/safe")
            .unwrap();
    }

    #[test]
    fn unresolved_attempt_forbids_job_completion() {
        let mut db = JobStore::in_memory().unwrap();
        db.enqueue("j", None).unwrap();
        db.advance(
            "j",
            JobState::Queued,
            JobState::Running,
            Confirmation::Unknown,
        )
        .unwrap();
        db.begin_bound_attempt("j", "attempt-a", "copy", "destination-1", "/safe")
            .unwrap();
        assert!(matches!(
            db.advance(
                "j",
                JobState::Running,
                JobState::Completed,
                Confirmation::Confirmed(EvidenceLevel::ContentVerified)
            ),
            Err(JobStoreError::InvalidTransition)
        ));
        db.resolve_attempt(
            "j",
            "attempt-a",
            &crate::verification::test_bound_receipt("attempt-a", "copy", "destination-1", "/safe"),
        )
        .unwrap();
        assert_eq!(db.state("j").unwrap(), Some(JobState::Running));
    }

    #[test]
    fn restart_preserves_attempt_identity_and_blocks_duplicate() {
        use std::time::{SystemTime, UNIX_EPOCH};
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "qadren-attempt-{}-{unique}.sqlite",
            std::process::id()
        ));
        {
            let mut store = JobStore::open(&path).unwrap();
            store.enqueue("job", None).unwrap();
            store
                .advance(
                    "job",
                    JobState::Queued,
                    JobState::Running,
                    Confirmation::Unknown,
                )
                .unwrap();
            store
                .begin_bound_attempt("job", "attempt-one", "copy", "destination-1", "/safe")
                .unwrap();
        }
        {
            let mut store = JobStore::open(&path).unwrap();
            assert_eq!(store.state("job").unwrap(), Some(JobState::Unknown));
            let status: String = store
                .db
                .query_row(
                    "SELECT status FROM job_attempts WHERE attempt_id='attempt-one'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(status, "unknown");
            assert!(matches!(
                store.begin_bound_attempt("job", "attempt-two", "copy", "destination-1", "/safe"),
                Err(JobStoreError::InvalidTransition)
            ));
            store
                .resolve_attempt(
                    "job",
                    "attempt-one",
                    &crate::verification::test_bound_receipt(
                        "attempt-one",
                        "copy",
                        "destination-1",
                        "/safe",
                    ),
                )
                .unwrap();
            assert_eq!(store.state("job").unwrap(), Some(JobState::Unknown));
        }
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn verification_is_persisted_atomically_with_attempt_resolution() {
        let mut db = JobStore::in_memory().unwrap();
        db.enqueue("j", None).unwrap();
        db.advance(
            "j",
            JobState::Queued,
            JobState::Running,
            Confirmation::Unknown,
        )
        .unwrap();
        db.begin_bound_attempt("j", "a", "copy", "destination-1", "/safe")
            .unwrap();
        db.resolve_attempt(
            "j",
            "a",
            &crate::verification::test_bound_receipt("a", "copy", "destination-1", "/safe"),
        )
        .unwrap();
        let level: String = db
            .db
            .query_row(
                "SELECT evidence_level FROM attempt_verifications WHERE attempt_id='a'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(level, "metadata_verified");
        assert!(matches!(
            db.resolve_attempt(
                "j",
                "a",
                &crate::verification::test_bound_receipt("a", "copy", "destination-1", "/safe")
            ),
            Err(JobStoreError::InvalidTransition)
        ));
        let total: i64 = db
            .db
            .query_row("SELECT count(*) FROM attempt_verifications", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(total, 1);
    }

    #[test]
    fn migrates_v3_attempts_without_erasing_unknown_state() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE jobs (
                   id TEXT PRIMARY KEY NOT NULL,
                   state TEXT NOT NULL,
                   scenario_id TEXT,
                   scenario_revision INTEGER
                 );
                 CREATE TABLE job_events (
                   sequence INTEGER PRIMARY KEY AUTOINCREMENT,
                   job_id TEXT NOT NULL REFERENCES jobs(id),
                   state TEXT NOT NULL,
                   kind TEXT NOT NULL
                 );
                 CREATE TABLE job_attempts (
                   attempt_id TEXT PRIMARY KEY NOT NULL,
                   job_id TEXT NOT NULL REFERENCES jobs(id),
                   status TEXT NOT NULL
                 );
                 INSERT INTO jobs(id,state) VALUES ('j','unknown');
                 INSERT INTO job_attempts VALUES ('old-attempt','j','unknown');
                 PRAGMA user_version = 3;",
            )
            .unwrap();
        let store = JobStore::initialize(connection).unwrap();
        assert_eq!(store.state("j").unwrap(), Some(JobState::Unknown));
        let status: String = store
            .db
            .query_row(
                "SELECT status FROM job_attempts WHERE attempt_id='old-attempt'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(status, "unknown");
    }

    #[test]
    fn bound_attempt_rejects_evidence_for_another_target() {
        let mut db = JobStore::in_memory().unwrap();
        db.enqueue("j", None).unwrap();
        db.advance(
            "j",
            JobState::Queued,
            JobState::Running,
            Confirmation::Unknown,
        )
        .unwrap();
        db.begin_bound_attempt("j", "a", "copy", "destination-1", "/safe")
            .unwrap();
        let wrong = crate::verification::test_bound_receipt("a", "copy", "destination-1", "/other");
        assert!(matches!(
            db.resolve_attempt("j", "a", &wrong),
            Err(JobStoreError::UntrustedEvidence)
        ));
        let right = crate::verification::test_bound_receipt("a", "copy", "destination-1", "/safe");
        db.resolve_attempt("j", "a", &right).unwrap();
        assert_eq!(db.state("j").unwrap(), Some(JobState::Running));
    }

    #[test]
    fn inventory_is_read_only_and_does_not_resolve_attempts() {
        let mut db = JobStore::in_memory().unwrap();
        db.enqueue("job", None).unwrap();
        db.advance(
            "job",
            JobState::Queued,
            JobState::Running,
            Confirmation::Unknown,
        )
        .unwrap();
        db.begin_bound_attempt("job", "a", "copy", "destination", "/object")
            .unwrap();
        let first = db.unresolved_attempts().unwrap();
        assert_eq!(first.len(), 1);
        assert!(!first[0].is_legacy_unbound);
        assert_eq!(first[0].target_path.as_deref(), Some("/object"));
        assert_eq!(db.unresolved_attempts().unwrap(), first);
        assert_eq!(db.state("job").unwrap(), Some(JobState::Running));
    }

    #[test]
    fn legacy_v3_unknown_is_preserved_and_requires_manual_investigation() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE jobs (
                   id TEXT PRIMARY KEY NOT NULL,
                   state TEXT NOT NULL,
                   scenario_id TEXT,
                   scenario_revision INTEGER
                 );
                 CREATE TABLE job_events (
                   sequence INTEGER PRIMARY KEY AUTOINCREMENT,
                   job_id TEXT NOT NULL REFERENCES jobs(id),
                   state TEXT NOT NULL,
                   kind TEXT NOT NULL
                 );
                 CREATE TABLE job_attempts (
                   attempt_id TEXT PRIMARY KEY NOT NULL,
                   job_id TEXT NOT NULL REFERENCES jobs(id),
                   status TEXT NOT NULL
                 );
                 INSERT INTO jobs(id,state) VALUES ('legacy','unknown');
                 INSERT INTO job_attempts VALUES ('old','legacy','unknown');
                 PRAGMA user_version = 3;",
            )
            .unwrap();
        let mut db = JobStore::initialize(connection).unwrap();
        let items = db.unresolved_attempts().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0].disposition(),
            ReconciliationDisposition::NeedsManualInvestigation
        );
        let receipt = crate::verification::test_bound_receipt("old", "copy", "target", "/file");
        assert!(matches!(
            db.resolve_attempt("legacy", "old", &receipt),
            Err(JobStoreError::UntrustedEvidence)
        ));
        assert_eq!(db.unresolved_attempts().unwrap(), items);
        assert_eq!(db.state("legacy").unwrap(), Some(JobState::Unknown));
    }

    #[test]
    fn expected_metadata_is_write_once_and_bound_to_attempt() {
        let mut db = JobStore::in_memory().unwrap();
        db.enqueue("job", None).unwrap();
        db.advance(
            "job",
            JobState::Queued,
            JobState::Running,
            Confirmation::Unknown,
        )
        .unwrap();
        db.begin_bound_attempt("job", "attempt", "copy", "target", "/file")
            .unwrap();
        let checksum = "abcdef0123456789abcdef0123456789";
        db.bind_expected_metadata("job", "attempt", 123, checksum)
            .unwrap();
        let found = db.expected_metadata("job", "attempt").unwrap().unwrap();
        assert_eq!(found.target_path, "/file");
        assert_eq!(found.expected_size, 123);
        assert_eq!(found.expected_md5, checksum);
        assert!(db.expected_metadata("other", "attempt").unwrap().is_none());
        assert!(matches!(
            db.bind_expected_metadata("job", "attempt", 123, checksum),
            Err(JobStoreError::Sqlite(_))
        ));
        assert!(matches!(
            db.bind_expected_metadata("other", "attempt", 123, checksum),
            Err(JobStoreError::InvalidTransition)
        ));
    }

    #[test]
    fn migrates_v5_attempts_without_synthesizing_expectations() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE jobs (
                   id TEXT PRIMARY KEY NOT NULL, state TEXT NOT NULL,
                   scenario_id TEXT, scenario_revision INTEGER
                 );
                 CREATE TABLE job_events (
                   sequence INTEGER PRIMARY KEY AUTOINCREMENT,
                   job_id TEXT NOT NULL REFERENCES jobs(id),
                   state TEXT NOT NULL, kind TEXT NOT NULL
                 );
                 CREATE TABLE job_attempts (
                   attempt_id TEXT PRIMARY KEY NOT NULL,
                   job_id TEXT NOT NULL REFERENCES jobs(id),
                   status TEXT NOT NULL, operation_kind TEXT,
                   target_connection_id TEXT, target_path TEXT
                 );
                 CREATE TABLE attempt_verifications (
                   attempt_id TEXT PRIMARY KEY NOT NULL REFERENCES job_attempts(attempt_id),
                   evidence_level TEXT NOT NULL
                 );
                 INSERT INTO jobs(id,state) VALUES ('job','unknown');
                 INSERT INTO job_attempts VALUES
                   ('old','job','unknown','copy','target','/file');
                 PRAGMA user_version = 5;",
            )
            .unwrap();
        let store = JobStore::initialize(connection).unwrap();
        assert!(store.expected_metadata("job", "old").unwrap().is_none());
        let attempts = store.unresolved_attempts().unwrap();
        assert_eq!(attempts.len(), 1);
        assert_eq!(attempts[0].status, "unknown");
        assert!(!attempts[0].is_legacy_unbound);
    }

    #[test]
    fn read_only_yandex_inspection_cannot_resolve_job() {
        let mut db = JobStore::in_memory().unwrap();
        db.enqueue("job", None).unwrap();
        db.advance("job", JobState::Queued, JobState::Running, Confirmation::Unknown)
            .unwrap();
        db.begin_bound_attempt("job", "attempt", "copy", "dest", "disk:/file")
            .unwrap();
        let checksum = "abcdef0123456789abcdef0123456789";
        db.bind_expected_metadata("job", "attempt", 7, checksum)
            .unwrap();
        let observation = crate::yandex::Resource {
            name: "file".to_owned(),
            path: "disk:/file".to_owned(),
            kind: "file".to_owned(),
            size: Some(7),
            modified: None,
            md5: Some(checksum.to_owned()),
            revision: None,
            embedded: None,
        };
        assert_eq!(
            db.inspect_yandex_metadata("job", "attempt", "dest", &observation)
                .unwrap(),
            Some(crate::yandex::MetadataMatch::MatchesExpectedMetadata)
        );
        assert!(matches!(
            db.inspect_yandex_metadata("job", "attempt", "other", &observation),
            Err(JobStoreError::UntrustedEvidence)
        ));
        assert_eq!(db.state("job").unwrap(), Some(JobState::Running));
        assert_eq!(db.unresolved_attempts().unwrap().len(), 1);
    }

    #[test]
    fn newer_schema_is_not_downgraded() {
        let db = Connection::open_in_memory().unwrap();
        db.pragma_update(None, "user_version", 99).unwrap();
        assert!(matches!(
            JobStore::initialize(db),
            Err(JobStoreError::UnsupportedSchema(99))
        ));
    }
}
