//! Durable Job metadata. No content bytes, credentials or transfer side effects.
//! Stored independently from StorageConnection Registry until cross-links are specified.
use crate::confirmation::Confirmation;
use crate::model::JobState;
use crate::policy::{Decision, OperationScope, ProtectedOperation};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;

#[derive(Debug)]
pub enum JobStoreError {
    Sqlite(rusqlite::Error),
    Io(std::io::Error),
    UnsupportedSchema(i32),
    InvalidId,
    InvalidTransition,
    MissingEvidence,
    UntrustedEvidence,
    NotFound,
    DuplicateJob,
    AlreadyOpen,
    UnsafeDatabaseLink,
    StorageIdentityChanged,
    ConsentMissing,
    ConsentDenied,
    ConsentExpired,
    ConsentScopeMismatch,
}
impl From<std::io::Error> for JobStoreError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}
impl From<rusqlite::Error> for JobStoreError {
    fn from(value: rusqlite::Error) -> Self {
        Self::Sqlite(value)
    }
}
pub type Result<T> = std::result::Result<T, JobStoreError>;

#[derive(Debug)]
pub enum YandexObservationError {
    Job(JobStoreError),
    Provider(crate::yandex::YandexError),
    Registry(crate::registry::RegistryError),
    ConnectionNotAuthorized,
    CredentialUnavailable,
}

impl From<crate::registry::RegistryError> for YandexObservationError {
    fn from(value: crate::registry::RegistryError) -> Self {
        Self::Registry(value)
    }
}

impl From<JobStoreError> for YandexObservationError {
    fn from(value: JobStoreError) -> Self {
        Self::Job(value)
    }
}

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

/// Durable, one-time authorization for a future dangerous effect. This is
/// metadata only: recording or consuming it never touches an adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredOperationDecision {
    pub decision_id: String,
    pub job_id: String,
    pub scope: OperationScope,
    pub decision: Decision,
    pub consumed: bool,
}

pub struct JobStore {
    db: Connection,
    // Separate SQLite file holds a process-lifetime write transaction.
    // SQLite releases this owner lock on crashes without stale PID files.
    _owner_lock: Option<Connection>,
    #[cfg(unix)]
    identity: Option<StoreIdentity>,
}

#[cfg(unix)]
struct StoreIdentity {
    database: std::path::PathBuf,
    lock: std::path::PathBuf,
    database_device: u64,
    database_inode: u64,
    lock_device: u64,
    lock_inode: u64,
}

fn label(state: JobState) -> &'static str {
    match state {
        JobState::Queued => "queued",
        JobState::Running => "running",
        JobState::Paused => "paused",
        JobState::WaitingForDecision => "waiting_for_decision",
        JobState::Completed => "completed",
        JobState::Failed => "failed",
        JobState::Cancelled => "cancelled",
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
        "cancelled" => Ok(JobState::Cancelled),
        "unknown" => Ok(JobState::Unknown),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}

