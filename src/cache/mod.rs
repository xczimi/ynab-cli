mod store;

use std::path::{Path, PathBuf};

use rusqlite::Connection;
use secrecy::{ExposeSecret, SecretString};

use crate::error::{Error, Result};
use crate::secrets::{SecretKind, SecretStore};

pub struct Cache {
    conn: Connection,
}

impl Cache {
    pub fn data_dir() -> Result<PathBuf> {
        if let Ok(dir) = std::env::var("YNAB_CLI_DATA_DIR") {
            return Ok(PathBuf::from(dir));
        }
        directories::ProjectDirs::from("", "", "ynab-cli")
            .map(|d| d.data_dir().to_path_buf())
            .ok_or_else(|| Error::Cache("cannot determine data directory".into()))
    }

    pub fn db_path() -> Result<PathBuf> {
        Ok(Self::data_dir()?.join("cache.db"))
    }

    pub fn open(store: &SecretStore) -> Result<Cache> {
        let path = Self::db_path()?;
        Self::open_at(store, &path)
    }

    /// A cache that cannot be opened or decrypted is discarded and rebuilt —
    /// never a user-facing error (CLAUDE.md).
    pub fn open_at(store: &SecretStore, path: &Path) -> Result<Cache> {
        let key = Self::resolve_key(store)?;
        match Self::try_open(path, &key) {
            Ok(conn) => Ok(Cache { conn }),
            Err(_) => {
                let _ = std::fs::remove_file(path);
                let conn = Self::try_open(path, &key).map_err(|e| Error::Cache(e.to_string()))?;
                Ok(Cache { conn })
            }
        }
    }

