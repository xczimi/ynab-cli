# M3: Encrypted Delta Cache Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** An SQLCipher-encrypted, delta-request cache (on by default) behind the four budget-scoped list commands, plus `ynab cache status|clear` and `--no-cache`. A corrupted or undecryptable cache is silently discarded and refetched, never a user-facing error.

**Architecture:** `src/cache/` owns storage (rusqlite + bundled SQLCipher; 32-byte random key generated on first run, held in the OS keychain as `SecretKind::CacheKey`) and sync (delta fetch → upsert → reconstruct an as-if-full-fetch envelope `{"<resource>": [...], "server_knowledge": N}` that flows into the existing `ListResult` shape, so command rendering and filtering are unchanged). Entities are stored as raw JSON per id — the `--json` mirror guarantee survives caching.

**Locked decisions (user-approved 2026-08-02):** filters stay in-memory over cached data (CLAUDE.md amended); `last-used` bypasses the cache; categories are full-fetched and replaced wholesale; `budgets list` is never cached. New test/power-user hook: `YNAB_CLI_CACHE_KEY` env var (hex key, read-only) mirrors the `YNAB_PAT` pattern, plus `YNAB_CLI_DATA_DIR` for the DB location.

**Tech Stack:** adds `rusqlite` (features `["bundled-sqlcipher-vendored-openssl"]`), `rand`, `hex`.

## Global Constraints

- **Read-only is structural**: no new HTTP verbs; delta params ride the existing GET helpers.
- **Nothing sensitive touches disk unencrypted**: the DB is SQLCipher-encrypted with a keychain-held key; the key itself never touches disk. Cache never stores tokens.
- **Corruption is never the user's problem**: any failure opening/decrypting the DB ⇒ delete file, recreate, refetch silently.
- **`--json` mirrors the API schema**: cached-mode output is the reconstructed envelope of raw entity JSON — unknown fields preserved.
- **Tests never touch the real keychain, network, config dir, or data dir** (mock keyring builder; wiremock; `YNAB_CLI_CONFIG_DIR`/`YNAB_CLI_DATA_DIR`/`YNAB_PAT`/`YNAB_CLI_CACHE_KEY` env hooks for binary e2e).
- **No `.unwrap()`/`.expect()` on fallible paths in non-test code.**
- **Commits**: conventional format, no attribution trailers. Gate: `cargo build`, `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` clean at milestone end.

## Execution notes

- Branch: `m3-encrypted-cache` off `main` (Task 1 Step 1).
- DAG: Tasks 1 and 2 in parallel → Task 3 → Tasks 4 and 5 in parallel.
- Worktree agents fork from main: FIRST `git reset --hard <controller-given SHA>`, verify a marker file, then work.
- rusqlite version: the plan says `0.32`; if cargo rejects it or a newer minor is current, use the latest stable version and note it — the API surface used here (Connection::open, pragma_update, execute, prepare/query_map, execute_batch, transaction) is stable.

## File Structure

```
src/cache/mod.rs     — Cache struct: paths, key mgmt, open-with-corruption-discard (Task 1)
src/cache/store.rs   — schema + upsert/replace/load/sync-state ops (Task 1)
src/cache/sync.rs    — per-resource sync: delta fetch → merge → envelope (Task 3)
src/api/client.rs    — last_knowledge_of_server params on 4 endpoints (Task 2)
src/cli/context.rs   — Ctx gains cache handle; policy wiring (Task 4)
src/cli/{accounts,categories,payees,transactions}.rs — cache-aware fetch (Task 4)
src/cli/cache_cmd.rs — cache status|clear (Task 5)
src/cli/mod.rs       — --no-cache global, Cache command (Tasks 4, 5)
tests/cli_cache.rs   — binary e2e: delta round-trip, cache status/clear (Tasks 4, 5)
```

---

### Task 1: Cache storage core (SQLCipher, key management, corruption discard)

**Files:**
- Create: `src/cache/mod.rs`, `src/cache/store.rs`
- Modify: `src/lib.rs` (add `pub mod cache;`), `Cargo.toml`, `src/error.rs`

**Interfaces:**
- Consumes: `secrets::{SecretStore, SecretKind}` (M1), `error::{Error, Result}`.
- Produces:
  - `error::Error::Cache(String)` displaying as `cache error: {0}`.
  - `cache::Cache` with:
    - `Cache::data_dir() -> Result<PathBuf>` — env `YNAB_CLI_DATA_DIR` override, else `directories::ProjectDirs::from("", "", "ynab-cli").data_dir()`.
    - `Cache::db_path() -> Result<PathBuf>` — `data_dir()/cache.db`.
    - `Cache::open(store: &SecretStore) -> Result<Cache>` and `Cache::open_at(store: &SecretStore, path: &Path) -> Result<Cache>` (tests use `open_at`). Key from `get_or_create_key`; on ANY open/decrypt/schema failure: delete the file and open fresh (silent discard). Only a second consecutive failure surfaces as `Error::Cache`.
    - key resolution: env `YNAB_CLI_CACHE_KEY` (non-empty, read-only) → keychain `SecretKind::CacheKey` → generate 32 random bytes (`rand::rngs::OsRng`), hex-encode, store in keychain, use.
  - In `store.rs` (all `impl Cache`, all rusqlite errors mapped `.map_err(|e| Error::Cache(e.to_string()))`):
    - `server_knowledge(&self, budget: &str, resource: &str) -> Result<Option<i64>>`
    - `set_server_knowledge(&self, budget: &str, resource: &str, sk: i64) -> Result<()>` (upsert)
    - `upsert_entities(&mut self, budget: &str, resource: &str, items: &[(String, serde_json::Value)]) -> Result<()>` (single transaction; INSERT OR REPLACE)
    - `replace_entities(&mut self, budget: &str, resource: &str, items: &[(String, serde_json::Value)]) -> Result<()>` (DELETE for (budget,resource) then insert in order, single transaction)
    - `load_entities(&self, budget: &str, resource: &str, order_json_field: Option<&str>) -> Result<Vec<serde_json::Value>>` — `Some("$.date")` ⇒ `ORDER BY json_extract(json, '$.date'), id`; `None` ⇒ `ORDER BY rowid` (insertion order — used by categories after wholesale replace).
    - `status_rows(&self) -> Result<Vec<(String, String, i64, i64)>>` — (budget_id, resource, server_knowledge, entity count) per sync_state row.

