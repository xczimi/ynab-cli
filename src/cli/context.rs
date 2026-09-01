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

/// Token sources in order: YNAB_PAT env var (CI/scripts) → keychain PAT →
/// OAuth access token (refreshed transparently if needed). Storage remains
/// keychain-only — the env var is read, never written.
pub async fn resolve_token(store: &SecretStore) -> Result<SecretString> {
    if let Ok(tok) = std::env::var("YNAB_PAT") {
        let trimmed = tok.trim();
        if !trimmed.is_empty() {
            return Ok(SecretString::from(trimmed.to_string()));
        }
    }
    if let Some(pat) = store.get(SecretKind::Pat)? {
        return Ok(pat);
    }
    match crate::auth::oauth::current_access_token(store).await {
        Ok(Some(token)) => Ok(token),
        Ok(None) | Err(Error::NotAuthenticated) => Err(Error::NotAuthenticated),
        Err(e) => Err(e),
    }
}

/// A budget ref is either the literal alias `last-used` or a loosely
/// UUID-shaped id: exactly 36 characters drawn from `[0-9a-fA-F-]`. This is
/// intentionally not full UUID validation (hyphen positions aren't checked)
/// — it exists to reject garbage before it reaches the network, not to
/// police YNAB's id format. Because the charset excludes anything that would
/// need percent-encoding, no URL-encoding of the budget segment is needed.
fn is_valid_budget_ref(s: &str) -> bool {
    s == "last-used"
        || (s.chars().count() == 36 && s.chars().all(|c| c.is_ascii_hexdigit() || c == '-'))
}

pub fn resolve_budget(flag: Option<&str>, config: &Config) -> Result<String> {
    let budget = match flag {
        Some(b) => b.to_string(),
        None => config
            .default_budget
            .clone()
            .unwrap_or_else(|| "last-used".to_string()),
    };
    if is_valid_budget_ref(&budget) {
        Ok(budget)
    } else {
        Err(Error::Config(
            "budget must be a budget id (UUID) or 'last-used'".into(),
        ))
    }
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
pub async fn build_ctx(json: bool, budget_flag: Option<&str>, no_cache: bool) -> Result<Ctx> {
    let config = Config::load()?;
    let store = SecretStore::new()?;
    let token = resolve_token(&store).await?;
    let client = match std::env::var("YNAB_CLI_API_BASE_URL").ok() {
        Some(base) => Client::with_base_url(token, base),
        None => Client::new(token),
    };
    let budget = resolve_budget(budget_flag, &config)?;
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
        let flag_budget = "11111111-1111-1111-1111-111111111111";
        let cfg_budget = "22222222-2222-2222-2222-222222222222";
        let cfg = Config {
            cache: None,
            default_budget: Some(cfg_budget.into()),
        };
        assert_eq!(
            resolve_budget(Some(flag_budget), &cfg).unwrap(),
            flag_budget
        );
        assert_eq!(resolve_budget(None, &cfg).unwrap(), cfg_budget);
        let empty = Config::default();
        assert_eq!(resolve_budget(None, &empty).unwrap(), "last-used");
    }

    #[test]
    fn budget_resolution_rejects_invalid_ref() {
        let cfg = Config::default();
        let err = resolve_budget(Some("not-a-valid-budget-id"), &cfg).unwrap_err();
        assert_eq!(
            err.to_string(),
            "config error: budget must be a budget id (UUID) or 'last-used'"
        );

        let bad_cfg = Config {
            cache: None,
            default_budget: Some("short".into()),
        };
        let err = resolve_budget(None, &bad_cfg).unwrap_err();
        assert_eq!(
            err.to_string(),
            "config error: budget must be a budget id (UUID) or 'last-used'"
        );
    }

    // Plain `#[test]` + `Runtime::block_on` (not `#[tokio::test]`), matching
    // `auth::oauth`'s tests: holding the std `TEST_LOCK` `MutexGuard` across
    // an `.await` inside an `async fn` trips clippy's `await_holding_lock`; a
    // sync test driving its own runtime avoids that entirely.

    #[test]
    fn token_prefers_keychain_when_no_env() {
        // Serial-safety: this test must not run with YNAB_PAT set; the
        // binary-level test in tests/cli_lists.rs covers the env path.
        let _guard = crate::cache::tests::TEST_LOCK.lock().unwrap();
        let store = mock_store();
        store
            .set(SecretKind::Pat, SecretString::from("kc-tok"))
            .unwrap();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let tok = rt.block_on(resolve_token(&store)).unwrap();
        assert_eq!(tok.expose_secret(), "kc-tok");
    }

    #[test]
    fn token_missing_is_not_authenticated() {
        let _guard = crate::cache::tests::TEST_LOCK.lock().unwrap();
        let store = mock_store();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let err = rt.block_on(resolve_token(&store)).unwrap_err();
        assert!(matches!(err, crate::error::Error::NotAuthenticated));
    }

    #[test]
    fn token_resolution_pat_beats_oauth() {
        let _guard = crate::cache::tests::TEST_LOCK.lock().unwrap();
        let store = mock_store();
        store
            .set(SecretKind::Pat, SecretString::from("pat-tok"))
            .unwrap();
        let stored_oauth = serde_json::json!({
            "access_token": "oauth-tok",
            "expires_at": crate::auth::unix_now() + 3600,
        });
        store
            .set(
                SecretKind::Oauth,
                SecretString::from(stored_oauth.to_string()),
            )
            .unwrap();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let tok = rt.block_on(resolve_token(&store)).unwrap();
        assert_eq!(tok.expose_secret(), "pat-tok");
    }
}
