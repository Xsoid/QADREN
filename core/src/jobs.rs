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
        if version > 3 {
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

    /// Persist an attempt identity BEFORE any external effect may be started.
    /// A caller must not create a second attempt while any prior one is unresolved.
    pub fn begin_attempt(&mut self, job_id: &str, attempt_id: &str) -> Result<()> {
        if attempt_id.is_empty() || job_id.is_empty() {
            return Err(JobStoreError::InvalidId);
        }
        let tx = self.db.transaction()?;
        let state: Option<String> = tx
            .query_row("SELECT state FROM jobs WHERE id=?1", [job_id], |r| r.get(0))
            .optional()?;
        if state.as_deref() != Some("running") {
            return Err(JobStoreError::InvalidTransition);
        }
        let pending: i64 = tx.query_row(
            "SELECT count(*) FROM job_attempts WHERE job_id=?1 AND status!='resolved'",
            [job_id],
            |row| row.get(0),
        )?;
        if pending > 0 {
            return Err(JobStoreError::InvalidTransition);
        }
        tx.execute(
            "INSERT INTO job_attempts(attempt_id,job_id,status) VALUES (?1,?2,'active')",
            params![attempt_id, job_id],
        )?;
        tx.execute(
            "INSERT INTO job_events(job_id,state,kind) VALUES (?1,'running','attempt_started')",
            [job_id],
        )?;
        tx.commit()?;
        Ok(())
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
        db.begin_attempt("job", "attempt-1").unwrap();
        assert!(matches!(
            db.resolve_attempt("job", "attempt-1", &crate::verification::test_receipt("other")),
            Err(JobStoreError::UntrustedEvidence)
        ));

        assert!(matches!(
            db.begin_attempt("job", "attempt-2"),
            Err(JobStoreError::InvalidTransition)
        ));
        db.resolve_attempt(
            "job",
            "attempt-1",
            &crate::verification::test_receipt("attempt-1"),
        )
        .unwrap();
        db.begin_attempt("job", "attempt-2").unwrap();
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
        db.begin_attempt("j", "attempt-a").unwrap();
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
            &crate::verification::test_receipt("attempt-a"),
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
            store.begin_attempt("job", "attempt-one").unwrap();
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
                store.begin_attempt("job", "attempt-two"),
                Err(JobStoreError::InvalidTransition)
            ));
            store
                .resolve_attempt(
                    "job",
                    "attempt-one",
                    &crate::verification::test_receipt("attempt-one"),
                )
                .unwrap();
            assert_eq!(store.state("job").unwrap(), Some(JobState::Unknown));
        }
        std::fs::remove_file(path).unwrap();
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
