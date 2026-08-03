//! MCP frontend (`ynab mcp serve`): a stdio MCP server exposing the same
//! read-only core the CLI uses. Every tool below resolves its own [`Ctx`]
//! per call via [`build_ctx`] and calls the exact fetch path (client, or
//! cache-then-fallback) that the equivalent CLI list command uses — see
//! CLAUDE.md's "read-only is structural" principle: no write verb exists
//! anywhere in this binary, MCP included.
//!
//! Tool results carry the raw JSON envelope (identical to the CLI's
//! `--json` output) as `structured_content`. Errors never propagate a
//! token: they are our [`crate::error::Error`] display strings, surfaced as
//! tool-level errors (`CallToolResult::error`) so the calling MCP client
//! renders them, per rmcp's guidance that protocol errors (`Err(ErrorData)`)
//! are rendered opaquely and should be reserved for truly unroutable
//! requests.

mod params;

use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock};
use rmcp::{ErrorData as McpError, ServerHandler, ServiceExt, tool, tool_handler, tool_router};

use crate::api::client::ListResult;
use crate::api::types::{AccountsWrapper, CategoryGroupsWrapper, PayeesWrapper};
use crate::cli::context::{Ctx, build_ctx};
use crate::cli::transactions::{self, Filters};
use crate::error::cache_error;
use params::{BudgetScopedParams, ListTransactionsParams};

#[derive(Clone)]
pub struct YnabServer {
    tool_router: rmcp::handler::server::router::tool::ToolRouter<Self>,
}

#[tool_router]
impl YnabServer {
    pub fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        name = "list_budgets",
        description = "List all budgets. Returns the same raw JSON envelope as `ynab budgets list --json`."
    )]
    async fn list_budgets(&self) -> Result<CallToolResult, McpError> {
        to_tool_result(fetch_budgets().await)
    }

    #[tool(
        name = "list_accounts",
        description = "List accounts for a budget. Returns the same raw JSON envelope as `ynab accounts list --json`. Optional `budget_id`: falls back to the configured default budget, then YNAB's `last-used` alias."
    )]
    async fn list_accounts(
        &self,
        Parameters(BudgetScopedParams { budget_id }): Parameters<BudgetScopedParams>,
    ) -> Result<CallToolResult, McpError> {
        to_tool_result(fetch_accounts(budget_id.as_deref()).await)
    }

    #[tool(
        name = "list_categories",
        description = "List categories for a budget. Returns the same raw JSON envelope as `ynab categories list --json`. Optional `budget_id`: falls back to the configured default budget, then YNAB's `last-used` alias."
    )]
    async fn list_categories(
        &self,
        Parameters(BudgetScopedParams { budget_id }): Parameters<BudgetScopedParams>,
    ) -> Result<CallToolResult, McpError> {
        to_tool_result(fetch_categories(budget_id.as_deref()).await)
    }

    #[tool(
        name = "list_payees",
        description = "List payees for a budget. Returns the same raw JSON envelope as `ynab payees list --json`. Optional `budget_id`: falls back to the configured default budget, then YNAB's `last-used` alias."
    )]
    async fn list_payees(
        &self,
        Parameters(BudgetScopedParams { budget_id }): Parameters<BudgetScopedParams>,
    ) -> Result<CallToolResult, McpError> {
        to_tool_result(fetch_payees(budget_id.as_deref()).await)
    }

    #[tool(
        name = "list_transactions",
        description = "List transactions for a budget with client-side filters (filters combine with AND). Same `--json` semantics as `ynab transactions list`: only the explicit filters given apply, deleted transactions are kept, and the full response envelope (e.g. `server_knowledge`) is preserved. Optional `budget_id`: falls back to the configured default budget, then YNAB's `last-used` alias."
    )]
    async fn list_transactions(
        &self,
        Parameters(params): Parameters<ListTransactionsParams>,
    ) -> Result<CallToolResult, McpError> {
        let budget_id = params.budget_id.clone();
        let filters = Filters {
            since: params.since,
            until: params.until,
            payee: params.payee,
            account: params.account,
            category: params.category,
            uncategorized: params.uncategorized,
            unapproved: params.unapproved,
        };
        to_tool_result(fetch_transactions(budget_id.as_deref(), filters).await)
    }
}