**Schema (created in open):**

```sql
CREATE TABLE IF NOT EXISTS sync_state (
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
);
```

- [ ] **Step 1: Create the branch**

```bash
git checkout -b m3-encrypted-cache
```

- [ ] **Step 2: Add dependencies**

```toml
rusqlite = { version = "0.32", features = ["bundled-sqlcipher-vendored-openssl"] }
rand = "0.8"
hex = "0.4"
```

Add to `src/error.rs` after `Decode`:

```rust
    #[error("cache error: {0}")]
    Cache(String),
```

- [ ] **Step 3: Write the failing tests**

`src/cache/mod.rs` test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::SecretStore;

    fn mock_store() -> SecretStore {
        keyring::set_default_credential_builder(keyring::mock::default_credential_builder());
        SecretStore::new().unwrap()
    }

    #[test]
    fn open_generates_key_and_roundtrips() {
        let store = mock_store();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.db");
        {
            let mut cache = Cache::open_at(&store, &path).unwrap();
            cache
                .upsert_entities(
                    "b-1",
                    "accounts",
                    &[("a-1".into(), serde_json::json!({"id": "a-1", "name": "Chequing"}))],
                )
                .unwrap();
            cache.set_server_knowledge("b-1", "accounts", 42).unwrap();
        }
        // reopen with the same store: key is reused, data decrypts
        let cache = Cache::open_at(&store, &path).unwrap();
        assert_eq!(cache.server_knowledge("b-1", "accounts").unwrap(), Some(42));
        let loaded = cache.load_entities("b-1", "accounts", Some("$.name")).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0]["name"], "Chequing");
    }

    #[test]
    fn encrypted_on_disk() {
        let store = mock_store();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.db");
        {
            let mut cache = Cache::open_at(&store, &path).unwrap();
            cache
                .upsert_entities(
                    "b-1",
                    "payees",
                    &[("p-1".into(), serde_json::json!({"id": "p-1", "name": "SecretGrocer"}))],
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
        let store = mock_store();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.db");
        std::fs::write(&path, b"this is not a database").unwrap();
        let cache = Cache::open_at(&store, &path).unwrap();
        assert_eq!(cache.server_knowledge("b-1", "accounts").unwrap(), None);
    }

    #[test]
    fn undecryptable_file_is_silently_discarded() {
        let store = mock_store();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.db");
        {
            let mut c1 = Cache::open_at(&store, &path).unwrap();
            c1.set_server_knowledge("b-1", "accounts", 7).unwrap();
        }
        // wrong key: fresh mock store has no CacheKey, generates a new one
        let other_store = mock_store();
        let cache = Cache::open_at(&other_store, &path).unwrap();
        assert_eq!(cache.server_knowledge("b-1", "accounts").unwrap(), None);
    }

    #[test]
    fn replace_and_ordering() {
        let store = mock_store();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.db");
        let mut cache = Cache::open_at(&store, &path).unwrap();
        cache
            .replace_entities(
                "b-1",
                "category_groups",
                &[
                    ("g-2".into(), serde_json::json!({"id": "g-2", "name": "Zed"})),
                    ("g-1".into(), serde_json::json!({"id": "g-1", "name": "Alpha"})),
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
                &[("g-9".into(), serde_json::json!({"id": "g-9", "name": "Only"}))],
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
                    ("t-2".into(), serde_json::json!({"id": "t-2", "date": "2026-07-20"})),
                    ("t-1".into(), serde_json::json!({"id": "t-1", "date": "2026-07-01"})),
                ],
            )
            .unwrap();
        let loaded = cache.load_entities("b-1", "transactions", Some("$.date")).unwrap();
        assert_eq!(loaded[0]["id"], "t-1");
    }

    #[test]
    fn env_key_overrides_keychain() {
        // Uses a store with NO key; env key must be used and NOT written back.
        let store = mock_store();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.db");
        temp_env::with_var("YNAB_CLI_CACHE_KEY", Some("aa".repeat(32)), || {
            let _cache = Cache::open_at(&store, &path).unwrap();
            assert!(store.get(crate::secrets::SecretKind::CacheKey).unwrap().is_none());
        });
    }
}
```

Add `temp-env = "0.3"` to `[dev-dependencies]` (scoped env-var testing; avoids the M2 carry-over's env-sensitivity problem).

- [ ] **Step 4: Run tests to verify they fail**

Run: `cargo test cache`
Expected: FAIL to compile.

- [ ] **Step 5: Write minimal implementation**

`src/cache/mod.rs`:

```rust
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
                let conn =
                    Self::try_open(path, &key).map_err(|e| Error::Cache(e.to_string()))?;
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
```

(Note: `hex_key.clone()` briefly holds the key in a plain String — same tradeoff the M2 carry-over notes for tokens; zeroize polish lands in M4.)

`src/cache/store.rs`:

```rust
use rusqlite::params;

use crate::cache::Cache;
use crate::error::{Error, Result};

fn db_err(e: rusqlite::Error) -> Error {
    Error::Cache(e.to_string())
}

impl Cache {
    pub fn server_knowledge(&self, budget: &str, resource: &str) -> Result<Option<i64>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT server_knowledge FROM sync_state
                 WHERE budget_id = ?1 AND resource = ?2",
            )
            .map_err(db_err)?;
        let mut rows = stmt
            .query_map(params![budget, resource], |row| row.get::<_, i64>(0))
            .map_err(db_err)?;
        match rows.next() {
            Some(v) => Ok(Some(v.map_err(db_err)?)),
            None => Ok(None),
        }
    }

    pub fn set_server_knowledge(&self, budget: &str, resource: &str, sk: i64) -> Result<()> {
        self.conn
            .execute(
                "INSERT OR REPLACE INTO sync_state (budget_id, resource, server_knowledge)
                 VALUES (?1, ?2, ?3)",
                params![budget, resource, sk],
            )
            .map_err(db_err)?;
        Ok(())
    }

    pub fn upsert_entities(
        &mut self,
        budget: &str,
        resource: &str,
        items: &[(String, serde_json::Value)],
    ) -> Result<()> {
        let tx = self.conn.transaction().map_err(db_err)?;
        for (id, value) in items {
            let text = serde_json::to_string(value).map_err(|e| Error::Cache(e.to_string()))?;
            tx.execute(
                "INSERT OR REPLACE INTO entities (budget_id, resource, id, json)
                 VALUES (?1, ?2, ?3, ?4)",
                params![budget, resource, id, text],
            )
            .map_err(db_err)?;
        }
        tx.commit().map_err(db_err)
    }

    pub fn replace_entities(
        &mut self,
        budget: &str,
        resource: &str,
        items: &[(String, serde_json::Value)],
    ) -> Result<()> {
        let tx = self.conn.transaction().map_err(db_err)?;
        tx.execute(
            "DELETE FROM entities WHERE budget_id = ?1 AND resource = ?2",
            params![budget, resource],
        )
        .map_err(db_err)?;
        for (id, value) in items {
            let text = serde_json::to_string(value).map_err(|e| Error::Cache(e.to_string()))?;
            tx.execute(
                "INSERT INTO entities (budget_id, resource, id, json)
                 VALUES (?1, ?2, ?3, ?4)",
                params![budget, resource, id, text],
            )
            .map_err(db_err)?;
        }
        tx.commit().map_err(db_err)
    }

    pub fn load_entities(
        &self,
        budget: &str,
        resource: &str,
        order_json_field: Option<&str>,
    ) -> Result<Vec<serde_json::Value>> {
        let sql = match order_json_field {
            Some(field) => format!(
                "SELECT json FROM entities WHERE budget_id = ?1 AND resource = ?2
                 ORDER BY json_extract(json, '{field}'), id"
            ),
            None => "SELECT json FROM entities WHERE budget_id = ?1 AND resource = ?2
                     ORDER BY rowid"
                .to_string(),
        };
        let mut stmt = self.conn.prepare(&sql).map_err(db_err)?;
        let rows = stmt
            .query_map(params![budget, resource], |row| row.get::<_, String>(0))
            .map_err(db_err)?;
        let mut out = Vec::new();
        for row in rows {
            let text = row.map_err(db_err)?;
            out.push(
                serde_json::from_str(&text).map_err(|e| Error::Cache(e.to_string()))?,
            );
        }
        Ok(out)
    }

    pub fn status_rows(&self) -> Result<Vec<(String, String, i64, i64)>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT s.budget_id, s.resource, s.server_knowledge,
                        (SELECT count(*) FROM entities e
                          WHERE e.budget_id = s.budget_id AND e.resource = s.resource)
                 FROM sync_state s ORDER BY s.budget_id, s.resource",
            )
            .map_err(db_err)?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })
            .map_err(db_err)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(db_err)?);
        }
        Ok(out)
    }
}
```

(`order_json_field` is interpolated into SQL — it is ALWAYS a compile-time constant supplied by our own sync layer, never user input; state this in a comment.)

Add `pub mod cache;` to `src/lib.rs` (alphabetical).

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test cache`
Expected: PASS (6 tests). First build compiles SQLCipher+OpenSSL — takes a few minutes once.

