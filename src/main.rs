use clap::Parser;

#[tokio::main]
async fn main() {
    let cli = ynab_cli::cli::Cli::parse();
    if let Err(e) = ynab_cli::cli::run(cli).await {
        // `ynab ... | head` closes stdout on us. The reader got what it
        // asked for, so exit clean and silent like any other Unix filter.
        if ynab_cli::error::broken_pipe(&e) {
            return;
        }
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