#[tool_handler(
    router = self.tool_router,
    name = "ynab-cli",
    instructions = "Absolutely read-only MCP server for the YNAB API. Every tool issues GET requests only, via the same read-only client and cache the CLI uses — no write operations exist anywhere in this binary."
)]
impl ServerHandler for YnabServer {}

impl Default for YnabServer {
    fn default() -> Self {
        Self::new()
    }
}

/// Maps a core fetch result to an MCP tool result. Success becomes
/// structured JSON content (the raw envelope). Failure becomes a
/// tool-level error carrying our `Error` display string — never a token —
/// so the caller's MCP client renders it (protocol errors are rendered
/// opaquely by most clients, so they're reserved for truly unroutable
/// requests, none of which occur on these read-only, all-optional-param
/// tools).
fn to_tool_result(
    result: crate::error::Result<serde_json::Value>,
) -> Result<CallToolResult, McpError> {
    match result {
        Ok(value) => Ok(CallToolResult::structured(value)),
        Err(e) => Ok(CallToolResult::error(vec![ContentBlock::text(
            e.to_string(),
        )])),
    }
}

async fn fetch_budgets() -> crate::error::Result<serde_json::Value> {
    let ctx = build_ctx(true, None, false).await?;
    let result = ctx.client.get_budgets().await?;
    Ok(result.raw)
}

async fn fetch_accounts(budget_id: Option<&str>) -> crate::error::Result<serde_json::Value> {
    let mut ctx = build_ctx(true, budget_id, false).await?;
    let Ctx {
        client,
        cache,
        budget,
        ..
    } = &mut ctx;
    let result: ListResult<AccountsWrapper> = match cache {
        Some(cache) => match crate::cache::sync::accounts(client, cache, budget).await {
            Ok(r) => r,
            Err(e) if cache_error(&e) => client.get_accounts(budget, None).await?,
            Err(e) => return Err(e),
        },
        None => client.get_accounts(budget, None).await?,
    };
    Ok(result.raw)
}

async fn fetch_categories(budget_id: Option<&str>) -> crate::error::Result<serde_json::Value> {
    let mut ctx = build_ctx(true, budget_id, false).await?;
    let Ctx {
        client,
        cache,
        budget,
        ..
    } = &mut ctx;
    let result: ListResult<CategoryGroupsWrapper> = match cache {
        Some(cache) => match crate::cache::sync::categories(client, cache, budget).await {
            Ok(r) => r,
            Err(e) if cache_error(&e) => client.get_categories(budget, None).await?,
            Err(e) => return Err(e),
        },
        None => client.get_categories(budget, None).await?,
    };
    Ok(result.raw)
}

async fn fetch_payees(budget_id: Option<&str>) -> crate::error::Result<serde_json::Value> {
    let mut ctx = build_ctx(true, budget_id, false).await?;
    let Ctx {
        client,
        cache,
        budget,
        ..
    } = &mut ctx;
    let result: ListResult<PayeesWrapper> = match cache {
        Some(cache) => match crate::cache::sync::payees(client, cache, budget).await {
            Ok(r) => r,
            Err(e) if cache_error(&e) => client.get_payees(budget, None).await?,
            Err(e) => return Err(e),
        },
        None => client.get_payees(budget, None).await?,
    };
    Ok(result.raw)
}

async fn fetch_transactions(
    budget_id: Option<&str>,
    filters: Filters,
) -> crate::error::Result<serde_json::Value> {
    let since = filters
        .since
        .as_deref()
        .map(|s| transactions::normalize_date(s, "--since"))
        .transpose()?;
    let until = filters
        .until
        .as_deref()
        .map(|u| transactions::normalize_date(u, "--until"))
        .transpose()?;
    let filters = Filters {
        since,
        until,
        ..filters
    };

    let mut ctx = build_ctx(true, budget_id, false).await?;
    let Ctx {
        client,
        cache,
        budget,
        ..
    } = &mut ctx;
    // A corrupted cache is never a user-facing error (CLAUDE.md): if the
    // sync path fails mid-operation, fall back to the same direct fetch the
    // no-cache arm uses — identical to `cli::transactions::list`.
    let result = match cache {
        Some(cache) => match crate::cache::sync::transactions(client, cache, budget).await {
            Ok(r) => r,
            Err(e) if cache_error(&e) => {
                client
                    .get_transactions(budget, filters.since.as_deref(), None)
                    .await?
            }
            Err(e) => return Err(e),
        },
        None => {
            client
                .get_transactions(budget, filters.since.as_deref(), None)
                .await?
        }
    };
    Ok(transactions::filtered_raw_transactions(&result, &filters))
}