    fn try_open(path: &Path, key: &SecretString) -> rusqlite::Result<Connection> {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "key", format!("x'{}'", key.expose_secret()))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS sync_state (
               budget_id TEXT NOT NULL,
               resource  TEXT NOT NULL,
               server_knowledge INTEGER NOT NULL,
               PRIMARY KEY (budget_id, resource)
             );
             CREATE TABLE IF NOT EXISTS entities (
               budget_id TEXT NOT NULL,
               resource  TEXT NOT NULL,
               id        TEXT NOT NULL,
               json      TEXT NOT NULL,
               PRIMARY KEY (budget_id, resource, id)
             );",
        )?;
        Ok(conn)
    }

    /// Key sources: YNAB_CLI_CACHE_KEY env (read-only, tests/power users) →
    /// keychain → generate-and-store. The key never touches disk.
    fn resolve_key(store: &SecretStore) -> Result<SecretString> {
        if let Ok(k) = std::env::var("YNAB_CLI_CACHE_KEY") {
            let trimmed = k.trim();
            if !trimmed.is_empty() {
                return Ok(SecretString::from(trimmed.to_string()));
            }
        }
        if let Some(key) = store.get(SecretKind::CacheKey)? {
            return Ok(key);
        }
        let mut bytes = [0u8; 32];
        rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut bytes);
        let hex_key = hex::encode(bytes);
        store.set(SecretKind::CacheKey, SecretString::from(hex_key.clone()))?;
        Ok(SecretString::from(hex_key))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::secrets::SecretStore;

    // `resolve_key` reads `YNAB_CLI_CACHE_KEY` via plain `std::env::var`, which
    // is process-global state. `temp_env::with_var` only serializes against
    // other `temp_env` callers, not against that plain read, so a test that
    // temporarily sets the var can leak it into an unrelated test running on
    // another thread under cargo's default parallel test runner. Serialize all
    // cache tests (they all resolve a key) against each other to avoid that.
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    fn mock_store() -> SecretStore {
        keyring::set_default_credential_builder(keyring::mock::default_credential_builder());
        SecretStore::new().unwrap()
    }

    #[test]
    fn open_generates_key_and_roundtrips() {
        let _guard = TEST_LOCK.lock().unwrap();
        let store = mock_store();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.db");
        {
            let mut cache = Cache::open_at(&store, &path).unwrap();
            cache
                .upsert_entities(
                    "b-1",
                    "accounts",
                    &[(
                        "a-1".into(),
                        serde_json::json!({"id": "a-1", "name": "Chequing"}),
                    )],
                )
                .unwrap();
            cache.set_server_knowledge("b-1", "accounts", 42).unwrap();
        }
        // reopen with the same store: key is reused, data decrypts
        let cache = Cache::open_at(&store, &path).unwrap();
        assert_eq!(cache.server_knowledge("b-1", "accounts").unwrap(), Some(42));
        let loaded = cache
            .load_entities("b-1", "accounts", Some("$.name"))
            .unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0]["name"], "Chequing");
    }

    #[test]
    fn encrypted_on_disk() {
        let _guard = TEST_LOCK.lock().unwrap();
        let store = mock_store();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.db");
        {
            let mut cache = Cache::open_at(&store, &path).unwrap();
            cache
                .upsert_entities(
                    "b-1",
                    "payees",
                    &[(
                        "p-1".into(),
                        serde_json::json!({"id": "p-1", "name": "SecretGrocer"}),
                    )],
                )
                .unwrap();
        }
        let bytes = std::fs::read(&path).unwrap();
        let hay = String::from_utf8_lossy(&bytes);
        assert!(!hay.contains("SecretGrocer"));
        assert!(!hay.contains("SQLite format 3")); // encrypted header
    }

    #[test]
    fn corrupted_file_is_silently_discarded() {
        let _guard = TEST_LOCK.lock().unwrap();
        let store = mock_store();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.db");
        std::fs::write(&path, b"this is not a database").unwrap();
        let cache = Cache::open_at(&store, &path).unwrap();
        assert_eq!(cache.server_knowledge("b-1", "accounts").unwrap(), None);
    }

    #[test]
    fn undecryptable_file_is_silently_discarded() {
        let _guard = TEST_LOCK.lock().unwrap();
        let store = mock_store();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.db");
        {
            let c1 = Cache::open_at(&store, &path).unwrap();
            c1.set_server_knowledge("b-1", "accounts", 7).unwrap();
        }
        // wrong key: fresh mock store has no CacheKey, generates a new one
        let other_store = mock_store();
        let cache = Cache::open_at(&other_store, &path).unwrap();
        assert_eq!(cache.server_knowledge("b-1", "accounts").unwrap(), None);
    }

    #[test]
    fn replace_and_ordering() {
        let _guard = TEST_LOCK.lock().unwrap();
        let store = mock_store();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.db");
        let mut cache = Cache::open_at(&store, &path).unwrap();
        cache
            .replace_entities(
                "b-1",
                "category_groups",
                &[
                    (
                        "g-2".into(),
                        serde_json::json!({"id": "g-2", "name": "Zed"}),
                    ),
                    (
                        "g-1".into(),
                        serde_json::json!({"id": "g-1", "name": "Alpha"}),
                    ),
                ],
            )
            .unwrap();
        // rowid order preserves insertion (API) order
        let loaded = cache.load_entities("b-1", "category_groups", None).unwrap();
        assert_eq!(loaded[0]["id"], "g-2");
        // replace wholesale drops old rows
        cache
            .replace_entities(
                "b-1",
                "category_groups",
                &[(
                    "g-9".into(),
                    serde_json::json!({"id": "g-9", "name": "Only"}),
                )],
            )
            .unwrap();
        let loaded = cache.load_entities("b-1", "category_groups", None).unwrap();
        assert_eq!(loaded.len(), 1);
        // date ordering for delta-merged resources
        cache
            .upsert_entities(
                "b-1",
                "transactions",
                &[
                    (
                        "t-2".into(),
                        serde_json::json!({"id": "t-2", "date": "2026-07-20"}),
                    ),
                    (
                        "t-1".into(),
                        serde_json::json!({"id": "t-1", "date": "2026-07-01"}),
                    ),
                ],
            )
            .unwrap();
        let loaded = cache
            .load_entities("b-1", "transactions", Some("$.date"))
            .unwrap();
        assert_eq!(loaded[0]["id"], "t-1");
    }

    #[test]
    fn env_key_overrides_keychain() {
        let _guard = TEST_LOCK.lock().unwrap();
        // Uses a store with NO key; env key must be used and NOT written back.
        let store = mock_store();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.db");
        temp_env::with_var("YNAB_CLI_CACHE_KEY", Some("aa".repeat(32)), || {
            let _cache = Cache::open_at(&store, &path).unwrap();
            assert!(
                store
                    .get(crate::secrets::SecretKind::CacheKey)
                    .unwrap()
                    .is_none()
            );
        });
    }
}
