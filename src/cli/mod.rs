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
