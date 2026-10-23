//! Persistent metadata-only storage connection registry.
//! Never stores credentials, tokens, file content or remote delete requests.
use crate::automation::{AutomaticTriggerResult, ScenarioAutomationState};
use crate::confirmation::{Confirmation, EvidenceLevel};
use crate::model::{
    ConfirmedFileVersion, FileSyncState, ScenarioDraft, ScenarioFilters, ScenarioPolicies,
    ScenarioReference, StorageEndpoint, SyncDirection,
};
use crate::scenario_planner::{self, ObservedFile, PlanItem};
use crate::scheduler::{OccurrenceKey, WeeklySchedule};
use chrono::{DateTime, Utc, Weekday};
use chrono_tz::Tz;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::path::Path;
use std::str::FromStr;

#[derive(Debug)]
pub enum RegistryError {
    Database(rusqlite::Error),
    InvalidName,
    InvalidAdapter,
    UnsupportedSchema(i32),
    IntegrityFailed,
    NotFound,
    InvalidSchedule,
    InvalidSyncState,
    InsufficientEvidence,
    PlanningFailed,
    ConnectionReferencedByScenario,
    /// One Scenario root is the same as, or inside, the other root.
    ScenarioRootsOverlap,
    /// A request uses a Scenario revision that is no longer current.
    StaleScenarioRevision,
}
impl From<rusqlite::Error> for RegistryError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Database(error)
    }
}
pub type Result<T> = std::result::Result<T, RegistryError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageConnection {
    pub id: String,
    pub adapter: String,
    pub provider_account_id: Option<String>,
    pub name: String,
    pub enabled: bool,
    pub status: String,
    /// Keychain item reference only, never a secret.
    pub credential_ref: Option<String>,
    pub last_checked_at: Option<i64>,
    pub diagnosis: Option<String>,
}

/// Persisted Scenario metadata. The row stores configuration only: never file
/// content, credentials, or a transfer plan. Confirmed FileSyncState records
/// live in a separate revision-bound Registry table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scenario {
    pub id: String,
    pub name: String,
    pub left_connection_id: String,
    pub left_root: String,
    pub right_connection_id: String,
    pub right_root: String,
    pub direction: String,
    pub enabled: bool,
    pub revision: u64,
}

/// Replacement configuration for a persisted Scenario. It intentionally omits
/// `enabled` and `revision`: configuration edits must not enable automation,
/// and the Registry owns monotonically increasing revisions.
pub struct ScenarioUpdate<'a> {
    pub name: &'a str,
    pub left_connection_id: &'a str,
    pub left_root: &'a str,
    pub right_connection_id: &'a str,
    pub right_root: &'a str,
    pub direction: &'a str,
}

/// Durable subset of automatic-trigger state. Active Job ownership stays in
/// JobStore and is supplied by the caller on each evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScenarioAutomationSnapshot {
    pub locally_paused: bool,
    pub pending_automatic_run: bool,
}

/// One durable automatic schedule for a Scenario. It is configuration only;
/// no evaluation or Job submission happens while storing this record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScenarioSchedule {
    pub id: String,
    pub scenario_id: String,
    pub timezone: String,
    /// Monday is 0 and Sunday is 6, matching `Weekday::num_days_from_monday`.
    pub weekday: u8,
    pub hour: u8,
    pub minute: u8,
    pub enabled: bool,
}

/// A read-only automatic-run candidate derived from persisted configuration.
/// It is not a Job, a durable claim, or authorization to begin an effect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DueScheduleOccurrence {
    pub scenario_id: String,
    pub schedule_id: String,
    pub occurrence_key: OccurrenceKey,
}

/// A durable request to create one automatic Job. It is intentionally kept in
/// the Registry database: a schedule slot is never consumed unless its
/// submission request was committed in the same transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduleOutboxEntry {
    pub id: i64,
    pub job_id: String,
    pub scenario_id: String,
    pub scenario_revision: u64,
}

/// A confirmed manual batch. It stores only immutable Scenario identity and
/// selected relative paths. Each child remains one independent Job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScenarioCreateBatchReceipt {
    pub id: String,
    pub scenario: ScenarioReference,
    pub cancellation_requested: bool,
    pub children: Vec<ScenarioBatchChild>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ScenarioBatchChild {
    pub relative_path: String,
    pub source_is_left: bool,
    pub job_id: String,
    pub state: String,
}

/// A user-owned conflict task. It is Registry metadata, not a JobStore Job;
/// pending conflict work never blocks ordinary Scenario transfers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScenarioConflictTask {
    pub id: String,
    pub scenario: ScenarioReference,
    pub relative_path: String,
    pub kind: String,
    /// Metadata observed by the authorized dry-run. These values are not
    /// content and must be revalidated before an effect is attempted.
    pub left: Option<ConfirmedFileVersion>,
    pub right: Option<ConfirmedFileVersion>,
    pub source_is_left: Option<bool>,
    pub decision: Option<String>,
    pub state: String,
}

pub struct Registry {
    db: Connection,
}

impl Registry {
    pub fn open(path: &Path) -> Result<Self> {
        let db = Connection::open(path)?;
        Self::initialize(db)
    }

    pub fn in_memory() -> Result<Self> {
        Self::initialize(Connection::open_in_memory()?)
    }

