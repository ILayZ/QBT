use qbacktest::cli::Cli;
use anyhow::Result;
use tracing_subscriber::EnvFilter;

fn main() -> Result<()> {
    // Initialize logging
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("qbacktest=info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .init();

    let cli: Cli = argh::from_env();

    match &cli.command {
        qbacktest::cli::Command::Run(args) => args.execute(),
        qbacktest::cli::Command::WalkForward(args) => args.execute(),
        qbacktest::cli::Command::Report(args) => args.execute(),
        qbacktest::cli::Command::Strategies(args) => args.execute(),
    }
}
