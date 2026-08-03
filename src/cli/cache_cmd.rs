use crate::cache::Cache;
use crate::config::Config;
use crate::error::Result;
use crate::output;
use crate::secrets::SecretStore;

const DEFAULT_BUDGET_TIP: &str =
    "Tip: set a default budget (ynab config set default_budget <id>) to enable delta caching.";

/// Delta caching requires a concrete `default_budget` (CLAUDE.md — the
/// `last-used` alias bypasses the cache). Nudge users who haven't set one
/// yet, but only while caching itself is still enabled — no point telling
/// someone who opted out of caching to configure it.
fn print_default_budget_tip_if_relevant(config: &Config) {
    if config.cache_enabled() && config.default_budget.is_none() {
        println!("{DEFAULT_BUDGET_TIP}");
    }
}

pub fn status(json: bool) -> Result<()> {
    let path = Cache::db_path()?;
    let config = Config::load()?;
    if !path.exists() {
        if json {
            return output::print_json(&serde_json::json!({
                "path": path.display().to_string(), "exists": false
            }));
        }
        println!("Cache: empty ({})", path.display());
        print_default_budget_tip_if_relevant(&config);
        return Ok(());
    }
    let size = std::fs::metadata(&path)?.len();

    // `open_readonly_probe` never deletes or otherwise modifies the file —
    // `cache status` must stay strictly read-only (CLAUDE.md / M3 review
    // finding 1). A corrupt/undecryptable cache is reported, not repaired.
    let cache = match Cache::open_readonly_probe(&SecretStore::new()?, &path)? {
        Some(cache) => cache,
        None => {
            if json {
                return output::print_json(&serde_json::json!({
                    "path": path.display().to_string(),
                    "exists": true,
                    "readable": false
                }));
            }
            println!(
                "Cache: unreadable, will be rebuilt on next use ({})",
                path.display()
            );
            return Ok(());
        }
    };
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
    crate::cache::remove_db_and_siblings(&path);
    println!("Cache cleared.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mock_store() -> SecretStore {
        keyring::set_default_credential_builder(keyring::mock::default_credential_builder());
        SecretStore::new().unwrap()
    }

    #[test]
    fn status_displays_populated_cache() {
        let _guard = crate::cache::tests::TEST_LOCK.lock().unwrap();
        let store = mock_store();
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path();
        let config_dir = tempfile::tempdir().unwrap();
        temp_env::with_vars(
            [
                (
                    "YNAB_CLI_DATA_DIR",
                    Some(data_dir.to_string_lossy().to_string()),
                ),
                (
                    "YNAB_CLI_CONFIG_DIR",
                    Some(config_dir.path().to_string_lossy().to_string()),
                ),
            ],
            || {
                // Create and populate cache
                {
                    let mut cache = Cache::open_at(&store, &data_dir.join("cache.db")).unwrap();
                    cache
                        .upsert_entities(
                            "b-1",
                            "transactions",
                            &[
                                (
                                    "t-1".into(),
                                    serde_json::json!({"id": "t-1", "date": "2026-07-01"}),
                                ),
                                (
                                    "t-2".into(),
                                    serde_json::json!({"id": "t-2", "date": "2026-07-02"}),
                                ),
                            ],
                        )
                        .unwrap();
                    cache
                        .set_server_knowledge("b-1", "transactions", 10)
                        .unwrap();
                }

                // Now test status doesn't panic and prints properly
                let result = status(false);
                assert!(result.is_ok());
            },
        );
    }

    #[test]
    fn status_reports_unreadable_cache_without_deleting_it() {
        let _guard = crate::cache::tests::TEST_LOCK.lock().unwrap();
        let _store = mock_store();
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path();
        let db_path = data_dir.join("cache.db");
        let garbage = b"not a real database".to_vec();
        std::fs::write(&db_path, &garbage).unwrap();
        let config_dir = tempfile::tempdir().unwrap();

        temp_env::with_vars(
            [
                (
                    "YNAB_CLI_DATA_DIR",
                    Some(data_dir.to_string_lossy().to_string()),
                ),
                (
                    "YNAB_CLI_CONFIG_DIR",
                    Some(config_dir.path().to_string_lossy().to_string()),
                ),
            ],
            || {
                let result = status(false);
                assert!(result.is_ok());
            },
        );

        // Read-only probe must never discard the unreadable file.
        assert!(db_path.exists());
        assert_eq!(std::fs::read(&db_path).unwrap(), garbage);
    }
}
