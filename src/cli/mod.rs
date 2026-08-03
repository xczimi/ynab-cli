pub mod auth;
pub mod budgets;
pub mod config_cmd;
pub mod context;
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
    /// List transactions
    Transactions {
        #[command(subcommand)]
        command: TransactionsCommand,
    },
}

#[derive(Debug, Subcommand)]
pub enum BudgetsCommand {
    /// List all budgets
    List,
}

#[derive(Debug, Subcommand)]
pub enum TransactionsCommand {
    /// List transactions (filters combine with AND)
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
    let json = cli.json;
    let budget = cli.budget.clone();
    match cli.command {
        Command::Auth { command } => {
            let store = crate::secrets::SecretStore::new()?;
            let base = std::env::var("YNAB_CLI_API_BASE_URL").ok();
            match command {
                AuthCommand::Login => auth::login(&store, base).await,
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
                let ctx = context::build_ctx(json, budget.as_deref())?;
                budgets::list(&ctx).await
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
                let ctx = context::build_ctx(json, budget.as_deref())?;
                transactions::list(
                    &ctx,
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
    }
}
