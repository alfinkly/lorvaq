//! Command line: `lorvaq server [--demo] [--listen ADDR]`.

use clap::{Parser, Subcommand};
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
    Server {
        /// Use the built-in simulator instead of real sources (no database needed).
        #[arg(long)]
        demo: bool,
        /// Address to listen on.
        #[arg(long, env = "LORVAQ_LISTEN", default_value = "0.0.0.0:8080")]
        listen: SocketAddr,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn server_demo_with_listen() {
        let cli = Cli::try_parse_from(["lorvaq", "server", "--demo", "--listen", "127.0.0.1:9000"])
            .unwrap();
        let Command::Server { demo, listen } = cli.command;
        assert!(demo);
        assert_eq!(listen.port(), 9000);
    }

    #[test]
    fn server_defaults_to_port_8080_without_demo() {
        let cli = Cli::try_parse_from(["lorvaq", "server"]).unwrap();
        let Command::Server { demo, listen } = cli.command;
        assert!(!demo);
        assert_eq!(listen.port(), 8080);
    }

    #[test]
    fn unknown_command_is_an_error() {
        assert!(Cli::try_parse_from(["lorvaq", "nope"]).is_err());
    }
}
