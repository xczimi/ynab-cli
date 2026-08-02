# M1: Foundation + PAT Auth Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A working `ynab` binary where `ynab auth login|status|logout` (PAT paste-in) and `ynab config get|set` work end-to-end, on top of the core modules (errors, keychain secrets, TOML config, minimal GET-only API client) that every later milestone builds on.

**Architecture:** Core library (`src/lib.rs`) with focused modules — `error`, `secrets` (keyring), `config` (TOML), `api` (GET-only reqwest client) — and a thin clap CLI frontend that only adapts core functions. The API client is born in M1 with a single endpoint (`GET /user`) so the GET-only structural guarantee exists from the first commit; M2 extends it.

**Tech Stack:** Rust 1.95 (mise), clap 4 (derive), tokio, reqwest (rustls), serde/serde_json, thiserror, keyring 3, secrecy, toml, directories, rpassword; tests: wiremock, tempfile, assert_cmd, predicates, keyring's mock store.

## Global Constraints

Copied from CLAUDE.md — every task's requirements implicitly include these:

- **Read-only is structural**: `src/api/client.rs` contains ONLY GET requests. No `post`, `put`, `patch`, `delete` call may appear anywhere in the codebase.
- **No secrets on disk**: tokens live only in the OS keychain via the `keyring` crate. The TOML config file never holds secrets.
- **No secrets in output**: secret-holding types use `secrecy::SecretString`; `Debug` output must never reveal a token.
- **Rate limit UX**: a 429 response prints `rate limited by YNAB (200 requests/hour) — resets within the hour`, never a stack trace or panic.
- **Errors are messages, not panics**: `main` prints `error: <message>` to stderr and exits 1. No `.unwrap()`/`.expect()` on fallible paths in non-test code.
- **API base**: `https://api.ynab.com/v1`. Auth header: `Authorization: Bearer <token>`.
- **Config keys (v1)**: `cache` (bool, default true), `default_budget` (string). Nothing else.
- **Binary name**: `ynab`. Crate: `ynab-cli`.
- **Commits**: conventional format (`feat:`, `fix:`, `test:`, `chore:`). No attribution trailers (disabled globally in user settings).
- **Verification**: `cargo build` and `cargo test` must pass at the end of every task. `cargo clippy -- -D warnings` must pass before the milestone is declared done.

## Execution notes

- Work on branch `m1-foundation-pat-auth` off `main` (create in Task 1, Step 1).
- Dependency DAG: Task 1 first; then Tasks 2, 3, 4, 5 in parallel; then Tasks 6 and 7 in parallel.
- Each parallel task touches disjoint files; the only shared file is `Cargo.toml`/`src/lib.rs` (tiny additive edits — if you hit a conflict, take both sides).

## File Structure

```
src/main.rs           — thin: parse CLI, dispatch, print errors (Task 5)
src/lib.rs            — module declarations (grows per task)
src/error.rs          — Error enum + Result alias (Task 1)
src/secrets.rs        — SecretStore over keyring (Task 2)
src/config.rs         — Config load/save, TOML (Task 3)
src/api/mod.rs        — module declarations (Task 4)
src/api/client.rs     — GET-only Client, get_user (Task 4)
src/api/types.rs      — User + response envelopes (Task 4)
src/cli/mod.rs        — clap command tree + dispatch (Task 5)
src/cli/auth.rs       — auth login/status/logout handlers (Task 6)
src/cli/config_cmd.rs — config get/set handlers (Task 7)
tests/cli_basics.rs   — assert_cmd: --help, unknown args (Task 5)
tests/cli_config.rs   — assert_cmd: config get/set round-trip (Task 7)
```

---

### Task 1: Crate layout + error module

**Files:**
- Create: `src/error.rs`, `src/lib.rs`
- Modify: `src/main.rs` (placeholder that compiles against the lib)

**Interfaces:**
- Consumes: nothing.
- Produces: `ynab_cli::error::{Error, Result}` — the error type every later task returns. Variants and exact messages below; later tasks match on `Error::NotAuthenticated`, `Error::RateLimited`, `Error::Api { status, message }`, `Error::Keychain`, `Error::Config`, `Error::Http`, `Error::Io`.

- [ ] **Step 1: Create the branch**

```bash
git checkout -b m1-foundation-pat-auth
```

- [ ] **Step 2: Write the failing test**

