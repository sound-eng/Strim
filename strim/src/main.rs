use anyhow::Result;
use clap::Parser;

mod cli;
mod server;
mod client;
mod config;

fn main() -> Result<()> {
    let args = cli::Args::parse();
    match args.mode {
        Some(cli::Mode::Client(a)) => client::run(a.host, a.port),
        Some(cli::Mode::Server(a)) => server::run(a.port, a.device_id),
        None => {
            // Default to server mode when no subcommand is provided
            server::run(args.server.port, args.server.device_id)
        }
    }
}

