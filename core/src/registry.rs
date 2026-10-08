//! Persistent metadata-only storage connection registry.
//! Never stores credentials, tokens, file content or remote delete requests.
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
        if version > 2 {
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
            db.execute_batch("BEGIN IMMEDIATE;
                ALTER TABLE storage_connections ADD COLUMN provider_account_id TEXT;
                PRAGMA user_version = 2;
                COMMIT;")?;
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
        assert_eq!(db.get(&a).unwrap().unwrap().provider_account_id.as_deref(), Some("account-123"));
        assert_eq!(db.get(&b).unwrap().unwrap().provider_account_id.as_deref(), Some("account-123"));
        db.remove(&a).unwrap();
        assert!(db.get(&b).unwrap().is_some());
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
