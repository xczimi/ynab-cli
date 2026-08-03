pub mod accounts;
pub mod auth;
pub mod budgets;
pub mod categories;
pub mod config_cmd;
pub mod context;
pub mod payees;

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
        Command::Accounts { command } => match command {
            AccountsCommand::List => {
                let ctx = context::build_ctx(json, budget.as_deref())?;
                accounts::list(&ctx).await
            }
        },
        Command::Categories { command } => match command {
            CategoriesCommand::List => {
                let ctx = context::build_ctx(json, budget.as_deref())?;
                categories::list(&ctx).await
            }
        },
        Command::Payees { command } => match command {
            PayeesCommand::List => {
                let ctx = context::build_ctx(json, budget.as_deref())?;
                payees::list(&ctx).await
            }
        },
    }
}