- [ ] **Step 7: Commit**

```bash
git add -A && git commit -m "feat: SQLCipher cache core with keychain key and corruption discard"
```

---

### Task 2: Delta params on the four budget-scoped endpoints

**Files:**
- Modify: `src/api/client.rs`, and the four call sites in `src/cli/{accounts,categories,payees,transactions}.rs` (mechanical: add `, None`).

**Interfaces:**
- Produces (Task 3 compiles against these):
  - `get_accounts(&self, budget: &str, last_knowledge: Option<i64>)`
  - `get_categories(&self, budget: &str, last_knowledge: Option<i64>)`
  - `get_payees(&self, budget: &str, last_knowledge: Option<i64>)`
  - `get_transactions(&self, budget: &str, since_date: Option<&str>, last_knowledge: Option<i64>)`
  - Query construction: append `last_knowledge_of_server=N` with `?` or `&` as appropriate (transactions may already carry `since_date`).

- [ ] **Step 1: Write the failing tests** — append to the client test module:

```rust
    #[tokio::test]
    async fn get_accounts_passes_last_knowledge() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/budgets/b-1/accounts"))
            .and(wiremock::matchers::query_param("last_knowledge_of_server", "42"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": { "accounts": [], "server_knowledge": 43 }
            })))
            .mount(&server)
            .await;
        client(&server).get_accounts("b-1", Some(42)).await.unwrap();
    }

    #[tokio::test]
    async fn get_transactions_combines_since_and_knowledge() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/budgets/b-1/transactions"))
            .and(wiremock::matchers::query_param("since_date", "2026-07-01"))
            .and(wiremock::matchers::query_param("last_knowledge_of_server", "7"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": { "transactions": [], "server_knowledge": 8 }
            })))
            .mount(&server)
            .await;
        client(&server)
            .get_transactions("b-1", Some("2026-07-01"), Some(7))
            .await
            .unwrap();
    }
```

