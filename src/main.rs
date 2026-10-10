use clap::Parser;
use lorvaq::app;
use lorvaq::cli::{Cli, Command};
use lorvaq::crypto::SecretBox;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();
    match Cli::parse().command {
        Command::Keygen => {
            println!("{}", SecretBox::generate_key());
            Ok(())
        }
        Command::Server(args) => {
            anyhow::ensure!(
                args.demo,
                "the live mode is wired in a later step of this plan; use --demo"
            );
            app::serve(args.listen, app::demo_scenes()?).await
        }
        Command::Admin { .. } => {
            anyhow::bail!("admin commands are wired in a later step of this plan")
        }
    }
}
