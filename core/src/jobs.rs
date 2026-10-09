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
        if version > 1 {
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
        // A previous process may have written remote effects before dying.
        // Reconciliation is required; never resubmit these Jobs blindly.
        db.execute("UPDATE jobs SET state='unknown' WHERE state='running'", [])?;
        Ok(Self { db })
    }

    pub fn enqueue(&self, id: &str, scenario: Option<(&str, u64)>) -> Result<()> {
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
        let result = self.db.execute(
            "INSERT OR IGNORE INTO jobs (id,state,scenario_id,scenario_revision)
             VALUES (?1,'queued',?2,?3)",
            params![id, scenario_id, revision],
        )?;
        if result == 0 {
            return Err(JobStoreError::DuplicateJob);
        }
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
        &self,
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
        let changed = self.db.execute(
            "UPDATE jobs SET state=?1 WHERE id=?2 AND state=?3",
            params![label(next), id, label(expected)],
        )?;
        if changed == 0 {
            if self.get(id)?.is_none() {
                Err(JobStoreError::NotFound)
            } else {
                Err(JobStoreError::InvalidTransition)
            }
        } else {
            Ok(())
        }
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
        let db = JobStore::in_memory().unwrap();
        db.enqueue("manual", None).unwrap();
        db.enqueue("sync", Some(("scenario-1", 4))).unwrap();
        assert_eq!(db.get("manual").unwrap().unwrap().scenario_id, None);
        assert_eq!(
            db.get("sync").unwrap().unwrap().scenario_revision,
            Some(4)
        );
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
        let db = JobStore::in_memory().unwrap();
        db.enqueue("j", None).unwrap();
        db.advance(
            "j",
            JobState::Queued,
            JobState::Running,
            Confirmation::Unknown,
        )
        .unwrap();
        assert!(matches!(
            db.advance("j", JobState::Running, JobState::Completed, Confirmation::Unknown),
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
            db.advance("j", JobState::Running, JobState::Failed, Confirmation::Unknown),
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
        let path =
            std::env::temp_dir().join(format!("qadren-jobs-{}-{unique}.sqlite", std::process::id()));
        {
            let db = JobStore::open(&path).unwrap();
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
            let db = JobStore::open(&path).unwrap();
            assert_eq!(db.state("interrupted").unwrap(), Some(JobState::Unknown));
            assert_eq!(db.state("waiting").unwrap(), Some(JobState::Queued));
            assert!(matches!(
                db.advance("interrupted", JobState::Unknown, JobState::Running, Confirmation::Unknown),
                Err(JobStoreError::InvalidTransition)
            ));
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
