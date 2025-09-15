use anyhow::Result;
use clap::Parser;

mod cli;
mod server;
mod client;
mod config;

fn main() -> Result<()> {
    let args = cli::Args::parse();
    
    // Display app version and mode on startup
    let version = env!("CARGO_PKG_VERSION");
    match args.mode {
        Some(cli::Mode::Client(a)) => {
            println!(">> Strim v{} - Client Mode", version);
            println!(">> Connecting to {}:{}\n", a.host, a.port);
            client::run(a.host, a.port)
        },
        Some(cli::Mode::Server(a)) => {
            println!("[] Strim v{} - Server Mode", version);
            println!("[] Listening on port {}\n", a.port);
            server::run(a.port, a.device_id)
        },
        None => {
            // Default to server mode when no subcommand is provided
            println!("Strim v{} - Server Mode (default)", version);
            println!("Starting server on port {}\n", args.server.port);
            server::run(args.server.port, args.server.device_id)
        }
    }
}