Create `src/error.rs` containing only the test module for now:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_facing_messages() {
        assert_eq!(
            Error::NotAuthenticated.to_string(),
            "not logged in — run `ynab auth login`"
        );
        assert_eq!(
            Error::RateLimited.to_string(),
            "rate limited by YNAB (200 requests/hour) — resets within the hour"
        );
        assert_eq!(
            Error::Api { status: 500, message: "boom".into() }.to_string(),
            "YNAB API error (500): boom"
        );
        assert_eq!(
            Error::Config("bad key".into()).to_string(),
            "config error: bad key"
        );
    }
}
```

Create `src/lib.rs`:

```rust
pub mod error;
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test`
Expected: FAIL to compile — `Error` not defined.

- [ ] **Step 4: Write minimal implementation**

Prepend to `src/error.rs`:

```rust
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("not logged in — run `ynab auth login`")]
    NotAuthenticated,
    #[error("rate limited by YNAB (200 requests/hour) — resets within the hour")]
    RateLimited,
    #[error("YNAB API error ({status}): {message}")]
    Api { status: u16, message: String },
    #[error("keychain error: {0}")]
    Keychain(#[from] keyring::Error),
    #[error("config error: {0}")]
    Config(String),
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
```

Replace `src/main.rs` with a placeholder that links the lib (Task 5 replaces it):

```rust
fn main() {
    // CLI wired up in the cli module tasks.
}
```

Note: keyring 3 needs platform features to compile. In `Cargo.toml` replace the keyring line with:

```toml
keyring = { version = "3", features = ["apple-native", "windows-native", "sync-secret-service"] }
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test`
Expected: PASS (1 test).

- [ ] **Step 6: Commit**

```bash
git add -A && git commit -m "feat: crate layout and core error type"
```

---

### Task 2: Secrets store (keychain)

**Files:**
- Create: `src/secrets.rs`
- Modify: `src/lib.rs` (add `pub mod secrets;`), `Cargo.toml`

**Interfaces:**
- Consumes: `crate::error::{Error, Result}`.
- Produces:
  - `secrets::SecretKind` — enum `{ Pat, OauthClientId, OauthClientSecret, OauthAccessToken, OauthRefreshToken, CacheKey }` (all six now; only `Pat` is used in M1, the rest exist so M3/M4 don't change this file's API).
  - `secrets::SecretStore::new() -> Result<SecretStore>` — creates the keyring `Entry` for all six kinds eagerly and holds them. This matters: keyring 3's mock store (used in tests) keeps credential state per-`Entry`, so entries must be reused across get/set/delete, not recreated per call.
  - `SecretStore::get(&self, kind: SecretKind) -> Result<Option<secrecy::SecretString>>`
  - `SecretStore::set(&self, kind: SecretKind, value: secrecy::SecretString) -> Result<()>`
  - `SecretStore::delete(&self, kind: SecretKind) -> Result<()>` (deleting a missing entry is Ok, not an error)

- [ ] **Step 1: Add dependencies**

In `Cargo.toml` `[dependencies]` add:

```toml
secrecy = "0.10"
```

- [ ] **Step 2: Write the failing test**

Create `src/secrets.rs` with the test module (keyring 3 ships an in-memory mock store for exactly this):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::{ExposeSecret, SecretString};

    fn mock_store() -> SecretStore {
        keyring::set_default_credential_builder(keyring::mock::default_credential_builder());
        SecretStore::new().unwrap()
    }

    #[test]
    fn set_get_delete_roundtrip() {
        let store = mock_store();
        assert!(store.get(SecretKind::Pat).unwrap().is_none());

        store.set(SecretKind::Pat, SecretString::from("tok-123")).unwrap();
        let got = store.get(SecretKind::Pat).unwrap().unwrap();
        assert_eq!(got.expose_secret(), "tok-123");

        store.delete(SecretKind::Pat).unwrap();
        assert!(store.get(SecretKind::Pat).unwrap().is_none());
        // deleting again is not an error
        store.delete(SecretKind::Pat).unwrap();
    }

    #[test]
    fn kinds_are_separate_entries() {
        let store = mock_store();
        store.set(SecretKind::Pat, SecretString::from("a")).unwrap();
        store.set(SecretKind::CacheKey, SecretString::from("b")).unwrap();
        assert_eq!(store.get(SecretKind::Pat).unwrap().unwrap().expose_secret(), "a");
        assert_eq!(store.get(SecretKind::CacheKey).unwrap().unwrap().expose_secret(), "b");
    }
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test secrets`
Expected: FAIL to compile — `SecretStore` not defined.

- [ ] **Step 4: Write minimal implementation**

Prepend to `src/secrets.rs`:

```rust
use keyring::Entry;
use secrecy::{ExposeSecret, SecretString};

use crate::error::Result;

const SERVICE: &str = "ynab-cli";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretKind {
    Pat,
    OauthClientId,
    OauthClientSecret,
    OauthAccessToken,
    OauthRefreshToken,
    CacheKey,
}

impl SecretKind {
    fn entry_name(self) -> &'static str {
        match self {
            SecretKind::Pat => "pat",
            SecretKind::OauthClientId => "oauth-client-id",
            SecretKind::OauthClientSecret => "oauth-client-secret",
            SecretKind::OauthAccessToken => "oauth-access-token",
            SecretKind::OauthRefreshToken => "oauth-refresh-token",
            SecretKind::CacheKey => "cache-key",
        }
    }
}

/// Holds one keyring Entry per SecretKind, created eagerly. Entries are
/// reused (not recreated per call) because keyring's mock store — used in
/// tests — keeps credential state per-Entry instance.
pub struct SecretStore {
    entries: [Entry; 6],
}

impl SecretStore {
    pub fn new() -> Result<Self> {
        // Array order MUST match SecretKind discriminant order (`as usize`).
        let mk = |kind: SecretKind| Entry::new(SERVICE, kind.entry_name());
        Ok(SecretStore {
            entries: [
                mk(SecretKind::Pat)?,
                mk(SecretKind::OauthClientId)?,
                mk(SecretKind::OauthClientSecret)?,
                mk(SecretKind::OauthAccessToken)?,
                mk(SecretKind::OauthRefreshToken)?,
                mk(SecretKind::CacheKey)?,
            ],
        })
    }

    fn entry(&self, kind: SecretKind) -> &Entry {
        &self.entries[kind as usize]
    }

    pub fn get(&self, kind: SecretKind) -> Result<Option<SecretString>> {
        match self.entry(kind).get_password() {
            Ok(value) => Ok(Some(SecretString::from(value))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn set(&self, kind: SecretKind, value: SecretString) -> Result<()> {
        self.entry(kind).set_password(value.expose_secret())?;
        Ok(())
    }

    pub fn delete(&self, kind: SecretKind) -> Result<()> {
        match self.entry(kind).delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}
```

Add to `src/lib.rs`:

```rust
pub mod secrets;
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test secrets`
Expected: PASS (2 tests). Also run `cargo test` — everything passes, and the tests must NOT touch the real OS keychain (the mock builder guarantees this).

- [ ] **Step 6: Commit**

```bash
git add -A && git commit -m "feat: keychain-backed secret store with mock-store tests"
```

---

### Task 3: Config module (TOML)

**Files:**
- Create: `src/config.rs`
- Modify: `src/lib.rs` (add `pub mod config;`), `Cargo.toml`

**Interfaces:**
- Consumes: `crate::error::{Error, Result}`.
- Produces:
  - `config::Config { pub cache: Option<bool>, pub default_budget: Option<String> }` (derives `Debug, Clone, Default, Serialize, Deserialize`)
  - `Config::dir() -> Result<std::path::PathBuf>` — platform config dir as resolved by the `directories` crate (user ruling 2026-08-02: follow OS convention — macOS `~/Library/Application Support/ynab-cli`, Linux XDG `~/.config/ynab-cli`, Windows `%APPDATA%`), overridable with env var `YNAB_CLI_CONFIG_DIR` (needed by CLI integration tests).
  - `Config::load() -> Result<Config>` — missing file ⇒ `Config::default()`.
  - `Config::save(&self) -> Result<()>` — creates the directory, writes `config.toml`.
  - `Config::cache_enabled(&self) -> bool` — `cache.unwrap_or(true)`.
  - `Config::get_key(&self, key: &str) -> Result<Option<String>>` and `Config::with_key(self, key: &str, value: &str) -> Result<Config>` — string-keyed access for the `config get|set` CLI; valid keys are exactly `"cache"` and `"default_budget"`; invalid key ⇒ `Error::Config("unknown key: <key> (valid keys: cache, default_budget)")`; non-bool value for `cache` ⇒ `Error::Config("cache must be true or false")`. `with_key` returns a new Config (no mutation).

- [ ] **Step 1: Add dependencies**

In `Cargo.toml` add to `[dependencies]`:

```toml
toml = "0.8"
directories = "6"
```

and to `[dev-dependencies]`:

```toml
tempfile = "3"
```

- [ ] **Step 2: Write the failing test**

Create `src/config.rs` with the test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_when_missing() {
        let cfg = Config::default();
        assert!(cfg.cache_enabled());
        assert!(cfg.default_budget.is_none());
    }

    #[test]
    fn toml_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let cfg = Config { cache: Some(false), default_budget: Some("b-1".into()) };
        cfg.save_to(&path).unwrap();
        let loaded = Config::load_from(&path).unwrap();
        assert_eq!(loaded.cache, Some(false));
        assert_eq!(loaded.default_budget.as_deref(), Some("b-1"));
    }

    #[test]
    fn load_from_missing_file_gives_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let loaded = Config::load_from(&dir.path().join("nope.toml")).unwrap();
        assert!(loaded.cache_enabled());
    }

    #[test]
    fn key_access() {
        let cfg = Config::default();
        assert_eq!(cfg.get_key("cache").unwrap(), None);
        let cfg = cfg.with_key("cache", "false").unwrap();
        assert_eq!(cfg.get_key("cache").unwrap().as_deref(), Some("false"));
        let cfg = cfg.with_key("default_budget", "b-9").unwrap();
        assert_eq!(cfg.get_key("default_budget").unwrap().as_deref(), Some("b-9"));

        assert!(cfg.get_key("nope").is_err());
        assert!(cfg.clone().with_key("cache", "maybe").is_err());
    }
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test config`
Expected: FAIL to compile — `Config` not defined.

- [ ] **Step 4: Write minimal implementation**

Prepend to `src/config.rs`:

```rust
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

