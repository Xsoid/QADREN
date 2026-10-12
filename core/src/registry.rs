//! Persistent metadata-only storage connection registry.
//! Never stores credentials, tokens, file content or remote delete requests.
use crate::automation::{AutomaticTriggerResult, ScenarioAutomationState};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;

#[derive(Debug)]
pub enum RegistryError {
    Database(rusqlite::Error),
    InvalidName,
    InvalidAdapter,
    UnsupportedSchema(i32),
    IntegrityFailed,
    NotFound,
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

/// Persisted Scenario metadata. It stores configuration only: never file content,
/// credentials, a transfer plan, or a confirmed FileSyncState.
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
        if version > 5 {
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

    fn changed(&self, count: usize) -> Result<()> {
        if count == 0 {
            Err(RegistryError::NotFound)
        } else {
            Ok(())
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

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
            .create_scenario("Documents", &left, "/", &right, "/archive", "left_to_right")
            .unwrap();
        let scenario = db.list_scenarios().unwrap().pop().unwrap();
        assert_eq!(scenario.id, id);
        assert_eq!(scenario.revision, 1);
        assert!(scenario.enabled);
        db.set_scenario_enabled(&id, false).unwrap();
        assert!(!db.list_scenarios().unwrap()[0].enabled);
        assert!(matches!(db.remove(&left), Err(RegistryError::Database(_))));
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
            .create_scenario("Documents", &left, "/", &right, "/archive", "left_to_right")
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