/// Runs the MCP server over stdio until the client disconnects.
///
/// Two distinct EOF points, both treated as a clean exit (`Ok(())`, so the
/// binary exits 0):
/// - stdin closes before any client ever completes the MCP handshake (e.g.
///   a bare `echo -n | ynab mcp serve`, or a health check). Verified against
///   rmcp 3.1.0: `.serve(stdio())` does NOT hang or return `Ok` here — it
///   returns `Err(ServerInitializeError::ConnectionClosed(_))`, which the
///   brief's sketch (`.serve(...).await?`) did not anticipate. That variant
///   is mapped to a clean shutdown; every other `ServerInitializeError`
///   variant (malformed handshake, unsupported protocol version, ...) stays
///   a hard error.
/// - stdin closes after a real session. `RunningService::waiting` returns
///   `Result<QuitReason, tokio::task::JoinError>`: the outer `Err` is a
///   `JoinError` from awaiting the service's own join handle (extremely
///   rare — the runtime itself misbehaving). The inner `QuitReason` (per
///   rmcp 3.1.0's `service.rs`) is `#[non_exhaustive]` with `Closed` and
///   `Cancelled` as clean-quit variants, but ALSO a `JoinError(JoinError)`
///   variant: the service loop's own task-driving code catches a panicked
///   handler task and returns it as `Ok(QuitReason::JoinError(e))`, not as
///   the outer `Err`. Treating every `Ok(_)` as success (the prior
///   behaviour) silently exits 0 on a handler panic; a caller relying on
///   the exit code to detect a crashed MCP server would never notice.
pub async fn serve() -> crate::error::Result<()> {
    let service = match YnabServer::new().serve(rmcp::transport::stdio()).await {
        Ok(service) => service,
        Err(rmcp::service::ServerInitializeError::ConnectionClosed(_)) => return Ok(()),
        Err(e) => {
            return Err(crate::error::Error::Config(format!(
                "mcp server failed to start: {e}"
            )));
        }
    };
    match service.waiting().await {
        Ok(rmcp::service::QuitReason::Closed) | Ok(rmcp::service::QuitReason::Cancelled) => Ok(()),
        Ok(rmcp::service::QuitReason::JoinError(e)) => Err(crate::error::Error::Config(format!(
            "MCP server task failed: {e}"
        ))),
        // `QuitReason` is `#[non_exhaustive]`: treat any variant rmcp adds
        // later as a clean quit rather than failing closed on an unknown
        // shutdown reason.
        Ok(_) => Ok(()),
        Err(e) => Err(crate::error::Error::Config(format!(
            "mcp server error: {e}"
        ))),
    }
}

// `TEST_LOCK` (a plain `std::sync::Mutex`, defined in `crate::cache`) is held
// across `.await` in every test below — deliberately: these tests set
// process-global env vars (`YNAB_CLI_CONFIG_DIR`, `YNAB_PAT`, ...) that
// `build_ctx` reads mid-fetch, and the lock's whole job (see
// `crate::cache::tests::TEST_LOCK`'s doc comment) is to serialize every test
// that touches that state — including the async fetch itself — against every
// other one, crate-wide. Shrinking the guard to only the env-var
// set/teardown would reopen exactly the race it exists to close.
#[cfg(test)]
#[allow(clippy::await_holding_lock)]
mod tests {
    use super::*;
    use crate::secrets::SecretStore;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn mock_store() -> SecretStore {
        keyring::set_default_credential_builder(keyring::mock::default_credential_builder());
        SecretStore::new().unwrap()
    }

