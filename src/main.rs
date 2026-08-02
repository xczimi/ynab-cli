use clap::Parser;

#[tokio::main]
async fn main() {
    let cli = ynab_cli::cli::Cli::parse();
    if let Err(e) = ynab_cli::cli::run(cli).await {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
