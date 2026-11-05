//! Persistent metadata-only storage connection registry.
//! Never stores credentials, tokens, file content or remote delete requests.
use crate::automation::{AutomaticTriggerResult, ScenarioAutomationState};
use crate::confirmation::{Confirmation, EvidenceLevel};
use crate::model::{
    ConfirmedFileVersion, DeletionMode, FileSyncState, ScenarioDraft, ScenarioFilters,
    ScenarioPolicies, ScenarioReference, ScenarioScope, StorageEndpoint, SyncDirection,
};
use crate::scenario_planner::{self, ObservedFile, PlanItem};
use crate::scheduler::{every_hours_anchor, OccurrenceKey, Schedule, ScheduleRule};
use crate::write_roots::{conflict, scenario_roots, ScenarioRoot};
use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::path::Path;
use std::str::FromStr;

/// SFTP settings of one ISPmanager connection (D-027 section 7a).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IspmanagerSettings {
    pub ssh_port: u16,
    pub host_key_sha256: Option<String>,
    /// The report of the last connection check, as Core produced it.
    pub last_check: Option<serde_json::Value>,
    pub last_check_at: Option<i64>,
}

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
    /// A selected path is empty, absolute, or leaves the Scenario root.
    InvalidSelection,
    /// An SMB share address is not `smb://host/share`, or it carries a
    /// login, a password, a query, or a fragment.
    InvalidEndpoint,
    /// A Scenario root of an SMB share is outside the QADREN mount of that
    /// connection (D-022).
    RootOutsideConnection,
    /// A Yandex Disk connection is not the source of a one-way Scenario
    /// with a destination on this Mac (D-023, stage 1).
    UnsupportedYandexScope,
    /// An ISPmanager panel URL is not `https://host[:port][/path]`, or it
    /// carries user information or a query (D-027).
    InvalidPanelUrl,
    /// The connection cannot be a Scenario endpoint yet. An ISPmanager
    /// connection is a source of a Backup/export Scenario only (D-027).
    UnsupportedScenarioEndpoint,
    /// Another run of the Scenario is active (D-025 section 2).
    ScenarioRunActive,
    /// A batch item of the Scenario has an unknown result. Its record must
    /// stay for reconciliation.
    ScenarioHasUnresolvedItems,
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
    /// Network address of the storage, for example `smb://nas/share`. It
    /// never contains a login or a password; those stay in the Keychain.
    pub endpoint_url: Option<String>,
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
    pub kind: ScheduleKind,
    /// Active local days. Monday is bit 0 and Sunday is bit 6.
    pub days_mask: u8,
    /// The local time of a days-at-time slot, or the local anchor time that
    /// the user selected for an every-N-hours schedule.
    pub hour: u8,
    pub minute: u8,
    pub enabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScheduleKind {
    DaysAtTime,
    /// Every N hours (1–24) from a UTC anchor.
    EveryHours {
        interval_hours: u8,
        anchor: DateTime<Utc>,
    },
}

impl ScenarioSchedule {
    /// The evaluable form of this record.
    pub fn schedule(&self) -> Result<Schedule> {
        let schedule = Schedule {
            timezone: Tz::from_str(&self.timezone).map_err(|_| RegistryError::InvalidSchedule)?,
            days_mask: self.days_mask,
            rule: match self.kind {
                ScheduleKind::DaysAtTime => ScheduleRule::DaysAtTime {
                    hour: self.hour.into(),
                    minute: self.minute.into(),
                },
                ScheduleKind::EveryHours {
                    interval_hours,
                    anchor,
                } => ScheduleRule::EveryHours {
                    interval_hours: interval_hours.into(),
                    anchor,
                },
            },
        };
        schedule
            .validate()
            .map_err(|_| RegistryError::InvalidSchedule)?;
        Ok(schedule)
    }
}

/// A read-only automatic-run candidate derived from persisted configuration.
/// It is not a Job, a durable claim, or authorization to begin an effect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DueScheduleOccurrence {
    pub scenario_id: String,
    pub schedule_id: String,
    pub occurrence_key: OccurrenceKey,
    pub due_at: DateTime<Utc>,
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
    /// The Worker starts no new child while the batch is paused (D-024).
    pub paused: bool,
    pub children: Vec<ScenarioBatchChild>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ScenarioBatchChild {
    pub relative_path: String,
    pub source_is_left: bool,
    pub job_id: String,
    pub state: String,
    /// Automatic retries already scheduled for this child (D-024).
    pub retries: u32,
    /// Unix seconds of the next retry of a `pending` child, if any.
    pub next_retry_at: Option<i64>,
    /// Stable code of the last failure, never a path or a secret.
    pub last_error: Option<String>,
}

/// Progress of one batch from database counts (D-024). It never lists all
/// children.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ScenarioBatchSummary {
    pub id: String,
    pub cancellation_requested: bool,
    pub paused: bool,
    pub total: u64,
    pub completed: u64,
    pub failed: u64,
    pub cancelled: u64,
    pub unknown: u64,
    /// `pending` children without a future retry time.
    pub due: u64,
    /// `pending` children with a future retry time.
    pub retry_waiting: u64,
    /// Unix seconds of the next retry, if any.
    pub next_retry_at: Option<i64>,
    /// The first due child in path order.
    pub current_path: Option<String>,
    /// The most frequent failure codes of finished children, with counts.
    pub failure_reasons: Vec<(String, u64)>,
    /// Up to 5 unfinished paths in path order, for the outcome message.
    pub unfinished_sample: Vec<String>,
}

/// Whether the Worker can start one batch child now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatchChildDue {
    Due,
    Paused,
    Cancelled,
    /// Another state, another Job ID, or no such child.
    NotDue,
}

/// The effect of one batch child (D-025 section 6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatchChildKind {
    Create,
    Update,
    Delete,
    /// A whole destination folder of a Backup/export mirror (D-027
    /// section 5.6b).
    DeleteFolder,
}

impl BatchChildKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Update => "update",
            Self::Delete => "delete",
            Self::DeleteFolder => "delete_folder",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "create" => Some(Self::Create),
            "update" => Some(Self::Update),
            "delete" => Some(Self::Delete),
            "delete_folder" => Some(Self::DeleteFolder),
            _ => None,
        }
    }
}

/// A batch child that the Worker can start now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DueBatchChild {
    pub batch_id: String,
    pub scenario: ScenarioReference,
    pub relative_path: String,
    pub source_is_left: bool,
    pub kind: BatchChildKind,
    pub job_id: String,
    pub retries: u32,
    pub cancellation_requested: bool,
}

/// The answer of `begin_schedule_run`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScheduleRunStart {
    /// The run of this schedule Job.
    Started(String),
    /// Another run of the Scenario was active; the slot started nothing.
    Skipped(String),
}

/// One durable execution of a Scenario revision (D-025): snapshots, a
/// stored plan, then the batch of its items.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScenarioRun {
    pub id: String,
    pub scenario: ScenarioReference,
    /// `manual` or `schedule`.
    pub trigger: String,
    pub schedule_job_id: Option<String>,
    pub plan_id: Option<String>,
    pub batch_id: Option<String>,
    /// `planning`, `executing`, `completed`, `failed`, or `cancelled`.
    pub state: String,
    /// Why an active run makes no progress now, for example `needs_sign_in`.
    pub waiting_reason: Option<String>,
    /// The Scenario whose run holds the write-root lock that this run waits
    /// for (`waiting_reason` is `waiting_for_scenario`).
    pub waiting_for_scenario_id: Option<String>,
    pub error: Option<String>,
    /// Plan items that this run does not execute yet (Update, deletion,
    /// conflict), until the executor supports them.
    pub deferred_items: u64,
    pub created_at: i64,
    pub finished_at: Option<i64>,
}

/// The result of a write-root lock request (D-025 section 11).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunWriteLock {
    Acquired,
    /// An active run of this Scenario holds a conflicting lock.
    WaitFor {
        scenario_id: String,
    },
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

/// Registry paths that passed `quick_check` in this process.
static VERIFIED_REGISTRIES: std::sync::Mutex<std::collections::BTreeSet<std::path::PathBuf>> =
    std::sync::Mutex::new(std::collections::BTreeSet::new());

fn check_integrity(db: &Connection) -> Result<()> {
    let integrity: String = db.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
    if integrity != "ok" {
        return Err(RegistryError::IntegrityFailed);
    }
    Ok(())
}

impl Registry {
    pub fn open(path: &Path) -> Result<Self> {
        let db = Connection::open(path)?;
        // The app, the Worker IPC thread, and the Worker run thread write the
        // Registry at the same time (D-024). Wait for a short write.
        db.busy_timeout(std::time::Duration::from_secs(10))?;
        // Callers open the Registry for each file of a run. `quick_check`
        // reads every page, so it runs once for each path in a process
        // (D-028). SQLite still reports corruption that it finds later.
        let mut verified = VERIFIED_REGISTRIES
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !verified.contains(path) {
            check_integrity(&db)?;
            // Readers never block the writer, and the Worker run threads
            // do not wait for one another for each read (D-028).
            db.pragma_update(None, "journal_mode", "WAL")?;
            verified.insert(path.to_path_buf());
        }
        drop(verified);
        Self::initialize(db)
    }

    pub fn in_memory() -> Result<Self> {
        let db = Connection::open_in_memory()?;
        check_integrity(&db)?;
        Self::initialize(db)
    }