pub const VALID_KEYS: &[&str] = &["cache", "default_budget"];

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_budget: Option<String>,
}

impl Config {
    pub fn dir() -> Result<PathBuf> {
        if let Ok(dir) = std::env::var("YNAB_CLI_CONFIG_DIR") {
            return Ok(PathBuf::from(dir));
        }
        directories::ProjectDirs::from("", "", "ynab-cli")
            .map(|d| d.config_dir().to_path_buf())
            .ok_or_else(|| Error::Config("cannot determine config directory".into()))
    }

    fn file_path() -> Result<PathBuf> {
        Ok(Self::dir()?.join("config.toml"))
    }

    pub fn load() -> Result<Config> {
        Self::load_from(&Self::file_path()?)
    }

    pub fn load_from(path: &Path) -> Result<Config> {
        if !path.exists() {
            return Ok(Config::default());
        }
        let text = std::fs::read_to_string(path)?;
        toml::from_str(&text).map_err(|e| Error::Config(format!("invalid config file: {e}")))
    }

    pub fn save(&self) -> Result<()> {
        self.save_to(&Self::file_path()?)
    }

    pub fn save_to(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = toml::to_string_pretty(self)
            .map_err(|e| Error::Config(format!("cannot serialize config: {e}")))?;
        std::fs::write(path, text)?;
        Ok(())
    }