    /// Common env wiring for a test: config dir + data dir are fresh
    /// tempdirs, base URL points at the wiremock server, and a PAT is
    /// present so `build_ctx` never hits the keychain. Mirrors
    /// `tests/cli_lists.rs`'s `ynab()` helper, but in-process.
    ///
    /// `temp_env` only supports synchronous closures (it restores vars as
    /// soon as the closure *returns*, which for `|| async { .. }` is
    /// immediately — before the async body runs). So this sets/clears the
    /// vars by hand around an explicit `.await`, relying on the caller
    /// holding `TEST_LOCK` for the whole call (same serialization
    /// contract `crate::cache::tests` documents for env-touching tests).
    async fn with_env<Fut, T>(base_url: &str, f: impl FnOnce() -> Fut) -> T
    where
        Fut: std::future::Future<Output = T>,
    {
        let config_dir = tempfile::tempdir().unwrap();
        let data_dir = tempfile::tempdir().unwrap();
        // SAFETY: caller holds `TEST_LOCK`, serializing against every other
        // test that reads/writes these process-global vars.
        unsafe {
            std::env::set_var("YNAB_CLI_CONFIG_DIR", config_dir.path());
            std::env::set_var("YNAB_CLI_DATA_DIR", data_dir.path());
            std::env::set_var("YNAB_CLI_API_BASE_URL", base_url);
            std::env::set_var("YNAB_PAT", "e2e-token");
        }
        let result = f().await;
        unsafe {
            std::env::remove_var("YNAB_CLI_CONFIG_DIR");
            std::env::remove_var("YNAB_CLI_DATA_DIR");
            std::env::remove_var("YNAB_CLI_API_BASE_URL");
            std::env::remove_var("YNAB_PAT");
        }
        result
    }

    fn structured(result: &CallToolResult) -> &serde_json::Value {
        result
            .structured_content
            .as_ref()
            .expect("expected structured content")
    }

    fn error_text(result: &CallToolResult) -> String {
        assert_eq!(result.is_error, Some(true));
        result.content[0].as_text().unwrap().text.clone()
    }