Update existing endpoint tests' call sites (`get_accounts("b-1")` → `get_accounts("b-1", None)`, etc.).

- [ ] **Step 2: Run tests to verify they fail** — `cargo test api` fails to compile.

- [ ] **Step 3: Implement** — in `impl Client`:

```rust
    fn append_param(path: String, param: &str) -> String {
        let sep = if path.contains('?') { '&' } else { '?' };
        format!("{path}{sep}{param}")
    }

    pub async fn get_accounts(
        &self,
        budget: &str,
        last_knowledge: Option<i64>,
    ) -> Result<ListResult<AccountsWrapper>> {
        let mut path = format!("/budgets/{budget}/accounts");
        if let Some(k) = last_knowledge {
            path = Self::append_param(path, &format!("last_knowledge_of_server={k}"));
        }
        self.get_data(&path).await
    }
```

Same pattern for categories/payees; transactions builds `since_date` first then optionally appends `last_knowledge_of_server`. Update the four CLI call sites to pass `None` (behavior unchanged).

- [ ] **Step 4: Run tests to verify they pass** — `cargo test` all green.
- [ ] **Step 5: Commit** — `git add -A && git commit -m "feat: last_knowledge_of_server params on budget-scoped endpoints"`

---

### Task 3: Sync layer (delta fetch → merge → envelope)

**Files:**
- Create: `src/cache/sync.rs`
- Modify: `src/cache/mod.rs` (add `pub mod sync;` — note `mod store;` stays private)

