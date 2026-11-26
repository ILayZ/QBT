use anyhow::Result;
use qbt::cli::Cli;
use tracing_subscriber::EnvFilter;

fn main() -> Result<()> {
    // Initialize logging
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("qbt=info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    let cli: Cli = argh::from_env();

    match &cli.command {
        qbt::cli::Command::Run(args) => args.execute(),
        qbt::cli::Command::WalkForward(args) => args.execute(),
        qbt::cli::Command::Report(args) => args.execute(),
        qbt::cli::Command::Strategies(args) => args.execute(),
    }
}
