mod cli;
mod manifest;

use anyhow::Result;
use cli::{Cli, Command};
use tracing_subscriber::EnvFilter;

fn main() -> Result<()> {
    // Initialize logging
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("qbt=info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    let cli: Cli = argh::from_env();

    match &cli.command {
        Command::Replay(args) => args.execute(),
        Command::Run(args) => args.execute(),
        Command::WalkForward(args) => args.execute(),
        Command::Report(args) => args.execute(),
        Command::Strategies(args) => args.execute(),
    }
}