    pub fn cache_enabled(&self) -> bool {
        self.cache.unwrap_or(true)
    }

    pub fn get_key(&self, key: &str) -> Result<Option<String>> {
        match key {
            "cache" => Ok(self.cache.map(|b| b.to_string())),
            "default_budget" => Ok(self.default_budget.clone()),
            _ => Err(Error::Config(format!(
                "unknown key: {key} (valid keys: {})",
                VALID_KEYS.join(", ")
            ))),
        }
    }

    pub fn with_key(self, key: &str, value: &str) -> Result<Config> {
        match key {
            "cache" => {
                let parsed: bool = value
                    .parse()
                    .map_err(|_| Error::Config("cache must be true or false".into()))?;
                Ok(Config { cache: Some(parsed), ..self })
            }
            "default_budget" => Ok(Config { default_budget: Some(value.to_string()), ..self }),
            _ => Err(Error::Config(format!(
                "unknown key: {key} (valid keys: {})",
                VALID_KEYS.join(", ")
            ))),
        }
    }
}
```

Add to `src/lib.rs`:

```rust
pub mod config;
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test config`
Expected: PASS (4 tests).

- [ ] **Step 6: Commit**

```bash
git add -A && git commit -m "feat: TOML config module with platform dir and key access"
```

---

### Task 4: Minimal GET-only API client (`GET /user`)

**Files:**
- Create: `src/api/mod.rs`, `src/api/client.rs`, `src/api/types.rs`
- Modify: `src/lib.rs` (add `pub mod api;`)

**Interfaces:**
- Consumes: `crate::error::{Error, Result}`, `secrecy::SecretString`.
- Produces:
  - `api::types::User { pub id: String }` (derives `Debug, Clone, Deserialize`)
  - `api::client::Client::new(token: secrecy::SecretString) -> Client` — base URL `https://api.ynab.com/v1`
  - `Client::with_base_url(token: secrecy::SecretString, base_url: String) -> Client` — for tests (wiremock) and M2+
  - `Client::get_user(&self) -> Result<User>` (async)
  - Error mapping used by ALL future endpoints: HTTP 401 ⇒ `Error::NotAuthenticated`, HTTP 429 ⇒ `Error::RateLimited`, other non-2xx ⇒ `Error::Api { status, message }` where message comes from YNAB's error envelope `{"error":{"detail":...}}` (fall back to the status canonical reason when the body doesn't parse).

**CRITICAL:** This file is the codebase's read-only guarantee. Only `reqwest::Client::get` may ever be called here. Do not add a generic `request(method, ...)` helper.

- [ ] **Step 1: Write the failing test**

Create `src/api/client.rs` with the test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::SecretString;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn client(server: &MockServer) -> Client {
        Client::with_base_url(SecretString::from("tok-1"), server.uri())
    }

    #[tokio::test]
    async fn get_user_success_sends_bearer_token() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/user"))
            .and(header("Authorization", "Bearer tok-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": { "user": { "id": "u-123" } }
            })))
            .mount(&server)
            .await;

        let user = client(&server).get_user().await.unwrap();
        assert_eq!(user.id, "u-123");
    }

    #[tokio::test]
    async fn unauthorized_maps_to_not_authenticated() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/user"))
            .respond_with(ResponseTemplate::new(401).set_body_json(serde_json::json!({
                "error": { "id": "401", "name": "unauthorized", "detail": "Unauthorized" }
            })))
            .mount(&server)
            .await;

        let err = client(&server).get_user().await.unwrap_err();
        assert!(matches!(err, crate::error::Error::NotAuthenticated));
    }

    #[tokio::test]
    async fn rate_limit_maps_to_rate_limited() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/user"))
            .respond_with(ResponseTemplate::new(429))
            .mount(&server)
            .await;

        let err = client(&server).get_user().await.unwrap_err();
        assert!(matches!(err, crate::error::Error::RateLimited));
    }

    #[tokio::test]
    async fn other_errors_carry_detail() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/user"))
            .respond_with(ResponseTemplate::new(500).set_body_json(serde_json::json!({
                "error": { "id": "500", "name": "internal", "detail": "server exploded" }
            })))
            .mount(&server)
            .await;

        let err = client(&server).get_user().await.unwrap_err();
        assert_eq!(err.to_string(), "YNAB API error (500): server exploded");
    }
}
```

Create `src/api/mod.rs`:

```rust
pub mod client;
pub mod types;
```

Create `src/api/types.rs`:

```rust
use serde::Deserialize;

