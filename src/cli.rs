//! Command line: `lorvaq server`, `lorvaq agent`, `lorvaq admin reset-password`, `lorvaq keygen`.

use clap::{Args, Parser, Subcommand};
use std::net::SocketAddr;
use std::path::PathBuf;

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
    /// Run next to gitlab-runner and report its job containers to a lorvaq server.
    Agent(AgentArgs),
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
pub struct AgentArgs {
    /// The lorvaq server, like https://lorvaq.example.com.
    #[arg(long, env = "LORVAQ_SERVER")]
    pub server: String,
    /// The join token from Settings → Hosts (prefer the environment variable).
    #[arg(long, env = "LORVAQ_JOIN", hide_env_values = true)]
    pub join: String,
    /// The host's name on the scene; agents with the same name share a host.
    #[arg(long, env = "LORVAQ_HOST_LABEL")]
    pub host_label: Option<String>,
    /// The name of the gitlab-runner process on the scene.
    #[arg(long, env = "LORVAQ_PROCESS", default_value = "gitlab-runner")]
    pub process: String,
    /// gitlab-runner's metrics address (listen_address in its config.toml), for
    /// `concurrent` and per-runner `limit`.
    #[arg(long, env = "LORVAQ_RUNNER_METRICS")]
    pub runner_metrics: Option<String>,
    /// A path on docker's data partition; its fill is the host's disk level.
    #[arg(long, env = "LORVAQ_DISK_PATH", default_value = "/")]
    pub disk_path: PathBuf,
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
    fn agent_takes_its_server_and_token() {
        let cli = Cli::try_parse_from([
            "lorvaq",
            "agent",
            "--server",
            "https://lorvaq.example.com",
            "--join",
            "t",
            "--runner-metrics",
            "http://127.0.0.1:9252/metrics",
        ])
        .unwrap();
        let Command::Agent(args) = cli.command else {
            panic!("agent expected")
        };
        assert_eq!(
            (
                args.server.as_str(),
                args.join.as_str(),
                args.process.as_str()
            ),
            ("https://lorvaq.example.com", "t", "gitlab-runner")
        );
        assert_eq!(args.disk_path, PathBuf::from("/"));
    }

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