    fn initialize(db: Connection) -> Result<Self> {
        db.pragma_update(None, "foreign_keys", "ON")?;

        let version: i32 = db.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version > 34 {
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
        if version <= 16 {
            // D-019: the selected objects and the deletion rule belong to the
            // immutable revision. Existing revisions keep the complete root
            // and the paranoid deletion rule.
            db.execute_batch(
                "BEGIN IMMEDIATE;
                 ALTER TABLE scenarios ADD COLUMN deletion_mode TEXT NOT NULL DEFAULT 'paranoid'
                   CHECK(deletion_mode IN ('paranoid','simple'));
                 ALTER TABLE scenario_revisions ADD COLUMN deletion_mode TEXT NOT NULL DEFAULT 'paranoid'
                   CHECK(deletion_mode IN ('paranoid','simple'));
                 CREATE TABLE scenario_revision_selection (
                    scenario_id TEXT NOT NULL,
                    revision INTEGER NOT NULL CHECK(revision > 0),
                    relative_path TEXT NOT NULL,
                    PRIMARY KEY(scenario_id,revision,relative_path),
                    FOREIGN KEY(scenario_id,revision)
                      REFERENCES scenario_revisions(scenario_id,revision) ON DELETE RESTRICT
                 );
                 PRAGMA user_version = 17;
                 COMMIT;",
            )?;
        }
        if version <= 17 {
            // D-021: an SMB share connection keeps its address in the
            // Registry. The login and the password stay in the Keychain.
            db.execute_batch(
                "BEGIN IMMEDIATE;
                 ALTER TABLE storage_connections ADD COLUMN endpoint_url TEXT;
                 PRAGMA user_version = 18;
                 COMMIT;",
            )?;
        }
        if version <= 18 {
            // D-024: a batch can pause, and a child keeps its retry state.
            // A child that waits for a retry stays 'pending' with a time.
            db.execute_batch(
                "BEGIN IMMEDIATE;
                 ALTER TABLE scenario_create_batches
                   ADD COLUMN paused INTEGER NOT NULL DEFAULT 0 CHECK(paused IN (0,1));
                 ALTER TABLE scenario_create_batch_children
                   ADD COLUMN retries INTEGER NOT NULL DEFAULT 0 CHECK(retries >= 0);
                 ALTER TABLE scenario_create_batch_children ADD COLUMN next_retry_at INTEGER;
                 ALTER TABLE scenario_create_batch_children ADD COLUMN last_error TEXT;
                 PRAGMA user_version = 19;
                 COMMIT;",
            )?;
        }
        if version <= 19 {
            // Settings that the app and the Worker share, for example the
            // retry mode of D-024.
            db.execute_batch(
                "BEGIN IMMEDIATE;
                 CREATE TABLE app_settings (
                    key TEXT PRIMARY KEY NOT NULL,
                    value TEXT NOT NULL
                 );
                 PRAGMA user_version = 20;
                 COMMIT;",
            )?;
        }
        if version <= 20 {
            // D-024 scale rules: the run thread and the progress view read
            // batch children through indexes, never by a full scan.
            db.execute_batch(
                "BEGIN IMMEDIATE;
                 CREATE INDEX batch_children_due
                   ON scenario_create_batch_children(state, next_retry_at);
                 CREATE INDEX batch_children_by_batch
                   ON scenario_create_batch_children(batch_id, state);
                 PRAGMA user_version = 21;
                 COMMIT;",
            )?;
        }
        if version <= 21 {
            // The retry-until-success mode of D-024 is a Scenario parameter.
            // It is not part of a revision: it changes no file scope, and a
            // change must not stop a running batch. The previous global value
            // moves to every existing Scenario.
            db.execute_batch(
                "BEGIN IMMEDIATE;
                 ALTER TABLE scenarios ADD COLUMN retry_until_success INTEGER NOT NULL DEFAULT 1
                   CHECK(retry_until_success IN (0,1));
                 UPDATE scenarios SET retry_until_success=0
                   WHERE EXISTS(SELECT 1 FROM app_settings
                                WHERE key='retry_until_success' AND value='0');
                 DELETE FROM app_settings WHERE key='retry_until_success';
                 PRAGMA user_version = 22;
                 COMMIT;",
            )?;
        }
        if version <= 22 {
            // D-025: one durable Scenario Run for a manual or a scheduled
            // execution. The plan lives in the snapshot store, so `plan_id`
            // has no foreign key. A run is active until `finished_at`.
            db.execute_batch(
                "BEGIN IMMEDIATE;
                 CREATE TABLE scenario_runs (
                    id TEXT PRIMARY KEY NOT NULL,
                    scenario_id TEXT NOT NULL,
                    scenario_revision INTEGER NOT NULL CHECK(scenario_revision > 0),
                    trigger TEXT NOT NULL CHECK(trigger IN ('manual','schedule')),
                    schedule_job_id TEXT UNIQUE,
                    plan_id TEXT,
                    batch_id TEXT REFERENCES scenario_create_batches(id) ON DELETE RESTRICT,
                    state TEXT NOT NULL
                      CHECK(state IN ('planning','executing','completed','failed','cancelled')),
                    waiting_reason TEXT,
                    error TEXT,
                    deferred_items INTEGER NOT NULL DEFAULT 0 CHECK(deferred_items >= 0),
                    created_at INTEGER NOT NULL,
                    finished_at INTEGER,
                    CHECK((trigger='schedule') = (schedule_job_id IS NOT NULL)),
                    FOREIGN KEY(scenario_id,scenario_revision)
                      REFERENCES scenario_revisions(scenario_id,revision) ON DELETE RESTRICT
                 );
                 CREATE INDEX scenario_runs_active
                   ON scenario_runs(state) WHERE state IN ('planning','executing');
                 CREATE INDEX scenario_runs_by_scenario
                   ON scenario_runs(scenario_id, created_at);
                 PRAGMA user_version = 23;
                 COMMIT;",
            )?;
        }
        if version <= 23 {
            // D-025 section 2: at most one active run for each Scenario, as a
            // constraint. An older duplicate active run cannot exist after
            // this migration; only the oldest one stays active. A slot that
            // arrives during an active run records that run and starts none.
            db.execute_batch(
                "BEGIN IMMEDIATE;
                 UPDATE scenario_runs
                   SET state='cancelled', error='duplicate_active_run', waiting_reason=NULL,
                       finished_at=CAST(strftime('%s','now') AS INTEGER)
                   WHERE state IN ('planning','executing')
                     AND rowid <> (SELECT min(r.rowid) FROM scenario_runs r
                                   WHERE r.scenario_id=scenario_runs.scenario_id
                                     AND r.state IN ('planning','executing'));
                 DROP INDEX scenario_runs_active;
                 CREATE UNIQUE INDEX scenario_runs_one_active
                   ON scenario_runs(scenario_id) WHERE state IN ('planning','executing');
                 ALTER TABLE schedule_occurrences ADD COLUMN skipped_active_run_id TEXT;
                 PRAGMA user_version = 24;
                 COMMIT;",
            )?;
        }
        if version <= 24 {
            // D-025 section 6: a batch child is a Create, an Update, or a
            // Delete. One-way Updates and Deletes need the one-time
            // confirmation of the "source always wins" rule (section 4).
            db.execute_batch(
                "BEGIN IMMEDIATE;
                 ALTER TABLE scenario_create_batch_children
                   ADD COLUMN kind TEXT NOT NULL DEFAULT 'create'
                   CHECK(kind IN ('create','update','delete'));
                 CREATE TABLE scenario_source_wins_consents (
                    scenario_id TEXT NOT NULL,
                    revision INTEGER NOT NULL CHECK(revision > 0),
                    confirmed_at INTEGER NOT NULL,
                    PRIMARY KEY(scenario_id, revision),
                    FOREIGN KEY(scenario_id,revision)
                      REFERENCES scenario_revisions(scenario_id,revision) ON DELETE CASCADE
                 );
                 PRAGMA user_version = 25;
                 COMMIT;",
            )?;
        }
        if version <= 25 {
            // Issue #34: selected days at a time (daily is all days) and every
            // N hours. A weekly row becomes one selected day. Its slot keys do
            // not change, so no slot repeats after the migration. `weekday`
            // stays for older readers and holds the first selected day.
            db.execute_batch(
                "BEGIN IMMEDIATE;
                 ALTER TABLE scenario_schedules
                   ADD COLUMN kind TEXT NOT NULL DEFAULT 'days_at_time'
                   CHECK(kind IN ('days_at_time','every_hours'));
                 ALTER TABLE scenario_schedules
                   ADD COLUMN days_mask INTEGER NOT NULL DEFAULT 127
                   CHECK(days_mask BETWEEN 1 AND 127);
                 ALTER TABLE scenario_schedules
                   ADD COLUMN interval_hours INTEGER CHECK(interval_hours BETWEEN 1 AND 24);
                 ALTER TABLE scenario_schedules ADD COLUMN anchor_utc TEXT;
                 UPDATE scenario_schedules SET days_mask = 1 << weekday;
                 PRAGMA user_version = 26;
                 COMMIT;",
            )?;
        }
        if version <= 26 {
            // D-025 section 11: the write-root lock. Only an active run can
            // hold it, so the end of a run releases it. A run that executes
            // now already writes and keeps its lock.
            db.execute_batch(
                "BEGIN IMMEDIATE;
                 ALTER TABLE scenario_runs
                   ADD COLUMN write_lock INTEGER NOT NULL DEFAULT 0 CHECK(write_lock IN (0,1));
                 ALTER TABLE scenario_runs ADD COLUMN waiting_for_scenario_id TEXT;
                 UPDATE scenario_runs SET write_lock=1 WHERE state='executing';
                 PRAGMA user_version = 27;
                 COMMIT;",
            )?;
        }
        if version <= 27 {
            // D-027 section 7a: the SFTP port and the confirmed host key of an
            // ISPmanager connection. A fingerprint is not a secret.
            db.execute_batch(
                "BEGIN IMMEDIATE;
                 CREATE TABLE ispmanager_settings (
                     connection_id TEXT PRIMARY KEY NOT NULL
                         REFERENCES storage_connections(id) ON DELETE CASCADE,
                     ssh_port INTEGER NOT NULL DEFAULT 22 CHECK(ssh_port BETWEEN 1 AND 65535),
                     host_key_sha256 TEXT
                 );
                 PRAGMA user_version = 28;
                 COMMIT;",
            )?;
        }
        if version <= 28 {
            // D-027: the last connection check survives a restart. The report
            // has result classes only, never a password or a session ID.
            db.execute_batch(
                "BEGIN IMMEDIATE;
                 ALTER TABLE ispmanager_settings ADD COLUMN last_check_json TEXT;
                 ALTER TABLE ispmanager_settings ADD COLUMN last_check_at INTEGER;
                 PRAGMA user_version = 29;
                 COMMIT;",
            )?;
        }
        if version <= 29 {
            // D-027 sections 5 and 6: the Backup/export Scenario mode and its
            // typed selection. A resource is a provider object, not a path
            // relative to a root. An empty selection means nothing.
            db.execute_batch(
                "BEGIN IMMEDIATE;
                 ALTER TABLE scenarios ADD COLUMN mode TEXT NOT NULL DEFAULT 'sync'
                   CHECK(mode IN ('sync','backup_export'));
                 ALTER TABLE scenario_revisions ADD COLUMN mode TEXT NOT NULL DEFAULT 'sync'
                   CHECK(mode IN ('sync','backup_export'));
                 CREATE TABLE scenario_revision_resources (
                    scenario_id TEXT NOT NULL,
                    revision INTEGER NOT NULL CHECK(revision > 0),
                    category TEXT NOT NULL,
                    provider_id TEXT NOT NULL,
                    kind TEXT NOT NULL CHECK(kind IN ('file','dir','symlink','object')),
                    PRIMARY KEY(scenario_id,revision,category,provider_id),
                    FOREIGN KEY(scenario_id,revision)
                      REFERENCES scenario_revisions(scenario_id,revision) ON DELETE RESTRICT
                 );
                 PRAGMA user_version = 30;
                 COMMIT;",
            )?;
        }
        if version <= 30 {
            // D-027 section 6: a rule can leave out a folder or a file inside
            // a selected folder.
            db.execute_batch(
                "BEGIN IMMEDIATE;
                 ALTER TABLE scenario_revision_resources ADD COLUMN rule TEXT NOT NULL
                   DEFAULT 'include' CHECK(rule IN ('include','exclude'));
                 PRAGMA user_version = 31;
                 COMMIT;",
            )?;
        }
        if version <= 31 {
            // D-027 section 7b: the result of each selected database in one
            // Backup/export run. It holds no access facts.
            db.execute_batch(
                "BEGIN IMMEDIATE;
                 CREATE TABLE backup_database_results (
                    run_id TEXT NOT NULL,
                    provider_id TEXT NOT NULL,
                    result TEXT NOT NULL
                      CHECK(result IN ('created','updated','unchanged','failed','cancelled')),
                    error TEXT,
                    bytes INTEGER,
                    md5 TEXT,
                    finished_at INTEGER NOT NULL,
                    PRIMARY KEY(run_id, provider_id)
                 );
                 PRAGMA user_version = 32;
                 COMMIT;",
            )?;
        }
        if version <= 32 {
            // D-027 section 5.6b: a Backup/export mirror deletes a whole
            // destination folder as one batch child. SQLite cannot change a
            // CHECK, so the table is copied once.
            db.execute_batch(
                "BEGIN IMMEDIATE;
                 CREATE TABLE scenario_create_batch_children_v33 (
                    batch_id TEXT NOT NULL REFERENCES scenario_create_batches(id) ON DELETE RESTRICT,
                    relative_path TEXT NOT NULL,
                    source_is_left INTEGER NOT NULL CHECK(source_is_left IN (0,1)),
                    job_id TEXT NOT NULL UNIQUE,
                    state TEXT NOT NULL DEFAULT 'pending'
                      CHECK(state IN ('pending','completed','failed','cancelled','unknown')),
                    retries INTEGER NOT NULL DEFAULT 0 CHECK(retries >= 0),
                    next_retry_at INTEGER,
                    last_error TEXT,
                    kind TEXT NOT NULL DEFAULT 'create'
                      CHECK(kind IN ('create','update','delete','delete_folder')),
                    PRIMARY KEY(batch_id,relative_path)
                 );
                 INSERT INTO scenario_create_batch_children_v33(
                    batch_id,relative_path,source_is_left,job_id,state,
                    retries,next_retry_at,last_error,kind)
                 SELECT batch_id,relative_path,source_is_left,job_id,state,
                    retries,next_retry_at,last_error,kind
                 FROM scenario_create_batch_children;
                 DROP TABLE scenario_create_batch_children;
                 ALTER TABLE scenario_create_batch_children_v33
                   RENAME TO scenario_create_batch_children;
                 CREATE INDEX batch_children_due
                   ON scenario_create_batch_children(state, next_retry_at);
                 CREATE INDEX batch_children_by_batch
                   ON scenario_create_batch_children(batch_id, state);
                 PRAGMA user_version = 33;
                 COMMIT;",
            )?;
        }
        if version <= 33 {
            // D-027 section 7b.10: a table of a selected database can keep
            // only its structure. SQLite cannot change a CHECK, so the table
            // is copied once.
            db.execute_batch(
                "BEGIN IMMEDIATE;
                 CREATE TABLE scenario_revision_resources_v34 (
                    scenario_id TEXT NOT NULL,
                    revision INTEGER NOT NULL CHECK(revision > 0),
                    category TEXT NOT NULL,
                    provider_id TEXT NOT NULL,
                    kind TEXT NOT NULL CHECK(kind IN ('file','dir','symlink','object')),
                    rule TEXT NOT NULL DEFAULT 'include'
                      CHECK(rule IN ('include','exclude','structure_only')),
                    PRIMARY KEY(scenario_id,revision,category,provider_id),
                    FOREIGN KEY(scenario_id,revision)
                      REFERENCES scenario_revisions(scenario_id,revision) ON DELETE RESTRICT
                 );
                 INSERT INTO scenario_revision_resources_v34(
                    scenario_id,revision,category,provider_id,kind,rule)
                 SELECT scenario_id,revision,category,provider_id,kind,rule
                 FROM scenario_revision_resources;
                 DROP TABLE scenario_revision_resources;
                 ALTER TABLE scenario_revision_resources_v34
                   RENAME TO scenario_revision_resources;
                 PRAGMA user_version = 34;
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

    /// Return the "This Mac" connection, and create it when it is missing.
    /// The product has exactly one Local Filesystem connection per Mac
    /// (D-020). The check and the insert are one immediate transaction, so a
    /// Worker and the interface cannot create two. When older data already
    /// has more than one, the oldest record is returned and nothing changes.
    pub fn ensure_local_computer(&self, name: &str) -> Result<String> {
        if !valid_name(name) {
            return Err(RegistryError::InvalidName);
        }
        let transaction = rusqlite::Transaction::new_unchecked(
            &self.db,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let existing: Option<String> = transaction
            .query_row(
                "SELECT id FROM storage_connections WHERE adapter='local_filesystem'
                 ORDER BY rowid LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        let id = match existing {
            Some(id) => id,
            None => {
                let id: String =
                    transaction
                        .query_row("SELECT lower(hex(randomblob(16)))", [], |row| row.get(0))?;
                transaction.execute(
                    "INSERT INTO storage_connections(id,adapter,name) VALUES (?1,'local_filesystem',?2)",
                    params![id, name.trim()],
                )?;
                id
            }
        };
        transaction.commit()?;
        Ok(id)
    }

    pub fn list(&self) -> Result<Vec<StorageConnection>> {
        let mut stmt = self.db.prepare(
            "SELECT id,adapter,name,enabled,status,credential_ref,last_checked_at,diagnosis,provider_account_id,endpoint_url
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
                "SELECT id,adapter,name,enabled,status,credential_ref,last_checked_at,diagnosis,provider_account_id,endpoint_url
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
            endpoint_url: row.get(9)?,
        })
    }

    /// Add one SMB share connection (D-021). The address is normalized and
    /// must not carry credentials. Any number of independent SMB connections
    /// can exist, also to the same share (D-016).
    pub fn add_smb_share(&self, name: &str, url: &str) -> Result<String> {
        if !valid_name(name) {
            return Err(RegistryError::InvalidName);
        }
        let url = normalized_smb_url(url).ok_or(RegistryError::InvalidEndpoint)?;
        let id: String = self
            .db
            .query_row("SELECT lower(hex(randomblob(16)))", [], |row| row.get(0))?;
        self.db.execute(
            "INSERT INTO storage_connections(id,adapter,name,endpoint_url)
             VALUES (?1,'smb_share',?2,?3)",
            params![id, name.trim(), url],
        )?;
        Ok(id)
    }

    /// Change the address of an SMB share connection. Other adapters have
    /// no address and stay unchanged.
    pub fn set_smb_endpoint(&self, id: &str, url: &str) -> Result<()> {
        let url = normalized_smb_url(url).ok_or(RegistryError::InvalidEndpoint)?;
        self.changed(self.db.execute(
            "UPDATE storage_connections SET endpoint_url=?1 WHERE id=?2 AND adapter='smb_share'",
            params![url, id],
        )?)
    }

    /// Add one ISPmanager panel connection (D-027). The panel URL is
    /// validated and has no credentials. The login and the password stay in
    /// the platform credential store. Connections are never deduplicated.
    pub fn add_ispmanager(&self, name: &str, url: &str) -> Result<String> {
        if !valid_name(name) {
            return Err(RegistryError::InvalidName);
        }
        let url = normalized_panel_url(url)?;
        let id: String = self
            .db
            .query_row("SELECT lower(hex(randomblob(16)))", [], |row| row.get(0))?;
        self.db.execute(
            "INSERT INTO storage_connections(id,adapter,name,endpoint_url)
             VALUES (?1,'ispmanager',?2,?3)",
            params![id, name.trim(), url],
        )?;
        Ok(id)
    }

    /// Change the panel URL of an ISPmanager connection. Another URL can be
    /// another server, so the confirmed host key is cleared.
    pub fn set_ispmanager_endpoint(&self, id: &str, url: &str) -> Result<()> {
        let url = normalized_panel_url(url)?;
        let current = self.get(id)?.ok_or(RegistryError::NotFound)?;
        if current.adapter != crate::storage_access::ISPMANAGER {
            return Err(RegistryError::NotFound);
        }
        if current.endpoint_url.as_deref() == Some(url.as_str()) {
            return Ok(());
        }
        self.db.execute(
            "UPDATE storage_connections SET endpoint_url=?1 WHERE id=?2",
            params![url, id],
        )?;
        self.db.execute(
            "UPDATE ispmanager_settings SET host_key_sha256=NULL WHERE connection_id=?1",
            [id],
        )?;
        Ok(())
    }

    /// The SFTP port and the confirmed host key fingerprint (D-027 section
    /// 7a). A connection without a settings row uses port 22 and has no key.
    pub fn ispmanager_settings(&self, id: &str) -> Result<IspmanagerSettings> {
        let row = self
            .db
            .query_row(
                "SELECT ssh_port,host_key_sha256,last_check_json,last_check_at
                 FROM ispmanager_settings WHERE connection_id=?1",
                [id],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<i64>>(3)?,
                    ))
                },
            )
            .optional()?;
        Ok(match row {
            Some((port, host_key_sha256, last_check, last_check_at)) => IspmanagerSettings {
                ssh_port: u16::try_from(port).unwrap_or(crate::ispmanager_sftp::DEFAULT_PORT),
                host_key_sha256,
                last_check: last_check.and_then(|text| serde_json::from_str(&text).ok()),
                last_check_at,
            },
            None => IspmanagerSettings {
                ssh_port: crate::ispmanager_sftp::DEFAULT_PORT,
                host_key_sha256: None,
                last_check: None,
                last_check_at: None,
            },
        })
    }

    /// Another port can be another SSH server, so the confirmed host key is
    /// cleared when the port changes.
    pub fn set_ispmanager_ssh_port(&self, id: &str, port: u16) -> Result<()> {
        if port == 0 {
            return Err(RegistryError::InvalidEndpoint);
        }
        self.require_ispmanager(id)?;
        self.db.execute(
            "INSERT INTO ispmanager_settings(connection_id,ssh_port) VALUES (?1,?2)
             ON CONFLICT(connection_id) DO UPDATE SET
                 host_key_sha256=CASE WHEN ssh_port=excluded.ssh_port THEN host_key_sha256 END,
                 ssh_port=excluded.ssh_port",
            params![id, port],
        )?;
        Ok(())
    }

    /// Stores the host key fingerprint that the user confirmed.
    pub fn confirm_ispmanager_host_key(&self, id: &str, fingerprint: &str) -> Result<()> {
        let valid = fingerprint.starts_with("SHA256:")
            && fingerprint.len() <= 128
            && fingerprint.bytes().all(|byte| byte.is_ascii_graphic());
        if !valid {
            return Err(RegistryError::InvalidEndpoint);
        }
        self.require_ispmanager(id)?;
        self.db.execute(
            "INSERT INTO ispmanager_settings(connection_id,host_key_sha256) VALUES (?1,?2)
             ON CONFLICT(connection_id) DO UPDATE SET host_key_sha256=excluded.host_key_sha256",
            params![id, fingerprint],
        )?;
        Ok(())
    }

    /// Stores the report of the last connection check. The report must be a
    /// JSON object from `ispmanager::check`.
    pub fn record_ispmanager_check(&self, id: &str, report: &str, at: i64) -> Result<()> {
        let parsed: serde_json::Value =
            serde_json::from_str(report).map_err(|_| RegistryError::InvalidEndpoint)?;
        if !parsed.is_object() || report.len() > 256 * 1024 {
            return Err(RegistryError::InvalidEndpoint);
        }
        self.require_ispmanager(id)?;
        self.db.execute(
            "INSERT INTO ispmanager_settings(connection_id,last_check_json,last_check_at)
             VALUES (?1,?2,?3)
             ON CONFLICT(connection_id) DO UPDATE SET
                 last_check_json=excluded.last_check_json,last_check_at=excluded.last_check_at",
            params![id, report, at],
        )?;
        Ok(())
    }

    fn require_ispmanager(&self, id: &str) -> Result<()> {
        match self.get(id)? {
            Some(entry) if entry.adapter == crate::storage_access::ISPMANAGER => Ok(()),
            _ => Err(RegistryError::NotFound),
        }
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
        // SMB roots are paths inside a QADREN mount on this Mac (D-022), so
        // they compare as paths too.
        let on_this_mac = |adapter: Option<String>| {
            matches!(
                adapter.as_deref(),
                Some("local_filesystem") | Some("smb_share")
            )
        };
        Ok(on_this_mac(adapter(left_connection_id)?)
            && on_this_mac(adapter(right_connection_id)?)
            && scenario_roots_overlap(left_root, right_root))
    }

    /// Refuse a Yandex Disk endpoint that is not the source of a one-way
    /// Scenario to this Mac (D-023, stage 1), and every ISPmanager endpoint
    /// (D-027).
    fn require_yandex_scope(
        &self,
        left_connection_id: &str,
        right_connection_id: &str,
        direction: &str,
    ) -> Result<()> {
        let adapter = |id: &str| -> Result<String> {
            Ok(self
                .db
                .query_row(
                    "SELECT adapter FROM storage_connections WHERE id=?1",
                    [id],
                    |row| row.get(0),
                )
                .optional()?
                .unwrap_or_default())
        };
        let (left, right) = (adapter(left_connection_id)?, adapter(right_connection_id)?);
        // The Backup/export Scenario mode does not exist yet (D-027 section 5).
        if left == crate::storage_access::ISPMANAGER || right == crate::storage_access::ISPMANAGER {
            return Err(RegistryError::UnsupportedScenarioEndpoint);
        }
        if crate::storage_access::yandex_scope_allowed(&left, &right, direction) {
            Ok(())
        } else {
            Err(RegistryError::UnsupportedYandexScope)
        }
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
        self.create_scenario_with_scope(
            name,
            left_connection_id,
            left_root,
            right_connection_id,
            right_root,
            direction,
            &ScenarioScope::default(),
        )
    }

    /// Create a Scenario whose first revision has the given selection and
    /// deletion rule. The selection is normalized before it is stored.
    #[allow(clippy::too_many_arguments)]
    pub fn create_scenario_with_scope(
        &self,
        name: &str,
        left_connection_id: &str,
        left_root: &str,
        right_connection_id: &str,
        right_root: &str,
        direction: &str,
        scope: &ScenarioScope,
    ) -> Result<String> {
        let selection = ScenarioScope::normalized_selection(&scope.selected_paths)
            .map_err(|_| RegistryError::InvalidSelection)?;
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
        self.require_yandex_scope(left_connection_id, right_connection_id, direction)?;
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
        let transaction = rusqlite::Transaction::new_unchecked(
            &self.db,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        transaction.execute(
            "INSERT INTO scenarios(id,name,left_connection_id,left_root,right_connection_id,right_root,direction,deletion_mode) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
            params![id, name.trim(), left_connection_id, left_root, right_connection_id, right_root, direction, scope.deletion_mode.as_str()],
        )?;
        transaction.execute(
            "INSERT INTO scenario_revisions(
                scenario_id,revision,name,left_connection_id,left_root,
                right_connection_id,right_root,direction,deletion_mode
             ) VALUES(?1,1,?2,?3,?4,?5,?6,?7,?8)",
            params![
                id,
                name.trim(),
                left_connection_id,
                left_root,
                right_connection_id,
                right_root,
                direction,
                scope.deletion_mode.as_str()
            ],
        )?;
        insert_revision_selection(&transaction, &id, 1, &selection)?;
        transaction.commit()?;
        Ok(id)
    }

    pub fn set_scenario_enabled(&self, id: &str, enabled: bool) -> Result<()> {
        let transaction = rusqlite::Transaction::new_unchecked(
            &self.db,
            rusqlite::TransactionBehavior::Immediate,
        )?;
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

    /// One weekly slot: a days-at-time schedule with one selected day.
    pub fn create_scenario_schedule(
        &self,
        scenario_id: &str,
        timezone: &str,
        weekday: u8,
        hour: u8,
        minute: u8,
    ) -> Result<String> {
        if weekday > 6 {
            return Err(RegistryError::InvalidSchedule);
        }
        self.create_scenario_days_schedule(scenario_id, timezone, 1 << weekday, hour, minute)
    }

    /// Selected local days (Monday is bit 0) at one local time.
    pub fn create_scenario_days_schedule(
        &self,
        scenario_id: &str,
        timezone: &str,
        days_mask: u8,
        hour: u8,
        minute: u8,
    ) -> Result<String> {
        self.insert_schedule(ScenarioSchedule {
            id: String::new(),
            scenario_id: scenario_id.to_owned(),
            timezone: timezone.to_owned(),
            kind: ScheduleKind::DaysAtTime,
            days_mask,
            hour,
            minute,
            enabled: true,
        })
    }

    /// Every `interval_hours` hours from the local time `hour:minute` of
    /// the current day in `timezone`, on the selected local days.
    pub fn create_scenario_hours_schedule(
        &self,
        scenario_id: &str,
        timezone: &str,
        interval_hours: u8,
        hour: u8,
        minute: u8,
        days_mask: u8,
    ) -> Result<String> {
        self.create_scenario_hours_schedule_at(
            scenario_id,
            timezone,
            interval_hours,
            hour,
            minute,
            days_mask,
            DateTime::<Utc>::from(std::time::SystemTime::now()),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn create_scenario_hours_schedule_at(
        &self,
        scenario_id: &str,
        timezone: &str,
        interval_hours: u8,
        hour: u8,
        minute: u8,
        days_mask: u8,
        now: DateTime<Utc>,
    ) -> Result<String> {
        let zone = Tz::from_str(timezone).map_err(|_| RegistryError::InvalidSchedule)?;
        let anchor = every_hours_anchor(
            zone,
            now.with_timezone(&zone).date_naive(),
            hour.into(),
            minute.into(),
        )
        .map_err(|_| RegistryError::InvalidSchedule)?;
        self.insert_schedule(ScenarioSchedule {
            id: String::new(),
            scenario_id: scenario_id.to_owned(),
            timezone: timezone.to_owned(),
            kind: ScheduleKind::EveryHours {
                interval_hours,
                anchor,
            },
            days_mask,
            hour,
            minute,
            enabled: true,
        })
    }

    fn insert_schedule(&self, schedule: ScenarioSchedule) -> Result<String> {
        schedule.schedule()?;
        let (kind, interval_hours, anchor) = match schedule.kind {
            ScheduleKind::DaysAtTime => ("days_at_time", None, None),
            ScheduleKind::EveryHours {
                interval_hours,
                anchor,
            } => (
                "every_hours",
                Some(interval_hours),
                Some(anchor.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)),
            ),
        };
        let first_day = schedule.days_mask.trailing_zeros();
        let id: String = self
            .db
            .query_row("SELECT lower(hex(randomblob(16)))", [], |row| row.get(0))?;
        let changed = self.db.execute(
            "INSERT INTO scenario_schedules(
                id,scenario_id,timezone,weekday,hour,minute,kind,days_mask,interval_hours,anchor_utc
             ) SELECT ?1,id,?2,?3,?4,?5,?6,?7,?8,?9 FROM scenarios WHERE id=?10",
            params![
                id,
                schedule.timezone,
                first_day,
                schedule.hour,
                schedule.minute,
                kind,
                schedule.days_mask,
                interval_hours,
                anchor,
                schedule.scenario_id
            ],
        )?;
        if changed == 0 {
            return Err(RegistryError::NotFound);
        }
        Ok(id)
    }

    pub fn list_scenario_schedules(&self, scenario_id: &str) -> Result<Vec<ScenarioSchedule>> {
        self.read_schedules(
            "WHERE ss.scenario_id=?1 ORDER BY ss.kind,ss.hour,ss.minute,ss.id",
            Some(scenario_id),
        )
    }

    fn read_schedules(
        &self,
        filter: &str,
        scenario_id: Option<&str>,
    ) -> Result<Vec<ScenarioSchedule>> {
        let mut statement = self.db.prepare(&format!(
            "SELECT ss.id,ss.scenario_id,ss.timezone,ss.kind,ss.days_mask,ss.interval_hours,
                    ss.anchor_utc,ss.hour,ss.minute,ss.enabled
             FROM scenario_schedules ss {filter}"
        ))?;
        let read = |row: &rusqlite::Row<'_>| {
            let kind = match row.get::<_, String>(3)?.as_str() {
                "days_at_time" => ScheduleKind::DaysAtTime,
                "every_hours" => {
                    let anchor: String = row.get(6)?;
                    ScheduleKind::EveryHours {
                        interval_hours: row.get(5)?,
                        anchor: DateTime::parse_from_rfc3339(&anchor)
                            .map_err(|_| rusqlite::Error::InvalidQuery)?
                            .with_timezone(&Utc),
                    }
                }
                _ => return Err(rusqlite::Error::InvalidQuery),
            };
            Ok(ScenarioSchedule {
                id: row.get(0)?,
                scenario_id: row.get(1)?,
                timezone: row.get(2)?,
                kind,
                days_mask: row.get(4)?,
                hour: row.get(7)?,
                minute: row.get(8)?,
                enabled: row.get(9)?,
            })
        };
        let schedules = match scenario_id {
            Some(scenario_id) => statement.query_map([scenario_id], read)?,
            None => statement.query_map([], read)?,
        }
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
        let schedules = self.read_schedules(
            "JOIN scenarios s ON s.id=ss.scenario_id
             LEFT JOIN scenario_automation a ON a.scenario_id=s.id
             WHERE ss.enabled=1 AND s.enabled=1 AND COALESCE(a.locally_paused,0)=0
             ORDER BY ss.scenario_id,ss.id",
            None,
        )?;
        let mut due = Vec::new();
        for record in schedules {
            for occurrence in record
                .schedule()?
                .occurrences_between(after, through)
                .map_err(|_| RegistryError::InvalidSchedule)?
            {
                due.push(DueScheduleOccurrence {
                    scenario_id: record.scenario_id.clone(),
                    schedule_id: record.id.clone(),
                    occurrence_key: occurrence.key,
                    due_at: occurrence.at,
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
        let transaction = rusqlite::Transaction::new_unchecked(
            &self.db,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let mut claimed_by_scenario: Vec<DueScheduleOccurrence> = Vec::new();
        let mut claimed: Vec<DueScheduleOccurrence> = Vec::new();
        for occurrence in due {
            let inserted = transaction.execute(
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
            if inserted == 0 {
                continue;
            }
            claimed.push(occurrence.clone());
            // `due` is sorted by Scenario, so the last claimed slot of each
            // Scenario replaces the earlier ones of that pass.
            match claimed_by_scenario.last_mut() {
                Some(last) if last.scenario_id == occurrence.scenario_id => *last = occurrence,
                _ => claimed_by_scenario.push(occurrence),
            }
        }
        let mut staged = 0;
        for occurrence in claimed_by_scenario {
            // D-025 section 2: a slot during an active run of its Scenario
            // starts nothing and makes no conflict. The active run continues
            // and no follow-up run is recorded.
            let active: Option<String> = transaction
                .query_row(
                    "SELECT id FROM scenario_runs WHERE scenario_id=?1
                       AND state IN ('planning','executing')",
                    [&occurrence.scenario_id],
                    |row| row.get(0),
                )
                .optional()?;
            if let Some(active) = active {
                for slot in claimed
                    .iter()
                    .filter(|slot| slot.scenario_id == occurrence.scenario_id)
                {
                    transaction.execute(
                        "UPDATE schedule_occurrences SET skipped_active_run_id=?3
                         WHERE schedule_id=?1 AND occurrence_key=?2",
                        params![slot.schedule_id, slot.occurrence_key.0, active],
                    )?;
                }
                continue;
            }
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
    /// Start an empty, paused Create batch for a large plan (D-024). The
    /// caller adds the children page by page and then resumes the batch, so
    /// no list of all paths is held in memory.
    pub fn begin_scenario_create_batch(&mut self, scenario: &ScenarioReference) -> Result<String> {
        if !self.is_current_scenario_revision(&scenario.scenario_id, scenario.revision)? {
            return Err(RegistryError::StaleScenarioRevision);
        }
        let id: String = self
            .db
            .query_row("SELECT lower(hex(randomblob(16)))", [], |row| row.get(0))?;
        self.db.execute(
            "INSERT INTO scenario_create_batches(id,scenario_id,scenario_revision,paused) VALUES(?1,?2,?3,1)",
            params![id, scenario.scenario_id, scenario.revision as i64],
        )?;
        Ok(id)
    }

    /// Add one page of children to a batch from `begin_scenario_create_batch`.
    /// An invalid or repeated path is skipped. Returns the number added.
    pub fn append_scenario_create_batch_children(
        &mut self,
        batch_id: &str,
        children: &[(String, bool)],
    ) -> Result<usize> {
        let children = children
            .iter()
            .map(|(path, source_is_left)| (path.clone(), *source_is_left, BatchChildKind::Create))
            .collect::<Vec<_>>();
        self.append_scenario_batch_children(batch_id, &children)
    }

    /// Add Create, Update, and Delete children to a batch (D-025 section
    /// 6). For a Delete, `source_is_left` names the side whose object is
    /// absent; the effect acts on the other side.
    pub fn append_scenario_batch_children(
        &mut self,
        batch_id: &str,
        children: &[(String, bool, BatchChildKind)],
    ) -> Result<usize> {
        let transaction = self.db.transaction()?;
        let mut added = 0;
        {
            let mut insert = transaction.prepare_cached(
                "INSERT OR IGNORE INTO scenario_create_batch_children(
                    batch_id,relative_path,source_is_left,kind,job_id
                 ) VALUES(?1,?2,?3,?4,'scenario-batch-copy-' || lower(hex(randomblob(16))))",
            )?;
            for (path, source_is_left, kind) in children {
                if valid_sync_path(path) {
                    added +=
                        insert.execute(params![batch_id, path, source_is_left, kind.as_str()])?;
                }
            }
        }
        transaction.commit()?;
        Ok(added)
    }

    /// Record the one-time confirmation of the one-way rule "the source
    /// always wins" for a revision (D-025 section 4).
    pub fn confirm_scenario_source_wins(&self, scenario_id: &str, revision: u64) -> Result<()> {
        let scenario = self
            .get_scenario_revision(scenario_id, revision)?
            .ok_or(RegistryError::NotFound)?;
        if scenario.direction == "bidirectional" {
            return Err(RegistryError::InvalidSyncState);
        }
        self.db.execute(
            "INSERT OR IGNORE INTO scenario_source_wins_consents(scenario_id,revision,confirmed_at)
             VALUES(?1,?2,?3)",
            params![scenario_id, revision as i64, unix_seconds()],
        )?;
        Ok(())
    }

    /// True when the user confirmed "the source always wins" for this
    /// revision, or for an earlier revision with the same direction,
    /// destination, and selection. A change of these asks again.
    pub fn scenario_source_wins_confirmed(&self, scenario_id: &str, revision: u64) -> Result<bool> {
        let confirmed: Option<i64> = self
            .db
            .query_row(
                "SELECT max(revision) FROM scenario_source_wins_consents
                 WHERE scenario_id=?1 AND revision<=?2",
                params![scenario_id, revision as i64],
                |row| row.get(0),
            )
            .optional()?
            .flatten();
        let Some(confirmed) = confirmed else {
            return Ok(false);
        };
        if confirmed as u64 == revision {
            return Ok(true);
        }
        // (direction, destination connection, destination root, selection)
        type Destination = (String, String, String, Vec<String>);
        let destination = |revision: u64| -> Result<Option<Destination>> {
            let Some(scenario) = self.get_scenario_revision(scenario_id, revision)? else {
                return Ok(None);
            };
            let (connection, root) = match scenario.direction.as_str() {
                "left_to_right" => (scenario.right_connection_id, scenario.right_root),
                "right_to_left" => (scenario.left_connection_id, scenario.left_root),
                _ => return Ok(None),
            };
            let scope = self.get_scenario_revision_scope(scenario_id, revision)?;
            Ok(Some((
                scenario.direction,
                connection,
                root,
                scope.selected_paths,
            )))
        };
        let current = destination(revision)?;
        Ok(current.is_some() && current == destination(confirmed as u64)?)
    }

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
                retries: 0,
                next_retry_at: None,
                last_error: None,
            });
        }
        transaction.commit()?;
        Ok(ScenarioCreateBatchReceipt {
            id,
            scenario: scenario.clone(),
            cancellation_requested: false,
            paused: false,
            children: receipt_children,
        })
    }

    /// Load a confirmed batch exactly as saved. This does not create Jobs or
    /// authorize an effect.
    pub fn scenario_create_batch(&self, id: &str) -> Result<Option<ScenarioCreateBatchReceipt>> {
        let parent: Option<(ScenarioReference, bool, bool)> = self
            .db
            .query_row(
            "SELECT scenario_id,scenario_revision,cancellation_requested,paused FROM scenario_create_batches WHERE id=?1",
                [id],
                |row| {
                    Ok((ScenarioReference {
                        scenario_id: row.get(0)?,
                        revision: row.get::<_, i64>(1)? as u64,
                    }, row.get::<_, bool>(2)?, row.get::<_, bool>(3)?))
                },
            )
            .optional()?;
        let Some((scenario, cancellation_requested, paused)) = parent else {
            return Ok(None);
        };
        let mut statement = self.db.prepare(
            "SELECT relative_path,source_is_left,job_id,state,retries,next_retry_at,last_error
             FROM scenario_create_batch_children WHERE batch_id=?1 ORDER BY relative_path",
        )?;
        let children = statement
            .query_map([id], |row| {
                Ok(ScenarioBatchChild {
                    relative_path: row.get(0)?,
                    source_is_left: row.get(1)?,
                    job_id: row.get(2)?,
                    state: row.get(3)?,
                    retries: row.get(4)?,
                    next_retry_at: row.get(5)?,
                    last_error: row.get(6)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(Some(ScenarioCreateBatchReceipt {
            id: id.into(),
            scenario,
            cancellation_requested,
            paused,
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

    /// Schedule an automatic retry of a child that failed before any effect
    /// (D-024). The child stays `pending` and gets a new Job ID; the failed
    /// Job stays in the JobStore as history. Only the current Job ID changes.
    pub fn schedule_scenario_create_batch_child_retry(
        &self,
        batch_id: &str,
        relative_path: &str,
        failed_job_id: &str,
        next_retry_at: i64,
        error: &str,
    ) -> Result<()> {
        let changed = self.db.execute(
            "UPDATE scenario_create_batch_children
             SET retries=retries+1, next_retry_at=?4, last_error=?5,
                 job_id='scenario-batch-copy-' || lower(hex(randomblob(16)))
             WHERE batch_id=?1 AND relative_path=?2 AND job_id=?3 AND state='pending'",
            params![batch_id, relative_path, failed_job_id, next_retry_at, error],
        )?;
        if changed == 1 {
            Ok(())
        } else {
            Err(RegistryError::NotFound)
        }
    }

    /// At most `limit` children that the Worker can start at `now`:
    /// `pending`, without a retry time in the future, in a batch that is not
    /// paused. A cancelled batch is included, so that the Worker records
    /// `cancelled`.
    /// Batches with a child that is due now, in creation order.
    pub fn due_scenario_batches(&self, now: i64) -> Result<Vec<String>> {
        let mut statement = self.db.prepare(
            "SELECT b.id FROM scenario_create_batches b
             WHERE b.paused=0 AND EXISTS (
               SELECT 1 FROM scenario_create_batch_children c
               WHERE c.batch_id=b.id AND c.state='pending'
                 AND (c.next_retry_at IS NULL OR c.next_retry_at<=?1
                      OR b.cancellation_requested=1))
             ORDER BY b.rowid",
        )?;
        let rows = statement
            .query_map([now], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// A bounded page of the due children of one batch, in row order. It
    /// reads through the `batch_children_by_batch` index.
    pub fn due_children_of_batch(
        &self,
        batch_id: &str,
        now: i64,
        limit: usize,
    ) -> Result<Vec<DueBatchChild>> {
        let mut statement = self.db.prepare_cached(
            "SELECT c.batch_id,b.scenario_id,b.scenario_revision,c.relative_path,c.source_is_left,
                    c.job_id,c.retries,b.cancellation_requested,c.kind
             FROM scenario_create_batch_children c
             JOIN scenario_create_batches b ON b.id=c.batch_id
             WHERE c.batch_id=?1 AND c.state='pending' AND b.paused=0
               AND (c.next_retry_at IS NULL OR c.next_retry_at<=?2 OR b.cancellation_requested=1)
             ORDER BY c.rowid
             LIMIT ?3",
        )?;
        let rows = statement
            .query_map(params![batch_id, now, limit as i64], |row| {
                Ok(DueBatchChild {
                    batch_id: row.get(0)?,
                    scenario: ScenarioReference {
                        scenario_id: row.get(1)?,
                        revision: row.get::<_, i64>(2)? as u64,
                    },
                    relative_path: row.get(3)?,
                    source_is_left: row.get(4)?,
                    job_id: row.get(5)?,
                    retries: row.get(6)?,
                    cancellation_requested: row.get(7)?,
                    kind: BatchChildKind::parse(&row.get::<_, String>(8)?)
                        .unwrap_or(BatchChildKind::Create),
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn due_scenario_create_batch_children(
        &self,
        now: i64,
        limit: usize,
    ) -> Result<Vec<DueBatchChild>> {
        // D-025 section 11: a bounded page from each batch in turn, so that
        // one large run does not stop the others.
        let batches = self.due_scenario_batches(now)?;
        if batches.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        let share = (limit / batches.len()).max(1);
        let mut pages = Vec::with_capacity(batches.len());
        for batch in batches.iter().take(limit) {
            pages.push(self.due_children_of_batch(batch, now, share)?.into_iter());
        }
        // Interleave the pages: one child of each batch in turn.
        let mut due = Vec::new();
        loop {
            let before = due.len();
            for page in &mut pages {
                due.extend(page.next());
            }
            if due.len() == before {
                return Ok(due);
            }
        }
    }

    /// When true, a batch child of this Scenario with a transient failure is
    /// retried every 24 h after the D-024 retry schedule, until it succeeds
    /// or the user stops the batch. The default is on. A missing Scenario
    /// reads as off.
    pub fn scenario_retry_until_success(&self, scenario_id: &str) -> Result<bool> {
        let value: Option<bool> = self
            .db
            .query_row(
                "SELECT retry_until_success FROM scenarios WHERE id=?1",
                [scenario_id],
                |row| row.get(0),
            )
            .optional()?;
        Ok(value.unwrap_or(false))
    }

    /// Change the retry mode of one Scenario. The revision does not change.
    pub fn set_scenario_retry_until_success(&self, scenario_id: &str, enabled: bool) -> Result<()> {
        self.changed(self.db.execute(
            "UPDATE scenarios SET retry_until_success=?1 WHERE id=?2",
            params![enabled, scenario_id],
        )?)
    }

    /// Check one child before the Worker starts it. This reads one row.
    pub fn batch_child_due(
        &self,
        batch_id: &str,
        relative_path: &str,
        job_id: &str,
    ) -> Result<BatchChildDue> {
        let row: Option<(String, String, bool, bool)> = self
            .db
            .query_row(
                "SELECT c.state,c.job_id,b.paused,b.cancellation_requested
                 FROM scenario_create_batch_children c
                 JOIN scenario_create_batches b ON b.id=c.batch_id
                 WHERE c.batch_id=?1 AND c.relative_path=?2",
                params![batch_id, relative_path],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()?;
        Ok(match row {
            Some((state, current, _, _)) if state != "pending" || current != job_id => {
                BatchChildDue::NotDue
            }
            Some((_, _, _, true)) => BatchChildDue::Cancelled,
            Some((_, _, true, _)) => BatchChildDue::Paused,
            Some(_) => BatchChildDue::Due,
            None => BatchChildDue::NotDue,
        })
    }

    /// Progress of one batch from counts (D-024). `now` separates due
    /// children from children that wait for a retry.
    pub fn scenario_create_batch_summary(
        &self,
        batch_id: &str,
        now: i64,
    ) -> Result<Option<ScenarioBatchSummary>> {
        let parent: Option<(bool, bool)> = self
            .db
            .query_row(
                "SELECT cancellation_requested,paused FROM scenario_create_batches WHERE id=?1",
                [batch_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((cancellation_requested, paused)) = parent else {
            return Ok(None);
        };
        let mut summary = ScenarioBatchSummary {
            id: batch_id.to_owned(),
            cancellation_requested,
            paused,
            total: 0,
            completed: 0,
            failed: 0,
            cancelled: 0,
            unknown: 0,
            due: 0,
            retry_waiting: 0,
            next_retry_at: None,
            current_path: None,
            failure_reasons: Vec::new(),
            unfinished_sample: Vec::new(),
        };
        let mut statement = self.db.prepare(
            "SELECT state,
                    CASE WHEN state='pending' AND next_retry_at>?2 THEN 1 ELSE 0 END,
                    count(*), min(CASE WHEN state='pending' THEN next_retry_at END)
             FROM scenario_create_batch_children WHERE batch_id=?1 GROUP BY 1,2",
        )?;
        let rows = statement.query_map(params![batch_id, now], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, bool>(1)?,
                row.get::<_, i64>(2)? as u64,
                row.get::<_, Option<i64>>(3)?,
            ))
        })?;
        for row in rows {
            let (state, waiting, count, next) = row?;
            summary.total += count;
            match (state.as_str(), waiting) {
                ("completed", _) => summary.completed += count,
                ("failed", _) => summary.failed += count,
                ("cancelled", _) => summary.cancelled += count,
                ("unknown", _) => summary.unknown += count,
                ("pending", true) => {
                    summary.retry_waiting += count;
                    summary.next_retry_at = next;
                }
                _ => summary.due += count,
            }
        }
        summary.current_path = self
            .db
            .query_row(
                "SELECT relative_path FROM scenario_create_batch_children
                 WHERE batch_id=?1 AND state='pending' AND (next_retry_at IS NULL OR next_retry_at<=?2)
                 ORDER BY relative_path LIMIT 1",
                params![batch_id, now],
                |row| row.get(0),
            )
            .optional()?;
        let mut statement = self.db.prepare(
            "SELECT last_error,count(*) FROM scenario_create_batch_children
             WHERE batch_id=?1 AND state IN ('failed','unknown') AND last_error IS NOT NULL
             GROUP BY last_error ORDER BY count(*) DESC LIMIT 5",
        )?;
        summary.failure_reasons = statement
            .query_map([batch_id], |row| {
                Ok((row.get(0)?, row.get::<_, i64>(1)? as u64))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut statement = self.db.prepare(
            "SELECT relative_path FROM scenario_create_batch_children
             WHERE batch_id=?1 AND state NOT IN ('completed','cancelled')
             ORDER BY relative_path LIMIT 5",
        )?;
        summary.unfinished_sample = statement
            .query_map([batch_id], |row| row.get(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(Some(summary))
    }

    /// Start the run of a dispatched schedule Job. A repeated call for the
    /// same Job returns the same answer, so a restart never starts two runs.
    /// When another run of the Scenario is active, the slot is skipped
    /// (D-025 section 2) and stays skipped after that run ends.
    pub fn begin_schedule_run(&self, schedule_job_id: &str) -> Result<ScheduleRunStart> {
        let reference = self
            .schedule_job_reference(schedule_job_id)?
            .ok_or(RegistryError::NotFound)?;
        let transaction = rusqlite::Transaction::new_unchecked(
            &self.db,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let existing: Option<String> = transaction
            .query_row(
                "SELECT id FROM scenario_runs WHERE schedule_job_id=?1",
                [schedule_job_id],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(id) = existing {
            return Ok(ScheduleRunStart::Started(id));
        }
        let (schedule_id, occurrence_key, skipped): (String, String, Option<String>) = transaction
            .query_row(
                "SELECT o.schedule_id,o.occurrence_key,so.skipped_active_run_id
                 FROM schedule_job_outbox o
                 LEFT JOIN schedule_occurrences so
                   ON so.schedule_id=o.schedule_id AND so.occurrence_key=o.occurrence_key
                 WHERE o.job_id=?1",
                [schedule_job_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )?;
        if let Some(active) = skipped {
            return Ok(ScheduleRunStart::Skipped(active));
        }
        let active: Option<String> = transaction
            .query_row(
                "SELECT id FROM scenario_runs WHERE scenario_id=?1
                   AND state IN ('planning','executing')",
                [&reference.scenario_id],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(active) = active {
            transaction.execute(
                "UPDATE schedule_occurrences SET skipped_active_run_id=?3
                 WHERE schedule_id=?1 AND occurrence_key=?2",
                params![schedule_id, occurrence_key, active],
            )?;
            transaction.commit()?;
            return Ok(ScheduleRunStart::Skipped(active));
        }
        let id: String =
            transaction.query_row("SELECT lower(hex(randomblob(16)))", [], |row| row.get(0))?;
        transaction.execute(
            "INSERT INTO scenario_runs(
                id,scenario_id,scenario_revision,trigger,schedule_job_id,state,created_at
             ) VALUES(?1,?2,?3,'schedule',?4,'planning',?5)",
            params![
                id,
                reference.scenario_id,
                reference.revision as i64,
                schedule_job_id,
                unix_seconds()
            ],
        )?;
        transaction.commit()?;
        Ok(ScheduleRunStart::Started(id))
    }

    /// The active run of a Scenario, if any. At most one exists.
    pub fn active_scenario_run(&self, scenario_id: &str) -> Result<Option<ScenarioRun>> {
        Ok(self
            .db
            .query_row(
                &format!(
                    "SELECT {} FROM scenario_runs WHERE scenario_id=?1
                       AND state IN ('planning','executing')",
                    Self::SCENARIO_RUN_COLUMNS
                ),
                [scenario_id],
                Self::scenario_run_from_row,
            )
            .optional()?)
    }

    /// Slots of a Scenario that were skipped because a run was active,
    /// newest first: `(schedule_id, occurrence_key, active_run_id)`.
    pub fn skipped_schedule_slots(
        &self,
        scenario_id: &str,
        limit: usize,
    ) -> Result<Vec<(String, String, String)>> {
        let mut statement = self.db.prepare(
            "SELECT so.schedule_id,so.occurrence_key,so.skipped_active_run_id
             FROM schedule_occurrences so
             JOIN scenario_schedules ss ON ss.id=so.schedule_id
             WHERE ss.scenario_id=?1 AND so.skipped_active_run_id IS NOT NULL
             ORDER BY so.rowid DESC LIMIT ?2",
        )?;
        let rows = statement
            .query_map(params![scenario_id, limit as i64], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// The newest claimed slot of one schedule and whether it was skipped
    /// because a run of its Scenario was active.
    pub fn last_schedule_slot(&self, schedule_id: &str) -> Result<Option<(String, bool)>> {
        Ok(self
            .db
            .query_row(
                "SELECT occurrence_key,skipped_active_run_id IS NOT NULL
                 FROM schedule_occurrences WHERE schedule_id=?1
                 ORDER BY rowid DESC LIMIT 1",
                [schedule_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?)
    }

    /// Record a manual run that plans and executes like a scheduled run: the
    /// Worker builds the plan and starts the batch. It serves the "Запустить"
    /// action of the monitoring Scenario widget (D-025 section 2). The batch
    /// step still requires the confirmed source-wins rule.
    pub fn begin_manual_planning_run(&self, scenario: &ScenarioReference) -> Result<String> {
        let id: String = self
            .db
            .query_row("SELECT lower(hex(randomblob(16)))", [], |row| row.get(0))?;
        self.db
            .execute(
                "INSERT INTO scenario_runs(
                id,scenario_id,scenario_revision,trigger,state,created_at
             ) VALUES(?1,?2,?3,'manual','planning',?4)",
                params![
                    id,
                    scenario.scenario_id,
                    scenario.revision as i64,
                    unix_seconds()
                ],
            )
            .map_err(|error| match error {
                // `scenario_runs_one_active`: another run of the Scenario is active.
                rusqlite::Error::SqliteFailure(failure, _)
                    if failure.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE =>
                {
                    RegistryError::ScenarioRunActive
                }
                other => other.into(),
            })?;
        Ok(id)
    }

    /// The newest finished run of a Scenario, for the monitoring widget.
    pub fn last_finished_scenario_run(&self, scenario_id: &str) -> Result<Option<ScenarioRun>> {
        Ok(self
            .db
            .query_row(
                &format!(
                    "SELECT {} FROM scenario_runs WHERE scenario_id=?1
                       AND state IN ('completed','failed','cancelled')
                     ORDER BY coalesce(finished_at,created_at) DESC, rowid DESC LIMIT 1",
                    Self::SCENARIO_RUN_COLUMNS
                ),
                [scenario_id],
                Self::scenario_run_from_row,
            )
            .optional()?)
    }

    /// Record a manual run whose batch the user confirmed from a plan.
    pub fn begin_manual_run(
        &self,
        scenario: &ScenarioReference,
        plan_id: &str,
        batch_id: &str,
        deferred_items: u64,
    ) -> Result<String> {
        let id: String = self
            .db
            .query_row("SELECT lower(hex(randomblob(16)))", [], |row| row.get(0))?;
        self.db
            .execute(
                "INSERT INTO scenario_runs(
                id,scenario_id,scenario_revision,trigger,plan_id,batch_id,state,
                deferred_items,created_at
             ) VALUES(?1,?2,?3,'manual',?4,?5,'executing',?6,?7)",
                params![
                    id,
                    scenario.scenario_id,
                    scenario.revision as i64,
                    plan_id,
                    batch_id,
                    deferred_items as i64,
                    unix_seconds()
                ],
            )
            .map_err(|error| match error {
                // `scenario_runs_one_active`: another run of the Scenario is active.
                rusqlite::Error::SqliteFailure(failure, _)
                    if failure.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE =>
                {
                    RegistryError::ScenarioRunActive
                }
                other => other.into(),
            })?;
        Ok(id)
    }

    fn scenario_run_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ScenarioRun> {
        Ok(ScenarioRun {
            id: row.get(0)?,
            scenario: ScenarioReference {
                scenario_id: row.get(1)?,
                revision: row.get::<_, i64>(2)? as u64,
            },
            trigger: row.get(3)?,
            schedule_job_id: row.get(4)?,
            plan_id: row.get(5)?,
            batch_id: row.get(6)?,
            state: row.get(7)?,
            waiting_reason: row.get(8)?,
            error: row.get(9)?,
            deferred_items: row.get::<_, i64>(10)? as u64,
            created_at: row.get(11)?,
            finished_at: row.get(12)?,
            waiting_for_scenario_id: row.get(13)?,
        })
    }

    const SCENARIO_RUN_COLUMNS: &'static str =
        "id,scenario_id,scenario_revision,trigger,schedule_job_id,plan_id,batch_id,state,
         waiting_reason,error,deferred_items,created_at,finished_at,waiting_for_scenario_id";

    pub fn scenario_run(&self, id: &str) -> Result<Option<ScenarioRun>> {
        Ok(self
            .db
            .query_row(
                &format!(
                    "SELECT {} FROM scenario_runs WHERE id=?1",
                    Self::SCENARIO_RUN_COLUMNS
                ),
                [id],
                Self::scenario_run_from_row,
            )
            .optional()?)
    }

    /// Runs that are planning or executing, oldest first, through an index.
    pub fn active_scenario_runs(&self) -> Result<Vec<ScenarioRun>> {
        let mut statement = self.db.prepare(&format!(
            "SELECT {} FROM scenario_runs WHERE state IN ('planning','executing')
             ORDER BY created_at, rowid",
            Self::SCENARIO_RUN_COLUMNS
        ))?;
        let rows = statement
            .query_map([], Self::scenario_run_from_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// The roots of a Scenario revision for the write-root lock.
    fn revision_roots(&self, scenario_id: &str, revision: u64) -> Result<Vec<ScenarioRoot>> {
        let scenario = self
            .get_scenario_revision(scenario_id, revision)?
            .ok_or(RegistryError::NotFound)?;
        Ok(scenario_roots(&scenario, |id| {
            self.get(id)
                .ok()
                .flatten()
                .map(|connection| connection.adapter)
        }))
    }

    /// Take the write-root lock for an active run, or name the Scenario whose
    /// run holds a conflicting lock (D-025 section 11). Only active runs hold
    /// a lock, so the end of a run releases it and a Worker restart keeps it.
    pub fn acquire_scenario_run_write_lock(&self, run_id: &str) -> Result<RunWriteLock> {
        let transaction = rusqlite::Transaction::new_unchecked(
            &self.db,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let run = self.scenario_run(run_id)?.ok_or(RegistryError::NotFound)?;
        if !matches!(run.state.as_str(), "planning" | "executing") {
            return Err(RegistryError::NotFound);
        }
        let holds: bool = transaction.query_row(
            "SELECT write_lock FROM scenario_runs WHERE id=?1",
            [run_id],
            |row| row.get(0),
        )?;
        if holds {
            return Ok(RunWriteLock::Acquired);
        }
        let mine = self.revision_roots(&run.scenario.scenario_id, run.scenario.revision)?;
        let holders = {
            let mut statement = transaction.prepare(
                "SELECT scenario_id,scenario_revision FROM scenario_runs
                 WHERE write_lock=1 AND state IN ('planning','executing') AND id<>?1
                 ORDER BY created_at, rowid",
            )?;
            let rows = statement
                .query_map([run_id], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as u64))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            rows
        };
        for (scenario_id, revision) in holders {
            if conflict(&mine, &self.revision_roots(&scenario_id, revision)?) {
                transaction.execute(
                    "UPDATE scenario_runs SET waiting_for_scenario_id=?2 WHERE id=?1",
                    params![run_id, scenario_id],
                )?;
                transaction.commit()?;
                return Ok(RunWriteLock::WaitFor { scenario_id });
            }
        }
        transaction.execute(
            "UPDATE scenario_runs SET write_lock=1, waiting_for_scenario_id=NULL WHERE id=?1",
            [run_id],
        )?;
        transaction.commit()?;
        Ok(RunWriteLock::Acquired)
    }

    /// Enabled Scenarios other than `exclude` whose runs cannot execute at the
    /// same time as runs of `candidate`, for the editor warning.
    pub fn overlapping_scenarios(
        &self,
        candidate: &Scenario,
        exclude: Option<&str>,
    ) -> Result<Vec<Scenario>> {
        let adapter = |id: &str| {
            self.get(id)
                .ok()
                .flatten()
                .map(|connection| connection.adapter)
        };
        let mine = scenario_roots(candidate, adapter);
        Ok(self
            .list_scenarios()?
            .into_iter()
            .filter(|other| other.enabled && Some(other.id.as_str()) != exclude)
            .filter(|other| conflict(&mine, &scenario_roots(other, adapter)))
            .collect())
    }

    /// The newest run of each listed Scenario is what the interface shows.
    pub fn latest_scenario_run(&self, scenario_id: &str) -> Result<Option<ScenarioRun>> {
        Ok(self
            .db
            .query_row(
                &format!(
                    "SELECT {} FROM scenario_runs WHERE scenario_id=?1
                     ORDER BY created_at DESC, rowid DESC LIMIT 1",
                    Self::SCENARIO_RUN_COLUMNS
                ),
                [scenario_id],
                Self::scenario_run_from_row,
            )
            .optional()?)
    }

    /// Attach the plan that a planning run waits for. `None` clears it, so
    /// the run requests a new plan.
    pub fn set_scenario_run_plan(&self, id: &str, plan_id: Option<&str>) -> Result<()> {
        self.update_active_run(
            "UPDATE scenario_runs SET plan_id=?2, waiting_reason=NULL
             WHERE id=?1 AND state='planning'",
            id,
            plan_id,
        )
    }

    /// Record why an active run makes no progress now; `None` clears it.
    pub fn set_scenario_run_waiting(&self, id: &str, reason: Option<&str>) -> Result<()> {
        self.update_active_run(
            "UPDATE scenario_runs SET waiting_reason=?2
             WHERE id=?1 AND state IN ('planning','executing')",
            id,
            reason,
        )
    }

    fn update_active_run(&self, sql: &str, id: &str, value: Option<&str>) -> Result<()> {
        if self.db.execute(sql, params![id, value])? == 0 {
            return Err(RegistryError::NotFound);
        }
        Ok(())
    }

    /// A planning run starts the batch made from its complete plan.
    pub fn set_scenario_run_executing(
        &self,
        id: &str,
        batch_id: &str,
        deferred_items: u64,
    ) -> Result<()> {
        let changed = self.db.execute(
            "UPDATE scenario_runs SET state='executing', batch_id=?2, deferred_items=?3,
                    waiting_reason=NULL
             WHERE id=?1 AND state='planning'",
            params![id, batch_id, deferred_items as i64],
        )?;
        if changed == 0 {
            return Err(RegistryError::NotFound);
        }
        Ok(())
    }

    /// Finish an active run. A finished run never changes again.
    pub fn finish_scenario_run(&self, id: &str, state: &str, error: Option<&str>) -> Result<()> {
        if !matches!(state, "completed" | "failed" | "cancelled") {
            return Err(RegistryError::InvalidSyncState);
        }
        let changed = self.db.execute(
            "UPDATE scenario_runs SET state=?2, error=?3, waiting_reason=NULL, finished_at=?4
             WHERE id=?1 AND state IN ('planning','executing')",
            params![id, state, error, unix_seconds()],
        )?;
        if changed == 0 {
            return Err(RegistryError::NotFound);
        }
        Ok(())
    }

    /// IDs of the batches of one Scenario revision that still have pending
    /// children (due or waiting for a retry).
    pub fn pending_scenario_create_batch_ids_for(
        &self,
        scenario: &ScenarioReference,
    ) -> Result<Vec<String>> {
        let mut statement = self.db.prepare(
            "SELECT b.id FROM scenario_create_batches b
             WHERE b.scenario_id=?1 AND b.scenario_revision=?2
               AND EXISTS (SELECT 1 FROM scenario_create_batch_children c
                           WHERE c.batch_id=b.id AND c.state='pending')
             ORDER BY b.rowid",
        )?;
        let ids = statement
            .query_map(
                params![scenario.scenario_id, scenario.revision as i64],
                |row| row.get(0),
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(ids)
    }

    /// Pause or resume a batch. A pause takes effect before the next child.
    pub fn set_scenario_create_batch_paused(&self, batch_id: &str, paused: bool) -> Result<()> {
        let changed = self.db.execute(
            "UPDATE scenario_create_batches SET paused=?2 WHERE id=?1",
            params![batch_id, paused],
        )?;
        if changed == 1 {
            Ok(())
        } else {
            Err(RegistryError::NotFound)
        }
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
        self.finish_scenario_create_batch_child(batch_id, relative_path, state, None)
    }

    /// Like `set_scenario_create_batch_child_state`, with the stable code of
    /// the failure. A finished child has no next retry.
    pub fn finish_scenario_create_batch_child(
        &self,
        batch_id: &str,
        relative_path: &str,
        state: &str,
        error: Option<&str>,
    ) -> Result<()> {
        if !matches!(state, "completed" | "failed" | "cancelled" | "unknown") {
            return Err(RegistryError::InvalidSyncState);
        }
        let changed = self.db.execute(
            "UPDATE scenario_create_batch_children
             SET state=?3, next_retry_at=NULL, last_error=COALESCE(?4, last_error)
             WHERE batch_id=?1 AND relative_path=?2 AND state='pending'",
            params![batch_id, relative_path, state, error],
        )?;
        if changed == 1 {
            Ok(())
        } else {
            Err(RegistryError::NotFound)
        }
    }

    /// Cancel every pending child of a batch whose stop the user requested, in
    /// one statement. A pending child has no Job yet, so nothing else changes.
    /// One update for each child took minutes for a large run (2026-11-04).
    pub fn backup_database_results(&self, run_id: &str) -> Result<Vec<BackupDatabaseResult>> {
        let mut statement = self.db.prepare(
            "SELECT provider_id, result, error, bytes, md5, finished_at
             FROM backup_database_results WHERE run_id=?1 ORDER BY provider_id",
        )?;
        let rows = statement
            .query_map([run_id], |row| {
                Ok(BackupDatabaseResult {
                    provider_id: row.get(0)?,
                    result: row.get(1)?,
                    error: row.get(2)?,
                    bytes: row.get::<_, Option<i64>>(3)?.map(|bytes| bytes as u64),
                    md5: row.get(4)?,
                    finished_at: row.get(5)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Record the result of one database. A second record keeps the first.
    pub fn record_backup_database_result(
        &self,
        run_id: &str,
        result: &BackupDatabaseResult,
    ) -> Result<()> {
        self.db.execute(
            "INSERT OR IGNORE INTO backup_database_results
             (run_id, provider_id, result, error, bytes, md5, finished_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                run_id,
                result.provider_id,
                result.result,
                result.error,
                result.bytes.map(|bytes| bytes as i64),
                result.md5,
                result.finished_at
            ],
        )?;
        Ok(())
    }

    pub fn cancel_pending_batch_children(&self, batch_id: &str) -> Result<usize> {
        Ok(self.db.execute(
            "UPDATE scenario_create_batch_children
             SET state='cancelled', next_retry_at=NULL
             WHERE batch_id=?1 AND state='pending'
               AND EXISTS (SELECT 1 FROM scenario_create_batches b
                           WHERE b.id=?1 AND b.cancellation_requested=1)",
            [batch_id],
        )?)
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
        let transaction = rusqlite::Transaction::new_unchecked(
            &self.db,
            rusqlite::TransactionBehavior::Immediate,
        )?;
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

    /// Removes a Scenario and all its local records: revisions, schedules,
    /// runs, batches, conflict tasks, and synchronization states. This
    /// changes only the Registry. It never touches a file or a provider
    /// object. The caller checks JobStore first; this check and the removal
    /// are one transaction, so a run cannot start in between.
    pub fn remove_scenario(&self, scenario_id: &str) -> Result<()> {
        let transaction = rusqlite::Transaction::new_unchecked(
            &self.db,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let exists: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM scenarios WHERE id=?1)",
            [scenario_id],
            |row| row.get(0),
        )?;
        if !exists {
            return Err(RegistryError::NotFound);
        }
        let active: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM scenario_runs
                           WHERE scenario_id=?1 AND state IN ('planning','executing'))",
            [scenario_id],
            |row| row.get(0),
        )?;
        if active {
            return Err(RegistryError::ScenarioRunActive);
        }
        let unknown: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM scenario_create_batch_children c
                           JOIN scenario_create_batches b ON b.id=c.batch_id
                           WHERE b.scenario_id=?1 AND c.state='unknown')",
            [scenario_id],
            |row| row.get(0),
        )?;
        if unknown {
            return Err(RegistryError::ScenarioHasUnresolvedItems);
        }
        // Children before parents: every foreign key is RESTRICT.
        for statement in [
            "DELETE FROM schedule_child_jobs WHERE schedule_job_id IN
               (SELECT job_id FROM schedule_job_outbox WHERE scenario_id=?1)",
            "DELETE FROM schedule_conflict_links WHERE conflict_id IN
               (SELECT id FROM scenario_conflict_tasks WHERE scenario_id=?1)
               OR schedule_job_id IN (SELECT job_id FROM schedule_job_outbox WHERE scenario_id=?1)",
            "DELETE FROM schedule_job_outbox WHERE scenario_id=?1",
            "DELETE FROM schedule_occurrences WHERE schedule_id IN
               (SELECT id FROM scenario_schedules WHERE scenario_id=?1)",
            "DELETE FROM scenario_schedules WHERE scenario_id=?1",
            "DELETE FROM scenario_runs WHERE scenario_id=?1",
            "DELETE FROM scenario_create_batch_children WHERE batch_id IN
               (SELECT id FROM scenario_create_batches WHERE scenario_id=?1)",
            "DELETE FROM scenario_create_batches WHERE scenario_id=?1",
            "DELETE FROM scenario_conflict_tasks WHERE scenario_id=?1",
            "DELETE FROM file_sync_states WHERE scenario_id=?1",
            "DELETE FROM scenario_revision_selection WHERE scenario_id=?1",
            "DELETE FROM scenario_revision_resources WHERE scenario_id=?1",
            "DELETE FROM scenario_source_wins_consents WHERE scenario_id=?1",
            "DELETE FROM scenario_automation WHERE scenario_id=?1",
            "DELETE FROM scenario_revisions WHERE scenario_id=?1",
            "DELETE FROM scenarios WHERE id=?1",
        ] {
            transaction.execute(statement, [scenario_id])?;
        }
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
        let transaction = rusqlite::Transaction::new_unchecked(
            &self.db,
            rusqlite::TransactionBehavior::Immediate,
        )?;
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
        self.update_scenario_with_scope(id, update, &ScenarioScope::default())
    }

    /// Like `update_scenario`, with the selection and deletion rule of the
    /// new revision. Older revisions keep their own selection and rule.
    pub fn update_scenario_with_scope(
        &self,
        id: &str,
        update: ScenarioUpdate<'_>,
        scope: &ScenarioScope,
    ) -> Result<()> {
        let selection = ScenarioScope::normalized_selection(&scope.selected_paths)
            .map_err(|_| RegistryError::InvalidSelection)?;
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
        self.require_yandex_scope(
            update.left_connection_id,
            update.right_connection_id,
            update.direction,
        )?;
        if self.local_roots_overlap(
            update.left_connection_id,
            update.left_root,
            update.right_connection_id,
            update.right_root,
        )? {
            return Err(RegistryError::ScenarioRootsOverlap);
        }
        let transaction = rusqlite::Transaction::new_unchecked(
            &self.db,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let changed = transaction.execute(
            "UPDATE scenarios
             SET name=?1,left_connection_id=?2,left_root=?3,
                 right_connection_id=?4,right_root=?5,direction=?6,
                 deletion_mode=?7,revision=revision+1
             WHERE id=?8",
            params![
                update.name.trim(),
                update.left_connection_id,
                update.left_root,
                update.right_connection_id,
                update.right_root,
                update.direction,
                scope.deletion_mode.as_str(),
                id
            ],
        )?;
        if changed == 0 {
            return Err(RegistryError::NotFound);
        }
        transaction.execute(
            "INSERT INTO scenario_revisions(
                scenario_id,revision,name,left_connection_id,left_root,
                right_connection_id,right_root,direction,deletion_mode
             )
             SELECT id,revision,name,left_connection_id,left_root,
                    right_connection_id,right_root,direction,deletion_mode
             FROM scenarios WHERE id=?1",
            [id],
        )?;
        let revision: i64 =
            transaction.query_row("SELECT revision FROM scenarios WHERE id=?1", [id], |row| {
                row.get(0)
            })?;
        insert_revision_selection(&transaction, id, revision as u64, &selection)?;
        transaction.commit()?;
        self.carry_forward_confirmed_states(id, revision as u64)?;
        Ok(())
    }

    /// Copy the confirmed file states of the previous revision into
    /// `revision` when both revisions use the same connections and roots.
    /// A confirmed state records the versions of one path on two endpoints;
    /// a new selection, direction, or name does not change that evidence.
    /// Without this, every file synchronized before an edit became a
    /// conflict (found on 2026-10-25). Existing states of `revision` stay.
    pub fn carry_forward_confirmed_states(&self, id: &str, revision: u64) -> Result<usize> {
        if revision < 2 {
            return Ok(0);
        }
        let endpoints = |revision: u64| -> Result<Option<(String, String, String, String)>> {
            Ok(self
                .db
                .query_row(
                    "SELECT left_connection_id,left_root,right_connection_id,right_root
                     FROM scenario_revisions WHERE scenario_id=?1 AND revision=?2",
                    params![id, revision as i64],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )
                .optional()?)
        };
        let Some(current) = endpoints(revision)? else {
            return Ok(0);
        };
        // The newest earlier revision with confirmed states.
        let source: Option<i64> = self
            .db
            .query_row(
                "SELECT max(scenario_revision) FROM file_sync_states
                 WHERE scenario_id=?1 AND scenario_revision<?2",
                params![id, revision as i64],
                |row| row.get(0),
            )
            .optional()?
            .flatten();
        let Some(source) = source else {
            return Ok(0);
        };
        // Every revision between the source and this one must keep the
        // endpoints; otherwise the evidence belongs to other storage.
        for between in source as u64..revision {
            if endpoints(between)?.as_ref() != Some(&current) {
                return Ok(0);
            }
        }
        Ok(self.db.execute(
            "INSERT OR IGNORE INTO file_sync_states(
                scenario_id,scenario_revision,relative_path,
                left_object_revision,left_size,left_checksum_md5,
                right_object_revision,right_size,right_checksum_md5)
             SELECT scenario_id,?3,relative_path,
                left_object_revision,left_size,left_checksum_md5,
                right_object_revision,right_size,right_checksum_md5
             FROM file_sync_states WHERE scenario_id=?1 AND scenario_revision=?2",
            params![id, source, revision as i64],
        )?)
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

    /// The selection and deletion rule of an exact revision. A missing
    /// revision is `NotFound`; it never falls back to the current row.
    pub fn get_scenario_revision_scope(&self, id: &str, revision: u64) -> Result<ScenarioScope> {
        let mode: Option<String> = self
            .db
            .query_row(
                "SELECT deletion_mode FROM scenario_revisions WHERE scenario_id=?1 AND revision=?2",
                params![id, revision as i64],
                |row| row.get(0),
            )
            .optional()?;
        let deletion_mode = DeletionMode::parse(&mode.ok_or(RegistryError::NotFound)?)
            .ok_or(RegistryError::IntegrityFailed)?;
        // A Backup/export revision selects server paths (D-027 section 6). The
        // source root is the home folder of the account, so a path is the
        // server path relative to it; the destination keeps it below `files/`.
        // The home folder itself selects the whole root.
        let mut statement = self.db.prepare(
            "SELECT relative_path FROM scenario_revision_selection
             WHERE scenario_id=?1 AND revision=?2
             UNION
             SELECT CASE WHEN r.provider_id=v.left_root THEN ''
                         ELSE substr(r.provider_id, length(rtrim(v.left_root,'/'))+2) END
             FROM scenario_revision_resources r
             JOIN scenario_revisions v ON v.scenario_id=r.scenario_id AND v.revision=r.revision
             WHERE r.scenario_id=?1 AND r.revision=?2 AND r.category='files' AND r.rule='include'
             ORDER BY 1",
        )?;
        let mut selected_paths = statement
            .query_map(params![id, revision as i64], |row| row.get(0))?
            .collect::<std::result::Result<Vec<String>, _>>()?;
        if selected_paths.iter().any(String::is_empty) {
            selected_paths.clear();
        }
        let mut statement = self.db.prepare(
            "SELECT CASE WHEN r.provider_id=v.left_root THEN ''
                         ELSE substr(r.provider_id, length(rtrim(v.left_root,'/'))+2) END
             FROM scenario_revision_resources r
             JOIN scenario_revisions v ON v.scenario_id=r.scenario_id AND v.revision=r.revision
             WHERE r.scenario_id=?1 AND r.revision=?2 AND r.category='files' AND r.rule='exclude'
             ORDER BY 1",
        )?;
        let excluded_paths = statement
            .query_map(params![id, revision as i64], |row| row.get(0))?
            .collect::<std::result::Result<Vec<String>, _>>()?;
        Ok(ScenarioScope {
            selected_paths,
            deletion_mode,
            excluded_paths,
        })
    }

    /// Remove the confirmed baseline of one path after an executor verified
    /// that the object is absent on both sides. A later plan then treats the
    /// path as never synchronized instead of as a deletion on both sides.
    pub fn forget_confirmed_file_state(
        &self,
        reference: &ScenarioReference,
        relative_path: &str,
    ) -> Result<()> {
        self.db.execute(
            "DELETE FROM file_sync_states
             WHERE scenario_id=?1 AND scenario_revision=?2 AND relative_path=?3",
            params![
                reference.scenario_id,
                reference.revision as i64,
                relative_path
            ],
        )?;
        Ok(())
    }

    /// Remove the confirmed baselines of every path under one folder after a
    /// verified folder deletion (D-027 section 5.6b). The range uses the
    /// primary key: `0` is the next character after `/`.
    pub fn forget_confirmed_file_states_under(
        &self,
        reference: &ScenarioReference,
        folder: &str,
    ) -> Result<()> {
        self.db.execute(
            "DELETE FROM file_sync_states
             WHERE scenario_id=?1 AND scenario_revision=?2
               AND relative_path>=?3 AND relative_path<?4",
            params![
                reference.scenario_id,
                reference.revision as i64,
                format!("{folder}/"),
                format!("{folder}0")
            ],
        )?;
        Ok(())
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

    /// The confirmed state of one path, or `None` (D-028: one indexed row,
    /// not the whole baseline).
    pub fn confirmed_file_state(
        &self,
        reference: &ScenarioReference,
        relative_path: &str,
    ) -> Result<Option<FileSyncState>> {
        Ok(self
            .db
            .query_row(
                "SELECT relative_path,
                    left_object_revision,left_size,left_checksum_md5,
                    right_object_revision,right_size,right_checksum_md5
                 FROM file_sync_states
                 WHERE scenario_id=?1 AND scenario_revision=?2 AND relative_path=?3",
                params![
                    reference.scenario_id,
                    reference.revision as i64,
                    relative_path
                ],
                |row| {
                    Ok(FileSyncState {
                        scenario: reference.clone(),
                        relative_path: row.get(0)?,
                        left: decode_confirmed_version(row, 1, 2, 3)?,
                        right: decode_confirmed_version(row, 4, 5, 6)?,
                    })
                },
            )
            .optional()?)
    }

    /// One page of confirmed file states after `after_path`, in path order
    /// (D-024: the streaming planner never loads all of them).
    pub fn confirmed_file_states_page(
        &self,
        reference: &ScenarioReference,
        after_path: &str,
        limit: usize,
    ) -> Result<Vec<FileSyncState>> {
        let mut statement = self.db.prepare(
            "SELECT relative_path,
                left_object_revision,left_size,left_checksum_md5,
                right_object_revision,right_size,right_checksum_md5
             FROM file_sync_states
             WHERE scenario_id=?1 AND scenario_revision=?2 AND relative_path>?3
             ORDER BY relative_path LIMIT ?4",
        )?;
        let states = statement
            .query_map(
                params![
                    reference.scenario_id,
                    reference.revision as i64,
                    after_path,
                    limit as i64
                ],
                |row| {
                    Ok(FileSyncState {
                        scenario: reference.clone(),
                        relative_path: row.get(0)?,
                        left: decode_confirmed_version(row, 1, 2, 3)?,
                        right: decode_confirmed_version(row, 4, 5, 6)?,
                    })
                },
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(states)
    }

    /// The direction and the scope of a Scenario revision, for a planner.
    pub fn scenario_plan_inputs(
        &self,
        reference: &ScenarioReference,
    ) -> Result<(SyncDirection, ScenarioScope)> {
        let stored = self
            .get_scenario_revision(&reference.scenario_id, reference.revision)?
            .ok_or(RegistryError::NotFound)?;
        let direction = match stored.direction.as_str() {
            "left_to_right" => SyncDirection::LeftToRight,
            "right_to_left" => SyncDirection::RightToLeft,
            "bidirectional" => SyncDirection::Bidirectional,
            _ => return Err(RegistryError::PlanningFailed),
        };
        let scope = self.get_scenario_revision_scope(&reference.scenario_id, reference.revision)?;
        Ok((direction, scope))
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
        let scope = self.get_scenario_revision_scope(&reference.scenario_id, reference.revision)?;
        let baseline = self.list_confirmed_file_states(reference)?;
        scenario_planner::plan_scoped(
            &scenario,
            reference,
            &baseline,
            left_snapshot,
            right_snapshot,
            skipped_left,
            skipped_right,
            &scope,
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

/// Store a normalized selection for one revision. An empty selection stores
/// no rows and means the complete root.
/// One selected resource of a Backup/export Scenario revision (D-027
/// section 6). For the Files category, `provider_id` is an absolute server
/// path; `kind` is `file`, `dir` (recursive, new descendants included),
/// `symlink`, or `object` for a logical provider object.
/// The result of one selected database in one Backup/export run (D-027
/// section 7b).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BackupDatabaseResult {
    pub provider_id: String,
    /// `created`, `updated`, `unchanged`, `failed`, or `cancelled`.
    pub result: String,
    pub error: Option<String>,
    pub bytes: Option<u64>,
    pub md5: Option<String>,
    pub finished_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct BackupResource {
    pub category: String,
    pub provider_id: String,
    pub kind: String,
    /// `include`; `exclude` for an object inside a selected folder that the
    /// Scenario leaves out with all its content; or `structure_only` for a
    /// table `<database>/<table>` whose dump has no data (D-027 7b.10).
    #[serde(default = "BackupResource::include")]
    pub rule: String,
}

impl BackupResource {
    fn include() -> String {
        "include".to_owned()
    }

    fn valid(&self) -> bool {
        if !matches!(self.rule.as_str(), "include" | "exclude" | "structure_only") {
            return false;
        }
        let text_ok = |text: &str| {
            !text.is_empty() && text.len() <= 4096 && !text.chars().any(char::is_control)
        };
        if !text_ok(&self.category) || !text_ok(&self.provider_id) {
            return false;
        }
        match self.category.as_str() {
            "files" => {
                self.rule != "structure_only"
                    && matches!(self.kind.as_str(), "file" | "dir" | "symlink")
                    && self.provider_id.starts_with('/')
                    && !self
                        .provider_id
                        .split('/')
                        .any(|part| part == "." || part == "..")
            }
            // A MySQL database name, which is also its dump file name, or
            // `<database>/<table>` for a table that keeps only its structure.
            "databases" => {
                let database_ok = |name: &str| {
                    name.len() <= 64
                        && !name.is_empty()
                        && !name.starts_with(['.', '-'])
                        && name.bytes().all(|byte| {
                            byte.is_ascii_alphanumeric()
                                || matches!(byte, b'_' | b'-' | b'.' | b'$')
                        })
                };
                self.kind == "object"
                    && match self.rule.as_str() {
                        "include" => database_ok(&self.provider_id),
                        "structure_only" => {
                            self.provider_id.split_once('/').is_some_and(|(db, table)| {
                                database_ok(db)
                                    && crate::ispmanager_sftp::plain_table_name(table)
                            })
                        }
                        _ => false,
                    }
            }
            _ => self.kind == "object" && self.rule != "structure_only",
        }
    }
}

/// The fields of a Backup/export Scenario revision.
#[derive(Debug, Clone, Copy)]
pub struct BackupScenarioDraft<'a> {
    pub name: &'a str,
    pub source_connection_id: &'a str,
    pub destination_connection_id: &'a str,
    pub destination_root: &'a str,
    /// The home folder of the account: the source root (D-027 section 6).
    pub source_root: &'a str,
    pub resources: &'a [BackupResource],
    /// Deletions inside selected folders (D-027 section 5.4).
    pub deletion_mode: DeletionMode,
}

impl Registry {
    /// Create a Backup/export Scenario (D-027 section 5). The source is an
    /// ISPmanager connection; the destination is This Mac or an SMB share.
    pub fn create_backup_scenario(&self, draft: BackupScenarioDraft<'_>) -> Result<String> {
        self.validate_backup_draft(&draft)?;
        let files_root = backup_files_root(draft.destination_root);
        let id: String = self
            .db
            .query_row("SELECT lower(hex(randomblob(16)))", [], |row| row.get(0))?;
        let transaction = rusqlite::Transaction::new_unchecked(
            &self.db,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        transaction.execute(
            "INSERT INTO scenarios(id,name,left_connection_id,left_root,right_connection_id,
                                   right_root,direction,mode,deletion_mode)
             VALUES(?1,?2,?3,?7,?4,?5,'left_to_right','backup_export',?6)",
            params![
                id,
                draft.name.trim(),
                draft.source_connection_id,
                draft.destination_connection_id,
                files_root,
                draft.deletion_mode.as_str(),
                draft.source_root
            ],
        )?;
        insert_backup_revision(&transaction, &id, 1, draft.resources)?;
        transaction.commit()?;
        Ok(id)
    }

    /// Store a new revision of a Backup/export Scenario and return it. Older
    /// revisions keep their own selection. The source connection does not
    /// change.
    pub fn update_backup_scenario(&self, id: &str, draft: BackupScenarioDraft<'_>) -> Result<u64> {
        self.validate_backup_draft(&draft)?;
        let files_root = backup_files_root(draft.destination_root);
        let transaction = rusqlite::Transaction::new_unchecked(
            &self.db,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let changed = transaction.execute(
            "UPDATE scenarios SET name=?1,right_connection_id=?2,right_root=?3,deletion_mode=?6,
                                  left_root=?7,revision=revision+1
             WHERE id=?4 AND mode='backup_export' AND left_connection_id=?5",
            params![
                draft.name.trim(),
                draft.destination_connection_id,
                files_root,
                id,
                draft.source_connection_id,
                draft.deletion_mode.as_str(),
                draft.source_root
            ],
        )?;
        if changed == 0 {
            return Err(RegistryError::NotFound);
        }
        let revision: i64 =
            transaction.query_row("SELECT revision FROM scenarios WHERE id=?1", [id], |row| {
                row.get(0)
            })?;
        insert_backup_revision(&transaction, id, revision as u64, draft.resources)?;
        transaction.commit()?;
        Ok(revision as u64)
    }

    /// `sync` or `backup_export`.
    pub fn scenario_mode(&self, id: &str) -> Result<String> {
        self.db
            .query_row("SELECT mode FROM scenarios WHERE id=?1", [id], |row| {
                row.get(0)
            })
            .optional()?
            .ok_or(RegistryError::NotFound)
    }

    /// The typed selection of one Backup/export Scenario revision.
    pub fn backup_resources(&self, id: &str, revision: u64) -> Result<Vec<BackupResource>> {
        let mut statement = self.db.prepare(
            "SELECT category,provider_id,kind,rule FROM scenario_revision_resources
             WHERE scenario_id=?1 AND revision=?2 ORDER BY category,provider_id",
        )?;
        let rows = statement.query_map(params![id, revision as i64], |row| {
            Ok(BackupResource {
                category: row.get(0)?,
                provider_id: row.get(1)?,
                kind: row.get(2)?,
                rule: row.get(3)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    fn validate_backup_draft(&self, draft: &BackupScenarioDraft<'_>) -> Result<()> {
        let root = draft.source_root;
        let root_valid = root.starts_with('/')
            && !root.chars().any(char::is_control)
            && !root.split('/').any(|part| part == "." || part == "..");
        if !valid_name(draft.name) || !draft.destination_root.starts_with('/') || !root_valid {
            return Err(RegistryError::InvalidName);
        }
        // Every file rule is the home folder or inside it.
        let inside = |path: &str| {
            path == root
                || root == "/"
                || path.starts_with(&format!("{}/", root.trim_end_matches('/')))
        };
        if draft
            .resources
            .iter()
            .any(|r| r.category == "files" && !inside(&r.provider_id))
        {
            return Err(RegistryError::InvalidSelection);
        }
        if !draft.resources.iter().all(BackupResource::valid) {
            return Err(RegistryError::InvalidSelection);
        }
        // An empty selection means nothing; an exclusion needs a selected
        // folder above it.
        let includes: Vec<&BackupResource> = draft
            .resources
            .iter()
            .filter(|r| r.rule == "include")
            .collect();
        let excluded_alone = draft.resources.iter().any(|r| {
            r.rule == "exclude"
                && !includes.iter().any(|include| {
                    include.kind == "dir"
                        && r.provider_id
                            .starts_with(&format!("{}/", include.provider_id))
                })
        });
        // A table rule needs its selected database.
        let table_alone = draft.resources.iter().any(|r| {
            r.rule == "structure_only"
                && !includes.iter().any(|include| {
                    include.category == "databases"
                        && r.provider_id
                            .starts_with(&format!("{}/", include.provider_id))
                })
        });
        if includes.is_empty() || excluded_alone || table_alone {
            return Err(RegistryError::InvalidSelection);
        }
        let adapter = |id: &str| -> Result<String> {
            Ok(self.get(id)?.ok_or(RegistryError::NotFound)?.adapter)
        };
        let source = adapter(draft.source_connection_id)?;
        let destination = adapter(draft.destination_connection_id)?;
        if source != crate::storage_access::ISPMANAGER
            || !matches!(
                destination.as_str(),
                crate::storage_access::LOCAL_FILESYSTEM | crate::storage_access::SMB_SHARE
            )
        {
            return Err(RegistryError::UnsupportedScenarioEndpoint);
        }
        Ok(())
    }
}

/// The destination root of the Files category: `<destination>/files`
/// (D-027 section 5.2). The Scenario stores this root; the other data types
/// use sibling folders of it.
pub fn backup_files_root(destination_root: &str) -> String {
    let base = destination_root.trim_end_matches('/');
    if base.ends_with("/files") {
        base.to_owned()
    } else {
        format!("{base}/files")
    }
}

fn insert_backup_revision(
    transaction: &rusqlite::Transaction<'_>,
    id: &str,
    revision: u64,
    resources: &[BackupResource],
) -> Result<()> {
    transaction.execute(
        "INSERT INTO scenario_revisions(
            scenario_id,revision,name,left_connection_id,left_root,
            right_connection_id,right_root,direction,deletion_mode,mode
         )
         SELECT id,?2,name,left_connection_id,left_root,
                right_connection_id,right_root,direction,deletion_mode,mode
         FROM scenarios WHERE id=?1",
        params![id, revision as i64],
    )?;
    for resource in resources {
        transaction.execute(
            "INSERT OR IGNORE INTO scenario_revision_resources(
                scenario_id,revision,category,provider_id,kind,rule
             ) VALUES(?1,?2,?3,?4,?5,?6)",
            params![
                id,
                revision as i64,
                resource.category,
                resource.provider_id,
                resource.kind,
                resource.rule
            ],
        )?;
    }
    Ok(())
}

fn insert_revision_selection(
    transaction: &rusqlite::Transaction<'_>,
    id: &str,
    revision: u64,
    selection: &[String],
) -> Result<()> {
    for path in selection {
        transaction.execute(
            "INSERT INTO scenario_revision_selection(scenario_id,revision,relative_path)
             VALUES(?1,?2,?3)",
            params![id, revision as i64, path],
        )?;
    }
    Ok(())
}

fn unix_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs() as i64)
}

fn valid_name(name: &str) -> bool {
    let trimmed = name.trim();
    !trimmed.is_empty() && trimmed.chars().count() <= 120 && !trimmed.chars().any(char::is_control)
}
/// Return `smb://host[:port]/share[/path]` without a trailing slash, or
/// `None`. An address with a login (`user@`), a domain (`domain;`), a query,
/// a fragment, or a control character is refused, so that no credential can
/// enter the Registry through the address.
fn normalized_smb_url(url: &str) -> Option<String> {
    let trimmed = url.trim();
    if trimmed.chars().count() > 1024 || trimmed.chars().any(char::is_control) {
        return None;
    }
    let scheme = trimmed.get(..6)?;
    if !scheme.eq_ignore_ascii_case("smb://") {
        return None;
    }
    let rest = &trimmed[6..];
    if rest.contains(['@', ';', '?', '#', '\\']) {
        return None;
    }
    let (authority, path) = rest.split_once('/')?;
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) if !host.contains(':') || host.ends_with(']') => (host, Some(port)),
        _ => (authority, None),
    };
    // A Bonjour service host, such as `My NAS._smb._tcp.local`, can have
    // spaces. It cannot start or end with one.
    if host.is_empty() || host.trim() != host {
        return None;
    }
    if let Some(port) = port {
        port.parse::<u16>().ok().filter(|port| *port > 0)?;
    }
    let segments: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
    if segments.is_empty() || segments.iter().any(|part| *part == "." || *part == "..") {
        return None;
    }
    Some(format!("smb://{}/{}", authority, segments.join("/")))
}

fn normalized_panel_url(url: &str) -> Result<String> {
    crate::ispmanager::PanelUrl::parse(url.trim())
        .map(|url| url.as_str().to_owned())
        .map_err(|_| RegistryError::InvalidPanelUrl)
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
    fn smb_share_keeps_only_a_credential_free_address() {
        let db = Registry::in_memory().unwrap();
        let id = db
            .add_smb_share(" NAS ", " SMB://nas.local/Media/Photos/ ")
            .unwrap();
        let entry = db.get(&id).unwrap().unwrap();
        assert_eq!(entry.adapter, "smb_share");
        assert_eq!(entry.name, "NAS");
        assert_eq!(
            entry.endpoint_url.as_deref(),
            Some("smb://nas.local/Media/Photos")
        );
        assert_eq!(entry.credential_ref, None);
        let second = db.add_smb_share("NAS 2", "smb://nas.local/Media").unwrap();
        let bonjour = db
            .add_smb_share("NAS 3", "smb://My NAS._smb._tcp.local/Общая папка")
            .unwrap();
        assert_eq!(
            db.get(&bonjour).unwrap().unwrap().endpoint_url.as_deref(),
            Some("smb://My NAS._smb._tcp.local/Общая папка")
        );
        assert_ne!(id, second);
        for bad in [
            "smb://user@nas/share",
            "smb://user:secret@nas/share",
            "smb://DOMAIN;user@nas/share",
            "smb://nas",
            "smb://nas/",
            "smb://nas/share?password=x",
            "smb://nas/share#x",
            "smb://nas/../share",
            "smb://nas:0/share",
            "afp://nas/share",
            "https://nas/share",
        ] {
            assert!(
                matches!(
                    db.add_smb_share("Bad", bad),
                    Err(RegistryError::InvalidEndpoint)
                ),
                "{bad}"
            );
        }
        db.set_smb_endpoint(&id, "smb://192.168.1.10:445/Backup")
            .unwrap();
        assert_eq!(
            db.get(&id).unwrap().unwrap().endpoint_url.as_deref(),
            Some("smb://192.168.1.10:445/Backup")
        );
        let local = db.add("local_filesystem", "Mac").unwrap();
        assert!(matches!(
            db.set_smb_endpoint(&local, "smb://nas/share"),
            Err(RegistryError::NotFound)
        ));
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
            vec![state.clone()]
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
        // The same connections and roots keep the evidence (`beecbb7`).
        let later = ScenarioReference {
            scenario_id: scenario_id.clone(),
            revision: 2,
        };
        let carried = db.list_confirmed_file_states(&later).unwrap();
        assert_eq!(carried.len(), 1);
        assert_eq!(carried[0].scenario, later);
        assert_eq!(carried[0].left, state.left);
        assert_eq!(carried[0].right, state.right);

        // Another root is other storage: the evidence stays with revision 2.
        db.update_scenario(
            &scenario_id,
            ScenarioUpdate {
                name: "Documents v3",
                left_connection_id: &left,
                left_root: "/documents",
                right_connection_id: &right,
                right_root: "/other-archive",
                direction: "left_to_right",
            },
        )
        .unwrap();
        let moved = ScenarioReference {
            scenario_id,
            revision: 3,
        };
        assert!(db.list_confirmed_file_states(&moved).unwrap().is_empty());
        assert_eq!(db.list_confirmed_file_states(&later).unwrap().len(), 1);
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
        // D-023: Yandex Disk is only the source of a one-way Scenario.
        let left = db.add("local_filesystem", "Left").unwrap();
        let right = db.add("yandex_disk", "Right").unwrap();
        let id = db
            .create_scenario(
                "Documents",
                &right,
                "/old",
                &left,
                "/archive",
                "left_to_right",
            )
            .unwrap();
        db.set_scenario_enabled(&id, false).unwrap();

        db.update_scenario(
            &id,
            ScenarioUpdate {
                name: "Updated documents",
                left_connection_id: &left,
                left_root: "/source",
                right_connection_id: &right,
                right_root: "/target",
                direction: "right_to_left",
            },
        )
        .unwrap();

        let scenario = db.list_scenarios().unwrap().pop().unwrap();
        assert_eq!(scenario.name, "Updated documents");
        assert_eq!(scenario.left_connection_id, left);
        assert_eq!(scenario.right_connection_id, right);
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
    fn this_mac_connection_is_created_once_and_then_reused() {
        let db = Registry::in_memory().unwrap();
        let first = db.ensure_local_computer("Этот Mac").unwrap();
        let again = db.ensure_local_computer("Другое имя").unwrap();
        assert_eq!(first, again);
        let local = db
            .list()
            .unwrap()
            .into_iter()
            .filter(|connection| connection.adapter == "local_filesystem")
            .collect::<Vec<_>>();
        assert_eq!(local.len(), 1);
        assert_eq!(local[0].name, "Этот Mac");
        assert!(matches!(
            db.ensure_local_computer("  "),
            Err(RegistryError::InvalidName)
        ));
    }

    #[test]
    fn selection_and_deletion_rule_belong_to_each_revision() {
        let db = Registry::in_memory().unwrap();
        let left = db.add("local_filesystem", "Left").unwrap();
        let right = db.add("local_filesystem", "Right").unwrap();
        let scope = ScenarioScope {
            selected_paths: vec!["Docs/a.txt".into(), "Docs".into(), "Photos".into()],
            deletion_mode: DeletionMode::Simple,
            excluded_paths: Vec::new(),
        };
        let id = db
            .create_scenario_with_scope(
                "Documents",
                &left,
                "/source",
                &right,
                "/target",
                "left_to_right",
                &scope,
            )
            .unwrap();
        assert_eq!(
            db.get_scenario_revision_scope(&id, 1).unwrap(),
            ScenarioScope {
                selected_paths: vec!["Docs".into(), "Photos".into()],
                deletion_mode: DeletionMode::Simple,
                excluded_paths: Vec::new(),
            }
        );

        db.update_scenario_with_scope(
            &id,
            ScenarioUpdate {
                name: "Documents",
                left_connection_id: &left,
                left_root: "/source",
                right_connection_id: &right,
                right_root: "/target",
                direction: "left_to_right",
            },
            &ScenarioScope::default(),
        )
        .unwrap();
        assert_eq!(
            db.get_scenario_revision_scope(&id, 2).unwrap(),
            ScenarioScope::default()
        );
        assert_eq!(
            db.get_scenario_revision_scope(&id, 1)
                .unwrap()
                .deletion_mode,
            DeletionMode::Simple
        );
        assert!(matches!(
            db.get_scenario_revision_scope(&id, 3),
            Err(RegistryError::NotFound)
        ));
    }

    #[test]
    fn unsafe_selection_is_refused_without_a_new_scenario() {
        let db = Registry::in_memory().unwrap();
        let left = db.add("local_filesystem", "Left").unwrap();
        let right = db.add("local_filesystem", "Right").unwrap();
        for path in ["../outside", "/absolute", ""] {
            let scope = ScenarioScope {
                selected_paths: vec![path.into()],
                deletion_mode: DeletionMode::Paranoid,
                excluded_paths: Vec::new(),
            };
            assert!(matches!(
                db.create_scenario_with_scope(
                    "Documents",
                    &left,
                    "/source",
                    &right,
                    "/target",
                    "left_to_right",
                    &scope,
                ),
                Err(RegistryError::InvalidSelection)
            ));
        }
        assert!(db.list_scenarios().unwrap().is_empty());
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
        // The list is ordered by local time.
        assert_eq!(schedules[0].timezone, "Europe/Berlin");
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
    fn removing_a_scenario_removes_its_records_and_waits_for_unresolved_work() {
        let mut db = Registry::in_memory().unwrap();
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
            .create_scenario_schedule(&scenario, "Europe/Samara", 0, 9, 30)
            .unwrap();
        assert!(db.claim_schedule_occurrence(&schedule, "slot").unwrap());
        let reference = ScenarioReference {
            scenario_id: scenario.clone(),
            revision: 1,
        };
        let batch = db
            .create_scenario_create_batch(&reference, &[("a.txt".into(), true)])
            .unwrap()
            .id;
        let run = db.begin_manual_run(&reference, "plan", &batch, 0).unwrap();

        // An active run keeps the Scenario.
        assert!(matches!(
            db.remove_scenario(&scenario),
            Err(RegistryError::ScenarioRunActive)
        ));
        db.finish_scenario_run(&run, "completed", None).unwrap();
        db.set_scenario_create_batch_child_state(&batch, "a.txt", "completed")
            .unwrap();

        // An unknown item keeps the Scenario: reconciliation needs its record.
        let other = db
            .create_scenario("Music", &left, "/music", &right, "/copy", "left_to_right")
            .unwrap();
        let other_batch = db
            .create_scenario_create_batch(
                &ScenarioReference {
                    scenario_id: other.clone(),
                    revision: 1,
                },
                &[("b.txt".into(), true)],
            )
            .unwrap()
            .id;
        db.set_scenario_create_batch_child_state(&other_batch, "b.txt", "unknown")
            .unwrap();
        assert!(matches!(
            db.remove_scenario(&other),
            Err(RegistryError::ScenarioHasUnresolvedItems)
        ));

        db.remove_scenario(&scenario).unwrap();
        assert_eq!(db.list_scenarios().unwrap().len(), 1);
        assert!(db.get_scenario_revision(&scenario, 1).unwrap().is_none());
        assert!(matches!(
            db.remove_scenario(&scenario),
            Err(RegistryError::NotFound)
        ));
        // The connections stay; only the Scenario records are gone.
        assert_eq!(db.list().unwrap().len(), 2);
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
            due_at: utc("2026-10-25T00:30:00Z"),
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
        db.create_scenario("Cloud", &cloud, "/data", &left, "/data", "left_to_right")
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

    fn started_run(start: ScheduleRunStart) -> String {
        match start {
            ScheduleRunStart::Started(id) => id,
            ScheduleRunStart::Skipped(active) => panic!("skipped for {active}"),
        }
    }

    /// Xsoid/QADREN#30: hourly slots during a run of three hours give one
    /// run and two skipped slots. The next slot after the run starts a new
    /// run, with no follow-up run in between.
    #[test]
    fn slots_during_an_active_run_are_skipped_without_a_follow_up_run() {
        let db = Registry::in_memory().unwrap();
        let scenario = schedule_fixture(&db, "hourly");
        for hour in [10, 11, 12] {
            db.create_scenario_schedule(&scenario, "UTC", 0, hour, 0)
                .unwrap();
        }
        let stage = |from: &str, through: &str| {
            db.stage_due_schedule_occurrences(schedule_utc(from), schedule_utc(through))
                .unwrap()
        };
        assert_eq!(stage("2026-10-05T09:59:00Z", "2026-10-05T10:00:00Z"), 1);
        let first_job = db.pending_schedule_job_outbox().unwrap()[0].job_id.clone();
        let run = started_run(db.begin_schedule_run(&first_job).unwrap());
        assert_eq!(
            db.begin_schedule_run(&first_job).unwrap(),
            ScheduleRunStart::Started(run.clone())
        );

        assert_eq!(stage("2026-10-05T10:59:00Z", "2026-10-05T11:00:00Z"), 0);
        assert_eq!(stage("2026-10-05T11:59:00Z", "2026-10-05T12:00:00Z"), 0);
        let skipped = db.skipped_schedule_slots(&scenario, 10).unwrap();
        assert_eq!(skipped.len(), 2);
        assert!(skipped.iter().all(|(_, _, active)| *active == run));
        assert_eq!(db.pending_schedule_job_outbox().unwrap().len(), 1);

        db.finish_scenario_run(&run, "completed", None).unwrap();
        assert_eq!(db.active_scenario_run(&scenario).unwrap(), None);
        // The skipped slots stay skipped: no follow-up run.
        assert_eq!(stage("2026-10-05T12:00:00Z", "2026-10-12T09:59:00Z"), 0);
        assert_eq!(stage("2026-10-12T09:59:00Z", "2026-10-12T10:00:00Z"), 1);
        let next_job = db
            .pending_schedule_job_outbox()
            .unwrap()
            .into_iter()
            .find(|entry| entry.job_id != first_job)
            .unwrap()
            .job_id;
        let next = started_run(db.begin_schedule_run(&next_job).unwrap());
        assert_ne!(next, run);
    }

    /// The database, not only code, allows one active run for each
    /// Scenario. A slot whose run starts after a manual run stays skipped.
    #[test]
    fn due_children_take_each_batch_in_turn() {
        let mut db = Registry::in_memory().unwrap();
        let mac = db.add("local_filesystem", "Mac").unwrap();
        let mut batches = Vec::new();
        for (name, files) in [("Large", 50), ("Small", 2)] {
            let scenario = db
                .create_scenario(
                    name,
                    &mac,
                    &format!("/{name}/a"),
                    &mac,
                    &format!("/{name}/b"),
                    "left_to_right",
                )
                .unwrap();
            let reference = ScenarioReference {
                scenario_id: scenario,
                revision: 1,
            };
            let batch = db.begin_scenario_create_batch(&reference).unwrap();
            let children: Vec<(String, bool)> = (0..files)
                .map(|index| (format!("{index:03}.txt"), true))
                .collect();
            db.append_scenario_create_batch_children(&batch, &children)
                .unwrap();
            db.set_scenario_create_batch_paused(&batch, false).unwrap();
            batches.push(batch);
        }
        // The large batch was created first, but the small one is not starved.
        let due = db.due_scenario_create_batch_children(0, 4).unwrap();
        assert_eq!(
            due.iter()
                .map(|child| child.batch_id.as_str())
                .collect::<Vec<_>>(),
            [&batches[0], &batches[1], &batches[0], &batches[1]]
        );
        db.set_scenario_create_batch_paused(&batches[0], true)
            .unwrap();
        assert!(db
            .due_scenario_create_batch_children(0, 4)
            .unwrap()
            .iter()
            .all(|child| child.batch_id == batches[1]));
    }

    #[test]
    fn a_run_waits_for_the_write_root_lock_of_an_overlapping_run() {
        let mut db = Registry::in_memory().unwrap();
        let mac = db.add("local_filesystem", "Mac").unwrap();
        let base = std::env::temp_dir().join("qadren-lock-test");
        let path = |name: &str| base.join(name).to_string_lossy().into_owned();
        let create = |name: &str, source: &str, destination: &str| {
            db.create_scenario(
                name,
                &mac,
                &path(source),
                &mac,
                &path(destination),
                "left_to_right",
            )
            .unwrap()
        };
        let photos = create("Photos", "photos", "backup");
        let music = create("Music", "music", "backup/music");
        let other = create("Other", "photos", "elsewhere");
        fn run(db: &mut Registry, scenario: &str) -> String {
            let reference = ScenarioReference {
                scenario_id: scenario.to_owned(),
                revision: 1,
            };
            let batch = db.begin_scenario_create_batch(&reference).unwrap();
            db.begin_manual_run(&reference, "plan", &batch, 0).unwrap()
        }
        let first = run(&mut db, &photos);
        let second = run(&mut db, &music);
        let third = run(&mut db, &other);
        assert_eq!(
            db.acquire_scenario_run_write_lock(&first).unwrap(),
            RunWriteLock::Acquired
        );
        assert_eq!(
            db.acquire_scenario_run_write_lock(&second).unwrap(),
            RunWriteLock::WaitFor {
                scenario_id: photos.clone()
            }
        );
        assert_eq!(
            db.scenario_run(&second)
                .unwrap()
                .unwrap()
                .waiting_for_scenario_id,
            Some(photos.clone())
        );
        // Both read "photos", but they write to different folders.
        assert_eq!(
            db.acquire_scenario_run_write_lock(&third).unwrap(),
            RunWriteLock::Acquired
        );
        // The lock is durable: a Worker restart keeps it.
        assert_eq!(
            db.acquire_scenario_run_write_lock(&first).unwrap(),
            RunWriteLock::Acquired
        );
        db.finish_scenario_run(&first, "completed", None).unwrap();
        assert_eq!(
            db.acquire_scenario_run_write_lock(&second).unwrap(),
            RunWriteLock::Acquired
        );
        assert_eq!(
            db.scenario_run(&second)
                .unwrap()
                .unwrap()
                .waiting_for_scenario_id,
            None
        );
        let candidate = db.get_scenario_revision(&music, 1).unwrap().unwrap();
        assert_eq!(
            db.overlapping_scenarios(&candidate, Some(&music))
                .unwrap()
                .into_iter()
                .map(|scenario| scenario.name)
                .collect::<Vec<_>>(),
            ["Photos"]
        );
    }

    #[test]
    fn one_active_run_per_scenario_is_a_database_constraint() {
        let mut db = Registry::in_memory().unwrap();
        let scenario = schedule_fixture(&db, "constraint");
        db.create_scenario_schedule(&scenario, "UTC", 0, 10, 0)
            .unwrap();
        db.stage_due_schedule_occurrences(
            schedule_utc("2026-10-05T09:59:00Z"),
            schedule_utc("2026-10-05T10:00:00Z"),
        )
        .unwrap();
        let job = db.pending_schedule_job_outbox().unwrap()[0].job_id.clone();
        let reference = ScenarioReference {
            scenario_id: scenario.clone(),
            revision: 1,
        };
        let batch = db.begin_scenario_create_batch(&reference).unwrap();
        let manual = db.begin_manual_run(&reference, "plan", &batch, 0).unwrap();

        assert!(matches!(
            db.begin_manual_run(&reference, "plan", &batch, 0),
            Err(RegistryError::ScenarioRunActive)
        ));
        assert!(db
            .db
            .execute(
                "INSERT INTO scenario_runs(id,scenario_id,scenario_revision,trigger,state,created_at)
                 VALUES('direct',?1,1,'manual','planning',0)",
                [&scenario],
            )
            .is_err());
        assert_eq!(
            db.begin_schedule_run(&job).unwrap(),
            ScheduleRunStart::Skipped(manual.clone())
        );
        db.finish_scenario_run(&manual, "completed", None).unwrap();
        assert_eq!(
            db.begin_schedule_run(&job).unwrap(),
            ScheduleRunStart::Skipped(manual)
        );
        assert_eq!(db.active_scenario_run(&scenario).unwrap(), None);
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
    fn migration_26_turns_a_weekly_row_into_one_day_without_a_repeated_slot() {
        let db = Registry::in_memory().unwrap();
        let left = db.add("local_filesystem", "Left").unwrap();
        let right = db.add("local_filesystem", "Right").unwrap();
        let scenario = db
            .create_scenario(
                "Weekly",
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
        let key = "2026-10-25 02:30@Europe/Berlin";
        assert!(db.claim_schedule_occurrence(&schedule, key).unwrap());
        // Make the version 25 table: weekday only.
        let connection = db.db;
        connection
            .execute_batch(
                "ALTER TABLE scenario_schedules DROP COLUMN kind;
                 ALTER TABLE scenario_schedules DROP COLUMN days_mask;
                 ALTER TABLE scenario_schedules DROP COLUMN interval_hours;
                 ALTER TABLE scenario_schedules DROP COLUMN anchor_utc;
                 ALTER TABLE scenario_runs DROP COLUMN write_lock;
                 ALTER TABLE scenario_runs DROP COLUMN waiting_for_scenario_id;
                 PRAGMA user_version = 25;",
            )
            .unwrap();
        let db = Registry::initialize(connection).unwrap();
        let migrated = &db.list_scenario_schedules(&scenario).unwrap()[0];
        assert_eq!(
            (
                migrated.kind,
                migrated.days_mask,
                migrated.hour,
                migrated.minute
            ),
            (ScheduleKind::DaysAtTime, 1 << 6, 2, 30)
        );
        let after: DateTime<Utc> = "2026-10-24T00:00:00Z".parse().unwrap();
        let through: DateTime<Utc> = "2026-11-02T00:00:00Z".parse().unwrap();
        let due = db.due_schedule_occurrences(after, through).unwrap();
        assert_eq!(
            due.iter()
                .map(|slot| slot.occurrence_key.0.as_str())
                .collect::<Vec<_>>(),
            [key, "2026-11-01 02:30@Europe/Berlin"]
        );
        // The slot claimed before the migration is not staged again.
        assert_eq!(
            db.stage_due_schedule_occurrences(after, through).unwrap(),
            1
        );
        assert_eq!(
            db.stage_due_schedule_occurrences(after, through).unwrap(),
            0
        );
    }

    #[test]
    fn every_hours_schedule_stages_each_slot_once_across_restarts() {
        let path = std::env::temp_dir().join(format!(
            "qadren-every-hours-{}.sqlite3",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let db = Registry::open(&path).unwrap();
        let left = db.add("local_filesystem", "Left").unwrap();
        let right = db.add("local_filesystem", "Right").unwrap();
        let scenario = db
            .create_scenario(
                "Hourly",
                &left,
                "/source",
                &right,
                "/target",
                "left_to_right",
            )
            .unwrap();
        let created: DateTime<Utc> = "2026-10-20T10:00:00Z".parse().unwrap();
        let schedule = db
            .create_scenario_hours_schedule_at(
                &scenario,
                "Europe/Berlin",
                6,
                0,
                0,
                crate::scheduler::ALL_DAYS,
                created,
            )
            .unwrap();
        let listed = &db.list_scenario_schedules(&scenario).unwrap()[0];
        assert_eq!(listed.id, schedule);
        assert_eq!(
            listed.kind,
            ScheduleKind::EveryHours {
                interval_hours: 6,
                anchor: "2026-10-19T22:00:00Z".parse().unwrap()
            }
        );
        for (interval, days) in [(0, 0x7f), (25, 0x7f), (6, 0)] {
            assert!(matches!(
                db.create_scenario_hours_schedule_at(
                    &scenario,
                    "Europe/Berlin",
                    interval,
                    0,
                    0,
                    days,
                    created
                ),
                Err(RegistryError::InvalidSchedule)
            ));
        }
        // A sleep across the DST change: one catch-up run, each slot claimed once.
        let after: DateTime<Utc> = "2026-10-24T12:00:00Z".parse().unwrap();
        let through: DateTime<Utc> = "2026-10-25T12:00:00Z".parse().unwrap();
        let due = db.due_schedule_occurrences(after, through).unwrap();
        assert_eq!(due.len(), 4);
        assert_eq!(
            db.stage_due_schedule_occurrences(after, through).unwrap(),
            1
        );
        drop(db);
        let db = Registry::open(&path).unwrap();
        assert_eq!(
            db.stage_due_schedule_occurrences(after, through).unwrap(),
            0
        );
        assert_eq!(
            db.last_schedule_slot(&schedule).unwrap(),
            Some(("every-6h@2026-10-25T10:00Z".to_owned(), false))
        );
        drop(db);
        std::fs::remove_file(path).unwrap();
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