**Interfaces:**
- Consumes: Task 1 store ops, Task 2 endpoint signatures, `api::client::{Client, ListResult}`, api wrappers.
- Produces (Task 4 calls exactly these):
  - `sync::accounts(client: &Client, cache: &mut Cache, budget: &str) -> Result<ListResult<AccountsWrapper>>`
  - `sync::categories(...) -> Result<ListResult<CategoryGroupsWrapper>>` (full fetch + `replace_entities`, load `None` ordering)
  - `sync::payees(...) -> Result<ListResult<PayeesWrapper>>`
  - `sync::transactions(client, cache, budget) -> Result<ListResult<TransactionsWrapper>>` — NEVER passes `since_date` (cache must stay complete; `--since` is filtered locally by M2's filter code).
- Envelope shape: `{"<resource-key>": [raw entities...], "server_knowledge": N}`; typed parse via `serde_json::from_value(raw.clone())` (same lockstep guarantee as M2).

- [ ] **Step 1: Write the failing tests** — `src/cache/sync.rs` test module (wiremock + tempdir + mock keychain):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::client::Client;
    use crate::cache::Cache;
    use crate::secrets::SecretStore;
    use secrecy::SecretString;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn mock_store() -> SecretStore {
        keyring::set_default_credential_builder(keyring::mock::default_credential_builder());
        SecretStore::new().unwrap()
    }

    fn cache_in(dir: &tempfile::TempDir) -> Cache {
        Cache::open_at(&mock_store(), &dir.path().join("cache.db")).unwrap()
    }

    fn client(server: &MockServer) -> Client {
        Client::with_base_url(SecretString::from("t"), server.uri())
    }

    #[tokio::test]
    async fn accounts_first_sync_then_delta_merge() {
        let server = MockServer::start().await;
        let dir = tempfile::tempdir().unwrap();
        let mut cache = cache_in(&dir);

        // first sync: no knowledge yet → no last_knowledge_of_server param
        Mock::given(method("GET"))
            .and(path("/budgets/b-1/accounts"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": { "accounts": [
                    { "id": "a-1", "name": "Chequing", "type": "checking", "on_budget": true,
                      "closed": false, "balance": 1000, "deleted": false, "note": "keepme" }
                ], "server_knowledge": 10 }
            })))
            .expect(1)
            .mount(&server)
            .await;
        let first = accounts(&client(&server), &mut cache, "b-1").await.unwrap();
        assert_eq!(first.parsed.accounts.len(), 1);
        assert_eq!(first.raw["server_knowledge"], 10);
        assert_eq!(first.raw["accounts"][0]["note"], "keepme"); // unknown field survives
        server.reset().await;

        // second sync: sends knowledge 10, gets a delta (one changed + one new)
        Mock::given(method("GET"))
            .and(path("/budgets/b-1/accounts"))
            .and(query_param("last_knowledge_of_server", "10"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": { "accounts": [
                    { "id": "a-1", "name": "Chequing RENAMED", "type": "checking",
                      "on_budget": true, "closed": false, "balance": 2000, "deleted": false },
                    { "id": "a-2", "name": "Savings", "type": "savings", "on_budget": true,
                      "closed": false, "balance": 500, "deleted": false }
                ], "server_knowledge": 11 }
            })))
            .expect(1)
            .mount(&server)
            .await;
        let second = accounts(&client(&server), &mut cache, "b-1").await.unwrap();
        assert_eq!(second.parsed.accounts.len(), 2);
        assert_eq!(second.raw["server_knowledge"], 11);
        let names: Vec<&str> = second
            .parsed
            .accounts
            .iter()
            .map(|a| a.name.as_str())
            .collect();
        assert!(names.contains(&"Chequing RENAMED"));
        assert!(names.contains(&"Savings"));
    }

    #[tokio::test]
    async fn categories_replace_wholesale() {
        let server = MockServer::start().await;
        let dir = tempfile::tempdir().unwrap();
        let mut cache = cache_in(&dir);
        let body = |groups: serde_json::Value, sk: i64| {
            serde_json::json!({ "data": { "category_groups": groups, "server_knowledge": sk } })
        };
        let group = |id: &str, name: &str| {
            serde_json::json!({ "id": id, "name": name, "hidden": false, "deleted": false,
                                "categories": [] })
        };

        Mock::given(method("GET"))
            .and(path("/budgets/b-1/categories"))
            .respond_with(ResponseTemplate::new(200)
                .set_body_json(body(serde_json::json!([group("g-1", "Bills")]), 5)))
            .expect(1)
            .mount(&server)
            .await;
        categories(&client(&server), &mut cache, "b-1").await.unwrap();
        server.reset().await;

        // full refetch replaces — g-1 gone, g-2 present; NO last_knowledge param sent
        Mock::given(method("GET"))
            .and(path("/budgets/b-1/categories"))
            .respond_with(ResponseTemplate::new(200)
                .set_body_json(body(serde_json::json!([group("g-2", "Fun")]), 6)))
            .expect(1)
            .mount(&server)
            .await;
        let second = categories(&client(&server), &mut cache, "b-1").await.unwrap();
        assert_eq!(second.parsed.category_groups.len(), 1);
        assert_eq!(second.parsed.category_groups[0].id, "g-2");
    }

    #[tokio::test]
    async fn transactions_sync_never_sends_since_date() {
        let server = MockServer::start().await;
        let dir = tempfile::tempdir().unwrap();
        let mut cache = cache_in(&dir);
        Mock::given(method("GET"))
            .and(path("/budgets/b-1/transactions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": { "transactions": [
                    { "id": "t-1", "date": "2026-07-15", "amount": -100, "memo": null,
                      "approved": true, "account_id": "a-1", "account_name": "Chq",
                      "payee_id": null, "payee_name": null, "category_id": null,
                      "category_name": null, "deleted": false }
                ], "server_knowledge": 3 }
            })))
            .mount(&server)
            .await;
        let result = transactions(&client(&server), &mut cache, "b-1").await.unwrap();
        assert_eq!(result.parsed.transactions.len(), 1);
        let requests = server.received_requests().await.unwrap();
        assert!(!requests[0].url.query().unwrap_or("").contains("since_date"));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail** — `cargo test sync` fails to compile.

- [ ] **Step 3: Implement** `src/cache/sync.rs`:

