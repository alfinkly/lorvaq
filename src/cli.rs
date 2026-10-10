//! Command line: `lorvaq server`, `lorvaq admin reset-password`, `lorvaq keygen`.

use clap::{Args, Parser, Subcommand};
use std::net::SocketAddr;

#[derive(Debug, Parser)]
#[command(
    name = "lorvaq",
    version,
    about = "Live scenes for real-time monitoring"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Run the server: sources, scenes, API and UI.
    Server(ServerArgs),
    /// Administration that works on the database directly.
    Admin {
        #[command(subcommand)]
        action: AdminAction,
        #[command(flatten)]
        db: DbArgs,
    },
    /// Print a new random LORVAQ_SECRET_KEY.
    Keygen,
}

#[derive(Debug, Args)]
pub struct ServerArgs {
    /// Use the built-in simulator instead of real sources (no database needed).
    #[arg(long)]
    pub demo: bool,
    /// Address to listen on.
    #[arg(long, env = "LORVAQ_LISTEN", default_value = "0.0.0.0:8080")]
    pub listen: SocketAddr,
    /// The URL people open lorvaq at; https:// turns on the Secure cookie flag.
    #[arg(long, env = "LORVAQ_PUBLIC_URL")]
    pub public_url: Option<String>,
    #[command(flatten)]
    pub db: DbArgs,
    /// 32 bytes in base64 that encrypt stored secrets (`lorvaq keygen`).
    #[arg(long, env = "LORVAQ_SECRET_KEY", hide_env_values = true)]
    pub secret_key: Option<String>,
}

#[derive(Debug, Args)]
pub struct DbArgs {
    /// Postgres connection URL.
    #[arg(long, env = "DATABASE_URL", hide_env_values = true, global = true)]
    pub database_url: Option<String>,
}

#[derive(Debug, Subcommand)]
pub enum AdminAction {
    /// Remove the admin password, sign everybody out and print a new one-time setup code.
    ResetPassword,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_demo_with_listen() {
        let cli = Cli::try_parse_from(["lorvaq", "server", "--demo", "--listen", "127.0.0.1:9000"])
            .unwrap();
        let Command::Server(args) = cli.command else {
            panic!("server expected")
        };
        assert!(args.demo);
        assert_eq!(args.listen.port(), 9000);
    }

    #[test]
    fn server_takes_database_and_key_from_flags() {
        let cli = Cli::try_parse_from([
            "lorvaq",
            "server",
            "--database-url",
            "postgres://x",
            "--secret-key",
            "k",
            "--public-url",
            "https://l",
        ])
        .unwrap();
        let Command::Server(args) = cli.command else {
            panic!("server expected")
        };
        assert!(!args.demo);
        assert_eq!(args.db.database_url.as_deref(), Some("postgres://x"));
        assert_eq!(args.secret_key.as_deref(), Some("k"));
        assert_eq!(args.public_url.as_deref(), Some("https://l"));
    }

    #[test]
    fn admin_reset_password_and_keygen() {
        let cli = Cli::try_parse_from([
            "lorvaq",
            "admin",
            "reset-password",
            "--database-url",
            "postgres://x",
        ])
        .unwrap();
        let Command::Admin {
            action: AdminAction::ResetPassword,
            db,
        } = cli.command
        else {
            panic!("admin expected")
        };
        assert_eq!(db.database_url.as_deref(), Some("postgres://x"));
        assert!(matches!(
            Cli::try_parse_from(["lorvaq", "keygen"]).unwrap().command,
            Command::Keygen
        ));
    }

    #[test]
    fn unknown_command_is_an_error() {
        assert!(Cli::try_parse_from(["lorvaq", "nope"]).is_err());
    }
}