    #[tokio::test]
    async fn list_budgets_returns_raw_envelope() {
        let _guard = crate::cache::tests::TEST_LOCK.lock().unwrap();
        let _store = mock_store();
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/budgets"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": { "budgets": [
                    { "id": "b-1", "name": "Family", "first_month": "2025-01-01",
                      "last_month": "2026-08-01", "extra": 7 }
                ], "default_budget": null }
            })))
            .mount(&server)
            .await;

        let uri = server.uri();
        let result = with_env(&uri, || async {
            YnabServer::new().list_budgets().await.unwrap()
        })
        .await;

        let value = structured(&result);
        assert_eq!(value["budgets"][0]["name"], "Family");
        assert_eq!(value["budgets"][0]["extra"], 7); // unknown fields survive
    }

    #[tokio::test]
    async fn list_accounts_uses_explicit_budget_id() {
        let _guard = crate::cache::tests::TEST_LOCK.lock().unwrap();
        let _store = mock_store();
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(
                "/budgets/99999999-9999-9999-9999-999999999999/accounts",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": { "accounts": [
                    { "id": "a-1", "name": "Chequing", "type": "checking",
                      "on_budget": true, "closed": false, "balance": 100500,
                      "deleted": false }
                ], "server_knowledge": 5 }
            })))
            .mount(&server)
            .await;

        let uri = server.uri();
        let result = with_env(&uri, || async {
            YnabServer::new()
                .list_accounts(Parameters(BudgetScopedParams {
                    budget_id: Some("99999999-9999-9999-9999-999999999999".into()),
                }))
                .await
                .unwrap()
        })
        .await;

        let value = structured(&result);
        assert_eq!(value["accounts"][0]["name"], "Chequing");
        assert_eq!(value["server_knowledge"], 5);
    }

    #[tokio::test]
    async fn list_categories_returns_raw_envelope() {
        let _guard = crate::cache::tests::TEST_LOCK.lock().unwrap();
        let _store = mock_store();
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(
                "/budgets/11111111-1111-1111-1111-111111111111/categories",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": { "category_groups": [
                    { "id": "g-1", "name": "Bills", "hidden": false, "deleted": false,
                      "categories": [] }
                ], "server_knowledge": 1 }
            })))
            .mount(&server)
            .await;

        let uri = server.uri();
        let result = with_env(&uri, || async {
            YnabServer::new()
                .list_categories(Parameters(BudgetScopedParams {
                    budget_id: Some("11111111-1111-1111-1111-111111111111".into()),
                }))
                .await
                .unwrap()
        })
        .await;

        let value = structured(&result);
        assert_eq!(value["category_groups"][0]["name"], "Bills");
    }

    #[tokio::test]
    async fn list_payees_returns_raw_envelope() {
        let _guard = crate::cache::tests::TEST_LOCK.lock().unwrap();
        let _store = mock_store();
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/budgets/11111111-1111-1111-1111-111111111111/payees"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": { "payees": [
                    { "id": "p-1", "name": "Grocer", "deleted": false }
                ], "server_knowledge": 1 }
            })))
            .mount(&server)
            .await;

        let uri = server.uri();
        let result = with_env(&uri, || async {
            YnabServer::new()
                .list_payees(Parameters(BudgetScopedParams {
                    budget_id: Some("11111111-1111-1111-1111-111111111111".into()),
                }))
                .await
                .unwrap()
        })
        .await;

        let value = structured(&result);
        assert_eq!(value["payees"][0]["name"], "Grocer");
    }

    #[tokio::test]
    async fn list_transactions_filters_keeps_deleted_and_envelope() {
        let _guard = crate::cache::tests::TEST_LOCK.lock().unwrap();
        let _store = mock_store();
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(
                "/budgets/11111111-1111-1111-1111-111111111111/transactions",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": { "transactions": [
                    { "id": "t-1", "date": "2026-07-15", "amount": -12340,
                      "memo": "weekly shop", "approved": true,
                      "account_id": "a-1", "account_name": "Chequing",
                      "payee_id": "p-1", "payee_name": "Corner Grocer",
                      "category_id": "c-1", "category_name": "Groceries",
                      "deleted": false, "subtransactions": [] },
                    { "id": "t-2", "date": "2026-07-20", "amount": -5000,
                      "memo": null, "approved": false,
                      "account_id": "a-1", "account_name": "Chequing",
                      "payee_id": null, "payee_name": "Mystery",
                      "category_id": null, "category_name": null,
                      "deleted": true, "subtransactions": [] }
                ], "server_knowledge": 9 }
            })))
            .mount(&server)
            .await;

        let uri = server.uri();
        let result = with_env(&uri, || async {
            YnabServer::new()
                .list_transactions(Parameters(ListTransactionsParams {
                    budget_id: Some("11111111-1111-1111-1111-111111111111".into()),
                    uncategorized: true,
                    ..Default::default()
                }))
                .await
                .unwrap()
        })
        .await;

        let value = structured(&result);
        let ids: Vec<&str> = value["transactions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["id"].as_str().unwrap())
            .collect();
        // uncategorized filter keeps only t-2; deleted is NOT dropped
        // (--json semantics); envelope (server_knowledge) is preserved.
        assert_eq!(ids, vec!["t-2"]);
        assert_eq!(value["transactions"][0]["deleted"], true);
        assert_eq!(value["server_knowledge"], 9);
    }

    #[tokio::test]
    async fn list_transactions_invalid_date_is_tool_error_with_cli_message() {
        let _guard = crate::cache::tests::TEST_LOCK.lock().unwrap();
        let _store = mock_store();
        let uri = "http://localhost:9".to_string(); // never reached

        let result = with_env(&uri, || async {
            YnabServer::new()
                .list_transactions(Parameters(ListTransactionsParams {
                    since: Some("yesterday".into()),
                    ..Default::default()
                }))
                .await
                .unwrap()
        })
        .await;

        assert_eq!(
            error_text(&result),
            "config error: --since must be an ISO date (YYYY-MM-DD)"
        );
    }

    #[tokio::test]
    async fn not_authenticated_maps_to_tool_error_never_a_token() {
        let _guard = crate::cache::tests::TEST_LOCK.lock().unwrap();
        let _store = mock_store();
        let config_dir = tempfile::tempdir().unwrap();
        let data_dir = tempfile::tempdir().unwrap();
        // No YNAB_PAT set and nothing in the (mock, empty) keychain.
        // SAFETY: guarded by TEST_LOCK, see `with_env` above.
        unsafe {
            std::env::set_var("YNAB_CLI_CONFIG_DIR", config_dir.path());
            std::env::set_var("YNAB_CLI_DATA_DIR", data_dir.path());
            std::env::set_var("YNAB_CLI_API_BASE_URL", "http://localhost:9");
            std::env::remove_var("YNAB_PAT");
        }
        let result = YnabServer::new().list_budgets().await.unwrap();
        unsafe {
            std::env::remove_var("YNAB_CLI_CONFIG_DIR");
            std::env::remove_var("YNAB_CLI_DATA_DIR");
            std::env::remove_var("YNAB_CLI_API_BASE_URL");
        }

        let text = error_text(&result);
        assert_eq!(
            text,
            "not logged in — run `ynab auth login` (or `ynab auth login --oauth`)"
        );
        assert!(!text.contains("token"));
    }
}
