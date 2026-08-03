pub mod accounts;
pub mod auth;
pub mod budgets;
pub mod cache_cmd;
pub mod categories;
pub mod config_cmd;
pub mod context;
pub mod payees;
pub mod transactions;

use clap::{Parser, Subcommand};

use crate::error::Result;

#[derive(Debug, Parser)]
#[command(
    name = "ynab",
    version,
    about = "Absolutely read-only CLI for the YNAB API"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
    /// Output raw API JSON instead of a table
    #[arg(long, global = true)]
    pub json: bool,
    /// Budget id (default: config default_budget, then the API's last-used)
    #[arg(long, global = true, value_name = "BUDGET")]
    pub budget: Option<String>,
    /// Bypass the local cache for this invocation
    #[arg(long, global = true)]
    pub no_cache: bool,
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
    /// List budgets
    Budgets {
        #[command(subcommand)]
        command: BudgetsCommand,
    },
    /// List accounts
    Accounts {
        #[command(subcommand)]
        command: AccountsCommand,
    },
    /// List categories
    Categories {
        #[command(subcommand)]
        command: CategoriesCommand,
    },
    /// List payees
    Payees {
        #[command(subcommand)]
        command: PayeesCommand,
    },
    /// List transactions
    Transactions {
        #[command(subcommand)]
        command: TransactionsCommand,
    },
    /// Inspect or clear the local cache
    Cache {
        #[command(subcommand)]
        command: CacheCommand,
    },
    /// Run the MCP server
    Mcp {
        #[command(subcommand)]
        command: McpCommand,
    },
}

#[derive(Debug, Subcommand)]
pub enum BudgetsCommand {
    /// List all budgets
    List,
}

#[derive(Debug, Subcommand)]
pub enum AccountsCommand {
    /// List all accounts
    List,
}

#[derive(Debug, Subcommand)]
pub enum CategoriesCommand {
    /// List all categories
    List,
}

#[derive(Debug, Subcommand)]
pub enum PayeesCommand {
    /// List all payees
    List,
}

#[derive(Debug, Subcommand)]
pub enum TransactionsCommand {
    /// List transactions (filters combine with AND)
    ///
    /// `--json` applies the explicit filters given above but keeps deleted
    /// transactions and preserves the full response envelope (e.g.
    /// `server_knowledge`).
    List {
        /// Only transactions on or after this ISO date (sent to the API)
        #[arg(long, value_name = "YYYY-MM-DD")]
        since: Option<String>,
        /// Only transactions on or before this ISO date
        #[arg(long, value_name = "YYYY-MM-DD")]
        until: Option<String>,
        /// Payee id, or case-insensitive name substring
        #[arg(long)]
        payee: Option<String>,
        /// Account id, or case-insensitive name substring
        #[arg(long)]
        account: Option<String>,
        /// Category id, or case-insensitive name substring
        #[arg(long)]
        category: Option<String>,
        /// Only transactions with no category
        #[arg(long)]
        uncategorized: bool,
        /// Only unapproved transactions
        #[arg(long)]
        unapproved: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum AuthCommand {
    /// Log in with a YNAB Personal Access Token, or via OAuth with --oauth
    Login {
        /// Use the OAuth Authorization Code flow instead of a pasted PAT
        #[arg(long)]
        oauth: bool,
        /// Force re-prompting for OAuth app credentials (client id/secret)
        #[arg(long)]
        reset_app: bool,
    },
    /// Show whether you are logged in and the token works
    Status,
    /// Remove stored credentials and cached data
    Logout,
}

#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    /// Print a config value
    Get { key: String },
    /// Set a config value
    Set { key: String, value: String },
}

#[derive(Debug, Subcommand)]
pub enum CacheCommand {
    /// Show cache status
    Status,
    /// Clear the cache
    Clear,
}

#[derive(Debug, Subcommand)]
pub enum McpCommand {
    /// Serve the MCP server over stdio until the client disconnects
    Serve,
}

pub async fn run(cli: Cli) -> Result<()> {
    let json = cli.json;
    let budget = cli.budget.clone();
    let no_cache = cli.no_cache;
    match cli.command {
        Command::Auth { command } => {
            let store = crate::secrets::SecretStore::new()?;
            let base = std::env::var("YNAB_CLI_API_BASE_URL").ok();
            match command {
                AuthCommand::Login { oauth, reset_app } => {
                    if oauth {
                        crate::auth::oauth::login(&store, reset_app).await
                    } else {
                        auth::login(&store, base).await
                    }
                }
                AuthCommand::Status => auth::status(&store, base).await,
                AuthCommand::Logout => auth::logout(&store),
            }
        }
        Command::Config { command } => match command {
            ConfigCommand::Get { key } => config_cmd::get(&key),
            ConfigCommand::Set { key, value } => config_cmd::set(&key, &value),
        },
        Command::Budgets { command } => match command {
            BudgetsCommand::List => {
                let mut ctx = context::build_ctx(json, budget.as_deref(), no_cache).await?;
                budgets::list(&mut ctx).await
            }
        },
        Command::Accounts { command } => match command {
            AccountsCommand::List => {
                let mut ctx = context::build_ctx(json, budget.as_deref(), no_cache).await?;
                accounts::list(&mut ctx).await
            }
        },
        Command::Categories { command } => match command {
            CategoriesCommand::List => {
                let mut ctx = context::build_ctx(json, budget.as_deref(), no_cache).await?;
                categories::list(&mut ctx).await
            }
        },
        Command::Payees { command } => match command {
            PayeesCommand::List => {
                let mut ctx = context::build_ctx(json, budget.as_deref(), no_cache).await?;
                payees::list(&mut ctx).await
            }
        },
        Command::Transactions { command } => match command {
            TransactionsCommand::List {
                since,
                until,
                payee,
                account,
                category,
                uncategorized,
                unapproved,
            } => {
                let mut ctx = context::build_ctx(json, budget.as_deref(), no_cache).await?;
                transactions::list(
                    &mut ctx,
                    transactions::Filters {
                        since,
                        until,
                        payee,
                        account,
                        category,
                        uncategorized,
                        unapproved,
                    },
                )
                .await
            }
        },
        Command::Cache { command } => match command {
            CacheCommand::Status => cache_cmd::status(json),
            CacheCommand::Clear => cache_cmd::clear(),
        },
        Command::Mcp { command } => match command {
            McpCommand::Serve => crate::mcp::serve().await,
        },
    }
}