/// YNAB wraps every success payload as {"data": ...}.
#[derive(Debug, Deserialize)]
pub struct DataEnvelope<T> {
    pub data: T,
}

/// YNAB wraps every error payload as {"error": {"detail": ...}}.
#[derive(Debug, Deserialize)]
pub struct ErrorEnvelope {
    pub error: ErrorDetail,
}

#[derive(Debug, Deserialize)]
pub struct ErrorDetail {
    pub detail: String,
}

#[derive(Debug, Deserialize)]
pub struct UserWrapper {
    pub user: User,
}

#[derive(Debug, Clone, Deserialize)]
pub struct User {
    pub id: String,
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test api`
Expected: FAIL to compile — `Client` not defined.

- [ ] **Step 3: Write minimal implementation**

Prepend to `src/api/client.rs`:

```rust
use reqwest::StatusCode;
use secrecy::{ExposeSecret, SecretString};

use crate::api::types::{DataEnvelope, ErrorEnvelope, User, UserWrapper};
use crate::error::{Error, Result};

const BASE_URL: &str = "https://api.ynab.com/v1";

pub struct Client {
    http: reqwest::Client,
    base_url: String,
    token: SecretString,
}

impl Client {
    pub fn new(token: SecretString) -> Self {
        Self::with_base_url(token, BASE_URL.to_string())
    }

    pub fn with_base_url(token: SecretString, base_url: String) -> Self {
        Client { http: reqwest::Client::new(), base_url, token }
    }

    /// The ONLY http verb in this codebase is GET. Read-only is structural.
    async fn get_json<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T> {
        let resp = self
            .http
            .get(format!("{}{}", self.base_url, path))
            .bearer_auth(self.token.expose_secret())
            .send()
            .await?;

        let status = resp.status();
        match status {
            s if s.is_success() => Ok(resp.json::<T>().await?),
            StatusCode::UNAUTHORIZED => Err(Error::NotAuthenticated),
            StatusCode::TOO_MANY_REQUESTS => Err(Error::RateLimited),
            s => {
                let message = match resp.json::<ErrorEnvelope>().await {
                    Ok(env) => env.error.detail,
                    Err(_) => s.canonical_reason().unwrap_or("unknown error").to_string(),
                };
                Err(Error::Api { status: s.as_u16(), message })
            }
        }
    }