```rust
use serde_json::Value;

use crate::api::client::{Client, ListResult};
use crate::api::types::{
    AccountsWrapper, CategoryGroupsWrapper, PayeesWrapper, TransactionsWrapper,
};
use crate::cache::Cache;
use crate::error::{Error, Result};

/// (id, raw entity) pairs from an envelope array; entities without a string
/// id are skipped (defensive — the API always sends ids).
fn entity_pairs(raw: &Value, key: &str) -> Vec<(String, Value)> {
    raw.get(key)
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|v| {
                    let id = v.get("id")?.as_str()?.to_string();
                    Some((id, v.clone()))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn envelope<T: serde::de::DeserializeOwned>(
    cache: &Cache,
    budget: &str,
    resource: &str,
    key: &str,
    order: Option<&str>,
) -> Result<ListResult<T>> {
    let all = cache.load_entities(budget, resource, order)?;
    let sk = cache.server_knowledge(budget, resource)?.unwrap_or(0);
    let raw = serde_json::json!({ key: all, "server_knowledge": sk });
    let parsed =
        serde_json::from_value(raw.clone()).map_err(|e| Error::Decode(e.to_string()))?;
    Ok(ListResult { raw, parsed })
}

fn store_knowledge(cache: &Cache, budget: &str, resource: &str, raw: &Value) -> Result<()> {
    if let Some(sk) = raw.get("server_knowledge").and_then(Value::as_i64) {
        cache.set_server_knowledge(budget, resource, sk)?;
    }
    Ok(())
}

pub async fn accounts(
    client: &Client,
    cache: &mut Cache,
    budget: &str,
) -> Result<ListResult<AccountsWrapper>> {
    let known = cache.server_knowledge(budget, "accounts")?;
    let fetched = client.get_accounts(budget, known).await?;
    cache.upsert_entities(budget, "accounts", &entity_pairs(&fetched.raw, "accounts"))?;
    store_knowledge(cache, budget, "accounts", &fetched.raw)?;
    envelope(cache, budget, "accounts", "accounts", Some("$.name"))
}

pub async fn payees(
    client: &Client,
    cache: &mut Cache,
    budget: &str,
) -> Result<ListResult<PayeesWrapper>> {
    let known = cache.server_knowledge(budget, "payees")?;
    let fetched = client.get_payees(budget, known).await?;
    cache.upsert_entities(budget, "payees", &entity_pairs(&fetched.raw, "payees"))?;
    store_knowledge(cache, budget, "payees", &fetched.raw)?;
    envelope(cache, budget, "payees", "payees", Some("$.name"))
}

/// Categories: always a FULL fetch, replaced wholesale (locked decision —
/// nested-group delta merge isn't worth it for a small payload).
pub async fn categories(
    client: &Client,
    cache: &mut Cache,
    budget: &str,
) -> Result<ListResult<CategoryGroupsWrapper>> {
    let fetched = client.get_categories(budget, None).await?;
    cache.replace_entities(
        budget,
        "category_groups",
        &entity_pairs(&fetched.raw, "category_groups"),
    )?;
    store_knowledge(cache, budget, "category_groups", &fetched.raw)?;
    envelope(cache, budget, "category_groups", "category_groups", None)
}

/// Transactions: delta sync, NEVER since_date — the cache must stay a
/// complete record; `--since` is applied locally by the filter layer.
pub async fn transactions(
    client: &Client,
    cache: &mut Cache,
    budget: &str,
) -> Result<ListResult<TransactionsWrapper>> {
    let known = cache.server_knowledge(budget, "transactions")?;
    let fetched = client.get_transactions(budget, None, known).await?;
    cache.upsert_entities(
        budget,
        "transactions",
        &entity_pairs(&fetched.raw, "transactions"),
    )?;
    store_knowledge(cache, budget, "transactions", &fetched.raw)?;
    envelope(cache, budget, "transactions", "transactions", Some("$.date"))
}
```

Add `pub mod sync;` to `src/cache/mod.rs`.

- [ ] **Step 4: Run tests to verify they pass** — `cargo test` all green.
- [ ] **Step 5: Commit** — `git add -A && git commit -m "feat: delta sync layer with envelope reconstruction"`

---

### Task 4: CLI wiring (--no-cache, cache-aware commands, local --since)

**Files:**
- Modify: `src/cli/mod.rs`, `src/cli/context.rs`, `src/cli/{accounts,categories,payees,transactions}.rs`
- Create: `tests/cli_cache.rs`

**Interfaces:**
- `Cli` gains `#[arg(long, global = true)] pub no_cache: bool` (help: "Bypass the local cache for this invocation").
- `Ctx` gains `pub cache: Option<Cache>`. `build_ctx(json: bool, budget_flag: Option<&str>, no_cache: bool) -> Result<Ctx>`: cache is `Some` iff `config.cache_enabled() && !no_cache && budget != "last-used"` (locked decision — the alias bypasses caching); built via `Cache::open(&store)`. If `Cache::open` itself errors (e.g. no writable data dir), fall back to `None` silently — cache trouble must never block a read.
- All five command `list` functions change to `pub async fn list(ctx: &mut Ctx) -> Result<()>` (budgets keeps ignoring the cache — never cached). Fetch pattern for the four budget-scoped commands (destructure to split borrows):

```rust
    let Ctx { client, cache, budget, json } = ctx;
    let result = match cache {
        Some(cache) => crate::cache::sync::accounts(client, cache, budget).await?,
        None => client.get_accounts(budget, None).await?,
    };
```

- `transactions.rs`: `matches_filters` gains the since check (`if let Some(s) = &f.since { if t.date.as_str() < s.as_str() { return false; } }`) so `--since` works over cached (full) data; the no-cache path still passes normalized `since` to the API (harmless double filter); the cache path never does (sync layer ignores it by design).
- Dispatch in `run()` reads `let no_cache = cli.no_cache;` and passes it to every `build_ctx` call; command calls become `module::list(&mut ctx)`.

**Behavior tests (binary e2e, `tests/cli_cache.rs`):** all env-isolated (`YNAB_CLI_CONFIG_DIR`, `YNAB_CLI_DATA_DIR` tempdirs, `YNAB_PAT`, `YNAB_CLI_CACHE_KEY`=64 hex chars, `YNAB_CLI_API_BASE_URL`).

- [ ] **Step 1: Write the failing e2e test**

