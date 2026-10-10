use clap::Parser;
use lorvaq::app;
use lorvaq::cli::{Cli, Command};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();
    match Cli::parse().command {
        Command::Server { demo, listen } => {
            anyhow::ensure!(
                demo,
                "this build only runs with --demo: real sources arrive in the next releases"
            );
            app::serve(listen, app::demo_scenes()?).await
        }
    }
}