    pub async fn get_user(&self) -> Result<User> {
        let env: DataEnvelope<UserWrapper> = self.get_json("/user").await?;
        Ok(env.data.user)
    }
}
```

Add to `src/lib.rs`:

```rust
pub mod api;
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test api`
Expected: PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat: GET-only API client with /user endpoint and error mapping"
```

---

### Task 5: CLI skeleton (clap) + main wiring

**Files:**
- Create: `src/cli/mod.rs`, `tests/cli_basics.rs`
- Modify: `src/main.rs`, `src/lib.rs` (add `pub mod cli;`), `Cargo.toml`

**Interfaces:**
- Consumes: `crate::error::Result`.
- Produces:
  - `cli::Cli` — clap derive struct with `#[command(name = "ynab", version, about)]`.
  - `cli::Command` enum: `Auth { command: AuthCommand }`, `Config { command: ConfigCommand }`.
  - `cli::AuthCommand` enum: `Login`, `Status`, `Logout`.
  - `cli::ConfigCommand` enum: `Get { key: String }`, `Set { key: String, value: String }`.
  - `cli::run(cli: Cli) -> crate::error::Result<()>` (async) — dispatches; in THIS task every arm returns `Err(Error::Config("not implemented".into()))`; Tasks 6 and 7 replace the arms with calls to `cli::auth::*` and `cli::config_cmd::*`.
  - `main.rs` contract: prints `error: <message>` to stderr and exits 1 on `Err`; exits 0 on `Ok`.

- [ ] **Step 1: Add dev-dependencies**

In `Cargo.toml` `[dev-dependencies]` add:

```toml
assert_cmd = "2"
predicates = "3"
```

- [ ] **Step 2: Write the failing test**

Create `tests/cli_basics.rs`:

```rust
use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn help_lists_commands() {
    Command::cargo_bin("ynab")
        .unwrap()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("auth"))
        .stdout(predicate::str::contains("config"));
}

#[test]
fn unknown_command_fails() {
    Command::cargo_bin("ynab")
        .unwrap()
        .arg("frobnicate")
        .assert()
        .failure();
}

#[test]
fn errors_go_to_stderr_with_prefix() {
    // `config get` is unimplemented in this task; its error must follow the contract.
    Command::cargo_bin("ynab")
        .unwrap()
        .args(["config", "get", "cache"])
        .assert()
        .failure()
        .stderr(predicate::str::starts_with("error: "));
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test --test cli_basics`
Expected: FAIL — `auth`/`config` subcommands don't exist yet.

- [ ] **Step 4: Write minimal implementation**

Create `src/cli/mod.rs`:

```rust
use clap::{Parser, Subcommand};

use crate::error::{Error, Result};

#[derive(Debug, Parser)]
#[command(name = "ynab", version, about = "Absolutely read-only CLI for the YNAB API")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Authenticate with YNAB
    Auth {
        #[command(subcommand)]
        command: AuthCommand,
    },
    /// Read or edit the ynab-cli config file
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
}

#[derive(Debug, Subcommand)]
pub enum AuthCommand {
    /// Log in with a YNAB Personal Access Token
    Login,
    /// Show whether you are logged in and the token works
    Status,
    /// Remove stored credentials from the OS keychain
    Logout,
}

#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    /// Print a config value
    Get { key: String },
    /// Set a config value
    Set { key: String, value: String },
}

pub async fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Command::Auth { .. } => Err(Error::Config("not implemented".into())),
        Command::Config { .. } => Err(Error::Config("not implemented".into())),
    }
}
```

Replace `src/main.rs`:

```rust
use clap::Parser;

#[tokio::main]
async fn main() {
    let cli = ynab_cli::cli::Cli::parse();
    if let Err(e) = ynab_cli::cli::run(cli).await {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
```

Add to `src/lib.rs`:

```rust
pub mod cli;
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test --test cli_basics`
Expected: PASS (3 tests). Run `cargo test` — all green.

- [ ] **Step 6: Commit**

```bash
git add -A && git commit -m "feat: clap CLI skeleton with auth/config command tree"
```

---

### Task 6: `ynab auth login|status|logout` (PAT)

**Files:**
- Create: `src/cli/auth.rs`
- Modify: `src/cli/mod.rs` (add `pub mod auth;` at top; replace the `Command::Auth` arm), `Cargo.toml`

**Interfaces:**
- Consumes: `secrets::{SecretStore, SecretKind}` (Task 2), `api::client::Client` + `api::types::User` (Task 4), `cli::AuthCommand` (Task 5).
- Produces (all in `src/cli/auth.rs`, all async, all `-> Result<()>`):
  - `pub async fn login(store: &SecretStore, api_base_url: Option<String>) -> Result<()>`
  - `pub async fn status(store: &SecretStore, api_base_url: Option<String>) -> Result<()>`
  - `pub fn logout(store: &SecretStore) -> Result<()>`
  - `api_base_url` is `None` in production (client uses the real base URL); tests pass `Some(wiremock_uri)`. The dispatch in `cli::mod` reads env var `YNAB_CLI_API_BASE_URL` (test hook) and passes it through: `std::env::var("YNAB_CLI_API_BASE_URL").ok()`.

**Behavior (exact):**
- `login`: read the PAT — if stdin is a TTY, prompt `Paste your YNAB Personal Access Token: ` with hidden input (`rpassword::prompt_password`); if stdin is NOT a TTY, read one trimmed line from stdin (enables `echo tok | ynab auth login` for scripts and tests). Empty token ⇒ `Error::Config("no token provided")`. Validate the token by calling `Client::get_user()` BEFORE storing; on success store as `SecretKind::Pat` and print `Logged in. YNAB user id: <id>`. On validation failure, store nothing and return the error.
- `status`: no stored PAT ⇒ print `Not logged in. Run \`ynab auth login\`.` and return `Err(Error::NotAuthenticated)` (so scripts get exit 1). Stored PAT ⇒ call `get_user`; success prints `Logged in (PAT). YNAB user id: <id>`; failure returns the error (e.g. revoked token surfaces as `not logged in — run \`ynab auth login\``).
- `logout`: delete `SecretKind::Pat` (and, harmlessly, the four OAuth kinds — forward-compatible with M4), print `Logged out. Credentials removed from the OS keychain.` Logging out when not logged in is fine (delete is idempotent).

- [ ] **Step 1: Add dependency**

In `Cargo.toml` `[dependencies]` add:

```toml
rpassword = "7"
```

- [ ] **Step 2: Write the failing test**

Create `src/cli/auth.rs` with the test module (uses keyring mock + wiremock; no OS keychain, no network):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::{SecretKind, SecretStore};
    use secrecy::SecretString;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn mock_store() -> SecretStore {
        keyring::set_default_credential_builder(keyring::mock::default_credential_builder());
        SecretStore::new().unwrap()
    }

    async fn user_ok_server() -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/user"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": { "user": { "id": "u-1" } }
            })))
            .mount(&server)
            .await;
        server
    }

    #[tokio::test]
    async fn status_without_token_is_not_authenticated() {
        let store = mock_store();
        let err = status(&store, None).await.unwrap_err();
        assert!(matches!(err, crate::error::Error::NotAuthenticated));
    }

    #[tokio::test]
    async fn status_with_valid_token_succeeds() {
        let store = mock_store();
        store.set(SecretKind::Pat, SecretString::from("tok")).unwrap();
        let server = user_ok_server().await;
        status(&store, Some(server.uri())).await.unwrap();
    }

    #[tokio::test]
    async fn login_with_token_validates_and_stores() {
        let store = mock_store();
        let server = user_ok_server().await;
        login_with_token(&store, SecretString::from("tok-new"), Some(server.uri()))
            .await
            .unwrap();
        assert!(store.get(SecretKind::Pat).unwrap().is_some());
    }

    #[tokio::test]
    async fn login_with_bad_token_stores_nothing() {
        let store = mock_store();
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/user"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;
        let err = login_with_token(&store, SecretString::from("bad"), Some(server.uri()))
            .await
            .unwrap_err();
        assert!(matches!(err, crate::error::Error::NotAuthenticated));
        assert!(store.get(SecretKind::Pat).unwrap().is_none());
    }

    #[tokio::test]
    async fn logout_removes_token() {
        let store = mock_store();
        store.set(SecretKind::Pat, SecretString::from("tok")).unwrap();
        logout(&store).unwrap();
        assert!(store.get(SecretKind::Pat).unwrap().is_none());
        // logout when already logged out is fine
        logout(&store).unwrap();
    }
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test cli::auth`
Expected: FAIL to compile — functions not defined.

- [ ] **Step 4: Write minimal implementation**

Prepend to `src/cli/auth.rs`:

```rust
use std::io::{BufRead, IsTerminal};

