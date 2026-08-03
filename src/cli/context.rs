use secrecy::SecretString;

use crate::api::client::Client;
use crate::cache::Cache;
use crate::config::Config;
use crate::error::{Error, Result};
use crate::secrets::{SecretKind, SecretStore};

pub struct Ctx {
    pub client: Client,
    pub cache: Option<Cache>,
    pub json: bool,
    pub budget: String,
}

/// Token sources in order: YNAB_PAT env var (CI/scripts), then the OS
/// keychain. Storage remains keychain-only — the env var is read, never
/// written.
pub fn resolve_token(store: &SecretStore) -> Result<SecretString> {
    if let Ok(tok) = std::env::var("YNAB_PAT") {
        let trimmed = tok.trim();
        if !trimmed.is_empty() {
            return Ok(SecretString::from(trimmed.to_string()));
        }
    }
    store.get(SecretKind::Pat)?.ok_or(Error::NotAuthenticated)
}

pub fn resolve_budget(flag: Option<&str>, config: &Config) -> String {
    if let Some(b) = flag {
        return b.to_string();
    }
    config
        .default_budget
        .clone()
        .unwrap_or_else(|| "last-used".to_string())
}

/// Env hooks (YNAB_CLI_API_BASE_URL, YNAB_PAT) are a CLI-frontend concern;
/// the M4 MCP frontend decides its own wiring.
///
/// Cache is `Some` iff the config has caching enabled, `--no-cache` wasn't
/// passed, and the budget isn't the `last-used` alias (locked decision — the
/// alias bypasses caching since it can resolve to different budgets between
/// invocations). Cache trouble (e.g. `Cache::open` failing because there is
/// no writable data dir) must never block a read: fall back to `None`
/// silently.
pub fn build_ctx(json: bool, budget_flag: Option<&str>, no_cache: bool) -> Result<Ctx> {
    let config = Config::load()?;
    let store = SecretStore::new()?;
    let token = resolve_token(&store)?;
    let client = match std::env::var("YNAB_CLI_API_BASE_URL").ok() {
        Some(base) => Client::with_base_url(token, base),
        None => Client::new(token),
    };
    let budget = resolve_budget(budget_flag, &config);
    let cache = if config.cache_enabled() && !no_cache && budget != "last-used" {
        Cache::open(&store).ok()
    } else {
        None
    };
    Ok(Ctx {
        client,
        cache,
        json,
        budget,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::secrets::{SecretKind, SecretStore};
    use secrecy::{ExposeSecret, SecretString};

    fn mock_store() -> SecretStore {
        keyring::set_default_credential_builder(keyring::mock::default_credential_builder());
        SecretStore::new().unwrap()
    }

    #[test]
    fn budget_resolution_order() {
        let cfg = Config {
            cache: None,
            default_budget: Some("cfg-b".into()),
        };
        assert_eq!(resolve_budget(Some("flag-b"), &cfg), "flag-b");
        assert_eq!(resolve_budget(None, &cfg), "cfg-b");
        let empty = Config::default();
        assert_eq!(resolve_budget(None, &empty), "last-used");
    }

    #[test]
    fn token_prefers_keychain_when_no_env() {
        // Serial-safety: this test must not run with YNAB_PAT set; the
        // binary-level test in tests/cli_lists.rs covers the env path.
        let store = mock_store();
        store
            .set(SecretKind::Pat, SecretString::from("kc-tok"))
            .unwrap();
        let tok = resolve_token(&store).unwrap();
        assert_eq!(tok.expose_secret(), "kc-tok");
    }

    #[test]
    fn token_missing_is_not_authenticated() {
        let store = mock_store();
        let err = resolve_token(&store).unwrap_err();
        assert!(matches!(err, crate::error::Error::NotAuthenticated));
    }
}
