use clap::Parser;
use lorvaq::cli::{Cli, Command};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();
    let Command::Server { demo, listen } = Cli::parse().command;
    anyhow::ensure!(demo, "this build only runs with --demo");
    tracing::info!(%listen, "nothing to serve yet");
    Ok(())
}