use secrecy::SecretString;

use crate::api::client::Client;
use crate::error::{Error, Result};
use crate::secrets::{SecretKind, SecretStore};

fn make_client(token: SecretString, base_url: Option<String>) -> Client {
    match base_url {
        Some(url) => Client::with_base_url(token, url),
        None => Client::new(token),
    }
}

fn read_token() -> Result<SecretString> {
    let raw = if std::io::stdin().is_terminal() {
        rpassword::prompt_password("Paste your YNAB Personal Access Token: ")?
    } else {
        let mut line = String::new();
        std::io::stdin().lock().read_line(&mut line)?;
        line
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(Error::Config("no token provided".into()));
    }
    Ok(SecretString::from(trimmed.to_string()))
}

pub async fn login(store: &SecretStore, api_base_url: Option<String>) -> Result<()> {
    let token = read_token()?;
    login_with_token(store, token, api_base_url).await
}

pub async fn login_with_token(
    store: &SecretStore,
    token: SecretString,
    api_base_url: Option<String>,
) -> Result<()> {
    let client = make_client(token.clone(), api_base_url);
    let user = client.get_user().await?;
    store.set(SecretKind::Pat, token)?;
    println!("Logged in. YNAB user id: {}", user.id);
    Ok(())
}

pub async fn status(store: &SecretStore, api_base_url: Option<String>) -> Result<()> {
    let Some(token) = store.get(SecretKind::Pat)? else {
        println!("Not logged in. Run `ynab auth login`.");
        return Err(Error::NotAuthenticated);
    };
    let client = make_client(token, api_base_url);
    let user = client.get_user().await?;
    println!("Logged in (PAT). YNAB user id: {}", user.id);
    Ok(())
}

pub fn logout(store: &SecretStore) -> Result<()> {
    for kind in [
        SecretKind::Pat,
        SecretKind::OauthClientId,
        SecretKind::OauthClientSecret,
        SecretKind::OauthAccessToken,
        SecretKind::OauthRefreshToken,
    ] {
        store.delete(kind)?;
    }
    println!("Logged out. Credentials removed from the OS keychain.");
    Ok(())
}
```

Note: `SecretString` doesn't implement `Clone` by default in secrecy 0.10 — if `token.clone()` fails to compile, restructure: expose once into a local `String` is NOT allowed (keeps secret in plain string longer than needed); instead build the client from a fresh `SecretString::from(token.expose_secret().to_string())`. Prefer whichever compiles cleanly; both keep the secret wrapped.

In `src/cli/mod.rs`: add `pub mod auth;` at the top and replace the `Command::Auth` arm of `run`:

```rust
Command::Auth { command } => {
    let store = crate::secrets::SecretStore::new()?;
    let base = std::env::var("YNAB_CLI_API_BASE_URL").ok();
    match command {
        AuthCommand::Login => auth::login(&store, base).await,
        AuthCommand::Status => auth::status(&store, base).await,
        AuthCommand::Logout => auth::logout(&store),
    }
}
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test`
Expected: PASS — all tests including the 5 new ones. None touch the real keychain or network.

- [ ] **Step 6: Commit**

```bash
git add -A && git commit -m "feat: ynab auth login/status/logout with PAT paste-in"
```

---

### Task 7: `ynab config get|set`

**Files:**
- Create: `src/cli/config_cmd.rs`, `tests/cli_config.rs`
- Modify: `src/cli/mod.rs` (add `pub mod config_cmd;`; replace the `Command::Config` arm)

**Interfaces:**
- Consumes: `config::Config` (Task 3), `cli::ConfigCommand` (Task 5).
- Produces (in `src/cli/config_cmd.rs`):
  - `pub fn get(key: &str) -> Result<()>` — loads config, prints the value, or `<unset>` when the key is valid but absent. Invalid key errors (from `Config::get_key`).
  - `pub fn set(key: &str, value: &str) -> Result<()>` — loads, `with_key`, saves, prints `<key> = <value>`.

- [ ] **Step 1: Write the failing test**

Create `tests/cli_config.rs` (integration test through the real binary, isolated via `YNAB_CLI_CONFIG_DIR`):

```rust
use assert_cmd::Command;
use predicates::prelude::*;