```rust
use assert_cmd::Command;
use predicates::prelude::*;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn ynab(config: &std::path::Path, data: &std::path::Path, base: &str) -> Command {
    let mut cmd = Command::cargo_bin("ynab").unwrap();
    cmd.env("YNAB_CLI_CONFIG_DIR", config);
    cmd.env("YNAB_CLI_DATA_DIR", data);
    cmd.env("YNAB_CLI_API_BASE_URL", base);
    cmd.env("YNAB_PAT", "e2e-token");
    cmd.env("YNAB_CLI_CACHE_KEY", "ab".repeat(32));
    cmd
}

fn tx_body(rows: serde_json::Value, sk: i64) -> serde_json::Value {
    serde_json::json!({ "data": { "transactions": rows, "server_knowledge": sk } })
}

fn tx(id: &str, date: &str, payee: &str) -> serde_json::Value {
    serde_json::json!({ "id": id, "date": date, "amount": -1000, "memo": null,
        "approved": true, "account_id": "a-1", "account_name": "Chq",
        "payee_id": null, "payee_name": payee, "category_id": null,
        "category_name": null, "deleted": false })
}

#[tokio::test(flavor = "multi_thread")]
async fn transactions_delta_cache_roundtrip() {
    let server = MockServer::start().await;
    // first call: full fetch (no last_knowledge_of_server)
    Mock::given(method("GET"))
        .and(path("/budgets/b-1/transactions"))
        .respond_with(ResponseTemplate::new(200)
            .set_body_json(tx_body(serde_json::json!([tx("t-1", "2026-07-01", "Grocer")]), 10)))
        .expect(1)
        .mount(&server)
        .await;

    let uri = server.uri();
    let config = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let (cfg, dat) = (config.path().to_path_buf(), data.path().to_path_buf());
    let u = uri.clone();
    tokio::task::spawn_blocking(move || {
        ynab(&cfg, &dat, &u)
            .args(["transactions", "list", "--budget", "b-1"])
            .assert()
            .success()
            .stdout(predicate::str::contains("Grocer"));
    })
    .await
    .unwrap();
    server.reset().await;

    // second call: MUST send last_knowledge_of_server=10; delta adds t-2;
    // output contains BOTH rows (t-1 from cache, t-2 from delta)
    Mock::given(method("GET"))
        .and(path("/budgets/b-1/transactions"))
        .and(query_param("last_knowledge_of_server", "10"))
        .respond_with(ResponseTemplate::new(200)
            .set_body_json(tx_body(serde_json::json!([tx("t-2", "2026-07-20", "Landlord")]), 11)))
        .expect(1)
        .mount(&server)
        .await;
    // third invocation (--since) syncs again with knowledge 11 → empty delta
    Mock::given(method("GET"))
        .and(path("/budgets/b-1/transactions"))
        .and(query_param("last_knowledge_of_server", "11"))
        .respond_with(ResponseTemplate::new(200)
            .set_body_json(tx_body(serde_json::json!([]), 11)))
        .expect(1)
        .mount(&server)
        .await;
    let (cfg, dat) = (config.path().to_path_buf(), data.path().to_path_buf());
    let u = uri.clone();
    tokio::task::spawn_blocking(move || {
        ynab(&cfg, &dat, &u)
            .args(["transactions", "list", "--budget", "b-1"])
            .assert()
            .success()
            .stdout(predicate::str::contains("Grocer"))
            .stdout(predicate::str::contains("Landlord"));
        // --since filters locally over the cached set (no new since_date request)
        ynab(&cfg, &dat, &u)
            .args(["transactions", "list", "--budget", "b-1", "--since", "2026-07-10"])
            .assert()
            .success()
            .stdout(predicate::str::contains("Landlord"))
            .stdout(predicate::str::contains("Grocer").not());
    })
    .await
    .unwrap();

    // exactly 3 requests total, none with since_date
    let requests = server.received_requests().await.unwrap();
    assert!(requests.iter().all(|r| !r.url.query().unwrap_or("").contains("since_date")));
}

#[tokio::test(flavor = "multi_thread")]
async fn no_cache_flag_bypasses() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/budgets/b-1/transactions"))
        .respond_with(ResponseTemplate::new(200)
            .set_body_json(tx_body(serde_json::json!([tx("t-1", "2026-07-01", "Grocer")]), 10)))
        .expect(2)
        .mount(&server)
        .await;
    let uri = server.uri();
    let config = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let (cfg, dat) = (config.path().to_path_buf(), data.path().to_path_buf());
    tokio::task::spawn_blocking(move || {
        for _ in 0..2 {
            ynab(&cfg, &dat, &uri)
                .args(["transactions", "list", "--budget", "b-1", "--no-cache"])
                .assert()
                .success();
        }
    })
    .await
    .unwrap();
    // no cache DB was created
    assert!(!data.path().join("cache.db").exists());
    // and neither request carried last_knowledge_of_server
    let requests = server.received_requests().await.unwrap();
    assert!(requests
        .iter()
        .all(|r| !r.url.query().unwrap_or("").contains("last_knowledge_of_server")));
}
```

- [ ] **Step 2: Run to verify failure** — `cargo test --test cli_cache` fails (`--no-cache` unknown, no caching behavior).
- [ ] **Step 3: Implement** per the Interfaces block above. Also add the since-check unit test to `transactions.rs`:

```rust
    #[test]
    fn since_filters_locally() {
        let f = Filters { since: Some("2026-07-16".into()), ..Default::default() };
        assert!(!matches_filters(&tx(), &f)); // fixture date 2026-07-15
        let f = Filters { since: Some("2026-07-15".into()), ..Default::default() };
        assert!(matches_filters(&tx(), &f));
    }
```

- [ ] **Step 4: Run tests to verify they pass** — `cargo test` all green.
- [ ] **Step 5: Commit** — `git add -A && git commit -m "feat: cache-aware list commands with --no-cache and local --since"`

---

### Task 5: `ynab cache status|clear`

