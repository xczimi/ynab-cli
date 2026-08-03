use rmcp::schemars;
use serde::Deserialize;

/// Shared by `list_accounts`, `list_categories`, `list_payees`. Budget
/// resolution mirrors the CLI's `--budget` flag: this value, then config
/// `default_budget`, then the API's `last-used` alias.
#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
pub struct BudgetScopedParams {
    /// Budget id. Defaults to the configured default budget, then YNAB's
    /// `last-used` alias.
    pub budget_id: Option<String>,
}

/// Parameters for `list_transactions`. Filters combine with AND, mirroring
/// `ynab transactions list`'s flags exactly.
#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
pub struct ListTransactionsParams {
    /// Budget id. Defaults to the configured default budget, then YNAB's
    /// `last-used` alias.
    pub budget_id: Option<String>,
    /// Only transactions on or after this ISO date (YYYY-MM-DD).
    pub since: Option<String>,
    /// Only transactions on or before this ISO date (YYYY-MM-DD).
    pub until: Option<String>,
    /// Payee id, or case-insensitive name substring.
    pub payee: Option<String>,
    /// Account id, or case-insensitive name substring.
    pub account: Option<String>,
    /// Category id, or case-insensitive name substring.
    pub category: Option<String>,
    /// Only transactions with no category.
    #[serde(default)]
    pub uncategorized: bool,
    /// Only unapproved transactions.
    #[serde(default)]
    pub unapproved: bool,
}