fn ynab(dir: &std::path::Path) -> Command {
    let mut cmd = Command::cargo_bin("ynab").unwrap();
    cmd.env("YNAB_CLI_CONFIG_DIR", dir);
    cmd
}

#[test]
fn set_then_get_roundtrip() {
    let dir = tempfile::tempdir().unwrap();

    ynab(dir.path())
        .args(["config", "set", "default_budget", "b-42"])
        .assert()
        .success()
        .stdout(predicate::str::contains("default_budget = b-42"));

    ynab(dir.path())
        .args(["config", "get", "default_budget"])
        .assert()
        .success()
        .stdout(predicate::str::contains("b-42"));
}

#[test]
fn get_unset_key_prints_unset() {
    let dir = tempfile::tempdir().unwrap();
    ynab(dir.path())
        .args(["config", "get", "cache"])
        .assert()
        .success()
        .stdout(predicate::str::contains("<unset>"));
}

#[test]
fn unknown_key_errors() {
    let dir = tempfile::tempdir().unwrap();
    ynab(dir.path())
        .args(["config", "set", "nope", "x"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unknown key"));
}

#[test]
fn config_file_never_contains_secrets_section() {
    let dir = tempfile::tempdir().unwrap();
    ynab(dir.path())
        .args(["config", "set", "cache", "false"])
        .assert()
        .success();
    let text = std::fs::read_to_string(dir.path().join("config.toml")).unwrap();
    assert!(!text.to_lowercase().contains("token"));
    assert!(!text.to_lowercase().contains("secret"));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test cli_config`
Expected: FAIL — config commands return "not implemented".

- [ ] **Step 3: Write minimal implementation**

Create `src/cli/config_cmd.rs`:

```rust
use crate::config::Config;
use crate::error::Result;

pub fn get(key: &str) -> Result<()> {
    let cfg = Config::load()?;
    match cfg.get_key(key)? {
        Some(value) => println!("{value}"),
        None => println!("<unset>"),
    }
    Ok(())
}

pub fn set(key: &str, value: &str) -> Result<()> {
    let cfg = Config::load()?.with_key(key, value)?;
    cfg.save()?;
    println!("{key} = {value}");
    Ok(())
}
```

In `src/cli/mod.rs`: add `pub mod config_cmd;` and replace the `Command::Config` arm:

```rust
Command::Config { command } => match command {
    ConfigCommand::Get { key } => config_cmd::get(&key),
    ConfigCommand::Set { key, value } => config_cmd::set(&key, &value),
},
```

Also update the `errors_go_to_stderr_with_prefix` test in `tests/cli_basics.rs` — `config get cache` now succeeds, so point it at a still-failing invocation:

```rust
#[test]
fn errors_go_to_stderr_with_prefix() {
    Command::cargo_bin("ynab")
        .unwrap()
        .args(["config", "get", "definitely-not-a-key"])
        .assert()
        .failure()
        .stderr(predicate::str::starts_with("error: "));
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test`
Expected: PASS — everything green.

- [ ] **Step 5: Milestone gate**

Run: `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check`
Expected: clean. Fix anything that isn't.

- [ ] **Step 6: Commit**

```bash
git add -A && git commit -m "feat: ynab config get/set commands"
```

---

## M1 carry-overs (from final review, 2026-08-02 — fold into later plans)

- Prune unused scaffold deps (`oauth2` re-enters at M4; `chrono` when dates need it; `anyhow` likely never — crate standardized on `thiserror`).
- Wrap token-reading intermediates in `zeroize::Zeroizing<String>` (crib cinnes-ynab-mcp pattern).
- Decide env-hook placement convention before M4's MCP frontend duplicates `YNAB_CLI_API_BASE_URL` reading; add a comment either way.
- Consider an `Error::Input` variant if more user-input errors appear (M2) — `no token provided` currently surfaces as `config error: ...`.
- Add CI grep guard for write verbs (`! grep -RE '\.(post|put|patch|delete)\(' src/`) to make the read-only guarantee mechanical.
- M3 decision: should `logout` also clear the cache encryption key / cache contents? (Today it deliberately preserves `SecretKind::CacheKey`.)
- Design note: `SecretStore` entries array order is tied to `SecretKind` discriminant order (comment + test enforced); revisit if M4 touches `SecretKind`.

## Roadmap after M1 (each gets its own plan at its checkpoint)

- **M2 — Client + list commands:** domain types (Budget, Account, Category, Payee, Transaction; milliunits, ISO dates), remaining GET endpoints, table/`--json` output layer, `budgets|accounts|categories|payees|transactions list` with `--budget` resolution (flag → config `default_budget` → `last-used`), `--since`/`--until` via API `since_date` + client-side filters (in-memory until M3).
- **M3 — Encrypted cache:** rusqlite + bundled SQLCipher, key in keychain, delta requests via `server_knowledge`, SQL-backed transaction filters, `cache clear|status`, `--no-cache`, corrupted-cache silent refetch.
- **M4 — OAuth + MCP:** bring-your-own-app OAuth (scope=read-only, localhost redirect, rotating refresh tokens), `auth login --oauth` UX, `ynab mcp serve` (rmcp, stdio) exposing the same core functions, README registration walkthrough.