    fn initialize(db: Connection) -> Result<Self> {
        db.pragma_update(None, "foreign_keys", "ON")?;
        let integrity: String = db.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
        if integrity != "ok" {
            return Err(RegistryError::IntegrityFailed);
        }

        let version: i32 = db.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version > 16 {
            return Err(RegistryError::UnsupportedSchema(version));
        }
        if version == 0 {
            db.execute_batch("BEGIN IMMEDIATE;
                CREATE TABLE IF NOT EXISTS storage_connections (
                    id TEXT PRIMARY KEY NOT NULL,
                    adapter TEXT NOT NULL,
                    name TEXT NOT NULL,
                    enabled INTEGER NOT NULL DEFAULT 1 CHECK(enabled IN (0,1)),
                    status TEXT NOT NULL DEFAULT 'disconnected'
                        CHECK(status IN ('disconnected', 'connected', 'requires_auth', 'offline', 'error')),
                    credential_ref TEXT,
                    last_checked_at INTEGER,
                    diagnosis TEXT
                );
                PRAGMA user_version = 1;
                COMMIT;")?;
        }
        if version <= 1 {
            db.execute_batch(
                "BEGIN IMMEDIATE;
                ALTER TABLE storage_connections ADD COLUMN provider_account_id TEXT;
                PRAGMA user_version = 2;
                COMMIT;",
            )?;
        }
        if version <= 2 {
            db.execute_batch(
                "BEGIN IMMEDIATE;
                CREATE TABLE scenarios (
                    id TEXT PRIMARY KEY NOT NULL,
                    name TEXT NOT NULL,
                    left_connection_id TEXT NOT NULL REFERENCES storage_connections(id) ON DELETE RESTRICT,
                    left_root TEXT NOT NULL,
                    right_connection_id TEXT NOT NULL REFERENCES storage_connections(id) ON DELETE RESTRICT,
                    right_root TEXT NOT NULL,
                    direction TEXT NOT NULL CHECK(direction IN ('left_to_right','right_to_left','bidirectional')),
                    enabled INTEGER NOT NULL DEFAULT 1 CHECK(enabled IN (0,1)),
                    revision INTEGER NOT NULL DEFAULT 1 CHECK(revision > 0),
                    CHECK(NOT (left_connection_id = right_connection_id AND left_root = right_root))
                );
                PRAGMA user_version = 3;
                COMMIT;",
            )?;
        }
        if version <= 3 {
            db.execute_batch(
                "BEGIN IMMEDIATE;
                CREATE TABLE scenario_revisions (
                    scenario_id TEXT NOT NULL REFERENCES scenarios(id) ON DELETE RESTRICT,
                    revision INTEGER NOT NULL CHECK(revision > 0),
                    name TEXT NOT NULL,
                    left_connection_id TEXT NOT NULL REFERENCES storage_connections(id) ON DELETE RESTRICT,
                    left_root TEXT NOT NULL,
                    right_connection_id TEXT NOT NULL REFERENCES storage_connections(id) ON DELETE RESTRICT,
                    right_root TEXT NOT NULL,
                    direction TEXT NOT NULL CHECK(direction IN ('left_to_right','right_to_left','bidirectional')),
                    PRIMARY KEY(scenario_id, revision)
                );
                INSERT INTO scenario_revisions(
                    scenario_id,revision,name,left_connection_id,left_root,
                    right_connection_id,right_root,direction
                )
                SELECT id,revision,name,left_connection_id,left_root,
                       right_connection_id,right_root,direction
                FROM scenarios;
                PRAGMA user_version = 4;
                COMMIT;",
            )?;
        }
        if version <= 4 {
            db.execute_batch(
                "BEGIN IMMEDIATE;
                CREATE TABLE scenario_automation (
                    scenario_id TEXT PRIMARY KEY NOT NULL REFERENCES scenarios(id) ON DELETE RESTRICT,
                    locally_paused INTEGER NOT NULL DEFAULT 0 CHECK(locally_paused IN (0,1)),
                    pending_automatic_run INTEGER NOT NULL DEFAULT 0 CHECK(pending_automatic_run IN (0,1))
                );
                PRAGMA user_version = 5;
                COMMIT;",
            )?;
        }
        if version <= 5 {
            db.execute_batch(
                "BEGIN IMMEDIATE;
                CREATE TABLE scenario_schedules (
                    id TEXT PRIMARY KEY NOT NULL,
                    scenario_id TEXT NOT NULL REFERENCES scenarios(id) ON DELETE RESTRICT,
                    timezone TEXT NOT NULL,
                    weekday INTEGER NOT NULL CHECK(weekday BETWEEN 0 AND 6),
                    hour INTEGER NOT NULL CHECK(hour BETWEEN 0 AND 23),
                    minute INTEGER NOT NULL CHECK(minute BETWEEN 0 AND 59),
                    enabled INTEGER NOT NULL DEFAULT 1 CHECK(enabled IN (0,1))
                );
                PRAGMA user_version = 6;
                COMMIT;",
            )?;
        }
        if version <= 6 {
            db.execute_batch(
                "BEGIN IMMEDIATE;
                CREATE TABLE schedule_occurrences (
                    schedule_id TEXT NOT NULL REFERENCES scenario_schedules(id) ON DELETE RESTRICT,
                    occurrence_key TEXT NOT NULL,
                    PRIMARY KEY(schedule_id, occurrence_key)
                );
                PRAGMA user_version = 7;
                COMMIT;",
            )?;
        }
        if version <= 7 {
            db.execute_batch(
                "BEGIN IMMEDIATE;
                CREATE TABLE file_sync_states (
                    scenario_id TEXT NOT NULL,
                    scenario_revision INTEGER NOT NULL CHECK(scenario_revision > 0),
                    relative_path TEXT NOT NULL,
                    left_object_revision TEXT,
                    left_size INTEGER,
                    left_checksum_md5 TEXT,
                    right_object_revision TEXT,
                    right_size INTEGER,
                    right_checksum_md5 TEXT,
                    PRIMARY KEY(scenario_id,scenario_revision,relative_path),
                    FOREIGN KEY(scenario_id,scenario_revision)
                      REFERENCES scenario_revisions(scenario_id,revision) ON DELETE RESTRICT
                );
                PRAGMA user_version = 8;
                COMMIT;",
            )?;
        }
        if version <= 8 {
            // The Registry and JobStore are separate SQLite databases. This
            // outbox makes claiming a schedule slot and preserving the future
            // Job submission one transaction; dispatch is then idempotent.
            db.execute_batch(
                "BEGIN IMMEDIATE;
                 CREATE TABLE schedule_job_outbox (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    schedule_id TEXT NOT NULL REFERENCES scenario_schedules(id) ON DELETE RESTRICT,
                    occurrence_key TEXT NOT NULL,
                    job_id TEXT NOT NULL UNIQUE,
                    scenario_id TEXT NOT NULL REFERENCES scenarios(id) ON DELETE RESTRICT,
                    scenario_revision INTEGER NOT NULL CHECK(scenario_revision > 0),
                    dispatched INTEGER NOT NULL DEFAULT 0 CHECK(dispatched IN (0,1)),
                    UNIQUE(schedule_id,occurrence_key)
                 );
                 PRAGMA user_version = 9;
                 COMMIT;",
            )?;
        }
        if version <= 9 {
            // Conflict work has an independent durable queue. It stores no
            // content and has no foreign key to JobStore, so an unresolved
            // user decision cannot block normal transfer execution.
            db.execute_batch(
                "BEGIN IMMEDIATE;
                 CREATE TABLE scenario_conflict_tasks (
                    id TEXT PRIMARY KEY NOT NULL,
                    scenario_id TEXT NOT NULL REFERENCES scenarios(id) ON DELETE RESTRICT,
                    scenario_revision INTEGER NOT NULL CHECK(scenario_revision > 0),
                    relative_path TEXT NOT NULL,
                    kind TEXT NOT NULL CHECK(kind IN ('content','deletion')),
                    source_is_left INTEGER CHECK(source_is_left IN (0,1)),
                    decision TEXT CHECK(decision IN (
                       'replace_from_source','replace_if_newer','skip_transfer',
                       'delete_as_source','keep_file'
                    )),
                    state TEXT NOT NULL CHECK(state IN ('pending','selected','completed','stale')),
                    UNIQUE(scenario_id,scenario_revision,relative_path,kind)
                 );
                 PRAGMA user_version = 10;
                 COMMIT;",
            )?;
        }
        if version <= 10 {
            db.execute_batch(
                "BEGIN IMMEDIATE;
                 ALTER TABLE scenario_conflict_tasks ADD COLUMN left_object_revision TEXT;
                 ALTER TABLE scenario_conflict_tasks ADD COLUMN left_size INTEGER;
                 ALTER TABLE scenario_conflict_tasks ADD COLUMN left_checksum_md5 TEXT;
                 ALTER TABLE scenario_conflict_tasks ADD COLUMN right_object_revision TEXT;
                 ALTER TABLE scenario_conflict_tasks ADD COLUMN right_size INTEGER;
                 ALTER TABLE scenario_conflict_tasks ADD COLUMN right_checksum_md5 TEXT;
                 PRAGMA user_version = 11;
                 COMMIT;",
            )?;
        }
        if version <= 11 {
            // This records only the stable identity of a child effect that
            // has already been selected from a live schedule scan. It is not
            // a durable multi-file execution plan.
            db.execute_batch(
                "BEGIN IMMEDIATE;
                 CREATE TABLE schedule_child_jobs (
                    schedule_job_id TEXT NOT NULL REFERENCES schedule_job_outbox(job_id) ON DELETE RESTRICT,
                    relative_path TEXT NOT NULL,
                    job_id TEXT NOT NULL UNIQUE,
                    PRIMARY KEY(schedule_job_id,relative_path)
                 );
                 PRAGMA user_version = 12;
                 COMMIT;",
            )?;
        }
        if version <= 12 {
            db.execute_batch(
                "BEGIN IMMEDIATE;
                 CREATE TABLE scenario_create_batches (
                    id TEXT PRIMARY KEY NOT NULL,
                    scenario_id TEXT NOT NULL,
                    scenario_revision INTEGER NOT NULL CHECK(scenario_revision > 0),
                    FOREIGN KEY(scenario_id,scenario_revision)
                      REFERENCES scenario_revisions(scenario_id,revision) ON DELETE RESTRICT
                 );
                 CREATE TABLE scenario_create_batch_children (
                    batch_id TEXT NOT NULL REFERENCES scenario_create_batches(id) ON DELETE RESTRICT,
                    relative_path TEXT NOT NULL,
                    source_is_left INTEGER NOT NULL CHECK(source_is_left IN (0,1)),
                    job_id TEXT NOT NULL UNIQUE,
                    state TEXT NOT NULL DEFAULT 'pending'
                      CHECK(state IN ('pending','completed','failed','cancelled','unknown')),
                    PRIMARY KEY(batch_id,relative_path)
                 );
                 PRAGMA user_version = 13;
                 COMMIT;",
            )?;
        }
        if version <= 13 {
            db.execute_batch(
                "BEGIN IMMEDIATE;
                 ALTER TABLE scenario_create_batches
                   ADD COLUMN cancellation_requested INTEGER NOT NULL DEFAULT 0
                   CHECK(cancellation_requested IN (0,1));
                 PRAGMA user_version = 14;
                 COMMIT;",
            )?;
        }
        if version <= 14 {
            db.execute_batch(
                "BEGIN IMMEDIATE;
                 CREATE TABLE schedule_conflict_links (
                    schedule_job_id TEXT NOT NULL,
                    conflict_id TEXT NOT NULL REFERENCES scenario_conflict_tasks(id) ON DELETE RESTRICT,
                    PRIMARY KEY(schedule_job_id, conflict_id)
                 );
                 PRAGMA user_version = 15;
                 COMMIT;",
            )?;
        }
        if version <= 15 {
            // The last UTC instant the schedule evaluator observed. It lets a
            // restarted or woken Worker merge missed slots into one catch-up.
            db.execute_batch(
                "BEGIN IMMEDIATE;
                 CREATE TABLE schedule_observation (
                    id INTEGER PRIMARY KEY NOT NULL CHECK(id = 1),
                    observed_through TEXT NOT NULL
                 );
                 PRAGMA user_version = 16;
                 COMMIT;",
            )?;
        }
        Ok(Self { db })
    }

    pub fn add(&self, adapter: &str, name: &str) -> Result<String> {
        if !valid_adapter(adapter) {
            return Err(RegistryError::InvalidAdapter);
        }
        if !valid_name(name) {
            return Err(RegistryError::InvalidName);
        }
        let id: String = self
            .db
            .query_row("SELECT lower(hex(randomblob(16)))", [], |row| row.get(0))?;
        self.db.execute(
            "INSERT INTO storage_connections(id,adapter,name) VALUES (?1,?2,?3)",
            params![id, adapter, name.trim()],
        )?;
        Ok(id)
    }

    pub fn list(&self) -> Result<Vec<StorageConnection>> {
        let mut stmt = self.db.prepare(
            "SELECT id,adapter,name,enabled,status,credential_ref,last_checked_at,diagnosis,provider_account_id
             FROM storage_connections ORDER BY name COLLATE NOCASE, id",
        )?;
        let rows = stmt.query_map([], Self::decode)?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }

    pub fn get(&self, id: &str) -> Result<Option<StorageConnection>> {
        Ok(self
            .db
            .query_row(
                "SELECT id,adapter,name,enabled,status,credential_ref,last_checked_at,diagnosis,provider_account_id
             FROM storage_connections WHERE id=?1",
                [id],
                Self::decode,
            )
            .optional()?)
    }

    fn decode(row: &rusqlite::Row<'_>) -> rusqlite::Result<StorageConnection> {
        Ok(StorageConnection {
            id: row.get(0)?,
            adapter: row.get(1)?,
            name: row.get(2)?,
            enabled: row.get(3)?,
            status: row.get(4)?,
            credential_ref: row.get(5)?,
            last_checked_at: row.get(6)?,
            diagnosis: row.get(7)?,
            provider_account_id: row.get(8)?,
        })
    }

    /// Provider account ID may repeat; connections are never deduplicated by it.
    pub fn set_provider_account_id(&self, id: &str, account_id: Option<&str>) -> Result<()> {
        self.changed(self.db.execute(
            "UPDATE storage_connections SET provider_account_id=?1 WHERE id=?2",
            params![account_id, id],
        )?)
    }

    pub fn rename(&self, id: &str, name: &str) -> Result<()> {
        if !valid_name(name) {
            return Err(RegistryError::InvalidName);
        }
        self.changed(self.db.execute(
            "UPDATE storage_connections SET name=?1 WHERE id=?2",
            params![name.trim(), id],
        )?)
    }

    pub fn set_enabled(&self, id: &str, enabled: bool) -> Result<()> {
        self.changed(self.db.execute(
            "UPDATE storage_connections SET enabled=?1 WHERE id=?2",
            params![enabled, id],
        )?)
    }

    /// Finalize a local credential disconnect in one Registry transaction.
    /// The platform Keychain cleanup must already have succeeded; this method
    /// never contacts a provider or deletes any provider object.
    pub fn finalize_disconnect(&self, id: &str) -> Result<()> {
        self.changed(self.db.execute(
            "UPDATE storage_connections
             SET enabled=1,status='disconnected',credential_ref=NULL,
                 last_checked_at=NULL,diagnosis=NULL
             WHERE id=?1",
            [id],
        )?)
    }

    /// Deletes *only* the local registry record. Never modifies remote cloud data.
    /// Credential revocation/removal is the platform credential store's responsibility.
    pub fn remove(&self, id: &str) -> Result<()> {
        let referenced: bool = self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM scenarios WHERE left_connection_id=?1 OR right_connection_id=?1)",
            [id],
            |row| row.get(0),
        )?;
        if referenced {
            return Err(RegistryError::ConnectionReferencedByScenario);
        }
        self.changed(
            self.db
                .execute("DELETE FROM storage_connections WHERE id=?1", [id])?,
        )
    }

    pub fn mark_checked(
        &self,
        id: &str,
        status: &str,
        at: i64,
        diagnosis: Option<&str>,
    ) -> Result<()> {
        if !matches!(
            status,
            "disconnected" | "connected" | "requires_auth" | "offline" | "error"
        ) {
            return Err(RegistryError::InvalidAdapter);
        }
        self.changed(self.db.execute(
            "UPDATE storage_connections SET status=?1,last_checked_at=?2,diagnosis=?3 WHERE id=?4",
            params![status, at, diagnosis, id],
        )?)
    }

    /// Reference points to an item in secure platform storage, not plaintext credentials.
    pub fn set_credential_ref(&self, id: &str, reference: Option<&str>) -> Result<()> {
        self.changed(self.db.execute(
            "UPDATE storage_connections SET credential_ref=?1 WHERE id=?2",
            params![reference, id],
        )?)
    }

    pub fn list_scenarios(&self) -> Result<Vec<Scenario>> {
        let mut statement = self.db.prepare(
            "SELECT id,name,left_connection_id,left_root,right_connection_id,right_root,direction,enabled,revision
             FROM scenarios ORDER BY name COLLATE NOCASE, id",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(Scenario {
                id: row.get(0)?,
                name: row.get(1)?,
                left_connection_id: row.get(2)?,
                left_root: row.get(3)?,
                right_connection_id: row.get(4)?,
                right_root: row.get(5)?,
                direction: row.get(6)?,
                enabled: row.get(7)?,
                revision: row.get::<_, i64>(8)? as u64,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    /// Both endpoints are Local Filesystem roots on this Mac and one root is
    /// the same as, or inside, the other. Provider paths are never compared.
    fn local_roots_overlap(
        &self,
        left_connection_id: &str,
        left_root: &str,
        right_connection_id: &str,
        right_root: &str,
    ) -> Result<bool> {
        let adapter = |id: &str| -> Result<Option<String>> {
            self.db
                .query_row(
                    "SELECT adapter FROM storage_connections WHERE id=?1",
                    [id],
                    |row| row.get(0),
                )
                .optional()
                .map_err(Into::into)
        };
        let local = Some("local_filesystem".to_owned());
        Ok(adapter(left_connection_id)? == local
            && adapter(right_connection_id)? == local
            && scenario_roots_overlap(left_root, right_root))
    }

    /// Return true when `revision` is the current revision of the Scenario.
    pub fn is_current_scenario_revision(&self, id: &str, revision: u64) -> Result<bool> {
        let current: Option<i64> = self
            .db
            .query_row("SELECT revision FROM scenarios WHERE id=?1", [id], |row| {
                row.get(0)
            })
            .optional()?;
        Ok(current == Some(revision as i64))
    }

    pub fn create_scenario(
        &self,
        name: &str,
        left_connection_id: &str,
        left_root: &str,
        right_connection_id: &str,
        right_root: &str,
        direction: &str,
    ) -> Result<String> {
        if !valid_name(name)
            || left_connection_id.is_empty()
            || right_connection_id.is_empty()
            || left_root.is_empty()
            || right_root.is_empty()
            || !matches!(
                direction,
                "left_to_right" | "right_to_left" | "bidirectional"
            )
        {
            return Err(RegistryError::InvalidName);
        }
        if self.local_roots_overlap(
            left_connection_id,
            left_root,
            right_connection_id,
            right_root,
        )? {
            return Err(RegistryError::ScenarioRootsOverlap);
        }
        let id: String = self
            .db
            .query_row("SELECT lower(hex(randomblob(16)))", [], |row| row.get(0))?;
        let transaction = self.db.unchecked_transaction()?;
        transaction.execute(
            "INSERT INTO scenarios(id,name,left_connection_id,left_root,right_connection_id,right_root,direction) VALUES(?1,?2,?3,?4,?5,?6,?7)",
            params![id, name.trim(), left_connection_id, left_root, right_connection_id, right_root, direction],
        )?;
        transaction.execute(
            "INSERT INTO scenario_revisions(
                scenario_id,revision,name,left_connection_id,left_root,
                right_connection_id,right_root,direction
             ) VALUES(?1,1,?2,?3,?4,?5,?6,?7)",
            params![
                id,
                name.trim(),
                left_connection_id,
                left_root,
                right_connection_id,
                right_root,
                direction
            ],
        )?;
        transaction.commit()?;
        Ok(id)
    }

    pub fn set_scenario_enabled(&self, id: &str, enabled: bool) -> Result<()> {
        let transaction = self.db.unchecked_transaction()?;
        let changed = transaction.execute(
            "UPDATE scenarios SET enabled=?1 WHERE id=?2",
            params![enabled, id],
        )?;
        if changed == 0 {
            return Err(RegistryError::NotFound);
        }
        if !enabled {
            transaction.execute(
                "UPDATE scenario_automation SET pending_automatic_run=0 WHERE scenario_id=?1",
                [id],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn scenario_automation_snapshot(&self, id: &str) -> Result<ScenarioAutomationSnapshot> {
        self.db
            .query_row(
                "SELECT COALESCE(a.locally_paused,0),COALESCE(a.pending_automatic_run,0)
             FROM scenarios s LEFT JOIN scenario_automation a ON a.scenario_id=s.id
             WHERE s.id=?1",
                [id],
                |row| {
                    Ok(ScenarioAutomationSnapshot {
                        locally_paused: row.get(0)?,
                        pending_automatic_run: row.get(1)?,
                    })
                },
            )
            .optional()?
            .ok_or(RegistryError::NotFound)
    }

    pub fn create_scenario_schedule(
        &self,
        scenario_id: &str,
        timezone: &str,
        weekday: u8,
        hour: u8,
        minute: u8,
    ) -> Result<String> {
        validate_schedule(timezone, weekday, hour, minute)?;
        let id: String = self
            .db
            .query_row("SELECT lower(hex(randomblob(16)))", [], |row| row.get(0))?;
        let changed = self.db.execute(
            "INSERT INTO scenario_schedules(id,scenario_id,timezone,weekday,hour,minute)
             SELECT ?1,id,?2,?3,?4,?5 FROM scenarios WHERE id=?6",
            params![id, timezone, weekday, hour, minute, scenario_id],
        )?;
        if changed == 0 {
            return Err(RegistryError::NotFound);
        }
        Ok(id)
    }

    pub fn list_scenario_schedules(&self, scenario_id: &str) -> Result<Vec<ScenarioSchedule>> {
        let mut statement = self.db.prepare(
            "SELECT id,scenario_id,timezone,weekday,hour,minute,enabled
             FROM scenario_schedules WHERE scenario_id=?1 ORDER BY weekday,hour,minute,id",
        )?;
        let schedules = statement
            .query_map([scenario_id], |row| {
                Ok(ScenarioSchedule {
                    id: row.get(0)?,
                    scenario_id: row.get(1)?,
                    timezone: row.get(2)?,
                    weekday: row.get(3)?,
                    hour: row.get(4)?,
                    minute: row.get(5)?,
                    enabled: row.get(6)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into);
        schedules
    }

    /// Evaluate enabled schedules for enabled, locally unpaused Scenarios in
    /// `(after, through]`. The result is deliberately read-only; callers that
    /// need durable submission must pass it to `stage_due_schedule_occurrences`.
    pub fn due_schedule_occurrences(
        &self,
        after: DateTime<Utc>,
        through: DateTime<Utc>,
    ) -> Result<Vec<DueScheduleOccurrence>> {
        if through < after {
            return Err(RegistryError::InvalidSchedule);
        }
        let mut statement = self.db.prepare(
            "SELECT ss.id,ss.scenario_id,ss.timezone,ss.weekday,ss.hour,ss.minute
             FROM scenario_schedules ss
             JOIN scenarios s ON s.id=ss.scenario_id
             LEFT JOIN scenario_automation a ON a.scenario_id=s.id
             WHERE ss.enabled=1 AND s.enabled=1 AND COALESCE(a.locally_paused,0)=0
             ORDER BY ss.scenario_id,ss.id",
        )?;
        let schedules = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, u8>(3)?,
                    row.get::<_, u8>(4)?,
                    row.get::<_, u8>(5)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut due = Vec::new();
        for (schedule_id, scenario_id, timezone, weekday, hour, minute) in schedules {
            let timezone = Tz::from_str(&timezone).map_err(|_| RegistryError::InvalidSchedule)?;
            let weekday = match weekday {
                0 => Weekday::Mon,
                1 => Weekday::Tue,
                2 => Weekday::Wed,
                3 => Weekday::Thu,
                4 => Weekday::Fri,
                5 => Weekday::Sat,
                6 => Weekday::Sun,
                _ => return Err(RegistryError::InvalidSchedule),
            };
            let schedule = WeeklySchedule {
                timezone,
                weekday,
                hour: hour as u32,
                minute: minute as u32,
            };
            for occurrence_key in schedule
                .occurrences_between(after, through)
                .map_err(|_| RegistryError::InvalidSchedule)?
            {
                due.push(DueScheduleOccurrence {
                    scenario_id: scenario_id.clone(),
                    schedule_id: schedule_id.clone(),
                    occurrence_key,
                });
            }
        }
        due.sort_by(|left, right| {
            (&left.scenario_id, &left.schedule_id, &left.occurrence_key).cmp(&(
                &right.scenario_id,
                &right.schedule_id,
                &right.occurrence_key,
            ))
        });
        Ok(due)
    }

    /// Persist every currently due slot with a Job submission request.  This
    /// does not open JobStore or execute a Job; it only closes the crash gap
    /// between a schedule receipt and future submission.
    ///
    /// All slots claimed in one pass for the same Scenario are merged into one
    /// Job for the last claimed slot. The other slots stay claimed, so a later
    /// pass never turns them into extra Jobs.
    pub fn stage_due_schedule_occurrences(
        &self,
        after: DateTime<Utc>,
        through: DateTime<Utc>,
    ) -> Result<usize> {
        let due = self.due_schedule_occurrences(after, through)?;
        self.stage_occurrences(due, None)
    }

    /// Stage due slots since the last durable observation and record
    /// `through` as the new observation in the same transaction. Slots missed
    /// while the Worker was stopped or the Mac was asleep become one catch-up
    /// Job per Scenario. Without an earlier observation, or after the clock
    /// moves backward, only the `overlap` window is evaluated.
    pub fn stage_schedule_catch_up(
        &self,
        through: DateTime<Utc>,
        overlap: chrono::Duration,
    ) -> Result<usize> {
        let window_start = through - overlap;
        let after = match self.schedule_observed_through()? {
            Some(observed) if observed < window_start => observed,
            _ => window_start,
        };
        let due = self.due_schedule_occurrences(after, through)?;
        self.stage_occurrences(due, Some(through))
    }

    /// The last UTC instant recorded by `stage_schedule_catch_up`.
    pub fn schedule_observed_through(&self) -> Result<Option<DateTime<Utc>>> {
        let value: Option<String> = self
            .db
            .query_row(
                "SELECT observed_through FROM schedule_observation WHERE id=1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        value
            .map(|value| {
                DateTime::parse_from_rfc3339(&value)
                    .map(|value| value.with_timezone(&Utc))
                    .map_err(|_| RegistryError::IntegrityFailed)
            })
            .transpose()
    }

    fn stage_occurrences(
        &self,
        due: Vec<DueScheduleOccurrence>,
        observed_through: Option<DateTime<Utc>>,
    ) -> Result<usize> {
        let transaction = self.db.unchecked_transaction()?;
        let mut claimed_by_scenario: Vec<DueScheduleOccurrence> = Vec::new();
        for occurrence in due {
            let claimed = transaction.execute(
                "INSERT OR IGNORE INTO schedule_occurrences(schedule_id,occurrence_key)
                 SELECT ss.id,?2 FROM scenario_schedules ss
                 JOIN scenarios s ON s.id=ss.scenario_id
                 LEFT JOIN scenario_automation a ON a.scenario_id=s.id
                 WHERE ss.id=?1 AND ss.scenario_id=?3 AND ss.enabled=1 AND s.enabled=1
                   AND COALESCE(a.locally_paused,0)=0",
                params![
                    occurrence.schedule_id,
                    occurrence.occurrence_key.0,
                    occurrence.scenario_id
                ],
            )?;
            if claimed == 0 {
                continue;
            }
            // `due` is sorted by Scenario, so the last claimed slot of each
            // Scenario replaces the earlier ones of that pass.
            match claimed_by_scenario.last_mut() {
                Some(last) if last.scenario_id == occurrence.scenario_id => *last = occurrence,
                _ => claimed_by_scenario.push(occurrence),
            }
        }
        let mut staged = 0;
        for occurrence in claimed_by_scenario {
            // The complete bounded, validated tuple is retained rather than
            // hashed: Job identity must not rely on collision resistance.
            let job_id = format!(
                "schedule-{}-{}",
                occurrence.schedule_id, occurrence.occurrence_key.0
            );
            let inserted = transaction.execute(
                "INSERT INTO schedule_job_outbox(
                    schedule_id,occurrence_key,job_id,scenario_id,scenario_revision
                 ) SELECT ss.id,?2,?3,s.id,s.revision
                   FROM scenario_schedules ss JOIN scenarios s ON s.id=ss.scenario_id
                   LEFT JOIN scenario_automation a ON a.scenario_id=s.id
                   WHERE ss.id=?1 AND ss.scenario_id=?4 AND ss.enabled=1 AND s.enabled=1
                     AND COALESCE(a.locally_paused,0)=0",
                params![
                    occurrence.schedule_id,
                    occurrence.occurrence_key.0,
                    job_id,
                    occurrence.scenario_id
                ],
            )?;
            // The same transaction predicates make this unreachable except
            // for corruption; rolling back preserves the slot for retry.
            if inserted != 1 {
                return Err(RegistryError::IntegrityFailed);
            }
            staged += 1;
        }
        if let Some(observed_through) = observed_through {
            transaction.execute(
                "INSERT INTO schedule_observation(id,observed_through) VALUES (1,?1)
                 ON CONFLICT(id) DO UPDATE SET observed_through=excluded.observed_through",
                [observed_through.to_rfc3339_opts(chrono::SecondsFormat::Nanos, true)],
            )?;
        }
        transaction.commit()?;
        Ok(staged)
    }

    pub fn pending_schedule_job_outbox(&self) -> Result<Vec<ScheduleOutboxEntry>> {
        let mut statement = self.db.prepare(
            "SELECT id,job_id,scenario_id,scenario_revision
             FROM schedule_job_outbox WHERE dispatched=0 ORDER BY id",
        )?;
        let entries = statement
            .query_map([], |row| {
                let revision: i64 = row.get(3)?;
                Ok(ScheduleOutboxEntry {
                    id: row.get(0)?,
                    job_id: row.get(1)?,
                    scenario_id: row.get(2)?,
                    scenario_revision: revision as u64,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(RegistryError::from)?;
        Ok(entries)
    }

    /// Mark only an existing pending outbox request dispatched. The caller
    /// must have durably observed/created the exact Job first.
    pub fn mark_schedule_job_dispatched(&self, outbox_id: i64) -> Result<bool> {
        if outbox_id <= 0 {
            return Err(RegistryError::InvalidSchedule);
        }
        let changed = self.db.execute(
            "UPDATE schedule_job_outbox SET dispatched=1 WHERE id=?1 AND dispatched=0",
            [outbox_id],
        )?;
        if changed == 0 {
            let exists: Option<i64> = self
                .db
                .query_row(
                    "SELECT id FROM schedule_job_outbox WHERE id=?1",
                    [outbox_id],
                    |row| row.get(0),
                )
                .optional()?;
            if exists.is_none() {
                return Err(RegistryError::NotFound);
            }
        }
        Ok(changed == 1)
    }

    /// Return the immutable Scenario identity for a Job created from a
    /// schedule receipt. This remains available after dispatch so the Worker
    /// can distinguish schedule coordinator Jobs from other queued Jobs.
    pub fn schedule_job_reference(&self, job_id: &str) -> Result<Option<ScenarioReference>> {
        self.db
            .query_row(
                "SELECT scenario_id,scenario_revision FROM schedule_job_outbox WHERE job_id=?1",
                [job_id],
                |row| {
                    Ok(ScenarioReference {
                        scenario_id: row.get(0)?,
                        revision: row.get::<_, i64>(1)? as u64,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }

    /// Reserve the stable ID for one safe child effect. The row contains no
    /// file content, source metadata, or execution plan. A restart receives
    /// the same ID and can therefore observe its existing Job instead of
    /// replaying the effect under another identity.
    pub fn reserve_schedule_child_job_id(
        &self,
        schedule_job_id: &str,
        relative_path: &str,
    ) -> Result<String> {
        if !valid_sync_path(relative_path) {
            return Err(RegistryError::InvalidSyncState);
        }
        let parent_exists: Option<i64> = self
            .db
            .query_row(
                "SELECT 1 FROM schedule_job_outbox WHERE job_id=?1",
                [schedule_job_id],
                |row| row.get(0),
            )
            .optional()?;
        if parent_exists.is_none() {
            return Err(RegistryError::NotFound);
        }
        if let Some(existing) = self
            .db
            .query_row(
                "SELECT job_id FROM schedule_child_jobs WHERE schedule_job_id=?1 AND relative_path=?2",
                params![schedule_job_id, relative_path],
                |row| row.get(0),
            )
            .optional()?
        {
            return Ok(existing);
        }
        for _ in 0..4 {
            let job_id: String = self.db.query_row(
                "SELECT 'schedule-copy-' || lower(hex(randomblob(16)))",
                [],
                |row| row.get(0),
            )?;
            let inserted = self.db.execute(
                "INSERT OR IGNORE INTO schedule_child_jobs(schedule_job_id,relative_path,job_id)
                 VALUES(?1,?2,?3)",
                params![schedule_job_id, relative_path, job_id],
            )?;
            if inserted == 1 {
                return Ok(job_id);
            }
            if let Some(existing) = self
                .db
                .query_row(
                    "SELECT job_id FROM schedule_child_jobs WHERE schedule_job_id=?1 AND relative_path=?2",
                    params![schedule_job_id, relative_path],
                    |row| row.get(0),
                )
                .optional()?
            {
                return Ok(existing);
            }
        }
        Err(RegistryError::IntegrityFailed)
    }

    /// Return the immutable Scenario identity that owns one scheduled child
    /// effect. The Worker uses this only to find the already-selected Local
    /// destination for read-only UNKNOWN reconciliation.
    pub fn schedule_child_job_scenario(&self, job_id: &str) -> Result<Option<ScenarioReference>> {
        self.db
            .query_row(
                "SELECT o.scenario_id,o.scenario_revision
                 FROM schedule_child_jobs c
                 JOIN schedule_job_outbox o ON o.job_id=c.schedule_job_id
                 WHERE c.job_id=?1",
                [job_id],
                |row| {
                    Ok(ScenarioReference {
                        scenario_id: row.get(0)?,
                        revision: row.get::<_, i64>(1)? as u64,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }

    /// Save one user-confirmed Create batch before the Worker starts a child.
    /// Paths are immutable and get stable Job IDs, so a controller reconnect
    /// can observe work without submitting the same effect again.
    pub fn create_scenario_create_batch(
        &mut self,
        scenario: &ScenarioReference,
        children: &[(String, bool)],
    ) -> Result<ScenarioCreateBatchReceipt> {
        if scenario.scenario_id.is_empty() || scenario.revision == 0 || children.is_empty() {
            return Err(RegistryError::InvalidSyncState);
        }
        let mut paths = children.to_vec();
        paths.sort_by(|left, right| left.0.cmp(&right.0));
        if paths.windows(2).any(|pair| pair[0].0 == pair[1].0)
            || paths.iter().any(|(path, _)| !valid_sync_path(path))
        {
            return Err(RegistryError::InvalidSyncState);
        }
        if !self.is_current_scenario_revision(&scenario.scenario_id, scenario.revision)? {
            return Err(RegistryError::StaleScenarioRevision);
        }
        let transaction = self.db.transaction()?;
        let exists: Option<i64> = transaction
            .query_row(
                "SELECT 1 FROM scenario_revisions WHERE scenario_id=?1 AND revision=?2",
                params![scenario.scenario_id, scenario.revision as i64],
                |row| row.get(0),
            )
            .optional()?;
        if exists.is_none() {
            return Err(RegistryError::NotFound);
        }
        let id: String =
            transaction.query_row("SELECT lower(hex(randomblob(16)))", [], |row| row.get(0))?;
        transaction.execute(
            "INSERT INTO scenario_create_batches(id,scenario_id,scenario_revision) VALUES(?1,?2,?3)",
            params![id, scenario.scenario_id, scenario.revision as i64],
        )?;
        let mut receipt_children = Vec::with_capacity(paths.len());
        for (path, source_is_left) in paths {
            let job_id: String = transaction.query_row(
                "SELECT 'scenario-batch-copy-' || lower(hex(randomblob(16)))",
                [],
                |row| row.get(0),
            )?;
            transaction.execute(
                "INSERT INTO scenario_create_batch_children(batch_id,relative_path,source_is_left,job_id) VALUES(?1,?2,?3,?4)",
                params![id, path, source_is_left, job_id],
            )?;
            receipt_children.push(ScenarioBatchChild {
                relative_path: path,
                source_is_left,
                job_id,
                state: "pending".into(),
            });
        }
        transaction.commit()?;
        Ok(ScenarioCreateBatchReceipt {
            id,
            scenario: scenario.clone(),
            cancellation_requested: false,
            children: receipt_children,
        })
    }

    /// Load a confirmed batch exactly as saved. This does not create Jobs or
    /// authorize an effect.
    pub fn scenario_create_batch(&self, id: &str) -> Result<Option<ScenarioCreateBatchReceipt>> {
        let parent: Option<(ScenarioReference, bool)> = self
            .db
            .query_row(
            "SELECT scenario_id,scenario_revision,cancellation_requested FROM scenario_create_batches WHERE id=?1",
                [id],
                |row| {
                    Ok((ScenarioReference {
                        scenario_id: row.get(0)?,
                        revision: row.get::<_, i64>(1)? as u64,
                    }, row.get::<_, bool>(2)?))
                },
            )
            .optional()?;
        let Some((scenario, cancellation_requested)) = parent else {
            return Ok(None);
        };
        let mut statement = self.db.prepare(
            "SELECT relative_path,source_is_left,job_id,state FROM scenario_create_batch_children WHERE batch_id=?1 ORDER BY relative_path",
        )?;
        let children = statement
            .query_map([id], |row| {
                Ok(ScenarioBatchChild {
                    relative_path: row.get(0)?,
                    source_is_left: row.get(1)?,
                    job_id: row.get(2)?,
                    state: row.get(3)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(Some(ScenarioCreateBatchReceipt {
            id: id.into(),
            scenario,
            cancellation_requested,
            children,
        }))
    }

    /// Return receipt IDs that still have effects the Worker has not started.
    /// Terminal and UNKNOWN child outcomes never become eligible for replay.
    pub fn pending_scenario_create_batch_ids(&self) -> Result<Vec<String>> {
        let mut statement = self.db.prepare(
            "SELECT DISTINCT batch_id FROM scenario_create_batch_children WHERE state='pending' ORDER BY batch_id",
        )?;
        let result = statement
            .query_map([], |row| row.get(0))?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into);
        result
    }

    /// Persist a stop request. The Worker reads it before each next child, so
    /// an already-started child keeps its durable real outcome.
    pub fn request_scenario_create_batch_cancellation(&self, batch_id: &str) -> Result<()> {
        let changed = self.db.execute(
            "UPDATE scenario_create_batches SET cancellation_requested=1
             WHERE id=?1 AND cancellation_requested=0",
            [batch_id],
        )?;
        if changed == 1 {
            Ok(())
        } else {
            Err(RegistryError::NotFound)
        }
    }

    /// Load unfinished Create batches for one exact Scenario revision. This is
    /// read-only. It lets a controller reconnect to durable Worker progress
    /// without submitting a second effect.
    pub fn pending_scenario_create_batches(
        &self,
        scenario: &ScenarioReference,
    ) -> Result<Vec<ScenarioCreateBatchReceipt>> {
        let mut statement = self.db.prepare(
            "SELECT DISTINCT b.id
             FROM scenario_create_batches b
             JOIN scenario_create_batch_children c ON c.batch_id=b.id
             WHERE b.scenario_id=?1 AND b.scenario_revision=?2 AND c.state='pending'
             ORDER BY b.id",
        )?;
        let ids = statement
            .query_map(
                params![scenario.scenario_id, scenario.revision as i64],
                |row| row.get::<_, String>(0),
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        ids.into_iter()
            .map(|id| {
                self.scenario_create_batch(&id)?
                    .ok_or(RegistryError::NotFound)
            })
            .collect()
    }

    /// Persist the authoritative Worker outcome for a selected batch child.
    pub fn set_scenario_create_batch_child_state(
        &self,
        batch_id: &str,
        relative_path: &str,
        state: &str,
    ) -> Result<()> {
        if !matches!(state, "completed" | "failed" | "cancelled" | "unknown") {
            return Err(RegistryError::InvalidSyncState);
        }
        let changed = self.db.execute(
            "UPDATE scenario_create_batch_children
             SET state=?3 WHERE batch_id=?1 AND relative_path=?2 AND state='pending'",
            params![batch_id, relative_path, state],
        )?;
        if changed == 1 {
            Ok(())
        } else {
            Err(RegistryError::NotFound)
        }
    }

    /// Enable or disable one schedule without changing its already claimed
    /// occurrences. Re-enabling must never replay an older local slot.
    pub fn set_scenario_schedule_enabled(
        &self,
        scenario_id: &str,
        schedule_id: &str,
        enabled: bool,
    ) -> Result<()> {
        let changed = self.db.execute(
            "UPDATE scenario_schedules SET enabled=?1 WHERE id=?2 AND scenario_id=?3",
            params![enabled, schedule_id, scenario_id],
        )?;
        if changed == 1 {
            Ok(())
        } else {
            Err(RegistryError::NotFound)
        }
    }

    /// Removing a schedule changes only future automatic configuration. Its
    /// occurrence receipts are removed in the same transaction; no Job is
    /// submitted, cancelled, or otherwise modified here.
    pub fn remove_scenario_schedule(&self, scenario_id: &str, schedule_id: &str) -> Result<()> {
        let transaction = self.db.unchecked_transaction()?;
        let exists: Option<String> = transaction
            .query_row(
                "SELECT id FROM scenario_schedules WHERE id=?1 AND scenario_id=?2",
                params![schedule_id, scenario_id],
                |row| row.get(0),
            )
            .optional()?;
        if exists.is_none() {
            return Err(RegistryError::NotFound);
        }
        transaction.execute(
            "DELETE FROM schedule_occurrences WHERE schedule_id=?1",
            [schedule_id],
        )?;
        transaction.execute(
            "DELETE FROM scenario_schedules WHERE id=?1 AND scenario_id=?2",
            params![schedule_id, scenario_id],
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// Atomically claim one evaluated local schedule slot. A duplicate claim
    /// is intentionally a no-op, preventing repeated DST hours and restart
    /// catch-up from creating duplicate future Jobs.
    pub fn claim_schedule_occurrence(
        &self,
        schedule_id: &str,
        occurrence_key: &str,
    ) -> Result<bool> {
        if occurrence_key.is_empty()
            || occurrence_key.len() > 256
            || occurrence_key.chars().any(char::is_control)
        {
            return Err(RegistryError::InvalidSchedule);
        }
        let changed = self.db.execute(
            "INSERT OR IGNORE INTO schedule_occurrences(schedule_id,occurrence_key) VALUES(?1,?2)",
            params![schedule_id, occurrence_key],
        )?;
        if changed == 0 {
            let exists: Option<String> = self
                .db
                .query_row(
                    "SELECT id FROM scenario_schedules WHERE id=?1",
                    [schedule_id],
                    |row| row.get(0),
                )
                .optional()?;
            if exists.is_none() {
                return Err(RegistryError::NotFound);
            }
        }
        Ok(changed == 1)
    }

    /// Pausing automatic work also removes its queued successor. Manual Job
    /// requests have a separate path and remain admissible.
    pub fn set_scenario_locally_paused(&self, id: &str, paused: bool) -> Result<()> {
        self.changed(self.db.execute(
            "INSERT INTO scenario_automation(scenario_id,locally_paused,pending_automatic_run)
             SELECT id,?1,0 FROM scenarios WHERE id=?2
             ON CONFLICT(scenario_id) DO UPDATE SET
               locally_paused=excluded.locally_paused,
               pending_automatic_run=CASE WHEN excluded.locally_paused=1 THEN 0 ELSE scenario_automation.pending_automatic_run END",
            params![paused, id],
        )?)
    }

    /// Persists coalesced automatic demand without submitting or mutating a
    /// Job. `active_job` is read from the durable Job layer by the caller.
    pub fn request_scenario_automatic_run(
        &self,
        id: &str,
        active_job: bool,
    ) -> Result<AutomaticTriggerResult> {
        let transaction = self.db.unchecked_transaction()?;
        let state = transaction
            .query_row(
                "SELECT s.enabled,COALESCE(a.locally_paused,0),COALESCE(a.pending_automatic_run,0)
             FROM scenarios s LEFT JOIN scenario_automation a ON a.scenario_id=s.id
             WHERE s.id=?1",
                [id],
                |row| {
                    Ok(ScenarioAutomationState {
                        enabled: row.get(0)?,
                        locally_paused: row.get(1)?,
                        active_job,
                        pending_automatic_run: row.get(2)?,
                    })
                },
            )
            .optional()?
            .ok_or(RegistryError::NotFound)?;
        let mut state = state;
        let result = state.request_automatic_run();
        transaction.execute(
            "INSERT INTO scenario_automation(scenario_id,locally_paused,pending_automatic_run)
             VALUES(?1,?2,?3)
             ON CONFLICT(scenario_id) DO UPDATE SET
               locally_paused=excluded.locally_paused,pending_automatic_run=excluded.pending_automatic_run",
            params![id, state.locally_paused, state.pending_automatic_run],
        )?;
        transaction.commit()?;
        Ok(result)
    }

    /// Replaces a Scenario configuration and creates a new immutable revision
    /// for future Jobs. Existing Jobs must continue to use their pinned
    /// revision; this Registry never rewrites their identity.
    pub fn update_scenario(&self, id: &str, update: ScenarioUpdate<'_>) -> Result<()> {
        if !valid_name(update.name)
            || update.left_connection_id.is_empty()
            || update.right_connection_id.is_empty()
            || update.left_root.is_empty()
            || update.right_root.is_empty()
            || !matches!(
                update.direction,
                "left_to_right" | "right_to_left" | "bidirectional"
            )
        {
            return Err(RegistryError::InvalidName);
        }
        if self.local_roots_overlap(
            update.left_connection_id,
            update.left_root,
            update.right_connection_id,
            update.right_root,
        )? {
            return Err(RegistryError::ScenarioRootsOverlap);
        }
        let transaction = self.db.unchecked_transaction()?;
        let changed = transaction.execute(
            "UPDATE scenarios
             SET name=?1,left_connection_id=?2,left_root=?3,
                 right_connection_id=?4,right_root=?5,direction=?6,
                 revision=revision+1
             WHERE id=?7",
            params![
                update.name.trim(),
                update.left_connection_id,
                update.left_root,
                update.right_connection_id,
                update.right_root,
                update.direction,
                id
            ],
        )?;
        if changed == 0 {
            return Err(RegistryError::NotFound);
        }
        transaction.execute(
            "INSERT INTO scenario_revisions(
                scenario_id,revision,name,left_connection_id,left_root,
                right_connection_id,right_root,direction
             )
             SELECT id,revision,name,left_connection_id,left_root,
                    right_connection_id,right_root,direction
             FROM scenarios WHERE id=?1",
            [id],
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// Retrieves the immutable configuration that belongs to a Job's pinned
    /// Scenario revision. It intentionally does not fall back to the current
    /// row, because using a later configuration would change a durable Job.
    pub fn get_scenario_revision(&self, id: &str, revision: u64) -> Result<Option<Scenario>> {
        self.db
            .query_row(
                "SELECT scenario_id,name,left_connection_id,left_root,right_connection_id,right_root,direction,revision
                 FROM scenario_revisions WHERE scenario_id=?1 AND revision=?2",
                params![id, revision as i64],
                |row| {
                    Ok(Scenario {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        left_connection_id: row.get(2)?,
                        left_root: row.get(3)?,
                        right_connection_id: row.get(4)?,
                        right_root: row.get(5)?,
                        direction: row.get(6)?,
                        enabled: false,
                        revision: row.get::<_, i64>(7)? as u64,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }

    /// Persist one last-confirmed state only after an executor has independently
    /// verified the operation. Requested/acknowledged/unknown outcomes never
    /// advance this baseline, and this method performs no scan or adapter I/O.
    pub fn record_confirmed_file_state(
        &self,
        state: &FileSyncState,
        confirmation: Confirmation,
    ) -> Result<()> {
        if !confirmation.meets(EvidenceLevel::MetadataVerified) {
            return Err(RegistryError::InsufficientEvidence);
        }
        if !valid_sync_path(&state.relative_path)
            || state.scenario.scenario_id.is_empty()
            || state.scenario.revision == 0
            || (state.left.is_none() && state.right.is_none())
            || !valid_confirmed_version(state.left.as_ref())
            || !valid_confirmed_version(state.right.as_ref())
        {
            return Err(RegistryError::InvalidSyncState);
        }
        let scenario_exists: Option<String> = self
            .db
            .query_row(
                "SELECT scenario_id FROM scenario_revisions WHERE scenario_id=?1 AND revision=?2",
                params![state.scenario.scenario_id, state.scenario.revision as i64],
                |row| row.get(0),
            )
            .optional()?;
        if scenario_exists.is_none() {
            return Err(RegistryError::NotFound);
        }
        self.db.execute(
            "INSERT INTO file_sync_states(
                scenario_id,scenario_revision,relative_path,
                left_object_revision,left_size,left_checksum_md5,
                right_object_revision,right_size,right_checksum_md5
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)
             ON CONFLICT(scenario_id,scenario_revision,relative_path) DO UPDATE SET
                left_object_revision=excluded.left_object_revision,
                left_size=excluded.left_size,
                left_checksum_md5=excluded.left_checksum_md5,
                right_object_revision=excluded.right_object_revision,
                right_size=excluded.right_size,
                right_checksum_md5=excluded.right_checksum_md5",
            params![
                state.scenario.scenario_id,
                state.scenario.revision as i64,
                state.relative_path,
                state
                    .left
                    .as_ref()
                    .and_then(|v| v.object_revision.as_deref()),
                state.left.as_ref().map(|v| v.size as i64),
                state.left.as_ref().and_then(|v| v.checksum_md5.as_deref()),
                state
                    .right
                    .as_ref()
                    .and_then(|v| v.object_revision.as_deref()),
                state.right.as_ref().map(|v| v.size as i64),
                state.right.as_ref().and_then(|v| v.checksum_md5.as_deref()),
            ],
        )?;
        Ok(())
    }

    /// Read only the verified baseline belonging to the exact Scenario
    /// revision. Later edits never silently change an older Job's comparison.
    pub fn list_confirmed_file_states(
        &self,
        reference: &ScenarioReference,
    ) -> Result<Vec<FileSyncState>> {
        let mut statement = self.db.prepare(
            "SELECT relative_path,
                left_object_revision,left_size,left_checksum_md5,
                right_object_revision,right_size,right_checksum_md5
             FROM file_sync_states
             WHERE scenario_id=?1 AND scenario_revision=?2
             ORDER BY relative_path",
        )?;
        let states = statement
            .query_map(
                params![reference.scenario_id, reference.revision as i64],
                |row| {
                    Ok(FileSyncState {
                        scenario: reference.clone(),
                        relative_path: row.get(0)?,
                        left: decode_confirmed_version(row, 1, 2, 3)?,
                        right: decode_confirmed_version(row, 4, 5, 6)?,
                    })
                },
            )?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into);
        states
    }

    /// Add or reopen one metadata-only conflict task. This has no adapter or
    /// JobStore effect and therefore cannot stop the main transfer flow.
    pub fn queue_conflict_task(
        &self,
        scenario: &ScenarioReference,
        relative_path: &str,
        kind: &str,
        source_is_left: Option<bool>,
        left: Option<&ConfirmedFileVersion>,
        right: Option<&ConfirmedFileVersion>,
    ) -> Result<String> {
        if !valid_sync_path(relative_path)
            || !matches!(kind, "content" | "deletion")
            || scenario.scenario_id.is_empty()
            || scenario.revision == 0
        {
            return Err(RegistryError::InvalidSyncState);
        }
        if (kind == "content" && (left.is_none() || right.is_none()))
            || (kind == "deletion" && left.is_none() && right.is_none())
        {
            return Err(RegistryError::InvalidSyncState);
        }
        let exists: Option<String> = self
            .db
            .query_row(
                "SELECT scenario_id FROM scenario_revisions WHERE scenario_id=?1 AND revision=?2",
                params![scenario.scenario_id, scenario.revision as i64],
                |row| row.get(0),
            )
            .optional()?;
        if exists.is_none() {
            return Err(RegistryError::NotFound);
        }
        let id: String = self
            .db
            .query_row("SELECT lower(hex(randomblob(16)))", [], |row| row.get(0))?;
        self.db.execute(
            "INSERT INTO scenario_conflict_tasks(
                id,scenario_id,scenario_revision,relative_path,kind,source_is_left,decision,state,
                left_object_revision,left_size,left_checksum_md5,
                right_object_revision,right_size,right_checksum_md5
             ) VALUES(?1,?2,?3,?4,?5,?6,NULL,'pending',?7,?8,?9,?10,?11,?12)
             ON CONFLICT(scenario_id,scenario_revision,relative_path,kind) DO UPDATE SET
                source_is_left=excluded.source_is_left, decision=NULL, state='pending',
                left_object_revision=excluded.left_object_revision,left_size=excluded.left_size,
                left_checksum_md5=excluded.left_checksum_md5,
                right_object_revision=excluded.right_object_revision,right_size=excluded.right_size,
                right_checksum_md5=excluded.right_checksum_md5",
            params![
                id,
                scenario.scenario_id,
                scenario.revision as i64,
                relative_path,
                kind,
                source_is_left,
                left.and_then(|value| value.object_revision.as_deref()),
                left.map(|value| value.size as i64),
                left.and_then(|value| value.checksum_md5.as_deref()),
                right.and_then(|value| value.object_revision.as_deref()),
                right.map(|value| value.size as i64),
                right.and_then(|value| value.checksum_md5.as_deref())
            ],
        )?;
        self.db
            .query_row(
                "SELECT id FROM scenario_conflict_tasks
                 WHERE scenario_id=?1 AND scenario_revision=?2 AND relative_path=?3 AND kind=?4",
                params![
                    scenario.scenario_id,
                    scenario.revision as i64,
                    relative_path,
                    kind
                ],
                |row| row.get(0),
            )
            .map_err(Into::into)
    }

    pub fn list_conflict_tasks(
        &self,
        scenario: &ScenarioReference,
    ) -> Result<Vec<ScenarioConflictTask>> {
        let mut statement = self.db.prepare(
            "SELECT id,scenario_id,scenario_revision,relative_path,kind,source_is_left,decision,state,
                    left_object_revision,left_size,left_checksum_md5,
                    right_object_revision,right_size,right_checksum_md5
             FROM scenario_conflict_tasks WHERE scenario_id=?1 AND scenario_revision=?2
             ORDER BY state, relative_path, id",
        )?;
        let tasks = statement
            .query_map(
                params![scenario.scenario_id, scenario.revision as i64],
                |row| {
                    Ok(ScenarioConflictTask {
                        id: row.get(0)?,
                        scenario: ScenarioReference {
                            scenario_id: row.get(1)?,
                            revision: row.get::<_, i64>(2)? as u64,
                        },
                        relative_path: row.get(3)?,
                        kind: row.get(4)?,
                        left: decode_confirmed_version(row, 8, 9, 10)?,
                        right: decode_confirmed_version(row, 11, 12, 13)?,
                        source_is_left: row.get(5)?,
                        decision: row.get(6)?,
                        state: row.get(7)?,
                    })
                },
            )?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(RegistryError::from)?;
        Ok(tasks)
    }

    pub fn mark_conflict_task_stale(&self, id: &str) -> Result<()> {
        self.changed(self.db.execute(
            "UPDATE scenario_conflict_tasks SET state='stale' WHERE id=?1 AND state IN ('pending','selected')",
            params![id],
        )?)
    }

    pub fn complete_conflict_task(&self, id: &str) -> Result<()> {
        self.changed(self.db.execute(
            "UPDATE scenario_conflict_tasks SET state='completed' WHERE id=?1 AND state='selected'",
            params![id],
        )?)
    }

    /// Associate a scheduler coordinator with a conflict task without adding a
    /// JobStore dependency to the conflict queue.
    pub fn link_schedule_conflict(&self, schedule_job_id: &str, conflict_id: &str) -> Result<()> {
        self.changed(self.db.execute(
            "INSERT OR IGNORE INTO schedule_conflict_links(schedule_job_id,conflict_id)
             SELECT ?1,id FROM scenario_conflict_tasks WHERE id=?2",
            params![schedule_job_id, conflict_id],
        )?)
    }

    /// Return the coordinators linked to `conflict_id` whose linked tasks are
    /// all terminal (`completed` or `stale`). The flag is true only when every
    /// linked task completed. Stale work never becomes a successful
    /// coordinator Job; a pending or selected task keeps it waiting.
    pub fn resolved_schedule_jobs_for_conflict(
        &self,
        conflict_id: &str,
    ) -> Result<Vec<(String, bool)>> {
        let mut statement = self.db.prepare(
            "SELECT l.schedule_job_id,
                    NOT EXISTS (
                        SELECT 1 FROM schedule_conflict_links other
                        JOIN scenario_conflict_tasks task ON task.id=other.conflict_id
                        WHERE other.schedule_job_id=l.schedule_job_id AND task.state='stale'
                    )
             FROM schedule_conflict_links l
             WHERE l.conflict_id=?1 AND NOT EXISTS (
                SELECT 1 FROM schedule_conflict_links other
                JOIN scenario_conflict_tasks task ON task.id=other.conflict_id
                WHERE other.schedule_job_id=l.schedule_job_id
                  AND task.state NOT IN ('completed','stale')
             )
             ORDER BY l.schedule_job_id",
        )?;
        let jobs = statement
            .query_map([conflict_id], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(RegistryError::from)?;
        Ok(jobs)
    }

    /// Read one task only when it is still eligible for an explicit effect.
    /// Callers must revalidate both persisted snapshots before acting.
    pub fn selected_conflict_task(&self, id: &str) -> Result<Option<ScenarioConflictTask>> {
        let Some(task) = self
            .db
            .query_row(
                "SELECT scenario_id,scenario_revision FROM scenario_conflict_tasks
             WHERE id=?1 AND state='selected'",
                params![id],
                |row| {
                    Ok(ScenarioReference {
                        scenario_id: row.get(0)?,
                        revision: row.get::<_, i64>(1)? as u64,
                    })
                },
            )
            .optional()?
        else {
            return Ok(None);
        };
        Ok(self
            .list_conflict_tasks(&task)?
            .into_iter()
            .find(|entry| entry.id == id))
    }

    pub fn select_conflict_task(
        &self,
        id: &str,
        source_is_left: Option<bool>,
        decision: &str,
    ) -> Result<()> {
        if id.is_empty() || source_is_left.is_none() {
            return Err(RegistryError::InvalidSyncState);
        }
        let kind: String = self
            .db
            .query_row(
                "SELECT kind FROM scenario_conflict_tasks
                 WHERE id=?1 AND state IN ('pending','selected')",
                params![id],
                |row| row.get(0),
            )
            .optional()?
            .ok_or(RegistryError::NotFound)?;
        let valid_decision = match kind.as_str() {
            "content" => matches!(
                decision,
                "replace_from_source" | "replace_if_newer" | "skip_transfer"
            ),
            "deletion" => matches!(decision, "delete_as_source" | "keep_file"),
            _ => false,
        };
        if !valid_decision {
            return Err(RegistryError::InvalidSyncState);
        }
        let state = if matches!(decision, "skip_transfer" | "keep_file") {
            "completed"
        } else {
            "selected"
        };
        self.changed(self.db.execute(
            "UPDATE scenario_conflict_tasks SET source_is_left=?1,decision=?2,state=?3
             WHERE id=?4 AND state IN ('pending','selected')",
            params![source_is_left, decision, state, id],
        )?)
    }

    /// Compose a deterministic, read-only dry-run from the exact persisted
    /// Scenario revision and its confirmed baseline. Snapshots are supplied by
    /// an already-authorized caller; this method never scans adapters, reads
    /// content, creates a Job, or advances FileSyncState.
    pub fn plan_scenario_dry_run(
        &self,
        reference: &ScenarioReference,
        left_snapshot: &[ObservedFile],
        right_snapshot: &[ObservedFile],
    ) -> Result<Vec<PlanItem>> {
        self.plan_scenario_dry_run_with_skipped(reference, left_snapshot, right_snapshot, &[], &[])
    }

    /// Plan with the paths of skipped symbolic links and special files.
    pub fn plan_scenario_dry_run_with_skipped(
        &self,
        reference: &ScenarioReference,
        left_snapshot: &[ObservedFile],
        right_snapshot: &[ObservedFile],
        skipped_left: &[String],
        skipped_right: &[String],
    ) -> Result<Vec<PlanItem>> {
        let stored = self
            .get_scenario_revision(&reference.scenario_id, reference.revision)?
            .ok_or(RegistryError::NotFound)?;
        let direction = match stored.direction.as_str() {
            "left_to_right" => SyncDirection::LeftToRight,
            "right_to_left" => SyncDirection::RightToLeft,
            "bidirectional" => SyncDirection::Bidirectional,
            _ => return Err(RegistryError::PlanningFailed),
        };
        let scenario = ScenarioDraft {
            name: stored.name,
            left: StorageEndpoint {
                connection_id: stored.left_connection_id,
                root: stored.left_root,
            },
            right: StorageEndpoint {
                connection_id: stored.right_connection_id,
                root: stored.right_root,
            },
            direction,
            // Filters and executable policy decisions are not persisted in the
            // current Registry schema, so this composition never invents them.
            filters: ScenarioFilters::default(),
            policies: ScenarioPolicies::default(),
        };
        let baseline = self.list_confirmed_file_states(reference)?;
        scenario_planner::plan_with_skipped(
            &scenario,
            reference,
            &baseline,
            left_snapshot,
            right_snapshot,
            skipped_left,
            skipped_right,
        )
        .map_err(|_| RegistryError::PlanningFailed)
    }

    fn changed(&self, count: usize) -> Result<()> {
        if count == 0 {
            Err(RegistryError::NotFound)
        } else {
            Ok(())
        }
    }
}

/// Lexical overlap check for two absolute Local roots: equal roots, or one root
/// inside the other. Components compare without letter case and Unicode form,
/// as on APFS. Relative or provider roots never overlap here; executors still
/// check the opened directories.
pub fn scenario_roots_overlap(left: &str, right: &str) -> bool {
    if !left.starts_with('/') || !right.starts_with('/') {
        return false;
    }
    let parts = |root: &str| -> Vec<String> {
        root.split('/')
            .filter(|part| !part.is_empty() && *part != ".")
            .map(crate::scenario_planner::name_key)
            .collect()
    };
    let (left, right) = (parts(left), parts(right));
    let shared = left.len().min(right.len());
    left[..shared] == right[..shared]
}

fn valid_name(name: &str) -> bool {
    let trimmed = name.trim();
    !trimmed.is_empty() && trimmed.chars().count() <= 120 && !trimmed.chars().any(char::is_control)
}
fn valid_adapter(adapter: &str) -> bool {
    !adapter.is_empty()
        && adapter.len() <= 64
        && adapter
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_' || c == b'-')
}

fn valid_sync_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
        && !path.chars().any(char::is_control)
}

fn valid_confirmed_version(version: Option<&ConfirmedFileVersion>) -> bool {
    let Some(version) = version else { return true };
    version.object_revision.as_deref().is_none_or(|value| {
        !value.is_empty() && value.len() <= 512 && !value.chars().any(char::is_control)
    }) && version
        .checksum_md5
        .as_deref()
        .is_none_or(|value| value.len() == 32 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

fn decode_confirmed_version(
    row: &rusqlite::Row<'_>,
    revision_index: usize,
    size_index: usize,
    checksum_index: usize,
) -> std::result::Result<Option<ConfirmedFileVersion>, rusqlite::Error> {
    let object_revision: Option<String> = row.get(revision_index)?;
    let size: Option<i64> = row.get(size_index)?;
    let checksum_md5: Option<String> = row.get(checksum_index)?;
    match (object_revision, size, checksum_md5) {
        (None, None, None) => Ok(None),
        (object_revision, Some(size), checksum_md5) if size >= 0 => {
            Ok(Some(ConfirmedFileVersion {
                object_revision,
                size: size as u64,
                checksum_md5,
            }))
        }
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}

fn validate_schedule(timezone: &str, weekday: u8, hour: u8, minute: u8) -> Result<()> {
    let timezone = Tz::from_str(timezone).map_err(|_| RegistryError::InvalidSchedule)?;
    let weekday = match weekday {
        0 => Weekday::Mon,
        1 => Weekday::Tue,
        2 => Weekday::Wed,
        3 => Weekday::Thu,
        4 => Weekday::Fri,
        5 => Weekday::Sat,
        6 => Weekday::Sun,
        _ => return Err(RegistryError::InvalidSchedule),
    };
    WeeklySchedule {
        timezone,
        weekday,
        hour: hour as u32,
        minute: minute as u32,
    }
    .validate()
    .map_err(|_| RegistryError::InvalidSchedule)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conflict_snapshot(revision: &str) -> ConfirmedFileVersion {
        ConfirmedFileVersion {
            object_revision: Some(revision.into()),
            size: 3,
            checksum_md5: None,
        }
    }

    #[test]
    fn registry_create_list_rename_disable_remove() {
        let db = Registry::in_memory().unwrap();
        let id = db.add("yandex_disk", "  Personal  ").unwrap();
        assert_eq!(db.get(&id).unwrap().unwrap().name, "Personal");
        assert_eq!(db.list().unwrap().len(), 1);
        db.rename(&id, "Archive").unwrap();
        db.set_enabled(&id, false).unwrap();
        let connection = db.get(&id).unwrap().unwrap();
        assert_eq!(connection.name, "Archive");
        assert!(!connection.enabled);
        db.remove(&id).unwrap();
        assert!(db.get(&id).unwrap().is_none());
        assert!(matches!(db.remove(&id), Err(RegistryError::NotFound)));
    }

    #[test]
    fn finalizing_disconnect_clears_only_local_credential_state() {
        let db = Registry::in_memory().unwrap();
        let id = db.add("yandex_disk", "Personal").unwrap();
        db.set_provider_account_id(&id, Some("account-a")).unwrap();
        db.set_credential_ref(&id, Some("keychain:connection-a"))
            .unwrap();
        db.set_enabled(&id, false).unwrap();
        db.mark_checked(&id, "error", 7, Some("cleanup pending"))
            .unwrap();
        db.finalize_disconnect(&id).unwrap();
        let entry = db.get(&id).unwrap().unwrap();
        assert!(entry.enabled);
        assert_eq!(entry.status, "disconnected");
        assert_eq!(entry.credential_ref, None);
        assert_eq!(entry.provider_account_id.as_deref(), Some("account-a"));
        assert_eq!(entry.last_checked_at, None);
        assert_eq!(entry.diagnosis, None);
    }

    #[test]
    fn rejects_invalid_names_and_adapters() {
        let db = Registry::in_memory().unwrap();
        assert!(matches!(
            db.add("bad adapter", "OK"),
            Err(RegistryError::InvalidAdapter)
        ));
        assert!(matches!(
            db.add("yandex_disk", "  "),
            Err(RegistryError::InvalidName)
        ));
        assert!(matches!(
            db.add("yandex_disk", "x\nsecret"),
            Err(RegistryError::InvalidName)
        ));
    }

    #[test]
    fn diagnostic_metadata_and_keychain_reference_are_separate() {
        let db = Registry::in_memory().unwrap();
        let id = db.add("icloud_documents", "Backup").unwrap();
        db.set_credential_ref(&id, Some("keychain-item-id"))
            .unwrap();
        db.mark_checked(&id, "offline", 1234, Some("No network"))
            .unwrap();
        let connection = db.get(&id).unwrap().unwrap();
        assert_eq!(
            connection.credential_ref.as_deref(),
            Some("keychain-item-id")
        );
        assert_eq!(connection.last_checked_at, Some(1234));
        assert_eq!(connection.status, "offline");
        assert_eq!(connection.diagnosis.as_deref(), Some("No network"));
    }

    #[test]
    fn multiple_connections_to_same_adapter_and_account_remain_independent() {
        let db = Registry::in_memory().unwrap();
        let first = db.add("yandex_disk", "Personal").unwrap();
        let second = db
            .add("yandex_disk", "Another connection to Personal")
            .unwrap();
        let third = db.add("yandex_disk", "Work").unwrap();
        assert_ne!(first, second);
        assert_ne!(second, third);
        db.set_credential_ref(&first, Some("keychain-connection-a"))
            .unwrap();
        db.set_credential_ref(&second, Some("keychain-connection-b"))
            .unwrap();
        db.set_credential_ref(&third, Some("keychain-connection-c"))
            .unwrap();
        db.mark_checked(&first, "connected", 10, None).unwrap();
        db.set_enabled(&second, false).unwrap();
        db.remove(&third).unwrap();
        let entries = db.list().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(db.get(&first).unwrap().unwrap().status, "connected");
        assert!(!db.get(&second).unwrap().unwrap().enabled);
        assert_eq!(
            db.get(&first).unwrap().unwrap().credential_ref.as_deref(),
            Some("keychain-connection-a")
        );
        assert_eq!(
            db.get(&second).unwrap().unwrap().credential_ref.as_deref(),
            Some("keychain-connection-b")
        );
    }

    #[test]
    fn same_provider_account_allows_independent_connections() {
        let db = Registry::in_memory().unwrap();
        let a = db.add("yandex_disk", "Primary").unwrap();
        let b = db.add("yandex_disk", "Second").unwrap();
        db.set_provider_account_id(&a, Some("account-123")).unwrap();
        db.set_provider_account_id(&b, Some("account-123")).unwrap();
        assert_eq!(
            db.get(&a).unwrap().unwrap().provider_account_id.as_deref(),
            Some("account-123")
        );
        assert_eq!(
            db.get(&b).unwrap().unwrap().provider_account_id.as_deref(),
            Some("account-123")
        );
        db.remove(&a).unwrap();
        assert!(db.get(&b).unwrap().is_some());
    }

    #[test]
    fn scenario_is_revisioned_and_blocks_referenced_connection_removal() {
        let db = Registry::in_memory().unwrap();
        let left = db.add("local_filesystem", "Left").unwrap();
        let right = db.add("local_filesystem", "Right").unwrap();
        let id = db
            .create_scenario(
                "Documents",
                &left,
                "/documents",
                &right,
                "/archive",
                "left_to_right",
            )
            .unwrap();
        let scenario = db.list_scenarios().unwrap().pop().unwrap();
        assert_eq!(scenario.id, id);
        assert_eq!(scenario.revision, 1);
        assert!(scenario.enabled);
        db.set_scenario_enabled(&id, false).unwrap();
        assert!(!db.list_scenarios().unwrap()[0].enabled);
        assert!(matches!(
            db.remove(&left),
            Err(RegistryError::ConnectionReferencedByScenario)
        ));
    }

    #[test]
    fn confirmed_file_state_requires_evidence_and_stays_bound_to_revision() {
        let db = Registry::in_memory().unwrap();
        let left = db.add("local_filesystem", "Left").unwrap();
        let right = db.add("local_filesystem", "Right").unwrap();
        let scenario_id = db
            .create_scenario(
                "Documents",
                &left,
                "/documents",
                &right,
                "/archive",
                "left_to_right",
            )
            .unwrap();
        let reference = ScenarioReference {
            scenario_id: scenario_id.clone(),
            revision: 1,
        };
        let state = FileSyncState {
            scenario: reference.clone(),
            relative_path: "report.pdf".into(),
            left: Some(ConfirmedFileVersion {
                object_revision: Some("local-1".into()),
                size: 12,
                checksum_md5: Some("abcdef0123456789abcdef0123456789".into()),
            }),
            right: Some(ConfirmedFileVersion {
                object_revision: Some("remote-1".into()),
                size: 12,
                checksum_md5: Some("abcdef0123456789abcdef0123456789".into()),
            }),
        };
        assert!(matches!(
            db.record_confirmed_file_state(
                &state,
                Confirmation::Confirmed(EvidenceLevel::ProviderAcknowledged)
            ),
            Err(RegistryError::InsufficientEvidence)
        ));
        db.record_confirmed_file_state(
            &state,
            Confirmation::Confirmed(EvidenceLevel::MetadataVerified),
        )
        .unwrap();
        assert_eq!(
            db.list_confirmed_file_states(&reference).unwrap(),
            vec![state]
        );

        db.update_scenario(
            &scenario_id,
            ScenarioUpdate {
                name: "Documents v2",
                left_connection_id: &left,
                left_root: "/documents",
                right_connection_id: &right,
                right_root: "/archive",
                direction: "left_to_right",
            },
        )
        .unwrap();
        let later = ScenarioReference {
            scenario_id,
            revision: 2,
        };
        assert!(db.list_confirmed_file_states(&later).unwrap().is_empty());
    }

    #[test]
    fn conflict_tasks_are_an_independent_reopenable_queue() {
        let db = Registry::in_memory().unwrap();
        let left = db.add("local_filesystem", "Left").unwrap();
        let right = db.add("local_filesystem", "Right").unwrap();
        let scenario_id = db
            .create_scenario(
                "Documents",
                &left,
                "/documents",
                &right,
                "/archive",
                "bidirectional",
            )
            .unwrap();
        let reference = ScenarioReference {
            scenario_id,
            revision: 1,
        };

        let id = db
            .queue_conflict_task(
                &reference,
                "report.pdf",
                "content",
                None,
                Some(&conflict_snapshot("left")),
                Some(&conflict_snapshot("right")),
            )
            .unwrap();
        assert_eq!(
            db.list_conflict_tasks(&reference).unwrap(),
            vec![ScenarioConflictTask {
                id: id.clone(),
                scenario: reference.clone(),
                relative_path: "report.pdf".into(),
                kind: "content".into(),
                left: Some(conflict_snapshot("left")),
                right: Some(conflict_snapshot("right")),
                source_is_left: None,
                decision: None,
                state: "pending".into(),
            }]
        );
        assert!(db
            .list_confirmed_file_states(&reference)
            .unwrap()
            .is_empty());

        db.select_conflict_task(&id, Some(false), "replace_from_source")
            .unwrap();
        let selected = db.list_conflict_tasks(&reference).unwrap();
        assert_eq!(selected[0].source_is_left, Some(false));
        assert_eq!(selected[0].decision.as_deref(), Some("replace_from_source"));
        assert_eq!(selected[0].state, "selected");
        assert_eq!(db.selected_conflict_task(&id).unwrap().unwrap().id, id);
        db.mark_conflict_task_stale(&id).unwrap();
        assert!(db.selected_conflict_task(&id).unwrap().is_none());
        assert_eq!(
            db.list_conflict_tasks(&reference).unwrap()[0].state,
            "stale"
        );

        let reopened = db
            .queue_conflict_task(
                &reference,
                "report.pdf",
                "content",
                None,
                Some(&conflict_snapshot("left")),
                Some(&conflict_snapshot("right")),
            )
            .unwrap();
        assert_eq!(reopened, id);
        let reopened_task = db.list_conflict_tasks(&reference).unwrap();
        assert_eq!(reopened_task[0].source_is_left, None);
        assert!(reopened_task[0].decision.is_none());
        assert_eq!(reopened_task[0].state, "pending");
    }

    #[test]
    fn conflict_task_selection_requires_a_source_and_matching_decision() {
        let db = Registry::in_memory().unwrap();
        let left = db.add("local_filesystem", "Left").unwrap();
        let right = db.add("local_filesystem", "Right").unwrap();
        let scenario_id = db
            .create_scenario(
                "Documents",
                &left,
                "/documents",
                &right,
                "/archive",
                "bidirectional",
            )
            .unwrap();
        let reference = ScenarioReference {
            scenario_id,
            revision: 1,
        };
        let content_id = db
            .queue_conflict_task(
                &reference,
                "report.pdf",
                "content",
                None,
                Some(&conflict_snapshot("left")),
                Some(&conflict_snapshot("right")),
            )
            .unwrap();
        assert!(matches!(
            db.select_conflict_task(&content_id, None, "skip_transfer"),
            Err(RegistryError::InvalidSyncState)
        ));
        assert!(matches!(
            db.select_conflict_task(&content_id, Some(true), "delete_as_source"),
            Err(RegistryError::InvalidSyncState)
        ));

        let deletion_id = db
            .queue_conflict_task(
                &reference,
                "removed.txt",
                "deletion",
                None,
                Some(&conflict_snapshot("left")),
                None,
            )
            .unwrap();
        assert!(matches!(
            db.select_conflict_task(&deletion_id, Some(true), "replace_if_newer"),
            Err(RegistryError::InvalidSyncState)
        ));
        db.select_conflict_task(&deletion_id, Some(true), "keep_file")
            .unwrap();
        assert_eq!(
            db.list_conflict_tasks(&reference)
                .unwrap()
                .into_iter()
                .find(|task| task.id == deletion_id)
                .unwrap()
                .state,
            "completed"
        );
    }

    #[test]
    fn dry_run_uses_pinned_revision_and_never_advances_baseline() {
        use crate::scenario_planner::PlanAction;

        let db = Registry::in_memory().unwrap();
        let left = db.add("local_filesystem", "Left").unwrap();
        let right = db.add("local_filesystem", "Right").unwrap();
        let scenario_id = db
            .create_scenario(
                "Documents",
                &left,
                "/source",
                &right,
                "/target",
                "left_to_right",
            )
            .unwrap();
        let reference = ScenarioReference {
            scenario_id: scenario_id.clone(),
            revision: 1,
        };

        // A later edit must not change an existing revision's preview.
        db.update_scenario(
            &scenario_id,
            ScenarioUpdate {
                name: "Documents reversed",
                left_connection_id: &left,
                left_root: "/source",
                right_connection_id: &right,
                right_root: "/target",
                direction: "right_to_left",
            },
        )
        .unwrap();

        let plan = db
            .plan_scenario_dry_run(
                &reference,
                &[ObservedFile {
                    relative_path: "new.txt".into(),
                    object_revision: Some("left-1".into()),
                    size: 4,
                    checksum_md5: Some("abcdef0123456789abcdef0123456789".into()),
                }],
                &[],
            )
            .unwrap();
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].relative_path, "new.txt");
        assert_eq!(plan[0].action, PlanAction::Create { from_left: true });
        assert!(db
            .list_confirmed_file_states(&reference)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn confirmed_create_batch_pins_revision_and_stable_child_job_ids() {
        let mut db = Registry::in_memory().unwrap();
        let left = db.add("local_filesystem", "Left").unwrap();
        let right = db.add("local_filesystem", "Right").unwrap();
        let scenario_id = db
            .create_scenario(
                "Documents",
                &left,
                "/source",
                &right,
                "/target",
                "left_to_right",
            )
            .unwrap();
        let receipt = db
            .create_scenario_create_batch(
                &ScenarioReference {
                    scenario_id: scenario_id.clone(),
                    revision: 1,
                },
                &[("b.txt".into(), false), ("a.txt".into(), true)],
            )
            .unwrap();
        assert_eq!(receipt.scenario.scenario_id, scenario_id);
        assert_eq!(receipt.scenario.revision, 1);
        assert_eq!(
            receipt
                .children
                .iter()
                .map(|child| child.relative_path.as_str())
                .collect::<Vec<_>>(),
            vec!["a.txt", "b.txt"]
        );
        assert!(receipt.children[0].source_is_left);
        assert!(!receipt.children[1].source_is_left);
        assert!(receipt
            .children
            .iter()
            .all(|child| child.job_id.starts_with("scenario-batch-copy-")));
        assert_eq!(
            db.scenario_create_batch(&receipt.id).unwrap(),
            Some(receipt.clone())
        );
        db.request_scenario_create_batch_cancellation(&receipt.id)
            .unwrap();
        assert!(
            db.scenario_create_batch(&receipt.id)
                .unwrap()
                .unwrap()
                .cancellation_requested
        );
        assert_eq!(db.scenario_create_batch("missing").unwrap(), None);
        assert_eq!(
            db.pending_scenario_create_batches(&receipt.scenario)
                .unwrap()
                .iter()
                .map(|batch| batch.id.as_str())
                .collect::<Vec<_>>(),
            vec![receipt.id.as_str()]
        );
        db.set_scenario_create_batch_child_state(&receipt.id, "a.txt", "completed")
            .unwrap();
        assert_eq!(
            db.pending_scenario_create_batches(&receipt.scenario)
                .unwrap()
                .len(),
            1
        );
        db.set_scenario_create_batch_child_state(&receipt.id, "b.txt", "unknown")
            .unwrap();
        assert!(db
            .pending_scenario_create_batches(&receipt.scenario)
            .unwrap()
            .is_empty());
        assert!(matches!(
            db.create_scenario_create_batch(
                &receipt.scenario,
                &[("a.txt".into(), true), ("a.txt".into(), false)],
            ),
            Err(RegistryError::InvalidSyncState)
        ));
    }

    #[test]
    fn scenario_update_creates_a_new_revision_without_changing_enablement() {
        let db = Registry::in_memory().unwrap();
        let left = db.add("local_filesystem", "Left").unwrap();
        let right = db.add("yandex_disk", "Right").unwrap();
        let id = db
            .create_scenario(
                "Documents",
                &left,
                "/old",
                &right,
                "/archive",
                "left_to_right",
            )
            .unwrap();
        db.set_scenario_enabled(&id, false).unwrap();

        db.update_scenario(
            &id,
            ScenarioUpdate {
                name: "Updated documents",
                left_connection_id: &right,
                left_root: "/source",
                right_connection_id: &left,
                right_root: "/target",
                direction: "right_to_left",
            },
        )
        .unwrap();

        let scenario = db.list_scenarios().unwrap().pop().unwrap();
        assert_eq!(scenario.name, "Updated documents");
        assert_eq!(scenario.left_connection_id, right);
        assert_eq!(scenario.right_connection_id, left);
        assert_eq!(scenario.revision, 2);
        assert!(!scenario.enabled);

        let revision_one = db.get_scenario_revision(&id, 1).unwrap().unwrap();
        assert_eq!(revision_one.name, "Documents");
        assert_eq!(revision_one.left_root, "/old");
        assert_eq!(revision_one.direction, "left_to_right");
        let revision_two = db.get_scenario_revision(&id, 2).unwrap().unwrap();
        assert_eq!(revision_two.name, "Updated documents");
        assert_eq!(revision_two.left_root, "/source");
        assert_eq!(revision_two.direction, "right_to_left");
    }

    #[test]
    fn automation_pause_and_coalesced_trigger_survive_reopen_without_starting_jobs() {
        use std::time::{SystemTime, UNIX_EPOCH};
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("qadren-automation-{unique}.sqlite3"));
        let db = Registry::open(&path).unwrap();
        let left = db.add("local_filesystem", "Left").unwrap();
        let right = db.add("local_filesystem", "Right").unwrap();
        let scenario = db
            .create_scenario(
                "Documents",
                &left,
                "/documents",
                &right,
                "/archive",
                "left_to_right",
            )
            .unwrap();
        assert_eq!(
            db.request_scenario_automatic_run(&scenario, false).unwrap(),
            AutomaticTriggerResult::Queued
        );
        assert_eq!(
            db.request_scenario_automatic_run(&scenario, true).unwrap(),
            AutomaticTriggerResult::Coalesced
        );
        drop(db);

        let db = Registry::open(&path).unwrap();
        assert_eq!(
            db.scenario_automation_snapshot(&scenario).unwrap(),
            ScenarioAutomationSnapshot {
                locally_paused: false,
                pending_automatic_run: true,
            }
        );
        db.set_scenario_locally_paused(&scenario, true).unwrap();
        assert_eq!(
            db.request_scenario_automatic_run(&scenario, false).unwrap(),
            AutomaticTriggerResult::IgnoredPaused
        );
        assert_eq!(
            db.scenario_automation_snapshot(&scenario).unwrap(),
            ScenarioAutomationSnapshot {
                locally_paused: true,
                pending_automatic_run: false,
            }
        );
        db.set_scenario_enabled(&scenario, false).unwrap();
        assert_eq!(
            db.request_scenario_automatic_run(&scenario, false).unwrap(),
            AutomaticTriggerResult::IgnoredDisabled
        );
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn scenario_supports_multiple_validated_schedules_without_job_submission() {
        let db = Registry::in_memory().unwrap();
        let left = db.add("local_filesystem", "Left").unwrap();
        let right = db.add("local_filesystem", "Right").unwrap();
        let scenario = db
            .create_scenario(
                "Documents",
                &left,
                "/documents",
                &right,
                "/archive",
                "left_to_right",
            )
            .unwrap();
        db.create_scenario_schedule(&scenario, "Europe/Samara", 0, 9, 30)
            .unwrap();
        db.create_scenario_schedule(&scenario, "Europe/Berlin", 6, 2, 30)
            .unwrap();
        let schedules = db.list_scenario_schedules(&scenario).unwrap();
        assert_eq!(schedules.len(), 2);
        assert_eq!(schedules[0].timezone, "Europe/Samara");
        assert!(matches!(
            db.create_scenario_schedule(&scenario, "not/a-zone", 0, 9, 0),
            Err(RegistryError::InvalidSchedule)
        ));
        assert!(matches!(
            db.create_scenario_schedule(&scenario, "Europe/Samara", 7, 9, 0),
            Err(RegistryError::InvalidSchedule)
        ));

        let schedule = schedules[0].id.clone();
        db.set_scenario_schedule_enabled(&scenario, &schedule, false)
            .unwrap();
        assert!(!db.list_scenario_schedules(&scenario).unwrap()[0].enabled);
        assert!(matches!(
            db.set_scenario_schedule_enabled("other", &schedule, true),
            Err(RegistryError::NotFound)
        ));
        db.remove_scenario_schedule(&scenario, &schedule).unwrap();
        assert_eq!(db.list_scenario_schedules(&scenario).unwrap().len(), 1);
        assert!(matches!(
            db.remove_scenario_schedule(&scenario, &schedule),
            Err(RegistryError::NotFound)
        ));
    }

    #[test]
    fn schedule_occurrence_claim_survives_restart_and_deduplicates_dst_slot() {
        use std::time::{SystemTime, UNIX_EPOCH};
        let path = std::env::temp_dir().join(format!(
            "qadren-schedule-{}.sqlite3",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let db = Registry::open(&path).unwrap();
        let left = db.add("local_filesystem", "Left").unwrap();
        let right = db.add("local_filesystem", "Right").unwrap();
        let scenario = db
            .create_scenario(
                "Documents",
                &left,
                "/documents",
                &right,
                "/archive",
                "left_to_right",
            )
            .unwrap();
        let schedule = db
            .create_scenario_schedule(&scenario, "Europe/Berlin", 6, 2, 30)
            .unwrap();
        let key = "2026-10-25 02:30@Europe/Berlin";
        assert!(db.claim_schedule_occurrence(&schedule, key).unwrap());
        assert!(!db.claim_schedule_occurrence(&schedule, key).unwrap());
        drop(db);
        let db = Registry::open(&path).unwrap();
        assert!(!db.claim_schedule_occurrence(&schedule, key).unwrap());
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn due_schedule_evaluation_respects_configuration_pause_and_dst_without_claiming() {
        fn utc(value: &str) -> DateTime<Utc> {
            value.parse().unwrap()
        }

        let db = Registry::in_memory().unwrap();
        let left = db.add("local_filesystem", "Left").unwrap();
        let right = db.add("local_filesystem", "Right").unwrap();
        let scenario = db
            .create_scenario(
                "Documents",
                &left,
                "/source",
                &right,
                "/target",
                "left_to_right",
            )
            .unwrap();
        let schedule = db
            .create_scenario_schedule(&scenario, "Europe/Berlin", 6, 2, 30)
            .unwrap();
        let after = utc("2026-10-25T00:00:00Z");
        let through = utc("2026-10-25T02:00:00Z");
        let expected = vec![DueScheduleOccurrence {
            scenario_id: scenario.clone(),
            schedule_id: schedule.clone(),
            occurrence_key: OccurrenceKey("2026-10-25 02:30@Europe/Berlin".into()),
        }];
        assert_eq!(
            db.due_schedule_occurrences(after, through).unwrap(),
            expected
        );
        // Repeat is identical because evaluation has no claim/write side effect.
        assert_eq!(
            db.due_schedule_occurrences(after, through).unwrap(),
            expected
        );

        db.set_scenario_schedule_enabled(&scenario, &schedule, false)
            .unwrap();
        assert!(db
            .due_schedule_occurrences(after, through)
            .unwrap()
            .is_empty());
        db.set_scenario_schedule_enabled(&scenario, &schedule, true)
            .unwrap();
        db.set_scenario_locally_paused(&scenario, true).unwrap();
        assert!(db
            .due_schedule_occurrences(after, through)
            .unwrap()
            .is_empty());
        db.set_scenario_locally_paused(&scenario, false).unwrap();
        db.set_scenario_enabled(&scenario, false).unwrap();
        assert!(db
            .due_schedule_occurrences(after, through)
            .unwrap()
            .is_empty());
        assert!(matches!(
            db.due_schedule_occurrences(through, after),
            Err(RegistryError::InvalidSchedule)
        ));
    }

    fn schedule_utc(value: &str) -> DateTime<Utc> {
        value.parse().unwrap()
    }

    fn schedule_fixture(db: &Registry, name: &str) -> String {
        let left = db.add("local_filesystem", "Left").unwrap();
        let right = db.add("local_filesystem", "Right").unwrap();
        db.create_scenario(
            name,
            &left,
            &format!("/{name}/source"),
            &right,
            &format!("/{name}/target"),
            "left_to_right",
        )
        .unwrap()
    }

    #[test]
    fn missed_slots_after_sleep_merge_into_one_catch_up_per_scenario() {
        let db = Registry::in_memory().unwrap();
        let overlap = chrono::Duration::seconds(90);
        let documents = schedule_fixture(&db, "documents");
        let photos = schedule_fixture(&db, "photos");
        // Several weekly rules, one of them in a non-UTC IANA timezone.
        db.create_scenario_schedule(&documents, "UTC", 0, 9, 0)
            .unwrap();
        db.create_scenario_schedule(&documents, "Europe/Berlin", 2, 18, 15)
            .unwrap();
        db.create_scenario_schedule(&photos, "America/New_York", 4, 7, 45)
            .unwrap();

        // The first observation has no history and never backfills old slots.
        assert_eq!(
            db.stage_schedule_catch_up(schedule_utc("2026-10-05T08:00:00Z"), overlap)
                .unwrap(),
            0
        );
        assert_eq!(
            db.schedule_observed_through().unwrap(),
            Some(schedule_utc("2026-10-05T08:00:00Z"))
        );

        // The Worker is unavailable for two weeks. Four slots of `documents`
        // and two slots of `photos` are missed.
        let woke = schedule_utc("2026-10-19T08:00:00Z");
        assert_eq!(db.stage_schedule_catch_up(woke, overlap).unwrap(), 2);
        let outbox = db.pending_schedule_job_outbox().unwrap();
        assert_eq!(outbox.len(), 2);
        let mut scenarios: Vec<_> = outbox.iter().map(|e| e.scenario_id.clone()).collect();
        scenarios.sort();
        let mut expected = vec![documents.clone(), photos.clone()];
        expected.sort();
        assert_eq!(scenarios, expected);
        let claimed: i64 = db
            .db
            .query_row("SELECT count(*) FROM schedule_occurrences", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(claimed, 6, "merged slots stay claimed and never replay");

        // Repeating the same observation or a short later poll adds nothing.
        assert_eq!(db.stage_schedule_catch_up(woke, overlap).unwrap(), 0);
        assert_eq!(
            db.stage_schedule_catch_up(schedule_utc("2026-10-19T08:01:00Z"), overlap)
                .unwrap(),
            0
        );
        assert_eq!(db.pending_schedule_job_outbox().unwrap().len(), 2);
    }

    #[test]
    fn catch_up_ignores_slots_missed_while_paused_or_disabled() {
        let db = Registry::in_memory().unwrap();
        let overlap = chrono::Duration::seconds(90);
        let scenario = schedule_fixture(&db, "documents");
        let schedule = db
            .create_scenario_schedule(&scenario, "UTC", 0, 9, 0)
            .unwrap();
        db.stage_schedule_catch_up(schedule_utc("2026-10-05T08:00:00Z"), overlap)
            .unwrap();

        // A durable local pause survives a Registry reopen in production.
        // Polls continue while it is active, so its slots are observed and
        // not converted into a later catch-up.
        db.set_scenario_locally_paused(&scenario, true).unwrap();
        assert_eq!(
            db.stage_schedule_catch_up(schedule_utc("2026-10-12T10:00:00Z"), overlap)
                .unwrap(),
            0
        );
        db.set_scenario_locally_paused(&scenario, false).unwrap();
        assert_eq!(
            db.stage_schedule_catch_up(schedule_utc("2026-10-12T10:01:00Z"), overlap)
                .unwrap(),
            0
        );

        db.set_scenario_schedule_enabled(&scenario, &schedule, false)
            .unwrap();
        assert_eq!(
            db.stage_schedule_catch_up(schedule_utc("2026-10-19T10:00:00Z"), overlap)
                .unwrap(),
            0
        );
        db.set_scenario_schedule_enabled(&scenario, &schedule, true)
            .unwrap();
        assert_eq!(
            db.stage_schedule_catch_up(schedule_utc("2026-10-26T09:00:00Z"), overlap)
                .unwrap(),
            1
        );
        assert!(db.pending_schedule_job_outbox().unwrap()[0]
            .job_id
            .ends_with("2026-10-26 09:00@UTC"));
    }

    #[test]
    fn clock_moving_backward_uses_only_the_overlap_window() {
        let db = Registry::in_memory().unwrap();
        let overlap = chrono::Duration::seconds(90);
        let scenario = schedule_fixture(&db, "documents");
        db.create_scenario_schedule(&scenario, "UTC", 0, 9, 0)
            .unwrap();
        db.stage_schedule_catch_up(schedule_utc("2026-10-26T12:00:00Z"), overlap)
            .unwrap();
        // The system clock is corrected to an earlier time. The evaluator does
        // not fail and does not schedule the skipped-back period twice.
        assert_eq!(
            db.stage_schedule_catch_up(schedule_utc("2026-10-19T09:00:30Z"), overlap)
                .unwrap(),
            1
        );
        assert_eq!(
            db.stage_schedule_catch_up(schedule_utc("2026-10-19T09:01:00Z"), overlap)
                .unwrap(),
            0
        );
        assert_eq!(
            db.schedule_observed_through().unwrap(),
            Some(schedule_utc("2026-10-19T09:01:00Z"))
        );
    }

    #[test]
    fn local_scenario_roots_must_not_overlap_and_stale_revisions_are_refused() {
        assert!(scenario_roots_overlap("/data", "/data"));
        assert!(scenario_roots_overlap("/data/", "/data/inner"));
        assert!(scenario_roots_overlap(
            "/Data/Caf\u{e9}",
            "/data/cafe\u{301}/x"
        ));
        assert!(!scenario_roots_overlap("/data", "/database"));
        assert!(!scenario_roots_overlap("/data/a", "/data/b"));

        let db = Registry::in_memory().unwrap();
        let left = db.add("local_filesystem", "Left").unwrap();
        let right = db.add("local_filesystem", "Right").unwrap();
        let cloud = db.add("yandex_disk", "Cloud").unwrap();
        assert!(matches!(
            db.create_scenario(
                "Nested",
                &left,
                "/data",
                &right,
                "/data/inner",
                "left_to_right"
            ),
            Err(RegistryError::ScenarioRootsOverlap)
        ));
        // Provider paths are not paths on this Mac.
        db.create_scenario("Cloud", &left, "/data", &cloud, "/data", "left_to_right")
            .unwrap();
        let id = db
            .create_scenario(
                "Plain",
                &left,
                "/data/a",
                &right,
                "/data/b",
                "left_to_right",
            )
            .unwrap();
        assert!(matches!(
            db.update_scenario(
                &id,
                ScenarioUpdate {
                    name: "Plain",
                    left_connection_id: &left,
                    left_root: "/data/a",
                    right_connection_id: &right,
                    right_root: "/data/a/copy",
                    direction: "left_to_right",
                },
            ),
            Err(RegistryError::ScenarioRootsOverlap)
        ));
        assert!(db.is_current_scenario_revision(&id, 1).unwrap());
        db.update_scenario(
            &id,
            ScenarioUpdate {
                name: "Plain",
                left_connection_id: &left,
                left_root: "/data/a",
                right_connection_id: &right,
                right_root: "/data/c",
                direction: "left_to_right",
            },
        )
        .unwrap();
        assert!(!db.is_current_scenario_revision(&id, 1).unwrap());
        let mut db = db;
        assert!(matches!(
            db.create_scenario_create_batch(
                &ScenarioReference {
                    scenario_id: id.clone(),
                    revision: 1,
                },
                &[("file.txt".into(), true)],
            ),
            Err(RegistryError::StaleScenarioRevision)
        ));
        db.create_scenario_create_batch(
            &ScenarioReference {
                scenario_id: id,
                revision: 2,
            },
            &[("file.txt".into(), true)],
        )
        .unwrap();
    }

    #[test]
    fn staged_schedule_job_keeps_its_scenario_revision_after_an_edit() {
        let db = Registry::in_memory().unwrap();
        let overlap = chrono::Duration::seconds(90);
        let scenario = schedule_fixture(&db, "documents");
        db.create_scenario_schedule(&scenario, "UTC", 0, 9, 0)
            .unwrap();
        db.stage_schedule_catch_up(schedule_utc("2026-10-05T08:00:00Z"), overlap)
            .unwrap();
        db.stage_schedule_catch_up(schedule_utc("2026-10-05T09:00:00Z"), overlap)
            .unwrap();
        let current = db.get_scenario_revision(&scenario, 1).unwrap().unwrap();
        db.update_scenario(
            &scenario,
            ScenarioUpdate {
                name: "Edited",
                left_connection_id: &current.left_connection_id,
                left_root: &current.left_root,
                right_connection_id: &current.right_connection_id,
                right_root: "/documents/other-target",
                direction: "left_to_right",
            },
        )
        .unwrap();
        let outbox = db.pending_schedule_job_outbox().unwrap();
        assert_eq!(outbox.len(), 1);
        assert_eq!(outbox[0].scenario_revision, 1);
        assert_eq!(
            db.schedule_job_reference(&outbox[0].job_id)
                .unwrap()
                .unwrap()
                .revision,
            1
        );
    }

    #[test]
    fn migrates_v3_scenario_into_its_initial_revision_snapshot() {
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch(
            "CREATE TABLE storage_connections (
                id TEXT PRIMARY KEY NOT NULL,
                adapter TEXT NOT NULL,
                name TEXT NOT NULL,
                enabled INTEGER NOT NULL DEFAULT 1,
                status TEXT NOT NULL DEFAULT 'disconnected',
                credential_ref TEXT,
                last_checked_at INTEGER,
                diagnosis TEXT,
                provider_account_id TEXT
            );
            CREATE TABLE scenarios (
                id TEXT PRIMARY KEY NOT NULL,
                name TEXT NOT NULL,
                left_connection_id TEXT NOT NULL REFERENCES storage_connections(id) ON DELETE RESTRICT,
                left_root TEXT NOT NULL,
                right_connection_id TEXT NOT NULL REFERENCES storage_connections(id) ON DELETE RESTRICT,
                right_root TEXT NOT NULL,
                direction TEXT NOT NULL,
                enabled INTEGER NOT NULL DEFAULT 1,
                revision INTEGER NOT NULL DEFAULT 1
            );
            INSERT INTO storage_connections(id,adapter,name) VALUES
                ('left','local_filesystem','Left'),
                ('right','yandex_disk','Right');
            INSERT INTO scenarios(
                id,name,left_connection_id,left_root,right_connection_id,right_root,direction,enabled,revision
            ) VALUES('scenario','Documents','left','/source','right','/archive','left_to_right',0,7);
            PRAGMA user_version = 3;",
        )
        .unwrap();

        let db = Registry::initialize(connection).unwrap();
        let snapshot = db.get_scenario_revision("scenario", 7).unwrap().unwrap();
        assert_eq!(snapshot.name, "Documents");
        assert_eq!(snapshot.left_root, "/source");
        assert_eq!(snapshot.right_root, "/archive");
        assert_eq!(snapshot.direction, "left_to_right");
        assert!(db.get_scenario_revision("scenario", 1).unwrap().is_none());
    }

    #[test]
    fn three_connections_to_same_account_keep_independent_ids_and_credentials() {
        let db = Registry::in_memory().unwrap();
        let ids: Vec<String> = (0..3)
            .map(|n| {
                let id = db.add("yandex_disk", &format!("Connection {n}")).unwrap();
                db.set_provider_account_id(&id, Some("shared-account"))
                    .unwrap();
                db.set_credential_ref(&id, Some(&format!("keychain:{id}")))
                    .unwrap();
                id
            })
            .collect();
        assert_ne!(ids[0], ids[1]);
        assert_ne!(ids[1], ids[2]);
        assert_ne!(ids[0], ids[2]);

        db.set_enabled(&ids[1], false).unwrap();
        db.remove(&ids[0]).unwrap();
        let entries = db.list().unwrap();
        assert_eq!(entries.len(), 2);
        for id in [&ids[1], &ids[2]] {
            let entry = db.get(id).unwrap().unwrap();
            assert_eq!(entry.provider_account_id.as_deref(), Some("shared-account"));
            assert_eq!(
                entry.credential_ref.as_deref(),
                Some(format!("keychain:{id}").as_str())
            );
        }
        assert!(!db.get(&ids[1]).unwrap().unwrap().enabled);
        assert!(db.get(&ids[2]).unwrap().unwrap().enabled);
        assert!(db.get(&ids[0]).unwrap().is_none());
    }

    #[test]
    fn migrate_existing_v1_keeps_connections() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE storage_connections (
                id TEXT PRIMARY KEY NOT NULL,
                adapter TEXT NOT NULL,
                name TEXT NOT NULL,
                enabled INTEGER NOT NULL DEFAULT 1,
                status TEXT NOT NULL DEFAULT 'disconnected',
                credential_ref TEXT,
                last_checked_at INTEGER,
                diagnosis TEXT
            );
            INSERT INTO storage_connections(id, adapter, name, credential_ref)
                VALUES('original-id', 'yandex_disk', 'Existing', 'legacy-credential-ref');
            PRAGMA user_version = 1;",
        )
        .unwrap();
        let registry = Registry::initialize(db).unwrap();
        let record = registry.get("original-id").unwrap().unwrap();
        assert_eq!(
            record.credential_ref.as_deref(),
            Some("legacy-credential-ref")
        );
        assert_eq!(record.name, "Existing");
        assert!(record.provider_account_id.is_none());
        registry
            .set_provider_account_id("original-id", Some("123"))
            .unwrap();
        assert_eq!(
            registry
                .get("original-id")
                .unwrap()
                .unwrap()
                .provider_account_id
                .as_deref(),
            Some("123")
        );
    }

    #[test]
    fn refuses_newer_database_version() {
        let db = Connection::open_in_memory().unwrap();
        db.pragma_update(None, "user_version", 999).unwrap();
        assert!(matches!(
            Registry::initialize(db),
            Err(RegistryError::UnsupportedSchema(999))
        ));
    }
}