impl JobStore {
    pub fn open(path: &Path) -> Result<Self> {
        // A dangling symlink must not be treated as a new database path.
        // Otherwise its alias would have an independent owner-lock name.
        #[cfg(unix)]
        if let Ok(metadata) = std::fs::symlink_metadata(path) {
            if metadata.file_type().is_symlink() && !path.exists() {
                return Err(JobStoreError::UnsafeDatabaseLink);
            }
        }
        // Canonicalize the actual database if present; otherwise resolve its
        // existing parent so relative paths and symlinked directories converge.
        let canonical = if path.exists() {
            std::fs::canonicalize(path)?
        } else {
            let filename = path.file_name().ok_or(JobStoreError::InvalidId)?;
            let parent = path
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            std::fs::canonicalize(parent)?.join(filename)
        };
        // Two hard links can name the same SQLite inode but produce distinct
        // owner lock paths. Refuse that layout rather than risking two writers.
        #[cfg(unix)]
        if canonical.exists() {
            use std::os::unix::fs::MetadataExt;
            if std::fs::metadata(&canonical)?.nlink() > 1 {
                return Err(JobStoreError::UnsafeDatabaseLink);
            }
        }
        let mut lock_path = canonical.as_os_str().to_os_string();
        lock_path.push(".owner.sqlite");
        let owner = Connection::open(Path::new(&lock_path))?;
        owner.busy_timeout(std::time::Duration::from_millis(0))?;
        match owner.execute_batch("BEGIN IMMEDIATE;") {
            Ok(()) => {}
            Err(rusqlite::Error::SqliteFailure(error, _))
                if error.code == rusqlite::ErrorCode::DatabaseBusy
                    || error.code == rusqlite::ErrorCode::DatabaseLocked =>
            {
                return Err(JobStoreError::AlreadyOpen);
            }
            Err(error) => return Err(JobStoreError::Sqlite(error)),
        }
        let mut store = Self::initialize(Connection::open(&canonical)?, Some(owner))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let database_metadata = std::fs::metadata(&canonical)?;
            let lock_metadata = std::fs::metadata(Path::new(&lock_path))?;
            store.identity = Some(StoreIdentity {
                database: canonical,
                lock: Path::new(&lock_path).to_path_buf(),
                database_device: database_metadata.dev(),
                database_inode: database_metadata.ino(),
                lock_device: lock_metadata.dev(),
                lock_inode: lock_metadata.ino(),
            });
        }
        Ok(store)
    }

    pub fn in_memory() -> Result<Self> {
        Self::initialize(Connection::open_in_memory()?, None)
    }

    fn initialize(db: Connection, owner_lock: Option<Connection>) -> Result<Self> {
        let version: i32 = db.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version > 9 {
            return Err(JobStoreError::UnsupportedSchema(version));
        }
        if version == 0 {
            db.execute_batch(
                "BEGIN IMMEDIATE;
                 CREATE TABLE jobs (
                   id TEXT PRIMARY KEY NOT NULL,
                   state TEXT NOT NULL CHECK (state IN
                     ('queued','running','paused','waiting_for_decision','completed','failed','cancelled','unknown')),
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
        if version <= 6 {
            // SQLite cannot expand a CHECK constraint in place. Rebuild the
            // small metadata-only graph in one transaction, preserving every
            // existing Job, event, attempt and verification record.
            db.execute_batch(
                "PRAGMA foreign_keys = OFF;
                 BEGIN IMMEDIATE;
                 CREATE TABLE jobs_new (
                   id TEXT PRIMARY KEY NOT NULL,
                   state TEXT NOT NULL CHECK (state IN
                     ('queued','running','paused','waiting_for_decision','completed','failed','cancelled','unknown')),
                   scenario_id TEXT,
                   scenario_revision INTEGER,
                   CHECK ((scenario_id IS NULL AND scenario_revision IS NULL) OR
                          (scenario_id IS NOT NULL AND scenario_revision IS NOT NULL AND scenario_revision > 0))
                 );
                 INSERT INTO jobs_new SELECT id,state,scenario_id,scenario_revision FROM jobs;
                 CREATE TABLE job_events_new (
                   sequence INTEGER PRIMARY KEY AUTOINCREMENT,
                   job_id TEXT NOT NULL REFERENCES jobs_new(id),
                   state TEXT NOT NULL,
                   kind TEXT NOT NULL
                 );
                 INSERT INTO job_events_new SELECT sequence,job_id,state,kind FROM job_events;
                 CREATE TABLE job_attempts_new (
                   attempt_id TEXT PRIMARY KEY NOT NULL,
                   job_id TEXT NOT NULL REFERENCES jobs_new(id),
                   status TEXT NOT NULL CHECK (status IN ('active','unknown','resolved')),
                   operation_kind TEXT,
                   target_connection_id TEXT,
                   target_path TEXT,
                   UNIQUE(job_id, attempt_id)
                 );
                 INSERT INTO job_attempts_new
                   SELECT attempt_id,job_id,status,operation_kind,target_connection_id,target_path FROM job_attempts;
                 CREATE TABLE attempt_verifications_new (
                   attempt_id TEXT PRIMARY KEY NOT NULL REFERENCES job_attempts_new(attempt_id),
                   evidence_level TEXT NOT NULL CHECK (evidence_level IN
                     ('metadata_verified','content_verified'))
                 );
                 INSERT INTO attempt_verifications_new SELECT attempt_id,evidence_level FROM attempt_verifications;
                 CREATE TABLE attempt_expectations_new (
                   attempt_id TEXT PRIMARY KEY NOT NULL REFERENCES job_attempts_new(attempt_id),
                   expected_size INTEGER NOT NULL CHECK(expected_size >= 0),
                   expected_md5 TEXT NOT NULL CHECK(length(expected_md5) = 32)
                 );
                 INSERT INTO attempt_expectations_new SELECT attempt_id,expected_size,expected_md5 FROM attempt_expectations;
                 DROP TABLE attempt_expectations;
                 DROP TABLE attempt_verifications;
                 DROP TABLE job_attempts;
                 DROP TABLE job_events;
                 DROP TABLE jobs;
                 ALTER TABLE jobs_new RENAME TO jobs;
                 ALTER TABLE job_events_new RENAME TO job_events;
                 ALTER TABLE job_attempts_new RENAME TO job_attempts;
                 ALTER TABLE attempt_verifications_new RENAME TO attempt_verifications;
                 ALTER TABLE attempt_expectations_new RENAME TO attempt_expectations;
                 PRAGMA user_version = 7;
                 COMMIT;
                 PRAGMA foreign_keys = ON;",
            )?;
        }
        if version <= 7 {
            db.execute_batch(
                "BEGIN IMMEDIATE;
                 CREATE TABLE operation_decisions (
                   decision_id TEXT PRIMARY KEY NOT NULL,
                   job_id TEXT NOT NULL REFERENCES jobs(id),
                   operation_kind TEXT NOT NULL CHECK(operation_kind IN ('overwrite','delete','rename')),
                   connection_id TEXT NOT NULL,
                   relative_path TEXT NOT NULL,
                   expires_at INTEGER NOT NULL CHECK(expires_at >= 0),
                   decision TEXT NOT NULL CHECK(decision IN ('accepted','denied')),
                   consumed INTEGER NOT NULL DEFAULT 0 CHECK(consumed IN (0,1))
                 );
                 CREATE INDEX operation_decisions_for_job
                   ON operation_decisions(job_id, consumed);
                 PRAGMA user_version = 8;
                 COMMIT;",
            )?;
        }
        if version <= 8 {
            // A known refusal before the adapter can create an effect is not
            // UNKNOWN. Rebuild the attempt graph to distinguish it from both
            // verified resolution and an ambiguous active/unknown attempt.
            db.execute_batch(
                "PRAGMA foreign_keys = OFF;
                 BEGIN IMMEDIATE;
                 CREATE TABLE job_attempts_new (
                   attempt_id TEXT PRIMARY KEY NOT NULL,
                   job_id TEXT NOT NULL REFERENCES jobs(id),
                   status TEXT NOT NULL CHECK (status IN ('active','unknown','resolved','failed_before_effect')),
                   operation_kind TEXT,
                   target_connection_id TEXT,
                   target_path TEXT,
                   UNIQUE(job_id, attempt_id)
                 );
                 INSERT INTO job_attempts_new
                   SELECT attempt_id,job_id,status,operation_kind,target_connection_id,target_path FROM job_attempts;
                 CREATE TABLE attempt_verifications_new (
                   attempt_id TEXT PRIMARY KEY NOT NULL REFERENCES job_attempts_new(attempt_id),
                   evidence_level TEXT NOT NULL CHECK (evidence_level IN
                     ('metadata_verified','content_verified'))
                 );
                 INSERT INTO attempt_verifications_new SELECT attempt_id,evidence_level FROM attempt_verifications;
                 CREATE TABLE attempt_expectations_new (
                   attempt_id TEXT PRIMARY KEY NOT NULL REFERENCES job_attempts_new(attempt_id),
                   expected_size INTEGER NOT NULL CHECK(expected_size >= 0),
                   expected_md5 TEXT NOT NULL CHECK(length(expected_md5) = 32)
                 );
                 INSERT INTO attempt_expectations_new SELECT attempt_id,expected_size,expected_md5 FROM attempt_expectations;
                 DROP TABLE attempt_expectations;
                 DROP TABLE attempt_verifications;
                 DROP TABLE job_attempts;
                 ALTER TABLE job_attempts_new RENAME TO job_attempts;
                 ALTER TABLE attempt_verifications_new RENAME TO attempt_verifications;
                 ALTER TABLE attempt_expectations_new RENAME TO attempt_expectations;
                 CREATE UNIQUE INDEX one_active_attempt_per_job
                   ON job_attempts(job_id) WHERE status='active';
                 PRAGMA user_version = 9;
                 COMMIT;
                 PRAGMA foreign_keys = ON;",
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
        Ok(Self {
            db,
            _owner_lock: owner_lock,
            #[cfg(unix)]
            identity: None,
        })
    }

    /// Refuse state mutations when an external actor replaces or unlinks the
    /// database or its lifetime lock. This does not replace directory isolation.
    fn check_storage_identity(&self) -> Result<()> {
        #[cfg(unix)]
        if let Some(identity) = &self.identity {
            use std::os::unix::fs::MetadataExt;
            let database = std::fs::metadata(&identity.database)
                .map_err(|_| JobStoreError::StorageIdentityChanged)?;
            let lock = std::fs::metadata(&identity.lock)
                .map_err(|_| JobStoreError::StorageIdentityChanged)?;
            if database.dev() != identity.database_device
                || database.ino() != identity.database_inode
                || database.nlink() != 1
                || lock.dev() != identity.lock_device
                || lock.ino() != identity.lock_inode
                || lock.nlink() != 1
            {
                return Err(JobStoreError::StorageIdentityChanged);
            }
        }
        Ok(())
    }

    pub fn enqueue(&mut self, id: &str, scenario: Option<(&str, u64)>) -> Result<()> {
        self.check_storage_identity()?;
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

    /// List persisted jobs for read-only UI/Controller monitoring.
    /// This query never starts or changes an operation.
    pub fn list_jobs(&self) -> Result<Vec<StoredJob>> {
        self.check_storage_identity()?;
        let mut statement = self.db.prepare(
            "SELECT id,state,scenario_id,scenario_revision FROM jobs ORDER BY rowid ASC",
        )?;
        let jobs = statement.query_map([], |row| {
            let state: String = row.get(1)?;
            let revision: Option<i64> = row.get(3)?;
            Ok(StoredJob {
                id: row.get(0)?,
                state: parse_state(&state)?,
                scenario_id: row.get(2)?,
                scenario_revision: revision.map(|value| value as u64),
            })
        })?;
        jobs.collect::<rusqlite::Result<Vec<_>>>()
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
        self.check_storage_identity()?;
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
        if matches!(next, JobState::Completed | JobState::Cancelled) {
            let unresolved: i64 = tx.query_row(
                "SELECT count(*) FROM job_attempts WHERE job_id=?1 AND status IN ('active','unknown')",
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

    /// Stop a Job only before any operation attempt is active or unresolved.
    /// Once a durable attempt exists, cancellation cannot erase a possibly
    /// external effect; reconciliation must classify it instead.
    pub fn cancel(&mut self, id: &str) -> Result<()> {
        self.check_storage_identity()?;
        let tx = self.db.transaction()?;
        let state: Option<String> = tx
            .query_row("SELECT state FROM jobs WHERE id=?1", [id], |row| row.get(0))
            .optional()?;
        let Some(state) = state else {
            return Err(JobStoreError::NotFound);
        };
        let current = parse_state(&state)?;
        if current
            .transition(JobState::Cancelled, Confirmation::Unknown)
            .is_err()
        {
            return Err(JobStoreError::InvalidTransition);
        }
        let unresolved: i64 = tx.query_row(
            "SELECT count(*) FROM job_attempts WHERE job_id=?1 AND status IN ('active','unknown')",
            [id],
            |row| row.get(0),
        )?;
        if unresolved != 0 {
            return Err(JobStoreError::InvalidTransition);
        }
        let changed = tx.execute(
            "UPDATE jobs SET state='cancelled' WHERE id=?1 AND state=?2",
            params![id, state],
        )?;
        if changed != 1 {
            return Err(JobStoreError::InvalidTransition);
        }
        tx.execute(
            "INSERT INTO job_events(job_id,state,kind) VALUES (?1,'cancelled','cancelled_before_effect')",
            [id],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Pause a Job only before any operation attempt is active or unresolved.
    /// Checkpointing an in-flight effect needs a separate adapter contract;
    /// this guard must not pretend that an active transfer was paused.
    pub fn pause(&mut self, id: &str) -> Result<()> {
        self.check_storage_identity()?;
        let tx = self.db.transaction()?;
        let state: Option<String> = tx
            .query_row("SELECT state FROM jobs WHERE id=?1", [id], |row| row.get(0))
            .optional()?;
        let Some(state) = state else {
            return Err(JobStoreError::NotFound);
        };
        let current = parse_state(&state)?;
        if current
            .transition(JobState::Paused, Confirmation::Unknown)
            .is_err()
        {
            return Err(JobStoreError::InvalidTransition);
        }
        let unresolved: i64 = tx.query_row(
            "SELECT count(*) FROM job_attempts WHERE job_id=?1 AND status IN ('active','unknown')",
            [id],
            |row| row.get(0),
        )?;
        if unresolved != 0 {
            return Err(JobStoreError::InvalidTransition);
        }
        let changed = tx.execute(
            "UPDATE jobs SET state='paused' WHERE id=?1 AND state=?2",
            params![id, state],
        )?;
        if changed != 1 {
            return Err(JobStoreError::InvalidTransition);
        }
        tx.execute(
            "INSERT INTO job_events(job_id,state,kind) VALUES (?1,'paused','paused_before_effect')",
            [id],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Resume a pre-effect paused Job by returning it to the durable queue.
    pub fn resume(&mut self, id: &str) -> Result<()> {
        self.advance(
            id,
            JobState::Paused,
            JobState::Queued,
            Confirmation::Unknown,
        )
    }

    /// Record an explicit decision while the Job is safely waiting before an
    /// external effect. There are no wildcards for operation, connection or path.
    pub fn record_operation_decision(
        &mut self,
        decision_id: &str,
        job_id: &str,
        scope: &OperationScope,
        decision: Decision,
    ) -> Result<()> {
        self.check_storage_identity()?;
        if decision_id.is_empty()
            || job_id.is_empty()
            || scope.connection_id.is_empty()
            || !crate::policy::valid_path(&scope.relative_path)
            || scope.expires_at > i64::MAX as u64
        {
            return Err(JobStoreError::InvalidId);
        }
        let tx = self.db.transaction()?;
        let state: Option<String> = tx
            .query_row("SELECT state FROM jobs WHERE id=?1", [job_id], |row| {
                row.get(0)
            })
            .optional()?;
        match state.as_deref() {
            None => return Err(JobStoreError::NotFound),
            Some("waiting_for_decision") => {}
            Some(_) => return Err(JobStoreError::InvalidTransition),
        }
        let unresolved: i64 = tx.query_row(
            "SELECT count(*) FROM job_attempts WHERE job_id=?1 AND status IN ('active','unknown')",
            [job_id],
            |row| row.get(0),
        )?;
        if unresolved != 0 {
            return Err(JobStoreError::InvalidTransition);
        }
        tx.execute(
            "INSERT INTO operation_decisions
              (decision_id,job_id,operation_kind,connection_id,relative_path,expires_at,decision)
             VALUES (?1,?2,?3,?4,?5,?6,?7)",
            params![
                decision_id,
                job_id,
                scope.operation.label(),
                scope.connection_id,
                scope.relative_path,
                scope.expires_at as i64,
                match decision {
                    Decision::Accepted => "accepted",
                    Decision::Denied => "denied",
                },
            ],
        )?;
        tx.execute(
            "INSERT INTO job_events(job_id,state,kind) VALUES (?1,'waiting_for_decision','decision_recorded')",
            [job_id],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Atomically consume one accepted decision for the exact requested
    /// operation. A crash after this call leaves the consent spent and the Job
    /// waiting; a new explicit decision is required before any retry.
    pub fn consume_operation_decision(
        &mut self,
        decision_id: &str,
        job_id: &str,
        requested: &OperationScope,
        now: u64,
    ) -> Result<StoredOperationDecision> {
        self.check_storage_identity()?;
        if decision_id.is_empty()
            || job_id.is_empty()
            || requested.connection_id.is_empty()
            || !crate::policy::valid_path(&requested.relative_path)
            || now > i64::MAX as u64
        {
            return Err(JobStoreError::InvalidId);
        }
        let tx = self.db.transaction()?;
        let stored: Option<(String, String, String, String, i64, String, i64)> = tx
            .query_row(
                "SELECT operation_kind,connection_id,relative_path,decision,expires_at,job_id,consumed
                 FROM operation_decisions WHERE decision_id=?1",
                [decision_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?)),
            )
            .optional()?;
        let Some((
            operation,
            connection_id,
            relative_path,
            decision,
            expires_at,
            stored_job_id,
            consumed,
        )) = stored
        else {
            return Err(JobStoreError::ConsentMissing);
        };
        let operation =
            ProtectedOperation::parse(&operation).ok_or(JobStoreError::ConsentScopeMismatch)?;
        if stored_job_id != job_id
            || operation != requested.operation
            || connection_id != requested.connection_id
            || relative_path != requested.relative_path
        {
            return Err(JobStoreError::ConsentScopeMismatch);
        }
        if decision != "accepted" {
            return Err(JobStoreError::ConsentDenied);
        }
        if expires_at < 0 || now > expires_at as u64 {
            return Err(JobStoreError::ConsentExpired);
        }
        if consumed != 0 {
            return Err(JobStoreError::ConsentMissing);
        }
        let state: String =
            tx.query_row("SELECT state FROM jobs WHERE id=?1", [job_id], |row| {
                row.get(0)
            })?;
        if state != "waiting_for_decision" {
            return Err(JobStoreError::InvalidTransition);
        }
        let changed = tx.execute(
            "UPDATE operation_decisions SET consumed=1 WHERE decision_id=?1 AND consumed=0",
            [decision_id],
        )?;
        if changed != 1 {
            return Err(JobStoreError::ConsentMissing);
        }
        tx.execute(
            "INSERT INTO job_events(job_id,state,kind) VALUES (?1,'waiting_for_decision','decision_consumed')",
            [job_id],
        )?;
        tx.commit()?;
        Ok(StoredOperationDecision {
            decision_id: decision_id.to_owned(),
            job_id: job_id.to_owned(),
            scope: OperationScope {
                operation,
                connection_id,
                relative_path,
                expires_at: expires_at as u64,
            },
            decision: Decision::Accepted,
            consumed: true,
        })
    }

    /// Atomically prepare an operation and its immutable expected metadata.
    /// Persist this transaction BEFORE initiating any external effect.
    #[allow(clippy::too_many_arguments)] // Atomic preparation takes a complete operation identity.
    pub fn prepare_bound_attempt(
        &mut self,
        job_id: &str,
        attempt_id: &str,
        operation_kind: &str,
        target_connection_id: &str,
        target_path: &str,
        expected_size: u64,
        expected_md5: &str,
    ) -> Result<()> {
        self.check_storage_identity()?;
        if job_id.is_empty()
            || attempt_id.is_empty()
            || operation_kind.is_empty()
            || target_connection_id.is_empty()
            || target_path.is_empty()
            || expected_size > i64::MAX as u64
            || expected_md5.len() != 32
            || !expected_md5.bytes().all(|b| b.is_ascii_hexdigit())
        {
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
            "SELECT count(*) FROM job_attempts WHERE job_id=?1 AND status IN ('active','unknown')",
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
            "INSERT INTO attempt_expectations(attempt_id,expected_size,expected_md5)
             VALUES (?1,?2,?3)",
            params![
                attempt_id,
                expected_size as i64,
                expected_md5.to_ascii_lowercase()
            ],
        )?;
        tx.execute(
            "INSERT INTO job_events(job_id,state,kind) VALUES (?1,'running','attempt_started')",
            [job_id],
        )?;
        tx.commit()?;
        Ok(())
    }

    // Historical split helpers exist only for migration/regression tests.
    // Production callers must use prepare_bound_attempt.
    #[cfg(test)]
    pub fn begin_bound_attempt(
        &mut self,
        job_id: &str,
        attempt_id: &str,
        operation_kind: &str,
        target_connection_id: &str,
        target_path: &str,
    ) -> Result<()> {
        self.check_storage_identity()?;
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
            "SELECT count(*) FROM job_attempts WHERE job_id=?1 AND status IN ('active','unknown')",
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
    #[cfg(test)]
    pub fn bind_expected_metadata(
        &mut self,
        job_id: &str,
        attempt_id: &str,
        expected_size: u64,
        expected_md5: &str,
    ) -> Result<()> {
        self.check_storage_identity()?;
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

    /// Resolve credentials only through the registered connection's secure-store reference.
    /// The resolver must be backed by macOS Keychain or equivalent platform storage.
    /// The provider account identity must have been independently established during
    /// connection authorization; this method does not authenticate account ownership.
    pub fn observe_registered_yandex_attempt<F>(
        &self,
        job_id: &str,
        attempt_id: &str,
        registry: &crate::registry::Registry,
        client: &crate::yandex::YandexMetadataClient,
        resolve_credential: F,
    ) -> std::result::Result<crate::yandex::MetadataMatch, YandexObservationError>
    where
        F: FnOnce(&str) -> Option<String>,
    {
        self.check_storage_identity()?;
        let expected = self
            .expected_metadata(job_id, attempt_id)?
            .ok_or(JobStoreError::MissingEvidence)?;
        let connection = registry
            .get(&expected.target_connection_id)?
            .ok_or(YandexObservationError::ConnectionNotAuthorized)?;
        if connection.adapter != "yandex_disk"
            || !connection.enabled
            || connection.status != "connected"
            || connection
                .provider_account_id
                .as_deref()
                .is_none_or(str::is_empty)
        {
            return Err(YandexObservationError::ConnectionNotAuthorized);
        }
        let credential_ref = connection
            .credential_ref
            .as_deref()
            .filter(|reference| !reference.is_empty())
            .ok_or(YandexObservationError::CredentialUnavailable)?;
        let token = resolve_credential(credential_ref)
            .filter(|value| !value.is_empty())
            .ok_or(YandexObservationError::CredentialUnavailable)?;
        let actual_uid = client
            .account_uid(&token)
            .map_err(YandexObservationError::Provider)?;
        if connection.provider_account_id.as_deref() != Some(actual_uid.as_str()) {
            return Err(YandexObservationError::ConnectionNotAuthorized);
        }
        self.observe_yandex_attempt(job_id, attempt_id, &connection.id, client, &token)
    }

    /// Perform a read-only provider metadata lookup for this exact recorded target.
    /// Credentials are supplied by the caller and are never persisted in JobStore.
    /// This is observation only: no transfer, receipt, retry or Job transition.
    fn observe_yandex_attempt(
        &self,
        job_id: &str,
        attempt_id: &str,
        target_connection_id: &str,
        client: &crate::yandex::YandexMetadataClient,
        token: &str,
    ) -> std::result::Result<crate::yandex::MetadataMatch, YandexObservationError> {
        self.check_storage_identity()?;
        let expected = self
            .expected_metadata(job_id, attempt_id)?
            .ok_or(JobStoreError::MissingEvidence)?;
        if expected.target_connection_id != target_connection_id {
            return Err(JobStoreError::UntrustedEvidence.into());
        }
        if expected.operation_kind != "copy" {
            return Err(JobStoreError::InvalidTransition.into());
        }
        let active: Option<String> = self
            .db
            .query_row(
                "SELECT status FROM job_attempts WHERE job_id=?1 AND attempt_id=?2",
                params![job_id, attempt_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(JobStoreError::from)?;
        if !matches!(active.as_deref(), Some("active" | "unknown")) {
            return Err(JobStoreError::InvalidTransition.into());
        }
        let expected_resource = crate::yandex::ExpectedResourceMetadata {
            path: expected.target_path,
            size: expected.expected_size,
            md5: expected.expected_md5,
        };
        match client.reconcile_uploaded_resource(token, &expected_resource) {
            Ok(observation) => Ok(observation),
            Err(crate::yandex::YandexError::NotFound) => Ok(crate::yandex::MetadataMatch::Mismatch),
            Err(error) => Err(YandexObservationError::Provider(error)),
        }
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
        self.check_storage_identity()?;
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

    /// Atomically persist a verified receipt and terminal Job completion.
    ///
    /// This is the normal executor/reconciliation success boundary. Keeping
    /// the attempt resolution and Job state in one SQLite transaction avoids a
    /// crash leaving a resolved attempt under a durable UNKNOWN Job that can no
    /// longer be reconciled or retried safely.
    pub fn complete_attempt(
        &mut self,
        job_id: &str,
        attempt_id: &str,
        receipt: &crate::verification::VerifiedReceipt,
    ) -> Result<()> {
        self.check_storage_identity()?;
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
            return Err(JobStoreError::UntrustedEvidence);
        }
        let evidence = receipt.confirmation();
        if !evidence.meets(crate::confirmation::EvidenceLevel::MetadataVerified) {
            return Err(JobStoreError::MissingEvidence);
        }
        let tx = self.db.transaction()?;
        let state: Option<String> = tx
            .query_row("SELECT state FROM jobs WHERE id=?1", [job_id], |row| {
                row.get(0)
            })
            .optional()?;
        let Some(state) = state else {
            return Err(JobStoreError::NotFound);
        };
        let current = parse_state(&state)?;
        if current.transition(JobState::Completed, evidence).is_err() {
            return Err(JobStoreError::InvalidTransition);
        }
        let updated_attempt = tx.execute(
            "UPDATE job_attempts SET status='resolved'
             WHERE job_id=?1 AND attempt_id=?2 AND status IN ('active','unknown')",
            params![job_id, attempt_id],
        )?;
        if updated_attempt != 1 {
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
        let updated_job = tx.execute(
            "UPDATE jobs SET state='completed' WHERE id=?1 AND state=?2",
            params![job_id, state],
        )?;
        if updated_job != 1 {
            return Err(JobStoreError::InvalidTransition);
        }
        tx.execute(
            "INSERT INTO job_events(job_id,state,kind) VALUES (?1,'completed','attempt_resolved')",
            [job_id],
        )?;
        tx.execute(
            "INSERT INTO job_events(job_id,state,kind) VALUES (?1,'completed','transition')",
            [job_id],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Record a refusal that the adapter proves occurred before it could make
    /// an external effect (for example, an atomic create-new collision).
    /// This is intentionally distinct from verification: no receipt is
    /// fabricated, and any I/O result that could be ambiguous stays UNKNOWN.
    pub fn fail_attempt_before_effect(&mut self, job_id: &str, attempt_id: &str) -> Result<()> {
        self.check_storage_identity()?;
        let tx = self.db.transaction()?;
        let state: Option<String> = tx
            .query_row("SELECT state FROM jobs WHERE id=?1", [job_id], |row| {
                row.get(0)
            })
            .optional()?;
        if state.as_deref() != Some("running") {
            return Err(JobStoreError::InvalidTransition);
        }
        if JobState::Running
            .transition(
                JobState::Failed,
                Confirmation::Confirmed(crate::confirmation::EvidenceLevel::Requested),
            )
            .is_err()
        {
            return Err(JobStoreError::InvalidTransition);
        }
        let changed_attempt = tx.execute(
            "UPDATE job_attempts SET status='failed_before_effect'
             WHERE job_id=?1 AND attempt_id=?2 AND status='active'",
            params![job_id, attempt_id],
        )?;
        if changed_attempt != 1 {
            return Err(JobStoreError::InvalidTransition);
        }
        let changed_job = tx.execute(
            "UPDATE jobs SET state='failed' WHERE id=?1 AND state='running'",
            [job_id],
        )?;
        if changed_job != 1 {
            return Err(JobStoreError::InvalidTransition);
        }
        tx.execute(
            "INSERT INTO job_events(job_id,state,kind)
             VALUES (?1,'failed','attempt_failed_before_effect')",
            [job_id],
        )?;
        tx.execute(
            "INSERT INTO job_events(job_id,state,kind) VALUES (?1,'failed','transition')",
            [job_id],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Inventory for operator investigation. Never changes or retries attempts.
    pub fn unresolved_attempts(&self) -> Result<Vec<UnresolvedAttempt>> {
        self.check_storage_identity()?;
        let mut statement = self.db.prepare(
            "SELECT attempt_id,job_id,status,operation_kind,target_connection_id,target_path
             FROM job_attempts WHERE status IN ('active','unknown') ORDER BY job_id,attempt_id",
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

    /// True when clearing this connection's credential could strand an
    /// unresolved operation. Historical attempts without a durable target
    /// binding also block cleanup: their target is unknowable, so allowing any
    /// credential removal would be a blind safety decision.
    pub fn connection_has_unresolved_attempts(&self, connection_id: &str) -> Result<bool> {
        self.check_storage_identity()?;
        if connection_id.is_empty() {
            return Err(JobStoreError::InvalidId);
        }
        let blocked: i64 = self.db.query_row(
            "SELECT EXISTS(
                SELECT 1 FROM job_attempts
                WHERE status IN ('active','unknown')
                  AND (target_connection_id = ?1 OR target_connection_id IS NULL)
            )",
            [connection_id],
            |row| row.get(0),
        )?;
        Ok(blocked != 0)
    }

    pub fn state(&self, id: &str) -> Result<Option<JobState>> {
        Ok(self.get(id)?.map(|job| job.state))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::confirmation::EvidenceLevel;
    use crate::policy::{Decision, OperationScope, ProtectedOperation};
    use std::io::{Read, Write};
    use std::net::TcpListener;

    fn waiting_job(store: &mut JobStore, id: &str) {
        store.enqueue(id, None).unwrap();
        store
            .advance(
                id,
                JobState::Queued,
                JobState::Running,
                Confirmation::Unknown,
            )
            .unwrap();
        store
            .advance(
                id,
                JobState::Running,
                JobState::WaitingForDecision,
                Confirmation::Unknown,
            )
            .unwrap();
    }

    fn destructive_scope() -> OperationScope {
        OperationScope {
            operation: ProtectedOperation::Delete,
            connection_id: "connection-a".into(),
            relative_path: "folder/file.txt".into(),
            expires_at: 100,
        }
    }

    #[test]
    fn durable_scoped_decision_is_exact_and_single_use() {
        let mut store = JobStore::in_memory().unwrap();
        waiting_job(&mut store, "job-a");
        let scope = destructive_scope();
        store
            .record_operation_decision("decision-a", "job-a", &scope, Decision::Accepted)
            .unwrap();
        let consumed = store
            .consume_operation_decision("decision-a", "job-a", &scope, 100)
            .unwrap();
        assert!(consumed.consumed);
        assert_eq!(consumed.scope, scope);
        assert!(matches!(
            store.consume_operation_decision("decision-a", "job-a", &scope, 100),
            Err(JobStoreError::ConsentMissing)
        ));
    }

    #[test]
    fn dangerous_decision_fails_closed_for_denial_expiry_or_scope_mismatch() {
        let mut store = JobStore::in_memory().unwrap();
        waiting_job(&mut store, "job-a");
        let scope = destructive_scope();
        store
            .record_operation_decision("denied", "job-a", &scope, Decision::Denied)
            .unwrap();
        assert!(matches!(
            store.consume_operation_decision("denied", "job-a", &scope, 1),
            Err(JobStoreError::ConsentDenied)
        ));
        let expired = OperationScope {
            expires_at: 5,
            ..scope.clone()
        };
        store
            .record_operation_decision("expired", "job-a", &expired, Decision::Accepted)
            .unwrap();
        assert!(matches!(
            store.consume_operation_decision("expired", "job-a", &expired, 6),
            Err(JobStoreError::ConsentExpired)
        ));
        store
            .record_operation_decision("mismatch", "job-a", &scope, Decision::Accepted)
            .unwrap();
        let other = OperationScope {
            relative_path: "other.txt".into(),
            ..scope
        };
        assert!(matches!(
            store.consume_operation_decision("mismatch", "job-a", &other, 1),
            Err(JobStoreError::ConsentScopeMismatch)
        ));
    }

    #[test]
    fn list_jobs_is_read_only_and_preserves_scenario_identity() {
        let mut store = JobStore::in_memory().unwrap();
        store.enqueue("job-a", None).unwrap();
        store.enqueue("job-b", Some(("scenario-a", 2))).unwrap();
        let jobs = store.list_jobs().unwrap();
        assert_eq!(jobs.len(), 2);
        assert_eq!(jobs[0].id, "job-a");
        assert_eq!(jobs[0].state, JobState::Queued);
        assert_eq!(jobs[0].scenario_id, None);
        assert_eq!(jobs[1].id, "job-b");
        assert_eq!(jobs[1].scenario_id.as_deref(), Some("scenario-a"));
        assert_eq!(jobs[1].scenario_revision, Some(2));
        assert_eq!(store.list_jobs().unwrap().len(), 2);
    }

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
    fn cancellation_is_terminal_only_before_an_attempt_is_active_or_unknown() {
        let mut db = JobStore::in_memory().unwrap();
        db.enqueue("queued", None).unwrap();
        db.cancel("queued").unwrap();
        assert_eq!(db.state("queued").unwrap(), Some(JobState::Cancelled));
        assert!(matches!(
            db.cancel("queued"),
            Err(JobStoreError::InvalidTransition)
        ));

        db.enqueue("effect", None).unwrap();
        db.advance(
            "effect",
            JobState::Queued,
            JobState::Running,
            Confirmation::Unknown,
        )
        .unwrap();
        db.prepare_bound_attempt(
            "effect",
            "effect-attempt",
            "copy",
            "target",
            "/new",
            1,
            "900150983cd24fb0d6963f7d28e17f72",
        )
        .unwrap();
        assert!(matches!(
            db.cancel("effect"),
            Err(JobStoreError::InvalidTransition)
        ));
        assert_eq!(db.state("effect").unwrap(), Some(JobState::Running));
    }

    #[test]
    fn pause_and_resume_are_durable_but_active_effects_cannot_pause() {
        let mut db = JobStore::in_memory().unwrap();
        db.enqueue("queued", None).unwrap();
        db.pause("queued").unwrap();
        assert_eq!(db.state("queued").unwrap(), Some(JobState::Paused));
        db.resume("queued").unwrap();
        assert_eq!(db.state("queued").unwrap(), Some(JobState::Queued));

        db.enqueue("effect", None).unwrap();
        db.advance(
            "effect",
            JobState::Queued,
            JobState::Running,
            Confirmation::Unknown,
        )
        .unwrap();
        db.prepare_bound_attempt(
            "effect",
            "effect-attempt",
            "copy",
            "target",
            "/new",
            1,
            "900150983cd24fb0d6963f7d28e17f72",
        )
        .unwrap();
        assert!(matches!(
            db.pause("effect"),
            Err(JobStoreError::InvalidTransition)
        ));
        assert_eq!(db.state("effect").unwrap(), Some(JobState::Running));
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
        let store = JobStore::initialize(connection, None).unwrap();
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
    fn verified_completion_updates_attempt_and_job_together() {
        let mut db = JobStore::in_memory().unwrap();
        db.enqueue("job", None).unwrap();
        db.advance(
            "job",
            JobState::Queued,
            JobState::Running,
            Confirmation::Unknown,
        )
        .unwrap();
        db.prepare_bound_attempt(
            "job",
            "attempt",
            "copy",
            "destination",
            "/new-file",
            1,
            "900150983cd24fb0d6963f7d28e17f72",
        )
        .unwrap();

        let receipt =
            crate::verification::test_bound_receipt("attempt", "copy", "destination", "/new-file");
        db.complete_attempt("job", "attempt", &receipt).unwrap();

        assert_eq!(db.state("job").unwrap(), Some(JobState::Completed));
        assert!(db.unresolved_attempts().unwrap().is_empty());
        let evidence: String = db
            .db
            .query_row(
                "SELECT evidence_level FROM attempt_verifications WHERE attempt_id='attempt'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(evidence, "metadata_verified");
        assert!(matches!(
            db.complete_attempt("job", "attempt", &receipt),
            Err(JobStoreError::InvalidTransition)
        ));
    }

    #[test]
    fn proven_pre_effect_refusal_fails_without_creating_an_unknown_attempt() {
        let mut db = JobStore::in_memory().unwrap();
        db.enqueue("job", None).unwrap();
        db.advance(
            "job",
            JobState::Queued,
            JobState::Running,
            Confirmation::Unknown,
        )
        .unwrap();
        db.prepare_bound_attempt(
            "job",
            "attempt",
            "create_new",
            "connection",
            "new-file",
            1,
            "900150983cd24fb0d6963f7d28e17f72",
        )
        .unwrap();

        db.fail_attempt_before_effect("job", "attempt").unwrap();

        assert_eq!(db.state("job").unwrap(), Some(JobState::Failed));
        assert!(db.unresolved_attempts().unwrap().is_empty());
        let status: String = db
            .db
            .query_row(
                "SELECT status FROM job_attempts WHERE attempt_id='attempt'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(status, "failed_before_effect");
        assert!(matches!(
            db.resolve_attempt(
                "job",
                "attempt",
                &crate::verification::test_bound_receipt(
                    "attempt",
                    "create_new",
                    "connection",
                    "new-file"
                )
            ),
            Err(JobStoreError::InvalidTransition)
        ));
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
        let store = JobStore::initialize(connection, None).unwrap();
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
    fn migrates_v6_records_and_accepts_cancelled_state() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE jobs (
                   id TEXT PRIMARY KEY NOT NULL,
                   state TEXT NOT NULL CHECK (state IN
                     ('queued','running','paused','waiting_for_decision','completed','failed','unknown')),
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
                   status TEXT NOT NULL CHECK (status IN ('active','unknown','resolved')),
                   operation_kind TEXT,
                   target_connection_id TEXT,
                   target_path TEXT,
                   UNIQUE(job_id, attempt_id)
                 );
                 CREATE UNIQUE INDEX one_active_attempt_per_job
                   ON job_attempts(job_id) WHERE status='active';
                 CREATE TABLE attempt_verifications (
                   attempt_id TEXT PRIMARY KEY NOT NULL REFERENCES job_attempts(attempt_id),
                   evidence_level TEXT NOT NULL CHECK (evidence_level IN
                     ('metadata_verified','content_verified'))
                 );
                 CREATE TABLE attempt_expectations (
                   attempt_id TEXT PRIMARY KEY NOT NULL REFERENCES job_attempts(attempt_id),
                   expected_size INTEGER NOT NULL CHECK(expected_size >= 0),
                   expected_md5 TEXT NOT NULL CHECK(length(expected_md5) = 32)
                 );
                 INSERT INTO jobs(id,state) VALUES ('kept','queued');
                 INSERT INTO job_events(job_id,state,kind) VALUES ('kept','queued','enqueued');
                 PRAGMA user_version = 6;",
            )
            .unwrap();
        let mut store = JobStore::initialize(connection, None).unwrap();
        assert_eq!(store.state("kept").unwrap(), Some(JobState::Queued));
        store.cancel("kept").unwrap();
        assert_eq!(store.state("kept").unwrap(), Some(JobState::Cancelled));
        let version: i32 = store
            .db
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, 9);
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
        let mut db = JobStore::initialize(connection, None).unwrap();
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
        assert!(db
            .connection_has_unresolved_attempts("any-connection")
            .unwrap());
    }

    #[test]
    fn only_the_bound_connection_is_blocked_until_its_attempt_is_resolved() {
        let mut db = JobStore::in_memory().unwrap();
        db.enqueue("job", None).unwrap();
        db.advance(
            "job",
            JobState::Queued,
            JobState::Running,
            Confirmation::Unknown,
        )
        .unwrap();
        let checksum = "abcdef0123456789abcdef0123456789";
        db.prepare_bound_attempt(
            "job",
            "attempt",
            "copy",
            "connection-a",
            "/object",
            7,
            checksum,
        )
        .unwrap();
        assert!(db
            .connection_has_unresolved_attempts("connection-a")
            .unwrap());
        assert!(!db
            .connection_has_unresolved_attempts("connection-b")
            .unwrap());
        let receipt =
            crate::verification::test_bound_receipt("attempt", "copy", "connection-a", "/object");
        db.resolve_attempt("job", "attempt", &receipt).unwrap();
        assert!(!db
            .connection_has_unresolved_attempts("connection-a")
            .unwrap());
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
        let store = JobStore::initialize(connection, None).unwrap();
        assert!(store.expected_metadata("job", "old").unwrap().is_none());
        let attempts = store.unresolved_attempts().unwrap();
        assert_eq!(attempts.len(), 1);
        assert_eq!(attempts[0].status, "unknown");
        assert!(!attempts[0].is_legacy_unbound);
    }

    #[test]
    fn unregistered_yandex_connection_never_resolves_credentials() {
        let mut store = JobStore::in_memory().unwrap();
        let registry = crate::registry::Registry::in_memory().unwrap();
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
            .prepare_bound_attempt(
                "job",
                "attempt",
                "copy",
                "missing-connection",
                "disk:/file",
                7,
                "abcdef0123456789abcdef0123456789",
            )
            .unwrap();
        let client = crate::yandex::YandexMetadataClient::new().unwrap();
        assert!(matches!(
            store.observe_registered_yandex_attempt(
                "job",
                "attempt",
                &registry,
                &client,
                |_| panic!("credential resolver must not be called"),
            ),
            Err(YandexObservationError::ConnectionNotAuthorized)
        ));
        assert_eq!(store.state("job").unwrap(), Some(JobState::Running));
    }

    #[test]
    fn read_only_yandex_inspection_cannot_resolve_job() {
        let mut db = JobStore::in_memory().unwrap();
        db.enqueue("job", None).unwrap();
        db.advance(
            "job",
            JobState::Queued,
            JobState::Running,
            Confirmation::Unknown,
        )
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
    fn registered_yandex_reconciliation_checks_identity_and_never_resolves_attempt() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            for (index, body) in [
                r#"{"user":{"uid":"account-42"},"total_space":10,"used_space":1}"#,
                r#"{"name":"file","path":"disk:/file","type":"file","size":7,"md5":"abcdef0123456789abcdef0123456789"}"#,
            ]
            .into_iter()
            .enumerate()
            {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(3)))
                    .unwrap();
                let mut request = [0u8; 8192];
                let bytes = stream.read(&mut request).unwrap();
                let request = String::from_utf8_lossy(&request[..bytes]);
                if index == 0 {
                    assert!(request.starts_with("GET /v1/disk/ HTTP/1.1"));
                } else {
                    assert!(request.starts_with("GET /v1/disk/resources?"));
                    assert!(request.contains("path=disk%3A%2Ffile"));
                }
                assert!(request.to_ascii_lowercase().contains("authorization: oauth test-token"));
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(), body
                );
                stream.write_all(response.as_bytes()).unwrap();
            }
        });

        let registry = crate::registry::Registry::in_memory().unwrap();
        let connection = registry.add("yandex_disk", "Test disk").unwrap();
        registry
            .set_provider_account_id(&connection, Some("account-42"))
            .unwrap();
        registry
            .set_credential_ref(&connection, Some("keychain:connection"))
            .unwrap();
        registry
            .mark_checked(&connection, "connected", 1, None)
            .unwrap();
        let mut store = JobStore::in_memory().unwrap();
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
            .prepare_bound_attempt(
                "job",
                "attempt",
                "copy",
                &connection,
                "disk:/file",
                7,
                "abcdef0123456789abcdef0123456789",
            )
            .unwrap();
        let client = crate::yandex::YandexMetadataClient::with_test_api_base(format!(
            "http://{address}/v1/disk"
        ));

        assert_eq!(
            store
                .observe_registered_yandex_attempt(
                    "job",
                    "attempt",
                    &registry,
                    &client,
                    |reference| {
                        assert_eq!(reference, "keychain:connection");
                        Some("test-token".into())
                    }
                )
                .unwrap(),
            crate::yandex::MetadataMatch::MatchesExpectedMetadata
        );
        server.join().unwrap();
        assert_eq!(store.state("job").unwrap(), Some(JobState::Running));
        assert_eq!(store.unresolved_attempts().unwrap().len(), 1);
    }

    #[test]
    fn registered_yandex_reconciliation_rejects_account_mismatch_before_resource_lookup() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 8192];
            let bytes = stream.read(&mut request).unwrap();
            let request = String::from_utf8_lossy(&request[..bytes]);
            assert!(request.starts_with("GET /v1/disk/ HTTP/1.1"));
            let body = r#"{"user":{"uid":"another-account"},"total_space":10,"used_space":1}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(), body
            );
            stream.write_all(response.as_bytes()).unwrap();
        });

        let registry = crate::registry::Registry::in_memory().unwrap();
        let connection = registry.add("yandex_disk", "Test disk").unwrap();
        registry
            .set_provider_account_id(&connection, Some("account-42"))
            .unwrap();
        registry
            .set_credential_ref(&connection, Some("keychain:connection"))
            .unwrap();
        registry
            .mark_checked(&connection, "connected", 1, None)
            .unwrap();
        let mut store = JobStore::in_memory().unwrap();
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
            .prepare_bound_attempt(
                "job",
                "attempt",
                "copy",
                &connection,
                "disk:/file",
                7,
                "abcdef0123456789abcdef0123456789",
            )
            .unwrap();
        let client = crate::yandex::YandexMetadataClient::with_test_api_base(format!(
            "http://{address}/v1/disk"
        ));

        assert!(matches!(
            store.observe_registered_yandex_attempt("job", "attempt", &registry, &client, |_| {
                Some("test-token".into())
            }),
            Err(YandexObservationError::ConnectionNotAuthorized)
        ));
        server.join().unwrap();
        assert_eq!(store.state("job").unwrap(), Some(JobState::Running));
        assert_eq!(store.unresolved_attempts().unwrap().len(), 1);
    }

    #[test]
    fn atomic_prepare_persists_expectation_and_rejects_incomplete_input() {
        let mut db = JobStore::in_memory().unwrap();
        db.enqueue("job", None).unwrap();
        db.advance(
            "job",
            JobState::Queued,
            JobState::Running,
            Confirmation::Unknown,
        )
        .unwrap();
        let checksum = "abcdef0123456789abcdef0123456789";
        assert!(matches!(
            db.prepare_bound_attempt("job", "a", "copy", "dest", "/file", 7, "invalid"),
            Err(JobStoreError::InvalidId)
        ));
        assert!(db.unresolved_attempts().unwrap().is_empty());
        db.prepare_bound_attempt("job", "a", "copy", "dest", "/file", 7, checksum)
            .unwrap();
        assert_eq!(db.unresolved_attempts().unwrap().len(), 1);
        assert_eq!(
            db.expected_metadata("job", "a")
                .unwrap()
                .unwrap()
                .expected_size,
            7
        );
        assert!(matches!(
            db.prepare_bound_attempt("job", "b", "copy", "dest", "/file", 7, checksum),
            Err(JobStoreError::InvalidTransition)
        ));
        assert!(db.expected_metadata("job", "b").unwrap().is_none());
    }

    #[test]
    fn relative_alias_cannot_bypass_exclusive_owner_lock() {
        use std::time::{SystemTime, UNIX_EPOCH};
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory =
            std::env::temp_dir().join(format!("qadren-alias-{}-{unique}", std::process::id()));
        std::fs::create_dir(&directory).unwrap();
        let database = directory.join("jobs.sqlite");
        let alias = directory.join(".").join("jobs.sqlite");
        let first = JobStore::open(&database).unwrap();
        assert!(matches!(
            JobStore::open(&alias),
            Err(JobStoreError::AlreadyOpen)
        ));
        drop(first);
        std::fs::remove_file(&database).unwrap();
        std::fs::remove_file(directory.join("jobs.sqlite.owner.sqlite")).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn removing_owner_lock_fails_closed_before_next_mutation() {
        use std::time::{SystemTime, UNIX_EPOCH};
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "qadren-owner-removed-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir(&directory).unwrap();
        let database = directory.join("jobs.sqlite");
        let mut store = JobStore::open(&database).unwrap();
        let lock = directory.join("jobs.sqlite.owner.sqlite");
        std::fs::remove_file(&lock).unwrap();
        assert!(matches!(
            store.enqueue("must-not-appear", None),
            Err(JobStoreError::StorageIdentityChanged)
        ));
        drop(store);
        std::fs::remove_file(database).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn dangling_database_symlink_is_rejected() {
        use std::os::unix::fs::symlink;
        use std::time::{SystemTime, UNIX_EPOCH};
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory =
            std::env::temp_dir().join(format!("qadren-dangling-{}-{unique}", std::process::id()));
        std::fs::create_dir(&directory).unwrap();
        let database = directory.join("missing.sqlite");
        let alias = directory.join("alias.sqlite");
        symlink(&database, &alias).unwrap();
        assert!(matches!(
            JobStore::open(&alias),
            Err(JobStoreError::UnsafeDatabaseLink)
        ));
        assert!(!database.exists());
        std::fs::remove_file(alias).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn hardlinked_database_is_rejected_before_recovery() {
        use std::time::{SystemTime, UNIX_EPOCH};
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory =
            std::env::temp_dir().join(format!("qadren-hardlink-{}-{unique}", std::process::id()));
        std::fs::create_dir(&directory).unwrap();
        let database = directory.join("jobs.sqlite");
        let alias = directory.join("alias.sqlite");
        let first = JobStore::open(&database).unwrap();
        std::fs::hard_link(&database, &alias).unwrap();
        assert!(matches!(
            JobStore::open(&alias),
            Err(JobStoreError::UnsafeDatabaseLink)
        ));
        drop(first);
        std::fs::remove_file(&alias).unwrap();
        std::fs::remove_file(&database).unwrap();
        std::fs::remove_file(directory.join("jobs.sqlite.owner.sqlite")).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn concurrent_open_does_not_mark_live_running_job_unknown() {
        use std::time::{SystemTime, UNIX_EPOCH};
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "qadren-owner-{}-{unique}.sqlite",
            std::process::id()
        ));
        let mut first = JobStore::open(&path).unwrap();
        first.enqueue("live", None).unwrap();
        first
            .advance(
                "live",
                JobState::Queued,
                JobState::Running,
                Confirmation::Unknown,
            )
            .unwrap();
        assert!(matches!(
            JobStore::open(&path),
            Err(JobStoreError::AlreadyOpen)
        ));
        assert_eq!(first.state("live").unwrap(), Some(JobState::Running));
        drop(first);
        let recovered = JobStore::open(&path).unwrap();
        assert_eq!(recovered.state("live").unwrap(), Some(JobState::Unknown));
        drop(recovered);
        std::fs::remove_file(&path).unwrap();
        let mut lock_path = path.as_os_str().to_os_string();
        lock_path.push(".owner.sqlite");
        std::fs::remove_file(std::path::Path::new(&lock_path)).unwrap();
    }

    #[test]
    fn newer_schema_is_not_downgraded() {
        let db = Connection::open_in_memory().unwrap();
        db.pragma_update(None, "user_version", 99).unwrap();
        assert!(matches!(
            JobStore::initialize(db, None),
            Err(JobStoreError::UnsupportedSchema(99))
        ));
    }
}