**Files:**
- Create: `src/cli/cache_cmd.rs`
- Modify: `src/cli/mod.rs` (add `pub mod cache_cmd;`, `Command::Cache { command: CacheCommand }`, `#[derive(Debug, Subcommand)] pub enum CacheCommand { Status, Clear }`, dispatch arm — Status/Clear ignore the global json/budget/no_cache flags except `json` for status)

**Interfaces:**
- `cache_cmd::status(json: bool) -> Result<()>`:
  - cache file missing ⇒ print `Cache: empty (<path>)` (json: `{"path": ..., "exists": false}`).
  - else open via `Cache::open(&SecretStore::new()?)` and print path, file size in bytes, and a table `Budget | Resource | Server Knowledge | Entities` from `status_rows()` (json: `{"path", "exists": true, "size_bytes", "resources": [{"budget_id", "resource", "server_knowledge", "entities"}]}`).
- `cache_cmd::clear() -> Result<()>`: delete the DB file if present (missing is fine), print `Cache cleared.` The keychain key is kept (reused on next run).
- Dispatch: `CacheCommand::Status => cache_cmd::status(json)`, `CacheCommand::Clear => cache_cmd::clear()`.

- [ ] **Step 1: Write the failing e2e test** — append to `tests/cli_cache.rs`:

```rust
#[tokio::test(flavor = "multi_thread")]
async fn cache_status_and_clear() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/budgets/b-1/transactions"))
        .respond_with(ResponseTemplate::new(200)
            .set_body_json(tx_body(serde_json::json!([tx("t-1", "2026-07-01", "Grocer")]), 10)))
        .mount(&server)
        .await;
    let uri = server.uri();
    let config = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let (cfg, dat) = (config.path().to_path_buf(), data.path().to_path_buf());
    tokio::task::spawn_blocking(move || {
        // empty status
        ynab(&cfg, &dat, &uri)
            .args(["cache", "status"])
            .assert()
            .success()
            .stdout(predicate::str::contains("empty"));
        // populate, then status shows the resource row
        ynab(&cfg, &dat, &uri)
            .args(["transactions", "list", "--budget", "b-1"])
            .assert()
            .success();
        ynab(&cfg, &dat, &uri)
            .args(["cache", "status"])
            .assert()
            .success()
            .stdout(predicate::str::contains("transactions"))
            .stdout(predicate::str::contains("10"));
        // clear deletes the file
        ynab(&cfg, &dat, &uri)
            .args(["cache", "clear"])
            .assert()
            .success()
            .stdout(predicate::str::contains("Cache cleared."));
        assert!(!dat.join("cache.db").exists());
        // clear again is fine
        ynab(&cfg, &dat, &uri).args(["cache", "clear"]).assert().success();
    })
    .await
    .unwrap();
}
```

- [ ] **Step 2: Run to verify failure** — unknown `cache` subcommand.
- [ ] **Step 3: Implement** `src/cli/cache_cmd.rs`:

```rust
use crate::cache::Cache;
use crate::error::Result;
use crate::output;
use crate::secrets::SecretStore;

pub fn status(json: bool) -> Result<()> {
    let path = Cache::db_path()?;
    if !path.exists() {
        if json {
            return output::print_json(&serde_json::json!({
                "path": path.display().to_string(), "exists": false
            }));
        }
        println!("Cache: empty ({})", path.display());
        return Ok(());
    }
    let size = std::fs::metadata(&path)?.len();
    let cache = Cache::open(&SecretStore::new()?)?;
    let rows = cache.status_rows()?;
    if json {
        let resources: Vec<serde_json::Value> = rows
            .iter()
            .map(|(b, r, sk, n)| {
                serde_json::json!({
                    "budget_id": b, "resource": r,
                    "server_knowledge": sk, "entities": n
                })
            })
            .collect();
        return output::print_json(&serde_json::json!({
            "path": path.display().to_string(), "exists": true,
            "size_bytes": size, "resources": resources
        }));
    }
    println!("Cache: {} ({} bytes)", path.display(), size);
    let table_rows = rows
        .into_iter()
        .map(|(b, r, sk, n)| vec![b, r, sk.to_string(), n.to_string()])
        .collect();
    println!(
        "{}",
        output::render_table(
            &["Budget", "Resource", "Server Knowledge", "Entities"],
            table_rows
        )
    );
    Ok(())
}

pub fn clear() -> Result<()> {
    let path = Cache::db_path()?;
    if path.exists() {
        std::fs::remove_file(&path)?;
    }
    println!("Cache cleared.");
    Ok(())
}
```

- [ ] **Step 4: Run tests to verify they pass** — `cargo test` all green.
- [ ] **Step 5: Milestone gate** — `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check`; fix in-scope issues, report others.
- [ ] **Step 6: Commit** — `git add -A && git commit -m "feat: ynab cache status/clear commands"`

---

## Carry-overs still parked (not this plan)

- M2 list: budget-id URL encoding/validation; accounts-list e2e; global-flag help polish; zeroize; `Error::Input`; CI write-verb grep. The env-sensitive `token_prefers_keychain_when_no_env` test can now be fixed cheaply with `temp-env` (added in Task 1) — allowed as a drive-by in Task 4 if the implementer has it in scope, else it stays parked.
- M4 will decide whether `logout` clears `SecretKind::CacheKey`/the cache file (final M1 review flagged: encrypted financial data stays decryptable post-logout today).
